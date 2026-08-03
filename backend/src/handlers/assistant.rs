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
//! session-authed mount; the assistant needs fourteen.
//!
//! Forwarding goes through `proxy::execute_proxy`, so credential injection,
//! identity propagation, per-agent rate limiting, approval gating, and audit
//! all apply unchanged -- the assistant is not a special case in the data
//! plane (PRD N4). SSE responses stream through it unbuffered.

use axum::{
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    response::Response,
};
use base64::Engine as _;
use serde::Serialize;
use std::collections::HashSet;

use crate::AppState;
use crate::crypto::jwt::{
    MCP_DELEGATION_TOKEN_TTL_SECS, TokenRestrictionClaims, generate_delegated_access_token,
};
use crate::errors::{AppError, AppResult};
use crate::handlers::proxy::execute_admin_proxy;
use crate::models::downstream_service::DownstreamService;
use crate::mw::auth::{AuthMethod, AuthUser, PROXY_SCOPE, scope_allows_rest_proxy};
use crate::services::{assistant_service, feature_flag_service};

/// Conversation indexes carry titles, timestamps, and counts per row, so the
/// list route gets its own buffering headroom before any merge/reshape.
const MAX_CONVERSATION_INDEX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Aggregate page-body budget for one cursor drain. This keeps a valid but
/// adversarial 40-page chain near the pre-drain memory ceiling instead of
/// allowing up to 160 MiB of history rows to accumulate in one request.
const MAX_HISTORY_INDEX_AGGREGATE_BYTES: usize = 8 * 1024 * 1024;
/// Safety ceiling for a corrupt or adversarial upstream cursor chain. The
/// response intentionally exposes only the rows drained through this page;
/// there is no synthetic cursor or truncation flag in NyxID's list contract.
const MAX_HISTORY_INDEX_PAGES: usize = 40;

const DEBUG_UPSTREAM_REQUEST_HEADER: &str = "x-nyxid-debug-upstream";
const DEBUG_UPSTREAM_RESPONSE_HEADER: &str = "x-nyxid-debug-upstream-log";
// Node's default `http.maxHeaderSize` limits the whole response header block,
// which includes this value when Vite's development proxy parses it. Leave
// about 4 KiB for the status line and security headers; production nginx is
// configured for 32 KiB, so local development is the binding constraint.
const DEBUG_UPSTREAM_HEADER_MAX_BYTES: usize = 12 * 1024;
const DEBUG_UPSTREAM_MAX_ECHOES: usize = 8;
const DEBUG_UPSTREAM_PATH_MAX_BYTES: usize = 256;
const DEBUG_UPSTREAM_COMMAND_TYPE_MAX_BYTES: usize = 64;
const DEBUG_UPSTREAM_HEADER_VALUE_MAX_BYTES: usize = 256;
const DEBUG_UPSTREAM_MIN_TRUNCATED_BODY_BYTES: usize = 16;

