//! Per-grant Meta identity; reconnect never copies retained source records.
use crate::{
    errors::AppResult,
    models::{
        channel_bot::ChannelBot,
        channel_meta::{GRANT_SCOPES, GrantRef, GrantScope},
    },
};
use bson::{Document, doc};
use mongodb::{ClientSession, Database};

pub(crate) async fn ensure_current(
    db: &Database,
    tx: &mut ClientSession,
    bot: &ChannelBot,
    fresh: bool,
) -> AppResult<GrantScope> {
    let binding = bot
        .meta_binding
        .as_ref()
        .ok_or_else(super::grants::incomplete)?;
    let identity = if binding.grant_subject_id.is_empty() {
        None
    } else {
        Some(super::provider::grant_identity(
            &bot.platform,
            &binding.app_id,
            &binding.grant_subject_id,
        )?)
    };
    let scope = GrantScope {
        id: uuid::Uuid::new_v4().to_string(),
        bot_id: bot.id.clone(),
        grant_id: binding.grant_id.clone(),
        identity,
        platform: bot.platform.clone(),
        app_id: binding.app_id.clone(),
        from_generation: if fresh {
            binding.generation
        } else {
            binding.grant_start_generation
        },
        to_generation: None,
        legacy_history: !fresh,
        binding_fingerprint: super::meta::binding(bot),
        authorized_at: binding.authorized_at,
        bot_created_at: bot.created_at,
        created_at: chrono::Utc::now(),
        sealed_at: None,
        status: "current".into(),
        outstanding: !binding.grant_subject_id.is_empty(),
        erasure_requested: false,
    };
    let rows = db.collection::<GrantScope>(GRANT_SCOPES);
    rows.update_one(
        doc! {"bot_id":&bot.id,"grant_id":&binding.grant_id},
        doc! {"$setOnInsert":bson::to_document(&scope).map_err(|_|super::grants::incomplete())?},
    )
    .upsert(true)
    .session(&mut *tx)
    .await?;
    let mut raw = db
        .collection::<Document>(GRANT_SCOPES)
        .find_one(doc! {"bot_id":&bot.id,"grant_id":&binding.grant_id})
        .session(&mut *tx)
        .await?
        .ok_or_else(super::grants::incomplete)?;
    // Terminal scopes intentionally omit attribution metadata. They are
    // readable through exact proof, and cannot re-enter mutable writer paths.
    if raw.get_str("status") == Ok("completed") {
        return Err(super::grants::incomplete());
    }
    // Chronology is evaluated separately from target identity. Preserve a
    // malformed stored date for repair; it must not abort sibling capture.
    if raw
        .get("authorized_at")
        .is_some_and(|value| !matches!(value, bson::Bson::DateTime(_) | bson::Bson::Null))
    {
        raw.remove("authorized_at");
    }
    bson::from_document(raw).map_err(|_| super::grants::incomplete())
}

/// Both subject fences and the bot CAS belong to the reconnect transaction.
/// Each cutover seals one row and inserts one row, independently of inbox size.
pub(crate) async fn cutover(
    db: &Database,
    tx: &mut ClientSession,
    old: &ChannelBot,
    next: &crate::models::channel_bot::MetaCredentialBinding,
) -> AppResult<()> {
    let previous = old
        .meta_binding
        .as_ref()
        .ok_or_else(super::grants::incomplete)?;
    if next.generation <= previous.generation || next.grant_id == previous.grant_id {
        return Err(super::grants::incomplete());
    }
    fence_reconnect(db, tx, old).await?;
    let scope = ensure_current(db, tx, old, false).await?;
    let changed = db
        .collection::<GrantScope>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":&scope.id,
            "status":"current","erasure_requested":{"$ne":true},"to_generation":null},
            doc! {"$set":{
            "status":"sealed","to_generation":previous.generation,"sealed_at":bson::DateTime::now(),
            "attribution_pending":true},"$inc":{"revision":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(super::grants::incomplete());
    }
    let mut successor = old.clone();
    successor.meta_binding = Some(next.clone());
    successor.app_id = Some(next.app_id.clone());
    successor.credential_source = "platform".into();
    ensure_current(db, tx, &successor, true).await?;
    Ok(())
}

