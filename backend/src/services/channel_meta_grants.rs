//! Exact Meta grant obligations shared by account retirement and provider jobs.
use crate::errors::{AppError, AppResult};
use bson::{Document, doc};

pub(crate) fn incomplete() -> AppError {
    AppError::Conflict("Meta grant evidence is incomplete; restore verified grant metadata before completing cleanup".into())
}

/// Presence never certifies completion. A malformed proof is preserved for
/// repair; it cannot remove attribution or acknowledge a different grant.
pub(crate) fn completed(row: &Document, grant: &str) -> AppResult<bool> {
    match (
        row.get("local_data_erased_at"),
        row.get("completed_grant_id"),
    ) {
        (None, None) => Ok(false),
        (Some(bson::Bson::DateTime(_)), Some(bson::Bson::String(id))) if id == grant => Ok(true),
        _ => Err(incomplete()),
    }
}

pub(crate) async fn clear_completed_retired(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    row: &Document,
) -> AppResult<bool> {
    let Some(retired) = row.get("retired_grant") else {
        return Ok(false);
    };
    let grant = retired
        .as_document()
        .and_then(|grant| grant.get_str("grant_id").ok())
        .ok_or_else(incomplete)?;
    if !completed(row, grant)? {
        return Ok(false);
    }
    let changed = db.collection::<Document>("channel_meta_erasure_fences")
        .update_one(doc! {"_id":row.get("_id").cloned(),"completed_grant_id":grant,
            "local_data_erased_at":row.get("local_data_erased_at").cloned(),"retired_grant":retired.clone()},
            doc! {"$unset":{"retired_grant":""}}).session(&mut *tx).await?;
    if changed.matched_count != 1 {
        return Err(incomplete());
    }
    Ok(true)
}

