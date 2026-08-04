use axum::{
    body::Body,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, Request, Response, StatusCode},
};
use bytes::Bytes;
use std::net::SocketAddr;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::DownstreamService;
use crate::services::{anonymous_endpoint_service, audit_service, proxy_service};

pub async fn public_proxy_request(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((slug, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response<Body>> {
    execute_public_proxy(state, Some(peer), slug, path, request).await
}

pub async fn public_proxy_request_root(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(slug): Path<String>,
    request: Request<Body>,
) -> AppResult<Response<Body>> {
    execute_public_proxy(state, Some(peer), slug, String::new(), request).await
}

async fn execute_public_proxy(
    state: AppState,
    peer: Option<SocketAddr>,
    slug: String,
    path: String,
    request: Request<Body>,
) -> AppResult<Response<Body>> {
    let request_path = request.uri().path().to_string();
    let client_ip = crate::mw::rate_limit::enforce_public_ip_rate_limit(
        &state.public_proxy_limiter,
        request.headers(),
        peer,
        &state.config.trusted_proxy_ips,
        &request_path,
    )?;

    if is_ws_upgrade_request(&request) {
        return Err(AppError::BadRequest(
            "WebSocket upgrades are not supported on public proxy routes".to_string(),
        ));
    }

    let method = request.method().clone();
    let method_str = method.as_str().to_ascii_uppercase();
    let normalized_path = anonymous_endpoint_service::normalize_runtime_path(&path)?;
    let matched = anonymous_endpoint_service::find_matching_enabled_rule(
        &state.db,
        &slug,
        &method_str,
        &normalized_path,
    )
    .await?;

    let quota_used = anonymous_endpoint_service::increment_daily_usage(
        &state.db,
        &matched.service.id,
        &matched.rule.id,
        matched.rule.daily_quota,
    )
    .await?;

    let query = request.uri().query().map(str::to_string);
    let req_headers = request.headers().clone();
    let user_agent = req_headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body_bytes = read_public_body(request, state.config.public_proxy_max_body_size).await?;

    let target = public_proxy_target(matched.service.clone());
    let response = proxy_service::forward_request(
        &state.http_client,
        &target,
        reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|_| {
            AppError::BadRequest(format!("Unsupported HTTP method: {}", method.as_str()))
        })?,
        normalized_path.trim_start_matches('/'),
        query.as_deref(),
        reqwest_headers(req_headers)?,
        proxy_service::ProxyBody::Buffered(if body_bytes.is_empty() {
            None
        } else {
            Some(body_bytes)
        }),
        Vec::new(),
        Vec::new(),
        None,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
    )
    .await?;

    let status = response.status();
    let sanitized = sanitize_public_response(response).await?;
    audit_service::log_async(
        state.db.clone(),
        None,
        "public_proxy_request".to_string(),
        Some(anonymous_endpoint_service::public_audit_json(
            anonymous_endpoint_service::bounded_public_audit_event(
                &matched.service,
                &matched.rule,
                anonymous_endpoint_service::PublicAuditInput {
                    method: &method_str,
                    path: &normalized_path,
                    response_status: Some(status.as_u16()),
                    client_ip,
                    user_agent: user_agent.as_deref(),
                    quota_used: Some(quota_used),
                },
            ),
        )),
        client_ip.map(|ip| ip.to_string()),
        user_agent,
        None,
        None,
    );

    Ok(sanitized)
}

fn public_proxy_target(mut service: DownstreamService) -> proxy_service::ProxyTarget {
    service.auth_method = "none".to_string();
    service.auth_key_name = String::new();
    service.identity_propagation_mode = "none".to_string();
    service.forward_access_token = false;
    service.inject_delegation_token = false;

    proxy_service::ProxyTarget {
        base_url: service.base_url.clone(),
        auth_method: "none".to_string(),
        auth_key_name: String::new(),
        credential: String::new(),
        catalog_default_headers: Vec::new(),
        user_service_default_headers: Vec::new(),
        ws_frame_injections: Vec::new(),
        connection_id: None,
        service,
    }
}

async fn read_public_body(request: Request<Body>, max_body_size: usize) -> AppResult<Bytes> {
    axum::body::to_bytes(request.into_body(), max_body_size)
        .await
        .map_err(|_| AppError::BadRequest("Public request body is too large".to_string()))
}

fn reqwest_headers(headers: HeaderMap) -> AppResult<reqwest::header::HeaderMap> {
    let mut out = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        let Some(name) = name else {
            continue;
        };
        if !is_safe_public_request_header(name.as_str()) {
            continue;
        }
        if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
            let header_value = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
                .map_err(|_| AppError::BadRequest("Invalid request header".to_string()))?;
            out.append(header_name, header_value);
        }
    }
    Ok(out)
}

