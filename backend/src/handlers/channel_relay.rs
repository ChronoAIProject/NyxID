//! Agent-facing channel relay endpoints (API-key or reply-token authenticated).
//!
//! These endpoints allow agents to send asynchronous replies to platform
//! conversations, edit previously-sent platform replies, list conversation
//! message history, and resolve platform senders to NyxID users.
//!
//! Reply tokens are single-use for the initial `POST /reply` send: their JTI is
//! consumed on first successful use. `POST /reply/update` reuses the same token
//! by validating signature + bindings and requiring that the JTI was already
//! consumed by the original send, which ties edits to a prior reply without
//! minting a separate edit-only token.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use base64::Engine as _;
use chrono::{Duration, Utc};
use mongodb::bson::{Bson, doc};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::crypto::jwt;
use crate::errors::{AppError, AppResult};
use crate::handlers::channel_bots::resolve_adapter;
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::channel_bot::ChannelBot;
use crate::models::channel_conversation::{COLLECTION_NAME as CONVERSATIONS, ChannelConversation};
use crate::models::channel_message::ChannelMessage;
use crate::models::notification_channel::{
    COLLECTION_NAME as NOTIFICATION_CHANNELS, NotificationChannel,
};
use crate::models::reply_token_use::{COLLECTION_NAME as REPLY_TOKEN_USES, ReplyTokenUse};
use crate::mw::auth::{AuthUser, OptionalAuthUser};
use crate::services::{
    audit_service, channel_bot_service,
    channel_platform::{OutboundEdit, OutboundReply},
    channel_relay_service,
};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AsyncReplyRequest {
    pub message_id: String,
    pub reply: AsyncReplyBody,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReplyRequest {
    pub message_id: String,
    pub reply: AsyncReplyBody,
}

#[derive(Debug, Deserialize)]
pub struct AsyncReplyBody {
    pub text: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

fn default_page() -> u64 {
    1
}

fn default_per_page() -> u64 {
    50
}

#[derive(Debug, Deserialize)]
pub struct ResolveSenderQuery {
    pub platform: String,
    pub platform_id: String,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AsyncReplyResponse {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_message_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UpdateReplyResponse {
    pub upstream_message_id: String,
    pub edited_at: String,
}

/// Metadata-only message summary returned from
/// `GET /api/v1/channel-relay/messages/{conversation_id}`.
///
/// **Breaking change (ADR-013):** this response used to include `text` and
/// `attachments`. Per the NyxID pure-passthrough principle, message content
/// is no longer stored or returned. Agents that need historical bodies must
/// keep their own conversation state.
#[derive(Debug, Serialize)]
pub struct MessageItem {
    pub id: String,
    pub direction: String,
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_platform_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_display_name: Option<String>,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct MessageListResponse {
    pub messages: Vec<MessageItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Serialize)]
pub struct ResolveSenderResponse {
    pub platform: String,
    pub platform_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nyxid_user_id: Option<String>,
    pub linked: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Platforms that support `metadata.card` as a content carrier.
///
/// Feishu Card JSON 2.0 is the only card format currently wired through the
/// proxy. Telegram and Discord have their own rich-message concepts
/// (inline keyboards, embeds) but route through different metadata keys,
/// so a `card` key sent to them would drop on the floor and the reply
/// would go out as empty text.
fn platform_supports_cards(platform: &str) -> bool {
    matches!(platform, "lark" | "feishu")
}

/// Validate that a reply body carries something the target platform can send.
///
/// Rules:
/// - Non-empty `text` is always accepted.
/// - `metadata.card` is accepted only for platforms where
///   [`platform_supports_cards`] returns true; per issue #306, agents may
///   send card-only replies with `text: null` to Lark/Feishu.
/// - Otherwise reject before hitting the platform API, so callers get a
///   clear error instead of an empty message going out.
fn validate_reply_for_platform(body: &AsyncReplyBody, platform: &str) -> AppResult<()> {
    let has_text = body.text.as_deref().is_some_and(|s| !s.is_empty());
    let has_card = body.metadata.as_ref().and_then(|m| m.get("card")).is_some();

    if has_text {
        return Ok(());
    }
    if has_card && platform_supports_cards(platform) {
        return Ok(());
    }
    if has_card {
        return Err(AppError::ValidationError(format!(
            "metadata.card is only supported on Lark/Feishu (got platform={platform})"
        )));
    }
    Err(AppError::ValidationError(
        "Reply must include non-empty text or metadata.card".to_string(),
    ))
}

/// Returns true if a reply to `(original_message, conversation)` must be
/// rejected because it targets a device channel (NyxID#221 / ADR-013).
///
/// Both fields are independently load-bearing:
///
/// * `conversation_platform == "device"` covers the happy path where the
///   conversation row itself is a device channel.
/// * `original_platform == "device"` covers **legacy `ChannelMessage` rows**
///   written by the prior event-gateway behavior — when device events
///   were stored with `platform="device"` on the message row while the
///   parent conversation was still Telegram/Discord/Lark/Feishu. Those
///   rows survived the upgrade and must not dispatch through a bot
///   adapter. The new `forward_event` filter (see
///   `channel_event_service::conversation_lookup_filter`) prevents new
///   such rows from being created, but pre-existing ones can only be
///   blocked here.
fn is_device_reply_forbidden(original_platform: &str, conversation_platform: &str) -> bool {
    original_platform == "device" || conversation_platform == "device"
}

fn message_to_item(msg: &crate::models::channel_message::ChannelMessage) -> MessageItem {
    MessageItem {
        id: msg.id.clone(),
        direction: msg.direction.clone(),
        platform: msg.platform.clone(),
        platform_message_id: msg.platform_message_id.clone(),
        sender_platform_id: msg.sender_platform_id.clone(),
        sender_display_name: msg.sender_display_name.clone(),
        content_type: msg.content_type.clone(),
        callback_status: msg.callback_status.clone(),
        reply_to_message_id: msg.reply_to_message_id.clone(),
        created_at: msg.created_at.to_rfc3339(),
    }
}

/// Padding applied to `ReplyTokenUse.exp_at` on insert so the TTL index does
/// not GC a usage record while the validator would still accept the same
/// token. Covers the validator's `RELAY_REPLY_CLOCK_SKEW_SECS` acceptance
/// window past `exp` plus MongoDB's ~60s TTL-monitor sweep interval.
const REPLY_TOKEN_USE_TTL_BUFFER_SECS: i64 = 120;

#[derive(Debug)]
struct ReplyRequestContext {
    original: ChannelMessage,
    conversation: ChannelConversation,
    attributed_api_key_id: String,
    validated_bot: Option<ChannelBot>,
}

#[derive(Debug)]
struct EditRequestContext {
    outbound: ChannelMessage,
    conversation: ChannelConversation,
    bot: ChannelBot,
    attributed_api_key_id: String,
}

fn extract_bearer_token(headers: &HeaderMap) -> AppResult<Option<String>> {
    let Some(raw) = headers.get("authorization") else {
        return Ok(None);
    };

    let auth = raw
        .to_str()
        .map_err(|_| AppError::Unauthorized("Invalid authorization header".to_string()))?;
    Ok(auth
        .strip_prefix("Bearer ")
        .map(std::string::ToString::to_string))
}

/// Peek at the (unverified) JWT payload to decide which auth branch this
/// request belongs to. Signature verification is intentionally deferred to
/// `validate_relay_reply_token` — a forged `aud` here only routes the
/// request into the reply-token pipeline, which then fails signature.
fn token_targets_reply_audience(token: &str) -> bool {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() < 2 {
        return false;
    }

    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(bytes) => bytes,
        Err(_) => match base64::engine::general_purpose::URL_SAFE.decode(parts[1]) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        },
    };

    let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&payload) else {
        return false;
    };

    match claims.get("aud") {
        Some(serde_json::Value::String(aud)) => aud == jwt::RELAY_REPLY_AUDIENCE,
        Some(serde_json::Value::Array(auds)) => auds
            .iter()
            .any(|aud| aud.as_str() == Some(jwt::RELAY_REPLY_AUDIENCE)),
        _ => false,
    }
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

async fn load_active_conversation(
    state: &AppState,
    conversation_id: &str,
) -> AppResult<ChannelConversation> {
    state
        .db
        .collection::<ChannelConversation>(CONVERSATIONS)
        .find_one(doc! { "_id": conversation_id, "is_active": true })
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Conversation not found or inactive: {conversation_id}"
            ))
        })
}

