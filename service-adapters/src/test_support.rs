//! Local TLS fixture. Never contacts IFTTT; DNS for maker.ifttt.com is pinned
//! to this listener and only its ephemeral certificate is trusted.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_rustls::{TlsAcceptor, rustls};

pub struct Fixture {
    pub client: crate::ifttt::Client,
    pub requests: mpsc::UnboundedReceiver<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn fixture(response: &str) -> Fixture {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert = rcgen::generate_simple_self_signed(vec!["maker.ifttt.com".into()]).unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = crate::ifttt::Client::new(
        reqwest::Client::builder()
            .no_proxy()
            .resolve("maker.ifttt.com", listener.local_addr().unwrap())
            .add_root_certificate(reqwest::Certificate::from_der(cert.cert.der()).unwrap()),
    )
    .unwrap();
    let (tx, requests) = mpsc::unbounded_channel();
    let response = response.as_bytes().to_vec();
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let mut stream = acceptor.accept(socket).await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            tx.send(request).unwrap();
            if !response.is_empty() {
                stream.write_all(&response).await.unwrap();
            }
            let _ = stream.shutdown().await;
        }
    });
    Fixture {
        client,
        requests,
        task,
    }
}
