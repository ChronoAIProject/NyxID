// Local TLS peer with HTTP/1.1 keep-alive. Count accepted sockets separately from GETs.
use super::{empty_response_cache, empty_token_cache, forward_request, make_proxy_target};
use crate::services::{proxy_destination::http, proxy_service::ProxyBody};
use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use tokio_rustls::rustls;

const HOST: &str = "pool-fixture.example";

struct Peer {
    address: SocketAddr,
    cert: reqwest::Certificate,
    connections: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    listener: tokio::task::JoinHandle<()>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

impl Peer {
    async fn start(keep_alive: bool) -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let key = rcgen::generate_simple_self_signed(vec![HOST.into()]).unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![key.cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(key.signing_key.serialize_der()).into(),
            )
            .unwrap();
        let cert = reqwest::Certificate::from_der(key.cert.der()).unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let accepts = connections.clone();
        let seen = requests.clone();
        let listener = tokio::spawn(async move {
            loop {
                let (tcp, _) = socket.accept().await.unwrap();
                tcp.set_nodelay(true).unwrap();
                accepts.fetch_add(1, Ordering::Relaxed);
                let tls = tls.clone();
                let seen = seen.clone();
                tokio::spawn(async move {
                    let stream = tls.accept(tcp).await.unwrap();
                    let serve = hyper::service::service_fn(
                        move |req: hyper::Request<hyper::body::Incoming>| {
                            assert_eq!(req.method(), "GET");
                            assert_eq!(req.uri().path(), "/pool");
                            let number = seen.fetch_add(1, Ordering::Relaxed) + 1;
                            let mut response = hyper::Response::new(http_body_util::Full::new(
                                bytes::Bytes::from(number.to_string()),
                            ));
                            if !keep_alive {
                                response
                                    .headers_mut()
                                    .insert("connection", "close".parse().unwrap());
                            }
                            std::future::ready(Ok::<_, std::convert::Infallible>(response))
                        },
                    );
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), serve)
                        .await;
                });
            }
        });
        Self {
            address,
            cert,
            connections,
            requests,
            listener,
        }
    }
}

struct FixtureDns {
    answers: Vec<Vec<SocketAddr>>,
    calls: AtomicUsize,
}

impl reqwest::dns::Resolve for FixtureDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        assert_eq!(name.as_str(), HOST);
        let index = self.calls.fetch_add(1, Ordering::Relaxed);
        let answers = self.answers[index.min(self.answers.len() - 1)].clone();
        Box::pin(async move { Ok(Box::new(answers.into_iter()) as reqwest::dns::Addrs) })
    }
}

async fn run_gets(count: usize) -> (usize, Vec<Duration>) {
    let peer = Peer::start(true).await;
    let base = format!("https://{HOST}:{}", peer.address.port());
    let target = make_proxy_target(base.clone());
    let cert = peer.cert.clone();
    let builder = Arc::new(move || {
        reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(cert.clone())
    });
    let client = super::super::no_redirect_http_client();
    let tokens = empty_token_cache();
    let responses = empty_response_cache();
    let calls = async {
        let mut durations = Vec::with_capacity(count);
        for number in 1..=count {
            let start = Instant::now();
            let response = forward_request(
                client,
                &target,
                reqwest::Method::GET,
                "pool",
                None,
                Default::default(),
                ProxyBody::Buffered(None),
                vec![],
                vec![],
                None,
                &tokens,
                &responses,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.bytes().await.unwrap().as_ref(),
                number.to_string().as_bytes()
            );
            durations.push(start.elapsed());
        }
        durations
    };
    let dns = Arc::new(FixtureDns {
        answers: vec![vec![peer.address]],
        calls: AtomicUsize::new(0),
    });
    let durations = http::with_test_pools(dns.clone(), builder, calls).await;
    assert_eq!(dns.calls.load(Ordering::Relaxed), 1);
    assert_eq!(peer.requests.load(Ordering::Relaxed), count);
    (peer.connections.load(Ordering::Relaxed), durations)
}

#[tokio::test]
async fn proxy_destination_reuses_public_http_connection() {
    assert_eq!(run_gets(2).await.0, 1);
}

#[tokio::test]
#[ignore = "local HTTPS latency measurement"]
async fn proxy_destination_https_latency() {
    let (connections, mut samples) = run_gets(200).await;
    samples.sort_unstable();
    eprintln!(
        "GETs=200 connections={connections} p50_us={:.3} p95_us={:.3}",
        samples[99].as_secs_f64() * 1e6,
        samples[189].as_secs_f64() * 1e6
    );
}

