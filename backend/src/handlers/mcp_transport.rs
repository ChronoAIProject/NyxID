use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::AppState;
use crate::crypto::jwt;
use crate::models::mcp_session::{MCP_SESSION_COLLECTION, McpSessionRecord};
use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::{self, AuthMethod};
use crate::services::{
    approval_service, audit_service, mcp_service, notification_service, operation_descriptor,
    oracle_pool_service, oracle_session_service, oracle_task_service, proxy_service, ssh_service,
    user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

use super::services_helpers::fetch_service;

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 types
// ---------------------------------------------------------------------------

const JSONRPC_VERSION: &str = "2.0";
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    params: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn rpc_success(id: Option<serde_json::Value>, result: serde_json::Value) -> Response {
    axum::Json(JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION.into(),
        id,
        result: Some(result),
        error: None,
    })
    .into_response()
}

fn rpc_error(id: Option<serde_json::Value>, code: i32, message: &str) -> Response {
    axum::Json(JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION.into(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
            data: None,
        }),
    })
    .into_response()
}

/// MCP tool result (success or error are conveyed via `isError`, not JSON-RPC error).
fn tool_result(id: Option<serde_json::Value>, text: &str, is_error: bool) -> Response {
    rpc_success(
        id,
        serde_json::json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        }),
    )
}

/// Check if the client's `Accept` header includes `text/event-stream`.
/// Used to decide whether we can send inline SSE notifications in POST responses
/// (Streamable HTTP transport).
fn accepts_sse(headers: &HeaderMap) -> bool {
    headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("text/event-stream"))
}

/// Return a tool result followed by JSON-RPC notifications in a single SSE
/// stream embedded in the POST response.  This allows clients to receive
/// `notifications/tools/list_changed` without depending on a separate GET SSE
/// channel (which is optional in Streamable HTTP transport).
fn tool_result_with_notifications(
    id: Option<serde_json::Value>,
    text: &str,
    is_error: bool,
    notifications: Vec<serde_json::Value>,
) -> Response {
    let result = JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION.into(),
        id,
        result: Some(serde_json::json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        })),
        error: None,
    };

    let mut events: Vec<Result<Event, Infallible>> = Vec::with_capacity(1 + notifications.len());

    events.push(Ok(Event::default()
        .event("message")
        .data(serde_json::to_string(&result).unwrap_or_default())));

    for notification in notifications {
        events.push(Ok(Event::default()
            .event("message")
            .data(serde_json::to_string(&notification).unwrap_or_default())));
    }

    Sse::new(tokio_stream::iter(events)).into_response()
}

/// MCP-formatted 401 with `WWW-Authenticate` pointing to the protected-resource
/// metadata endpoint (RFC 9728).
fn mcp_401(base_url: &str) -> Response {
    let resource_url = format!(
        "{}/.well-known/oauth-protected-resource",
        base_url.trim_end_matches('/')
    );
    let body = serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "error": { "code": -32001, "message": "Authentication required" },
        "id": null,
    });

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            "www-authenticate",
            format!("Bearer resource_metadata=\"{resource_url}\""),
        )
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("failed to build 401 response")
}

fn mcp_403_insufficient_scope() -> Response {
    let body = serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "error": {
            "code": -32003,
            "message": format!(
                "Missing required scope for MCP access. Expected one of: {}, {}",
                auth::PROXY_SCOPE,
                auth::WIDE_PROXY_SCOPE
            ),
        },
        "id": null,
    });

    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("failed to build 403 response")
}

/// 403 returned when an `x-api-key` lacks the scope required for MCP.
/// API keys currently only accept `proxy` (see `VALID_API_KEY_SCOPES`),
/// so the message names just that scope.
fn mcp_403_api_key_insufficient_scope() -> Response {
    let body = serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "error": {
            "code": -32003,
            "message": format!(
                "API key is missing the required scope for MCP access. Expected: {}",
                auth::PROXY_SCOPE
            ),
        },
        "id": null,
    });

    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("failed to build 403 response")
}

/// JSON-RPC-style forbidden response for scope/binding violations.
fn rpc_scope_forbidden(id: Option<serde_json::Value>, message: &str) -> Response {
    axum::Json(JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION.into(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code: -32003,
            message: message.into(),
            data: None,
        }),
    })
    .into_response()
}

// ---------------------------------------------------------------------------
// Auth helper (manual token validation, NOT AuthUser extractor)
// ---------------------------------------------------------------------------

/// Result of MCP authentication.
///
/// Carries the full API-key identity and scope fields so that MCP requests
/// honor the same agent-isolation model as the REST proxy path:
/// service/node allow-lists, per-agent credential bindings, per-agent rate
/// limits, and audit attribution.
#[derive(Debug, Clone)]
struct McpAuthContext {
    user_id: String,
    auth_method: AuthMethod,
    acting_client_id: Option<String>,
    approval_owner_user_id: Option<String>,
    /// True when auth was via `x-api-key`. API-key requests are stateless: each
    /// request authenticates independently, no MCP session is created or required.
    is_api_key: bool,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
    /// If false, `allowed_service_ids` constrains which UserServices this request may call.
    allow_all_services: bool,
    /// If false, `allowed_node_ids` constrains which nodes this request may route through.
    allow_all_nodes: bool,
    allowed_service_ids: Vec<String>,
    allowed_node_ids: Vec<String>,
    rate_limit_per_second: Option<u32>,
    rate_limit_burst: Option<u32>,
    ip_address: Option<String>,
    user_agent: Option<String>,
}

impl McpAuthContext {
    fn user(user_id: String, auth_method: AuthMethod) -> Self {
        Self {
            user_id,
            auth_method,
            acting_client_id: None,
            approval_owner_user_id: None,
            is_api_key: false,
            api_key_id: None,
            api_key_name: None,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: Vec::new(),
            allowed_node_ids: Vec::new(),
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        }
    }

    fn effective_approval_owner_user_id(&self) -> String {
        self.approval_owner_user_id
            .clone()
            .unwrap_or_else(|| self.user_id.clone())
    }

    fn approval_requester_type(&self) -> Option<&'static str> {
        match &self.auth_method {
            AuthMethod::ApiKey => Some("api_key"),
            AuthMethod::Delegated => Some("delegated"),
            AuthMethod::ServiceAccount => Some("service_account"),
            AuthMethod::AccessToken => Some("access_token"),
            AuthMethod::Relay => Some("relay"),
            AuthMethod::Session => None,
        }
    }

    fn approval_requester_id(&self) -> String {
        self.acting_client_id
            .clone()
            .unwrap_or_else(|| self.user_id.clone())
    }
}

fn mcp_extract_ip(headers: &HeaderMap) -> Option<String> {
    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(forwarded);
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn mcp_extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

/// Extract and validate the request credentials, returning the user_id.
///
/// Auth precedence:
/// 1. `x-api-key` header (stateless, for headless integrations like n8n/Make/Zapier)
/// 2. `Authorization: Bearer <JWT>` (OAuth access token)
/// 3. `Mcp-Session-Id` header (session fallback; only when `session_fallback` is true)
///
/// When `session_fallback` is true (all methods except `initialize`), an
/// expired JWT is tolerated as long as a valid MCP session exists.  This
/// allows long-lived MCP sessions (30 days) to survive past the short-lived
/// access-token TTL without forcing re-authentication.
///
/// On failure returns an MCP-formatted 401 response with `WWW-Authenticate`.
async fn authenticate_mcp(
    state: &AppState,
    headers: &HeaderMap,
    session_fallback: bool,
) -> Result<McpAuthContext, Response> {
    let request_ip = mcp_extract_ip(headers);
    let request_ua = mcp_extract_user_agent(headers);

    // --- Try API key first (stateless auth for headless clients) ---
    if let Some(api_key_header) = headers.get("x-api-key") {
        let raw_key = api_key_header
            .to_str()
            .map_err(|_| mcp_401(&state.config.base_url))?;

        match crate::services::key_service::validate_api_key(&state.db, raw_key).await {
            Ok((user_id, api_key)) => {
                if !auth::scope_allows_rest_proxy(&api_key.scopes) {
                    return Err(mcp_403_api_key_insufficient_scope());
                }
                let user_id = verify_user_active(state, user_id).await?;
                return Ok(McpAuthContext {
                    user_id,
                    auth_method: AuthMethod::ApiKey,
                    acting_client_id: None,
                    approval_owner_user_id: None,
                    is_api_key: true,
                    api_key_id: Some(api_key.id.clone()),
                    api_key_name: Some(api_key.name.clone()),
                    allow_all_services: api_key.allow_all_services,
                    allow_all_nodes: api_key.allow_all_nodes,
                    allowed_service_ids: api_key.allowed_service_ids.clone(),
                    allowed_node_ids: api_key.allowed_node_ids.clone(),
                    rate_limit_per_second: api_key.rate_limit_per_second,
                    rate_limit_burst: api_key.rate_limit_burst,
                    ip_address: request_ip.clone(),
                    user_agent: request_ua.clone(),
                });
            }
            Err(_) => return Err(mcp_401(&state.config.base_url)),
        }
    }

    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    // --- Try JWT-based auth ---
    if let Some(token) = token {
        match jwt::verify_token(&state.jwt_keys, &state.config, token) {
            Ok(claims) if claims.token_type == "access" => {
                if !auth::scope_allows_rest_proxy(&claims.scope) {
                    return Err(mcp_403_insufficient_scope());
                }

                // Service account tokens have sa=true; verify against
                // the service_accounts collection instead of users.
                let (user_id, approval_owner_user_id) = if claims.sa == Some(true) {
                    let (sa_id, owner_id) =
                        verify_service_account_active(state, claims.sub).await?;
                    (sa_id, Some(owner_id))
                } else {
                    (verify_user_active(state, claims.sub).await?, None)
                };

                let auth_method = if claims.sa == Some(true) {
                    AuthMethod::ServiceAccount
                } else if claims.act.is_some() {
                    AuthMethod::Delegated
                } else if claims.relay == Some(true) {
                    AuthMethod::Relay
                } else {
                    AuthMethod::AccessToken
                };

                let mut ctx = McpAuthContext::user(user_id, auth_method);
                ctx.acting_client_id = claims.act.map(|a| a.sub);
                ctx.approval_owner_user_id = approval_owner_user_id;
                ctx.ip_address = request_ip.clone();
                ctx.user_agent = request_ua.clone();
                return Ok(ctx);
            }
            Err(_) if session_fallback => {
                // Any JWT error (expired, invalid issuer, etc.) -- fall through
                // to session-based auth. The MCP session ID is the real auth
                // mechanism for long-lived connections; the JWT is only needed
                // for the initial `initialize` call.
            }
            _ => return Err(mcp_401(&state.config.base_url)),
        }
    } else if !session_fallback {
        // No token at all and session fallback not allowed (initialize)
        return Err(mcp_401(&state.config.base_url));
    }

    // --- Session-based auth fallback ---
    let session_id = headers.get("mcp-session-id").and_then(|v| v.to_str().ok());

    if let Some(sid) = session_id
        && let Some(user_id) = state.mcp_sessions.get_user_id(sid)
    {
        if !state.mcp_sessions.allows_proxy_access(sid) {
            return Err(mcp_403_insufficient_scope());
        }
        let user_id = verify_user_active(state, user_id).await?;
        let mut ctx = McpAuthContext::user(user_id, AuthMethod::Session);
        ctx.ip_address = request_ip;
        ctx.user_agent = request_ua;
        return Ok(ctx);
    }

    Err(mcp_401(&state.config.base_url))
}

/// Check that a user account exists and is active.
async fn verify_user_active(state: &AppState, user_id: String) -> Result<String, Response> {
    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await
        .map_err(|_| rpc_error(None, -32603, "Internal error"))?;

    match user {
        Some(u) if u.is_active => Ok(user_id),
        _ => Err(mcp_401(&state.config.base_url)),
    }
}

/// Check that a service account exists and is active.
async fn verify_service_account_active(
    state: &AppState,
    sa_id: String,
) -> Result<(String, String), Response> {
    let sa = state
        .db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "_id": &sa_id, "is_active": true })
        .await
        .map_err(|_| rpc_error(None, -32603, "Internal error"))?;

    match sa {
        Some(sa) => Ok((sa_id, sa.effective_owner_user_id().to_string())),
        None => Err(mcp_401(&state.config.base_url)),
    }
}

