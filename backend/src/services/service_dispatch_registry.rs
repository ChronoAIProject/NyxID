//! Static assembly of dispatch policies and their private, per-call bindings.
use super::{proxy_service::ProxyTarget, service_dispatch_guard::ServiceDispatchGuard};
use crate::{
    config::AppConfig,
    crypto::{aes::EncryptionKeys, jwt::JwtKeys},
    errors::{AppError, AppResult},
    models::default_request_header::SecretHeaders,
};
use mongodb::Database;

/// Facts supplied by the authenticated entry point, independently of admission.
pub(crate) struct DispatchCaller<'a> {
    pub(crate) actor_user_id: &'a str,
    pub(crate) resource_owner_id: &'a str,
    pub(crate) user_service_id: Option<&'a str>,
}

pub(crate) struct DispatchAdmission {
    state: AdmissionState,
}

enum AdmissionState {
    Unprotected,
    Protected {
        policy: Policy,
        binding: Box<Binding>,
        guard: Box<dyn ServiceDispatchGuard>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Cma,
}

#[derive(PartialEq, Eq)]
struct Binding {
    actor_user_id: String,
    resource_owner_id: String,
    user_service_id: Option<String>,
    catalog_id: String,
    catalog_slug: Option<String>,
    connection_id: Option<String>,
    target_id: Option<String>,
    base_url: url::Url,
}

impl Binding {
    fn current(target: &ProxyTarget, caller: &DispatchCaller<'_>) -> AppResult<Self> {
        Ok(Self {
            actor_user_id: caller.actor_user_id.into(),
            resource_owner_id: caller.resource_owner_id.into(),
            user_service_id: caller.user_service_id.map(str::to_owned),
            catalog_id: target.service.id.clone(),
            catalog_slug: target.catalog_slug.clone(),
            connection_id: target.connection_id.clone(),
            target_id: target.target_id.clone(),
            base_url: url::Url::parse(&target.base_url).map_err(|_| refused())?,
        })
    }
}

fn policy_for(target: &ProxyTarget) -> Option<Policy> {
    match target.catalog_slug.as_deref() {
        Some("cma") => Some(Policy::Cma),
        _ => None,
    }
}

fn refused() -> AppError {
    AppError::Forbidden("This service requires matching dispatch admission".into())
}

/// The entry points retain their own verified eligibility/transport semantics.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn admit(
    db: &Database,
    keys: &EncryptionKeys,
    jwt: &JwtKeys,
    config: &AppConfig,
    caller: &DispatchCaller<'_>,
    personal: bool,
    target: &ProxyTarget,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    prepare_headers: impl FnOnce() -> AppResult<SecretHeaders> + Send,
    unsupported_transport: bool,
) -> AppResult<DispatchAdmission> {
    let Some(policy) = policy_for(target) else {
        return Ok(DispatchAdmission {
            state: AdmissionState::Unprotected,
        });
    };
    let canonical = super::proxy_authorization::CanonicalPath::from_mcp_built(path)?;
    let headers = prepare_headers()?;
    let binding = Binding::current(target, caller)?;
    let guard: Box<dyn ServiceDispatchGuard> = match policy {
        Policy::Cma => Box::new(
            super::assistant_cma_admission::admit(
                db,
                keys,
                jwt,
                config,
                caller.resource_owner_id,
                personal,
                target,
                method,
                &canonical.as_policy_path(),
                query,
                body,
                &headers,
                unsupported_transport,
            )
            .await
            .map_err(|error| connection_configuration_error(error, caller))?,
        ),
    };
    Ok(DispatchAdmission {
        state: AdmissionState::Protected {
            policy,
            binding: Box::new(binding),
            guard,
        },
    })
}

// Keep the stable conflict code/prefix; identify only the caller's already
// authorized connection, never URLs, credentials or another owner's account.
fn connection_configuration_error(error: AppError, caller: &DispatchCaller<'_>) -> AppError {
    match (error, caller.user_service_id) {
        (AppError::Conflict(message), Some(connection))
            if message == "cma_configuration_required" =>
        {
            AppError::Conflict(format!(
                "cma_configuration_required: Check CMA connection {connection}; its target must match the current catalog deployment. For a managed connection, contact the administrator. Keep existing resource and request identities"
            ))
        }
        (error, _) => error,
    }
}

