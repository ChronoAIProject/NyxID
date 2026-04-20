use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::mw::auth::AuthUser;
use crate::services::{
    audit_service, channel_bot_service, channel_relay_service, channel_routing_service, org_service,
};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateConversationRequest {
    /// Required for bot-backed conversations (`telegram`/`discord`/`lark`/
    /// `feishu`). Must be omitted (or null) when `platform == "device"`.
    #[serde(default)]
    pub channel_bot_id: Option<String>,
    /// Required for device conversations. When omitted, the platform is
    /// inferred from the bot referenced by `channel_bot_id`.
    #[serde(default)]
    pub platform: Option<String>,
    pub agent_api_key_id: String,
    #[serde(default)]
    pub platform_conversation_id: Option<String>,
    #[serde(default)]
    pub platform_conversation_type: Option<String>,
    #[serde(default)]
    pub platform_sender_id: Option<String>,
    #[serde(default)]
    pub default_agent: Option<bool>,
    /// When set, create this conversation route under the given org.
    /// The referenced `channel_bot_id` and `agent_api_key_id` must both
    /// belong to the same org. Caller must be an admin of the target org.
    #[serde(default)]
    pub target_org_id: Option<String>,
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
    /// Scope the list to an org (caller must be admin of the org).
    /// Omit to list the caller's personal conversations.
    #[serde(default)]
    pub org_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ConversationItem {
    pub id: String,
    /// `None` for device channels (platform="device").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_bot_id: Option<String>,
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

/// Resolve the effective owner id for creating a conversation. When
/// `target_org_id` is set the caller must be an admin of that org.
async fn resolve_create_owner(
    state: &AppState,
    actor: &str,
    target_org_id: Option<&str>,
) -> AppResult<String> {
    if let Some(org_id) = target_org_id {
        let access = org_service::resolve_owner_access(&state.db, actor, org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to create a conversation under it"
                    .to_string(),
            ));
        }
        Ok(org_id.to_string())
    } else {
        Ok(actor.to_string())
    }
}

/// Resolve the effective owner id for listing conversations. Admin-only
/// when `org_id` is set.
async fn resolve_list_owner(
    state: &AppState,
    actor: &str,
    org_id: Option<&str>,
) -> AppResult<String> {
    if let Some(org_id) = org_id {
        let access = org_service::resolve_owner_access(&state.db, actor, org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its conversations".to_string(),
            ));
        }
        Ok(org_id.to_string())
    } else {
        Ok(actor.to_string())
    }
}

/// Resolve the owner id for a read/write on an existing conversation.
/// Returns the conversation's `user_id` on success, gated on the caller's
/// access level via `OwnerAccess`.
async fn resolve_conversation_owner(
    state: &AppState,
    actor: &str,
    conversation_id: &str,
    require_write: bool,
) -> AppResult<(
    String,
    crate::models::channel_conversation::ChannelConversation,
)> {
    let conv = state
        .db
        .collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .find_one(doc! { "_id": conversation_id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Conversation not found: {conversation_id}")))?;
    let access = org_service::resolve_owner_access(&state.db, actor, &conv.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound(format!(
            "Conversation not found: {conversation_id}"
        )));
    }
    if require_write && !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this conversation".to_string(),
        ));
    }
    Ok((conv.user_id.clone(), conv))
}

/// Load an ApiKey by id without scoping it to a particular user_id.
/// Used to cross-check ownership during conversation creation.
async fn load_api_key_any_owner(state: &AppState, key_id: &str) -> AppResult<ApiKey> {
    state
        .db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("API key not found: {key_id}")))
}