/// Extract the `Mcp-Session-Id` header value.
#[allow(clippy::result_large_err)]
fn require_session(headers: &HeaderMap) -> Result<String, Response> {
    headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .ok_or_else(|| rpc_error(None, -32002, "Mcp-Session-Id header required"))
}

/// Validate session exists and belongs to user, then touch it.
#[allow(clippy::result_large_err)]
fn validate_session(
    state: &AppState,
    session_id: &str,
    user_id: &str,
    request_id: Option<serde_json::Value>,
) -> Result<(), Response> {
    if !state.mcp_sessions.validate(session_id, user_id) {
        return Err(rpc_error(request_id, -32002, "Invalid or expired session"));
    }
    state.mcp_sessions.touch(session_id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Notification helper
// ---------------------------------------------------------------------------

/// True when the caller is an API key with any scope restriction (either a
/// service or node allow-list). Such callers must not reach SSH meta-tools,
/// which have no per-service/per-node binding to enforce against.
fn is_scoped_api_key(auth: &McpAuthContext) -> bool {
    auth.is_api_key && (!auth.allow_all_services || !auth.allow_all_nodes)
}

/// Names of tools that SSH-gate on agent scope.
const SSH_META_TOOL_NAMES: &[&str] = &["nyx__ssh_exec", "nyx__ssh_list_services"];

/// Drop user-managed services the API key is not scoped to, and reject all
/// platform services (scoped API keys are UserService-only, matching the REST
/// proxy check in `execute_proxy_inner`). OAuth/session callers keep every
/// service since `allow_all_services` is `true` in `McpAuthContext::user`.
fn filter_services_by_scope(
    services: Vec<mcp_service::McpToolService>,
    auth: &McpAuthContext,
) -> Vec<mcp_service::McpToolService> {
    if auth.allow_all_services {
        return services;
    }
    services
        .into_iter()
        .filter(|svc| match &svc.source {
            mcp_service::McpToolSource::UserManaged { .. } => {
                auth.allowed_service_ids.contains(&svc.service_id)
            }
            mcp_service::McpToolSource::Platform { .. } => false,
        })
        .collect()
}

/// Return an error response when the authenticated API key does not have
/// access to this service. Mirrors `AppError::ApiKeyScopeForbidden` framing.
#[allow(clippy::result_large_err)]
fn ensure_service_in_scope(
    auth: &McpAuthContext,
    service: &mcp_service::McpToolService,
    request_id: Option<serde_json::Value>,
) -> Result<(), Response> {
    if auth.allow_all_services {
        return Ok(());
    }
    match &service.source {
        mcp_service::McpToolSource::UserManaged { .. }
            if auth.allowed_service_ids.contains(&service.service_id) =>
        {
            Ok(())
        }
        mcp_service::McpToolSource::UserManaged { .. } => Err(tool_result(
            request_id,
            "API key does not have access to this service",
            true,
        )),
        mcp_service::McpToolSource::Platform { .. } => Err(tool_result(
            request_id,
            "Scoped API keys cannot call platform services through MCP",
            true,
        )),
    }
}

/// Send a `notifications/tools/list_changed` JSON-RPC notification
/// to the session's SSE stream.
fn send_tools_list_changed(state: &AppState, session_id: &str) {
    let notification = serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": "notifications/tools/list_changed",
    });

    if !state
        .mcp_sessions
        .send_notification(session_id, notification)
    {
        tracing::debug!(
            session_id,
            "Failed to send tools/list_changed notification (no SSE listener)"
        );
    }
}

// ---------------------------------------------------------------------------
// POST /mcp -- JSON-RPC request handler
// ---------------------------------------------------------------------------

pub async fn mcp_post(State(state): State<AppState>, headers: HeaderMap, body: String) -> Response {
    // Manual JSON parse for proper JSON-RPC error on malformed input
    let request: JsonRpcRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(_) => return rpc_error(None, -32700, "Parse error"),
    };

    // `initialize` requires a valid JWT or API key (no session exists yet).
    // All other methods allow session-based auth fallback.
    let is_initialize = request.method == "initialize";
    let auth = match authenticate_mcp(&state, &headers, !is_initialize).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    // Per-agent rate limit runs after auth, before any work. For OAuth/session
    // callers it is a no-op (no api_key_id / rps). This mirrors the REST proxy.
    if let Err(e) = crate::mw::rate_limit::check_agent_rate_limit_raw(
        &state.per_agent_limiter,
        auth.api_key_id.as_deref(),
        auth.rate_limit_per_second,
        auth.rate_limit_burst,
    ) {
        return app_error_to_rpc(request.id.clone(), &e);
    }

    let user_id = auth.user_id.clone();

    match request.method.as_str() {
        "initialize" => {
            let tele = TelemetryContext::from_headers(
                headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
                headers
                    .get("x-nyxid-client-version")
                    .and_then(|v| v.to_str().ok()),
            );
            handle_initialize(
                &state,
                &user_id,
                &request,
                auth.is_api_key,
                auth.api_key_id.as_deref(),
                auth.api_key_name.as_deref(),
                auth.ip_address.as_deref(),
                auth.user_agent.as_deref(),
                &tele,
            )
        }

        "notifications/initialized" => {
            if let Ok(sid) = require_session(&headers) {
                state.mcp_sessions.touch(&sid);
            }
            StatusCode::ACCEPTED.into_response()
        }

        "tools/list" => {
            let sid = match resolve_session(&state, &headers, &user_id, &auth, request.id.clone()) {
                Ok(s) => s,
                Err(r) => return r,
            };
            handle_tools_list(&state, &auth, sid.as_deref(), &request).await
        }

        "tools/call" => {
            let sid = match resolve_session(&state, &headers, &user_id, &auth, request.id.clone()) {
                Ok(s) => s,
                Err(r) => return r,
            };
            let sse_capable = accepts_sse(&headers);
            handle_tools_call(&state, &auth, sid.as_deref(), &request, sse_capable).await
        }

        "ping" => rpc_success(request.id, serde_json::json!({})),

        _ => rpc_error(request.id, -32601, "Method not found"),
    }
}

/// Translate an `AppError` into a JSON-RPC response. Used when callers hold the
/// raw request id and need uniform error framing (e.g. rate-limit rejections).
fn app_error_to_rpc(id: Option<serde_json::Value>, err: &crate::errors::AppError) -> Response {
    use crate::errors::AppError;
    match err {
        AppError::RateLimited => rpc_error(id, -32005, "Rate limit exceeded"),
        AppError::ApiKeyScopeForbidden(msg) => rpc_scope_forbidden(id, msg),
        _ => rpc_error(id, -32603, "Internal error"),
    }
}

/// Resolve the MCP session for a request.
///
/// For OAuth/JWT/session auth: session is required and validated against user_id.
/// For API-key auth: session is optional. If the header is present it must be
/// valid; if absent the request proceeds statelessly (returns `None`).
#[allow(clippy::result_large_err)]
fn resolve_session(
    state: &AppState,
    headers: &HeaderMap,
    user_id: &str,
    auth: &McpAuthContext,
    request_id: Option<serde_json::Value>,
) -> Result<Option<String>, Response> {
    let raw_sid = headers.get("mcp-session-id").and_then(|v| v.to_str().ok());

    match (auth.is_api_key, raw_sid) {
        (true, None) => Ok(None),
        (true, Some(sid)) => {
            let sid = sid.to_string();
            validate_session(state, &sid, user_id, request_id)?;
            Ok(Some(sid))
        }
        (false, _) => {
            let sid = require_session(headers)?;
            validate_session(state, &sid, user_id, request_id)?;
            Ok(Some(sid))
        }
    }
}

// ---------------------------------------------------------------------------
// GET /mcp -- SSE notification stream
// ---------------------------------------------------------------------------

pub async fn mcp_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let auth = match authenticate_mcp(&state, &headers, true).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    if let Err(e) = crate::mw::rate_limit::check_agent_rate_limit_raw(
        &state.per_agent_limiter,
        auth.api_key_id.as_deref(),
        auth.rate_limit_per_second,
        auth.rate_limit_burst,
    ) {
        return app_error_to_rpc(None, &e);
    }

    // API-key requests without a session have nothing to stream: return empty
    // keep-alive SSE rather than 400. Real clients use POST /mcp for each call.
    if auth.is_api_key && headers.get("mcp-session-id").is_none() {
        let stream = tokio_stream::empty::<Result<Event, Infallible>>();
        return Sse::new(stream)
            .keep_alive(
                KeepAlive::new()
                    .interval(Duration::from_secs(30))
                    .text("keepalive"),
            )
            .into_response();
    }

    let user_id = auth.user_id;
    let sid = match require_session(&headers) {
        Ok(s) => s,
        Err(r) => return r,
    };

    if let Err(r) = validate_session(&state, &sid, &user_id, None) {
        return r;
    }

    // Take the notification receiver for this session.
    // If already taken (reconnect), create a new channel pair.
    let rx = match state.mcp_sessions.take_notification_rx(&sid) {
        Some(rx) => rx,
        None => {
            // Reconnect: create new channel, update session's tx
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            state.mcp_sessions.set_notification_tx(&sid, tx);
            rx
        }
    };

    // Convert mpsc::Receiver into an SSE-compatible stream
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|notification| {
        Ok::<_, Infallible>(
            Event::default()
                .event("message")
                .data(notification.to_string()),
        )
    });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("keepalive"),
        )
        .into_response()
}

// ---------------------------------------------------------------------------
// DELETE /mcp -- session termination
// ---------------------------------------------------------------------------