#[derive(Clone, Debug, Serialize)]
struct UpstreamIdentityEcho {
    mode: String,
    forward_access_token: bool,
    inject_delegation_token: bool,
    bridge_minted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpstreamEcho {
    degraded: bool,
    method: String,
    path: String,
    command_type: Option<String>,
    body: serde_json::Value,
    headers: serde_json::Map<String, serde_json::Value>,
    identity: UpstreamIdentityEcho,
    truncated: bool,
    response: Option<UpstreamResponseEcho>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_outcome: Option<UpstreamOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dropped_headers: Option<bool>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum UpstreamOutcome {
    Response,
    NoResponse,
}

#[derive(Clone, Debug, Serialize)]
struct UpstreamResponseHeaderEcho {
    value: String,
    truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
struct UpstreamResponseEcho {
    status: u16,
    headers: serde_json::Map<String, serde_json::Value>,
    sse: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MinimalUpstreamEcho {
    degraded: bool,
    method: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_outcome: Option<UpstreamOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum EncodedUpstreamEcho {
    Full(Box<UpstreamEcho>),
    Minimal(MinimalUpstreamEcho),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpstreamEchoHeader {
    version: u8,
    echoes: Vec<EncodedUpstreamEcho>,
    dropped_echo_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EchoEncodingRung {
    Full,
    TruncatedBodies,
    DroppedBodies,
    DroppedHeaders,
    Minimal,
    DroppedEchoes,
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

/// Decide whether this request captures a wire-log echo, and start an empty
/// collector when it does.
///
/// **Gate order is load-bearing for latency.** The `X-NyxID-Debug-Upstream: 1`
/// request header is a free in-memory lookup; the feature flag is resolved
/// from MongoDB. Normal chat traffic — the overwhelming majority — never sends
/// the header, so the header is checked first and that traffic performs zero
/// additional database work. This deliberately inverts the previous
/// flag-then-header order, which was only correct while the gate was static
/// process config with no I/O cost.
///
/// Fails **closed**: a flag-resolution error suppresses the echo rather than
/// exposing raw upstream payloads to the browser.
async fn upstream_echo_collector(
    state: &AppState,
    auth_user: &AuthUser,
    request_headers: &HeaderMap,
) -> Option<Vec<UpstreamEcho>> {
    let requested = request_headers
        .get(DEBUG_UPSTREAM_REQUEST_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if !requested {
        return None;
    }
    let enabled = match feature_flag_service::aevatar_chat_wire_log_enabled(
        &state.db,
        &auth_user.user_id.to_string(),
    )
    .await
    {
        Ok(enabled) => enabled,
        Err(_) => {
            // Metadata only — never the request body or any echo content.
            tracing::warn!(
                "assistant: wire-log flag resolution failed; suppressing the debug echo"
            );
            false
        }
    };
    if !enabled {
        return None;
    }
    Some(Vec::new())
}

fn echoed_headers(
    request_headers: &HeaderMap,
    extra_outbound_headers: &[(String, String)],
) -> serde_json::Map<String, serde_json::Value> {
    let mut headers = serde_json::Map::new();
    if let Some(value) = request_headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    {
        let (value, _) = truncate_utf8(value, DEBUG_UPSTREAM_HEADER_VALUE_MAX_BYTES);
        headers.insert("content-type".to_string(), serde_json::Value::String(value));
    }
    for (name, value) in extra_outbound_headers {
        let normalized = name.to_ascii_lowercase();
        if matches!(normalized.as_str(), "idempotency-key" | "accept") {
            let (value, _) = truncate_utf8(value, DEBUG_UPSTREAM_HEADER_VALUE_MAX_BYTES);
            headers.insert(normalized, serde_json::Value::String(value));
        }
    }
    headers
}

fn build_upstream_echo(
    request: &Request<Body>,
    path: String,
    command_type: Option<&str>,
    body: Option<serde_json::Value>,
    extra_outbound_headers: &[(String, String)],
    identity: UpstreamIdentityEcho,
) -> UpstreamEcho {
    // The query string is part of what went on the wire. `list_conversations`
    // drains cursor pages by rewriting only the request URI's query, so
    // without it every drained page echoes an identical path and pagination
    // reads as duplicate calls in the wire-log panel.
    let path = match request.uri().query() {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path,
    };
    let (path, _) = truncate_utf8(&path, DEBUG_UPSTREAM_PATH_MAX_BYTES);
    UpstreamEcho {
        degraded: false,
        method: request.method().as_str().to_string(),
        path,
        command_type: command_type
            .map(|value| truncate_utf8(value, DEBUG_UPSTREAM_COMMAND_TYPE_MAX_BYTES).0),
        body: body.unwrap_or(serde_json::Value::Null),
        headers: echoed_headers(request.headers(), extra_outbound_headers),
        identity,
        truncated: false,
        response: None,
        upstream_outcome: Some(UpstreamOutcome::NoResponse),
        dropped_headers: None,
    }
}

fn response_echo(response: &Response) -> UpstreamResponseEcho {
    let mut headers = serde_json::Map::new();
    for name in ["content-type", "x-request-id", "x-correlation-id"] {
        let Some(value) = response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
        else {
            continue;
        };
        let (value, truncated) = truncate_utf8(value, DEBUG_UPSTREAM_HEADER_VALUE_MAX_BYTES);
        let value = serde_json::to_value(UpstreamResponseHeaderEcho { value, truncated })
            .expect("response header echo is serializable");
        headers.insert(name.to_string(), value);
    }
    let sse = response
        .headers()
        .get_all(header::CONTENT_TYPE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(crate::mw::security_headers::is_sse_media_type);
    UpstreamResponseEcho {
        status: response.status().as_u16(),
        headers,
        sse,
    }
}

fn record_upstream_response(
    collector: Option<&mut Vec<UpstreamEcho>>,
    echo_index: Option<usize>,
    result: &AppResult<Response>,
) {
    let (Some(echoes), Some(index), Ok(response)) = (collector, echo_index, result) else {
        return;
    };
    if let Some(echo) = echoes.get_mut(index) {
        echo.response = Some(response_echo(response));
        echo.upstream_outcome = Some(UpstreamOutcome::Response);
    }
}

fn serialize_echoes(header: &UpstreamEchoHeader) -> AppResult<Vec<u8>> {
    serde_json::to_vec(header)
        .map_err(|_| AppError::Internal("assistant: failed to encode upstream echoes".to_string()))
}

fn full_header(echoes: &[UpstreamEcho]) -> UpstreamEchoHeader {
    UpstreamEchoHeader {
        version: 2,
        echoes: echoes
            .iter()
            .cloned()
            .map(Box::new)
            .map(EncodedUpstreamEcho::Full)
            .collect(),
        dropped_echo_count: 0,
    }
}

fn minimal_echo(echo: &UpstreamEcho) -> MinimalUpstreamEcho {
    MinimalUpstreamEcho {
        degraded: true,
        method: truncate_utf8(&echo.method, 16).0,
        path: truncate_utf8(&echo.path, DEBUG_UPSTREAM_PATH_MAX_BYTES).0,
        command_type: echo
            .command_type
            .as_deref()
            .map(|value| truncate_utf8(value, DEBUG_UPSTREAM_COMMAND_TYPE_MAX_BYTES).0),
        upstream_outcome: echo.upstream_outcome,
        status: echo.response.as_ref().map(|response| response.status),
    }
}

fn encode_header(header: &UpstreamEchoHeader) -> Option<String> {
    Some(base64::engine::general_purpose::STANDARD.encode(serialize_echoes(header).ok()?))
}

fn encoded_header_if_fits(
    header: &UpstreamEchoHeader,
    rung: EchoEncodingRung,
    max_bytes: usize,
) -> Option<(HeaderValue, EchoEncodingRung)> {
    let encoded = encode_header(header)?;
    if encoded.len() > max_bytes {
        return None;
    }
    Some((HeaderValue::from_str(&encoded).ok()?, rung))
}

fn encode_echo_header_with_rung(
    echoes: &[UpstreamEcho],
) -> Option<(HeaderValue, EchoEncodingRung)> {
    encode_echo_header_with_limit(echoes, DEBUG_UPSTREAM_HEADER_MAX_BYTES)
}

fn encode_echo_header_with_limit(
    echoes: &[UpstreamEcho],
    max_bytes: usize,
) -> Option<(HeaderValue, EchoEncodingRung)> {
    let mut candidates = echoes.to_vec();
    if let Some(encoded) =
        encoded_header_if_fits(&full_header(&candidates), EchoEncodingRung::Full, max_bytes)
    {
        return Some(encoded);
    }

    let mut bodies = candidates
        .iter()
        .enumerate()
        .filter(|(_, echo)| !echo.body.is_null())
        .filter_map(|(index, echo)| {
            serde_json::to_string(&echo.body)
                .ok()
                .map(|body| (index, body))
        })
        .collect::<Vec<_>>();
    bodies.sort_by_key(|(_, body)| std::cmp::Reverse(body.len()));
    for (index, body) in bodies {
        candidates[index].body = serde_json::Value::String(String::new());
        candidates[index].truncated = true;
        if encoded_header_if_fits(
            &full_header(&candidates),
            EchoEncodingRung::TruncatedBodies,
            max_bytes,
        )
        .is_none()
        {
            continue;
        }

        let mut low = 0;
        let mut high = body.len();
        let mut best = String::new();
        while low <= high {
            let middle = low + (high - low) / 2;
            let (prefix, _) = truncate_utf8(&body, middle);
            candidates[index].body = serde_json::Value::String(prefix);
            if encoded_header_if_fits(
                &full_header(&candidates),
                EchoEncodingRung::TruncatedBodies,
                max_bytes,
            )
            .is_some()
            {
                best = candidates[index]
                    .body
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                low = middle + 1;
            } else if middle == 0 {
                break;
            } else {
                high = middle - 1;
            }
        }
        candidates[index].body = serde_json::Value::String(best.clone());
        if best.len() >= DEBUG_UPSTREAM_MIN_TRUNCATED_BODY_BYTES {
            return encoded_header_if_fits(
                &full_header(&candidates),
                EchoEncodingRung::TruncatedBodies,
                max_bytes,
            );
        }
    }

    // JSON null is two bytes larger than an empty JSON string, but this rung
    // remains reachable because the prior rung deliberately refuses prefixes
    // shorter than 16 bytes. When only a tiny prefix fits, null reports the
    // body loss honestly instead of retaining a misleading sliver of JSON.
    candidates = echoes.to_vec();
    for echo in &mut candidates {
        if !echo.body.is_null() {
            echo.body = serde_json::Value::Null;
            echo.truncated = true;
        }
    }
    if let Some(encoded) = encoded_header_if_fits(
        &full_header(&candidates),
        EchoEncodingRung::DroppedBodies,
        max_bytes,
    ) {
        return Some(encoded);
    }

    for echo in &mut candidates {
        echo.headers.clear();
        if let Some(response) = &mut echo.response {
            response.headers.clear();
        }
        echo.dropped_headers = Some(true);
    }
    if let Some(encoded) = encoded_header_if_fits(
        &full_header(&candidates),
        EchoEncodingRung::DroppedHeaders,
        max_bytes,
    ) {
        return Some(encoded);
    }

    let mut minimal = UpstreamEchoHeader {
        version: 2,
        echoes: echoes
            .iter()
            .map(minimal_echo)
            .map(EncodedUpstreamEcho::Minimal)
            .collect(),
        dropped_echo_count: 0,
    };
    if let Some(encoded) = encoded_header_if_fits(&minimal, EchoEncodingRung::Minimal, max_bytes) {
        return Some(encoded);
    }

    let dropped = minimal
        .echoes
        .len()
        .saturating_sub(DEBUG_UPSTREAM_MAX_ECHOES);
    minimal.echoes.truncate(DEBUG_UPSTREAM_MAX_ECHOES);
    minimal.dropped_echo_count = u32::try_from(dropped).unwrap_or(u32::MAX);
    encoded_header_if_fits(&minimal, EchoEncodingRung::DroppedEchoes, max_bytes)
}

fn encode_echo_header(echoes: &[UpstreamEcho]) -> Option<HeaderValue> {
    encode_echo_header_with_rung(echoes).map(|(value, _)| value)
}

fn attach_upstream_echoes(mut response: Response, echoes: Option<&[UpstreamEcho]>) -> Response {
    if let Some(value) = echoes
        .filter(|echoes| !echoes.is_empty())
        .and_then(encode_echo_header)
    {
        response
            .headers_mut()
            .insert(DEBUG_UPSTREAM_RESPONSE_HEADER, value);
    }
    response
}

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
    let mut request = builder.body(Body::empty())?;
    request.extensions_mut().insert(
        crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Proxy,
        ),
    );
    Ok(request)
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

/// Build the `Authorization: Bearer <delegated token>` value the bridge
/// forwards to Aevatar. Extracted as a seam so the exact security-sensitive
/// wiring (delegated scope, actor = the Aevatar slug, inherited service
/// restrictions, delegation TTL) is unit-testable without a live proxy or DB.
///
/// The delegated capability prefers the service row's `delegation_token_scope`
/// — the single source of truth the standard `inject_delegation_token` path
/// reads (`proxy.rs`) — so the token this bridge delivers in `Authorization`
/// and the token the standard path delivers in `X-NyxID-Delegation-Token`
/// grant the same capability when the row is aligned. The only deviation from
/// the standard is the delivery header (dictated by Aevatar's deployed
/// validator reusing `Authorization`).
///
/// The scope MUST grant REST proxy access, because Aevatar's LLM callback
/// arrives as a `/proxy/s/{slug}` passthrough enforcing
/// `ensure_rest_proxy_access`. If the row does not (e.g. the historical
/// `llm:proxy` default), fall back to `PROXY_SCOPE` — the assistant must work
/// on deploy without a coupled DB change, and a hard failure here would take
/// the whole surface down over one config field. This is an intentional,
/// transitional compatibility override of an insufficient row value: the
/// minimum capability is dictated by the integration (Aevatar's callback
/// path), and every REST-capable configured scope is still honored verbatim.
/// The warning fires once per process (not per request) to flag the drift
/// without spamming while every call exercises the fallback.
fn resolve_forward_scope(service: &DownstreamService) -> &str {
    if scope_allows_rest_proxy(&service.delegation_token_scope) {
        return service.delegation_token_scope.as_str();
    }
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            service_slug = %service.slug,
            configured_scope = %service.delegation_token_scope,
            "assistant: aevatar delegation_token_scope does not grant REST proxy; \
             falling back to 'proxy' for the callback bridge. Set the row's \
             delegation_token_scope to 'proxy' to align it before the TD-3 \
             identity-token cutover."
        );
    });
    PROXY_SCOPE
}

fn build_forward_authorization(
    state: &AppState,
    auth_user: &AuthUser,
    service: &DownstreamService,
) -> AppResult<HeaderValue> {
    let restrictions = TokenRestrictionClaims::from_auth_user(auth_user);
    let token = generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &auth_user.user_id,
        resolve_forward_scope(service),
        &service.slug,
        MCP_DELEGATION_TOKEN_TTL_SECS,
        Some(&restrictions),
    )?;
    HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
        AppError::Internal(
            "assistant: failed to build the forward authorization header".to_string(),
        )
    })
}

/// Resolve the admin-managed Aevatar service and forward `path` to it.
///
/// `path` is always built server-side by the callers below from
/// `auth_user.user_id`; no caller-supplied scope reaches this function.
struct ForwardEcho<'a> {
    command_type: Option<&'a str>,
    body: Option<serde_json::Value>,
    collector: Option<&'a mut Vec<UpstreamEcho>>,
}

impl<'a> ForwardEcho<'a> {
    fn enabled(
        command_type: Option<&'a str>,
        body: Option<serde_json::Value>,
        collector: Option<&'a mut Vec<UpstreamEcho>>,
    ) -> Self {
        Self {
            command_type,
            body,
            collector,
        }
    }

    fn disabled() -> Self {
        Self {
            command_type: None,
            body: None,
            collector: None,
        }
    }
}

async fn forward(
    state: &AppState,
    auth_user: &AuthUser,
    path: String,
    mut request: Request<Body>,
    extra_outbound_headers: Vec<(String, String)>,
    echo: ForwardEcho<'_>,
) -> AppResult<Response> {
    let ForwardEcho {
        command_type,
        body,
        mut collector,
    } = echo;
    request.headers_mut().remove(DEBUG_UPSTREAM_REQUEST_HEADER);
    let service = assistant_service::resolve_admin_service(&state.db).await?;
    let bridge_minted =
        needs_forward_token_bridge(&auth_user.auth_method, service.forward_access_token);
    let echo_index = if let Some(echoes) = collector.as_deref_mut() {
        let index = echoes.len();
        echoes.push(build_upstream_echo(
            &request,
            path.clone(),
            command_type,
            body,
            &extra_outbound_headers,
            UpstreamIdentityEcho {
                mode: service.identity_propagation_mode.clone(),
                forward_access_token: service.forward_access_token,
                inject_delegation_token: service.inject_delegation_token,
                bridge_minted,
            },
        ));
        Some(index)
    } else {
        None
    };

    // TD-3 bridge: cookie sessions carry no bearer for `forward_access_token`
    // to forward, and prod Aevatar authenticates only `Authorization: Bearer
    // <NyxID JWT>`. Mint a DELEGATED access token and OVERWRITE Authorization
    // — `AuthMethod::Session` means bearer auth did not happen, so any header
    // present is not an authenticated credential. A delegated token is the
    // platform standard for "downstream calls NyxID on the user's behalf":
    // Aevatar reuses this same bearer to reach NyxID's LLM/proxy routes
    // (`/proxy/s/chrono-llm-public`, `/llm/*`), which the delegated router
    // accepts, while `reject_delegated_tokens` keeps a leaked copy off every
    // account-management, admin, and key surface. The delegated capability
    // comes from the row's `delegation_token_scope` (same source of truth as
    // the standard `inject_delegation_token` path). Bearer callers (CLI login
    // JWTs) never enter this branch and keep their token byte-for-byte.
    if bridge_minted {
        let value = build_forward_authorization(state, auth_user, &service)?;
        request.headers_mut().insert(header::AUTHORIZATION, value);
        // Metadata-only: lets operators watch bridge dependence fall to
        // zero after the Aevatar identity-token rollout (TD-3 row flip).
        tracing::debug!(
            service_slug = %service.slug,
            "assistant_delegation_token_minted"
        );
    }
    // Addressing the catalog service by id drives the DownstreamService
    // (admin/master-credential) resolution path. Never route by slug here:
    // the slug resolver would prefer a caller-owned `UserService`.
    //
    // `execute_admin_proxy` (not `execute_proxy`) because the caller never
    // named this target: the platform did. Caller-owned routing state must
    // not decide whether a platform surface works — see its doc comment for
    // the three inputs it switches off and why the delegation token still
    // carries the caller's restrictions.
    let mut resolved_slug = String::new();
    let response = execute_admin_proxy(
        state,
        auth_user,
        &service.id,
        &path,
        request,
        extra_outbound_headers,
        &mut resolved_slug,
    )
    .await;
    record_upstream_response(collector, echo_index, &response);
    response
}

/// `GET /api/v1/assistant/conversations` -- fully drained shared Chat History
/// index, filtered to the typed and workflow conversation families.
pub async fn list_conversations(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let authorization = request.headers().get(header::AUTHORIZATION).cloned();
    let mut echoes = upstream_echo_collector(&state, &auth_user, request.headers()).await;
    let user_id = auth_user.user_id.to_string();
    let mut response_parts = None;
    let mut conversations = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_cursors = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut aggregate_page_bytes = 0usize;

    for _ in 0..MAX_HISTORY_INDEX_PAGES {
        let mut page_request =
            synthetic_request(Method::GET, authorization.as_ref()).map_err(|_| {
                AppError::Internal(
                    "assistant: failed to build the history list request".to_string(),
                )
            })?;
        page_request
            .headers_mut()
            .insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        if let Some(cursor) = cursor.as_deref() {
            *page_request.uri_mut() = format!("/?cursor={}", urlencoding::encode(cursor))
                .parse()
                .map_err(|_| {
                    AppError::Internal("assistant: failed to encode the history cursor".to_string())
                })?;
        }
        let response = forward(
            &state,
            &auth_user,
            assistant_service::history_index_path(&user_id),
            page_request,
            Vec::new(),
            ForwardEcho::enabled(None, None, echoes.as_mut()),
        )
        .await?;
        if !response.status().is_success() {
            return Ok(attach_upstream_echoes(response, echoes.as_deref()));
        }
        let (parts, body) = response.into_parts();
        if response_parts.is_none() {
            response_parts = Some(parts);
        }
        let bytes = to_bytes(body, MAX_CONVERSATION_INDEX_RESPONSE_BYTES)
            .await
            .map_err(|_| {
                AppError::Internal(
                    "assistant: conversation index page exceeded the buffer cap".to_string(),
                )
            })?;
        let Some(next_aggregate_bytes) = aggregate_page_bytes.checked_add(bytes.len()) else {
            break;
        };
        if next_aggregate_bytes > MAX_HISTORY_INDEX_AGGREGATE_BYTES {
            break;
        }
        aggregate_page_bytes = next_aggregate_bytes;
        let Ok(page) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            // Preserve already-collected rows across a mixed-version or
            // partially deployed upstream response shape. On the first page,
            // the same deploy-independent posture intentionally degrades to an
            // empty successful index using that upstream response's metadata.
            break;
        };
        let next_cursor = assistant_service::append_addressable_history_page(
            &page,
            &mut conversations,
            &mut seen_ids,
        )?;
        let Some(next_cursor) = next_cursor else {
            break;
        };
        if !seen_cursors.insert(next_cursor.clone()) {
            return Err(AppError::Internal(
                "assistant: chat history index repeated a cursor".to_string(),
            ));
        }
        cursor = Some(next_cursor);
    }

    assistant_service::sort_conversation_rows_newest_first(&mut conversations);
    let filtered = serde_json::to_vec(&serde_json::json!({
        "conversations": conversations,
    }))
    .map_err(|_| {
        AppError::Internal("assistant: failed to encode the conversation index".to_string())
    })?;
    let mut parts = response_parts.ok_or_else(|| {
        AppError::Internal("assistant: conversation index returned no pages".to_string())
    })?;
    parts.headers.remove(header::CONTENT_LENGTH);
    Ok(attach_upstream_echoes(
        Response::from_parts(parts, Body::from(filtered)),
        echoes.as_deref(),
    ))
}

/// `GET /api/v1/assistant/conversations/{id}` -- family-aware conversation
/// transcript wrapper.
///
/// The body is opaque here: NyxID never parses or reshapes it. Aevatar PR
/// #2923 wrapped the flat `[StoredChatMessage]` array in
/// `{messages, stateVersion}`, and both shapes stream through this route
/// unmodified -- the client owns the decoding (see the transcript reader in
/// `frontend/src/lib/assistant/aevatar-transport.ts`). Keeping the route
/// shape-agnostic is what lets Aevatar and NyxID deploy independently.
pub async fn get_history(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let mut echoes = upstream_echo_collector(&state, &auth_user, request.headers()).await;
    let user_id = auth_user.user_id.to_string();
    let path = match assistant_service::conversation_resource_family(&conversation_id)? {
        assistant_service::ConversationResourceFamily::Typed => {
            assistant_service::canonical_conversation_path(&conversation_id)?
        }
        assistant_service::ConversationResourceFamily::Workflow => {
            assistant_service::history_conversation_path(&user_id, &conversation_id)?
        }
    };
    let response = forward(
        &state,
        &auth_user,
        path,
        request,
        Vec::new(),
        ForwardEcho::enabled(None, None, echoes.as_mut()),
    )
    .await?;
    Ok(attach_upstream_echoes(response, echoes.as_deref()))
}

/// `DELETE /api/v1/assistant/conversations/{id}` -- typed composite lifecycle
/// delete or workflow Chat History delete, selected by id family.
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let mut echoes = upstream_echo_collector(&state, &auth_user, request.headers()).await;
    let user_id = auth_user.user_id.to_string();
    let family = assistant_service::conversation_resource_family(&conversation_id)?;
    let path = match family {
        assistant_service::ConversationResourceFamily::Typed => {
            assistant_service::canonical_conversation_path(&conversation_id)?
        }
        assistant_service::ConversationResourceFamily::Workflow => {
            assistant_service::history_conversation_path(&user_id, &conversation_id)?
        }
    };
    let response = forward(
        &state,
        &auth_user,
        path,
        request,
        Vec::new(),
        ForwardEcho::enabled(None, None, echoes.as_mut()),
    )
    .await?;
    if family == assistant_service::ConversationResourceFamily::Workflow
        && response.status().is_success()
    {
        let (mut parts, _) = response.into_parts();
        parts.status = StatusCode::NO_CONTENT;
        parts.headers.remove(header::CONTENT_LENGTH);
        parts.headers.remove(header::CONTENT_TYPE);
        return Ok(attach_upstream_echoes(
            Response::from_parts(parts, Body::empty()),
            echoes.as_deref(),
        ));
    }
    Ok(attach_upstream_echoes(response, echoes.as_deref()))
}

