//! HTTP projection for NyxID's internal Voice service transport. The ordinary
//! proxy still owns service admission, concurrency, metering and attribution.
use super::*;
use crate::services::voice::execution;
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
    let state_arg = State(state.clone());
    let auth_arg = auth.clone();
    match (method, parts.as_slice()) {
        ("GET", ["voice", "agents"]) => {
            Ok(
                client_key::agents_for(state.clone(), auth.clone(), Some(&identity.agent.id))
                    .await?
                    .into_response(),
            )
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
            Ok(options(
                state_arg,
                auth_arg,
                Query(OptionsQuery {
                    conversation_id: Some(conversation),
                }),
            )
            .await?
            .into_response())
        }
        ("POST", ["voice", "sessions"]) => {
            let mut value: serde_json::Value = body(bytes)?;
            if value
                .get("service_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| id != service_id)
            {
                return Err(AppError::ApiKeyScopeForbidden(
                    "Voice connection does not match proxy target".into(),
                ));
            }
            value
                .as_object_mut()
                .ok_or_else(|| AppError::ValidationError("Invalid Voice start body".into()))?
                .insert("service_id".into(), service_id.into());
            let body = serde_json::from_value(value)
                .map_err(|_| AppError::ValidationError("Invalid Voice start body".into()))?;
            Ok(
                client_key::service_start(state_arg, auth_arg, headers, Json(body))
                    .await?
                    .into_response(),
            )
        }
        ("GET", ["voice", "sessions", sid]) => {
            execution::call(&state.db, &identity, sid).await?;
            Ok(
                client_key::service_status(state_arg, auth_arg, Path((*sid).into()))
                    .await?
                    .into_response(),
            )
        }
        ("POST", ["voice", "sessions", sid, "control"]) => {
            execution::call(&state.db, &identity, sid).await?;
            Ok(client_key::service_control(
                state_arg,
                auth_arg,
                Path((*sid).into()),
                headers,
                Json(body(bytes)?),
            )
            .await?
            .into_response())
        }
        ("POST", ["voice", "sessions", sid, "announcements"]) => {
            execution::call(&state.db, &identity, sid).await?;
            Ok(client_key::service_announcement(
                state_arg,
                auth_arg,
                Path((*sid).into()),
                headers,
                Json(body(bytes)?),
            )
            .await?
            .into_response())
        }
        ("GET", ["voice", "sessions", sid, "stream"]) => {
            execution::call(&state.db, &identity, sid).await?;
            let (mut parts, _) = websocket
                .ok_or_else(|| {
                    AppError::BadRequest("Voice stream requires WebSocket upgrade".into())
                })?
                .into_parts();
            let upgrade = axum::extract::WebSocketUpgrade::from_request_parts(&mut parts, &())
                .await
                .map_err(|_| AppError::BadRequest("Invalid Voice WebSocket upgrade".into()))?;
            client_key::service_stream(state_arg, auth_arg, Path((*sid).into()), headers, upgrade)
                .await
        }
        ("GET", ["conversations", id, "voice-sessions", "active"]) => {
            execution::conversation(&state.db, &identity, id).await?;
            Ok(active_session(state_arg, auth_arg, Path((*id).into()))
                .await?
                .into_response())
        }
        ("GET", ["conversations", id, "voice-requests", rid]) => {
            execution::conversation(&state.db, &identity, id).await?;
            Ok(
                request(state_arg, auth_arg, Path(((*id).into(), (*rid).into())))
                    .await?
                    .into_response(),
            )
        }
        ("POST", ["conversations", id, "voice-requests", rid, "cancel"]) => {
            execution::conversation(&state.db, &identity, id).await?;
            cancel(state_arg, auth_arg, Path(((*id).into(), (*rid).into()))).await?;
            Ok(axum::http::StatusCode::NO_CONTENT.into_response())
        }
        _ => Err(AppError::NotFound("Unknown Voice service operation".into())),
    }
}
