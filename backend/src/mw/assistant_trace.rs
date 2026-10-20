//! Opaque support IDs for assistant HTTP requests. Never log request bodies,
//! query strings, credentials, or upstream error text here.
use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use tracing::Instrument;

pub const TRACE_HEADER: &str = "x-nyxid-trace-id";

pub async fn assistant_trace(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if path != "/api/v1/assistant" && !path.starts_with("/api/v1/assistant/") {
        return next.run(request).await;
    }
    // Always issue our own ID; callers and upstreams cannot impersonate one.
    let trace_id = uuid::Uuid::new_v4().to_string();
    let span =
        tracing::info_span!("assistant_request", trace_id = %trace_id, method = %request.method());
    async move {
        let mut response = next.run(request).await;
        response.headers_mut().insert(
            TRACE_HEADER,
            HeaderValue::from_str(&trace_id).expect("UUID is a valid header"),
        );
        if response.status().is_client_error() || response.status().is_server_error() {
            tracing::warn!(
                status = response.status().as_u16(),
                "Assistant request failed"
            );
        } else {
            // A stream can break in the browser even when execution succeeds.
            // Keep its returned ID searchable in that case too.
            tracing::info!(
                status = response.status().as_u16(),
                "Assistant response started"
            );
        }
        response
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::StatusCode,
        middleware,
        response::{Sse, sse::Event},
        routing::get,
    };
    use std::{convert::Infallible, time::Duration};
    use tower::ServiceExt;

    #[tokio::test]
    async fn traces_errors_and_streams_without_trusting_supplied_ids() {
        let app = Router::new()
            .route(
                "/api/v1/assistant/error",
                get(|| async { StatusCode::BAD_GATEWAY }),
            )
            .route(
                "/api/v1/assistant/stream",
                get(|| async { Sse::new(futures::stream::pending::<Result<Event, Infallible>>()) }),
            )
            .route("/unrelated", get(|| async { "ok" }))
            .layer(middleware::from_fn(assistant_trace));
        let mut previous = String::new();
        for path in [
            "/api/v1/assistant/error",
            "/api/v1/assistant/stream",
            "/api/v1/assistant/missing",
        ] {
            // Headers must be returned without waiting for the SSE body.
            let response = tokio::time::timeout(
                Duration::from_secs(1),
                app.clone().oneshot(
                    Request::builder()
                        .uri(path)
                        .header(TRACE_HEADER, "caller-controlled")
                        .body(Body::empty())
                        .unwrap(),
                ),
            )
            .await
            .unwrap()
            .unwrap();
            let id = response.headers()[TRACE_HEADER].to_str().unwrap();
            assert!(uuid::Uuid::parse_str(id).is_ok());
            assert_ne!(id, previous);
            previous = id.to_owned();
        }
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/unrelated")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!response.headers().contains_key(TRACE_HEADER));
    }
}
