use axum::{Json, extract::State};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct RuntimeConfigResponse {
    pub api_base_url: String,
    pub release_integrity: ReleaseIntegrityRuntimeConfig,
}

#[derive(Serialize)]
pub struct ReleaseIntegrityRuntimeConfig {
    pub enabled: bool,
    pub manifest_url: Option<String>,
    pub verification_ttl_secs: i64,
}

/// GET /api/v1/runtime-config
///
/// Returns public runtime values the frontend cannot safely infer from its own
/// origin in split-origin deployments.
pub async fn get_runtime_config(State(state): State<AppState>) -> Json<RuntimeConfigResponse> {
    Json(runtime_config_response(
        &state.config.base_url,
        state.config.release_integrity_manifest_url.as_deref(),
        state.config.jwt_relay_reply_ttl_secs,
    ))
}

fn runtime_config_response(
    base_url: &str,
    release_integrity_manifest_url: Option<&str>,
    verification_ttl_secs: i64,
) -> RuntimeConfigResponse {
    let manifest_url = release_integrity_manifest_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    RuntimeConfigResponse {
        api_base_url: base_url.trim_end_matches('/').to_string(),
        release_integrity: ReleaseIntegrityRuntimeConfig {
            enabled: manifest_url.is_some(),
            manifest_url,
            verification_ttl_secs,
        },
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[test]
    fn runtime_config_response_trims_trailing_slash() {
        let resp = super::runtime_config_response("https://api.example.com/", None, 1800);
        assert_eq!(resp.api_base_url, "https://api.example.com");
    }

    #[test]
    fn runtime_config_response_no_trailing_slash_unchanged() {
        let resp = super::runtime_config_response("https://api.example.com", None, 1800);
        assert_eq!(resp.api_base_url, "https://api.example.com");
    }

    #[test]
    fn runtime_config_response_serializes_correctly() {
        let resp = super::runtime_config_response("http://localhost:3001/", None, 1800);
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["api_base_url"], "http://localhost:3001");
        assert_eq!(
            json["release_integrity"],
            serde_json::json!({
                "enabled": false,
                "manifest_url": null,
                "verification_ttl_secs": 1800,
            })
        );
    }

    #[test]
    fn runtime_config_response_trims_configured_manifest_url() {
        let resp = super::runtime_config_response(
            "http://localhost:3001/",
            Some(" https://release.example.test/releases.json "),
            900,
        );
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            json["release_integrity"],
            serde_json::json!({
                "enabled": true,
                "manifest_url": "https://release.example.test/releases.json",
                "verification_ttl_secs": 900,
            })
        );
    }

    #[tokio::test]
    async fn runtime_config_route_returns_trimmed_api_base_url() {
        let client = mongodb::Client::with_uri_str("mongodb://localhost:27017")
            .await
            .expect("build test MongoDB client");
        let db = client.database("runtime_config_route_test");
        let mut state = crate::test_utils::test_app_state(db);
        state.config.base_url = "https://nyx-api.chrono-ai.fun/".to_string();
        state.config.release_integrity_manifest_url =
            Some("https://release.example.test/releases.json".to_string());
        state.config.jwt_relay_reply_ttl_secs = 1800;

        let (_, private_api) = crate::routes::build_router(1024 * 1024);
        let response = private_api
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/runtime-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "api_base_url": "https://nyx-api.chrono-ai.fun",
                "release_integrity": {
                    "enabled": true,
                    "manifest_url": "https://release.example.test/releases.json",
                    "verification_ttl_secs": 1800,
                },
            }),
        );
    }
}
