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

pub async fn with_lifecycle<T>(
    db: &mongodb::Database,
    serialize: bool,
    bot_id: &str,
    work: impl Future<Output = AppResult<T>>,
) -> AppResult<T> {
    with_claim(db, serialize, "channel-bot-lifecycle", bot_id, work).await
}

pub async fn with_ingress<T>(
    db: &mongodb::Database,
    bot_id: &str,
    work: impl Future<Output = AppResult<T>>,
) -> AppResult<T> {
    with_claim(db, true, "channel-bot-ingress", bot_id, work).await
}

async fn with_claim<T>(
    db: &mongodb::Database,
    serialize: bool,
    namespace: &str,
    bot_id: &str,
    work: impl Future<Output = AppResult<T>>,
) -> AppResult<T> {
    if !serialize {
        return work.await;
    }
    let EventDedupClaimResult::Claimed(claim) =
        EventDedupStore::claim(db, namespace, bot_id, "mutation", Duration::from_secs(120)).await?
    else {
        return Err(retry_later());
    };
    let result = tokio::time::timeout(Duration::from_secs(90), work)
        .await
        .unwrap_or_else(|_| Err(retry_later()));
    EventDedupStore::release(db, &claim).await?;
    result
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
