use super::*;
use crate::services::execution_permission_dispatch::proposals::v2::connected;
use axum::routing::get;

async fn enable_observation(f: &Fixture, enabled: bool) {
    crate::services::feature_flag_admin_service::mutate(
        &f.state.db,
        connected::READINESS,
        crate::services::feature_flag_admin_service::Mutation::Override {
            kind: crate::models::feature_flag_override::FlagTargetKind::User,
            target_id: Some(f.owner.clone()),
            enabled: Some(enabled),
        },
        &crate::services::audit_service::AuditActor {
            user_id: f.owner.clone(),
            api_key_id: None,
            api_key_name: None,
            ip_address: None,
            user_agent: None,
        },
    )
    .await
    .unwrap();
}

fn observation_router(f: &Fixture) -> Router {
    Router::new()
        .route(
            "/api/v1/approvals/connected-operations/{key}/status",
            get(crate::handlers::connected_approvals::observe),
        )
        .route(
            "/api/v1/approvals/connected-operations/{key}/cancel",
            post(crate::handlers::connected_approvals::cancel),
        )
        .with_state(f.state.clone())
}

async fn observe_http(f: &Fixture, key: &str) -> (StatusCode, serde_json::Value) {
    let response = observation_router(f)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/approvals/connected-operations/{key}/status"
                ))
                .header("x-api-key", &f.raw)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn waiting(
    f: &Fixture,
    key: &str,
) -> tokio::task::JoinHandle<Result<axum::response::Response, std::convert::Infallible>> {
    let (app, mut request) = connected_request(f, "llm").await;
    start_waiter(app, &mut request, key)
}

fn start_waiter(
    app: Router,
    request: &mut Request<Body>,
    key: &str,
) -> tokio::task::JoinHandle<Result<axum::response::Response, std::convert::Infallible>> {
    request
        .headers_mut()
        .insert(connected::HEADER, format!("v1:{key}").parse().unwrap());
    let app = app.layer(axum::middleware::from_fn(
        crate::handlers::proxy::ingress::approval_wait_budget,
    ));
    tokio::spawn(app.oneshot(std::mem::take(request)))
}

async fn pending_observation(f: &Fixture, key: &str) -> serde_json::Value {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let (status, view) = observe_http(f, key).await;
            if status == StatusCode::OK {
                return view;
            }
            assert_eq!(status, StatusCode::NOT_FOUND, "{view}");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("pending receipt must be observable before the response")
}

