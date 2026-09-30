use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub(crate) struct Server {
    pub address: std::net::SocketAddr,
    pub ca_pem: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn start(host: &str, websocket: bool) -> Self {
        Self::with_leaf(host, websocket, false).await
    }
    async fn with_leaf(host: &str, websocket: bool, self_signed: bool) -> Self {
        let mut params = rcgen::CertificateParams::new(vec!["Private Test CA".into()]).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        let key = rcgen::KeyPair::generate().unwrap();
        let ca = params.self_signed(&key).unwrap();
        let issuer = rcgen::Issuer::from_params(&params, &key);
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec![host.into()]).unwrap();
        let leaf = if self_signed {
            params.self_signed(&leaf_key).unwrap()
        } else {
            params.signed_by(&leaf_key, &issuer).unwrap()
        };
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
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    if websocket {
                        let _ = tokio_tungstenite::accept_async(stream).await;
                    } else {
                        let mut buf = vec![0; 4096];
                        let _ = stream.read(&mut buf).await;
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await;
                    }
                });
            }
        });
        Self {
            address,
            ca_pem: ca.pem(),
            task,
        }
    }
    pub fn url(&self) -> String {
        format!("https://localhost:{}/", self.address.port())
    }
    pub fn ca_file(&self) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(self.ca_pem.as_bytes()).unwrap();
        file
    }
}

pub(crate) fn configured(ca_file: Option<PathBuf>) -> Trust {
    load(
        &Sources {
            ca_file,
            ..Default::default()
        },
        Default::default,
    )
}
pub(crate) fn client(trust: &Trust) -> reqwest::Client {
    http_builder(trust.config.as_ref().unwrap())
        .no_proxy()
        .build()
        .unwrap()
}

#[test]
fn bundled_roots_survive_empty_or_unavailable_platform_store() {
    let trust = configured(None);
    assert!(trust.sources[0].certificates > 0);
    assert_eq!(trust.report().sources.last().unwrap().certificates, 0);
    assert!(trust.config.is_ok());
}

