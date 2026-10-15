use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Exercise real reqwest + Telegram/FCM serializers against local TLS endpoints.
/// DNS overrides are scoped to this client's fixture hosts, never process-global.
pub(crate) async fn result_transport() -> (
    reqwest::Client,
    Arc<Mutex<Vec<(String, Value)>>>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let hosts = vec![
        "api.telegram.org".into(),
        "fcm.googleapis.com".into(),
        "oauth2.googleapis.com".into(),
    ];
    let certified = rcgen::generate_simple_self_signed(hosts).unwrap();
    let root = reqwest::Certificate::from_der(certified.cert.der()).unwrap();
    let private =
        rustls::pki_types::PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![certified.cert.der().clone()], private.into())
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let sink = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let sink = sink.clone();
            tokio::spawn(async move {
                let mut stream = acceptor.accept(stream).await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let body_at = loop {
                    let n = stream.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(pos) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..body_at]);
                let route = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or_default();
                while bytes.len() < body_at + length {
                    let n = stream.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let body = if route == "/token" {
                    json!({ "access_token": "fixture-access-token", "expires_in": 3600 })
                } else {
                    sink.lock().await.push((
                        route.clone(),
                        serde_json::from_slice(&bytes[body_at..body_at + length]).unwrap_or_else(|_| json!({"multipart":String::from_utf8_lossy(&bytes[body_at..body_at + length])})),
                    ));
                    if route.ends_with("sendPhoto") && bytes.windows(10).any(|b| b == b"FAIL_IMAGE") {
                        json!({"ok":false})
                    } else if route.ends_with("sendMessage") || route.ends_with("sendPhoto") {
                        json!({ "ok": true, "result": {"message_id":123} })
                    } else {
                        json!({ "name": "projects/test/messages/result" })
                    }
                }
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            });
        }
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .add_root_certificate(root)
        .resolve("api.telegram.org", address)
        .resolve("fcm.googleapis.com", address)
        .resolve("oauth2.googleapis.com", address)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    (client, requests, server)
}