async fn retire(
    f: &Fixture,
    key: &str,
    receipt: &serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let body =
        json!({"request_id":receipt["request_id"],"receipt_revision":receipt["receipt_revision"]});
    let response = observation_router(f)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/approvals/connected-operations/{key}/cancel"
                ))
                .header("x-api-key", &f.raw)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn connected_observation_concurrent_readers_publish_one_durable_sequence_per_content() {
    let f = Fixture::new("connected_sequence").await;
    f.enroll(true).await;
    enable_observation(&f, true).await;
    approval_timeout(&f, 30).await;
    let key = Uuid::new_v4().to_string();
    let waiter = waiting(&f, &key).await;
    let pending = pending_observation(&f, &key).await;
    assert_eq!(pending["observation_sequence"], 1);
    let original = row(&f).await;
    let mut decision = choice(&original, "operation");
    decision.approved = false;
    proposals::decide(&f.state, &f.owner, &original.id, &decision)
        .await
        .unwrap();

    // Decision replaces the typed proposal. It must preserve the durable
    // publication record while changing the receipt's decision and state.
    let decided = row(&f).await;
    assert_eq!(
        v2::proposal(&decided)
            .unwrap()
            .connected_observation
            .as_ref()
            .unwrap()
            .sequence,
        1
    );
    let responses = futures::future::join_all((0..12).map(|_| observe_http(&f, &key))).await;
    let expected = &responses[0].1;
    assert_eq!(expected["observation_sequence"], 2);
    assert_eq!(expected["decision"], "rejected");
    for (status, receipt) in &responses {
        assert_eq!(*status, StatusCode::OK, "{receipt}");
        assert_eq!(receipt, expected);
    }
    assert_eq!(retire(&f, &key, &pending).await.1, *expected);

    // Reconstruct from BSON, without a process cache or in-flight observer.
    let restored: ApprovalRequest = bson::from_document(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .find_one(doc! {"_id": &original.id})
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let replay = connected::record_observation(&f.state.db, &restored, &key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), *expected);
    let late = connected::record_observation(&f.state.db, &original, &key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(late).unwrap(), pending);
    assert!(!waiter.await.unwrap().unwrap().status().is_success());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn connected_observation_stale_unpublished_read_cannot_publish_after_retirement() {
    let f = Fixture::new("connected_stale_sequence").await;
    f.enroll(true).await;
    enable_observation(&f, true).await;
    approval_timeout(&f, 30).await;
    let key = Uuid::new_v4().to_string();
    let waiter = waiting(&f, &key).await;
    let original = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(row) = f
                .state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"execution_proposal.version": 2})
                .await
                .unwrap()
            {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Historical rows without the additive field retain the same empty state.
    let mut document = bson::to_document(&original).unwrap();
    document
        .get_document_mut("execution_proposal")
        .unwrap()
        .get_document_mut("v2")
        .unwrap()
        .remove("connected_observation");
    let original: ApprovalRequest = bson::from_document(document).unwrap();
    assert!(
        v2::proposal(&original)
            .unwrap()
            .connected_observation
            .is_none()
    );
    v2::blocking::abandon(&f.state.db, &original.id, "approval_wait_cancelled")
        .await
        .unwrap();
    assert!(
        connected::record_observation(&f.state.db, &original, &key)
            .await
            .unwrap()
            .is_none()
    );
    let (status, receipt) = observe_http(&f, &key).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(receipt["observation_sequence"], 1);
    assert_eq!(receipt["effect"], "not_sent");
    assert_eq!(observe_http(&f, &key).await.1, receipt);
    assert!(!waiter.await.unwrap().unwrap().status().is_success());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn connected_observation_invalid_or_exhausted_sequence_never_resets() {
    let f = Fixture::new("connected_sequence_exhausted").await;
    f.enroll(true).await;
    enable_observation(&f, true).await;
    approval_timeout(&f, 30).await;
    let key = Uuid::new_v4().to_string();
    let waiter = waiting(&f, &key).await;
    let pending = pending_observation(&f, &key).await;
    for sequence in [-1_i64, 0, i64::MAX] {
        f.state
            .db
            .collection::<Document>("approval_requests")
            .update_one(
                doc! {"_id": pending["request_id"].as_str().unwrap()},
                doc! {"$set": {"execution_proposal.v2.connected_observation": {
                    "sequence": sequence, "digest": "different-snapshot"
                }}},
            )
            .await
            .unwrap();
        assert_eq!(observe_http(&f, &key).await.0, StatusCode::CONFLICT);
        assert_eq!(
            v2::proposal(&row(&f).await)
                .unwrap()
                .connected_observation
                .as_ref()
                .unwrap()
                .sequence,
            sequence
        );
    }
    v2::blocking::abandon(
        &f.state.db,
        pending["request_id"].as_str().unwrap(),
        "approval_wait_cancelled",
    )
    .await
    .unwrap();
    assert!(!waiter.await.unwrap().unwrap().status().is_success());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn connected_observation_default_off_refuses_before_review_or_model_send() {
    let f = Fixture::new("connected_off").await;
    f.enroll(true).await;
    let waiter = waiting(&f, &Uuid::new_v4().to_string()).await;
    let response = waiter.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.cleanup().await;
}

#[tokio::test]
async fn connected_observation_cannot_fall_back_to_unobserved_legacy_approval() {
    let f = Fixture::new("connected_unenrolled").await;
    enable_observation(&f, true).await;
    let response = waiting(&f, &Uuid::new_v4().to_string())
        .await
        .await
        .unwrap()
        .unwrap();
    assert!(!response.status().is_success());
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn connected_observation_exposes_early_bound_receipt_without_replacing_the_stream() {
    let f = Fixture::new("connected_early").await;
    f.enroll(true).await;
    enable_observation(&f, true).await;
    approval_timeout(&f, 30).await;
    let key = Uuid::new_v4().to_string();
    let waiter = waiting(&f, &key).await;
    let observed = pending_observation(&f, &key).await;
    assert_eq!(observed["version"], 1);
    assert_eq!(observed["operation_key"], key);
    assert_eq!(observed["decision"], "pending");
    assert_eq!(observed["execution_state"], "pending");
    assert_eq!(observed["effect"], "unclaimed");
    assert_eq!(observed["observation_sequence"], 1);
    assert_eq!(
        observed["review_path"],
        crate::services::assistant_links::AssistantPage::Approvals.path()
    );
    assert!(observed.get("result").is_none());
    assert!(observed.get("choices").is_none());
    assert!(!observed.to_string().contains(&f.raw));
    assert!(!waiter.is_finished());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(observe_http(&f, &key).await.1, observed);
    let auth = f.auth().await;
    for changed in 0..3 {
        let mut wrong = auth.clone();
        match changed {
            0 => wrong.api_key_id = Some(Uuid::new_v4().to_string()),
            1 => wrong.api_key_credential_id = Some(Uuid::new_v4().to_string()),
            _ => wrong.user_id = Uuid::new_v4(),
        }
        assert!(matches!(
            connected::observe(&f.state, &wrong, &key).await,
            Err(AppError::NotFound(_))
        ));
    }
    let requested = row(&f).await;
    assert_eq!(observed["request_id"], requested.id);
    proposals::decide(
        &f.state,
        &f.owner,
        &requested.id,
        &choice(&requested, "operation"),
    )
    .await
    .unwrap();
    let response = waiter.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("data:"));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    let completed = observe_http(&f, &key).await.1;
    assert_eq!(completed["effect"], "response_received");
    assert!(completed["observation_sequence"].as_i64().unwrap() > 1);
    assert_ne!(completed["receipt_revision"], observed["receipt_revision"]);
    assert_eq!(completed["expires_at"], observed["expires_at"]);
    assert_eq!(
        retire(&f, &key, &observed).await.1["effect"],
        "response_received"
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn connected_observation_cancel_wins_and_remains_readable_with_producer_disabled() {
    let f = Fixture::new("connected_cancel").await;
    f.enroll(true).await;
    enable_observation(&f, true).await;
    approval_timeout(&f, 30).await;
    let key = Uuid::new_v4().to_string();
    let waiter = waiting(&f, &key).await;
    let observed = pending_observation(&f, &key).await;
    let original = row(&f).await;
    enable_observation(&f, false).await;
    let (status, cancelled) = retire(&f, &key, &observed).await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["effect"], "not_sent");
    assert_eq!(cancelled["observation_sequence"], 2);
    assert_eq!(cancelled["failure_code"], "approval_wait_cancelled");
    assert_eq!(cancelled["expires_at"], observed["expires_at"]);
    assert_eq!(retire(&f, &key, &observed).await.1, cancelled);
    assert_eq!(observe_http(&f, &key).await.1, cancelled);
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &original.id,
            &choice(&original, "operation")
        )
        .await
        .is_err()
    );
    assert!(!waiter.await.unwrap().unwrap().status().is_success());
    v2::sweep(&f.state).await.unwrap();
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        observe_http(&f, &Uuid::new_v4().to_string()).await.0,
        StatusCode::NOT_FOUND
    );
    f.cleanup().await;
}

