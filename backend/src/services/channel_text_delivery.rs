//! Resumable ordered sends. The durable plan contains hashes and receipts only.
use crate::{
    errors::{AppError, AppResult},
    models::channel_text_delivery::{
        COLLECTION_NAME, ChannelTextDelivery, TextPart, TextPartState,
    },
};
use bson::doc;
use mongodb::{Database, IndexModel, options::IndexOptions};

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<ChannelTextDelivery>(COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"logical_key": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "source_id": 1})
                .build(),
            // Non-sparse: historical plans omit the bot parent. Support the
            // exact owner/null-parent page without scanning modern plans.
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "channel_bot_id": 1, "_id": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"channel_bot_id": 1})
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
        ])
        .await?;
    Ok(())
}

pub async fn load(db: &Database, key: &str) -> AppResult<Option<ChannelTextDelivery>> {
    Ok(db
        .collection(COLLECTION_NAME)
        .find_one(doc! {"logical_key": key})
        .await?)
}

#[cfg(test)]
pub async fn begin(
    db: &Database,
    key: &str,
    owner: &str,
    source: &str,
    fingerprint: &str,
    count: usize,
) -> AppResult<ChannelTextDelivery> {
    begin_versioned(db, key, owner, source, fingerprint, count, None, None, None).await
}

pub(crate) async fn begin_bound(
    db: &Database,
    key: &str,
    bot: &crate::models::channel_bot::ChannelBot,
    source: &str,
    fingerprint: &str,
    count: usize,
) -> AppResult<ChannelTextDelivery> {
    begin_parented(db, key, &bot.user_id, source, fingerprint, count, bot).await
}

