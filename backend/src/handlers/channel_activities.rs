use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use bson::doc;
use serde::{Deserialize, Serialize};

use crate::models::{
    channel_conversation::{COLLECTION_NAME as ROUTES, ChannelConversation},
    channel_message::ChannelMessage,
};
use crate::services::{
    channel_activity_callback_service as callbacks, channel_activity_service as activities,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthMethod, AuthUser},
};

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub kind: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ActivityItem {
    pub id: String,
    pub conversation_id: String,
    pub platform_conversation_id: Option<String>,
    pub platform_event_id: Option<String>,
    pub sender_platform_id: Option<String>,
    pub kind: String,
    pub provider_event_type: Option<String>,
    pub content_availability: String,
    pub reply_supported: Option<bool>,
    pub callback_status: Option<String>,
    pub received_at: String,
    pub occurred_at: Option<String>,
}

impl From<ChannelMessage> for ActivityItem {
    fn from(message: ChannelMessage) -> Self {
        let activity = message.activity;
        Self {
            id: message.id,
            conversation_id: message.conversation_id,
            platform_conversation_id: message.platform_conversation_id,
            platform_event_id: message.platform_message_id,
            sender_platform_id: message.sender_platform_id,
            kind: activity
                .as_ref()
                .map_or("message", |value| value.kind.as_str())
                .into(),
            provider_event_type: activity
                .as_ref()
                .map(|value| value.provider_event_type.clone()),
            content_availability: activity
                .as_ref()
                .map_or("unknown", |value| value.content_availability.as_str())
                .into(),
            reply_supported: activity.as_ref().map(|value| value.reply_supported),
            callback_status: message.callback_status,
            received_at: message.created_at.to_rfc3339(),
            occurred_at: activity
                .and_then(|value| value.occurred_at)
                .map(|value| value.to_rfc3339()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RouteActivityItem {
    pub conversation_id: String,
    pub count: u64,
    pub last_activity: ActivityItem,
}

#[derive(Debug, Serialize)]
pub struct ActivityResponse {
    pub activities: Vec<ActivityItem>,
    pub total: u64,
    pub routes: Vec<RouteActivityItem>,
    pub retention_days: i64,
    pub page: u32,
    pub per_page: u32,
}

async fn list(
    state: &AppState,
    owner: &str,
    bot: Option<&str>,
    route: Option<&str>,
    query: ActivityQuery,
) -> AppResult<Json<ActivityResponse>> {
    if query
        .kind
        .as_ref()
        .is_some_and(|kind| kind.len() > 64 || kind.is_empty())
    {
        return Err(AppError::ValidationError("Invalid activity kind".into()));
    }
    let page = query.page.unwrap_or(1).clamp(1, 10_000);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);
    let result = activities::list(
        &state.db,
        owner,
        bot,
        route,
        query.kind.as_deref(),
        page,
        per_page,
    )
    .await?;
    Ok(Json(ActivityResponse {
        activities: result.activities.into_iter().map(Into::into).collect(),
        total: result.total,
        routes: result
            .routes
            .into_iter()
            .map(|value| RouteActivityItem {
                conversation_id: value.conversation_id,
                count: value.count,
                last_activity: value.last.into(),
            })
            .collect(),
        retention_days: activities::RETENTION_DAYS,
        page,
        per_page,
    }))
}

pub async fn bot_activities(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ActivityQuery>,
) -> AppResult<Json<ActivityResponse>> {
    let (owner, _) =
        super::channel_bots::resolve_bot_owner_for_read(&state, &auth.user_id.to_string(), &id)
            .await?;
    list(&state, &owner, Some(&id), None, query).await
}

pub async fn route_activities(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ActivityQuery>,
) -> AppResult<Json<ActivityResponse>> {
    let (owner, _) = super::channel_conversations::resolve_conversation_owner(
        &state,
        &auth.user_id.to_string(),
        &id,
        false,
    )
    .await?;
    list(&state, &owner, None, Some(&id), query).await
}

pub async fn callback_support(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<callbacks::CallbackSupport>> {
    let (_, route) = super::channel_conversations::resolve_conversation_owner(
        &state,
        &auth.user_id.to_string(),
        &id,
        false,
    )
    .await?;
    Ok(Json(callbacks::status(&state.db, &route).await?))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnableRequest {
    pub enabled: bool,
}

pub async fn enable_callback(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<EnableRequest>,
) -> AppResult<StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    let (_, route) = super::channel_conversations::resolve_conversation_owner(
        &state,
        &auth.user_id.to_string(),
        &id,
        true,
    )
    .await?;
    callbacks::set_enabled(&state.db, &route, body.enabled).await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_activity_callback_updated",
        Some(serde_json::json!({"conversation_id": id, "enabled": body.enabled})),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn declare_callback(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<callbacks::Declaration>,
) -> AppResult<StatusCode> {
    if auth.auth_method != AuthMethod::ApiKey {
        return Err(AppError::Forbidden(
            "Assigned agent API key required".into(),
        ));
    }
    let agent = auth
        .api_key_id
        .as_deref()
        .ok_or_else(|| AppError::Forbidden("Assigned agent API key required".into()))?;
    let route = state.db.collection::<ChannelConversation>(ROUTES)
        .find_one(doc! {"_id": &id, "user_id": auth.user_id.to_string(), "agent_api_key_id": agent, "is_active": true, "retired_by_transfer": {"$ne": true}}).await?
        .ok_or_else(|| AppError::NotFound("Assigned channel route not found".into()))?;
    let adapter = crate::services::channel_adapters::resolve_adapter(
        &route.platform,
        &state.token_exchange_cache,
    )?;
    callbacks::declare(
        &state.db,
        &id,
        &route.user_id,
        agent,
        body,
        adapter.as_ref(),
    )
    .await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_activity_callback_declared",
        Some(serde_json::json!({"conversation_id": id})),
    );
    Ok(StatusCode::NO_CONTENT)
}