/// Every HTTP send uses this gate, including wrappers without an admission.
/// Guards remain private until both the live caller and target match.
pub(crate) fn verify<'a>(
    admission: Option<&'a DispatchAdmission>,
    target: &ProxyTarget,
    caller: Option<&DispatchCaller<'_>>,
) -> AppResult<Option<&'a dyn ServiceDispatchGuard>> {
    match (
        policy_for(target),
        admission.map(|admission| &admission.state),
    ) {
        (None, None | Some(AdmissionState::Unprotected)) => Ok(None),
        (
            Some(expected),
            Some(AdmissionState::Protected {
                policy,
                binding,
                guard,
            }),
        ) if expected == *policy => {
            let caller = caller.ok_or_else(refused)?;
            if binding.as_ref() != &Binding::current(target, caller)? {
                return Err(refused());
            }
            Ok(Some(guard.as_ref()))
        }
        _ => Err(refused()),
    }
}

#[cfg(test)]
pub(crate) fn test_admission(
    target: &ProxyTarget,
    caller: &DispatchCaller<'_>,
    guard: Box<dyn ServiceDispatchGuard>,
) -> DispatchAdmission {
    DispatchAdmission {
        state: AdmissionState::Protected {
            policy: Policy::Cma,
            binding: Box::new(Binding::current(target, caller).unwrap()),
            guard,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        cloud_response_cache::CloudResponseCache,
        provider_token_exchange_service::TokenExchangeCache,
        proxy_service::{self, ForwardRequestError, ProxyBody},
    };
    use bytes::Bytes;
    use std::sync::{Arc, Mutex};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    struct Guard {
        calls: Arc<Mutex<Vec<&'static str>>>,
        reject_before: bool,
        reject_response: bool,
    }

    #[async_trait::async_trait]
    impl ServiceDispatchGuard for Guard {
        async fn before_send(&self, request: &reqwest::Request) -> AppResult<()> {
            self.calls.lock().unwrap().push("before");
            assert_eq!(request.method(), reqwest::Method::POST);
            assert_eq!(request.url().path(), "/operation");
            assert_eq!(request.url().query(), Some("selected=1"));
            assert_eq!(request.headers()["idempotency-key"], "original-key");
            assert_eq!(
                request.body().and_then(reqwest::Body::as_bytes),
                Some(&b"original body"[..])
            );
            if self.reject_before {
                return Err(AppError::Forbidden("test dispatch refused".into()));
            }
            Ok(())
        }

        async fn observe_response(
            &self,
            response: reqwest::Response,
        ) -> AppResult<reqwest::Response> {
            self.calls.lock().unwrap().push("response");
            if self.reject_response {
                return Err(AppError::Internal("test receipt persistence failed".into()));
            }
            Ok(response)
        }
    }

    #[tokio::test]
    async fn configuration_detail_keeps_public_error_code_and_native_recovery_marker() {
        use axum::response::IntoResponse;
        let error = connection_configuration_error(
            AppError::Conflict("cma_configuration_required".into()),
            &caller(),
        );
        let response = error.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error_code"], 1004);
        assert_eq!(body["error"], "conflict");
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .contains("cma_configuration_required: Check CMA connection service-connection")
        );
        let native = DispatchCaller {
            user_service_id: None,
            ..caller()
        };
        assert!(matches!(connection_configuration_error(
            AppError::Conflict("cma_configuration_required".into()), &native,
        ), AppError::Conflict(message) if message == "cma_configuration_required"));
    }

    fn caller() -> DispatchCaller<'static> {
        DispatchCaller {
            actor_user_id: "actor",
            resource_owner_id: "owner",
            user_service_id: Some("service-connection"),
        }
    }

    fn target(base_url: String) -> ProxyTarget {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.base_url = base_url.clone();
        service.slug = "connection-alias".into();
        ProxyTarget {
            catalog_slug: Some("cma".into()),
            execution_request: None,
            execution_permission: None,
            credential_deferred: false,
            workspace_destinations_pending: false,
            target_id: None,
            base_url,
            auth_method: "none".into(),
            auth_key_name: String::new(),
            credential: Default::default(),
            service,
            catalog_default_headers: vec![],
            user_service_default_headers: vec![],
            ws_frame_injections: vec![],
            connection_id: Some("credential-connection".into()),
        }
    }

    fn admission(
        target: &ProxyTarget,
        before: bool,
        response: bool,
    ) -> (DispatchAdmission, Arc<Mutex<Vec<&'static str>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let admission = test_admission(
            target,
            &caller(),
            Box::new(Guard {
                calls: calls.clone(),
                reject_before: before,
                reject_response: response,
            }),
        );
        (admission, calls)
    }

    async fn send(
        target: &ProxyTarget,
        admission: Option<&DispatchAdmission>,
        caller: Option<&DispatchCaller<'_>>,
        body: ProxyBody,
    ) -> Result<reqwest::Response, ForwardRequestError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("idempotency-key", "original-key".parse().unwrap());
        proxy_service::forward_request_with_resource_proof(
            &reqwest::Client::new(),
            target,
            reqwest::Method::POST,
            "/operation",
            Some("selected=1"),
            headers,
            body,
            vec![],
            vec![],
            None,
            &TokenExchangeCache::new(),
            &CloudResponseCache::new(0),
            vec![],
            crate::services::billing::route_inventory::internal_node_dispatch_permit(),
            None,
            None,
            admission,
            caller,
        )
        .await
    }

    fn body() -> ProxyBody {
        ProxyBody::Buffered(Some(Bytes::from_static(b"original body")))
    }

    #[tokio::test]
    async fn service_dispatch_rejects_missing_admission_caller_and_every_binding_mismatch_without_sending()
     {
        let server = MockServer::start().await;
        let mut target = target(server.uri());
        let (admission, calls) = admission(&target, false, false);
        let unprotected = DispatchAdmission {
            state: AdmissionState::Unprotected,
        };
        for (admission, caller) in [
            (None, Some(caller())),
            (Some(&unprotected), Some(caller())),
            (Some(&admission), None),
        ] {
            assert!(matches!(
                send(&target, admission, caller.as_ref(), body()).await,
                Err(ForwardRequestError::PreDispatchRejected(_))
            ));
        }
        for caller in [
            DispatchCaller {
                actor_user_id: "other",
                ..caller()
            },
            DispatchCaller {
                resource_owner_id: "other",
                ..caller()
            },
            DispatchCaller {
                user_service_id: Some("other"),
                ..caller()
            },
            DispatchCaller {
                user_service_id: None,
                ..caller()
            },
        ] {
            assert!(matches!(
                send(&target, Some(&admission), Some(&caller), body()).await,
                Err(ForwardRequestError::PreDispatchRejected(_))
            ));
        }
        for field in [
            "catalog_id",
            "catalog_slug",
            "connection_id",
            "target_id",
            "base_url",
        ] {
            let mut changed = self::target(server.uri());
            changed.service.id.clone_from(&target.service.id);
            match field {
                "catalog_id" => changed.service.id = "other".into(),
                "catalog_slug" => changed.catalog_slug = Some("ordinary".into()),
                "connection_id" => changed.connection_id = Some("other".into()),
                "target_id" => changed.target_id = Some("other".into()),
                "base_url" => changed.base_url.push_str("/other"),
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    send(&changed, Some(&admission), Some(&caller()), body()).await,
                    Err(ForwardRequestError::PreDispatchRejected(_))
                ),
                "{field}"
            );
        }
        target.catalog_slug = None;
        assert!(verify(Some(&admission), &target, Some(&caller())).is_err());
        assert!(calls.lock().unwrap().is_empty());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn service_dispatch_guard_checks_final_request_and_preserves_readable_response() {
        let server = MockServer::start().await;
        Mock::given(path("/operation"))
            .respond_with(ResponseTemplate::new(201).set_body_string("receipt"))
            .expect(1)
            .mount(&server)
            .await;
        let target = target(server.uri());
        let (admission, calls) = admission(&target, false, false);
        let response = send(&target, Some(&admission), Some(&caller()), body())
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        assert_eq!(response.text().await.unwrap(), "receipt");
        assert_eq!(*calls.lock().unwrap(), vec!["before", "response"]);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn service_dispatch_rejects_before_send_and_refuses_streamed_protected_requests() {
        let server = MockServer::start().await;
        let target = target(server.uri());
        let (admission, calls) = admission(&target, true, false);
        assert!(matches!(
            send(&target, Some(&admission), Some(&caller()), body()).await,
            Err(ForwardRequestError::PreDispatchRejected(_))
        ));
        assert_eq!(*calls.lock().unwrap(), vec!["before"]);
        assert!(matches!(
            send(
                &target,
                Some(&admission),
                Some(&caller()),
                ProxyBody::Streaming(axum::body::Body::from("stream"))
            )
            .await,
            Err(ForwardRequestError::PreDispatchRejected(_))
        ));
        assert_eq!(*calls.lock().unwrap(), vec!["before"]);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn service_dispatch_ordinary_targets_accept_none_and_explicit_unprotected_without_guards()
    {
        let server = MockServer::start().await;
        Mock::given(path("/operation"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ordinary"))
            .expect(2)
            .mount(&server)
            .await;
        let mut target = target(server.uri());
        target.catalog_slug = None;
        target.service.slug = "cma".into(); // A custom alias cannot select a policy.
        let unprotected = DispatchAdmission {
            state: AdmissionState::Unprotected,
        };
        for admission in [None, Some(&unprotected)] {
            assert!(verify(admission, &target, None).unwrap().is_none());
            assert_eq!(
                send(&target, admission, None, body())
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "ordinary"
            );
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn service_dispatch_receipt_failure_is_unknown_after_exactly_one_send() {
        let server = MockServer::start().await;
        Mock::given(path("/operation"))
            .respond_with(ResponseTemplate::new(201).set_body_string("receipt"))
            .expect(1)
            .mount(&server)
            .await;
        let target = target(server.uri());
        let (admission, calls) = admission(&target, false, true);
        assert!(matches!(
            send(&target, Some(&admission), Some(&caller()), body()).await,
            Err(ForwardRequestError::OutcomeUnknown)
        ));
        assert_eq!(*calls.lock().unwrap(), vec!["before", "response"]);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn service_dispatch_connection_refusal_retains_connect_transport_classification() {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);
        let target = target(format!("http://{address}"));
        let (admission, calls) = admission(&target, false, false);
        let Err(ForwardRequestError::Transport(error)) =
            send(&target, Some(&admission), Some(&caller()), body()).await
        else {
            panic!("expected transport failure")
        };
        assert!(error.is_connect());
        assert_eq!(*calls.lock().unwrap(), vec!["before"]);
    }

    #[tokio::test]
    async fn service_dispatch_lost_response_retains_non_connect_transport_classification() {
        use tokio::io::AsyncReadExt;
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = target(format!("http://{}", socket.local_addr().unwrap()));
        let received = tokio::spawn(async move {
            let (mut stream, _) = socket.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 2048];
            loop {
                let count = stream.read(&mut chunk).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.ends_with(b"original body") {
                    break;
                }
            }
            bytes
        });
        let (admission, calls) = admission(&target, false, false);
        let Err(ForwardRequestError::Transport(error)) =
            send(&target, Some(&admission), Some(&caller()), body()).await
        else {
            panic!("expected transport failure")
        };
        assert!(!error.is_connect() && !error.is_builder());
        assert!(received.await.unwrap().ends_with(b"original body"));
        assert_eq!(*calls.lock().unwrap(), vec!["before"]);
    }

    #[tokio::test]
    async fn service_dispatch_ordinary_sse_delivers_before_the_provider_finishes() {
        use futures::StreamExt;
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(2);
        sender
            .send(Ok(Bytes::from_static(b"data: first\n\n")))
            .await
            .unwrap();
        let receiver = Arc::new(Mutex::new(Some(receiver)));
        let router = axum::Router::new().route(
            "/operation",
            axum::routing::post(move || {
                let receiver = receiver.lock().unwrap().take().unwrap();
                async move {
                    axum::http::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from_stream(
                            tokio_stream::wrappers::ReceiverStream::new(receiver),
                        ))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut target = target(format!("http://{}", listener.local_addr().unwrap()));
        target.catalog_slug = Some("ordinary".into());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            send(&target, None, None, body()),
        )
        .await
        .unwrap()
        .unwrap();
        let mut chunks = response.bytes_stream();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(3), chunks.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            Bytes::from_static(b"data: first\n\n")
        );
        sender
            .send(Ok(Bytes::from_static(b"data: second\n\n")))
            .await
            .unwrap();
        drop(sender);
        assert_eq!(
            chunks.next().await.unwrap().unwrap(),
            Bytes::from_static(b"data: second\n\n")
        );
        assert!(chunks.next().await.is_none());
        server.abort();
    }

    #[tokio::test]
    async fn service_dispatch_registry_preserves_concrete_caller_and_transport_refusals() {
        let db =
            crate::test_utils::connect_transaction_test_database("dispatch_caller_qualification")
                .await;
        let keys = crate::test_utils::test_encryption_keys();
        let jwt = crate::test_utils::cached_test_jwt_keys();
        let config = crate::test_utils::test_app_config();
        let target = target("http://127.0.0.1:1".into());
        for (personal, unsupported_transport) in [(false, false), (true, true)] {
            assert!(matches!(
                admit(
                    &db,
                    &keys,
                    &jwt,
                    &config,
                    &caller(),
                    personal,
                    &target,
                    "POST",
                    "/api/v1/agents",
                    None,
                    b"{}",
                    || Ok(SecretHeaders::default()),
                    unsupported_transport
                )
                .await,
                Err(AppError::Forbidden(_))
            ));
        }
        let mut ordinary = self::target("http://127.0.0.1:1".into());
        ordinary.catalog_slug = None;
        ordinary.service.slug = "cma".into();
        let admission = admit(
            &db,
            &keys,
            &jwt,
            &config,
            &caller(),
            false,
            &ordinary,
            "POST",
            "/operation",
            None,
            b"{}",
            || panic!("ordinary services must not prepare dispatch-policy headers"),
            true,
        )
        .await
        .unwrap();
        assert!(verify(Some(&admission), &ordinary, None).unwrap().is_none());
        assert_eq!(
            db.collection::<mongodb::bson::Document>(
                crate::models::assistant_cma_creation::COLLECTION_NAME
            )
            .count_documents(mongodb::bson::doc! {})
            .await
            .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
}
