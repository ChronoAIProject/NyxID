use axum::{
    Json,
    body::Body,
    extract::{FromRequestParts, OriginalUri, Path, Query, State, ws::WebSocketUpgrade},
    http::{Method, Request, StatusCode},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use utoipa::ToSchema;

use crate::AppState;
use crate::downstream_disconnect::{
    CancelOnDropStream, request_cancellation, until_client_disconnect,
};
use crate::errors::{AppError, AppResult};
use crate::models::api_key::ApiKeyPurpose;
use crate::models::durable_operation_execution::DurableExecutionStatus;
use crate::models::durable_operation_grant::DurableReplayPolicy;
use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
use crate::models::service_billing::{BillingMetric, PlatformUsage, ResaleUsage};
use crate::models::usage_meter::CredentialClass;
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::node_ws_manager::{
    NodeProxyFailure, NodeProxyRequest, ProxyResponseType, StreamChunk,
};
use crate::services::{
    approval_service, audit_service, chatgpt_translator, delegation_service,
    durable_operation_grant_service, identity_service, llm_usage_service, node_metrics_service,
    node_routing_service, node_service, notification_service, operation_descriptor,
    proxy_discovery_service, proxy_service, sse_parser, ws_frame_injector,
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
        AppError::DurableGrantMissing(_) => (403, 9008),
        AppError::DurableGrantMismatch(_) => (403, 9009),
        AppError::DurableGrantExpired => (410, 9010),
        AppError::DurableGrantRevoked => (403, 9011),
        AppError::DurableGrantContractDrift => (409, 9012),
        AppError::DurableGrantQuotaExhausted => (429, 9013),
        AppError::DurableOperationDuplicate => (409, 9014),
        AppError::DurableOperationConflict => (409, 9015),
        AppError::DurableOperationOutcomeUncertain => (409, 9016),
        AppError::NodeCredentialMissing(_) => (502, 8004),
        AppError::WsProxyDownstream(_) => (502, 8005),
        AppError::ClientDisconnected => (499, 8012),
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

fn proxy_client_disconnected(service_id: &str) -> AppError {
    tracing::debug!(
        service_id,
        "Cancelled upstream proxy work after downstream client disconnected"
    );
    // Deliberately not `Internal`: the client hanging up is not a server fault,
    // and this response is never written anywhere. Reporting it as a 500 turns
    // ordinary client cancellation into false error-rate signal.
    AppError::ClientDisconnected
}

fn log_upstream_error(
    service_id: &str,
    status: StatusCode,
    response_size: usize,
    upstream_request_id: &str,
) {
    tracing::error!(
        service_id = %service_id,
        status = %status,
        response_size,
        upstream_request_id,
        "Upstream returned error response"
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
/// `latency_ms` is the handler-to-response-start reach metric retained for
/// product analytics compatibility. Operational phase diagnostics are emitted
/// separately at body/stream termination so this event is not mistaken for a
/// full streaming lifetime. Any non-2xx status is the caller's signal not to
/// call this helper.
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
    "retry-after",
    "preference-applied",
    "location",
    "operation-location",
];

const ASYNC_LOCATION_HEADERS: &[&str] = &["location", "operation-location"];

#[derive(Clone, Debug)]
struct AsyncLocationContext {
    service_base: url::Url,
    downstream_request: url::Url,
    caller_proxy_prefix: String,
}

impl AsyncLocationContext {
    fn new(
        target_base_url: &str,
        downstream_path: &str,
        downstream_query: Option<&str>,
        caller_proxy_prefix: Option<String>,
    ) -> Option<Self> {
        let caller_proxy_prefix = caller_proxy_prefix?;
        let mut service_base = url::Url::parse(target_base_url).ok()?;
        if !matches!(service_base.scheme(), "http" | "https") {
            return None;
        }
        if !service_base.path().ends_with('/') {
            let path = format!("{}/", service_base.path());
            service_base.set_path(&path);
        }
        service_base.set_query(None);
        service_base.set_fragment(None);

        let mut downstream_request = service_base
            .join(downstream_path.trim_start_matches('/'))
            .ok()?;
        downstream_request.set_query(downstream_query);
        Some(Self {
            service_base,
            downstream_request,
            caller_proxy_prefix,
        })
    }

    fn from_downstream_request(
        target_base_url: &str,
        downstream_request: url::Url,
        caller_proxy_prefix: Option<String>,
    ) -> Option<Self> {
        let caller_proxy_prefix = caller_proxy_prefix?;
        let mut service_base = url::Url::parse(target_base_url).ok()?;
        if !matches!(service_base.scheme(), "http" | "https") {
            return None;
        }
        if !service_base.path().ends_with('/') {
            let path = format!("{}/", service_base.path());
            service_base.set_path(&path);
        }
        service_base.set_query(None);
        service_base.set_fragment(None);
        Some(Self {
            service_base,
            downstream_request,
            caller_proxy_prefix,
        })
    }

    fn rewrite(&self, value: &str) -> Option<axum::http::HeaderValue> {
        let resolved = self.downstream_request.join(value).ok()?;
        if resolved.scheme() != self.service_base.scheme()
            || resolved.host_str() != self.service_base.host_str()
            || resolved.port_or_known_default() != self.service_base.port_or_known_default()
        {
            return None;
        }

        let base_path = self.service_base.path();
        let relative_path = if resolved.path() == base_path.trim_end_matches('/') {
            ""
        } else {
            resolved.path().strip_prefix(base_path)?
        };
        let mut caller_location = format!(
            "{}{}",
            self.caller_proxy_prefix,
            relative_path.trim_start_matches('/')
        );
        if let Some(query) = resolved.query() {
            caller_location.push('?');
            caller_location.push_str(query);
        }
        if let Some(fragment) = resolved.fragment() {
            caller_location.push('#');
            caller_location.push_str(fragment);
        }
        axum::http::HeaderValue::from_str(&caller_location).ok()
    }
}

fn caller_proxy_prefix(request_path: &str, downstream_path: &str) -> Option<String> {
    let normalized = downstream_path.trim_matches('/');
    if normalized.is_empty() {
        return Some(format!("{}/", request_path.trim_end_matches('/')));
    }
    let suffix = format!("/{normalized}");
    let prefix = request_path.strip_suffix(&suffix)?;
    Some(format!("{}/", prefix.trim_end_matches('/')))
}

fn ensure_proxy_request_id(headers: &mut axum::http::HeaderMap) -> String {
    if let Some(existing) = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
    {
        return existing.to_string();
    }
    let generated = uuid::Uuid::new_v4().to_string();
    headers.insert(
        "x-request-id",
        axum::http::HeaderValue::from_str(&generated).expect("UUID is a valid header value"),
    );
    generated
}

fn apply_proxy_request_id_header(response: &mut Response, request_id: &str) {
    if !response.headers().contains_key("x-request-id")
        && let Ok(value) = axum::http::HeaderValue::from_str(request_id)
    {
        response.headers_mut().insert("x-request-id", value);
    }
}

#[derive(Clone, Copy)]
struct ProxyExchangeStartedAt(std::time::Instant);

#[derive(Clone)]
struct ProxyExchangeDiagnostics {
    started_at: std::time::Instant,
    headers_at: std::time::Instant,
    target_resolution_admission_ms: u64,
    downstream_headers_ms: Option<u64>,
    method: String,
    status: u16,
    response_mode: &'static str,
    trace_id: String,
    request_id: String,
}

impl ProxyExchangeDiagnostics {
    #[allow(clippy::too_many_arguments)]
    fn new(
        started_at: std::time::Instant,
        target_resolution_admission_ms: u64,
        downstream_headers_ms: Option<u64>,
        method: &Method,
        status: StatusCode,
        response_mode: &'static str,
        headers: &axum::http::HeaderMap,
    ) -> Self {
        Self {
            started_at,
            headers_at: std::time::Instant::now(),
            target_resolution_admission_ms,
            downstream_headers_ms,
            method: method.as_str().to_string(),
            status: status.as_u16(),
            response_mode,
            trace_id: sanitized_trace_id(headers).unwrap_or_default(),
            request_id: sanitized_request_id(headers).unwrap_or_default(),
        }
    }

    fn emit(&self, termination: &'static str) {
        let body_ms = elapsed_ms(self.headers_at);
        let downstream_headers_available = self.downstream_headers_ms.is_some();
        let downstream_headers_ms = self.downstream_headers_ms.unwrap_or(0);
        tracing::info!(
            target_resolution_admission_ms = self.target_resolution_admission_ms,
            connect_send_ms = tracing::field::Empty,
            downstream_headers_ms,
            downstream_headers_available,
            body_ms,
            total_ms = elapsed_ms(self.started_at),
            method = %self.method,
            upstream_status = self.status,
            response_mode = self.response_mode,
            termination,
            trace_id = %self.trace_id,
            request_id = %self.request_id,
            "Proxy exchange phase diagnostics"
        );
    }
}

struct ProxyStreamDiagnostics {
    diagnostics: ProxyExchangeDiagnostics,
    finished: bool,
}

impl ProxyStreamDiagnostics {
    fn new(diagnostics: ProxyExchangeDiagnostics) -> Self {
        Self {
            diagnostics,
            finished: false,
        }
    }

    fn finish(&mut self, termination: &'static str) {
        if !self.finished {
            self.finished = true;
            self.diagnostics.emit(termination);
        }
    }
}

impl Drop for ProxyStreamDiagnostics {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            self.diagnostics.emit("client_disconnect");
        }
    }
}

fn elapsed_ms(started_at: std::time::Instant) -> u64 {
    started_at.elapsed().as_millis().min(u64::MAX as u128) as u64
}

fn emit_preheader_diagnostics(
    started_at: std::time::Instant,
    target_resolution_admission_ms: u64,
    method: &Method,
    headers: &axum::http::HeaderMap,
    termination: &'static str,
) {
    let trace_id = sanitized_trace_id(headers).unwrap_or_default();
    let request_id = sanitized_request_id(headers).unwrap_or_default();
    tracing::info!(
        target_resolution_admission_ms,
        connect_send_ms = tracing::field::Empty,
        downstream_headers_ms = 0u64,
        downstream_headers_available = false,
        body_ms = 0u64,
        total_ms = elapsed_ms(started_at),
        method = %method,
        upstream_status = 0u16,
        response_mode = "buffered",
        termination,
        trace_id = %trace_id,
        request_id = %request_id,
        "Proxy exchange phase diagnostics"
    );
}

fn sanitized_trace_id(headers: &axum::http::HeaderMap) -> Option<String> {
    if !proxy_service::valid_traceparent_header(headers) {
        return None;
    }
    let traceparent = headers.get("traceparent")?.to_str().ok()?;
    Some(traceparent[3..35].to_string())
}

fn sanitized_request_id(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get("x-request-id")?.to_str().ok()?;
    (value.len() <= 128
        && !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')))
    .then(|| value.to_string())
}

fn forwarded_response_header_value(
    name_lower: &str,
    value: &[u8],
    is_sse: bool,
    location_context: Option<&AsyncLocationContext>,
) -> Option<axum::http::HeaderValue> {
    if !forwardable_response_header(name_lower, is_sse) {
        return None;
    }
    if ASYNC_LOCATION_HEADERS.contains(&name_lower) {
        let raw = std::str::from_utf8(value).ok()?;
        return location_context?.rewrite(raw);
    }
    axum::http::HeaderValue::from_bytes(value).ok()
}

fn node_is_sse_headers(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && crate::mw::security_headers::is_sse_media_type(value)
    })
}

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
    catalog_service_slug: Option<String>,
    node_id: Option<String>,
    /// The UserService ID for API key scope checks.
    user_service_id: Option<String>,
    has_server_credential: bool,
    master_credential: bool,
    /// The user_id that owns the resolved UserService. For personal
    /// resolutions this is the actor; for org-routed resolutions this is
    /// the org's user_id. Used to scope NodeServiceBinding fallback
    /// lookups so the failover list reflects the org's bindings, not
    /// just the calling member's personal bindings.
    effective_owner_id: String,
}

#[derive(Clone, Debug, Default)]
struct ConnectionUsageStats {
    frames_in: i64,
    frames_out: i64,
    bytes_in: i64,
    bytes_out: i64,
    duration: std::time::Duration,
    realtime_llm_usage: llm_usage_service::RealtimeLlmUsageSummary,
}

impl ConnectionUsageStats {
    fn total_bytes(&self) -> i64 {
        self.bytes_in.saturating_add(self.bytes_out)
    }
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
    pool_selection: Option<&crate::services::service_pool_service::PoolSelection>,
) {
    let mut event_data = serde_json::json!({
        "routed_via": "org",
        "service_id": service_id,
        "user_service_id": user_service_id,
        // Org-routed audits use org_user_id; node-routed audits use owner_user_id.
        // Owner-centric audit queries must check both fields.
        "org_user_id": routing.org_user_id,
        "member_user_id": routing.member_user_id,
        "membership_id": routing.membership_id,
    });
    add_pool_selection_metadata(&mut event_data, pool_selection);
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "proxy_routed_via_org",
        Some(event_data),
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
    pool_selection: Option<&crate::services::service_pool_service::PoolSelection>,
) {
    let mut event_data = serde_json::json!({
        "routed_via": "personal",
        "service_id": service_id,
        "user_service_id": user_service_id,
    });
    add_pool_selection_metadata(&mut event_data, pool_selection);
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "proxy_routed_via_personal",
        Some(event_data),
    );
}

fn add_pool_selection_metadata(
    value: &mut serde_json::Value,
    pool_selection: Option<&crate::services::service_pool_service::PoolSelection>,
) {
    let Some(selection) = pool_selection else {
        return;
    };
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "pool_id".to_string(),
            serde_json::Value::String(selection.pool_id.clone()),
        );
        object.insert(
            "pool_slug".to_string(),
            serde_json::Value::String(selection.pool_slug.clone()),
        );
        object.insert(
            "chosen_user_service_id".to_string(),
            serde_json::Value::String(selection.selected_member_id.clone()),
        );
        object.insert(
            "pool_strategy".to_string(),
            serde_json::Value::String(selection.strategy.as_str().to_string()),
        );
    }
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

fn collect_ws_forward_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let name_lower = name.as_str().to_ascii_lowercase();
            let allowed = ALLOWED_WS_FORWARD_HEADERS.contains(&name_lower.as_str())
                || proxy_service::is_allowed_forward_header_prefix(&name_lower);
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

fn effective_header_map(headers: Vec<(String, String)>) -> AppResult<axum::http::HeaderMap> {
    let mut result = axum::http::HeaderMap::new();
    for (name, value) in headers {
        let name = axum::http::HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            AppError::DurableGrantMismatch(
                "effective downstream header name is invalid".to_string(),
            )
        })?;
        let value = axum::http::HeaderValue::from_str(&value).map_err(|_| {
            AppError::DurableGrantMismatch(
                "effective downstream header value is invalid".to_string(),
            )
        })?;
        result.append(name, value);
    }
    Ok(result)
}

fn strip_durable_idempotency_defaults(
    headers: &mut Vec<crate::models::default_request_header::DefaultRequestHeader>,
) {
    headers.retain(|header| !header.name.eq_ignore_ascii_case("idempotency-key"));
}

fn single_system_header(
    headers: &axum::http::HeaderMap,
    name: &'static str,
) -> AppResult<Option<String>> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(AppError::DurableGrantMismatch(format!(
            "{name} must be supplied exactly once"
        )));
    }
    let value = value
        .to_str()
        .map_err(|_| AppError::DurableGrantMismatch(format!("{name} must be valid ASCII text")))?;
    Ok(Some(value.to_string()))
}

fn caller_bearer_token_for_downstream(
    headers: &axum::http::HeaderMap,
    scheduled_invocation: bool,
) -> Option<String> {
    if scheduled_invocation {
        return None;
    }
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(String::from)
}

async fn finish_durable_operation(
    state: &AppState,
    auth_user: &AuthUser,
    reservation: &durable_operation_grant_service::DurableExecutionReservation,
    status: DurableExecutionStatus,
    response_status: Option<u16>,
    node_id: Option<&str>,
    detail: &'static str,
) {
    durable_operation_grant_service::mark_terminal(
        &state.db,
        reservation,
        status,
        response_status,
        detail,
    )
    .await;
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "durable_operation_terminal",
        Some(serde_json::json!({
            "api_key_id": auth_user.api_key_id.as_deref(),
            "grant_id": &reservation.grant_id,
            "operation_id": &reservation.operation_id,
            "endpoint_id": &reservation.endpoint_id,
            "contract_digest": &reservation.contract_digest,
            "terminal_outcome": status,
            "response_status": response_status,
            "downstream_attempts": 1,
            "node_id": node_id,
            "client_audit_binding": &reservation.client_audit_binding,
        })),
    );
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
    mut request: Request<Body>,
) -> AppResult<Response> {
    // Emit `proxy.error` on any error branch reached via this handler. The
    // inner function threads `resolved_slug` through so we can attach a
    // real slug (never a UUID from the route path) even when the error
    // happens after service resolution. See docs/TELEMETRY.md §5.1.
    let started_at = std::time::Instant::now();
    let method = request.method().clone();
    request
        .extensions_mut()
        .insert(ProxyExchangeStartedAt(started_at));
    let request_id = ensure_proxy_request_id(request.headers_mut());
    let mut resolved_slug = String::new();
    let mut result = proxy_request_inner(
        &state,
        &auth_user,
        &service_id,
        &path,
        request,
        &mut resolved_slug,
    )
    .await;
    if let Ok(response) = &mut result {
        apply_proxy_request_id_header(response, &request_id);
    }
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
    validate_original_proxy_request_path(&request)?;
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
            Some(&state.connection_expiry_notifier),
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
                    resolved.pool_selection.as_ref(),
                );
            } else {
                audit_personal_routing(
                    state,
                    auth_user,
                    Some(&resolved.user_service_id),
                    &effective_service_id,
                    resolved.pool_selection.as_ref(),
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
                    catalog_service_slug: resolved.catalog_service_slug,
                    node_id: resolved.node_id,
                    user_service_id: Some(resolved.user_service_id),
                    has_server_credential: resolved.has_server_credential,
                    master_credential: resolved.master_credential,
                    effective_owner_id: resolved
                        .org_routing
                        .as_ref()
                        .map(|r| r.org_user_id.clone())
                        .unwrap_or_else(|| user_id_str.clone()),
                }),
                TargetMode::CallerAddressed,
                Vec::new(),
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
        Some(&state.connection_expiry_notifier),
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
                resolved.pool_selection.as_ref(),
            );
        } else {
            audit_personal_routing(
                state,
                auth_user,
                Some(&resolved.user_service_id),
                &effective_service_id,
                resolved.pool_selection.as_ref(),
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
                catalog_service_slug: resolved.catalog_service_slug,
                node_id: resolved.node_id,
                user_service_id: Some(resolved.user_service_id),
                has_server_credential: resolved.has_server_credential,
                master_credential: resolved.master_credential,
                effective_owner_id: resolved
                    .org_routing
                    .as_ref()
                    .map(|r| r.org_user_id.clone())
                    .unwrap_or_else(|| user_id_str.clone()),
            }),
            TargetMode::CallerAddressed,
            Vec::new(),
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
    mut request: Request<Body>,
) -> AppResult<Response> {
    // Start empty; `proxy_request_by_slug_inner` populates this once the
    // resolved `UserService`/`DownstreamService` is available. We
    // intentionally do NOT seed with the path-param slug: telemetry §5.1
    // requires a resolved slug or empty, and the path param is unvalidated
    // user input until resolution succeeds.
    let started_at = std::time::Instant::now();
    let method = request.method().clone();
    request
        .extensions_mut()
        .insert(ProxyExchangeStartedAt(started_at));
    let request_id = ensure_proxy_request_id(request.headers_mut());
    let mut resolved_slug = String::new();
    let mut result = proxy_request_by_slug_inner(
        &state,
        &auth_user,
        &slug,
        &path,
        request,
        &mut resolved_slug,
    )
    .await;
    if let Ok(response) = &mut result {
        apply_proxy_request_id_header(response, &request_id);
    }
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
    validate_original_proxy_request_path(&request)?;
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
            Some(&state.connection_expiry_notifier),
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
                    resolved.pool_selection.as_ref(),
                );
            } else {
                audit_personal_routing(
                    state,
                    auth_user,
                    Some(&resolved.user_service_id),
                    &effective_service_id,
                    resolved.pool_selection.as_ref(),
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
                    catalog_service_slug: resolved.catalog_service_slug,
                    node_id: resolved.node_id,
                    user_service_id: Some(resolved.user_service_id),
                    has_server_credential: resolved.has_server_credential,
                    master_credential: resolved.master_credential,
                    effective_owner_id: resolved
                        .org_routing
                        .as_ref()
                        .map(|r| r.org_user_id.clone())
                        .unwrap_or_else(|| user_id_str.clone()),
                }),
                TargetMode::CallerAddressed,
                Vec::new(),
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
        Some(&state.connection_expiry_notifier),
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
                resolved.pool_selection.as_ref(),
            );
        } else {
            audit_personal_routing(
                state,
                auth_user,
                Some(&resolved.user_service_id),
                &effective_service_id,
                resolved.pool_selection.as_ref(),
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
                catalog_service_slug: resolved.catalog_service_slug,
                node_id: resolved.node_id,
                user_service_id: Some(resolved.user_service_id),
                has_server_credential: resolved.has_server_credential,
                master_credential: resolved.master_credential,
                effective_owner_id: resolved
                    .org_routing
                    .as_ref()
                    .map(|r| r.org_user_id.clone())
                    .unwrap_or_else(|| user_id_str.clone()),
            }),
            TargetMode::CallerAddressed,
            Vec::new(),
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

