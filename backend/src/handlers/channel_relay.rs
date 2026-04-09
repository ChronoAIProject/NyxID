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

    let reply_text = body.reply.text.as_deref().unwrap_or("");
    if reply_text.is_empty() {
        return Err(AppError::ValidationError(
            "Reply text must not be empty".to_string(),
        ));
    }

    // Get the bot and verify it is still active
    let bot = channel_bot_service::get_bot(&state.db, &original.channel_bot_id).await?;
    if !bot.is_active {
        return Err(AppError::ChannelBotInactive(
            "Bot has been deactivated".to_string(),
        ));
    }
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
    //    with up to 5 follow-ups. Two different reply windows apply:
    //
    //    - **Webhook-driven original** (the usual case):
    //      `original.created_at` IS the real interaction timestamp,
    //      so 14 minutes leaves a 1-minute safety margin.
    //    - **Device-event original** (`original.platform == "device"`):
    //      the token was inherited from an older webhook inbound by
    //      `channel_event_service::lookup_recent_inbound_thread_id`,
    //      which caps source age at 2 min. The device event row's
    //      `created_at` is NOT the real interaction timestamp — use
    //      a 12-minute reply window so combined `source_age + reply_delay`
    //      stays at 14 min < 15 min TTL.
    //
    // 2. **Telegram forum-topic id** (numeric `message_thread_id`).
    //    Injected as `message_thread_id` so `telegram::send_reply()`
    //    passes it to Telegram's `sendMessage` and the reply stays
    //    scoped to the originating topic rather than the root chat.
    //    Topic ids do not expire, so no TTL guard is applied.
    //
    //    Dispatch uses **`conversation.platform`**, not
    //    `original.platform`. For webhook-driven Telegram messages they
    //    agree, but device events store `original.platform = "device"`
    //    even when the underlying bot is Telegram, so checking the
    //    conversation's platform catches both cases.
    //
    // Other platforms currently have no thread-context routing, so we
    // leave their metadata untouched.
    let mut metadata = body.reply.metadata;
    if let Some(ref tid) = original.thread_id {
        if tid.starts_with("interaction:") {
            let interaction_window = if original.platform == "device" {
                Duration::minutes(12)
            } else {
                Duration::minutes(14)
            };
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
        }
    }

    let outbound = OutboundReply {
        text: Some(reply_text.to_string()),
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
