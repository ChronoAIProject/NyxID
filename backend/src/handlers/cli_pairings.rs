//! HTTP handlers for the remote CLI pairing flow (Mode B wizard).
//!
//! Five endpoints, all under `/api/v1/cli-pairings`:
//!
//!   - `POST /`                 (CLI): create a pairing, get code + urls
//!   - `GET  /{id}/poll`        (CLI): poll for status / ack
//!   - `POST /{id}/cancel`      (CLI or browser): user gave up
//!   - `POST /claim`            (browser): user entered the code
//!   - `POST /{id}/complete`    (browser): wizard finished, here's the ack
//!
//! All endpoints require a *human* session (rejected for service
//! accounts via the human-only middleware in `routes.rs`). The CLI
//! side authenticates with the user's access token; the browser side
//! with the session cookie.

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::auth::extract_ip;
use crate::mw::auth::AuthUser;
use crate::services::cli_pairing_service::{self, ClaimedPairing, CreateParams, PollResponse};

/// Max bytes a `kind` string may contain. Matches the service-layer
/// guard; duplicated here so the handler fails fast before the service
/// allocation.
const MAX_KIND_LEN: usize = 64;

// ── request / response types ────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePairingRequest {
    /// Wizard flow identifier. Must match a known `FlowKind` on the
    /// frontend (e.g. `"api-key-create"`, `"node-register-token"`).
    pub kind: String,
    /// Optional flow-specific prefill. Treated as opaque JSON by the
    /// server; echoed back to the frontend on `claim`.
    #[serde(default)]
    pub prefill: serde_json::Value,
    /// Optional TTL override in seconds (60..=3600). Defaults to 900s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatePairingResponse {
    pub id: String,
    pub code: String,
    pub pair_url: String,
    pub poll_url: String,
    pub expires_at: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ClaimPairingRequest {
    /// User-entered code, with or without dashes; case-insensitive.
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ClaimPairingResponse {
    pub id: String,
    pub kind: String,
    pub prefill: serde_json::Value,
    /// `true` when this was a re-claim of an already-claimed record
    /// (same user, refreshed tab, etc.). The frontend uses this flag
    /// to avoid replaying destructive confirm-step API calls for
    /// flows that mint one-time secrets.
    pub resumed: bool,
    /// `true` iff `reserve-action` was called at least once. Paired
    /// with `resumed` to distinguish safe "pre-action refresh" from
    /// dangerous "post-mint refresh" on the frontend.
    pub action_started: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CompletePairingRequest {
    /// Typed ack payload. Shape depends on `kind`; the browser builds
    /// this from the DisplayOnce wizard panel, and the CLI validates
    /// the exact shape (see `cli/src/wizard/mod.rs` ack structs).
    pub ack: serde_json::Value,
}

// ── handlers ────────────────────────────────────────────────────────

/// Create a pairing. CLI-side: authenticated via access token; the
/// `user_id` on the record is bound to the caller's session.
pub async fn create_pairing(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreatePairingRequest>,
) -> AppResult<Json<CreatePairingResponse>> {
    if req.kind.is_empty() || req.kind.len() > MAX_KIND_LEN {
        return Err(AppError::BadRequest(format!(
            "kind must be 1..={MAX_KIND_LEN} chars"
        )));
    }
    let created = cli_pairing_service::create(
        &state.db,
        state.cli_pairing_hmac_key.as_slice(),
        CreateParams {
            user_id: &user.user_id.to_string(),
            kind: &req.kind,
            prefill: req.prefill,
            ttl_secs: req.ttl_secs,
        },
    )
    .await?;

    let frontend = state.config.frontend_url.trim_end_matches('/');
    let backend = state.config.base_url.trim_end_matches('/');

    Ok(Json(CreatePairingResponse {
        pair_url: format!("{frontend}/cli/pair"),
        poll_url: format!("{backend}/api/v1/cli-pairings/{}/poll", created.id),
        id: created.id,
        code: created.code,
        expires_at: created.expires_at,
    }))
}

/// CLI-side poll. Returns `{status: "pending" | "claimed" | "completed"
/// | "cancelled" | "expired"}`, with `ack` only populated in the
/// `completed` shape.
pub async fn poll_pairing(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<PollResponse>> {
    let resp = cli_pairing_service::poll(&state.db, &id, &user.user_id.to_string()).await?;
    Ok(Json(resp))
}

/// Browser-side claim. Looks up by (hashed) code, enforces user_id
/// binding, transitions status Pending → Claimed, returns the
/// wizard-kind + prefill so the frontend can render the right panel.
pub async fn claim_pairing(
    State(state): State<AppState>,
    user: AuthUser,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ClaimPairingRequest>,
) -> AppResult<Json<ClaimPairingResponse>> {
    // Rate-limit KEY: prefer `X-Forwarded-For` / `X-Real-IP` only
    // when the TCP peer appears in `config.trusted_proxy_ips`,
    // otherwise fall back to the peer address. This closes two
    // otherwise-exclusive failure modes:
    //
    //   - Direct-exposure (no proxy allowlist configured): headers
    //     are spoofable, so keying on them would let an attacker
    //     rotate `X-Forwarded-For` and bypass the 5/60s throttle
    //     that is the primary guard for the 8-char pairing code.
    //     The peer IP is the only trustworthy key here.
    //   - Proxy deployments (nginx / ALB / etc.): the peer IP is
    //     always the proxy, so a per-peer bucket collapses into a
    //     site-wide 5/60s bucket and legitimate users collide. The
    //     operator opts in to per-real-client keying by listing
    //     proxy IPs in `TRUSTED_PROXY_IPS`; headers from other
    //     peers are still ignored, so the spoofing hole stays
    //     closed.
    //
    // Failure modes left over:
    //   - A misconfigured reverse proxy that passes client-supplied
    //     `X-Forwarded-For` through unchanged will now honor the
    //     spoofed header. Operators are explicitly responsible for
    //     configuring the proxy to overwrite forwarded headers
    //     (see docs/ENV.md) when listing it in `TRUSTED_PROXY_IPS`.
    //   - A request with no peer info (unreachable in normal axum
    //     wiring, but possible in tests) falls through to the
    //     rate-limiter's `None` path below — we refuse the request
    //     rather than allow it to skip throttling.
    let client_ip = crate::mw::rate_limit::resolve_client_ip_for_rate_limit(
        &headers,
        Some(peer),
        &state.config.trusted_proxy_ips,
    );
    let Some(client_ip) = client_ip else {
        tracing::warn!(
            user_id = %user.user_id,
            "cli_pairings/claim: could not resolve client IP for rate limit; refusing"
        );
        return Err(AppError::RateLimited);
    };
    if !state.cli_pairing_claim_limiter.check(client_ip) {
        tracing::warn!(
            client_ip = %client_ip,
            peer_ip = %peer.ip(),
            user_id = %user.user_id,
            "cli_pairings/claim rate-limited"
        );
        return Err(AppError::RateLimited);
    }

    // `extract_ip` IS still used below for audit logging only — where
    // spoofability is informational, not security-critical — so the
    // X-Forwarded-For hint can be captured without gating throttling.
    let ip_string = extract_ip(&headers, Some(peer));

    let ClaimedPairing {
        id,
        kind,
        prefill,
        resumed,
        action_started,
    } = cli_pairing_service::claim(
        &state.db,
        state.cli_pairing_hmac_key.as_slice(),
        &req.code,
        &user.user_id.to_string(),
        ip_string.as_deref(),
    )
    .await?;

    Ok(Json(ClaimPairingResponse {
        id,
        kind,
        prefill,
        resumed,
        action_started,
    }))
}

/// Browser-side: mark that the destructive API call is about to fire.
/// Called just before `POST /api-keys`, `POST /api-keys/{id}/rotate`,
/// etc. so a concurrent re-claim can tell the mint already happened.
pub async fn reserve_action(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    cli_pairing_service::reserve_action(&state.db, &id, &user.user_id.to_string()).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Browser-side: undo a prior `reserve_action` so the user can retry
/// a cancelled OAuth / device-code sub-flow on the same pairing.
/// Safe by construction — the service layer refuses to rewind a
/// Completed pairing, so this can't be abused to replay a minted
/// secret. A no-op when the pairing isn't in `Claimed` state.
pub async fn rewind_action(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    cli_pairing_service::rewind_action(&state.db, &id, &user.user_id.to_string()).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Browser-side completion. Stores the typed ack; the next CLI poll
/// returns it and the flow ends.
pub async fn complete_pairing(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<CompletePairingRequest>,
) -> AppResult<Json<serde_json::Value>> {
    cli_pairing_service::complete(
        &state.db,
        &state.encryption_keys,
        &id,
        &user.user_id.to_string(),
        req.ack,
    )
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Cancel by the originating user (CLI or browser side). Transitions
/// (Pending | Claimed) → Cancelled.
pub async fn cancel_pairing(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    cli_pairing_service::cancel(&state.db, &id, &user.user_id.to_string()).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::AppError;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user};
    use axum::extract::{ConnectInfo, Path, State};
    use axum::http::HeaderMap;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_create_pairing_success() {
        let Some(db) = connect_test_database("h_cli_pair_create").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(resp) = create_pairing(
            State(state),
            auth,
            Json(CreatePairingRequest {
                kind: "api-key-create".to_string(),
                prefill: serde_json::json!({"name": "test"}),
                ttl_secs: None,
            }),
        )
        .await
        .expect("create should succeed");

        assert!(!resp.id.is_empty());
        assert!(!resp.code.is_empty());
        assert!(resp.pair_url.contains("/cli/pair"));
        assert!(resp.poll_url.contains(&resp.id));
    }

    #[tokio::test]
    async fn test_create_pairing_empty_kind_rejected() {
        let Some(db) = connect_test_database("h_cli_pair_empty_kind").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = create_pairing(
            State(state),
            auth,
            Json(CreatePairingRequest {
                kind: String::new(),
                prefill: serde_json::json!(null),
                ttl_secs: None,
            }),
        )
        .await
        .expect_err("empty kind should be rejected");

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_poll_pairing_pending() {
        let Some(db) = connect_test_database("h_cli_pair_poll").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_pairing(
            State(state.clone()),
            auth.clone(),
            Json(CreatePairingRequest {
                kind: "api-key-create".to_string(),
                prefill: serde_json::json!({}),
                ttl_secs: None,
            }),
        )
        .await
        .expect("create");

        let Json(poll) = poll_pairing(State(state), auth, Path(created.id))
            .await
            .expect("poll should succeed");

        assert!(matches!(
            poll.status,
            cli_pairing_service::PollStatus::Pending
        ));
    }

    #[tokio::test]
    async fn test_claim_pairing_success() {
        let Some(db) = connect_test_database("h_cli_pair_claim").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_pairing(
            State(state.clone()),
            auth.clone(),
            Json(CreatePairingRequest {
                kind: "api-key-create".to_string(),
                prefill: serde_json::json!({"name": "agent"}),
                ttl_secs: None,
            }),
        )
        .await
        .expect("create");

        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();

        let Json(claimed) = claim_pairing(
            State(state),
            auth,
            ConnectInfo(peer),
            HeaderMap::new(),
            Json(ClaimPairingRequest {
                code: created.code.clone(),
            }),
        )
        .await
        .expect("claim should succeed");

        assert_eq!(claimed.kind, "api-key-create");
        assert!(!claimed.resumed);
    }

    #[tokio::test]
    async fn test_complete_pairing_success() {
        let Some(db) = connect_test_database("h_cli_pair_complete").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_pairing(
            State(state.clone()),
            auth.clone(),
            Json(CreatePairingRequest {
                kind: "api-key-create".to_string(),
                prefill: serde_json::json!({}),
                ttl_secs: None,
            }),
        )
        .await
        .expect("create");

        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        let _ = claim_pairing(
            State(state.clone()),
            auth.clone(),
            ConnectInfo(peer),
            HeaderMap::new(),
            Json(ClaimPairingRequest {
                code: created.code.clone(),
            }),
        )
        .await
        .expect("claim");

        let Json(resp) = complete_pairing(
            State(state),
            auth,
            Path(created.id),
            Json(CompletePairingRequest {
                ack: serde_json::json!({"acknowledged": true, "api_key_id": "abc-123"}),
            }),
        )
        .await
        .expect("complete should succeed");

        assert_eq!(resp.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn test_cancel_pairing_success() {
        let Some(db) = connect_test_database("h_cli_pair_cancel").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(created) = create_pairing(
            State(state.clone()),
            auth.clone(),
            Json(CreatePairingRequest {
                kind: "node-register-token".to_string(),
                prefill: serde_json::json!({}),
                ttl_secs: None,
            }),
        )
        .await
        .expect("create");

        let Json(resp) = cancel_pairing(State(state), auth, Path(created.id))
            .await
            .expect("cancel should succeed");

        assert_eq!(resp.get("ok").and_then(|v| v.as_bool()), Some(true));
    }
}