/// Stop exactly the owner-visible installation and capture manual purge
/// authority in that same transaction. OAuth cutover cannot inherit a Delete.
pub(crate) async fn stop_for_delete(
    db: &mongodb::Database,
    actor: &str,
    observed: &crate::models::channel_bot::ChannelBot,
) -> AppResult<crate::models::channel_bot::ChannelBot> {
    #[cfg(test)]
    super::super::durable_wake_tests::after("meta_delete_before_stop").await;
    super::super::transaction_service::run_db_only(db,&(db,actor,observed),|tx,&(db,actor,observed)|{
        Box::pin(async move {
            let binding=observed.meta_binding.as_ref().ok_or_else(incomplete)?;
            if db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find_one(doc! {"_id":&observed.id}).session(&mut *tx).await?.is_none() {
                let proof=db.collection::<Document>("channel_meta_erasure_fences").find_one(doc! {"_id":&observed.id}).session(&mut *tx).await?.ok_or_else(incomplete)?;
                if completed(&proof,&binding.grant_id)? {return Ok(observed.clone());}
                return Err(incomplete());
            }
            super::MetaService::fence_installation_authority(db,tx,actor,&observed.user_id,&observed.platform,false).await?;
            let binding = observed.meta_binding.as_ref().ok_or_else(incomplete)?;
            let generation = binding.generation + i64::from(observed.is_active);
            let mut fields = doc! {"is_active":false,"status":"inactive","webhook_registered":false,
                "meta_binding.generation":generation,"meta_binding.grant_id":&binding.grant_id,"updated_at":bson::DateTime::now()};
            if binding.subscription != "removed" { fields.insert("meta_binding.subscription","removal_pending"); }
            fields.insert("meta_binding.next_reconcile_at",bson::DateTime::now());
            if observed.credential_source != "platform" {
                fields.insert("meta_manual_purge",doc! {"actor_id":actor,"owner_id":&observed.user_id,
                    "credential_source":&observed.credential_source,"grant_id":&binding.grant_id,
                    "generation":generation,"requested_at":bson::DateTime::now()});
            }
            let mut filter = doc! {"_id":&observed.id,"user_id":&observed.user_id,
                "platform":&observed.platform,"platform_bot_id":&observed.platform_bot_id,
                "credential_source":&observed.credential_source,"ownership_version":observed.ownership_version,
                "is_active":observed.is_active,"meta_binding.generation":binding.generation};
            filter.extend(grant_filter(&binding.grant_id));
            let stopped=db.collection::<crate::models::channel_bot::ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
                .find_one_and_update(filter,doc! {"$set":fields})
                .return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?
                .ok_or_else(||AppError::Conflict("Inbox grant changed; refresh before deleting".into()))?;
            let scope=super::scopes::ensure_current(db,tx,&stopped,false).await?;
            if observed.credential_source!="platform" {
                let closed=db.collection::<Document>(crate::models::channel_meta::GRANT_SCOPES).update_one(
                    doc! {"_id":&scope.id,"bot_id":&stopped.id,"grant_id":&binding.grant_id,"status":{"$ne":"completed"}},
                    doc! {"$set":{"erasure_requested":true},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
                if closed.matched_count!=1 {return Err(incomplete());}
            }
            Ok(stopped)
        })
    }).await
}

pub(crate) async fn manual_purge_authorized(
    db: &mongodb::Database,
    bot: &crate::models::channel_bot::ChannelBot,
) -> AppResult<bool> {
    let binding = bot.meta_binding.as_ref().ok_or_else(incomplete)?;
    if bot.is_active || bot.credential_source == "platform" {
        return Ok(false);
    }
    let authorized = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find_one(doc! {
        "_id":&bot.id,"user_id":&bot.user_id,"credential_source":&bot.credential_source,
        "is_active":false,"meta_binding.grant_id":&binding.grant_id,"meta_binding.generation":binding.generation,
        "meta_manual_purge.owner_id":&bot.user_id,"meta_manual_purge.credential_source":&bot.credential_source,
        "meta_manual_purge.grant_id":&binding.grant_id,"meta_manual_purge.generation":binding.generation,
        "meta_manual_purge.actor_id":{"$type":"string"},"meta_manual_purge.requested_at":{"$type":"date"},
    }).projection(doc! {"_id":1}).await?.is_some();
    if authorized {
        return Ok(true);
    }
    // A completed concurrent sweep may remove the parent after the caller's
    // first reread. Only exact typed proof for this captured grant settles it.
    if db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&bot.id})
        .await?
        .is_none()
    {
        return completion_for(db, &bot.id, &binding.grant_id).await;
    }
    recover_manual_delete(db, bot).await
}

/// Derived app-drain state. Malformed retained dates require repair; a signed
/// identity with no installation history is already drained once revoked.
pub(crate) fn authority_state() -> Vec<Document> {
    vec![doc! {"$set":{"outstanding":{"$cond":[
        {"$eq":[{"$type":"$revoked_at"},"date"]},
        {"$switch":{"branches":[
            {"case":{"$eq":[{"$type":"$last_authorized_at"},"missing"]},"then":false},
            {"case":{"$eq":[{"$type":"$last_authorized_at"},"date"]},
                "then":{"$gt":["$last_authorized_at","$revoked_at"]}}
        ],"default":true}},true
    ]}}}]
}

/// Bounded conversion of this feature's earlier derived authority rows. The
/// identity and dates stay on their original record; newer writes win the CAS.
pub(crate) async fn reconcile_authority_state(db: &mongodb::Database) -> AppResult<()> {
    use futures::TryStreamExt;
    let rows = db.collection::<Document>("channel_meta_grant_authority");
    let batch: Vec<_> = rows
        .find(doc! {"outstanding":{"$exists":false}})
        .projection(doc! {"_id":1})
        .sort(doc! {"_id":1})
        .limit(100)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    if !batch.is_empty() {
        let ids: Vec<_> = batch
            .into_iter()
            .filter_map(|row| row.get("_id").cloned())
            .collect();
        rows.update_many(
            doc! {"_id":{"$in":ids},"outstanding":{"$exists":false}},
            authority_state(),
        )
        .await?;
    }
    Ok(())
}

