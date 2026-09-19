//! Bounded inline delivery for adapters whose provider owns notification retries.
//! Claims and message records contain routing metadata only (ADR-013).

use super::{
    channel_platform::InboundMessage,
    channel_relay_service, channel_routing_service,
    coordination_service::{EventDedupClaimResult, EventDedupStore},
};
use crate::models::{
    api_key::{ApiKey, COLLECTION_NAME as API_KEYS},
    channel_bot::ChannelBot,
    channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
};
use crate::{
    config::AppConfig,
    crypto::{aes::EncryptionKeys, jwt::JwtKeys},
    errors::{AppError, AppResult},
};
use bson::doc;
use std::{future::Future, time::Duration};

pub struct IngressContext<'a> {
    pub db: &'a mongodb::Database,
    pub config: &'a AppConfig,
    pub jwt_keys: &'a JwtKeys,
    pub encryption_keys: &'a EncryptionKeys,
    pub http: &'a reqwest::Client,
    pub rate_limiter: &'a crate::mw::rate_limit::PerChannelEventLimiter,
}

pub fn retry_later() -> AppError {
    AppError::ChannelRelayFailed("Channel delivery is not ready; retry this notification".into())
}

// Box work before constructing the returned future. Boxing inside an async fn
// would still embed the original work in that function's initial state.
pub fn with_lifecycle<'a, T: 'a>(
    db: &'a mongodb::Database,
    serialize: bool,
    bot_id: &'a str,
    work: impl Future<Output = AppResult<T>> + 'a,
) -> impl Future<Output = AppResult<T>> + 'a {
    let work = Box::pin(work);
    Box::pin(async move {
        if serialize {
            let bot = super::channel_bot_service::get_bot(db, bot_id).await?;
            if bot.platform == "aurinko" && bot.credential_source == "connection" {
                let connection = bot.connection_id.as_deref().ok_or_else(retry_later)?;
                return with_connection(db, connection, async {
                    let live = super::channel_bot_service::get_bot(db, bot_id).await?;
                    if live.connection_id != bot.connection_id
                        || live.user_id != bot.user_id
                        || live.updated_at != bot.updated_at
                    {
                        return Err(retry_later());
                    }
                    with_claim(db, true, "channel-bot-lifecycle", bot_id, work).await
                })
                .await;
            }
        }
        with_claim(db, serialize, "channel-bot-lifecycle", bot_id, work).await
    })
}

pub fn with_ingress<'a, T: 'a>(
    db: &'a mongodb::Database,
    bot_id: &'a str,
    work: impl Future<Output = AppResult<T>> + 'a,
) -> impl Future<Output = AppResult<T>> + 'a {
    let work = Box::pin(work);
    Box::pin(async move {
        let bot = super::channel_bot_service::get_bot(db, bot_id).await?;
        if bot.platform == "aurinko" && bot.credential_source == "connection" {
            return with_connection(
                db,
                bot.connection_id.as_deref().ok_or_else(retry_later)?,
                async {
                    let live = super::channel_bot_service::get_bot(db, bot_id).await?;
                    if live.connection_id != bot.connection_id
                        || live.user_id != bot.user_id
                        || live.updated_at != bot.updated_at
                    {
                        return Err(retry_later());
                    }
                    with_claim(db, true, "channel-bot-ingress", bot_id, work).await
                },
            )
            .await;
        }
        with_claim(db, true, "channel-bot-ingress", bot_id, work).await
    })
}

/// Used only inside an already-held lifecycle/connection guard during deletion.
pub(crate) fn with_bot_ingress<'a, T: 'a>(
    db: &'a mongodb::Database,
    bot_id: &'a str,
    work: impl Future<Output = AppResult<T>> + 'a,
) -> impl Future<Output = AppResult<T>> + 'a {
    with_claim(db, true, "channel-bot-ingress", bot_id, work)
}

/// Shared mailbox effects and connection mutations use this claim before any
/// per-bot claim, preventing reconnect/disable/delete from racing an effect.
pub fn with_connection<'a, T: 'a>(
    db: &'a mongodb::Database,
    key_id: &'a str,
    work: impl Future<Output = AppResult<T>> + 'a,
) -> impl Future<Output = AppResult<T>> + 'a {
    with_claim(db, true, "aurinko-connection", key_id, work)
}

