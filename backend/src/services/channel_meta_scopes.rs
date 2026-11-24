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
    rows.find_one(doc! {"bot_id":&bot.id,"grant_id":&binding.grant_id})
        .session(&mut *tx)
        .await?
        .ok_or_else(super::grants::incomplete)
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
    if !previous.grant_subject_id.is_empty() {
        let identity = super::provider::grant_identity(
            &old.platform,
            &previous.app_id,
            &previous.grant_subject_id,
        )?;
        db.collection::<Document>("channel_meta_grant_authority").update_one(doc! {"identity":&identity},doc! {
            "$inc":{"revision":1_i64},"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),
                "identity":identity,"platform":&old.platform,"app_id":&previous.app_id,"outstanding":true}})
            .upsert(true).session(&mut *tx).await?;
    }
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
        // Older exact whole-inbox completion can predate scope records. It
        // cannot certify a different/missing scope or overwrite corrupt proof.
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
    if super::grants::completed(&row, grant)? {
        return Ok(());
    }
    if row.get_str("status") == Ok("completed") {
        return Err(super::grants::incomplete());
    }
    let changed=rows.update_one(doc! {"_id":row.get("_id").cloned(),"bot_id":bot,"grant_id":grant,
        "local_data_erased_at":{"$exists":false},"completed_grant_id":{"$exists":false}},
        doc! {"$set":{"status":"completed","outstanding":false,"local_data_erased_at":bson::DateTime::now(),"completed_grant_id":grant},
            "$unset":{"identity":"","platform":"","app_id":"","from_generation":"","to_generation":"",
                "legacy_history":"","binding_fingerprint":"","authorized_at":"","bot_created_at":"","created_at":"",
                "sealed_at":"","erasure_cutoff":"","attribution_pending":""},
            "$inc":{"revision":1_i64}}).session(&mut *tx).await?;
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
    if !super::meta::is_meta(row.get_str("platform").unwrap_or_default()) {
        return Ok(None);
    }
    if let Some(value) = row.get("meta_grant") {
        return bson::from_bson(value.clone())
            .map(Some)
            .map_err(|_| super::grants::incomplete());
    }
    let bot = row
        .get_str("channel_bot_id")
        .map_err(|_| super::grants::incomplete())?;
    let authority = row
        .get_document("meta_authority")
        .map_err(|_| super::grants::incomplete())?;
    let generation = authority
        .get_i64("generation")
        .map_err(|_| super::grants::incomplete())?;
    let mut filter = doc! {"bot_id":bot,"from_generation":{"$lte":generation},
    "$or":[{"to_generation":null},{"to_generation":{"$gte":generation}}]};
    if let Some(value) = authority.get("grant_id") {
        let grant = value.as_str().ok_or_else(super::grants::incomplete)?;
        filter.insert("grant_id", grant);
    }
    let mut rows = db
        .collection::<GrantScope>(GRANT_SCOPES)
        .find(filter)
        .limit(2)
        .session(&mut *tx)
        .await?;
    let first = rows
        .next(&mut *tx)
        .await
        .transpose()?
        .ok_or_else(super::grants::incomplete)?;
    if rows.next(&mut *tx).await.transpose()?.is_some() {
        return Err(super::grants::incomplete());
    }
    Ok(Some(GrantRef {
        bot_id: bot.into(),
        grant_id: first.grant_id,
        generation,
    }))
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