pub async fn mcp_delete(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let auth = match authenticate_mcp(&state, &headers, true).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    if let Err(e) = crate::mw::rate_limit::check_agent_rate_limit_raw(
        &state.per_agent_limiter,
        auth.api_key_id.as_deref(),
        auth.rate_limit_per_second,
        auth.rate_limit_burst,
    ) {
        return app_error_to_rpc(None, &e);
    }

    // API-key request without a session: nothing to delete, return 204.
    if auth.is_api_key && headers.get("mcp-session-id").is_none() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let sid = match require_session(&headers) {
        Ok(s) => s,
        Err(r) => return r,
    };

    if let Err(r) = validate_session(&state, &sid, &auth.user_id, None) {
        return r;
    }

    // Look up the persisted session's `created_at` before removing it so we
    // can emit `mcp.session_ended` with a duration. Best-effort: the write
    // that persists the record is fire-and-forget at create time, so the
    // record may not exist yet for very-short-lived sessions -- duration
    // falls back to 0 in that case. See `docs/TELEMETRY.md` §6.5.
    let persisted_created_at = state
        .db
        .collection::<McpSessionRecord>(MCP_SESSION_COLLECTION)
        .find_one(doc! { "_id": &sid })
        .await
        .ok()
        .flatten()
        .map(|rec| rec.created_at);

    state.mcp_sessions.remove(&sid);

    // Audit log for session deletion -- attribute API key when present.
    audit_service::log_async(
        state.db.clone(),
        Some(auth.user_id.clone()),
        "mcp_session_deleted".to_string(),
        Some(serde_json::json!({ "session_id": &sid })),
        auth.ip_address.clone(),
        auth.user_agent.clone(),
        auth.api_key_id.clone(),
        auth.api_key_name.clone(),
    );

    // Telemetry: mcp.session_ended. Best-effort: normal close only, abrupt
    // close may miss -- see TELEMETRY.md §6.5. `reason = "client_close"`
    // since this handler only runs on explicit DELETE. Build context from
    // request headers so CLI/UI/SDK sessions keep the correct `surface`
    // instead of collapsing to `backend` via `TelemetryContext::default()`.
    let tele = TelemetryContext::from_headers(
        headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
        headers
            .get("x-nyxid-client-version")
            .and_then(|v| v.to_str().ok()),
    );
    let duration_ms = persisted_created_at
        .map(|start| {
            let elapsed = chrono::Utc::now().signed_duration_since(start);
            let ms = elapsed.num_milliseconds();
            if ms < 0 { 0 } else { ms as u64 }
        })
        .unwrap_or(0);
    emit_event(
        state.telemetry.as_deref(),
        &auth.user_id,
        auth.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::McpSessionEnded {
            duration_ms,
            reason: "client_close".to_string(),
        },
    );

    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// Method handlers
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_initialize(
    state: &AppState,
    user_id: &str,
    request: &JsonRpcRequest,
    is_api_key: bool,
    api_key_id: Option<&str>,
    api_key_name: Option<&str>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    tele: &TelemetryContext,
) -> Response {
    // API-key auth is stateless: each request authenticates independently,
    // so no MCP session is created on initialize.
    let session_id = if is_api_key {
        None
    } else {
        match state.mcp_sessions.create_with_proxy_access(user_id, true) {
            Some(id) => {
                audit_service::log_async(
                    state.db.clone(),
                    Some(user_id.to_string()),
                    "mcp_session_created".to_string(),
                    Some(serde_json::json!({ "session_id": &id })),
                    ip_address.map(String::from),
                    user_agent.map(String::from),
                    api_key_id.map(String::from),
                    api_key_name.map(String::from),
                );
                Some(id)
            }
            None => return rpc_error(request.id.clone(), -32000, "Too many active MCP sessions"),
        }
    };

    // Telemetry: mcp.session_started. `client` is taken from the MCP
    // `clientInfo.name` field when provided by the initialize params, else
    // None. Fire for both session-backed and stateless API-key callers so
    // per-client usage is visible in either mode.
    let client = request
        .params
        .as_ref()
        .and_then(|p| p.get("clientInfo"))
        .and_then(|ci| ci.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_owned);
    emit_event(
        state.telemetry.as_deref(),
        user_id,
        api_key_id,
        tele,
        TelemetryEvent::McpSessionStarted { client },
    );

    let result = serde_json::json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": true },
        },
        "serverInfo": {
            "name": "NyxID",
            "version": env!("CARGO_PKG_VERSION"),
        }
    });

    let body = JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION.into(),
        id: request.id.clone(),
        result: Some(result),
        error: None,
    };

    let mut response = axum::Json(body).into_response();

    if let Some(sid) = session_id {
        let header_value = match axum::http::HeaderValue::from_str(&sid) {
            Ok(v) => v,
            Err(_) => {
                return rpc_error(
                    request.id.clone(),
                    -32603,
                    "Failed to create session header",
                );
            }
        };
        response.headers_mut().insert(
            axum::http::HeaderName::from_static("mcp-session-id"),
            header_value,
        );
    }

    response
}

async fn handle_tools_list(
    state: &AppState,
    auth: &McpAuthContext,
    session_id: Option<&str>,
    request: &JsonRpcRequest,
) -> Response {
    // Thread API-key node scope into the discovery chain so scoped
    // keys don't see tools whose only viable routes are all out of
    // scope (seventeenth-round Codex review P2).
    let services = match mcp_service::load_user_tools_scoped(
        &state.db,
        state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(auth),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load user tools: {e}");
            return rpc_error(request.id.clone(), -32603, "Failed to load tools");
        }
    };

    // Enforce API-key service scope: scoped keys only see the UserServices in
    // their allow-list, mirroring the REST proxy's ApiKeyScopeForbidden check.
    let services = filter_services_by_scope(services, auth);

    // Session-backed clients get meta-tools + activated service tools only.
    // Stateless (API-key) clients with no session get the full tool list up front.
    let mut tool_defs = match session_id {
        Some(sid) => {
            let activated = state.mcp_sessions.get_activated_service_ids(sid);
            mcp_service::generate_tool_definitions(&services, Some(&activated))
        }
        None => mcp_service::generate_tool_definitions(&services, None),
    };

    // Scoped API keys do not get SSH meta-tools. SSH invocations would
    // otherwise escape the service/node allow-list entirely.
    if is_scoped_api_key(auth) {
        tool_defs.retain(|t| !SSH_META_TOOL_NAMES.contains(&t.name.as_str()));
    }

    let tools_json: Vec<serde_json::Value> = tool_defs
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();

    rpc_success(
        request.id.clone(),
        serde_json::json!({ "tools": tools_json }),
    )
}

async fn handle_tools_call(
    state: &AppState,
    auth: &McpAuthContext,
    session_id: Option<&str>,
    request: &JsonRpcRequest,
    client_accepts_sse: bool,
) -> Response {
    let params = match &request.params {
        Some(p) => p,
        None => return rpc_error(request.id.clone(), -32602, "Missing params"),
    };

    let tool_name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return rpc_error(request.id.clone(), -32602, "Missing tool name"),
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    // -- Meta-tools --
    match tool_name {
        "nyx__search_tools" => {
            return handle_meta_search(
                state,
                auth,
                session_id,
                &arguments,
                request.id.clone(),
                client_accepts_sse,
            )
            .await;
        }
        "nyx__discover_services" => {
            return handle_meta_discover(state, &auth.user_id, &arguments, request.id.clone())
                .await;
        }
        "nyx__connect_service" => {
            return handle_meta_connect(
                state,
                auth,
                session_id,
                &arguments,
                request.id.clone(),
                client_accepts_sse,
            )
            .await;
        }
        "nyx__call_tool" => {
            return handle_meta_call_tool(
                state,
                auth,
                session_id,
                &arguments,
                request.id.clone(),
                client_accepts_sse,
            )
            .await;
        }
        "nyx__ssh_exec" | "nyx__ssh_list_services" => {
            if is_scoped_api_key(auth) {
                return tool_result(
                    request.id.clone(),
                    "SSH meta-tools are not available for scoped API keys. \
                     Use an unrestricted API key or OAuth.",
                    true,
                );
            }
            if tool_name == "nyx__ssh_exec" {
                return handle_mcp_ssh_exec(state, auth, &arguments, request.id.clone()).await;
            }
            return handle_mcp_ssh_list(state, auth, request.id.clone()).await;
        }
        "nyx__oracle_pools" => {
            return handle_oracle_pools(state, auth, request.id.clone()).await;
        }
        "nyx__oracle_ask" => {
            return handle_oracle_ask(state, auth, &arguments, request.id.clone()).await;
        }
        "nyx__oracle_result" => {
            return handle_oracle_result(state, auth, &arguments, request.id.clone()).await;
        }
        "nyx__oracle_attach" => {
            return handle_oracle_attach(state, auth, &arguments, request.id.clone()).await;
        }
        "nyx__oracle_extract" => {
            return handle_oracle_extract(state, auth, &arguments, request.id.clone()).await;
        }
        "nyx__oracle_session" => {
            return handle_oracle_session(state, auth, &arguments, request.id.clone()).await;
        }
        _ => {}
    }

    // -- Service tool: verify activation (when stateful), load, resolve, execute --
    let activated = session_id.map(|sid| state.mcp_sessions.get_activated_service_ids(sid));

    // Scoped discovery so resolve_tool_call can't match tools whose
    // only viable routes fall outside the caller's API-key node scope
    // (twentieth-round Codex P2).
    let services = match mcp_service::load_user_tools_scoped(
        &state.db,
        state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(auth),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load tools for execution: {e}");
            return tool_result(request.id.clone(), "Failed to load tools", true);
        }
    };

    let (service, endpoint) = match mcp_service::resolve_tool_call(tool_name, &services) {
        Some(pair) => pair,
        None => {
            return tool_result(
                request.id.clone(),
                &format!(
                    "Unknown tool: {tool_name}. Use nyx__search_tools to find and activate tools."
                ),
                true,
            );
        }
    };

    // Enforce API-key service scope before activation/execute -- scoped keys
    // must not reach execute_tool for services outside their allow-list.
    if let Err(resp) = ensure_service_in_scope(auth, service, request.id.clone()) {
        return resp;
    }

    // Guard: only allow execution if the service is activated (stateful mode).
    // Stateless API-key requests bypass the activation gate.
    if let Some(ref activated_set) = activated
        && !activated_set.contains(&service.service_id)
    {
        return tool_result(
            request.id.clone(),
            &format!(
                "Tool '{}' belongs to service '{}' which is not activated. \
                 Use nyx__search_tools to activate it first.",
                tool_name, service.service_name,
            ),
            true,
        );
    }

    let operation = match mcp_service::build_mcp_operation_descriptor(service, endpoint, &arguments)
    {
        Ok(operation) => operation,
        Err(e) => {
            return tool_result(
                request.id.clone(),
                &format!("Invalid tool arguments: {e}"),
                true,
            );
        }
    };
    if let Err(resp) = authorize_mcp_operation(
        state,
        auth,
        approval_target_for_tool(auth, service),
        &operation,
        request.id.clone(),
    )
    .await
    {
        return resp;
    }

    let exec_ctx = mcp_exec_context(auth);
    let (status, body) = match mcp_service::execute_tool(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &auth.user_id,
        service,
        endpoint,
        &arguments,
        &state.jwt_keys,
        &state.config,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &exec_ctx,
    )
    .await
    {
        Ok(r) => r,
        Err(crate::errors::AppError::ApiKeyScopeForbidden(msg)) => {
            return tool_result(request.id.clone(), &msg, true);
        }
        Err(e) => {
            tracing::warn!("Tool execution failed for {tool_name}: {e}");
            return tool_result(
                request.id.clone(),
                &format!("Tool execution failed: {e}"),
                true,
            );
        }
    };

    // Audit log -- attribute the API key when the caller is an agent.
    audit_service::log_async(
        state.db.clone(),
        Some(auth.user_id.clone()),
        "mcp_tool_call".to_string(),
        Some(serde_json::json!({
            "tool": tool_name,
            "service_id": service.service_id,
            "response_status": status,
        })),
        auth.ip_address.clone(),
        auth.user_agent.clone(),
        auth.api_key_id.clone(),
        auth.api_key_name.clone(),
    );

    let is_error = !(200..300).contains(&status);
    let content_text = if is_error {
        format!("Error ({status}): {body}")
    } else {
        body
    };

    tool_result(request.id.clone(), &content_text, is_error)
}

/// Build the execution context passed to `mcp_service::execute_tool` from
/// the authenticated MCP caller -- API key identity + node scope.
fn mcp_exec_context<'a>(auth: &'a McpAuthContext) -> mcp_service::McpExecContext<'a> {
    mcp_service::McpExecContext {
        api_key_id: auth.api_key_id.as_deref(),
        allow_all_nodes: auth.allow_all_nodes,
        allowed_node_ids: &auth.allowed_node_ids,
    }
}

/// Derive the MCP node-scope filter from the authenticated caller.
/// Scoped API keys get an `Allowed` scope so the discovery chain hides
/// tools whose only viable routes are outside the allow-list (matches
/// `execute_tool`'s runtime scope check). Non-API-key callers and keys
/// with `allow_all_nodes` get `Unrestricted`.
fn mcp_node_scope<'a>(auth: &'a McpAuthContext) -> mcp_service::NodeScope<'a> {
    if auth.allow_all_nodes {
        mcp_service::NodeScope::Unrestricted
    } else {
        mcp_service::NodeScope::Allowed(auth.allowed_node_ids.as_slice())
    }
}

#[derive(Clone, Debug)]
struct McpApprovalTarget {
    service_id: String,
    service_name: String,
    service_slug: String,
    service_owner_user_id: String,
}

