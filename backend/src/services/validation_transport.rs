//! Validation egress is separate from ordinary forwarding: public provider origins,
//! DNS pinned per attempt, no redirects, and one deadline through the decoded body.
//! Node egress uses the owner's network boundary, so server DNS pinning does not
//! apply there. Probes carry uncredentialed paths and the node injects once.
//! Reuse additionally requires the routed slot revision advertised by the agent.
//! Nodes must advertise no-redirect support; the server still bounds
//! the entire exchange and the received body, including streaming responses.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::TryStreamExt;
use tokio::io::{AsyncRead, AsyncReadExt};

use reqwest::{Client, Url};

use super::proxy_service::{self, ProxyTarget};
use super::validator_profiles::{ProbeResponse, ProbeTarget, ValidatorProfile};

pub const PROBE_DEADLINE: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    Configuration,
    Unavailable,
    BodyTooLarge,
    NodeUpgradeRequired,
}

pub fn provider_base(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "api-github" | "api-github-pat" => "https://api.github.com",
        "llm-openai" => "https://api.openai.com/v1",
        "llm-anthropic" => "https://api.anthropic.com/v1",
        "llm-google-ai" => "https://generativelanguage.googleapis.com/v1beta",
        "llm-mistral" => "https://api.mistral.ai/v1",
        "llm-cohere" => "https://api.cohere.com/v2",
        "llm-deepseek" => "https://api.deepseek.com/v1",
        "llm-openrouter" => "https://openrouter.ai/api/v1",
        "api-slack" | "api-slack-bot" => "https://slack.com/api",
        "api-lark" => "https://open.larksuite.com/open-apis",
        "api-feishu" => "https://open.feishu.cn/open-apis",
        "api-telegram-bot" => "https://api.telegram.org",
        "api-twitch" => "https://api.twitch.tv/helix",
        _ => return None,
    })
}

/// Validate before decrypting or refreshing any credential.
pub fn profile_url(
    profile: &ValidatorProfile,
    slug: &str,
    base_url: &str,
) -> Result<Url, TransportError> {
    if profile.billable || !profile.catalog_slugs.contains(&slug) {
        return Err(TransportError::Configuration);
    }
    let base = Url::parse(base_url).map_err(|_| TransportError::Configuration)?;
    let seeded = Url::parse(provider_base(slug).ok_or(TransportError::Configuration)?)
        .map_err(|_| TransportError::Configuration)?;
    if base.origin() != seeded.origin()
        || base.path().trim_end_matches('/') != seeded.path().trim_end_matches('/')
        || base.query().is_some()
        || base.fragment().is_some()
        || !base.username().is_empty()
        || base.password().is_some()
    {
        return Err(TransportError::Configuration);
    }
    let url = match profile.target_for(slug) {
        ProbeTarget::Relative(path) => {
            Url::parse(&format!("{}/{}", base_url.trim_end_matches('/'), path))
        }
        ProbeTarget::AbsoluteAllowlisted(url) => Url::parse(url),
    }
    .map_err(|_| TransportError::Configuration)?;
    if url.origin() != base.origin() {
        return Err(TransportError::Configuration);
    }
    Ok(url)
}

fn validate_addresses(addresses: &[SocketAddr]) -> Result<(), TransportError> {
    if addresses.is_empty()
        || addresses.iter().any(|addr| {
            super::url_validation::is_private_or_internal_ip(addr.ip())
                || addr.ip() == IpAddr::from([168, 63, 129, 16])
                || addr.ip().is_multicast()
        })
    {
        return Err(TransportError::Configuration);
    }
    Ok(())
}

fn pinned_client(host: &str, addresses: &[SocketAddr]) -> Result<Client, TransportError> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_DEADLINE)
        .resolve_to_addrs(host, addresses)
        .no_gzip()
        .build()
        .map_err(|_| TransportError::Unavailable)
}

