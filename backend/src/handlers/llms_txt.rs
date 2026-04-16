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
