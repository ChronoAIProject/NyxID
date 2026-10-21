//! Actual AWS SDK JSON/SigV4 requests over local authenticated TLS. No default
//! credential provider, metadata service, cloud account, or production endpoint.
use super::*;
use aws_smithy_http_client::{Builder, tls};
use base64::{Engine, engine::general_purpose::STANDARD};
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CURRENT: &str = "arn:aws:kms:us-east-1:000000000000:key/local-current";
const PREVIOUS: &str = "arn:aws:kms:us-east-1:000000000000:key/local-previous";
const DEK: &[u8] = b"fixture-dek-no-live-key";
const WRAPPED: &[u8] = b"fixture-ciphertext";

struct Fixture {
    port: u16,
    ca: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn start(deny: bool) -> Self {
        let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_key = KeyPair::generate().unwrap();
        let ca_certificate = ca.self_signed(&ca_key).unwrap();
        let issuer = rcgen::Issuer::new(ca, ca_key);
        let leaf = CertificateParams::new(vec!["localhost".into()]).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let certificate = leaf.signed_by(&leaf_key, &issuer).unwrap();
        let server = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let Ok(Ok(mut stream)) =
                    tokio::time::timeout(Duration::from_secs(2), acceptor.accept(socket)).await
                else {
                    continue;
                };
                let response = tokio::time::timeout(Duration::from_secs(2), async {
                    let mut headers = Vec::new();
                    while !headers.ends_with(b"\r\n\r\n") {
                        assert!(headers.len() < 16 * 1024);
                        headers.push(stream.read_u8().await.unwrap());
                    }
                    let headers = String::from_utf8(headers).unwrap();
                    assert!(headers.starts_with("POST / HTTP/1.1\r\n"));
                    let field = |name: &str| {
                        headers.lines().filter_map(|line| line.split_once(':'))
                            .find(|(key, _)| key.eq_ignore_ascii_case(name))
                            .map(|(_, value)| value.trim()).unwrap()
                    };
                    assert!(field("authorization").starts_with("AWS4-HMAC-SHA256 Credential=fixture-access/"));
                    assert!(field("authorization").contains("/us-east-1/kms/aws4_request"));
                    let target = field("x-amz-target");
                    let length: usize = field("content-length").parse().unwrap();
                    assert!(length < 16 * 1024);
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).await.unwrap();
                    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let key = body["KeyId"].as_str().unwrap();
                    let output = if target == "TrentService.Encrypt" {
                        assert_eq!(body["Plaintext"], STANDARD.encode(DEK));
                        serde_json::json!({"CiphertextBlob":STANDARD.encode(WRAPPED),"KeyId":key})
                    } else {
                        assert_eq!(target, "TrentService.Decrypt");
                        assert_eq!(body["CiphertextBlob"], STANDARD.encode(WRAPPED));
                        serde_json::json!({"Plaintext":STANDARD.encode(DEK),"KeyId":key})
                    };
                    captured.lock().unwrap().push((target.into(), key.into()));
                    let (status, output) = if deny {
                        ("400 Bad Request", serde_json::json!({"__type":"AccessDeniedException","message":"fixture denied detail"}))
                    } else {
                        ("200 OK", output)
                    };
                    let body = output.to_string();
                    let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/x-amz-json-1.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    stream.write_all(response.as_bytes()).await.unwrap();
                    stream.shutdown().await.unwrap();
                }).await;
                response.unwrap();
            }
        });
        Self {
            port,
            ca: ca_certificate.pem(),
            requests,
            task,
        }
    }

    fn provider(&self, trust_ca: bool, hostname: &str) -> AwsKmsProvider {
        let mut trust = tls::TrustStore::empty();
        if trust_ca {
            trust = trust.with_pem_certificate(self.ca.as_bytes());
        }
        let http = Builder::new()
            .tls_provider(tls::Provider::Rustls(
                tls::rustls_provider::CryptoMode::AwsLc,
            ))
            .tls_context(
                tls::TlsContext::builder()
                    .with_trust_store(trust)
                    .build()
                    .unwrap(),
            )
            .build_https();
        let config = aws_sdk_kms::config::Builder::new()
            .behavior_version(aws_sdk_kms::config::BehaviorVersion::latest())
            .region(aws_sdk_kms::config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_kms::config::Credentials::new(
                "fixture-access",
                "fixture-secret",
                None,
                None,
                "local-fixture",
            ))
            .endpoint_url(format!("https://{hostname}:{}/", self.port))
            .http_client(http)
            .retry_config(aws_sdk_kms::config::retry::RetryConfig::disabled())
            .build();
        AwsKmsProvider {
            client: KmsClient::from_conf(config),
            keys: AwsKmsKeyMetadata::new(CURRENT.into(), Some(PREVIOUS.into())),
        }
    }
}

#[tokio::test]
async fn local_sdk_https_wrap_unwrap_preserves_current_and_previous_key_routing() {
    let fixture = Fixture::start(false).await;
    let provider = fixture.provider(true, "localhost");
    tokio::time::timeout(Duration::from_secs(8), async {
        let wrapped = provider.wrap_dek(DEK).await.unwrap();
        assert_eq!(wrapped.key_id, provider.current_key_id());
        assert_eq!(&*wrapped.ciphertext, WRAPPED);
        assert_eq!(&*provider.unwrap_dek(&wrapped).await.unwrap(), DEK);
        let previous = WrappedKey {
            key_id: derive_key_id_from_str(PREVIOUS),
            ciphertext: Zeroizing::new(WRAPPED.into()),
        };
        assert_eq!(&*provider.unwrap_dek(&previous).await.unwrap(), DEK);
        assert_eq!(
            *fixture.requests.lock().unwrap(),
            [
                ("TrentService.Encrypt".into(), CURRENT.into()),
                ("TrentService.Decrypt".into(), CURRENT.into()),
                ("TrentService.Decrypt".into(), PREVIOUS.into()),
            ]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn local_sdk_rejects_untrusted_tls_wrong_hostname_and_denied_calls() {
    let fixture = Fixture::start(true).await;
    for (trust, hostname) in [(false, "localhost"), (true, "127.0.0.1")] {
        let provider = fixture.provider(trust, hostname);
        let result = tokio::time::timeout(Duration::from_secs(5), provider.wrap_dek(DEK))
            .await
            .unwrap();
        assert!(result.is_err());
        assert!(fixture.requests.lock().unwrap().is_empty());
    }
    let provider = fixture.provider(true, "localhost");
    let error = tokio::time::timeout(Duration::from_secs(5), provider.wrap_dek(DEK))
        .await
        .unwrap()
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("fixture denied detail"));
    assert!(!error.contains(CURRENT));
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
}