/// `GET /api/v1/assistant/conversations/{id}/state` -- conditional
/// current-state query. The `afterStateVersion` / `turnId` cursors ride the
/// forwarded query string (`execute_proxy` preserves it); results are the
/// contract's `current` / `not_modified` / `reload_required` / `not_found`
/// envelope. This is the reconnect surface for a page reload mid-turn.
pub async fn get_state(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    if assistant_service::conversation_resource_family(&conversation_id)?
        == assistant_service::ConversationResourceFamily::Workflow
    {
        return Err(AppError::NotFound(
            "Conversation state not found.".to_string(),
        ));
    }
    let mut echoes = upstream_echo_collector(&state, &auth_user, request.headers()).await;
    let path = assistant_service::canonical_state_path(&conversation_id)?;
    let response = forward(
        &state,
        &auth_user,
        path,
        request,
        Vec::new(),
        ForwardEcho::enabled(None, None, echoes.as_mut()),
    )
    .await?;
    Ok(attach_upstream_echoes(response, echoes.as_deref()))
}

/// `GET /api/v1/assistant/conversations/create-recovery/{commandId}` --
/// workflow create identity recovery from scoped Chat History.
pub async fn get_create_recovery(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(command_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let mut echoes = upstream_echo_collector(&state, &auth_user, request.headers()).await;
    let path = assistant_service::history_create_recovery_path(
        &auth_user.user_id.to_string(),
        &command_id,
    )?;
    let response = forward(
        &state,
        &auth_user,
        path,
        request,
        Vec::new(),
        ForwardEcho::enabled(None, None, echoes.as_mut()),
    )
    .await?;
    Ok(attach_upstream_echoes(response, echoes.as_deref()))
}

/// `POST /api/v1/assistant/completions` -- OpenAI-compatible SSE stream.
pub async fn completions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let mut echoes = upstream_echo_collector(&state, &auth_user, request.headers()).await;
    let response = forward(
        &state,
        &auth_user,
        assistant_service::completions_path(),
        request,
        Vec::new(),
        ForwardEcho::enabled(None, None, echoes.as_mut()),
    )
    .await?;
    Ok(attach_upstream_echoes(response, echoes.as_deref()))
}