/// Acquire a sorted set once for grant-cascade mutations, before any bot claims.
pub(crate) fn with_connections<'a, T: 'a>(
    db: &'a mongodb::Database,
    key_ids: &'a [String],
    work: impl Future<Output = AppResult<T>> + 'a,
) -> impl Future<Output = AppResult<T>> + 'a {
    let work = Box::pin(work);
    Box::pin(async move {
        if key_ids.is_empty() {
            return work.await;
        }
        let mut ids = key_ids.to_vec();
        ids.sort();
        ids.dedup();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut claims = Vec::new();
        let result = async {
            for id in ids {
                match tokio::time::timeout_at(
                    deadline,
                    EventDedupStore::claim(
                        db,
                        "aurinko-connection",
                        &id,
                        "mutation",
                        Duration::from_secs(120),
                    ),
                )
                .await
                .map_err(|_| retry_later())??
                {
                    EventDedupClaimResult::Claimed(claim) => claims.push(claim),
                    EventDedupClaimResult::Duplicate => return Err(retry_later()),
                }
            }
            tokio::time::timeout_at(deadline, work)
                .await
                .unwrap_or_else(|_| Err(retry_later()))
        }
        .await;
        for claim in &claims {
            EventDedupStore::release(db, claim).await?;
        }
        result
    })
}

fn with_claim<'a, T: 'a>(
    db: &'a mongodb::Database,
    serialize: bool,
    namespace: &'a str,
    bot_id: &'a str,
    work: impl Future<Output = AppResult<T>> + 'a,
) -> impl Future<Output = AppResult<T>> + 'a {
    let work = Box::pin(work);
    Box::pin(async move {
        if !serialize {
            return work.await;
        }
        let EventDedupClaimResult::Claimed(claim) =
            EventDedupStore::claim(db, namespace, bot_id, "mutation", Duration::from_secs(120))
                .await?
        else {
            return Err(retry_later());
        };
        let result = tokio::time::timeout(Duration::from_secs(90), work)
            .await
            .unwrap_or_else(|_| Err(retry_later()));
        EventDedupStore::release(db, &claim).await?;
        result
    })
}