async fn load_active_api_key(state: &AppState, api_key_id: &str) -> AppResult<ApiKey> {
    let api_key = state
        .db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": api_key_id })
        .await?
        .ok_or_else(|| AppError::Unauthorized("Reply token API key not found".to_string()))?;

    let expired = api_key.expires_at.is_some_and(|exp| exp <= Utc::now());
    if !api_key.is_active || expired {
        return Err(AppError::Unauthorized(
            "Reply token API key is inactive".to_string(),
        ));
    }

    Ok(api_key)
}

async fn load_active_bot(state: &AppState, original: &ChannelMessage) -> AppResult<ChannelBot> {
    let channel_bot_id = original.channel_bot_id.as_deref().ok_or_else(|| {
        AppError::Internal(
            "bot-backed conversation is missing channel_bot_id on its message row".to_string(),
        )
    })?;
    let bot = channel_bot_service::get_bot(&state.db, channel_bot_id).await?;
    if !bot.is_active {
        return Err(AppError::ChannelBotInactive(
            "Bot has been deactivated".to_string(),
        ));
    }
    Ok(bot)
}

async fn consume_reply_token_use(
    state: &AppState,
    claims: &jwt::RelayReplyClaims,
) -> AppResult<()> {
    // Pad `exp_at` beyond the validator's clock-skew tolerance so the TTL
    // index never GCs a usage record while the same JWT would still pass
    // validation on a skewed verifier. `REPLY_TOKEN_USE_TTL_BUFFER_SECS`
    // covers both the validator's `exp + RELAY_REPLY_CLOCK_SKEW_SECS`
    // acceptance window and MongoDB's 60s TTL-monitor sweep interval.
    let exp_at = chrono::DateTime::from_timestamp(claims.exp + REPLY_TOKEN_USE_TTL_BUFFER_SECS, 0)
        .ok_or_else(|| AppError::Unauthorized("Invalid relay reply token".to_string()))?;
    let usage = ReplyTokenUse {
        id: claims.jti.clone(),
        exp_at,
        api_key_id: claims.api_key_id.clone(),
        conversation_id: claims.conversation_id.clone(),
        consumed_at: Utc::now(),
    };

    match state
        .db
        .collection::<ReplyTokenUse>(REPLY_TOKEN_USES)
        .insert_one(&usage)
        .await
    {
        Ok(_) => Ok(()),
        Err(err) => {
            if let mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(
                write_error,
            )) = err.kind.as_ref()
                && write_error.code == 11000
            {
                return Err(AppError::Unauthorized(
                    "Reply token already used".to_string(),
                ));
            }
            Err(AppError::from(err))
        }
    }
}

async fn reply_token_has_been_consumed(state: &AppState, jti: &str) -> AppResult<bool> {
    Ok(state
        .db
        .collection::<ReplyTokenUse>(REPLY_TOKEN_USES)
        .find_one(doc! { "_id": jti })
        .await?
        .is_some())
}

fn check_message_edit_rate_limit(state: &AppState, platform_message_id: &str) -> AppResult<()> {
    if !state.per_message_edit_limiter.check(platform_message_id) {
        tracing::warn!(
            platform_message_id = %platform_message_id,
            "Per-message edit rate limit exceeded"
        );
        return Err(AppError::RateLimited);
    }

    Ok(())
}

async fn resolve_reply_token_context(
    state: &AppState,
    token: &str,
    body: &AsyncReplyRequest,
) -> AppResult<ReplyRequestContext> {
    let claims = jwt::validate_relay_reply_token(&state.jwt_keys, &state.config, token)?;

    let original = channel_relay_service::get_message(&state.db, &body.message_id).await?;
    if claims.inbound_message_id != body.message_id {
        return Err(AppError::Unauthorized(
            "Reply token message_id mismatch".to_string(),
        ));
    }

    let conversation = load_active_conversation(state, &original.conversation_id).await?;
    if claims.conversation_id != conversation.id || original.conversation_id != conversation.id {
        return Err(AppError::Unauthorized(
            "Reply token conversation mismatch".to_string(),
        ));
    }

    // Device conversations have no bot and no reply surface (ADR-013).
    // Reject with the shared device error before `load_active_bot` would
    // surface an `Internal` 500 on the missing `channel_bot_id`.
    if is_device_reply_forbidden(&original.platform, &conversation.platform) {
        return Err(AppError::DeviceChannelReplyNotAllowed);
    }

    // Unlike the API-key branch we do NOT re-check
    // `conversation.agent_api_key_id` against `claims.api_key_id`. The token
    // was minted for this specific inbound message; allowing the agent who
    // received that callback to complete its reply — even if the
    // conversation has since been reassigned — avoids dropping in-flight
    // LLM responses. Scope narrowness is enforced by the token's other
    // bindings (conversation_id, inbound_message_id) and by the live
    // `api_key.is_active` re-check below.
    let api_key = load_active_api_key(state, &claims.api_key_id).await?;
    let bot = load_active_bot(state, &original).await?;

    if claims.platform != conversation.platform {
        return Err(AppError::Unauthorized(
            "Reply token platform mismatch".to_string(),
        ));
    }

    consume_reply_token_use(state, &claims).await?;

    Ok(ReplyRequestContext {
        original,
        conversation,
        attributed_api_key_id: api_key.id,
        validated_bot: Some(bot),
    })
}