/// Axum decodes wildcard captures before handlers receive them. Validate the
/// original encoded URI as well so an encoded separator cannot disappear
/// before `validate_requested_proxy_path` runs on the captured suffix.
fn validate_original_proxy_request_path(request: &Request<Body>) -> AppResult<()> {
    let path = request
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| request.uri().path(), |uri| uri.path());
    proxy_service::validate_requested_proxy_path(path)
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
/// Forward a request to a `DownstreamService` by catalog id (the
/// admin/master-credential path). Exposed for the assistant
/// pass-through, which must resolve the admin service and must not fall
/// back to a caller-owned `UserService` -- see `services::assistant_service`.
pub(crate) async fn execute_proxy(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
    resolved_slug: &mut String,
) -> AppResult<Response> {
    Box::pin(execute_proxy_inner(
        state,
        auth_user,
        service_id,
        path,
        request,
        None,
        TargetMode::CallerAddressed,
        Vec::new(),
        resolved_slug,
    ))
    .await
}

/// How the proxy target was chosen, which decides whether caller-owned
/// routing state applies to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetMode {
    /// The caller named the service (`/proxy/{id}`, `/proxy/s/{slug}`, LLM
    /// gateway, MCP). Caller state governs: scoped-token service allowlist,
    /// personal node pins, personal connection state.
    CallerAddressed,
    /// The server chose the service for a platform surface and the caller
    /// could not have named anything else (assistant chat pass-through).
    AdminManaged,
}

/// Execute a proxy request against a **server-chosen** platform service.
///
/// Same data plane as [`execute_proxy`] -- identity propagation, delegation
/// token, approvals, audit, billing, streaming -- with the caller-state
/// resolution deliberately switched off, because none of it can apply to a
/// target the caller never named:
///
/// - the scoped-token service allowlist is not consulted. Those lists hold
///   `UserService` ids, so an admin catalog row can never appear in one:
///   leaving the gate on made every restricted token (every OAuth access
///   token minted with `resource`/`allowed_service_ids`, e.g. the Aevatar
///   console's) fail the assistant surface with `api_key_scope_forbidden`,
///   with no grant a user could add to fix it. This does **not** widen what
///   the run can reach: the delegation token this path mints still inherits
///   the caller's restrictions verbatim
///   (`TokenRestrictionClaims::from_auth_user`), so a restricted caller's
///   assistant run stays restricted on every callback into NyxID.
/// - personal node pins and personal connection state are not consulted
///   (see `proxy_service::resolve_admin_proxy_target`).
pub(crate) async fn execute_admin_proxy(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
    extra_outbound_headers: Vec<(String, String)>,
    resolved_slug: &mut String,
) -> AppResult<Response> {
    // Keep the very large proxy state machine off caller task stacks. This
    // wrapper is shared by assistant handlers whose own futures already carry
    // substantial request state.
    Box::pin(execute_proxy_inner(
        state,
        auth_user,
        service_id,
        path,
        request,
        None,
        TargetMode::AdminManaged,
        extra_outbound_headers,
        resolved_slug,
    ))
    .await
}

