use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::AppState;
use crate::crypto::aes::EncryptionDecryptStats;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub commit: String,
    pub encryption: EncryptionHealthResponse,
}

#[derive(Serialize)]
pub struct EncryptionHealthResponse {
    pub previous_key_configured: bool,
    pub decrypt_stats: EncryptionDecryptStats,
}

/// GET /health
///
/// Returns service health status. Used by load balancers and monitoring.
pub async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: env!("NYXID_GIT_HASH").to_string(),
        encryption: EncryptionHealthResponse {
            previous_key_configured: state.encryption_keys.has_previous(),
            decrypt_stats: state.encryption_keys.decrypt_stats(),
        },
    })
}

#[derive(Serialize)]
pub struct PublicConfigResponse {
    pub frontend_url: String,
    pub mcp_url: String,
    pub node_ws_url: String,
    pub version: String,
    pub social_providers: Vec<String>,
    pub invite_code_required: bool,
    pub email_auth_enabled: bool,
    /// Public PostHog ingest key for the frontend. Non-secret by design
    /// (PostHog ingest keys are write-only and project-scoped). Empty
    /// when telemetry is off. See docs/TELEMETRY.md §3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_dsn: Option<String>,
    /// Host for the telemetry vendor, e.g. `https://us.i.posthog.com`.
    /// Empty when the frontend should use its compiled-in default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_host: Option<String>,
    /// Whether community share-back is opted in on this deployment.
    /// Frontend uses this to decide whether to fall back to the
    /// compiled-in public DSN when `telemetry_dsn` is empty. Omitted
    /// from the response when false so the default-off deploy's
    /// `/public/config` shape remains byte-identical to pre-telemetry.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub telemetry_share_analytics: bool,
}

/// GET /api/v1/public/config
///
/// Returns public configuration needed by the frontend (no auth required).
pub async fn public_config(State(state): State<AppState>) -> Json<PublicConfigResponse> {
    let base = state.config.base_url.trim_end_matches('/');

    let mut social_providers = Vec::new();
    if state.config.github_client_id.is_some() && state.config.github_client_secret.is_some() {
        social_providers.push("github".to_string());
    }
    if state.config.google_client_id.is_some() && state.config.google_client_secret.is_some() {
        social_providers.push("google".to_string());
    }
    if state.config.apple_configured() {
        social_providers.push("apple".to_string());
    }

    let ws_base = base
        .replace("https://", "wss://")
        .replace("http://", "ws://");

    // Telemetry fields are only populated when the deployment has
    // opted in — either by setting an explicit DSN or by enabling
    // community share-back. This keeps the default-off `/public/config`
    // response byte-identical to a pre-telemetry deploy (no new JSON
    // keys leak). A deploy that sets only HOST but no DSN / no
    // share-back is treated as off.
    let telemetry_enabled = state
        .config
        .telemetry_dsn
        .as_ref()
        .is_some_and(|s| !s.is_empty())
        || state.config.share_analytics;

    let (telemetry_dsn, telemetry_host, telemetry_share_analytics) = if telemetry_enabled {
        (
            state
                .config
                .telemetry_dsn
                .as_ref()
                .filter(|s| !s.is_empty())
                .cloned(),
            state
                .config
                .telemetry_host
                .as_ref()
                .filter(|s| !s.is_empty())
                .cloned(),
            state.config.share_analytics,
        )
    } else {
        (None, None, false)
    };

    Json(PublicConfigResponse {
        frontend_url: state.config.frontend_url.trim_end_matches('/').to_string(),
        mcp_url: format!("{base}/mcp"),
        node_ws_url: format!("{ws_base}/api/v1/nodes/ws"),
        version: env!("CARGO_PKG_VERSION").to_string(),
        social_providers,
        invite_code_required: state.config.invite_code_required,
        email_auth_enabled: state.config.email_auth_enabled,
        telemetry_dsn,
        telemetry_host,
        telemetry_share_analytics,
    })
}
