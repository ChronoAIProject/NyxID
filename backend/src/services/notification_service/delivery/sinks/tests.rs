use super::*;
use crate::{
    models::audit_outbox::{AuditOwner, AuditPrincipal, AuditStage, SystemAuditActor},
    services::{
        audit_outbox_service::AuditEvent,
        audit_runtime::intent::EffectReservation,
        notification_service::delivery::{DeliveryKey, DeliveryRequest, PayloadRef, PayloadSource},
    },
};
use axum::{Router, http::StatusCode, routing::post};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;

struct Server {
    target: String,
    calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(status: StatusCode, value: &'static str) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let router = Router::new().route(
        "/",
        post(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                (status, value)
            }
        }),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server {
        target,
        calls,
        task,
    }
}

fn context(channel: Channel) -> (DeliveryBinding, QueuedDelivery) {
    let operation = Uuid::new_v4();
    let product = Uuid::new_v4();
    let binding = DeliveryBinding {
        destination_id: Uuid::new_v4(),
        payload: PayloadRef::new(PayloadSource::ConnectLink, product, Uuid::new_v4()).unwrap(),
    };
    let request = DeliveryRequest::new(
        DeliveryKey {
            operation_id: operation,
            product_event_id: product,
            destination_id: binding.destination_id,
            channel,
        },
        binding.payload.clone(),
        EffectReservation {
            event_id: Uuid::new_v4(),
            terminal_event_id: Uuid::new_v4(),
            event: AuditEvent {
                operation_id: operation.to_string(),
                stage: AuditStage::Intent,
                principal: AuditPrincipal::System {
                    actor: SystemAuditActor::Delivery,
                },
                owner: AuditOwner::Platform,
                action: "notification_delivery",
                target_id: Some(product.to_string()),
            },
        },
    )
    .unwrap();
    (binding, request.record)
}

fn prepared(binding: DeliveryBinding, channel: Channel, target: &str) -> PreparedSend {
    let client = client_builder().build().unwrap();
    let request = client.post(target).body("PRIVATE_DELIVERY_BODY");
    PreparedSend::finish(binding, channel, client, request).unwrap()
}