fn approval_target_for_tool(
    auth: &McpAuthContext,
    service: &mcp_service::McpToolService,
) -> McpApprovalTarget {
    let service_owner_user_id = match &service.source {
        mcp_service::McpToolSource::UserManaged {
            effective_owner_id, ..
        } => effective_owner_id.clone(),
        mcp_service::McpToolSource::Platform { .. } => auth.effective_approval_owner_user_id(),
    };

    McpApprovalTarget {
        service_id: service.service_id.clone(),
        service_name: service.service_name.clone(),
        service_slug: service.service_slug.clone(),
        service_owner_user_id,
    }
}

#[allow(clippy::result_large_err)]
async fn authorize_mcp_operation(
    state: &AppState,
    auth: &McpAuthContext,
    target: McpApprovalTarget,
    operation: &operation_descriptor::OperationDescriptor,
    request_id: Option<serde_json::Value>,
) -> Result<(), Response> {
    let approval_owner_user_id = auth.effective_approval_owner_user_id();
    let approval_outcome = approval_service::evaluate_and_check(
        &state.db,
        &approval_owner_user_id,
        &target.service_owner_user_id,
        &target.service_id,
        operation,
        auth.approval_requester_type(),
        &auth.approval_requester_id(),
        auth.auth_method == AuthMethod::Session,
    )
    .await
    .map_err(|e| {
        tool_result(
            request_id.clone(),
            &format!("Approval check failed: {e}"),
            true,
        )
    })?;

    let pending = match approval_outcome {
        approval_service::ApprovalOutcome::Allowed { .. } => return Ok(()),
        approval_service::ApprovalOutcome::Denied => {
            return Err(tool_result(
                request_id,
                "Operation denied by approval policy",
                true,
            ));
        }
        approval_service::ApprovalOutcome::NeedsApproval(pending) => pending,
    };

    let notify_user_ids = approval_service::approval_notification_recipients(
        &state.db,
        &approval_owner_user_id,
        &pending,
    )
    .await
    .map_err(|e| {
        tool_result(
            request_id.clone(),
            &format!("Approval check failed: {e}"),
            true,
        )
    })?;
    let timeout_recipient = notify_user_ids.first().cloned().ok_or_else(|| {
        tool_result(
            request_id.clone(),
            "Approval recipient list unexpectedly empty",
            true,
        )
    })?;
    let channel = notification_service::get_or_create_channel(&state.db, &timeout_recipient)
        .await
        .map_err(|e| {
            tool_result(
                request_id.clone(),
                &format!("Approval channel lookup failed: {e}"),
                true,
            )
        })?;
    let timeout_secs = channel.approval_timeout_secs;
    let request_operation = approval_service::ApprovalRequestOperation::from_descriptor(
        operation,
        pending.resolution.grant_scope.clone(),
    );
    let approval_request = approval_service::create_approval_request(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        &pending.primary_owner_user_id,
        &target.service_id,
        &target.service_name,
        &target.service_slug,
        &pending.requester_type,
        &pending.requester_id,
        auth.api_key_name.as_deref(),
        request_operation,
        pending.resolution.mode.clone(),
        timeout_secs,
        notify_user_ids,
        pending.resolution.from_org_policy,
    )
    .await
    .map_err(|e| {
        tool_result(
            request_id.clone(),
            &format!("Approval request failed: {e}"),
            true,
        )
    })?;

    let req_id = approval_request.id.clone();
    approval_service::wait_for_decision(&state.db, &approval_request.id, timeout_secs)
        .await
        .map_err(|error| {
            let mapped = approval_service::map_wait_for_decision_error(
                error,
                &req_id,
                &state.config.frontend_url,
            );
            tool_result(request_id, &format!("{mapped}"), true)
        })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Meta-tool dispatch helpers
// ---------------------------------------------------------------------------

/// `nyx__call_tool` -- universal proxy that lets clients invoke any connected
/// tool by name, bypassing the need for a `tools/list` refresh.  The AI
/// discovers tools via `nyx__search_tools` and then calls them through this
/// meta-tool, which is always in the static tool list.
async fn handle_meta_call_tool(
    state: &AppState,
    auth: &McpAuthContext,
    session_id: Option<&str>,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
    client_accepts_sse: bool,
) -> Response {
    let tool_name = match arguments.get("tool_name").and_then(|n| n.as_str()) {
        Some(n) if !n.is_empty() => n,
        _ => return tool_result(request_id, "tool_name is required", true),
    };

    if tool_name.len() > 200 {
        return tool_result(request_id, "tool_name too long (max 200 chars)", true);
    }

    // Accept arguments in multiple formats (LLMs are unpredictable):
    //   1. { "tool_name": "x", "arguments_json": "{\"foo\":1}" }  -- JSON string (preferred)
    //   2. { "tool_name": "x", "arguments": { "foo": 1 } }       -- nested object (legacy)
    //   3. { "tool_name": "x", "foo": 1 }                         -- flat (fallback)
    let inner_args =
        if let Some(json_str) = arguments.get("arguments_json").and_then(|v| v.as_str()) {
            // Preferred: arguments_json is a JSON string — parse it
            match serde_json::from_str::<serde_json::Value>(json_str) {
                Ok(parsed) => parsed,
                Err(e) => {
                    tracing::warn!("Failed to parse arguments_json as JSON: {e}, raw: {json_str}");
                    return tool_result(
                        request_id,
                        &format!("arguments_json must be a valid JSON string. Parse error: {e}"),
                        true,
                    );
                }
            }
        } else if let Some(nested) = arguments.get("arguments") {
            // Legacy: arguments as a nested object
            nested.clone()
        } else {
            // Flat fallback: collect all keys except "tool_name" and "arguments_json"
            let mut flat = serde_json::Map::new();
            if let Some(obj) = arguments.as_object() {
                for (k, v) in obj {
                    if k != "tool_name" && k != "arguments_json" {
                        flat.insert(k.clone(), v.clone());
                    }
                }
            }
            serde_json::Value::Object(flat)
        };

    // Load user tools with API-key node scope applied so
    // `nyx__call_tool` can't auto-invoke a tool whose only viable
    // routes are outside the caller's allow-list (twentieth-round
    // Codex P2). Service scope is still applied downstream below.
    let services = match mcp_service::load_user_tools_scoped(
        &state.db,
        state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(auth),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load tools for call_tool: {e}");
            return tool_result(request_id, "Failed to load tools", true);
        }
    };
    let services = filter_services_by_scope(services, auth);

    // Resolve tool (no activation gate -- that's the whole point)
    let (service, endpoint) = match mcp_service::resolve_tool_call(tool_name, &services) {
        Some(pair) => pair,
        None => {
            return tool_result(
                request_id,
                &format!(
                    "Unknown tool: {tool_name}. Use nyx__search_tools to find available tools."
                ),
                true,
            );
        }
    };

    let operation =
        match mcp_service::build_mcp_operation_descriptor(service, endpoint, &inner_args) {
            Ok(operation) => operation,
            Err(e) => {
                return tool_result(request_id, &format!("Invalid tool arguments: {e}"), true);
            }
        };
    if let Err(resp) = authorize_mcp_operation(
        state,
        auth,
        approval_target_for_tool(auth, service),
        &operation,
        request_id.clone(),
    )
    .await
    {
        return resp;
    }

    // Auto-activate so future tools/list responses include this service.
    // Stateless (API-key, no session) requests skip activation tracking.
    let changed = match session_id {
        Some(sid) => {
            let changed = state
                .mcp_sessions
                .activate_services(sid, std::slice::from_ref(&service.service_id));
            if changed {
                send_tools_list_changed(state, sid);
            }
            changed
        }
        None => false,
    };

    let exec_ctx = mcp_exec_context(auth);
    let (status, body) = match mcp_service::execute_tool(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &auth.user_id,
        service,
        endpoint,
        &inner_args,
        &state.jwt_keys,
        &state.config,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &exec_ctx,
    )
    .await
    {
        Ok(r) => r,
        Err(crate::errors::AppError::ApiKeyScopeForbidden(msg)) => {
            return tool_result(request_id, &msg, true);
        }
        Err(e) => {
            tracing::warn!("Tool execution failed for {tool_name}: {e}");
            return tool_result(request_id, &format!("Tool execution failed: {e}"), true);
        }
    };

    // Audit log -- attribute the API key when the caller is an agent.
    audit_service::log_async(
        state.db.clone(),
        Some(auth.user_id.clone()),
        "mcp_tool_call".to_string(),
        Some(serde_json::json!({
            "tool": tool_name,
            "service_id": service.service_id,
            "response_status": status,
            "via": "nyx__call_tool",
        })),
        auth.ip_address.clone(),
        auth.user_agent.clone(),
        auth.api_key_id.clone(),
        auth.api_key_name.clone(),
    );

    let is_error = !(200..300).contains(&status);
    let content_text = if is_error {
        format!("Error ({status}): {body}")
    } else {
        body
    };

    // Embed tools/list_changed inline for SSE-capable clients
    if changed && client_accepts_sse {
        tool_result_with_notifications(
            request_id,
            &content_text,
            is_error,
            vec![serde_json::json!({
                "jsonrpc": JSONRPC_VERSION,
                "method": "notifications/tools/list_changed",
            })],
        )
    } else {
        tool_result(request_id, &content_text, is_error)
    }
}

async fn handle_meta_search(
    state: &AppState,
    auth: &McpAuthContext,
    _session_id: Option<&str>,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
    _client_accepts_sse: bool,
) -> Response {
    let query = arguments
        .get("query")
        .and_then(|q| q.as_str())
        .unwrap_or("");

    if query.is_empty() {
        return tool_result(request_id, "Search query is required", true);
    }

    if query.len() > 200 {
        return tool_result(request_id, "Search query too long (max 200 chars)", true);
    }

    // Load ALL user tools including non-executable (for discovery).
    // Scope-aware so scoped keys don't see tools whose only viable
    // routes are outside their node allow-list (twentieth-round Codex
    // P2).
    let services = match mcp_service::load_user_tools_all_scoped(
        &state.db,
        state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(auth),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load tools for search: {e}");
            return tool_result(request_id, "Failed to load tools", true);
        }
    };

    // Scoped API keys only see services in their allow-list.
    let services = filter_services_by_scope(services, auth);

    // Search across ALL tools (does NOT activate services -- use nyx__call_tool
    // to invoke discovered tools, which auto-activates on first call)
    let search_result = mcp_service::search_all_tools(&services, query);

    let results: Vec<serde_json::Value> = search_result
        .matches
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();

    let response_json = serde_json::json!({
        "matches": results,
        "count": results.len(),
        "hint": "Use nyx__call_tool to invoke any of these tools by name. \
            Pass the tool name and arguments as shown in the match results.",
    });

    let text = serde_json::to_string_pretty(&response_json).unwrap_or_default();
    tool_result(request_id, &text, false)
}

async fn handle_meta_discover(
    state: &AppState,
    user_id: &str,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let query = arguments.get("query").and_then(|q| q.as_str());
    let category = arguments.get("category").and_then(|c| c.as_str());

    match mcp_service::discover_services(&state.db, user_id, query, category).await {
        Ok(result) => {
            let text = serde_json::to_string_pretty(&result).unwrap_or_default();
            tool_result(request_id, &text, false)
        }
        Err(e) => {
            tracing::error!("Failed to discover services: {e}");
            tool_result(request_id, "Failed to discover services", true)
        }
    }
}

