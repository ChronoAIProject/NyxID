//! Voice application operations shared by native HTTP and internal service
//! adapters. Identity and service authorization are enforced here; transports
//! only extract input, validate their origin and encode the result.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_voice::{VoiceInputMode, VoiceKeySource, VoicePreferences},
    mw::auth::AuthUser,
    services::assistant_voice as voice,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub service_id: String,
    pub connection_id: Option<String>,
    pub key_source: VoiceKeySource,
    pub model: String,
    pub voice: Option<String>,
    #[serde(default)]
    pub input_mode: VoiceInputMode,
    pub language: Option<String>,
    #[serde(default)]
    pub notify_on_completion: bool,
}
impl From<VoicePreferences> for Preferences {
    fn from(v: VoicePreferences) -> Self {
        Self {
            service_id: v.service_id,
            connection_id: v.connection_id,
            key_source: v.key_source,
            model: v.model,
            voice: v.voice,
            input_mode: v.input_mode,
            language: v.language,
            notify_on_completion: v.notify_on_completion,
        }
    }
}
impl From<Preferences> for VoicePreferences {
    fn from(v: Preferences) -> Self {
        Self {
            service_id: v.service_id,
            connection_id: v.connection_id,
            key_source: v.key_source,
            model: v.model,
            voice: v.voice,
            input_mode: v.input_mode,
            language: v.language,
            notify_on_completion: v.notify_on_completion,
        }
    }
}
pub fn preference_patch<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<Preferences>>, D::Error> {
    Option::<Preferences>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionsQuery {
    pub(crate) conversation_id: Option<String>,
}

pub async fn options(
    state: &AppState,
    auth: &AuthUser,
    q: OptionsQuery,
) -> AppResult<serde_json::Value> {
    if let Some(id) = &q.conversation_id {
        crate::services::voice::client_key::operation(
            &state.db,
            auth,
            id,
            "GET",
            "/voice/options",
            false,
        )
        .await?;
    } else if crate::services::voice::client_key::authenticate(&state.db, auth)
        .await?
        .is_some()
    {
        return Err(AppError::ValidationError(
            "An Agent Key must select a voice conversation".into(),
        ));
    }
    let user = auth.user_id.to_string();
    voice::require_enabled(&state.db, &user).await?;
    let options = if let Some(id) = q.conversation_id {
        let thread = voice::thread(&state.db, &user, &id).await?;
        voice::options(&state.db, &state.encryption_keys, &thread).await?
    } else {
        voice::settings_options(&state.db, &state.encryption_keys, &user).await?
    };
    Ok(serde_json::json!({"options":options,"limits": {
        "call_seconds":crate::models::assistant_voice::MAX_CALL_SECONDS,"idle_seconds":crate::models::assistant_voice::IDLE_SECONDS,"idle_warning_seconds":crate::models::assistant_voice::IDLE_WARNING_SECONDS,
        "transmitting_calls":1,"queued_requests":crate::models::assistant_voice::MAX_QUEUED},"recording":false}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub client_request_id: String,
    pub preferences: Preferences,
    #[serde(default)]
    pub sdp_offer: String,
}
#[derive(Serialize)]
pub struct StartResponse {
    pub(crate) session: SessionResponse,
    pub(crate) sdp_answer: String,
}
pub async fn start(
    state: &AppState,
    auth: &AuthUser,
    id: String,
    body: StartRequest,
) -> AppResult<StartResponse> {
    let caller = match keys::caller(&state.db, auth).await {
        Ok(caller) => caller,
        Err(error) => {
            return crate::services::voice::diagnostics::finish(
                &state.db,
                &auth.user_id.to_string(),
                Err(crate::errors::voice_start::Stage::Origin.error(error)),
                crate::errors::voice_start::Stage::Transport,
            )
            .await;
        }
    };
    start_for_caller(state, &caller, id, body).await
}

async fn start_for_caller(
    state: &AppState,
    caller: &keys::Caller,
    id: String,
    body: StartRequest,
) -> AppResult<StartResponse> {
    let auth = &caller.auth;
    use crate::errors::voice_start::Stage;
    let result = async {
        let proof = keys::authorize_for_caller(&state.db, caller, &id)
            .await
            .map_err(|e| Stage::Origin.error(e))?;
        let started = Box::pin(crate::services::voice::runtime::start_for_client_key(
            state,
            &auth.user_id.to_string(),
            &id,
            &body.client_request_id,
            body.preferences.into(),
            &body.sdp_offer,
            auth.execution_enrolled && caller.key.is_none(),
            proof,
        ))
        .await?;
        Ok(StartResponse {
            session: started.session.into(),
            sdp_answer: started.sdp,
        })
    }
    .await;
    crate::services::voice::diagnostics::finish(
        &state.db,
        &auth.user_id.to_string(),
        result,
        Stage::Transport,
    )
    .await
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub(crate) created_at: chrono::DateTime<chrono::Utc>,
    pub(crate) closed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) id: String,
    pub(crate) generation: i64,
    pub(crate) state: crate::models::assistant_voice_session::SessionState,
    pub(crate) muted: bool,
    pub(crate) input_muted: bool,
    pub(crate) end_requested: bool,
    pub(crate) control_revision: i64,
    pub(crate) observed_seconds: i64,
    pub(crate) reserved_until: i64,
    pub(crate) final_usage_confirmed: bool,
    pub(crate) end_reason: Option<String>,
    pub(crate) idle_warning: bool,
    pub(crate) resumable: bool,
}
impl From<crate::models::assistant_voice_session::VoiceSession> for SessionResponse {
    fn from(row: crate::models::assistant_voice_session::VoiceSession) -> Self {
        let idle = crate::services::voice::progress::idle_seconds(&row, chrono::Utc::now());
        Self {
            id: row.id,
            generation: row.generation,
            created_at: row.created_at,
            closed_at: row.closed_at,
            state: row.state,
            muted: row.desired_muted,
            input_muted: row.input_muted,
            end_requested: row.end_requested,
            control_revision: row.control_revision,
            observed_seconds: row.observed_seconds,
            reserved_until: row.reserved_until,
            final_usage_confirmed: row.final_usage_confirmed,
            end_reason: row.end_reason,
            idle_warning: idle >= 165,
            resumable: !row.dictation
                && row.protocol
                    != Some(crate::models::downstream_service::VoiceProtocol::XaiRealtime)
                && row.provider_session_id.is_some()
                && row.live_slot,
        }
    }
}

#[derive(Serialize)]
pub struct ActiveSessionResponse {
    pub(crate) session: Option<SessionResponse>,
}

pub async fn active_session(
    state: &AppState,
    auth: &AuthUser,
    id: String,
) -> AppResult<ActiveSessionResponse> {
    let binding = keys::operation_binding(
        &state.db,
        auth,
        &id,
        "GET",
        &format!("/conversations/{id}/voice-sessions/active"),
        false,
    )
    .await?;
    let user = auth.user_id.to_string();
    let mut session =
        crate::services::voice::session::active_for_conversation(&state.db, &user, &id).await?;
    if binding.as_ref().is_some_and(|proof| {
        session
            .as_ref()
            .is_some_and(|call| call.client_key.as_ref() != Some(proof))
    }) {
        session = None;
    }
    Ok(ActiveSessionResponse {
        session: session.map(Into::into),
    })
}
pub async fn session_status(
    state: &AppState,
    auth: &AuthUser,
    id: String,
    sid: String,
) -> AppResult<SessionResponse> {
    let call =
        crate::services::voice::session::get(&state.db, &auth.user_id.to_string(), &id, &sid)
            .await?;
    keys::access_call(
        &state.db,
        auth,
        &call,
        "GET",
        &format!("/voice/sessions/{sid}"),
        false,
    )
    .await?;
    Ok(call.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub(crate) command_id: String,
    pub(crate) expected_revision: i64,
    pub(crate) action: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnouncementRequest {
    pub(crate) request_id: String,
    pub(crate) text: String,
}
pub async fn announcement(
    state: &AppState,
    auth: &AuthUser,
    id: String,
    sid: String,
    body: AnnouncementRequest,
) -> AppResult<serde_json::Value> {
    let call =
        crate::services::voice::session::get(&state.db, &auth.user_id.to_string(), &id, &sid)
            .await?;
    keys::access_call(
        &state.db,
        auth,
        &call,
        "POST",
        &format!("/voice/sessions/{sid}/announcements"),
        false,
    )
    .await?;
    let row = crate::services::voice::announcements::enqueue(
        &state.db,
        &call,
        &body.request_id,
        &body.text,
    )
    .await?;
    Ok(serde_json::json!({"request_id":row.request_id,"state":row.state}))
}
pub async fn control(
    state: &AppState,
    auth: &AuthUser,
    id: String,
    sid: String,
    body: ControlRequest,
) -> AppResult<SessionResponse> {
    let call =
        crate::services::voice::session::get(&state.db, &auth.user_id.to_string(), &id, &sid)
            .await?;
    keys::access_call(
        &state.db,
        auth,
        &call,
        "POST",
        &format!("/voice/sessions/{sid}/control"),
        false,
    )
    .await?;
    if call.dictation && body.action != "end" {
        return Err(AppError::Forbidden(
            "Dictation only supports ending capture".into(),
        ));
    }
    Ok(crate::services::voice::session::control_for_caller(
        &state.db,
        &auth.user_id.to_string(),
        &id,
        &sid,
        &body.command_id,
        body.expected_revision,
        &body.action,
        auth.auth_method == crate::mw::auth::AuthMethod::ApiKey,
    )
    .await?
    .into())
}

#[derive(Serialize)]
pub struct RequestResponse {
    pub(crate) outcome: Option<&'static str>,
    pub(crate) pending_acknowledgement_ids: Vec<String>,
    pub(crate) id: String,
    pub(crate) state: crate::models::assistant_voice::RequestState,
    pub(crate) turn_id: String,
}
pub async fn request(
    state: &AppState,
    auth: &AuthUser,
    id: String,
    rid: String,
) -> AppResult<RequestResponse> {
    let user = auth.user_id.to_string();
    if auth.auth_method != crate::mw::auth::AuthMethod::ApiKey {
        crate::mw::auth::require_first_party_human(auth)?;
    }
    let row = voice::get(&state.db, &user, &id, &rid).await?;
    if auth.auth_method == crate::mw::auth::AuthMethod::ApiKey {
        let call =
            crate::services::voice::session::get(&state.db, &user, &id, &row.session_id).await?;
        keys::access_call(
            &state.db,
            auth,
            &call,
            "GET",
            &format!("/conversations/{id}/voice-requests/{rid}"),
            false,
        )
        .await?;
    }
    Ok(RequestResponse {
        outcome: voice::request_outcome(&row),
        pending_acknowledgement_ids: row.pending_acknowledgement_ids,
        id: row.id,
        state: row.state,
        turn_id: row.turn_id,
    })
}
pub async fn cancel(
    state: &AppState,
    auth: &AuthUser,
    id: String,
    rid: String,
) -> AppResult<Vec<String>> {
    let user = auth.user_id.to_string();
    if auth.auth_method != crate::mw::auth::AuthMethod::ApiKey {
        crate::mw::auth::require_first_party_human(auth)?;
    }
    let request = voice::get(&state.db, &user, &id, &rid).await?;
    if auth.auth_method == crate::mw::auth::AuthMethod::ApiKey {
        let call = crate::services::voice::session::get(&state.db, &user, &id, &request.session_id)
            .await?;
        keys::access_call(
            &state.db,
            auth,
            &call,
            "POST",
            &format!("/conversations/{id}/voice-requests/{rid}/cancel"),
            false,
        )
        .await?;
    }
    voice::cancel(&state.db, &user, &id, &rid).await?;
    let mut cancelled = vec![id];
    if let Some(task) = request.task_conversation_id {
        cancelled.push(task);
    }
    Ok(cancelled)
}

use crate::services::{assistant_team_service as team, voice::client_key as keys};
use serde_json::json;

pub async fn agents(
    state: &AppState,
    auth: &AuthUser,
    selected_bot: Option<&str>,
) -> AppResult<serde_json::Value> {
    let caller = keys::caller(&state.db, auth).await?;
    let auth = &caller.auth;
    // Resolve the authorized connection set once per discovery request.
    let connections = if caller.key.is_some() {
        Some(super::access::services(&state.db, auth).await?)
    } else {
        None
    };
    let user = auth.user_id.to_string();
    crate::services::assistant_voice::require_enabled(&state.db, &user).await?;
    let mut rows = Vec::new();
    let agents = match selected_bot {
        Some(id) => vec![team::agent(&state.db, &user, id).await?],
        None => team::agents(&state.db, &user, false).await?,
    };
    for agent in agents {
        if agent.user_id != user || selected_bot.is_some_and(|id| id != agent.id) {
            continue;
        }
        let mut service_id = None;
        if let Some(connections) = &connections {
            let Some(service) = connections
                .iter()
                .find(|s| s.source_id.as_deref() == Some(agent.id.as_str()))
            else {
                continue;
            };
            service_id = Some(service.id.clone());
            match crate::services::voice::access::policy(
                &state.db,
                auth,
                service,
                "GET",
                "/voice/agents",
                false,
            )
            .await
            {
                Ok(()) => {}
                Err(error)
                    if crate::services::voice::client_key::permanent_authority_failure(&error) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        let thread = Box::pin(team::home_thread(&state.db, &state.encryption_keys, &agent)).await?;
        rows.push(json!({"agent_id":agent.id,"name":agent.name,
            "display_name":agent.display_name,"description":agent.description,
            "service_id":service_id,"conversation_id":thread.id}));
    }
    let settings = crate::services::assistant_settings_service::get(&state.db, &user).await?;
    Ok(json!({"agents":rows,"preferences":settings.voice.map(Preferences::from)}))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceStart {
    #[serde(default)]
    pub(crate) service_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    client_request_id: String,
    preferences: Preferences,
    sdp_offer: String,
}

pub async fn service_start(
    state: &AppState,
    auth: &AuthUser,
    body: ServiceStart,
) -> AppResult<StartResponse> {
    let caller = keys::caller(&state.db, auth).await?;
    let auth = &caller.auth;
    let user = auth.user_id.to_string();
    let agent = if let Some(service) = &body.service_id {
        let identity =
            crate::services::voice::execution::resolve_for_caller(&state.db, auth, service).await?;
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
                crate::services::voice::access::authorize(&state.db, auth, &agent).await?;
            crate::services::voice::execution::resolve_for_caller(&state.db, auth, &service.id)
                .await?
                .agent
        } else {
            agent
        }
    };
    let thread = Box::pin(team::home_thread(&state.db, &state.encryption_keys, &agent)).await?;
    start_for_caller(
        state,
        &caller,
        thread.id,
        StartRequest {
            client_request_id: body.client_request_id,
            preferences: body.preferences,
            sdp_offer: body.sdp_offer,
        },
    )
    .await
}
