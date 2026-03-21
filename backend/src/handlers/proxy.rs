use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{Method, Request, StatusCode},
    response::Response,
};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
use crate::mw::auth::AuthUser;
use crate::services::node_ws_manager::{NodeProxyRequest, ProxyResponseType, StreamChunk};
use crate::services::{
    approval_service, audit_service, chatgpt_translator, delegation_service, identity_service,
    node_metrics_service, node_routing_service, node_service, notification_service, proxy_service,
};

/// Response headers that are safe to forward back to the client.
/// Uses an allowlist to prevent leaking internal headers from downstream services.
/// NOTE: CORS headers (access-control-*) are intentionally excluded — the NyxID
/// CorsLayer handles CORS for all responses. Forwarding downstream CORS headers
/// would cause duplicate headers and browser CORS failures.
const ALLOWED_RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "content-language",
    "content-disposition",
    "cache-control",
    "etag",
    "last-modified",
    "x-request-id",
    "x-correlation-id",
];

/// Request headers safe to forward to node agents for proxy requests.
const ALLOWED_FORWARD_HEADERS: &[&str] = &[
    "content-type",
    "accept",
    "accept-encoding",
    "accept-language",
    "user-agent",
    "x-request-id",
    "x-correlation-id",
];

/// ANY /api/v1/proxy/{service_id}/{*path}
///
/// Forward the request to the downstream service with credential injection,
/// identity propagation, and delegated provider credentials.
pub async fn proxy_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    execute_proxy(&state, &auth_user, &service_id, &path, request).await
}