/// Reuse the message ID on a failed attempt so downstream agents can deduplicate
/// a callback whose acceptance response was lost in transit.
pub async fn deliver(
    context: &IngressContext<'_>,
    bot: &ChannelBot,
    inbound: &InboundMessage,
    message_id: &str,
) -> AppResult<()> {
    let db = context.db;
    let live = super::channel_bot_service::get_bot(db, &bot.id).await?;
    if !live.is_active
        || live.status != "active"
        || live.user_id != bot.user_id
        || live.platform_bot_id != bot.platform_bot_id
        || live.updated_at != bot.updated_at
    {
        return Err(retry_later());
    }
    let route = channel_routing_service::resolve_agent(
        db,
        &bot.id,
        &inbound.conversation_id,
        Some(&inbound.sender_platform_id),
    )
    .await?
    .ok_or_else(retry_later)?;
    if route.conversation.user_id != bot.user_id || route.conversation.platform != bot.platform {
        return Err(retry_later());
    }
    let api_key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": &route.api_key_id, "user_id": &bot.user_id, "is_active": true })
        .await?
        .filter(|key| {
            key.expires_at
                .is_none_or(|expiry| expiry > chrono::Utc::now())
        })
        .ok_or_else(retry_later)?;
    let callback_url = api_key
        .callback_url
        .as_deref()
        .filter(|url| !url.is_empty())
        .ok_or_else(retry_later)?;
    if !context
        .rate_limiter
        .check_shared(&route.conversation.id)
        .await?
    {
        return Err(AppError::RateLimited);
    }
    let mut stored = match db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {
            "channel_bot_id": &bot.id, "platform": &bot.platform,
            "direction": "inbound", "platform_message_id": &inbound.platform_message_id,
        })
        .await?
    {
        Some(message) => message,
        None => {
            channel_relay_service::store_inbound_message_with_id(
                db,
                &bot.id,
                &route.conversation.id,
                &bot.user_id,
                &bot.platform,
                inbound,
                &api_key.id,
                message_id,
            )
            .await?
        }
    };
    if stored.user_id != bot.user_id
        || stored.platform_conversation_id.as_deref() != Some(&inbound.conversation_id)
        || stored.thread_id != inbound.thread_id
    {
        return Err(retry_later());
    }
    if stored.callback_status.as_deref() == Some("delivered") {
        return Ok(());
    }
    // Pending retries follow the current owner-approved route while keeping the
    // UUID stable. Old route-bound reply JWTs cease to match this message.
    if stored.conversation_id != route.conversation.id
        || stored.agent_api_key_id.as_deref() != Some(&api_key.id)
    {
        db.collection::<ChannelMessage>(MESSAGES)
            .update_one(
                doc! { "_id": &stored.id },
                doc! { "$set": {
                    "conversation_id": &route.conversation.id, "agent_api_key_id": &api_key.id,
                }},
            )
            .await?;
        stored.conversation_id = route.conversation.id.clone();
        stored.agent_api_key_id = Some(api_key.id.clone());
    }
    let reply_token = crate::crypto::jwt::generate_relay_reply_token(
        context.jwt_keys,
        context.config,
        &api_key.id,
        &route.conversation.id,
        &stored.id,
        &bot.platform,
    )?;
    let payload = channel_relay_service::build_callback_payload(
        &stored,
        &route.conversation,
        &api_key.id,
        &api_key.name,
        inbound,
        Some(reply_token),
        &context.config.base_url,
    );
    // The sender is untrusted mail input. The runtime receives only the normal
    // agent-scoped delegation, never the mailbox credential.
    let owner = bot.user_id.parse().map_err(|_| retry_later())?;
    let scope = super::token_service::FIRST_PARTY_ACCESS_SCOPES;
    let rbac = super::rbac_helpers::build_rbac_claim_data(db, &bot.user_id, scope)
        .await
        .ok();
    let access = crate::crypto::jwt::generate_relay_access_token(
        context.jwt_keys,
        context.config,
        &owner,
        scope,
        rbac.as_ref(),
        &crate::crypto::jwt::RelayAgentScope {
            api_key_id: api_key.id.clone(),
            api_key_name: api_key.name.clone(),
            allowed_service_ids: api_key.allowed_service_ids.clone(),
            allowed_node_ids: api_key.allowed_node_ids.clone(),
            allow_all_services: api_key.allow_all_services,
            allow_all_nodes: api_key.allow_all_nodes,
        },
    )?;
    let mut config = context.config.clone();
    config.channel_relay_callback_timeout_secs = config.channel_relay_callback_timeout_secs.min(10);
    let result = channel_relay_service::forward_to_agent(
        context.http,
        &config,
        context.jwt_keys,
        callback_url,
        payload,
        &api_key.id,
        &api_key.key_hash,
        Some(&access),
    )
    .await
    .result;
    channel_relay_service::update_callback_status(
        db,
        &stored.id,
        if result.is_ok() {
            "delivered"
        } else {
            "failed"
        },
    )
    .await?;
    if result.is_ok() {
        channel_routing_service::touch_conversation(db, &route.conversation.id).await?;
    }
    super::audit_service::log_async(
        db.clone(),
        Some(bot.user_id.clone()),
        "channel_email_callback".into(),
        Some(
            serde_json::json!({"bot_id": bot.id, "message_id": stored.id, "conversation_id": route.conversation.id,
            "outcome": if result.is_ok() { "delivered" } else { "failed" }}),
        ),
        None,
        None,
        Some(api_key.id),
        Some(api_key.name),
    );
    result
}

#[cfg(test)]
mod future_size_tests {
    use super::*;

    fn large_work() -> impl Future<Output = AppResult<usize>> {
        let bytes = [0_u8; 64 * 1024];
        async move {
            std::future::pending::<()>().await;
            Ok(std::hint::black_box(bytes).len())
        }
    }

    fn assert_bounded(future: impl Future) {
        assert!(
            std::mem::size_of_val(&future) <= 1024,
            "claim wrapper embeds work on the caller's stack: {} bytes",
            std::mem::size_of_val(&future),
        );
    }

    #[tokio::test]
    async fn claim_wrappers_bound_large_unpolled_work_without_database_access() {
        let db =
            mongodb::Client::with_uri_str("mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=50")
                .await
                .unwrap()
                .database("unused_future_size_regression");
        assert!(std::mem::size_of_val(&large_work()) >= 64 * 1024);
        assert_bounded(with_lifecycle(&db, false, "bot", large_work()));
        assert_bounded(with_lifecycle(&db, true, "bot", large_work()));
        assert_bounded(with_ingress(&db, "bot", large_work()));
        assert_bounded(with_bot_ingress(&db, "bot", large_work()));
        assert_bounded(with_connection(&db, "connection", large_work()));
        assert_bounded(with_connections(&db, &[], large_work()));
        assert_bounded(with_claim(&db, true, "namespace", "bot", large_work()));

        // Ordinary providers and an empty managed-connection set keep the
        // direct execution path and must never attempt a database claim.
        assert_eq!(
            with_lifecycle(&db, false, "bot", async { Ok(1) })
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            with_connections(&db, &[], async { Ok(2) }).await.unwrap(),
            2
        );
    }
}