async fn resolve_api_key_reply_context(
    state: &AppState,
    auth_user: &AuthUser,
    body: &AsyncReplyRequest,
) -> AppResult<ReplyRequestContext> {
    let original = channel_relay_service::get_message(&state.db, &body.message_id).await?;
    let conversation = load_active_conversation(state, &original.conversation_id).await?;

    let caller_api_key_id = auth_user.api_key_id.as_deref().ok_or_else(|| {
        AppError::Forbidden("This endpoint requires API key authentication".to_string())
    })?;

    if conversation.agent_api_key_id != caller_api_key_id {
        return Err(AppError::Forbidden(
            "API key is not the assigned agent for this conversation".to_string(),
        ));
    }

    Ok(ReplyRequestContext {
        original,
        conversation,
        attributed_api_key_id: caller_api_key_id.to_string(),
        validated_bot: None,
    })
}

async fn find_outbound_message_for_api_key(
    state: &AppState,
    api_key_id: &str,
    platform_message_id: &str,
) -> AppResult<ChannelMessage> {
    let platforms = state
        .db
        .collection::<ChannelConversation>(CONVERSATIONS)
        .distinct("platform", doc! { "agent_api_key_id": api_key_id })
        .await?;

    let mut found: Option<ChannelMessage> = None;

    for platform in platforms {
        let Bson::String(platform) = platform else {
            continue;
        };

        match channel_relay_service::get_outbound_message_by_platform_id(
            &state.db,
            &platform,
            platform_message_id,
        )
        .await
        {
            Ok(message) => {
                if found.is_some() {
                    return Err(AppError::Conflict(format!(
                        "Multiple outbound messages found for platform message ID: {platform_message_id}"
                    )));
                }
                found = Some(message);
            }
            Err(AppError::NotFound(_)) => {}
            Err(err) => return Err(err),
        }
    }

    found.ok_or_else(|| {
        AppError::NotFound(format!("Outbound message not found: {platform_message_id}"))
    })
}

async fn resolve_reply_token_edit_context(
    state: &AppState,
    token: &str,
    body: &UpdateReplyRequest,
) -> AppResult<EditRequestContext> {
    let claims = jwt::validate_relay_reply_token(&state.jwt_keys, &state.config, token)?;

    if !reply_token_has_been_consumed(state, &claims.jti).await? {
        return Err(AppError::Unauthorized(
            "reply token must be used to send before it can edit".to_string(),
        ));
    }

    let outbound = channel_relay_service::get_outbound_message_by_platform_id(
        &state.db,
        &claims.platform,
        &body.message_id,
    )
    .await?;

    if outbound.reply_to_message_id.as_deref() != Some(claims.inbound_message_id.as_str()) {
        return Err(AppError::Unauthorized(
            "Reply token inbound_message_id mismatch".to_string(),
        ));
    }

    let conversation = load_active_conversation(state, &outbound.conversation_id).await?;
    if claims.conversation_id != conversation.id || outbound.conversation_id != conversation.id {
        return Err(AppError::Unauthorized(
            "Reply token conversation mismatch".to_string(),
        ));
    }

    if claims.platform != outbound.platform || claims.platform != conversation.platform {
        return Err(AppError::Unauthorized(
            "Reply token platform mismatch".to_string(),
        ));
    }

    if is_device_reply_forbidden(&outbound.platform, &conversation.platform) {
        return Err(AppError::DeviceChannelReplyNotAllowed);
    }

    let api_key = load_active_api_key(state, &claims.api_key_id).await?;
    let bot = load_active_bot(state, &outbound).await?;

    Ok(EditRequestContext {
        outbound,
        conversation,
        bot,
        attributed_api_key_id: api_key.id,
    })
}

async fn resolve_api_key_edit_context(
    state: &AppState,
    auth_user: &AuthUser,
    body: &UpdateReplyRequest,
) -> AppResult<EditRequestContext> {
    let caller_api_key_id = auth_user.api_key_id.as_deref().ok_or_else(|| {
        AppError::Forbidden("This endpoint requires API key authentication".to_string())
    })?;

    let outbound =
        find_outbound_message_for_api_key(state, caller_api_key_id, &body.message_id).await?;
    let conversation = load_active_conversation(state, &outbound.conversation_id).await?;

    if conversation.agent_api_key_id != caller_api_key_id {
        return Err(AppError::Forbidden(
            "API key is not the assigned agent for this conversation".to_string(),
        ));
    }

    if is_device_reply_forbidden(&outbound.platform, &conversation.platform) {
        return Err(AppError::DeviceChannelReplyNotAllowed);
    }

    let bot = load_active_bot(state, &outbound).await?;

    Ok(EditRequestContext {
        outbound,
        conversation,
        bot,
        attributed_api_key_id: caller_api_key_id.to_string(),
    })
}

async fn resolve_reply_request_context(
    state: &AppState,
    headers: &HeaderMap,
    auth_user: Option<&AuthUser>,
    body: &AsyncReplyRequest,
) -> AppResult<ReplyRequestContext> {
    if let Some(token) = extract_bearer_token(headers)?
        && token_targets_reply_audience(&token)
    {
        return resolve_reply_token_context(state, &token, body).await;
    }

    let auth_user = auth_user.ok_or_else(|| {
        AppError::Unauthorized("API key or relay reply token required".to_string())
    })?;
    resolve_api_key_reply_context(state, auth_user, body).await
}