/// ANY /api/v1/proxy/s/{slug}/{*path}
///
/// Resolve the service by slug, then forward via the shared proxy pipeline.
pub async fn proxy_request_by_slug(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((slug, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    let service = proxy_service::resolve_service_by_slug(&state.db, &slug).await?;
    execute_proxy(&state, &auth_user, &service.id, &path, request).await
}

/// Core proxy execution logic shared by UUID and slug handlers.
///
/// Takes the resolved service_id (UUID string) and executes the full proxy
/// pipeline: resolve target, build identity headers, inject delegation token,
/// resolve delegated credentials, forward request, and return response.
async fn execute_proxy(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
) -> AppResult<Response> {
    let user_id_str = auth_user.user_id.to_string();
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();
    proxy_service::validate_requested_proxy_path(path)?;

    // Resolve node routing FIRST so node-backed users bypass credential checks
    let node_route = node_routing_service::resolve_node_route(
        &state.db,
        &user_id_str,
        service_id,
        &state.node_ws_manager,
    )
    .await?;

    let (target, has_server_credential) = if node_route.is_some() {
        // Node route available: use lenient resolution (no credential required)
        match proxy_service::resolve_proxy_target_lenient(
            &state.db,
            &state.encryption_keys,
            &user_id_str,
            service_id,
        )
        .await
        {
            Ok(result) => result,
            Err(e) => {
                audit_service::log_async(
                    state.db.clone(),
                    Some(user_id_str.clone()),
                    "proxy_request_denied".to_string(),
                    Some(serde_json::json!({
                        "service_id": service_id,
                        "reason": e.to_string(),
                    })),
                    None,
                    None,
                );
                return Err(e);
            }
        }
    } else {
        // No node route: strict credential resolution (unchanged behavior)
        match proxy_service::resolve_proxy_target(
            &state.db,
            &state.encryption_keys,
            &user_id_str,
            service_id,
        )
        .await
        {
            Ok(t) => (t, true),
            Err(e) => {
                audit_service::log_async(
                    state.db.clone(),
                    Some(user_id_str.clone()),
                    "proxy_request_denied".to_string(),
                    Some(serde_json::json!({
                        "service_id": service_id,
                        "reason": e.to_string(),
                    })),
                    None,
                    None,
                );
                return Err(e);
            }
        }
    };

    // Check approval if user has it enabled for this service.
    // Only direct browser sessions bypass approval; API keys, delegated tokens,
    // and service accounts all require approval since they represent programmatic access.
    let requires_approval = approval_service::requires_approval_for_service(
        &state.db,
        &approval_owner_user_id,
        service_id,
    )
    .await?;

    if should_enforce_runtime_approval(requires_approval, &auth_user.auth_method) {
        let requester_type = auth_user.approval_requester_type().ok_or_else(|| {
            AppError::Forbidden("Session auth does not require approval".to_string())
        })?;
        let requester_id = auth_user.approval_requester_id();

        let has_grant = approval_service::check_approval(
            &state.db,
            &approval_owner_user_id,
            service_id,
            requester_type,
            &requester_id,
        )
        .await?;

        if !has_grant {
            let channel =
                notification_service::get_or_create_channel(&state.db, &approval_owner_user_id)
                    .await?;

            let request_method = request.method().as_str().to_string();
            let timeout_secs = channel.approval_timeout_secs;
            let approval_request = approval_service::create_approval_request(
                &state.db,
                &state.config,
                &state.http_client,
                state.fcm_auth.as_deref(),
                state.apns_auth.as_deref(),
                &approval_owner_user_id,
                service_id,
                &target.service.name,
                &target.service.slug,
                requester_type,
                &requester_id,
                None,
                &format!("proxy:{} {}", request_method, path),
                timeout_secs,
            )
            .await?;

            // Block until the user approves/rejects or timeout expires
            approval_service::wait_for_decision(&state.db, &approval_request.id, timeout_secs)
                .await?;
        }
    }

    // === Request Decomposition ===
    // Extract method, query, headers, and body before branching between
    // node proxy and standard proxy paths, since both need them.
    let method = request.method().clone();
    let method_str = method.as_str().to_string();
    let query = request.uri().query().map(String::from);
    let all_headers = request.headers().clone();

    // Headers safe to forward to node agents
    let node_forward_headers: Vec<(String, String)> = all_headers
        .iter()
        .filter_map(|(name, value)| {
            let name_lower = name.as_str().to_lowercase();
            if ALLOWED_FORWARD_HEADERS.contains(&name_lower.as_str()) {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.to_string(), v.to_string()))
            } else {
                None
            }
        })
        .collect();

    // Read the request body (10MB limit)
    let body_bytes = axum::body::to_bytes(request.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read body: {e}")))?;

    let body = if body_bytes.is_empty() {
        None
    } else {
        Some(body_bytes)
    };

    // === Node Proxy Routing (v2: failover + streaming + metrics + HMAC signing) ===
    // node_route was resolved earlier (before credential check) to allow node-backed
    // users to bypass credential requirements.
    if let Some(node_route) = node_route {
        let node_path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };

        // Build base node request (will be cloned for failover retries)
        let node_request = NodeProxyRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            service_id: service_id.to_string(),
            service_slug: target.service.slug.clone(),
            base_url: target.base_url.clone(),
            method: method_str.clone(),
            path: node_path,
            query: query.clone(),
            headers: node_forward_headers,
            body: body.as_ref().map(|b| b.to_vec()),
        };

        // Try primary node, then fallbacks
        let all_node_ids: Vec<&str> = std::iter::once(node_route.node_id.as_str())
            .chain(node_route.fallback_node_ids.iter().map(|s| s.as_str()))
            .collect();

        let mut last_error: Option<AppError> = None;
        for node_id in &all_node_ids {
            // Generate a new request_id for each attempt to avoid correlation conflicts
            let mut attempt_request = node_request.clone();
            attempt_request.request_id = uuid::Uuid::new_v4().to_string();

            // Resolve signing secret for this specific node. When HMAC signing is
            // enabled, unsigned requests are treated as a routing failure rather
            // than silently downgrading integrity guarantees.
            let signing_secret: Option<Vec<u8>> = if state.config.node_hmac_signing_enabled {
                let Some(node) = node_service::get_node_by_id(&state.db, node_id).await? else {
                    last_error = Some(AppError::NodeNotFound(format!(
                        "Node {node_id} not found during proxy routing"
                    )));
                    continue;
                };

                let Some(encrypted_secret) = node.signing_secret_encrypted.as_deref() else {
                    tracing::warn!(
                        node_id = %node_id,
                        "Skipping node route because signing secret is missing"
                    );
                    last_error = Some(AppError::NodeOffline(format!(
                        "Node {node_id} is missing its signing secret"
                    )));
                    continue;
                };

                let secret_hex = String::from_utf8(
                    state
                        .encryption_keys
                        .decrypt(encrypted_secret)
                        .await
                        .map_err(|e| {
                            AppError::Internal(format!(
                                "Failed to decrypt node signing secret for {node_id}: {e}"
                            ))
                        })?,
                )
                .map_err(|e| {
                    AppError::Internal(format!(
                        "Node signing secret for {node_id} is not valid UTF-8: {e}"
                    ))
                })?;

                Some(hex::decode(&secret_hex).map_err(|e| {
                    AppError::Internal(format!(
                        "Node signing secret for {node_id} is not valid hex: {e}"
                    ))
                })?)
            } else {
                None
            };

            let start = std::time::Instant::now();
            let result = state
                .node_ws_manager
                .send_proxy_request(node_id, attempt_request, signing_secret.as_deref())
                .await;
            let latency_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(proxy_response) => {
                    // Record success metrics (fire-and-forget)
                    let db_clone = state.db.clone();
                    let nid = node_id.to_string();
                    tokio::spawn(async move {
                        let _ =
                            node_metrics_service::record_success(db_clone, nid, latency_ms).await;
                    });

                    let response = match proxy_response {
                        ProxyResponseType::Complete(node_response) => {
                            let status = StatusCode::from_u16(node_response.status)
                                .unwrap_or(StatusCode::BAD_GATEWAY);
                            let mut response_builder = Response::builder().status(status);
                            for (name, value) in &node_response.headers {
                                let name_lower = name.to_lowercase();
                                if ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str())
                                    && let (Ok(hn), Ok(hv)) = (
                                        axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                                        axum::http::header::HeaderValue::from_bytes(
                                            value.as_bytes(),
                                        ),
                                    )
                                {
                                    response_builder = response_builder.header(hn, hv);
                                }
                            }
                            response_builder
                                .body(Body::from(node_response.body))
                                .map_err(|e| {
                                    AppError::Internal(format!("Failed to build response: {e}"))
                                })?
                        }
                        ProxyResponseType::Streaming(mut rx) => {
                            // Wait for the Start chunk
                            let first = rx.recv().await.ok_or_else(|| {
                                AppError::NodeOffline("Stream closed before start".to_string())
                            })?;

                            let (status, resp_headers) = match first {
                                StreamChunk::Start { status, headers } => (status, headers),
                                StreamChunk::Error(e) => {
                                    return Err(AppError::Internal(format!("Stream error: {e}")));
                                }
                                _ => {
                                    return Err(AppError::Internal(
                                        "Expected stream start chunk".to_string(),
                                    ));
                                }
                            };

                            let http_status =
                                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
                            let mut response_builder = Response::builder().status(http_status);

                            for (name, value) in &resp_headers {
                                let name_lower = name.to_lowercase();
                                // Skip content-length for streaming responses
                                if name_lower == "content-length" {
                                    continue;
                                }
                                if ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str())
                                    && let (Ok(hn), Ok(hv)) = (
                                        axum::http::header::HeaderName::from_bytes(name.as_bytes()),
                                        axum::http::header::HeaderValue::from_bytes(
                                            value.as_bytes(),
                                        ),
                                    )
                                {
                                    response_builder = response_builder.header(hn, hv);
                                }
                            }

                            // Convert the mpsc receiver into a streaming body
                            let stream = async_stream::stream! {
                                while let Some(chunk) = rx.recv().await {
                                    match chunk {
                                        StreamChunk::Data(bytes) => {
                                            yield Ok::<_, std::io::Error>(bytes::Bytes::from(bytes));
                                        }
                                        StreamChunk::End => break,
                                        StreamChunk::Error(e) => {
                                            tracing::error!(error = %e, "Stream error from node");
                                            break;
                                        }
                                        StreamChunk::Start { .. } => {
                                            // Duplicate start, ignore
                                        }
                                    }
                                }
                            };

                            response_builder
                                .body(Body::from_stream(stream))
                                .map_err(|e| {
                                    AppError::Internal(format!("Failed to build response: {e}"))
                                })?
                        }
                    };

                    audit_service::log_async(
                        state.db.clone(),
                        Some(user_id_str),
                        "proxy_request".to_string(),
                        Some(serde_json::json!({
                            "service_id": service_id,
                            "method": method_str,
                            "path": path,
                            "response_status": response.status().as_u16(),
                            "routed_via": "node",
                            "node_id": node_id,
                        })),
                        None,
                        None,
                    );

                    return Ok(response);
                }
                Err(AppError::NodeOffline(_) | AppError::NodeProxyTimeout) => {
                    tracing::warn!(node_id = %node_id, "Node proxy failed, trying next");

                    // Record error metrics (fire-and-forget)
                    let db_clone = state.db.clone();
                    let nid = node_id.to_string();
                    let err_msg = "Node offline or timeout".to_string();
                    tokio::spawn(async move {
                        let _ = node_metrics_service::record_error(db_clone, nid, err_msg).await;
                    });

                    last_error = Some(AppError::NodeOffline(format!("Node {node_id} failed")));
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        // All nodes failed
        if !has_server_credential {
            return Err(last_error.unwrap_or_else(|| {
                AppError::NodeOffline(
                    "All node routes failed and no server-side credential is available".to_string(),
                )
            }));
        }

        // Fall through to standard proxy with server-side credential
        if let Some(err) = last_error {
            tracing::warn!(
                service_id = %service_id,
                error = %err,
                "All node proxies failed, falling through to standard proxy"
            );
        }
    }
    // === END Node Proxy Routing ===

    // Build identity headers if configured on the service
    let mut identity_headers = Vec::new();

    if target.service.identity_propagation_mode != "none" {
        // Fetch user for identity propagation
        let user = state
            .db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id_str })
            .await?;

        if let Some(ref user) = user {
            // Add identity headers (for "headers" or "both" modes)
            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "headers" | "both"
            ) {
                identity_headers = identity_service::build_identity_headers(user, &target.service);
            }

            // Generate identity JWT assertion (for "jwt" or "both" modes)
            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "jwt" | "both"
            ) {
                match identity_service::generate_identity_assertion(
                    &state.jwt_keys,
                    &state.config,
                    user,
                    &target.service,
                ) {
                    Ok(assertion) => {
                        identity_headers.push(("X-NyxID-Identity-Token".to_string(), assertion));
                    }
                    Err(e) => {
                        tracing::warn!(
                            service_id = %service_id,
                            error = %e,
                            "Failed to generate identity assertion"
                        );
                    }
                }
            }
        }

        // Resolve user RBAC and inject as headers so downstream services can
        // enforce permission checks without needing JWT verification.
        match crate::services::rbac_helpers::resolve_user_rbac(&state.db, &user_id_str).await {
            Ok(rbac) => {
                if !rbac.role_slugs.is_empty() {
                    identity_headers
                        .push(("X-NyxID-User-Roles".to_string(), rbac.role_slugs.join(",")));
                }
                if !rbac.permissions.is_empty() {
                    identity_headers.push((
                        "X-NyxID-User-Permissions".to_string(),
                        rbac.permissions.join(","),
                    ));
                }
                if !rbac.group_slugs.is_empty() {
                    identity_headers.push((
                        "X-NyxID-User-Groups".to_string(),
                        rbac.group_slugs.join(","),
                    ));
                }
            }
            Err(e) => {
                tracing::warn!(
                    user_id = %user_id_str,
                    error = %e,
                    "Failed to resolve RBAC for identity headers"
                );
            }
        }
    }

    // Generate delegation token if configured on the service
    if target.service.inject_delegation_token {
        let user_uuid = auth_user.user_id;

        match crate::crypto::jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &user_uuid,
            &target.service.delegation_token_scope,
            &target.service.slug,
            crate::crypto::jwt::MCP_DELEGATION_TOKEN_TTL_SECS,
        ) {
            Ok(delegation_token) => {
                identity_headers.push(("X-NyxID-Delegation-Token".to_string(), delegation_token));
            }
            Err(e) => {
                tracing::warn!(
                    service_id = %service_id,
                    error = %e,
                    "Failed to generate delegation token for proxy"
                );
            }
        }
    }

    // Resolve delegated credentials. Required provider connections must succeed.
    let delegated = delegation_service::resolve_delegated_credentials(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        service_id,
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("Provider credentials not available: {e}")))?;

    // method, query, all_headers, body were already extracted above
    let reqwest_method = match method {
        Method::GET => reqwest::Method::GET,
        Method::POST => reqwest::Method::POST,
        Method::PUT => reqwest::Method::PUT,
        Method::DELETE => reqwest::Method::DELETE,
        Method::PATCH => reqwest::Method::PATCH,
        Method::HEAD => reqwest::Method::HEAD,
        Method::OPTIONS => reqwest::Method::OPTIONS,
        _ => return Err(AppError::BadRequest("Unsupported HTTP method".to_string())),
    };

    // Convert axum HeaderMap to reqwest HeaderMap
    let mut reqwest_headers = reqwest::header::HeaderMap::new();
    for (name, value) in all_headers.iter() {
        if let Ok(reqwest_name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes())
            && let Ok(reqwest_value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
        {
            reqwest_headers.insert(reqwest_name, reqwest_value);
        }
    }

    // OpenAI Codex: use the specialized ChatGPT HTTP client for supported
    // model endpoints. It sets the required Codex headers (originator,
    // User-Agent, etc.), while preserving the caller's requested response mode.
    let is_codex = target.service.slug == "llm-openai-codex";

    if is_codex
        && is_codex_transport_path(path)
        && let Some(body_ref) = body.as_ref()
    {
        let body_json: serde_json::Value = serde_json::from_slice(body_ref)
            .map_err(|e| AppError::BadRequest(format!("Invalid JSON body: {e}")))?;

        // Use the ChatGPT translator to normalize the request. This handles
        // both Chat Completions format (messages → input + instructions) and
        // Responses API format (enriched with store=false, etc.).
        let translator = chatgpt_translator::ChatgptTranslator;
        let translated =
            <chatgpt_translator::ChatgptTranslator as crate::services::llm_gateway_service::LlmTranslator>::translate_request(
                &translator, path, &body_json,
            )?;
        let is_chat_completions_path = is_chat_completions_proxy_path(path);

        let bearer_token = delegated
            .iter()
            .find(|c| c.injection_method == "bearer")
            .map(|c| c.credential.clone())
            .ok_or_else(|| {
                AppError::BadRequest(
                    "No bearer token for Codex. Connect the provider first.".to_string(),
                )
            })?;

        let is_streaming = body_json
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let response = chatgpt_translator::send_to_chatgpt(
            &translated.body,
            &bearer_token,
            is_streaming,
            is_chat_completions_path,
            query.as_deref(),
        )
        .await?;

        let status = response.status();

        audit_service::log_async(
            state.db.clone(),
            Some(user_id_str),
            "proxy_request".to_string(),
            Some(serde_json::json!({
                "service_id": service_id,
                "method": method.as_str(),
                "path": path,
                "response_status": status.as_u16(),
                "acting_client_id": &auth_user.acting_client_id,
                "codex_transport": true,
            })),
            None,
            None,
        );

        return Ok(response);
    }

    // Reuse the shared reqwest::Client from AppState for connection pooling
    let downstream_response = proxy_service::forward_request(
        &state.http_client,
        &target,
        reqwest_method,
        path,
        query.as_deref(),
        reqwest_headers,
        body,
        identity_headers,
        delegated,
    )
    .await?;

    // Convert reqwest Response back to axum Response
    let status = StatusCode::from_u16(downstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);

    let is_sse = downstream_response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    let mut response_builder = Response::builder().status(status);

    // Forward only allowlisted response headers
    for (name, value) in downstream_response.headers().iter() {
        let name_lower = name.as_str().to_lowercase();
        // Skip content-length for SSE — the body is streamed, length unknown
        if is_sse && name_lower == "content-length" {
            continue;
        }
        if ALLOWED_RESPONSE_HEADERS.contains(&name_lower.as_str())
            && let Ok(header_name) =
                axum::http::header::HeaderName::from_bytes(name.as_str().as_bytes())
            && let Ok(header_value) = axum::http::header::HeaderValue::from_bytes(value.as_bytes())
        {
            response_builder = response_builder.header(header_name, header_value);
        }
    }

    let response = if is_sse && status.is_success() {
        // Stream SSE responses directly without buffering.
        // Wrap the byte stream to log errors from the upstream so we can
        // diagnose premature connection drops.
        let service_id_owned = service_id.to_string();
        let stream = downstream_response.bytes_stream().map(move |chunk| {
            if let Err(e) = &chunk {
                tracing::error!(
                    service_id = %service_id_owned,
                    error = %e,
                    error_debug = ?e,
                    "SSE stream error from upstream — connection dropped"
                );
            }
            chunk
        });
        let body = Body::from_stream(stream);
        response_builder
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
    } else {
        let response_body = downstream_response
            .bytes()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read downstream response: {e}")))?;

        // Log non-2xx response bodies for diagnostics
        if !status.is_success() {
            let body_preview =
                String::from_utf8_lossy(&response_body[..response_body.len().min(1024)]);
            tracing::error!(
                service_id = %service_id,
                status = %status,
                body = %body_preview,
                "Upstream returned error response"
            );
        }

        response_builder
            .body(Body::from(response_body))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
    };

    // Audit log the proxy request
    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "proxy_request".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "method": method.as_str(),
            "path": path,
            "response_status": status.as_u16(),
            "acting_client_id": &auth_user.acting_client_id,
        })),
        None,
        None,
    );

    Ok(response)
}

