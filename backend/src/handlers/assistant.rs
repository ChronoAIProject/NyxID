//! Thin NyxID pass-through for the assistant chat surface (PRD decision 4).
//!
//! The browser calls these NyxID-owned routes with its session cookie;
//! NyxID resolves the admin-managed Aevatar service and forwards the request,
//! deriving the Aevatar scope from the verified session user. The browser
//! never names a scope and never reaches Aevatar directly.
//!
//! Every route is an explicit mapping onto one Aevatar path. A blanket
//! `/{*path}` pass-through would expose all ~248 routes of Aevatar's spec
//! (admin, workflow, and streaming-proxy surfaces included) through a
//! session-authed mount; the assistant needs nine.
//!
//! Forwarding goes through `proxy::execute_proxy`, so credential injection,
//! identity propagation, per-agent rate limiting, approval gating, and audit
//! all apply unchanged -- the assistant is not a special case in the data
//! plane (PRD N4). SSE responses stream through it unbuffered.

use std::time::Duration;

use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{HeaderValue, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::crypto::jwt::generate_assistant_forward_access_token;
use crate::errors::{AppError, AppResult};
use crate::handlers::proxy::execute_proxy;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::assistant_service;
use crate::services::oauth_resource_service;

/// Create responses are a ~300-byte accepted envelope; the cap only guards
/// against a misbehaving upstream.
const MAX_CREATE_RESPONSE_BYTES: usize = 64 * 1024;
/// The actor index is bare `{actorId}` rows (~60 bytes each).
const MAX_INDEX_RESPONSE_BYTES: usize = 512 * 1024;

/// Server-initiated upstream request derived from a caller request (the
/// materialization polls, the history half of the composite delete).
///
/// Carries the caller's `Authorization` so a `forward_access_token`
/// service treats the derived call exactly like the original — without it,
/// Bearer callers would see their polls 401 and their composite delete
/// fail halfway while the row config still forwards tokens. Everything
/// else (cookies, client metadata) is intentionally absent.
fn synthetic_request(
    method: Method,
    authorization: Option<&HeaderValue>,
) -> Result<Request<Body>, axum::http::Error> {
    let mut builder = Request::builder().method(method).uri("/");
    if let Some(value) = authorization {
        builder = builder.header(header::AUTHORIZATION, value.clone());
    }
    builder.body(Body::empty())
}

/// Whether this call needs the TD-3 forward-token bridge: cookie sessions
/// carry no bearer for `forward_access_token` to forward, so Aevatar —
/// which today authenticates only `Authorization: Bearer <NyxID JWT>` —
/// answers 401 for exactly the browser. Minting is gated on the row still
/// being in Bearer-forwarding mode: when the TD-3 rollout flips
/// `forward_access_token` off (Aevatar validates the identity token
/// instead), the bridge retires itself with no code change.
fn needs_forward_token_bridge(auth_method: &AuthMethod, forward_access_token: bool) -> bool {
    *auth_method == AuthMethod::Session && forward_access_token
}

/// Resolve the admin-managed Aevatar service and forward `path` to it.
///
/// `path` is always built server-side by the callers below from
/// `auth_user.user_id`; no caller-supplied scope reaches this function.
async fn forward(
    state: &AppState,
    auth_user: &AuthUser,
    path: String,
    mut request: Request<Body>,
) -> AppResult<Response> {
    let service = assistant_service::resolve_admin_service(&state.db).await?;

    // TD-3 bridge: mint the outbound-only bearer for cookie sessions and
    // OVERWRITE any stray Authorization header — `AuthMethod::Session`
    // means bearer auth did not happen, so whatever is there is not an
    // authenticated credential and must not reach the downstream. Bearer
    // callers (CLI login JWTs) never enter this branch and keep their
    // token byte-for-byte.
    if needs_forward_token_bridge(&auth_user.auth_method, service.forward_access_token) {
        let resource_uri =
            oauth_resource_service::user_service_resource_uri(&state.config, &service.slug);
        let token = generate_assistant_forward_access_token(
            &state.jwt_keys,
            &state.config,
            &auth_user.user_id,
            &resource_uri,
        )?;
        let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
            AppError::Internal(
                "assistant: failed to build the forward authorization header".to_string(),
            )
        })?;
        request.headers_mut().insert(header::AUTHORIZATION, value);
    }
    // Addressing the catalog service by id drives the DownstreamService
    // (admin/master-credential) resolution path. Never route by slug here:
    // the slug resolver would prefer a caller-owned `UserService`.
    let mut resolved_slug = String::new();
    execute_proxy(
        state,
        auth_user,
        &service.id,
        &path,
        request,
        &mut resolved_slug,
    )
    .await
}