/// Native transports historically journal under the linking person's owner.
/// Preserve that identity while recording the independently verified bot parent.
pub(crate) async fn begin_parented(
    db: &Database,
    key: &str,
    owner: &str,
    source: &str,
    fingerprint: &str,
    count: usize,
    bot: &crate::models::channel_bot::ChannelBot,
) -> AppResult<ChannelTextDelivery> {
    begin_versioned(
        db,
        key,
        owner,
        source,
        fingerprint,
        count,
        None,
        None,
        Some(bot),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn begin_versioned(
    db: &Database,
    key: &str,
    owner: &str,
    source: &str,
    fingerprint: &str,
    count: usize,
    notice: Option<crate::models::channel_text_delivery::TextDeliveryNotice>,
    renderer_version: Option<u32>,
    bound_bot: Option<&crate::models::channel_bot::ChannelBot>,
) -> AppResult<ChannelTextDelivery> {
    if count == 0 || count > super::channel_reply_delivery::MAX_REPLY_PARTS {
        return Err(AppError::ValidationError("Invalid multipart plan".into()));
    }
    let record = ChannelTextDelivery {
        meta_grant: bound_bot
            .filter(|bot| super::channel_adapters::meta::is_meta(&bot.platform))
            .and_then(|bot| {
                bot.meta_binding
                    .as_ref()
                    .map(|binding| crate::models::channel_meta::GrantRef {
                        bot_id: bot.id.clone(),
                        grant_id: binding.grant_id.clone(),
                        generation: binding.generation,
                    })
            }),
        id: uuid::Uuid::new_v4().to_string(),
        logical_key: key.into(),
        user_id: owner.into(),
        source_id: source.into(),
        channel_bot_id: bound_bot.map(|bot| bot.id.clone()),
        fingerprint: fingerprint.into(),
        renderer_version,
        notice,
        retry_at: None,
        parts: vec![
            TextPart {
                preparation: None,
                formatting_fallback: None,
                state: TextPartState::Pending,
                attempt_id: None,
                platform_message_id: None,
                outbound_message_id: None
            };
            count
        ],
        created_at: chrono::Utc::now(),
    };
    if let Some(bot) = bound_bot.filter(|bot| super::channel_adapters::meta::is_meta(&bot.platform))
    {
        #[cfg(test)]
        super::durable_wake_tests::after("meta_text_plan_before_fence").await;
        return super::transaction_service::run_db_only(
            db,
            &(db, bot, &record),
            |tx, &(db, bot, record)| {
                Box::pin(async move {
                    super::channel_relay_service::fence_meta_observation(
                        db, tx, &bot.id, &bot.platform, true,
                    )
                    .await?;
                    if db
                        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
                        .find_one(doc! {"_id": &bot.id, "user_id": &bot.user_id,"meta_binding.generation":bot.meta_binding.as_ref().map(|binding|binding.generation)})
                        .session(&mut *tx)
                        .await?
                        .is_none()
                    {
                        return Err(AppError::Conflict("Inbox ownership changed".into()));
                    }
                    super::channel_meta_service::scopes::fence_current(db,tx,bot).await?;
                    db.collection::<ChannelTextDelivery>(COLLECTION_NAME)
                        .update_one(
                            doc! {"logical_key": &record.logical_key},
                            doc! {"$setOnInsert": bson::to_document(record).map_err(|_| unconfirmed())?},
                        )
                        .upsert(true)
                        .session(&mut *tx)
                        .await?;
                    let stored = db
                        .collection::<ChannelTextDelivery>(COLLECTION_NAME)
                        .find_one(doc! {"logical_key": &record.logical_key})
                        .session(&mut *tx)
                        .await?
                        .ok_or_else(unconfirmed)?;
                    validate_plan(&stored, record)?;
                    // An existing legacy plan is adopted under the same fence.
                    if stored.channel_bot_id.is_none() {
                        db.collection::<ChannelTextDelivery>(COLLECTION_NAME)
                            .update_one(
                                doc! {"_id": &stored.id, "channel_bot_id": null},
                                doc! {"$set": {"channel_bot_id": &bot.id}},
                            )
                            .session(&mut *tx)
                            .await?;
                    }
                    let mut stored = stored;
                    stored.channel_bot_id = Some(bot.id.clone());
                    Ok(stored)
                })
            },
        )
        .await;
    }
    let inserted = db
        .collection::<ChannelTextDelivery>(COLLECTION_NAME)
        .update_one(
            doc! {"logical_key": key},
            doc! {"$setOnInsert": bson::to_document(&record).map_err(|_| unconfirmed())?},
        )
        .upsert(true)
        .await;
    if let Err(error) = inserted
        && !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(
            mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000)
    {
        return Err(error.into());
    }
    let mut stored = load(db, key).await?.ok_or_else(unconfirmed)?;
    validate_plan(&stored, &record)?;
    if let Some(bot) = bound_bot
        && stored.channel_bot_id.is_none()
    {
        // Adopting an old non-Meta plan changes metadata only. Keep the
        // original ordinary write path and never recreate a deleted receipt.
        db.collection::<ChannelTextDelivery>(COLLECTION_NAME).update_one(
            doc! {"_id": &stored.id, "user_id": owner, "source_id": source, "channel_bot_id": null},
            doc! {"$set": {"channel_bot_id": &bot.id}},
        ).await?;
        stored = load(db, key).await?.ok_or_else(unconfirmed)?;
        validate_plan(&stored, &record)?;
        if stored.channel_bot_id.as_deref() != Some(bot.id.as_str()) {
            return Err(AppError::Conflict(
                "Multipart delivery parent changed".into(),
            ));
        }
    }
    Ok(stored)
}

fn validate_plan(stored: &ChannelTextDelivery, expected: &ChannelTextDelivery) -> AppResult<()> {
    if stored.fingerprint != expected.fingerprint
        || stored.parts.len() != expected.parts.len()
        || stored.user_id != expected.user_id
        || stored.source_id != expected.source_id
        || stored.channel_bot_id.as_ref().is_some_and(|id| {
            expected
                .channel_bot_id
                .as_ref()
                .is_some_and(|expected| id != expected)
        })
    {
        return Err(AppError::Conflict("Multipart delivery plan changed".into()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn assistant_plan(
    db: &Database,
    key: &str,
    owner: &str,
    source: &str,
    platform: &str,
    notice: Option<crate::models::channel_text_delivery::TextDeliveryNotice>,
    build: impl Fn(
        Option<u32>,
        Option<&crate::models::channel_text_delivery::TextDeliveryNotice>,
    ) -> AppResult<(Vec<super::channel_platform::OutboundReply>, String)>,
) -> AppResult<(
    ChannelTextDelivery,
    Vec<super::channel_platform::OutboundReply>,
)> {
    for _ in 0..2 {
        let previous = load(db, key).await?;
        #[cfg(test)]
        if previous.is_none() {
            super::durable_wake_tests::after("text_renderer_before_insert").await;
        }
        let version = previous.as_ref().map_or_else(
            || super::channel_reply_plan::renderer_version(platform),
            |row| row.renderer_version,
        );
        let notice = previous
            .as_ref()
            .map_or_else(|| notice.clone(), |row| row.notice.clone());
        let (parts, fingerprint) = build(version, notice.as_ref())?;
        match begin_versioned(
            db,
            key,
            owner,
            source,
            &fingerprint,
            parts.len(),
            notice,
            version,
            None,
        )
        .await
        {
            Ok(row) => return Ok((row, parts)),
            Err(AppError::Conflict(_)) if previous.is_none() => continue,
            Err(error) => return Err(error),
        }
    }
    Err(AppError::Conflict("Multipart delivery plan changed".into()))
}

pub fn recoverable(row: &ChannelTextDelivery) -> bool {
    row.parts
        .iter()
        .all(|part| matches!(part.state, TextPartState::Pending | TextPartState::Sent))
}

pub enum PartClaim {
    Send(String),
    Sent(TextPart),
}

pub async fn claim(db: &Database, row: &ChannelTextDelivery, index: usize) -> AppResult<PartClaim> {
    let attempt = uuid::Uuid::new_v4().to_string();
    let mut filter = doc! {"_id": &row.id, "fingerprint": &row.fingerprint,
    format!("parts.{index}.state"): "pending",
    "$or":[{"retry_at":bson::Bson::Null},{"retry_at":{"$lte":bson::DateTime::now()}}]};
    for previous in 0..index {
        filter.insert(format!("parts.{previous}.state"), "sent");
    }
    let changed = db
        .collection::<ChannelTextDelivery>(COLLECTION_NAME)
        .update_one(
            filter,
            doc! {"$set": {format!("parts.{index}.state"): "sending",
            format!("parts.{index}.attempt_id"): &attempt}},
        )
        .await?;
    if changed.matched_count == 1 {
        #[cfg(test)]
        super::durable_wake_tests::after("text_part_prepared").await;
        return Ok(PartClaim::Send(attempt));
    }
    let current = load(db, &row.logical_key).await?.ok_or_else(unconfirmed)?;
    match current.parts.get(index) {
        Some(part) if part.state == TextPartState::Sent => Ok(PartClaim::Sent(part.clone())),
        Some(part)
            if part.state == TextPartState::Pending
                && current.retry_at.is_some_and(|at| at > chrono::Utc::now()) =>
        {
            let seconds = (current.retry_at.unwrap() - chrono::Utc::now())
                .num_seconds()
                .max(0) as u64
                + 1;
            Err(AppError::RateLimitedWithRetry {
                retry_after: seconds,
            })
        }
        Some(part) if part.state == TextPartState::Refused => {
            Err(AppError::Conflict("Multipart delivery was refused".into()))
        }
        _ => Err(unconfirmed()),
    }
}

pub async fn finish(
    db: &Database,
    row: &ChannelTextDelivery,
    index: usize,
    attempt: &str,
    state: TextPartState,
    platform_id: Option<&str>,
    outbound_id: Option<&str>,
) -> AppResult<()> {
    #[cfg(test)]
    if state == TextPartState::Sent {
        super::durable_wake_tests::after("text_part_provider_accepted").await;
    }
    let mut filter = doc! {"_id": &row.id, format!("parts.{index}.attempt_id"): attempt};
    if state == TextPartState::Sent {
        filter.insert(format!("parts.{index}.state"), "sending");
    } else {
        filter.insert("$or", bson::bson!([
            {format!("parts.{index}.state"):"sending"},
            {format!("parts.{index}.state"):"pending",format!("parts.{index}.preparation.version"):1}
        ]));
    }
    let changed = db.collection::<ChannelTextDelivery>(COLLECTION_NAME).update_one(
        filter,
        doc! {"$set": {format!("parts.{index}.state"): bson::to_bson(&state).map_err(|_| unconfirmed())?,
            format!("parts.{index}.platform_message_id"): platform_id,
            format!("parts.{index}.outbound_message_id"): outbound_id},
            "$unset":{format!("parts.{index}.preparation"):""}},
    ).await?;
    if changed.matched_count != 1 {
        return Err(unconfirmed());
    }
    Ok(())
}

pub const UNCERTAIN: &str =
    "Multipart delivery is incomplete or uncertain; accepted parts will not be resent";

// Aggregation updates do not interpret numeric dotted components as array
// indexes. Replace the selected element while preserving the rest of the plan.
pub(super) fn part_update(index: usize, fields: bson::Document) -> bson::Document {
    doc! {"parts": {"$map": {
        "input": {"$range": [0, {"$size": "$parts"}]}, "as": "i",
        "in": {"$let": {"vars": {"part": {"$arrayElemAt": ["$parts", "$$i"]}},
            "in": {"$cond": [
                {"$eq": ["$$i", index as i64]},
                {"$mergeObjects": ["$$part", fields]},
                "$$part",
            ]},
        }},
    }}}
}

pub(super) fn part_field(index: usize, field: &str) -> bson::Document {
    doc! {"$let": {"vars": {"part": {"$arrayElemAt": ["$parts", index as i64]}},
    "in": format!("$$part.{field}")}}
}

pub async fn defer(
    db: &Database,
    row: &ChannelTextDelivery,
    index: usize,
    attempt: &str,
    retry_after: u64,
) -> AppResult<()> {
    let mut set = part_update(
        index,
        doc! {"state":"pending", "preparation":bson::Bson::Null},
    );
    set.insert(
        "retry_at",
        doc! {"$add":["$$NOW", retry_after.clamp(1,86400) as i64 * 1000]},
    );
    let changed = db.collection::<ChannelTextDelivery>(COLLECTION_NAME).update_one(
        doc! {"_id":&row.id, format!("parts.{index}.attempt_id"):attempt,"$or":[
            {format!("parts.{index}.state"):"sending"},
            {format!("parts.{index}.state"):"pending",format!("parts.{index}.preparation.version"):1}
        ]},
        vec![doc! {"$set": set}],
    ).await?;
    if changed.matched_count != 1 {
        return Err(unconfirmed());
    }
    Ok(())
}

pub fn unconfirmed() -> AppError {
    AppError::ChannelPlatformError(UNCERTAIN.into())
}

/// The original claim remains Sending throughout a formatting retry.
pub fn formatting_receipt(
    row: &ChannelTextDelivery,
    index: usize,
    attempt: &str,
) -> super::telegram_formatting::Receipt {
    super::telegram_formatting::Receipt {
        collection: COLLECTION_NAME,
        filter: doc! {"_id": &row.id, format!("parts.{index}.state"): "sending", format!("parts.{index}.attempt_id"): attempt},
        field: format!("parts.{index}.formatting_fallback"),
        plain: row.parts[index].formatting_fallback.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rendered(
        version: Option<u32>,
        platform: &str,
    ) -> AppResult<(Vec<super::super::channel_platform::OutboundReply>, String)> {
        let parts = super::super::channel_reply_plan::recorded_assistant_parts(
            &"A literal (value). https://example.test/a-b?x=1&y=2\n\n".repeat(250),
            platform,
            "https://nyxid.test/thread",
            version,
        )?;
        let digest = super::super::channel_reply_plan::fingerprint(&parts);
        Ok((parts, digest))
    }

    #[tokio::test]
    async fn non_meta_saved_results_record_and_adopt_bot_parents_without_a_transaction_or_losing_receipts()
     {
        let (state, _, _server, mut bot) =
            crate::services::channel_adapters::meta::tests::fixture("instagram").await;
        bot.platform = "slack".into();
        bot.meta_binding = None;
        state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .replace_one(doc! {"_id": &bot.id}, &bot)
            .await
            .unwrap();
        let (db, commands) = super::super::durable_wake_tests::monitored(&state.db).await;
        let key = format!("saved-result:{}:{}", uuid::Uuid::new_v4(), "d".repeat(64));
        let fresh = begin_bound(&db, &key, &bot, "source", "hash", 1)
            .await
            .unwrap();
        assert_eq!(fresh.channel_bot_id.as_deref(), Some(bot.id.as_str()));
        let PartClaim::Send(attempt) = claim(&db, &fresh, 0).await.unwrap() else {
            panic!("new part");
        };
        finish(
            &db,
            &fresh,
            0,
            &attempt,
            TextPartState::Sent,
            Some("slack-provider-id"),
            Some("outbound"),
        )
        .await
        .unwrap();
        let resumed = begin_bound(&db, &key, &bot, "source", "hash", 1)
            .await
            .unwrap();
        assert!(
            matches!(claim(&db, &resumed, 0).await.unwrap(), PartClaim::Sent(part)
            if part.platform_message_id.as_deref() == Some("slack-provider-id"))
        );
        begin(
            &db,
            "legacy",
            &bot.user_id,
            "legacy-source",
            "legacy-hash",
            1,
        )
        .await
        .unwrap();
        let adopted = begin_bound(&db, "legacy", &bot, "legacy-source", "legacy-hash", 1)
            .await
            .unwrap();
        assert_eq!(adopted.channel_bot_id.as_deref(), Some(bot.id.as_str()));
        let transport_person = uuid::Uuid::new_v4().to_string();
        let native = begin_parented(
            &db,
            "native-saved-result",
            &transport_person,
            "native-event",
            "native-hash",
            1,
            &bot,
        )
        .await
        .unwrap();
        assert_eq!(native.user_id, transport_person);
        assert_eq!(native.channel_bot_id.as_deref(), Some(bot.id.as_str()));
        let observed = super::super::durable_wake_tests::take(&commands);
        assert!(
            !observed
                .keys()
                .any(|command| command.starts_with("commitTransaction:")
                    || command.starts_with("abortTransaction:"))
        );
    }

    #[tokio::test]
    async fn renderer_versions_resume_only_the_pending_suffix_and_old_workers_refuse_v2() {
        let db = crate::test_utils::connect_transaction_test_database("text_renderer_resume").await;
        ensure_indexes(&db).await.unwrap();
        for platform in ["discord", "whatsapp"] {
            for version in [None, Some(2)] {
                let key = format!("{platform}-{version:?}");
                let (parts, fingerprint) = rendered(version, platform).unwrap();
                let row = if version.is_none() {
                    begin(&db, &key, "owner", "source", &fingerprint, parts.len())
                        .await
                        .unwrap()
                } else {
                    assistant_plan(&db, &key, "owner", "source", platform, None, |v, _| {
                        rendered(v, platform)
                    })
                    .await
                    .unwrap()
                    .0
                };
                assert_eq!(row.renderer_version, version);
                let PartClaim::Send(attempt) = claim(&db, &row, 0).await.unwrap() else {
                    panic!("pending");
                };
                finish(
                    &db,
                    &row,
                    0,
                    &attempt,
                    TextPartState::Sent,
                    Some("accepted-prefix"),
                    Some("outbound-prefix"),
                )
                .await
                .unwrap();
                let (resumed, resumed_parts) =
                    assistant_plan(&db, &key, "owner", "source", platform, None, |v, _| {
                        rendered(v, platform)
                    })
                    .await
                    .unwrap();
                assert_eq!(resumed.id, row.id);
                assert_eq!(resumed.fingerprint, fingerprint);
                assert_eq!(
                    super::super::channel_reply_plan::fingerprint(&resumed_parts),
                    fingerprint
                );
                assert!(
                    matches!(claim(&db, &resumed, 0).await.unwrap(), PartClaim::Sent(part) if part.platform_message_id.as_deref() == Some("accepted-prefix"))
                );
                if version.is_some() {
                    let (old_parts, old_fingerprint) = rendered(None, platform).unwrap();
                    assert!(matches!(
                        begin(
                            &db,
                            &key,
                            "owner",
                            "source",
                            &old_fingerprint,
                            old_parts.len()
                        )
                        .await,
                        Err(AppError::Conflict(_))
                    ));
                    assert_eq!(
                        load(&db, &key).await.unwrap().unwrap().parts[1].state,
                        TextPartState::Pending
                    );
                }
                let PartClaim::Send(attempt) = claim(&db, &resumed, 1).await.unwrap() else {
                    panic!("suffix");
                };
                assert!(
                    finish(
                        &db,
                        &resumed,
                        1,
                        "stale",
                        TextPartState::Sent,
                        Some("wrong"),
                        None
                    )
                    .await
                    .is_err()
                );
                // A crash after claiming cannot be made safe by selecting a renderer.
                let (uncertain, _) =
                    assistant_plan(&db, &key, "owner", "source", platform, None, |v, _| {
                        rendered(v, platform)
                    })
                    .await
                    .unwrap();
                assert!(claim(&db, &uncertain, 1).await.is_err());
                assert!(claim(&db, &uncertain, 2).await.is_err());
                finish(
                    &db,
                    &resumed,
                    1,
                    &attempt,
                    TextPartState::Sent,
                    Some("accepted-suffix"),
                    None,
                )
                .await
                .unwrap();
                let raw = db
                    .collection::<bson::Document>(COLLECTION_NAME)
                    .find_one(doc! {"_id":&row.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(raw.contains_key("renderer_version"), version.is_some());
                assert!(!raw.contains_key("text") && !raw.contains_key("body"));
            }
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn renderer_initialization_race_reloads_the_legacy_winner_and_pinned_notice() {
        let db = crate::test_utils::connect_transaction_test_database("text_renderer_race").await;
        ensure_indexes(&db).await.unwrap();
        let versions = std::sync::Mutex::new(Vec::new());
        let old = rendered(None, "discord").unwrap();
        super::super::durable_wake_tests::resume_after(
            "text_renderer_before_insert",
            async {
                let (row, _) = assistant_plan(
                    &db,
                    "race",
                    "owner",
                    "source",
                    "discord",
                    Some(crate::models::channel_text_delivery::TextDeliveryNotice {
                        attempted: 1,
                        sent: 0,
                        failed: 1,
                        unknown: 0,
                        skipped: 0,
                        unsupported: false,
                        app_visible: true,
                        automation: true,
                        omitted: 0,
                    }),
                    |version, notice| {
                        versions.lock().unwrap().push(version);
                        assert_eq!(notice.is_some(), version == Some(2));
                        rendered(version, "discord")
                    },
                )
                .await
                .unwrap();
                assert_eq!(row.renderer_version, None);
                assert_eq!(row.fingerprint, old.1);
            },
            async {
                begin(&db, "race", "owner", "source", &old.1, old.0.len())
                    .await
                    .unwrap();
            },
        )
        .await;
        assert_eq!(*versions.lock().unwrap(), vec![Some(2), None]);
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn renderer_new_plans_converge_without_changing_slack_or_telegram() {
        let db = crate::test_utils::connect_transaction_test_database("text_renderer_new").await;
        ensure_indexes(&db).await.unwrap();
        for platform in ["discord", "whatsapp", "slack", "telegram"] {
            let build =
                |version, _: Option<&crate::models::channel_text_delivery::TextDeliveryNotice>| {
                    rendered(version, platform)
                };
            let (a, b) = tokio::join!(
                assistant_plan(&db, platform, "owner", "source", platform, None, build),
                assistant_plan(&db, platform, "owner", "source", platform, None, build),
            );
            let a = a.unwrap().0;
            let b = b.unwrap().0;
            assert_eq!(a.id, b.id);
            assert_eq!(a.fingerprint, b.fingerprint);
            assert_eq!(
                a.renderer_version,
                super::super::channel_reply_plan::renderer_version(platform)
            );
            assert!(
                assistant_plan(
                    &db,
                    platform,
                    "different-owner",
                    "source",
                    platform,
                    None,
                    build
                )
                .await
                .is_err()
            );
            if matches!(platform, "slack" | "telegram") {
                assert_eq!(a.fingerprint, rendered(None, platform).unwrap().1);
            }
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn multipart_refusal_delay_survives_reload_and_fences_stale_attempts() {
        let db = crate::test_utils::connect_transaction_test_database("text_retry_delay").await;
        ensure_indexes(&db).await.unwrap();
        let row = begin(&db, "key", "owner", "source", "digest", 1)
            .await
            .unwrap();
        let PartClaim::Send(attempt) = claim(&db, &row, 0).await.unwrap() else {
            panic!("first attempt");
        };
        assert!(defer(&db, &row, 0, "stale", 25).await.is_err());
        defer(&db, &row, 0, &attempt, 25).await.unwrap();
        let saved = load(&db, "key").await.unwrap().unwrap();
        assert!(recoverable(&saved));
        let failure = claim(&db, &saved, 0).await.err().unwrap();
        assert!(
            failure
                .retry_after_seconds()
                .is_some_and(|delay| (1..=26).contains(&delay))
        );
        let document = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(document.get_datetime("retry_at").is_ok());
        db.collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"retry_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let PartClaim::Send(next) = claim(&db, &saved, 0).await.unwrap() else {
            panic!("retry attempt");
        };
        assert!(defer(&db, &row, 0, &attempt, 25).await.is_err());
        finish(
            &db,
            &row,
            0,
            &next,
            TextPartState::Sent,
            Some("receipt"),
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            claim(&db, &saved, 0).await.unwrap(),
            PartClaim::Sent(_)
        ));
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn multipart_claims_resume_only_proven_unsent_parts_and_bind_the_plan() {
        let db = crate::test_utils::connect_transaction_test_database("text_delivery").await;
        ensure_indexes(&db).await.unwrap();
        let plan = begin(&db, "request", "owner", "source", "digest", 3)
            .await
            .unwrap();
        let (a, b) = tokio::join!(claim(&db, &plan, 0), claim(&db, &plan, 0));
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let attempt = match a.or(b).unwrap() {
            PartClaim::Send(attempt) => attempt,
            _ => panic!("new claim"),
        };
        assert!(!recoverable(&load(&db, "request").await.unwrap().unwrap()));
        assert!(claim(&db, &plan, 1).await.is_err());
        assert!(
            finish(
                &db,
                &plan,
                0,
                "stale",
                TextPartState::Sent,
                Some("wrong"),
                None
            )
            .await
            .is_err()
        );
        finish(
            &db,
            &plan,
            0,
            &attempt,
            TextPartState::Sent,
            Some("first"),
            Some("row-first"),
        )
        .await
        .unwrap();
        let restarted = begin(&db, "request", "owner", "source", "digest", 3)
            .await
            .unwrap();
        assert!(recoverable(&restarted));
        assert!(
            matches!(claim(&db,&restarted,0).await.unwrap(),PartClaim::Sent(part) if part.platform_message_id.as_deref()==Some("first"))
        );
        let second = match claim(&db, &restarted, 1).await.unwrap() {
            PartClaim::Send(attempt) => attempt,
            _ => panic!("unsent part"),
        };
        // A documented provider rejection can retry, without replaying part 0.
        finish(
            &db,
            &restarted,
            1,
            &second,
            TextPartState::Pending,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(recoverable(&load(&db, "request").await.unwrap().unwrap()));
        assert!(matches!(
            claim(&db, &restarted, 1).await.unwrap(),
            PartClaim::Send(_)
        ));
        // Crash/lost response: sending remains uncertain and fences the suffix.
        assert!(claim(&db, &restarted, 1).await.is_err());
        assert!(claim(&db, &restarted, 2).await.is_err());
        assert!(
            begin(&db, "request", "owner", "source", "changed", 3)
                .await
                .is_err()
        );
        assert!(
            begin(&db, "request", "other", "source", "digest", 3)
                .await
                .is_err()
        );
        let raw = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert!(!raw.contains_key("text") && !raw.contains_key("body"));
        db.drop().await.unwrap();
    }
}
