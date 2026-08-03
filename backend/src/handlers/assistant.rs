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
use crate::handlers::admin_helpers;
use crate::handlers::proxy::execute_admin_proxy;
use crate::models::downstream_service::DownstreamService;
use crate::mw::auth::{AuthMethod, AuthUser, PROXY_SCOPE, scope_allows_rest_proxy};
use crate::services::assistant_service;

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
    method: String,
    path: String,
    command_type: Option<String>,
    body: serde_json::Value,
    headers: serde_json::Map<String, serde_json::Value>,
    identity: UpstreamIdentityEcho,
    truncated: bool,
}

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
    if admin_helpers::require_admin(state, auth_user).await.is_ok() {
        Some(Vec::new())
    } else {
        None
    }
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
        headers.insert(
            "content-type".to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    for (name, value) in extra_outbound_headers {
        let normalized = name.to_ascii_lowercase();
        if matches!(normalized.as_str(), "idempotency-key" | "accept") {
            headers.insert(normalized, serde_json::Value::String(value.clone()));
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
    UpstreamEcho {
        method: request.method().as_str().to_string(),
        path,
        command_type: command_type.map(str::to_string),
        body: body.unwrap_or(serde_json::Value::Null),
        headers: echoed_headers(request.headers(), extra_outbound_headers),
        identity,
        truncated: false,
    }
}

fn serialize_echoes(echoes: &[UpstreamEcho]) -> AppResult<Vec<u8>> {
    serde_json::to_vec(echoes)
        .map_err(|_| AppError::Internal("assistant: failed to encode upstream echoes".to_string()))
}

fn encode_echo_header(echoes: &[UpstreamEcho]) -> Option<HeaderValue> {
    let encode = |candidate: &[UpstreamEcho]| -> Option<String> {
        Some(base64::engine::general_purpose::STANDARD.encode(serialize_echoes(candidate).ok()?))
    };
    let encoded = encode(echoes)?;
    if encoded.len() <= DEBUG_UPSTREAM_HEADER_MAX_BYTES {
        return HeaderValue::from_str(&encoded).ok();
    }

    let mut bodies = Vec::new();
    for (index, echo) in echoes.iter().enumerate() {
        if echo.body.is_null() {
            continue;
        }
        let body = serde_json::to_string(&echo.body).ok()?;
        bodies.push((index, body.chars().collect::<Vec<_>>()));
    }
    bodies.sort_by_key(|(_, body)| std::cmp::Reverse(body.len()));

    let mut candidates = echoes.to_vec();
    for (index, body) in bodies {
        candidates[index].body = serde_json::Value::String(String::new());
        candidates[index].truncated = true;

        if encode(&candidates)?.len() > DEBUG_UPSTREAM_HEADER_MAX_BYTES {
            continue;
        }

        let mut low = 0;
        let mut high = body.len();
        let mut best = String::new();
        while low <= high {
            let middle = low + (high - low) / 2;
            candidates[index].body = serde_json::Value::String(body[..middle].iter().collect());
            let candidate_encoded = encode(&candidates)?;
            if candidate_encoded.len() <= DEBUG_UPSTREAM_HEADER_MAX_BYTES {
                best = candidate_encoded;
                low = middle + 1;
            } else if middle == 0 {
                break;
            } else {
                high = middle - 1;
            }
        }
        return HeaderValue::from_str(&best).ok();
    }

    None
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
    request.headers_mut().remove(DEBUG_UPSTREAM_REQUEST_HEADER);
    let service = assistant_service::resolve_admin_service(&state.db).await?;
    let bridge_minted =
        needs_forward_token_bridge(&auth_user.auth_method, service.forward_access_token);
    if let Some(echoes) = echo.collector {
        echoes.push(build_upstream_echo(
            &request,
            path.clone(),
            echo.command_type,
            echo.body,
            &extra_outbound_headers,
            UpstreamIdentityEcho {
                mode: service.identity_propagation_mode.clone(),
                forward_access_token: service.forward_access_token,
                inject_delegation_token: service.inject_delegation_token,
                bridge_minted,
            },
        ));
    }

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
    execute_admin_proxy(
        state,
        auth_user,
        &service.id,
        &path,
        request,
        extra_outbound_headers,
        &mut resolved_slug,
    )
    .await
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
        let serialized = String::from_utf8(serialize_echoes(&[echo]).unwrap()).unwrap();
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

    #[tokio::test]
    async fn absent_echo_gate_returns_before_admin_lookup() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let auth_user = crate::test_utils::test_auth_user("00000000-0000-4000-8000-000000000001");

        let echoes = upstream_echo_collector(&state, &auth_user, &HeaderMap::new()).await;

        assert!(echoes.is_none());
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

        let header = encode_echo_header(&[echo]).unwrap();
        assert!(header.as_bytes().len() <= DEBUG_UPSTREAM_HEADER_MAX_BYTES);
        let decoded = decode_echo_header(&header);
        assert_eq!(decoded[0]["method"], "POST");
        assert_eq!(decoded[0]["path"], "api/chat");
        assert_eq!(decoded[0]["commandType"], "task.steer");
        assert_eq!(decoded[0]["truncated"], true);
        assert!(decoded[0]["body"].as_str().is_some());
        assert!(decoded[0]["body"].as_str().unwrap().len() < original.len());
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
        assert_eq!(decoded[0]["body"], small_body);
        assert_eq!(decoded[0]["truncated"], false);
        assert!(decoded[1]["body"].as_str().is_some());
        assert_eq!(decoded[1]["truncated"], true);
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
