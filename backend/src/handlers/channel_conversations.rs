use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{
    audit_service, channel_bot_service, channel_relay_service, channel_routing_service,
};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateConversationRequest {
    pub channel_bot_id: String,
    pub agent_api_key_id: String,
    #[serde(default)]
    pub platform_conversation_id: Option<String>,
    #[serde(default)]
    pub platform_conversation_type: Option<String>,
    #[serde(default)]
    pub platform_sender_id: Option<String>,
    #[serde(default)]
    pub default_agent: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateConversationRequest {
    #[serde(default)]
    pub agent_api_key_id: Option<String>,
    #[serde(default)]
    pub default_agent: Option<bool>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListConversationsQuery {
    pub bot_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ConversationItem {
    pub id: String,
    pub channel_bot_id: String,
    pub platform: String,
    pub platform_conversation_id: String,
    pub platform_conversation_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_sender_id: Option<String>,
    pub agent_api_key_id: String,
    pub default_agent: bool,
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ConversationListResponse {
    pub conversations: Vec<ConversationItem>,
    pub total: u64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn conversation_to_item(
    conv: &crate::models::channel_conversation::ChannelConversation,
) -> ConversationItem {
    ConversationItem {
        id: conv.id.clone(),
        channel_bot_id: conv.channel_bot_id.clone(),
        platform: conv.platform.clone(),
        platform_conversation_id: conv.platform_conversation_id.clone(),
        platform_conversation_type: conv.platform_conversation_type.clone(),
        platform_sender_id: conv.platform_sender_id.clone(),
        agent_api_key_id: conv.agent_api_key_id.clone(),
        default_agent: conv.default_agent,
        is_active: conv.is_active,
        last_message_at: conv.last_message_at.map(|dt| dt.to_rfc3339()),
        created_at: conv.created_at.to_rfc3339(),
        updated_at: conv.updated_at.to_rfc3339(),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/channel-conversations
pub async fn create_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateConversationRequest>,
) -> AppResult<(StatusCode, Json<ConversationItem>)> {
    let user_id_str = auth_user.user_id.to_string();

    // Verify the bot exists and belongs to this user
    let bot = channel_bot_service::get_bot_for_user(&state.db, &body.channel_bot_id, &user_id_str)
        .await?;

    // When no conversation ID is provided (or empty), treat as a wildcard.
    let has_conversation_id = body
        .platform_conversation_id
        .as_deref()
        .is_some_and(|s| !s.is_empty() && s != "*");
    let has_sender_id = body
        .platform_sender_id
        .as_deref()
        .is_some_and(|s| !s.is_empty());
    let platform_conversation_id = if has_conversation_id {
        body.platform_conversation_id.as_deref().unwrap()
    } else {
        "*"
    };
    // Only set default_agent=true for true catch-all routes (no conversation
    // ID AND no sender ID). Sender-specific routes should NOT become catch-all
    // even if conversation ID is omitted -- they match by sender only.
    let default_agent = if has_conversation_id || has_sender_id {
        body.default_agent.unwrap_or(false)
    } else {
        // No conversation ID and no sender ID = true catch-all
        true
    };
    let platform_conversation_type = body
        .platform_conversation_type
        .as_deref()
        .unwrap_or("private");

    if platform_conversation_id.len() > 256 {
        return Err(AppError::ValidationError(
            "platform_conversation_id exceeds 256 characters".to_string(),
        ));
    }

    let conversation = channel_routing_service::create_conversation(
        &state.db,
        &user_id_str,
        &body.channel_bot_id,
        &bot.platform,
        platform_conversation_id,
        platform_conversation_type,
        body.platform_sender_id.as_deref(),
        &body.agent_api_key_id,
        default_agent,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "channel_conversation_created".to_string(),
        Some(serde_json::json!({
            "conversation_id": &conversation.id,
            "channel_bot_id": &body.channel_bot_id,
            "agent_api_key_id": &body.agent_api_key_id,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok((
        StatusCode::CREATED,
        Json(conversation_to_item(&conversation)),
    ))
}

/// GET /api/v1/channel-conversations
pub async fn list_conversations(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(params): Query<ListConversationsQuery>,
) -> AppResult<Json<ConversationListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let conversations = channel_routing_service::list_conversations(
        &state.db,
        &user_id_str,
        params.bot_id.as_deref(),
    )
    .await?;
    let total = conversations.len() as u64;
    let items = conversations.iter().map(conversation_to_item).collect();
    Ok(Json(ConversationListResponse {
        conversations: items,
        total,
    }))
}

/// GET /api/v1/channel-conversations/{id}
pub async fn get_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
) -> AppResult<Json<ConversationItem>> {
    let user_id_str = auth_user.user_id.to_string();

    // Fetch the conversation directly from MongoDB with ownership check
    let conversation = state
        .db
        .collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {
            "_id": &conversation_id,
            "user_id": &user_id_str,
        })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Conversation not found: {conversation_id}")))?;

    Ok(Json(conversation_to_item(&conversation)))
}

/// PUT /api/v1/channel-conversations/{id}
pub async fn update_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    Json(body): Json<UpdateConversationRequest>,
) -> AppResult<Json<ConversationItem>> {
    let user_id_str = auth_user.user_id.to_string();

    let updated = channel_routing_service::update_conversation(
        &state.db,
        &conversation_id,
        &user_id_str,
        body.agent_api_key_id.as_deref(),
        body.default_agent,
        body.is_active,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "channel_conversation_updated".to_string(),
        Some(serde_json::json!({
            "conversation_id": &conversation_id,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(Json(conversation_to_item(&updated)))
}

/// DELETE /api/v1/channel-conversations/{id}
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();

    channel_routing_service::delete_conversation(&state.db, &conversation_id, &user_id_str).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "channel_conversation_deleted".to_string(),
        Some(serde_json::json!({
            "conversation_id": &conversation_id,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Conversation messages (owner-accessible)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ConversationMessagesQuery {
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

#[derive(Debug, Serialize)]
/// Metadata-only message summary for the bot-owner message-history view.
///
/// **Breaking change (ADR-013):** `text` and `attachments` used to appear
/// here. Per the NyxID pure-passthrough principle, message content is no
/// longer persisted.
pub struct ConversationMessageItem {
    pub id: String,
    pub direction: String,
    pub platform: String,
    pub platform_message_id: Option<String>,
    pub sender_platform_id: Option<String>,
    pub sender_display_name: Option<String>,
    pub content_type: String,
    pub agent_api_key_id: Option<String>,
    pub callback_status: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ConversationMessagesResponse {
    pub messages: Vec<ConversationMessageItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

/// GET /api/v1/channel-conversations/{id}/messages
///
/// List messages for a conversation. Accessible by the bot owner (session auth).
pub async fn list_conversation_messages(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    Query(params): Query<ConversationMessagesQuery>,
) -> AppResult<Json<ConversationMessagesResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Verify the conversation belongs to the user via the bot
    let conversation = state
        .db
        .collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! { "_id": &conversation_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Conversation not found".to_string()))?;

    // Verify ownership through the bot
    channel_bot_service::get_bot_for_user(&state.db, &conversation.channel_bot_id, &user_id_str)
        .await?;

    let per_page = params.per_page.min(100);
    let (messages, total) =
        channel_relay_service::list_messages(&state.db, &conversation_id, params.page, per_page)
            .await?;

    let items = messages
        .into_iter()
        .map(|m| ConversationMessageItem {
            id: m.id,
            direction: m.direction,
            platform: m.platform,
            platform_message_id: m.platform_message_id,
            sender_platform_id: m.sender_platform_id,
            sender_display_name: m.sender_display_name,
            content_type: m.content_type,
            agent_api_key_id: m.agent_api_key_id,
            callback_status: m.callback_status,
            reply_to_message_id: m.reply_to_message_id,
            created_at: m.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ConversationMessagesResponse {
        messages: items,
        total,
        page: params.page,
        per_page,
    }))
}
