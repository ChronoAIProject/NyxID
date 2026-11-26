//! Channel cleanup precedes credential erasure. Bot rows retain the encrypted
//! capability until the existing adapter confirms cleanup or shared retention.
use super::*;

/// Provider-requested erasure is scoped to one stopped inbox. Reuse normal
/// connection retirement and conversation deletion so credentials, guest lanes,
/// uploads and pending work follow their existing erasure paths.
pub(crate) async fn erase_provider_inbox(
    state: &crate::AppState,
    bot_id: &str,
    grant_id: &str,
) -> AppResult<()> {
    use futures::TryStreamExt;
    let retired = state
        .db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":bot_id})
        .await?;
    if super::super::channel_meta_service::scope_erasure::required(&state.db, bot_id, grant_id)
        .await?
    {
        if super::super::channel_meta_service::grants::completion_for(&state.db, bot_id, grant_id)
            .await?
        {
            return Ok(());
        }
        let captured = retired
            .as_ref()
            .filter(|bot| {
                bot.meta_binding
                    .as_ref()
                    .is_some_and(|binding| binding.grant_id == grant_id)
            })
            .ok_or_else(super::super::channel_meta_service::grants::incomplete)?;
        if super::super::channel_meta_service::grants::manual_purge_authorized(&state.db, captured)
            .await?
        {
            if super::super::channel_meta_service::scope_erasure::manual_pass(state, captured)
                .await?
            {
                return Ok(());
            }
            return Err(AppError::Conflict("This inbox grant is still being erased; retry Delete after its cleanup worker finishes".into()));
        }
        return Err(super::super::channel_meta_service::grants::incomplete());
    }
    let mut retired_context = None;
    if retired.is_none() {
        let proof = state
            .db
            .collection::<Document>("channel_meta_erasure_fences")
            .find_one(doc! {"_id":bot_id})
            .await?;
        if let Some(proof) = &proof {
            if super::super::channel_meta_service::grants::completed(proof, grant_id)? {
                let args = (state.db.clone(), proof.clone());
                super::super::transaction_service::run_db_only(
                    &state.db,
                    &args,
                    |tx, (db, proof)| {
                        Box::pin(
                            super::super::channel_meta_service::grants::clear_completed_retired(
                                db, tx, proof,
                            ),
                        )
                    },
                )
                .await?;
                return Ok(());
            }
            if let Ok(context) = proof.get_document("retired_grant") {
                let context: crate::models::channel_meta::RetiredGrant =
                    bson::from_document(context.clone()).map_err(|_| {
                        AppError::Conflict("Retired inbox attribution is incomplete".into())
                    })?;
                if context.grant_id == grant_id {
                    if context.authorized_at.is_none() {
                        return Err(AppError::Conflict("Retired grant chronology is incomplete; restore verified attribution before completing cleanup".into()));
                    }
                    retired_context = Some(context);
                }
            }
        }
        if retired_context.is_none() {
            return Err(AppError::Conflict("Retired inbox attribution is unavailable; restore verified erasure metadata before completing cleanup".into()));
        }
    }
    if retired.as_ref().is_some_and(|bot| {
        bot.is_active
            || bot.meta_binding.as_ref().is_none_or(|binding| {
                binding.grant_id != grant_id || binding.subscription != "removed"
            })
    }) {
        return Err(AppError::Conflict(
            "Finish cleanup of the captured inbox grant before erasure".into(),
        ));
    }
    super::super::channel_meta_service::scopes::claim_purge(&state.db, bot_id, grant_id).await?;
    #[cfg(test)]
    super::super::durable_wake_tests::after("meta_purge_scope_claimed").await;
    // Install the erasure fence before deleting payloads. Late provider
    // responses check this durable marker before persisting outbound metadata,
    // so a send that was already in flight cannot repopulate an erased inbox.
    state
        .db
        .collection::<Document>("channel_meta_erasure_fences")
        .update_one(
            doc! {"_id": bot_id},
            doc! {"$set": {"requested_at": bson::DateTime::now()}},
        )
        .upsert(true)
        .await?;
    state
        .db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": bot_id},
            doc! {"$inc": {"meta_observation_revision": 1_i64}},
        )
        .await?;
    if let Some(bot) = &retired {
        super::super::channel_meta_journal_erasure::erase_legacy(
            &state.db,
            bot,
            &state.token_exchange_cache,
        )
        .await?;
    } else if let Some(context) = &retired_context {
        super::super::channel_meta_journal_erasure::erase_retired_legacy(
            &state.db,
            bot_id,
            context,
            &state.token_exchange_cache,
        )
        .await?;
    }
    let channels: Vec<crate::models::nyxbot_channel::NyxbotChannel> = state
        .db
        .collection("nyxbot_channels")
        .find(doc! {"channel_bot_id":bot_id})
        .await?
        .try_collect()
        .await?;
    for channel in channels {
        if matches!(channel.status.as_str(), "active" | "pending") {
            super::super::nyxbot_gateway_release_service::retire_connection(
                state,
                &channel,
                crate::models::nyxbot_gateway_release::ReleaseReason::Disconnect,
                "disconnected",
                None,
            )
            .await?;
        }
        let conversations: Vec<Document> = state
            .db
            .collection::<Document>("assistant_conversations")
            .find(doc! {"channel.nyxbot_channel_id":&channel.id,"guest_turn":true})
            .projection(doc! {"_id":1,"user_id":1})
            .await?
            .try_collect()
            .await?;
        for conversation in conversations {
            super::super::assistant_nyxagent::delete(
                &state.db,
                conversation
                    .get_str("user_id")
                    .map_err(|_| AppError::Internal("Invalid guest conversation".into()))?,
                conversation
                    .get_str("_id")
                    .map_err(|_| AppError::Internal("Invalid guest conversation".into()))?,
            )
            .await?;
        }
        for collection in [
            crate::models::nyxbot_channel::THREADS_COLLECTION_NAME,
            crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME,
            crate::models::org_channel::MESSAGES,
            crate::models::org_channel::LINKS,
            crate::models::org_channel::CHALLENGES,
        ] {
            state
                .db
                .collection::<Document>(collection)
                .delete_many(doc! {"channel_id":&channel.id})
                .await?;
        }
        state
            .db
            .collection::<Document>("nyxbot_channels")
            .delete_one(doc! {"_id":&channel.id,"status":{"$nin":["active","pending"]}})
            .await?;
    }
    for (collection, field) in [
        (
            crate::models::channel_text_delivery::COLLECTION_NAME,
            "channel_bot_id",
        ),
        (
            crate::models::channel_message::COLLECTION_NAME,
            "channel_bot_id",
        ),
        (
            crate::models::channel_conversation::COLLECTION_NAME,
            "channel_bot_id",
        ),
        (
            crate::models::channel_reply_delivery::COLLECTION_NAME,
            "channel_bot_id",
        ),
        (crate::models::channel_delivery::COLLECTION_NAME, "bot_id"),
        (
            crate::models::channel_connect_link::COLLECTION_NAME,
            "bot_id",
        ),
        (
            crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME,
            "channel_bot_id",
        ),
        (crate::services::channel_adapters::meta::CONTROLS, "bot_id"),
    ] {
        state
            .db
            .collection::<Document>(collection)
            .delete_many(doc! {field:bot_id})
            .await?;
    }
    state
        .db
        .collection::<crate::models::channel_meta::OAuthSession>(
            crate::models::channel_meta::SESSIONS,
        )
        .delete_many(doc! {"bot_id":bot_id})
        .await?;
    super::super::org_channel_service::defaults::erase(&state.db, doc! {"channel_bot_id": bot_id})
        .await?;
    #[cfg(test)]
    super::super::durable_wake_tests::after("meta_provider_before_completion").await;
    let args = (state.db.clone(), bot_id.to_owned(), grant_id.to_owned());
    super::super::transaction_service::run_db_only(&state.db, &args, |tx, (db, bot, grant)| {
        Box::pin(complete_provider_inbox(db, bot, grant, tx))
    })
    .await
}

