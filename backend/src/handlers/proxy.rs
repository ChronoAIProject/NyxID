use axum::{
    Json,
    body::Body,
    extract::{FromRequestParts, Path, Query, State, ws::WebSocketUpgrade},
    http::{Method, Request, StatusCode},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt, TryStreamExt};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tokio_stream::wrappers::ReceiverStream;
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, legacy_http_service_type_filter,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};
use crate::mw::auth::AuthUser;
use crate::services::node_ws_manager::{NodeProxyRequest, ProxyResponseType, StreamChunk};
use crate::services::{
    approval_service, audit_service, chatgpt_translator, delegation_service, identity_service,
    llm_usage_service, node_metrics_service, node_routing_service, node_service,
    notification_service, operation_descriptor, proxy_service, sse_parser, user_service_service,
    ws_frame_injector,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

/// Map an `AppError` surfaced by the proxy handler to the `(status, error_code)`
/// pair used by `TelemetryEvent::ProxyError`.
///
/// Scoped to the variants the proxy handler actually emits. Unknown variants
/// fall back to `(500, 0)` per the §5.1 "use 0 if no direct mapping" rule —
/// the real `AppError::error_code()` is crate-private, and we intentionally
/// don't widen its visibility just for telemetry. Keep this list in sync with
/// the `return Err(...)` sites in this file.
fn proxy_error_telemetry_fields(err: &AppError) -> (u16, u32) {
    match err {
        AppError::BadRequest(_) => (400, 1000),
        AppError::Unauthorized(_) => (401, 1001),
        AppError::Forbidden(_) => (403, 1002),
        AppError::NotFound(_) => (404, 1003),
        AppError::RateLimited => (429, 1005),
        AppError::Internal(_) => (500, 1006),
        AppError::DatabaseError(_) => (500, 1007),
        AppError::ValidationError(_) => (400, 1008),
        AppError::NodeNotFound(_) => (404, 8000),
        AppError::NodeOffline(_) => (503, 8001),
        AppError::NodeProxyTimeout => (504, 8002),
        AppError::NodeCredentialMissing(_) => (502, 8004),
        AppError::WsProxyDownstream(_) => (502, 8005),
        AppError::ApiKeyScopeForbidden(_) => (403, 9000),
        AppError::ApiKeyScopeInactive => (403, 9001),
        AppError::ApiKeyScopeNotFound(_) => (404, 9002),
        AppError::OrgApprovalNoAdmin(_) => (503, 8106),
        AppError::ApprovalRequired { .. } => (403, 7000),
        AppError::ApprovalFailed { .. } => (403, 7001),
        // Catch-all: unknown / less-common variants emit a 500 + error_code=0
        // placeholder. This is acceptable per the telemetry spec.
        _ => (500, 0),
    }
}

/// Fire-and-forget emission of `TelemetryEvent::ProxyError` from any proxy
/// error branch. `resolved_slug` should be the slug of the resolved
/// `UserService` / `DownstreamService`, or empty if resolution never
/// succeeded — NEVER a UUID from the route path.
fn emit_proxy_error_telemetry(
    state: &AppState,
    auth_user: &AuthUser,
    tele: &TelemetryContext,
    resolved_slug: &str,
    err: &AppError,
) {
    let (status, error_code) = proxy_error_telemetry_fields(err);
    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        tele,
        TelemetryEvent::ProxyError {
            service_slug: resolved_slug.to_string(),
            error_code,
            status,
        },
    );
}

/// Stable string label for the auth method that issued this proxy request.
/// Pairs with `TelemetryEvent::ProxySuccess.auth_kind`. The values are part
/// of the public PostHog property contract — if you rename one, update the
/// HogQL queries in `.claude/skills/daily/SKILL.md` and the strategy doc
/// before merging.
fn auth_kind_label(method: &crate::mw::auth::AuthMethod) -> &'static str {
    use crate::mw::auth::AuthMethod::*;
    match method {
        Session => "session",
        AccessToken => "access_token",
        Relay => "relay",
        ApiKey => "api_key",
        ServiceAccount => "service_account",
        Delegated => "delegated",
    }
}

/// Fire-and-forget emission of `TelemetryEvent::ProxySuccess` from the
/// outer proxy wrappers when the upstream returned 2xx. Mirror of
/// `emit_proxy_error_telemetry`: `resolved_slug` MUST be the slug of the
/// resolved service (populated by `execute_proxy_inner`), not the route
/// path parameter, so success and error events join cleanly on
/// `service_slug` for success-rate computation.
///
/// `started_at` is the handler entry timestamp; the difference is recorded
/// as `latency_ms`. The status is the actual upstream status echoed back
/// in the response we are about to return to the client. Any non-2xx
/// status is the caller's signal NOT to call this helper — it returns Ok
/// for any status to keep the emit-site call brief, and the caller is
/// expected to gate on `response.status().is_success()`.
fn emit_proxy_success_telemetry(
    state: &AppState,
    auth_user: &AuthUser,
    tele: &TelemetryContext,
    resolved_slug: &str,
    method: &Method,
    status: StatusCode,
    started_at: std::time::Instant,
) {
    let latency_ms = started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        tele,
        TelemetryEvent::ProxySuccess {
            service_slug: resolved_slug.to_string(),
            method: method.as_str().to_string(),
            status: status.as_u16(),
            latency_ms,
            auth_kind: auth_kind_label(&auth_user.auth_method),
        },
    );
}

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
    "accept-ranges",
    "content-range",
];

/// Request headers safe to forward to node agents for proxy requests.
///
/// In addition to the explicit list, any caller-supplied header matching a
/// prefix in `ALLOWED_FORWARD_HEADER_PREFIXES` is forwarded. That covers
/// OpenClaw gateway semantics such as `x-openclaw-scopes` (NyxID#161) without
/// requiring a new allowlist entry every time OpenClaw adds a header.
const ALLOWED_FORWARD_HEADERS: &[&str] = &[
    "content-type",
    "accept",
    "accept-encoding",
    "accept-language",
    "user-agent",
    "x-request-id",
    "x-correlation-id",
    "range",
    "if-range",
    "if-none-match",
    "if-modified-since",
    "content-length",
];

/// Caller-supplied header prefixes forwarded transparently. Keep this narrow:
/// the prefix namespace belongs to the downstream service, not to NyxID or
/// infrastructure headers.
///
/// Must stay in sync with `proxy_service::ALLOWED_FORWARD_HEADER_PREFIXES`,
/// which is the equivalent list on the direct-HTTP path. Node-routed
/// requests fan out through this list before being serialized into the
/// `NodeProxyRequest` frame, so a prefix missing here gets stripped on the
/// node path even when it survives the direct path (Codex review
/// BLOCKER 5 — `x-amz-target` was lost on Cost Explorer calls routed
/// via node because the prefix wasn't here).
const ALLOWED_FORWARD_HEADER_PREFIXES: &[&str] = &["x-openclaw-", "x-amz-", "x-goog-"];

/// Headers worth preserving on proxied WebSocket handshakes.
/// Upgrade mechanics and key/version headers are regenerated by the WS client,
/// but downstream services may still depend on origin or subprotocol negotiation.
const ALLOWED_WS_FORWARD_HEADERS: &[&str] = &[
    "accept",
    "accept-encoding",
    "accept-language",
    "origin",
    "sec-websocket-protocol",
    "user-agent",
    "x-request-id",
    "x-correlation-id",
];

/// Pre-resolved proxy target from the new UserService path.
struct PreResolved {
    target: proxy_service::ProxyTarget,
    node_id: Option<String>,
    /// The UserService ID for API key scope checks.
    user_service_id: Option<String>,
    has_server_credential: bool,
    /// The user_id that owns the resolved UserService. For personal
    /// resolutions this is the actor; for org-routed resolutions this is
    /// the org's user_id. Used to scope NodeServiceBinding fallback
    /// lookups so the failover list reflects the org's bindings, not
    /// just the calling member's personal bindings.
    effective_owner_id: String,
}

/// Emit a single audit entry recording that this proxy call was routed via
/// an org's shared credential. The request itself produces additional
/// audit entries via execute_proxy_inner; this is the org-attribution side.
fn audit_org_routing(
    state: &AppState,
    auth_user: &AuthUser,
    routing: &proxy_service::OrgRouting,
    user_service_id: &str,
    service_id: &str,
) {
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "proxy_routed_via_org",
        Some(serde_json::json!({
            "routed_via": "org",
            "service_id": service_id,
            "user_service_id": user_service_id,
            // Org-routed audits use org_user_id; node-routed audits use owner_user_id.
            // Owner-centric audit queries must check both fields.
            "org_user_id": routing.org_user_id,
            "member_user_id": routing.member_user_id,
            "membership_id": routing.membership_id,
        })),
    );
}

/// Emit a single audit entry recording that this proxy call was routed via
/// the actor's own personal credential (no org inheritance). Mirrors
/// `audit_org_routing` so audit consumers can distinguish personal vs org
/// routing attribution without inferring it from the absence of org fields
/// (see docs/ORG_MODEL.md "Audit Trail").
///
/// `user_service_id` is `None` for the legacy DownstreamService / provider-
/// token path, which resolves directly from the catalog service + the
/// caller's stored provider credentials without a `UserService` record.
/// Even there the event must still fire so the audit trail is complete for
/// unmigrated users during the migration window.
fn audit_personal_routing(
    state: &AppState,
    auth_user: &AuthUser,
    user_service_id: Option<&str>,
    service_id: &str,
) {
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "proxy_routed_via_personal",
        Some(serde_json::json!({
            "routed_via": "personal",
            "service_id": service_id,
            "user_service_id": user_service_id,
        })),
    );
}

fn add_owner_user_id_if_shared(
    value: &mut serde_json::Value,
    owner_user_id: &str,
    actor_user_id: &str,
) {
    if owner_user_id != actor_user_id
        && let Some(object) = value.as_object_mut()
    {
        object.insert(
            "owner_user_id".to_string(),
            serde_json::Value::String(owner_user_id.to_string()),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn node_proxy_audit_event_data(
    service_id: &str,
    method: &str,
    path: &str,
    response_status: u16,
    node_id: &str,
    service_owner_user_id: &str,
    proxy_actor_user_id: &str,
    connection_id: Option<&str>,
) -> serde_json::Value {
    let mut event_data = serde_json::json!({
        "service_id": service_id,
        "method": method,
        "path": path,
        "response_status": response_status,
        "routed_via": "node",
        "node_id": node_id,
    });
    if let Some(conn_id) = connection_id {
        event_data["connection_id"] = serde_json::Value::String(conn_id.to_string());
    }

    // Node-routed audits use polymorphic owner_user_id; org-routed audits use org_user_id.
    // Owner-centric audit queries must check both fields.
    add_owner_user_id_if_shared(&mut event_data, service_owner_user_id, proxy_actor_user_id);

    event_data
}

struct DownstreamWsConnection {
    stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    selected_protocol: Option<String>,
}

fn collect_forward_headers_with_prefixes(
    headers: &axum::http::HeaderMap,
    allowed_headers: &[&str],
    allowed_prefixes: &[&str],
) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let name_lower = name.as_str().to_lowercase();
            let allowed = allowed_headers.contains(&name_lower.as_str())
                || allowed_prefixes
                    .iter()
                    .any(|prefix| name_lower.starts_with(prefix));
            if allowed {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.to_string(), v.to_string()))
            } else {
                None
            }
        })
        .collect()
}

/// Extract `?_nyxid_via=<user_service_id>` from the request URI.
///
/// When present, the proxy handler bypasses the auto-resolution cascade
/// and uses the specified UserService directly. The caller gets the id
/// from `GET /api/v1/user-services` or `GET /api/v1/keys`.
fn extract_via_service(request: &Request<Body>) -> Option<String> {
    request.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|pair| pair.strip_prefix("_nyxid_via="))
            .map(|v| urlencoding::decode(v).unwrap_or_default().to_string())
    })
}