#[tokio::test]
async fn proxy_destination_checks_the_complete_answer_on_reconnect() {
    let peer = Peer::start(false).await;
    let target = make_proxy_target(format!("https://{HOST}:{}", peer.address.port()));
    let dns = Arc::new(FixtureDns {
        answers: vec![
            vec![peer.address],
            vec![
                peer.address,
                SocketAddr::from(([0, 0, 0, 0], peer.address.port())),
            ],
        ],
        calls: AtomicUsize::new(0),
    });
    let cert = peer.cert.clone();
    let builder = Arc::new(move || {
        reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(cert.clone())
    });
    let requests = async {
        let client = super::super::no_redirect_http_client();
        let tokens = empty_token_cache();
        let responses = empty_response_cache();
        let send = || {
            forward_request(
                client,
                &target,
                reqwest::Method::GET,
                "pool",
                None,
                Default::default(),
                ProxyBody::Buffered(None),
                vec![],
                vec![],
                None,
                &tokens,
                &responses,
            )
        };
        let first = send().await.unwrap();
        assert_eq!(first.headers().get("connection").unwrap(), "close");
        assert_eq!(first.text().await.unwrap(), "1");
        let error = send().await.unwrap_err();
        assert!(
            matches!(error, crate::errors::AppError::ValidationError(ref message)
            if message == "Direct proxy destination is not permitted")
        );
    };
    http::with_test_pools(dns.clone(), builder, requests).await;
    assert_eq!(dns.calls.load(Ordering::Relaxed), 2);
    assert_eq!(peer.connections.load(Ordering::Relaxed), 1);
    assert_eq!(peer.requests.load(Ordering::Relaxed), 1);
}

#[derive(Clone, Copy)]
enum LookupFailure {
    Error,
    Timeout,
    Empty,
}

