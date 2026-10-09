//! Real CLI transport boundaries for the credential-bearing hidden release.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[derive(Clone, Copy)]
enum Flow {
    Success,
    RetrySuccess,
    DeleteRedirect,
    RetryDeleteRedirect,
    RefreshRedirect,
}
struct TlsApi {
    url: String,
    ca: tempfile::NamedTempFile,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for TlsApi {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl TlsApi {
    async fn start(flow: Flow, redirect: String) -> Self {
        use std::io::Write;
        let mut params = rcgen::CertificateParams::new(vec!["Release Test CA".into()]).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let ca = params.self_signed(&ca_key).unwrap();
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .signed_by(&leaf_key, &rcgen::Issuer::from_params(&params, &ca_key))
            .unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let deletes = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let captured = captured.clone();
                let deletes = deletes.clone();
                let redirect = redirect.clone();
                tokio::spawn(async move {
                    let Ok(mut socket) = acceptor.accept(socket).await else {
                        return;
                    };
                    let mut bytes = Vec::new();
                    let mut buf = [0; 2048];
                    loop {
                        let n = socket.read(&mut buf).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&buf[..n]);
                        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&bytes[..end]);
                            let length = head
                                .lines()
                                .find_map(|line| {
                                    line.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .and_then(|s| s.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if bytes.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                    let request = String::from_utf8(bytes).unwrap();
                    let first = request.lines().next().unwrap();
                    let mut status = "200 OK";
                    let mut location = String::new();
                    let mut body = r#"{"groups":[],"version":7,"updated_at":null}"#;
                    if first.starts_with("GET /api/v1/keys ") {
                        body = r#"{"keys":[]}"#;
                    } else if first.starts_with("DELETE /api/v1/service-preferences/hidden ") {
                        let attempt = deletes.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0
                            && matches!(
                                flow,
                                Flow::RetrySuccess
                                    | Flow::RetryDeleteRedirect
                                    | Flow::RefreshRedirect
                            )
                        {
                            status = "401 Unauthorized";
                            body = r#"{"error":"expired","error_code":1000,"message":"Expired fixture session"}"#;
                        } else if matches!(flow, Flow::DeleteRedirect | Flow::RetryDeleteRedirect) {
                            status = "307 Temporary Redirect";
                            location = format!("Location: {redirect}\r\n");
                        }
                    } else if first.starts_with("POST /api/v1/auth/refresh ") {
                        if matches!(flow, Flow::RefreshRedirect) {
                            status = "307 Temporary Redirect";
                            location = format!("Location: {redirect}\r\n");
                        } else {
                            body = r#"{"access_token":"fresh-fixture-access","refresh_token":"fresh-fixture-refresh"}"#;
                        }
                    }
                    captured.lock().unwrap().push(request);
                    let response = format!(
                        "HTTP/1.1 {status}\r\n{location}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        let mut ca_file = tempfile::NamedTempFile::new().unwrap();
        ca_file.write_all(ca.pem().as_bytes()).unwrap();
        Self {
            url,
            ca: ca_file,
            requests,
            task,
        }
    }
}
fn command(home: &std::path::Path, url: &str, saved: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
    command.args([
        "service",
        "preference",
        "release-hidden",
        "--yes",
        "--output",
        "json",
        "--base-url",
        url,
    ]);
    if saved {
        let dir = home.join(".nyxid/profiles/transport");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, value) in [
            ("base_url", url),
            ("access_token", "old-fixture-access"),
            ("refresh_token", "old-fixture-refresh"),
        ] {
            std::fs::write(dir.join(name), value).unwrap();
        }
        command.args(["--profile", "transport"]);
    } else {
        command.args(["--access-token", "explicit-fixture-access"]);
    }
    command
        .env("HOME", home)
        .env("CI", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_ACCESS_TOKEN")
        .env_remove("NYXID_API_KEY")
        .env_remove("NYXID_CA_CERT")
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    command
}
#[tokio::test]
async fn hidden_release_rejects_remote_cleartext_before_any_request() {
    for url in [
        "http://service.example",
        "http://192.168.1.1",
        "http://127.0.0.2",
        "http://localhost.service.example",
        "http://[::ffff:127.0.0.1]",
        "https://user:dummy@service.example",
        "https://service.example/#fragment",
    ] {
        let home = tempfile::tempdir().unwrap();
        let output = command(home.path(), url, false).output().await.unwrap();
        assert!(!output.status.success(), "{url}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("Credential DELETE"), "{error}");
        assert!(!error.contains("explicit-fixture-access"), "{error}");
    }
}
#[tokio::test]
async fn hidden_release_loopback_bypasses_proxy_and_refuses_redirects() {
    let origin = MockServer::start().await;
    let target = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/service-preferences"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"groups":[],"version":7,"updated_at":null})),
        )
        .expect(1)
        .mount(&origin)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"keys":[]})))
        .expect(1)
        .mount(&origin)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/service-preferences/hidden"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", target.uri()))
        .expect(1)
        .mount(&origin)
        .await;
    let home = tempfile::tempdir().unwrap();
    let url = format!("http://localhost:{}", origin.address().port());
    let output = command(home.path(), &url, false)
        .env("HTTP_PROXY", target.uri())
        .env("HTTPS_PROXY", target.uri())
        .env("ALL_PROXY", target.uri())
        .env("NO_PROXY", "")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("307"));
    assert!(target.received_requests().await.unwrap().is_empty());
    let requests = origin.received_requests().await.unwrap();
    let delete = requests
        .iter()
        .find(|request| request.method == "DELETE")
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&delete.body).unwrap(),
        serde_json::json!({"expected_version":7})
    );
    origin.verify().await;
}
#[tokio::test]
async fn hidden_release_verified_tls_and_saved_profile_refresh_succeed() {
    for flow in [Flow::Success, Flow::RetrySuccess] {
        let api = TlsApi::start(flow, String::new()).await;
        let home = tempfile::tempdir().unwrap();
        if matches!(flow, Flow::Success) {
            let untrusted = command(home.path(), &api.url, false)
                .output()
                .await
                .unwrap();
            assert!(
                !untrusted.status.success(),
                "untrusted private CA must be rejected"
            );
            assert!(api.requests.lock().unwrap().is_empty());
        }
        let saved = matches!(flow, Flow::RetrySuccess);
        let output = command(home.path(), &api.url, saved)
            .env("NYXID_CA_CERT", api.ca.path())
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["version"],
            7
        );
        let requests = api.requests.lock().unwrap();
        let deletes: Vec<_> = requests
            .iter()
            .filter(|request| request.starts_with("DELETE "))
            .collect();
        assert_eq!(deletes.len(), if saved { 2 } else { 1 });
        for delete in &deletes {
            assert!(delete.ends_with(r#"{"expected_version":7}"#));
        }
        if saved {
            assert!(
                deletes[0]
                    .to_lowercase()
                    .contains("authorization: bearer old-fixture-access")
            );
            assert!(
                deletes[1]
                    .to_lowercase()
                    .contains("authorization: bearer fresh-fixture-access")
            );
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.starts_with("POST /api/v1/auth/refresh "))
                    .count(),
                1
            );
            assert_eq!(
                std::fs::read_to_string(home.path().join(".nyxid/profiles/transport/access_token"))
                    .unwrap(),
                "fresh-fixture-access"
            );
        }
    }
}
#[tokio::test]
async fn hidden_release_tls_downgrade_is_refused_initially_after_refresh_and_at_refresh() {
    for flow in [
        Flow::DeleteRedirect,
        Flow::RetryDeleteRedirect,
        Flow::RefreshRedirect,
    ] {
        let target = MockServer::start().await;
        let api = TlsApi::start(flow, target.uri()).await;
        let home = tempfile::tempdir().unwrap();
        let saved = !matches!(flow, Flow::DeleteRedirect);
        let output = command(home.path(), &api.url, saved)
            .env("NYXID_CA_CERT", api.ca.path())
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
        assert!(
            target.received_requests().await.unwrap().is_empty(),
            "redirect must never reach cleartext target"
        );
        let requests = api.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("DELETE "))
                .count(),
            if matches!(flow, Flow::RetryDeleteRedirect) {
                2
            } else {
                1
            }
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST /api/v1/auth/refresh "))
                .count(),
            usize::from(saved)
        );
        if matches!(flow, Flow::RefreshRedirect) {
            assert_eq!(
                std::fs::read_to_string(home.path().join(".nyxid/profiles/transport/access_token"))
                    .unwrap(),
                "old-fixture-access"
            );
        }
    }
}
