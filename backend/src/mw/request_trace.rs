//! HTTP diagnostics use the route registered with the router.
use std::borrow::Cow;

use axum::{extract::OriginalUri, http::Request};

pub(crate) fn route_label(extensions: &axum::http::Extensions) -> &str {
    extensions
        .get::<axum::extract::MatchedPath>()
        .map_or("<unmatched>", axum::extract::MatchedPath::as_str)
}

fn service_label<'a, B>(request: &'a Request<B>, route: &str) -> Option<Cow<'a, str>> {
    let (prefix, parameters) = route.split_once('{')?;
    let (name, _) = parameters.split_once('}')?;
    match (prefix, name) {
        ("/api/v1/proxy/s/" | "/public/s/", "slug")
        | ("/api/v1/llm/", "provider_slug")
        | (
            "/api/v1/proxy/"
            | "/api/v1/proxy/nyxbot-voice/"
            | "/api/v1/proxy/services/"
            | "/api/v1/public-nyxbot/models/",
            "service_id",
        ) => {}
        _ => return None,
    }

    // MatchedPath includes the nesting prefix. Pair it with the original URI.
    let uri = request
        .extensions()
        .get::<OriginalUri>()
        .map_or(request.uri(), |uri| &uri.0);
    let segment = uri.path().strip_prefix(prefix)?.split('/').next()?;
    let service = urlencoding::decode(segment).ok()?;
    if name == "service_id" {
        uuid::Uuid::try_parse(&service).ok()?;
    } else if service.is_empty()
        || service.len() > 100
        || service
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '?' | '#'))
    {
        return None;
    }
    Some(service)
}

pub fn span<B>(request: &Request<B>) -> tracing::Span {
    let path = route_label(request.extensions());
    tracing::info_span!(
        "http_request",
        method = %request.method(),
        path = %path,
        service = service_label(request, path).as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    use tower_http::trace::TraceLayer;
    use tracing::instrument::WithSubscriber;

    #[tokio::test]
    async fn http_logs_record_route_labels() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let (oauth, api) = crate::routes::build_router_with_state(state.clone());
        let app = oauth
            .merge(api)
            .with_state(state)
            .layer(TraceLayer::new_for_http().make_span_with(super::span::<Body>));
        let marker = "synthetic-route-value";
        let templates = [
            "/api/v1/connect-links/preview",
            "/api/v1/connect-links/complete",
            "/api/v1/connect-links/cancel",
            "/api/v1/channel-connect-links/preview",
            "/api/v1/channel-connect-links/complete",
            "/api/v1/channel-connect-links/decline",
            "/api/v1/channel-connect-links/retry",
            "/api/v1/channel-connect-links/managed/start",
            "/api/v1/channel-connect-links/managed/complete",
            "/api/v1/channel-connect-links/telegram/start",
            "/api/v1/auth/login-code/redeem",
            "/api/v1/auth/login-code/{id}",
            "/api/v1/devices/code/request",
            "/api/v1/devices/code/poll",
            "/api/v1/devices/code/approve",
            "/api/v1/auth/device/preview",
            "/api/v1/orgs/join/{nonce}",
            "/api/v1/users/me/oauth-consent-requests/{handle}",
            "/nodes/{node_id}/credentials/pending/{pending_id}/accept",
            "/nodes/credentials/pending/{pending_id}/fan-out/accept",
            "/credential-accept/assets/{*file}",
            "/api/v1/webhooks/triggers/{trigger_id}",
            "/api/v1/webhooks/channel/{platform}/{bot_id}",
            "/api/v1/webhooks/channel/telegram-new/manager/{id}",
            "/api/v1/webhooks/call-adapters/{id}",
            "/api/v1/webhooks/call-adapters/{id}/socket/{sid}",
            "/api/v1/webhooks/call-adapters/{id}/sms-receipts/{send}",
            "/oauth/authorize",
            "/health",
            "<unmatched>",
        ];
        let slug = "synthetic-service";
        let id = "03e802a8-73f0-4c24-b1ed-08bcd1f94963";
        let service_routes = [
            ("/api/v1/proxy/s/{slug}", slug),
            ("/api/v1/proxy/s/{slug}/", slug),
            ("/api/v1/proxy/s/{slug}/{*path}", slug),
            ("/api/v1/proxy/{service_id}", id),
            ("/api/v1/proxy/{service_id}/", id),
            ("/api/v1/proxy/{service_id}/{*path}", id),
            ("/api/v1/proxy/services/{service_id}/docs", id),
            ("/api/v1/proxy/nyxbot-voice/{service_id}/{*path}", id),
            ("/api/v1/public-nyxbot/models/{service_id}/{*path}", id),
            ("/api/v1/llm/{provider_slug}/v1/{*path}", slug),
            ("/public/s/{slug}", slug),
            ("/public/s/{slug}/", slug),
            ("/public/s/{slug}/{*path}", slug),
        ];
        let mut cases: Vec<_> = templates
            .into_iter()
            .map(|template| {
                let path = if template == "<unmatched>" {
                    format!("/unknown/{marker}")
                } else {
                    template
                        .split('/')
                        .map(|part| if part.starts_with('{') { marker } else { part })
                        .collect::<Vec<_>>()
                        .join("/")
                };
                (template, path, None)
            })
            .collect();
        for (template, service) in service_routes {
            let path = template
                .replace("{slug}", service)
                .replace("{service_id}", service)
                .replace("{provider_slug}", service)
                .replace("{*path}", &format!("mcp/{marker}"));
            cases.push((template, path, Some(service)));
        }
        cases.extend([
            (
                "/api/v1/llm/gateway/v1/{*path}",
                format!("/api/v1/llm/gateway/v1/{marker}"),
                None,
            ),
            ("/mcp", "/mcp".into(), None),
            ("/public/mcp", "/public/mcp".into(), None),
            (
                "/api/v1/permission-execution/mcp",
                "/api/v1/permission-execution/mcp".into(),
                None,
            ),
            (
                "/api/v1/proxy/s/{slug}/{*path}",
                format!("/api/v1/proxy/s/synthetic%2Dservice/{marker}"),
                Some(slug),
            ),
            (
                "/api/v1/proxy/s/{slug}/{*path}",
                format!("/api/v1/proxy/s/bad%2F{marker}/mcp"),
                None,
            ),
            (
                "/api/v1/proxy/{service_id}/{*path}",
                format!("/api/v1/proxy/{marker}/mcp"),
                None,
            ),
        ]);
        for (template, path, service) in cases {
            let captured = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
                .with_writer(captured.reopen().unwrap())
                .finish();
            // Exercise the registered routes without valid transport or authentication context.
            let request = Request::builder()
                .method("TRACE")
                .uri(format!("{path}?ticket={marker}"))
                .header("authorization", format!("Bearer {marker}"))
                .body(Body::empty())
                .unwrap();
            let response = app
                .clone()
                .oneshot(request)
                .with_subscriber(subscriber)
                .await
                .unwrap();
            assert!(
                response.status().is_client_error() || response.status().is_server_error(),
                "{template}"
            );
            let logs = std::fs::read_to_string(captured.path()).unwrap();
            assert!(
                logs.contains("http_request") && logs.contains("method=TRACE"),
                "{template}"
            );
            assert!(
                logs.contains(&format!("path={template}")),
                "{template}: {logs}"
            );
            if let Some(service) = service {
                assert!(
                    logs.contains(&format!(r#"service="{service}""#)),
                    "{template}: {logs}"
                );
            } else {
                assert!(!logs.contains(" service="), "{template}: {logs}");
            }
            assert!(!logs.contains(marker), "{template}");
        }
    }
}
