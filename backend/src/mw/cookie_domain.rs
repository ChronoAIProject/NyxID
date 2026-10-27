//! Host-aware domains for NyxID's own auth cookies during an opt-in migration.
use crate::{AppState, config::AppConfig};
use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{HeaderMap, Request, header},
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;

fn request_authority(headers: &HeaderMap, trusted_proxy: bool) -> Option<&str> {
    let name = if trusted_proxy && headers.contains_key("x-forwarded-host") {
        "x-forwarded-host"
    } else {
        "host"
    };
    if headers.get_all(name).iter().count() != 1 {
        return None;
    }
    let raw = headers.get(name)?.to_str().ok()?;
    // A single authority, never a comma-separated chain, URL, or userinfo.
    if raw
        .bytes()
        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        || raw.contains([',', '/', '\\', '@', '?', '#'])
    {
        return None;
    }
    raw.parse::<axum::http::uri::Authority>().ok()?;
    Some(raw)
}

fn alias_domain<'a>(
    config: &'a AppConfig,
    headers: &HeaderMap,
    trusted_proxy: bool,
) -> Option<&'a str> {
    let authority: axum::http::uri::Authority =
        request_authority(headers, trusted_proxy)?.parse().ok()?;
    let host = authority.host().to_ascii_lowercase();
    config
        .domain_aliases
        .cookie_domains
        .iter()
        .filter(|domain| {
            let domain = domain.trim_start_matches('.');
            host == domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
        .max_by_key(|domain| domain.trim_start_matches('.').len())
        .map(String::as_str)
}

/// Transport-only selection, constructed from the configured API origin set.
#[derive(Clone)]
pub struct SocialCallbackOrigin(pub(crate) String);

fn callback_origin<'a>(
    config: &'a AppConfig,
    headers: &HeaderMap,
    trusted_proxy: bool,
) -> Option<&'a str> {
    if config.domain_aliases.base_urls.is_empty() {
        return None;
    }
    let authority: axum::http::uri::Authority =
        request_authority(headers, trusted_proxy)?.parse().ok()?;
    config.accepted_base_urls().find(|base| {
        url::Url::parse(base).is_ok_and(|url| {
            url.host_str()
                .is_some_and(|host| host.eq_ignore_ascii_case(authority.host()))
                && url.port_or_known_default()
                    == authority.port_u16().or_else(|| match url.scheme() {
                        "https" => Some(443),
                        "http" => Some(80),
                        _ => None,
                    })
        })
    })
}