fn is_codex_transport_path(path: &str) -> bool {
    let normalized = path.trim_matches('/');
    normalized == "responses"
        || normalized == "chat/completions"
        || normalized.ends_with("/responses")
        || normalized.ends_with("/chat/completions")
}

fn is_chat_completions_proxy_path(path: &str) -> bool {
    let normalized = path.trim_matches('/');
    normalized == "chat/completions" || normalized.ends_with("/chat/completions")
}

fn should_enforce_runtime_approval(
    requires_approval: bool,
    auth_method: &crate::mw::auth::AuthMethod,
) -> bool {
    requires_approval && *auth_method != crate::mw::auth::AuthMethod::Session
}

#[cfg(test)]
mod tests {
    use super::{
        is_chat_completions_proxy_path, is_codex_transport_path, should_enforce_runtime_approval,
    };
    use crate::services::proxy_service::validate_requested_proxy_path;
    use crate::mw::auth::AuthMethod;
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::Path,
        http::{Request, StatusCode},
        routing::get,
    };
    use tower::ServiceExt;

    #[test]
    fn session_auth_bypasses_even_when_required() {
        assert!(!should_enforce_runtime_approval(true, &AuthMethod::Session));
    }

    #[test]
    fn non_session_auth_requires_enforcement_when_required() {
        assert!(should_enforce_runtime_approval(true, &AuthMethod::ApiKey));
        assert!(should_enforce_runtime_approval(
            true,
            &AuthMethod::AccessToken
        ));
        assert!(should_enforce_runtime_approval(
            true,
            &AuthMethod::Delegated
        ));
        assert!(should_enforce_runtime_approval(
            true,
            &AuthMethod::ServiceAccount
        ));
    }

    #[test]
    fn no_enforcement_when_approval_not_required() {
        assert!(!should_enforce_runtime_approval(
            false,
            &AuthMethod::Session
        ));
        assert!(!should_enforce_runtime_approval(false, &AuthMethod::ApiKey));
    }

    #[test]
    fn codex_transport_only_handles_supported_endpoints() {
        assert!(is_codex_transport_path("responses"));
        assert!(is_codex_transport_path("/responses"));
        assert!(is_codex_transport_path("chat/completions"));
        assert!(is_codex_transport_path("v1/chat/completions"));
        assert!(!is_codex_transport_path("models"));
        assert!(!is_codex_transport_path("responses/items"));
    }

    #[test]
    fn codex_chat_completions_detection_handles_prefixed_paths() {
        assert!(is_chat_completions_proxy_path("chat/completions"));
        assert!(is_chat_completions_proxy_path("/v1/chat/completions"));
        assert!(!is_chat_completions_proxy_path("responses"));
    }

    #[tokio::test]
    async fn wildcard_path_extractor_decodes_percent_encoded_path_injection_breakers() {
        async fn capture_path(Path((service_id, path)): Path<(String, String)>) -> String {
            format!("{service_id}:{path}")
        }

        let app = Router::new().route("/{service_id}/{*path}", get(capture_path));

        let slash_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/folder%2FsendMessage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(slash_response.status(), StatusCode::OK);
        let slash_body = to_bytes(slash_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&slash_body).unwrap(),
            "svc:folder/sendMessage"
        );

        let backslash_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/folder%5CsendMessage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(backslash_response.status(), StatusCode::OK);
        let backslash_body = to_bytes(backslash_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&backslash_body).unwrap(),
            "svc:folder\\sendMessage"
        );

        let dotdot_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/%2e%2e")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(dotdot_response.status(), StatusCode::OK);
        let dotdot_body = to_bytes(dotdot_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&dotdot_body).unwrap(),
            "svc:.."
        );

        let double_encoded_dotdot_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/%252e%252e")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(double_encoded_dotdot_response.status(), StatusCode::OK);
        let double_encoded_dotdot_body =
            to_bytes(double_encoded_dotdot_response.into_body(), usize::MAX)
                .await
                .unwrap();
        assert_eq!(
            std::str::from_utf8(&double_encoded_dotdot_body).unwrap(),
            "svc:%2e%2e"
        );

        let double_encoded_slash_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/folder%252FsendMessage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(double_encoded_slash_response.status(), StatusCode::OK);
        let double_encoded_slash_body =
            to_bytes(double_encoded_slash_response.into_body(), usize::MAX)
                .await
                .unwrap();
        assert_eq!(
            std::str::from_utf8(&double_encoded_slash_body).unwrap(),
            "svc:folder%2FsendMessage"
        );

        let double_encoded_backslash_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/folder%255CsendMessage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(double_encoded_backslash_response.status(), StatusCode::OK);
        let double_encoded_backslash_body =
            to_bytes(double_encoded_backslash_response.into_body(), usize::MAX)
                .await
                .unwrap();
        assert_eq!(
            std::str::from_utf8(&double_encoded_backslash_body).unwrap(),
            "svc:folder%5CsendMessage"
        );

        let double_encoded_nul_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/%2500")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(double_encoded_nul_response.status(), StatusCode::OK);
        let double_encoded_nul_body = to_bytes(double_encoded_nul_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&double_encoded_nul_body).unwrap(),
            "svc:%00"
        );
    }

    #[test]
    fn node_proxy_path_injection_rejects_breakers() {
        for path in [
            "/folder%2FsendMessage",
            "/folder%2fsendMessage",
            "/%2e%2e",
            "/%2e.",
            "/.%2e",
            "/%2E%2E",
            "/%2E.",
            "/.%2E",
            "/folder%5CsendMessage",
            "/folder%5csendMessage",
            "/%00",
            "/folder\\sendMessage",
        ] {
            let err =
                validate_requested_proxy_path(path).expect_err("path breaker should be rejected");
            assert!(
                err.to_string().contains("Invalid proxy path"),
                "unexpected error for '{path}': {err}"
            );
        }
    }

    #[test]
    fn node_proxy_path_injection_allows_non_segment_dot_sequences() {
        validate_requested_proxy_path("/v1/foo..bar/foo%2ebar")
            .expect("non-segment dot sequences should be allowed");
    }
}