// Completion proof and parent retirement have one commit boundary. Account
// cleanup cannot recreate an outstanding grant between these two writes.
async fn complete_provider_inbox(
    db: &Database,
    bot: &str,
    grant: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let changed = db
        .collection::<Document>("channel_meta_erasure_fences")
        .update_one(
            doc! {"_id":bot},
            doc! {
            "$set":{"local_data_erased_at":bson::DateTime::now(),"completed_grant_id":grant},
            "$unset":{"retired_grant":""}},
        )
        .session(&mut *tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(super::super::channel_meta_service::grants::incomplete());
    }
    super::super::channel_meta_service::scopes::mark_complete(db, tx, bot, grant).await?;
    let bots = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
    let deleted = bots
        .delete_one(doc! {"_id":bot,"is_active":false,
        "meta_binding.grant_id":grant,"meta_binding.subscription":"removed"})
        .session(&mut *tx)
        .await?;
    if deleted.deleted_count != 1
        && bots
            .find_one(doc! {"_id":bot})
            .session(&mut *tx)
            .await?
            .is_some()
    {
        return Err(AppError::Conflict(
            "The inbox changed during erasure; cleanup remains pending".into(),
        ));
    }
    Ok(())
}

pub(super) async fn pass(state: &crate::AppState, job: &PersonalErasureJob) -> AppResult<bool> {
    let db = super::super::transaction_service::durable_database(&state.db);
    let Some(bot) = db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find_one(doc! {"user_id":&job.user_id,"$or":[
            {"personal_erasure_cleanup_complete":{"$ne":true}},
            {"platform":{"$in":["messenger","instagram"]}}
        ]})
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
        state,
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
