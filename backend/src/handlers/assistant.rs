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
//! session-authed mount; the assistant needs eight.
//!
//! Forwarding goes through `proxy::execute_proxy`, so credential injection,
//! identity propagation, per-agent rate limiting, approval gating, and audit
//! all apply unchanged -- the assistant is not a special case in the data
//! plane (PRD N4). SSE responses stream through it unbuffered.

use axum::{
    body::Body,
    extract::{Path, State},
    http::Request,
    response::Response,
};

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::proxy::execute_proxy;
use crate::mw::auth::AuthUser;
use crate::services::assistant_service;

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
pub async fn create_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let path = assistant_service::conversations_path(&auth_user.user_id.to_string());
    forward(&state, &auth_user, path, request).await
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

/// `DELETE /api/v1/assistant/conversations/{id}`.
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let path =
        assistant_service::conversation_path(&auth_user.user_id.to_string(), &conversation_id)?;
    forward(&state, &auth_user, path, request).await
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