async fn handle_meta_connect(
    state: &AppState,
    auth: &McpAuthContext,
    session_id: Option<&str>,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
    client_accepts_sse: bool,
) -> Response {
    let service_id = match arguments.get("service_id").and_then(|s| s.as_str()) {
        Some(id) if uuid::Uuid::try_parse(id).is_ok() => id,
        Some(_) => return tool_result(request_id, "Invalid service_id format", true),
        None => return tool_result(request_id, "service_id is required", true),
    };

    // Scoped API keys can only connect services already in their allow-list.
    if !auth.allow_all_services && !auth.allowed_service_ids.contains(&service_id.to_string()) {
        return tool_result(
            request_id,
            "API key does not have access to this service",
            true,
        );
    }

    let credential = arguments.get("credential").and_then(|c| c.as_str());
    let credential_label = arguments.get("credential_label").and_then(|l| l.as_str());

    match mcp_service::connect_service(
        &state.db,
        &state.encryption_keys,
        state.node_ws_manager.as_ref(),
        &auth.user_id,
        service_id,
        credential,
        credential_label,
    )
    .await
    {
        Ok(result) => {
            // Activate the newly connected service. Stateless (API-key,
            // no session) callers skip activation tracking.
            let changed = match session_id {
                Some(sid) => {
                    let changed = state
                        .mcp_sessions
                        .activate_services(sid, &[service_id.to_string()]);
                    // Send via GET SSE channel (fallback for clients that have it)
                    if changed {
                        send_tools_list_changed(state, sid);
                    }
                    changed
                }
                None => false,
            };

            audit_service::log_async(
                state.db.clone(),
                Some(auth.user_id.clone()),
                "mcp_connect_service".to_string(),
                Some(serde_json::json!({ "service_id": service_id })),
                auth.ip_address.clone(),
                auth.user_agent.clone(),
                auth.api_key_id.clone(),
                auth.api_key_name.clone(),
            );

            // Construct response directly with activation note (no mutation)
            let response_json = serde_json::json!({
                "status": result.get("status").and_then(|v| v.as_str()).unwrap_or("connected"),
                "service_name": result.get("service_name").and_then(|v| v.as_str()).unwrap_or(""),
                "connected_at": result.get("connected_at").and_then(|v| v.as_str()).unwrap_or(""),
                "note": "Service tools are now available. Your tool list has been updated.",
            });
            let text = serde_json::to_string_pretty(&response_json).unwrap_or_default();

            // Embed notification inline for SSE-capable clients
            if changed && client_accepts_sse {
                tool_result_with_notifications(
                    request_id,
                    &text,
                    false,
                    vec![serde_json::json!({
                        "jsonrpc": JSONRPC_VERSION,
                        "method": "notifications/tools/list_changed",
                    })],
                )
            } else {
                tool_result(request_id, &text, false)
            }
        }
        Err(e) => {
            tracing::warn!("connect_service failed: {e}");
            let msg = match &e {
                crate::errors::AppError::Internal(_)
                | crate::errors::AppError::DatabaseError(_) => {
                    "Failed to connect to service".to_string()
                }
                other => other.to_string(),
            };
            tool_result(request_id, &msg, true)
        }
    }
}

// ---------------------------------------------------------------------------
// Oracle meta-tool dispatch helpers
// ---------------------------------------------------------------------------

fn oracle_submitter(auth: &McpAuthContext) -> oracle_task_service::SubmitterIdentity {
    oracle_task_service::SubmitterIdentity {
        user_id: auth.user_id.clone(),
        api_key_id: auth.api_key_id.clone(),
        api_key_name: auth.api_key_name.clone(),
    }
}

fn required_arg<'a>(arguments: &'a serde_json::Value, name: &str) -> Result<&'a str, &'static str> {
    arguments
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or("required")
}

