use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::AppState;
use crate::crypto::aes::EncryptionDecryptStats;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
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
        encryption: EncryptionHealthResponse {
            previous_key_configured: state.encryption_keys.has_previous(),
            decrypt_stats: state.encryption_keys.decrypt_stats(),
        },
    })
}

#[derive(Serialize)]
pub struct PublicConfigResponse {
    pub mcp_url: String,
    pub node_ws_url: String,
    pub version: String,
    pub social_providers: Vec<String>,
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

    Json(PublicConfigResponse {
        mcp_url: format!("{base}/mcp"),
        node_ws_url: format!("{ws_base}/api/v1/nodes/ws"),
        version: env!("CARGO_PKG_VERSION").to_string(),
        social_providers,
    })
}