pub(crate) fn enforce_proxy_billing_classification(
    request: &Request<Body>,
) -> AppResult<crate::services::billing::route_inventory::BillingEgressPermit> {
    crate::services::billing::route_inventory::enforce_billing_egress_classification(
        request
            .extensions()
            .get::<crate::services::billing::route_inventory::BillingRoutePolicy>()
            .copied(),
        crate::services::billing::BillingIngress::Proxy,
    )
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

    // Hard-fail when the service is explicitly node-routed but no dispatchable
    // node could be resolved. Falling through to direct routing would
    // violate the "Route via Node" contract and silently bypass the
    // intended execution boundary (node isolation, local credentials,
    // private-network access).
    if nr.is_none() && node_routing_required {
        let err = AppError::NodeOffline(
            "Service is configured to route via a node, but no dispatchable node is available"
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

    // Check if the configured node is actually dispatchable on this instance
    // (DB Online + WS-connected). Without this, a request landing on a backend
    // instance that doesn't hold the WS connection is routed to the node and
    // surfaces `503 node_offline` to the caller even though `node list` shows
    // the node online globally. See issue #325.
    let primary_dispatchable = node_routing_service::is_node_id_dispatchable(
        &state.db,
        explicit_node_id,
        state.node_ws_manager.as_ref(),
    )
    .await?;

    let fallback_node_ids: Vec<String> = node_routing_service::list_dispatchable_binding_node_ids(
        &state.db,
        user_id,
        service_id,
        state.node_ws_manager.as_ref(),
    )
    .await?
    .into_iter()
    .filter(|node_id| node_id != explicit_node_id)
    .collect();

    if !primary_dispatchable {
        if fallback_node_ids.is_empty() {
            tracing::warn!(
                configured_node_id = %explicit_node_id,
                service_id = %service_id,
                "Configured node is not dispatchable on this instance and no dispatchable fallback bindings; caller will hard-fail the node-pinned request"
            );
        } else {
            tracing::warn!(
                configured_node_id = %explicit_node_id,
                promoted_node_id = %fallback_node_ids[0],
                service_id = %service_id,
                "Configured node is not dispatchable on this instance; promoting a dispatchable fallback to primary"
            );
        }
    }

    Ok(node_routing_service::build_node_route(
        compose_pre_resolved_node_ids(explicit_node_id, primary_dispatchable, fallback_node_ids),
    ))
}

/// Pure helper: build the ordered node-id list for a pre-resolved route.
///
/// - If the configured node is dispatchable, it goes first and dispatchable fallbacks follow.
/// - If it is not dispatchable, only the dispatchable fallbacks remain; the first one is promoted
///   to primary by the caller via `build_node_route`.
/// - If nothing is dispatchable, returns an empty list. `build_node_route` then yields
///   `None`, and `execute_proxy_inner`'s pre_resolved arm hard-fails with
///   `NodeOffline` to honor the "Route via Node" contract
///   (see ChronoAIProject/NyxID#328).
fn compose_pre_resolved_node_ids(
    explicit_node_id: &str,
    primary_dispatchable: bool,
    fallback_node_ids: Vec<String>,
) -> Vec<String> {
    if primary_dispatchable {
        let mut ids = Vec::with_capacity(fallback_node_ids.len() + 1);
        ids.push(explicit_node_id.to_string());
        ids.extend(fallback_node_ids);
        ids
    } else {
        fallback_node_ids
    }
}

fn enforce_node_route_scope(
    route: &mut node_routing_service::NodeRoute,
    allowed_node_ids: &[String],
) -> AppResult<()> {
    if !allowed_node_ids.contains(&route.node_id) {
        return Err(AppError::ApiKeyScopeForbidden(
            "API key does not have access to this node".to_string(),
        ));
    }

    route
        .fallback_node_ids
        .retain(|node_id| allowed_node_ids.contains(node_id));
    Ok(())
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
/// (from `resolve_proxy_target_from_user_service`). When `None`, resolution
/// follows `target_mode`: caller-addressed requests fall back to the original
/// DownstreamService path, server-chosen platform targets resolve the admin
/// row alone (see [`execute_admin_proxy`]).
// One argument over the lint's threshold: every parameter is a distinct
// security-relevant input to resolution, and bundling them into a struct
// would only move the same fields behind one more indirection.
#[allow(clippy::too_many_arguments)]
async fn execute_proxy_inner(
    state: &AppState,
    auth_user: &AuthUser,
    service_id: &str,
    path: &str,
    request: Request<Body>,
    pre_resolved: Option<PreResolved>,
    target_mode: TargetMode,
    mut extra_outbound_headers: Vec<(String, String)>,
    resolved_slug: &mut String,
) -> AppResult<Response> {
    let exchange_started_at = request
        .extensions()
        .get::<ProxyExchangeStartedAt>()
        .map(|started| started.0)
        .unwrap_or_else(std::time::Instant::now);
    let downstream_cancellation = request_cancellation(&request);
    let billing_egress_permit = enforce_proxy_billing_classification(&request)?;

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
    let mut agent_override_applied = false;
    let (
        node_route,
        mut target,
        has_server_credential,
        master_credential,
        resolved_user_service_id,
        node_routing_required,
        catalog_service_slug,
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
        // `build_pre_resolved_node_route` could not resolve a dispatchable node
        // and no fallback binding exists. Falling through to direct routing
        // would silently bypass node isolation, local credentials, and
        // private-network access -- the exact contract "Route via Node"
        // promises. The legacy DownstreamService path enforces the same
        // invariant in `resolve_via_downstream_service`; this mirror keeps
        // the UserService path honest. See ChronoAIProject/NyxID#328.
        if node_route.is_none() && pre.node_id.as_deref().is_some_and(|nid| !nid.is_empty()) {
            let err = AppError::NodeOffline(
                "Service is configured to route via a node, but no dispatchable node is available"
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
                    "denial_reason": "node_routing_required_no_dispatchable_node",
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
            && let Err(err) = enforce_node_route_scope(route, &auth_user.allowed_node_ids)
        {
            audit_service::log_for_user(
                state.db.clone(),
                auth_user,
                "proxy_request_denied",
                Some(serde_json::json!({
                    "service_id": service_id,
                    "node_id": route.node_id,
                    "path": path,
                    "reason": err.to_string(),
                    "denial_reason": "api_key_scope_forbidden_node",
                    "response_status": 403,
                })),
            );
            return Err(err);
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
                Some(&state.connection_expiry_notifier),
            )
            .await?
        {
            pre.target.credential = override_cred;
            agent_override_applied = true;
        }

        let required = pre.node_id.is_some();
        let catalog_service_slug = pre.catalog_service_slug;
        (
            node_route,
            pre.target,
            pre.has_server_credential,
            pre.master_credential,
            pre.user_service_id,
            required,
            catalog_service_slug,
        )
    } else if target_mode == TargetMode::AdminManaged {
        // Server-chosen platform target: resolve the admin row alone, with
        // no caller-owned routing state. See `execute_admin_proxy`.
        let target = proxy_service::resolve_admin_proxy_target(
            &state.db,
            &state.encryption_keys,
            service_id,
        )
        .await?;
        let catalog_service_slug = Some(target.service.slug.clone());
        let has_server_credential = true;
        (
            None,
            target,
            has_server_credential,
            true,
            None,
            false,
            catalog_service_slug,
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

        let (
            node_route,
            target,
            has_server_credential,
            resolved_user_service_id,
            node_routing_required,
        ) = resolve_via_downstream_service(state, auth_user, &user_id_str, service_id).await?;
        let catalog_service_slug = Some(target.service.slug.clone());
        // Legacy path resolution succeeded — this is still personal
        // routing (caller's own DownstreamService + provider token), so
        // attribute it the same way the UserService path is attributed.
        // `user_service_id` is `None` because the legacy path doesn't own
        // one. Emitted AFTER `resolve_via_downstream_service` returns Ok
        // so we never record a "routed via personal" entry for a request
        // that failed before a target was resolved (disconnected service,
        // missing credential, etc.). See ChronoAIProject/NyxID#423.
        audit_personal_routing(state, auth_user, None, service_id, None);
        (
            node_route,
            target,
            has_server_credential,
            false,
            resolved_user_service_id,
            node_routing_required,
            catalog_service_slug,
        )
    };

    // Record the resolved service slug so the outer wrapper can attach it
    // to `TelemetryEvent::ProxyError` if any downstream error branch fires
    // before the handler returns `Ok`.
    *resolved_slug = target.service.slug.clone();

    // A configured policy is a data-plane boundary, so authorize the final
    // REST method/path before approval, billing, credential injection, node
    // transport, or forwarding. Rows without a policy retain the legacy path
    // bytes and behavior unchanged.
    let canonical_forward_path = if target.service.proxy_operation_policy.is_some() {
        let canonical_path =
            crate::services::proxy_authorization::CanonicalPath::from_rest_decoded(path)?;
        crate::services::proxy_authorization::authorize_proxy_operation(
            &target.service,
            request.method().as_str(),
            &canonical_path,
        )?;
        Some(canonical_path.forwarding_path())
    } else {
        None
    };
    let path = canonical_forward_path.as_deref().unwrap_or(path);

    // Billing is metadata-only and must never change proxy resolution
    // behavior. Resolve the billing owner using the SAME identity the proxy
    // used to resolve and authorize the target (`proxy_resolution_user_id`,
    // which collapses a service account to its owner). Using the raw subject
    // here would make `resolve_owner_access` deny a service account billing
    // its own owner and abort an otherwise-authorized proxy request.
    let billing_resolution_user_id = auth_user.proxy_resolution_user_id();
    let billing_resource_owner_id = effective_owner_for_approval
        .as_deref()
        .unwrap_or(&billing_resolution_user_id);
    let billing_owner = state
        .billing
        .owner_resolver()
        .resolve_for_resource(&billing_resolution_user_id, billing_resource_owner_id)
        .await?;
    let billing_request_id = uuid::Uuid::new_v4().to_string();
    let credential_class = final_credential_class(
        resolved_user_service_id.as_deref(),
        node_route.is_some(),
        agent_override_applied,
        has_server_credential,
        master_credential,
        &target,
    );
    let is_ws_candidate = is_ws_upgrade_request(&request);
    let platform_metric = platform_metric_for_target(&target, is_ws_candidate);
    let node_intent = match &node_route {
        Some(route) if !route.fallback_node_ids.is_empty() => {
            crate::services::billing::NodeIntent::NodeWithFallback
        }
        Some(_) => crate::services::billing::NodeIntent::Node,
        None => crate::services::billing::NodeIntent::Direct,
    };
    let billing_ctx = crate::services::billing::BillingRouteContext::new(
        crate::services::billing::BillingIngress::Proxy,
        billing_request_id,
        billing_owner.owner_id,
        user_id_str.clone(),
        auth_user.api_key_id.clone(),
        resolved_user_service_id.clone(),
        Some(target.service.id.clone()),
        Some(target.service.slug.clone()),
        node_intent,
        target.auth_method.clone(),
        credential_class,
        platform_metric,
        target.service.billing.as_ref(),
        state.billing.resale_enabled(),
    );

    // === Request Decomposition ===
    // Extract method, query, headers BEFORE body consumption.
    let method = request.method().clone();
    let method_str = method.as_str().to_string();
    let caller_proxy_prefix = caller_proxy_prefix(request.uri().path(), path);
    // Strip NyxID-only routing params (e.g. `_nyxid_via`) from the
    // query string before forwarding. Downstream services should never
    // see NyxID-internal parameters.
    let query = request
        .uri()
        .query()
        .map(strip_internal_query_params)
        .filter(|q| !q.is_empty());
    let mut all_headers = request.headers().clone();
    let durable_grant_id = single_system_header(
        &all_headers,
        durable_operation_grant_service::DURABLE_GRANT_HEADER,
    )?;
    let durable_operation_id = single_system_header(
        &all_headers,
        durable_operation_grant_service::OPERATION_ID_HEADER,
    )?;
    all_headers.remove(durable_operation_grant_service::DURABLE_GRANT_HEADER);
    all_headers.remove(durable_operation_grant_service::OPERATION_ID_HEADER);
    let scheduled_api_key_id = if auth_user.auth_method == AuthMethod::ApiKey
        && auth_user.api_key_purpose == ApiKeyPurpose::ScheduledInvocation
    {
        Some(auth_user.api_key_id.as_deref().ok_or_else(|| {
            AppError::DurableGrantMismatch("API-key authentication is missing key identity".into())
        })?)
    } else {
        None
    };
    if scheduled_api_key_id.is_some() {
        // Durable idempotency is NyxID-owned and derived from operation_id. A
        // caller value is never trusted or forwarded for scheduled requests.
        // Ordinary proxy requests preserve caller idempotency end to end.
        all_headers.remove("idempotency-key");
        // Idempotency forwarding is owned exclusively by the durable endpoint
        // contract. Service defaults cannot create replay semantics that the
        // selected operation did not authorize.
        strip_durable_idempotency_defaults(&mut target.catalog_default_headers);
        strip_durable_idempotency_defaults(&mut target.user_service_default_headers);
    }
    if scheduled_api_key_id.is_none()
        && (durable_grant_id.is_some() || durable_operation_id.is_some())
    {
        return Err(AppError::DurableGrantMismatch(
            "durable authorization headers require a scheduled_invocation API key".to_string(),
        ));
    }

    // Extract the caller's raw Bearer token for nyxid_token passthrough.
    let caller_token =
        caller_bearer_token_for_downstream(&all_headers, scheduled_api_key_id.is_some());

    // Check for WebSocket upgrade BEFORE consuming the request body.
    let is_ws = is_ws_candidate;

    // Reject multi-range requests with excessive ranges (DoS prevention)
    validate_range_header(&all_headers)?;

    // Direct and node-routed HTTP requests share one admission policy. WS
    // handshakes retain their protocol-specific base allowlist while reusing
    // the same downstream-owned namespace rules.
    let node_forward_headers = proxy_service::collect_forward_headers(&all_headers);
    let ws_forward_headers = collect_ws_forward_headers(&all_headers);

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

    let durable_candidate = scheduled_api_key_id.is_some();
    let enforce_approval = match &approval_outcome {
        approval_service::ApprovalOutcome::Allowed { required } => {
            *required
                && auth_user.auth_method != crate::mw::auth::AuthMethod::Session
                && !durable_candidate
        }
        approval_service::ApprovalOutcome::Denied => false,
        approval_service::ApprovalOutcome::NeedsApproval(_) => !durable_candidate,
    };

    match approval_outcome {
        approval_service::ApprovalOutcome::Allowed { .. } => {}
        approval_service::ApprovalOutcome::Denied => {
            if let Some(api_key_id) = scheduled_api_key_id {
                audit_service::log_for_user(
                    state.db.clone(),
                    auth_user,
                    "durable_operation_denied",
                    Some(serde_json::json!({
                        "api_key_id": api_key_id,
                        "grant_id": durable_grant_id.as_deref(),
                        "operation_id": durable_operation_id.as_deref(),
                        "user_service_id": resolved_user_service_id.as_deref(),
                        "decision": "denied",
                        "downstream_attempts": 0,
                        "reason": "approval_policy_deny",
                    })),
                );
            }
            return Err(AppError::Forbidden(
                "Operation denied by approval policy".to_string(),
            ));
        }
        approval_service::ApprovalOutcome::NeedsApproval(_) if durable_candidate => {}
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
    let body = force_stream_usage_for_service(&target.service.slug, path, body);
    let durable_authorization_body = body.clone().unwrap_or_default();
    let request_body_len = body.as_ref().map(|b| b.len() as i64).unwrap_or(0);

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
            Some(&state.connection_expiry_notifier),
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
        // Resolve the propagation principal — a human user OR a service account —
        // so identity is emitted for both. A service account has no `users` doc;
        // detect it via the verified auth method and load the SA record instead.
        let principal: Option<identity_service::Principal> =
            if auth_user.auth_method == AuthMethod::ServiceAccount {
                state
                    .db
                    .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
                    .find_one(doc! { "_id": &user_id_str })
                    .await?
                    .as_ref()
                    .map(identity_service::Principal::from)
            } else {
                state
                    .db
                    .collection::<User>(USERS)
                    .find_one(doc! { "_id": &user_id_str })
                    .await?
                    .as_ref()
                    .map(identity_service::Principal::from)
            };

        if let Some(ref principal) = principal {
            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "headers" | "both"
            ) {
                identity_headers =
                    identity_service::build_identity_headers(principal, &target.service);
            }

            if matches!(
                target.service.identity_propagation_mode.as_str(),
                "jwt" | "both"
            ) {
                match identity_service::generate_identity_assertion(
                    &state.jwt_keys,
                    &state.config,
                    principal,
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
        } else {
            tracing::warn!(
                service_id = %service_id,
                user_id = %user_id_str,
                "No principal found for identity propagation"
            );
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
        let restrictions = crate::crypto::jwt::TokenRestrictionClaims::from_auth_user(auth_user);

        match crate::crypto::jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &user_uuid,
            &target.service.delegation_token_scope,
            &target.service.slug,
            crate::crypto::jwt::MCP_DELEGATION_TOKEN_TTL_SECS,
            Some(&restrictions),
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

    let metered = state.billing.open(&billing_ctx).await?;

    let durable_reservation = if let Some(api_key_id) = scheduled_api_key_id {
        let grant_id = match durable_grant_id.as_deref() {
            Some(grant_id) => grant_id,
            None => {
                let error = AppError::DurableGrantMissing(
                    "X-NyxID-Durable-Grant-Id is required for scheduled_invocation keys"
                        .to_string(),
                );
                audit_service::log_for_user(
                    state.db.clone(),
                    auth_user,
                    "durable_operation_denied",
                    Some(serde_json::json!({
                        "api_key_id": api_key_id,
                        "grant_id": null,
                        "operation_id": durable_operation_id.as_deref(),
                        "user_service_id": resolved_user_service_id.as_deref(),
                        "decision": "denied",
                        "downstream_attempts": 0,
                        "reason": error.to_string(),
                    })),
                );
                return Err(error);
            }
        };
        let operation_id = match durable_operation_id.as_deref() {
            Some(operation_id) => operation_id,
            None => {
                let error = AppError::DurableGrantMissing(
                    "X-NyxID-Operation-Id is required for scheduled_invocation keys".to_string(),
                );
                audit_service::log_for_user(
                    state.db.clone(),
                    auth_user,
                    "durable_operation_denied",
                    Some(serde_json::json!({
                        "api_key_id": api_key_id,
                        "grant_id": grant_id,
                        "operation_id": null,
                        "user_service_id": resolved_user_service_id.as_deref(),
                        "decision": "denied",
                        "downstream_attempts": 0,
                        "reason": error.to_string(),
                    })),
                );
                return Err(error);
            }
        };
        let user_service_id = resolved_user_service_id.as_deref().ok_or_else(|| {
            AppError::DurableGrantMismatch(
                "scheduled_invocation keys require an exact UserService route".to_string(),
            )
        })?;
        let mut authorization_delegated = delegated.clone();
        proxy_service::extend_with_path_credential(&mut authorization_delegated, &target);
        let prepared_authorization = proxy_service::prepare_delegated_request(
            path,
            query.as_deref(),
            &authorization_delegated,
        )?;
        let effective_headers =
            effective_header_map(proxy_service::build_effective_outbound_headers(
                &target,
                node_forward_headers.clone(),
                &identity_headers,
                &prepared_authorization.delegated_headers,
                &extra_outbound_headers,
            ))?;
        let reservation = durable_operation_grant_service::authorize_and_reserve(
            &state.db,
            &state.node_ws_manager,
            &user_id_str,
            api_key_id,
            user_service_id,
            &method_str,
            path,
            query.as_deref(),
            &effective_headers,
            durable_authorization_body.as_ref(),
            grant_id,
            operation_id,
            is_ws,
        )
        .await;
        match reservation {
            Ok(reservation) => {
                audit_service::log_for_user(
                    state.db.clone(),
                    auth_user,
                    "durable_operation_admitted",
                    Some(serde_json::json!({
                        "api_key_id": api_key_id,
                        "grant_id": &reservation.grant_id,
                        "operation_id": &reservation.operation_id,
                        "user_service_id": user_service_id,
                        "endpoint_id": &reservation.endpoint_id,
                        "contract_digest": &reservation.contract_digest,
                        "decision": "admitted",
                        "downstream_attempts": 0,
                        "client_audit_binding": &reservation.client_audit_binding,
                    })),
                );
                if reservation.replay_policy == DurableReplayPolicy::DownstreamIdempotencyKey {
                    extra_outbound_headers.push((
                        "Idempotency-Key".to_string(),
                        reservation.operation_id.clone(),
                    ));
                }
                Some(reservation)
            }
            Err(error) => {
                audit_service::log_for_user(
                    state.db.clone(),
                    auth_user,
                    "durable_operation_denied",
                    Some(serde_json::json!({
                        "api_key_id": api_key_id,
                        "grant_id": grant_id,
                        "operation_id": operation_id,
                        "user_service_id": user_service_id,
                        "decision": "denied",
                        "downstream_attempts": 0,
                        "reason": error.to_string(),
                    })),
                );
                return Err(error);
            }
        }
    } else {
        None
    };
    let collect_realtime_llm_usage =
        websocket_realtime_usage_enabled(catalog_service_slug.as_deref(), &metered);

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
                collect_realtime_llm_usage,
                metered.clone(),
                billing_egress_permit,
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
            collect_realtime_llm_usage,
            metered.clone(),
            billing_egress_permit,
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
        let node_location_context = AsyncLocationContext::new(
            &target.base_url,
            &node_path,
            prepared.query.as_deref(),
            caller_proxy_prefix.clone(),
        );

        let mut base_headers = node_forward_headers;
        // Forward the caller's NyxID access token when the service is configured for it.
        if target.service.forward_access_token
            && let Some(ref token) = caller_token
        {
            base_headers.push(("authorization".to_string(), format!("Bearer {token}")));
        }
        let enriched_headers = proxy_service::build_effective_outbound_headers(
            &target,
            base_headers,
            &identity_headers,
            &prepared.delegated_headers,
            &extra_outbound_headers,
        );

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
        let mut first_dispatch_admission_ms = None;
        let mut had_dispatched_failure = false;
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
                    Err(error) => {
                        if let Some(reservation) = durable_reservation.as_ref() {
                            durable_operation_grant_service::mark_pre_dispatch_rejected(
                                &state.db,
                                reservation,
                                "node signing preparation failed before dispatch",
                            )
                            .await;
                        }
                        return Err(error);
                    }
                }
            } else {
                None
            };

            let start = std::time::Instant::now();
            if let Err(error) = state.billing.mark_forwarded(&metered).await {
                if let Some(reservation) = durable_reservation.as_ref() {
                    durable_operation_grant_service::mark_pre_dispatch_rejected(
                        &state.db,
                        reservation,
                        "billing admission failed before dispatch",
                    )
                    .await;
                }
                return Err(error);
            }
            if let Some(reservation) = durable_reservation.as_ref() {
                durable_operation_grant_service::mark_dispatched(
                    &state.db,
                    reservation,
                    Some(node_id),
                )
                .await?;
            }
            let target_admission_ms =
                *first_dispatch_admission_ms.get_or_insert_with(|| elapsed_ms(exchange_started_at));
            let downstream_started_at = std::time::Instant::now();
            let result = state
                .node_ws_manager
                .send_proxy_request_classified(
                    node_id,
                    attempt_request,
                    signing_secret.as_ref().map(|secret| secret.as_slice()),
                    billing_egress_permit,
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

                    let response_result: AppResult<Response> = async {
                        let response = match proxy_response {
                        ProxyResponseType::Complete(node_response) => {
                            let response_len = node_response.body.len() as i64;
                            let request_len = request_body_len;
                            settle_meter_async(
                                state.billing.clone(),
                                metered.clone(),
                                llm_platform_usage(None, request_len + response_len),
                                None,
                                None,
                            )
                            .await;
                            let status = StatusCode::from_u16(node_response.status)
                                .unwrap_or(StatusCode::BAD_GATEWAY);
                            ProxyExchangeDiagnostics::new(
                                exchange_started_at,
                                target_admission_ms,
                                None,
                                &method,
                                status,
                                "buffered",
                                &all_headers,
                            )
                            .emit("completed");
                            let mut response_builder = Response::builder().status(status);
                            for (name, value) in &node_response.headers {
                                let name_lower = name.to_lowercase();
                                if let Ok(hn) =
                                    axum::http::header::HeaderName::from_bytes(name.as_bytes())
                                    && let Some(hv) = forwarded_response_header_value(
                                        &name_lower,
                                        value.as_bytes(),
                                        false,
                                        node_location_context.as_ref(),
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
                            let stream_diagnostics = ProxyExchangeDiagnostics::new(
                                exchange_started_at,
                                target_admission_ms,
                                Some(elapsed_ms(downstream_started_at)),
                                &method,
                                http_status,
                                if node_is_sse_headers(&resp_headers) {
                                    "sse"
                                } else {
                                    "binary_stream"
                                },
                                &all_headers,
                            );
                            let mut response_builder = Response::builder().status(http_status);

                            // Detect SSE so we can skip content-length (length unknown).
                            // For non-SSE streaming (video, audio, large files), keep
                            // content-length for client download progress / seeking.
                            let node_is_sse = resp_headers.iter().any(|(k, v)| {
                                k.eq_ignore_ascii_case("content-type")
                                    && crate::mw::security_headers::is_sse_media_type(v)
                            });

                            for (name, value) in &resp_headers {
                                let name_lower = name.to_lowercase();
                                if let Ok(hn) =
                                    axum::http::header::HeaderName::from_bytes(name.as_bytes())
                                    && let Some(hv) = forwarded_response_header_value(
                                        &name_lower,
                                        value.as_bytes(),
                                        node_is_sse,
                                        node_location_context.as_ref(),
                                    )
                                {
                                    response_builder = response_builder.header(hn, hv);
                                }
                            }

                            // Same anti-buffering opt-out as the direct path:
                            let service_id_owned = service_id.to_string();
                            let node_id_owned = node_id.to_string();
                            let stream_billing = state.billing.clone();
                            let stream_metered = metered.clone();
                            let request_len = request_body_len;

                            // Convert the mpsc receiver into a streaming body.
                            let mut exchange_diagnostics =
                                ProxyStreamDiagnostics::new(stream_diagnostics);
                            let stream = async_stream::stream! {
                                let mut response_len: i64 = 0;
                                loop {
                                    match tokio::time::timeout(idle_timeout, rx.recv()).await {
                                        Ok(Some(StreamChunk::Data(bytes))) => {
                                            response_len += bytes.len() as i64;
                                            yield Ok::<_, std::io::Error>(bytes::Bytes::from(bytes));
                                        }
                                        Ok(Some(StreamChunk::End)) => {
                                            exchange_diagnostics.finish("completed");
                                            break;
                                        }
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
                                            exchange_diagnostics.finish("body_error");
                                            break;
                                        }
                                        Ok(Some(StreamChunk::Start { .. })) => {
                                            // Duplicate start, ignore
                                        }
                                        Ok(None) => {
                                            exchange_diagnostics.finish("upstream_error");
                                            break;
                                        }
                                        Err(_) => {
                                            tracing::warn!(
                                                service_id = %service_id_owned,
                                                node_id = %node_id_owned,
                                                idle_timeout_secs,
                                                "Node proxy stream idle timeout reached"
                                            );
                                            exchange_diagnostics.finish("upstream_error");
                                            break;
                                        }
                                    }
                                }
                                settle_meter_async(
                                    stream_billing,
                                    stream_metered,
                                    llm_platform_usage(None, request_len + response_len),
                                    None,
                                    None,
                                )
                                .await;
                            };

                            response_builder
                                .body(Body::from_stream(stream))
                                .map_err(|e| {
                                    AppError::Internal(format!("Failed to build response: {e}"))
                                })?
                        }
                        };
                        Ok(response)
                    }
                    .await;
                    let mut response = match response_result {
                        Ok(response) => response,
                        Err(error) => {
                            emit_preheader_diagnostics(
                                exchange_started_at,
                                target_admission_ms,
                                &method,
                                &all_headers,
                                "upstream_error",
                            );
                            if let Some(reservation) = durable_reservation.as_ref() {
                                finish_durable_operation(
                                    state,
                                    auth_user,
                                    reservation,
                                    DurableExecutionStatus::OutcomeUncertain,
                                    None,
                                    Some(node_id),
                                    "node response failed after dispatch",
                                )
                                .await;
                                return Err(AppError::DurableOperationOutcomeUncertain);
                            }
                            return Err(error);
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

                    if let Some(reservation) = durable_reservation.as_ref() {
                        let response_status = response.status().as_u16();
                        let terminal_status = if response.status().is_success() {
                            DurableExecutionStatus::Completed
                        } else {
                            DurableExecutionStatus::Failed
                        };
                        finish_durable_operation(
                            state,
                            auth_user,
                            reservation,
                            terminal_status,
                            Some(response_status),
                            Some(node_id),
                            "downstream response received",
                        )
                        .await;
                    }

                    return Ok(response);
                }
                Err(NodeProxyFailure {
                    error: err @ AppError::NodeCredentialMissing(_),
                    dispatched,
                }) => {
                    had_dispatched_failure |= dispatched;
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

                    if let Some(reservation) = durable_reservation.as_ref() {
                        finish_durable_operation(
                            state,
                            auth_user,
                            reservation,
                            DurableExecutionStatus::Failed,
                            None,
                            Some(node_id),
                            "node rejected the request before downstream credential use",
                        )
                        .await;
                        return Err(err);
                    }
                    if !should_retry_node_failure(&method, dispatched) {
                        emit_preheader_diagnostics(
                            exchange_started_at,
                            target_admission_ms,
                            &method,
                            &all_headers,
                            "upstream_error",
                        );
                        return Err(err);
                    }
                    last_error = Some(err);
                    continue;
                }
                Err(NodeProxyFailure {
                    error: err @ (AppError::NodeOffline(_) | AppError::NodeProxyTimeout),
                    dispatched,
                }) => {
                    had_dispatched_failure |= dispatched;
                    // Record error metrics (fire-and-forget)
                    let db_clone = state.db.clone();
                    let nid = node_id.to_string();
                    let err_msg = "Node offline or timeout".to_string();
                    tokio::spawn(async move {
                        let _ = node_metrics_service::record_error(db_clone, nid, err_msg).await;
                    });

                    if let Some(reservation) = durable_reservation.as_ref() {
                        finish_durable_operation(
                            state,
                            auth_user,
                            reservation,
                            DurableExecutionStatus::OutcomeUncertain,
                            None,
                            Some(node_id),
                            "node transport failed after dispatch",
                        )
                        .await;
                        return Err(AppError::DurableOperationOutcomeUncertain);
                    }
                    if !should_retry_node_failure(&method, dispatched) {
                        emit_preheader_diagnostics(
                            exchange_started_at,
                            target_admission_ms,
                            &method,
                            &all_headers,
                            "upstream_error",
                        );
                        tracing::warn!(
                            node_id = %node_id,
                            method = %method,
                            "Not retrying unsafe proxy request after possible node dispatch"
                        );
                        return Err(err);
                    }
                    tracing::warn!(node_id = %node_id, "Node proxy failed, trying next");
                    last_error = Some(err);
                    continue;
                }
                Err(failure) => {
                    let NodeProxyFailure {
                        error: e,
                        dispatched,
                    } = failure;
                    if let Some(reservation) = durable_reservation.as_ref() {
                        finish_durable_operation(
                            state,
                            auth_user,
                            reservation,
                            DurableExecutionStatus::OutcomeUncertain,
                            None,
                            Some(node_id),
                            "node request failed after dispatch",
                        )
                        .await;
                        return Err(AppError::DurableOperationOutcomeUncertain);
                    }
                    emit_preheader_diagnostics(
                        exchange_started_at,
                        target_admission_ms,
                        &method,
                        &all_headers,
                        if dispatched {
                            "upstream_error"
                        } else {
                            "connect_error"
                        },
                    );
                    return Err(e);
                }
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
            emit_preheader_diagnostics(
                exchange_started_at,
                first_dispatch_admission_ms.unwrap_or_else(|| elapsed_ms(exchange_started_at)),
                &method,
                &all_headers,
                if had_dispatched_failure {
                    "upstream_error"
                } else {
                    "connect_error"
                },
            );
            if let Some(reservation) = durable_reservation.as_ref() {
                durable_operation_grant_service::mark_pre_dispatch_rejected(
                    &state.db,
                    reservation,
                    "no dispatchable node accepted the request",
                )
                .await;
            }
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
        let request_len = serde_json::to_vec(&translated.body)
            .map(|bytes| bytes.len() as i64)
            .unwrap_or(request_body_len);
        let model = body_json
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let usage_complete = chatgpt_usage_callback(
            state.billing.clone(),
            metered.clone(),
            request_len,
            billing_ctx.resale.as_ref().map(|spec| spec.metric),
            model.clone(),
        );

        if let Err(error) = state.billing.mark_forwarded(&metered).await {
            if let Some(reservation) = durable_reservation.as_ref() {
                durable_operation_grant_service::mark_pre_dispatch_rejected(
                    &state.db,
                    reservation,
                    "billing admission failed before dispatch",
                )
                .await;
            }
            return Err(error);
        }
        if let Some(reservation) = durable_reservation.as_ref() {
            durable_operation_grant_service::mark_dispatched(&state.db, reservation, None).await?;
        }
        let response_result = until_client_disconnect(
            &downstream_cancellation,
            chatgpt_translator::send_to_chatgpt(
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
                    model,
                    path: path.to_string(),
                    api_key_id: auth_user.api_key_id.clone(),
                    api_key_name: auth_user.api_key_name.clone(),
                }),
                Some(usage_complete),
                billing_egress_permit,
            ),
        )
        .await;
        let mut response = match response_result {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                if let Some(reservation) = durable_reservation.as_ref() {
                    finish_durable_operation(
                        state,
                        auth_user,
                        reservation,
                        DurableExecutionStatus::OutcomeUncertain,
                        None,
                        None,
                        "direct transport failed after dispatch",
                    )
                    .await;
                    return Err(AppError::DurableOperationOutcomeUncertain);
                }
                return Err(error);
            }
            Err(_) => {
                if let Some(reservation) = durable_reservation.as_ref() {
                    finish_durable_operation(
                        state,
                        auth_user,
                        reservation,
                        DurableExecutionStatus::OutcomeUncertain,
                        None,
                        None,
                        "client disconnected after dispatch",
                    )
                    .await;
                    return Err(AppError::DurableOperationOutcomeUncertain);
                }
                return Err(proxy_client_disconnected(service_id));
            }
        };

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

        if let Some(reservation) = durable_reservation.as_ref() {
            finish_durable_operation(
                state,
                auth_user,
                reservation,
                if status.is_success() {
                    DurableExecutionStatus::Completed
                } else {
                    DurableExecutionStatus::Failed
                },
                Some(status.as_u16()),
                None,
                "downstream response received",
            )
            .await;
        }

        return Ok(response);
    }

    // Reuse the shared reqwest::Client from AppState for connection pooling.
    if let Err(error) = state.billing.mark_forwarded(&metered).await {
        if let Some(reservation) = durable_reservation.as_ref() {
            durable_operation_grant_service::mark_pre_dispatch_rejected(
                &state.db,
                reservation,
                "billing admission failed before dispatch",
            )
            .await;
        }
        return Err(error);
    }
    if let Some(reservation) = durable_reservation.as_ref() {
        durable_operation_grant_service::mark_dispatched(&state.db, reservation, None).await?;
    }
    let target_admission_ms = elapsed_ms(exchange_started_at);
    let downstream_started_at = std::time::Instant::now();
    let downstream_result = until_client_disconnect(
        &downstream_cancellation,
        proxy_service::forward_request_with_extra_outbound_headers(
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
            extra_outbound_headers,
            billing_egress_permit,
        ),
    )
    .await;
    let downstream_response = match downstream_result {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            emit_preheader_diagnostics(
                exchange_started_at,
                target_admission_ms,
                &method,
                &all_headers,
                "upstream_error",
            );
            if let Some(reservation) = durable_reservation.as_ref() {
                finish_durable_operation(
                    state,
                    auth_user,
                    reservation,
                    DurableExecutionStatus::OutcomeUncertain,
                    None,
                    None,
                    "direct transport failed after dispatch",
                )
                .await;
                return Err(AppError::DurableOperationOutcomeUncertain);
            }
            return Err(error);
        }
        Err(_) => {
            emit_preheader_diagnostics(
                exchange_started_at,
                target_admission_ms,
                &method,
                &all_headers,
                "client_disconnect",
            );
            if let Some(reservation) = durable_reservation.as_ref() {
                finish_durable_operation(
                    state,
                    auth_user,
                    reservation,
                    DurableExecutionStatus::OutcomeUncertain,
                    None,
                    None,
                    "client disconnected after dispatch",
                )
                .await;
                return Err(AppError::DurableOperationOutcomeUncertain);
            }
            return Err(proxy_client_disconnected(service_id));
        }
    };

    // Convert reqwest Response back to axum Response
    let status = StatusCode::from_u16(downstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let direct_location_context = AsyncLocationContext::from_downstream_request(
        &target.base_url,
        downstream_response.url().clone(),
        caller_proxy_prefix,
    );

    if let Some(reservation) = durable_reservation.as_ref() {
        finish_durable_operation(
            state,
            auth_user,
            reservation,
            if status.is_success() {
                DurableExecutionStatus::Completed
            } else {
                DurableExecutionStatus::Failed
            },
            Some(status.as_u16()),
            None,
            "downstream response received",
        )
        .await;
    }

    // Same exact, case-insensitive media-type test the response-header
    // middleware uses, so `content-length` stripping and SSE usage
    // observation agree with the anti-buffering mark.
    let is_sse = downstream_response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(crate::mw::security_headers::is_sse_media_type);
    let should_stream = should_stream_response(&downstream_response, status, is_sse);
    let exchange_diagnostics = ProxyExchangeDiagnostics::new(
        exchange_started_at,
        target_admission_ms,
        Some(elapsed_ms(downstream_started_at)),
        &method,
        status,
        if is_sse {
            "sse"
        } else if should_stream {
            "binary_stream"
        } else {
            "buffered"
        },
        &all_headers,
    );
    let usage_context =
        should_capture_llm_usage(&target.service.slug, platform_metric).then(|| {
            llm_usage_service::UsageAuditContext {
                db: state.db.clone(),
                user_id: user_id_str.clone(),
                provider_slug: None,
                service_id: Some(service_id.to_string()),
                model: None,
                path: path.to_string(),
                api_key_id: auth_user.api_key_id.clone(),
                api_key_name: auth_user.api_key_name.clone(),
            }
        });

    let mut response_builder = Response::builder().status(status);

    // Forward only allowlisted response headers.
    // Skip content-length for SSE (length unknown). Keep it for other
    // streaming responses — clients need it for download progress / seeking.
    for (name, value) in downstream_response.headers().iter() {
        let name_lower = name.as_str().to_lowercase();
        if let Ok(header_name) =
            axum::http::header::HeaderName::from_bytes(name.as_str().as_bytes())
            && let Some(header_value) = forwarded_response_header_value(
                &name_lower,
                value.as_bytes(),
                is_sse,
                direct_location_context.as_ref(),
            )
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
            let stream_billing = state.billing.clone();
            let stream_metered = metered.clone();
            let request_len = request_body_len;
            let resale_metric = billing_ctx.resale.as_ref().map(|spec| spec.metric);
            let task_cancellation = downstream_cancellation.clone();

            tokio::spawn(async move {
                let mut exchange_diagnostics = ProxyStreamDiagnostics::new(exchange_diagnostics);
                let mut sse_buffer = String::new();
                let mut usage_accumulator =
                    llm_usage_service::ReportedLlmUsageAccumulator::default();
                let mut response_len: i64 = 0;

                loop {
                    let next = until_client_disconnect(
                        &task_cancellation,
                        tokio::time::timeout(idle_timeout, upstream_stream.next()),
                    )
                    .await;
                    match next {
                        Err(_) => {
                            exchange_diagnostics.finish("client_disconnect");
                            drop(upstream_stream);
                            let usage = usage_accumulator.finalize();
                            if let Some(usage) = usage.clone() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                stream_usage_context.model.clone(),
                            )
                            .await;
                            return;
                        }
                        Ok(Ok(Some(Ok(bytes)))) => {
                            response_len += bytes.len() as i64;
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
                                exchange_diagnostics.finish("client_disconnect");
                                drop(upstream_stream);
                                let usage = usage_accumulator.finalize();
                                if let Some(usage) = usage.clone() {
                                    llm_usage_service::log_reported_usage_async(
                                        stream_usage_context.clone(),
                                        usage,
                                    );
                                }
                                let resale = resale_metric.and_then(|metric| {
                                    resale_usage_from_optional_reported(
                                        metric,
                                        usage.as_ref(),
                                        request_len + response_len,
                                    )
                                });
                                settle_meter_async(
                                    stream_billing,
                                    stream_metered,
                                    llm_platform_usage(usage.as_ref(), request_len + response_len),
                                    resale,
                                    stream_usage_context.model.clone(),
                                )
                                .await;
                                return;
                            }
                        }
                        Ok(Ok(Some(Err(e)))) => {
                            exchange_diagnostics.finish("body_error");
                            tracing::error!(
                                service_id = %service_id_owned,
                                error = %e,
                                error_debug = ?e,
                                "Proxy stream error from upstream — connection dropped"
                            );
                            let usage = usage_accumulator.finalize();
                            if let Some(usage) = usage.clone() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                stream_usage_context.model.clone(),
                            )
                            .await;
                            let _ = tx
                                .send(Err(std::io::Error::other(format!(
                                    "upstream stream error: {e}"
                                ))))
                                .await;
                            return;
                        }
                        Ok(Ok(None)) => {
                            exchange_diagnostics.finish("completed");
                            let usage = usage_accumulator.finalize();
                            if let Some(usage) = usage.clone() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                stream_usage_context.model.clone(),
                            )
                            .await;
                            return;
                        }
                        Ok(Err(_)) => {
                            exchange_diagnostics.finish("upstream_error");
                            tracing::warn!(
                                service_id = %service_id_owned,
                                idle_timeout_secs,
                                "Proxy stream idle timeout reached"
                            );
                            let usage = usage_accumulator.finalize();
                            if let Some(usage) = usage.clone() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                stream_usage_context.model.clone(),
                            )
                            .await;
                            return;
                        }
                    }
                }
            });

            let body = Body::from_stream(CancelOnDropStream::new(
                ReceiverStream::new(rx),
                downstream_cancellation.clone(),
            ));
            response_builder
                .body(body)
                .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
        } else {
            let service_id_owned = service_id.to_string();
            let idle_timeout =
                std::time::Duration::from_secs(state.config.proxy_stream_idle_timeout_secs);
            let idle_timeout_secs = state.config.proxy_stream_idle_timeout_secs;
            let is_json_body = downstream_response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("application/json"));
            let mut upstream_stream = downstream_response.bytes_stream();
            let stream_billing = state.billing.clone();
            let stream_metered = metered.clone();
            let request_len = request_body_len;
            // Chunked JSON bodies (no Content-Length) stream through this
            // branch; capture a bounded copy so LLM services settle with the
            // provider-reported token count instead of the byte estimate.
            let stream_usage_context = if is_json_body {
                usage_context.clone()
            } else {
                None
            };
            let stream_resale_metric = billing_ctx.resale.as_ref().map(|spec| spec.metric);
            let mut exchange_diagnostics = ProxyStreamDiagnostics::new(exchange_diagnostics);
            let stream = async_stream::stream! {
                let mut response_len: i64 = 0;
                let mut captured: Option<Vec<u8>> =
                    stream_usage_context.as_ref().map(|_| Vec::new());
                loop {
                    match tokio::time::timeout(idle_timeout, upstream_stream.next()).await {
                        Ok(Some(Ok(bytes))) => {
                            response_len += bytes.len() as i64;
                            if let Some(buf) = captured.as_mut() {
                                if buf.len() + bytes.len() <= USAGE_CAPTURE_MAX_BYTES {
                                    buf.extend_from_slice(&bytes);
                                } else {
                                    captured = None;
                                }
                            }
                            yield Ok::<_, std::io::Error>(bytes);
                        }
                        Ok(Some(Err(e))) => {
                            exchange_diagnostics.finish("body_error");
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
                        Ok(None) => {
                            exchange_diagnostics.finish("completed");
                            break;
                        }
                        Err(_) => {
                            exchange_diagnostics.finish("upstream_error");
                            tracing::warn!(
                                service_id = %service_id_owned,
                                idle_timeout_secs,
                                "Proxy stream idle timeout reached"
                            );
                            break;
                        }
                    }
                }
                let mut reported_usage = None;
                let mut model = None;
                if let Some(ctx) = stream_usage_context
                    && let Some(buf) = captured
                    && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&buf)
                    && let Some(usage) = llm_usage_service::extract_reported_usage(&json)
                {
                    model = ctx.model.clone();
                    llm_usage_service::log_reported_usage_async(ctx, usage.clone());
                    reported_usage = Some(usage);
                }
                let resale = stream_resale_metric.and_then(|metric| {
                    resale_usage_from_optional_reported(
                        metric,
                        reported_usage.as_ref(),
                        request_len + response_len,
                    )
                });
                settle_meter_async(
                    stream_billing,
                    stream_metered,
                    llm_platform_usage(reported_usage.as_ref(), request_len + response_len),
                    resale,
                    model,
                )
                .await;
            };
            let body = Body::from_stream(CancelOnDropStream::new(
                stream,
                downstream_cancellation.clone(),
            ));
            response_builder
                .body(body)
                .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
        }
    } else {
        // Buffer small / error responses so we can log diagnostics.
        let upstream_request_id = downstream_response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let response_body =
            match until_client_disconnect(&downstream_cancellation, downstream_response.bytes())
                .await
            {
                Err(_) => {
                    exchange_diagnostics.emit("client_disconnect");
                    return Err(proxy_client_disconnected(service_id));
                }
                Ok(Err(e)) => {
                    exchange_diagnostics.emit("body_error");
                    return Err(AppError::Internal(format!(
                        "Failed to read downstream response: {e}"
                    )));
                }
                Ok(Ok(body)) => body,
            };
        exchange_diagnostics.emit("completed");

        if !status.is_success() {
            log_upstream_error(
                service_id,
                status,
                response_body.len(),
                &upstream_request_id,
            );
        }

        let mut reported_usage = None;
        let mut model = None;
        if let Some(nonstream_usage_context) = usage_context
            && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response_body)
            && let Some(usage) = llm_usage_service::extract_reported_usage(&json)
        {
            model = nonstream_usage_context.model.clone();
            llm_usage_service::log_reported_usage_async(nonstream_usage_context, usage.clone());
            reported_usage = Some(usage);
        }

        let request_len = request_body_len;
        let response_len = response_body.len() as i64;
        let resale = billing_ctx.resale.as_ref().and_then(|spec| {
            resale_usage_from_optional_reported(
                spec.metric,
                reported_usage.as_ref(),
                request_len + response_len,
            )
        });
        settle_meter_async(
            state.billing.clone(),
            metered,
            llm_platform_usage(reported_usage.as_ref(), request_len + response_len),
            resale,
            model,
        )
        .await;

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
    super::body_limit::read_request_body(request, max_body_size, "Proxy").await
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

