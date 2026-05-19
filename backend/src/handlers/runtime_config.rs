use axum::{Json, extract::State};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct RuntimeConfigResponse {
    pub api_base_url: String,
}

/// GET /api/v1/runtime-config
///
/// Returns public runtime values the frontend cannot safely infer from its own
/// origin in split-origin deployments.
pub async fn get_runtime_config(State(state): State<AppState>) -> Json<RuntimeConfigResponse> {
    Json(runtime_config_response(&state.config.base_url))
}

fn runtime_config_response(base_url: &str) -> RuntimeConfigResponse {
    RuntimeConfigResponse {
        api_base_url: base_url.trim_end_matches('/').to_string(),
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn runtime_config_route_returns_trimmed_api_base_url() {
        let client = mongodb::Client::with_uri_str("mongodb://localhost:27017")
            .await
            .expect("build test MongoDB client");
        let db = client.database("runtime_config_route_test");
        let mut state = crate::test_utils::test_app_state(db);
        state.config.base_url = "https://nyx-api.chrono-ai.fun/".to_string();

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
            }),
        );
    }
}