/// Return `value` unchanged if non-empty, otherwise return `default`.
///
/// react-hook-form (and most plain HTML forms) submits `""` rather than
/// omitting the field when a text input is cleared. A plain
/// `Option::unwrap_or(default)` fires only on `None`, which means a
/// cleared-then-submitted input would overwrite the caller's intended
/// default with an empty string. This helper treats empty and missing
/// identically so the default always wins when the user didn't type
/// anything meaningful.
fn normalize_conversation_type<'a>(value: Option<&'a str>, default: &'a str) -> &'a str {
    value.filter(|s| !s.is_empty()).unwrap_or(default)
}

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
///
/// Two regimes, selected by `platform`:
///
/// * **Bot conversation** (`platform` omitted OR one of
///   `telegram`/`discord`/`lark`/`feishu`): requires `channel_bot_id`;
///   behaves as before.
/// * **Device conversation** (`platform == "device"`): requires
///   `platform_conversation_id`, rejects `channel_bot_id` and
///   `platform_sender_id`, and does not support `default_agent`. See
///   NyxID#221 / nyxid-event-gateway.md.
pub async fn create_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateConversationRequest>,
) -> AppResult<(StatusCode, Json<ConversationItem>)> {
    let actor = auth_user.user_id.to_string();

    // Resolve the effective owner. Admins of the target org when
    // target_org_id is set; the actor themselves otherwise.
    let owner_id = resolve_create_owner(&state, &actor, body.target_org_id.as_deref()).await?;

    // Cross-scope rule: the agent api key must belong to the resolved owner.
    // Otherwise a conversation could mix personal + org resources (or two
    // different orgs), which would silently cross-authorize downstream
    // services through the api key.
    let agent_key = load_api_key_any_owner(&state, &body.agent_api_key_id).await?;
    if agent_key.user_id != owner_id {
        return Err(AppError::ValidationError(
            "agent_api_key and conversation owner must match (personal key must be bound to a \
             personal conversation, org key must be bound to an org conversation under the \
             same org)"
                .to_string(),
        ));
    }

    let is_device = matches!(body.platform.as_deref(), Some("device"));

    let conversation = if is_device {
        create_device_conversation(&state, &owner_id, &body).await?
    } else {
        create_bot_conversation(&state, &owner_id, &body).await?
    };

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "channel_conversation_created".to_string(),
        Some(serde_json::json!({
            "conversation_id": &conversation.id,
            "channel_bot_id": &body.channel_bot_id,
            "platform": &conversation.platform,
            "agent_api_key_id": &body.agent_api_key_id,
            "owner_user_id": &owner_id,
            "target_org_id": body.target_org_id,
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

/// Create a bot-backed conversation (telegram/discord/lark/feishu).
async fn create_bot_conversation(
    state: &AppState,
    owner_id: &str,
    body: &CreateConversationRequest,
) -> AppResult<crate::models::channel_conversation::ChannelConversation> {
    let channel_bot_id = body.channel_bot_id.as_deref().ok_or_else(|| {
        AppError::ValidationError("channel_bot_id is required for bot conversations".to_string())
    })?;

    let bot = channel_bot_service::get_bot(&state.db, channel_bot_id).await?;
    if bot.user_id != owner_id {
        return Err(AppError::ValidationError(
            "channel_bot and conversation owner must match (personal bot must be bound to a \
             personal conversation, org bot must be bound to an org conversation under the \
             same org)"
                .to_string(),
        ));
    }

    // If the caller supplied `platform` explicitly, it must match the bot's
    // actual platform. This lets clients send `platform` redundantly for
    // symmetry with the device branch without risking a silent mismatch.
    if let Some(declared) = body.platform.as_deref()
        && declared != bot.platform
    {
        return Err(AppError::ValidationError(format!(
            "declared platform '{declared}' does not match bot platform '{}'",
            bot.platform
        )));
    }

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
    let platform_conversation_type =
        normalize_conversation_type(body.platform_conversation_type.as_deref(), "private");

    if platform_conversation_id.len() > 256 {
        return Err(AppError::ValidationError(
            "platform_conversation_id exceeds 256 characters".to_string(),
        ));
    }

    channel_routing_service::create_conversation(
        &state.db,
        owner_id,
        Some(channel_bot_id),
        &bot.platform,
        platform_conversation_id,
        platform_conversation_type,
        body.platform_sender_id.as_deref(),
        &body.agent_api_key_id,
        default_agent,
    )
    .await
}

/// Create a device-backed conversation (HTTP Event Gateway, no bot).
async fn create_device_conversation(
    state: &AppState,
    owner_id: &str,
    body: &CreateConversationRequest,
) -> AppResult<crate::models::channel_conversation::ChannelConversation> {
    if body.channel_bot_id.is_some() {
        return Err(AppError::ValidationError(
            "channel_bot_id must be omitted for device conversations".to_string(),
        ));
    }
    if body
        .platform_sender_id
        .as_deref()
        .is_some_and(|s| !s.is_empty())
    {
        return Err(AppError::ValidationError(
            "platform_sender_id is not supported on device conversations".to_string(),
        ));
    }
    if body.default_agent == Some(true) {
        return Err(AppError::ValidationError(
            "default_agent is not supported on device conversations".to_string(),
        ));
    }

    let platform_conversation_id = body
        .platform_conversation_id
        .as_deref()
        .filter(|s| !s.is_empty() && *s != "*")
        .ok_or_else(|| {
            AppError::ValidationError(
                "platform_conversation_id is required for device conversations".to_string(),
            )
        })?;

    if platform_conversation_id.len() > 256 {
        return Err(AppError::ValidationError(
            "platform_conversation_id exceeds 256 characters".to_string(),
        ));
    }

    let platform_conversation_type =
        normalize_conversation_type(body.platform_conversation_type.as_deref(), "device");

    channel_routing_service::create_conversation(
        &state.db,
        owner_id,
        None,
        "device",
        platform_conversation_id,
        platform_conversation_type,
        None,
        &body.agent_api_key_id,
        false,
    )
    .await
}

/// GET /api/v1/channel-conversations
pub async fn list_conversations(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(params): Query<ListConversationsQuery>,
) -> AppResult<Json<ConversationListResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_list_owner(&state, &actor, params.org_id.as_deref()).await?;
    let conversations =
        channel_routing_service::list_conversations(&state.db, &owner_id, params.bot_id.as_deref())
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
    let actor = auth_user.user_id.to_string();
    let (_owner_id, conversation) =
        resolve_conversation_owner(&state, &actor, &conversation_id, false).await?;
    Ok(Json(conversation_to_item(&conversation)))
}

/// PUT /api/v1/channel-conversations/{id}
pub async fn update_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    Json(body): Json<UpdateConversationRequest>,
) -> AppResult<Json<ConversationItem>> {
    let actor = auth_user.user_id.to_string();
    let (owner_id, _conv) =
        resolve_conversation_owner(&state, &actor, &conversation_id, true).await?;

    // If the caller is switching the agent_api_key_id, re-enforce the
    // cross-scope rule: the new key must belong to the same owner as
    // the conversation.
    if let Some(new_key_id) = body.agent_api_key_id.as_deref() {
        let agent_key = load_api_key_any_owner(&state, new_key_id).await?;
        if agent_key.user_id != owner_id {
            return Err(AppError::ValidationError(
                "agent_api_key and conversation owner must match".to_string(),
            ));
        }
    }

    let updated = channel_routing_service::update_conversation(
        &state.db,
        &conversation_id,
        &owner_id,
        body.agent_api_key_id.as_deref(),
        body.default_agent,
        body.is_active,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "channel_conversation_updated".to_string(),
        Some(serde_json::json!({
            "conversation_id": &conversation_id,
            "owner_user_id": &owner_id,
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
    let actor = auth_user.user_id.to_string();
    let (owner_id, _conv) =
        resolve_conversation_owner(&state, &actor, &conversation_id, true).await?;

    channel_routing_service::delete_conversation(&state.db, &conversation_id, &owner_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(actor),
        "channel_conversation_deleted".to_string(),
        Some(serde_json::json!({
            "conversation_id": &conversation_id,
            "owner_user_id": &owner_id,
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
    let actor = auth_user.user_id.to_string();

    // Resolve owner access on the conversation itself. Read-level access
    // (any active member of the owning org) is sufficient to browse
    // message metadata.
    let (_owner_id, _conversation) =
        resolve_conversation_owner(&state, &actor, &conversation_id, false).await?;

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

#[cfg(test)]
mod tests {
    use super::normalize_conversation_type;

    #[test]
    fn normalize_returns_default_when_none() {
        assert_eq!(normalize_conversation_type(None, "device"), "device");
    }

    #[test]
    fn normalize_returns_default_when_empty_string() {
        // Regression for Codex review finding: react-hook-form submits
        // "" for cleared text inputs, and `Option::unwrap_or` alone
        // would let that empty string overwrite the caller's default.
        assert_eq!(normalize_conversation_type(Some(""), "device"), "device");
        assert_eq!(normalize_conversation_type(Some(""), "private"), "private");
    }

    #[test]
    fn normalize_passes_through_non_empty_value() {
        assert_eq!(
            normalize_conversation_type(Some("camera"), "device"),
            "camera"
        );
        assert_eq!(
            normalize_conversation_type(Some("group"), "private"),
            "group"
        );
    }
}