/// Recover only a verified earlier Delete on this unchanged installation.
/// Account retirement and an unrelated management audit convey no purge scope.
async fn recover_manual_delete(
    db: &mongodb::Database,
    bot: &crate::models::channel_bot::ChannelBot,
) -> AppResult<bool> {
    super::super::transaction_service::run_db_only(db,&(db,bot),|tx,&(db,bot)|{
        Box::pin(async move {
            let binding = bot.meta_binding.as_ref().ok_or_else(incomplete)?;
            let source = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .find_one(doc! {"_id":&bot.id}).session(&mut *tx).await?.ok_or_else(incomplete)?;
            if source.contains_key("meta_manual_purge") { return Err(incomplete()); }
            let witness = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .find_one(doc! {"event_type":"channel_bot_deleted","event_data.bot_id":&bot.id,
                    "event_data.owner_user_id":&bot.user_id,"event_data.platform":&bot.platform,
                    "event_data.managed_webhook_cleanup":{"$in":["removed","failed"]},
                    "created_at":{"$gte":bson::DateTime::from_chrono(bot.updated_at)}})
                .sort(doc! {"created_at":-1}).max_time(std::time::Duration::from_secs(5))
                .session(&mut *tx).await?;
            let Some(witness) = witness else { return Ok(false); };
            let actor = witness.get_str("user_id").map_err(|_|incomplete())?;
            let changed = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .update_one(doc! {"_id":&bot.id,"user_id":&bot.user_id,"credential_source":&bot.credential_source,
                    "is_active":false,"meta_binding.grant_id":&binding.grant_id,"meta_binding.generation":binding.generation,
                    "updated_at":bson::DateTime::from_chrono(bot.updated_at),"meta_manual_purge":{"$exists":false}},
                    doc! {"$set":{"meta_manual_purge":{"actor_id":actor,"owner_id":&bot.user_id,
                        "credential_source":&bot.credential_source,"grant_id":&binding.grant_id,"generation":binding.generation,
                        "requested_at":witness.get("created_at").cloned(),"audit_id":witness.get("_id").cloned()}}})
                .session(&mut *tx).await?;
            if changed.matched_count != 1 { return Err(incomplete()); }
            Ok(true)
        })
    }).await
}

/// The model's empty legacy grant accepts absence, never a present malformed
/// value. Callers retain owner, asset, generation and logical credential fences.
pub(crate) fn grant_filter(grant: &str) -> Document {
    if grant.is_empty() {
        doc! {"$or":[{"meta_binding.grant_id":""},{"meta_binding.grant_id":{"$exists":false}}]}
    } else {
        doc! {"meta_binding.grant_id":grant}
    }
}

pub(crate) async fn completion_for(
    db: &mongodb::Database,
    bot: &str,
    grant: &str,
) -> AppResult<bool> {
    match db
        .collection::<Document>("channel_meta_erasure_fences")
        .find_one(doc! {"_id":bot})
        .await?
    {
        Some(row) => completed(&row, grant),
        None => Ok(false),
    }
}

/// Remove only proved completed context in bounded independent commits. The
/// credential transaction still refuses every remaining app obligation.
pub(crate) async fn reconcile_retired_app(
    db: &mongodb::Database,
    platform: &str,
    app: &str,
) -> AppResult<()> {
    for _ in 0..100 {
        let Some(row) = db.collection::<Document>("channel_meta_erasure_fences")
            .find_one(doc! {"retired_grant.platform":platform,"retired_grant.app_id":{"$in":[app,bson::Bson::Null]},
                "local_data_erased_at":{"$type":"date"},"completed_grant_id":{"$type":"string"}})
            .max_time(std::time::Duration::from_secs(5)).await? else { return Ok(()); };
        let changed =
            super::super::transaction_service::run_db_only(db, &(db, &row), |tx, &(db, row)| {
                Box::pin(clear_completed_retired(db, tx, row))
            })
            .await?;
        if !changed {
            return Ok(());
        }
    }
    Ok(())
}