#[tokio::test]
async fn private_ca_rejected_by_default_and_accepted_by_explicit_pem() {
    let server = Server::start("localhost", false).await;
    assert!(
        client(&configured(None))
            .get(server.url())
            .send()
            .await
            .is_err()
    );
    let ca = server.ca_file();
    let trust = configured(Some(ca.path().into()));
    assert_eq!(trust.sources.last().unwrap().certificates, 1);
    assert!(
        client(&trust)
            .get(server.url())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
}

#[tokio::test]
async fn private_ca_accepted_by_ssl_cert_file() {
    let server = Server::start("localhost", false).await;
    let ca = server.ca_file();
    let trust = load(
        &Sources {
            system_file: Some(ca.path().into()),
            ..Default::default()
        },
        || panic!("explicit paths replace platform lookup"),
    );
    assert_eq!(trust.sources[1].name, "SSL_CERT_FILE");
    assert_eq!(trust.sources[1].certificates, 1);
    assert!(
        client(&trust)
            .get(server.url())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
}

#[tokio::test]
async fn trusted_ca_does_not_disable_hostname_verification_or_trust_another_issuer() {
    let wrong_host = Server::start("different.test", false).await;
    let ca = wrong_host.ca_file();
    let trusted = client(&configured(Some(ca.path().into())));
    assert!(trusted.get(wrong_host.url()).send().await.is_err());
    let untrusted = Server::start("localhost", false).await;
    assert!(trusted.get(untrusted.url()).send().await.is_err());
}

#[test]
fn explicit_ca_missing_garbage_empty_and_invalid_der_fail_with_variable_and_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ca.pem");
    for contents in [
        None,
        Some("garbage"),
        Some(""),
        Some("-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n"),
    ] {
        if let Some(contents) = contents {
            std::fs::write(&path, contents).unwrap();
        }
        let error = configured(Some(path.clone()))
            .config
            .unwrap_err()
            .to_string();
        assert!(error.contains("NYXID_CA_CERT"), "{error}");
        assert!(error.contains(path.to_str().unwrap()), "{error}");
    }
}

#[test]
fn explicit_system_sources_must_each_yield_usable_certificates() {
    let empty = tempfile::tempdir().unwrap();
    for sources in [
        Sources {
            system_file: Some(empty.path().join("missing")),
            ..Default::default()
        },
        Sources {
            system_dirs: Some(empty.path().as_os_str().into()),
            ..Default::default()
        },
    ] {
        let trust = load(&sources, Default::default);
        let name = if sources.system_file.is_some() {
            "SSL_CERT_FILE"
        } else {
            "SSL_CERT_DIR"
        };
        assert!(trust.config.unwrap_err().to_string().contains(name));
    }
}

#[tokio::test]
async fn private_ca_tls_through_connect_proxy() {
    let server = Server::start("localhost", false).await;
    let ca = server.ca_file();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = listener.local_addr().unwrap();
    let target = server.address;
    let proxy = tokio::spawn(async move {
        let (mut incoming, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(incoming.read_u8().await.unwrap());
        }
        assert!(request.starts_with(b"CONNECT localhost:"));
        let mut upstream = tokio::net::TcpStream::connect(target).await.unwrap();
        incoming
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        let _ = tokio::io::copy_bidirectional(&mut incoming, &mut upstream).await;
    });
    let trust = configured(Some(ca.path().into()));
    let client = http_builder(trust.config.as_ref().unwrap())
        .proxy(reqwest::Proxy::all(format!("http://{proxy_address}")).unwrap())
        .build()
        .unwrap();
    assert!(
        client
            .get(server.url())
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    proxy.await.unwrap();
}

#[tokio::test]
async fn websocket_handshake_uses_same_trust_as_http() {
    let server = Server::start("localhost", true).await;
    let ca = server.ca_file();
    let trust = configured(Some(ca.path().into()));
    let url = format!("wss://localhost:{}/", server.address.port());
    let connect = |config| {
        tokio_tungstenite::connect_async_tls_with_config(
            url.clone(),
            None,
            false,
            Some(tokio_tungstenite::Connector::Rustls(config)),
        )
    };
    assert!(connect(configured(None).config.unwrap()).await.is_err());
    assert!(connect(trust.config.unwrap()).await.is_ok());
}

#[tokio::test]
async fn self_signed_leaf_is_rejected_even_with_an_additional_ca() {
    let server = Server::with_leaf("localhost", false, true).await;
    let ca = server.ca_file();
    for trust in [configured(None), configured(Some(ca.path().into()))] {
        assert!(client(&trust).get(server.url()).send().await.is_err());
    }
}

#[tokio::test]
async fn system_directory_skips_invalid_certs_and_combines_with_explicit_bundle() {
    let server = Server::start("localhost", false).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("valid.pem"), &server.ca_pem).unwrap();
    std::fs::write(
        dir.path().join("invalid.pem"),
        "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    let ca = server.ca_file();
    let trust = load(
        &Sources {
            ca_file: Some(ca.path().into()),
            system_dirs: Some(dir.path().as_os_str().into()),
            ..Default::default()
        },
        Default::default,
    );
    assert_eq!(trust.sources[1].certificates, 1);
    assert_eq!(trust.sources[2].certificates, 1);
    assert!(client(&trust).get(server.url()).send().await.is_ok());
}

#[test]
fn trust_and_config_are_cached_for_the_process() {
    assert!(std::ptr::eq(trust(), trust()));
    assert!(Arc::ptr_eq(
        &shared_config().unwrap(),
        &shared_config().unwrap()
    ));
}

fn counted_platform(
    pem: String,
    count: Arc<std::sync::atomic::AtomicUsize>,
) -> impl FnOnce() -> rustls_native_certs::CertificateResult + Send {
    move || {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut result = rustls_native_certs::CertificateResult::default();
        result.certs = CertificateDer::pem_slice_iter(pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        result
    }
}

#[tokio::test]
async fn eager_trusted_handshakes_never_load_platform_roots() {
    let server = Server::start("localhost", false).await;
    let ca = server.ca_file();
    let trust = load(
        &Sources {
            ca_file: Some(ca.path().into()),
            ..Default::default()
        },
        || panic!("eager trust must not load the OS store"),
    );
    for _ in 0..3 {
        assert!(
            client(&trust)
                .get(server.url())
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );
    }
}

#[tokio::test]
async fn native_only_ca_loads_once_across_concurrent_clients_and_handshakes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let server = Server::start("localhost", false).await;
    let count = Arc::new(AtomicUsize::new(0));
    let trust = load(
        &Sources::default(),
        counted_platform(server.ca_pem.clone(), count.clone()),
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let clients = [client(&trust), client(&trust), client(&trust)];
    let results =
        futures::future::join_all(clients.iter().map(|client| client.get(server.url()).send()))
            .await;
    assert!(
        results
            .into_iter()
            .all(|result| result.unwrap().status().is_success())
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(trust.report().sources.last().unwrap().certificates, 1);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_fallback_preserves_hostname_and_issuer_checks() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let wrong_host = Server::start("different.test", false).await;
    let count = Arc::new(AtomicUsize::new(0));
    let trust = load(
        &Sources::default(),
        counted_platform(wrong_host.ca_pem.clone(), count.clone()),
    );
    assert!(client(&trust).get(wrong_host.url()).send().await.is_err());
    let untrusted = Server::start("localhost", false).await;
    assert!(client(&trust).get(untrusted.url()).send().await.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_or_empty_native_store_keeps_eager_roots_usable() {
    let eager = Server::start("localhost", false).await;
    let other = Server::start("localhost", false).await;
    let ca = eager.ca_file();
    for fail in [false, true] {
        let trust = load(
            &Sources {
                ca_file: Some(ca.path().into()),
                ..Default::default()
            },
            move || {
                if fail {
                    let dir = tempfile::tempdir().unwrap();
                    rustls_native_certs::load_certs_from_paths(
                        Some(&dir.path().join("missing.pem")),
                        None,
                    )
                } else {
                    Default::default()
                }
            },
        );
        assert!(client(&trust).get(other.url()).send().await.is_err());
        assert_eq!(trust.report().sources.last().unwrap().certificates, 0);
        assert!(trust.sources[0].certificates > 0);
        assert!(
            client(&trust)
                .get(eager.url())
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );
    }
}

#[tokio::test]
async fn websocket_uses_lazy_native_fallback_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let server = Server::start("localhost", true).await;
    let count = Arc::new(AtomicUsize::new(0));
    let trust = load(
        &Sources::default(),
        counted_platform(server.ca_pem.clone(), count.clone()),
    );
    for _ in 0..3 {
        assert!(
            tokio_tungstenite::connect_async_tls_with_config(
                format!("wss://localhost:{}/", server.address.port()),
                None,
                false,
                Some(tokio_tungstenite::Connector::Rustls(
                    trust.config.as_ref().unwrap().clone()
                ))
            )
            .await
            .is_ok()
        );
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn empty_ca_values_are_unset_and_doctor_forces_native_loading() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let count = Arc::new(AtomicUsize::new(0));
    let trust = load(
        &Sources {
            ca_file: Some(PathBuf::new()),
            system_file: Some(PathBuf::new()),
            system_dirs: Some(OsString::new()),
        },
        counted_platform(String::new(), count.clone()),
    );
    assert!(trust.config.is_ok());
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let report = trust.report();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(report.sources.len(), 2);
    assert_eq!(report.sources[1].name, "OS store");
}
