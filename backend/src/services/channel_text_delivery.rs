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

pub async fn begin(
    db: &Database,
    key: &str,
    owner: &str,
    source: &str,
    fingerprint: &str,
    count: usize,
) -> AppResult<ChannelTextDelivery> {
    begin_with_notice(db, key, owner, source, fingerprint, count, None).await
}

#[allow(clippy::too_many_arguments)]
pub async fn begin_with_notice(
    db: &Database,
    key: &str,
    owner: &str,
    source: &str,
    fingerprint: &str,
    count: usize,
    notice: Option<crate::models::channel_text_delivery::TextDeliveryNotice>,
) -> AppResult<ChannelTextDelivery> {
    if count == 0 || count > super::channel_reply_delivery::MAX_REPLY_PARTS {
        return Err(AppError::ValidationError("Invalid multipart plan".into()));
    }
    let record = ChannelTextDelivery {
        id: uuid::Uuid::new_v4().to_string(),
        logical_key: key.into(),
        user_id: owner.into(),
        source_id: source.into(),
        fingerprint: fingerprint.into(),
        notice,
        retry_at: None,
        parts: vec![
            TextPart {
                state: TextPartState::Pending,
                attempt_id: None,
                platform_message_id: None,
                outbound_message_id: None
            };
            count
        ],
        created_at: chrono::Utc::now(),
    };
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
    let stored = load(db, key).await?.ok_or_else(unconfirmed)?;
    if stored.fingerprint != fingerprint
        || stored.parts.len() != count
        || stored.user_id != owner
        || stored.source_id != source
    {
        return Err(AppError::Conflict("Multipart delivery plan changed".into()));
    }
    Ok(stored)
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
    let changed = db.collection::<ChannelTextDelivery>(COLLECTION_NAME).update_one(
        doc! {"_id": &row.id, format!("parts.{index}.state"): "sending",
            format!("parts.{index}.attempt_id"): attempt},
        doc! {"$set": {format!("parts.{index}.state"): bson::to_bson(&state).map_err(|_| unconfirmed())?,
            format!("parts.{index}.platform_message_id"): platform_id,
            format!("parts.{index}.outbound_message_id"): outbound_id}},
    ).await?;
    if changed.matched_count != 1 {
        return Err(unconfirmed());
    }
    Ok(())
}

pub const UNCERTAIN: &str =
    "Multipart delivery is incomplete or uncertain; accepted parts will not be resent";

pub async fn defer(
    db: &Database,
    row: &ChannelTextDelivery,
    index: usize,
    attempt: &str,
    retry_after: u64,
) -> AppResult<()> {
    let retry_at =
        chrono::Utc::now() + chrono::Duration::seconds(retry_after.clamp(1, 86400) as i64);
    let changed = db.collection::<ChannelTextDelivery>(COLLECTION_NAME).update_one(
        doc! {"_id":&row.id, format!("parts.{index}.state"):"sending", format!("parts.{index}.attempt_id"):attempt},
        doc! {"$set": {format!("parts.{index}.state"):"pending", "retry_at":bson::DateTime::from_chrono(retry_at)}},
    ).await?;
    if changed.matched_count != 1 {
        return Err(unconfirmed());
    }
    Ok(())
}

pub fn unconfirmed() -> AppError {
    AppError::ChannelPlatformError(UNCERTAIN.into())
}

#[cfg(test)]
mod tests {
    use super::*;
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
