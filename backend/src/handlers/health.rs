use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// GET /health
///
/// Returns service health status. Used by load balancers and monitoring.
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[derive(Serialize)]
pub struct PublicConfigResponse {
    pub mcp_url: String,
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

    Json(PublicConfigResponse {
        mcp_url: format!("{base}/mcp"),
        version: env!("CARGO_PKG_VERSION").to_string(),
        social_providers,
    })
}
