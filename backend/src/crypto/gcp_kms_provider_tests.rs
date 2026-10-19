//! Local SDK wire/caller tests. The emulator transport is explicit and test-only;
//! production still constructs its authenticated TLS client from configuration.
use super::*;
use google_cloud_kms::grpc::kms::v1::{
    DecryptRequest, DecryptResponse, EncryptRequest, EncryptResponse,
};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tonic::{Request, Response, Status};

const CURRENT: &str = "projects/fixture/locations/local/keyRings/test/cryptoKeys/current";
const PREVIOUS: &str = "projects/fixture/locations/local/keyRings/test/cryptoKeys/previous";
const DEK: &[u8] = b"local-fixture-dek-never-a-live-key";
const WRAPPED: &[u8] = b"local-fixture-wrapped-dek";

#[derive(Default)]
struct State {
    requests: Mutex<Vec<(String, String)>>,
    fail_first: usize,
    deny: bool,
}

impl State {
    fn record(&self, method: &str, name: &str) -> Result<(), Status> {
        let mut requests = self.requests.lock().unwrap();
        requests.push((method.into(), name.into()));
        if self.deny {
            return Err(Status::permission_denied("fixture denied"));
        }
        if requests.len() <= self.fail_first {
            // Not retried by the SDK's Unavailable/Unknown policy, so this
            // exercises the provider's existing three-attempt outer retry.
            return Err(Status::deadline_exceeded("fixture temporary failure"));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Fixture(Arc<State>);

impl tonic::server::NamedService for Fixture {
    const NAME: &'static str = "google.cloud.kms.v1.KeyManagementService";
}

impl tonic::server::UnaryService<EncryptRequest> for Fixture {
    type Response = EncryptResponse;
    type Future = std::future::Ready<Result<Response<Self::Response>, Status>>;
    fn call(&mut self, request: Request<EncryptRequest>) -> Self::Future {
        let request = request.into_inner();
        assert_eq!(request.plaintext, DEK);
        std::future::ready(self.0.record("encrypt", &request.name).map(|()| {
            Response::new(EncryptResponse {
                ciphertext: WRAPPED.into(),
                ..Default::default()
            })
        }))
    }
}

impl tonic::server::UnaryService<DecryptRequest> for Fixture {
    type Response = DecryptResponse;
    type Future = std::future::Ready<Result<Response<Self::Response>, Status>>;
    fn call(&mut self, request: Request<DecryptRequest>) -> Self::Future {
        let request = request.into_inner();
        assert_eq!(request.ciphertext, WRAPPED);
        std::future::ready(self.0.record("decrypt", &request.name).map(|()| {
            Response::new(DecryptResponse {
                plaintext: DEK.into(),
                ..Default::default()
            })
        }))
    }
}

impl tower::Service<http::Request<tonic::body::Body>> for Fixture {
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, request: http::Request<tonic::body::Body>) -> Self::Future {
        let fixture = self.clone();
        Box::pin(async move {
            let response = match request.uri().path() {
                "/google.cloud.kms.v1.KeyManagementService/Encrypt" => {
                    tonic::server::Grpc::new(tonic_prost::ProstCodec::<
                        EncryptResponse,
                        EncryptRequest,
                    >::default())
                    .unary(fixture, request)
                    .await
                }
                "/google.cloud.kms.v1.KeyManagementService/Decrypt" => {
                    tonic::server::Grpc::new(tonic_prost::ProstCodec::<
                        DecryptResponse,
                        DecryptRequest,
                    >::default())
                    .unary(fixture, request)
                    .await
                }
                _ => panic!("unexpected local KMS RPC"),
            };
            Ok(response)
        })
    }
}

async fn provider(state: Arc<State>) -> (GcpKmsProvider, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(Fixture(state))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let connection = google_cloud_kms::grpc::apiv1::conn_pool::ConnectionManager::new(
        1,
        "unused-in-explicit-emulator",
        &google_cloud_gax::conn::Environment::Emulator(address.to_string()),
        &google_cloud_gax::conn::ConnectionOptions::default(),
    )
    .await
    .unwrap();
    let provider = GcpKmsProvider {
        client: google_cloud_kms::grpc::apiv1::kms_client::Client::new(Arc::new(connection)),
        keys: GcpKmsKeyMetadata::new(CURRENT.into(), Some(PREVIOUS.into())),
    };
    (provider, server)
}

#[tokio::test]
async fn local_sdk_wrap_unwrap_preserves_current_and_previous_key_routing() {
    let state = Arc::new(State::default());
    let (provider, server) = provider(state.clone()).await;
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
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
            *state.requests.lock().unwrap(),
            [
                ("encrypt".into(), CURRENT.into()),
                ("decrypt".into(), CURRENT.into()),
                ("decrypt".into(), PREVIOUS.into())
            ]
        );
    })
    .await;
    server.abort();
    result.unwrap();
}

#[tokio::test]
async fn local_sdk_provider_retries_and_fails_without_exposing_upstream_details() {
    for deny in [false, true] {
        let state = Arc::new(State {
            fail_first: 2,
            deny,
            ..Default::default()
        });
        let (provider, server) = provider(state.clone()).await;
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(5), provider.wrap_dek(DEK)).await;
        server.abort();
        let result = result.unwrap();
        if deny {
            let error = result.expect_err("denied KMS must fail").to_string();
            assert!(!error.contains("fixture denied"));
            assert!(!error.contains(CURRENT));
        } else {
            assert_eq!(&*result.unwrap().ciphertext, WRAPPED);
        }
        assert_eq!(state.requests.lock().unwrap().len(), 3);
    }
}

#[tokio::test]
async fn local_service_account_exchange_uses_signed_assertion_and_kms_scope() {
    use axum::{Form, Json, Router, routing::post};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    // Fresh fixture key, no ADC environment, metadata service, or live account.
    let key = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
    let private_pem = key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let public_pem = key
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .unwrap();
    let decoding_key =
        Arc::new(jsonwebtoken::DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let audience = format!("http://{}/token", listener.local_addr().unwrap());
    let expected_audience = audience.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let router = Router::new().route("/token", post(move |Form(fields): Form<HashMap<String, String>>| {
        let decoding_key = decoding_key.clone();
        let audience = expected_audience.clone();
        let calls = observed.clone();
        async move {
            assert_eq!(fields.get("grant_type").unwrap(), "urn:ietf:params:oauth:grant-type:jwt-bearer");
            let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
            validation.set_audience(&[audience]);
            validation.set_issuer(&["fixture@local.invalid"]);
            let claims = jsonwebtoken::decode::<serde_json::Value>(fields.get("assertion").unwrap(), &decoding_key, &validation).unwrap().claims;
            assert_eq!(claims["scope"], "https://www.googleapis.com/auth/cloud-platform");
            calls.fetch_add(1, Ordering::SeqCst);
            Json(serde_json::json!({"access_token": "fixture-only-kms-token", "token_type": "Bearer", "expires_in": 3600}))
        }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let credentials = serde_json::from_value(serde_json::json!({
        "type": "service_account", "project_id": "local-fixture", "private_key_id": "fixture-key",
        "private_key": private_pem.as_str(), "client_email": "fixture@local.invalid", "token_uri": audience,
    })).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let config = google_cloud_kms::client::ClientConfig::default()
            .with_credentials(credentials)
            .await
            .unwrap();
        let token = config
            .token_source_provider
            .token_source()
            .token()
            .await
            .unwrap();
        assert_eq!(token, "Bearer fixture-only-kms-token");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    })
    .await;
    server.abort();
    result.unwrap();
}
