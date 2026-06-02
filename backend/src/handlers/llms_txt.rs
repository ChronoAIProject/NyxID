use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::AppState;

static PLAYBOOK: &str = include_str!("../../../docs/AI_AGENT_PLAYBOOK.md");

fn markdown_response(body: String) -> Response {
    (
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        body,
    )
        .into_response()
}

/// GET /llms.txt
///
/// Full AI Agent Playbook with deployment-specific URLs.
/// Same content as /llms-full.txt -- one comprehensive reference.
pub async fn llms_txt(State(state): State<AppState>) -> Response {
    llms_full_txt(State(state)).await
}

/// GET /llms-full.txt
///
/// Full AI Agent Playbook with deployment-specific URLs substituted in.
pub async fn llms_full_txt(State(state): State<AppState>) -> Response {
    let base = state.config.base_url.trim_end_matches('/');
    let frontend = state.config.frontend_url.trim_end_matches('/');

    // Derive ws/wss base for WebSocket URLs in the playbook
    let ws_base = base
        .replace("https://", "wss://")
        .replace("http://", "ws://");

    let body = PLAYBOOK
        .replace("ws://localhost:3001", &ws_base)
        .replace("http://localhost:3001", base)
        .replace("http://localhost:3000", frontend);

    markdown_response(body)
}

#[cfg(test)]
mod tests {
    use super::{llms_full_txt, llms_txt};
    use axum::{
        body::to_bytes,
        extract::State,
        http::{StatusCode, header},
    };

    async fn response_body(response: axum::response::Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn llms_full_txt_returns_markdown_with_deployment_urls() {
        let mut state = crate::test_utils::test_app_state_no_db().await;
        state.config.base_url = "https://api.example.test/".to_string();
        state.config.frontend_url = "https://console.example.test/".to_string();

        let response = llms_full_txt(State(state)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/markdown; charset=utf-8")
        );

        let body = response_body(response).await;
        assert!(body.contains("- Backend API: `https://api.example.test`"));
        assert!(body.contains("- Frontend Dashboard: `https://console.example.test`"));
        assert!(body.contains("nyxid login --base-url https://api.example.test"));
    }

    #[tokio::test]
    async fn llms_txt_matches_full_playbook_response() {
        let mut state = crate::test_utils::test_app_state_no_db().await;
        state.config.base_url = "http://nyxid.local:3001".to_string();
        state.config.frontend_url = "http://nyxid.local:3000".to_string();

        let short = llms_txt(State(state.clone())).await;
        let full = llms_full_txt(State(state)).await;

        assert_eq!(
            short
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            full.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
        );
        assert_eq!(response_body(short).await, response_body(full).await);
    }
}
