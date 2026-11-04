//! HTTP projection for NyxID's internal Voice service transport. The ordinary
//! proxy still owns service admission, concurrency, metering and attribution.
use super::*;
use crate::services::voice::{application as app, execution};
use axum::{
    body::Body,
    extract::FromRequestParts,
    http::Request,
    response::{IntoResponse, Response},
};

fn body<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> AppResult<T> {
    serde_json::from_slice(bytes)
        .map_err(|_| AppError::ValidationError("Invalid Voice operation body".into()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch(
    state: &AppState,
    auth: &AuthUser,
    service_id: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    bytes: &[u8],
    headers: axum::http::HeaderMap,
    websocket: Option<Request<Body>>,
) -> AppResult<Response> {
    let identity = execution::resolve(&state.db, auth, service_id).await?;
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();

    match (method, parts.as_slice()) {
        ("GET", ["voice", "agents"]) => {
            Ok(Json(app::agents(state, auth, Some(&identity.agent.id)).await?).into_response())
        }
        ("GET", ["voice", "options"]) => {
            let selected = query.and_then(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .find(|(key, _)| key == "conversation_id")
                    .map(|(_, value)| value.into_owned())
            });
            let conversation = match selected {
                Some(id) => {
                    execution::conversation(&state.db, &identity, &id).await?;
                    id
                }
                None => {
                    crate::services::assistant_team_service::home_thread(
                        &state.db,
                        &state.encryption_keys,
                        &identity.agent,
                    )
                    .await?
                    .id
                }
            };
            Ok(Json(
                app::options(
                    state,
                    auth,
                    OptionsQuery {
                        conversation_id: Some(conversation),
                    },
                )
                .await?,
            )
            .into_response())
        }
        ("POST", ["voice", "sessions"]) => {
            let mut input: app::ServiceStart = body(bytes)?;
            if input
                .service_id
                .as_deref()
                .is_some_and(|id| id != service_id)
            {
                return Err(AppError::ApiKeyScopeForbidden(
                    "Voice connection does not match proxy target".into(),
                ));
            }
            input.service_id = Some(service_id.into());
            client_key::start_origin(state, auth, &headers).await?;
            Ok(Json(app::service_start(state, auth, input).await?).into_response())
        }
        ("GET", ["voice", "sessions", sid]) => {
            let call = execution::call(&state.db, &identity, sid).await?;
            Ok(
                Json(app::session_status(state, auth, call.conversation_id, (*sid).into()).await?)
                    .into_response(),
            )
        }
        ("POST", ["voice", "sessions", sid, "control"]) => {
            let call = execution::call(&state.db, &identity, sid).await?;
            client_key::origin(state, auth, &headers)?;
            Ok(Json(
                app::control(
                    state,
                    auth,
                    call.conversation_id,
                    (*sid).into(),
                    body(bytes)?,
                )
                .await?,
            )
            .into_response())
        }
        ("POST", ["voice", "sessions", sid, "announcements"]) => {
            let call = execution::call(&state.db, &identity, sid).await?;
            client_key::origin(state, auth, &headers)?;
            Ok(Json(
                app::announcement(
                    state,
                    auth,
                    call.conversation_id,
                    (*sid).into(),
                    body(bytes)?,
                )
                .await?,
            )
            .into_response())
        }
        ("GET", ["voice", "sessions", sid, "stream"]) => {
            let call = execution::call(&state.db, &identity, sid).await?;
            let (mut parts, _) = websocket
                .ok_or_else(|| {
                    AppError::BadRequest("Voice stream requires WebSocket upgrade".into())
                })?
                .into_parts();
            let upgrade = axum::extract::WebSocketUpgrade::from_request_parts(&mut parts, &())
                .await
                .map_err(|_| AppError::BadRequest("Invalid Voice WebSocket upgrade".into()))?;
            socket::open(
                state.clone(),
                auth.clone(),
                call.conversation_id,
                (*sid).into(),
                headers,
                upgrade,
            )
            .await
        }
        ("GET", ["conversations", id, "voice-sessions", "active"]) => {
            execution::conversation(&state.db, &identity, id).await?;
            Ok(Json(app::active_session(state, auth, (*id).into()).await?).into_response())
        }
        ("GET", ["conversations", id, "voice-requests", rid]) => {
            execution::conversation(&state.db, &identity, id).await?;
            Ok(Json(app::request(state, auth, (*id).into(), (*rid).into()).await?).into_response())
        }
        ("POST", ["conversations", id, "voice-requests", rid, "cancel"]) => {
            execution::conversation(&state.db, &identity, id).await?;
            let cancelled = app::cancel(state, auth, (*id).into(), (*rid).into()).await?;
            for conversation in cancelled {
                crate::handlers::mcp_transport::async_operations::drain_cancellations(
                    state,
                    &auth.user_id.to_string(),
                    &conversation,
                )
                .await?;
            }
            Ok(axum::http::StatusCode::NO_CONTENT.into_response())
        }
        _ => Err(AppError::NotFound("Unknown Voice service operation".into())),
    }
}