async fn resolve_edit_request_context(
    state: &AppState,
    headers: &HeaderMap,
    auth_user: Option<&AuthUser>,
    body: &UpdateReplyRequest,
) -> AppResult<EditRequestContext> {
    if let Some(token) = extract_bearer_token(headers)?
        && token_targets_reply_audience(&token)
    {
        return resolve_reply_token_edit_context(state, &token, body).await;
    }

    let auth_user = auth_user.ok_or_else(|| {
        AppError::Unauthorized("API key or relay reply token required".to_string())
    })?;
    resolve_api_key_edit_context(state, auth_user, body).await
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/channel-relay/reply
///
/// Send an asynchronous reply to a platform conversation. The agent identifies
/// the original inbound message by `message_id` and provides the reply text.
pub async fn async_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    OptionalAuthUser(auth_user): OptionalAuthUser,
    Json(body): Json<AsyncReplyRequest>,
) -> AppResult<Json<AsyncReplyResponse>> {
    let ReplyRequestContext {
        original,
        conversation,
        attributed_api_key_id,
        validated_bot,
    } = resolve_reply_request_context(&state, &headers, auth_user.as_ref(), &body).await?;

    // Device channels are one-way (HTTP Event Gateway, NyxID#221 / ADR-013):
    // the spec explicitly says device events have no reply surface. Refuse
    // here before any bot lookup — device conversations carry no bot token
    // and no adapter.
    if is_device_reply_forbidden(&original.platform, &conversation.platform) {
        return Err(AppError::DeviceChannelReplyNotAllowed);
    }

    let bot = match validated_bot {
        Some(bot) => bot,
        None => load_active_bot(&state, &original).await?,
    };

    // Validate reply content against the target platform's capabilities.
    // Runs after bot lookup so we can reject card-only replies destined
    // for platforms (Telegram/Discord) that would otherwise emit an empty
    // message downstream.
    validate_reply_for_platform(&body.reply, &bot.platform)?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    let bot_token = channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?;

    // Use the actual platform conversation ID from the original inbound message
    // (not the route's configured value, which may be "*" for default routes).
    let platform_conversation_id = original
        .platform_conversation_id
        .as_deref()
        .unwrap_or(&conversation.platform_conversation_id);

    // Translate the original inbound message's `thread_id` into the
    // platform-specific metadata key that the outbound adapter
    // understands. Two kinds of thread context need to flow forward:
    //
    // 1. **Discord deferred-interaction follow-up token**
    //    (`thread_id = "interaction:{app}:{token}"`). Injected as
    //    `interaction_thread_id` so `discord::send_reply()` posts to the
    //    follow-up webhook endpoint instead of `/channels/{id}/messages`.
    //
    //    **TTL guard:** Discord interaction tokens are valid for ~15 min
    //    with up to 5 follow-ups. `original.created_at` IS the real
    //    interaction timestamp, so 14 minutes leaves a 1-minute safety
    //    margin. (Device channels are guarded out above and never reach
    //    this branch.)
    //
    // 2. **Telegram forum-topic id** (numeric `message_thread_id`).
    //    Injected as `message_thread_id` so `telegram::send_reply()`
    //    passes it to Telegram's `sendMessage` and the reply stays
    //    scoped to the originating topic rather than the root chat.
    //    Topic ids do not expire, so no TTL guard is applied.
    //
    // Other platforms currently have no thread-context routing, so we
    // leave their metadata untouched.
    let mut metadata = body.reply.metadata;
    if let Some(ref tid) = original.thread_id {
        if tid.starts_with("interaction:") {
            let interaction_window = Duration::minutes(14);
            let age = Utc::now() - original.created_at;
            if age < interaction_window {
                let md = metadata.get_or_insert_with(|| serde_json::json!({}));
                if let Some(obj) = md.as_object_mut() {
                    obj.entry("interaction_thread_id")
                        .or_insert_with(|| serde_json::json!(tid));
                }
            } else {
                tracing::info!(
                    message_id = %original.id,
                    platform = %original.platform,
                    age_secs = age.num_seconds(),
                    "Skipping Discord interaction follow-up webhook: token past TTL, \
                     falling through to regular channel message API"
                );
            }
        } else if conversation.platform == "telegram" {
            let md = metadata.get_or_insert_with(|| serde_json::json!({}));
            if let Some(obj) = md.as_object_mut() {
                obj.entry("message_thread_id")
                    .or_insert_with(|| serde_json::json!(tid));
            }
        } else if conversation.platform == "slack" {
            // Slack threading uses the ROOT message's `ts` as `thread_ts`.
            // The inbound `thread_id` already holds the root (`thread_ts`
            // from the original event), while `reply_to_platform_message_id`
            // can be a child reply's `ts`. Surface the root explicitly so
            // the adapter doesn't anchor replies on the wrong message.
            let md = metadata.get_or_insert_with(|| serde_json::json!({}));
            if let Some(obj) = md.as_object_mut() {
                obj.entry("thread_ts")
                    .or_insert_with(|| serde_json::json!(tid));
            }
        }
    }

    let outbound = OutboundReply {
        text: body.reply.text,
        reply_to_platform_message_id: original.platform_message_id.clone(),
        metadata,
    };

    // Send reply to platform
    let platform_msg_id = adapter
        .send_reply(
            &state.http_client,
            &bot_token,
            platform_conversation_id,
            &outbound,
        )
        .await?;

    // Store outbound-message metadata only (per ADR-013). The reply text
    // is already on the wire to the platform; we do not persist it.
    let stored = channel_relay_service::store_outbound_message(
        &state.db,
        &bot.id,
        &conversation.id,
        &bot.user_id,
        &bot.platform,
        &attributed_api_key_id,
        Some(&original.id),
        platform_msg_id.as_deref(),
    )
    .await?;

    Ok(Json(AsyncReplyResponse {
        message_id: stored.id,
        platform_message_id: platform_msg_id,
    }))
}

/// POST /api/v1/channel-relay/reply/update
///
/// Edit a previously-sent asynchronous reply by its upstream platform message
/// ID. The platform edit surface is adapter-specific; unsupported platforms
/// return `edit_unsupported`.
pub async fn update_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    OptionalAuthUser(auth_user): OptionalAuthUser,
    Json(body): Json<UpdateReplyRequest>,
) -> AppResult<Json<UpdateReplyResponse>> {
    // Deliberately rate-limit before auth/DB work so unauth floods fail on one cheap hashmap check.
    check_message_edit_rate_limit(&state, &body.message_id)?;

    let EditRequestContext {
        outbound,
        conversation,
        bot,
        attributed_api_key_id,
    } = resolve_edit_request_context(&state, &headers, auth_user.as_ref(), &body).await?;

    if is_device_reply_forbidden(&outbound.platform, &conversation.platform) {
        return Err(AppError::DeviceChannelReplyNotAllowed);
    }

    validate_reply_for_platform(&body.reply, &bot.platform)?;

    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    let bot_token = channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?;
    let edit = OutboundEdit {
        text: body.reply.text,
        metadata: body.reply.metadata,
    };

    adapter
        .edit_reply(&state.http_client, &bot_token, &body.message_id, &edit)
        .await?;

    let inbound_message_id = outbound.reply_to_message_id.clone().ok_or_else(|| {
        AppError::Internal("outbound channel relay row is missing reply_to_message_id".to_string())
    })?;
    let edited_at = Utc::now();

    channel_relay_service::update_outbound_message_timestamp(&state.db, &outbound.id, edited_at)
        .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(bot.user_id.clone()),
        "channel_relay.reply.edit".to_string(),
        Some(serde_json::json!({
            "outbound_message_id": outbound.id,
            "inbound_message_id": inbound_message_id,
            "conversation_id": conversation.id,
            "api_key_id": attributed_api_key_id.clone(),
            "bot_id": bot.id,
            "platform": outbound.platform,
            "platform_message_id": body.message_id,
        })),
        None,
        header_value(&headers, "user-agent"),
        Some(attributed_api_key_id),
        None,
    );

    Ok(Json(UpdateReplyResponse {
        upstream_message_id: body.message_id,
        edited_at: edited_at.to_rfc3339(),
    }))
}

