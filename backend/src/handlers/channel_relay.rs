//! Agent-facing channel relay endpoints (API-key authenticated).
//!
//! These endpoints allow agents to send asynchronous replies to platform
//! conversations, list conversation message history, and resolve platform
//! senders to NyxID users.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{Duration, Utc};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::channel_bots::resolve_adapter;
use crate::models::channel_conversation::{COLLECTION_NAME as CONVERSATIONS, ChannelConversation};
use crate::models::notification_channel::{
    COLLECTION_NAME as NOTIFICATION_CHANNELS, NotificationChannel,
};
use crate::mw::auth::AuthUser;
use crate::services::{
    channel_bot_service, channel_platform::OutboundReply, channel_relay_service,
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

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/channel-relay/reply
///
/// Send an asynchronous reply to a platform conversation. The agent identifies
/// the original inbound message by `message_id` and provides the reply text.
pub async fn async_reply(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AsyncReplyRequest>,
) -> AppResult<Json<AsyncReplyResponse>> {
    // Look up the original inbound message
    let original = channel_relay_service::get_message(&state.db, &body.message_id).await?;

    // Verify the conversation exists, is active, and the agent API key matches
    let conversation = state
        .db
        .collection::<ChannelConversation>(CONVERSATIONS)
        .find_one(doc! { "_id": &original.conversation_id, "is_active": true })
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Conversation not found or inactive: {}",
                original.conversation_id
            ))
        })?;

    // Ensure the calling API key is the agent assigned to this conversation
    let caller_api_key_id = auth_user.api_key_id.as_deref().ok_or_else(|| {
        AppError::Forbidden("This endpoint requires API key authentication".to_string())
    })?;

    if conversation.agent_api_key_id != caller_api_key_id {
        return Err(AppError::Forbidden(
            "API key is not the assigned agent for this conversation".to_string(),
        ));
    }

    // Device channels are one-way (HTTP Event Gateway, NyxID#221 / ADR-013):
    // the spec explicitly says device events have no reply surface. Refuse
    // here before any bot lookup — device conversations carry no bot token
    // and no adapter.
    if is_device_reply_forbidden(&original.platform, &conversation.platform) {
        return Err(AppError::DeviceChannelReplyNotAllowed);
    }

    // Get the bot and verify it is still active. `channel_bot_id` is always
    // present on non-device conversations; the guard above ensures that.
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
        caller_api_key_id,
        Some(&original.id),
        platform_msg_id.as_deref(),
    )
    .await?;

    Ok(Json(AsyncReplyResponse {
        message_id: stored.id,
        platform_message_id: platform_msg_id,
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

    fn body(text: Option<&str>, metadata: Option<serde_json::Value>) -> AsyncReplyBody {
        AsyncReplyBody {
            text: text.map(String::from),
            metadata,
        }
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
}