pub async fn send(
    profile: &ValidatorProfile,
    slug: &str,
    target: &ProxyTarget,
    dispatched: &AtomicBool,
) -> Result<ProbeResponse, TransportError> {
    tokio::time::timeout(PROBE_DEADLINE, async {
        let url = profile_url(profile, slug, &target.base_url)?;
        let host = url
            .host_str()
            .ok_or(TransportError::Configuration)?
            .to_string();
        let port = url
            .port_or_known_default()
            .ok_or(TransportError::Configuration)?;
        let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|_| TransportError::Unavailable)?
            .collect();
        validate_addresses(&addresses)?;
        let client = pinned_client(&host, &addresses)?;
        let request = prepare_request(&client, profile, slug, target, url)?;
        dispatched.store(true, Ordering::Relaxed);
        let response = request
            .send()
            .await
            .map_err(|_| TransportError::Unavailable)?;
        read_response(response, profile.max_body_bytes).await
    })
    .await
    .map_err(|_| TransportError::Unavailable)?
}

fn prepare_request(
    client: &Client,
    profile: &ValidatorProfile,
    slug: &str,
    target: &ProxyTarget,
    mut url: Url,
) -> Result<reqwest::RequestBuilder, TransportError> {
    let mut credentials = Vec::new();
    proxy_service::extend_with_path_credential(&mut credentials, target);
    let (prefix, profile_path) = match profile.target_for(slug) {
        ProbeTarget::Relative(path) => (
            Url::parse(&target.base_url)
                .map_err(|_| TransportError::Configuration)?
                .path()
                .trim_end_matches('/')
                .to_string(),
            path,
        ),
        ProbeTarget::AbsoluteAllowlisted(_) => (String::new(), url.path()),
    };
    let prepared = proxy_service::prepare_delegated_request(profile_path, None, &credentials)
        .map_err(|_| TransportError::Configuration)?;
    if target.auth_method == "path" {
        url.set_path(&format!("{prefix}/{}", prepared.path));
    }
    let headers = proxy_service::build_effective_outbound_headers(
        target,
        vec![("accept".into(), "application/json".into())],
        &[],
        &prepared.delegated_headers,
        &[],
    );
    let mut request = client.request(profile.method.clone(), url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request = proxy_service::inject_simple_auth(request, target)
        .map_err(|_| TransportError::Configuration)?;
    if let Some(body) = profile.body {
        request = request.body(body);
    }
    Ok(request.header("accept-encoding", "gzip, identity"))
}

async fn read_response(
    response: reqwest::Response,
    limit: usize,
) -> Result<ProbeResponse, TransportError> {
    let encoding = response.headers().get("content-encoding");
    let gzip = encoding.is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"gzip"));
    if encoding.is_some_and(|value| value != "identity") && !gzip {
        return Err(TransportError::Unavailable);
    }
    if !gzip
        && response
            .content_length()
            .is_some_and(|size| size > limit as u64)
    {
        return Err(TransportError::BodyTooLarge);
    }
    let mut result = ProbeResponse {
        status: response.status().as_u16(),
        headers: response.headers().clone(),
        body: Vec::new(),
    };
    // Decode only here: enabling reqwest's gzip feature would alter every shared
    // proxy client's content negotiation and encoded-body pass-through.
    let wire =
        tokio_util::io::StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
    let reader: std::pin::Pin<Box<dyn AsyncRead + Send>> = if gzip {
        let mut decoder = async_compression::tokio::bufread::GzipDecoder::new(wire);
        decoder.multiple_members(true);
        Box::pin(decoder)
    } else {
        Box::pin(wire)
    };
    // Read at most one decoded byte beyond the cap, including gzip bombs. The
    // caller's total deadline and the client's body timeout both remain active.
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut result.body)
        .await
        .map_err(|_| TransportError::Unavailable)?;
    if result.body.len() > limit {
        return Err(TransportError::BodyTooLarge);
    }
    Ok(result)
}

