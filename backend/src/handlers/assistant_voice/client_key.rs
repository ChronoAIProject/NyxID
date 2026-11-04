//! Bounded discovery for devices using an ordinary Agent Key.
use crate::services::assistant_team_service as team;
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};
use axum::{Json, extract::State};
use serde_json::json;

pub async fn agents(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    crate::services::voice::application::agents(&state, &auth, None)
        .await
        .map(Json)
}

pub async fn service_start(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<crate::services::voice::application::ServiceStart>,
) -> AppResult<Json<super::StartResponse>> {
    start_origin(&state, &auth, &headers).await?;
    crate::services::voice::application::service_start(&state, &auth, body)
        .await
        .map(Json)
}

pub async fn voice_service(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(agent_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    voice_service_owner(&state, &auth, &agent_id).await?;
    let service =
        crate::services::voice::binding::get(&state.db, &auth.user_id.to_string(), &agent_id)
            .await?;
    Ok(Json(
        json!({"service":service.map(|s| json!({"id":s.id,"slug":s.slug,"agent_id":agent_id,"is_active":s.is_active}))}),
    ))
}

pub async fn publish_voice_service(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(agent_id): axum::extract::Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    voice_service_owner(&state, &auth, &agent_id).await?;
    let service =
        crate::services::voice::binding::publish(&state.db, &auth.user_id.to_string(), &agent_id)
            .await?;
    Ok(Json(
        json!({"service":{"id":service.id,"slug":service.slug,"agent_id":agent_id,"is_active":service.is_active}}),
    ))
}

async fn voice_service_owner(state: &AppState, auth: &AuthUser, agent_id: &str) -> AppResult<()> {
    crate::handlers::login_client_context::require_first_party_human(auth)?;
    let owner = auth.user_id.to_string();
    crate::services::assistant_voice::require_enabled(&state.db, &owner).await?;
    if !crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        &owner,
        crate::services::feature_flag_service::VOICE_AGENT_KEY_FLAG_KEY,
    )
    .await?
    {
        return Err(crate::errors::AppError::Forbidden(
            "Agent Key voice is not enabled".into(),
        ));
    }
    let agent = team::agent(&state.db, &owner, agent_id).await?;
    if agent.user_id != owner || agent.destroyed_at.is_some() {
        return Err(crate::errors::AppError::Forbidden(
            "Select an active personal Bot you own".into(),
        ));
    }
    Ok(())
}

async fn call(
    state: &AppState,
    auth: &AuthUser,
    sid: &str,
) -> AppResult<crate::models::assistant_voice_session::VoiceSession> {
    use mongodb::bson::doc;
    state
        .db
        .collection(crate::models::assistant_voice_session::COLLECTION_NAME)
        .find_one(doc! {"_id":sid,"user_id":auth.user_id.to_string()})
        .await?
        .ok_or_else(|| crate::errors::AppError::NotFound("Voice session not found".into()))
}

pub async fn service_status(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
) -> AppResult<Json<super::SessionResponse>> {
    let row = call(&state, &auth, &sid).await?;
    crate::services::voice::application::session_status(&state, &auth, row.conversation_id, sid)
        .await
        .map(Json)
}

pub async fn service_control(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<super::ControlRequest>,
) -> AppResult<Json<super::SessionResponse>> {
    let row = call(&state, &auth, &sid).await?;
    origin(&state, &auth, &headers)?;
    crate::services::voice::application::control(&state, &auth, row.conversation_id, sid, body)
        .await
        .map(Json)
}

pub async fn service_announcement(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<super::AnnouncementRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let row = call(&state, &auth, &sid).await?;
    origin(&state, &auth, &headers)?;
    crate::services::voice::application::announcement(&state, &auth, row.conversation_id, sid, body)
        .await
        .map(Json)
}

pub async fn service_stream(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    upgrade: axum::extract::WebSocketUpgrade,
) -> AppResult<axum::response::Response> {
    let row = call(&state, &auth, &sid).await?;
    super::socket::open(state, auth, row.conversation_id, sid, headers, upgrade).await
}

pub(super) fn origin(
    state: &AppState,
    auth: &AuthUser,
    headers: &axum::http::HeaderMap,
) -> AppResult<()> {
    if auth.auth_method != crate::mw::auth::AuthMethod::ApiKey
        || headers.contains_key(axum::http::header::ORIGIN)
    {
        super::require_origin(state, headers)?;
    }
    Ok(())
}

/// Preserve the public Voice start diagnostics while keeping HTTP Origin out
/// of the application service used by the internal proxy.
pub(super) async fn start_origin(
    state: &AppState,
    auth: &AuthUser,
    headers: &axum::http::HeaderMap,
) -> AppResult<()> {
    use crate::errors::voice_start::Stage;
    match origin(state, auth, headers) {
        Ok(()) => Ok(()),
        Err(error) => {
            crate::services::voice::diagnostics::finish(
                &state.db,
                &auth.user_id.to_string(),
                Err(Stage::Origin.error(error)),
                Stage::Transport,
            )
            .await
        }
    }
}