pub(crate) async fn fence_current(
    db: &Database,
    tx: &mut ClientSession,
    bot: &ChannelBot,
) -> AppResult<GrantRef> {
    super::super::erasure_admission::fence_in_session(db, &bot.user_id, tx).await?;
    let binding = bot
        .meta_binding
        .as_ref()
        .ok_or_else(super::grants::incomplete)?;
    let scope = ensure_current(db, tx, bot, false).await?;
    if scope.status != "current" || scope.erasure_requested {
        return Err(super::grants::incomplete());
    }
    fence_subject(
        db,
        tx,
        &bson::to_document(&scope).map_err(|_| super::grants::incomplete())?,
        true,
    )
    .await?;
    let changed = db
        .collection::<GrantScope>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":scope.id,"status":"current",
            "erasure_requested":{"$ne":true}},
            doc! {"$inc":{"revision":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(super::grants::incomplete());
    }
    Ok(GrantRef {
        bot_id: bot.id.clone(),
        grant_id: binding.grant_id.clone(),
        generation: binding.generation,
    })
}

pub(crate) async fn has_history(db: &Database, bot: &str) -> AppResult<bool> {
    use futures::TryStreamExt;
    let rows: Vec<Document> = db
        .collection::<Document>(GRANT_SCOPES)
        .find(doc! {"bot_id":bot})
        .projection(doc! {"_id":1,"status":1})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    Ok(rows.len() > 1 || rows.iter().any(|row| row.get_str("status") == Ok("sealed")))
}

/// Scope proof has the same typed, exact grant semantics as inbox completion.
pub(crate) async fn mark_complete(
    db: &Database,
    tx: &mut ClientSession,
    bot: &str,
    grant: &str,
) -> AppResult<()> {
    let rows = db.collection::<Document>(GRANT_SCOPES);
    let Some(row) = rows
        .find_one(doc! {"bot_id":bot,"grant_id":grant})
        .session(&mut *tx)
        .await?
    else {
        let prior = db
            .collection::<Document>("channel_meta_erasure_fences")
            .find_one(doc! {"_id":bot})
            .session(&mut *tx)
            .await?
            .ok_or_else(super::grants::incomplete)?;
        return if super::grants::completed(&prior, grant)? {
            Ok(())
        } else {
            Err(super::grants::incomplete())
        };
    };
    let complete = super::grants::completed(&row, grant)?;
    let mut filter = doc! {"_id":row.get("_id").cloned(),"bot_id":bot,"grant_id":grant};
    let at = if complete {
        filter.insert(
            "local_data_erased_at",
            row.get("local_data_erased_at").cloned(),
        );
        filter.insert("completed_grant_id", grant);
        *row.get_datetime("local_data_erased_at")
            .map_err(|_| super::grants::incomplete())?
    } else {
        filter.insert("local_data_erased_at", doc! {"$exists":false});
        filter.insert("completed_grant_id", doc! {"$exists":false});
        if row.get_str("status") == Ok("completed") {
            // This predecessor shape was published atomically with exact
            // whole-inbox proof. An absent/malformed/foreign fence cannot
            // upgrade it or overwrite any contradictory scope evidence.
            let historical = *row
                .get_datetime("completed_at")
                .map_err(|_| super::grants::incomplete())?;
            filter.insert("completed_at", historical);
            let prior = db
                .collection::<Document>("channel_meta_erasure_fences")
                .find_one(doc! {"_id":bot})
                .session(&mut *tx)
                .await?
                .ok_or_else(super::grants::incomplete)?;
            if !super::grants::completed(&prior, grant)? {
                return Err(super::grants::incomplete());
            }
            *prior
                .get_datetime("local_data_erased_at")
                .map_err(|_| super::grants::incomplete())?
        } else {
            bson::DateTime::now()
        }
    };
    let changed=rows.update_one(filter,doc! {"$set":{"status":"completed","outstanding":false,"local_data_erased_at":at,"completed_grant_id":grant},
        "$unset":{"identity":"","platform":"","app_id":"","from_generation":"","to_generation":"","legacy_history":"",
            "binding_fingerprint":"","authorized_at":"","bot_created_at":"","created_at":"","sealed_at":"","erasure_cutoff":"",
            "attribution_pending":"","completed_at":"","erasure_requested":"","purge":""},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
    if changed.matched_count != 1 {
        return Err(super::grants::incomplete());
    }
    Ok(())
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<GrantScope>(GRANT_SCOPES)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"bot_id":1,"grant_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"bot_id":1,"from_generation":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"identity":1,"outstanding":1,"_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"platform":1,"app_id":1,"outstanding":1,"_id":1})
                .build(),
        ])
        .await?;
    Ok(())
}