pub async fn send_via_node(
    state: &crate::AppState,
    node_id: &str,
    profile: &ValidatorProfile,
    slug: &str,
    target: &ProxyTarget,
    dispatched: &AtomicBool,
) -> Result<ProbeResponse, TransportError> {
    use super::node_ws_manager::{NodeProxyRequest, ProxyResponseType, StreamChunk};
    // The service owner's routing authority selects this node. Caller node
    // grants were checked by the execution snapshot; node-management ACLs do
    // not apply to an authorized service execution.
    let node = state
        .db
        .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .find_one(mongodb::bson::doc! { "_id": node_id })
        .await
        .map_err(|_| TransportError::Unavailable)?
        .ok_or(TransportError::Unavailable)?;
    if !super::node_routing_service::is_node_id_dispatchable(
        &state.db,
        node_id,
        &state.node_ws_manager,
    )
    .await
    .map_err(|_| TransportError::Unavailable)?
    {
        return Err(TransportError::Unavailable);
    }
    let local = state.node_ws_manager.session_info(node_id);
    let capable = if local.is_connected {
        local.capabilities.no_redirect_proxy
    } else {
        node.connection_owner
            .as_ref()
            .is_some_and(|owner| owner.no_redirect_proxy && owner.expires_at > chrono::Utc::now())
    };
    if !capable {
        return Err(TransportError::NodeUpgradeRequired);
    }
    let url = profile_url(profile, slug, &target.base_url)?;
    let (base_url, path) = match profile.target_for(slug) {
        ProbeTarget::Relative(path) => (target.base_url.clone(), path.to_string()),
        ProbeTarget::AbsoluteAllowlisted(_) => {
            (url.origin().ascii_serialization(), url.path().to_string())
        }
    };
    // The node injects its local credential exactly as on ordinary node requests.
    let headers = proxy_service::build_effective_outbound_headers(
        target,
        vec![
            ("accept".into(), "application/json".into()),
            ("accept-encoding".into(), "identity".into()),
        ],
        &[],
        &[],
        &[],
    );
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = NodeProxyRequest {
        follow_redirects: false,
        request_id: request_id.clone(),
        service_id: target.service.id.clone(),
        service_slug: target.service.slug.clone(),
        base_url,
        method: profile.method.to_string(),
        path,
        query: None,
        headers,
        body: profile.body.map(|body| body.as_bytes().to_vec()),
    };
    let secret = if state.config.node_hmac_signing_enabled {
        Some(zeroize::Zeroizing::new(
            super::node_service::get_node_signing_secret(
                &state.db,
                state.encryption_keys.as_ref(),
                node_id,
            )
            .await
            .map_err(|_| TransportError::Unavailable)?,
        ))
    } else {
        None
    };
    let permit =
        super::billing::route_inventory::enforce_billing_exempt_egress_classification(Some(
            super::billing::route_inventory::BillingRoutePolicy::Exempt("service_validation"),
        ))
        .map_err(|_| TransportError::Configuration)?;
    let result = tokio::time::timeout(PROBE_DEADLINE, async {
        dispatched.store(true, Ordering::Relaxed);
        let response = state
            .node_ws_manager
            .send_proxy_request_classified(
                node_id,
                request,
                secret.as_deref().map(|secret| secret.as_slice()),
                permit,
            )
            .await
            .map_err(|error| {
                dispatched.fetch_or(error.dispatched, Ordering::Relaxed);
                TransportError::Unavailable
            })?;
        match response {
            ProxyResponseType::Complete(response) => node_response(
                response.status,
                response.headers,
                response.body,
                profile.max_body_bytes,
            ),
            ProxyResponseType::Streaming(mut stream) => {
                let mut response = None;
                while let Some(chunk) = stream.recv().await {
                    match chunk {
                        StreamChunk::Start { status, headers } => {
                            if response.is_some() {
                                return Err(TransportError::Unavailable);
                            }
                            response = Some(node_response(
                                status,
                                headers,
                                Vec::new(),
                                profile.max_body_bytes,
                            )?);
                        }
                        StreamChunk::Data(data) => {
                            let current = response.as_mut().ok_or(TransportError::Unavailable)?;
                            if data.len()
                                > profile.max_body_bytes.saturating_sub(current.body.len())
                            {
                                return Err(TransportError::BodyTooLarge);
                            }
                            current.body.extend_from_slice(&data);
                        }
                        StreamChunk::End => return response.ok_or(TransportError::Unavailable),
                        StreamChunk::Error(_) => return Err(TransportError::Unavailable),
                    }
                }
                Err(TransportError::Unavailable)
            }
        }
    })
    .await
    .unwrap_or(Err(TransportError::Unavailable));
    state
        .node_ws_manager
        .cancel_proxy_request(node_id, &request_id);
    result
}