/// Strip NyxID-internal query params before forwarding to downstream.
///
/// Currently strips `_nyxid_via` (the explicit credential-selection
/// param added by this PR). Future NyxID-internal params should be
/// added to the filter here so downstream services never see them.
fn strip_internal_query_params(raw: &str) -> String {
    const INTERNAL_PARAMS: &[&str] = &["_nyxid_via"];
    raw.split('&')
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or("");
            !INTERNAL_PARAMS.contains(&key)
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn append_query_param(url: &str, param_name: &str, param_value: &str) -> String {
    let separator = if url.contains('?') { "&" } else { "?" };
    let encoded_name = urlencoding::encode(param_name);
    let encoded_value = urlencoding::encode(param_value);
    format!("{url}{separator}{encoded_name}={encoded_value}")
}

#[utoipa::path(
    post,
    path = "/api/v1/proxy/{service_id}/{path}",
    params(
        ("service_id" = String, Path, description = "Downstream service ID (UUID)"),
        ("path" = String, Path, description = "Downstream API path")
    ),
    responses(
        (status = 200, description = "Proxied response from downstream service"),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden / approval required", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy"
)]
/// ANY /api/v1/proxy/:service_id/*path
///
/// Forward the request to the downstream service with credential injection,
/// identity propagation, and delegated provider credentials.
/// Tries the new UserService path first (by catalog_service_id), falls back to old.
///
/// Accepts an optional `?_nyxid_via=<user_service_id>` query param that
/// bypasses the auto-resolution cascade and uses the specified UserService
/// directly. The caller gets the id from `GET /api/v1/user-services` or
/// `GET /api/v1/keys`, which list both personal and org-inherited services
/// tagged with `credential_source`. This lets a user who has both a
/// personal and an org credential for the same service explicitly choose
/// which one to use for a given request.
pub async fn proxy_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path((service_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    // Emit `proxy.error` on any error branch reached via this handler. The
    // inner function threads `resolved_slug` through so we can attach a
    // real slug (never a UUID from the route path) even when the error
    // happens after service resolution. See docs/TELEMETRY.md §5.1.
    let started_at = std::time::Instant::now();
    let method = request.method().clone();
    let mut resolved_slug = String::new();
    let result = proxy_request_inner(
        &state,
        &auth_user,
        &service_id,
        &path,
        request,
        &mut resolved_slug,
    )
    .await;
    match &result {
        Ok(response) if response.status().is_success() => {
            emit_proxy_success_telemetry(
                &state,
                &auth_user,
                &tele,
                &resolved_slug,
                &method,
                response.status(),
                started_at,
            );
        }
        Ok(_) => {
            // Non-2xx Ok responses come from upstream-error passthrough
            // (e.g. 4xx from the target service). They are neither a
            // NyxID-side `proxy.error` nor a `proxy.success`; the proxy
            // layer behaved correctly while the downstream rejected.
            // Skip telemetry — counting upstream 4xx as either side
            // distorts both signals.
        }
        Err(err) => emit_proxy_error_telemetry(&state, &auth_user, &tele, &resolved_slug, err),
    }
    result
}

async fn proxy_request_inner(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
    resolved_slug: &mut String,
) -> AppResult<Response> {
    auth_user.ensure_rest_proxy_access()?;

    let user_id_str = auth_user.proxy_resolution_user_id();
    let via_service = extract_via_service(&request);
    preflight_proxy_deny_before_resolution(
        state,
        auth_user,
        via_service.as_deref(),
        None,
        Some(service_id),
        path,
        request.method().as_str(),
    )
    .await?;

    // Direct resolution by UserService ID if ?_nyxid_via= is present.
    // Constrained to the catalog service_id in the route path so the
    // override cannot silently proxy through a different service.
    if let Some(ref us_id) = via_service {
        if let Some(resolved) = proxy_service::resolve_proxy_target_by_user_service_id(
            &state.db,
            &state.encryption_keys,
            &user_id_str,
            us_id,
            None,
            Some(service_id),
        )
        .await?
        {
            let effective_service_id = resolved.target.service.id.clone();
            if let Some(routing) = &resolved.org_routing {
                audit_org_routing(
                    state,
                    auth_user,
                    routing,
                    &resolved.user_service_id,
                    &effective_service_id,
                );
            } else {
                audit_personal_routing(
                    state,
                    auth_user,
                    Some(&resolved.user_service_id),
                    &effective_service_id,
                );
            }
            return execute_proxy_inner(
                state,
                auth_user,
                &effective_service_id,
                path,
                request,
                Some(PreResolved {
                    target: resolved.target,
                    node_id: resolved.node_id,
                    user_service_id: Some(resolved.user_service_id),
                    has_server_credential: resolved.has_server_credential,
                    effective_owner_id: resolved
                        .org_routing
                        .as_ref()
                        .map(|r| r.org_user_id.clone())
                        .unwrap_or_else(|| user_id_str.clone()),
                }),
                resolved_slug,
            )
            .await;
        }
        return Err(AppError::NotFound(format!(
            "UserService '{us_id}' not found"
        )));
    }

    // Try new UserService path first (lookup by catalog_service_id)
    if let Some(resolved) = proxy_service::resolve_proxy_target_from_user_service(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &user_id_str,
        None,
        Some(service_id),
    )
    .await?
    {
        let effective_service_id = resolved.target.service.id.clone();
        if let Some(routing) = &resolved.org_routing {
            audit_org_routing(
                state,
                auth_user,
                routing,
                &resolved.user_service_id,
                &effective_service_id,
            );
        } else {
            audit_personal_routing(
                state,
                auth_user,
                Some(&resolved.user_service_id),
                &effective_service_id,
            );
        }
        return execute_proxy_inner(
            state,
            auth_user,
            &effective_service_id,
            path,
            request,
            Some(PreResolved {
                target: resolved.target,
                node_id: resolved.node_id,
                user_service_id: Some(resolved.user_service_id),
                has_server_credential: resolved.has_server_credential,
                effective_owner_id: resolved
                    .org_routing
                    .as_ref()
                    .map(|r| r.org_user_id.clone())
                    .unwrap_or_else(|| user_id_str.clone()),
            }),
            resolved_slug,
        )
        .await;
    }

    // Fall back to old path. Before we do, block org viewers whose org
    // has any presence for this catalog service from slipping into the
    // legacy approval flow (see ChronoAIProject/NyxID#375).
    proxy_service::guard_slug_against_viewer_orgs(&state.db, &user_id_str, None, Some(service_id))
        .await?;
    execute_proxy(state, auth_user, service_id, path, request, resolved_slug).await
}

#[utoipa::path(
    post,
    path = "/api/v1/proxy/s/{slug}/{path}",
    params(
        ("slug" = String, Path, description = "Service slug (e.g., llm-openai, api-github)"),
        ("path" = String, Path, description = "Downstream API path")
    ),
    responses(
        (status = 200, description = "Proxied response from downstream service"),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden / approval required", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy"
)]
/// ANY /api/v1/proxy/s/:slug/*path
///
/// Resolve the service by slug, then forward via the shared proxy pipeline.
/// Tries the new UserService path first (by slug), then falls back to old
/// DownstreamService resolution.
///
/// Accepts `?_nyxid_via=<user_service_id>` — see `proxy_request` doc.
pub async fn proxy_request_by_slug(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path((slug, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    // Start empty; `proxy_request_by_slug_inner` populates this once the
    // resolved `UserService`/`DownstreamService` is available. We
    // intentionally do NOT seed with the path-param slug: telemetry §5.1
    // requires a resolved slug or empty, and the path param is unvalidated
    // user input until resolution succeeds.
    let started_at = std::time::Instant::now();
    let method = request.method().clone();
    let mut resolved_slug = String::new();
    let result = proxy_request_by_slug_inner(
        &state,
        &auth_user,
        &slug,
        &path,
        request,
        &mut resolved_slug,
    )
    .await;
    match &result {
        Ok(response) if response.status().is_success() => {
            emit_proxy_success_telemetry(
                &state,
                &auth_user,
                &tele,
                &resolved_slug,
                &method,
                response.status(),
                started_at,
            );
        }
        Ok(_) => {
            // See `proxy_request` for the upstream-error passthrough
            // rationale: a 4xx echoed from the downstream is not a
            // NyxID-side success or error, so it is intentionally not
            // emitted on either side.
        }
        Err(err) => emit_proxy_error_telemetry(&state, &auth_user, &tele, &resolved_slug, err),
    }
    result
}

async fn proxy_request_by_slug_inner(
    state: &AppState,
    auth_user: &AuthUser,
    slug: &str,
    path: &str,
    request: Request<Body>,
    resolved_slug: &mut String,
) -> AppResult<Response> {
    auth_user.ensure_rest_proxy_access()?;

    let user_id_str = auth_user.proxy_resolution_user_id();
    let via_service = extract_via_service(&request);
    preflight_proxy_deny_before_resolution(
        state,
        auth_user,
        via_service.as_deref(),
        Some(slug),
        None,
        path,
        request.method().as_str(),
    )
    .await?;

    // Direct resolution by UserService ID if ?_nyxid_via= is present.
    // Constrained to the slug in the route path so the override cannot
    // silently proxy through a different service.
    if let Some(ref us_id) = via_service {
        if let Some(resolved) = proxy_service::resolve_proxy_target_by_user_service_id(
            &state.db,
            &state.encryption_keys,
            &user_id_str,
            us_id,
            Some(slug),
            None,
        )
        .await?
        {
            let effective_service_id = resolved.target.service.id.clone();
            if let Some(routing) = &resolved.org_routing {
                audit_org_routing(
                    state,
                    auth_user,
                    routing,
                    &resolved.user_service_id,
                    &effective_service_id,
                );
            } else {
                audit_personal_routing(
                    state,
                    auth_user,
                    Some(&resolved.user_service_id),
                    &effective_service_id,
                );
            }
            return execute_proxy_inner(
                state,
                auth_user,
                &effective_service_id,
                path,
                request,
                Some(PreResolved {
                    target: resolved.target,
                    node_id: resolved.node_id,
                    user_service_id: Some(resolved.user_service_id),
                    has_server_credential: resolved.has_server_credential,
                    effective_owner_id: resolved
                        .org_routing
                        .as_ref()
                        .map(|r| r.org_user_id.clone())
                        .unwrap_or_else(|| user_id_str.clone()),
                }),
                resolved_slug,
            )
            .await;
        }
        return Err(AppError::NotFound(format!(
            "UserService '{us_id}' not found"
        )));
    }

    // Try new UserService path first (by slug)
    if let Some(resolved) = proxy_service::resolve_proxy_target_from_user_service(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &user_id_str,
        Some(slug),
        None,
    )
    .await?
    {
        let effective_service_id = resolved.target.service.id.clone();
        if let Some(routing) = &resolved.org_routing {
            audit_org_routing(
                state,
                auth_user,
                routing,
                &resolved.user_service_id,
                &effective_service_id,
            );
        } else {
            audit_personal_routing(
                state,
                auth_user,
                Some(&resolved.user_service_id),
                &effective_service_id,
            );
        }
        return execute_proxy_inner(
            state,
            auth_user,
            &effective_service_id,
            path,
            request,
            Some(PreResolved {
                target: resolved.target,
                node_id: resolved.node_id,
                user_service_id: Some(resolved.user_service_id),
                has_server_credential: resolved.has_server_credential,
                effective_owner_id: resolved
                    .org_routing
                    .as_ref()
                    .map(|r| r.org_user_id.clone())
                    .unwrap_or_else(|| user_id_str.clone()),
            }),
            resolved_slug,
        )
        .await;
    }

    // Fall back to old path. Before we do, block org viewers whose org
    // has any presence for this slug from slipping into the legacy
    // approval flow (see ChronoAIProject/NyxID#375).
    proxy_service::guard_slug_against_viewer_orgs(&state.db, &user_id_str, Some(slug), None)
        .await?;
    let service = proxy_service::resolve_service_by_slug(&state.db, slug).await?;
    execute_proxy(state, auth_user, &service.id, path, request, resolved_slug).await
}

/// ANY /api/v1/proxy/:service_id (no trailing path)
pub async fn proxy_request_root(
    state: State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(service_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    proxy_request(
        state,
        auth_user,
        tele,
        Path((service_id, String::new())),
        request,
    )
    .await
}

/// ANY /api/v1/proxy/s/:slug (no trailing path)
pub async fn proxy_request_by_slug_root(
    state: State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(slug): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    proxy_request_by_slug(state, auth_user, tele, Path((slug, String::new())), request).await
}

/// Core proxy execution logic shared by UUID and slug handlers (old path).
///
/// Reached only when no `UserService` match was found for the caller, so the
/// request resolves against the legacy `DownstreamService` + provider-token
/// path. The `proxy_routed_via_personal` audit event is emitted inside
/// `execute_proxy_inner` once legacy resolution actually succeeds — emitting
/// it here would record a "routed via personal" attribution even for
/// requests that never resolved a target (e.g. disconnected service,
/// missing credential). See ChronoAIProject/NyxID#423.
async fn execute_proxy(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
    resolved_slug: &mut String,
) -> AppResult<Response> {
    execute_proxy_inner(
        state,
        auth_user,
        service_id,
        path,
        request,
        None,
        resolved_slug,
    )
    .await
}

/// Resolve proxy target and node routing via the old DownstreamService path.
///
/// Returns `(node_route, target, has_server_credential, user_service_id, node_routing_required)`.
/// `node_routing_required` is `true` when the user's UserService for this
/// catalog service explicitly pins a node, regardless of whether the node is
/// currently online. The caller must use this flag to enforce the "Route via
/// Node" contract (see ChronoAIProject/NyxID#328).
async fn resolve_via_downstream_service(
    state: &AppState,
    auth_user: &AuthUser,
    user_id_str: &str,
    service_id: &str,
) -> AppResult<(
    Option<node_routing_service::NodeRoute>,
    proxy_service::ProxyTarget,
    bool,
    Option<String>,
    bool,
)> {
    let nr = node_routing_service::resolve_node_route(
        &state.db,
        user_id_str,
        service_id,
        &state.node_ws_manager,
    )
    .await?;

    let node_routing_required =
        node_routing_service::user_service_has_explicit_node(&state.db, user_id_str, service_id)
            .await?;

    // Hard-fail when the service is explicitly node-routed but no viable
    // node could be resolved. Falling through to direct routing would
    // violate the "Route via Node" contract and silently bypass the
    // intended execution boundary (node isolation, local credentials,
    // private-network access).
    if nr.is_none() && node_routing_required {
        let err = AppError::NodeOffline(
            "Service is configured to route via a node, but no viable node is available"
                .to_string(),
        );
        audit_service::log_for_user(
            state.db.clone(),
            auth_user,
            "proxy_request_denied",
            Some(serde_json::json!({
                "service_id": service_id,
                "reason": err.to_string(),
                "node_routing_required": true,
            })),
        );
        return Err(err);
    }

    let (t, has_cred) = if nr.is_some() {
        match proxy_service::resolve_proxy_target_lenient(
            &state.db,
            &state.encryption_keys,
            user_id_str,
            service_id,
        )
        .await
        {
            Ok(result) => result,
            Err(e) => {
                audit_service::log_for_user(
                    state.db.clone(),
                    auth_user,
                    "proxy_request_denied",
                    Some(serde_json::json!({
                        "service_id": service_id,
                        "reason": e.to_string(),
                    })),
                );
                return Err(e);
            }
        }
    } else {
        match proxy_service::resolve_proxy_target(
            &state.db,
            &state.encryption_keys,
            user_id_str,
            service_id,
        )
        .await
        {
            Ok(t) => (t, true),
            Err(e) => {
                audit_service::log_for_user(
                    state.db.clone(),
                    auth_user,
                    "proxy_request_denied",
                    Some(serde_json::json!({
                        "service_id": service_id,
                        "reason": e.to_string(),
                    })),
                );
                return Err(e);
            }
        }
    };

    Ok((nr, t, has_cred, None, node_routing_required))
}

async fn build_pre_resolved_node_route(
    state: &AppState,
    user_id: &str,
    service_id: &str,
    explicit_node_id: Option<&str>,
) -> AppResult<Option<node_routing_service::NodeRoute>> {
    let Some(explicit_node_id) = explicit_node_id else {
        return Ok(None);
    };

    // Check if the configured node is actually viable on this instance (DB Online
    // + WS-connected + healthy). Without this, a request landing on a backend
    // instance that doesn't hold the WS connection is routed to the node and
    // surfaces `503 node_offline` to the caller even though `node list` shows
    // the node online globally. See issue #325.
    let primary_viable = node_routing_service::is_node_id_viable(
        &state.db,
        explicit_node_id,
        state.node_ws_manager.as_ref(),
    )
    .await?;

    let fallback_node_ids: Vec<String> = node_routing_service::list_viable_binding_node_ids(
        &state.db,
        user_id,
        service_id,
        state.node_ws_manager.as_ref(),
    )
    .await?
    .into_iter()
    .filter(|node_id| node_id != explicit_node_id)
    .collect();

    if !primary_viable {
        if fallback_node_ids.is_empty() {
            tracing::warn!(
                configured_node_id = %explicit_node_id,
                service_id = %service_id,
                "Configured node is not viable on this instance and no viable fallback bindings; caller will hard-fail the node-pinned request"
            );
        } else {
            tracing::warn!(
                configured_node_id = %explicit_node_id,
                promoted_node_id = %fallback_node_ids[0],
                service_id = %service_id,
                "Configured node is not viable on this instance; promoting a viable fallback to primary"
            );
        }
    }

    Ok(node_routing_service::build_node_route(
        compose_pre_resolved_node_ids(explicit_node_id, primary_viable, fallback_node_ids),
    ))
}

/// Pure helper: build the ordered node-id list for a pre-resolved route.
///
/// - If the configured node is viable, it goes first and viable fallbacks follow.
/// - If it is not viable, only the viable fallbacks remain; the first one is promoted
///   to primary by the caller via `build_node_route`.
/// - If nothing is viable, returns an empty list. `build_node_route` then yields
///   `None`, and `execute_proxy_inner`'s pre_resolved arm hard-fails with
///   `NodeOffline` to honor the "Route via Node" contract
///   (see ChronoAIProject/NyxID#328).
fn compose_pre_resolved_node_ids(
    explicit_node_id: &str,
    primary_viable: bool,
    fallback_node_ids: Vec<String>,
) -> Vec<String> {
    if primary_viable {
        let mut ids = Vec::with_capacity(fallback_node_ids.len() + 1);
        ids.push(explicit_node_id.to_string());
        ids.extend(fallback_node_ids);
        ids
    } else {
        fallback_node_ids
    }
}

async fn preflight_proxy_deny_before_resolution(
    state: &AppState,
    auth_user: &AuthUser,
    via_service: Option<&str>,
    slug: Option<&str>,
    catalog_service_id: Option<&str>,
    path: &str,
    method: &str,
) -> AppResult<()> {
    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();
    let hint = if let Some(user_service_id) = via_service {
        proxy_service::find_approval_resolution_hint_by_user_service_id(
            &state.db,
            &approval_owner_user_id,
            user_service_id,
            slug,
            catalog_service_id,
        )
        .await?
    } else {
        proxy_service::find_approval_resolution_hint(
            &state.db,
            &approval_owner_user_id,
            slug,
            catalog_service_id,
        )
        .await?
    };

    let hint = if let Some(hint) = hint {
        Some(hint)
    } else if let Some(service_id) = catalog_service_id {
        Some(proxy_service::ApprovalResolutionHint {
            service_id: service_id.to_string(),
            service_owner_id: approval_owner_user_id.clone(),
        })
    } else if let Some(slug) = slug {
        let service = proxy_service::resolve_service_by_slug(&state.db, slug).await?;
        Some(proxy_service::ApprovalResolutionHint {
            service_id: service.id,
            service_owner_id: approval_owner_user_id.clone(),
        })
    } else {
        None
    };

    let Some(hint) = hint else {
        return Ok(());
    };

    let operation = operation_descriptor::build_http_descriptor(method, path, None);
    let denied = approval_service::evaluate_deny_only(
        &state.db,
        &approval_owner_user_id,
        &hint.service_owner_id,
        &hint.service_id,
        &operation,
    )
    .await?;

    if denied {
        return Err(AppError::Forbidden(
            "Operation denied by approval policy".to_string(),
        ));
    }

    Ok(())
}

/// Inner proxy execution with optional pre-resolved target from UserService path.
///
/// When `pre_resolved` is `Some`, the target and node routing are already known
/// (from `resolve_proxy_target_from_user_service`). When `None`, falls back to
/// the original DownstreamService resolution.
async fn execute_proxy_inner(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
    pre_resolved: Option<PreResolved>,
    resolved_slug: &mut String,
) -> AppResult<Response> {
    let user_id_str = auth_user.user_id.to_string();

    // Per-agent rate limit check (before any work). Emit a
    // `proxy_request_denied` audit event on 429 so Usage aggregation can
    // count rate-limited requests in both `request_count` and `error_count`
    // (see ChronoAIProject/NyxID#341).
    if let Err(e) =
        crate::mw::rate_limit::check_agent_rate_limit(&state.per_agent_limiter, auth_user)
    {
        audit_service::log_for_user(
            state.db.clone(),
            auth_user,
            "proxy_request_denied",
            Some(serde_json::json!({
                "service_id": service_id,
                "path": path,
                "reason": e.to_string(),
                "denial_reason": "rate_limited",
                "response_status": 429,
            })),
        );
        return Err(e);
    }

    let approval_owner_user_id = auth_user.effective_approval_owner_user_id();

    // The user_id that owns the resolved UserService, when known. For
    // personal services or for legacy fallback paths this stays None and
    // approval policy resolution falls back to the actor's settings.
    // Captured outside the resolution match so the downstream approval
    // block can apply the org-aware cascade.
    let mut effective_owner_for_approval: Option<String> = None;

    // Resolve target and node routing.
    //
    // `node_routing_required` is true when the service is explicitly
    // configured to route through a node (UserService.node_id is set).
    // When true, the request must NOT silently fall back to direct
    // routing if all node attempts fail (ChronoAIProject/NyxID#328).
    let (
        node_route,
        target,
        has_server_credential,
        resolved_user_service_id,
        node_routing_required,
    ) = if let Some(mut pre) = pre_resolved {
        effective_owner_for_approval = Some(pre.effective_owner_id.clone());
        // New UserService path: target already resolved.
        // Use the resolved service's effective owner (the org's user_id
        // for org-routed calls, the actor for personal) when looking up
        // the node fallback list, so the failover candidates reflect
        // the org's bindings rather than just the actor's personal ones.
        let mut node_route = build_pre_resolved_node_route(
            state,
            &pre.effective_owner_id,
            service_id,
            pre.node_id.as_deref(),
        )
        .await?;

        // Hard-fail when the UserService pins a node (Route via Node) but
        // `build_pre_resolved_node_route` could not resolve a viable node
        // and no fallback binding exists. Falling through to direct routing
        // would silently bypass node isolation, local credentials, and
        // private-network access -- the exact contract "Route via Node"
        // promises. The legacy DownstreamService path enforces the same
        // invariant in `resolve_via_downstream_service`; this mirror keeps
        // the UserService path honest. See ChronoAIProject/NyxID#328.
        if node_route.is_none() && pre.node_id.as_deref().is_some_and(|nid| !nid.is_empty()) {
            let err = AppError::NodeOffline(
                "Service is configured to route via a node, but no viable node is available"
                    .to_string(),
            );
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "proxy_request_denied",
                Some(serde_json::json!({
                    "service_id": service_id,
                    "user_service_id": pre.user_service_id,
                    "configured_node_id": pre.node_id,
                    "path": path,
                    "reason": err.to_string(),
                    "denial_reason": "node_routing_required_no_viable_node",
                    "node_routing_required": true,
                })),
            );
            return Err(err);
        }

        // API key scope enforcement. Emit a `proxy_request_denied` audit
        // event on 403 so Usage aggregation can count scope-forbidden
        // requests in both `request_count` and `error_count`
        // (see ChronoAIProject/NyxID#341).
        if let Some(ref us_id) = pre.user_service_id
            && !auth_user.allow_all_services
            && !auth_user.allowed_service_ids.contains(us_id)
        {
            let err = AppError::ApiKeyScopeForbidden(
                "API key does not have access to this service".to_string(),
            );
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "proxy_request_denied",
                Some(serde_json::json!({
                    "service_id": service_id,
                    "user_service_id": us_id,
                    "path": path,
                    "reason": err.to_string(),
                    "denial_reason": "api_key_scope_forbidden_service",
                    "response_status": 403,
                })),
            );
            return Err(err);
        }
        if let Some(ref nid) = pre.node_id
            && !auth_user.allow_all_nodes
            && !auth_user.allowed_node_ids.contains(nid)
        {
            let err = AppError::ApiKeyScopeForbidden(
                "API key does not have access to this node".to_string(),
            );
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "proxy_request_denied",
                Some(serde_json::json!({
                    "service_id": service_id,
                    "node_id": nid,
                    "path": path,
                    "reason": err.to_string(),
                    "denial_reason": "api_key_scope_forbidden_node",
                    "response_status": 403,
                })),
            );
            return Err(err);
        }
        if !auth_user.allow_all_nodes
            && let Some(route) = node_route.as_mut()
        {
            route
                .fallback_node_ids
                .retain(|nid| auth_user.allowed_node_ids.contains(nid));
        }

        // Per-agent credential override: if this request is via an API key and
        // the user has bound a different credential for this service, swap it in.
        if let (Some(ak_id), Some(us_id)) = (&auth_user.api_key_id, &pre.user_service_id)
            && let Some(override_cred) = proxy_service::resolve_agent_credential_override(
                &state.db,
                &state.encryption_keys,
                &user_id_str,
                ak_id,
                us_id,
            )
            .await?
        {
            pre.target.credential = override_cred;
        }

        let required = pre.node_id.is_some();
        (
            node_route,
            pre.target,
            pre.has_server_credential,
            pre.user_service_id,
            required,
        )
    } else {
        // Old DownstreamService path -- scoped keys must use configured
        // services. Emit a `proxy_request_denied` audit event on 403 so
        // Usage aggregation counts these failures
        // (see ChronoAIProject/NyxID#341).
        if !auth_user.allow_all_services {
            let err = AppError::ApiKeyScopeForbidden(
                "Scoped API keys must use configured services".to_string(),
            );
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "proxy_request_denied",
                Some(serde_json::json!({
                    "service_id": service_id,
                    "path": path,
                    "reason": err.to_string(),
                    "denial_reason": "api_key_scope_forbidden_legacy",
                    "response_status": 403,
                })),
            );
            return Err(err);
        }

        let resolved =
            resolve_via_downstream_service(state, auth_user, &user_id_str, service_id).await?;
        // Legacy path resolution succeeded — this is still personal
        // routing (caller's own DownstreamService + provider token), so
        // attribute it the same way the UserService path is attributed.
        // `user_service_id` is `None` because the legacy path doesn't own
        // one. Emitted AFTER `resolve_via_downstream_service` returns Ok
        // so we never record a "routed via personal" entry for a request
        // that failed before a target was resolved (disconnected service,
        // missing credential, etc.). See ChronoAIProject/NyxID#423.
        audit_personal_routing(state, auth_user, None, service_id);
        resolved
    };

    // Record the resolved service slug so the outer wrapper can attach it
    // to `TelemetryEvent::ProxyError` if any downstream error branch fires
    // before the handler returns `Ok`.
    *resolved_slug = target.service.slug.clone();

    // === Request Decomposition ===
    // Extract method, query, headers BEFORE body consumption.
    let method = request.method().clone();
    let method_str = method.as_str().to_string();
    // Strip NyxID-only routing params (e.g. `_nyxid_via`) from the
    // query string before forwarding. Downstream services should never
    // see NyxID-internal parameters.
    let query = request
        .uri()
        .query()
        .map(strip_internal_query_params)
        .filter(|q| !q.is_empty());
    let all_headers = request.headers().clone();

    // Extract the caller's raw Bearer token for nyxid_token passthrough.
    let caller_token = all_headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(String::from);

    // Check for WebSocket upgrade BEFORE consuming the request body.
    let is_ws = is_ws_upgrade_request(&request);

    // Reject multi-range requests with excessive ranges (DoS prevention)
    validate_range_header(&all_headers)?;

    // Headers safe to forward to node agents. The HTTP and WS paths both
    // honor the `x-openclaw-*` prefix so OpenClaw routing / scope headers
    // reach the downstream even when a new variant is introduced (NyxID#161).
    let node_forward_headers = collect_forward_headers_with_prefixes(
        &all_headers,
        ALLOWED_FORWARD_HEADERS,
        ALLOWED_FORWARD_HEADER_PREFIXES,
    );
    let ws_forward_headers = collect_forward_headers_with_prefixes(
        &all_headers,
        ALLOWED_WS_FORWARD_HEADERS,
        ALLOWED_FORWARD_HEADER_PREFIXES,
    );

    // === Request body handling ===
    // For WebSocket upgrades, skip body buffering -- WS handshakes have no
    // meaningful body, and consuming it would prevent the protocol upgrade.
    // The request is kept intact for WebSocketUpgrade extraction later.
    let (body_bytes, ws_request) = if is_ws {
        (bytes::Bytes::new(), Some(request))
    } else {
        // Always buffer proxy request bodies up to the configured limit.
        //
        // This preserves a hard cap for all proxy uploads, including raw
        // Request<Body> handlers where DefaultBodyLimit alone would not apply.
        let bytes = read_proxy_request_body(request, state.config.proxy_max_body_size).await?;
        (bytes, None)
    };

    let operation = operation_descriptor::build_http_descriptor(
        &method_str,
        path,
        if body_bytes.is_empty() {
            None
        } else {
            Some(body_bytes.as_ref())
        },
    );

    // Resolve approval policy with org-cascade. The "service owner" (the
    // user_id that owns the resolved UserService) determines whether an
    // org policy applies. For the legacy DownstreamService fallback path
    // where no PreResolved was supplied, the service owner is the actor
    // (no org context available).
    let service_owner_for_approval = effective_owner_for_approval
        .as_deref()
        .unwrap_or(&approval_owner_user_id);
    let approval_outcome = approval_service::evaluate_and_check(
        &state.db,
        &approval_owner_user_id,
        service_owner_for_approval,
        service_id,
        &operation,
        auth_user.approval_requester_type(),
        &auth_user.approval_requester_id(),
        auth_user.auth_method == crate::mw::auth::AuthMethod::Session,
    )
    .await?;

    let enforce_approval = match &approval_outcome {
        approval_service::ApprovalOutcome::Allowed { required } => {
            *required && auth_user.auth_method != crate::mw::auth::AuthMethod::Session
        }
        approval_service::ApprovalOutcome::Denied => false,
        approval_service::ApprovalOutcome::NeedsApproval(_) => true,
    };

    match approval_outcome {
        approval_service::ApprovalOutcome::Allowed { .. } => {}
        approval_service::ApprovalOutcome::Denied => {
            return Err(AppError::Forbidden(
                "Operation denied by approval policy".to_string(),
            ));
        }
        approval_service::ApprovalOutcome::NeedsApproval(pending) => {
            let notify_user_ids = approval_service::approval_notification_recipients(
                &state.db,
                &approval_owner_user_id,
                &pending,
            )
            .await?;
            let timeout_recipient = notify_user_ids.first().cloned().ok_or_else(|| {
                AppError::Internal("approval recipient list unexpectedly empty".to_string())
            })?;
            let channel =
                notification_service::get_or_create_channel(&state.db, &timeout_recipient).await?;

            let timeout_secs = channel.approval_timeout_secs;
            let request_operation = approval_service::ApprovalRequestOperation::from_descriptor(
                &operation,
                pending.resolution.grant_scope.clone(),
            );
            let approval_request = approval_service::create_approval_request(
                &state.db,
                &state.config,
                &state.http_client,
                state.fcm_auth.as_deref(),
                state.apns_auth.as_deref(),
                &pending.primary_owner_user_id,
                service_id,
                &target.service.name,
                &target.service.slug,
                &pending.requester_type,
                &pending.requester_id,
                None,
                request_operation,
                pending.resolution.mode.clone(),
                timeout_secs,
                notify_user_ids,
                pending.resolution.from_org_policy,
            )
            .await?;

            // Block until the user approves/rejects or timeout expires
            let req_id = approval_request.id.clone();
            approval_service::wait_for_decision(&state.db, &approval_request.id, timeout_secs)
                .await
                .map_err(|error| {
                    approval_service::map_wait_for_decision_error(
                        error,
                        &req_id,
                        &state.config.frontend_url,
                    )
                })?;
        }
    }

    let body = if body_bytes.is_empty() {
        None
    } else {
        Some(body_bytes)
    };

    // === Delegated Credentials ===
    // Delegation resolves a legacy `UserProviderToken` and injects it as a
    // header / bearer / query / path credential. That flow belongs to the
    // pre-streamlined-services world where the user "connected" a provider
    // separately from choosing a service. The new-path `UserService` carries
    // its own `UserApiKey` credential plus an `auth_method` snapshot, so the
    // proxy already injects the right credential directly from
    // `target.credential` -- calling delegation on top would either
    // double-inject (if both paths hold credentials) or hard-fail with
    // "Provider ... connection required" for users who only set up their
    // credential via AI Services (no UserProviderToken ever created).
    //
    // Skip delegation entirely when the target came from the new path.
    // For node-routed legacy services the node agent injects credentials
    // locally, so a missing server-side provider token is not fatal.
    let delegated = if resolved_user_service_id.is_some() {
        Vec::new()
    } else {
        let delegated_owner = effective_owner_for_approval
            .as_deref()
            .unwrap_or(&user_id_str);
        match delegation_service::resolve_delegated_credentials(
            &state.db,
            &state.encryption_keys,
            delegated_owner,
            service_id,
        )
        .await
        {
            Ok(creds) => creds,
            Err(e) if node_route.is_some() => {
                tracing::debug!(
                    service_id = %service_id,
                    error = %e,
                    "Server-side provider credentials unavailable; \
                     node agent will inject credentials"
                );
                vec![]
            }
            Err(e) => {
                return Err(AppError::BadRequest(format!(
                    "Provider credentials not available: {e}"
                )));
            }
        }
    };

    // Build identity headers before the node/direct split so both proxy paths
    // preserve the same downstream identity and delegation context.
    let mut identity_headers = Vec::new();

    if target.service.identity_propagation_mode != "none" {
        let user = state
            .db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id_str })
            .await?;

        if let Some(ref user) = user {
            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "headers" | "both"
            ) {
                identity_headers = identity_service::build_identity_headers(user, &target.service);
            }

            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "jwt" | "both"
            ) {
                match identity_service::generate_identity_assertion(
                    &state.jwt_keys,
                    &state.config,
                    user,
                    &target.service,
                    &state.db,
                )
                .await
                {
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

    // === WebSocket Passthrough ===
    // If this is a WS upgrade request, branch into the WS path now that
    // target, credentials, and identity headers are fully resolved.
    if let Some(ws_request) = ws_request {
        // WS connections are not compatible with per-request approval.
        if enforce_approval {
            return Err(AppError::BadRequest(
                "WebSocket connections are not supported for services requiring approval"
                    .to_string(),
            ));
        }

        let (mut parts, _body) = ws_request.into_parts();
        let ws_upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(ws) => ws,
            Err(rejection) => {
                return Ok(rejection.into_response());
            }
        };

        // Node-routed WS passthrough: tunnel through the management WS.
        if let Some(ref node_route) = node_route {
            let proxy_actor_user_id = auth_user.proxy_resolution_user_id();
            return handle_ws_passthrough_via_node(
                ws_upgrade,
                state,
                auth_user,
                service_id,
                path,
                &target,
                &delegated,
                &identity_headers,
                query.as_deref(),
                node_route,
                &ws_forward_headers,
                service_owner_for_approval,
                &proxy_actor_user_id,
            )
            .await;
        }

        // Direct WS passthrough: connect to downstream directly.
        return handle_ws_passthrough(
            ws_upgrade,
            state,
            auth_user,
            service_id,
            path,
            &target,
            &delegated,
            &identity_headers,
            query.as_deref(),
            &ws_forward_headers,
            caller_token.as_deref(),
        )
        .await;
    }

    // === Node Proxy Routing (v2: failover + streaming + metrics + HMAC signing) ===
    // node_route was resolved earlier (before credential check) to allow node-backed
    // users to bypass credential requirements.
    if let Some(node_route) = node_route {
        let mut node_delegated = delegated.clone();
        proxy_service::extend_with_path_credential(&mut node_delegated, &target);
        let prepared =
            proxy_service::prepare_delegated_request(path, query.as_deref(), &node_delegated)?;
        let node_path = if prepared.path.starts_with('/') {
            prepared.path.clone()
        } else {
            format!("/{}", prepared.path)
        };

        let mut enriched_headers = node_forward_headers;
        enriched_headers.extend(identity_headers.iter().cloned());

        // Override User-Agent if the service specifies a custom one.
        // By default (None), the client's User-Agent is forwarded as-is.
        // NyxID#514: when neither caller nor service supplies a UA,
        // inject `NyxID-Proxy/{version}` as a benign fallback so
        // UA-required APIs (GitHub etc.) don't 403 silently.
        if let Some(ref ua) = target.service.custom_user_agent {
            enriched_headers.retain(|(name, _)| !name.eq_ignore_ascii_case("user-agent"));
            enriched_headers.push(("user-agent".to_string(), ua.clone()));
        } else if !enriched_headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        {
            enriched_headers.push((
                "user-agent".to_string(),
                proxy_service::DEFAULT_PROXY_USER_AGENT.to_string(),
            ));
        }

        // Forward the caller's NyxID access token when the service is configured for it.
        if target.service.forward_access_token
            && let Some(ref token) = caller_token
        {
            enriched_headers.push(("authorization".to_string(), format!("Bearer {token}")));
        }

        // Merge service-level default headers (NyxID#356) into the node
        // request. The node agent applies these alongside the caller /
        // identity headers when it builds the outbound request on the
        // user's machine. Merge semantics match the direct HTTP path in
        // `proxy_service::forward_request`.
        //
        // Delegated provider credentials (e.g. Anthropic `x-api-key`,
        // Google `x-goog-api-key`) are appended AFTER this merge, so a
        // colliding non-overridable default cannot replace the real
        // downstream token — see the equivalent block in `forward_request`.
        enriched_headers = crate::models::default_request_header::merge_into_header_list(
            enriched_headers,
            &[
                target.catalog_default_headers.as_slice(),
                target.user_service_default_headers.as_slice(),
            ],
        );

        // Strip any header whose name will collide with what the node
        // agent appends locally when it applies `auth_method`, plus any
        // delegated-credential names we are about to re-append below.
        // Without this, a catalog/user default called `x-api-key`
        // (or any other `auth_key_name` / delegated name) would ride
        // along on the frame and the node would append the real
        // credential on top — the wire would then carry BOTH values.
        if let Some(cred_name) = proxy_service::credential_header_name(&target) {
            enriched_headers.retain(|(n, _)| !n.eq_ignore_ascii_case(&cred_name));
        }
        for (delegated_name, _) in &prepared.delegated_headers {
            enriched_headers.retain(|(n, _)| !n.eq_ignore_ascii_case(delegated_name));
        }
        // Re-append delegated headers last so they win over any
        // colliding default.
        enriched_headers.extend(prepared.delegated_headers.iter().cloned());

        // Build base node request (will be cloned for failover retries)
        let node_request = NodeProxyRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            service_id: service_id.to_string(),
            service_slug: target.service.slug.clone(),
            base_url: target.base_url.clone(),
            method: method_str.clone(),
            path: node_path,
            query: prepared.query,
            headers: enriched_headers,
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
            let signing_secret = if state.config.node_hmac_signing_enabled {
                match node_service::get_node_signing_secret(
                    &state.db,
                    state.encryption_keys.as_ref(),
                    node_id,
                )
                .await
                {
                    Ok(secret) => Some(secret),
                    Err(AppError::NodeNotFound(message)) => {
                        last_error = Some(AppError::NodeNotFound(message));
                        continue;
                    }
                    Err(AppError::NodeOffline(message)) => {
                        tracing::warn!(
                            node_id = %node_id,
                            "Skipping node route because signing secret is missing"
                        );
                        last_error = Some(AppError::NodeOffline(message));
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            } else {
                None
            };

            let start = std::time::Instant::now();
            let result = state
                .node_ws_manager
                .send_proxy_request(
                    node_id,
                    attempt_request,
                    signing_secret.as_ref().map(|secret| secret.as_slice()),
                )
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

                    let mut response = match proxy_response {
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
                            let idle_timeout_secs = state.config.proxy_stream_idle_timeout_secs;
                            let idle_timeout = std::time::Duration::from_secs(idle_timeout_secs);

                            // Wait for the Start chunk
                            let first = tokio::time::timeout(idle_timeout, rx.recv())
                                .await
                                .map_err(|_| AppError::NodeProxyTimeout)?
                                .ok_or_else(|| {
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

                            // Detect SSE so we can skip content-length (length unknown).
                            // For non-SSE streaming (video, audio, large files), keep
                            // content-length for client download progress / seeking.
                            let node_is_sse = resp_headers.iter().any(|(k, v)| {
                                k.eq_ignore_ascii_case("content-type")
                                    && v.contains("text/event-stream")
                            });

                            for (name, value) in &resp_headers {
                                let name_lower = name.to_lowercase();
                                if node_is_sse && name_lower == "content-length" {
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

                            let service_id_owned = service_id.to_string();
                            let node_id_owned = node_id.to_string();
                            let stream_db = state.db.clone();
                            let stream_user_id = user_id_str.clone();
                            let stream_api_key_id = auth_user.api_key_id.clone();
                            let stream_api_key_name = auth_user.api_key_name.clone();
                            let stream_ip = auth_user.ip_address.clone();
                            let stream_ua = auth_user.user_agent.clone();

                            // Convert the mpsc receiver into a streaming body.
                            let stream = async_stream::stream! {
                                loop {
                                    match tokio::time::timeout(idle_timeout, rx.recv()).await {
                                        Ok(Some(StreamChunk::Data(bytes))) => {
                                            yield Ok::<_, std::io::Error>(bytes::Bytes::from(bytes));
                                        }
                                        Ok(Some(StreamChunk::End)) => break,
                                        Ok(Some(StreamChunk::Error(e))) => {
                                            tracing::error!(
                                                service_id = %service_id_owned,
                                                node_id = %node_id_owned,
                                                error = %e,
                                                "Stream error from node"
                                            );
                                            yield Err(std::io::Error::other(format!(
                                                "node stream error: {e}"
                                            )));
                                            break;
                                        }
                                        Ok(Some(StreamChunk::Start { .. })) => {
                                            // Duplicate start, ignore
                                        }
                                        Ok(Some(StreamChunk::Injected { trigger_kind, frame_index })) => {
                                            audit_service::log_async(
                                                stream_db.clone(),
                                                Some(stream_user_id.clone()),
                                                "ws_frame_auth_injected".to_string(),
                                                Some(serde_json::json!({
                                                    "service_id": service_id_owned,
                                                    "trigger_kind": trigger_kind,
                                                    "frame_index_in": frame_index,
                                                    "routed_via": "node",
                                                    "node_id": node_id_owned,
                                                })),
                                                stream_ip.clone(),
                                                stream_ua.clone(),
                                                stream_api_key_id.clone(),
                                                stream_api_key_name.clone(),
                                            );
                                        }
                                        Ok(None) => break,
                                        Err(_) => {
                                            tracing::warn!(
                                                service_id = %service_id_owned,
                                                node_id = %node_id_owned,
                                                idle_timeout_secs,
                                                "Node proxy stream idle timeout reached"
                                            );
                                            break;
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

                    let proxy_actor_user_id = auth_user.proxy_resolution_user_id();
                    audit_service::log_for_user(
                        state.db.clone(),
                        auth_user,
                        "proxy_request",
                        Some(node_proxy_audit_event_data(
                            service_id,
                            &method_str,
                            path,
                            response.status().as_u16(),
                            node_id,
                            service_owner_for_approval,
                            &proxy_actor_user_id,
                            target.connection_id.as_deref(),
                        )),
                    );

                    apply_agent_attribution_headers(
                        &mut response,
                        auth_user.api_key_id.as_deref(),
                        target.connection_id.as_deref(),
                    );

                    return Ok(response);
                }
                Err(err @ AppError::NodeCredentialMissing(_)) => {
                    // A different fallback node may have the credential
                    // configured locally, so we still try the rest of
                    // the pool. Preserve the original error class in
                    // `last_error` so if every attempt ends up reporting
                    // a missing credential the caller sees the specific
                    // 8004 / 502 rather than a generic `NodeOffline` —
                    // which is the contract issue #418 asks for.
                    tracing::warn!(
                        node_id = %node_id,
                        "Node rejected proxy request: credential missing locally, trying next"
                    );

                    let db_clone = state.db.clone();
                    let nid = node_id.to_string();
                    let err_msg = "Node credential missing".to_string();
                    tokio::spawn(async move {
                        let _ = node_metrics_service::record_error(db_clone, nid, err_msg).await;
                    });

                    last_error = Some(err);
                    continue;
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

        // All nodes failed.
        //
        // Hard-fail when:
        //   * The service is explicitly node-routed (Route via Node) — falling
        //     back to direct routing would violate the routing contract and
        //     silently bypass node isolation, local credentials, or
        //     private-network access. (ChronoAIProject/NyxID#328)
        //   * No server-side credential is available, so direct routing
        //     cannot succeed anyway.
        if node_routing_required || !has_server_credential {
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "proxy_request_denied",
                Some(serde_json::json!({
                    "service_id": service_id,
                    "reason": "all_node_routes_failed",
                    "node_routing_required": node_routing_required,
                    "attempted_node_ids": all_node_ids,
                })),
            );
            return Err(last_error.unwrap_or_else(|| {
                AppError::NodeOffline(if node_routing_required {
                    "Service is configured to route via a node, but all node routes failed"
                        .to_string()
                } else {
                    "All node routes failed and no server-side credential is available".to_string()
                })
            }));
        }

        // Fall through to standard proxy with server-side credential.
        // Reachable only when the service is NOT explicitly node-routed
        // (e.g. opportunistic NodeServiceBinding fallback).
        if let Some(err) = last_error {
            tracing::warn!(
                service_id = %service_id,
                error = %err,
                "All node proxies failed, falling through to standard proxy"
            );
        }
    }
    // === END Node Proxy Routing ===

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

        let mut response = chatgpt_translator::send_to_chatgpt(
            &translated.body,
            &bearer_token,
            is_streaming,
            is_chat_completions_path,
            query.as_deref(),
            Some(llm_usage_service::UsageAuditContext {
                db: state.db.clone(),
                user_id: user_id_str.clone(),
                provider_slug: None,
                service_id: Some(service_id.to_string()),
                model: body_json
                    .get("model")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                path: path.to_string(),
                api_key_id: auth_user.api_key_id.clone(),
                api_key_name: auth_user.api_key_name.clone(),
            }),
        )
        .await?;

        let status = response.status();

        audit_service::log_for_user(
            state.db.clone(),
            auth_user,
            "proxy_request",
            Some(serde_json::json!({
                "service_id": service_id,
                "method": method.as_str(),
                "path": path,
                "response_status": status.as_u16(),
                "acting_client_id": &auth_user.acting_client_id,
                "codex_transport": true,
                "connection_id": target.connection_id.as_deref(),
            })),
        );

        apply_agent_attribution_headers(
            &mut response,
            auth_user.api_key_id.as_deref(),
            target.connection_id.as_deref(),
        );

        return Ok(response);
    }

    // Reuse the shared reqwest::Client from AppState for connection pooling.
    let downstream_response = proxy_service::forward_request(
        &state.http_client,
        &target,
        reqwest_method,
        path,
        query.as_deref(),
        reqwest_headers,
        proxy_service::ProxyBody::Buffered(body),
        identity_headers,
        delegated,
        caller_token.as_deref(),
        &state.token_exchange_cache,
        &state.cloud_response_cache,
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
    let should_stream = should_stream_response(&downstream_response, status, is_sse);
    let usage_context =
        target
            .service
            .slug
            .starts_with("llm-")
            .then(|| llm_usage_service::UsageAuditContext {
                db: state.db.clone(),
                user_id: user_id_str.clone(),
                provider_slug: None,
                service_id: Some(service_id.to_string()),
                model: None,
                path: path.to_string(),
                api_key_id: auth_user.api_key_id.clone(),
                api_key_name: auth_user.api_key_name.clone(),
            });

    let mut response_builder = Response::builder().status(status);

    // Forward only allowlisted response headers.
    // Skip content-length for SSE (length unknown). Keep it for other
    // streaming responses — clients need it for download progress / seeking.
    for (name, value) in downstream_response.headers().iter() {
        let name_lower = name.as_str().to_lowercase();
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

    let mut response = if should_stream {
        // Stream responses without buffering, but use a forwarding task when
        // we need to observe SSE usage so client disconnects are visible.
        if let Some(stream_usage_context) = if is_sse { usage_context.clone() } else { None } {
            let service_id_owned = service_id.to_string();
            let idle_timeout =
                std::time::Duration::from_secs(state.config.proxy_stream_idle_timeout_secs);
            let idle_timeout_secs = state.config.proxy_stream_idle_timeout_secs;
            let mut upstream_stream = downstream_response.bytes_stream();
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);

            tokio::spawn(async move {
                let mut sse_buffer = String::new();
                let mut usage_accumulator =
                    llm_usage_service::ReportedLlmUsageAccumulator::default();

                loop {
                    match tokio::time::timeout(idle_timeout, upstream_stream.next()).await {
                        Ok(Some(Ok(bytes))) => {
                            sse_buffer.push_str(&String::from_utf8_lossy(&bytes));
                            while let Some(event) = parse_sse_event(&mut sse_buffer) {
                                if let Some((usage, mode)) =
                                    llm_usage_service::extract_reported_usage_from_sse_event(
                                        event.event_type.as_deref(),
                                        &event.data,
                                    )
                                {
                                    usage_accumulator.observe(usage, mode);
                                }
                            }

                            if tx.send(Ok(bytes)).await.is_err() {
                                if let Some(usage) = usage_accumulator.finalize() {
                                    llm_usage_service::log_reported_usage_async(
                                        stream_usage_context.clone(),
                                        usage,
                                    );
                                }
                                return;
                            }
                        }
                        Ok(Some(Err(e))) => {
                            tracing::error!(
                                service_id = %service_id_owned,
                                error = %e,
                                error_debug = ?e,
                                "Proxy stream error from upstream — connection dropped"
                            );
                            if let Some(usage) = usage_accumulator.finalize() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let _ = tx
                                .send(Err(std::io::Error::other(format!(
                                    "upstream stream error: {e}"
                                ))))
                                .await;
                            return;
                        }
                        Ok(None) => {
                            if let Some(usage) = usage_accumulator.finalize() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            return;
                        }
                        Err(_) => {
                            tracing::warn!(
                                service_id = %service_id_owned,
                                idle_timeout_secs,
                                "Proxy stream idle timeout reached"
                            );
                            if let Some(usage) = usage_accumulator.finalize() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            return;
                        }
                    }
                }
            });

            let body = Body::from_stream(ReceiverStream::new(rx));
            response_builder
                .body(body)
                .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
        } else {
            let service_id_owned = service_id.to_string();
            let idle_timeout =
                std::time::Duration::from_secs(state.config.proxy_stream_idle_timeout_secs);
            let idle_timeout_secs = state.config.proxy_stream_idle_timeout_secs;
            let mut upstream_stream = downstream_response.bytes_stream();
            let stream = async_stream::stream! {
                loop {
                    match tokio::time::timeout(idle_timeout, upstream_stream.next()).await {
                        Ok(Some(Ok(bytes))) => {
                            yield Ok::<_, std::io::Error>(bytes);
                        }
                        Ok(Some(Err(e))) => {
                            tracing::error!(
                                service_id = %service_id_owned,
                                error = %e,
                                error_debug = ?e,
                                "Proxy stream error from upstream — connection dropped"
                            );
                            yield Err(std::io::Error::other(format!(
                                "upstream stream error: {e}"
                            )));
                            break;
                        }
                        Ok(None) => break,
                        Err(_) => {
                            tracing::warn!(
                                service_id = %service_id_owned,
                                idle_timeout_secs,
                                "Proxy stream idle timeout reached"
                            );
                            break;
                        }
                    }
                }
            };
            let body = Body::from_stream(stream);
            response_builder
                .body(body)
                .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
        }
    } else {
        // Buffer small / error responses so we can log diagnostics.
        let response_body = downstream_response
            .bytes()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read downstream response: {e}")))?;

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

        if let Some(nonstream_usage_context) = usage_context
            && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response_body)
            && let Some(usage) = llm_usage_service::extract_reported_usage(&json)
        {
            llm_usage_service::log_reported_usage_async(nonstream_usage_context, usage);
        }

        response_builder
            .body(Body::from(response_body))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
    };

    // Audit log the proxy request
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "proxy_request",
        Some(serde_json::json!({
            "service_id": service_id,
            "method": method.as_str(),
            "path": path,
            "response_status": status.as_u16(),
            "acting_client_id": &auth_user.acting_client_id,
            "connection_id": target.connection_id.as_deref(),
        })),
    );

    apply_agent_attribution_headers(
        &mut response,
        auth_user.api_key_id.as_deref(),
        target.connection_id.as_deref(),
    );

    Ok(response)
}

/// Attach agent/connection attribution headers to a proxy response.
///
/// `X-NyxID-Agent-Id` is set only when the request authenticated via an API
/// key (`AuthUser.api_key_id` is `Some`), letting callers confirm which agent
/// identity NyxID attributed the request to. Session-token (browser) auth
/// leaves `api_key_id` `None`, so the header is omitted. `X-NyxID-Connection-Id`
/// is set when the resolved target carries a connection id. Values that cannot
/// be encoded as header values are silently skipped.
fn apply_agent_attribution_headers(
    response: &mut Response,
    api_key_id: Option<&str>,
    connection_id: Option<&str>,
) {
    if let Some(agent_id) = api_key_id
        && let Ok(val) = axum::http::HeaderValue::from_str(agent_id)
    {
        response.headers_mut().insert("x-nyxid-agent-id", val);
    }
    if let Some(conn_id) = connection_id
        && let Ok(val) = axum::http::HeaderValue::from_str(conn_id)
    {
        response.headers_mut().insert("x-nyxid-connection-id", val);
    }
}

async fn read_proxy_request_body(
    request: Request<Body>,
    max_body_size: usize,
) -> AppResult<bytes::Bytes> {
    axum::body::to_bytes(request.into_body(), max_body_size)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read body: {e}")))
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

#[cfg(test)]
fn should_enforce_runtime_approval(
    requires_approval: bool,
    auth_method: &crate::mw::auth::AuthMethod,
) -> bool {
    requires_approval && *auth_method != crate::mw::auth::AuthMethod::Session
}

/// Convenience alias so existing call-sites compile without renaming.
fn parse_sse_event(buffer: &mut String) -> Option<sse_parser::SseEvent> {
    sse_parser::parse_next_event(buffer)
}

/// Threshold below which non-error responses are buffered (so small API
/// responses keep the existing diagnostic-logging path).
const STREAM_SIZE_THRESHOLD: u64 = 256 * 1024;

/// Content types that should always be streamed regardless of size.
const STREAMING_CONTENT_TYPES: &[&str] = &[
    "text/event-stream",
    "video/",
    "audio/",
    "application/octet-stream",
    "image/",
    "application/pdf",
];

/// Decide whether a downstream response should be streamed to the client
/// instead of buffered in memory.
///
/// Streams when ANY of these is true:
/// - Content-Type is SSE, video, audio, octet-stream, image, or PDF
/// - Content-Length is absent (unknown size) or exceeds [`STREAM_SIZE_THRESHOLD`]
/// - HTTP status is 206 Partial Content (range response)
///
/// Buffers when the response is small and not a streaming content type,
/// preserving the error-body diagnostic logging for typical API errors.
fn should_stream_response(response: &reqwest::Response, status: StatusCode, is_sse: bool) -> bool {
    // SSE always streams (existing behaviour)
    if is_sse {
        return true;
    }

    // 206 Partial Content always streams (range responses)
    if status == StatusCode::PARTIAL_CONTENT {
        return true;
    }

    // Check content type for media / binary types
    if let Some(ct) = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
    {
        let ct_lower = ct.to_lowercase();
        if STREAMING_CONTENT_TYPES
            .iter()
            .any(|prefix| ct_lower.starts_with(prefix))
        {
            return true;
        }
    }

    // Stream when content-length is absent (unknown size) or large
    match response.content_length() {
        None => true,
        Some(len) => len > STREAM_SIZE_THRESHOLD,
    }
}

/// Validate that a Range header doesn't contain too many ranges (DoS prevention).
/// RFC 7233 recommends limiting multi-range requests.
fn validate_range_header(headers: &axum::http::HeaderMap) -> AppResult<()> {
    const MAX_RANGES: usize = 4;
    if let Some(range) = headers.get("range").and_then(|v| v.to_str().ok()) {
        let range_count = range.matches(',').count() + 1;
        if range_count > MAX_RANGES {
            return Err(AppError::BadRequest(format!(
                "Too many byte ranges requested ({range_count}), maximum is {MAX_RANGES}"
            )));
        }
    }
    Ok(())
}

// === WebSocket Passthrough Support ===

/// Detect whether an inbound request is a WebSocket upgrade by checking
/// the `Connection: upgrade` and `Upgrade: websocket` headers.
fn is_ws_upgrade_request(request: &Request<Body>) -> bool {
    let headers = request.headers();
    let has_upgrade = headers
        .get(axum::http::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let has_connection = headers
        .get(axum::http::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        });
    has_upgrade && has_connection
}

/// Build a downstream WebSocket URL from the proxy target, applying
/// credential injection (path, query) via `prepare_delegated_request`.
fn build_downstream_ws_url(
    target: &proxy_service::ProxyTarget,
    path: &str,
    query: Option<&str>,
    delegated: &[delegation_service::DelegatedCredential],
) -> AppResult<String> {
    let mut all_delegated = delegated.to_vec();
    proxy_service::extend_with_path_credential(&mut all_delegated, target);
    let prepared = proxy_service::prepare_delegated_request(path, query, &all_delegated)?;

    let base = target.base_url.trim_end_matches('/');
    let ws_base = if base.starts_with("https://") {
        base.replacen("https://", "wss://", 1)
    } else if base.starts_with("http://") {
        base.replacen("http://", "ws://", 1)
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_string()
    } else {
        return Err(AppError::Internal(format!(
            "Unsupported base URL scheme for WebSocket passthrough: {}",
            base.split("://").next().unwrap_or("unknown")
        )));
    };

    let ws_path = if prepared.path.is_empty() {
        String::new()
    } else if prepared.path.starts_with('/') {
        prepared.path
    } else {
        format!("/{}", prepared.path)
    };

    let mut url = match prepared.query {
        Some(q) => format!("{ws_base}{ws_path}?{q}"),
        None => format!("{ws_base}{ws_path}"),
    };

    if target.auth_method == "query" {
        url = append_query_param(&url, &target.auth_key_name, &target.credential);
    }

    Ok(url)
}

/// Connect to a downstream WebSocket, injecting credentials and identity
/// headers into the upgrade request.
async fn connect_downstream_ws(
    url: &str,
    target: &proxy_service::ProxyTarget,
    delegated: &[delegation_service::DelegatedCredential],
    identity_headers: &[(String, String)],
    forward_headers: &[(String, String)],
    caller_token: Option<&str>,
) -> AppResult<DownstreamWsConnection> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let mut request = url
        .into_client_request()
        .map_err(|e| AppError::Internal(format!("Failed to build downstream WS request: {e}")))?;

    let headers = request.headers_mut();

    // Helper to build a header (name, value) pair, returning an error instead
    // of silently dropping credentials that fail to parse.
    let make_header =
        |name_bytes: &[u8],
         value_str: &str|
         -> AppResult<(reqwest::header::HeaderName, reqwest::header::HeaderValue)> {
            let name = reqwest::header::HeaderName::from_bytes(name_bytes).map_err(|e| {
                AppError::Internal(format!("Invalid header name for WS credential: {e}"))
            })?;
            let value = reqwest::header::HeaderValue::from_str(value_str).map_err(|e| {
                AppError::Internal(format!("Invalid header value for WS credential: {e}"))
            })?;
            Ok((name, value))
        };

    // Header injection order mirrors the direct HTTP path so the two
    // transports produce the same wire output for the same config.
    //
    // Precedence (low → high; later layers overwrite earlier ones via
    // `HeaderMap::insert`):
    //   1. Caller handshake metadata (`forward_headers`)
    //   2. Identity propagation headers
    //   3. Service default headers (catalog + user-service, NyxID#356)
    //   4. Service `custom_user_agent` override, OR `NyxID-Proxy/{version}`
    //      fallback when neither caller nor service supplies a UA (NyxID#514)
    //   5. Delegated provider credential headers
    //   6. `forward_access_token` NyxID bearer
    //   7. Service auth credential (auth_method)
    //
    // Delegated provider credentials (5) run AFTER defaults (3) so a
    // non-overridable default cannot clobber the real downstream token
    // (e.g. Anthropic `x-api-key`, Google `x-goog-api-key`) for services
    // using `auth_method = "none"` plus `ServiceProviderRequirement`.
    // The service `auth_method` credential (7) still wins over
    // everything when it also sets the same name.

    // [1] Caller handshake metadata (Origin, Sec-WebSocket-*, etc.)
    for (name, value) in forward_headers {
        if let (Ok(hn), Ok(hv)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            headers.insert(hn, hv);
        }
    }

    // [2] Identity propagation headers (best-effort -- these are internal)
    for (name, value) in identity_headers {
        if let (Ok(hn), Ok(hv)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            headers.insert(hn, hv);
        }
    }

    // [3] Service-level default headers. Catalog layer first, then
    // user-service overrides. Non-overridable defaults replace anything
    // set by layers 1–2; overridable defaults only fill in when the
    // handshake doesn't already carry that header.
    for h in target
        .catalog_default_headers
        .iter()
        .chain(target.user_service_default_headers.iter())
    {
        let hn = match reqwest::header::HeaderName::from_bytes(h.name.as_bytes()) {
            Ok(n) => n,
            Err(_) => continue, // validated on write, but stay defensive
        };
        let hv = match reqwest::header::HeaderValue::from_str(&h.value) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if h.overridable {
            headers.entry(hn).or_insert(hv);
        } else {
            headers.insert(hn, hv);
        }
    }

    // [4] Override User-Agent if the service specifies a custom one.
    // NyxID#514: when neither caller nor service supplies a UA, inject
    // `NyxID-Proxy/{version}` so UA-required APIs don't 403 silently.
    // The service `custom_user_agent` and any caller-supplied UA still win.
    if let Some(ref ua) = target.service.custom_user_agent {
        if let Ok(hv) = reqwest::header::HeaderValue::from_str(ua) {
            headers.insert(reqwest::header::USER_AGENT, hv);
        }
    } else if !headers.contains_key(reqwest::header::USER_AGENT)
        && let Ok(hv) =
            reqwest::header::HeaderValue::from_str(proxy_service::DEFAULT_PROXY_USER_AGENT)
    {
        headers.insert(reqwest::header::USER_AGENT, hv);
    }

    // [5] Delegated provider credential headers. Applied AFTER defaults
    // via `HeaderMap::insert` so a colliding non-overridable default
    // gets replaced by the real downstream token — the service
    // `auth_method` credential below still wins if it targets the same
    // name.
    for cred in delegated {
        match cred.injection_method.as_str() {
            "bearer" => {
                let (name, value) = make_header(
                    cred.injection_key.as_bytes(),
                    &format!("Bearer {}", cred.credential),
                )?;
                headers.insert(name, value);
            }
            "header" => {
                let (name, value) = make_header(cred.injection_key.as_bytes(), &cred.credential)?;
                headers.insert(name, value);
            }
            // "query" and "path" already handled in URL construction
            _ => {}
        }
    }

    // [6] Forward the caller's NyxID access token when the service is
    // configured for it.
    if target.service.forward_access_token
        && let Some(token) = caller_token
    {
        let (_, value) = make_header(b"authorization", &format!("Bearer {token}"))?;
        headers.insert(reqwest::header::AUTHORIZATION, value);
    }

    // [7] Service credential — injected LAST so it always wins over any
    // default/delegated/identity header with the same name.
    match target.auth_method.as_str() {
        "none" => {}
        "header" => {
            let (name, value) = make_header(target.auth_key_name.as_bytes(), &target.credential)?;
            headers.insert(name, value);
        }
        "bearer" => {
            let (_, value) =
                make_header(b"authorization", &format!("Bearer {}", target.credential))?;
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        "basic" => {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&target.credential);
            let (_, value) = make_header(b"authorization", &format!("Basic {encoded}"))?;
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        // "query" and "path" are already handled in URL construction
        "query" | "path" => {}
        other => {
            return Err(AppError::Internal(format!(
                "Unsupported auth method for WS passthrough: {other}"
            )));
        }
    }

    let mut ws_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
    ws_config.max_message_size = Some(WS_PASSTHROUGH_MAX_MESSAGE_SIZE);
    ws_config.max_frame_size = Some(WS_PASSTHROUGH_MAX_MESSAGE_SIZE);
    let (ws_stream, response) = tokio::time::timeout(
        std::time::Duration::from_secs(WS_PASSTHROUGH_CONNECT_TIMEOUT_SECS),
        tokio_tungstenite::connect_async_with_config(request, Some(ws_config), false),
    )
    .await
    .map_err(|_| AppError::Internal("Downstream WS connection timed out".to_string()))?
    .map_err(|e| AppError::Internal(format!("Downstream WS connection failed: {e}")))?;

    let selected_protocol = response
        .headers()
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    Ok(DownstreamWsConnection {
        stream: ws_stream,
        selected_protocol,
    })
}

/// Convert an axum WS message to a tungstenite WS message.
fn axum_msg_to_tungstenite(
    msg: axum::extract::ws::Message,
) -> tokio_tungstenite::tungstenite::Message {
    use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

    match msg {
        axum::extract::ws::Message::Text(t) => {
            tokio_tungstenite::tungstenite::Message::Text(t.to_string().into())
        }
        axum::extract::ws::Message::Binary(b) => {
            tokio_tungstenite::tungstenite::Message::Binary(b.to_vec().into())
        }
        axum::extract::ws::Message::Ping(p) => {
            tokio_tungstenite::tungstenite::Message::Ping(p.to_vec().into())
        }
        axum::extract::ws::Message::Pong(p) => {
            tokio_tungstenite::tungstenite::Message::Pong(p.to_vec().into())
        }
        axum::extract::ws::Message::Close(frame) => {
            tokio_tungstenite::tungstenite::Message::Close(frame.map(|f| CloseFrame {
                code: CloseCode::from(f.code),
                reason: f.reason.to_string().into(),
            }))
        }
    }
}

/// Convert a tungstenite WS message to an axum WS message.
fn tungstenite_msg_to_axum(
    msg: tokio_tungstenite::tungstenite::Message,
) -> axum::extract::ws::Message {
    match msg {
        tokio_tungstenite::tungstenite::Message::Text(t) => {
            axum::extract::ws::Message::Text(t.to_string().into())
        }
        tokio_tungstenite::tungstenite::Message::Binary(b) => axum::extract::ws::Message::Binary(b),
        tokio_tungstenite::tungstenite::Message::Ping(p) => axum::extract::ws::Message::Ping(p),
        tokio_tungstenite::tungstenite::Message::Pong(p) => axum::extract::ws::Message::Pong(p),
        tokio_tungstenite::tungstenite::Message::Close(frame) => {
            axum::extract::ws::Message::Close(frame.map(|f| axum::extract::ws::CloseFrame {
                code: f.code.into(),
                reason: f.reason.to_string().into(),
            }))
        }
        tokio_tungstenite::tungstenite::Message::Frame(_) => {
            tracing::warn!("Received unexpected raw tungstenite frame in WS bridge");
            axum::extract::ws::Message::Binary(bytes::Bytes::new())
        }
    }
}

fn axum_msg_payload_for_injection(
    msg: &axum::extract::ws::Message,
) -> Option<(crate::models::ws_frame_injection::WsFrameKind, Vec<u8>)> {
    match msg {
        axum::extract::ws::Message::Text(t) => Some((
            crate::models::ws_frame_injection::WsFrameKind::Text,
            t.to_string().into_bytes(),
        )),
        axum::extract::ws::Message::Binary(b) => Some((
            crate::models::ws_frame_injection::WsFrameKind::Binary,
            b.to_vec(),
        )),
        _ => None,
    }
}

fn tungstenite_msg_payload_for_injection(
    msg: &tokio_tungstenite::tungstenite::Message,
) -> Option<(crate::models::ws_frame_injection::WsFrameKind, Vec<u8>)> {
    match msg {
        tokio_tungstenite::tungstenite::Message::Text(t) => Some((
            crate::models::ws_frame_injection::WsFrameKind::Text,
            t.to_string().into_bytes(),
        )),
        tokio_tungstenite::tungstenite::Message::Binary(b) => Some((
            crate::models::ws_frame_injection::WsFrameKind::Binary,
            b.to_vec(),
        )),
        _ => None,
    }
}

fn injection_frame_to_tungstenite(
    frame: ws_frame_injector::WsFrame,
) -> tokio_tungstenite::tungstenite::Message {
    match frame.kind {
        crate::models::ws_frame_injection::WsFrameKind::Text => {
            let text = String::from_utf8(frame.payload).unwrap_or_default();
            tokio_tungstenite::tungstenite::Message::Text(text.into())
        }
        crate::models::ws_frame_injection::WsFrameKind::Binary => {
            tokio_tungstenite::tungstenite::Message::Binary(frame.payload.into())
        }
    }
}

fn injection_frame_to_axum(frame: ws_frame_injector::WsFrame) -> axum::extract::ws::Message {
    match frame.kind {
        crate::models::ws_frame_injection::WsFrameKind::Text => {
            let text = String::from_utf8(frame.payload).unwrap_or_default();
            axum::extract::ws::Message::Text(text.into())
        }
        crate::models::ws_frame_injection::WsFrameKind::Binary => {
            axum::extract::ws::Message::Binary(frame.payload.into())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn audit_ws_frame_auth_injected(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    action: &ws_frame_injector::InjectionAction,
    routed_node_id: Option<&str>,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
    ip_address: Option<String>,
    user_agent: Option<String>,
) {
    let mut data = serde_json::json!({
        "service_id": service_id,
        "trigger_kind": action.trigger_kind,
        "frame_index_in": action.frame_index_in,
    });
    if let Some(node_id) = routed_node_id
        && let Some(obj) = data.as_object_mut()
    {
        obj.insert("routed_via".to_string(), serde_json::json!("node"));
        obj.insert("node_id".to_string(), serde_json::json!(node_id));
    }

    audit_service::log_async(
        db.clone(),
        Some(user_id.to_string()),
        "ws_frame_auth_injected".to_string(),
        Some(data),
        ip_address,
        user_agent,
        api_key_id,
        api_key_name,
    );
}

/// Maximum duration for a single WS passthrough session (seconds).
const WS_PASSTHROUGH_MAX_DURATION_SECS: u64 = 3600;
/// Idle timeout: close the bridge if no frames pass in either direction (seconds).
const WS_PASSTHROUGH_IDLE_TIMEOUT_SECS: u64 = 300;

/// Bridge two WebSocket connections bidirectionally, forwarding frames
/// between the client (axum) and downstream (tungstenite) sides.
///
/// Uses a single-loop `tokio::select!` over both streams so that cleanup
/// (close frames) runs for **both** sides regardless of which side closes
/// first. Enforces idle timeout and max session duration.
///
/// Returns the session duration for audit logging.
#[allow(clippy::too_many_arguments)]
async fn bridge_websockets(
    client_ws: axum::extract::ws::WebSocket,
    downstream_ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    service_id: String,
    ws_frame_injections: Vec<crate::models::ws_frame_injection::WsFrameInjection>,
    credential: String,
    db: mongodb::Database,
    user_id: String,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> std::time::Duration {
    let start = std::time::Instant::now();
    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut downstream_sink, mut downstream_stream) = downstream_ws.split();
    let mut injector_state = ws_frame_injector::InjectorState::default();

    let max_duration = tokio::time::sleep(std::time::Duration::from_secs(
        WS_PASSTHROUGH_MAX_DURATION_SECS,
    ));
    tokio::pin!(max_duration);

    let idle_timeout = tokio::time::sleep(std::time::Duration::from_secs(
        WS_PASSTHROUGH_IDLE_TIMEOUT_SECS,
    ));
    tokio::pin!(idle_timeout);

    loop {
        tokio::select! {
            _ = &mut max_duration => {
                tracing::info!(
                    service_id = %service_id,
                    "WS passthrough max duration reached"
                );
                break;
            }
            _ = &mut idle_timeout => {
                tracing::info!(
                    service_id = %service_id,
                    "WS passthrough idle timeout reached"
                );
                break;
            }
            msg = client_stream.next() => {
                match msg {
                    Some(Ok(axum_msg)) => {
                        idle_timeout
                            .as_mut()
                            .reset(tokio::time::Instant::now() + std::time::Duration::from_secs(WS_PASSTHROUGH_IDLE_TIMEOUT_SECS));
                        if let Some((kind, payload)) = axum_msg_payload_for_injection(&axum_msg) {
                            let frame = ws_frame_injector::IncomingFrame {
                                direction: crate::models::ws_frame_injection::WsFrameDirection::Upstream,
                                kind,
                                payload,
                            };
                            if let Some(action) = ws_frame_injector::evaluate(
                                &ws_frame_injections,
                                &mut injector_state,
                                &frame,
                                &credential,
                            ) {
                                tracing::info!(
                                    service_id = %service_id,
                                    trigger_kind = action.trigger_kind,
                                    frame_index_in = action.frame_index_in,
                                    credential_sha256_prefix = %action.credential_sha256_prefix,
                                    "Injected WebSocket auth frame"
                                );
                                audit_ws_frame_auth_injected(
                                    &db,
                                    &user_id,
                                    &service_id,
                                    &action,
                                    None,
                                    api_key_id.clone(),
                                    api_key_name.clone(),
                                    ip_address.clone(),
                                    user_agent.clone(),
                                );
                                // NOTE: `direction` is the trigger direction. The injected
                                // frame is sent to the opposite side so a downstream
                                // auth challenge can produce an upstream auth response.
                                if client_sink
                                    .send(injection_frame_to_axum(action.send_frame))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                if !action.forward_original {
                                    continue;
                                }
                            }
                        }
                        let is_close = matches!(axum_msg, axum::extract::ws::Message::Close(_));
                        let _ = downstream_sink.send(axum_msg_to_tungstenite(axum_msg)).await;
                        if is_close {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::debug!(service_id = %service_id, error = %e, "Client WS recv error");
                        break;
                    }
                    None => break,
                }
            }
            msg = downstream_stream.next() => {
                match msg {
                    Some(Ok(tung_msg)) => {
                        idle_timeout
                            .as_mut()
                            .reset(tokio::time::Instant::now() + std::time::Duration::from_secs(WS_PASSTHROUGH_IDLE_TIMEOUT_SECS));
                        if let Some((kind, payload)) = tungstenite_msg_payload_for_injection(&tung_msg) {
                            let frame = ws_frame_injector::IncomingFrame {
                                direction: crate::models::ws_frame_injection::WsFrameDirection::Downstream,
                                kind,
                                payload,
                            };
                            if let Some(action) = ws_frame_injector::evaluate(
                                &ws_frame_injections,
                                &mut injector_state,
                                &frame,
                                &credential,
                            ) {
                                tracing::info!(
                                    service_id = %service_id,
                                    trigger_kind = action.trigger_kind,
                                    frame_index_in = action.frame_index_in,
                                    credential_sha256_prefix = %action.credential_sha256_prefix,
                                    "Injected WebSocket auth frame"
                                );
                                audit_ws_frame_auth_injected(
                                    &db,
                                    &user_id,
                                    &service_id,
                                    &action,
                                    None,
                                    api_key_id.clone(),
                                    api_key_name.clone(),
                                    ip_address.clone(),
                                    user_agent.clone(),
                                );
                                // NOTE: `direction` is the trigger direction. For the HA
                                // preset this sends the auth frame back upstream to the
                                // downstream socket and consumes the auth_required frame.
                                if downstream_sink
                                    .send(injection_frame_to_tungstenite(action.send_frame))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                if !action.forward_original {
                                    continue;
                                }
                            }
                        }
                        let is_close = matches!(tung_msg, tokio_tungstenite::tungstenite::Message::Close(_));
                        let _ = client_sink.send(tungstenite_msg_to_axum(tung_msg)).await;
                        if is_close {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::debug!(service_id = %service_id, error = %e, "Downstream WS recv error");
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    // Close both sides -- runs regardless of which side triggered the break.
    let _ = downstream_sink.close().await;
    let _ = client_sink.close().await;

    start.elapsed()
}

/// Maximum WS message size (16 MB) -- limits both client and downstream frames.
const WS_PASSTHROUGH_MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;
/// Maximum time spent establishing the downstream WS connection.
const WS_PASSTHROUGH_CONNECT_TIMEOUT_SECS: u64 = 10;

/// RAII guard that decrements the WS passthrough connection counter on drop.
/// Prevents counter leaks if the `on_upgrade` callback is never invoked
/// (e.g. client disconnects between HTTP response and WS handshake).
struct WsPassthroughGuard {
    counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl WsPassthroughGuard {
    fn new(counter: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Self {
        Self { counter }
    }
}

impl Drop for WsPassthroughGuard {
    fn drop(&mut self) {
        self.counter
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Handle a WebSocket passthrough: connect to the downstream service,
/// upgrade the client connection, and bridge frames bidirectionally.
#[allow(clippy::too_many_arguments)]
async fn handle_ws_passthrough(
    ws_upgrade: WebSocketUpgrade,
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    target: &proxy_service::ProxyTarget,
    delegated: &[delegation_service::DelegatedCredential],
    identity_headers: &[(String, String)],
    query: Option<&str>,
    forward_headers: &[(String, String)],
    caller_token: Option<&str>,
) -> AppResult<Response> {
    let downstream_url = build_downstream_ws_url(target, path, query, delegated)?;

    // Enforce global connection limit.
    let current = state
        .ws_passthrough_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if current >= state.config.ws_passthrough_max_connections {
        state
            .ws_passthrough_count
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        return Err(AppError::RateLimited);
    }
    // Guard auto-decrements the counter on all error-return paths and if
    // the on_upgrade callback is never invoked.
    let guard = WsPassthroughGuard::new(state.ws_passthrough_count.clone());

    // Connect to downstream BEFORE upgrading the client connection.
    // If the downstream is unreachable, the client gets a normal HTTP error.
    let downstream = connect_downstream_ws(
        &downstream_url,
        target,
        delegated,
        identity_headers,
        forward_headers,
        caller_token,
    )
    .await?;

    let user_id_str = auth_user.user_id.to_string();
    let service_id_owned = service_id.to_string();
    let acting_client_id = auth_user.acting_client_id.clone();
    let ak_id = auth_user.api_key_id.clone();
    let ak_name = auth_user.api_key_name.clone();
    let req_ip = auth_user.ip_address.clone();
    let req_ua = auth_user.user_agent.clone();

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str.clone()),
        "proxy_ws_upgrade".to_string(),
        Some(serde_json::json!({
            "service_id": service_id,
            "path": path,
            "acting_client_id": &acting_client_id,
        })),
        req_ip.clone(),
        req_ua.clone(),
        ak_id.clone(),
        ak_name.clone(),
    );

    let db = state.db.clone();
    let sid = service_id_owned.clone();
    let ws_frame_injections = target.ws_frame_injections.clone();
    let credential = target.credential.clone();
    let ws_upgrade = ws_upgrade.max_message_size(WS_PASSTHROUGH_MAX_MESSAGE_SIZE);
    let ws_upgrade = if let Some(protocol) = downstream.selected_protocol.clone() {
        ws_upgrade.protocols([protocol])
    } else {
        ws_upgrade
    };

    Ok(ws_upgrade
        .on_upgrade(move |client_ws| async move {
            let duration = bridge_websockets(
                client_ws,
                downstream.stream,
                sid.clone(),
                ws_frame_injections,
                credential,
                db.clone(),
                user_id_str.clone(),
                ak_id.clone(),
                ak_name.clone(),
                req_ip.clone(),
                req_ua.clone(),
            )
            .await;
            drop(guard); // decrement counter (guard moved into closure)

            audit_service::log_async(
                db,
                Some(user_id_str),
                "proxy_ws_disconnect".to_string(),
                Some(serde_json::json!({
                    "service_id": sid,
                    "duration_secs": duration.as_secs(),
                    "acting_client_id": &acting_client_id,
                })),
                req_ip,
                req_ua,
                ak_id,
                ak_name,
            );
        })
        .into_response())
}

/// Handle a WebSocket passthrough routed through a credential node.
/// Opens a WS proxy session via the node's management WS, then bridges
/// frames between the client and the node-relayed downstream connection.
#[allow(clippy::too_many_arguments)]
async fn handle_ws_passthrough_via_node(
    ws_upgrade: WebSocketUpgrade,
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    target: &proxy_service::ProxyTarget,
    delegated: &[delegation_service::DelegatedCredential],
    identity_headers: &[(String, String)],
    query: Option<&str>,
    node_route: &node_routing_service::NodeRoute,
    forward_headers: &[(String, String)],
    service_owner_user_id: &str,
    proxy_actor_user_id: &str,
) -> AppResult<Response> {
    use crate::services::node_ws_manager::NodeWsProxyRequest;

    // Prepare headers for the node request (same as HTTP node proxy).
    let mut node_ws_delegated = delegated.to_vec();
    proxy_service::extend_with_path_credential(&mut node_ws_delegated, target);
    let prepared = proxy_service::prepare_delegated_request(path, query, &node_ws_delegated)?;
    let node_path = if prepared.path.starts_with('/') {
        prepared.path.clone()
    } else {
        format!("/{}", prepared.path)
    };
    let mut enriched_headers = forward_headers.to_vec();
    enriched_headers.extend(identity_headers.iter().cloned());

    // Override User-Agent if the service specifies a custom one.
    // NyxID#514: when neither caller nor service supplies a UA, inject
    // `NyxID-Proxy/{version}` so UA-required APIs don't 403 silently.
    // The service `custom_user_agent` and any caller-supplied UA still win.
    if let Some(ref ua) = target.service.custom_user_agent {
        enriched_headers.retain(|(name, _)| !name.eq_ignore_ascii_case("user-agent"));
        enriched_headers.push(("user-agent".to_string(), ua.clone()));
    } else if !enriched_headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
    {
        enriched_headers.push((
            "user-agent".to_string(),
            proxy_service::DEFAULT_PROXY_USER_AGENT.to_string(),
        ));
    }

    // Merge service-level default headers (NyxID#356) — same semantics as
    // the direct WS path and the node-routed HTTP path. Delegated
    // credentials are appended AFTER this merge so a colliding
    // non-overridable default cannot clobber the real provider token.
    enriched_headers = crate::models::default_request_header::merge_into_header_list(
        enriched_headers,
        &[
            target.catalog_default_headers.as_slice(),
            target.user_service_default_headers.as_slice(),
        ],
    );

    // Strip the name the node agent will append its own credential on,
    // plus any delegated-credential names we are about to re-append
    // below, so the WS handshake doesn't carry both the default and the
    // real credential.
    if let Some(cred_name) = proxy_service::credential_header_name(target) {
        enriched_headers.retain(|(n, _)| !n.eq_ignore_ascii_case(&cred_name));
    }
    for (delegated_name, _) in &prepared.delegated_headers {
        enriched_headers.retain(|(n, _)| !n.eq_ignore_ascii_case(delegated_name));
    }
    // Re-append delegated headers last so they win over colliding defaults.
    enriched_headers.extend(prepared.delegated_headers.iter().cloned());

    let all_node_ids: Vec<&str> = std::iter::once(node_route.node_id.as_str())
        .chain(node_route.fallback_node_ids.iter().map(|id| id.as_str()))
        .collect();

    // Enforce global connection limit.
    let current = state
        .ws_passthrough_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if current >= state.config.ws_passthrough_max_connections {
        state
            .ws_passthrough_count
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        return Err(AppError::RateLimited);
    }
    // Guard auto-decrements the counter on all error-return paths and if
    // the on_upgrade callback is never invoked.
    let guard = WsPassthroughGuard::new(state.ws_passthrough_count.clone());

    let session_id = uuid::Uuid::new_v4().to_string();
    let mut last_error: Option<AppError> = None;
    let mut selected_node_id: Option<String> = None;
    let mut ws_proxy_session = None;
    let mut saw_ws_proxy_timeout = false;

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
                Err(AppError::NodeNotFound(message)) => {
                    last_error = Some(AppError::NodeNotFound(message));
                    continue;
                }
                Err(AppError::NodeOffline(message)) => {
                    tracing::warn!(
                        node_id = %node_id,
                        "Skipping WS node route because signing secret is missing"
                    );
                    last_error = Some(AppError::NodeOffline(message));
                    continue;
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };

        let ws_proxy_request = NodeWsProxyRequest {
            session_id: session_id.clone(),
            service_slug: target.service.slug.clone(),
            base_url: target.base_url.clone(),
            path: node_path.clone(),
            query: prepared.query.clone(),
            headers: enriched_headers.clone(),
            ws_frame_injections: target.ws_frame_injections.clone(),
        };

        match state
            .node_ws_manager
            .open_ws_proxy(
                node_id,
                ws_proxy_request,
                signing_secret.as_ref().map(|secret| secret.as_slice()),
            )
            .await
        {
            Ok(session) => {
                selected_node_id = Some((*node_id).to_string());
                ws_proxy_session = Some(session);
                break;
            }
            Err(AppError::NodeOffline(_)) => {
                tracing::warn!(node_id = %node_id, "WS node proxy failed, trying next");
                last_error = Some(AppError::NodeOffline(format!("Node {node_id} failed")));
            }
            Err(AppError::NodeProxyTimeout) => {
                tracing::warn!(node_id = %node_id, "WS node proxy timed out, trying next");
                saw_ws_proxy_timeout = true;
                last_error = Some(AppError::NodeProxyTimeout);
            }
            Err(error) => return Err(error),
        }
    }

    let Some(node_id) = selected_node_id else {
        // guard drops here, decrementing the counter
        return if saw_ws_proxy_timeout {
            Err(AppError::BadRequest(
                "WebSocket proxy timed out. The node agent may not support WebSocket \
                 passthrough. Update the node CLI: \
                 bash -c \"$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)\" \
                 then restart the node with: nyxid node daemon restart"
                    .to_string(),
            ))
        } else {
            match last_error {
                Some(error) => Err(error),
                None => Err(AppError::NodeOffline(
                    "All node routes failed for WebSocket passthrough".to_string(),
                )),
            }
        };
    };
    let ws_proxy_session = ws_proxy_session.expect("selected node must have an open session");

    let user_id_str = auth_user.user_id.to_string();
    let service_id_owned = service_id.to_string();
    let acting_client_id = auth_user.acting_client_id.clone();
    let node_id_owned = node_id.to_string();
    let ak_id = auth_user.api_key_id.clone();
    let ak_name = auth_user.api_key_name.clone();
    let req_ip = auth_user.ip_address.clone();
    let req_ua = auth_user.user_agent.clone();
    let service_owner_user_id_owned = service_owner_user_id.to_string();
    let proxy_actor_user_id_owned = proxy_actor_user_id.to_string();

    let mut upgrade_event = serde_json::json!({
        "service_id": service_id,
        "path": path,
        "acting_client_id": &acting_client_id,
        "routed_via": "node",
        "node_id": node_id,
    });
    add_owner_user_id_if_shared(
        &mut upgrade_event,
        service_owner_user_id,
        proxy_actor_user_id,
    );
    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str.clone()),
        "proxy_ws_upgrade".to_string(),
        Some(upgrade_event),
        req_ip.clone(),
        req_ua.clone(),
        ak_id.clone(),
        ak_name.clone(),
    );

    let db = state.db.clone();
    let ws_manager = state.node_ws_manager.clone();
    let sid = service_id_owned.clone();
    let sess_id = session_id.clone();
    let owner_for_audit = service_owner_user_id_owned.clone();
    let actor_for_audit = proxy_actor_user_id_owned.clone();
    let ws_upgrade = ws_upgrade.max_message_size(WS_PASSTHROUGH_MAX_MESSAGE_SIZE);
    let ws_upgrade = if let Some(protocol) = ws_proxy_session.selected_protocol.clone() {
        ws_upgrade.protocols([protocol])
    } else {
        ws_upgrade
    };

    Ok(ws_upgrade
        .on_upgrade(move |client_ws| async move {
            let duration = bridge_websockets_via_node(
                client_ws,
                ws_proxy_session.frames,
                &ws_manager,
                &node_id_owned,
                &sess_id,
                sid.clone(),
                db.clone(),
                user_id_str.clone(),
                ak_id.clone(),
                ak_name.clone(),
                owner_for_audit.clone(),
                actor_for_audit.clone(),
                req_ip.clone(),
                req_ua.clone(),
            )
            .await;

            // Best-effort close the node-side session.
            let _ = ws_manager.send_ws_proxy_close(&node_id_owned, &sess_id, None, None);
            drop(guard); // explicitly decrement counter

            let mut disconnect_event = serde_json::json!({
                "service_id": sid,
                "duration_secs": duration.as_secs(),
                "acting_client_id": &acting_client_id,
                "routed_via": "node",
                "node_id": node_id_owned,
            });
            add_owner_user_id_if_shared(&mut disconnect_event, &owner_for_audit, &actor_for_audit);
            audit_service::log_async(
                db,
                Some(user_id_str),
                "proxy_ws_disconnect".to_string(),
                Some(disconnect_event),
                req_ip,
                req_ua,
                ak_id,
                ak_name,
            );
        })
        .into_response())
}

/// Bridge client WS frames to/from a node-relayed WS proxy session.
#[allow(clippy::too_many_arguments)]
async fn bridge_websockets_via_node(
    client_ws: axum::extract::ws::WebSocket,
    mut ws_proxy_rx: tokio::sync::mpsc::Receiver<crate::services::node_ws_manager::WsProxyFrame>,
    ws_manager: &crate::services::node_ws_manager::NodeWsManager,
    node_id: &str,
    session_id: &str,
    service_id: String,
    db: mongodb::Database,
    user_id: String,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
    service_owner_user_id: String,
    proxy_actor_user_id: String,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> std::time::Duration {
    use crate::services::node_ws_manager::WsProxyFrame;

    let start = std::time::Instant::now();
    let (mut client_sink, mut client_stream) = client_ws.split();

    let max_duration = tokio::time::sleep(std::time::Duration::from_secs(
        WS_PASSTHROUGH_MAX_DURATION_SECS,
    ));
    tokio::pin!(max_duration);

    let idle_timeout = tokio::time::sleep(std::time::Duration::from_secs(
        WS_PASSTHROUGH_IDLE_TIMEOUT_SECS,
    ));
    tokio::pin!(idle_timeout);

    loop {
        tokio::select! {
            _ = &mut max_duration => {
                tracing::info!(service_id = %service_id, "Node WS passthrough max duration reached");
                break;
            }
            _ = &mut idle_timeout => {
                tracing::info!(service_id = %service_id, "Node WS passthrough idle timeout reached");
                break;
            }
            // Client -> Node
            msg = client_stream.next() => {
                match msg {
                    Some(Ok(axum::extract::ws::Message::Text(t))) => {
                        idle_timeout.as_mut().reset(
                            tokio::time::Instant::now()
                                + std::time::Duration::from_secs(WS_PASSTHROUGH_IDLE_TIMEOUT_SECS),
                        );
                        if ws_manager.send_ws_proxy_text(node_id, session_id, &t).is_err() {
                            break;
                        }
                    }
                    Some(Ok(axum::extract::ws::Message::Binary(b))) => {
                        idle_timeout.as_mut().reset(
                            tokio::time::Instant::now()
                                + std::time::Duration::from_secs(WS_PASSTHROUGH_IDLE_TIMEOUT_SECS),
                        );
                        if ws_manager.send_ws_proxy_binary(node_id, session_id, &b).is_err() {
                            break;
                        }
                    }
                    Some(Ok(axum::extract::ws::Message::Close(frame))) => {
                        let (code, reason) = frame
                            .map(|f| (Some(f.code), Some(f.reason.to_string())))
                            .unwrap_or((None, None));
                        let _ = ws_manager.send_ws_proxy_close(node_id, session_id, code, reason);
                        break;
                    }
                    Some(Ok(axum::extract::ws::Message::Ping(p))) => {
                        let _ = client_sink.send(axum::extract::ws::Message::Pong(p)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::debug!(service_id = %service_id, error = %e, "Client WS error in node bridge");
                        break;
                    }
                    None => break,
                }
            }
            // Node -> Client
            frame = ws_proxy_rx.recv() => {
                match frame {
                    Some(WsProxyFrame::Text(t)) => {
                        idle_timeout.as_mut().reset(
                            tokio::time::Instant::now()
                                + std::time::Duration::from_secs(WS_PASSTHROUGH_IDLE_TIMEOUT_SECS),
                        );
                        if client_sink
                            .send(axum::extract::ws::Message::Text(t.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Some(WsProxyFrame::Binary(b)) => {
                        idle_timeout.as_mut().reset(
                            tokio::time::Instant::now()
                                + std::time::Duration::from_secs(WS_PASSTHROUGH_IDLE_TIMEOUT_SECS),
                        );
                        if client_sink
                            .send(axum::extract::ws::Message::Binary(b.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Some(WsProxyFrame::Injected { trigger_kind, frame_index }) => {
                        let mut event_data = serde_json::json!({
                            "service_id": service_id,
                            "trigger_kind": trigger_kind,
                            "frame_index_in": frame_index,
                            "routed_via": "node",
                            "node_id": node_id,
                        });
                        add_owner_user_id_if_shared(
                            &mut event_data,
                            &service_owner_user_id,
                            &proxy_actor_user_id,
                        );
                        audit_service::log_async(
                            db.clone(),
                            Some(user_id.clone()),
                            "ws_frame_auth_injected".to_string(),
                            Some(event_data),
                            ip_address.clone(),
                            user_agent.clone(),
                            api_key_id.clone(),
                            api_key_name.clone(),
                        );
                    }
                    Some(WsProxyFrame::Closed { code, reason }) => {
                        let close_frame = code.map(|c| axum::extract::ws::CloseFrame {
                            code: c,
                            reason: reason.unwrap_or_default().into(),
                        });
                        let _ = client_sink
                            .send(axum::extract::ws::Message::Close(close_frame))
                            .await;
                        break;
                    }
                    Some(WsProxyFrame::Error(e)) => {
                        let _ = client_sink
                            .send(axum::extract::ws::Message::Close(Some(
                                axum::extract::ws::CloseFrame {
                                    code: 1011,
                                    reason: e.into(),
                                },
                            )))
                            .await;
                        break;
                    }
                    None => {
                        // Node disconnected
                        let _ = client_sink
                            .send(axum::extract::ws::Message::Close(Some(
                                axum::extract::ws::CloseFrame {
                                    code: 1001,
                                    reason: "Node disconnected".into(),
                                },
                            )))
                            .await;
                        break;
                    }
                }
            }
        }
    }

    let _ = client_sink.close().await;
    start.elapsed()
}

#[cfg(test)]
mod tests {
    use super::{
        ALLOWED_FORWARD_HEADER_PREFIXES, ALLOWED_FORWARD_HEADERS, ALLOWED_WS_FORWARD_HEADERS,
        WsPassthroughGuard, apply_agent_attribution_headers, auth_kind_label,
        collect_forward_headers_with_prefixes, compose_pre_resolved_node_ids,
        is_chat_completions_proxy_path, is_codex_transport_path, is_ws_upgrade_request,
        should_enforce_runtime_approval, validate_range_header,
    };
    use crate::mw::auth::AuthMethod;
    use crate::services::proxy_service::validate_requested_proxy_path;
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::{Path, State, ws::WebSocketUpgrade},
        http::{Request, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use futures::{SinkExt, StreamExt};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tower::ServiceExt;

    // ---- validate_range_header tests ----

    #[test]
    fn auth_kind_label_covers_every_method() {
        // Exhaustive mapping check. If a new variant is added to
        // `AuthMethod`, `auth_kind_label` will fail to compile (match
        // is non-exhaustive) — this test exists to lock in the
        // *string values* that the PostHog property contract depends on.
        assert_eq!(auth_kind_label(&AuthMethod::Session), "session");
        assert_eq!(auth_kind_label(&AuthMethod::AccessToken), "access_token");
        assert_eq!(auth_kind_label(&AuthMethod::Relay), "relay");
        assert_eq!(auth_kind_label(&AuthMethod::ApiKey), "api_key");
        assert_eq!(
            auth_kind_label(&AuthMethod::ServiceAccount),
            "service_account"
        );
        assert_eq!(auth_kind_label(&AuthMethod::Delegated), "delegated");
    }

    // ---- X-NyxID-Agent-Id attribution header tests (issue #788) ----

    #[test]
    fn agent_attribution_sets_agent_id_header_for_api_key_auth() {
        // API-key-authed proxy request (api_key_id = Some) must surface the
        // agent identity to the caller via X-NyxID-Agent-Id.
        let mut response = Body::empty().into_response();
        apply_agent_attribution_headers(&mut response, Some("ag-key-123"), None);

        assert_eq!(
            response
                .headers()
                .get("x-nyxid-agent-id")
                .and_then(|v| v.to_str().ok()),
            Some("ag-key-123"),
            "X-NyxID-Agent-Id must equal the authenticating API key id"
        );
        // No connection id was resolved, so that header stays absent.
        assert!(!response.headers().contains_key("x-nyxid-connection-id"));
    }

    #[test]
    fn agent_attribution_omits_agent_id_header_for_session_auth() {
        // Session/browser auth leaves api_key_id = None; the header must be
        // omitted entirely (callers rely on its absence to distinguish auth).
        let mut response = Body::empty().into_response();
        apply_agent_attribution_headers(&mut response, None, None);

        assert!(
            !response.headers().contains_key("x-nyxid-agent-id"),
            "session-authed responses must not carry X-NyxID-Agent-Id"
        );
    }

    #[test]
    fn agent_attribution_sets_connection_id_header_independently() {
        // Connection id is attached whenever the resolved target has one,
        // independent of the agent-id header.
        let mut response = Body::empty().into_response();
        apply_agent_attribution_headers(&mut response, Some("ag-key-9"), Some("conn-42"));

        assert_eq!(
            response
                .headers()
                .get("x-nyxid-agent-id")
                .and_then(|v| v.to_str().ok()),
            Some("ag-key-9")
        );
        assert_eq!(
            response
                .headers()
                .get("x-nyxid-connection-id")
                .and_then(|v| v.to_str().ok()),
            Some("conn-42")
        );
    }

    #[test]
    fn range_header_absent_is_ok() {
        let headers = axum::http::HeaderMap::new();
        assert!(validate_range_header(&headers).is_ok());
    }

    #[test]
    fn range_header_single_range_is_ok() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("range", "bytes=0-1023".parse().unwrap());
        assert!(validate_range_header(&headers).is_ok());
    }

    #[test]
    fn range_header_four_ranges_is_ok() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("range", "bytes=0-1,2-3,4-5,6-7".parse().unwrap());
        assert!(validate_range_header(&headers).is_ok());
    }

    #[test]
    fn range_header_five_ranges_rejected() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("range", "bytes=0-1,2-3,4-5,6-7,8-9".parse().unwrap());
        assert!(validate_range_header(&headers).is_err());
    }

    // ---- compose_pre_resolved_node_ids tests (issue #325) ----

    #[test]
    fn compose_pre_resolved_node_ids_uses_configured_as_primary_when_viable() {
        let result = compose_pre_resolved_node_ids(
            "configured",
            true,
            vec!["fallback-a".to_string(), "fallback-b".to_string()],
        );

        assert_eq!(
            result,
            vec![
                "configured".to_string(),
                "fallback-a".to_string(),
                "fallback-b".to_string(),
            ]
        );
    }

    #[test]
    fn compose_pre_resolved_node_ids_drops_configured_when_not_viable() {
        let result = compose_pre_resolved_node_ids(
            "configured",
            false,
            vec!["fallback-a".to_string(), "fallback-b".to_string()],
        );

        assert_eq!(
            result,
            vec!["fallback-a".to_string(), "fallback-b".to_string()]
        );
    }

    #[test]
    fn compose_pre_resolved_node_ids_returns_empty_when_nothing_viable() {
        // When the configured node is not viable and no fallback bindings exist,
        // the list must be empty so `build_node_route` returns None. The caller
        // (execute_proxy_inner pre_resolved arm) then hard-fails with
        // `NodeOffline` to honor the "Route via Node" contract rather than
        // silently dropping to direct routing. See ChronoAIProject/NyxID#328.
        let result = compose_pre_resolved_node_ids("configured", false, vec![]);

        assert!(result.is_empty());
    }

    #[test]
    fn compose_pre_resolved_node_ids_keeps_viable_configured_without_fallbacks() {
        let result = compose_pre_resolved_node_ids("configured", true, vec![]);

        assert_eq!(result, vec!["configured".to_string()]);
    }

    #[test]
    fn session_auth_bypasses_even_when_required() {
        assert!(!should_enforce_runtime_approval(true, &AuthMethod::Session));
    }

    #[test]
    fn relay_auth_enforces_approval_when_required() {
        assert!(should_enforce_runtime_approval(true, &AuthMethod::Relay));
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
    fn ws_passthrough_guard_drops_when_upgrade_callback_is_discarded() {
        let counter = Arc::new(AtomicUsize::new(1));
        let guard = WsPassthroughGuard::new(counter.clone());

        // Model axum storing the on_upgrade callback and then dropping it
        // before invocation because the HTTP upgrade never completes.
        let callback = move || {
            let _guard = guard;
        };

        drop(callback);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
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

        let question_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/folder%3Fchat_id=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(question_response.status(), StatusCode::OK);
        let question_body = to_bytes(question_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&question_body).unwrap(),
            "svc:folder?chat_id=1"
        );

        let hash_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/folder%23fragment")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(hash_response.status(), StatusCode::OK);
        let hash_body = to_bytes(hash_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&hash_body).unwrap(),
            "svc:folder#fragment"
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
        assert_eq!(std::str::from_utf8(&dotdot_body).unwrap(), "svc:..");

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

        let double_encoded_question_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/folder%253Fchat_id=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(double_encoded_question_response.status(), StatusCode::OK);
        let double_encoded_question_body =
            to_bytes(double_encoded_question_response.into_body(), usize::MAX)
                .await
                .unwrap();
        assert_eq!(
            std::str::from_utf8(&double_encoded_question_body).unwrap(),
            "svc:folder%3Fchat_id=1"
        );

        let double_encoded_hash_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/svc/folder%2523fragment")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(double_encoded_hash_response.status(), StatusCode::OK);
        let double_encoded_hash_body =
            to_bytes(double_encoded_hash_response.into_body(), usize::MAX)
                .await
                .unwrap();
        assert_eq!(
            std::str::from_utf8(&double_encoded_hash_body).unwrap(),
            "svc:folder%23fragment"
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
            "/sendMessage?chat_id=1",
            "/sendMessage#fragment",
            "/folder%2FsendMessage",
            "/folder%2fsendMessage",
            "/folder%252FsendMessage",
            "/folder%25252FsendMessage",
            "/folder%3Fchat_id=1",
            "/folder%3fchat_id=1",
            "/folder%253Fchat_id=1",
            "/folder%25253Fchat_id=1",
            "/folder%23fragment",
            "/folder%2523fragment",
            "/folder%252523fragment",
            "/%2e%2e",
            "/%252e%252e",
            "/%25252e%25252e",
            "/%2e.",
            "/.%2e",
            "/%2E%2E",
            "/%2E.",
            "/.%2E",
            "/folder%5CsendMessage",
            "/folder%5csendMessage",
            "/folder%255CsendMessage",
            "/folder%25255CsendMessage",
            "/%00",
            "/%2500",
            "/%252500",
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

    // ---- is_ws_upgrade_request tests ----

    #[test]
    fn ws_upgrade_detected_with_correct_headers() {
        let request = Request::builder()
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();
        assert!(is_ws_upgrade_request(&request));
    }

    #[test]
    fn ws_upgrade_case_insensitive() {
        let request = Request::builder()
            .header("connection", "Upgrade")
            .header("upgrade", "WebSocket")
            .body(Body::empty())
            .unwrap();
        assert!(is_ws_upgrade_request(&request));
    }

    #[test]
    fn ws_upgrade_connection_header_with_multiple_values() {
        let request = Request::builder()
            .header("connection", "keep-alive, Upgrade")
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();
        assert!(is_ws_upgrade_request(&request));
    }

    #[test]
    fn ws_upgrade_not_detected_without_upgrade_header() {
        let request = Request::builder()
            .header("connection", "upgrade")
            .body(Body::empty())
            .unwrap();
        assert!(!is_ws_upgrade_request(&request));
    }

    #[test]
    fn ws_upgrade_not_detected_without_connection_header() {
        let request = Request::builder()
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();
        assert!(!is_ws_upgrade_request(&request));
    }

    #[test]
    fn ws_upgrade_not_detected_for_non_websocket_upgrade() {
        let request = Request::builder()
            .header("connection", "upgrade")
            .header("upgrade", "h2c")
            .body(Body::empty())
            .unwrap();
        assert!(!is_ws_upgrade_request(&request));
    }

    #[test]
    fn ws_upgrade_not_detected_for_normal_request() {
        let request = Request::builder().body(Body::empty()).unwrap();
        assert!(!is_ws_upgrade_request(&request));
    }

    #[derive(Clone)]
    struct WsBridgeTestState {
        downstream: Arc<
            tokio::sync::Mutex<
                Option<
                    tokio_tungstenite::WebSocketStream<
                        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                    >,
                >,
            >,
        >,
        db: mongodb::Database,
    }

    async fn ws_bridge_test_handler(
        State(state): State<WsBridgeTestState>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |client_ws| async move {
            let downstream_ws = state
                .downstream
                .lock()
                .await
                .take()
                .expect("downstream socket available");
            super::bridge_websockets(
                client_ws,
                downstream_ws,
                "svc-ha".to_string(),
                vec![crate::models::ws_frame_injection::WsFrameInjection {
                    trigger: crate::models::ws_frame_injection::WsFrameTrigger::JsonFieldEquals {
                        path: "$.type".to_string(),
                        value: serde_json::json!("auth_required"),
                    },
                    template: r#"{"type":"auth","access_token":"${credential}"}"#.to_string(),
                    frame_kind: crate::models::ws_frame_injection::WsFrameKind::Text,
                    consume_trigger: true,
                    direction: crate::models::ws_frame_injection::WsFrameDirection::Downstream,
                }],
                "TEST_CRED".to_string(),
                state.db,
                "user-1".to_string(),
                None,
                None,
                None,
                None,
            )
            .await;
        })
    }

    #[tokio::test]
    async fn bridge_websockets_injects_ha_auth_frame_and_consumes_challenge() {
        let downstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind downstream");
        let downstream_addr = downstream_listener.local_addr().expect("downstream addr");
        let (auth_tx, auth_rx) = tokio::sync::oneshot::channel::<String>();

        let downstream_task = tokio::spawn(async move {
            let (stream, _) = downstream_listener
                .accept()
                .await
                .expect("accept downstream");
            let mut ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept downstream ws");
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                r#"{"type":"auth_required"}"#.into(),
            ))
            .await
            .expect("send auth_required");
            let auth = ws
                .next()
                .await
                .expect("auth response")
                .expect("auth response ok");
            let auth_text = auth.into_text().expect("auth response text").to_string();
            auth_tx.send(auth_text).expect("send auth text");
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                r#"{"type":"auth_ok"}"#.into(),
            ))
            .await
            .expect("send auth_ok");
        });

        let (downstream_ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{downstream_addr}"))
                .await
                .expect("connect bridge downstream");

        let mut client_options =
            mongodb::options::ClientOptions::parse("mongodb://127.0.0.1:27099")
                .await
                .expect("parse mongodb options");
        client_options.server_selection_timeout = Some(std::time::Duration::from_millis(10));
        let db = mongodb::Client::with_options(client_options)
            .expect("mongodb client")
            .database("nyxid_ws_frame_injection_test");

        let state = WsBridgeTestState {
            downstream: Arc::new(tokio::sync::Mutex::new(Some(downstream_ws))),
            db,
        };
        let app = Router::new()
            .route("/ws", get(ws_bridge_test_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind bridge");
        let bridge_addr = listener.local_addr().expect("bridge addr");
        let server_task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve bridge");
        });

        let (mut client_ws, _) = tokio_tungstenite::connect_async(format!("ws://{bridge_addr}/ws"))
            .await
            .expect("connect client");
        let client_msg = client_ws
            .next()
            .await
            .expect("client receives auth_ok")
            .expect("client frame ok")
            .into_text()
            .expect("client text")
            .to_string();

        assert_eq!(client_msg, r#"{"type":"auth_ok"}"#);
        assert_eq!(
            auth_rx.await.expect("auth frame captured"),
            r#"{"type":"auth","access_token":"TEST_CRED"}"#
        );

        let _ = client_ws.close(None).await;
        downstream_task.await.expect("downstream task");
        server_task.abort();
    }

    // ---- build_downstream_ws_url tests ----

    use super::build_downstream_ws_url;
    use crate::services::proxy_service;

    fn make_target(base_url: &str) -> proxy_service::ProxyTarget {
        proxy_service::ProxyTarget {
            base_url: base_url.to_string(),
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            credential: String::new(),
            service: crate::models::downstream_service::test_helpers::dummy_service(),
            catalog_default_headers: Vec::new(),
            user_service_default_headers: Vec::new(),
            ws_frame_injections: Vec::new(),
            connection_id: None,
        }
    }

    #[test]
    fn ws_url_converts_http_to_ws() {
        let target = make_target("http://localhost:8080");
        let url = build_downstream_ws_url(&target, "socket", None, &[]).unwrap();
        assert_eq!(url, "ws://localhost:8080/socket");
    }

    #[test]
    fn ws_url_converts_https_to_wss() {
        let target = make_target("https://api.example.com");
        let url = build_downstream_ws_url(&target, "ws", None, &[]).unwrap();
        assert_eq!(url, "wss://api.example.com/ws");
    }

    #[test]
    fn ws_url_preserves_query_params() {
        let target = make_target("http://localhost:8080");
        let url = build_downstream_ws_url(&target, "socket", Some("token=abc&v=1"), &[]).unwrap();
        assert_eq!(url, "ws://localhost:8080/socket?token=abc&v=1");
    }

    #[test]
    fn ws_url_appends_service_query_auth() {
        let mut target = make_target("https://api.example.com");
        target.auth_method = "query".to_string();
        target.auth_key_name = "api_key".to_string();
        target.credential = "secret value".to_string();

        let url = build_downstream_ws_url(&target, "socket", Some("stream=true"), &[]).unwrap();
        assert_eq!(
            url,
            "wss://api.example.com/socket?stream=true&api_key=secret%20value"
        );
    }

    #[test]
    fn ws_url_handles_trailing_slash_on_base() {
        let target = make_target("http://localhost:8080/");
        let url = build_downstream_ws_url(&target, "socket.io", None, &[]).unwrap();
        assert_eq!(url, "ws://localhost:8080/socket.io");
    }

    #[test]
    fn ws_url_passes_through_ws_scheme() {
        let target = make_target("ws://localhost:8080");
        let url = build_downstream_ws_url(&target, "socket", None, &[]).unwrap();
        assert_eq!(url, "ws://localhost:8080/socket");
    }

    #[test]
    fn ws_url_passes_through_wss_scheme() {
        let target = make_target("wss://secure.example.com");
        let url = build_downstream_ws_url(&target, "ws", None, &[]).unwrap();
        assert_eq!(url, "wss://secure.example.com/ws");
    }

    #[test]
    fn ws_url_rejects_unsupported_scheme() {
        let target = make_target("ftp://internal-server");
        let result = build_downstream_ws_url(&target, "socket", None, &[]);
        assert!(result.is_err());
    }

    // ---- forward header allowlist tests (NyxID#161) ----

    fn node_forward_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
        collect_forward_headers_with_prefixes(
            headers,
            ALLOWED_FORWARD_HEADERS,
            ALLOWED_FORWARD_HEADER_PREFIXES,
        )
    }

    fn ws_forward_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
        collect_forward_headers_with_prefixes(
            headers,
            ALLOWED_WS_FORWARD_HEADERS,
            ALLOWED_FORWARD_HEADER_PREFIXES,
        )
    }

    #[test]
    fn node_forward_preserves_openclaw_scopes_header() {
        // NyxID#161: `x-openclaw-scopes` was silently dropped because the
        // allowlist enumerated only a few x-openclaw-* names. Assert the
        // prefix rule keeps caller-supplied scopes reaching the downstream.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-openclaw-scopes",
            "operator.read,operator.write".parse().unwrap(),
        );

        let forwarded = node_forward_headers(&headers);
        assert!(
            forwarded
                .iter()
                .any(|(n, v)| n.eq_ignore_ascii_case("x-openclaw-scopes")
                    && v == "operator.read,operator.write"),
            "x-openclaw-scopes must reach node-routed downstream (NyxID#161)"
        );
    }

    #[test]
    fn node_forward_preserves_arbitrary_openclaw_prefixed_headers() {
        // Any future x-openclaw-* header should pass through automatically.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-openclaw-tenant", "acme".parse().unwrap());
        headers.insert("x-openclaw-trace-id", "abc-123".parse().unwrap());

        let forwarded = node_forward_headers(&headers);
        assert!(
            forwarded
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("x-openclaw-tenant")),
            "x-openclaw-tenant must pass through",
        );
        assert!(
            forwarded
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("x-openclaw-trace-id")),
            "x-openclaw-trace-id must pass through",
        );
    }

    #[test]
    fn node_forward_still_drops_sensitive_headers() {
        // Guard against the prefix rule accidentally widening the gate for
        // non-OpenClaw sensitive headers.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer leaked".parse().unwrap());
        headers.insert("cookie", "session=leaked".parse().unwrap());
        headers.insert("x-nyxid-internal", "should-not-leak".parse().unwrap());

        let forwarded = node_forward_headers(&headers);
        assert!(
            forwarded.is_empty(),
            "sensitive headers must be dropped, got {forwarded:?}"
        );
    }

    #[test]
    fn ws_forward_preserves_openclaw_prefixed_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-openclaw-scopes", "operator.read".parse().unwrap());
        headers.insert("origin", "https://example.com".parse().unwrap());

        let forwarded = ws_forward_headers(&headers);
        assert!(
            forwarded
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("x-openclaw-scopes")),
            "x-openclaw-scopes must pass through on WS handshakes too",
        );
        assert!(
            forwarded
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("origin")),
            "origin should still be forwarded on WS",
        );
    }

    #[test]
    fn extract_via_service_returns_none_when_absent() {
        let request = Request::builder()
            .uri("/api/v1/proxy/s/openai/v1/chat/completions")
            .body(Body::empty())
            .unwrap();
        assert!(super::extract_via_service(&request).is_none());
    }

    #[test]
    fn extract_via_service_returns_value_when_present() {
        let request = Request::builder()
            .uri("/api/v1/proxy/s/openai/v1/chat/completions?_nyxid_via=us-123&foo=bar")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            super::extract_via_service(&request).as_deref(),
            Some("us-123")
        );
    }

    #[test]
    fn strip_internal_query_params_removes_nyxid_via() {
        let result = super::strip_internal_query_params("_nyxid_via=us-123&foo=bar&baz=1");
        assert_eq!(result, "foo=bar&baz=1");
    }

    #[test]
    fn strip_internal_query_params_preserves_all_when_no_internal() {
        let result = super::strip_internal_query_params("foo=bar&baz=1");
        assert_eq!(result, "foo=bar&baz=1");
    }

    #[test]
    fn strip_internal_query_params_returns_empty_when_only_internal() {
        let result = super::strip_internal_query_params("_nyxid_via=us-123");
        assert_eq!(result, "");
    }

    #[test]
    fn append_query_param_adds_to_clean_url() {
        let result = super::append_query_param("https://api.example.com/v1", "key", "value");
        assert_eq!(result, "https://api.example.com/v1?key=value");
    }

    #[test]
    fn append_query_param_appends_to_existing_query() {
        let result = super::append_query_param("https://api.example.com/v1?a=1", "key", "val ue");
        assert_eq!(result, "https://api.example.com/v1?a=1&key=val%20ue");
    }

    #[test]
    fn proxy_error_telemetry_fields_maps_common_errors() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;

        assert_eq!(
            proxy_error_telemetry_fields(&AppError::BadRequest("x".into())),
            (400, 1000)
        );
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::Unauthorized("x".into())),
            (401, 1001)
        );
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::Forbidden("x".into())),
            (403, 1002)
        );
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::NotFound("x".into())),
            (404, 1003)
        );
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::RateLimited),
            (429, 1005)
        );
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::NodeProxyTimeout),
            (504, 8002)
        );
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::ApiKeyScopeForbidden("x".into())),
            (403, 9000)
        );
    }

    #[test]
    fn collect_forward_headers_empty_input() {
        let headers = axum::http::HeaderMap::new();
        let result = collect_forward_headers_with_prefixes(
            &headers,
            ALLOWED_FORWARD_HEADERS,
            ALLOWED_FORWARD_HEADER_PREFIXES,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn collect_forward_headers_forwards_aws_prefix() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-amz-target",
            "CostExplorer.GetCostAndUsage".parse().unwrap(),
        );
        headers.insert("content-type", "application/json".parse().unwrap());

        let result = collect_forward_headers_with_prefixes(
            &headers,
            ALLOWED_FORWARD_HEADERS,
            ALLOWED_FORWARD_HEADER_PREFIXES,
        );
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|(n, _)| n == "x-amz-target"));
        assert!(result.iter().any(|(n, _)| n == "content-type"));
    }

    #[test]
    fn node_proxy_audit_event_data_includes_node_and_connection() {
        let data = super::node_proxy_audit_event_data(
            "svc-1",
            "POST",
            "/v1/chat",
            200,
            "node-1",
            "owner-1",
            "actor-1",
            Some("conn-1"),
        );
        assert_eq!(data["routed_via"], "node");
        assert_eq!(data["node_id"], "node-1");
        assert_eq!(data["connection_id"], "conn-1");
        assert_eq!(data["owner_user_id"], "owner-1");
    }

    #[test]
    fn node_proxy_audit_event_data_omits_owner_when_same_as_actor() {
        let data = super::node_proxy_audit_event_data(
            "svc-1", "GET", "/models", 200, "node-1", "user-1", "user-1", None,
        );
        assert!(data.get("owner_user_id").is_none());
        assert!(data.get("connection_id").is_none());
    }

    // ── proxy_error_telemetry_fields additional coverage ────────────

    #[test]
    fn proxy_error_telemetry_fields_internal_error() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::Internal("x".into())),
            (500, 1006)
        );
    }

    #[test]
    fn proxy_error_telemetry_fields_validation_error() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::ValidationError("x".into())),
            (400, 1008)
        );
    }

    #[test]
    fn proxy_error_telemetry_fields_node_not_found() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::NodeNotFound("x".into())),
            (404, 8000)
        );
    }

    #[test]
    fn proxy_error_telemetry_fields_node_offline() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::NodeOffline("x".into())),
            (503, 8001)
        );
    }

    #[test]
    fn proxy_error_telemetry_fields_node_credential_missing() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::NodeCredentialMissing("x".into())),
            (502, 8004)
        );
    }

    #[test]
    fn proxy_error_telemetry_fields_ws_proxy_downstream() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::WsProxyDownstream("x".into())),
            (502, 8005)
        );
    }

    #[test]
    fn proxy_error_telemetry_fields_api_key_scope_inactive() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::ApiKeyScopeInactive),
            (403, 9001)
        );
    }

    #[test]
    fn proxy_error_telemetry_fields_api_key_scope_not_found() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::ApiKeyScopeNotFound("x".into())),
            (404, 9002)
        );
    }

    #[test]
    fn proxy_error_telemetry_fields_catchall_maps_to_500_0() {
        use super::proxy_error_telemetry_fields;
        use crate::errors::AppError;
        // Conflict is not explicitly handled by the proxy telemetry map
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::Conflict("x".into())),
            (500, 0)
        );
    }

    // ── is_codex_transport_path additional edge cases ───────────────

    #[test]
    fn codex_transport_trailing_and_leading_slashes() {
        assert!(is_codex_transport_path("/responses/"));
        assert!(is_codex_transport_path("///responses///"));
    }

    #[test]
    fn codex_transport_deeply_nested() {
        assert!(is_codex_transport_path("v1/v2/responses"));
        assert!(is_codex_transport_path("/a/b/c/chat/completions"));
    }

    #[test]
    fn codex_transport_false_for_partial_match() {
        assert!(!is_codex_transport_path("my-responses"));
        assert!(!is_codex_transport_path("chat/completions/extra"));
    }

    // ── is_chat_completions_proxy_path additional cases ─────────────

    #[test]
    fn chat_completions_path_with_trailing_slash() {
        assert!(is_chat_completions_proxy_path("chat/completions/"));
    }

    #[test]
    fn chat_completions_path_false_for_responses() {
        assert!(!is_chat_completions_proxy_path("v1/responses"));
    }

    // ── is_ws_upgrade_request tests ─────────────────────────────────

    #[test]
    fn ws_upgrade_request_requires_both_headers() {
        // Only connection: upgrade, no upgrade header
        let req = Request::builder()
            .uri("/test")
            .header("connection", "Upgrade")
            .body(Body::empty())
            .unwrap();
        assert!(!is_ws_upgrade_request(&req));

        // Only upgrade: websocket, no connection header
        let req = Request::builder()
            .uri("/test")
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();
        assert!(!is_ws_upgrade_request(&req));
    }

    #[test]
    fn ws_upgrade_request_case_insensitive() {
        let req = Request::builder()
            .uri("/test")
            .header("connection", "UPGRADE")
            .header("upgrade", "WEBSOCKET")
            .body(Body::empty())
            .unwrap();
        assert!(is_ws_upgrade_request(&req));
    }

    #[test]
    fn ws_upgrade_request_connection_header_with_multiple_values() {
        let req = Request::builder()
            .uri("/test")
            .header("connection", "keep-alive, Upgrade")
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();
        assert!(is_ws_upgrade_request(&req));
    }

    #[test]
    fn ws_upgrade_request_non_websocket_upgrade_is_false() {
        let req = Request::builder()
            .uri("/test")
            .header("connection", "Upgrade")
            .header("upgrade", "h2c")
            .body(Body::empty())
            .unwrap();
        assert!(!is_ws_upgrade_request(&req));
    }

    // ── should_enforce_runtime_approval additional cases ────────────

    #[test]
    fn should_enforce_runtime_approval_relay_enforced() {
        assert!(should_enforce_runtime_approval(true, &AuthMethod::Relay));
    }

    #[test]
    fn should_enforce_runtime_approval_delegated_enforced() {
        assert!(should_enforce_runtime_approval(
            true,
            &AuthMethod::Delegated
        ));
    }

    // ── extract_via_service additional edge cases ───────────────────

    #[test]
    fn extract_via_service_percent_encoded_value() {
        let req = Request::builder()
            .uri("/test?_nyxid_via=svc%20with%20spaces")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            super::extract_via_service(&req).as_deref(),
            Some("svc with spaces")
        );
    }

    #[test]
    fn extract_via_service_empty_value() {
        let req = Request::builder()
            .uri("/test?_nyxid_via=")
            .body(Body::empty())
            .unwrap();
        assert_eq!(super::extract_via_service(&req).as_deref(), Some(""));
    }

    // ── strip_internal_query_params additional edge cases ───────────

    #[test]
    fn strip_internal_query_params_multiple_internal() {
        // If we had two internal params, both should be stripped
        let result = super::strip_internal_query_params("_nyxid_via=us-123");
        assert_eq!(result, "");
    }

    #[test]
    fn strip_internal_query_params_empty_input() {
        let result = super::strip_internal_query_params("");
        // Empty string split on & gives one empty part, which isn't "_nyxid_via"
        assert_eq!(result, "");
    }

    // ── append_query_param additional edge cases ────────────────────

    #[test]
    fn append_query_param_encodes_special_chars_in_name() {
        let result = super::append_query_param("https://example.com", "key name", "val");
        assert_eq!(result, "https://example.com?key%20name=val");
    }

    // ── collect_forward_headers_with_prefixes additional cases ──────

    #[test]
    fn collect_forward_headers_filters_unauthorized_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        headers.insert("x-custom-internal", "value".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());

        let result = collect_forward_headers_with_prefixes(
            &headers,
            ALLOWED_FORWARD_HEADERS,
            ALLOWED_FORWARD_HEADER_PREFIXES,
        );
        // authorization and x-custom-internal are not in the allowlist
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "content-type");
    }

    #[test]
    fn collect_forward_headers_openclaw_prefix() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-openclaw-scopes", "openai:*".parse().unwrap());
        headers.insert("x-openclaw-model", "gpt-4".parse().unwrap());

        let result = collect_forward_headers_with_prefixes(
            &headers,
            ALLOWED_FORWARD_HEADERS,
            ALLOWED_FORWARD_HEADER_PREFIXES,
        );
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn validate_range_header_three_ranges_ok() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("range", "bytes=0-1,2-3,4-5".parse().unwrap());
        assert!(validate_range_header(&headers).is_ok());
    }

    #[test]
    fn validate_range_header_exactly_four_ranges_ok() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("range", "bytes=0-1,2-3,4-5,6-7".parse().unwrap());
        assert!(validate_range_header(&headers).is_ok());
    }

    // ---- add_owner_user_id_if_shared tests ----

    #[test]
    fn add_owner_user_id_if_shared_adds_when_different() {
        let mut value = serde_json::json!({ "service_id": "svc-1" });
        super::add_owner_user_id_if_shared(&mut value, "owner-user", "actor-user");
        assert_eq!(value["owner_user_id"], "owner-user");
    }

    #[test]
    fn add_owner_user_id_if_shared_skips_when_same() {
        let mut value = serde_json::json!({ "service_id": "svc-1" });
        super::add_owner_user_id_if_shared(&mut value, "same-user", "same-user");
        assert!(value.get("owner_user_id").is_none());
    }

    #[test]
    fn add_owner_user_id_if_shared_on_non_object_is_noop() {
        let mut value = serde_json::json!("plain string");
        super::add_owner_user_id_if_shared(&mut value, "owner", "actor");
        // Non-object value should not panic and not be modified
        assert!(value.is_string());
    }

    // ---- axum_msg_to_tungstenite conversion tests ----

    #[test]
    fn axum_text_to_tungstenite_text() {
        let msg = axum::extract::ws::Message::Text("hello".into());
        let converted = super::axum_msg_to_tungstenite(msg);
        match converted {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert_eq!(t.to_string(), "hello");
            }
            _ => panic!("expected Text message"),
        }
    }

    #[test]
    fn axum_binary_to_tungstenite_binary() {
        let data = bytes::Bytes::from(vec![1u8, 2, 3]);
        let msg = axum::extract::ws::Message::Binary(data);
        let converted = super::axum_msg_to_tungstenite(msg);
        match converted {
            tokio_tungstenite::tungstenite::Message::Binary(b) => {
                assert_eq!(b.as_ref(), &[1, 2, 3]);
            }
            _ => panic!("expected Binary message"),
        }
    }

    #[test]
    fn axum_ping_to_tungstenite_ping() {
        let data = bytes::Bytes::from(vec![42u8]);
        let msg = axum::extract::ws::Message::Ping(data);
        let converted = super::axum_msg_to_tungstenite(msg);
        assert!(matches!(
            converted,
            tokio_tungstenite::tungstenite::Message::Ping(_)
        ));
    }

    #[test]
    fn axum_pong_to_tungstenite_pong() {
        let data = bytes::Bytes::from(vec![42u8]);
        let msg = axum::extract::ws::Message::Pong(data);
        let converted = super::axum_msg_to_tungstenite(msg);
        assert!(matches!(
            converted,
            tokio_tungstenite::tungstenite::Message::Pong(_)
        ));
    }

    #[test]
    fn axum_close_to_tungstenite_close() {
        let msg = axum::extract::ws::Message::Close(Some(axum::extract::ws::CloseFrame {
            code: 1000,
            reason: "normal".into(),
        }));
        let converted = super::axum_msg_to_tungstenite(msg);
        match converted {
            tokio_tungstenite::tungstenite::Message::Close(Some(f)) => {
                assert_eq!(u16::from(f.code), 1000);
                assert_eq!(f.reason.to_string(), "normal");
            }
            _ => panic!("expected Close message with frame"),
        }
    }

    #[test]
    fn axum_close_none_to_tungstenite_close_none() {
        let msg = axum::extract::ws::Message::Close(None);
        let converted = super::axum_msg_to_tungstenite(msg);
        assert!(matches!(
            converted,
            tokio_tungstenite::tungstenite::Message::Close(None)
        ));
    }

    // ---- tungstenite_msg_to_axum conversion tests ----

    #[test]
    fn tungstenite_text_to_axum_text() {
        let msg = tokio_tungstenite::tungstenite::Message::Text("world".into());
        let converted = super::tungstenite_msg_to_axum(msg);
        match converted {
            axum::extract::ws::Message::Text(t) => {
                assert_eq!(t.to_string(), "world");
            }
            _ => panic!("expected Text message"),
        }
    }

    #[test]
    fn tungstenite_binary_to_axum_binary() {
        let msg = tokio_tungstenite::tungstenite::Message::Binary(vec![4u8, 5, 6].into());
        let converted = super::tungstenite_msg_to_axum(msg);
        match converted {
            axum::extract::ws::Message::Binary(b) => {
                assert_eq!(b.as_ref(), &[4, 5, 6]);
            }
            _ => panic!("expected Binary message"),
        }
    }

    #[test]
    fn tungstenite_ping_to_axum_ping() {
        let msg = tokio_tungstenite::tungstenite::Message::Ping(vec![99u8].into());
        let converted = super::tungstenite_msg_to_axum(msg);
        assert!(matches!(converted, axum::extract::ws::Message::Ping(_)));
    }

    #[test]
    fn tungstenite_pong_to_axum_pong() {
        let msg = tokio_tungstenite::tungstenite::Message::Pong(vec![99u8].into());
        let converted = super::tungstenite_msg_to_axum(msg);
        assert!(matches!(converted, axum::extract::ws::Message::Pong(_)));
    }

    #[test]
    fn tungstenite_close_to_axum_close() {
        use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
        let msg = tokio_tungstenite::tungstenite::Message::Close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "goodbye".into(),
        }));
        let converted = super::tungstenite_msg_to_axum(msg);
        match converted {
            axum::extract::ws::Message::Close(Some(f)) => {
                assert_eq!(f.code, 1000);
                assert_eq!(f.reason.to_string(), "goodbye");
            }
            _ => panic!("expected Close message with frame"),
        }
    }

    #[test]
    fn tungstenite_close_none_to_axum_close_none() {
        let msg = tokio_tungstenite::tungstenite::Message::Close(None);
        let converted = super::tungstenite_msg_to_axum(msg);
        assert!(matches!(converted, axum::extract::ws::Message::Close(None)));
    }

    #[test]
    fn tungstenite_frame_to_axum_empty_binary() {
        // The raw Frame variant should map to empty Binary as the fallback.
        let raw = tokio_tungstenite::tungstenite::protocol::frame::Frame::pong(bytes::Bytes::new());
        let msg = tokio_tungstenite::tungstenite::Message::Frame(raw);
        let converted = super::tungstenite_msg_to_axum(msg);
        match converted {
            axum::extract::ws::Message::Binary(b) => {
                assert!(b.is_empty());
            }
            _ => panic!("expected Binary message for Frame fallback"),
        }
    }

    // ---- axum_msg_payload_for_injection tests ----

    #[test]
    fn axum_text_payload_extraction() {
        let msg = axum::extract::ws::Message::Text("payload".into());
        let result = super::axum_msg_payload_for_injection(&msg);
        assert!(result.is_some());
        let (kind, bytes) = result.unwrap();
        assert_eq!(kind, crate::models::ws_frame_injection::WsFrameKind::Text);
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), "payload");
    }

    #[test]
    fn axum_binary_payload_extraction() {
        let data = bytes::Bytes::from(vec![10u8, 20]);
        let msg = axum::extract::ws::Message::Binary(data);
        let result = super::axum_msg_payload_for_injection(&msg);
        assert!(result.is_some());
        let (kind, bytes) = result.unwrap();
        assert_eq!(kind, crate::models::ws_frame_injection::WsFrameKind::Binary);
        assert_eq!(bytes, vec![10, 20]);
    }

    #[test]
    fn axum_ping_payload_extraction_returns_none() {
        let msg = axum::extract::ws::Message::Ping(bytes::Bytes::new());
        assert!(super::axum_msg_payload_for_injection(&msg).is_none());
    }

    #[test]
    fn axum_pong_payload_extraction_returns_none() {
        let msg = axum::extract::ws::Message::Pong(bytes::Bytes::new());
        assert!(super::axum_msg_payload_for_injection(&msg).is_none());
    }

    #[test]
    fn axum_close_payload_extraction_returns_none() {
        let msg = axum::extract::ws::Message::Close(None);
        assert!(super::axum_msg_payload_for_injection(&msg).is_none());
    }

    // ---- tungstenite_msg_payload_for_injection tests ----

    #[test]
    fn tungstenite_text_payload_extraction() {
        let msg = tokio_tungstenite::tungstenite::Message::Text("tung-payload".into());
        let result = super::tungstenite_msg_payload_for_injection(&msg);
        assert!(result.is_some());
        let (kind, bytes) = result.unwrap();
        assert_eq!(kind, crate::models::ws_frame_injection::WsFrameKind::Text);
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), "tung-payload");
    }

    #[test]
    fn tungstenite_binary_payload_extraction() {
        let msg = tokio_tungstenite::tungstenite::Message::Binary(vec![30u8, 40].into());
        let result = super::tungstenite_msg_payload_for_injection(&msg);
        assert!(result.is_some());
        let (kind, bytes) = result.unwrap();
        assert_eq!(kind, crate::models::ws_frame_injection::WsFrameKind::Binary);
        assert_eq!(bytes, vec![30, 40]);
    }

    #[test]
    fn tungstenite_ping_payload_extraction_returns_none() {
        let msg = tokio_tungstenite::tungstenite::Message::Ping(vec![].into());
        assert!(super::tungstenite_msg_payload_for_injection(&msg).is_none());
    }

    #[test]
    fn tungstenite_close_payload_extraction_returns_none() {
        let msg = tokio_tungstenite::tungstenite::Message::Close(None);
        assert!(super::tungstenite_msg_payload_for_injection(&msg).is_none());
    }

    // ---- injection_frame_to_tungstenite tests ----

    #[test]
    fn injection_text_frame_to_tungstenite() {
        use crate::services::ws_frame_injector::WsFrame;
        let frame = WsFrame {
            kind: crate::models::ws_frame_injection::WsFrameKind::Text,
            payload: b"injected text".to_vec(),
        };
        let msg = super::injection_frame_to_tungstenite(frame);
        match msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert_eq!(t.to_string(), "injected text");
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn injection_binary_frame_to_tungstenite() {
        use crate::services::ws_frame_injector::WsFrame;
        let frame = WsFrame {
            kind: crate::models::ws_frame_injection::WsFrameKind::Binary,
            payload: vec![0xDE, 0xAD],
        };
        let msg = super::injection_frame_to_tungstenite(frame);
        match msg {
            tokio_tungstenite::tungstenite::Message::Binary(b) => {
                assert_eq!(b.as_ref(), &[0xDE, 0xAD]);
            }
            _ => panic!("expected Binary"),
        }
    }

    #[test]
    fn injection_text_frame_invalid_utf8_defaults_to_empty() {
        use crate::services::ws_frame_injector::WsFrame;
        let frame = WsFrame {
            kind: crate::models::ws_frame_injection::WsFrameKind::Text,
            payload: vec![0xFF, 0xFE],
        };
        let msg = super::injection_frame_to_tungstenite(frame);
        match msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                assert!(t.is_empty(), "invalid UTF-8 should produce empty text");
            }
            _ => panic!("expected Text"),
        }
    }

    // ---- injection_frame_to_axum tests ----

    #[test]
    fn injection_text_frame_to_axum() {
        use crate::services::ws_frame_injector::WsFrame;
        let frame = WsFrame {
            kind: crate::models::ws_frame_injection::WsFrameKind::Text,
            payload: b"axum text".to_vec(),
        };
        let msg = super::injection_frame_to_axum(frame);
        match msg {
            axum::extract::ws::Message::Text(t) => {
                assert_eq!(t.to_string(), "axum text");
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn injection_binary_frame_to_axum() {
        use crate::services::ws_frame_injector::WsFrame;
        let frame = WsFrame {
            kind: crate::models::ws_frame_injection::WsFrameKind::Binary,
            payload: vec![0xBE, 0xEF],
        };
        let msg = super::injection_frame_to_axum(frame);
        match msg {
            axum::extract::ws::Message::Binary(b) => {
                assert_eq!(b.as_ref(), &[0xBE, 0xEF]);
            }
            _ => panic!("expected Binary"),
        }
    }

    #[test]
    fn injection_text_frame_to_axum_invalid_utf8_defaults_to_empty() {
        use crate::services::ws_frame_injector::WsFrame;
        let frame = WsFrame {
            kind: crate::models::ws_frame_injection::WsFrameKind::Text,
            payload: vec![0xFF, 0xFE],
        };
        let msg = super::injection_frame_to_axum(frame);
        match msg {
            axum::extract::ws::Message::Text(t) => {
                assert!(t.is_empty(), "invalid UTF-8 should produce empty text");
            }
            _ => panic!("expected Text"),
        }
    }

    // ---- node_proxy_audit_event_data additional coverage ----

    #[test]
    fn node_proxy_audit_event_data_includes_all_fields() {
        let data = super::node_proxy_audit_event_data(
            "svc-2",
            "PUT",
            "/v1/update",
            201,
            "node-2",
            "owner-2",
            "actor-2",
            Some("conn-2"),
        );
        assert_eq!(data["service_id"], "svc-2");
        assert_eq!(data["method"], "PUT");
        assert_eq!(data["path"], "/v1/update");
        assert_eq!(data["response_status"], 201);
        assert_eq!(data["routed_via"], "node");
        assert_eq!(data["node_id"], "node-2");
        assert_eq!(data["connection_id"], "conn-2");
        assert_eq!(data["owner_user_id"], "owner-2");
    }

    #[test]
    fn node_proxy_audit_event_data_no_connection_no_owner() {
        let data = super::node_proxy_audit_event_data(
            "svc-3", "DELETE", "/items/1", 204, "node-3", "user-3", "user-3", None,
        );
        assert_eq!(data["method"], "DELETE");
        assert_eq!(data["response_status"], 204);
        assert!(data.get("connection_id").is_none());
        assert!(data.get("owner_user_id").is_none());
    }

    // ---- ALLOWED_RESPONSE_HEADERS coverage ----

    #[test]
    fn allowed_response_headers_does_not_include_cors() {
        // Verify the CORS exclusion documented in the constant comment.
        for header in super::ALLOWED_RESPONSE_HEADERS {
            assert!(
                !header.starts_with("access-control"),
                "CORS headers must not be in ALLOWED_RESPONSE_HEADERS: {header}"
            );
        }
    }

    #[test]
    fn allowed_response_headers_includes_content_type() {
        assert!(super::ALLOWED_RESPONSE_HEADERS.contains(&"content-type"));
    }

    #[test]
    fn allowed_response_headers_includes_range_support() {
        assert!(super::ALLOWED_RESPONSE_HEADERS.contains(&"accept-ranges"));
        assert!(super::ALLOWED_RESPONSE_HEADERS.contains(&"content-range"));
    }

    // ---- STREAMING_CONTENT_TYPES coverage ----

    #[test]
    fn streaming_content_types_includes_sse() {
        assert!(super::STREAMING_CONTENT_TYPES.contains(&"text/event-stream"));
    }

    #[test]
    fn streaming_content_types_includes_media() {
        assert!(super::STREAMING_CONTENT_TYPES.contains(&"video/"));
        assert!(super::STREAMING_CONTENT_TYPES.contains(&"audio/"));
        assert!(super::STREAMING_CONTENT_TYPES.contains(&"image/"));
    }

    #[test]
    fn streaming_content_types_includes_octet_stream() {
        assert!(super::STREAMING_CONTENT_TYPES.contains(&"application/octet-stream"));
    }

    #[test]
    fn streaming_content_types_includes_pdf() {
        assert!(super::STREAMING_CONTENT_TYPES.contains(&"application/pdf"));
    }

    // ---- ALLOWED_FORWARD_HEADER_PREFIXES coverage ----

    #[test]
    fn forward_header_prefixes_include_aws() {
        assert!(super::ALLOWED_FORWARD_HEADER_PREFIXES.contains(&"x-amz-"));
    }

    #[test]
    fn forward_header_prefixes_include_gcp() {
        assert!(super::ALLOWED_FORWARD_HEADER_PREFIXES.contains(&"x-goog-"));
    }

    #[test]
    fn forward_header_prefixes_include_openclaw() {
        assert!(super::ALLOWED_FORWARD_HEADER_PREFIXES.contains(&"x-openclaw-"));
    }

    // ---- WS forward headers tests ----

    #[test]
    fn ws_forward_includes_origin() {
        assert!(super::ALLOWED_WS_FORWARD_HEADERS.contains(&"origin"));
    }

    #[test]
    fn ws_forward_includes_subprotocol() {
        assert!(super::ALLOWED_WS_FORWARD_HEADERS.contains(&"sec-websocket-protocol"));
    }

    #[test]
    fn ws_forward_does_not_include_sensitive() {
        assert!(!super::ALLOWED_WS_FORWARD_HEADERS.contains(&"authorization"));
        assert!(!super::ALLOWED_WS_FORWARD_HEADERS.contains(&"cookie"));
    }

    // ---- collect_forward_headers: GCP prefix ----

    #[test]
    fn collect_forward_headers_forwards_gcp_prefix() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-goog-user-project", "my-project".parse().unwrap());
        headers.insert("x-goog-request-reason", "cost-report".parse().unwrap());

        let result = collect_forward_headers_with_prefixes(
            &headers,
            ALLOWED_FORWARD_HEADERS,
            ALLOWED_FORWARD_HEADER_PREFIXES,
        );
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|(n, _)| n == "x-goog-user-project"));
        assert!(result.iter().any(|(n, _)| n == "x-goog-request-reason"));
    }

    // ---- proxy_error_telemetry_fields: approval variants ----

    #[test]
    fn proxy_error_telemetry_fields_approval_required() {
        let (status, code) =
            super::proxy_error_telemetry_fields(&crate::errors::AppError::ApprovalRequired {
                request_id: "req-1".into(),
            });
        assert_eq!(status, 403);
        assert_eq!(code, 7000);
    }

    #[test]
    fn proxy_error_telemetry_fields_approval_failed() {
        let (status, code) =
            super::proxy_error_telemetry_fields(&crate::errors::AppError::ApprovalFailed {
                request_id: "req-1".into(),
                approve_url: "https://example.com/approvals".into(),
                reason: "denied".into(),
            });
        assert_eq!(status, 403);
        assert_eq!(code, 7001);
    }

    #[test]
    fn proxy_error_telemetry_fields_org_approval_no_admin() {
        let (status, code) = super::proxy_error_telemetry_fields(
            &crate::errors::AppError::OrgApprovalNoAdmin("x".into()),
        );
        assert_eq!(status, 503);
        assert_eq!(code, 8106);
    }

    // ---- validate_requested_proxy_path safe paths ----

    #[test]
    fn validate_proxy_path_allows_simple_paths() {
        validate_requested_proxy_path("/v1/chat/completions").expect("simple path should be ok");
        validate_requested_proxy_path("/models").expect("single segment should be ok");
        validate_requested_proxy_path("/v1/files/abc-123").expect("alphanumeric segments ok");
    }

    #[test]
    fn validate_proxy_path_allows_empty_path() {
        validate_requested_proxy_path("/").expect("root path should be ok");
        validate_requested_proxy_path("").expect("empty path should be ok");
    }

    #[test]
    fn validate_proxy_path_rejects_null_bytes() {
        assert!(validate_requested_proxy_path("/path\0evil").is_err());
    }

    // ---- WsPassthroughGuard ----

    #[test]
    fn ws_passthrough_guard_decrements_on_drop() {
        let counter = Arc::new(AtomicUsize::new(5));
        let guard = WsPassthroughGuard::new(counter.clone());
        assert_eq!(counter.load(Ordering::Relaxed), 5);
        drop(guard);
        assert_eq!(counter.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn ws_passthrough_guard_wraps_on_underflow() {
        // AtomicUsize::fetch_sub wraps on underflow. The guard uses
        // Relaxed ordering and does not saturate -- this is acceptable
        // because in production the counter is always incremented
        // before the guard is created. This test documents the raw
        // wrapping behavior.
        let counter = Arc::new(AtomicUsize::new(0));
        let guard = WsPassthroughGuard::new(counter.clone());
        drop(guard);
        assert_eq!(counter.load(Ordering::Relaxed), usize::MAX);
    }
}

#[cfg(test)]
mod proxy_resolution_integration_tests {
    use super::{proxy_request_by_slug_inner, proxy_request_inner};
    use crate::AppState;
    use crate::crypto::token::hash_token;
    use crate::errors::AppError;
    use crate::models::approval_request::{ApprovalRequest, COLLECTION_NAME as APPROVAL_REQUESTS};
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::notification_channel::{
        COLLECTION_NAME as NOTIFICATION_CHANNELS, NotificationChannel,
    };
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::service_approval_config::{
        ApprovalMode, COLLECTION_NAME as SERVICE_APPROVAL_CONFIGS, ServiceApprovalConfig,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::mw::auth::{AuthMethod, AuthUser};
    use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_membership, test_user, test_user_endpoint,
        test_user_service,
    };
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::{Path, State},
        http::{Method, Request, StatusCode, Uri},
        response::{IntoResponse, Response},
        routing::get,
    };
    use chrono::Utc;
    use mongodb::bson::doc;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    async fn downstream_ok(uri: Uri) -> (StatusCode, String) {
        (StatusCode::OK, format!("ok:{}", uri.path()))
    }

    async fn downstream_auth_header(headers: axum::http::HeaderMap) -> (StatusCode, String) {
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (StatusCode::OK, format!("auth:{auth}"))
    }

    async fn start_downstream() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/{*path}", get(downstream_ok));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind downstream test listener");
        let addr = listener.local_addr().expect("downstream listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve downstream test app");
        });
        (format!("http://{addr}"), server)
    }

    async fn start_auth_downstream() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/{*path}", get(downstream_auth_header));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind downstream auth test listener");
        let addr = listener.local_addr().expect("downstream listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve downstream auth test app");
        });
        (format!("http://{addr}"), server)
    }

    fn proxy_request(uri: &str) -> Request<Body> {
        Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .expect("build proxy request")
    }

    async fn ws_proxy_test_route(
        State((state, auth)): State<(AppState, AuthUser)>,
        Path((slug, path)): Path<(String, String)>,
        request: Request<Body>,
    ) -> Response {
        let mut resolved_slug = String::new();
        match proxy_request_by_slug_inner(&state, &auth, &slug, &path, request, &mut resolved_slug)
            .await
        {
            Ok(response) if resolved_slug == slug => response,
            Ok(_) => AppError::Internal("proxy resolved unexpected service slug".to_string())
                .into_response(),
            Err(error) => error.into_response(),
        }
    }

    async fn assert_ws_proxy_upgrade(state: AppState, auth: AuthUser, path: &str) {
        let app = Router::new()
            .route("/proxy/s/{slug}/{*path}", get(ws_proxy_test_route))
            .with_state((state, auth));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ws proxy test listener");
        let addr = listener.local_addr().expect("ws proxy listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve ws proxy test app");
        });

        let url = format!("ws://{addr}{path}");
        let (_socket, response) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("websocket proxy should upgrade");
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        server.abort();
    }

    fn service_account_auth(service_account_id: &str, owner_user_id: &str) -> AuthUser {
        AuthUser {
            user_id: Uuid::parse_str(service_account_id).expect("valid service account id"),
            session_id: None,
            scope: "proxy".to_string(),
            acting_client_id: None,
            approval_owner_user_id: Some(owner_user_id.to_string()),
            auth_method: AuthMethod::ServiceAccount,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            api_key_id: None,
            api_key_name: None,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        }
    }

    fn access_token_auth(user_id: &str) -> AuthUser {
        AuthUser {
            user_id: Uuid::parse_str(user_id).expect("valid user id"),
            session_id: None,
            scope: "proxy".to_string(),
            acting_client_id: None,
            approval_owner_user_id: None,
            auth_method: AuthMethod::AccessToken,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            api_key_id: None,
            api_key_name: None,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        }
    }

    fn notification_channel(user_id: &str, timeout_secs: u32) -> NotificationChannel {
        let now = Utc::now();
        NotificationChannel {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            telegram_chat_id: None,
            telegram_username: None,
            telegram_enabled: false,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: timeout_secs,
            grant_expiry_days: 30,
            approval_required: false,
            push_enabled: false,
            push_devices: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    fn approval_config(owner_user_id: &str, service_id: &str) -> ServiceApprovalConfig {
        let now = Utc::now();
        ServiceApprovalConfig {
            id: Uuid::new_v4().to_string(),
            user_id: owner_user_id.to_string(),
            service_id: service_id.to_string(),
            service_name: "Org Proxy Target".to_string(),
            approval_required: true,
            approval_mode: ApprovalMode::PerRequest,
            rules: vec![],
            default_effect: None,
            created_at: now,
            updated_at: now,
        }
    }

    async fn seed_org_actor(db: &mongodb::Database, org_id: &str, actor_id: &str, role: OrgRole) {
        db.collection::<crate::models::user::User>(USERS)
            .insert_many([
                test_user(org_id, UserType::Org),
                test_user(actor_id, UserType::Person),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(org_id, actor_id, role, None))
            .await
            .unwrap();
    }

    async fn insert_user_service(
        db: &mongodb::Database,
        owner_user_id: &str,
        slug: &str,
        base_url: &str,
        catalog_service_id: Option<&str>,
    ) -> UserService {
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            owner_user_id,
            slug,
            base_url,
            None,
            catalog_service_id,
        );
        let service = test_user_service(
            &Uuid::new_v4().to_string(),
            owner_user_id,
            slug,
            &endpoint.id,
            catalog_service_id,
            None,
        );

        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .expect("insert user endpoint");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service.clone())
            .await
            .expect("insert user service");

        service
    }

    async fn insert_gcp_service_account_key(
        state: &AppState,
        owner_user_id: &str,
        access_token: &[u8],
    ) -> String {
        let credential_key = state
            .encryption_keys
            .encrypt(br#"{"type":"service_account","private_key":"redacted"}"#)
            .await
            .expect("encrypt service-account JSON");
        let access_token = state
            .encryption_keys
            .encrypt(access_token)
            .await
            .expect("encrypt cached GCP access token");
        let api_key_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        state
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(UserApiKey {
                id: api_key_id.clone(),
                user_id: owner_user_id.to_string(),
                label: "Org GCP SA".to_string(),
                credential_type: "gcp_service_account".to_string(),
                credential_encrypted: Some(credential_key),
                access_token_encrypted: Some(access_token),
                refresh_token_encrypted: None,
                token_scopes: Some("https://www.googleapis.com/auth/cloud-platform".to_string()),
                expires_at: Some(now + chrono::Duration::hours(1)),
                provider_config_id: None,
                connection_id: None,
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                status: "active".to_string(),
                last_used_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert org GCP SA key");
        api_key_id
    }

    async fn insert_online_node(state: &AppState, owner_user_id: &str, name: &str) -> Node {
        let raw_signing_secret = "11".repeat(32);
        let signing_secret_encrypted = state
            .encryption_keys
            .encrypt(raw_signing_secret.as_bytes())
            .await
            .expect("encrypt node signing secret");
        let now = Utc::now();
        let node = Node {
            id: Uuid::new_v4().to_string(),
            user_id: owner_user_id.to_string(),
            name: name.to_string(),
            status: NodeStatus::Online,
            auth_token_hash: hash_token("test-node-auth-token"),
            signing_secret_encrypted: Some(signing_secret_encrypted),
            signing_secret_hash: hash_token(&raw_signing_secret),
            last_heartbeat_at: Some(now),
            connected_at: Some(now),
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        };

        state
            .db
            .collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert online node");

        node
    }

    async fn wait_for_node_audit_event(
        db: &mongodb::Database,
        node_id: &str,
        event_type: &str,
    ) -> Option<AuditLog> {
        for _ in 0..100 {
            let found = db
                .collection::<AuditLog>(AUDIT_LOG)
                .find_one(doc! {
                    "event_type": event_type,
                    "event_data.routed_via": "node",
                    "event_data.node_id": node_id,
                })
                .await
                .expect("query audit log");
            if found.is_some() {
                return found;
            }

            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        None
    }

    async fn wait_for_node_proxy_audit(db: &mongodb::Database, node_id: &str) -> Option<AuditLog> {
        wait_for_node_audit_event(db, node_id, "proxy_request").await
    }

    async fn find_org_routing_audit(
        db: &mongodb::Database,
        org_id: &str,
        member_id: &str,
        user_service_id: &str,
    ) -> Option<AuditLog> {
        for _ in 0..100 {
            let found = db
                .collection::<AuditLog>(AUDIT_LOG)
                .find_one(doc! {
                    "event_type": "proxy_routed_via_org",
                    "event_data.routed_via": "org",
                    "event_data.org_user_id": org_id,
                    "event_data.member_user_id": member_id,
                    "event_data.user_service_id": user_service_id,
                })
                .await
                .expect("query org routing audit");
            if found.is_some() {
                return found;
            }
            tokio::task::yield_now().await;
        }
        None
    }

    fn spawn_ws_open_responder(
        state: &AppState,
        node_id: &str,
        mut rx: mpsc::Receiver<NodeOutboundMessage>,
        expected_slug: String,
    ) -> tokio::task::JoinHandle<()> {
        let manager = state.node_ws_manager.clone();
        let node_id = node_id.to_string();
        tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(msg)) = rx.recv().await else {
                panic!("expected outbound node ws_proxy_open request");
            };
            let parsed: serde_json::Value = serde_json::from_str(&msg).expect("valid ws request");
            assert_eq!(parsed["type"].as_str(), Some("ws_proxy_open"));
            assert_eq!(
                parsed["service_slug"].as_str(),
                Some(expected_slug.as_str())
            );
            let session_id = parsed["session_id"].as_str().expect("session id");
            assert!(
                manager.deliver_ws_proxy_opened(&node_id, session_id, None),
                "ws proxy open ack should be delivered"
            );
        })
    }

    async fn find_approval_request(
        db: &mongodb::Database,
        owner_user_id: &str,
        service_id: &str,
    ) -> ApprovalRequest {
        db.collection::<ApprovalRequest>(APPROVAL_REQUESTS)
            .find_one(doc! {
                "user_id": owner_user_id,
                "service_id": service_id,
            })
            .await
            .expect("query approval request")
            .expect("approval request should exist")
    }

    #[tokio::test]
    async fn org_service_account_resolves_org_owned_slug_service() {
        let Some(db) = connect_test_database("proxy_org_sa_slug").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let org_id = Uuid::new_v4().to_string();
        let sa_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .unwrap();
        let service = insert_user_service(&db, &org_id, "org-sa-target", &base_url, None).await;

        let state = test_app_state(db.clone());
        let mut resolved_slug = String::new();
        let response = proxy_request_by_slug_inner(
            &state,
            &service_account_auth(&sa_id, &org_id),
            &service.slug,
            "status",
            proxy_request("/proxy/s/org-sa-target/status"),
            &mut resolved_slug,
        )
        .await
        .expect("org service account should resolve owner-owned service");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(resolved_slug, service.slug);
        server.abort();
    }

    #[tokio::test]
    async fn org_member_proxy_through_org_node_audits_owner_user_id() {
        let Some(db) = connect_test_database("proxy_org_node").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        seed_org_actor(&db, &org_id, &member_id, OrgRole::Member).await;

        let state = test_app_state(db.clone());
        let node = insert_online_node(&state, &org_id, "org-node").await;

        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &org_id,
            "Org Node Target",
            "https://node-target.example.test",
            None,
            None,
        );
        let service = test_user_service(
            &Uuid::new_v4().to_string(),
            &org_id,
            "org-node-target",
            &endpoint.id,
            None,
            Some(&node.id),
        );
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .expect("insert endpoint");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service.clone())
            .await
            .expect("insert user service");

        let (tx, mut rx) = mpsc::channel(256);
        state.node_ws_manager.register_connection(&node.id, tx);
        let manager = state.node_ws_manager.clone();
        let node_id = node.id.clone();
        let expected_slug = service.slug.clone();
        let responder = tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(msg)) = rx.recv().await else {
                panic!("expected outbound node proxy request");
            };
            let parsed: serde_json::Value = serde_json::from_str(&msg).expect("valid node request");
            assert_eq!(
                parsed["service_slug"].as_str(),
                Some(expected_slug.as_str())
            );
            let request_id = parsed["request_id"]
                .as_str()
                .expect("request id")
                .to_string();
            manager.deliver_proxy_response(
                &node_id,
                NodeProxyResponse {
                    request_id,
                    status: 200,
                    headers: vec![("content-type".to_string(), "text/plain".to_string())],
                    body: b"proxied-through-node".to_vec(),
                },
            );
        });

        let mut resolved_slug = String::new();
        let response = proxy_request_by_slug_inner(
            &state,
            &access_token_auth(&member_id),
            &service.slug,
            "status",
            proxy_request("/proxy/s/org-node-target/status"),
            &mut resolved_slug,
        )
        .await
        .expect("org member should proxy through org node");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(resolved_slug, service.slug);
        responder.await.expect("node responder task");

        let audit = wait_for_node_proxy_audit(&db, &node.id)
            .await
            .expect("node-routed proxy audit should be written");
        let data = audit.event_data.expect("event data");
        assert_eq!(
            data.get("routed_via").and_then(|v| v.as_str()),
            Some("node")
        );
        assert_eq!(
            data.get("owner_user_id").and_then(|v| v.as_str()),
            Some(org_id.as_str())
        );
    }

    #[tokio::test]
    async fn org_member_proxy_uses_bound_org_gcp_service_account_credential() {
        let Some(db) = connect_test_database("proxy_org_member_gcp_sa").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_auth_downstream().await;
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        seed_org_actor(&db, &org_id, &member_id, OrgRole::Member).await;

        let state = test_app_state(db.clone());
        let api_key_id =
            insert_gcp_service_account_key(&state, &org_id, b"ya29.bound-org-sa").await;

        let mut service =
            insert_user_service(&db, &org_id, "org-gcp-billing", &base_url, None).await;
        service.api_key_id = Some(api_key_id);
        service.auth_method = "bearer".to_string();
        db.collection::<UserService>(USER_SERVICES)
            .replace_one(doc! { "_id": &service.id }, service.clone())
            .await
            .expect("bind org service to GCP SA key");

        let mut resolved_slug = String::new();
        let response = proxy_request_by_slug_inner(
            &state,
            &access_token_auth(&member_id),
            &service.slug,
            "v1/billingAccounts",
            proxy_request("/proxy/s/org-gcp-billing/v1/billingAccounts"),
            &mut resolved_slug,
        )
        .await
        .expect("org member should proxy through bound org GCP SA credential");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(resolved_slug, service.slug);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "auth:Bearer ya29.bound-org-sa"
        );

        let audit = find_org_routing_audit(&db, &org_id, &member_id, &service.id)
            .await
            .expect("org routing audit should be written");
        let data = audit.event_data.expect("event data");
        assert_eq!(data.get("routed_via").and_then(|v| v.as_str()), Some("org"));
        assert_eq!(
            data.get("org_user_id").and_then(|v| v.as_str()),
            Some(org_id.as_str())
        );
        assert_eq!(
            data.get("member_user_id").and_then(|v| v.as_str()),
            Some(member_id.as_str())
        );
        server.abort();
    }

    #[tokio::test]
    async fn org_member_ws_upgrade_through_org_node_audits_owner_user_id() {
        let Some(db) = connect_test_database("proxy_ws_org_node").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        seed_org_actor(&db, &org_id, &member_id, OrgRole::Member).await;

        let state = test_app_state(db.clone());
        let node = insert_online_node(&state, &org_id, "org-ws-node").await;
        let service = insert_user_service(
            &db,
            &org_id,
            "org-node-ws-target",
            "https://node-ws-target.example.test",
            None,
        )
        .await;
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service.id },
                doc! { "$set": { "node_id": &node.id } },
            )
            .await
            .expect("attach node to service");

        let (tx, rx) = mpsc::channel(256);
        state.node_ws_manager.register_connection(&node.id, tx);
        let responder = spawn_ws_open_responder(&state, &node.id, rx, service.slug.clone());

        assert_ws_proxy_upgrade(
            state.clone(),
            access_token_auth(&member_id),
            "/proxy/s/org-node-ws-target/socket",
        )
        .await;
        responder.await.expect("node ws responder task");

        let audit = wait_for_node_audit_event(&db, &node.id, "proxy_ws_upgrade")
            .await
            .expect("node-routed ws upgrade audit should be written");
        let data = audit.event_data.expect("event data");
        assert_eq!(
            data.get("routed_via").and_then(|v| v.as_str()),
            Some("node")
        );
        assert_eq!(
            data.get("owner_user_id").and_then(|v| v.as_str()),
            Some(org_id.as_str())
        );
    }

    #[tokio::test]
    async fn personal_ws_upgrade_through_personal_node_omits_owner_user_id() {
        let Some(db) = connect_test_database("proxy_ws_personal").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let owner_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .unwrap();

        let state = test_app_state(db.clone());
        let node = insert_online_node(&state, &owner_id, "personal-ws-node").await;
        let service = insert_user_service(
            &db,
            &owner_id,
            "personal-node-ws-target",
            "https://node-ws-target.example.test",
            None,
        )
        .await;
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &service.id },
                doc! { "$set": { "node_id": &node.id } },
            )
            .await
            .expect("attach node to service");

        let (tx, rx) = mpsc::channel(256);
        state.node_ws_manager.register_connection(&node.id, tx);
        let responder = spawn_ws_open_responder(&state, &node.id, rx, service.slug.clone());

        assert_ws_proxy_upgrade(
            state.clone(),
            access_token_auth(&owner_id),
            "/proxy/s/personal-node-ws-target/socket",
        )
        .await;
        responder.await.expect("node ws responder task");

        let audit = wait_for_node_audit_event(&db, &node.id, "proxy_ws_upgrade")
            .await
            .expect("node-routed ws upgrade audit should be written");
        let data = audit.event_data.expect("event data");
        assert_eq!(
            data.get("routed_via").and_then(|v| v.as_str()),
            Some("node")
        );
        assert!(data.get("owner_user_id").is_none());
    }

    #[tokio::test]
    async fn org_service_account_org_policy_creates_org_approval_request() {
        let Some(db) = connect_test_database("proxy_org_sa_approval").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let org_id = Uuid::new_v4().to_string();
        let admin_id = Uuid::new_v4().to_string();
        let sa_id = Uuid::new_v4().to_string();
        seed_org_actor(&db, &org_id, &admin_id, OrgRole::Admin).await;
        db.collection::<NotificationChannel>(NOTIFICATION_CHANNELS)
            .insert_one(notification_channel(&admin_id, 0))
            .await
            .unwrap();
        let service = insert_user_service(&db, &org_id, "org-sa-approval", &base_url, None).await;
        db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
            .insert_one(approval_config(&org_id, &service.id))
            .await
            .unwrap();

        let state = test_app_state(db.clone());
        let mut resolved_slug = String::new();
        let err = proxy_request_by_slug_inner(
            &state,
            &service_account_auth(&sa_id, &org_id),
            &service.slug,
            "sensitive",
            proxy_request("/proxy/s/org-sa-approval/sensitive"),
            &mut resolved_slug,
        )
        .await
        .expect_err("approval should block until timeout in test");

        assert!(
            matches!(err, AppError::ApprovalFailed { .. }),
            "unexpected proxy error: {err}"
        );
        let approval = find_approval_request(&db, &org_id, &service.id).await;
        assert!(approval.from_org_policy);
        assert_eq!(approval.user_id, org_id);
        assert_eq!(approval.requester_type, "service_account");
        assert_eq!(approval.requester_id, sa_id);
        assert_eq!(approval.notify_user_ids, vec![admin_id]);
        server.abort();
    }

    #[tokio::test]
    async fn personal_service_account_resolves_owner_catalog_service_on_uuid_path() {
        let Some(db) = connect_test_database("proxy_personal_sa_uuid").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let owner_id = Uuid::new_v4().to_string();
        let sa_id = Uuid::new_v4().to_string();
        let catalog_service_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .unwrap();
        let service = insert_user_service(
            &db,
            &owner_id,
            "personal-sa-target",
            &base_url,
            Some(&catalog_service_id),
        )
        .await;

        let state = test_app_state(db.clone());
        let mut resolved_slug = String::new();
        let response = proxy_request_inner(
            &state,
            &service_account_auth(&sa_id, &owner_id),
            &catalog_service_id,
            "status",
            proxy_request("/proxy/catalog/status"),
            &mut resolved_slug,
        )
        .await
        .expect("personal service account should resolve owner-owned service");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(resolved_slug, service.slug);
        server.abort();
    }

    #[tokio::test]
    async fn service_account_scope_denial_happens_after_owner_resolution() {
        let Some(db) = connect_test_database("proxy_sa_scope_denied").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let org_id = Uuid::new_v4().to_string();
        let sa_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .unwrap();
        let service = insert_user_service(&db, &org_id, "org-sa-scoped", &base_url, None).await;

        let state = test_app_state(db.clone());
        let mut auth = service_account_auth(&sa_id, &org_id);
        auth.allow_all_services = false;
        auth.allowed_service_ids = vec![Uuid::new_v4().to_string()];
        let mut resolved_slug = String::new();
        let err = proxy_request_by_slug_inner(
            &state,
            &auth,
            &service.slug,
            "status",
            proxy_request(&format!(
                "/proxy/s/org-sa-scoped/status?_nyxid_via={}",
                service.id
            )),
            &mut resolved_slug,
        )
        .await
        .expect_err("scope check should deny the resolved service");

        assert!(
            matches!(err, AppError::ApiKeyScopeForbidden(_)),
            "expected scope denial after resolution, got: {err}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn regular_admin_user_still_resolves_org_service_via_membership_policy() {
        let Some(db) = connect_test_database("proxy_admin_org_control").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let org_id = Uuid::new_v4().to_string();
        let admin_id = Uuid::new_v4().to_string();
        seed_org_actor(&db, &org_id, &admin_id, OrgRole::Admin).await;
        db.collection::<NotificationChannel>(NOTIFICATION_CHANNELS)
            .insert_one(notification_channel(&admin_id, 0))
            .await
            .unwrap();
        let service = insert_user_service(&db, &org_id, "admin-org-control", &base_url, None).await;
        db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
            .insert_one(approval_config(&org_id, &service.id))
            .await
            .unwrap();

        let state = test_app_state(db.clone());
        let mut resolved_slug = String::new();
        let err = proxy_request_by_slug_inner(
            &state,
            &access_token_auth(&admin_id),
            &service.slug,
            "sensitive",
            proxy_request("/proxy/s/admin-org-control/sensitive"),
            &mut resolved_slug,
        )
        .await
        .expect_err("org approval should block until timeout in test");

        assert!(
            matches!(err, AppError::ApprovalFailed { .. }),
            "unexpected proxy error: {err}"
        );
        let approval = find_approval_request(&db, &org_id, &service.id).await;
        assert!(approval.from_org_policy);
        assert_eq!(approval.user_id, org_id);
        assert_eq!(approval.requester_type, "access_token");
        assert_eq!(approval.requester_id, admin_id);
        server.abort();
    }
}