/// `POST /api/v1/assistant/conversations` -- create a conversation.
///
/// Create is async-accepted upstream (202 + `actorId`), and streaming into
/// an actor before it appears in the `nyxid-chat` actor index races the
/// materialization. Mirror the reference client (`waitForConversation`):
/// after a successful create, poll the actor index with bounded backoff
/// before returning, so a caller that immediately streams (the first-send
/// flow) never hits the race. Best-effort — a poll failure never fails the
/// create.
pub async fn create_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let user_id = auth_user.user_id.to_string();
    let authorization = request.headers().get(header::AUTHORIZATION).cloned();
    let path = assistant_service::conversations_path(&user_id);
    let response = forward(&state, &auth_user, path, request).await?;
    if !response.status().is_success() {
        return Ok(response);
    }
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, MAX_CREATE_RESPONSE_BYTES)
        .await
        .map_err(|_| {
            AppError::Internal(
                "assistant: conversation create response exceeded the buffer cap".to_string(),
            )
        })?;
    if let Some(actor_id) = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .as_ref()
        .and_then(assistant_service::extract_actor_id)
        .map(str::to_owned)
    {
        wait_for_conversation_materialization(
            &state,
            &auth_user,
            &user_id,
            &actor_id,
            authorization.as_ref(),
        )
        .await;
    }
    Ok(Response::from_parts(parts, Body::from(bytes)))
}

/// Poll the `nyxid-chat` actor index until it lists `actor_id`, giving up
/// after the bounded backoff schedule. Never fails: the create already
/// succeeded, and a stream that still races simply fails visibly and can be
/// retried.
async fn wait_for_conversation_materialization(
    state: &AppState,
    auth_user: &AuthUser,
    user_id: &str,
    actor_id: &str,
    authorization: Option<&HeaderValue>,
) {
    for attempt in 0..assistant_service::CREATE_POLL_ATTEMPTS {
        let Ok(index_request) = synthetic_request(Method::GET, authorization) else {
            return;
        };
        let path = assistant_service::conversations_path(user_id);
        let Ok(response) = forward(state, auth_user, path, index_request).await else {
            return;
        };
        if response.status().is_success() {
            let Ok(bytes) = to_bytes(response.into_body(), MAX_INDEX_RESPONSE_BYTES).await else {
                return;
            };
            if serde_json::from_slice::<serde_json::Value>(&bytes)
                .map(|index| assistant_service::actor_index_contains(&index, actor_id))
                .unwrap_or(false)
            {
                return;
            }
        }
        if attempt + 1 < assistant_service::CREATE_POLL_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(
                assistant_service::create_poll_delay_ms(attempt),
            ))
            .await;
        }
    }
}

/// `GET /api/v1/assistant/conversations` -- list the caller's conversations
/// from the Chat History index (server titles, timestamps, message counts).
///
/// This reads a different upstream family than `create_conversation` above:
/// creation targets the `nyxid-chat` actor, while the list is the materialized
/// history read model, so a freshly created conversation appears here only
/// after its first completed turn.
pub async fn list_conversations(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let path = assistant_service::history_index_path(&auth_user.user_id.to_string());
    forward(&state, &auth_user, path, request).await
}

/// `GET /api/v1/assistant/conversations/{id}` -- transcript.
pub async fn get_history(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let path = assistant_service::history_path(&auth_user.user_id.to_string(), &conversation_id)?;
    forward(&state, &auth_user, path, request).await
}

