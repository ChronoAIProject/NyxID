//! Bounded discovery for devices using an ordinary Agent Key.
use crate::services::{assistant_team_service as team, voice::client_key as keys};
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};
use axum::{Json, extract::State};
use serde_json::json;

pub async fn agents(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    agents_for(state, auth, None).await
}

pub(super) async fn agents_for(
    state: AppState,
    auth: AuthUser,
    selected_bot: Option<&str>,
) -> AppResult<Json<serde_json::Value>> {
    let key = keys::authenticate(&state.db, &auth).await?;
    let auth = match &key {
        Some(key) => {
            crate::mw::auth::api_key_auth_user(
                &state.db,
                key,
                auth.api_key_credential_id.clone(),
                None,
                None,
            )
            .await?
        }
        None => auth,
    };
    if key.is_some() {
        crate::services::voice::access::services(&state.db, &auth).await?;
    }
    let user = auth.user_id.to_string();
    crate::services::assistant_voice::require_enabled(&state.db, &user).await?;
    let mut rows = Vec::new();
    for agent in team::agents(&state.db, &user, false).await? {
        if agent.user_id != user || selected_bot.is_some_and(|id| id != agent.id) {
            continue;
        }
        let mut service_id = None;
        if let Some(key) = &key {
            match keys::covers(&state.db, key, &agent).await {
                Ok(()) => {}
                Err(crate::errors::AppError::ApiKeyScopeForbidden(_)) => continue,
                Err(error) => return Err(error),
            }
            let service =
                crate::services::voice::access::authorize(&state.db, &auth, &agent).await?;
            service_id = Some(service.id.clone());
            match crate::services::voice::access::policy(
                &state.db,
                &auth,
                &service,
                "GET",
                "/voice/agents",
                false,
            )
            .await
            {
                Ok(()) => {}
                Err(error) if keys::permanent_authority_failure(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        let thread = Box::pin(team::home_thread(&state.db, &state.encryption_keys, &agent)).await?;
        rows.push(json!({"agent_id":agent.id,"name":agent.name,
            "display_name":agent.display_name,"service_id":service_id,"conversation_id":thread.id}));
    }
    let settings = crate::services::assistant_settings_service::get(&state.db, &user).await?;
    Ok(Json(
        json!({"agents":rows,"preferences":settings.voice.map(super::Preferences::from)}),
    ))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceStart {
    #[serde(default)]
    service_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    client_request_id: String,
    preferences: super::Preferences,
    sdp_offer: String,
}

pub async fn service_start(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<ServiceStart>,
) -> AppResult<Json<super::StartResponse>> {
    keys::authenticate(&state.db, &auth).await?;
    let user = auth.user_id.to_string();
    let agent = if let Some(service) = &body.service_id {
        let identity =
            crate::services::voice::execution::resolve(&state.db, &auth, service).await?;
        if body
            .agent_id
            .as_deref()
            .is_some_and(|id| id != identity.agent.id)
        {
            return Err(crate::errors::AppError::ApiKeyScopeForbidden(
                "Bot does not match the selected Voice connection".into(),
            ));
        }
        identity.agent
    } else {
        let agent = team::agent(
            &state.db,
            &user,
            body.agent_id.as_deref().ok_or_else(|| {
                crate::errors::AppError::ValidationError("Select a Voice connection".into())
            })?,
        )
        .await?;
        if agent.user_id != user || agent.destroyed_at.is_some() {
            return Err(crate::errors::AppError::Forbidden(
                "Select a personal Bot you own".into(),
            ));
        }
        if auth.auth_method == crate::mw::auth::AuthMethod::ApiKey {
            let service =
                crate::services::voice::access::authorize(&state.db, &auth, &agent).await?;
            crate::services::voice::execution::resolve(&state.db, &auth, &service.id)
                .await?
                .agent
        } else {
            agent
        }
    };
    let thread = Box::pin(team::home_thread(&state.db, &state.encryption_keys, &agent)).await?;
    super::start(
        State(state),
        auth,
        axum::extract::Path(thread.id),
        headers,
        Json(super::StartRequest {
            client_request_id: body.client_request_id,
            preferences: body.preferences,
            sdp_offer: body.sdp_offer,
        }),
    )
    .await
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
    super::session_status(
        State(state),
        auth,
        axum::extract::Path((row.conversation_id, sid)),
    )
    .await
}

pub async fn service_control(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: Json<super::ControlRequest>,
) -> AppResult<Json<super::SessionResponse>> {
    let row = call(&state, &auth, &sid).await?;
    super::control(
        State(state),
        auth,
        axum::extract::Path((row.conversation_id, sid)),
        headers,
        body,
    )
    .await
}

pub async fn service_announcement(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    body: Json<super::AnnouncementRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let row = call(&state, &auth, &sid).await?;
    super::announcement(
        State(state),
        auth,
        axum::extract::Path((row.conversation_id, sid)),
        headers,
        body,
    )
    .await
}

pub async fn service_stream(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(sid): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    upgrade: axum::extract::WebSocketUpgrade,
) -> AppResult<axum::response::Response> {
    let row = call(&state, &auth, &sid).await?;
    super::stream(
        State(state),
        auth,
        axum::extract::Path((row.conversation_id, sid)),
        headers,
        upgrade,
    )
    .await
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