#[derive(Debug, Serialize, ToSchema)]
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
    /// Whether NyxID can serve a Scalar UI for this service
    pub docs_url: Option<String>,
    /// Proxied OpenAPI JSON URL
    pub openapi_url: Option<String>,
    /// Proxied AsyncAPI JSON URL
    pub asyncapi_url: Option<String>,
    /// Whether the service advertises streaming support
    pub streaming_supported: bool,
    /// Whether the service supports WebSocket passthrough via
    /// `/api/v1/proxy/{service_id}` or `/api/v1/proxy/s/{slug}`.
    /// Derived from the service's `capabilities.supports_websocket`
    /// flag. Returns `false` when the capability is not declared.
    pub websocket_supported: bool,
}

#[derive(Debug, Deserialize)]
pub struct ProxyServicesQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProxyServicesResponse {
    pub services: Vec<ProxyServiceItem>,
    pub custom_services: Vec<ProxyServiceItem>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

/// GET /api/v1/proxy/services
///
/// List downstream services available for proxying with their proxy URLs.
/// Excludes "provider" category services (not proxyable).
/// Supports pagination via `page` and `per_page` query parameters.
#[utoipa::path(
    get,
    path = "/api/v1/proxy/services",
    params(
        ("page" = Option<u64>, Query, description = "Page number"),
        ("per_page" = Option<u64>, Query, description = "Items per page")
    ),
    responses(
        (status = 200, description = "Proxyable downstream services", body = ProxyServicesResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy"
)]
pub async fn list_proxy_services(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ProxyServicesQuery>,
) -> AppResult<Json<ProxyServicesResponse>> {
    auth_user.ensure_rest_proxy_access()?;

    let user_id_str = auth_user.user_id.to_string();
    let base = state.config.base_url.trim_end_matches('/');

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let mut filter = doc! {
        "is_active": true,
        "service_category": { "$ne": "provider" },
    };
    filter.extend(legacy_http_service_type_filter());

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
            docs_url: s
                .openapi_spec_url
                .as_ref()
                .or(s.asyncapi_spec_url.as_ref())
                .map(|_| format!("{base}/api/v1/proxy/services/{}/docs", s.id)),
            openapi_url: s
                .openapi_spec_url
                .as_ref()
                .map(|_| format!("{base}/api/v1/proxy/services/{}/openapi.json", s.id)),
            asyncapi_url: s
                .asyncapi_spec_url
                .as_ref()
                .map(|_| format!("{base}/api/v1/proxy/services/{}/asyncapi.json", s.id)),
            streaming_supported: s.streaming_supported,
            websocket_supported: s
                .capabilities
                .as_ref()
                .is_some_and(|c| c.supports_websocket),
        })
        .collect();

    let visible_user_services =
        user_service_service::list_user_services_with_sources(&state.db, &user_id_str).await?;
    let custom_user_services: Vec<_> = visible_user_services
        .into_iter()
        .map(|entry| entry.service)
        .filter(|service| service.catalog_service_id.is_none() && service.service_type == "http")
        .collect();

    let endpoint_ids: Vec<&str> = custom_user_services
        .iter()
        .map(|service| service.endpoint_id.as_str())
        .collect();
    let endpoints: Vec<UserEndpoint> = if endpoint_ids.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find(doc! { "_id": { "$in": &endpoint_ids } })
            .await?
            .try_collect()
            .await?
    };
    let endpoint_by_id: HashMap<String, UserEndpoint> = endpoints
        .into_iter()
        .map(|endpoint| (endpoint.id.clone(), endpoint))
        .collect();

    let mut custom_services: Vec<ProxyServiceItem> = custom_user_services
        .into_iter()
        .filter_map(|service| {
            let endpoint = endpoint_by_id.get(&service.endpoint_id)?;
            endpoint.openapi_spec_url.as_ref()?;

            let name = if endpoint.label.trim().is_empty() {
                service.slug.clone()
            } else {
                endpoint.label.clone()
            };

            Some(ProxyServiceItem {
                id: service.id.clone(),
                name,
                slug: service.slug.clone(),
                description: None,
                service_category: "custom".to_string(),
                connected: true,
                requires_connection: false,
                has_node_binding: service.node_id.is_some(),
                proxy_url: format!("{base}/api/v1/proxy/{}/{{path}}", service.id),
                proxy_url_slug: format!("{base}/api/v1/proxy/s/{}/{{path}}", service.slug),
                docs_url: Some(format!("{base}/api/v1/proxy/services/{}/docs", service.id)),
                openapi_url: Some(format!(
                    "{base}/api/v1/proxy/services/{}/openapi.json",
                    service.id
                )),
                asyncapi_url: None,
                streaming_supported: false,
                websocket_supported: false,
            })
        })
        .collect();
    custom_services.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.slug.cmp(&right.slug))
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(Json(ProxyServicesResponse {
        services: items,
        custom_services,
        total,
        page,
        per_page,
    }))
}