#[tokio::test]
async fn sink_webhook_status_is_uncertain_and_never_retried() {
    let destination = server(StatusCode::SERVICE_UNAVAILABLE, "PRIVATE_PROVIDER_ERROR").await;
    let (binding, queued) = context(Channel::Webhook);
    let resolved = prepared(binding, Channel::Webhook, &destination.target).resolved();
    assert!(check_binding(&queued, &resolved.binding, resolved.channel).is_ok());
    let attempt = resolved.prepared;
    assert_eq!(format!("{attempt:?}"), "PreparedSend([REDACTED])");
    assert!(matches!(
        attempt.send_once(&queued).await,
        SendOutcome::UncertainResponse { status: 503 }
    ));
    assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sink_telegram_requires_positive_api_acceptance() {
    for (value, accepted) in [
        (r#"{"ok":true,"result":{"message_id":42}}"#, true),
        (
            r#"{"ok":false,"error_code":403,"description":"PRIVATE_PROVIDER_ERROR"}"#,
            false,
        ),
    ] {
        let destination = server(StatusCode::OK, value).await;
        let (binding, queued) = context(Channel::Telegram);
        let outcome = prepared(binding, Channel::Telegram, &destination.target)
            .send_once(&queued)
            .await;
        if accepted {
            assert!(matches!(
                outcome,
                SendOutcome::Accepted(ProviderReceipt::Telegram { message_id: 42 })
            ));
        } else {
            assert!(matches!(
                outcome,
                SendOutcome::Rejected {
                    status: Some(200),
                    invalid_destination: false
                }
            ));
        }
        assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
    }
    for value in ["PRIVATE_PROVIDER_ERROR", r#"{"ok":true}"#] {
        let destination = server(StatusCode::OK, value).await;
        let (binding, queued) = context(Channel::Telegram);
        assert!(matches!(
            prepared(binding, Channel::Telegram, &destination.target)
                .send_once(&queued)
                .await,
            SendOutcome::UncertainResponse { status: 200 }
        ));
        assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn sink_expired_authentication_never_refreshes_or_resends() {
    for (channel, status, response) in [
        (
            Channel::Fcm,
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"status":"UNAUTHENTICATED"}}"#,
        ),
        (
            Channel::Apns,
            StatusCode::FORBIDDEN,
            r#"{"reason":"ExpiredProviderToken"}"#,
        ),
    ] {
        let destination = server(status, response).await;
        let (binding, queued) = context(channel);
        assert!(matches!(
            prepared(binding, channel, &destination.target)
                .send_once(&queued)
                .await,
            SendOutcome::Rejected {
                invalid_destination: false,
                ..
            }
        ));
        assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn sink_invalid_destination_needs_the_documented_provider_code() {
    for (channel, status, response, invalid) in [
        (
            Channel::Fcm,
            StatusCode::NOT_FOUND,
            r#"{"error":{"status":"NOT_FOUND"}}"#,
            false,
        ),
        (
            Channel::Fcm,
            StatusCode::NOT_FOUND,
            r#"{"error":{"details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}}"#,
            true,
        ),
        (
            Channel::Fcm,
            StatusCode::NOT_FOUND,
            r#"{"error":{"details":[{"@type":"unexpected","errorCode":"UNREGISTERED"}]}}"#,
            false,
        ),
        (
            Channel::Apns,
            StatusCode::BAD_REQUEST,
            r#"{"reason":"BadDeviceToken"}"#,
            true,
        ),
        (
            Channel::Apns,
            StatusCode::BAD_REQUEST,
            r#"{"reason":"BadPayload"}"#,
            false,
        ),
        (
            Channel::Apns,
            StatusCode::GONE,
            r#"{"reason":"Unregistered"}"#,
            true,
        ),
        (
            Channel::Apns,
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"reason":"Unregistered"}"#,
            false,
        ),
    ] {
        let destination = server(status, response).await;
        let (binding, queued) = context(channel);
        let outcome = prepared(binding, channel, &destination.target)
            .send_once(&queued)
            .await;
        match outcome {
            SendOutcome::Rejected {
                invalid_destination,
                ..
            } => assert_eq!(invalid_destination, invalid),
            SendOutcome::UncertainResponse { status: 500 } if !invalid => (),
            _ => panic!("unexpected closed provider outcome"),
        }
        assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn sink_binding_mismatch_performs_no_request() {
    let destination = server(StatusCode::OK, "").await;
    for changed in 0..3 {
        let (mut binding, queued) = context(Channel::Webhook);
        let channel = match changed {
            0 => {
                binding.destination_id = Uuid::new_v4();
                Channel::Webhook
            }
            1 => {
                binding.payload =
                    PayloadRef::new(PayloadSource::ConnectLink, Uuid::new_v4(), Uuid::new_v4())
                        .unwrap();
                Channel::Webhook
            }
            _ => Channel::Fcm,
        };
        assert!(matches!(
            prepared(binding, channel, &destination.target)
                .send_once(&queued)
                .await,
            SendOutcome::Rejected {
                status: None,
                invalid_destination: false
            }
        ));
    }
    assert_eq!(destination.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sink_redirects_never_forward_the_protected_body() {
    let second = server(StatusCode::OK, "PRIVATE_RECEIPT").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}", listener.local_addr().unwrap());
    let second_target = second.target.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let router = Router::new().route(
        "/",
        post(move || {
            let second_target = second_target.clone();
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [("Location", second_target)],
                    "PRIVATE_REDIRECT",
                )
            }
        }),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let (binding, queued) = context(Channel::Webhook);
    assert!(matches!(
        prepared(binding, Channel::Webhook, &target)
            .send_once(&queued)
            .await,
        SendOutcome::UncertainResponse { status: 307 }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.calls.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn sink_webhook_refuses_private_http_and_credential_destinations() {
    for target in [
        "http://example.com/hook",
        "https://127.0.0.1/hook",
        "https://[::1]/hook",
        "https://localhost/hook",
        "https://user:pass@127.0.0.1/hook",
        "https://example.com/hook#private",
    ] {
        let (binding, _) = context(Channel::Webhook);
        let error = PreparedSend::webhook(
            binding,
            target,
            &Zeroizing::new(b"PRIVATE_KEY".to_vec()),
            "connection.expired",
            "PRIVATE_ID",
            Zeroizing::new(b"PRIVATE_BODY".to_vec()),
            SignatureContract::Timestamped,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), unavailable().to_string());
        assert!(!error.to_string().contains(target));
    }
}

#[tokio::test]
async fn sink_webhook_pinning_refuses_private_and_mixed_dns_answers() {
    let public: SocketAddr = "93.184.216.34:443".parse().unwrap();
    assert!(pinned_webhook_client("example.com", &[public]).is_ok());
    assert!(pinned_webhook_client("example.com", &[]).is_err());
    for address in [
        "127.0.0.1:443",
        "10.0.0.1:443",
        "192.168.1.1:443",
        "169.254.169.254:443",
        "[::1]:443",
        "[fe80::1]:443",
        "[::ffff:127.0.0.1]:443",
    ] {
        let private = address.parse().unwrap();
        for answers in [vec![private], vec![public, private], vec![private, public]] {
            let error = pinned_webhook_client("example.com", &answers).unwrap_err();
            assert_eq!(error.to_string(), unavailable().to_string());
            assert!(!error.to_string().contains(address));
        }
    }
    // Public IP literals need no external DNS. Exercise the shared validator's
    // normalized URL and IPv6 brackets through the production preparation path.
    let (_, normalized) = public_webhook_client(" https://[2606:4700:4700::1111]/hook ")
        .await
        .unwrap();
    assert_eq!(normalized, "https://[2606:4700:4700::1111]/hook");
}

#[tokio::test]
async fn sink_provider_error_text_cannot_become_a_receipt_or_audit_detail() {
    use crate::models::audit_outbox::{AuditDetail, WebhookEventKind, WebhookFailureKind};

    const SENTINEL: &str = "SENTINEL_PROVIDER_ERROR_MUST_NOT_ESCAPE";
    for (channel, status, response) in [
        (Channel::Webhook, StatusCode::SERVICE_UNAVAILABLE, SENTINEL),
        (
            Channel::Fcm,
            StatusCode::UNAUTHORIZED,
            r#"{"name":"SENTINEL_PROVIDER_ERROR_MUST_NOT_ESCAPE","error":{"message":"SENTINEL_PROVIDER_ERROR_MUST_NOT_ESCAPE","details":[]}}"#,
        ),
        (
            Channel::Apns,
            StatusCode::BAD_REQUEST,
            r#"{"reason":"SENTINEL_PROVIDER_ERROR_MUST_NOT_ESCAPE"}"#,
        ),
        (
            Channel::Telegram,
            StatusCode::BAD_REQUEST,
            r#"{"ok":false,"error_code":400,"description":"SENTINEL_PROVIDER_ERROR_MUST_NOT_ESCAPE","result":{"message_id":42}}"#,
        ),
    ] {
        let destination = server(status, response).await;
        let (binding, queued) = context(channel);
        let outcome = prepared(binding, channel, &destination.target)
            .send_once(&queued)
            .await;
        // An error body cannot create a protected success receipt, even when
        // it includes fields named like the provider's acceptance identity.
        let http_status = match outcome {
            SendOutcome::UncertainResponse { status } => status,
            SendOutcome::Rejected {
                status: Some(status),
                invalid_destination: false,
            } => status,
            _ => panic!("provider error escaped the closed failure outcome"),
        };
        assert_eq!(http_status, status.as_u16());
        // A consumer can project only closed reason and numeric evidence from
        // this failure; provider strings are absent from the returned type.
        let detail = AuditDetail::WebhookDeliveryFailure {
            event: WebhookEventKind::ConnectLinkCompleted,
            reason: WebhookFailureKind::NonSuccessStatus,
            attempts: 1,
            last_http_status: Some(http_status),
        };
        let evidence = serde_json::to_string(&detail).unwrap();
        assert!(!evidence.contains(SENTINEL));
        assert!(!evidence.contains(&destination.target));
        assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn sink_webhook_preserves_both_signature_protocols_and_exact_bytes() {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    for contract in [SignatureContract::Timestamped, SignatureContract::BodyOnly] {
        let (binding, _) = context(Channel::Webhook);
        let private_body = br#"{"event":"connection.expired","private":"secret"}"#;
        let signing_key = Zeroizing::new(b"PRIVATE_SIGNING_KEY".to_vec());
        let client = client_builder().build().unwrap();
        let prepared = PreparedSend::signed_webhook(
            binding,
            client,
            "https://example.com/callback",
            &signing_key,
            "connection.expired",
            "PRIVATE_DELIVERY_ID",
            Zeroizing::new(private_body.to_vec()),
            contract,
            Some("PRIVATE_KEY_ID"),
        )
        .unwrap();
        let request = &prepared.request;
        let headers = request.headers();
        let mut expected = Hmac::<Sha256>::new_from_slice(signing_key.as_slice()).unwrap();
        if contract == SignatureContract::Timestamped {
            let timestamp = headers.get("X-NyxID-Timestamp").unwrap().to_str().unwrap();
            let parsed: i64 = timestamp.parse().unwrap();
            assert!((chrono::Utc::now().timestamp() - parsed).abs() <= 5);
            expected.update(timestamp.as_bytes());
            expected.update(b".");
        } else {
            assert!(!headers.contains_key("X-NyxID-Timestamp"));
        }
        expected.update(private_body);
        let signature = format!("sha256={}", hex::encode(expected.finalize().into_bytes()));
        assert_eq!(headers["X-NyxID-Signature"], signature);
        assert_eq!(headers["X-NyxID-Event"], "connection.expired");
        assert_eq!(headers["X-NyxID-Delivery-Id"], "PRIVATE_DELIVERY_ID");
        assert_eq!(headers["X-NyxID-Key-Id"], "PRIVATE_KEY_ID");
        assert_eq!(request.body().unwrap().as_bytes().unwrap(), private_body);
        for name in ["X-NyxID-Signature", "X-NyxID-Delivery-Id", "X-NyxID-Key-Id"] {
            assert!(headers[name].is_sensitive());
        }
        let debug = format!("{prepared:?}");
        for secret in ["secret", "PRIVATE", "example.com"] {
            assert!(!debug.contains(secret));
        }
    }
}

#[test]
fn sink_provider_constructors_preserve_authentication_and_recipient_payloads() {
    let empty = HashMap::new();
    for (project, token, destination) in [
        ("project/path", "PRIVATE_TOKEN", "PRIVATE_DEVICE"),
        ("project", "", "PRIVATE_DEVICE"),
        ("project", "PRIVATE_TOKEN", ""),
    ] {
        let (binding, _) = context(Channel::Fcm);
        let error = PreparedSend::fcm(
            binding,
            project,
            Zeroizing::new(token.into()),
            Zeroizing::new(destination.into()),
            PushNotification {
                alert: None,
                data: &empty,
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), unavailable().to_string());
        assert!(!error.to_string().contains("PRIVATE"));
    }
    let (binding, _) = context(Channel::Apns);
    let error = PreparedSend::apns(
        binding,
        Zeroizing::new("PRIVATE_JWT".into()),
        Zeroizing::new("PRIVATE_NOT_HEX".into()),
        "topic",
        false,
        PushNotification {
            alert: None,
            data: &empty,
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), unavailable().to_string());
    let (binding, _) = context(Channel::Telegram);
    let error = PreparedSend::telegram(
        binding,
        Zeroizing::new("PRIVATE/TOKEN".into()),
        42,
        Zeroizing::new("PRIVATE_TEXT".into()),
        None,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), unavailable().to_string());
    let data = HashMap::from([("request_id".into(), "PRIVATE_REQUEST_ID".into())]);
    let (binding, _) = context(Channel::Fcm);
    let fcm = PreparedSend::fcm(
        binding,
        "nyxid-project",
        Zeroizing::new("PRIVATE_ACCESS_TOKEN".into()),
        Zeroizing::new("PRIVATE_DEVICE_TOKEN".into()),
        PushNotification {
            alert: Some(("PRIVATE_TITLE", "PRIVATE_BODY")),
            data: &data,
        },
    )
    .unwrap();
    assert_eq!(
        fcm.request.url().as_str(),
        "https://fcm.googleapis.com/v1/projects/nyxid-project/messages:send"
    );
    assert_eq!(
        fcm.request.headers()["authorization"],
        "Bearer PRIVATE_ACCESS_TOKEN"
    );
    assert!(fcm.request.headers()["authorization"].is_sensitive());
    let payload: serde_json::Value =
        serde_json::from_slice(fcm.request.body().unwrap().as_bytes().unwrap()).unwrap();
    assert_eq!(payload["message"]["token"], "PRIVATE_DEVICE_TOKEN");
    assert_eq!(payload["message"]["notification"]["body"], "PRIVATE_BODY");
    assert_eq!(
        payload["message"]["data"]["request_id"],
        "PRIVATE_REQUEST_ID"
    );
    assert!(!format!("{fcm:?}").contains("PRIVATE"));
    let (binding, _) = context(Channel::Fcm);
    let silent = PreparedSend::fcm(
        binding,
        "nyxid-project",
        Zeroizing::new("PRIVATE_ACCESS_TOKEN".into()),
        Zeroizing::new("PRIVATE_DEVICE_TOKEN".into()),
        PushNotification {
            alert: None,
            data: &data,
        },
    )
    .unwrap();
    let payload: serde_json::Value =
        serde_json::from_slice(silent.request.body().unwrap().as_bytes().unwrap()).unwrap();
    assert!(payload["message"].get("notification").is_none());
    assert!(payload["message"].get("android").is_none());

    for (sandbox, alert) in [
        (false, None),
        (true, Some(("PRIVATE_TITLE", "PRIVATE_BODY"))),
    ] {
        let (binding, _) = context(Channel::Apns);
        let apns = PreparedSend::apns(
            binding,
            Zeroizing::new("PRIVATE_PROVIDER_JWT".into()),
            Zeroizing::new("aabbccdd".into()),
            "dev.nyxid.approvals",
            sandbox,
            PushNotification { alert, data: &data },
        )
        .unwrap();
        let expected_host = if sandbox {
            "api.sandbox.push.apple.com"
        } else {
            "api.push.apple.com"
        };
        assert_eq!(apns.request.url().host_str(), Some(expected_host));
        assert_eq!(apns.request.url().path(), "/3/device/aabbccdd");
        assert_eq!(
            apns.request.headers()["authorization"],
            "Bearer PRIVATE_PROVIDER_JWT"
        );
        assert!(apns.request.headers()["authorization"].is_sensitive());
        assert_eq!(
            apns.request.headers()["apns-push-type"],
            if alert.is_some() {
                "alert"
            } else {
                "background"
            }
        );
        let payload: serde_json::Value =
            serde_json::from_slice(apns.request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(payload["request_id"], "PRIVATE_REQUEST_ID");
        if alert.is_none() {
            assert_eq!(payload["aps"]["content-available"], 1);
            assert_eq!(apns.request.headers()["apns-expiration"], "0");
        }
        assert!(!format!("{apns:?}").contains("PRIVATE"));
    }

    let (binding, _) = context(Channel::Telegram);
    let telegram = PreparedSend::telegram(
        binding,
        Zeroizing::new("123:PRIVATE_TOKEN".into()),
        -42,
        Zeroizing::new("PRIVATE_TEXT".into()),
        None,
    )
    .unwrap();
    assert_eq!(telegram.request.url().host_str(), Some("api.telegram.org"));
    let payload: serde_json::Value =
        serde_json::from_slice(telegram.request.body().unwrap().as_bytes().unwrap()).unwrap();
    assert_eq!(payload["chat_id"], -42);
    assert_eq!(payload["text"], "PRIVATE_TEXT");
    assert!(!format!("{telegram:?}").contains("PRIVATE"));
}

#[tokio::test]
async fn sink_transport_drop_is_uncertain_without_an_http_status() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}", listener.local_addr().unwrap());
    let closed = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
    });
    let (binding, queued) = context(Channel::Webhook);
    assert!(matches!(
        prepared(binding, Channel::Webhook, &target)
            .send_once(&queued)
            .await,
        SendOutcome::Uncertain
    ));
    closed.await.unwrap();
}

#[tokio::test]
async fn sink_timeout_is_uncertain_and_never_sends_again() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let router = Router::new().route(
        "/",
        post(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<()>().await;
                "never returned"
            }
        }),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let _server = Server {
        target: target.clone(),
        calls: calls.clone(),
        task,
    };
    let client = client_builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let (binding, queued) = context(Channel::Webhook);
    let request = client.post(&target).body("PRIVATE_BODY");
    let send = PreparedSend::finish(binding, Channel::Webhook, client, request).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(3), send.send_once(&queued))
        .await
        .unwrap();
    assert!(matches!(outcome, SendOutcome::Uncertain));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sink_oversized_provider_response_never_implies_acceptance() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().route("/", post(|| async { "x".repeat(MAX_RESPONSE_BYTES + 1) }));
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let _server = Server {
        target: target.clone(),
        calls: Arc::new(AtomicUsize::new(0)),
        task,
    };
    let (binding, queued) = context(Channel::Telegram);
    assert!(matches!(
        prepared(binding, Channel::Telegram, &target)
            .send_once(&queued)
            .await,
        SendOutcome::UncertainResponse { status: 200 }
    ));
}

#[tokio::test]
async fn sink_success_preserves_only_protected_provider_receipts() {
    for (channel, response) in [
        (Channel::Webhook, "PRIVATE_RESPONSE_BODY"),
        (
            Channel::Fcm,
            r#"{"name":"projects/test/messages/PRIVATE_PROVIDER_RECEIPT"}"#,
        ),
        (Channel::Apns, ""),
    ] {
        let destination = server(StatusCode::OK, response).await;
        let (binding, queued) = context(channel);
        let outcome = prepared(binding, channel, &destination.target)
            .send_once(&queued)
            .await;
        let SendOutcome::Accepted(receipt) = outcome else {
            panic!("provider acceptance was lost")
        };
        match receipt {
            ProviderReceipt::Webhook { status } => assert_eq!(status, 200),
            ProviderReceipt::Fcm { ref message_name } => assert_eq!(
                message_name.as_deref().map(|name| name.as_str()),
                Some("projects/test/messages/PRIVATE_PROVIDER_RECEIPT")
            ),
            ProviderReceipt::Apns { ref request_id } => assert!(request_id.is_none()),
            _ => panic!("provider receipt channel changed"),
        }
        assert!(!format!("{receipt:?}").contains("PRIVATE"));
        assert_eq!(destination.calls.load(Ordering::SeqCst), 1);
    }
}