fn node_response(
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    limit: usize,
) -> Result<ProbeResponse, TransportError> {
    if body.len() > limit {
        return Err(TransportError::BodyTooLarge);
    }
    let mut parsed = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| TransportError::Unavailable)?;
        let value = reqwest::header::HeaderValue::from_str(&value)
            .map_err(|_| TransportError::Unavailable)?;
        parsed.insert(name, value);
    }
    if parsed
        .get("content-encoding")
        .is_some_and(|v| v != "identity")
    {
        return Err(TransportError::Unavailable);
    }
    Ok(ProbeResponse {
        status,
        headers: parsed,
        body,
    })
}

#[cfg(test)]
mod node_signature_tests {
    use crate::services::node_ws_manager::{NodeProxyRequest, sign_proxy_request};
    use nyxid_node_proxy_test::{NodeMetrics, ReplayGuard};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    #[tokio::test]
    async fn validation_server_signature_and_agent_redirect_flag_agree() {
        let server = MockServer::start().await;
        Mock::given(path("/user"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "http://169.254.169.254/latest/meta-data"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let credentials =
            nyxid_node_proxy_test::no_auth_credentials("fixture", &server.uri()).unwrap();
        let request = NodeProxyRequest {
            follow_redirects: false,
            request_id: uuid::Uuid::new_v4().to_string(),
            service_id: "fixture".into(),
            service_slug: "fixture".into(),
            base_url: server.uri(),
            method: "GET".into(),
            path: "user".into(),
            query: None,
            headers: vec![],
            body: None,
        };
        let secret = [0x42; 32];
        let signature = sign_proxy_request(&secret, &request);
        let mut frame = serde_json::to_value(request).unwrap();
        frame["timestamp"] = signature.timestamp.into();
        frame["nonce"] = signature.nonce.into();
        frame["signature"] = signature.signature.into();
        let client = nyxid_node_proxy_test::proxy_executor::build_http_clients().unwrap();
        for tamper in [false, true] {
            if tamper {
                frame["follow_redirects"] = true.into();
            }
            let (tx, mut rx) = tokio::sync::mpsc::channel(16);
            nyxid_node_proxy_test::proxy_executor::execute_proxy_request(
                &frame,
                &credentials,
                Some(&hex::encode(secret)),
                &tokio::sync::Mutex::new(ReplayGuard::new()),
                &NodeMetrics::new(),
                &tx,
                false,
                &client,
            )
            .await;
            let nyxid_node_proxy_test::ws_client::NodeWsMessage::Text(response) =
                rx.recv().await.unwrap()
            else {
                panic!("expected start")
            };
            let response: serde_json::Value = serde_json::from_str(&response).unwrap();
            assert_eq!(response["status"], if tamper { 403 } else { 302 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::validator_profiles::for_slug;
    use super::*;
    use axum::{Router, routing::get};

    #[test]
    fn validation_rejects_rebinding_private_and_metadata_addresses() {
        for address in [
            "127.0.0.1:443",
            "169.254.169.254:443",
            "10.0.0.1:443",
            "100.100.100.200:443",
            "168.63.129.16:443",
            "[::1]:443",
            "[::ffff:127.0.0.1]:443",
            "[fe80::1]:443",
            "[fd00:ec2::254]:443",
        ] {
            assert_eq!(
                validate_addresses(&[address.parse().unwrap()]),
                Err(TransportError::Configuration)
            );
        }
        let public = "140.82.112.5:443".parse().unwrap();
        assert!(validate_addresses(&[public]).is_ok());
        assert!(validate_addresses(&[public, "127.0.0.1:443".parse().unwrap()]).is_err());
    }

    #[test]
    fn validation_targets_reject_userinfo_and_origin_or_path_overrides() {
        let profile = for_slug("api-github").unwrap();
        for base in [
            "https://secret@api.github.com",
            "https://api.github.com@evil.test",
            "http://api.github.com",
            "https://api.github.com:444",
            "https://api.github.com/public",
            "https://api.github.com?url=http://127.0.0.1",
            "https://127.0.0.1",
        ] {
            assert!(profile_url(profile, "api-github", base).is_err());
        }
        assert_eq!(
            profile_url(profile, "api-github", "https://api.github.com/")
                .unwrap()
                .as_str(),
            "https://api.github.com/user"
        );
        assert_eq!(
            profile_url(
                for_slug("llm-cohere").unwrap(),
                "llm-cohere",
                "https://api.cohere.com/v2"
            )
            .unwrap()
            .as_str(),
            "https://api.cohere.com/v1/models"
        );
    }

    async fn server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, task)
    }

    #[tokio::test]
    async fn validation_transport_does_not_follow_redirect_to_private() {
        let (url, task) = server(Router::new().route(
            "/",
            get(|| async {
                axum::response::Redirect::temporary("http://169.254.169.254/latest/meta-data/")
            }),
        ))
        .await;
        // The test server bypasses only public-address admission; the production
        // client builder (DNS override, no proxy, no redirects) is exercised.
        let parsed = Url::parse(&url).unwrap();
        let client = pinned_client(
            "validation.invalid",
            &[parsed.socket_addrs(|| None).unwrap()[0]],
        )
        .unwrap();
        let response = client
            .get(url.replace("127.0.0.1", "validation.invalid"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 307);
        task.abort();
    }

    #[test]
    fn validation_auth_injection_matches_proxy_for_all_simple_methods() {
        use mongodb::bson::{self, doc};
        let now = bson::DateTime::now();
        let service = bson::from_document(doc! {
            "_id": "fixture", "name": "fixture", "slug": "api-github", "base_url": "https://api.github.com",
            "auth_method": "bearer", "auth_key_name": "Authorization", "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] },
            "is_active": true, "created_by": "fixture", "created_at": now, "updated_at": now,
        }).unwrap();
        let mut target = ProxyTarget {
            service,
            base_url: "https://api.github.com".into(),
            auth_method: "bearer".into(),
            auth_key_name: "Authorization".into(),
            credential: "fixture-token".into(),
            catalog_default_headers: vec![],
            user_service_default_headers: vec![],
            ws_frame_injections: vec![],
            connection_id: None,
        };
        let client = Client::new();
        for (method, name, credential, expected) in [
            (
                "bearer",
                "Authorization",
                "fixture-token",
                "Bearer fixture-token",
            ),
            ("header", "x-api-key", "fixture-token", "fixture-token"),
            (
                "basic",
                "Authorization",
                "user:password:with:colons",
                "Basic dXNlcjpwYXNzd29yZDp3aXRoOmNvbG9ucw==",
            ),
        ] {
            target.auth_method = method.into();
            target.auth_key_name = name.into();
            target.credential = credential.into();
            let profile = for_slug("api-github").unwrap();
            let url = profile_url(profile, "api-github", &target.base_url).unwrap();
            let probe = prepare_request(&client, profile, "api-github", &target, url.clone())
                .unwrap()
                .build()
                .unwrap();
            let proxy = proxy_service::inject_simple_auth(client.get(url), &target)
                .unwrap()
                .build()
                .unwrap();
            assert_eq!(probe.headers()["accept-encoding"], "gzip, identity");
            assert_eq!(probe.headers()[name], expected);
            assert_eq!(probe.headers()[name], proxy.headers()[name]);
        }
        target.auth_method = "query".into();
        target.auth_key_name = "key".into();
        target.credential = "a&b=c".into();
        let profile = for_slug("api-github").unwrap();
        let url = profile_url(profile, "api-github", &target.base_url).unwrap();
        let request = prepare_request(&client, profile, "api-github", &target, url)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            request.url().query_pairs().collect::<Vec<_>>(),
            vec![("key".into(), "a&b=c".into())]
        );
        target.auth_method = "path".into();
        target.auth_key_name = "bot".into();
        target.credential = "123:token".into();
        target.base_url = "https://api.telegram.org".into();
        let profile = for_slug("api-telegram-bot").unwrap();
        let url = profile_url(profile, "api-telegram-bot", &target.base_url).unwrap();
        let request = prepare_request(&client, profile, "api-telegram-bot", &target, url)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://api.telegram.org/bot123:token/getMe"
        );
    }

    #[tokio::test]
    async fn validation_transport_caps_decoded_gzip_and_slow_streams() {
        use axum::{body::Body, http::Response};
        // Recorded gzip encoding of 4096 'x' bytes (the wire body is only 38 bytes).
        let compressed = hex::decode(
            "1f8b08000000000002ffedc1010d000000c2a0da8f6f0f0714000000f06ec177103e00100000",
        )
        .unwrap();
        let (url, task) = server(Router::new()
            .route("/gzip", get(move || { let body = compressed.clone(); async move { Response::builder().header("content-encoding", "gzip").body(Body::from(body)).unwrap() } }))
            .route("/stream", get(|| async { Body::from_stream(async_stream::stream! { yield Ok::<_, std::io::Error>(vec![b'x'; 512]); yield Ok(vec![b'x'; 513]); }) }))
            .route("/slowbody", get(|| async { Body::from_stream(async_stream::stream! { yield Ok::<_, std::io::Error>(vec![b'x']); tokio::time::sleep(Duration::from_secs(5)).await; yield Ok(vec![b'x']); }) }))).await;
        let parsed = Url::parse(&url).unwrap();
        let client = pinned_client(
            "validation.invalid",
            &[parsed.socket_addrs(|| None).unwrap()[0]],
        )
        .unwrap();
        for path in ["gzip", "stream"] {
            let response = client.get(format!("{url}/{path}")).send().await.unwrap();
            assert_eq!(
                read_response(response, 1024).await.unwrap_err(),
                TransportError::BodyTooLarge
            );
        }
        let response = client.get(format!("{url}/slowbody")).send().await.unwrap();
        assert_eq!(
            read_response(response, 1024).await.unwrap_err(),
            TransportError::Unavailable
        );
        task.abort();
    }

    #[tokio::test]
    async fn validation_gzip_bomb_is_capped_during_decoding() {
        use axum::{body::Body, http::Response};
        let compressed = hex::decode(
            "1f8b08000000000002ffedc1010d000000c2a0da8f6f0f0714000000f06ec177103e00100000",
        )
        .unwrap();
        assert!(compressed.len() < 64);
        let (url, task) = server(Router::new().route(
            "/",
            get(move || {
                let compressed = compressed.clone();
                async move {
                    Response::builder()
                        .header("content-encoding", "gzip")
                        .body(Body::from(compressed))
                        .unwrap()
                }
            }),
        ))
        .await;
        let parsed = Url::parse(&url).unwrap();
        let client =
            pinned_client("validation.invalid", &parsed.socket_addrs(|| None).unwrap()).unwrap();
        let response = client
            .get(&url)
            .header("accept-encoding", "gzip, identity")
            .send()
            .await
            .unwrap();
        assert_eq!(
            read_response(response, 1024).await.unwrap_err(),
            TransportError::BodyTooLarge
        );
        // A sufficient decoded cap parses the same compressed stream successfully.
        let response = client.get(&url).send().await.unwrap();
        assert_eq!(
            read_response(response, 4096).await.unwrap().body,
            vec![b'x'; 4096]
        );
        task.abort();
    }

    #[tokio::test]
    async fn validation_does_not_enable_compression_on_shared_proxy_clients() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        Mock::given(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let clients = nyxid_node_proxy_test::proxy_executor::build_http_clients().unwrap();
        for client in [Client::new(), clients.default, clients.no_redirect] {
            client.get(server.uri()).send().await.unwrap();
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        for request in requests {
            assert!(!request.headers.contains_key("accept-encoding"));
        }
    }

    #[tokio::test]
    async fn validation_transport_deadline_includes_body_and_rejects_oversize() {
        let (url, task) = server(
            Router::new()
                .route("/large", get(|| async { "x".repeat(1025) }))
                .route(
                    "/slow",
                    get(|| async {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        "{}"
                    }),
                ),
        )
        .await;
        let parsed = Url::parse(&url).unwrap();
        let client = pinned_client(
            "validation.invalid",
            &[parsed.socket_addrs(|| None).unwrap()[0]],
        )
        .unwrap();
        let response = client.get(format!("{url}/large")).send().await.unwrap();
        assert!(matches!(
            read_response(response, 1024).await,
            Err(TransportError::BodyTooLarge)
        ));
        let started = tokio::time::Instant::now();
        assert!(
            client
                .get(format!("{url}/slow"))
                .send()
                .await
                .unwrap_err()
                .is_timeout()
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        task.abort();
    }
}
