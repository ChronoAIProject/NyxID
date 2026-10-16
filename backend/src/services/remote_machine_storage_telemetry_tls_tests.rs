//! Constructed-client transport fixtures use the production TLS/redirect/time
//! settings and call the actual collector summary method. They do not exercise
//! environment loading, kubelet TokenReview/RBAC, or a deployed cluster.
use super::*;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

const AUDIENCE: &str = "local-kubelet-stats";
const NODE: &str = "node-fixture";

#[derive(Clone)]
enum Reply {
    Summary,
    Redirect(String, u16),
    Status(u16),
    Oversized,
    Stall,
}
#[derive(Default)]
struct Observed {
    connections: AtomicUsize,
    requests: AtomicUsize,
    authenticated: AtomicUsize,
    expected_token: Mutex<Zeroizing<String>>,
}
struct TlsFixture {
    address: std::net::SocketAddr,
    root: reqwest::Certificate,
    observed: Arc<Observed>,
    task: JoinHandle<()>,
}
impl Drop for TlsFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn certificate_chain() -> (reqwest::Certificate, rustls::ServerConfig) {
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_key = KeyPair::generate().unwrap();
    let ca_certificate = ca.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(ca, ca_key);
    let mut leaf = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = KeyPair::generate().unwrap();
    let leaf_certificate = leaf.signed_by(&leaf_key, &issuer).unwrap();
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf_certificate.der().clone()],
        rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
    )
    .unwrap();
    (
        reqwest::Certificate::from_der(ca_certificate.der()).unwrap(),
        tls,
    )
}
impl TlsFixture {
    async fn start(reply: Reply) -> Self {
        let (root, tls) = certificate_chain();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let observed = Arc::new(Observed::default());
        let captured = observed.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        captured.connections.fetch_add(1, Ordering::SeqCst);
                        let acceptor = acceptor.clone();
                        let observed = captured.clone();
                        let reply = reply.clone();
                        connections.spawn(async move {
                            let Ok(Ok(mut stream)) = tokio::time::timeout(Duration::from_secs(2), acceptor.accept(stream)).await else { return; };
                            let mut headers = Zeroizing::new(Vec::new());
                            let parsed = tokio::time::timeout(Duration::from_secs(2), async {
                                while !headers.ends_with(b"\r\n\r\n") {
                                    if headers.len() >= 16 * 1024 { return false; }
                                    match stream.read_u8().await { Ok(byte) => headers.push(byte), Err(_) => return false }
                                }
                                true
                            }).await.unwrap_or(false);
                            if !parsed { return; }
                            observed.requests.fetch_add(1, Ordering::SeqCst);
                            let authenticated = {
                                let expected = observed.expected_token.lock().unwrap();
                                std::str::from_utf8(&headers).ok().is_some_and(|h|
                                    h.lines().next() == Some("GET /stats/summary HTTP/1.1")
                                    && h.lines().any(|line| line.split_once(':').is_some_and(|(name, value)|
                                        name.eq_ignore_ascii_case("authorization") && value.trim().strip_prefix("Bearer ") == Some(expected.as_str()))))
                            };
                            if authenticated { observed.authenticated.fetch_add(1, Ordering::SeqCst); }
                            let reply = if authenticated { reply } else { Reply::Status(401) };
                            let response = match reply {
                                Reply::Summary => {
                                    let body = json!({"node":{"nodeName":NODE},"pods":[]}).to_string();
                                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                                }
                                Reply::Redirect(location, status) => format!("HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                                Reply::Status(status) => format!("HTTP/1.1 {status} Rejected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                                Reply::Stall => {
                                    // The configured client timeout must win. The fixture's
                                    // JoinSet is aborted on drop, so this never delays cleanup.
                                    tokio::time::sleep(Duration::from_secs(8)).await;
                                    return;
                                }
                                Reply::Oversized => {
                                    let body = format!("{{\"padding\":\"{}\"}}", "x".repeat(MAX_SUMMARY));
                                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                                }
                            };
                            let _ = tokio::time::timeout(Duration::from_secs(2), stream.write_all(response.as_bytes())).await;
                            let _ = stream.shutdown().await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self {
            address,
            root,
            observed,
            task,
        }
    }
    fn origin(&self, host: &str) -> reqwest::Url {
        format!("https://{host}:{}/", self.address.port())
            .parse()
            .unwrap()
    }
    fn collector(
        &self,
        token_file: PathBuf,
        trusted_root: reqwest::Certificate,
        host: &str,
    ) -> KubeletCollector {
        // These are the exact production security settings. no_proxy and the
        // client-scoped DNS override confine this fixture to its loopback socket.
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(4))
            .add_root_certificate(trusted_root)
            .no_proxy()
            .resolve(host, self.address)
            .build()
            .unwrap();
        KubeletCollector {
            client,
            endpoints: BTreeMap::from([(NODE.into(), self.origin(host))]),
            token_file,
            audience: AUDIENCE.into(),
        }
    }
    fn expect_token(&self, token: &str) {
        *self.observed.expected_token.lock().unwrap() = Zeroizing::new(token.to_owned());
    }
}
fn token(audience: &str, expires: i64) -> Zeroizing<String> {
    // Disposable fixture bearer. The loopback peer authenticates its exact
    // value; this does not impersonate a real Kubernetes service account.
    let key = Zeroizing::new(uuid::Uuid::new_v4().to_string());
    Zeroizing::new(
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &json!({"aud":[audience],"exp":expires,"jti":uuid::Uuid::new_v4().to_string()}),
            &jsonwebtoken::EncodingKey::from_secret(key.as_bytes()),
        )
        .unwrap(),
    )
}
fn write_token(directory: &tempfile::TempDir, value: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.path().join("kubelet-token");
    std::fs::write(&path, value).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path
}
async fn bounded_summary(collector: &KubeletCollector) -> AppResult<Value> {
    tokio::time::timeout(Duration::from_secs(6), collector.summary(NODE))
        .await
        .expect("collector exceeded its configured transport deadline")
}

#[tokio::test]
async fn kubelet_https_trusted_ca_hostname_and_rotating_audience_token_succeed() {
    let peer = TlsFixture::start(Reply::Summary).await;
    let directory = tempfile::tempdir().unwrap();
    let initial = token(AUDIENCE, Utc::now().timestamp() + 120);
    peer.expect_token(&initial);
    let path = write_token(&directory, &initial);
    let collector = peer.collector(path, peer.root.clone(), "localhost");
    for credential in [initial, token(AUDIENCE, Utc::now().timestamp() + 120)] {
        peer.expect_token(&credential);
        write_token(&directory, &credential);
        let summary = bounded_summary(&collector).await.unwrap();
        assert_eq!(summary["node"]["nodeName"], NODE);
    }
    assert_eq!(peer.observed.requests.load(Ordering::SeqCst), 2);
    assert_eq!(peer.observed.authenticated.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn kubelet_https_wrong_ca_or_hostname_never_delivers_bearer() {
    for wrong_hostname in [false, true] {
        let peer = TlsFixture::start(Reply::Summary).await;
        let directory = tempfile::tempdir().unwrap();
        let credential = token(AUDIENCE, Utc::now().timestamp() + 120);
        peer.expect_token(&credential);
        let path = write_token(&directory, &credential);
        let (root, host) = if wrong_hostname {
            (peer.root.clone(), "wrong-host.invalid")
        } else {
            (certificate_chain().0, "localhost")
        };
        let collector = peer.collector(path, root, host);
        assert!(bounded_summary(&collector).await.is_err());
        assert!(peer.observed.connections.load(Ordering::SeqCst) > 0);
        assert_eq!(peer.observed.requests.load(Ordering::SeqCst), 0);
        assert_eq!(peer.observed.authenticated.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn kubelet_https_wrong_audience_or_expired_token_is_rejected_before_connect() {
    let peer = TlsFixture::start(Reply::Summary).await;
    let directory = tempfile::tempdir().unwrap();
    for (audience, expires) in [
        ("api-server-only", Utc::now().timestamp() + 120),
        (AUDIENCE, Utc::now().timestamp() - 1),
    ] {
        let credential = token(audience, expires);
        peer.expect_token(&credential);
        let path = write_token(&directory, &credential);
        let collector = peer.collector(path, peer.root.clone(), "localhost");
        assert!(bounded_summary(&collector).await.is_err());
    }
    assert_eq!(peer.observed.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn kubelet_https_redirect_never_reaches_second_peer_or_forwards_credential() {
    let destination = TlsFixture::start(Reply::Summary).await;
    let directory = tempfile::tempdir().unwrap();
    let credential = token(AUDIENCE, Utc::now().timestamp() + 120);
    destination.expect_token(&credential);
    let path = write_token(&directory, &credential);
    for status in [302, 307] {
        let redirect = TlsFixture::start(Reply::Redirect(
            destination
                .origin("localhost")
                .join("/stats/summary")
                .unwrap()
                .into(),
            status,
        ))
        .await;
        redirect.expect_token(&credential);
        let collector = redirect.collector(path.clone(), redirect.root.clone(), "localhost");
        assert!(bounded_summary(&collector).await.is_err());
        assert_eq!(redirect.observed.authenticated.load(Ordering::SeqCst), 1);
        assert_eq!(destination.observed.connections.load(Ordering::SeqCst), 0);
        assert_eq!(destination.observed.requests.load(Ordering::SeqCst), 0);
        assert_eq!(destination.observed.authenticated.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn kubelet_https_rejections_oversized_body_and_stall_fail_within_budget() {
    for reply in [
        Reply::Status(401),
        Reply::Status(403),
        Reply::Oversized,
        Reply::Stall,
    ] {
        let peer = TlsFixture::start(reply).await;
        let directory = tempfile::tempdir().unwrap();
        let credential = token(AUDIENCE, Utc::now().timestamp() + 120);
        peer.expect_token(&credential);
        let path = write_token(&directory, &credential);
        let collector = peer.collector(path, peer.root.clone(), "localhost");
        assert!(bounded_summary(&collector).await.is_err());
        assert_eq!(peer.observed.authenticated.load(Ordering::SeqCst), 1);
    }
}