/// `DELETE /api/v1/assistant/conversations/{id}` -- composite delete.
///
/// Removes both the `nyxid-chat` actor and the chat-history row (the Chat
/// History contract's dual-delete): an `/api/chat`-created row has no
/// actor, and a cascaded actor delete may have already dropped the history
/// row, so 404 from either side counts as done. Anything else propagates
/// that upstream response unchanged.
///
/// Deliberate divergence from the reference BFF (which attempts both
/// deletes even when the first hard-fails): a non-404 actor-delete failure
/// short-circuits here so the conversation stays fully intact and
/// retryable, instead of deleting the history row and leaving an orphaned
/// actor that no list surface can show. Retrying converges either way
/// (a half-gone actor answers 404 next time and the history delete runs).
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let user_id = auth_user.user_id.to_string();
    let authorization = request.headers().get(header::AUTHORIZATION).cloned();
    let actor_path = assistant_service::conversation_path(&user_id, &conversation_id)?;
    let history_path = assistant_service::history_path(&user_id, &conversation_id)?;

    let actor_response = forward(&state, &auth_user, actor_path, request).await?;
    if !assistant_service::delete_status_acceptable(actor_response.status().as_u16()) {
        return Ok(actor_response);
    }

    let history_request =
        synthetic_request(Method::DELETE, authorization.as_ref()).map_err(|_| {
            AppError::Internal("assistant: failed to build the history delete request".to_string())
        })?;
    let history_response = forward(&state, &auth_user, history_path, history_request).await?;
    if !assistant_service::delete_status_acceptable(history_response.status().as_u16()) {
        return Ok(history_response);
    }

    Ok((StatusCode::OK, Json(serde_json::json!({}))).into_response())
}

/// `POST /api/v1/assistant/conversations/{id}/stream` -- AG-UI SSE turn.
pub async fn stream_turn(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let path = assistant_service::stream_path(&auth_user.user_id.to_string(), &conversation_id)?;
    forward(&state, &auth_user, path, request).await
}

/// `POST /api/v1/assistant/conversations/{id}/approve` -- approval decision.
/// Human-only by virtue of the router this mounts under (PRD N3).
pub async fn decide_approval(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let path = assistant_service::approve_path(&auth_user.user_id.to_string(), &conversation_id)?;
    forward(&state, &auth_user, path, request).await
}

/// `POST /api/v1/assistant/completions` -- OpenAI-compatible SSE stream.
pub async fn completions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    forward(
        &state,
        &auth_user,
        assistant_service::completions_path(),
        request,
    )
    .await
}

/// `POST /api/v1/assistant/workflow-chat` -- ad-hoc workflow chat SSE stream.
///
/// Streams Aevatar's raw workflow engine events (`aevatar.raw.observed`
/// envelopes carrying workflow YAML, system prompts, and kernel state) to the
/// authenticated session. PRD §5.8 excludes that telemetry from the chat UI;
/// exposing it on this surface was accepted explicitly (2026-07-17), so the
/// filtering, if any, is the client's choice.
pub async fn workflow_chat(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    forward(
        &state,
        &auth_user,
        assistant_service::workflow_chat_path(),
        request,
    )
    .await
}

/// `GET /api/v1/assistant/workflow-chat/ws` -- WebSocket twin of the workflow
/// chat. `execute_proxy` detects the upgrade headers and bridges the socket;
/// browser callers authenticate via the session cookie since WebSocket
/// clients cannot set an `Authorization` header.
pub async fn workflow_chat_ws(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    forward(
        &state,
        &auth_user,
        assistant_service::workflow_chat_ws_path(),
        request,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_requests_carry_the_callers_authorization() {
        let value = HeaderValue::from_static("Bearer nyx_test_token");
        let request = synthetic_request(Method::DELETE, Some(&value)).unwrap();
        assert_eq!(request.method(), Method::DELETE);
        assert_eq!(
            request.headers().get(header::AUTHORIZATION),
            Some(&HeaderValue::from_static("Bearer nyx_test_token"))
        );
    }

    #[test]
    fn synthetic_requests_without_authorization_stay_bare() {
        let request = synthetic_request(Method::GET, None).unwrap();
        assert_eq!(request.method(), Method::GET);
        assert!(request.headers().get(header::AUTHORIZATION).is_none());
        assert!(request.headers().get(header::COOKIE).is_none());
    }

    #[test]
    fn bridge_mints_only_for_cookie_sessions_on_a_forwarding_row() {
        // The one broken caller class: cookie session + Bearer-forwarding row.
        assert!(needs_forward_token_bridge(&AuthMethod::Session, true));
        // Bearer callers keep their own token byte-for-byte.
        assert!(!needs_forward_token_bridge(&AuthMethod::AccessToken, true));
        // The TD-3 row flip (forward_access_token -> false, identity-token
        // mode) retires the bridge with no code change.
        assert!(!needs_forward_token_bridge(&AuthMethod::Session, false));
        assert!(!needs_forward_token_bridge(&AuthMethod::AccessToken, false));
    }
}
