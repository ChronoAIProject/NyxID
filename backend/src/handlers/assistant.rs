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
    http::{HeaderValue, Method, Request, header},
    response::Response,
};

use crate::AppState;
use crate::crypto::jwt::{
    MCP_DELEGATION_TOKEN_TTL_SECS, TokenRestrictionClaims, generate_delegated_access_token,
};
use crate::errors::{AppError, AppResult};
use crate::handlers::proxy::execute_admin_proxy;
use crate::models::downstream_service::DownstreamService;
use crate::mw::auth::{AuthMethod, AuthUser, PROXY_SCOPE, scope_allows_rest_proxy};
use crate::services::assistant_service;

/// Conversation indexes carry titles, timestamps, and counts per row, so the
/// list route gets its own buffering headroom before any merge/reshape.
const MAX_CONVERSATION_INDEX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

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
async fn forward(
    state: &AppState,
    auth_user: &AuthUser,
    path: String,
    mut request: Request<Body>,
) -> AppResult<Response> {
    let service = assistant_service::resolve_admin_service(&state.db).await?;

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
    if needs_forward_token_bridge(&auth_user.auth_method, service.forward_access_token) {
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
        &mut resolved_slug,
    )
    .await
}

/// `GET /api/v1/assistant/conversations` -- canonical typed list, with a
/// best-effort legacy workflow (`chatc-…`) merge when the upstream list has
/// not absorbed those rows yet.
pub async fn list_conversations(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let authorization = request.headers().get(header::AUTHORIZATION).cloned();
    let user_id = auth_user.user_id.to_string();
    let response = forward(
        &state,
        &auth_user,
        assistant_service::canonical_conversations_path(),
        request,
    )
    .await?;
    if !response.status().is_success() {
        return Ok(response);
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_CONVERSATION_INDEX_RESPONSE_BYTES).await else {
        return Err(AppError::Internal(
            "assistant: conversation index exceeded the buffer cap".to_string(),
        ));
    };
    let Ok(mut canonical) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(Response::from_parts(parts, Body::from(bytes)));
    };
    let mut changed = assistant_service::filter_addressable_conversation_index(&mut canonical);
    if !assistant_service::conversation_index_includes_workflow(&canonical) {
        let legacy_request =
            synthetic_request(Method::GET, authorization.as_ref()).map_err(|_| {
                AppError::Internal(
                    "assistant: failed to build the workflow list request".to_string(),
                )
            })?;
        if let Ok(legacy_response) = forward(
            &state,
            &auth_user,
            assistant_service::history_index_path(&user_id),
            legacy_request,
        )
        .await
        {
            if legacy_response.status().is_success() {
                if let Ok(legacy_bytes) = to_bytes(
                    legacy_response.into_body(),
                    MAX_CONVERSATION_INDEX_RESPONSE_BYTES,
                )
                .await
                {
                    if let Ok(legacy_index) =
                        serde_json::from_slice::<serde_json::Value>(&legacy_bytes)
                    {
                        changed |=
                            assistant_service::merge_workflow_history_rows(&mut canonical, &legacy_index);
                    }
                }
            }
        }
    }
    if !changed {
        return Ok(Response::from_parts(parts, Body::from(bytes)));
    }
    let Ok(filtered) = serde_json::to_vec(&canonical) else {
        return Ok(Response::from_parts(parts, Body::from(bytes)));
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    Ok(Response::from_parts(parts, Body::from(filtered)))
}

/// `GET /api/v1/assistant/conversations/{id}` -- canonical conversation
/// transcript/state wrapper.
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
    let path = assistant_service::canonical_conversation_path(&conversation_id)?;
    forward(&state, &auth_user, path, request).await
}

/// `DELETE /api/v1/assistant/conversations/{id}` -- canonical single delete.
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    let path = assistant_service::canonical_conversation_path(&conversation_id)?;
    forward(&state, &auth_user, path, request).await
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
    let path = assistant_service::canonical_state_path(&conversation_id)?;
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

/// Bounds caller chat bodies: a 32k-char prompt is at most 128 KiB of UTF-8,
/// with additional room for JSON escaping.
const MAX_ASSISTANT_CHAT_REQUEST_BYTES: usize = 256 * 1024;

/// `POST /api/v1/assistant/chat` -- typed NyxIdChat create-and-first-turn SSE.
///
/// The browser request is parsed with an explicit command allowlist, rebuilt
/// into the exact canonical `/api/chat` body, and forwarded with
/// `Idempotency-Key: clientRequestId`.
pub async fn typed_chat(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_ASSISTANT_CHAT_REQUEST_BYTES)
        .await
        .map_err(|_| AppError::BadRequest("Assistant chat request body is too large.".to_string()))?;
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
    let idempotency = HeaderValue::from_str(&prepared.client_request_id).map_err(|_| {
        AppError::Internal("assistant: failed to encode the idempotency key".to_string())
    })?;
    parts.headers.insert(header::HeaderName::from_static("idempotency-key"), idempotency);
    let request = Request::from_parts(parts, Body::from(payload));
    forward(
        &state,
        &auth_user,
        assistant_service::typed_chat_path(),
        request,
    )
    .await
}

/// `POST /api/v1/assistant/workflow-chat` -- workflow ("studio") chat turn,
/// answered as the upstream SSE stream.
///
/// The caller body is the typed `WorkflowChatTurnRequest` (prompt +
/// conversation continuation + idempotency id); the upstream `/api/chat`
/// body is built server-side with the workflow pinned to `studio` and the
/// `conversation` object always present, so every turn persists to chat
/// history and no caller can select another engine or smuggle fields into
/// Aevatar's strict `HttpChatInput`. Scope comes from the propagated
/// identity token (Aevatar ignores any body scope).
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
    forward(
        &state,
        &auth_user,
        assistant_service::workflow_chat_path(),
        request,
    )
    .await
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