#[test]
fn connected_observation_header_is_strict_and_never_requests_background_execution() {
    use axum::http::HeaderMap;
    let key = Uuid::new_v4().to_string();
    let mut headers = HeaderMap::new();
    headers.insert(connected::HEADER, format!("v1:{key}").parse().unwrap());
    assert_eq!(
        connected::take_operation(&mut headers).unwrap(),
        Some(key.clone())
    );
    assert!(!headers.contains_key(connected::HEADER));
    for value in [
        "v2:00000000-0000-4000-8000-000000000001",
        "v1:invalid",
        "v1:00000000-0000-0000-0000-000000000001",
    ] {
        headers.insert(connected::HEADER, value.parse().unwrap());
        assert!(connected::take_operation(&mut headers).is_err());
    }
    headers.insert(connected::HEADER, format!("v1:{key}").parse().unwrap());
    headers.insert("x-nyxid-approval-mode", "async".parse().unwrap());
    assert!(connected::take_operation(&mut headers).is_err());
    headers.remove("x-nyxid-approval-mode");
    headers.append(connected::HEADER, format!("v1:{key}").parse().unwrap());
    assert!(connected::take_operation(&mut headers).is_err());
}

#[tokio::test]
async fn connected_observation_retry_cannot_replace_or_restart_a_retired_operation() {
    let f = Fixture::new("connected_retry").await;
    f.enroll(true).await;
    enable_observation(&f, true).await;
    approval_timeout(&f, 30).await;
    let key = Uuid::new_v4().to_string();
    let (app, mut request) = connected_request(&f, "llm").await;
    let waiter = start_waiter(app.clone(), &mut request, &key);
    let observed = pending_observation(&f, &key).await;
    for content in ["hello", "different model request"] {
        let mut retry = Request::builder()
            .method("POST")
            .uri("/llm/openai/v1/chat/completions")
            .header("x-api-key", &f.raw)
            .header("content-type", "application/json")
            .body(Body::from(json!({"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":content}]}).to_string()))
            .unwrap();
        let response = start_waiter(app.clone(), &mut retry, &key)
            .await
            .unwrap()
            .unwrap();
        assert!(!response.status().is_success());
        assert_eq!(observe_http(&f, &key).await.1, observed);
    }
    let retired = retire(&f, &key, &observed).await.1;
    assert_eq!(retired["effect"], "not_sent");
    assert!(!waiter.await.unwrap().unwrap().status().is_success());
    let mut retry = Request::builder()
        .method("POST")
        .uri("/llm/openai/v1/chat/completions")
        .header("x-api-key", &f.raw)
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hello"}]})
                .to_string(),
        ))
        .unwrap();
    assert!(
        !start_waiter(app, &mut retry, &key)
            .await
            .unwrap()
            .unwrap()
            .status()
            .is_success()
    );
    assert_eq!(observe_http(&f, &key).await.1, retired);
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn connected_observation_timeout_and_disconnect_keep_original_identity_and_deadline() {
    for disconnect in [false, true] {
        let f = Fixture::new("connected_retirement").await;
        f.enroll(true).await;
        enable_observation(&f, true).await;
        approval_timeout(&f, if disconnect { 30 } else { 1 }).await;
        let key = Uuid::new_v4().to_string();
        let waiter = waiting(&f, &key).await;
        let observed = pending_observation(&f, &key).await;
        if disconnect {
            waiter.abort();
            assert!(waiter.await.unwrap_err().is_cancelled());
        } else {
            assert!(!waiter.await.unwrap().unwrap().status().is_success());
        }
        let retired = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let (_, receipt) = observe_http(&f, &key).await;
                if receipt["effect"] == "not_sent" {
                    break receipt;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(retired["request_id"], observed["request_id"]);
        assert_eq!(retired["expires_at"], observed["expires_at"]);
        assert_eq!(
            retired["failure_code"],
            if disconnect {
                "approval_wait_disconnected"
            } else {
                "approval_wait_timed_out"
            }
        );
        // A fresh handler observes the durable outcome without the original waiter.
        assert_eq!(observe_http(&f, &key).await.1, retired);
        v2::sweep(&f.state).await.unwrap();
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[tokio::test]
async fn connected_observation_cancel_revalidates_authority_inside_its_transaction() {
    let f = Fixture::new("connected_authority").await;
    f.enroll(true).await;
    enable_observation(&f, true).await;
    approval_timeout(&f, 30).await;
    let key = Uuid::new_v4().to_string();
    let waiter = waiting(&f, &key).await;
    let observed = pending_observation(&f, &key).await;
    let original = row(&f).await;
    let snapshot = authority::Snapshot::load(&f.state.db, &f.connection, &f.key, None)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(doc! {"_id":&f.key}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        v2::blocking::abandon_observed(
            &f.state.db,
            &original,
            &snapshot,
            "approval_wait_cancelled"
        )
        .await
        .is_err()
    );
    assert_eq!(
        row(&f).await.status,
        "pending",
        "refusal must roll back cancellation"
    );
    assert!(!retire(&f, &key, &observed).await.0.is_success());
    assert!(!observe_http(&f, &key).await.0.is_success());
    waiter.abort();
    let _ = waiter.await;
    v2::blocking::abandon(&f.state.db, &original.id, "approval_wait_disconnected")
        .await
        .unwrap();
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn connected_observation_stale_cancel_cannot_overwrite_an_approval_or_denial() {
    for approved in [false, true] {
        let f = Fixture::new("connected_decision").await;
        f.enroll(true).await;
        enable_observation(&f, true).await;
        approval_timeout(&f, 30).await;
        let key = Uuid::new_v4().to_string();
        let waiter = waiting(&f, &key).await;
        let observed = pending_observation(&f, &key).await;
        let original = row(&f).await;
        let mut decision = choice(&original, "operation");
        decision.approved = approved;
        proposals::decide(&f.state, &f.owner, &original.id, &decision)
            .await
            .unwrap();
        let (status, receipt) = retire(&f, &key, &observed).await;
        if approved {
            // The stale revision can conflict or read the later dispatch state,
            // but cannot certify that the approved occurrence was not sent.
            assert!(
                status == StatusCode::CONFLICT
                    || receipt["effect"] == "dispatch_claimed"
                    || receipt["effect"] == "response_received"
            );
            let response = waiter.await.unwrap().unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let _ = axum::body::to_bytes(response.into_body(), 16384)
                .await
                .unwrap();
            assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        } else {
            assert_eq!(status, StatusCode::OK);
            assert_eq!(receipt["decision"], "rejected");
            assert_eq!(receipt["effect"], "not_sent");
            assert!(!waiter.await.unwrap().unwrap().status().is_success());
            assert!(f.server.received_requests().await.unwrap().is_empty());
        }
        f.cleanup().await;
    }
}

#[tokio::test]
async fn connected_observation_dispatch_winner_cannot_be_certified_not_sent() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = Fixture::new("connected_dispatched").await;
    f.enroll(true).await;
    enable_observation(&f, true).await;
    approval_timeout(&f, 30).await;
    let (app, mut request) = connected_request(&f, "llm").await;
    let received = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let count = Arc::new(AtomicUsize::new(0));
    let upstream = Router::new().fallback({
        let received = received.clone();
        let release = release.clone();
        let count = count.clone();
        move || {
            let received = received.clone();
            let release = release.clone();
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                received.notify_one();
                release.notified().await;
                ([("content-type", "text/event-stream")], "data: [DONE]\n\n")
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = format!("http://{}", listener.local_addr().unwrap());
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&f.catalog},
            doc! {"$set":{"base_url":&destination}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_many(
            doc! {"user_id":&f.owner},
            doc! {"$set":{"url":&destination}},
        )
        .await
        .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let key = Uuid::new_v4().to_string();
    let waiter = start_waiter(app, &mut request, &key);
    let observed = pending_observation(&f, &key).await;
    let original = row(&f).await;
    proposals::decide(
        &f.state,
        &f.owner,
        &original.id,
        &choice(&original, "operation"),
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), received.notified())
        .await
        .unwrap();
    let (_, dispatched) = retire(&f, &key, &observed).await;
    assert_eq!(dispatched["effect"], "dispatch_claimed");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(!waiter.is_finished());
    release.notify_one();
    let response = waiter.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    assert_eq!(
        observe_http(&f, &key).await.1["effect"],
        "response_received"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    server.abort();
    let _ = server.await;
    f.cleanup().await;
}
