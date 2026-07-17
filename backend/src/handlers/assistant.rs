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
    http::{Method, Request, StatusCode},
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::proxy::execute_proxy;
use crate::mw::auth::AuthUser;
use crate::services::assistant_service;

/// Create responses are a ~300-byte accepted envelope; the cap only guards
/// against a misbehaving upstream.
const MAX_CREATE_RESPONSE_BYTES: usize = 64 * 1024;
/// The actor index is bare `{actorId}` rows (~60 bytes each).
const MAX_INDEX_RESPONSE_BYTES: usize = 512 * 1024;

/// Resolve the admin-managed Aevatar service and forward `path` to it.
///
/// `path` is always built server-side by the callers below from
/// `auth_user.user_id`; no caller-supplied scope reaches this function.
async fn forward(
    state: &AppState,
    auth_user: &AuthUser,
    path: String,
    request: Request<Body>,
) -> AppResult<Response> {
    let service = assistant_service::resolve_admin_service(&state.db).await?;
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
        wait_for_conversation_materialization(&state, &auth_user, &user_id, &actor_id).await;
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
) {
    for attempt in 0..assistant_service::CREATE_POLL_ATTEMPTS {
        let Ok(index_request) = Request::builder()
            .method(Method::GET)
            .uri("/")
            .body(Body::empty())
        else {
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
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let user_id = auth_user.user_id.to_string();
    let actor_path = assistant_service::conversation_path(&user_id, &conversation_id)?;
    let history_path = assistant_service::history_path(&user_id, &conversation_id)?;

    let actor_response = forward(&state, &auth_user, actor_path, request).await?;
    if !assistant_service::delete_status_acceptable(actor_response.status().as_u16()) {
        return Ok(actor_response);
    }

    let history_request = Request::builder()
        .method(Method::DELETE)
        .uri("/")
        .body(Body::empty())
        .map_err(|_| {
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
