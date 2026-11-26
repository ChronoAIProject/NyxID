//! Bounded inline delivery for adapters whose provider owns notification retries.
//! Claims and message records contain routing metadata only (ADR-013).

use super::{
    channel_guest_access::{self as guest, GuestMode, relay as guest_relay},
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
    adapter: &dyn super::channel_platform::PlatformAdapter,
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
    let messages = db.collection::<ChannelMessage>(MESSAGES);
    let previous = messages
        .find_one(doc! {
            "channel_bot_id": &bot.id, "platform": &bot.platform,
            "direction": "inbound", "platform_message_id": &inbound.platform_message_id,
        })
        .await?;
    if let Some(message) = &previous
        && settle_obsolete_guest_retry(db, bot, inbound, message).await?
    {
        return Ok(());
    }
    let route = channel_routing_service::resolve_agent(
        db,
        &bot.id,
        &inbound.conversation_id,
        Some(&inbound.sender_platform_id),
        &bot.user_id,
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
    let mut stored = match previous {
        Some(message) => message,
        None => {
            let mut message = channel_relay_service::inbound_metadata(
                &bot.id,
                &route.conversation.id,
                &bot.user_id,
                &bot.platform,
                inbound,
                &api_key.id,
                message_id,
            );
            if !guest_relay::prepare(db, bot, &route.conversation, inbound, &mut message).await? {
                return Err(retry_later());
            }
            if message.guest_relay.is_some() {
                super::channel_admission_service::store_current_owner(db, message.clone())
                    .await?
                    .ok_or_else(retry_later)?;
            } else {
                db.collection::<ChannelMessage>(MESSAGES)
                    .insert_one(&message)
                    .await?;
            }
            message
        }
    };
    if stored.thread_context.is_none()
        && let Ok(Some(facts)) =
            super::channel_thread_service::inbound_facts(db, bot, &api_key.id, adapter, inbound)
                .await
    {
        db.collection::<ChannelMessage>(MESSAGES)
            .update_one(
                doc! {"_id": &stored.id, "thread_context": bson::Bson::Null},
                doc! {"$set": {"thread_context": bson::to_bson(&facts).map_err(|e| AppError::Internal(e.to_string()))?}},
            )
            .await?;
        stored.thread_context = Some(facts);
    }
    if stored.user_id != bot.user_id
        || stored.platform_conversation_id.as_deref() != Some(&inbound.conversation_id)
        || stored.thread_id != inbound.thread_id
    {
        return Err(retry_later());
    }
    if stored.callback_status.as_deref() == Some("delivered")
        || stored.callback_failure_reason.as_deref()
            == Some(super::webhook_delivery_service::DESTINATION_REFUSED)
    {
        return Ok(());
    }
    guest_relay::revalidate_retry(db, bot, &route.conversation, &api_key.id, inbound, &stored)
        .await?;
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
    let reply_token = guest_relay::reply_token(
        context.jwt_keys,
        context.config,
        &stored,
        &route.conversation,
    )?;
    let mut payload = channel_relay_service::build_callback_payload(
        &stored,
        &route.conversation,
        &api_key.id,
        &api_key.name,
        inbound,
        Some(reply_token),
        &context.config.base_url,
    );
    payload.guest_access_token = guest_relay::guest_data_token(
        context.jwt_keys,
        context.config,
        &stored,
        &route.conversation,
    )?;
    // The sender is untrusted mail input. The runtime receives only the normal
    // agent-scoped delegation, never the mailbox credential.
    let owner = bot.user_id.parse().map_err(|_| retry_later())?;
    let access = if stored.guest_relay.is_some()
        || channel_relay_service::is_linked_org_nyxbot_route(db, &bot.id, &api_key.id).await?
    {
        None
    } else {
        let scope = super::token_service::FIRST_PARTY_ACCESS_SCOPES;
        let rbac = super::rbac_helpers::build_rbac_claim_data(db, &bot.user_id, scope)
            .await
            .ok();
        Some(crate::crypto::jwt::generate_relay_access_token(
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
        )?)
    };
    let mut config = context.config.clone();
    config.channel_relay_callback_timeout_secs = config.channel_relay_callback_timeout_secs.min(10);
    if stored.guest_relay.is_some() {
        guest_relay::revalidate_delivery(db, &stored, &route.conversation, bot, &api_key.id, None)
            .await?;
    }
    let delivery = channel_relay_service::forward_to_agent(
        channel_relay_service::callback_request(
            db,
            &config,
            Some(&bot.id),
            &api_key.id,
            callback_url,
        )
        .await,
        &config,
        context.jwt_keys,
        payload,
        &api_key.id,
        &api_key.key_hash,
        access.as_deref(),
    )
    .await;
    if let Err(error) = &delivery.result {
        super::channel_activity_service::record_callback_failure(
            db,
            &stored,
            delivery.http_status,
            error,
        )
        .await?;
    } else {
        channel_relay_service::update_callback_status(db, &stored.id, "delivered").await?;
        channel_routing_service::touch_conversation(db, &route.conversation.id).await?;
    }
    super::audit_service::log_async(
        db.clone(),
        Some(bot.user_id.clone()),
        "channel_email_callback".into(),
        Some(
            serde_json::json!({"bot_id": bot.id, "message_id": stored.id, "conversation_id": route.conversation.id,
            "outcome": if delivery.result.is_ok() { "delivered" } else { "failed" }}),
        ),
        None,
        None,
        Some(api_key.id),
        Some(api_key.name),
    );
    match delivery.result {
        Err(error) if channel_relay_service::destination_refused(&error) => Ok(()),
        result => result,
    }
}

/// A successfully read generation or policy change makes the original
/// admission terminal. A failed authority read leaves the receipt retryable.
async fn settle_obsolete_guest_retry(
    db: &mongodb::Database,
    bot: &ChannelBot,
    inbound: &InboundMessage,
    message: &ChannelMessage,
) -> AppResult<bool> {
    let Some(turn) = message.guest_relay.as_ref() else {
        return Ok(false);
    };
    if message.user_id != bot.user_id
        || turn.bot_id != bot.id
        || message.platform_conversation_id.as_deref() != Some(&inbound.conversation_id)
        || message.sender_platform_id.as_deref() != Some(&inbound.sender_platform_id)
        || message.thread_id != inbound.thread_id
    {
        return Err(retry_later());
    }
    let rows = db.collection::<bson::Document>(MESSAGES);
    let source = doc! {
        "_id": &message.id, "channel_bot_id": &bot.id, "user_id": &bot.user_id,
        "guest_relay.activation_epoch": turn.activation_epoch,
        "guest_relay.bot_revision": turn.bot_revision,
        "guest_relay.policy_revision": turn.policy_revision,
    };
    let mut terminal = source.clone();
    terminal.insert("guest_relay_obsolete_at", doc! {"$type":"date"});
    if rows
        .find_one(terminal)
        .projection(doc! {"_id": 1})
        .await?
        .is_some()
    {
        return Ok(true);
    }
    let mode = guest::select_mode(db, &bot.id).await?;
    if mode
        == (GuestMode::AddOn {
            activation_epoch: turn.activation_epoch,
        })
        && guest::policy::load_policy(db, &bot.id)
            .await?
            .map_or(0, |policy| policy.revision)
            == turn.policy_revision
    {
        return Ok(false);
    }
    let mut pending = source;
    pending.insert("callback_status", doc! {"$ne":"delivered"});
    pending.insert("guest_relay_obsolete_at", bson::Bson::Null);
    rows.update_one(
        pending,
        doc! {"$set": {"guest_relay_obsolete_at": bson::DateTime::now(), "callback_status": "failed"}},
    ).await?;
    Ok(true)
}
