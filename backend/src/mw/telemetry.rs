//! Per-request middleware that derives `TelemetryContext` from the
//! `X-NyxID-Client` and `X-NyxID-Client-Version` request headers and
//! stashes it in request extensions so handlers can read it at emit
//! time.
//!
//! The middleware runs on every route (authenticated or not) because
//! header-only derivation is safe and side-effect-free. The `surface
//! = "agent"` override for API-key-authenticated requests happens at
//! emit time inside [`crate::telemetry::emit_event`], not here —
//! `AuthUser` is an axum `FromRequestParts` extractor, not a
//! middleware-populated extension, so the decision naturally lives
//! with the handler that holds the `AuthUser`.
//!
//! See `docs/TELEMETRY.md` §5.1 + review-history notes on the
//! middleware/extractor reconciliation.

use axum::{extract::Request, middleware::Next, response::Response};

use crate::telemetry::TelemetryContext;

const CLIENT_HEADER: &str = "x-nyxid-client";
const CLIENT_VERSION_HEADER: &str = "x-nyxid-client-version";

/// Header-only `TelemetryContext` builder. Runs on every request.
pub async fn telemetry_mw(mut req: Request, next: Next) -> Response {
    let client = req
        .headers()
        .get(CLIENT_HEADER)
        .and_then(|v| v.to_str().ok());
    let version = req
        .headers()
        .get(CLIENT_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        // Cap at 64 chars: semver strings are short; an absurdly long
        // header is likely malformed or an attacker probing.
        .filter(|v| v.len() <= 64);

    let ctx = TelemetryContext::from_headers(client, version);
    req.extensions_mut().insert(ctx);

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Extension, Json, Router,
        body::{Body, to_bytes},
        http::StatusCode,
        middleware,
        routing::get,
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn echo_context(Extension(ctx): Extension<TelemetryContext>) -> Json<Value> {
        Json(json!({
            "surface": ctx.surface,
            "client_version": ctx.client_version,
        }))
    }

    fn test_router() -> Router {
        Router::new()
            .route("/", get(echo_context))
            .layer(middleware::from_fn(telemetry_mw))
    }

    #[tokio::test]
    async fn telemetry_middleware_inserts_context_from_headers() {
        let response = test_router()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header(CLIENT_HEADER, "cli")
                    .header(CLIENT_VERSION_HEADER, "1.2.3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["surface"], "cli");
        assert_eq!(json["client_version"], "1.2.3");
    }

    #[tokio::test]
    async fn telemetry_middleware_falls_back_for_unknown_client() {
        let response = test_router()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header(CLIENT_HEADER, "browser-extension")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["surface"], "backend");
        assert!(json["client_version"].is_null());
    }

    #[tokio::test]
    async fn telemetry_middleware_drops_oversized_client_version() {
        let long_version = "v".repeat(65);
        let response = test_router()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header(CLIENT_HEADER, "sdk")
                    .header(CLIENT_VERSION_HEADER, long_version)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["surface"], "sdk");
        assert!(json["client_version"].is_null());
    }
}