pub async fn middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if (state.config.domain_aliases.cookie_domains.is_empty()
        && state.config.domain_aliases.base_urls.is_empty())
        || !request.uri().path().starts_with("/api/v1/auth/")
    {
        return next.run(request).await;
    }
    let trusted = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|ConnectInfo(peer)| {
            super::rate_limit::is_trusted_proxy(peer.ip(), &state.config.trusted_proxy_ips)
        });
    let domain = alias_domain(&state.config, request.headers(), trusted).map(str::to_owned);
    if let Some(origin) = callback_origin(&state.config, request.headers(), trusted) {
        request
            .extensions_mut()
            .insert(SocialCallbackOrigin(origin.to_owned()));
    }
    let mut response = next.run(request).await;
    if let Some(domain) = domain {
        let cookies: Vec<_> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .cloned()
            .collect();
        response.headers_mut().remove(header::SET_COOKIE);
        for cookie in cookies {
            let replacement = cookie
                .to_str()
                .ok()
                .and_then(|value| {
                    let mut parts = value.split(';');
                    let first = parts.next()?;
                    // No downstream cookies: this layer only handles first-party auth routes.
                    if !first.starts_with("nyx_") {
                        return None;
                    }
                    let mut value = first.to_owned();
                    for part in parts {
                        if !part.trim().to_ascii_lowercase().starts_with("domain=") {
                            value.push(';');
                            value.push_str(part);
                        }
                    }
                    value.push_str("; Domain=");
                    value.push_str(&domain);
                    value.parse().ok()
                })
                .unwrap_or(cookie);
            response
                .headers_mut()
                .append(header::SET_COOKIE, replacement);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use tower::ServiceExt;

    async fn app(aliases: bool) -> Router {
        let mut state = crate::test_utils::test_app_state_no_db().await;
        state.config.cookie_domain = Some(".new.example".into());
        state.config.trusted_proxy_ips = vec!["127.0.0.1".parse().unwrap()];
        if aliases {
            state.config.domain_aliases.cookie_domains = vec![".old.example".into()];
        }
        async fn cookies() -> HeaderMap {
            let mut headers = HeaderMap::new();
            for cookie in [
                "nyx_social_state=value; Path=/api/v1/auth/social; HttpOnly; Secure; SameSite=Lax; Domain=.new.example",
                "nyx_session=; Max-Age=0; Path=/; Domain=.new.example",
                "unrelated=value; Domain=.new.example",
            ] {
                headers.append(header::SET_COOKIE, cookie.parse().unwrap());
            }
            headers
        }
        Router::new()
            .route("/api/v1/auth/social/provider/callback", get(cookies))
            .route("/api/v1/proxy/s/test", get(cookies))
            .layer(axum::middleware::from_fn_with_state(state, middleware))
    }

    async fn response_cookies(
        app: Router,
        host: &str,
        forwarded: Option<&str>,
        peer: &str,
        path: &str,
    ) -> Vec<String> {
        let mut request = Request::builder().uri(path).header(header::HOST, host);
        if let Some(value) = forwarded {
            request = request.header("x-forwarded-host", value);
        }
        let mut request = request.body(Body::empty()).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        app.oneshot(request)
            .await
            .unwrap()
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|h| h.to_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn domain_alias_social_route_keeps_callback_and_cookie_on_the_same_configured_api() {
        let mut state = crate::test_utils::test_app_state_no_db().await;
        state.config.base_url = "https://new.example".into();
        state.config.cookie_domain = Some(".new.example".into());
        state.config.domain_aliases.base_urls = vec!["https://old.example".into()];
        state.config.domain_aliases.cookie_domains = vec![".old.example".into()];
        state.config.github_client_id = Some(uuid::Uuid::new_v4().to_string());
        state.config.github_client_secret = Some(uuid::Uuid::new_v4().to_string());
        let (_, private) = crate::routes::build_router_with_state(state.clone());
        let app = private.with_state(state);
        for (host, expected) in [
            ("old.example", "https://old.example"),
            ("unconfigured.example", "https://new.example"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/v1/auth/social/github")
                        .header(header::HOST, host)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::FOUND);
            let location =
                url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
            let redirect = location
                .query_pairs()
                .find(|(key, _)| key == "redirect_uri")
                .unwrap()
                .1
                .into_owned();
            assert_eq!(
                redirect,
                format!("{expected}/api/v1/auth/social/github/callback")
            );
            let cookies = response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .map(|c| c.to_str().unwrap())
                .collect::<Vec<_>>();
            assert!(cookies.iter().any(|c| c.starts_with("nyx_social_state=")
                && c.contains(&format!(
                    "Domain=.{}",
                    expected.trim_start_matches("https://")
                ))));
        }
    }

    #[test]
    fn domain_alias_callback_origin_does_not_trust_forwarded_hosts_or_ports() {
        let mut config = crate::test_utils::test_app_config();
        config.base_url = "https://new.example".into();
        config.domain_aliases.base_urls = vec!["https://old.example".into()];
        let mut headers = HeaderMap::new();
        headers.insert("host", "new.example".parse().unwrap());
        headers.insert("x-forwarded-host", "old.example".parse().unwrap());
        assert_eq!(
            callback_origin(&config, &headers, false),
            Some("https://new.example")
        );
        assert_eq!(
            callback_origin(&config, &headers, true),
            Some("https://old.example")
        );
        headers.insert("x-forwarded-host", "old.example:444".parse().unwrap());
        assert_eq!(callback_origin(&config, &headers, true), None);
        headers.insert("x-forwarded-host", "old.example.evil".parse().unwrap());
        assert_eq!(callback_origin(&config, &headers, true), None);
        config.domain_aliases.base_urls.clear();
        assert_eq!(callback_origin(&config, &headers, false), None);
    }

    #[tokio::test]
    async fn domain_alias_cookies_use_only_the_matching_configured_domain() {
        let app = app(true).await;
        for host in ["api.old.example", "old.example", "API.OLD.EXAMPLE:443"] {
            let cookies = response_cookies(
                app.clone(),
                host,
                None,
                "127.0.0.2:1234",
                "/api/v1/auth/social/provider/callback",
            )
            .await;
            assert!(cookies[0].contains("; Domain=.old.example"));
            assert!(cookies[0].contains("HttpOnly; Secure; SameSite=Lax"));
            assert!(cookies[1].contains("Max-Age=0; Path=/; Domain=.old.example"));
            assert_eq!(cookies[2], "unrelated=value; Domain=.new.example");
        }
        for host in [
            "api.new.example",
            "old.example.evil",
            "evilold.example",
            "old.example@evil",
        ] {
            let cookies = response_cookies(
                app.clone(),
                host,
                None,
                "127.0.0.2:1234",
                "/api/v1/auth/social/provider/callback",
            )
            .await;
            assert!(
                cookies.iter().all(|c| c.contains("Domain=.new.example")),
                "{host}"
            );
        }
    }

    #[tokio::test]
    async fn domain_alias_cookies_forwarded_host_requires_a_trusted_peer() {
        let app = app(true).await;
        for (peer, forwarded, expected) in [
            ("127.0.0.1:1234", "api.old.example", ".old.example"),
            ("127.0.0.2:1234", "api.old.example", ".new.example"),
            ("127.0.0.1:1234", "api.old.example, evil", ".new.example"),
        ] {
            let cookies = response_cookies(
                app.clone(),
                "internal.example",
                Some(forwarded),
                peer,
                "/api/v1/auth/social/provider/callback",
            )
            .await;
            assert!(cookies[0].contains(&format!("Domain={expected}")));
        }
    }

    #[tokio::test]
    async fn domain_alias_cookies_unset_and_proxy_responses_are_unchanged() {
        for (aliases, path) in [
            (false, "/api/v1/auth/social/provider/callback"),
            (true, "/api/v1/proxy/s/test"),
        ] {
            let cookies = response_cookies(
                app(aliases).await,
                "api.old.example",
                None,
                "127.0.0.2:1234",
                path,
            )
            .await;
            assert!(cookies.iter().all(|c| c.contains("Domain=.new.example")));
        }
    }
}
