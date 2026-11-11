//! HTTP diagnostics deliberately omit query strings and authentication headers.
pub fn span<B>(request: &axum::http::Request<B>) -> tracing::Span {
    tracing::info_span!("http_request", method = %request.method(), path = %request.uri().path())
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };
    use tower::ServiceExt;
    use tower_http::trace::TraceLayer;
    use tracing::instrument::WithSubscriber;

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn actual_request_trace_contains_method_path_and_no_query_or_bearer() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
            .with_writer(move || Capture(writer.clone()))
            .finish();
        let app = Router::new()
            .fallback(|| async { StatusCode::NO_CONTENT })
            .layer(TraceLayer::new_for_http().make_span_with(super::span::<Body>));
        let path = "/api/v1/webhooks/call-adapters/binding/socket/session";
        let response = app.oneshot(Request::builder()
            .method("GET")
            .uri(format!("{path}?ticket=relay_ticket_must_never_be_logged&other=query_value_must_never_be_logged"))
            .header("authorization", "Bearer header_bearer_must_never_be_logged")
            .body(Body::empty()).unwrap())
            .with_subscriber(subscriber).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let trace = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(
            trace.contains("http_request"),
            "request span must actually be captured"
        );
        assert!(trace.contains("method=GET"));
        assert!(trace.contains(&format!("path={path}")));
        for forbidden in [
            "?",
            "ticket=",
            "relay_ticket_must_never_be_logged",
            "query_value_must_never_be_logged",
            "Bearer",
            "header_bearer_must_never_be_logged",
        ] {
            assert!(
                !trace.contains(forbidden),
                "request diagnostics exposed {forbidden}"
            );
        }
    }
}