impl reqwest::dns::Resolve for LookupFailure {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        match self {
            Self::Error => Box::pin(async { Err(std::io::Error::other("lookup failed").into()) }),
            Self::Timeout => Box::pin(std::future::pending()),
            Self::Empty => {
                Box::pin(async { Ok(Box::new(std::iter::empty()) as reqwest::dns::Addrs) })
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn proxy_destination_dns_failure_is_unavailable_before_send() {
    let peer = Peer::start(false).await;
    let target = make_proxy_target(format!("https://{HOST}:{}", peer.address.port()));
    for failure in [
        LookupFailure::Error,
        LookupFailure::Timeout,
        LookupFailure::Empty,
    ] {
        let result = http::with_test_pools(
            Arc::new(failure),
            Arc::new(|| reqwest::Client::builder().no_proxy()),
            classified_get(&target, "pool"),
        )
        .await
        .unwrap_err();
        assert_eq!(peer.connections.load(Ordering::Relaxed), 0);
        assert_eq!(peer.requests.load(Ordering::Relaxed), 0);
        let super::super::ForwardRequestError::PreDispatchRejected(error) = result else {
            panic!("lookup failed before dispatch: {result:?}");
        };
        assert_eq!(error.status_code(), 502);
        assert!(matches!(
            error,
            crate::errors::AppError::PoolAttemptTransport(
                crate::errors::PoolAttemptTransportKind::ResolutionUnavailable
            )
        ));
    }
}

async fn classified_get(
    target: &super::super::ProxyTarget,
    path: &str,
) -> Result<reqwest::Response, super::super::ForwardRequestError> {
    super::super::forward_request_with_extra_outbound_headers(
        super::super::Redirects::SameOrigin,
        target,
        reqwest::Method::GET,
        path,
        Some("token=private-query"),
        Default::default(),
        ProxyBody::Buffered(None),
        vec![],
        vec![],
        None,
        &empty_token_cache(),
        &empty_response_cache(),
        vec![],
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    )
    .await
}

#[tokio::test]
async fn proxy_destination_redirect_failure_retains_first_dispatch() {
    use super::super::ForwardRequestError;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    let server = MockServer::start().await;
    Mock::given(path("/pool"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("connection", "close")
                .insert_header("location", "/next?query=opaque"),
        )
        .mount(&server)
        .await;
    let target = make_proxy_target(format!("http://{HOST}:{}", server.address().port()));
    for answers in [vec!["10.0.0.1:0".parse().unwrap()], vec![]] {
        let unavailable = answers.is_empty();
        let dns = Arc::new(FixtureDns {
            answers: vec![vec![*server.address()], answers],
            calls: AtomicUsize::new(0),
        });
        let error = http::with_test_pools(
            dns.clone(),
            Arc::new(|| reqwest::Client::builder().no_proxy()),
            classified_get(&target, "pool"),
        )
        .await
        .unwrap_err();
        let ForwardRequestError::Application(error) = error else {
            panic!("original request reached the server: {error:?}")
        };
        assert_eq!(dns.calls.load(Ordering::Relaxed), 2);
        if unavailable {
            assert_eq!(error.status_code(), 502);
            assert!(matches!(
                error,
                crate::errors::AppError::PoolAttemptTransport(
                    crate::errors::PoolAttemptTransportKind::ResolutionUnavailable
                )
            ));
        } else {
            assert_eq!(error.status_code(), 400);
            assert_eq!(error.error_code(), 1008);
        }
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.url.path() == "/pool"));
}

#[tokio::test(start_paused = true)]
async fn proxy_destination_websocket_resolution_failure_never_dials() {
    let peer = Peer::start(false).await;
    let url = format!("wss://{HOST}:{}/socket?token=secret", peer.address.port());
    let target = make_proxy_target(format!("https://{HOST}:{}", peer.address.port()));
    for failure in [
        LookupFailure::Timeout,
        LookupFailure::Empty,
        LookupFailure::Error,
    ] {
        let result = http::with_test_pools(
            Arc::new(failure),
            Arc::new(reqwest::Client::builder),
            target.direct_destination(&url, "socket"),
        )
        .await;
        assert!(matches!(
            result,
            Err(crate::errors::AppError::PoolAttemptTransport(
                crate::errors::PoolAttemptTransportKind::ResolutionUnavailable
            ))
        ));
    }
    assert_eq!(peer.connections.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn proxy_destination_logs_reason_method_and_safe_path() {
    use crate::services::proxy_destination::INTERNAL_CALLER;
    use tracing::instrument::WithSubscriber;
    let mut target = make_proxy_target(format!("https://{HOST}"));
    target.service.slug = "ornn-api".into();
    let path = "api/v1/skills/private-skill/versions/private-version/download";
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(log.reopen().unwrap())
        .finish();
    let cases = [
        (Some("agent_skill_ornn"), None),
        (None, None),
        (Some("agent_skill_ornn"), Some("10.0.0.1:0")),
        (None, Some("0.0.0.0:0")),
        (None, Some("169.254.169.254:0")),
    ];
    async {
        for (caller, address) in cases {
            let answers = address.map(|ip| ip.parse().unwrap()).into_iter().collect();
            let request = http::with_test_pools(
                Arc::new(FixtureDns {
                    answers: vec![answers],
                    calls: AtomicUsize::new(0),
                }),
                Arc::new(|| reqwest::Client::builder().no_proxy()),
                classified_get(&target, path),
            );
            let result = match caller {
                Some(label) => INTERNAL_CALLER.scope(label, request).await,
                None => request.await,
            }
            .unwrap_err();
            let super::super::ForwardRequestError::PreDispatchRejected(error) = result else {
                panic!("unexpected dispatch evidence: {result:?}");
            };
            if address.is_some() {
                assert_eq!(error.status_code(), 400);
                assert_eq!(error.error_code(), 1008);
            } else {
                assert_eq!(error.error_code(), 1006);
            }
        }
    }
    .with_subscriber(subscriber)
    .await;
    let text = std::fs::read_to_string(log.path()).unwrap();
    for secret in [
        HOST,
        "private-skill",
        "private-version",
        "private-query",
        "token=",
    ] {
        assert!(!text.contains(secret));
    }
    let events: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|e: &serde_json::Value| e["fields"]["service_id"] == target.service.id)
        .collect();
    assert_eq!(events.len(), cases.len());
    for ((caller, address), event) in cases.into_iter().zip(events) {
        let fields = &event["fields"];
        assert_eq!(event["level"], "WARN");
        assert_eq!(fields["method"], "GET");
        assert_eq!(fields["path"], "/{path}");
        match caller {
            Some(label) => assert_eq!(fields["caller"], label),
            None => assert!(fields.get("caller").is_none()),
        }
        assert_eq!(fields["redirect"], false);
        let (reason, message) = if address.is_none() {
            (
                "resolution_unavailable",
                "Direct proxy destination unavailable",
            )
        } else {
            ("address_not_permitted", "Direct proxy destination refused")
        };
        assert_eq!(fields["reason"], reason);
        assert_eq!(fields["message"], message);
    }
}