/// Bounds caller chat bodies: a 32k-char prompt is at most 128 KiB of UTF-8,
/// with additional room for JSON escaping.
const MAX_ASSISTANT_CHAT_REQUEST_BYTES: usize = 256 * 1024;

/// `POST /api/v1/assistant/chat` -- typed NyxIdChat create-and-first-turn SSE.
///
/// The browser request is parsed with an explicit command allowlist, rebuilt
/// into the exact canonical `/api/chat` body, and forwarded with an
/// internally-derived `Idempotency-Key` and canonical `Accept` header.
pub async fn typed_chat(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_ASSISTANT_CHAT_REQUEST_BYTES)
        .await
        .map_err(|_| {
            AppError::BadRequest("Assistant chat request body is too large.".to_string())
        })?;
    let command = assistant_service::parse_assistant_chat_command(&bytes)?;
    let prepared = assistant_service::prepare_assistant_chat_command(&command)?;
    let payload = serde_json::to_vec(&prepared.body).map_err(|_| {
        AppError::Internal("assistant: failed to encode the assistant chat body".to_string())
    })?;
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let request = Request::from_parts(parts, Body::from(payload));
    let extra_outbound_headers = vec![
        (
            "idempotency-key".to_string(),
            prepared.client_request_id.clone(),
        ),
        (
            "accept".to_string(),
            prepared.response_kind.accept_header_value().to_string(),
        ),
    ];
    let mut echoes = upstream_echo_collector(&state, &auth_user, request.headers()).await;
    let echoed_body = echoes.as_ref().map(|_| prepared.body.clone());
    let response = forward(
        &state,
        &auth_user,
        assistant_service::typed_chat_path(),
        request,
        extra_outbound_headers,
        ForwardEcho::enabled(
            prepared
                .body
                .get("type")
                .and_then(serde_json::Value::as_str),
            echoed_body,
            echoes.as_mut(),
        ),
    )
    .await?;
    Ok(attach_upstream_echoes(response, echoes.as_deref()))
}