#[derive(Debug, Serialize)]
pub struct ProxyServiceItem {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub service_category: String,
    /// Whether the user has an active connection to this service
    pub connected: bool,
    /// Whether a connection is required before proxying
    pub requires_connection: bool,
    /// Whether the user currently has a viable node route for this service
    pub has_node_binding: bool,
    /// UUID-based proxy URL
    pub proxy_url: String,
    /// Slug-based proxy URL (developer-friendly)
    pub proxy_url_slug: String,
}

#[derive(Debug, Deserialize)]
pub struct ProxyServicesQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ProxyServicesResponse {
    pub services: Vec<ProxyServiceItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

/// GET /api/v1/proxy/services
///
/// List downstream services available for proxying with their proxy URLs.
/// Excludes "provider" category services (not proxyable).
/// Supports pagination via `page` and `per_page` query parameters.
pub async fn list_proxy_services(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ProxyServicesQuery>,
) -> AppResult<Json<ProxyServicesResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let base = state.config.base_url.trim_end_matches('/');

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let filter = doc! {
        "is_active": true,
        "service_category": { "$ne": "provider" },
    };

    // Get total count for pagination metadata
    let total = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .count_documents(filter.clone())
        .await?;

    // Get paginated active, non-provider services
    let services: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
        .sort(doc! { "name": 1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    // Get user's active connections in a single query
    let service_ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    let connections: Vec<UserServiceConnection> = if service_ids.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
            .find(doc! {
                "user_id": &user_id_str,
                "service_id": { "$in": &service_ids },
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?
    };

    let connected_set: HashSet<&str> = connections.iter().map(|c| c.service_id.as_str()).collect();

    let bound_service_ids = node_routing_service::list_routable_service_ids(
        &state.db,
        &user_id_str,
        state.node_ws_manager.as_ref(),
    )
    .await?;
    let node_bound_set: HashSet<&str> = bound_service_ids.iter().map(|s| s.as_str()).collect();

    let items: Vec<ProxyServiceItem> = services
        .iter()
        .map(|s| ProxyServiceItem {
            id: s.id.clone(),
            name: s.name.clone(),
            slug: s.slug.clone(),
            description: s.description.clone(),
            service_category: s.service_category.clone(),
            connected: connected_set.contains(s.id.as_str()),
            requires_connection: s.requires_user_credential,
            has_node_binding: node_bound_set.contains(s.id.as_str()),
            proxy_url: format!("{base}/api/v1/proxy/{}/{{path}}", s.id),
            proxy_url_slug: format!("{base}/api/v1/proxy/s/{}/{{path}}", s.slug),
        })
        .collect();

    Ok(Json(ProxyServicesResponse {
        services: items,
        total,
        page,
        per_page,
    }))
}