/// Callback acceptance and every admitted grant write share this keyed fence.
/// The accepted erasure cutoff is independent from later deauthorization.
async fn fence_subject(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    fresh: bool,
) -> AppResult<()> {
    let Some(identity) = scope.get("identity") else {
        return Ok(());
    };
    if identity == &bson::Bson::Null {
        return Ok(());
    }
    let identity = identity.as_str().ok_or_else(super::grants::incomplete)?;
    let row=db.collection::<Document>("channel_meta_grant_authority").find_one_and_update(doc! {"identity":identity},
        doc! {"$inc":{"revision":1_i64},"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"identity":identity,
            "platform":scope.get("platform").cloned(),"app_id":scope.get("app_id").cloned(),"outstanding":true}})
        .upsert(true).return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?
        .ok_or_else(super::grants::incomplete)?;
    let revoked = if fresh {
        row.get_datetime("revoked_at")
            .ok()
            .map(|at| bson::DateTime::from_millis(at.timestamp_millis() + 1000))
    } else {
        None
    };
    let erased = row.get_datetime("erasure_cutoff").ok().copied();
    for cutoff in [revoked, erased].into_iter().flatten() {
        let authorized = scope
            .get_datetime("authorized_at")
            .map_err(|_| super::grants::incomplete())?;
        if authorized < &cutoff {
            return Err(super::grants::incomplete());
        }
    }
    Ok(())
}

pub(crate) fn reference(bot: &ChannelBot) -> AppResult<GrantRef> {
    let binding = bot
        .meta_binding
        .as_ref()
        .ok_or_else(super::grants::incomplete)?;
    Ok(GrantRef {
        bot_id: bot.id.clone(),
        grant_id: binding.grant_id.clone(),
        generation: binding.generation,
    })
}

/// Fresh work requires the current consent. An already issued attempt may
/// settle against a sealed scope until that exact scope is claimed for erasure.
pub(crate) async fn fence_reference(
    db: &Database,
    tx: &mut ClientSession,
    grant: &GrantRef,
    fresh: bool,
) -> AppResult<()> {
    let mut filter = doc! {"bot_id":&grant.bot_id,"grant_id":&grant.grant_id,
    "from_generation":{"$lte":grant.generation},"erasure_requested":{"$ne":true},
    "$or":[{"to_generation":null},{"to_generation":{"$gte":grant.generation}}]};
    filter.insert(
        "status",
        if fresh {
            bson::bson!("current")
        } else {
            bson::bson!({"$in":["current","sealed","retired"]})
        },
    );
    if fresh {
        let mut parent =
            doc! {"_id":&grant.bot_id,"is_active":true,"meta_binding.generation":grant.generation};
        parent.extend(super::grants::grant_filter(&grant.grant_id));
        let live = db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one_and_update(parent, doc! {"$inc":{"meta_observation_revision":1_i64}})
            .return_document(mongodb::options::ReturnDocument::After)
            .session(&mut *tx)
            .await?
            .ok_or_else(|| super::meta::unreachable("credential_authority_changed"))?;
        super::super::erasure_admission::fence_in_session(db, &live.user_id, tx).await?;
        ensure_current(db, tx, &live, false).await?;
    }
    let scope = db
        .collection::<Document>(GRANT_SCOPES)
        .find_one(filter.clone())
        .session(&mut *tx)
        .await?
        .ok_or_else(super::grants::incomplete)?;
    fence_subject(db, tx, &scope, fresh).await?;
    if db
        .collection::<Document>(GRANT_SCOPES)
        .update_one(filter, doc! {"$inc":{"revision":1_i64}})
        .session(&mut *tx)
        .await?
        .matched_count
        != 1
    {
        return Err(super::grants::incomplete());
    }
    Ok(())
}