/// `POST /api/v1/assistant/workflow-chat` -- workflow ("studio") chat turn,
/// answered as the upstream SSE stream.
///
/// The caller body is the typed `WorkflowChatTurnRequest`: prompt, session,
/// create-only command id, or a conversation id plus observed state fence.
/// The upstream `/api/chat` body is built server-side with the workflow pinned
/// to `studio` and the `conversation` object always present, so every turn
/// persists to chat history and no caller can select another engine or smuggle
/// fields into Aevatar's strict `HttpChatInput`. Scope comes from the
/// propagated identity token (Aevatar ignores any body scope).
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
    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_ASSISTANT_CHAT_REQUEST_BYTES)
        .await
        .map_err(|_| {
            AppError::BadRequest("Workflow chat request body is too large.".to_string())
        })?;
    let turn: assistant_service::WorkflowChatTurnRequest = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::BadRequest(format!("Invalid workflow chat request: {e}")))?;
    let upstream = assistant_service::workflow_chat_body(&turn)?;
    let payload = serde_json::to_vec(&upstream).map_err(|_| {
        AppError::Internal("assistant: failed to encode the workflow chat body".to_string())
    })?;
    // The upstream length is the rebuilt body's, not the caller's.
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let request = Request::from_parts(parts, Body::from(payload));
    let mut echoes = upstream_echo_collector(&state, &auth_user, request.headers()).await;
    let echoed_body = echoes.as_ref().map(|_| upstream.clone());
    let response = forward(
        &state,
        &auth_user,
        assistant_service::workflow_chat_path(),
        request,
        Vec::new(),
        ForwardEcho::enabled(Some("workflow.studio"), echoed_body, echoes.as_mut()),
    )
    .await?;
    Ok(attach_upstream_echoes(response, echoes.as_deref()))
}