#[cfg(test)]
mod discovery_tests {
    use super::{ProxyServicesQuery, list_proxy_services};
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::COLLECTION_NAME as USERS;
    use crate::models::user::UserType;
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
        test_user_endpoint, test_user_service,
    };
    use axum::{
        Json,
        extract::{Query, State},
    };
    use uuid::Uuid;

    fn catalog_service(service_id: &str) -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = service_id.to_string();
        service.slug = "catalog-service".to_string();
        service.name = "Catalog Service".to_string();
        service.base_url = "https://catalog.example.com".to_string();
        service.openapi_spec_url = Some("https://example.com/catalog-openapi.json".to_string());
        service
    }

    #[tokio::test]
    async fn list_proxy_services_separates_custom_services_and_dedupes_catalog_backed_rows() {
        let Some(db) = connect_test_database("proxy_services_custom").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_many([
                test_user(&caller_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &caller_id, OrgRole::Member, None))
            .await
            .unwrap();

        let catalog = catalog_service(&Uuid::new_v4().to_string());
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog.clone())
            .await
            .unwrap();

        let custom_endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "Personal Custom",
            "https://personal.example.com",
            Some("https://example.com/personal-openapi.json"),
            None,
        );
        let custom_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "personal-custom",
            &custom_endpoint.id,
            None,
            Some("node-1"),
        );
        let no_spec_endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "No Spec",
            "https://nospec.example.com",
            None,
            None,
        );
        let no_spec_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "no-spec",
            &no_spec_endpoint.id,
            None,
            None,
        );
        let catalog_backed_endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "Catalog Backed",
            "https://catalog-user.example.com",
            Some("https://example.com/catalog-user-openapi.json"),
            Some(&catalog.id),
        );
        let catalog_backed_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "catalog-backed",
            &catalog_backed_endpoint.id,
            Some(&catalog.id),
            None,
        );
        let org_endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &org_id,
            "Org Shared",
            "https://org.example.com",
            Some("https://example.com/org-openapi.json"),
            None,
        );
        let org_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &org_id,
            "org-shared",
            &org_endpoint.id,
            None,
            None,
        );

        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_many([
                custom_endpoint.clone(),
                no_spec_endpoint,
                catalog_backed_endpoint,
                org_endpoint.clone(),
            ])
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([
                custom_service.clone(),
                no_spec_service.clone(),
                catalog_backed_service.clone(),
                org_service.clone(),
            ])
            .await
            .unwrap();

        let state = test_app_state(db);
        let Json(response) = list_proxy_services(
            State(state),
            test_auth_user(&caller_id),
            Query(ProxyServicesQuery {
                page: None,
                per_page: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.total, 1);
        assert_eq!(response.page, 1);
        assert_eq!(response.per_page, 50);
        assert_eq!(response.services.len(), 1);
        assert_eq!(response.services[0].id, catalog.id);

        let custom_ids: Vec<&str> = response
            .custom_services
            .iter()
            .map(|service| service.id.as_str())
            .collect();
        assert!(custom_ids.contains(&custom_service.id.as_str()));
        assert!(custom_ids.contains(&org_service.id.as_str()));
        assert!(!custom_ids.contains(&catalog_backed_service.id.as_str()));
        assert!(!custom_ids.contains(&no_spec_service.id.as_str()));

        let personal = response
            .custom_services
            .iter()
            .find(|service| service.id == custom_service.id)
            .expect("personal custom service should be included");
        let expected_docs_url = format!(
            "http://localhost:3001/api/v1/proxy/services/{}/docs",
            custom_service.id
        );
        let expected_openapi_url = format!(
            "http://localhost:3001/api/v1/proxy/services/{}/openapi.json",
            custom_service.id
        );
        assert_eq!(personal.name, custom_endpoint.label);
        assert_eq!(personal.slug, custom_service.slug);
        assert_eq!(personal.service_category, "custom");
        assert!(personal.connected);
        assert!(!personal.requires_connection);
        assert!(personal.has_node_binding);
        assert_eq!(
            personal.docs_url.as_deref(),
            Some(expected_docs_url.as_str())
        );
        assert_eq!(
            personal.openapi_url.as_deref(),
            Some(expected_openapi_url.as_str())
        );
        assert!(personal.asyncapi_url.is_none());
        assert!(!personal.streaming_supported);
        assert!(!personal.websocket_supported);
    }
}
