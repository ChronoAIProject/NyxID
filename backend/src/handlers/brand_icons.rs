//! NyxID's icon on the API host. Clients that look up an icon for a server
//! URL, such as Claude's connector list, otherwise fall back to the parent
//! domain's favicon. The MCP `initialize` result also advertises these URLs
//! in `serverInfo.icons`.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::AppState;

const SVG_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'";

static FAVICON_ICO: &[u8] = include_bytes!("../../assets/favicon.ico");
static FAVICON_SVG: &[u8] = include_bytes!("../../assets/favicon.svg");
static ICON_PNG: &[u8] = include_bytes!("../../assets/icon-512.png");

fn icon_response(content_type: &'static str, bytes: &'static [u8]) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=86400"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}

/// GET /favicon.ico
pub async fn favicon_ico() -> Response {
    icon_response("image/x-icon", FAVICON_ICO)
}

/// GET /favicon.svg
///
/// The SVG colours its gradient with inline `style` attributes, which the
/// global `default-src 'none'` policy would block when the file is opened
/// directly, so this response allows inline styles and nothing else.
pub async fn favicon_svg() -> Response {
    let mut response = icon_response("image/svg+xml", FAVICON_SVG);
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(SVG_CSP),
    );
    response
}

/// GET /icon-512.png
pub async fn icon_png() -> Response {
    icon_response("image/png", ICON_PNG)
}

/// Icons for the MCP `serverInfo.icons` field (MCP 2025-11-25 `Implementation`).
pub fn mcp_server_icons(state: &AppState) -> serde_json::Value {
    let base = state.config.base_url.trim_end_matches('/');
    serde_json::json!([
        {
            "src": format!("{base}/icon-512.png"),
            "mimeType": "image/png",
            "sizes": ["512x512"],
        },
        {
            "src": format!("{base}/favicon.svg"),
            "mimeType": "image/svg+xml",
            "sizes": ["any"],
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn assert_icon(response: Response, content_type: &str, expected: &[u8]) {
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some(content_type)
        );
        assert_eq!(
            response
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), expected);
    }

    #[tokio::test]
    async fn serves_each_icon_with_its_content_type() {
        assert_icon(favicon_ico().await, "image/x-icon", FAVICON_ICO).await;
        assert_icon(favicon_svg().await, "image/svg+xml", FAVICON_SVG).await;
        assert_icon(icon_png().await, "image/png", ICON_PNG).await;
    }

    #[tokio::test]
    async fn svg_allows_only_inline_styles() {
        let response = favicon_svg().await;
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_SECURITY_POLICY)
                .and_then(|value| value.to_str().ok()),
            Some(SVG_CSP)
        );
        assert!(
            !FAVICON_SVG
                .windows(b"<script".len())
                .any(|window| window.eq_ignore_ascii_case(b"<script"))
        );
    }

    #[test]
    fn embedded_icons_have_expected_formats() {
        // ICO header: reserved 0, type 1 (icon).
        assert_eq!(&FAVICON_ICO[..4], &[0, 0, 1, 0]);
        assert!(FAVICON_SVG.starts_with(b"<svg"));
        assert_eq!(&ICON_PNG[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[tokio::test]
    async fn mcp_server_icons_use_absolute_base_url() {
        let mut state = crate::test_utils::test_app_state_no_db().await;
        state.config.base_url = "https://api.example.test/".to_string();

        let icons = mcp_server_icons(&state);

        assert_eq!(icons[0]["src"], "https://api.example.test/icon-512.png");
        assert_eq!(icons[0]["mimeType"], "image/png");
        assert_eq!(icons[1]["src"], "https://api.example.test/favicon.svg");
        assert_eq!(icons[1]["sizes"][0], "any");
    }
}