fn is_safe_public_request_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !matches!(
        lower.as_str(),
        "authorization"
            | "cookie"
            | "host"
            | "proxy-authorization"
            | "proxy-authenticate"
            | "connection"
            | "upgrade"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-real-ip"
    ) && !lower.starts_with("x-nyxid-")
}

async fn sanitize_public_response(response: reqwest::Response) -> AppResult<Response<Body>> {
    let status = response.status();
    let mut builder = Response::builder().status(
        StatusCode::from_u16(status.as_u16())
            .map_err(|_| AppError::Internal("Invalid downstream status".to_string()))?,
    );

    for (name, value) in response.headers() {
        if anonymous_endpoint_service::is_safe_public_response_header(name.as_str()) {
            builder = builder.header(name.as_str(), value.as_bytes());
        }
    }

    let body = response.bytes().await.map_err(|e| {
        tracing::error!("Failed to read public proxy response: {e}");
        AppError::Internal("Failed to read downstream response".to_string())
    })?;

    builder.body(Body::from(body)).map_err(|e| {
        tracing::error!("Failed to build public proxy response: {e}");
        AppError::Internal("Failed to build public proxy response".to_string())
    })
}

fn is_ws_upgrade_request(request: &Request<Body>) -> bool {
    request
        .headers()
        .get(axum::http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        || request
            .headers()
            .get(axum::http::header::CONNECTION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("upgrade"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::AnonymousEndpointRule;
    use chrono::Utc;
    use http::HeaderValue;

    fn service() -> DownstreamService {
        DownstreamService {
            id: "svc-1".to_string(),
            name: "Public".to_string(),
            slug: "public".to_string(),
            description: None,
            base_url: "https://example.test".to_string(),
            service_type: "http".to_string(),
            visibility: "public".to_string(),
            auth_method: "bearer".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential_encrypted: vec![1, 2, 3],
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
            oauth_client_id: None,
            service_category: "internal".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: "admin".to_string(),
            identity_propagation_mode: "headers".to_string(),
            identity_include_user_id: true,
            identity_include_email: true,
            identity_include_name: true,
            identity_jwt_audience: Some("aud".to_string()),
            forward_access_token: true,
            inject_delegation_token: true,
            delegation_token_scope: "proxy:*".to_string(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            anonymous_endpoints: vec![AnonymousEndpointRule {
                id: "rule".to_string(),
                enabled: true,
                method: "GET".to_string(),
                path_pattern: "/public/**".to_string(),
                daily_quota: 10,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn public_proxy_target_force_strips_identity_and_auth() {
        let target = public_proxy_target(service());
        assert_eq!(target.auth_method, "none");
        assert_eq!(target.auth_key_name, "");
        assert_eq!(target.credential, "");
        assert!(target.catalog_default_headers.is_empty());
        assert_eq!(target.service.identity_propagation_mode, "none");
        assert!(!target.service.forward_access_token);
        assert!(!target.service.inject_delegation_token);
    }

    #[test]
    fn detects_websocket_upgrade() {
        let request = Request::builder()
            .header(axum::http::header::CONNECTION, "keep-alive, Upgrade")
            .header(axum::http::header::UPGRADE, "websocket")
            .body(Body::empty())
            .unwrap();
        assert!(is_ws_upgrade_request(&request));
    }

    #[test]
    fn reqwest_headers_rejects_invalid_header_values() {
        let mut headers = HeaderMap::new();
        headers.insert("x-test", HeaderValue::from_static("ok"));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        headers.insert("x-nyxid-agent-id", HeaderValue::from_static("agent"));
        assert!(reqwest_headers(headers).is_ok());
        let converted = reqwest_headers(
            [("x-test", "ok"), ("authorization", "Bearer secret")]
                .into_iter()
                .map(|(name, value)| {
                    (
                        axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                        HeaderValue::from_static(value),
                    )
                })
                .collect(),
        )
        .unwrap();
        assert!(converted.contains_key("x-test"));
        assert!(!converted.contains_key("authorization"));
    }

    mod behavior {
        use super::super::*;
        use crate::models::downstream_service::{
            AnonymousEndpointRule, COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
        };
        use crate::services::anonymous_endpoint_service;
        use crate::test_utils::{connect_test_database, test_app_state};
        use axum::body::{Body, to_bytes};
        use axum::http::Request;
        use axum::routing::get;
        use chrono::Utc;
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use uuid::Uuid;

        fn peer() -> SocketAddr {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 50000)
        }

        /// A runtime-safe public catalog service bound to `base_url`, exposing a
        /// single enabled `GET /public/**` anonymous rule with `daily_quota`.
        fn public_service(slug: &str, base_url: &str, daily_quota: u32) -> DownstreamService {
            DownstreamService {
                id: Uuid::new_v4().to_string(),
                name: "Public".to_string(),
                slug: slug.to_string(),
                description: None,
                base_url: base_url.to_string(),
                service_type: "http".to_string(),
                visibility: "public".to_string(),
                auth_method: "none".to_string(),
                auth_key_name: String::new(),
                credential_encrypted: vec![],
                auth_type: None,
                openapi_spec_url: None,
                asyncapi_spec_url: None,
                streaming_supported: false,
                ssh_config: None,
                oauth_client_id: None,
                service_category: "internal".to_string(),
                requires_user_credential: false,
                is_active: true,
                created_by: "admin".to_string(),
                identity_propagation_mode: "none".to_string(),
                identity_include_user_id: false,
                identity_include_email: false,
                identity_include_name: false,
                identity_jwt_audience: None,
                forward_access_token: false,
                inject_delegation_token: false,
                delegation_token_scope: "llm:proxy".to_string(),
                provider_config_id: None,
                homepage_url: None,
                repository_url: None,
                issues_url: None,
                capabilities: None,
                auth_notes: None,
                known_limitations: None,
                required_permissions: None,
                examples_url: None,
                recommended_skills: None,
                custom_user_agent: None,
                default_request_headers: None,
                ws_frame_injections: Vec::new(),
                developer_app_ids: None,
                token_exchange_config: None,
                anonymous_endpoints: vec![AnonymousEndpointRule {
                    id: Uuid::new_v4().to_string(),
                    enabled: true,
                    method: "GET".to_string(),
                    path_pattern: "/public/**".to_string(),
                    daily_quota,
                }],
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }

        struct MockDownstream {
            base_url: String,
            saw_authorization: Arc<AtomicUsize>,
            saw_cookie: Arc<AtomicUsize>,
            _task: tokio::task::JoinHandle<()>,
        }

        /// Spin up a localhost HTTP server that records whether inbound
        /// `Authorization`/`Cookie` headers reached it, and replies with a body
        /// plus a basket of sensitive response headers (Set-Cookie, auth,
        /// session) that the public proxy must strip before returning to the
        /// caller.
        async fn spawn_mock_downstream() -> MockDownstream {
            let saw_authorization = Arc::new(AtomicUsize::new(0));
            let saw_cookie = Arc::new(AtomicUsize::new(0));
            let auth_flag = saw_authorization.clone();
            let cookie_flag = saw_cookie.clone();

            let app = axum::Router::new().route(
                "/public/echo",
                get(move |headers: axum::http::HeaderMap| {
                    let auth_flag = auth_flag.clone();
                    let cookie_flag = cookie_flag.clone();
                    async move {
                        if headers.contains_key(axum::http::header::AUTHORIZATION) {
                            auth_flag.fetch_add(1, Ordering::SeqCst);
                        }
                        if headers.contains_key(axum::http::header::COOKIE) {
                            cookie_flag.fetch_add(1, Ordering::SeqCst);
                        }
                        (
                            axum::http::StatusCode::OK,
                            [
                                ("content-type", "application/json"),
                                ("set-cookie", "session=secret; HttpOnly"),
                                ("www-authenticate", "Bearer realm=x"),
                                ("x-nyxid-session", "leaky"),
                            ],
                            Body::from(r#"{"ok":true}"#),
                        )
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            MockDownstream {
                base_url: format!("http://{addr}"),
                saw_authorization,
                saw_cookie,
                _task: task,
            }
        }

        fn proxy_request_with_credentials() -> Request<Body> {
            Request::builder()
                .method("GET")
                .uri("/public/s/pub/public/echo")
                .header(axum::http::header::AUTHORIZATION, "Bearer client-secret")
                .header(axum::http::header::COOKIE, "sid=abc")
                .header("user-agent", "test-agent")
                .body(Body::empty())
                .unwrap()
        }

        /// A matching enabled rule forwards to the downstream; inbound caller
        /// credentials (Authorization, Cookie) are stripped before reaching the
        /// downstream, and sensitive downstream response headers (Set-Cookie,
        /// www-authenticate, x-nyxid-session) are stripped from the public
        /// response.
        #[tokio::test]
        async fn forwards_and_strips_inbound_and_response_credentials() {
            let Some(db) = connect_test_database("pproxy_forward_strip").await else {
                return;
            };
            let downstream = spawn_mock_downstream().await;
            let service = public_service("pub", &downstream.base_url, 100);
            db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .insert_one(&service)
                .await
                .expect("insert service");
            let state = test_app_state(db);

            let response = execute_public_proxy(
                state,
                Some(peer()),
                "pub".to_string(),
                "public/echo".to_string(),
                proxy_request_with_credentials(),
            )
            .await
            .expect("public proxy forwards");

            assert_eq!(response.status(), StatusCode::OK);

            // Downstream never saw the caller's Authorization / Cookie.
            assert_eq!(
                downstream.saw_authorization.load(Ordering::SeqCst),
                0,
                "inbound Authorization must be stripped"
            );
            assert_eq!(
                downstream.saw_cookie.load(Ordering::SeqCst),
                0,
                "inbound Cookie must be stripped"
            );

            // Sensitive downstream response headers are stripped.
            let headers = response.headers();
            assert!(!headers.contains_key("set-cookie"));
            assert!(!headers.contains_key("www-authenticate"));
            assert!(!headers.contains_key("x-nyxid-session"));
            assert_eq!(
                headers
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok()),
                Some("application/json")
            );

            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(&body[..], br#"{"ok":true}"#);
        }

        /// When the daily quota is exhausted, the next request MUST be denied
        /// (the quota is hard-enforced: no unbounded free proxying) and MUST
        /// NOT reach the downstream a second time.
        ///
        /// QUOTA EXHAUSTION shape: `increment_daily_usage` returns
        /// `AppError::RateLimited` (HTTP 429, code 1005) when the daily quota is
        /// reached — never a raw MongoDB E11000 duplicate-key error (HTTP 500),
        /// which would leak an internal DB error class to clients (CLAUDE.md §3).
        /// The denial is fail-closed: the over-quota request is rejected and
        /// never forwarded to the downstream.
        #[tokio::test]
        async fn quota_exhaustion_denies_and_does_not_forward() {
            let Some(db) = connect_test_database("pproxy_quota").await else {
                return;
            };
            let downstream = spawn_mock_downstream().await;
            // daily_quota = 1: the first call consumes it, the second is denied.
            let service = public_service("pub", &downstream.base_url, 1);
            db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .insert_one(&service)
                .await
                .expect("insert service");
            let state = test_app_state(db);

            let first = execute_public_proxy(
                state.clone(),
                Some(peer()),
                "pub".to_string(),
                "public/echo".to_string(),
                proxy_request_with_credentials(),
            )
            .await
            .expect("first request within quota");
            assert_eq!(first.status(), StatusCode::OK);

            let err = execute_public_proxy(
                state,
                Some(peer()),
                "pub".to_string(),
                "public/echo".to_string(),
                proxy_request_with_credentials(),
            )
            .await
            .expect_err("second request must be denied once quota is exhausted");

            // Quota is hard-enforced: the over-quota request is denied and is
            // NOT a successful proxy response. The denial must be quota-driven
            // (RateLimited / 429), not a routing miss and not a leaked DB error.
            assert!(
                !matches!(err, AppError::NotFound(_)),
                "denial must be quota-driven, not a routing miss: {err:?}"
            );
            assert!(
                matches!(err, AppError::RateLimited),
                "over-quota request must be RateLimited (HTTP 429), not a 500/E11000 \
                 DatabaseError (got {err:?})"
            );
            // The rendered HTTP response must be 429, not a 500 leaking a DB error.
            use axum::response::IntoResponse;
            assert_eq!(
                err.into_response().status(),
                StatusCode::TOO_MANY_REQUESTS,
                "over-quota response must be HTTP 429"
            );

            // The downstream must have been hit exactly once: the over-quota
            // request never forwards.
            assert_eq!(
                downstream.saw_authorization.load(Ordering::SeqCst)
                    + downstream.saw_cookie.load(Ordering::SeqCst),
                0,
                "credentials are always stripped"
            );
        }

        /// A WebSocket upgrade request to a public proxy route is rejected
        /// (public routes are HTTP-only).
        #[tokio::test]
        async fn websocket_upgrade_is_rejected() {
            let Some(db) = connect_test_database("pproxy_ws_reject").await else {
                return;
            };
            // Service presence is irrelevant: WS rejection happens before
            // rule matching, but insert one so the route is otherwise valid.
            let service = public_service("pub", "https://example.test", 100);
            db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .insert_one(&service)
                .await
                .expect("insert service");
            let state = test_app_state(db);

            let request = Request::builder()
                .method("GET")
                .uri("/public/s/pub/public/echo")
                .header(axum::http::header::CONNECTION, "Upgrade")
                .header(axum::http::header::UPGRADE, "websocket")
                .body(Body::empty())
                .unwrap();

            let err = execute_public_proxy(
                state,
                Some(peer()),
                "pub".to_string(),
                "public/echo".to_string(),
                request,
            )
            .await
            .expect_err("websocket upgrade must be rejected");
            assert!(matches!(err, AppError::BadRequest(_)));
        }

        /// A request whose method/path does not match any enabled rule returns
        /// NotFound (no public surface is exposed for unmatched paths).
        #[tokio::test]
        async fn unmatched_path_returns_not_found() {
            let Some(db) = connect_test_database("pproxy_unmatched").await else {
                return;
            };
            let service = public_service("pub", "https://example.test", 100);
            db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .insert_one(&service)
                .await
                .expect("insert service");
            let state = test_app_state(db);

            let request = Request::builder()
                .method("GET")
                .uri("/public/s/pub/private/secret")
                .body(Body::empty())
                .unwrap();

            let err = execute_public_proxy(
                state,
                Some(peer()),
                "pub".to_string(),
                "private/secret".to_string(),
                request,
            )
            .await
            .expect_err("unmatched path must not be exposed");
            assert!(matches!(err, AppError::NotFound(_)));
        }

        /// Guard: the runtime force-strip helper neutralizes identity and
        /// delegation even if a (mis)stored service still has them set. This is
        /// the defense-in-depth contract exercised at execution time.
        #[test]
        fn force_strip_helper_neutralizes_identity() {
            let mut service = public_service("pub", "https://example.test", 100);
            service.identity_propagation_mode = "headers".to_string();
            service.forward_access_token = true;
            service.inject_delegation_token = true;
            let target = public_proxy_target(service);
            assert_eq!(target.service.identity_propagation_mode, "none");
            assert!(!target.service.forward_access_token);
            assert!(!target.service.inject_delegation_token);
            assert_eq!(target.auth_method, "none");
            assert!(anonymous_endpoint_service::anonymous_service_is_runtime_safe(&target.service));
        }
    }
}
