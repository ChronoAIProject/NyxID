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
        from_generation: if fresh { binding.generation } else { 0 },
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
        .projection(doc! {"_id":1})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    Ok(rows.len() > 1)
}

pub(crate) async fn mark_complete(
    db: &Database,
    tx: &mut ClientSession,
    bot: &str,
    grant: &str,
) -> AppResult<()> {
    db.collection::<GrantScope>(GRANT_SCOPES)
        .update_one(
            doc! {"bot_id":bot,"grant_id":grant},
            doc! {
            "$set":{"status":"completed","outstanding":false,"completed_at":bson::DateTime::now()}},
        )
        .session(&mut *tx)
        .await?;
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
                .keys(doc! {"identity":1,"outstanding":1,"authorized_at":1,"_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"platform":1,"app_id":1,"outstanding":1,"_id":1})
                .build(),
        ])
        .await?;
    Ok(())
}