/// GET /api/v1/channel-relay/messages/{conversation_id}
///
/// List messages for a conversation. The calling agent must be the assigned
/// agent for the conversation.
pub async fn list_messages(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    Query(params): Query<ListMessagesQuery>,
) -> AppResult<Json<MessageListResponse>> {
    let caller_api_key_id = auth_user.api_key_id.as_deref().ok_or_else(|| {
        AppError::Forbidden("This endpoint requires API key authentication".to_string())
    })?;

    // Verify the conversation exists and the API key has access
    let conversation = state
        .db
        .collection::<ChannelConversation>(CONVERSATIONS)
        .find_one(doc! { "_id": &conversation_id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Conversation not found: {conversation_id}")))?;

    if conversation.agent_api_key_id != caller_api_key_id {
        return Err(AppError::Forbidden(
            "API key is not the assigned agent for this conversation".to_string(),
        ));
    }

    let per_page = params.per_page.min(100);
    let (messages, total) =
        channel_relay_service::list_messages(&state.db, &conversation_id, params.page, per_page)
            .await?;

    let items = messages.iter().map(message_to_item).collect();

    Ok(Json(MessageListResponse {
        messages: items,
        total,
        page: params.page,
        per_page,
    }))
}

/// GET /api/v1/channel-relay/resolve-sender
///
/// Resolve a platform sender to a NyxID user by checking the
/// `notification_channels` collection for matching `telegram_chat_id`.
pub async fn resolve_sender(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(params): Query<ResolveSenderQuery>,
) -> AppResult<Json<ResolveSenderResponse>> {
    if params.platform.is_empty() || params.platform_id.is_empty() {
        return Err(AppError::ValidationError(
            "platform and platform_id are required".to_string(),
        ));
    }

    // Require API-key authentication and scope lookups to the bot owner's
    // account. This prevents cross-tenant probing of notification metadata.
    let _api_key_id = auth_user.api_key_id.as_deref().ok_or_else(|| {
        AppError::Forbidden("This endpoint requires API key authentication".to_string())
    })?;
    let owner_user_id = auth_user.user_id.to_string();

    // Currently only Telegram is supported for sender resolution
    let (nyxid_user_id, linked) = match params.platform.as_str() {
        "telegram" => {
            // Parse the platform_id as an i64 chat ID
            let chat_id: i64 = params.platform_id.parse().map_err(|_| {
                AppError::ValidationError(
                    "platform_id must be a numeric Telegram chat ID".to_string(),
                )
            })?;

            // Scoped to the bot owner's account only
            let channel = state
                .db
                .collection::<NotificationChannel>(NOTIFICATION_CHANNELS)
                .find_one(doc! {
                    "user_id": &owner_user_id,
                    "telegram_chat_id": chat_id,
                })
                .await?;

            match channel {
                Some(nc) => (Some(nc.user_id), true),
                None => (None, false),
            }
        }
        _ => (None, false),
    };

    Ok(Json(ResolveSenderResponse {
        platform: params.platform,
        platform_id: params.platform_id,
        nyxid_user_id,
        linked,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, Header, encode};
    use mongodb::bson::doc;
    use uuid::Uuid;

    use crate::test_utils::{connect_test_database, test_app_state};

    struct ReplyTokenFixture {
        state: AppState,
        api_key: ApiKey,
        bot: ChannelBot,
        conversation: ChannelConversation,
        message: ChannelMessage,
        outbound_message: ChannelMessage,
    }

    fn body(text: Option<&str>, metadata: Option<serde_json::Value>) -> AsyncReplyBody {
        AsyncReplyBody {
            text: text.map(String::from),
            metadata,
        }
    }

    fn reply_request(message_id: &str) -> AsyncReplyRequest {
        AsyncReplyRequest {
            message_id: message_id.to_string(),
            reply: body(Some("hello"), None),
        }
    }

    fn update_reply_request(platform_message_id: &str) -> UpdateReplyRequest {
        UpdateReplyRequest {
            message_id: platform_message_id.to_string(),
            reply: body(Some("hello"), None),
        }
    }

    fn bearer_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .expect("authorization header"),
        );
        headers
    }

    fn api_key_auth_user(api_key: &ApiKey) -> AuthUser {
        AuthUser {
            user_id: Uuid::parse_str(&api_key.user_id).expect("valid api key user id"),
            session_id: None,
            scope: api_key.scopes.clone(),
            acting_client_id: None,
            approval_owner_user_id: None,
            auth_method: crate::mw::auth::AuthMethod::ApiKey,
            allow_all_services: api_key.allow_all_services,
            allow_all_nodes: api_key.allow_all_nodes,
            allowed_service_ids: api_key.allowed_service_ids.clone(),
            allowed_node_ids: api_key.allowed_node_ids.clone(),
            api_key_id: Some(api_key.id.clone()),
            api_key_name: Some(api_key.name.clone()),
            rate_limit_per_second: api_key.rate_limit_per_second,
            rate_limit_burst: api_key.rate_limit_burst,
        }
    }

    async fn insert_reply_token_use(
        fixture: &ReplyTokenFixture,
        claims: &jwt::RelayReplyClaims,
    ) -> usize {
        let count_before = fixture
            .state
            .db
            .collection::<ReplyTokenUse>(REPLY_TOKEN_USES)
            .count_documents(doc! {})
            .await
            .expect("count reply token uses before");

        let exp_at =
            chrono::DateTime::from_timestamp(claims.exp + REPLY_TOKEN_USE_TTL_BUFFER_SECS, 0)
                .expect("valid exp_at");
        fixture
            .state
            .db
            .collection::<ReplyTokenUse>(REPLY_TOKEN_USES)
            .insert_one(&ReplyTokenUse {
                id: claims.jti.clone(),
                exp_at,
                api_key_id: claims.api_key_id.clone(),
                conversation_id: claims.conversation_id.clone(),
                consumed_at: Utc::now(),
            })
            .await
            .expect("insert reply token use");

        count_before as usize
    }

    fn encode_reply_claims(state: &AppState, claims: &jwt::RelayReplyClaims) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(state.jwt_keys.kid.clone());
        encode(&header, claims, &state.jwt_keys.encoding).expect("encode relay reply token")
    }

    fn valid_reply_claims(fixture: &ReplyTokenFixture) -> jwt::RelayReplyClaims {
        let now = Utc::now().timestamp();
        jwt::RelayReplyClaims {
            iss: fixture.state.config.jwt_issuer.clone(),
            aud: jwt::RELAY_REPLY_AUDIENCE.to_string(),
            exp: now + fixture.state.config.jwt_relay_reply_ttl_secs,
            iat: now,
            jti: Uuid::new_v4().to_string(),
            token_type: jwt::RELAY_REPLY_TOKEN_TYPE.to_string(),
            api_key_id: fixture.api_key.id.clone(),
            conversation_id: fixture.conversation.id.clone(),
            inbound_message_id: fixture.message.id.clone(),
            platform: fixture.conversation.platform.clone(),
        }
    }

    fn valid_reply_token(fixture: &ReplyTokenFixture) -> String {
        jwt::generate_relay_reply_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &fixture.api_key.id,
            &fixture.conversation.id,
            &fixture.message.id,
            &fixture.conversation.platform,
        )
        .expect("generate relay reply token")
    }

    async fn setup_reply_token_fixture(prefix: &str) -> Option<ReplyTokenFixture> {
        let db = connect_test_database(prefix).await?;
        let state = test_app_state(db.clone());
        let now = Utc::now();
        let user_id = Uuid::new_v4().to_string();

        let api_key = ApiKey {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.clone(),
            name: "agent".to_string(),
            key_prefix: "nyxid_ag".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: "read write".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: now,
            description: None,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: Some("codex".to_string()),
            callback_url: Some("https://agent.example.com/callback".to_string()),
        };

        let bot = ChannelBot {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.clone(),
            platform: "telegram".to_string(),
            label: "Test Bot".to_string(),
            bot_token_encrypted: vec![1, 2, 3],
            platform_bot_id: "bot_123".to_string(),
            platform_bot_username: "test_bot".to_string(),
            webhook_registered: true,
            webhook_secret_hash: "secret".to_string(),
            app_id: None,
            app_secret_encrypted: None,
            lark_verification_token_encrypted: None,
            lark_encrypt_key_encrypted: None,
            public_key: None,
            status: "active".to_string(),
            is_active: true,
            created_at: now,
            updated_at: now,
        };

        let conversation = ChannelConversation {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.clone(),
            channel_bot_id: Some(bot.id.clone()),
            platform: "telegram".to_string(),
            platform_conversation_id: "chat_123".to_string(),
            platform_conversation_type: "private".to_string(),
            platform_sender_id: None,
            agent_api_key_id: api_key.id.clone(),
            default_agent: false,
            is_active: true,
            last_message_at: None,
            created_at: now,
            updated_at: now,
        };

        let message = ChannelMessage {
            id: Uuid::new_v4().to_string(),
            channel_bot_id: Some(bot.id.clone()),
            conversation_id: conversation.id.clone(),
            platform_conversation_id: Some(conversation.platform_conversation_id.clone()),
            user_id,
            direction: "inbound".to_string(),
            platform: conversation.platform.clone(),
            platform_message_id: Some("msg_123".to_string()),
            sender_platform_id: Some("user_123".to_string()),
            sender_display_name: Some("Alice".to_string()),
            content_type: "text".to_string(),
            thread_id: None,
            agent_api_key_id: Some(api_key.id.clone()),
            callback_status: Some("delivered".to_string()),
            reply_to_message_id: None,
            platform_reply_message_id: None,
            created_at: now,
            updated_at: None,
        };

        let outbound_message = ChannelMessage {
            id: Uuid::new_v4().to_string(),
            channel_bot_id: Some(bot.id.clone()),
            conversation_id: conversation.id.clone(),
            platform_conversation_id: Some(conversation.platform_conversation_id.clone()),
            user_id: api_key.user_id.clone(),
            direction: "outbound".to_string(),
            platform: conversation.platform.clone(),
            platform_message_id: Some("platform_reply_123".to_string()),
            sender_platform_id: None,
            sender_display_name: None,
            content_type: "text".to_string(),
            thread_id: None,
            agent_api_key_id: Some(api_key.id.clone()),
            callback_status: None,
            reply_to_message_id: Some(message.id.clone()),
            platform_reply_message_id: None,
            created_at: now,
            updated_at: Some(now),
        };

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(&api_key)
            .await
            .expect("insert api key");
        db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(&bot)
            .await
            .expect("insert bot");
        db.collection::<ChannelConversation>(CONVERSATIONS)
            .insert_one(&conversation)
            .await
            .expect("insert conversation");
        db.collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(&message)
            .await
            .expect("insert message");
        db.collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(&outbound_message)
            .await
            .expect("insert outbound message");

        Some(ReplyTokenFixture {
            state,
            api_key,
            bot,
            conversation,
            message,
            outbound_message,
        })
    }

    #[test]
    fn device_reply_guard_rejects_device_conversation() {
        assert!(is_device_reply_forbidden("telegram", "device"));
    }

    #[test]
    fn device_reply_guard_rejects_legacy_device_message_on_bot_conversation() {
        // Pre-split rows: ChannelMessage.platform == "device" while the
        // parent ChannelConversation is still Telegram/Discord/Lark/Feishu.
        // Without the original-platform check, those rows would still
        // dispatch replies through the bot adapter.
        for platform in ["telegram", "discord", "lark", "feishu"] {
            assert!(
                is_device_reply_forbidden("device", platform),
                "legacy device message must be blocked when conversation.platform == {platform}"
            );
        }
    }

    #[test]
    fn device_reply_guard_allows_pure_bot_flow() {
        // Same platform on both sides is the normal bot-chat case — must
        // NOT trip the guard.
        for platform in ["telegram", "discord", "lark", "feishu"] {
            assert!(
                !is_device_reply_forbidden(platform, platform),
                "bot reply on platform={platform} must be allowed"
            );
        }
    }

    #[test]
    fn card_support_matrix() {
        assert!(platform_supports_cards("lark"));
        assert!(platform_supports_cards("feishu"));
        assert!(!platform_supports_cards("telegram"));
        assert!(!platform_supports_cards("discord"));
        assert!(!platform_supports_cards("openclaw"));
        assert!(!platform_supports_cards(""));
    }

    #[test]
    fn text_only_ok_on_any_platform() {
        for platform in ["lark", "feishu", "telegram", "discord", "openclaw"] {
            assert!(
                validate_reply_for_platform(&body(Some("hello"), None), platform).is_ok(),
                "text-only should be accepted on {platform}"
            );
        }
    }

    #[test]
    fn card_only_ok_on_lark() {
        let md = serde_json::json!({ "card": { "elements": [] } });
        assert!(validate_reply_for_platform(&body(None, Some(md)), "lark").is_ok());
    }

    #[test]
    fn card_only_ok_on_feishu_with_null_text() {
        // Matches issue #306 example payload: { text: null, metadata: { card: {...} } }
        let md = serde_json::json!({ "card": { "header": {} } });
        assert!(validate_reply_for_platform(&body(None, Some(md)), "feishu").is_ok());
    }

    #[test]
    fn card_only_rejected_on_telegram() {
        let md = serde_json::json!({ "card": { "elements": [] } });
        let err = validate_reply_for_platform(&body(None, Some(md)), "telegram").unwrap_err();
        match err {
            AppError::ValidationError(msg) => {
                assert!(msg.contains("Lark/Feishu"), "unexpected message: {msg}");
                assert!(msg.contains("telegram"), "unexpected message: {msg}");
            }
            other => panic!("expected ValidationError, got {other:?}"),
        }
    }

    #[test]
    fn card_only_rejected_on_discord() {
        let md = serde_json::json!({ "card": { "elements": [] } });
        let err = validate_reply_for_platform(&body(None, Some(md)), "discord").unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn text_and_card_ok_even_on_non_card_platform() {
        // Text is present, so card is irrelevant. This shape is legal and
        // card will silently drop on platforms that don't support it.
        let md = serde_json::json!({ "card": {} });
        assert!(validate_reply_for_platform(&body(Some("hi"), Some(md)), "telegram").is_ok());
    }

    #[test]
    fn empty_text_no_card_rejected_on_any_platform() {
        for platform in ["lark", "feishu", "telegram", "discord"] {
            let err = validate_reply_for_platform(&body(Some(""), None), platform).unwrap_err();
            assert!(
                matches!(err, AppError::ValidationError(_)),
                "expected ValidationError on {platform}"
            );
        }
    }

    #[test]
    fn no_text_no_card_rejected_on_any_platform() {
        for platform in ["lark", "feishu", "telegram", "discord"] {
            let err = validate_reply_for_platform(&body(None, None), platform).unwrap_err();
            assert!(matches!(err, AppError::ValidationError(_)));
        }
    }

    #[test]
    fn metadata_without_card_rejected() {
        // Other metadata keys (e.g. thread ids injected by the handler)
        // must not count as content.
        let md = serde_json::json!({ "message_thread_id": 42 });
        let err = validate_reply_for_platform(&body(None, Some(md)), "lark").unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn reply_token_context_rejects_mismatched_message_id() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_message_mismatch").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let other_message = ChannelMessage {
            id: Uuid::new_v4().to_string(),
            platform_message_id: Some("msg_456".to_string()),
            created_at: Utc::now(),
            ..fixture.message.clone()
        };
        db.collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(&other_message)
            .await
            .unwrap();

        let token = valid_reply_token(&fixture);
        let err =
            resolve_reply_token_context(&fixture.state, &token, &reply_request(&other_message.id))
                .await
                .unwrap_err();

        assert!(matches!(err, AppError::Unauthorized(msg) if msg.contains("message_id mismatch")));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reply_token_context_rejects_mismatched_conversation_id() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_conversation_mismatch").await
        else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let token = jwt::generate_relay_reply_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &fixture.api_key.id,
            &Uuid::new_v4().to_string(),
            &fixture.message.id,
            &fixture.conversation.platform,
        )
        .unwrap();

        let err = resolve_reply_token_context(
            &fixture.state,
            &token,
            &reply_request(&fixture.message.id),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, AppError::Unauthorized(msg) if msg.contains("conversation mismatch"))
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reply_token_context_rejects_expired_token() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_expired").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let now = Utc::now().timestamp();
        // Push well past the clock-skew tolerance so the token is unambiguously expired.
        let claims = jwt::RelayReplyClaims {
            exp: now - 120,
            iat: now - 130,
            ..valid_reply_claims(&fixture)
        };
        let token = encode_reply_claims(&fixture.state, &claims);

        let err = resolve_reply_token_context(
            &fixture.state,
            &token,
            &reply_request(&fixture.message.id),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::TokenExpired));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reply_token_context_rejects_inactive_api_key() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_inactive_api_key").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        db.collection::<ApiKey>(API_KEYS)
            .update_one(
                doc! { "_id": &fixture.api_key.id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let token = valid_reply_token(&fixture);
        let err = resolve_reply_token_context(
            &fixture.state,
            &token,
            &reply_request(&fixture.message.id),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Unauthorized(msg) if msg.contains("inactive")));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reply_token_context_rejects_inactive_bot() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_inactive_bot").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! { "_id": &fixture.bot.id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let token = valid_reply_token(&fixture);
        let err = resolve_reply_token_context(
            &fixture.state,
            &token,
            &reply_request(&fixture.message.id),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::ChannelBotInactive(_)));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reply_token_context_rejects_reused_jti() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_reused_jti").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let token = valid_reply_token(&fixture);
        resolve_reply_token_context(&fixture.state, &token, &reply_request(&fixture.message.id))
            .await
            .unwrap();

        let err = resolve_reply_token_context(
            &fixture.state,
            &token,
            &reply_request(&fixture.message.id),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Unauthorized(msg) if msg.contains("already used")));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reply_token_context_rejects_platform_mismatch() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_platform_mismatch").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let token = jwt::generate_relay_reply_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &fixture.api_key.id,
            &fixture.conversation.id,
            &fixture.message.id,
            "discord",
        )
        .unwrap();

        let err = resolve_reply_token_context(
            &fixture.state,
            &token,
            &reply_request(&fixture.message.id),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Unauthorized(msg) if msg.contains("platform mismatch")));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reply_token_context_rejects_device_conversation() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_device_guard").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        db.collection::<ChannelConversation>(CONVERSATIONS)
            .update_one(
                doc! { "_id": &fixture.conversation.id },
                doc! { "$set": { "platform": "device" } },
            )
            .await
            .unwrap();

        let token = jwt::generate_relay_reply_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &fixture.api_key.id,
            &fixture.conversation.id,
            &fixture.message.id,
            "device",
        )
        .unwrap();

        let err = resolve_reply_token_context(
            &fixture.state,
            &token,
            &reply_request(&fixture.message.id),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::DeviceChannelReplyNotAllowed));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn reply_token_context_happy_path_consumes_jti() {
        let Some(fixture) = setup_reply_token_fixture("reply_token_happy_path").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let token = valid_reply_token(&fixture);
        let ctx = resolve_reply_token_context(
            &fixture.state,
            &token,
            &reply_request(&fixture.message.id),
        )
        .await
        .unwrap();

        assert_eq!(ctx.original.id, fixture.message.id);
        assert_eq!(ctx.conversation.id, fixture.conversation.id);
        assert_eq!(ctx.attributed_api_key_id, fixture.api_key.id);
        assert_eq!(
            ctx.validated_bot.as_ref().map(|bot| bot.id.as_str()),
            Some(fixture.bot.id.as_str())
        );

        let consumed = db
            .collection::<ReplyTokenUse>(REPLY_TOKEN_USES)
            .count_documents(doc! { "api_key_id": &fixture.api_key.id, "conversation_id": &fixture.conversation.id })
            .await
            .unwrap();
        assert_eq!(consumed, 1);

        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn update_reply_token_context_happy_path_no_jti_consumption() {
        let Some(fixture) = setup_reply_token_fixture("update_reply_token_happy_path").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let claims = valid_reply_claims(&fixture);
        let token = encode_reply_claims(&fixture.state, &claims);
        let count_before = insert_reply_token_use(&fixture, &claims).await;

        let ctx = resolve_edit_request_context(
            &fixture.state,
            &bearer_headers(&token),
            None,
            &update_reply_request(
                fixture
                    .outbound_message
                    .platform_message_id
                    .as_deref()
                    .expect("outbound platform message id"),
            ),
        )
        .await
        .unwrap();

        assert_eq!(ctx.outbound.id, fixture.outbound_message.id);
        assert_eq!(ctx.conversation.id, fixture.conversation.id);
        assert_eq!(ctx.bot.id, fixture.bot.id);
        assert_eq!(ctx.attributed_api_key_id, fixture.api_key.id);

        let count_after = db
            .collection::<ReplyTokenUse>(REPLY_TOKEN_USES)
            .count_documents(doc! {})
            .await
            .unwrap();
        assert_eq!(count_after as usize, count_before + 1);

        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn update_reply_token_context_rejects_unused_jti() {
        let Some(fixture) = setup_reply_token_fixture("update_reply_token_unused_jti").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let claims = valid_reply_claims(&fixture);
        let token = encode_reply_claims(&fixture.state, &claims);
        let err = resolve_edit_request_context(
            &fixture.state,
            &bearer_headers(&token),
            None,
            &update_reply_request(
                fixture
                    .outbound_message
                    .platform_message_id
                    .as_deref()
                    .expect("outbound platform message id"),
            ),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, AppError::Unauthorized(msg) if msg.contains("must be used to send before it can edit"))
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn update_reply_rejects_mismatched_inbound_id() {
        let Some(fixture) = setup_reply_token_fixture("update_reply_inbound_mismatch").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let claims = jwt::RelayReplyClaims {
            inbound_message_id: Uuid::new_v4().to_string(),
            ..valid_reply_claims(&fixture)
        };
        let token = encode_reply_claims(&fixture.state, &claims);
        insert_reply_token_use(&fixture, &claims).await;

        let err = resolve_edit_request_context(
            &fixture.state,
            &bearer_headers(&token),
            None,
            &update_reply_request(
                fixture
                    .outbound_message
                    .platform_message_id
                    .as_deref()
                    .expect("outbound platform message id"),
            ),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, AppError::Unauthorized(msg) if msg.contains("inbound_message_id mismatch"))
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn update_reply_rejects_device_conversation() {
        let Some(fixture) = setup_reply_token_fixture("update_reply_device_guard").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        db.collection::<ChannelConversation>(CONVERSATIONS)
            .update_one(
                doc! { "_id": &fixture.conversation.id },
                doc! { "$set": { "platform": "device" } },
            )
            .await
            .unwrap();

        let err = update_reply(
            State(fixture.state.clone()),
            HeaderMap::new(),
            OptionalAuthUser(Some(api_key_auth_user(&fixture.api_key))),
            Json(update_reply_request(
                fixture
                    .outbound_message
                    .platform_message_id
                    .as_deref()
                    .expect("outbound platform message id"),
            )),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::DeviceChannelReplyNotAllowed));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn update_reply_returns_501_on_telegram() {
        let cache = std::sync::Arc::new(
            crate::services::provider_token_exchange_service::TokenExchangeCache::new(),
        );
        let adapter = resolve_adapter("telegram", &cache).expect("telegram adapter");

        let err = adapter
            .edit_reply(
                &reqwest::Client::new(),
                "bot-token",
                "platform-msg-id",
                &OutboundEdit {
                    text: Some("hello".to_string()),
                    metadata: None,
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::ChannelPlatformEditUnsupported));
    }

    #[tokio::test]
    async fn update_reply_returns_404_on_unknown_platform_message_id() {
        let Some(fixture) = setup_reply_token_fixture("update_reply_unknown_msg").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();
        let auth_user = api_key_auth_user(&fixture.api_key);

        let err = resolve_edit_request_context(
            &fixture.state,
            &HeaderMap::new(),
            Some(&auth_user),
            &update_reply_request("missing_platform_message"),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, AppError::NotFound(msg) if msg.contains("Outbound message not found"))
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn update_reply_rate_limits_per_platform_message_id() {
        let Some(fixture) = setup_reply_token_fixture("update_reply_rate_limit").await else {
            eprintln!("skipping channel_relay reply-token test: no local MongoDB available");
            return;
        };
        let db = fixture.state.db.clone();

        let mut state = fixture.state.clone();
        state.per_message_edit_limiter =
            std::sync::Arc::new(crate::mw::rate_limit::PerMessageEditRateLimiter::new(1, 1));

        let platform_message_id = fixture
            .outbound_message
            .platform_message_id
            .as_deref()
            .expect("outbound platform message id")
            .to_string();

        let first = update_reply(
            State(state.clone()),
            HeaderMap::new(),
            OptionalAuthUser(Some(api_key_auth_user(&fixture.api_key))),
            Json(update_reply_request(&platform_message_id)),
        )
        .await;
        assert!(matches!(
            first,
            Err(AppError::ChannelPlatformEditUnsupported)
        ));

        let second = update_reply(
            State(state),
            HeaderMap::new(),
            OptionalAuthUser(Some(api_key_auth_user(&fixture.api_key))),
            Json(update_reply_request(&platform_message_id)),
        )
        .await;
        assert!(matches!(second, Err(AppError::RateLimited)));

        db.drop().await.unwrap();
    }
}