/// `GET /api/v1/assistant/workflow-chat/ws` -- WebSocket twin of the workflow
/// chat. The proxy detects the upgrade headers and bridges the socket;
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
        Vec::new(),
        ForwardEcho::disabled(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_echo(body: serde_json::Value) -> UpstreamEcho {
        UpstreamEcho {
            degraded: false,
            method: "POST".to_string(),
            path: "api/chat".to_string(),
            command_type: Some("task.steer".to_string()),
            body,
            headers: serde_json::Map::from_iter([
                (
                    "accept".to_string(),
                    serde_json::Value::String("application/json".to_string()),
                ),
                (
                    "content-type".to_string(),
                    serde_json::Value::String("application/json".to_string()),
                ),
            ]),
            identity: UpstreamIdentityEcho {
                mode: "jwt".to_string(),
                forward_access_token: false,
                inject_delegation_token: true,
                bridge_minted: false,
            },
            truncated: false,
            response: None,
            upstream_outcome: Some(UpstreamOutcome::NoResponse),
            dropped_headers: None,
        }
    }

    fn decode_echo_header(value: &HeaderValue) -> serde_json::Value {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(value.as_bytes())
            .expect("echo header is base64");
        serde_json::from_slice(&bytes).expect("echo header is JSON")
    }

    #[test]
    fn echo_uses_an_allowlist_before_identity_injection() {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/")
            .header(header::AUTHORIZATION, "Bearer caller-secret")
            .header(header::COOKIE, "session=private")
            .header("x-nyxid-user-token", "identity-secret")
            .header(header::ACCEPT, "text/event-stream")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"type":"text","prompt":"hello"}"#))
            .unwrap();
        let echo = build_upstream_echo(
            &request,
            "api/chat".to_string(),
            Some("text"),
            Some(serde_json::json!({ "type": "text", "prompt": "hello" })),
            &[
                ("idempotency-key".to_string(), "request-1".to_string()),
                ("accept".to_string(), "text/event-stream".to_string()),
                ("authorization".to_string(), "Bearer injected".to_string()),
                (
                    "x-nyxid-identity-token".to_string(),
                    "injected-identity".to_string(),
                ),
            ],
            UpstreamIdentityEcho {
                mode: "jwt".to_string(),
                forward_access_token: false,
                inject_delegation_token: true,
                bridge_minted: false,
            },
        );

        assert_eq!(
            request.headers().get(header::AUTHORIZATION),
            Some(&HeaderValue::from_static("Bearer caller-secret"))
        );
        assert_eq!(
            echo.headers,
            serde_json::Map::from_iter([
                (
                    "idempotency-key".to_string(),
                    serde_json::Value::String("request-1".to_string()),
                ),
                (
                    "accept".to_string(),
                    serde_json::Value::String("text/event-stream".to_string()),
                ),
                (
                    "content-type".to_string(),
                    serde_json::Value::String("application/json".to_string()),
                ),
            ])
        );
        let serialized =
            String::from_utf8(serialize_echoes(&full_header(&[echo])).unwrap()).unwrap();
        for secret in [
            "caller-secret",
            "session=private",
            "identity-secret",
            "Bearer injected",
            "injected-identity",
            "authorization",
            "cookie",
            "x-nyxid-user-token",
        ] {
            assert!(!serialized.contains(secret), "echo leaked {secret}");
        }
    }

    #[test]
    fn echo_path_carries_the_request_query_string() {
        // `list_conversations` drains the history index by setting the cursor
        // on the synthetic request URI while passing the same bare index path
        // for every page. Without the query, each drained page echoes an
        // identical path and pagination reads as duplicate calls.
        let request = Request::builder()
            .method(Method::GET)
            .uri("/?cursor=abc")
            .body(Body::empty())
            .unwrap();

        let echo = build_upstream_echo(
            &request,
            "api/scopes/user-1/chat-history".to_string(),
            None,
            None,
            &[],
            UpstreamIdentityEcho {
                mode: "jwt".to_string(),
                forward_access_token: false,
                inject_delegation_token: true,
                bridge_minted: false,
            },
        );

        assert_eq!(echo.path, "api/scopes/user-1/chat-history?cursor=abc");
    }

    #[test]
    fn echo_path_has_no_trailing_separator_without_a_query() {
        let request = Request::builder()
            .method(Method::GET)
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let echo = build_upstream_echo(
            &request,
            "api/scopes/user-1/chat-history".to_string(),
            None,
            None,
            &[],
            UpstreamIdentityEcho {
                mode: "jwt".to_string(),
                forward_access_token: false,
                inject_delegation_token: true,
                bridge_minted: false,
            },
        );

        assert_eq!(echo.path, "api/scopes/user-1/chat-history");
    }

    /// A valid, well-formed capture request. Deliberately *not* an
    /// invalid-UTF-8 header value: that would trip the header parse gate and
    /// make a flag assertion vacuous.
    fn capture_request_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(DEBUG_UPSTREAM_REQUEST_HEADER, HeaderValue::from_static("1"));
        headers
    }

    /// Register a person account so per-user platform overrides can target it.
    async fn seed_flag_user(db: &mongodb::Database) -> String {
        use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(&user_id, UserType::Person))
            .await
            .expect("insert wire-log flag test user");
        user_id
    }

    #[tokio::test]
    async fn code_default_suppresses_a_valid_capture_request() {
        let Some(db) = crate::test_utils::connect_test_database("assistant_wire_log_default").await
        else {
            eprintln!("skipping assistant wire-log flag test: no local MongoDB available");
            return;
        };
        let user_id = seed_flag_user(&db).await;
        let state = crate::test_utils::test_app_state(db);
        let auth_user = crate::test_utils::test_auth_user(&user_id);

        // No override rows at all: the registry default (off) must win even
        // though the caller asked for a capture with a valid header.
        let echoes = upstream_echo_collector(&state, &auth_user, &capture_request_headers()).await;

        assert!(echoes.is_none());
    }

    #[tokio::test]
    async fn global_override_enables_the_capture_request() {
        use crate::services::feature_flag_service::{
            AEVATAR_CHAT_WIRE_LOG_FLAG_KEY, FlagTarget, clear_platform_override,
            set_platform_override,
        };

        let Some(db) = crate::test_utils::connect_test_database("assistant_wire_log_global").await
        else {
            eprintln!("skipping assistant wire-log flag test: no local MongoDB available");
            return;
        };
        let user_id = seed_flag_user(&db).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let auth_user = crate::test_utils::test_auth_user(&user_id);

        set_platform_override(
            &db,
            AEVATAR_CHAT_WIRE_LOG_FLAG_KEY,
            &FlagTarget::Global,
            true,
            "admin",
        )
        .await
        .expect("enable the wire-log flag globally");

        let echoes = upstream_echo_collector(&state, &auth_user, &capture_request_headers()).await;
        assert!(matches!(echoes, Some(ref values) if values.is_empty()));

        // Clearing the override at runtime takes effect on the next request
        // with no redeploy — the whole point of the DB-backed gate.
        clear_platform_override(&db, AEVATAR_CHAT_WIRE_LOG_FLAG_KEY, &FlagTarget::Global)
            .await
            .expect("clear the global override");
        assert!(
            upstream_echo_collector(&state, &auth_user, &capture_request_headers())
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn per_user_override_enables_only_that_user() {
        use crate::services::feature_flag_service::{
            AEVATAR_CHAT_WIRE_LOG_FLAG_KEY, FlagTarget, set_platform_override,
        };

        let Some(db) = crate::test_utils::connect_test_database("assistant_wire_log_user").await
        else {
            eprintln!("skipping assistant wire-log flag test: no local MongoDB available");
            return;
        };
        let flagged_id = seed_flag_user(&db).await;
        let other_id = seed_flag_user(&db).await;
        let state = crate::test_utils::test_app_state(db.clone());

        set_platform_override(
            &db,
            AEVATAR_CHAT_WIRE_LOG_FLAG_KEY,
            &FlagTarget::User(flagged_id.clone()),
            true,
            "admin",
        )
        .await
        .expect("enable the wire-log flag for one user");

        let flagged = crate::test_utils::test_auth_user(&flagged_id);
        assert!(
            upstream_echo_collector(&state, &flagged, &capture_request_headers())
                .await
                .is_some(),
            "the targeted user must get the echo"
        );

        let other = crate::test_utils::test_auth_user(&other_id);
        assert!(
            upstream_echo_collector(&state, &other, &capture_request_headers())
                .await
                .is_none(),
            "a per-user override must not leak to another caller"
        );
    }

    /// An `AppState` whose MongoDB handle points at a closed loopback port, so
    /// any database access costs at least the server-selection timeout and
    /// then fails.
    async fn unreachable_db_state() -> AppState {
        let mut options = mongodb::options::ClientOptions::parse(UNREACHABLE_MONGO_URI)
            .await
            .expect("parse unreachable mongo uri");
        options.server_selection_timeout = Some(std::time::Duration::from_millis(
            UNREACHABLE_MONGO_SELECTION_TIMEOUT_MS,
        ));
        options.connect_timeout = Some(std::time::Duration::from_millis(
            UNREACHABLE_MONGO_SELECTION_TIMEOUT_MS,
        ));
        let client =
            mongodb::Client::with_options(options).expect("build unreachable mongo client");
        crate::test_utils::test_app_state(client.database("nyxid_unreachable"))
    }

    // Port 1 has no listener on any sane host, so every connection attempt is
    // refused immediately and server selection burns the full timeout.
    const UNREACHABLE_MONGO_URI: &str =
        "mongodb://127.0.0.1:1/nyxid_unreachable?directConnection=true";
    const UNREACHABLE_MONGO_SELECTION_TIMEOUT_MS: u64 = 750;

    #[tokio::test]
    async fn absent_debug_header_performs_no_flag_resolution() {
        // Latency guarantee: normal chat traffic never sends the debug header
        // and must therefore never touch the database. With an unreachable
        // MongoDB, any flag resolution would cost the full 750 ms selection
        // timeout; the header-first ordering keeps this in the microseconds.
        let state = unreachable_db_state().await;
        let auth_user = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());

        let started = std::time::Instant::now();
        let echoes = upstream_echo_collector(&state, &auth_user, &HeaderMap::new()).await;
        let elapsed = started.elapsed();

        assert!(echoes.is_none());
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "no-header path did database work ({elapsed:?}); the flag must be resolved \
             only after the header check"
        );
    }

    #[tokio::test]
    async fn flag_resolution_failure_fails_closed() {
        let state = unreachable_db_state().await;
        let auth_user = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());

        let started = std::time::Instant::now();
        let echoes = upstream_echo_collector(&state, &auth_user, &capture_request_headers()).await;
        let elapsed = started.elapsed();

        assert!(
            echoes.is_none(),
            "an unresolvable flag must never open the gate"
        );
        // Proves the failure really came from an attempted resolution rather
        // than from an earlier gate short-circuiting the call.
        assert!(
            elapsed >= std::time::Duration::from_millis(UNREACHABLE_MONGO_SELECTION_TIMEOUT_MS / 2),
            "expected a real resolution attempt against the unreachable database, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn malformed_debug_header_is_not_a_capture_request() {
        // The header gate rejects this before any flag resolution, so an
        // unreachable database is also proof that no lookup was attempted.
        let state = unreachable_db_state().await;
        let auth_user = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        let mut headers = HeaderMap::new();
        headers.insert(
            DEBUG_UPSTREAM_REQUEST_HEADER,
            HeaderValue::from_bytes(&[0xff]).expect("opaque invalid UTF-8 header"),
        );

        let started = std::time::Instant::now();
        let echoes = upstream_echo_collector(&state, &auth_user, &headers).await;

        assert!(echoes.is_none());
        assert!(started.elapsed() < std::time::Duration::from_millis(250));
    }

    #[test]
    fn absent_echo_collector_leaves_response_headers_untouched() {
        let response = Response::builder()
            .header("x-existing", "unchanged")
            .body(Body::empty())
            .unwrap();
        let expected = response.headers().clone();

        let response = attach_upstream_echoes(response, None);

        assert_eq!(response.headers(), &expected);
    }

    #[test]
    fn failed_upstream_attempt_remains_no_response() {
        let mut echoes = vec![test_echo(serde_json::Value::Null)];
        let result = Err(AppError::Internal("upstream unavailable".to_string()));

        record_upstream_response(Some(&mut echoes), Some(0), &result);

        assert!(matches!(
            echoes[0].upstream_outcome,
            Some(UpstreamOutcome::NoResponse)
        ));
        assert!(echoes[0].response.is_none());
    }

    #[tokio::test]
    async fn echo_header_leaves_sse_and_json_response_bodies_unmodified() {
        let upstream = b"data: {\"type\":\"RUN_FINISHED\"}\n\n";
        let echo = test_echo(serde_json::json!({ "type": "text", "prompt": "hello" }));
        for content_type in ["text/event-stream; charset=utf-8", "application/json"] {
            let response = Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_LENGTH, upstream.len())
                .body(Body::from(upstream.as_slice()))
                .unwrap();
            let response = attach_upstream_echoes(response, Some(std::slice::from_ref(&echo)));
            assert_eq!(
                response.headers().get(header::CONTENT_LENGTH),
                Some(&HeaderValue::from_static("31"))
            );
            assert!(
                response
                    .headers()
                    .get(DEBUG_UPSTREAM_RESPONSE_HEADER)
                    .is_some()
            );
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(bytes.as_ref(), upstream);
        }
    }

    #[test]
    fn oversized_header_echo_truncates_only_the_body_within_configured_limit() {
        let original = "steer ".repeat(4_000);
        let echo = test_echo(serde_json::json!({
            "type": "task.steer",
            "instruction": original,
        }));

        let (header, rung) = encode_echo_header_with_rung(&[echo]).unwrap();
        assert_eq!(rung, EchoEncodingRung::TruncatedBodies);
        assert!(header.as_bytes().len() <= DEBUG_UPSTREAM_HEADER_MAX_BYTES);
        let decoded = decode_echo_header(&header);
        assert_eq!(decoded["version"], 2);
        assert_eq!(decoded["droppedEchoCount"], 0);
        let echo = &decoded["echoes"][0];
        assert_eq!(echo["degraded"], false);
        assert_eq!(echo["method"], "POST");
        assert_eq!(echo["path"], "api/chat");
        assert_eq!(echo["commandType"], "task.steer");
        assert_eq!(echo["truncated"], true);
        assert!(echo["body"].as_str().is_some());
        assert!(echo["body"].as_str().unwrap().len() < original.len());
    }

    #[test]
    fn oversized_multi_echo_header_preserves_smaller_body_type_and_value() {
        let small_body = serde_json::json!({
            "type": "step.skip",
            "stepId": "step-small",
        });
        let small_echo = test_echo(small_body.clone());
        let large_echo = test_echo(serde_json::json!({
            "type": "task.steer",
            "instruction": "steer ".repeat(4_000),
        }));

        let header = encode_echo_header(&[small_echo, large_echo]).unwrap();
        assert!(header.as_bytes().len() <= DEBUG_UPSTREAM_HEADER_MAX_BYTES);
        let decoded = decode_echo_header(&header);
        assert_eq!(decoded["echoes"][0]["body"], small_body);
        assert_eq!(decoded["echoes"][0]["truncated"], false);
        assert!(decoded["echoes"][1]["body"].as_str().is_some());
        assert_eq!(decoded["echoes"][1]["truncated"], true);
    }

    #[test]
    fn response_echo_is_allowlisted_bounded_and_rfc_aware() {
        let long_request_id = format!("{}secret-tail", "r".repeat(300));
        let response = Response::builder()
            .status(202)
            .header(header::CONTENT_TYPE, "Text/Event-Stream; charset=utf-8")
            .header("x-request-id", long_request_id)
            .header("x-correlation-id", "correlation-1")
            .header(header::SET_COOKIE, "session=upstream-secret")
            .header(header::AUTHORIZATION, "Bearer upstream-secret")
            .header("x-nyxid-identity-token", "identity-secret")
            .body(Body::empty())
            .unwrap();

        let response_echo = response_echo(&response);
        let serialized = serde_json::to_string(&response_echo).unwrap();
        assert_eq!(response_echo.status, 202);
        assert!(response_echo.sse);
        assert_eq!(
            response_echo.headers["content-type"]["value"],
            "Text/Event-Stream; charset=utf-8"
        );
        assert_eq!(
            response_echo.headers["x-request-id"]["value"]
                .as_str()
                .unwrap()
                .len(),
            256
        );
        assert_eq!(response_echo.headers["x-request-id"]["truncated"], true);
        assert_eq!(
            response_echo.headers["x-correlation-id"]["truncated"],
            false
        );
        let (utf8_prefix, utf8_truncated) = truncate_utf8(&"界".repeat(100), 256);
        assert_eq!(utf8_prefix.len(), 255);
        assert!(utf8_truncated);
        for forbidden in [
            "set-cookie",
            "session=upstream-secret",
            "authorization",
            "upstream-secret",
            "x-nyxid-identity-token",
            "identity-secret",
        ] {
            assert!(
                !serialized.to_ascii_lowercase().contains(forbidden),
                "response echo leaked {forbidden}"
            );
        }
    }

    #[test]
    fn degradation_ladder_emits_only_full_or_minimal_echoes() {
        let (_, full_rung) = encode_echo_header_with_rung(&[test_echo(serde_json::json!({
            "prompt": "short",
        }))])
        .unwrap();
        assert_eq!(full_rung, EchoEncodingRung::Full);

        let (_, truncated_rung) = encode_echo_header_with_rung(&[test_echo(serde_json::json!({
            "prompt": "x".repeat(20_000),
        }))])
        .unwrap();
        assert_eq!(truncated_rung, EchoEncodingRung::TruncatedBodies);

        let dropped_body = (6_000..12_000)
            .step_by(8)
            .find_map(|identity_bytes| {
                let mut echo = test_echo(serde_json::json!({ "prompt": "x".repeat(20_000) }));
                echo.identity.mode = "i".repeat(identity_bytes);
                let encoded = encode_echo_header_with_rung(&[echo])?;
                (encoded.1 == EchoEncodingRung::DroppedBodies).then_some(encoded)
            })
            .expect("drop-body rung must be reachable");
        assert_eq!(
            decode_echo_header(&dropped_body.0)["echoes"][0]["body"],
            serde_json::Value::Null
        );

        let mut header_heavy = Vec::new();
        for index in 0..8 {
            let mut echo = test_echo(serde_json::Value::Null);
            echo.path = format!("{}-{index}", "p".repeat(250));
            echo.headers = serde_json::Map::from_iter([
                ("accept".to_string(), serde_json::json!("a".repeat(256))),
                (
                    "content-type".to_string(),
                    serde_json::json!("c".repeat(256)),
                ),
                (
                    "idempotency-key".to_string(),
                    serde_json::json!("i".repeat(256)),
                ),
            ]);
            echo.response = Some(UpstreamResponseEcho {
                status: 200,
                headers: serde_json::Map::from_iter([
                    (
                        "content-type".to_string(),
                        serde_json::to_value(UpstreamResponseHeaderEcho {
                            value: "r".repeat(256),
                            truncated: false,
                        })
                        .unwrap(),
                    ),
                    (
                        "x-request-id".to_string(),
                        serde_json::to_value(UpstreamResponseHeaderEcho {
                            value: "q".repeat(256),
                            truncated: false,
                        })
                        .unwrap(),
                    ),
                    (
                        "x-correlation-id".to_string(),
                        serde_json::to_value(UpstreamResponseHeaderEcho {
                            value: "z".repeat(256),
                            truncated: false,
                        })
                        .unwrap(),
                    ),
                ]),
                sse: false,
            });
            echo.upstream_outcome = Some(UpstreamOutcome::Response);
            header_heavy.push(echo);
        }
        let (dropped_headers, dropped_headers_rung) =
            encode_echo_header_with_rung(&header_heavy).unwrap();
        assert_eq!(dropped_headers_rung, EchoEncodingRung::DroppedHeaders);
        assert!(
            decode_echo_header(&dropped_headers)["echoes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|echo| echo["degraded"] == false && echo["droppedHeaders"] == true)
        );

        let mut identity_heavy = test_echo(serde_json::Value::Null);
        identity_heavy.identity.mode = "identity".repeat(4_000);
        let (minimal, minimal_rung) = encode_echo_header_with_rung(&[identity_heavy]).unwrap();
        assert_eq!(minimal_rung, EchoEncodingRung::Minimal);
        assert_eq!(decode_echo_header(&minimal)["echoes"][0]["degraded"], true);

        let many = (0..40)
            .map(|index| {
                let mut echo = test_echo(serde_json::Value::Null);
                echo.path = format!("{}-{index}", "p".repeat(250));
                echo.command_type = Some("c".repeat(64));
                echo.identity.mode = "identity".repeat(4_000);
                echo
            })
            .collect::<Vec<_>>();
        let (dropped, dropped_rung) = encode_echo_header_with_rung(&many).unwrap();
        assert_eq!(dropped_rung, EchoEncodingRung::DroppedEchoes);
        let dropped = decode_echo_header(&dropped);
        assert_eq!(dropped["echoes"].as_array().unwrap().len(), 8);
        assert_eq!(dropped["droppedEchoCount"], 32);
        assert_eq!(dropped["echoes"][0]["path"], many[0].path);

        let fixture_header = |echoes: &[UpstreamEcho], target| {
            (128..=4_096)
                .step_by(4)
                .find_map(|max_bytes| {
                    let encoded = encode_echo_header_with_limit(echoes, max_bytes)?;
                    (encoded.1 == target).then(|| decode_echo_header(&encoded.0))
                })
                .unwrap_or_else(|| panic!("fixture rung {target:?} must be reachable"))
        };
        let full_fixture = vec![test_echo(serde_json::json!({ "prompt": "short" }))];
        let truncated_fixture = vec![test_echo(serde_json::json!({
            "prompt": "x".repeat(500),
        }))];
        let mut dropped_body_fixture = test_echo(serde_json::json!({
            "prompt": "x".repeat(500),
        }));
        dropped_body_fixture.identity.mode = "i".repeat(100);
        let mut minimal_fixture = test_echo(serde_json::Value::Null);
        minimal_fixture.identity.mode = "identity".repeat(100);
        let mut dropped_headers_fixture = test_echo(serde_json::Value::Null);
        dropped_headers_fixture.headers = serde_json::Map::from_iter([
            ("accept".to_string(), serde_json::json!("a".repeat(100))),
            (
                "content-type".to_string(),
                serde_json::json!("c".repeat(100)),
            ),
        ]);
        dropped_headers_fixture.response = Some(UpstreamResponseEcho {
            status: 200,
            headers: serde_json::Map::from_iter([(
                "x-request-id".to_string(),
                serde_json::to_value(UpstreamResponseHeaderEcho {
                    value: "r".repeat(100),
                    truncated: false,
                })
                .unwrap(),
            )]),
            sse: false,
        });
        dropped_headers_fixture.upstream_outcome = Some(UpstreamOutcome::Response);
        let many_fixture = (0..12)
            .map(|index| {
                let mut echo = test_echo(serde_json::Value::Null);
                echo.path = format!("api/chat/{index}");
                echo.identity.mode = "identity".repeat(100);
                echo
            })
            .collect::<Vec<_>>();
        let fixture = serde_json::json!([
            { "rung": "full", "header": fixture_header(&full_fixture, EchoEncodingRung::Full) },
            { "rung": "truncated_bodies", "header": fixture_header(&truncated_fixture, EchoEncodingRung::TruncatedBodies) },
            { "rung": "dropped_bodies", "header": fixture_header(&[dropped_body_fixture], EchoEncodingRung::DroppedBodies) },
            { "rung": "dropped_headers", "header": fixture_header(&[dropped_headers_fixture], EchoEncodingRung::DroppedHeaders) },
            { "rung": "minimal", "header": fixture_header(&[minimal_fixture], EchoEncodingRung::Minimal) },
            { "rung": "dropped_echoes", "header": fixture_header(&many_fixture, EchoEncodingRung::DroppedEchoes) },
        ]);
        let committed_fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../frontend/src/stores/__fixtures__/assistant-upstream-envelope-ladder.json"
        ))
        .expect("ladder fixture must be valid JSON");
        assert_eq!(fixture, committed_fixture, "backend ladder fixture drifted");
    }

    #[test]
    fn worst_case_minimal_header_fits_with_total_header_headroom() {
        // Methods are validated HTTP tokens, paths come from axum URI paths,
        // and command types are fixed server literals. Raw control characters
        // that would expand under JSON escaping cannot reach these fields.
        let worst_echo = MinimalUpstreamEcho {
            degraded: true,
            method: "M".repeat(16),
            path: "界".repeat(85),
            command_type: Some("C".repeat(DEBUG_UPSTREAM_COMMAND_TYPE_MAX_BYTES)),
            upstream_outcome: Some(UpstreamOutcome::Response),
            status: Some(u16::MAX),
        };
        assert_eq!(worst_echo.path.len(), 255);
        let header = UpstreamEchoHeader {
            version: 2,
            echoes: vec![EncodedUpstreamEcho::Minimal(worst_echo); 8],
            dropped_echo_count: u32::MAX,
        };
        let json = serialize_echoes(&header).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&json);

        assert!(encoded.len() >= json.len() * 4 / 3);
        assert!(encoded.len() <= DEBUG_UPSTREAM_HEADER_MAX_BYTES);
        assert!(encoded.len() + 4 * 1024 <= 16 * 1024);
    }

    #[test]
    fn synthetic_requests_carry_the_callers_authorization() {
        let value = HeaderValue::from_static("Bearer nyx_test_token");
        let request = synthetic_request(Method::DELETE, Some(&value)).unwrap();
        assert_eq!(request.method(), Method::DELETE);
        assert_eq!(
            request.headers().get(header::AUTHORIZATION),
            Some(&HeaderValue::from_static("Bearer nyx_test_token"))
        );
        assert_eq!(
            request
                .extensions()
                .get::<crate::services::billing::route_inventory::BillingRoutePolicy>(),
            Some(
                &crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy
                )
            )
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

    fn aevatar_row(delegation_token_scope: &str) -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.slug = "aevatar".to_string();
        service.delegation_token_scope = delegation_token_scope.to_string();
        service
    }

    /// Locks the security-sensitive wiring of the forwarded token: a silent
    /// change to the actor, TTL, or a dropped `delegated` flag would slip past
    /// the generator's own tests but break the replay boundary. The scope now
    /// comes from the row (single source of truth with `inject_delegation_token`).
    #[tokio::test]
    async fn forward_authorization_is_a_delegated_proxy_token_for_aevatar() {
        use crate::crypto::jwt::verify_token;
        use crate::test_utils::{test_app_state_no_db, test_auth_user};

        let state = test_app_state_no_db().await;
        let user_id = "add69059-bece-4f0e-9559-99cfd10b47eb";
        let auth_user = test_auth_user(user_id);
        // A rest-proxy scope that is deliberately NOT the former hardcoded
        // `proxy` constant, so this test fails if the code ever reverts to
        // ignoring the row and hardcoding a scope again.
        let service = aevatar_row("proxy:*");

        let header = build_forward_authorization(&state, &auth_user, &service).unwrap();
        let token = header
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .expect("forwarded header is a Bearer credential");

        // Accepted by NyxID (the delegated router will admit it) — NOT a
        // reject-everywhere `assistant_forward` marker.
        let claims = verify_token(&state.jwt_keys, &state.config, token).unwrap();
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.delegated, Some(true), "must be a delegated token");
        assert_eq!(
            claims.act.as_ref().map(|a| a.sub.as_str()),
            Some("aevatar"),
            "actor is the Aevatar service slug"
        );
        // The EXACT row scope must reach the JWT (proves SSOT, not a hardcode).
        assert_eq!(claims.scope, "proxy:*");
        assert!(scope_allows_rest_proxy(&claims.scope));
        assert_eq!(claims.token_type, "access");
        assert_eq!(claims.assistant_forward, None, "not the retired marker");
        // Session user is unrestricted, so the delegated token inherits that.
        assert_eq!(claims.allow_all_services, Some(true));
        // Delegation TTL parity (300s), well under the 900s general access TTL.
        assert_eq!(claims.exp - claims.iat, MCP_DELEGATION_TOKEN_TTL_SECS);
    }

    /// A row left at the `llm:proxy` default cannot reach `/proxy/s/{slug}`.
    /// Rather than 500 the whole assistant over one config field, the bridge
    /// falls back to `proxy` (with a warning) so chat works on deploy. The
    /// minted token still carries a rest-proxy scope — never the row's
    /// insufficient `llm:proxy`.
    #[tokio::test]
    async fn forward_authorization_falls_back_to_proxy_when_row_scope_is_insufficient() {
        use crate::crypto::jwt::verify_token;
        use crate::test_utils::{test_app_state_no_db, test_auth_user};

        let state = test_app_state_no_db().await;
        let auth_user = test_auth_user("add69059-bece-4f0e-9559-99cfd10b47eb");

        for insufficient in ["llm:proxy", ""] {
            assert_eq!(
                resolve_forward_scope(&aevatar_row(insufficient)),
                PROXY_SCOPE
            );
            let header =
                build_forward_authorization(&state, &auth_user, &aevatar_row(insufficient))
                    .expect("bridge must still mint a usable token");
            let token = header.to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let claims = verify_token(&state.jwt_keys, &state.config, token).unwrap();
            assert!(
                scope_allows_rest_proxy(&claims.scope),
                "fallback token must reach the LLM proxy callback, got {}",
                claims.scope
            );
        }
    }
}