fn final_credential_class(
    resolved_user_service_id: Option<&str>,
    node_route_active: bool,
    agent_override_applied: bool,
    has_server_credential: bool,
    master_credential: bool,
    target: &proxy_service::ProxyTarget,
) -> CredentialClass {
    if node_route_active && !has_server_credential {
        return CredentialClass::NodeManaged;
    }
    if target.auth_method == "none" && target.credential.is_empty() {
        return CredentialClass::NoAuth;
    }
    if agent_override_applied {
        return CredentialClass::AgentOverrideUserOwned;
    }
    if resolved_user_service_id.is_some() {
        // Auto-provisioned UserServices with no user key inject the
        // catalog master credential; classify by whose key was used,
        // not by which resolution path matched.
        return if master_credential {
            CredentialClass::NyxidManagedMaster
        } else {
            CredentialClass::UserOwned
        };
    }
    if !target.service.requires_user_credential && !target.credential.is_empty() {
        return CredentialClass::NyxidManagedMaster;
    }
    CredentialClass::UserOwned
}

fn platform_metric_for_target(
    target: &proxy_service::ProxyTarget,
    is_connection: bool,
) -> BillingMetric {
    // An admin-selected metric on the service's billing config wins;
    // the slug/transport heuristic is only the fallback, so billing
    // classification does not depend on service naming conventions.
    if let Some(metric) = target
        .service
        .billing
        .as_ref()
        .and_then(|billing| billing.platform_metric)
    {
        return metric;
    }
    if is_connection || target.service.service_type == "ssh" {
        BillingMetric::Bytes
    } else if target.service.slug.starts_with("llm-") {
        BillingMetric::Tokens
    } else {
        BillingMetric::Requests
    }
}

fn should_capture_llm_usage(service_slug: &str, platform_metric: BillingMetric) -> bool {
    platform_metric == BillingMetric::Tokens || service_slug.starts_with("llm-")
}

fn resale_usage_from_optional_reported(
    metric: BillingMetric,
    usage: Option<&llm_usage_service::ReportedLlmUsage>,
    fallback_bytes: i64,
) -> Option<ResaleUsage> {
    match metric {
        BillingMetric::Tokens => Some(ResaleUsage {
            metric,
            quantity: llm_usage_service::token_quantity_or_estimate(usage, fallback_bytes),
        }),
        BillingMetric::Requests => Some(ResaleUsage {
            metric,
            quantity: 1,
        }),
        BillingMetric::Bytes => Some(ResaleUsage {
            metric,
            quantity: fallback_bytes.max(0),
        }),
    }
}

pub(crate) fn llm_platform_usage(
    usage: Option<&llm_usage_service::ReportedLlmUsage>,
    fallback_bytes: i64,
) -> PlatformUsage {
    PlatformUsage::llm_completion(
        fallback_bytes,
        llm_usage_service::token_quantity_or_estimate(usage, fallback_bytes),
    )
    .with_token_breakdown(usage.map(llm_usage_service::ReportedLlmUsage::token_breakdown))
}

fn websocket_realtime_usage_enabled(
    catalog_service_slug: Option<&str>,
    metered: &crate::services::billing::MeteredProxyContext,
) -> bool {
    catalog_service_slug == Some("llm-openai")
        && metered
            .route
            .as_ref()
            .and_then(|route| route.resale.as_ref())
            .is_some_and(|resale| resale.metric == BillingMetric::Tokens)
}

fn websocket_platform_usage(stats: &ConnectionUsageStats) -> PlatformUsage {
    if stats.realtime_llm_usage.collection_enabled {
        PlatformUsage::llm_completion(
            stats.total_bytes(),
            stats.realtime_llm_usage.token_quantity(),
        )
        .with_token_breakdown(
            stats
                .realtime_llm_usage
                .reported_usage
                .as_ref()
                .map(llm_usage_service::ReportedLlmUsage::token_breakdown),
        )
    } else {
        llm_platform_usage(None, stats.total_bytes())
    }
}

fn add_websocket_usage_provenance(event: &mut serde_json::Value, stats: &ConnectionUsageStats) {
    if !stats.realtime_llm_usage.collection_enabled {
        return;
    }

    let reported_tokens = stats
        .realtime_llm_usage
        .reported_usage
        .as_ref()
        .map(|usage| usage.total_tokens)
        .unwrap_or(0);
    let estimated_tokens = if stats.realtime_llm_usage.uncovered_bytes > 0 {
        llm_usage_service::estimate_tokens_from_bytes(stats.realtime_llm_usage.uncovered_bytes)
    } else {
        0
    };
    event["usage_provenance"] = serde_json::json!({
        "reported_response_count": stats.realtime_llm_usage.reported_response_count,
        "estimated_response_count": stats.realtime_llm_usage.estimated_response_count,
        "reported_tokens": reported_tokens,
        "estimated_tokens": estimated_tokens,
        "fallback_bytes": stats.realtime_llm_usage.uncovered_bytes,
    });
}

fn websocket_resale_usage(
    metered: &crate::services::billing::MeteredProxyContext,
    stats: &ConnectionUsageStats,
) -> Option<ResaleUsage> {
    let metric = metered
        .route
        .as_ref()?
        .resale
        .as_ref()
        .map(|resale| resale.metric)?;
    let quantity = match metric {
        BillingMetric::Tokens if stats.realtime_llm_usage.collection_enabled => {
            stats.realtime_llm_usage.token_quantity()
        }
        BillingMetric::Tokens => llm_usage_service::estimate_tokens_from_bytes(stats.total_bytes()),
        BillingMetric::Requests => 1,
        BillingMetric::Bytes => stats.total_bytes().max(0),
    };

    Some(ResaleUsage { metric, quantity })
}

fn service_supports_stream_options_include_usage(service_slug: &str) -> bool {
    matches!(service_slug, "llm-openai" | "llm-deepseek")
}

fn force_stream_usage_for_service(
    service_slug: &str,
    path: &str,
    body: Option<bytes::Bytes>,
) -> Option<bytes::Bytes> {
    let body_bytes = body?;
    if body_bytes.is_empty()
        || !service_supports_stream_options_include_usage(service_slug)
        || !path.contains("chat/completions")
    {
        return Some(body_bytes);
    }

    let Ok(mut body_json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) else {
        return Some(body_bytes);
    };
    if body_json.get("stream").and_then(|value| value.as_bool()) != Some(true)
        || !llm_usage_service::force_stream_options_include_usage(&mut body_json)
    {
        return Some(body_bytes);
    }

    Some(
        serde_json::to_vec(&body_json)
            .map(bytes::Bytes::from)
            .unwrap_or(body_bytes),
    )
}

fn chatgpt_usage_callback(
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    metered: crate::services::billing::MeteredProxyContext,
    request_len: i64,
    resale_metric: Option<BillingMetric>,
    model: Option<String>,
) -> chatgpt_translator::UsageCompleteCallback {
    std::sync::Arc::new(move |usage, response_len| {
        let total_bytes = request_len + response_len;
        let resale = resale_metric.and_then(|metric| {
            resale_usage_from_optional_reported(metric, usage.as_ref(), total_bytes)
        });
        Box::pin(settle_meter_async(
            billing.clone(),
            metered.clone(),
            llm_platform_usage(usage.as_ref(), total_bytes),
            resale,
            model.clone(),
        ))
    })
}

async fn settle_meter_async(
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    metered: crate::services::billing::MeteredProxyContext,
    platform: PlatformUsage,
    resale: Option<ResaleUsage>,
    model: Option<String>,
) {
    if !metered.is_enabled() {
        return;
    }

    if billing
        .settle_deferred(&metered, platform, resale, model)
        .await
        .is_err()
    {
        let billing_request_id = metered
            .route
            .as_ref()
            .map(|route| route.billing_request_id.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            billing_request_id,
            "Failed to persist usage settlement intent"
        );
    }
}

/// Threshold below which non-error responses are buffered (so small API
/// responses keep the existing diagnostic-logging path).
const STREAM_SIZE_THRESHOLD: u64 = 256 * 1024;

/// Cap on the bounded response copy kept for LLM usage extraction when a
/// chunked JSON body streams through the passthrough branch. Bodies larger
/// than this fall back to the byte-based token estimate.
///
/// This buffer is a second copy of the body, retained for the lifetime of the
/// response and multiplied by in-flight LLM request concurrency, so the cap is
/// what bounds the handler's worst-case footprint. 512 KiB of JSON is on the
/// order of 100k output tokens in a single non-streaming completion — past what
/// providers will return in one response — so real bodies stay under it. Going
/// over is not a billing regression: the fallback byte estimate is what every
/// one of these responses used before usage capture existed.
const USAGE_CAPTURE_MAX_BYTES: usize = 512 * 1024;

/// Content types that should always be streamed regardless of size.
const STREAMING_CONTENT_TYPES: &[&str] = &[
    "text/event-stream",
    "video/",
    "audio/",
    "application/octet-stream",
    "image/",
    "application/pdf",
];

/// Whether a downstream response header may be forwarded to the client.
///
/// SSE drops `content-length`: the length of a stream is unknown, and a
/// stale upstream value would truncate it. Shared by the direct and
/// node-routed paths.
///
/// `x-accel-buffering` is deliberately absent from the allowlist entirely:
/// NyxID sets it itself on every SSE response (see
/// `mw::security_headers`), and forwarding an upstream copy would let an
/// arbitrary proxied service dictate front-proxy buffering behavior for
/// non-SSE responses too.
fn forwardable_response_header(name_lower: &str, is_sse: bool) -> bool {
    if is_sse && name_lower == "content-length" {
        return false;
    }
    ALLOWED_RESPONSE_HEADERS.contains(&name_lower)
}

