//! Opaque support IDs for assistant HTTP requests. Never log request bodies,
//! query strings, credentials, or upstream error text here.
use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use tracing::Instrument;

pub const TRACE_HEADER: &str = "x-nyxid-trace-id";

pub async fn assistant_trace(mut request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if path != "/api/v1/assistant"
        && !path.starts_with("/api/v1/assistant/")
        && !path.starts_with("/api/v1/nyxbot/")
    {
        return next.run(request).await;
    }
    // Always issue our own ID; callers and upstreams cannot impersonate one.
    let ingress = crate::turn_trace::Ingress::new();
    let trace_id = ingress.request_id.to_string();
    request.extensions_mut().insert(ingress.clone());
    let span =
        tracing::info_span!("assistant_request", trace_id = %trace_id, method = %request.method());
    async move {
        let mut response = ingress.scope(next.run(request)).await;
        ingress.finish(response.status().as_u16());
        let context = ingress.admitted().or_else(|| {
            response
                .extensions()
                .get::<crate::turn_trace::TurnContext>()
                .copied()
        });
        if let Some(context) = context {
            context.response_header(&mut response);
        } else {
            response.headers_mut().insert(
                TRACE_HEADER,
                HeaderValue::from_str(&trace_id).expect("UUID header"),
            );
        }
        response.headers_mut().insert(
            crate::turn_trace::REQUEST_HEADER,
            HeaderValue::from_str(&trace_id).expect("UUID header"),
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
    async fn preserves_typed_handler_association_without_trusting_raw_headers() {
        let turn = uuid::Uuid::new_v4().to_string();
        let context = crate::turn_trace::TurnContext::for_turn(&turn).unwrap();
        let app = Router::new()
            .route(
                "/api/v1/assistant/turn",
                get(move || async move {
                    let mut response = Response::new(Body::empty());
                    context.response_header(&mut response);
                    response
                }),
            )
            .layer(middleware::from_fn(assistant_trace));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/assistant/turn")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()[TRACE_HEADER], turn);
        assert_ne!(response.headers()[crate::turn_trace::REQUEST_HEADER], turn);
    }

    #[tokio::test]
    async fn canonical_turn_header_is_distinct_from_request_and_does_not_wait_for_stream() {
        let turn = uuid::Uuid::new_v4().to_string();
        let admitted = turn.clone();
        let app = Router::new()
            .route(
                "/api/v1/assistant/turn",
                get(move || {
                    let admitted = admitted.clone();
                    async move {
                        crate::turn_trace::admitted(&admitted, chrono::Utc::now(), None);
                        Sse::new(futures::stream::pending::<Result<Event, Infallible>>())
                    }
                }),
            )
            .layer(middleware::from_fn(assistant_trace));
        let response = tokio::time::timeout(
            Duration::from_secs(1),
            app.oneshot(
                Request::builder()
                    .uri("/api/v1/assistant/turn")
                    .header(TRACE_HEADER, "untrusted")
                    .header("x-request-id", "untrusted")
                    .body(Body::empty())
                    .unwrap(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.headers()[TRACE_HEADER], turn);
        let request = response.headers()["x-request-id"].to_str().unwrap();
        assert_ne!(request, turn);
        assert!(uuid::Uuid::parse_str(request).is_ok());
    }

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
