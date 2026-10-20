//! Channel cleanup precedes credential erasure. Bot rows retain the encrypted
//! capability until the existing adapter confirms cleanup or shared retention.
use super::*;

pub(super) async fn pass(state: &crate::AppState, job: &PersonalErasureJob) -> AppResult<bool> {
    let db = super::super::transaction_service::durable_database(&state.db);
    let Some(bot) = db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find_one(doc! {"user_id":&job.user_id,"personal_erasure_cleanup_complete":{"$ne":true}})
        .sort(doc! {"_id":1})
        .await?
    else {
        return Ok(true);
    };
    let adapter = super::super::channel_adapters::resolve_adapter(
        &bot.platform,
        &state.token_exchange_cache,
    )?;
    super::super::channel_bot_service::erase_personal_bot(
        &db,
        &state.config,
        &state.http_client,
        &state.encryption_keys,
        adapter.as_ref(),
        &job.user_id,
        &bot.id,
    )
    .await?;
    Ok(false)
}

/// Unknown registered connection webhooks require owner-authorized repair while
/// the account and connection credentials are still usable, before acceptance.
pub(super) async fn guard_in_session(
    db: &Database,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    use futures::TryStreamExt;
    let cache = std::sync::Arc::new(
        super::super::provider_token_exchange_service::TokenExchangeCache::new(),
    );
    let mut cursor=db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .aggregate([doc! {"$match":{"user_id":owner,"credential_source":"connection","webhook_registered":true}},doc! {"$group":{"_id":"$platform"}}])
        .session(&mut *tx).await?;
    while let Some(row) = cursor.stream(&mut *tx).try_next().await? {
        let platform = row.get_str("_id").map_err(|_| {
            AppError::Conflict("Repair the channel platform before deleting this account".into())
        })?;
        let adapter = super::super::channel_adapters::resolve_adapter(platform, &cache)?;
        if !matches!(
            adapter.ingestion(),
            super::super::channel_platform::Ingestion::Poll { .. }
        ) && !super::super::channel_connection_webhook_service::supports(adapter.as_ref())
        {
            return Err(AppError::Conflict("Remove or repair the registered connection webhook before deleting this account; provider credentials remain available for that operation".into()));
        }
    }
    Ok(())
}