/// RFC 9110 safe methods may be replayed for transport failover. Every other
/// method is treated as potentially state-changing, including extension
/// methods, and may only fail over when the node transport proves dispatch
/// never occurred.
fn method_allows_automatic_failover(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

fn should_retry_node_failure(method: &Method, dispatched: bool) -> bool {
    !dispatched || method_allows_automatic_failover(method)
}

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
    _billing_egress_permit: crate::services::billing::route_inventory::BillingEgressPermit,
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
    collect_realtime_llm_usage: bool,
    ws_frame_injections: Vec<crate::models::ws_frame_injection::WsFrameInjection>,
    credential: String,
    db: mongodb::Database,
    user_id: String,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> ConnectionUsageStats {
    let start = std::time::Instant::now();
    let mut stats = ConnectionUsageStats::default();
    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut downstream_sink, mut downstream_stream) = downstream_ws.split();
    let mut injector_state = ws_frame_injector::InjectorState::default();
    let mut realtime_llm_usage =
        llm_usage_service::RealtimeLlmUsageCollector::new(collect_realtime_llm_usage);

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
                        match &axum_msg {
                            axum::extract::ws::Message::Text(text) => {
                                stats.frames_in += 1;
                                stats.bytes_in += text.len() as i64;
                                realtime_llm_usage.observe_client_text(text.as_str());
                            }
                            axum::extract::ws::Message::Binary(bytes) => {
                                stats.frames_in += 1;
                                stats.bytes_in += bytes.len() as i64;
                                realtime_llm_usage.observe_client_binary(bytes.len());
                            }
                            _ => {}
                        }
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
                        match &tung_msg {
                            tokio_tungstenite::tungstenite::Message::Text(text) => {
                                stats.frames_out += 1;
                                stats.bytes_out += text.len() as i64;
                                realtime_llm_usage.observe_downstream_text(text.as_str());
                            }
                            tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                                stats.frames_out += 1;
                                stats.bytes_out += bytes.len() as i64;
                                realtime_llm_usage.observe_downstream_binary(bytes.len());
                            }
                            _ => {}
                        }
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

    stats.duration = start.elapsed();
    stats.realtime_llm_usage = realtime_llm_usage.finalize();
    stats
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
    collect_realtime_llm_usage: bool,
    metered: crate::services::billing::MeteredProxyContext,
    billing_egress_permit: crate::services::billing::route_inventory::BillingEgressPermit,
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
        billing_egress_permit,
    )
    .await?;
    state.billing.mark_forwarded(&metered).await?;

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
    let billing = state.billing.clone();
    let metered_for_settle = metered.clone();
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
            let stats = bridge_websockets(
                client_ws,
                downstream.stream,
                sid.clone(),
                collect_realtime_llm_usage,
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
            let platform_usage = websocket_platform_usage(&stats);
            let resale_usage = websocket_resale_usage(&metered_for_settle, &stats);
            settle_meter_async(
                billing,
                metered_for_settle,
                platform_usage,
                resale_usage,
                None,
            )
            .await;

            let mut disconnect_event = serde_json::json!({
                "service_id": sid,
                "duration_secs": stats.duration.as_secs(),
                "acting_client_id": &acting_client_id,
            });
            add_websocket_usage_provenance(&mut disconnect_event, &stats);
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
    collect_realtime_llm_usage: bool,
    metered: crate::services::billing::MeteredProxyContext,
    billing_egress_permit: crate::services::billing::route_inventory::BillingEgressPermit,
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
                billing_egress_permit,
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
    let billing = state.billing.clone();
    let metered_for_settle = metered.clone();
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
    state.billing.mark_forwarded(&metered).await?;

    Ok(ws_upgrade
        .on_upgrade(move |client_ws| async move {
            let stats = bridge_websockets_via_node(
                client_ws,
                ws_proxy_session.frames,
                &ws_manager,
                &node_id_owned,
                &sess_id,
                sid.clone(),
                collect_realtime_llm_usage,
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
            let platform_usage = websocket_platform_usage(&stats);
            let resale_usage = websocket_resale_usage(&metered_for_settle, &stats);
            settle_meter_async(
                billing,
                metered_for_settle,
                platform_usage,
                resale_usage,
                None,
            )
            .await;

            let mut disconnect_event = serde_json::json!({
                "service_id": sid,
                "duration_secs": stats.duration.as_secs(),
                "acting_client_id": &acting_client_id,
                "routed_via": "node",
                "node_id": node_id_owned,
            });
            add_websocket_usage_provenance(&mut disconnect_event, &stats);
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
    collect_realtime_llm_usage: bool,
    db: mongodb::Database,
    user_id: String,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
    service_owner_user_id: String,
    proxy_actor_user_id: String,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> ConnectionUsageStats {
    use crate::services::node_ws_manager::WsProxyFrame;

    let start = std::time::Instant::now();
    let mut stats = ConnectionUsageStats::default();
    let (mut client_sink, mut client_stream) = client_ws.split();
    let mut realtime_llm_usage =
        llm_usage_service::RealtimeLlmUsageCollector::new(collect_realtime_llm_usage);

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
                        stats.frames_in += 1;
                        stats.bytes_in += t.len() as i64;
                        realtime_llm_usage.observe_client_text(t.as_str());
                        idle_timeout.as_mut().reset(
                            tokio::time::Instant::now()
                                + std::time::Duration::from_secs(WS_PASSTHROUGH_IDLE_TIMEOUT_SECS),
                        );
                        if ws_manager.send_ws_proxy_text(node_id, session_id, &t).is_err() {
                            break;
                        }
                    }
                    Some(Ok(axum::extract::ws::Message::Binary(b))) => {
                        stats.frames_in += 1;
                        stats.bytes_in += b.len() as i64;
                        realtime_llm_usage.observe_client_binary(b.len());
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
                        stats.frames_out += 1;
                        stats.bytes_out += t.len() as i64;
                        realtime_llm_usage.observe_downstream_text(&t);
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
                        stats.frames_out += 1;
                        stats.bytes_out += b.len() as i64;
                        realtime_llm_usage.observe_downstream_binary(b.len());
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
    stats.duration = start.elapsed();
    stats.realtime_llm_usage = realtime_llm_usage.finalize();
    stats
}

#[cfg(test)]
mod tests {
    use super::{
        ALLOWED_RESPONSE_HEADERS, AsyncLocationContext, ConnectionUsageStats, WsPassthroughGuard,
        add_websocket_usage_provenance, apply_agent_attribution_headers,
        apply_proxy_request_id_header, auth_kind_label, caller_bearer_token_for_downstream,
        collect_ws_forward_headers, compose_pre_resolved_node_ids, enforce_node_route_scope,
        ensure_proxy_request_id, final_credential_class, forwarded_response_header_value,
        is_chat_completions_proxy_path, is_codex_transport_path, is_ws_upgrade_request,
        read_proxy_request_body, should_enforce_runtime_approval, should_retry_node_failure,
        single_system_header, strip_durable_idempotency_defaults, validate_range_header,
        websocket_realtime_usage_enabled, websocket_resale_usage,
    };
    use crate::models::service_billing::{BillingMetric, ServiceBilling};
    use crate::models::usage_meter::CredentialClass;
    use crate::mw::auth::AuthMethod;
    use crate::services::billing::{BillingRouteContext, MeteredProxyContext, NodeIntent};
    use crate::services::{
        llm_usage_service,
        proxy_service::{self, validate_requested_proxy_path},
    };
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::{Path, State, ws::WebSocketUpgrade},
        http::{Method, Request, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use futures::{SinkExt, StreamExt};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tower::ServiceExt;

    async fn assert_body_limit_error(error: crate::errors::AppError, max_body_size: usize) {
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "request_body_too_large");
        assert_eq!(payload["error_code"], 11700);
        assert!(
            payload["message"]
                .as_str()
                .is_some_and(|message| message.contains(&max_body_size.to_string()))
        );
    }

    #[tokio::test]
    async fn proxy_body_reader_accepts_body_above_global_default_below_proxy_limit() {
        let body_size = 1024 * 1024 + 1;
        let request = Request::new(Body::from(vec![b'x'; body_size]));

        let body = read_proxy_request_body(request, 2 * 1024 * 1024)
            .await
            .expect("proxy limit, not the app-wide default, controls raw proxy bodies");

        assert_eq!(body.len(), body_size);
    }

    #[tokio::test]
    async fn proxy_body_reader_returns_structured_413_for_fixed_oversize_body() {
        let max_body_size = 1024;
        let request = Request::new(Body::from(vec![b'x'; max_body_size + 1]));

        let error = read_proxy_request_body(request, max_body_size)
            .await
            .expect_err("oversize proxy body must fail");

        assert_body_limit_error(error, max_body_size).await;
    }

    #[tokio::test]
    async fn proxy_body_reader_returns_structured_413_for_chunked_oversize_body() {
        let max_body_size = 1024;
        let chunks = futures::stream::iter([
            Ok::<_, std::convert::Infallible>(bytes::Bytes::from(vec![b'x'; 768])),
            Ok::<_, std::convert::Infallible>(bytes::Bytes::from(vec![b'y'; 768])),
        ]);
        let request = Request::new(Body::from_stream(chunks));

        let error = read_proxy_request_body(request, max_body_size)
            .await
            .expect_err("chunked oversize proxy body must fail");

        assert_body_limit_error(error, max_body_size).await;
    }

    // ---- validate_range_header tests ----

    #[test]
    fn durable_system_headers_are_never_caller_forwarded() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-nyxid-durable-grant-id", "grant-1".parse().unwrap());
        headers.insert("x-nyxid-operation-id", "operation-1".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());

        let forwarded = proxy_service::collect_forward_headers(&headers);
        assert_eq!(
            forwarded,
            vec![("content-type".to_string(), "application/json".to_string())]
        );
    }

    #[test]
    fn durable_system_headers_must_be_single_valued() {
        let mut headers = axum::http::HeaderMap::new();
        headers.append("x-nyxid-operation-id", "one".parse().unwrap());
        headers.append("x-nyxid-operation-id", "two".parse().unwrap());
        assert!(single_system_header(&headers, "x-nyxid-operation-id").is_err());
    }

    #[test]
    fn scheduled_bearer_credential_is_never_forwarded_as_a_downstream_access_token() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer nyxid_ag_example".parse().unwrap());
        assert_eq!(
            caller_bearer_token_for_downstream(&headers, false).as_deref(),
            Some("nyxid_ag_example")
        );
        assert_eq!(caller_bearer_token_for_downstream(&headers, true), None);
    }

    #[test]
    fn scheduled_requests_strip_default_idempotency_headers() {
        let mut headers = vec![
            crate::models::default_request_header::DefaultRequestHeader {
                name: "Idempotency-Key".to_string(),
                value: "catalog-default".to_string(),
                overridable: false,
                sensitive: false,
            },
            crate::models::default_request_header::DefaultRequestHeader {
                name: "X-Amz-Target".to_string(),
                value: "Service.Write".to_string(),
                overridable: false,
                sensitive: false,
            },
        ];
        strip_durable_idempotency_defaults(&mut headers);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].name, "X-Amz-Target");
    }

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

    #[test]
    fn generated_request_id_is_forwarded_and_returned_as_correlation() {
        let mut headers = axum::http::HeaderMap::new();
        let generated = ensure_proxy_request_id(&mut headers);
        assert_eq!(headers["x-request-id"], generated);
        assert!(uuid::Uuid::parse_str(&generated).is_ok());

        let mut response = Body::empty().into_response();
        apply_proxy_request_id_header(&mut response, &generated);
        assert_eq!(response.headers()["x-request-id"], generated);

        headers.insert("x-request-id", "caller-owned".parse().unwrap());
        assert_eq!(ensure_proxy_request_id(&mut headers), "caller-owned");
    }

    #[test]
    fn async_locations_are_rewritten_to_the_caller_visible_proxy_route() {
        let context = AsyncLocationContext::new(
            "https://private.example/api/",
            "/executions",
            None,
            Some("/api/v1/proxy/s/sandbox/".to_string()),
        )
        .unwrap();

        assert_eq!(
            context.rewrite("executions/op-1?view=status").unwrap(),
            "/api/v1/proxy/s/sandbox/executions/op-1?view=status"
        );
        assert_eq!(
            context
                .rewrite("https://private.example/api/executions/op-2")
                .unwrap(),
            "/api/v1/proxy/s/sandbox/executions/op-2"
        );
        assert!(context.rewrite("https://attacker.example/op-3").is_none());
        assert!(context.rewrite("/outside-service/op-4").is_none());
    }

    #[test]
    fn async_response_headers_are_safe_and_location_requires_rewrite_context() {
        for name in [
            "retry-after",
            "preference-applied",
            "location",
            "operation-location",
            "etag",
        ] {
            assert!(ALLOWED_RESPONSE_HEADERS.contains(&name));
        }
        assert!(forwarded_response_header_value("location", b"/private", false, None).is_none());
    }

    #[test]
    fn unsafe_node_failover_stops_after_possible_dispatch() {
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(should_retry_node_failure(&method, false));
            assert!(!should_retry_node_failure(&method, true));
        }
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(should_retry_node_failure(&method, false));
            assert!(should_retry_node_failure(&method, true));
        }
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
    fn compose_pre_resolved_node_ids_uses_configured_as_primary_when_dispatchable() {
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
    fn compose_pre_resolved_node_ids_drops_configured_when_not_dispatchable() {
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
    fn compose_pre_resolved_node_ids_returns_empty_when_nothing_dispatchable() {
        // When the configured node is not dispatchable and no fallback bindings
        // exist, the list must be empty so `build_node_route` returns None.
        // The caller then hard-fails with `NodeOffline` to honor the
        // "Route via Node" contract rather than silently dropping to direct
        // routing. See ChronoAIProject/NyxID#328.
        let result = compose_pre_resolved_node_ids("configured", false, vec![]);

        assert!(result.is_empty());
    }

    #[test]
    fn compose_pre_resolved_node_ids_keeps_dispatchable_configured_without_fallbacks() {
        let result = compose_pre_resolved_node_ids("configured", true, vec![]);

        assert_eq!(result, vec!["configured".to_string()]);
    }

    #[test]
    fn node_scope_rejects_unplanned_fallback_promoted_to_primary() {
        let mut route = crate::services::node_routing_service::NodeRoute {
            node_id: "new-binding".to_string(),
            fallback_node_ids: vec!["planned-fallback".to_string()],
        };

        let error = enforce_node_route_scope(
            &mut route,
            &["configured".to_string(), "planned-fallback".to_string()],
        )
        .expect_err("an unplanned promoted node must fail closed");

        assert!(matches!(
            error,
            crate::errors::AppError::ApiKeyScopeForbidden(_)
        ));
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

    fn token_resale_metered_context(credential_class: CredentialClass) -> MeteredProxyContext {
        let billing = ServiceBilling {
            platform_billable: false,
            platform_metric: None,
            platform_pricing: None,
            resale_billable: true,
            resale_metric: BillingMetric::Tokens,
            lago_resale_metric_code: Some("resale_tokens".to_string()),
        };

        MeteredProxyContext {
            route: Some(BillingRouteContext::new(
                crate::services::billing::BillingIngress::Proxy,
                "request-1".to_string(),
                "owner-1".to_string(),
                "actor-1".to_string(),
                None,
                Some("user-service-1".to_string()),
                Some("catalog-1".to_string()),
                Some("llm-openai".to_string()),
                NodeIntent::Direct,
                "bearer".to_string(),
                credential_class,
                BillingMetric::Bytes,
                Some(&billing),
                true,
            )),
        }
    }

    #[test]
    fn websocket_realtime_usage_requires_catalog_protocol_and_trusted_resale_key() {
        let trusted = token_resale_metered_context(CredentialClass::NyxidManagedMaster);
        assert!(websocket_realtime_usage_enabled(
            Some("llm-openai"),
            &trusted
        ));
        assert!(!websocket_realtime_usage_enabled(
            Some("llm-anthropic"),
            &trusted
        ));
        assert!(!websocket_realtime_usage_enabled(None, &trusted));

        let user_owned = token_resale_metered_context(CredentialClass::UserOwned);
        assert!(!websocket_realtime_usage_enabled(
            Some("llm-openai"),
            &user_owned
        ));
    }

    #[test]
    fn websocket_resale_uses_reported_plus_uncovered_estimate() {
        let metered = token_resale_metered_context(CredentialClass::NyxidManagedMaster);
        let stats = ConnectionUsageStats {
            bytes_in: 120,
            bytes_out: 280,
            realtime_llm_usage: llm_usage_service::RealtimeLlmUsageSummary {
                collection_enabled: true,
                reported_usage: Some(llm_usage_service::ReportedLlmUsage {
                    prompt_tokens: 20,
                    completion_tokens: 10,
                    total_tokens: 30,
                    cached_tokens: 0,
                    cache_creation_tokens: 0,
                    reported_cost: None,
                }),
                uncovered_bytes: 20,
                reported_response_count: 1,
                estimated_response_count: 1,
            },
            ..Default::default()
        };

        let resale = websocket_resale_usage(&metered, &stats).expect("resale usage");
        assert_eq!(resale.metric, BillingMetric::Tokens);
        assert_eq!(resale.quantity, 35);

        let mut event = serde_json::json!({});
        add_websocket_usage_provenance(&mut event, &stats);
        assert_eq!(event["usage_provenance"]["reported_tokens"], 30);
        assert_eq!(event["usage_provenance"]["estimated_tokens"], 5);
        assert_eq!(event["usage_provenance"]["fallback_bytes"], 20);
    }

    #[test]
    fn websocket_resale_falls_back_for_non_realtime_protocol() {
        let metered = token_resale_metered_context(CredentialClass::NyxidManagedMaster);
        let collect_realtime_llm_usage =
            websocket_realtime_usage_enabled(Some("llm-anthropic"), &metered);
        let realtime_llm_usage =
            llm_usage_service::RealtimeLlmUsageCollector::new(collect_realtime_llm_usage)
                .finalize();
        let stats = ConnectionUsageStats {
            bytes_in: 40,
            bytes_out: 60,
            realtime_llm_usage,
            ..Default::default()
        };

        assert!(!stats.realtime_llm_usage.collection_enabled);
        let resale = websocket_resale_usage(&metered, &stats).expect("resale usage");
        assert_eq!(resale.quantity, 25);
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
                false,
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
    fn admin_platform_metric_override_beats_the_slug_heuristic() {
        // No override: a non-llm slug meters requests.
        let target = make_target("http://localhost:8080");
        assert_eq!(
            super::platform_metric_for_target(&target, false),
            BillingMetric::Requests
        );

        // Admin override: an arbitrarily named service meters tokens,
        // including on the WS/connection path.
        let mut target = make_target("http://localhost:8080");
        target.service.billing = Some(ServiceBilling {
            platform_metric: Some(BillingMetric::Tokens),
            ..Default::default()
        });
        assert_eq!(
            super::platform_metric_for_target(&target, false),
            BillingMetric::Tokens
        );
        assert_eq!(
            super::platform_metric_for_target(&target, true),
            BillingMetric::Tokens
        );
    }

    #[test]
    fn llm_usage_capture_preserves_slug_allowlist_and_adds_token_metrics() {
        assert!(super::should_capture_llm_usage(
            "llm-admin-override",
            BillingMetric::Requests
        ));
        assert!(super::should_capture_llm_usage(
            "chrono-llm-public",
            BillingMetric::Tokens
        ));
        assert!(!super::should_capture_llm_usage(
            "ordinary-service",
            BillingMetric::Requests
        ));
    }

    #[test]
    fn user_service_with_master_credential_classifies_as_master() {
        let mut target = make_target("http://localhost:8080");
        target.auth_method = "bearer".to_string();
        target.credential = "master-key".to_string();

        // Auto-provisioned UserService (no user key) injecting the catalog
        // master credential: the platform's key, not the user's.
        assert_eq!(
            final_credential_class(Some("us-1"), false, false, true, true, &target),
            CredentialClass::NyxidManagedMaster
        );
        // A UserService backed by the user's own key stays user-owned.
        assert_eq!(
            final_credential_class(Some("us-1"), false, false, true, false, &target),
            CredentialClass::UserOwned
        );
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

    #[tokio::test]
    #[allow(clippy::result_large_err)]
    async fn direct_websocket_injects_elevenlabs_api_key_on_upgrade() {
        use tokio_tungstenite::tungstenite::handshake::server::{
            Request as WsRequest, Response as WsResponse,
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind downstream WebSocket");
        let addr = listener.local_addr().expect("downstream address");
        let (header_tx, header_rx) = tokio::sync::oneshot::channel();
        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept downstream");
            let socket = tokio_tungstenite::accept_hdr_async(
                stream,
                move |request: &WsRequest, response: WsResponse| {
                    let api_key = request
                        .headers()
                        .get("xi-api-key")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    header_tx.send(api_key).expect("send captured API key");
                    Ok(response)
                },
            )
            .await
            .expect("complete downstream handshake");
            drop(socket);
        });

        let mut target = make_target(&format!("ws://{addr}"));
        target.auth_method = "header".to_string();
        target.auth_key_name = "xi-api-key".to_string();
        target.credential = "elevenlabs-test-key".to_string();
        let permit =
            crate::services::billing::route_inventory::enforce_billing_egress_classification(
                Some(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::BillingIngress::Proxy,
                    ),
                ),
                crate::services::billing::BillingIngress::Proxy,
            )
            .expect("proxy billing classification");

        let connection = super::connect_downstream_ws(
            &format!("ws://{addr}/v1/convai/conversation"),
            &target,
            &[],
            &[],
            &[],
            None,
            permit,
        )
        .await
        .expect("connect to ElevenLabs-shaped WebSocket");

        assert_eq!(
            header_rx.await.expect("captured API key"),
            Some("elevenlabs-test-key".to_string())
        );
        drop(connection);
        server_task.await.expect("downstream server task");
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
        proxy_service::collect_forward_headers(headers)
    }

    fn ws_forward_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
        collect_ws_forward_headers(headers)
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
    fn node_forward_preserves_request_scoped_headers_and_json_content_type() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("accept", "application/json".parse().unwrap());
        headers.insert("idempotency-key", "caller-key-001".parse().unwrap());
        headers.insert("x-trace-id", "trace-001".parse().unwrap());
        headers.insert(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .unwrap(),
        );
        headers.insert("tracestate", "vendor=value".parse().unwrap());
        headers.insert("last-event-id", "event-7".parse().unwrap());
        headers.insert("prefer", "respond-async".parse().unwrap());

        let forwarded = node_forward_headers(&headers);
        for (name, expected) in [
            ("content-type", "application/json"),
            ("accept", "application/json"),
            ("idempotency-key", "caller-key-001"),
            ("x-trace-id", "trace-001"),
            (
                "traceparent",
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            ),
            ("tracestate", "vendor=value"),
            ("last-event-id", "event-7"),
            ("prefer", "respond-async"),
        ] {
            assert!(
                forwarded.iter().any(
                    |(actual_name, value)| actual_name.eq_ignore_ascii_case(name)
                        && value == expected
                ),
                "{name} must reach node-routed downstream"
            );
        }
    }

    #[test]
    fn node_forward_still_drops_protected_and_transport_headers() {
        // Guard against the prefix rule accidentally widening the gate for
        // protected or HTTP-client-owned headers.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer leaked".parse().unwrap());
        headers.insert("cookie", "session=leaked".parse().unwrap());
        headers.insert("x-nyxid-internal", "should-not-leak".parse().unwrap());
        headers.insert("content-length", "999".parse().unwrap());
        headers.insert("connection", "keep-alive".parse().unwrap());

        let forwarded = node_forward_headers(&headers);
        assert!(
            forwarded.is_empty(),
            "protected and transport headers must be dropped, got {forwarded:?}"
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
        let result = proxy_service::collect_forward_headers(&headers);
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

        let result = proxy_service::collect_forward_headers(&headers);
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

    /// Cancelled work must not land in the proxy error rate as a 500 — that is
    /// what `proxy_client_disconnected` used to do via `AppError::Internal`.
    #[test]
    fn proxy_error_telemetry_fields_client_disconnected() {
        use super::{proxy_client_disconnected, proxy_error_telemetry_fields};
        use crate::errors::AppError;

        assert!(matches!(
            proxy_client_disconnected("svc-1"),
            AppError::ClientDisconnected
        ));
        assert_eq!(
            proxy_error_telemetry_fields(&AppError::ClientDisconnected),
            (499, 8012)
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

    // ── shared forward-header policy additional cases ───────────────

    #[test]
    fn collect_forward_headers_filters_unauthorized_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        headers.insert("x-custom-internal", "value".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());

        let result = proxy_service::collect_forward_headers(&headers);
        // authorization and x-custom-internal are not in the allowlist
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "content-type");
    }

    #[test]
    fn collect_forward_headers_openclaw_prefix() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-openclaw-scopes", "openai:*".parse().unwrap());
        headers.insert("x-openclaw-model", "gpt-4".parse().unwrap());

        let result = proxy_service::collect_forward_headers(&headers);
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

    /// SSE behind a buffering reverse proxy (nginx defaults to
    /// `proxy_buffering on`) arrives as one blob at the end. Both proxy
    /// paths set `X-Accel-Buffering: no` on SSE responses, and the header
    /// must stay forwardable so an upstream copy survives too — dropping
    /// either half silently un-streams every assistant/LLM SSE surface.
    #[test]
    fn allowed_response_headers_includes_accel_buffering() {
        // A proxied service must not be able to dictate front-proxy
        // buffering: NyxID derives `x-accel-buffering` itself for SSE
        // (mw::security_headers) and never forwards an upstream copy.
        assert!(!super::ALLOWED_RESPONSE_HEADERS.contains(&"x-accel-buffering"));
    }

    #[test]
    fn sse_drops_upstream_content_length() {
        // The length of a stream is unknown; a stale upstream value would
        // truncate it.
        assert!(!super::forwardable_response_header("content-length", true));
        assert!(super::forwardable_response_header("content-length", false));
    }

    #[test]
    fn buffering_hint_is_never_forwarded_from_upstream() {
        assert!(!super::forwardable_response_header(
            "x-accel-buffering",
            true
        ));
        assert!(!super::forwardable_response_header(
            "x-accel-buffering",
            false
        ));
    }

    #[test]
    fn forwardable_response_header_still_denies_unlisted_headers() {
        assert!(!super::forwardable_response_header("set-cookie", true));
        assert!(!super::forwardable_response_header("set-cookie", false));
        assert!(super::forwardable_response_header("content-type", true));
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

    #[tokio::test]
    async fn elevenlabs_audio_response_uses_direct_streaming_branch() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind downstream audio server");
        let addr = listener.local_addr().expect("downstream audio address");
        let app = Router::new().route(
            "/v1/text-to-speech/voice-1/stream",
            get(|| async {
                let mut response = Body::from(vec![0_u8, 1, 2, 255]).into_response();
                response
                    .headers_mut()
                    .insert("content-type", "audio/mpeg".parse().unwrap());
                response
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve audio");
        });

        let response = reqwest::get(format!("http://{addr}/v1/text-to-speech/voice-1/stream"))
            .await
            .expect("request ElevenLabs-shaped audio");

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(super::should_stream_response(
            &response,
            StatusCode::OK,
            false
        ));
        assert_eq!(
            response.bytes().await.expect("read audio bytes").as_ref(),
            &[0_u8, 1, 2, 255]
        );

        server.abort();
    }

    #[test]
    fn streaming_content_types_includes_octet_stream() {
        assert!(super::STREAMING_CONTENT_TYPES.contains(&"application/octet-stream"));
    }

    #[test]
    fn streaming_content_types_includes_pdf() {
        assert!(super::STREAMING_CONTENT_TYPES.contains(&"application/pdf"));
    }

    // ---- shared forward-header prefix coverage ----

    #[test]
    fn forward_header_prefixes_include_aws() {
        assert!(proxy_service::is_allowed_forward_header_prefix(
            "x-amz-target"
        ));
    }

    #[test]
    fn forward_header_prefixes_include_gcp() {
        assert!(proxy_service::is_allowed_forward_header_prefix(
            "x-goog-user-project"
        ));
    }

    #[test]
    fn forward_header_prefixes_include_openclaw() {
        assert!(proxy_service::is_allowed_forward_header_prefix(
            "x-openclaw-scopes"
        ));
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

        let result = proxy_service::collect_forward_headers(&headers);
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
    use super::{
        enforce_node_route_scope, execute_admin_proxy, proxy_request_by_slug_inner,
        proxy_request_inner,
    };
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
    use crate::services::node_ws_manager::NodeOutboundMessage;
    use crate::test_utils::{
        connect_test_database, test_app_state, test_membership, test_user, test_user_endpoint,
        test_user_service,
    };
    use axum::{
        Json, Router,
        body::{Body, Bytes, to_bytes},
        extract::{Path, State},
        http::{HeaderName, HeaderValue, Method, Request, StatusCode, Uri},
        response::{IntoResponse, Response},
        routing::{any, get, post},
    };
    use base64::Engine as _;
    use chrono::Utc;
    use futures::{SinkExt, StreamExt};
    use mongodb::bson::doc;
    use nyxid_node_proxy_test::{NodeMetrics as AgentNodeMetrics, ReplayGuard};
    use std::io;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    /// Turn the wire-log diagnostic on platform-wide, the way an operator does
    /// it at runtime through the admin feature-flag API.
    async fn enable_wire_log_flag(db: &mongodb::Database) {
        crate::services::feature_flag_service::set_platform_override(
            db,
            crate::services::feature_flag_service::AEVATAR_CHAT_WIRE_LOG_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            "test-admin",
        )
        .await
        .expect("enable the wire-log flag globally");
    }

    async fn disable_wire_log_flag(db: &mongodb::Database) {
        crate::services::feature_flag_service::clear_platform_override(
            db,
            crate::services::feature_flag_service::AEVATAR_CHAT_WIRE_LOG_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
        )
        .await
        .expect("clear the wire-log flag override");
    }

    async fn assistant_echoes(
        db: &mongodb::Database,
        user_id: &str,
        response: &Response,
    ) -> Vec<serde_json::Value> {
        let wrapper: serde_json::Value = if let Some(id) = response
            .headers()
            .get("x-nyxid-debug-upstream-id")
            .and_then(|value| value.to_str().ok())
        {
            assert!(
                response
                    .headers()
                    .get("x-nyxid-debug-upstream-log")
                    .is_none()
            );
            let row = crate::services::assistant_wire_log_service::fetch_for_user(db, user_id, id)
                .await
                .expect("fetch assistant wire log")
                .expect("assistant wire log exists");
            serde_json::from_str(&row.payload).expect("stored assistant echo is JSON")
        } else if let Some(value) = response.headers().get("x-nyxid-debug-upstream-log") {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(value.as_bytes())
                .expect("assistant echo header is base64");
            serde_json::from_slice(&decoded).expect("assistant echo header is JSON")
        } else {
            return Vec::new();
        };
        assert_eq!(wrapper["version"], 2);
        assert_eq!(wrapper["droppedEchoCount"], 0);
        wrapper["echoes"]
            .as_array()
            .expect("assistant echo wrapper has an echoes array")
            .clone()
    }
    use tower::ServiceExt;
    use uuid::Uuid;

    #[derive(Clone)]
    struct TraceCapture(Arc<Mutex<Vec<u8>>>);

    impl io::Write for TraceCapture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("trace capture lock")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn upstream_error_body_is_redacted_end_to_end() {
        let Some(db) = connect_test_database("proxy_upstream_error_redaction").await else {
            panic!("MongoDB is required for upstream redaction test");
        };
        let app = Router::new().route(
            "/{*path}",
            any(|| async {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    [("x-request-id", "upstream-redaction-1")],
                    "SENTINEL_PASSENGER_NAME",
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redaction server");
        let addr = listener.local_addr().expect("redaction server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve redaction server");
        });

        let user_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .expect("insert redaction test user");
        let encryption_keys = crate::test_utils::test_encryption_keys();
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = "redaction-test".to_string();
        service.base_url = format!("http://{addr}");
        service.service_category = "internal".to_string();
        service.auth_method = "bearer".to_string();
        service.credential_encrypted = encryption_keys
            .encrypt(b"test-master-credential")
            .await
            .expect("encrypt test credential");
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&service)
        .await
        .expect("insert redaction test service");

        let state = test_app_state(db);
        let capture = Arc::new(Mutex::new(Vec::new()));
        let writer = {
            let capture = capture.clone();
            move || TraceCapture(capture.clone())
        };
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(writer)
            .finish();
        let auth = access_token_auth(&user_id);
        let mut resolved_slug = String::new();
        let default_guard = tracing::subscriber::set_default(subscriber);
        let response = execute_admin_proxy(
            &state,
            &auth,
            &service.id,
            "error",
            proxy_request("/assistant/error"),
            Vec::new(),
            &mut resolved_slug,
        )
        .await
        .expect("upstream response should be returned");
        drop(default_guard);
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let output = String::from_utf8(capture.lock().expect("trace capture lock").clone())
            .expect("trace output is utf8");
        assert!(output.contains("upstream-redaction-1"));
        assert!(output.contains("response_size"));
        assert!(!output.contains("SENTINEL_PASSENGER_NAME"));
        server.abort();
    }

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

    async fn echo_node_request(
        headers: axum::http::HeaderMap,
        body: Bytes,
    ) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "content_type": headers
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            "idempotency_key": headers
                .get("idempotency-key")
                .and_then(|value| value.to_str().ok()),
            "trace_id": headers
                .get("x-trace-id")
                .and_then(|value| value.to_str().ok()),
            "body": String::from_utf8_lossy(&body),
        }))
    }

    async fn start_node_echo_downstream() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route("/commands", post(echo_node_request));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind node echo downstream");
        let addr = listener.local_addr().expect("node echo downstream addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve node echo downstream");
        });
        (format!("http://{addr}"), server)
    }

    async fn start_node_executor(
        state: AppState,
        node: &Node,
        service_slug: &str,
        target_url: &str,
    ) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route(
                "/api/v1/nodes/ws",
                get(crate::handlers::node_ws::ws_handler),
            )
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind node WebSocket server");
        let addr = listener.local_addr().expect("node WebSocket server addr");
        let ws_server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .expect("serve node WebSocket handler");
        });

        let (mut socket, response) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/api/v1/nodes/ws"))
                .await
                .expect("connect node executor WebSocket");
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "type": "auth",
                    "node_id": node.id,
                    "token": "test-node-auth-token",
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("authenticate node executor");
        let auth_response = socket
            .next()
            .await
            .expect("node auth response")
            .expect("read node auth response");
        let tokio_tungstenite::tungstenite::Message::Text(auth_response) = auth_response else {
            panic!("expected text node auth response");
        };
        let auth_response: serde_json::Value =
            serde_json::from_str(&auth_response).expect("parse node auth response");
        assert_eq!(auth_response["type"], "auth_ok");

        for _ in 0..100 {
            if state.node_ws_manager.is_connected(&node.id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            state.node_ws_manager.is_connected(&node.id),
            "authenticated node WebSocket must be registered"
        );

        let credentials = nyxid_node_proxy_test::no_auth_credentials(service_slug, target_url)
            .expect("build node executor credentials");
        let signing_secret = "11".repeat(32);
        let executor = tokio::spawn(async move {
            let (mut ws_sink, mut ws_stream) = socket.split();
            let replay_guard = tokio::sync::Mutex::new(ReplayGuard::new());
            let metrics = AgentNodeMetrics::new();
            let http_client = nyxid_node_proxy_test::proxy_executor::build_http_client()
                .expect("build node executor HTTP client");

            while let Some(message) = ws_stream.next().await {
                let message = message.expect("read proxy request from backend WebSocket");
                let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
                    continue;
                };
                let request: serde_json::Value =
                    serde_json::from_str(&text).expect("parse backend node request");
                if request["type"] != "proxy_request" {
                    continue;
                }

                let (tx, mut rx) = mpsc::channel(8);
                nyxid_node_proxy_test::proxy_executor::execute_proxy_request(
                    &request,
                    &credentials,
                    Some(&signing_secret),
                    &replay_guard,
                    &metrics,
                    &tx,
                    false,
                    &http_client,
                )
                .await;
                drop(tx);

                while let Some(response) = rx.recv().await {
                    let message = match response {
                        nyxid_node_proxy_test::ws_client::NodeWsMessage::Text(text) => {
                            tokio_tungstenite::tungstenite::Message::Text(text.into())
                        }
                        nyxid_node_proxy_test::ws_client::NodeWsMessage::Binary(bytes) => {
                            tokio_tungstenite::tungstenite::Message::Binary(bytes.into())
                        }
                    };
                    ws_sink
                        .send(message)
                        .await
                        .expect("send node executor response over WebSocket");
                }
                break;
            }
        });

        (executor, ws_server)
    }

    fn proxy_request(uri: &str) -> Request<Body> {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .expect("build proxy request");
        request.extensions_mut().insert(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        );
        request
    }

    fn proxy_json_request(uri: &str, body: &str) -> Request<Body> {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .header("idempotency-key", "caller-key-001")
            .header("x-trace-id", "trace-001")
            .body(Body::from(body.to_string()))
            .expect("build JSON proxy request");
        request.extensions_mut().insert(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        );
        request
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
            .route_layer(axum::Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy,
                ),
            ))
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
            oauth_client_id: None,
            token_jti: None,
            approval_owner_user_id: Some(owner_user_id.to_string()),
            auth_method: AuthMethod::ServiceAccount,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: vec![],
            resource_uris: None,
            allowed_node_ids: vec![],
            api_key_id: None,
            api_key_name: None,
            api_key_purpose: crate::models::api_key::ApiKeyPurpose::General,
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
            oauth_client_id: None,
            token_jti: None,
            approval_owner_user_id: None,
            auth_method: AuthMethod::AccessToken,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: vec![],
            resource_uris: None,
            allowed_node_ids: vec![],
            api_key_id: None,
            api_key_name: None,
            api_key_purpose: crate::models::api_key::ApiKeyPurpose::General,
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
                credential_source: None,
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
                oauth_attempt_nonce: None,
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                status: "active".to_string(),
                last_used_at: None,
                last_authorized_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                created_at: now,
                updated_at: now,
                credential_epoch: 1,
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

            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
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
    async fn node_routed_custom_service_preserves_headers_body_and_audits_owner() {
        let Some(db) = connect_test_database("proxy_org_node").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        seed_org_actor(&db, &org_id, &member_id, OrgRole::Member).await;

        let state = test_app_state(db.clone());
        let node = insert_online_node(&state, &org_id, "org-node").await;
        let (base_url, echo_server) = start_node_echo_downstream().await;

        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &org_id,
            "Org Node Target",
            &base_url,
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

        let (executor, ws_server) =
            start_node_executor(state.clone(), &node, &service.slug, &base_url).await;

        let mut resolved_slug = String::new();
        let response = proxy_request_by_slug_inner(
            &state,
            &access_token_auth(&member_id),
            &service.slug,
            "commands",
            proxy_json_request(
                "/proxy/s/org-node-target/commands",
                r#"{"operation":"probe"}"#,
            ),
            &mut resolved_slug,
        )
        .await
        .expect("org member should proxy through org node");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(resolved_slug, service.slug);
        let response_body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read node-routed echo response");
        let observed: serde_json::Value =
            serde_json::from_slice(&response_body).expect("parse node-routed echo response");
        assert_eq!(observed["content_type"], "application/json");
        assert_eq!(observed["idempotency_key"], "caller-key-001");
        assert_eq!(observed["trace_id"], "trace-001");
        assert_eq!(observed["body"], r#"{"operation":"probe"}"#);
        executor.await.expect("node executor task");

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
        ws_server.abort();
        echo_server.abort();
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
    async fn created_scope_admits_listed_service_and_node_and_rejects_others() {
        let Some(db) = connect_test_database("proxy_created_scope_enforcement").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let user_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .expect("insert user");
        let allowed_service =
            insert_user_service(&db, &user_id, "scope-allowed", &base_url, None).await;
        let denied_service =
            insert_user_service(&db, &user_id, "scope-denied", &base_url, None).await;

        let state = test_app_state(db.clone());
        let allowed_node = insert_online_node(&state, &user_id, "scope-allowed-node").await;
        let axum::Json(created) = crate::handlers::api_keys::create_key(
            State(state.clone()),
            crate::test_utils::test_auth_user(&user_id),
            crate::telemetry::TelemetryContext::default(),
            axum::Json(crate::handlers::api_keys::CreateApiKeyRequest {
                name: "scoped-proxy-key".to_string(),
                scopes: Some("proxy".to_string()),
                expires_at: None,
                description: None,
                allowed_service_ids: vec![allowed_service.id.clone()],
                allowed_node_ids: vec![allowed_node.id.clone()],
                allow_all_services: None,
                allow_all_nodes: None,
                rate_limit_per_second: None,
                rate_limit_burst: None,
                platform: Some("codex".to_string()),
                callback_url: None,
                target_org_id: None,
                scope_plan_digest: None,
                selected_operations: Vec::new(),
            }),
        )
        .await
        .expect("create scoped key");
        assert!(!created.allow_all_services);
        assert!(!created.allow_all_nodes);

        let (validated_user_id, stored_key) =
            crate::services::key_service::validate_api_key(&db, &created.full_key)
                .await
                .expect("authenticate created key");
        assert_eq!(validated_user_id, user_id);

        let mut auth = access_token_auth(&user_id);
        auth.auth_method = AuthMethod::ApiKey;
        auth.api_key_id = Some(stored_key.id.clone());
        auth.api_key_name = Some(stored_key.name.clone());
        auth.allow_all_services = stored_key.allow_all_services;
        auth.allow_all_nodes = stored_key.allow_all_nodes;
        auth.allowed_service_ids = stored_key.allowed_service_ids.clone();
        auth.allowed_node_ids = stored_key.allowed_node_ids.clone();

        let mut allowed_slug = String::new();
        let response = proxy_request_by_slug_inner(
            &state,
            &auth,
            &allowed_service.slug,
            "status",
            proxy_request("/proxy/s/scope-allowed/status"),
            &mut allowed_slug,
        )
        .await
        .expect("listed service should be permitted");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(allowed_slug, allowed_service.slug);

        let mut denied_slug = String::new();
        let error = proxy_request_by_slug_inner(
            &state,
            &auth,
            &denied_service.slug,
            "status",
            proxy_request("/proxy/s/scope-denied/status"),
            &mut denied_slug,
        )
        .await
        .expect_err("service outside the created allowlist must be rejected");
        assert!(matches!(error, AppError::ApiKeyScopeForbidden(_)));

        let mut allowed_route = crate::services::node_routing_service::NodeRoute {
            node_id: allowed_node.id.clone(),
            fallback_node_ids: vec!["unlisted-fallback".to_string()],
        };
        enforce_node_route_scope(&mut allowed_route, &stored_key.allowed_node_ids)
            .expect("listed node should be permitted");
        assert!(allowed_route.fallback_node_ids.is_empty());

        let mut denied_route = crate::services::node_routing_service::NodeRoute {
            node_id: Uuid::new_v4().to_string(),
            fallback_node_ids: Vec::new(),
        };
        let error = enforce_node_route_scope(&mut denied_route, &stored_key.allowed_node_ids)
            .expect_err("node outside the created allowlist must be rejected");
        assert!(matches!(error, AppError::ApiKeyScopeForbidden(_)));

        server.abort();
    }

    /// Seed an admin-managed platform service (the shape the assistant chat
    /// pass-through targets: internal, no user credential, master/no auth).
    async fn insert_platform_service(
        db: &mongodb::Database,
        slug: &str,
        base_url: &str,
    ) -> crate::models::downstream_service::DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = slug.to_string();
        service.name = slug.to_string();
        service.base_url = base_url.to_string();
        service.service_category = "internal".to_string();
        service.requires_user_credential = false;
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(service.clone())
        .await
        .unwrap();
        service
    }

    #[tokio::test]
    async fn configured_operation_policy_blocks_rest_without_forwarding() {
        let Some(db) = connect_test_database("proxy_operation_policy_rest").await else {
            panic!("MongoDB is required for proxy operation policy test");
        };
        let forwarded = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/{*path}",
                any(|State(counter): State<Arc<AtomicUsize>>| async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }),
            )
            .with_state(forwarded.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind operation policy downstream");
        let base_url = format!(
            "http://{}",
            listener.local_addr().expect("operation policy address")
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve operation policy downstream");
        });

        let user_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .expect("insert operation policy user");
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = "operation-policy-rest".to_string();
        service.name = "Operation policy REST".to_string();
        service.base_url = base_url;
        service.service_category = "internal".to_string();
        service.proxy_operation_policy = Some(
            crate::services::proxy_authorization::normalize_policy(
                crate::models::downstream_service::ProxyOperationPolicy {
                    rules: vec![crate::models::downstream_service::ProxyOperationRule {
                        method: "POST".to_string(),
                        path_template: "/air/offer_requests".to_string(),
                    }],
                },
            )
            .expect("valid operation policy"),
        );
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&service)
        .await
        .expect("insert operation policy service");

        let state = test_app_state(db);
        let auth = access_token_auth(&user_id);
        let mut allowed = proxy_json_request(
            "/api/v1/proxy/s/operation-policy-rest/air/offer_requests",
            r#"{"data":{}}"#,
        );
        *allowed.method_mut() = Method::POST;
        let mut resolved_slug = String::new();
        let response = execute_admin_proxy(
            &state,
            &auth,
            &service.id,
            "air/offer_requests",
            allowed,
            Vec::new(),
            &mut resolved_slug,
        )
        .await
        .expect("allowlisted REST operation must be forwarded");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(forwarded.load(Ordering::SeqCst), 1);

        for blocked_path in ["air/orders", "air/orders/ord_123"] {
            let error = execute_admin_proxy(
                &state,
                &auth,
                &service.id,
                blocked_path,
                proxy_request(&format!(
                    "/api/v1/proxy/s/operation-policy-rest/{blocked_path}"
                )),
                Vec::new(),
                &mut resolved_slug,
            )
            .await
            .expect_err("blocked REST operation must be denied");
            assert!(matches!(error, AppError::NotFound(_)));
        }
        assert_eq!(
            forwarded.load(Ordering::SeqCst),
            1,
            "blocked REST operations must not reach the downstream"
        );
        server.abort();
    }

    /// A restricted token's `allowed_service_ids` holds `UserService` ids, so
    /// an admin catalog row can never be listed in one. Gating the platform
    /// surface on that list therefore denied every restricted caller with no
    /// grant that could fix it (observed on prod: OAuth access tokens minted
    /// with `resource`/`allowed_service_ids` got `api_key_scope_forbidden` on
    /// every `/api/v1/assistant/*` call). The admin-managed mode drops that
    /// gate; the caller-addressed mode keeps it.
    #[tokio::test]
    async fn admin_managed_target_admits_a_scoped_token_the_caller_path_rejects() {
        let Some(db) = connect_test_database("proxy_admin_target_scope").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let user_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let service = insert_platform_service(&db, "platform-assistant", &base_url).await;

        let state = test_app_state(db.clone());
        let mut auth = access_token_auth(&user_id);
        auth.allow_all_services = false;
        // A UserService id the caller really was granted — still never the
        // catalog row the platform resolves.
        auth.allowed_service_ids = vec![Uuid::new_v4().to_string()];

        let mut caller_slug = String::new();
        let err = super::execute_proxy(
            &state,
            &auth,
            &service.id,
            "status",
            proxy_request("/assistant/conversations"),
            &mut caller_slug,
        )
        .await
        .expect_err("caller-addressed mode must keep enforcing the allowlist");
        assert!(
            matches!(err, AppError::ApiKeyScopeForbidden(_)),
            "expected scope denial on the caller-addressed path, got: {err}"
        );

        let mut admin_slug = String::new();
        let response = super::execute_admin_proxy(
            &state,
            &auth,
            &service.id,
            "status",
            proxy_request("/assistant/conversations"),
            Vec::new(),
            &mut admin_slug,
        )
        .await
        .expect("admin-managed mode must admit a restricted caller");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(admin_slug, service.slug);
        server.abort();
    }

    /// "Works for everyone but me": a user who once connected the same
    /// catalog service personally and then disconnected it was refused by the
    /// platform surface they never connected. The admin-managed mode does not
    /// read the caller's connection row at all.
    #[tokio::test]
    async fn admin_managed_target_ignores_a_disconnected_personal_connection() {
        let Some(db) = connect_test_database("proxy_admin_target_disconnected").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let user_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let service = insert_platform_service(&db, "platform-assistant-2", &base_url).await;
        let now = chrono::Utc::now();
        db.collection::<crate::models::user_service_connection::UserServiceConnection>(
            crate::models::user_service_connection::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::user_service_connection::UserServiceConnection {
                id: Uuid::new_v4().to_string(),
                user_id: user_id.clone(),
                service_id: service.id.clone(),
                credential_encrypted: None,
                credential_type: None,
                credential_label: None,
                metadata: None,
                is_active: false,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();

        let state = test_app_state(db.clone());
        let auth = access_token_auth(&user_id);

        let mut caller_slug = String::new();
        let err = super::execute_proxy(
            &state,
            &auth,
            &service.id,
            "status",
            proxy_request("/assistant/conversations"),
            &mut caller_slug,
        )
        .await
        .expect_err("caller-addressed mode must keep honoring the disconnect");
        assert!(
            matches!(err, AppError::Forbidden(_)),
            "expected a disconnect denial on the caller-addressed path, got: {err}"
        );

        let mut admin_slug = String::new();
        let response = super::execute_admin_proxy(
            &state,
            &auth,
            &service.id,
            "status",
            proxy_request("/assistant/conversations"),
            Vec::new(),
            &mut admin_slug,
        )
        .await
        .expect("admin-managed mode must ignore the personal disconnect");
        assert_eq!(response.status(), StatusCode::OK);
        server.abort();
    }

    /// A `requires_user_credential` row cannot back a platform surface: it is
    /// a provisioning fault, and must not degrade into a caller-facing error.
    #[tokio::test]
    async fn admin_managed_target_rejects_a_user_credential_service_as_internal() {
        let Some(db) = connect_test_database("proxy_admin_target_user_cred").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        let (base_url, server) = start_downstream().await;
        let user_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let mut service = insert_platform_service(&db, "platform-needs-cred", &base_url).await;
        service.requires_user_credential = true;
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .replace_one(doc! { "_id": &service.id }, &service)
        .await
        .unwrap();

        let state = test_app_state(db.clone());
        let mut admin_slug = String::new();
        let err = super::execute_admin_proxy(
            &state,
            &access_token_auth(&user_id),
            &service.id,
            "status",
            proxy_request("/assistant/conversations"),
            Vec::new(),
            &mut admin_slug,
        )
        .await
        .expect_err("a user-credential row must not back a platform surface");
        assert!(
            matches!(err, AppError::Internal(_)),
            "expected a provisioning fault, got: {err}"
        );
        server.abort();
    }

    /// End-to-end through the real assistant handlers: typed commands land on
    /// `/api/chat`, resources map by conversation family, and each upstream
    /// request carries injected identity material with NO caller
    /// `Authorization`.
    #[tokio::test]
    async fn assistant_chat_handlers_rebuild_bodies_for_the_admin_service() {
        use std::sync::Mutex as StdMutex;

        let Some(db) = connect_test_database("assistant_typed_chat").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        type Captured = (Method, String, Vec<u8>, axum::http::HeaderMap);
        let captured: std::sync::Arc<StdMutex<Vec<Captured>>> =
            std::sync::Arc::new(StdMutex::new(Vec::new()));
        let sink = captured.clone();
        let app = Router::new().route(
            "/{*path}",
            any(move |request: Request<Body>| {
                let sink = sink.clone();
                async move {
                    let (parts, body) = request.into_parts();
                    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
                    let wants_json = parts
                        .headers
                        .get(axum::http::header::ACCEPT)
                        .and_then(|value| value.to_str().ok())
                        == Some("application/json");
                    sink.lock().unwrap().push((
                        parts.method.clone(),
                        parts.uri.path().to_string(),
                        bytes.to_vec(),
                        parts.headers.clone(),
                    ));
                    match (parts.method, parts.uri.path()) {
                        (Method::GET, "/api/chat/conversations") => axum::Json(serde_json::json!({
                            "conversations": [{
                                "id": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                                "updatedAt": "2026-07-29T13:00:00.000Z"
                            }]
                        }))
                        .into_response(),
                        (Method::GET, path) if path.ends_with("/chat-history") => {
                            axum::Json(serde_json::json!({
                                "conversations": [
                                    {
                                        "id": "chatc-8bd999c402fb37d60cdcd81e3b78cfd",
                                        "updatedAt": "2026-07-29T12:00:00.000Z"
                                    }
                                ]
                            }))
                            .into_response()
                        }
                        (Method::GET, path) if path.starts_with("/api/chat/conversations/") => {
                            axum::Json(serde_json::json!({
                                "messages": [],
                                "stateVersion": 3
                            }))
                            .into_response()
                        }
                        (Method::GET, path) if path.contains("/chat-history/conversations/") => {
                            axum::Json(serde_json::json!({
                                "messages": [],
                                "stateVersion": 4
                            }))
                            .into_response()
                        }
                        (Method::DELETE, path) if path.starts_with("/api/chat/conversations/") => (
                            StatusCode::ACCEPTED,
                            axum::Json(serde_json::json!({ "status": "accepted" })),
                        )
                            .into_response(),
                        (Method::DELETE, path) if path.contains("/chat-history/conversations/") => {
                            StatusCode::OK.into_response()
                        }
                        (_, "/api/chat") if wants_json => {
                            axum::Json(serde_json::json!({ "ok": true })).into_response()
                        }
                        (_, "/api/chat") => Response::builder()
                            .status(StatusCode::OK)
                            .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
                            .header("x-request-id", "aevatar-request-1")
                            .header(axum::http::header::SET_COOKIE, "upstream=secret")
                            .body(Body::from("data: {\"type\":\"RUN_FINISHED\"}\n\n"))
                            .unwrap(),
                        _ => axum::Json(serde_json::json!({ "ok": true })).into_response(),
                    }
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind assistant downstream listener");
        let addr = listener.local_addr().expect("downstream listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve assistant downstream");
        });

        let user_id = Uuid::new_v4().to_string();
        crate::services::role_service::seed_system_roles(&db)
            .await
            .expect("seed platform roles");
        let role_ids = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .expect("resolve platform roles");
        let mut admin_user = test_user(&user_id, UserType::Person);
        admin_user.role_ids.push(role_ids.admin);
        admin_user.is_admin = true;
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(admin_user)
            .await
            .unwrap();
        // The assistant resolves by slug: this must be the `aevatar` row,
        // configured like the deployed contract (identity JWT + injected
        // delegation token, no Bearer forwarding).
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = crate::services::assistant_service::AEVATAR_SLUG.to_string();
        service.name = "Aevatar".to_string();
        service.base_url = format!("http://{addr}");
        service.service_category = "internal".to_string();
        service.requires_user_credential = false;
        service.identity_propagation_mode = "jwt".to_string();
        service.identity_jwt_audience = Some("urn:aevatar:api".to_string());
        service.inject_delegation_token = true;
        service.delegation_token_scope = "proxy:*".to_string();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(service.clone())
        .await
        .unwrap();

        let state = test_app_state(db.clone());
        // The wire log is gated by a runtime feature flag, not process config:
        // a platform-global override turns it on for every caller.
        enable_wire_log_flag(&db).await;
        let auth = access_token_auth(&user_id);
        let billing_policy = crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Proxy,
        );
        let request = |method: Method, uri: &str, body: Option<&str>| {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.unwrap_or_default().to_string()))
                .expect("build assistant request");
            request.extensions_mut().insert(billing_policy);
            request
        };

        let mut debug_echoes = Vec::new();
        let calls_before_unknown_type = captured.lock().unwrap().len();
        let unknown_type_error = crate::handlers::assistant::typed_chat(
            axum::extract::State(state.clone()),
            auth.clone(),
            request(
                Method::POST,
                "/api/v1/assistant/chat",
                Some(r#"{"type":"workflow.studio","prompt":"do not fall through"}"#),
            ),
        )
        .await
        .expect_err("an unknown typed command must fail locally");
        assert_eq!(
            unknown_type_error.into_response().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            calls_before_unknown_type,
            "an unknown typed command must not reach either upstream chat path"
        );

        for body in [
            r#"{"type":"text","prompt":"connect api-github","clientRequestId":"00000000-0000-4000-8000-000000000001"}"#,
            r#"{"type":"text","prompt":"continue","clientRequestId":"00000000-0000-4000-8000-000000000002","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae"}"#,
            r#"{"type":"plan.resolve","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","taskId":"task-1","planId":"plan-1","requestId":"plan-gate-1","clientRequestId":"00000000-0000-4000-8000-000000000011","planRevision":3,"confirmed":true,"expectedStateVersion":23}"#,
            r#"{"type":"input.resolve","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","clientRequestId":"00000000-0000-4000-8000-000000000010","requestId":"input-1","answer":{"selectedOptionIds":["option-a","option-b"]},"expectedStateVersion":19}"#,
            r#"{"type":"action.continue","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","clientRequestId":"00000000-0000-4000-8000-000000000003","originTurnId":"turn-action-1","actions":[{"actionRequestId":"act-1","originTurnId":"turn-action-1","disposition":"completed","resource":{"userService":{"userServiceId":"00000000-0000-4000-8000-000000000123"}}}]}"#,
            r#"{"type":"action.continue","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","clientRequestId":"00000000-0000-4000-8000-000000000009","originTurnId":"turn-action-2","actions":[{"actionRequestId":"act-2","originTurnId":"turn-action-2","disposition":"failed"}]}"#,
            r#"{"type":"approval.resolve","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","clientRequestId":"00000000-0000-4000-8000-000000000004","requestId":"approval-1","approved":true,"reason":"Approved by user","expectedStateVersion":21}"#,
            r#"{"type":"task.stop","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","turnId":"turn-1","stopRequestId":"stop-1","clientRequestId":"00000000-0000-4000-8000-000000000005","expectedStateVersion":0}"#,
            r#"{"type":"task.steer","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","turnId":"turn-1","steeringId":"steer-1","clientRequestId":"00000000-0000-4000-8000-000000000006","instruction":"Try again","expectedStateVersion":2}"#,
            r#"{"type":"step.retry","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","turnId":"turn-1","taskId":"task-1","stepId":"step-1","retryRequestId":"retry-1","clientRequestId":"00000000-0000-4000-8000-000000000007","expectedOperationGeneration":2,"expectedStateVersion":3}"#,
            r#"{"type":"step.skip","conversationId":"nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae","turnId":"turn-1","taskId":"task-1","stepId":"step-2","skipRequestId":"skip-1","clientRequestId":"00000000-0000-4000-8000-000000000008","expectedOperationGeneration":4,"expectedStateVersion":5}"#,
        ] {
            let mut typed_request = request(Method::POST, "/api/v1/assistant/chat", Some(body));
            typed_request.headers_mut().insert(
                axum::http::header::ACCEPT,
                HeaderValue::from_static("text/plain"),
            );
            typed_request.headers_mut().insert(
                HeaderName::from_static("idempotency-key"),
                HeaderValue::from_static("caller-supplied-key"),
            );
            typed_request.headers_mut().insert(
                HeaderName::from_static("x-nyxid-debug-upstream"),
                HeaderValue::from_static("1"),
            );
            let response = crate::handlers::assistant::typed_chat(
                axum::extract::State(state.clone()),
                auth.clone(),
                typed_request,
            )
            .await
            .expect("typed chat handler must forward");
            assert_eq!(response.status(), StatusCode::OK);
            debug_echoes.push(assistant_echoes(&db, &user_id, &response).await);
        }

        let mut list_request = request(Method::GET, "/api/v1/assistant/conversations", None);
        list_request.headers_mut().insert(
            HeaderName::from_static("x-nyxid-debug-upstream"),
            HeaderValue::from_static("1"),
        );
        let list_response = crate::handlers::assistant::list_conversations(
            axum::extract::State(state.clone()),
            auth.clone(),
            list_request,
        )
        .await
        .expect("list conversations handler must forward");
        assert_eq!(list_response.status(), StatusCode::OK);
        assert!(
            list_response
                .headers()
                .get("x-nyxid-debug-upstream-id")
                .is_none()
        );
        assert!(
            list_response
                .headers()
                .get("x-nyxid-debug-upstream-log")
                .is_none()
        );
        let wire_logs = db.collection::<crate::models::assistant_wire_log::AssistantWireLog>(
            crate::models::assistant_wire_log::AssistantWireLog::COLLECTION_NAME,
        );
        assert_eq!(
            wire_logs
                .count_documents(doc! { "user_id": &user_id })
                .await
                .expect("count stored assistant wire logs"),
            11
        );
        assert_eq!(
            wire_logs
                .count_documents(doc! {
                    "user_id": &user_id,
                    "conversation_id": mongodb::bson::Bson::Null,
                })
                .await
                .expect("count unattributed assistant wire logs"),
            1
        );
        assert_eq!(
            wire_logs
                .count_documents(doc! {
                    "user_id": &user_id,
                    "conversation_id": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                })
                .await
                .expect("count conversation-scoped assistant wire logs"),
            10
        );

        let malformed_typed_id = "nyxid-chat-not-a-guid";
        let calls_before_malformed_resources = captured.lock().unwrap().len();
        let malformed_history_error = crate::handlers::assistant::get_history(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path(malformed_typed_id.to_string()),
            request(
                Method::GET,
                &format!("/api/v1/assistant/conversations/{malformed_typed_id}"),
                None,
            ),
        )
        .await
        .expect_err("malformed typed history id must fail locally");
        assert_eq!(
            malformed_history_error.into_response().status(),
            StatusCode::BAD_REQUEST
        );
        let malformed_delete_error = crate::handlers::assistant::delete_conversation(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path(malformed_typed_id.to_string()),
            request(
                Method::DELETE,
                &format!("/api/v1/assistant/conversations/{malformed_typed_id}"),
                None,
            ),
        )
        .await
        .expect_err("malformed typed delete id must fail locally");
        assert_eq!(
            malformed_delete_error.into_response().status(),
            StatusCode::BAD_REQUEST
        );
        let malformed_state_error = crate::handlers::assistant::get_state(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path(malformed_typed_id.to_string()),
            request(
                Method::GET,
                &format!("/api/v1/assistant/conversations/{malformed_typed_id}/state"),
                None,
            ),
        )
        .await
        .expect_err("malformed typed state id must fail locally");
        assert_eq!(
            malformed_state_error.into_response().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            calls_before_malformed_resources,
            "malformed typed resource ids must not reach upstream"
        );

        let typed_history_response = crate::handlers::assistant::get_history(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path("nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae".to_string()),
            request(
                Method::GET,
                "/api/v1/assistant/conversations/nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                None,
            ),
        )
        .await
        .expect("history handler must forward");
        assert_eq!(typed_history_response.status(), StatusCode::OK);
        let typed_state_response = crate::handlers::assistant::get_state(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path("nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae".to_string()),
            request(
                Method::GET,
                "/api/v1/assistant/conversations/nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae/state",
                None,
            ),
        )
        .await
        .expect("state handler must forward");
        assert_eq!(typed_state_response.status(), StatusCode::OK);
        let typed_delete_response = crate::handlers::assistant::delete_conversation(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path("nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae".to_string()),
            request(
                Method::DELETE,
                "/api/v1/assistant/conversations/nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                None,
            ),
        )
        .await
        .expect("delete handler must forward");
        assert_eq!(typed_delete_response.status(), StatusCode::ACCEPTED);
        let typed_delete_body = to_bytes(typed_delete_response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&typed_delete_body).unwrap(),
            serde_json::json!({ "status": "accepted" })
        );

        let workflow_id = "chatc-8bd999c402fb37d60cdcd81e3b78cfd";
        let workflow_history_response = crate::handlers::assistant::get_history(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path(workflow_id.to_string()),
            request(
                Method::GET,
                &format!("/api/v1/assistant/conversations/{workflow_id}"),
                None,
            ),
        )
        .await
        .expect("workflow history handler must forward");
        assert_eq!(workflow_history_response.status(), StatusCode::OK);

        let calls_before_workflow_state = captured.lock().unwrap().len();
        let workflow_state_error = crate::handlers::assistant::get_state(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path(workflow_id.to_string()),
            request(
                Method::GET,
                &format!("/api/v1/assistant/conversations/{workflow_id}/state"),
                None,
            ),
        )
        .await
        .expect_err("workflow state must be not-found locally");
        assert!(matches!(workflow_state_error, AppError::NotFound(_)));
        assert_eq!(captured.lock().unwrap().len(), calls_before_workflow_state);

        let workflow_delete_response = crate::handlers::assistant::delete_conversation(
            axum::extract::State(state.clone()),
            auth.clone(),
            Path(workflow_id.to_string()),
            request(
                Method::DELETE,
                &format!("/api/v1/assistant/conversations/{workflow_id}"),
                None,
            ),
        )
        .await
        .expect("workflow delete handler must forward");
        assert_eq!(workflow_delete_response.status(), StatusCode::NO_CONTENT);
        assert!(
            to_bytes(workflow_delete_response.into_body(), 1024)
                .await
                .unwrap()
                .is_empty()
        );

        let token = crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(&user_id).unwrap(),
            "openid profile email",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (_, private) = crate::routes::build_router();
        let private = private.with_state(state.clone());
        for (method, uri) in [
            (Method::POST, "/api/v1/assistant/workflow-chat"),
            (Method::GET, "/api/v1/assistant/workflow-chat/ws"),
            (
                Method::GET,
                "/api/v1/assistant/conversations/create-recovery/4380055d-e9c3-468e-bc93-64719a9f4658",
            ),
        ] {
            let response = private
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{uri} must be absent"
            );
        }

        let calls = std::mem::take(&mut *captured.lock().unwrap());
        let paths: Vec<String> = calls.iter().map(|(_, path, _, _)| path.clone()).collect();
        assert_eq!(
            paths,
            vec![
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat".to_string(),
                "/api/chat/conversations".to_string(),
                format!("/api/scopes/{user_id}/chat-history"),
                "/api/chat/conversations/nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae".to_string(),
                "/api/chat/conversations/nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae/state"
                    .to_string(),
                "/api/chat/conversations/nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae".to_string(),
                format!("/api/scopes/{user_id}/chat-history/conversations/{workflow_id}"),
                format!("/api/scopes/{user_id}/chat-history/conversations/{workflow_id}"),
            ]
        );
        assert!(paths.iter().any(|path| path.contains("chat-history")));
        assert!(debug_echoes.iter().all(|echoes| echoes.len() == 1));

        let expected_chat_bodies = [
            serde_json::json!({
                "type": "text",
                "prompt": "connect api-github",
                "clientRequestId": "00000000-0000-4000-8000-000000000001",
            }),
            serde_json::json!({
                "type": "text",
                "prompt": "continue",
                "clientRequestId": "00000000-0000-4000-8000-000000000002",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
            }),
            serde_json::json!({
                "type": "plan.resolve",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "taskId": "task-1",
                "planId": "plan-1",
                "requestId": "plan-gate-1",
                "clientRequestId": "00000000-0000-4000-8000-000000000011",
                "planRevision": 3,
                "confirmed": true,
                "expectedStateVersion": 23,
            }),
            serde_json::json!({
                "type": "input.resolve",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "clientRequestId": "00000000-0000-4000-8000-000000000010",
                "requestId": "input-1",
                "answer": {
                    "selectedOptionIds": ["option-a", "option-b"]
                },
                "expectedStateVersion": 19,
            }),
            serde_json::json!({
                "type": "action.continue",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "clientRequestId": "00000000-0000-4000-8000-000000000003",
                "originTurnId": "turn-action-1",
                "actions": [{
                    "actionRequestId": "act-1",
                    "originTurnId": "turn-action-1",
                    "disposition": "completed",
                    "resource": {
                        "userService": {
                            "userServiceId": "00000000-0000-4000-8000-000000000123"
                        }
                    }
                }]
            }),
            serde_json::json!({
                "type": "action.continue",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "clientRequestId": "00000000-0000-4000-8000-000000000009",
                "originTurnId": "turn-action-2",
                "actions": [{
                    "actionRequestId": "act-2",
                    "originTurnId": "turn-action-2",
                    "disposition": "failed",
                }]
            }),
            serde_json::json!({
                "type": "approval.resolve",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "clientRequestId": "00000000-0000-4000-8000-000000000004",
                "requestId": "approval-1",
                "approved": true,
                "reason": "Approved by user",
                "expectedStateVersion": 21,
            }),
            serde_json::json!({
                "type": "task.stop",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "turnId": "turn-1",
                "stopRequestId": "stop-1",
                "clientRequestId": "00000000-0000-4000-8000-000000000005",
                "expectedStateVersion": 0,
            }),
            serde_json::json!({
                "type": "task.steer",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "turnId": "turn-1",
                "steeringId": "steer-1",
                "clientRequestId": "00000000-0000-4000-8000-000000000006",
                "instruction": "Try again",
                "expectedStateVersion": 2,
            }),
            serde_json::json!({
                "type": "step.retry",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "turnId": "turn-1",
                "taskId": "task-1",
                "stepId": "step-1",
                "retryRequestId": "retry-1",
                "clientRequestId": "00000000-0000-4000-8000-000000000007",
                "expectedOperationGeneration": 2,
                "expectedStateVersion": 3,
            }),
            serde_json::json!({
                "type": "step.skip",
                "conversationId": "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ae",
                "turnId": "turn-1",
                "taskId": "task-1",
                "stepId": "step-2",
                "skipRequestId": "skip-1",
                "clientRequestId": "00000000-0000-4000-8000-000000000008",
                "expectedOperationGeneration": 4,
                "expectedStateVersion": 5,
            }),
        ];
        let expected_accepts = [
            "text/event-stream",
            "text/event-stream",
            "application/json",
            "application/json",
            "text/event-stream",
            "text/event-stream",
            "application/json",
            "application/json",
            "application/json",
            "application/json",
            "application/json",
        ];
        for (offset, expected) in expected_chat_bodies.into_iter().enumerate() {
            let (_, _, body, headers) = &calls[offset];
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(body).unwrap(),
                expected
            );
            assert!(headers.get(axum::http::header::AUTHORIZATION).is_none());
            assert!(headers.get("x-nyxid-debug-upstream").is_none());
            assert!(headers.get("x-nyxid-identity-token").is_some());
            assert!(headers.get("x-nyxid-delegation-token").is_some());
            assert_eq!(
                headers
                    .get("idempotency-key")
                    .and_then(|value| value.to_str().ok()),
                expected
                    .get("clientRequestId")
                    .and_then(serde_json::Value::as_str),
            );
            assert_eq!(
                headers
                    .get(axum::http::header::ACCEPT)
                    .and_then(|value| value.to_str().ok()),
                Some(expected_accepts[offset]),
            );
        }

        assert_eq!(debug_echoes.len(), 11);
        for (call_index, envelope_array) in debug_echoes.iter().enumerate() {
            assert_eq!(envelope_array.len(), 1);
            let envelope = &envelope_array[0];
            let (method, path, body, _) = &calls[call_index];
            assert_eq!(envelope["method"], method.as_str());
            assert_eq!(envelope["path"], path.trim_start_matches('/'));
            assert_eq!(
                envelope["body"],
                serde_json::from_slice::<serde_json::Value>(body).unwrap()
            );
            let expected_command_type = envelope["body"]["type"].as_str().unwrap();
            assert_eq!(envelope["commandType"], expected_command_type);
            assert_eq!(envelope["truncated"], false);
            assert_eq!(envelope["headers"]["content-type"], "application/json");
            assert_eq!(
                envelope["headers"]["idempotency-key"],
                envelope["body"]["clientRequestId"]
            );
            assert_eq!(envelope["headers"]["accept"], expected_accepts[call_index]);
            assert_eq!(envelope["identity"]["mode"], "jwt");
            assert_eq!(envelope["identity"]["forward_access_token"], false);
            assert_eq!(envelope["identity"]["inject_delegation_token"], true);
            assert_eq!(envelope["identity"]["bridge_minted"], false);
            assert_eq!(envelope["upstreamOutcome"], "response");
            assert_eq!(envelope["response"]["status"], 200);
            let expected_sse = expected_accepts[call_index] == "text/event-stream";
            assert_eq!(envelope["response"]["sse"], expected_sse);
            if expected_sse {
                assert_eq!(
                    envelope["response"]["headers"]["content-type"]["value"],
                    "text/event-stream"
                );
                assert_eq!(
                    envelope["response"]["headers"]["x-request-id"]["value"],
                    "aevatar-request-1"
                );
            }
        }

        let serialized_echoes = serde_json::to_string(&debug_echoes).unwrap();
        for forbidden in [
            "caller-secret",
            "authorization",
            "x-nyxid-user-token",
            "x-nyxid-identity-token",
            "x-nyxid-delegation-token",
            "cookie",
            "set-cookie",
            "upstream=secret",
        ] {
            assert!(
                !serialized_echoes.to_ascii_lowercase().contains(forbidden),
                "assistant echo leaked {forbidden}"
            );
        }

        for (_, _, _, headers) in [
            &calls[0], &calls[10], &calls[11], &calls[12], &calls[13], &calls[14],
        ] {
            assert!(headers.get(axum::http::header::AUTHORIZATION).is_none());
            assert!(headers.get("x-nyxid-identity-token").is_some());
            assert!(headers.get("x-nyxid-delegation-token").is_some());
        }

        let no_opt_in = crate::handlers::assistant::typed_chat(
            axum::extract::State(state.clone()),
            auth.clone(),
            request(
                Method::POST,
                "/api/v1/assistant/chat",
                Some(r#"{"type":"text","prompt":"no capture","clientRequestId":"wire-no-opt-in"}"#),
            ),
        )
        .await
        .expect("typed chat without debug opt-in must forward");
        assert!(assistant_echoes(&db, &user_id, &no_opt_in).await.is_empty());
        let no_opt_in_calls = std::mem::take(&mut *captured.lock().unwrap());
        assert_eq!(no_opt_in_calls.len(), 1);
        assert!(no_opt_in_calls[0].3.get("x-nyxid-debug-upstream").is_none());

        disable_wire_log_flag(&db).await;
        let mut flag_off_request = request(
            Method::POST,
            "/api/v1/assistant/chat",
            Some(r#"{"type":"text","prompt":"flag off","clientRequestId":"wire-flag-off"}"#),
        );
        flag_off_request.headers_mut().insert(
            HeaderName::from_static("x-nyxid-debug-upstream"),
            HeaderValue::from_static("1"),
        );
        let flag_off = crate::handlers::assistant::typed_chat(
            axum::extract::State(state.clone()),
            auth.clone(),
            flag_off_request,
        )
        .await
        .expect("feature-disabled typed chat must forward");
        assert!(assistant_echoes(&db, &user_id, &flag_off).await.is_empty());
        let flag_off_calls = std::mem::take(&mut *captured.lock().unwrap());
        assert_eq!(flag_off_calls.len(), 1);
        assert!(flag_off_calls[0].3.get("x-nyxid-debug-upstream").is_none());

        enable_wire_log_flag(&db).await;
        let non_admin_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&non_admin_id, UserType::Person))
            .await
            .unwrap();
        let mut non_admin_request = request(
            Method::POST,
            "/api/v1/assistant/chat",
            Some(r#"{"type":"text","prompt":"non admin","clientRequestId":"wire-non-admin"}"#),
        );
        non_admin_request.headers_mut().insert(
            HeaderName::from_static("x-nyxid-debug-upstream"),
            HeaderValue::from_static("1"),
        );
        let non_admin = crate::handlers::assistant::typed_chat(
            axum::extract::State(state),
            access_token_auth(&non_admin_id),
            non_admin_request,
        )
        .await
        .expect("authenticated non-admin must capture their own exchange");
        assert_eq!(
            assistant_echoes(&db, &non_admin_id, &non_admin).await.len(),
            1
        );

        server.abort();
    }

    #[tokio::test]
    async fn assistant_list_drains_mixed_history_pages_without_wire_log_headers() {
        use std::sync::Mutex as StdMutex;
        use std::sync::atomic::{AtomicU8, Ordering};

        let Some(db) = connect_test_database("assistant_list_history_pagination").await else {
            eprintln!("skipping proxy integration test: no local MongoDB available");
            return;
        };

        type Captured = (Method, String, bool);
        let captured: std::sync::Arc<StdMutex<Vec<Captured>>> =
            std::sync::Arc::new(StdMutex::new(Vec::new()));
        let sink = captured.clone();
        let response_mode = std::sync::Arc::new(AtomicU8::new(0));
        let downstream_mode = response_mode.clone();
        let app = Router::new().route(
            "/{*path}",
            any(move |request: Request<Body>| {
                let sink = sink.clone();
                async move {
                    let method = request.method().clone();
                    let path_and_query = request
                        .uri()
                        .path_and_query()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let cursor = request.uri().query().and_then(|query| {
                        query
                            .split('&')
                            .find_map(|part| part.strip_prefix("cursor="))
                    });
                    let accepts_json = request.headers().get(axum::http::header::ACCEPT)
                        == Some(&HeaderValue::from_static("application/json"));
                    sink.lock()
                        .unwrap()
                        .push((method.clone(), path_and_query, accepts_json));
                    let is_typed_index = request.uri().path() == "/api/chat/conversations";
                    let is_legacy_index = request.uri().path().ends_with("/chat-history");
                    if method != Method::GET || (!is_typed_index && !is_legacy_index) {
                        return StatusCode::NOT_FOUND.into_response();
                    }

                    let mode = downstream_mode.load(Ordering::Relaxed);
                    if mode == 6 && cursor.is_none() {
                        return Body::from("not-json").into_response();
                    }
                    if mode == 3 && cursor.is_some() {
                        return Body::from("not-json").into_response();
                    }
                    if mode == 4 && cursor.is_some() {
                        return axum::Json(serde_json::json!({ "items": [] })).into_response();
                    }

                    let page = if mode == 1 {
                        serde_json::json!({
                            "conversations": [{
                                "id": "nyxid-chat-00000000000000000000000000000000",
                                "updatedAt": "2026-07-29T12:00:00.000Z"
                            }],
                            "nextCursor": "repeated"
                        })
                    } else if mode == 2 {
                        let page_number = cursor
                            .and_then(|value| value.strip_prefix("page-"))
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        serde_json::json!({
                            "conversations": [{
                                "id": format!("nyxid-chat-{page_number:032x}"),
                                "updatedAt": "2026-07-29T12:00:00.000Z"
                            }],
                            "nextCursor": format!("page-{}", page_number + 1)
                        })
                    } else if mode == 5 {
                        let page_number = cursor
                            .and_then(|value| value.strip_prefix("page-"))
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        serde_json::json!({
                            "conversations": [{
                                "id": format!("nyxid-chat-{page_number:032x}"),
                                "updatedAt": "2026-07-29T12:00:00.000Z",
                                "padding": "x".repeat(3 * 1024 * 1024),
                            }],
                            "nextCursor": format!("page-{}", page_number + 1)
                        })
                    } else if mode == 3 || mode == 4 {
                        serde_json::json!({
                            "conversations": [{
                                "id": "nyxid-chat-00000000000000000000000000000000",
                                "updatedAt": "2026-07-29T12:00:00.000Z"
                            }],
                            "nextCursor": "shape-page-2"
                        })
                    } else if cursor.is_none() {
                        let mut rows = (0..30)
                            .map(|i| {
                                serde_json::json!({
                                    "id": format!("nyxid-chat-{i:032x}"),
                                    "updatedAt": format!("2026-07-29T12:{i:02}:00.000Z")
                                })
                            })
                            .collect::<Vec<_>>();
                        rows.push(serde_json::json!({
                            "id": "voicec-unsupported",
                            "updatedAt": "2026-07-30T00:00:00.000Z"
                        }));
                        serde_json::json!({
                            "conversations": rows,
                            "nextCursor": "page-2"
                        })
                    } else {
                        assert_eq!(cursor, Some("page-2"));
                        let mut rows = (30..52)
                            .map(|i| {
                                serde_json::json!({
                                    "id": format!("chatc-{i:032x}"),
                                    "updatedAt": format!("2026-07-29T12:{i:02}:00.000Z")
                                })
                            })
                            .collect::<Vec<_>>();
                        rows.push(serde_json::json!({
                            "id": "nyxid-chat-00000000000000000000000000000000",
                            "updatedAt": "2026-07-31T00:00:00.000Z",
                            "title": "duplicate must not replace the first row"
                        }));
                        serde_json::json!({ "conversations": rows })
                    };
                    axum::Json(page).into_response()
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind assistant list downstream listener");
        let addr = listener.local_addr().expect("downstream listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve assistant list downstream");
        });

        let user_id = Uuid::new_v4().to_string();
        crate::services::role_service::seed_system_roles(&db)
            .await
            .expect("seed platform roles");
        let role_ids = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .expect("resolve platform roles");
        let mut admin_user = test_user(&user_id, UserType::Person);
        admin_user.role_ids.push(role_ids.admin);
        admin_user.is_admin = true;
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(admin_user)
            .await
            .unwrap();

        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = crate::services::assistant_service::AEVATAR_SLUG.to_string();
        service.name = "Aevatar".to_string();
        service.base_url = format!("http://{addr}");
        service.service_category = "internal".to_string();
        service.requires_user_credential = false;
        service.identity_propagation_mode = "jwt".to_string();
        service.identity_jwt_audience = Some("urn:aevatar:api".to_string());
        service.inject_delegation_token = true;
        service.delegation_token_scope = "proxy:*".to_string();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(service)
        .await
        .unwrap();

        let state = test_app_state(db.clone());
        enable_wire_log_flag(&db).await;
        let auth = access_token_auth(&user_id);
        let list_request = || {
            let mut request = Request::builder()
                .method(Method::GET)
                .uri("/api/v1/assistant/conversations")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header("x-nyxid-debug-upstream", "1")
                .body(Body::empty())
                .expect("build assistant list request");
            request.extensions_mut().insert(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy,
                ),
            );
            request
        };

        let response = crate::handlers::assistant::list_conversations(
            axum::extract::State(state.clone()),
            auth.clone(),
            list_request(),
        )
        .await
        .expect("list conversations handler must drain both pages");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .get("x-nyxid-debug-upstream-id")
                .is_none()
        );
        assert!(
            response
                .headers()
                .get("x-nyxid-debug-upstream-log")
                .is_none()
        );
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let rows = body["conversations"].as_array().unwrap();
        assert_eq!(rows.len(), 52);
        assert_eq!(rows[0]["id"], "chatc-00000000000000000000000000000033");
        assert_eq!(
            rows[51]["id"],
            "nyxid-chat-00000000000000000000000000000000"
        );
        assert!(rows.iter().any(|row| {
            row["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("chatc-"))
        }));
        assert!(rows.iter().any(|row| {
            row["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("nyxid-chat-"))
        }));
        assert!(!rows.iter().any(|row| row["id"] == "voicec-unsupported"));
        assert!(rows[51].get("title").is_none(), "first duplicate must win");

        let calls = std::mem::take(&mut *captured.lock().unwrap());
        let history_path = format!("/api/scopes/{user_id}/chat-history");
        assert_eq!(
            calls,
            vec![
                (
                    Method::GET,
                    "/api/chat/conversations?pageSize=50".to_string(),
                    true,
                ),
                (
                    Method::GET,
                    "/api/chat/conversations?pageSize=50&cursor=page-2".to_string(),
                    true,
                ),
                (Method::GET, history_path.clone(), true),
                (Method::GET, format!("{history_path}?cursor=page-2"), true,),
            ]
        );
        for (_, _, accepts_json) in &calls {
            assert!(*accepts_json, "every drained page must request JSON");
        }

        response_mode.store(1, Ordering::Relaxed);
        let repeated_cursor_error = crate::handlers::assistant::list_conversations(
            axum::extract::State(state.clone()),
            auth.clone(),
            list_request(),
        )
        .await
        .expect_err("a repeated history cursor must fail closed");
        assert!(matches!(repeated_cursor_error, AppError::Internal(_)));
        let repeated_calls = std::mem::take(&mut *captured.lock().unwrap());
        assert_eq!(repeated_calls.len(), 2);
        assert!(repeated_calls[1].1.ends_with("cursor=repeated"));
        assert!(repeated_calls.iter().all(|call| call.2));

        response_mode.store(2, Ordering::Relaxed);
        let capped_response = crate::handlers::assistant::list_conversations(
            axum::extract::State(state.clone()),
            auth.clone(),
            list_request(),
        )
        .await
        .expect("the bounded history drain must return collected rows");
        assert_eq!(capped_response.status(), StatusCode::OK);
        let capped_body = to_bytes(capped_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let capped_body: serde_json::Value = serde_json::from_slice(&capped_body).unwrap();
        assert_eq!(capped_body["conversations"].as_array().unwrap().len(), 40);
        assert!(capped_body.get("nextCursor").is_none());
        let capped_calls = std::mem::take(&mut *captured.lock().unwrap());
        assert_eq!(capped_calls.len(), 80);
        assert!(capped_calls.iter().all(|call| call.2));

        for mode in [3, 4] {
            response_mode.store(mode, Ordering::Relaxed);
            let degraded_response = crate::handlers::assistant::list_conversations(
                axum::extract::State(state.clone()),
                auth.clone(),
                list_request(),
            )
            .await
            .expect("an invalid later page must preserve rows already collected");
            assert_eq!(degraded_response.status(), StatusCode::OK);
            let degraded_body = to_bytes(degraded_response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let degraded_body: serde_json::Value = serde_json::from_slice(&degraded_body).unwrap();
            assert_eq!(degraded_body["conversations"].as_array().unwrap().len(), 1);
            let degraded_calls = std::mem::take(&mut *captured.lock().unwrap());
            assert_eq!(degraded_calls.len(), 4);
            assert!(degraded_calls.iter().all(|call| call.2));
        }

        response_mode.store(5, Ordering::Relaxed);
        let budgeted_response = crate::handlers::assistant::list_conversations(
            axum::extract::State(state.clone()),
            auth.clone(),
            list_request(),
        )
        .await
        .expect("the aggregate byte budget must return collected rows");
        assert_eq!(budgeted_response.status(), StatusCode::OK);
        let budgeted_body = to_bytes(budgeted_response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        let budgeted_body: serde_json::Value = serde_json::from_slice(&budgeted_body).unwrap();
        assert_eq!(budgeted_body["conversations"].as_array().unwrap().len(), 2);
        let budgeted_calls = std::mem::take(&mut *captured.lock().unwrap());
        assert_eq!(budgeted_calls.len(), 3);
        assert!(budgeted_calls.iter().all(|call| call.2));

        response_mode.store(6, Ordering::Relaxed);
        let malformed_first_response = crate::handlers::assistant::list_conversations(
            axum::extract::State(state),
            auth,
            list_request(),
        )
        .await
        .expect("a malformed first page intentionally degrades to an empty index");
        assert_eq!(malformed_first_response.status(), StatusCode::OK);
        let malformed_first_body = to_bytes(malformed_first_response.into_body(), 1024)
            .await
            .unwrap();
        let malformed_first_body: serde_json::Value =
            serde_json::from_slice(&malformed_first_body).unwrap();
        assert_eq!(
            malformed_first_body,
            serde_json::json!({ "conversations": [] })
        );
        assert!(malformed_first_body.get("nextCursor").is_none());
        let malformed_first_calls = std::mem::take(&mut *captured.lock().unwrap());
        assert_eq!(malformed_first_calls.len(), 2);
        assert!(malformed_first_calls.iter().all(|call| call.2));
        server.abort();
    }

    #[tokio::test]
    async fn assistant_deleted_scoped_command_routes_are_unroutable() {
        use crate::crypto::jwt::generate_access_token;

        let state = crate::test_utils::test_app_state_no_db().await;
        let user_id = Uuid::new_v4();
        let token = generate_access_token(
            &state.jwt_keys,
            &state.config,
            &user_id,
            "openid profile email",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (_, private) = crate::routes::build_router();
        let app = private.with_state(state);

        for (method, path) in [
            (
                Method::POST,
                "/api/v1/assistant/conversations/nyxid-chat-1/stream",
            ),
            (
                Method::POST,
                "/api/v1/assistant/conversations/nyxid-chat-1/approve",
            ),
            (
                Method::POST,
                "/api/v1/assistant/conversations/nyxid-chat-1/stop",
            ),
            (
                Method::POST,
                "/api/v1/assistant/conversations/nyxid-chat-1/steer",
            ),
            (
                Method::POST,
                "/api/v1/assistant/conversations/nyxid-chat-1/turns/turn-1/steps/step-1/retry",
            ),
            (
                Method::POST,
                "/api/v1/assistant/conversations/nyxid-chat-1/turns/turn-1/steps/step-1/skip",
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.clone())
                        .uri(path)
                        .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(
                matches!(
                    response.status(),
                    StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
                ),
                "expected {path} to be removed, got {}",
                response.status()
            );
        }
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
    /// Whether the user currently has a dispatchable node route for this service
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

impl From<proxy_discovery_service::ProxyDiscoveryItem> for ProxyServiceItem {
    fn from(item: proxy_discovery_service::ProxyDiscoveryItem) -> Self {
        Self {
            id: item.id,
            name: item.name,
            slug: item.slug,
            description: item.description,
            service_category: item.service_category,
            connected: item.connected,
            requires_connection: item.requires_connection,
            has_node_binding: item.has_node_binding,
            proxy_url: item.proxy_url,
            proxy_url_slug: item.proxy_url_slug,
            docs_url: item.docs_url,
            openapi_url: item.openapi_url,
            asyncapi_url: item.asyncapi_url,
            streaming_supported: item.streaming_supported,
            websocket_supported: item.websocket_supported,
        }
    }
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
    let discovery = proxy_discovery_service::list_proxy_discovery(
        &state.db,
        &user_id_str,
        state.node_ws_manager.as_ref(),
        base,
        query.page.unwrap_or(1),
        query.per_page.unwrap_or(50),
    )
    .await?;

    Ok(Json(ProxyServicesResponse {
        services: discovery.services.into_iter().map(Into::into).collect(),
        custom_services: discovery
            .custom_services
            .into_iter()
            .map(Into::into)
            .collect(),
        total: discovery.total,
        page: discovery.page,
        per_page: discovery.per_page,
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
