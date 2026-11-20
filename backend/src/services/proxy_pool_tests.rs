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
