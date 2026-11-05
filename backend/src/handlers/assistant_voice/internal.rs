//! HTTP/WebSocket entry for a published Bot Voice connection. The service
//! binding selects the Bot; the existing Voice runtime owns calls and billing.
use super::*;
use crate::services::voice::{application as app, execution};
use axum::{
    body::Body,
    extract::{FromRequestParts, Path, State},
    http::Request,
    response::{IntoResponse, Response},
};

pub(crate) async fn service_root(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Response> {
    auth.ensure_rest_proxy_access()?;
    crate::mw::rate_limit::check_agent_rate_limit(&state.per_agent_limiter, &auth).await?;
    execution::resolve(&state.db, &auth, &service_id).await?;
    Err(AppError::NotFound(
        "Select a Voice service operation".into(),
    ))
}

pub(crate) async fn service_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((service_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    auth.ensure_rest_proxy_access()?;
    crate::mw::rate_limit::check_agent_rate_limit(&state.per_agent_limiter, &auth).await?;
    let method = request.method().clone();
    let query = request.uri().query().map(str::to_owned);
    let headers = request.headers().clone();
    let (parts, body) = request.into_parts();
    let bytes = crate::handlers::body_limit::read_body(body, 128 * 1024, "Voice service").await?;
    // Preserve the original request parts, including Hyper's upgrade handle,
    // while bounding the HTTP body for both ordinary and upgrade requests.
    let request = Request::from_parts(parts, Body::from(bytes.clone()));
    dispatch(
        &state,
        &auth,
        &service_id,
        method.as_str(),
        &path,
        query.as_deref(),
        &bytes,
        headers,
        Some(request),
    )
    .await
}

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
    let caller = crate::services::voice::client_key::caller(&state.db, auth).await?;
    let auth = &caller.auth;
    let identity = execution::resolve_for_caller(&state.db, auth, service_id).await?;
    let operation_path = format!("/{}", path.trim_start_matches('/'));
    let is_websocket = headers
        .get(axum::http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    crate::services::voice::access::policy(
        &state.db,
        auth,
        &identity.service,
        method,
        &operation_path,
        is_websocket,
    )
    .await?;
    // Only this boundary has the caller's actual inputs. Application and
    // runtime rechecks must not substitute an empty request for them.
    let canonical =
        crate::services::proxy_authorization::CanonicalPath::from_mcp_literal(&operation_path)?;
    crate::services::agent_operation_scope_service::check_inputs(
        &auth.assistant_operation_scopes,
        &identity.service.id,
        identity.service.catalog_service_id.as_deref(),
        None,
        method,
        &canonical,
        query,
        bytes,
        headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
    )?;
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