fn optional_string_arg(arguments: &serde_json::Value, name: &str) -> Option<String> {
    arguments
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn wait_seconds(
    arguments: &serde_json::Value,
    default_secs: u64,
    min_secs: u64,
    max_secs: u64,
) -> u64 {
    arguments
        .get("wait_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(default_secs)
        .clamp(min_secs, max_secs)
}

async fn poll_oracle_task(
    db: &mongodb::Database,
    actor: &str,
    task_id: &str,
    wait_secs: u64,
) -> crate::errors::AppResult<(crate::models::oracle_task::OracleTask, u64)> {
    let started = tokio::time::Instant::now();
    loop {
        let (task, position) =
            oracle_task_service::get_task_for_consumer(db, actor, task_id).await?;
        if task.status.is_terminal() || started.elapsed() >= Duration::from_secs(wait_secs) {
            return Ok((task, position));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

fn render_oracle_task_result(
    task: &crate::models::oracle_task::OracleTask,
    queue_position: u64,
    pending_message: &str,
) -> (String, bool) {
    match task.status {
        crate::models::oracle_task::OracleTaskStatus::Completed => {
            let mut text = task.response.clone().unwrap_or_default();
            if text.is_empty() {
                text = format!("Task {} completed with an empty response.", task.id);
            }
            (text, false)
        }
        crate::models::oracle_task::OracleTaskStatus::Failed => (
            format!(
                "Task {} failed: {}",
                task.id,
                task.failure_reason
                    .as_deref()
                    .unwrap_or("worker did not provide a failure reason")
            ),
            true,
        ),
        crate::models::oracle_task::OracleTaskStatus::Cancelled => {
            (format!("Task {} was cancelled.", task.id), true)
        }
        crate::models::oracle_task::OracleTaskStatus::Queued
        | crate::models::oracle_task::OracleTaskStatus::Dispatched => {
            let mut details = format!(
                "{pending_message} (status {}, queue_position {}).",
                task.status.as_str(),
                queue_position
            );
            if let Some(phase) = &task.phase {
                details.push_str(&format!(" Phase: {phase}."));
            }
            if let Some(conversation_id) = &task.conversation_id {
                details.push_str(&format!(" conversation_id: {conversation_id}."));
            }
            (details, false)
        }
    }
}

async fn handle_oracle_pools(
    state: &AppState,
    auth: &McpAuthContext,
    request_id: Option<serde_json::Value>,
) -> Response {
    match oracle_pool_service::list_visible_pools(&state.db, &auth.user_id).await {
        Ok(pools) => {
            let mut pool_lines = Vec::new();
            for pool in pools {
                if oracle_pool_service::ensure_can_submit(&state.db, &auth.user_id, &pool)
                    .await
                    .is_err()
                {
                    continue;
                }
                pool_lines.push(format!(
                    "- {} ({}) visibility={} id={}",
                    pool.slug,
                    pool.name,
                    pool.visibility.as_str(),
                    pool.id
                ));
            }
            let mut lines = vec![format!(
                "{} oracle pool(s) available for submission:",
                pool_lines.len()
            )];
            if pool_lines.is_empty() {
                lines.push("No active oracle pools are available for submission.".to_string());
            } else {
                lines.extend(pool_lines);
            }
            tool_result(request_id, &lines.join("\n"), false)
        }
        Err(e) => tool_result(request_id, &format!("Error: {e}"), true),
    }
}

async fn handle_oracle_ask(
    state: &AppState,
    auth: &McpAuthContext,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let pool_ref = match required_arg(arguments, "pool") {
        Ok(v) => v,
        Err(_) => return tool_result(request_id, "\"pool\" is required", true),
    };
    let prompt = match required_arg(arguments, "prompt") {
        Ok(v) => v,
        Err(_) => return tool_result(request_id, "\"prompt\" is required", true),
    };
    let wait_secs = wait_seconds(arguments, 120, 5, 300);

    let result = async {
        let pool = oracle_pool_service::get_pool(&state.db, pool_ref).await?;
        oracle_pool_service::ensure_can_submit(&state.db, &auth.user_id, &pool).await?;
        let outcome = oracle_task_service::submit_task(
            &state.db,
            &pool,
            &oracle_submitter(auth),
            oracle_task_service::SubmitTaskInput {
                prompt: prompt.to_string(),
                model_label: optional_string_arg(arguments, "model"),
                project_url: optional_string_arg(arguments, "project_url"),
                conversation_id: optional_string_arg(arguments, "conversation_id"),
                ..Default::default()
            },
        )
        .await?;
        poll_oracle_task(&state.db, &auth.user_id, &outcome.task.id, wait_secs).await
    }
    .await;

    match result {
        Ok((task, position)) => {
            let (text, is_error) = render_oracle_task_result(
                &task,
                position,
                &format!(
                    "Task {} still processing. Call nyx__oracle_result with this task_id",
                    task.id
                ),
            );
            tool_result(request_id, &text, is_error)
        }
        Err(e) => tool_result(request_id, &format!("Error: {e}"), true),
    }
}

async fn handle_oracle_result(
    state: &AppState,
    auth: &McpAuthContext,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let task_id = match required_arg(arguments, "task_id") {
        Ok(v) => v,
        Err(_) => return tool_result(request_id, "\"task_id\" is required", true),
    };
    let wait_secs = wait_seconds(arguments, 60, 0, 300);

    match poll_oracle_task(&state.db, &auth.user_id, task_id, wait_secs).await {
        Ok((task, position)) => {
            let (text, is_error) = render_oracle_task_result(
                &task,
                position,
                &format!(
                    "Task {} still processing. Call nyx__oracle_result with this task_id",
                    task.id
                ),
            );
            tool_result(request_id, &text, is_error)
        }
        Err(e) => tool_result(request_id, &format!("Error: {e}"), true),
    }
}

async fn handle_oracle_attach(
    state: &AppState,
    auth: &McpAuthContext,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let pool_ref = match required_arg(arguments, "pool") {
        Ok(v) => v,
        Err(_) => return tool_result(request_id, "\"pool\" is required", true),
    };
    let chatgpt_url = match required_arg(arguments, "chatgpt_url") {
        Ok(v) => v,
        Err(_) => return tool_result(request_id, "\"chatgpt_url\" is required", true),
    };

    let result = async {
        let pool = oracle_pool_service::get_pool(&state.db, pool_ref).await?;
        oracle_pool_service::ensure_can_submit(&state.db, &auth.user_id, &pool).await?;
        oracle_task_service::attach_conversation(
            &state.db,
            &pool,
            &oracle_submitter(auth),
            chatgpt_url,
            None,
        )
        .await
    }
    .await;

    match result {
        Ok((session, task)) => {
            let text = format!(
                "Attached conversation.\nconversation_id: {}\nscrape_task_id: {}\nPoll nyx__oracle_result with the scrape task_id, then call nyx__oracle_session with the conversation_id.",
                session.id, task.id
            );
            tool_result(request_id, &text, false)
        }
        Err(e) => tool_result(request_id, &format!("Error: {e}"), true),
    }
}

async fn handle_oracle_extract(
    state: &AppState,
    auth: &McpAuthContext,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let pool_ref = match required_arg(arguments, "pool") {
        Ok(v) => v,
        Err(_) => return tool_result(request_id, "\"pool\" is required", true),
    };
    let url = match required_arg(arguments, "url") {
        Ok(v) => v,
        Err(_) => return tool_result(request_id, "\"url\" is required", true),
    };
    let wait_secs = wait_seconds(arguments, 120, 5, 300);

    let result = async {
        let pool = oracle_pool_service::get_pool(&state.db, pool_ref).await?;
        oracle_pool_service::ensure_can_submit(&state.db, &auth.user_id, &pool).await?;
        let task = oracle_task_service::extract_url(
            &state.db,
            &pool,
            &oracle_submitter(auth),
            url,
            optional_string_arg(arguments, "model"),
        )
        .await?;
        poll_oracle_task(&state.db, &auth.user_id, &task.id, wait_secs).await
    }
    .await;

    match result {
        Ok((task, position)) => {
            let (text, is_error) = render_oracle_task_result(
                &task,
                position,
                &format!(
                    "Task {} still processing. Call nyx__oracle_result with this task_id",
                    task.id
                ),
            );
            tool_result(request_id, &text, is_error)
        }
        Err(e) => tool_result(request_id, &format!("Error: {e}"), true),
    }
}

async fn handle_oracle_session(
    state: &AppState,
    auth: &McpAuthContext,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let conversation_id = match required_arg(arguments, "conversation_id") {
        Ok(v) => v,
        Err(_) => return tool_result(request_id, "\"conversation_id\" is required", true),
    };

    match oracle_session_service::list_session_tasks(&state.db, &auth.user_id, conversation_id)
        .await
    {
        Ok((session, tasks)) => {
            let mut lines = vec![format!(
                "conversation_id: {}\npool_id: {}\nturn_count: {}",
                session.id, session.pool_id, session.turn_count
            )];
            for task in tasks {
                if !task.prompt.trim().is_empty() {
                    lines.push(format!("Q: {}", task.prompt));
                }
                if let Some(response) = task.response.as_deref().filter(|r| !r.trim().is_empty()) {
                    lines.push(format!("A: {response}"));
                } else if !task.status.is_terminal() {
                    lines.push(format!("A: [{}]", task.status.as_str()));
                } else if let Some(reason) = task.failure_reason.as_deref() {
                    lines.push(format!("A: [{}: {}]", task.status.as_str(), reason));
                }
            }
            tool_result(request_id, &lines.join("\n\n"), false)
        }
        Err(e) => tool_result(request_id, &format!("Error: {e}"), true),
    }
}

// ---------------------------------------------------------------------------
// SSH meta-tool dispatch helpers
// ---------------------------------------------------------------------------

/// `nyx__ssh_exec` -- execute a command on a remote SSH service.
async fn handle_mcp_ssh_exec(
    state: &AppState,
    auth: &McpAuthContext,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
) -> Response {
    let service_ref = match arguments.get("service").and_then(|s| s.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return tool_result(request_id, "\"service\" is required (slug or ID)", true),
    };

    let command = match arguments.get("command").and_then(|c| c.as_str()) {
        Some(c) if !c.is_empty() => c,
        _ => return tool_result(request_id, "\"command\" is required", true),
    };

    let principal = arguments
        .get("principal")
        .and_then(|p| p.as_str())
        .unwrap_or("");

    let timeout_secs = arguments
        .get("timeout_secs")
        .and_then(|t| t.as_u64())
        .unwrap_or(30)
        .min(300) as u32;

    // Resolve service by slug or ID
    let service_id =
        match resolve_ssh_service_id(&state.db, auth.user_id.as_str(), service_ref).await {
            Ok(id) => id,
            Err(msg) => return tool_result(request_id, &msg, true),
        };

    // Get SSH config
    let ssh_svc = match ssh_service::get_ssh_service(&state.db, &service_id).await {
        Ok(svc) => svc,
        Err(e) => return tool_result(request_id, &format!("SSH service error: {e}"), true),
    };

    if !ssh_svc.certificate_auth_enabled {
        return tool_result(
            request_id,
            "SSH certificate auth is not enabled for this service",
            true,
        );
    }

    // Resolve principal: use provided or pick first allowed
    let resolved_principal = if principal.is_empty() {
        match ssh_svc.allowed_principals.first() {
            Some(p) => p.clone(),
            None => {
                return tool_result(
                    request_id,
                    "No allowed principals configured and none provided",
                    true,
                );
            }
        }
    } else {
        principal.to_string()
    };

    if !ssh_svc
        .allowed_principals
        .iter()
        .any(|p| p == &resolved_principal)
    {
        return tool_result(
            request_id,
            &format!(
                "Principal '{}' is not allowed. Available: {}",
                resolved_principal,
                ssh_svc.allowed_principals.join(", ")
            ),
            true,
        );
    }

    let service = match fetch_service(state, &service_id).await {
        Ok(service) => service,
        Err(e) => return tool_result(request_id, &format!("SSH service error: {e}"), true),
    };
    let approval_owner_user_id = auth.effective_approval_owner_user_id();
    let service_owner_user_id = match proxy_service::find_effective_service_owner(
        &state.db,
        &approval_owner_user_id,
        None,
        Some(&service_id),
    )
    .await
    {
        Ok(Some(owner)) => owner,
        Ok(None) => approval_owner_user_id.clone(),
        Err(e) => return tool_result(request_id, &format!("SSH service error: {e}"), true),
    };
    let operation = operation_descriptor::build_ssh_descriptor(
        operation_descriptor::SshOperationKind::Exec,
        Some(command),
    );
    if let Err(resp) = authorize_mcp_operation(
        state,
        auth,
        McpApprovalTarget {
            service_id: service_id.clone(),
            service_name: service.name,
            service_slug: service.slug,
            service_owner_user_id,
        },
        &operation,
        request_id.clone(),
    )
    .await
    {
        return resp;
    }

    // Build the request body and call the SSH exec endpoint internally
    let body = super::ssh_exec::SshExecRequest {
        command: command.to_string(),
        principal: resolved_principal.clone(),
        timeout_secs,
    };

    // Reuse the core logic from the ssh_exec module
    let result = execute_ssh_command_internal(state, auth, &service_id, &ssh_svc, &body).await;

    match result {
        Ok(response) => {
            let response_json = serde_json::json!({
                "exit_code": response.exit_code,
                "stdout": response.stdout,
                "stderr": response.stderr,
                "duration_ms": response.duration_ms,
                "timed_out": response.timed_out,
                "service_id": service_id,
                "principal": resolved_principal,
            });
            let text = serde_json::to_string_pretty(&response_json).unwrap_or_default();
            let is_error = response.exit_code != 0;
            tool_result(request_id, &text, is_error)
        }
        Err(e) => tool_result(request_id, &format!("SSH exec failed: {e}"), true),
    }
}

/// `nyx__ssh_list_services` -- list available SSH services.
async fn handle_mcp_ssh_list(
    state: &AppState,
    auth: &McpAuthContext,
    request_id: Option<serde_json::Value>,
) -> Response {
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use futures::TryStreamExt;
    use std::collections::HashMap;

    let user_services =
        match user_service_service::list_user_services(&state.db, &auth.user_id).await {
            Ok(services) => services
                .into_iter()
                .filter(|svc| svc.service_type == "ssh")
                .collect::<Vec<_>>(),
            Err(e) => {
                tracing::error!("Failed to query user SSH services: {e}");
                return tool_result(request_id, "Failed to list SSH services", true);
            }
        };

    let downstream_service_ids: Vec<String> = user_services
        .iter()
        .filter_map(|svc| svc.catalog_service_id.clone())
        .collect();
    let downstream_services: Vec<DownstreamService> = if downstream_service_ids.is_empty() {
        Vec::new()
    } else {
        match state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! {
                "_id": { "$in": &downstream_service_ids },
                "is_active": true,
                "service_type": "ssh",
            })
            .await
        {
            Ok(cursor) => match cursor.try_collect().await {
                Ok(svcs) => svcs,
                Err(e) => {
                    tracing::error!("Failed to query SSH services: {e}");
                    return tool_result(request_id, "Failed to list SSH services", true);
                }
            },
            Err(e) => {
                tracing::error!("Failed to query SSH services: {e}");
                return tool_result(request_id, "Failed to list SSH services", true);
            }
        }
    };
    let downstream_by_id: HashMap<&str, &DownstreamService> = downstream_services
        .iter()
        .map(|svc| (svc.id.as_str(), svc))
        .collect();

    let results: Vec<serde_json::Value> = user_services
        .iter()
        .filter_map(|svc| {
            let downstream_service = svc
                .catalog_service_id
                .as_deref()
                .and_then(|id| downstream_by_id.get(id).copied())?;
            let ssh = downstream_service.ssh_config.as_ref()?;
            Some(serde_json::json!({
                "service_id": downstream_service.id,
                "name": downstream_service.name,
                // User-facing SSH slugs live on `UserService`. The backing
                // `DownstreamService.slug` is internal for new rows.
                "slug": svc.slug,
                "description": downstream_service.description,
                "host": ssh.host,
                "port": ssh.port,
                "certificate_auth_enabled": ssh.certificate_auth_enabled,
                "allowed_principals": ssh.allowed_principals,
            }))
        })
        .collect();

    let count = results.len();
    let response_json = serde_json::json!({
        "services": results,
        "count": count,
    });
    let text = serde_json::to_string_pretty(&response_json).unwrap_or_default();

    audit_service::log_async(
        state.db.clone(),
        Some(auth.user_id.clone()),
        "mcp_ssh_list_services".to_string(),
        Some(serde_json::json!({ "count": count })),
        auth.ip_address.clone(),
        auth.user_agent.clone(),
        auth.api_key_id.clone(),
        auth.api_key_name.clone(),
    );

    tool_result(request_id, &text, false)
}

/// Resolve slug refs through the caller's `UserService` so SSH backing
/// `DownstreamService.slug` can stay internal without cross-user lookups.
async fn resolve_ssh_service_id(
    db: &mongodb::Database,
    user_id: &str,
    service_ref: &str,
) -> Result<String, String> {
    // Try UUID parse first
    if uuid::Uuid::try_parse(service_ref).is_ok() {
        let user_service =
            user_service_service::find_by_catalog_service_id(db, user_id, service_ref)
                .await
                .map_err(|e| format!("Database error: {e}"))?
                .ok_or_else(|| format!("SSH service not found: {service_ref}"))?;

        if user_service.service_type != "ssh" {
            return Err(format!("SSH service not found: {service_ref}"));
        }

        return Ok(service_ref.to_string());
    }

    let service = user_service_service::find_by_slug(db, user_id, service_ref)
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("SSH service not found: {service_ref}"))?;

    if service.service_type != "ssh" {
        return Err(format!("SSH service not found: {service_ref}"));
    }

    service
        .catalog_service_id
        .ok_or_else(|| format!("SSH service not found: {service_ref}"))
}

/// Internal SSH command execution (reusable by REST handler and MCP handler).
/// Routes through the node agent for execution.
async fn execute_ssh_command_internal(
    state: &AppState,
    auth: &McpAuthContext,
    service_id: &str,
    ssh_svc: &crate::models::downstream_service::SshServiceConfig,
    body: &super::ssh_exec::SshExecRequest,
) -> Result<super::ssh_exec::SshExecResponse, crate::errors::AppError> {
    use crate::errors::AppError;
    use crate::services::{node_routing_service, node_service};

    let user_id = auth.user_id.as_str();

    let principal = body.principal.trim();
    let command = body.command.trim();
    let timeout_secs = body.timeout_secs.clamp(1, 300);

    // Validate command
    if command.is_empty() {
        return Err(AppError::ValidationError(
            "command must not be empty".to_string(),
        ));
    }
    if command.len() > 8192 {
        return Err(AppError::ValidationError(
            "command must not exceed 8192 characters".to_string(),
        ));
    }
    super::ssh_exec::check_dangerous_command(command)?;

    // Require a node agent for SSH execution
    let node_route = node_routing_service::resolve_node_route(
        &state.db,
        user_id,
        service_id,
        &state.node_ws_manager,
    )
    .await
    .ok()
    .flatten()
    .ok_or_else(|| {
        AppError::BadRequest(
            "No node agent is bound to this SSH service. \
             Deploy a NyxID node agent and bind it to this service to execute commands."
                .to_string(),
        )
    })?;

    // Session limiting
    let session_guard = state.ssh_session_manager.try_acquire(user_id)?;

    // Generate ephemeral SSH credentials (key + cert as strings, no files)
    let ephemeral = super::ssh_web_terminal::generate_ephemeral_credentials(
        state, ssh_svc, service_id, user_id, principal,
    )
    .await?;

    // Execute via node agent with failover
    let all_node_ids: Vec<&str> = std::iter::once(node_route.node_id.as_str())
        .chain(node_route.fallback_node_ids.iter().map(|id| id.as_str()))
        .collect();

    let request_id = uuid::Uuid::new_v4().to_string();
    let mut last_error = None;

    for node_id in &all_node_ids {
        let signing_secret = if state.config.node_hmac_signing_enabled {
            match node_service::get_node_signing_secret(
                &state.db,
                state.encryption_keys.as_ref(),
                node_id,
            )
            .await
            {
                Ok(secret) => Some(secret),
                Err(error) => {
                    tracing::warn!(
                        service_id = %service_id,
                        node_id = %node_id,
                        error = %error,
                        "MCP SSH exec node signing secret resolution failed"
                    );
                    last_error = Some(format!("Signing secret error: {error}"));
                    continue;
                }
            }
        } else {
            None
        };

        match state
            .node_ws_manager
            .exec_ssh_command(
                node_id,
                crate::services::node_ws_manager::NodeSshExecRequest {
                    request_id: request_id.clone(),
                    host: ssh_svc.host.clone(),
                    port: ssh_svc.port,
                    principal: principal.to_string(),
                    private_key_pem: ephemeral.private_key_pem.clone(),
                    certificate_openssh: ephemeral.certificate_openssh.clone(),
                    command: command.to_string(),
                    timeout_secs,
                },
                signing_secret.as_ref().map(|s| s.as_slice()),
            )
            .await
        {
            Ok(result) => {
                let _ = &session_guard;
                drop(session_guard);

                let response = super::ssh_exec::SshExecResponse {
                    exit_code: result.exit_code,
                    stdout: super::ssh_exec::truncate_output(result.stdout.as_bytes()),
                    stderr: super::ssh_exec::truncate_output(result.stderr.as_bytes()),
                    duration_ms: result.duration_ms,
                    timed_out: result.timed_out,
                };

                // Audit log -- attribute API key when acting as an agent.
                audit_service::log_async(
                    state.db.clone(),
                    Some(auth.user_id.clone()),
                    "ssh_exec_command".to_string(),
                    Some(serde_json::json!({
                        "service_id": service_id,
                        "principal": principal,
                        "command": super::ssh_exec::redact_command_for_audit(command),
                        "exit_code": response.exit_code,
                        "duration_ms": response.duration_ms,
                        "timed_out": response.timed_out,
                        "via": "mcp",
                        "routed_via": "node",
                        "node_id": node_id,
                    })),
                    auth.ip_address.clone(),
                    auth.user_agent.clone(),
                    auth.api_key_id.clone(),
                    auth.api_key_name.clone(),
                );

                return Ok(response);
            }
            Err(error) => {
                tracing::warn!(
                    service_id = %service_id,
                    node_id = %node_id,
                    error = %error,
                    "MCP SSH exec via node failed, trying next"
                );
                last_error = Some(error.to_string());
            }
        }
    }

    let _ = &session_guard;
    drop(session_guard);

    Err(AppError::Internal(format!(
        "SSH exec failed on all nodes: {}",
        last_error.unwrap_or_else(|| "no nodes available".to_string()),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::services::mcp_service::{McpToolService, McpToolSource};
    use crate::test_utils::{connect_test_database, test_user_service};

    fn api_key_auth(allowed_service_ids: Vec<String>) -> McpAuthContext {
        McpAuthContext {
            user_id: "user-1".into(),
            auth_method: AuthMethod::ApiKey,
            acting_client_id: None,
            approval_owner_user_id: None,
            is_api_key: true,
            api_key_id: Some("key-1".into()),
            api_key_name: Some("agent".into()),
            allow_all_services: false,
            allow_all_nodes: false,
            allowed_service_ids,
            allowed_node_ids: Vec::new(),
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        }
    }

    fn user_managed(id: &str) -> McpToolService {
        McpToolService {
            service_id: id.into(),
            service_name: id.into(),
            service_slug: id.into(),
            description: None,
            service_category: "user_service".into(),
            endpoints: Vec::new(),
            source: McpToolSource::UserManaged {
                user_service_id: id.into(),
                effective_owner_id: "user-1".into(),
                node_id: None,
                has_server_credential: true,
            },
            is_generic_proxy: false,
        }
    }

    fn platform(id: &str) -> McpToolService {
        McpToolService {
            service_id: id.into(),
            service_name: id.into(),
            service_slug: id.into(),
            description: None,
            service_category: "http".into(),
            endpoints: Vec::new(),
            source: McpToolSource::Platform {
                downstream_service_id: id.into(),
            },
            is_generic_proxy: false,
        }
    }

    #[test]
    fn filter_keeps_user_services_in_allow_list() {
        let auth = api_key_auth(vec!["svc-a".into()]);
        let services = vec![user_managed("svc-a"), user_managed("svc-b")];
        let filtered = filter_services_by_scope(services, &auth);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].service_id, "svc-a");
    }

    #[test]
    fn filter_drops_platform_services_for_scoped_keys() {
        let auth = api_key_auth(vec!["svc-a".into()]);
        let services = vec![user_managed("svc-a"), platform("svc-a")];
        let filtered = filter_services_by_scope(services, &auth);
        // The platform-sourced service with the same id is dropped -- scoped
        // keys cannot call platform services through MCP.
        assert!(
            filtered
                .iter()
                .all(|s| matches!(s.source, McpToolSource::UserManaged { .. }))
        );
    }

    #[test]
    fn filter_is_noop_for_unrestricted_auth() {
        let auth = McpAuthContext::user("user-1".into(), AuthMethod::Session);
        let services = vec![user_managed("svc-a"), platform("svc-b")];
        assert_eq!(filter_services_by_scope(services, &auth).len(), 2);
    }

    #[test]
    fn ensure_scope_rejects_disallowed_user_service() {
        let auth = api_key_auth(vec!["svc-a".into()]);
        let svc = user_managed("svc-b");
        let res = ensure_service_in_scope(&auth, &svc, None);
        assert!(res.is_err());
    }

    #[test]
    fn ensure_scope_allows_unrestricted_auth() {
        let auth = McpAuthContext::user("user-1".into(), AuthMethod::Session);
        let svc = platform("svc-x");
        assert!(ensure_service_in_scope(&auth, &svc, None).is_ok());
    }

    #[test]
    fn approval_target_for_user_managed_service_uses_effective_owner() {
        let auth = McpAuthContext::user("actor-1".into(), AuthMethod::AccessToken);
        let mut svc = user_managed("svc-a");
        svc.source = McpToolSource::UserManaged {
            user_service_id: "svc-a".into(),
            effective_owner_id: "org-1".into(),
            node_id: None,
            has_server_credential: true,
        };

        let target = approval_target_for_tool(&auth, &svc);

        assert_eq!(target.service_id, "svc-a");
        assert_eq!(target.service_owner_user_id, "org-1");
    }

    #[test]
    fn approval_target_for_platform_service_uses_approval_owner() {
        let mut auth = McpAuthContext::user("sa-1".into(), AuthMethod::ServiceAccount);
        auth.approval_owner_user_id = Some("owner-1".into());
        let svc = platform("svc-a");

        let target = approval_target_for_tool(&auth, &svc);

        assert_eq!(target.service_id, "svc-a");
        assert_eq!(target.service_owner_user_id, "owner-1");
    }

    #[test]
    fn scoped_api_key_detection() {
        // No restrictions -> not scoped.
        let mut auth = api_key_auth(Vec::new());
        auth.allow_all_services = true;
        auth.allow_all_nodes = true;
        assert!(!is_scoped_api_key(&auth));

        // Service allow-list -> scoped.
        auth.allow_all_services = false;
        assert!(is_scoped_api_key(&auth));

        // Node allow-list -> scoped.
        auth.allow_all_services = true;
        auth.allow_all_nodes = false;
        assert!(is_scoped_api_key(&auth));

        // OAuth/session auth is never "scoped" in this sense.
        let oauth = McpAuthContext::user("user-1".into(), AuthMethod::Session);
        assert!(!is_scoped_api_key(&oauth));
    }

    #[tokio::test]
    async fn resolve_ssh_service_id_scopes_slug_lookup_to_user() {
        let Some(db) = connect_test_database("mcp_ssh_resolve").await else {
            eprintln!("skipping mcp_transport integration test: no local MongoDB available");
            return;
        };

        let downstream_service_id = uuid::Uuid::new_v4().to_string();
        let mut ssh_service = test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            "user-a",
            "shared-label",
            "ep-1",
            Some(&downstream_service_id),
            None,
        );
        ssh_service.service_type = "ssh".to_string();

        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&ssh_service)
            .await
            .unwrap();

        let resolved = resolve_ssh_service_id(&db, "user-a", "shared-label")
            .await
            .expect("owner should resolve SSH slug");
        assert_eq!(resolved, downstream_service_id);

        let err = resolve_ssh_service_id(&db, "user-b", "shared-label")
            .await
            .expect_err("other users should not resolve someone else's SSH slug");
        assert_eq!(err, "SSH service not found: shared-label");
    }

    #[test]
    fn accepts_sse_returns_true_for_event_stream() {
        let mut headers = HeaderMap::new();
        headers.insert("accept", "text/event-stream".parse().unwrap());
        assert!(accepts_sse(&headers));
    }

    #[test]
    fn accepts_sse_returns_false_for_json() {
        let mut headers = HeaderMap::new();
        headers.insert("accept", "application/json".parse().unwrap());
        assert!(!accepts_sse(&headers));
    }

    #[test]
    fn accepts_sse_returns_false_for_missing() {
        let headers = HeaderMap::new();
        assert!(!accepts_sse(&headers));
    }

    #[test]
    fn rpc_success_returns_valid_json_rpc_response() {
        let response = rpc_success(Some(serde_json::json!(1)), serde_json::json!({"ok": true}));
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn rpc_error_returns_valid_json_rpc_error() {
        let response = rpc_error(Some(serde_json::json!(1)), -32600, "Invalid Request");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn mcp_auth_context_user_has_unrestricted_access() {
        let ctx = McpAuthContext::user("user-1".to_string(), AuthMethod::Session);
        assert!(!ctx.is_api_key);
        assert!(ctx.allow_all_services);
        assert!(ctx.allow_all_nodes);
        assert!(ctx.api_key_id.is_none());
    }

    #[test]
    fn mcp_extract_ip_prefers_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
        headers.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        assert_eq!(mcp_extract_ip(&headers).as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn mcp_extract_ip_falls_back_to_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "9.9.9.9".parse().unwrap());
        assert_eq!(mcp_extract_ip(&headers).as_deref(), Some("9.9.9.9"));
    }

    #[test]
    fn mcp_extract_ip_returns_none_when_absent() {
        let headers = HeaderMap::new();
        assert!(mcp_extract_ip(&headers).is_none());
    }

    #[tokio::test]
    async fn resolve_ssh_service_id_scopes_uuid_lookup_to_user() {
        let Some(db) = connect_test_database("mcp_ssh_resolve").await else {
            eprintln!("skipping mcp_transport integration test: no local MongoDB available");
            return;
        };

        let downstream_service_id = uuid::Uuid::new_v4().to_string();
        let mut ssh_service = test_user_service(
            &uuid::Uuid::new_v4().to_string(),
            "user-a",
            "shared-label",
            "ep-1",
            Some(&downstream_service_id),
            None,
        );
        ssh_service.service_type = "ssh".to_string();

        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&ssh_service)
            .await
            .unwrap();

        let resolved = resolve_ssh_service_id(&db, "user-a", &downstream_service_id)
            .await
            .expect("owner should resolve SSH service UUID");
        assert_eq!(resolved, downstream_service_id);

        let err = resolve_ssh_service_id(&db, "user-b", &downstream_service_id)
            .await
            .expect_err("other users should not resolve someone else's SSH UUID");
        assert_eq!(
            err,
            format!("SSH service not found: {downstream_service_id}")
        );
    }

    // -----------------------------------------------------------------------
    // tool_result tests
    // -----------------------------------------------------------------------

    #[test]
    fn tool_result_success_returns_ok_status() {
        let resp = tool_result(Some(serde_json::json!(42)), "all good", false);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn tool_result_error_returns_ok_status_with_is_error() {
        // MCP conveys errors through isError, not HTTP status
        let resp = tool_result(Some(serde_json::json!(42)), "boom", true);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn tool_result_with_null_id() {
        let resp = tool_result(None, "test", false);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // mcp_401 tests
    // -----------------------------------------------------------------------

    #[test]
    fn mcp_401_returns_unauthorized_status() {
        let resp = mcp_401("https://auth.example.com");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn mcp_401_includes_www_authenticate_header() {
        let resp = mcp_401("https://auth.example.com/");
        let header = resp
            .headers()
            .get("www-authenticate")
            .expect("missing www-authenticate");
        let value = header.to_str().unwrap();
        assert!(value.contains("Bearer"));
        assert!(value.contains("/.well-known/oauth-protected-resource"));
    }

    #[test]
    fn mcp_401_strips_trailing_slash_from_base_url() {
        let resp = mcp_401("https://auth.example.com/");
        let header = resp
            .headers()
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap();
        // Should not produce double slash before .well-known
        assert!(!header.contains("//."));
        assert!(header.contains("https://auth.example.com/.well-known/oauth-protected-resource"));
    }

    #[test]
    fn mcp_401_includes_json_content_type() {
        let resp = mcp_401("https://example.com");
        let ct = resp
            .headers()
            .get("content-type")
            .expect("missing content-type");
        assert_eq!(ct.to_str().unwrap(), "application/json");
    }

    // -----------------------------------------------------------------------
    // mcp_403 variants tests
    // -----------------------------------------------------------------------

    #[test]
    fn mcp_403_insufficient_scope_returns_forbidden() {
        let resp = mcp_403_insufficient_scope();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn mcp_403_api_key_insufficient_scope_returns_forbidden() {
        let resp = mcp_403_api_key_insufficient_scope();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -----------------------------------------------------------------------
    // rpc_scope_forbidden tests
    // -----------------------------------------------------------------------

    #[test]
    fn rpc_scope_forbidden_returns_ok_status() {
        // JSON-RPC errors use 200 OK for the HTTP layer
        let resp = rpc_scope_forbidden(Some(serde_json::json!(5)), "not allowed");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn rpc_scope_forbidden_with_none_id() {
        let resp = rpc_scope_forbidden(None, "scope missing");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // mcp_extract_user_agent tests
    // -----------------------------------------------------------------------

    #[test]
    fn mcp_extract_user_agent_returns_ua() {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::USER_AGENT, "MyCLI/1.0".parse().unwrap());
        assert_eq!(
            mcp_extract_user_agent(&headers).as_deref(),
            Some("MyCLI/1.0")
        );
    }

    #[test]
    fn mcp_extract_user_agent_returns_none_when_absent() {
        let headers = HeaderMap::new();
        assert!(mcp_extract_user_agent(&headers).is_none());
    }

    // -----------------------------------------------------------------------
    // mcp_extract_ip edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn mcp_extract_ip_ignores_empty_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "".parse().unwrap());
        headers.insert("x-real-ip", "10.0.0.1".parse().unwrap());
        assert_eq!(mcp_extract_ip(&headers).as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn mcp_extract_ip_trims_whitespace_in_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "  8.8.8.8  ".parse().unwrap());
        assert_eq!(mcp_extract_ip(&headers).as_deref(), Some("8.8.8.8"));
    }

    #[test]
    fn mcp_extract_ip_single_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert_eq!(mcp_extract_ip(&headers).as_deref(), Some("203.0.113.50"));
    }

    // -----------------------------------------------------------------------
    // require_session tests
    // -----------------------------------------------------------------------

    #[test]
    fn require_session_returns_session_id_when_present() {
        let mut headers = HeaderMap::new();
        headers.insert("mcp-session-id", "sess-abc-123".parse().unwrap());
        let sid = require_session(&headers).expect("should succeed");
        assert_eq!(sid, "sess-abc-123");
    }

    #[test]
    fn require_session_returns_error_when_missing() {
        let headers = HeaderMap::new();
        let err = require_session(&headers);
        assert!(err.is_err());
    }

    // -----------------------------------------------------------------------
    // app_error_to_rpc tests
    // -----------------------------------------------------------------------

    #[test]
    fn app_error_to_rpc_handles_rate_limited() {
        let resp = app_error_to_rpc(
            Some(serde_json::json!(1)),
            &crate::errors::AppError::RateLimited,
        );
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn app_error_to_rpc_handles_scope_forbidden() {
        let resp = app_error_to_rpc(
            Some(serde_json::json!(2)),
            &crate::errors::AppError::ApiKeyScopeForbidden("nope".into()),
        );
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn app_error_to_rpc_handles_generic_error() {
        let resp = app_error_to_rpc(None, &crate::errors::AppError::Internal("unknown".into()));
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // mcp_exec_context tests
    // -----------------------------------------------------------------------

    #[test]
    fn mcp_exec_context_from_api_key_auth() {
        let auth = api_key_auth(vec!["svc-1".into()]);
        let ctx = mcp_exec_context(&auth);
        assert_eq!(ctx.api_key_id, Some("key-1"));
        assert!(!ctx.allow_all_nodes);
        assert!(ctx.allowed_node_ids.is_empty());
    }

    #[test]
    fn mcp_exec_context_from_user_auth() {
        let auth = McpAuthContext::user("user-1".into(), AuthMethod::Session);
        let ctx = mcp_exec_context(&auth);
        assert!(ctx.api_key_id.is_none());
        assert!(ctx.allow_all_nodes);
    }

    // -----------------------------------------------------------------------
    // mcp_node_scope tests
    // -----------------------------------------------------------------------

    #[test]
    fn mcp_node_scope_unrestricted_for_user_auth() {
        let auth = McpAuthContext::user("user-1".into(), AuthMethod::Session);
        assert!(matches!(
            mcp_node_scope(&auth),
            crate::services::mcp_service::NodeScope::Unrestricted
        ));
    }

    #[test]
    fn mcp_node_scope_unrestricted_when_allow_all_nodes() {
        let mut auth = api_key_auth(Vec::new());
        auth.allow_all_nodes = true;
        assert!(matches!(
            mcp_node_scope(&auth),
            crate::services::mcp_service::NodeScope::Unrestricted
        ));
    }

    #[test]
    fn mcp_node_scope_allowed_when_scoped_nodes() {
        let mut auth = api_key_auth(Vec::new());
        auth.allow_all_nodes = false;
        auth.allowed_node_ids = vec!["node-a".into(), "node-b".into()];
        match mcp_node_scope(&auth) {
            crate::services::mcp_service::NodeScope::Allowed(ids) => {
                assert_eq!(ids.len(), 2);
                assert_eq!(ids[0], "node-a");
                assert_eq!(ids[1], "node-b");
            }
            _ => panic!("expected NodeScope::Allowed"),
        }
    }

    // -----------------------------------------------------------------------
    // accepts_sse extended tests
    // -----------------------------------------------------------------------

    #[test]
    fn accepts_sse_returns_true_for_mixed_accept_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "accept",
            "application/json, text/event-stream".parse().unwrap(),
        );
        assert!(accepts_sse(&headers));
    }

    // -----------------------------------------------------------------------
    // rpc_success / rpc_error with various id types
    // -----------------------------------------------------------------------

    #[test]
    fn rpc_success_with_string_id() {
        let resp = rpc_success(Some(serde_json::json!("abc")), serde_json::json!(null));
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn rpc_success_with_null_id() {
        let resp = rpc_success(None, serde_json::json!({"data": [1,2,3]}));
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn rpc_error_with_null_id() {
        let resp = rpc_error(None, -32700, "Parse error");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // McpAuthContext::user field defaults
    // -----------------------------------------------------------------------

    #[test]
    fn mcp_auth_context_user_defaults() {
        let ctx = McpAuthContext::user("u-xyz".to_string(), AuthMethod::Session);
        assert_eq!(ctx.user_id, "u-xyz");
        assert_eq!(ctx.auth_method, AuthMethod::Session);
        assert!(ctx.acting_client_id.is_none());
        assert!(ctx.approval_owner_user_id.is_none());
        assert!(!ctx.is_api_key);
        assert!(ctx.api_key_id.is_none());
        assert!(ctx.api_key_name.is_none());
        assert!(ctx.allow_all_services);
        assert!(ctx.allow_all_nodes);
        assert!(ctx.allowed_service_ids.is_empty());
        assert!(ctx.allowed_node_ids.is_empty());
        assert!(ctx.rate_limit_per_second.is_none());
        assert!(ctx.rate_limit_burst.is_none());
        assert!(ctx.ip_address.is_none());
        assert!(ctx.user_agent.is_none());
    }

    #[test]
    fn mcp_auth_context_requester_identity_matches_auth_method() {
        let mut delegated = McpAuthContext::user("user-1".into(), AuthMethod::Delegated);
        delegated.acting_client_id = Some("client-1".into());
        assert_eq!(delegated.approval_requester_type(), Some("delegated"));
        assert_eq!(delegated.approval_requester_id(), "client-1");

        let session = McpAuthContext::user("user-1".into(), AuthMethod::Session);
        assert_eq!(session.approval_requester_type(), None);
        assert_eq!(session.approval_requester_id(), "user-1");
    }

    // -----------------------------------------------------------------------
    // is_scoped_api_key extended edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn is_scoped_api_key_false_for_non_api_key_even_with_restrictions() {
        // User auth (not API key) is never "scoped" even with mismatched fields
        let mut auth = McpAuthContext::user("u-1".into(), AuthMethod::Session);
        auth.allow_all_services = false;
        auth.allow_all_nodes = false;
        assert!(!is_scoped_api_key(&auth));
    }

    #[test]
    fn is_scoped_api_key_both_restricted() {
        let mut auth = api_key_auth(vec!["svc-1".into()]);
        auth.allow_all_services = false;
        auth.allow_all_nodes = false;
        assert!(is_scoped_api_key(&auth));
    }

    // -----------------------------------------------------------------------
    // filter_services_by_scope extended tests
    // -----------------------------------------------------------------------

    #[test]
    fn filter_services_by_scope_empty_allow_list_drops_everything() {
        let auth = api_key_auth(Vec::new());
        let services = vec![
            user_managed("svc-a"),
            user_managed("svc-b"),
            platform("svc-c"),
        ];
        let filtered = filter_services_by_scope(services, &auth);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_services_by_scope_preserves_order() {
        let auth = api_key_auth(vec!["svc-b".into(), "svc-a".into()]);
        let services = vec![
            user_managed("svc-a"),
            user_managed("svc-b"),
            user_managed("svc-c"),
        ];
        let filtered = filter_services_by_scope(services, &auth);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].service_id, "svc-a");
        assert_eq!(filtered[1].service_id, "svc-b");
    }

    // -----------------------------------------------------------------------
    // ensure_service_in_scope extended tests
    // -----------------------------------------------------------------------

    #[test]
    fn ensure_scope_allows_user_managed_in_list() {
        let auth = api_key_auth(vec!["svc-a".into()]);
        let svc = user_managed("svc-a");
        assert!(ensure_service_in_scope(&auth, &svc, Some(serde_json::json!(1))).is_ok());
    }

    #[test]
    fn ensure_scope_rejects_platform_service_for_scoped_key() {
        let auth = api_key_auth(vec!["svc-a".into()]);
        let svc = platform("svc-a");
        assert!(ensure_service_in_scope(&auth, &svc, None).is_err());
    }

    // -----------------------------------------------------------------------
    // tool_result_with_notifications tests
    // -----------------------------------------------------------------------

    #[test]
    fn tool_result_with_notifications_returns_ok() {
        let notifications = vec![serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed",
        })];
        let resp = tool_result_with_notifications(
            Some(serde_json::json!(1)),
            "connected",
            false,
            notifications,
        );
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn tool_result_with_empty_notifications() {
        let resp =
            tool_result_with_notifications(Some(serde_json::json!(1)), "data", false, Vec::new());
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // SSH_META_TOOL_NAMES constant tests
    // -----------------------------------------------------------------------

    #[test]
    fn ssh_meta_tool_names_contains_expected_tools() {
        assert!(SSH_META_TOOL_NAMES.contains(&"nyx__ssh_exec"));
        assert!(SSH_META_TOOL_NAMES.contains(&"nyx__ssh_list_services"));
        assert!(!SSH_META_TOOL_NAMES.contains(&"nyx__call_tool"));
    }

    // -----------------------------------------------------------------------
    // JSONRPC_VERSION / MCP_PROTOCOL_VERSION constants
    // -----------------------------------------------------------------------

    #[test]
    fn jsonrpc_version_is_2_0() {
        assert_eq!(JSONRPC_VERSION, "2.0");
    }

    #[test]
    fn mcp_protocol_version_is_set() {
        assert!(!MCP_PROTOCOL_VERSION.is_empty());
    }
}
