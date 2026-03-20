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
use crate::models::mcp_session;
use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::services::{audit_service, mcp_service, ssh_service};

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

// ---------------------------------------------------------------------------
// Auth helper (manual token validation, NOT AuthUser extractor)
// ---------------------------------------------------------------------------

/// Extract and validate the Bearer token, returning the user_id string.
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
) -> Result<String, Response> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    // --- Try JWT-based auth first ---
    if let Some(token) = token {
        match jwt::verify_token(&state.jwt_keys, &state.config, token) {
            Ok(claims) if claims.token_type == "access" => {
                // Service account tokens have sa=true; verify against
                // the service_accounts collection instead of users.
                if claims.sa == Some(true) {
                    return verify_service_account_active(state, claims.sub).await;
                }
                return verify_user_active(state, claims.sub).await;
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
        return verify_user_active(state, user_id).await;
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
) -> Result<String, Response> {
    let sa = state
        .db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "_id": &sa_id, "is_active": true })
        .await
        .map_err(|_| rpc_error(None, -32603, "Internal error"))?;

    match sa {
        Some(_) => Ok(sa_id),
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

    // `initialize` requires a valid JWT (no session exists yet).
    // All other methods allow session-based auth fallback.
    let is_initialize = request.method == "initialize";
    let user_id = match authenticate_mcp(&state, &headers, !is_initialize).await {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    match request.method.as_str() {
        "initialize" => handle_initialize(&state, &user_id, &request),

        "notifications/initialized" => {
            if let Ok(sid) = require_session(&headers) {
                state.mcp_sessions.touch(&sid);
            }
            StatusCode::ACCEPTED.into_response()
        }

        "tools/list" => {
            let sid = match require_session(&headers) {
                Ok(s) => s,
                Err(r) => return r,
            };
            if let Err(r) = validate_session(&state, &sid, &user_id, request.id.clone()) {
                return r;
            }
            handle_tools_list(&state, &user_id, &sid, &request).await
        }

        "tools/call" => {
            let sid = match require_session(&headers) {
                Ok(s) => s,
                Err(r) => return r,
            };
            if let Err(r) = validate_session(&state, &sid, &user_id, request.id.clone()) {
                return r;
            }
            let sse_capable = accepts_sse(&headers);
            handle_tools_call(&state, &user_id, &sid, &request, sse_capable).await
        }

        "ping" => rpc_success(request.id, serde_json::json!({})),

        _ => rpc_error(request.id, -32601, "Method not found"),
    }
}

// ---------------------------------------------------------------------------
// GET /mcp -- SSE notification stream
// ---------------------------------------------------------------------------

pub async fn mcp_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user_id = match authenticate_mcp(&state, &headers, true).await {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

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
    let user_id = match authenticate_mcp(&state, &headers, true).await {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };

    let sid = match require_session(&headers) {
        Ok(s) => s,
        Err(r) => return r,
    };

    if let Err(r) = validate_session(&state, &sid, &user_id, None) {
        return r;
    }

    state.mcp_sessions.remove(&sid);

    // Audit log for session deletion
    audit_service::log_async(
        state.db.clone(),
        Some(user_id.to_string()),
        "mcp_session_deleted".to_string(),
        Some(serde_json::json!({ "session_id": &sid })),
        None,
        None,
    );

    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// Method handlers
// ---------------------------------------------------------------------------

fn handle_initialize(state: &AppState, user_id: &str, request: &JsonRpcRequest) -> Response {
    let session_id = match state.mcp_sessions.create(user_id) {
        Some(id) => id,
        None => return rpc_error(request.id.clone(), -32000, "Too many active MCP sessions"),
    };

    // Audit log for session creation
    audit_service::log_async(
        state.db.clone(),
        Some(user_id.to_string()),
        "mcp_session_created".to_string(),
        Some(serde_json::json!({ "session_id": &session_id })),
        None,
        None,
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

    let header_value = match axum::http::HeaderValue::from_str(&session_id) {
        Ok(v) => v,
        Err(_) => {
            return rpc_error(
                request.id.clone(),
                -32603,
                "Failed to create session header",
            );
        }
    };

    let mut response = axum::Json(body).into_response();
    response.headers_mut().insert(
        axum::http::HeaderName::from_static("mcp-session-id"),
        header_value,
    );
    response
}

async fn handle_tools_list(
    state: &AppState,
    user_id: &str,
    session_id: &str,
    request: &JsonRpcRequest,
) -> Response {
    let services = match mcp_service::load_user_tools(
        &state.db,
        state.node_ws_manager.as_ref(),
        user_id,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load user tools: {e}");
            return rpc_error(request.id.clone(), -32603, "Failed to load tools");
        }
    };

    // Get activated service IDs for this session
    let activated = state.mcp_sessions.get_activated_service_ids(session_id);

    // Generate only meta-tools + activated service tools
    let tool_defs = mcp_service::generate_tool_definitions(&services, Some(&activated));

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
    user_id: &str,
    session_id: &str,
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
                user_id,
                session_id,
                &arguments,
                request.id.clone(),
                client_accepts_sse,
            )
            .await;
        }
        "nyx__discover_services" => {
            return handle_meta_discover(state, user_id, &arguments, request.id.clone()).await;
        }
        "nyx__connect_service" => {
            return handle_meta_connect(
                state,
                user_id,
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
                user_id,
                session_id,
                &arguments,
                request.id.clone(),
                client_accepts_sse,
            )
            .await;
        }
        "nyx__ssh_exec" => {
            return handle_mcp_ssh_exec(state, user_id, &arguments, request.id.clone()).await;
        }
        "nyx__ssh_list_services" => {
            return handle_mcp_ssh_list(state, user_id, request.id.clone()).await;
        }
        _ => {}
    }

    // -- Service tool: verify activation, load, resolve, execute --
    let activated = state.mcp_sessions.get_activated_service_ids(session_id);

    let services = match mcp_service::load_user_tools(
        &state.db,
        state.node_ws_manager.as_ref(),
        user_id,
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

    // Guard: only allow execution if the service is activated
    if !activated.contains(&service.service_id) {
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

    let (status, body) = match mcp_service::execute_tool(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        user_id,
        service,
        endpoint,
        &arguments,
        &state.jwt_keys,
        &state.config,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Tool execution failed for {tool_name}: {e}");
            return tool_result(
                request.id.clone(),
                &format!("Tool execution failed: {e}"),
                true,
            );
        }
    };

    // Audit log
    audit_service::log_async(
        state.db.clone(),
        Some(user_id.to_string()),
        "mcp_tool_call".to_string(),
        Some(serde_json::json!({
            "tool": tool_name,
            "service_id": service.service_id,
            "response_status": status,
        })),
        None,
        None,
    );

    let is_error = !(200..300).contains(&status);
    let content_text = if is_error {
        format!("Error ({status}): {body}")
    } else {
        body
    };

    tool_result(request.id.clone(), &content_text, is_error)
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
    user_id: &str,
    session_id: &str,
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

    // Load user tools
    let services = match mcp_service::load_user_tools(
        &state.db,
        state.node_ws_manager.as_ref(),
        user_id,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load tools for call_tool: {e}");
            return tool_result(request_id, "Failed to load tools", true);
        }
    };

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

    // Auto-activate so future tools/list responses include this service
    let changed = state
        .mcp_sessions
        .activate_services(session_id, std::slice::from_ref(&service.service_id));

    if changed {
        send_tools_list_changed(state, session_id);
    }

    // Execute
    let (status, body) = match mcp_service::execute_tool(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        user_id,
        service,
        endpoint,
        &inner_args,
        &state.jwt_keys,
        &state.config,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Tool execution failed for {tool_name}: {e}");
            return tool_result(request_id, &format!("Tool execution failed: {e}"), true);
        }
    };

    // Audit log
    audit_service::log_async(
        state.db.clone(),
        Some(user_id.to_string()),
        "mcp_tool_call".to_string(),
        Some(serde_json::json!({
            "tool": tool_name,
            "service_id": service.service_id,
            "response_status": status,
            "via": "nyx__call_tool",
        })),
        None,
        None,
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
    user_id: &str,
    session_id: &str,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
    client_accepts_sse: bool,
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

    // Load ALL user tools (not just activated)
    let services = match mcp_service::load_user_tools(
        &state.db,
        state.node_ws_manager.as_ref(),
        user_id,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to load tools for search: {e}");
            return tool_result(request_id, "Failed to load tools", true);
        }
    };

    // Search across ALL tools
    let search_result = mcp_service::search_all_tools(&services, query);

    // Activate the services that had matches
    let changed = state
        .mcp_sessions
        .activate_services(session_id, &search_result.matched_service_ids);

    // Send notification via GET SSE channel (fallback for clients that have it)
    if changed {
        send_tools_list_changed(state, session_id);
    }

    // Return search results (include inputSchema so the AI knows what arguments
    // to pass when calling tools via nyx__call_tool)
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

    let activated_count = state
        .mcp_sessions
        .get_activated_service_ids(session_id)
        .len();

    let mut response_json = serde_json::json!({
        "matches": results,
        "count": results.len(),
        "services_activated": search_result.matched_service_ids.len(),
        "total_activated_services": activated_count,
        "hint": "Use nyx__call_tool to invoke any of these tools by name. \
            Pass the tool name and arguments as shown in the match results.",
        "note": if changed {
            "Matching service tools have been activated. Your tool list has been updated."
        } else {
            "Tools were already activated."
        },
    });

    if activated_count >= mcp_session::MAX_ACTIVATED_SERVICES {
        response_json.as_object_mut().unwrap().insert(
            "max_activated_services_warning".to_string(),
            serde_json::Value::String(
                "Maximum activated services reached. Some tools may not have been activated."
                    .to_string(),
            ),
        );
    }

    let text = serde_json::to_string_pretty(&response_json).unwrap_or_default();

    // When tools changed and client supports SSE, embed the notification
    // inline in the POST response so the client picks it up without needing
    // the separate GET SSE channel.
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
    user_id: &str,
    session_id: &str,
    arguments: &serde_json::Value,
    request_id: Option<serde_json::Value>,
    client_accepts_sse: bool,
) -> Response {
    let service_id = match arguments.get("service_id").and_then(|s| s.as_str()) {
        Some(id) if uuid::Uuid::try_parse(id).is_ok() => id,
        Some(_) => return tool_result(request_id, "Invalid service_id format", true),
        None => return tool_result(request_id, "service_id is required", true),
    };
    let credential = arguments.get("credential").and_then(|c| c.as_str());
    let credential_label = arguments.get("credential_label").and_then(|l| l.as_str());

    match mcp_service::connect_service(
        &state.db,
        &state.encryption_keys,
        state.node_ws_manager.as_ref(),
        user_id,
        service_id,
        credential,
        credential_label,
    )
    .await
    {
        Ok(result) => {
            // Activate the newly connected service
            let changed = state
                .mcp_sessions
                .activate_services(session_id, &[service_id.to_string()]);

            // Send via GET SSE channel (fallback for clients that have it)
            if changed {
                send_tools_list_changed(state, session_id);
            }

            audit_service::log_async(
                state.db.clone(),
                Some(user_id.to_string()),
                "mcp_connect_service".to_string(),
                Some(serde_json::json!({ "service_id": service_id })),
                None,
                None,
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
// SSH meta-tool dispatch helpers
// ---------------------------------------------------------------------------

/// `nyx__ssh_exec` -- execute a command on a remote SSH service.
async fn handle_mcp_ssh_exec(
    state: &AppState,
    user_id: &str,
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
    let service_id = match resolve_ssh_service_id(&state.db, service_ref).await {
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

    // Build the request body and call the SSH exec endpoint internally
    let body = super::ssh_exec::SshExecRequest {
        command: command.to_string(),
        principal: resolved_principal.clone(),
        timeout_secs,
    };

    // Reuse the core logic from the ssh_exec module
    let result = execute_ssh_command_internal(state, user_id, &service_id, &ssh_svc, &body).await;

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
    user_id: &str,
    request_id: Option<serde_json::Value>,
) -> Response {
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use futures::TryStreamExt;

    let filter = doc! {
        "is_active": true,
        "service_type": "ssh",
    };

    let services: Vec<DownstreamService> = match state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
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
    };

    let results: Vec<serde_json::Value> = services
        .iter()
        .filter_map(|svc| {
            let ssh = svc.ssh_config.as_ref()?;
            Some(serde_json::json!({
                "service_id": svc.id,
                "name": svc.name,
                "slug": svc.slug,
                "description": svc.description,
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
        Some(user_id.to_string()),
        "mcp_ssh_list_services".to_string(),
        Some(serde_json::json!({ "count": count })),
        None,
        None,
    );

    tool_result(request_id, &text, false)
}

/// Resolve an SSH service by slug or UUID ID.
async fn resolve_ssh_service_id(
    db: &mongodb::Database,
    service_ref: &str,
) -> Result<String, String> {
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };

    // Try UUID parse first
    if uuid::Uuid::try_parse(service_ref).is_ok() {
        return Ok(service_ref.to_string());
    }

    // Otherwise resolve by slug
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": service_ref, "service_type": "ssh", "is_active": true })
        .await
        .map_err(|e| format!("Database error: {e}"))?
        .ok_or_else(|| format!("SSH service not found: {service_ref}"))?;

    Ok(service.id)
}

/// Internal SSH command execution (reusable by REST handler and MCP handler).
async fn execute_ssh_command_internal(
    state: &AppState,
    user_id: &str,
    service_id: &str,
    ssh_svc: &crate::models::downstream_service::SshServiceConfig,
    body: &super::ssh_exec::SshExecRequest,
) -> Result<super::ssh_exec::SshExecResponse, crate::errors::AppError> {
    use crate::errors::AppError;
    use crate::services::node_routing_service;
    use std::time::Instant;

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

    // Session limiting
    let session_guard = state.ssh_session_manager.try_acquire(user_id)?;

    // Generate ephemeral SSH key + certificate
    let temp_dir =
        super::ssh_exec::write_ephemeral_ssh_files(state, ssh_svc, service_id, user_id, principal)
            .await?;

    let key_path = temp_dir.path().join("id_ed25519");
    let cert_path = temp_dir.path().join("id_ed25519-cert.pub");

    // Check for node routing
    let node_route = node_routing_service::resolve_node_route(
        &state.db,
        user_id,
        service_id,
        &state.node_ws_manager,
    )
    .await
    .ok()
    .flatten();

    // Build ssh command
    let mut cmd = tokio::process::Command::new("ssh");
    cmd.arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg("-o")
        .arg("UserKnownHostsFile=/dev/null")
        .arg("-o")
        .arg(format!("IdentityFile={}", key_path.display()))
        .arg("-o")
        .arg(format!("CertificateFile={}", cert_path.display()))
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg("LogLevel=FATAL")
        .arg("-o")
        .arg("RequestTTY=no");

    if node_route.is_some() {
        let user_uuid = uuid::Uuid::try_parse(user_id)
            .map_err(|e| AppError::Internal(format!("Invalid user ID format: {e}")))?;
        let proxy_token = crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &user_uuid,
            "openid",
            None,
        )
        .map_err(|e| AppError::Internal(format!("Failed to generate proxy token: {e}")))?;

        let nyxid_binary =
            std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("nyxid"));
        let base_url = state.config.base_url.trim_end_matches('/');

        let proxy_cmd = format!(
            "{} ssh proxy --base-url {} --service-id {}",
            nyxid_binary.display(),
            base_url,
            service_id,
        );
        cmd.arg("-o").arg(format!("ProxyCommand={proxy_cmd}"));
        cmd.env("NYXID_ACCESS_TOKEN", &proxy_token);
    }

    cmd.arg("-p")
        .arg(ssh_svc.port.to_string())
        .arg(format!("{principal}@{}", ssh_svc.host))
        .arg(command);

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    let started_at = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to spawn ssh process: {e}")))?;

    // Take stdout/stderr handles so we can read them without consuming `child`.
    let child_stdout = child.stdout.take();
    let child_stderr = child.stderr.take();

    let read_and_wait = async {
        let stdout_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut out) = child_stdout {
                use tokio::io::AsyncReadExt;
                let _ = out.read_to_end(&mut buf).await;
            }
            buf
        });
        let stderr_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut err) = child_stderr {
                use tokio::io::AsyncReadExt;
                let _ = err.read_to_end(&mut buf).await;
            }
            buf
        });

        let status = child.wait().await;
        let stdout_bytes = stdout_handle.await.unwrap_or_default();
        let stderr_bytes = stderr_handle.await.unwrap_or_default();
        (status, stdout_bytes, stderr_bytes)
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs as u64),
        read_and_wait,
    )
    .await;

    let duration_ms = started_at.elapsed().as_millis() as u64;

    let _ = &session_guard;
    drop(session_guard);

    let response = match result {
        Ok((Ok(status), stdout_bytes, stderr_bytes)) => {
            let stdout = super::ssh_exec::truncate_output(&stdout_bytes);
            let stderr = super::ssh_exec::truncate_output(&stderr_bytes);
            let exit_code = status.code().unwrap_or(-1);

            super::ssh_exec::SshExecResponse {
                exit_code,
                stdout,
                stderr,
                duration_ms,
                timed_out: false,
            }
        }
        Ok((Err(e), _, _)) => {
            return Err(AppError::Internal(format!("SSH process failed: {e}")));
        }
        Err(_) => {
            // Timeout: the async block is dropped, which drops `child`,
            // killing the process automatically.
            super::ssh_exec::SshExecResponse {
                exit_code: -1,
                stdout: String::new(),
                stderr: "Command execution timed out".to_string(),
                duration_ms,
                timed_out: true,
            }
        }
    };

    // Audit log
    audit_service::log_async(
        state.db.clone(),
        Some(user_id.to_string()),
        "ssh_exec_command".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "principal": principal,
            "command": super::ssh_exec::truncate_for_audit(command),
            "exit_code": response.exit_code,
            "duration_ms": response.duration_ms,
            "timed_out": response.timed_out,
            "via": "mcp",
        })),
        None,
        None,
    );

    Ok(response)
}