/// One exact source reference; generation ranges are independently recorded at
/// consent cutover, never inferred from an expired source's timestamp.
pub(crate) async fn source_in_session(
    db: &Database,
    tx: &mut ClientSession,
    source: &str,
) -> AppResult<Option<GrantRef>> {
    let Some(row) = db
        .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {"_id":source})
        .session(&mut *tx)
        .await?
    else {
        return Ok(None);
    };
    source_document_in_session(db, tx, &row).await
}

/// Descendant writers borrow authority from the retained original assistant
/// message inside their own transaction. A source removed by erasure cannot
/// be used to recreate feedback or artifact payloads afterwards.
pub(crate) async fn fence_assistant_source(
    db: &Database,
    tx: &mut ClientSession,
    source: &Document,
) -> AppResult<()> {
    let proofs: Vec<GrantRef> = match source.get("meta_grants") {
        None | Some(bson::Bson::Null) => return Ok(()),
        Some(value) => bson::from_bson(value.clone()).map_err(|_| super::grants::incomplete())?,
    };
    for proof in &proofs {
        fence_reference(db, tx, proof, false).await?;
    }
    Ok(())
}

pub(crate) async fn fence_assistant_turn(
    db: &Database,
    tx: &mut ClientSession,
    owner: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<()> {
    let mut cursor = db
        .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
        // A long ordinary turn has no grant evidence to validate.
        .find(
            doc! {"user_id":owner,"conversation_id":conversation,"turn_id":turn,
            "meta_grants":{"$exists":true, "$ne":null}},
        )
        .projection(doc! {"_id":0,"meta_grants":1})
        .limit(101)
        .session(&mut *tx)
        .await?;
    let mut count = 0;
    while let Some(source) = cursor.next(&mut *tx).await.transpose()? {
        count += 1;
        if count > 100 {
            return Err(super::grants::incomplete());
        }
        fence_assistant_source(db, tx, &source).await?;
    }
    Ok(())
}

pub(crate) async fn source_document_in_session(
    db: &Database,
    tx: &mut ClientSession,
    row: &Document,
) -> AppResult<Option<GrantRef>> {
    let platform = row.get_str("platform").unwrap_or_default();
    if !super::meta::is_meta(platform) {
        return Ok(None);
    }
    if row.get_str("direction") != Ok("inbound") {
        return Err(super::grants::incomplete());
    }
    let bot = row
        .get_str("channel_bot_id")
        .map_err(|_| super::grants::incomplete())?;
    let raw = row
        .get_document("meta_authority")
        .map_err(|_| super::grants::incomplete())?;
    let authority: crate::models::channel_message::MetaMessageAuthority =
        bson::from_document(raw.clone()).map_err(|_| super::grants::incomplete())?;
    let chat = super::meta::conversation(platform, bot, &authority.asset_id, &authority.sender_id);
    if authority.asset_id.is_empty()
        || authority.sender_id.is_empty()
        || authority.binding.len() != 64
        || row.get_str("platform_conversation_id") != Ok(chat.as_str())
    {
        return Err(super::grants::incomplete());
    }
    let stamped: Option<GrantRef> = row
        .get("meta_grant")
        .map(|value| bson::from_bson(value.clone()).map_err(|_| super::grants::incomplete()))
        .transpose()?;
    if let Some(stamped) = &stamped
        && (stamped.bot_id != bot
            || stamped.generation != authority.generation
            || authority
                .grant_id
                .as_ref()
                .is_some_and(|grant| grant != &stamped.grant_id))
    {
        return Err(super::grants::incomplete());
    }
    let mut filter = doc! {"bot_id":bot,"from_generation":{"$lte":authority.generation},
    "$or":[{"to_generation":null},{"to_generation":{"$gte":authority.generation}}]};
    if let Some(grant) = authority
        .grant_id
        .as_deref()
        .or_else(|| stamped.as_ref().map(|grant| grant.grant_id.as_str()))
    {
        filter.insert("grant_id", grant);
    }
    let mut rows = db
        .collection::<GrantScope>(GRANT_SCOPES)
        .find(filter)
        .limit(2)
        .session(&mut *tx)
        .await?;
    let scope = rows
        .next(&mut *tx)
        .await
        .transpose()?
        .ok_or_else(super::grants::incomplete)?;
    if rows.next(&mut *tx).await.transpose()?.is_some() {
        return Err(super::grants::incomplete());
    }
    if authority.grant_id.is_none()
        && (scope.from_generation <= 0 || scope.binding_fingerprint != authority.binding)
    {
        // A lazily initialized zero lower bound alone cannot prove a prior
        // installation. Keep ambiguous historical provenance repairable.
        return Err(super::grants::incomplete());
    }
    let proven = GrantRef {
        bot_id: bot.into(),
        grant_id: scope.grant_id,
        generation: authority.generation,
    };
    if stamped.as_ref().is_some_and(|stamped| stamped != &proven) {
        return Err(super::grants::incomplete());
    }
    Ok(Some(proven))
}

pub(crate) async fn update_record(
    db: &Database,
    grant: Option<&GrantRef>,
    fresh: bool,
    collection: &str,
    filter: Document,
    update: impl Into<mongodb::options::UpdateModifications>,
) -> AppResult<mongodb::results::UpdateResult> {
    let update = update.into();
    if let Some(grant) = grant {
        super::super::transaction_service::run_db_only(
            db,
            &(db, grant, fresh, collection, &filter, &update),
            |tx, &(db, grant, fresh, collection, filter, update)| {
                Box::pin(async move {
                    fence_reference(db, tx, grant, fresh).await?;
                    if let Some(record) = db
                        .collection::<Document>(collection)
                        .find_one(filter.clone())
                        .projection(doc! {"user_id":1})
                        .session(&mut *tx)
                        .await?
                    {
                        let owner = record
                            .get_str("user_id")
                            .map_err(|_| super::grants::incomplete())?;
                        super::super::erasure_admission::fence_in_session(db, owner, tx).await?;
                    }
                    Ok(db
                        .collection::<Document>(collection)
                        .update_one(filter.clone(), update.clone())
                        .session(tx)
                        .await?)
                })
            },
        )
        .await
    } else {
        Ok(db
            .collection::<Document>(collection)
            .update_one(filter, update)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn completion_requires_exact_typed_proof_and_preserves_corruption() {
        let (state, _, _server, bot) =
            crate::services::channel_adapters::meta::tests::fixture("instagram").await;
        let grant = bot.meta_binding.as_ref().unwrap().grant_id.clone();
        let scope = crate::services::transaction_service::run_db_only(
            &state.db,
            &(&state.db, &bot),
            |tx, &(db, bot)| Box::pin(ensure_current(db, tx, bot, true)),
        )
        .await
        .unwrap();
        let rows = state.db.collection::<Document>(GRANT_SCOPES);
        for corrupt in [
            doc! {},
            doc! {"local_data_erased_at":"date","completed_grant_id":&grant},
            doc! {"local_data_erased_at":bson::DateTime::now(),"completed_grant_id":9_i32},
            doc! {"local_data_erased_at":bson::DateTime::now(),"completed_grant_id":"other-grant"},
        ] {
            rows.update_one(
                doc! {"_id":&scope.id},
                doc! {"$unset":{"local_data_erased_at":"","completed_grant_id":""}},
            )
            .await
            .unwrap();
            let mut fields = corrupt.clone();
            fields.insert("status", "completed");
            rows.update_one(doc! {"_id":&scope.id}, doc! {"$set":fields})
                .await
                .unwrap();
            let result = crate::services::transaction_service::run_db_only(
                &state.db,
                &(&state.db, bot.id.as_str(), grant.as_str()),
                |tx, &(db, bot, grant)| Box::pin(mark_complete(db, tx, bot, grant)),
            )
            .await;
            assert!(result.is_err());
            let preserved = rows
                .find_one(doc! {"_id":&scope.id})
                .await
                .unwrap()
                .unwrap();
            for (key, value) in corrupt {
                assert_eq!(preserved.get(&key), Some(&value));
            }
        }
        rows.update_one(doc! {"_id":&scope.id},doc! {"$set":{"status":"sealed"},"$unset":{"local_data_erased_at":"","completed_grant_id":""}}).await.unwrap();
        crate::services::transaction_service::run_db_only(
            &state.db,
            &(&state.db, bot.id.as_str(), grant.as_str()),
            |tx, &(db, bot, grant)| Box::pin(mark_complete(db, tx, bot, grant)),
        )
        .await
        .unwrap();
        let completed = rows
            .find_one(doc! {"_id":&scope.id})
            .await
            .unwrap()
            .unwrap();
        assert!(super::super::grants::completed(&completed, &grant).unwrap());
        assert!(
            crate::services::transaction_service::run_db_only(
                &state.db,
                &(&state.db, &bot),
                |tx, &(db, bot)| Box::pin(ensure_current(db, tx, bot, false))
            )
            .await
            .is_err()
        );
        assert!(
            crate::services::transaction_service::run_db_only(
                &state.db,
                &(&state.db, &bot),
                |tx, &(db, bot)| Box::pin(fence_current(db, tx, bot))
            )
            .await
            .is_err()
        );
        let still_terminal = rows
            .find_one(doc! {"_id":&scope.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!still_terminal.contains_key("identity"));

        for field in [
            "identity",
            "app_id",
            "from_generation",
            "to_generation",
            "binding_fingerprint",
            "authorized_at",
        ] {
            assert!(!completed.contains_key(field));
        }
        assert!(
            crate::services::transaction_service::run_db_only(
                &state.db,
                &(&state.db, bot.id.as_str()),
                |tx, &(db, bot)| { Box::pin(mark_complete(db, tx, bot, "absent-grant")) }
            )
            .await
            .is_err()
        );
    }
}

/// The exact event keeps provenance after its inbound source reaches its TTL.
pub(crate) async fn event_reference(db: &Database, event: &str) -> AppResult<Option<GrantRef>> {
    let row = db
        .collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .find_one(doc! {"_id":event})
        .projection(doc! {"meta_grant":1})
        .await?
        .ok_or_else(super::grants::incomplete)?;
    row.get("meta_grant")
        .map(|value| bson::from_bson(value.clone()).map_err(|_| super::grants::incomplete()))
        .transpose()
}

/// Keep the chat/settings identity while rotating only its internal execution
/// session when consent changes. Explicit per-chat specialist/settings survive.
pub(crate) async fn record_thread(
    db: &Database,
    bot: &str,
    source: &str,
    partition: Document,
    update: Vec<Document>,
) -> AppResult<crate::models::nyxbot_channel::NyxbotThread> {
    super::super::transaction_service::run_db_only(db,&(db,bot,source,&partition,&update),|tx,&(db,bot,source,partition,update)| {
        Box::pin(async move {
            let grant=source_in_session(db,tx,source).await?.ok_or_else(super::grants::incomplete)?;
            if grant.bot_id!=bot {return Err(super::grants::incomplete());}
            fence_reference(db,tx,&grant,true).await?;
            let mut update=update.clone();
            update.push(doc! {"$set":{
                "conversation_id":{"$cond":[{"$ne":[{"$ifNull":["$meta_grant.grant_id",null]},&grant.grant_id]},null,"$conversation_id"]},
                "owner_seen":{"$cond":[{"$ne":[{"$ifNull":["$meta_grant.grant_id",null]},&grant.grant_id]},false,"$owner_seen"]},
                "meta_grant":{"$literal":bson::to_bson(&grant).map_err(|_|super::grants::incomplete())?},
                "channel_bot_id":{"$literal":bot},
            }});
            db.collection::<crate::models::nyxbot_channel::NyxbotThread>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
                .find_one_and_update(partition.clone(),update).upsert(true).return_document(mongodb::options::ReturnDocument::After)
                .session(tx).await?.ok_or_else(super::grants::incomplete)
        })
    }).await
}

/// Bind a conversation to the thread observed for this delivery. An owner
/// chat selects its home. A guest keeps the existing consent's reservation.
pub(crate) async fn bind_thread_conversation(
    db: &Database,
    grant: &GrantRef,
    thread: &crate::models::nyxbot_channel::NyxbotThread,
    home: Option<&str>,
) -> AppResult<String> {
    let mut filter = doc! {
        "_id": &thread.id, "channel_id": &thread.channel_id, "partition": &thread.partition,
        "meta_grant.bot_id": &grant.bot_id, "meta_grant.grant_id": &grant.grant_id,
        "meta_grant.generation": grant.generation,
    };
    let observed = doc! {"last_message_id": &thread.last_message_id, "last_relay_message_id": &thread.last_relay_message_id};
    // A newer message from this grant may have already bound the same home.
    // Guest reservation only adopts an existing nonempty ID in that case.
    let already_bound = match home {
        Some(id) => doc! {"conversation_id": id},
        None => doc! {"conversation_id":{"$type":"string","$ne":""}},
    };
    filter.insert("$or", vec![observed, already_bound]);
    let conversation = match home {
        Some(id) => doc! { "$literal": id },
        None => {
            let reserved = format!("nyxa-{}", uuid::Uuid::new_v4().simple());
            doc! { "$ifNull": ["$conversation_id", {"$literal": reserved}] }
        }
    };
    let update = doc! {"$set": {"conversation_id": conversation}};
    let context = (db.clone(), grant.clone(), filter, update);
    super::super::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, grant, filter, update)| {
            Box::pin(async move {
                fence_reference(db, tx, grant, true).await?;
                let row = db
                    .collection::<Document>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
                    .find_one_and_update(filter.clone(), vec![update.clone()])
                    .projection(doc! {"conversation_id": 1})
                    .return_document(mongodb::options::ReturnDocument::After)
                    .session(tx)
                    .await?
                    .ok_or_else(super::grants::incomplete)?;
                row.get_str("conversation_id")
                    .map(str::to_owned)
                    .map_err(|_| super::grants::incomplete())
            })
        },
    )
    .await
}

pub(crate) async fn observe_thread(
    db: &Database,
    bot: &str,
    thread: &crate::models::nyxbot_channel::NyxbotThread,
    fields: Document,
) -> AppResult<()> {
    super::super::transaction_service::run_db_only(db, &(db, bot, thread, &fields), |tx, &(db, bot, thread, fields)| {
        Box::pin(async move {
            let source = thread.last_relay_message_id.as_deref().or(thread.last_message_id.as_deref())
                .ok_or_else(super::grants::incomplete)?;
            let grant = source_in_session(db,tx,source).await?.filter(|grant|grant.bot_id==bot)
                .ok_or_else(super::grants::incomplete)?;
            fence_reference(db,tx,&grant,true).await?;
            let changed = db.collection::<Document>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
                .update_one(doc! {"_id":&thread.id,"channel_id":&thread.channel_id,
                    "last_message_id":&thread.last_message_id,"last_relay_message_id":&thread.last_relay_message_id,
                    "meta_grant.bot_id":bot,"meta_grant.grant_id":&grant.grant_id}, doc! {"$set":fields.clone()})
                .session(tx).await?;
            if changed.matched_count!=1 {return Err(super::grants::incomplete());}
            Ok(())
        })
    }).await
}

/// Every authorized whole-inbox purge closes its exact grant writer fence
/// before its first inventory read, including recovered manual Delete retries.
pub(crate) async fn claim_purge(db: &Database, bot: &str, grant: &str) -> AppResult<()> {
    super::super::transaction_service::run_db_only(db,&(db,bot,grant),|tx,&(db,bot,grant)| {
        Box::pin(async move {
            if let Some(scope)=db.collection::<Document>(GRANT_SCOPES).find_one(doc! {"bot_id":bot,"grant_id":grant}).session(&mut *tx).await?
                && scope.get_str("status")==Ok("completed") {
                mark_complete(db,tx,bot,grant).await?;
                return Ok(());
            }
            if let Some(parent)=db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME).find_one(doc! {"_id":bot}).session(&mut *tx).await? {
                if parent.meta_binding.as_ref().is_none_or(|binding|binding.grant_id!=grant) {return Err(super::grants::incomplete());}
                ensure_current(db,tx,&parent,false).await?;
                let mut exact=doc! {"_id":bot,"meta_binding.generation":parent.meta_binding.as_ref().map(|binding|binding.generation)};
                exact.extend(super::grants::grant_filter(grant));
                if db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME).update_one(exact,doc! {"$inc":{"meta_observation_revision":1_i64}}).session(&mut *tx).await?.matched_count!=1 {return Err(super::grants::incomplete());}
            }
            // A retired draft without a scope never admits scoped writes.
            if let Some(scope)=db.collection::<Document>(GRANT_SCOPES).find_one(doc! {"bot_id":bot,"grant_id":grant}).session(&mut *tx).await? {
                if super::grants::completed(&scope,grant)? {return Ok(());}
                let changed=db.collection::<Document>(GRANT_SCOPES).update_one(doc! {"_id":scope.get("_id").cloned(),"bot_id":bot,"grant_id":grant,"status":{"$ne":"completed"}},
                    doc! {"$set":{"erasure_requested":true},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
                if changed.matched_count!=1 {return Err(super::grants::incomplete());}
            }
            Ok(())
        })
    }).await
}

/// A still-present matching parent is authoritative for current chronology.
/// Preserve stored evidence when it is incomplete, but do not use a stale
/// scope timestamp to claim deletion while the live parent needs repair.
pub(crate) async fn synchronize_chronology(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
) -> AppResult<Document> {
    if scope.get_str("status") == Ok("completed") {
        return Ok(scope.clone());
    }
    let bot = scope
        .get_str("bot_id")
        .map_err(|_| super::grants::incomplete())?;
    let grant = scope
        .get_str("grant_id")
        .map_err(|_| super::grants::incomplete())?;
    let mut filter = doc! {"_id":bot};
    filter.extend(super::grants::grant_filter(grant));
    let Some(parent) = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one_and_update(filter, doc! {"$inc":{"meta_chronology_revision":1_i64}})
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *tx)
        .await?
    else {
        return Ok(scope.clone());
    };
    let binding = parent
        .get_document("meta_binding")
        .map_err(|_| super::grants::incomplete())?;
    let mut result = scope.clone();
    if let Ok(at) = binding.get_datetime("authorized_at") {
        let changed = db
            .collection::<Document>(GRANT_SCOPES)
            .update_one(
                doc! {"_id":scope.get("_id").cloned(),"bot_id":bot,"grant_id":grant},
                doc! {"$set":{"authorized_at":at},"$inc":{"revision":1_i64}},
            )
            .session(&mut *tx)
            .await?;
        if changed.matched_count != 1 {
            return Err(super::grants::incomplete());
        }
        result.insert("authorized_at", *at);
    } else {
        result.insert("authorized_at", bson::Bson::Null);
    }
    Ok(result)
}

pub(crate) async fn check_subject_cutoff(
    db: &Database,
    platform: &str,
    app: &str,
    subject: &str,
    authorized: chrono::DateTime<chrono::Utc>,
    fresh: bool,
) -> AppResult<()> {
    if subject.is_empty() {
        return Ok(());
    }
    let identity = super::provider::grant_identity(platform, app, subject)?;
    if let Some(row) = db
        .collection::<Document>("channel_meta_grant_authority")
        .find_one(doc! {"identity":identity})
        .await?
    {
        let at = bson::DateTime::from_chrono(authorized);
        let revoked = if fresh {
            row.get_datetime("revoked_at")
                .ok()
                .map(|at| bson::DateTime::from_millis(at.timestamp_millis().saturating_add(1000)))
        } else {
            None
        };
        for cutoff in [revoked, row.get_datetime("erasure_cutoff").ok().copied()]
            .into_iter()
            .flatten()
        {
            if at < cutoff {
                return Err(super::meta::unreachable("reconnect_required"));
            }
        }
    }
    Ok(())
}

pub(crate) async fn require_current_operation(db: &Database, bot: &ChannelBot) -> AppResult<()> {
    let grant = reference(bot)?;
    super::super::transaction_service::run_db_only(db, &(db, &grant), |tx, &(db, grant)| {
        Box::pin(fence_reference(db, tx, grant, true))
    })
    .await
}

/// Fresh consent may recover a deauthorization. An independently accepted
/// deletion of the installation must finish before its public ID can be reused.
pub(crate) async fn fence_reconnect(
    db: &Database,
    tx: &mut ClientSession,
    bot: &ChannelBot,
) -> AppResult<()> {
    let scope = ensure_current(db, tx, bot, false).await?;
    if scope.erasure_requested {
        return Err(super::grants::incomplete());
    }
    fence_subject(
        db,
        tx,
        &bson::to_document(&scope).map_err(|_| super::grants::incomplete())?,
        false,
    )
    .await
}
