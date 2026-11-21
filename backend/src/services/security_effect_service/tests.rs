use std::sync::{Arc, Mutex};

use super::*;
use crate::models::audit_log::AuditLog;
use crate::services::audit_service;

async fn fixture(name: &str) -> Database {
    let db = crate::test_utils::connect_transaction_test_database(name).await;
    ensure_indexes(&db).await.unwrap();
    db.collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"seq":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name("audit_log_seq_unique".to_owned())
                        .unique(true)
                        .partial_filter_expression(doc! {"seq":{"$exists":true}})
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    db
}

fn occurrence() -> SecurityEffectIntent {
    audit_service::required::intent(
        None,
        "fixture.authority_committed",
        Some(serde_json::json!({
            "resource_id": "fixture", "active": false,
        })),
    )
    .unwrap()
}

#[tokio::test]
async fn append_before_receipt_crash_and_concurrent_replay_chain_exactly_once() {
    let db = fixture("required_audit_replay").await;
    let pending = occurrence();
    enqueue(&db, &pending).await.unwrap();
    let first = claim(&db, &pending).await.unwrap().unwrap();
    let SecurityEffect::Audit { event } = &first.effect else {
        panic!("audit")
    };
    // Simulate append committed, process died before the intent receipt.
    let (left, right) = tokio::join!(
        audit_service::append_intent(&db, event),
        audit_service::append_intent(&db, event)
    );
    left.unwrap();
    right.unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &first.id},
            doc! {"$set":{"next_attempt_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let state = crate::test_utils::test_app_state(db.clone());
    let (left, right) = tokio::join!(
        sweep(&db, &state.encryption_keys),
        sweep(&db, &state.encryption_keys)
    );
    left.unwrap();
    right.unwrap();
    assert!(
        db.collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": &first.id})
            .await
            .unwrap()
            .is_none()
    );
    // A delayed former holder can append only the same immutable occurrence.
    audit_service::append_intent(&db, event).await.unwrap();
    record_outcome(&db, &first, Outcome::Retry("late_worker"))
        .await
        .unwrap();
    assert!(
        db.collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": &first.id})
            .await
            .unwrap()
            .is_none()
    );

    assert_eq!(
        db.collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let report = crate::services::audit_chain_service::verify_chain(
        &db,
        audit_service::audit_chain_hmac_key().unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        report.status,
        crate::services::audit_chain_service::AuditChainStatus::Ok
    );
    assert_eq!(report.head_seq, Some(1));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn aborted_source_transaction_cannot_publish_an_intent() {
    let db = fixture("required_audit_abort").await;
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    db.collection::<Document>("source")
        .insert_one(doc! {"_id":"change"})
        .session(&mut session)
        .await
        .unwrap();
    enqueue_in_session(&db, &mut session, &[occurrence()])
        .await
        .unwrap();
    session.abort_transaction().await.unwrap();
    assert_eq!(
        db.collection::<Document>("source")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn idle_pass_is_one_read_and_terminal_receipts_are_not_retried() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(Mutex::new(Vec::new()));
    let capture = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "security_effect_idle",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                capture.lock().unwrap().push(event.command_name);
            }
        }),
    )
    .await;
    ensure_indexes(&db).await.unwrap();
    for state in [
        IntentState::Delivered,
        IntentState::Failed,
        IntentState::Uncertain,
    ] {
        let mut row = occurrence();
        row.state = state;
        enqueue(&db, &row).await.unwrap();
    }
    let app = crate::test_utils::test_app_state(db.clone());
    commands.lock().unwrap().clear();
    sweep(&db, &app.encryption_keys).await.unwrap();
    assert_eq!(*commands.lock().unwrap(), ["find"]);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn retries_back_off_but_required_audits_are_never_abandoned() {
    let db = fixture("security_effect_backoff").await;
    let mut row = occurrence();
    row.attempts = CAE_MAX_ATTEMPTS + 20;
    enqueue(&db, &row).await.unwrap();
    let claimed = claim(&db, &row).await.unwrap().unwrap();
    record_outcome(&db, &claimed, Outcome::Retry("fixture_failure"))
        .await
        .unwrap();
    let saved = db
        .collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.state, IntentState::Pending);
    assert!(claim(&db, &saved).await.unwrap().is_none());
    let next = db
        .collection::<Document>(COLLECTION_NAME)
        .aggregate([
            doc! {"$match":{"_id": &row.id}},
            doc! {"$project":{"delay":{"$subtract":["$next_attempt_at","$$NOW"]}}},
        ])
        .await
        .unwrap()
        .try_next()
        .await
        .unwrap()
        .unwrap();
    let delay = next.get_i64("delay").unwrap();
    assert!((3_590_000..=3_600_000).contains(&delay));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lost_claim_cannot_dispatch_or_change_a_successors_receipt() {
    let db = fixture("security_effect_lost_claim").await;
    let row = occurrence();
    enqueue(&db, &row).await.unwrap();
    let old = claim(&db, &row).await.unwrap().unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set":{"next_attempt_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let successor = claim(&db, &row).await.unwrap().unwrap();
    let before = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap();
    assert!(!begin_send(&db, &old).await.unwrap());
    record_outcome(&db, &old, Outcome::Delivered).await.unwrap();
    let after = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_ne!(old.claim_token, successor.claim_token);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn dispatch_without_receipt_is_uncertain_and_never_reclaimed() {
    let db = fixture("security_effect_uncertain").await;
    let row = new_intent(
        Uuid::new_v4().to_string(),
        SecurityEffect::Cae {
            binding_id: "fixture".into(),
            client_id: "client".into(),
            revoke_source: "user".into(),
        },
    );
    enqueue(&db, &row).await.unwrap();
    let claimed = claim(&db, &row).await.unwrap().unwrap();
    assert!(begin_send(&db, &claimed).await.unwrap());
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set":{"next_attempt_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let app = crate::test_utils::test_app_state(db.clone());
    sweep(&db, &app.encryption_keys).await.unwrap();
    let after = db
        .collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.state, IntentState::Uncertain);
    assert_eq!(after.outcome.as_deref(), Some("dispatch_receipt_missing"));
    assert!(claim(&db, &after).await.unwrap().is_none());
    record_outcome(&db, &claimed, Outcome::Delivered)
        .await
        .unwrap();
    let final_row = db
        .collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_row.state, IntentState::Uncertain);
    db.drop().await.unwrap();
}

async fn cae_occurrence(
    db: &Database,
    app: &crate::AppState,
    address: std::net::SocketAddr,
) -> SecurityEffectIntent {
    db.collection::<Document>("oauth_broker_bindings")
        .insert_one(doc! {
            "_id":"binding-fixture","client_id":"client-fixture","user_id":"user-fixture",
            "refresh_token_jti":"refresh-fixture","revoked":true,"revoke_reason":"user_revoked",
            "revoked_at":bson::DateTime::from_millis(1_700_000_000_000),
            "created_at":bson::DateTime::from_millis(1_600_000_000_000),
        })
        .await
        .unwrap();
    let encrypted = app
        .encryption_keys
        .encrypt(b"fixture-webhook-secret")
        .await
        .unwrap();
    db.collection::<Document>("oauth_clients").insert_one(doc! {
        "_id":"client-fixture","client_name":"Fixture","client_secret_hash":"fixture-hash",
        "redirect_uris":[],"allowed_scopes":"openid","grant_types":"authorization_code",
        "client_type":"confidential","is_active":true,"created_at":bson::DateTime::now(),
        "updated_at":bson::DateTime::now(),"revocation_webhook_url":format!("http://{address}/hook"),
        "revocation_webhook_secret_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
    }).await.unwrap();
    let row = new_intent(
        Uuid::new_v4().to_string(),
        SecurityEffect::Cae {
            binding_id: "binding-fixture".into(),
            client_id: "client-fixture".into(),
            revoke_source: "user".into(),
        },
    );
    enqueue(db, &row).await.unwrap();
    let stored = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    for field in ["body", "secret", "ciphertext", "token", "reason"] {
        assert!(!stored.contains_key(field));
    }
    assert!(!stored.to_string().contains("fixture-webhook-secret"));
    row
}

async fn expire_claim(db: &Database, row: &SecurityEffectIntent) {
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"next_attempt_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
}

async fn saved_intent(db: &Database, row: &SecurityEffectIntent) -> SecurityEffectIntent {
    db.collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn cae_unsent_claim_replays_stable_metadata_and_definite_rejection_backs_off() {
    use axum::{
        Router,
        body::Bytes,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    let captures = Arc::new(Mutex::new(Vec::new()));
    let capture = captures.clone();
    let router = Router::new().route(
        "/hook",
        post(move |headers: HeaderMap, body: Bytes| {
            let capture = capture.clone();
            async move {
                let mut records = capture.lock().unwrap();
                records.push((headers, body));
                if records.len() == 1 {
                    StatusCode::TOO_MANY_REQUESTS
                } else {
                    StatusCode::NO_CONTENT
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let _delivery_origin =
        crate::services::resource_adapter::register_test_origin(&format!("http://{address}"));
    let server = crate::services::shutdown::process()
        .spawn(async move { axum::serve(listener, router).await.unwrap() });
    let db = fixture("cae_retry_wire").await;
    let app = crate::test_utils::test_app_state(db.clone());
    let row = cae_occurrence(&db, &app, address).await;
    let _abandoned = claim(&db, &row).await.unwrap().unwrap();
    expire_claim(&db, &row).await;
    super::super::webhook_delivery_service::with_test_webhook_target(address, async {
        sweep(&db, &app.encryption_keys).await.unwrap();
        assert_eq!(saved_intent(&db, &row).await.state, IntentState::Pending);
        sweep(&db, &app.encryption_keys).await.unwrap();
        assert_eq!(
            captures.lock().unwrap().len(),
            1,
            "backoff must not resend inline"
        );
        expire_claim(&db, &row).await;
        sweep(&db, &app.encryption_keys).await.unwrap();
        assert_eq!(saved_intent(&db, &row).await.state, IntentState::Delivered);
        sweep(&db, &app.encryption_keys).await.unwrap();
    })
    .await;
    {
        let records = captures.lock().unwrap();
        assert_eq!(records.len(), 2);
        for (headers, body) in records.iter() {
            assert_eq!(headers["X-NyxID-Delivery-Id"], row.id.as_str());
            let expected = format!(
                "sha256={}",
                crate::services::webhook_delivery_service::compute_body_signature(
                    b"fixture-webhook-secret",
                    body
                )
            );
            assert_eq!(headers["X-NyxID-Signature"], expected.as_str());
            let event: serde_json::Value = serde_json::from_slice(body).unwrap();
            assert_eq!(event["binding_hash"], "binding-fixture");
            assert_eq!(event["reason"], "user_revoked");
            assert_eq!(event["revoked_at"], "2023-11-14T22:13:20Z");
        }
        assert_eq!(records[0].1, records[1].1);
    }
    server.abort();
    let _ = server.await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cae_crash_after_receiver_acceptance_never_blindly_resends() {
    use axum::{Router, http::StatusCode, routing::post};
    let accepted = Arc::new(tokio::sync::Notify::new());
    let sends = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let signal = accepted.clone();
    let count = sends.clone();
    let router = Router::new().route(
        "/hook",
        post(move || {
            let signal = signal.clone();
            let count = count.clone();
            async move {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                signal.notify_one();
                std::future::pending::<()>().await;
                StatusCode::NO_CONTENT
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let _delivery_origin =
        crate::services::resource_adapter::register_test_origin(&format!("http://{address}"));
    let server = crate::services::shutdown::process()
        .spawn(async move { axum::serve(listener, router).await.unwrap() });
    let db = fixture("cae_crash_wire").await;
    let app = crate::test_utils::test_app_state(db.clone());
    let row = cae_occurrence(&db, &app, address).await;
    crate::services::webhook_delivery_service::with_test_webhook_target(address, async {
        let mut pass = Box::pin(sweep(&db, &app.encryption_keys));
        tokio::select! {
            _=accepted.notified()=>{},
            result=&mut pass=>panic!("send returned before acceptance barrier: {result:?}"),
        }
        assert_eq!(saved_intent(&db, &row).await.state, IntentState::Sending);
        drop(pass);
        expire_claim(&db, &row).await;
        sweep(&db, &app.encryption_keys).await.unwrap();
        sweep(&db, &app.encryption_keys).await.unwrap();
    })
    .await;
    assert_eq!(saved_intent(&db, &row).await.state, IntentState::Uncertain);
    assert_eq!(sends.load(std::sync::atomic::Ordering::SeqCst), 1);
    server.abort();
    let _ = server.await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cae_pre_dispatch_loser_sends_nothing_and_preserves_successor() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let _delivery_origin =
        crate::services::resource_adapter::register_test_origin(&format!("http://{address}"));
    let db = fixture("cae_dispatch_loser").await;
    let app = crate::test_utils::test_app_state(db.clone());
    let row = cae_occurrence(&db, &app, address).await;
    let old = claim(&db, &row).await.unwrap().unwrap();
    expire_claim(&db, &row).await;
    let _successor = claim(&db, &row).await.unwrap().unwrap();
    let before = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap();
    let outcome = crate::services::webhook_delivery_service::with_test_webhook_target(
        address,
        crate::services::cae_webhook_service::deliver_intent(&db, &app.encryption_keys, &old),
    )
    .await
    .unwrap();
    assert!(matches!(outcome, Outcome::LostClaim));
    record_outcome(&db, &old, outcome).await.unwrap();
    assert_eq!(
        before,
        db.collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn drain_stops_new_claims_but_does_not_revoke_an_admitted_claim() {
    let db = fixture("effect_drain").await;
    let app = crate::test_utils::test_app_state(db.clone());
    let runtime = crate::services::shutdown::Runtime::default();
    let first = occurrence();
    enqueue(&db, &first).await.unwrap();
    let admitted = runtime.background().unwrap();
    let owned = claim(&db, &first).await.unwrap().unwrap();
    let second = occurrence();
    enqueue(&db, &second).await.unwrap();
    runtime.begin_drain();
    sweep_with_runtime(&db, &app.encryption_keys, &runtime)
        .await
        .unwrap();
    assert_eq!(saved_intent(&db, &second).await.state, IntentState::Pending);
    let SecurityEffect::Audit { event } = &owned.effect else {
        panic!("audit")
    };
    audit_service::append_intent(&db, event).await.unwrap();
    record_outcome(&db, &owned, Outcome::Delivered)
        .await
        .unwrap();
    drop(admitted);
    assert!(
        db.collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": &first.id})
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cae_send_barrier_outcomes_are_terminal_or_bounded_safe_retries() {
    for (status, expected) in [
        (204, IntentState::Delivered),
        (400, IntentState::Failed),
        (500, IntentState::Uncertain),
        (429, IntentState::Failed),
    ] {
        let server = wiremock::MockServer::start().await;
        let _delivery_origin =
            crate::services::resource_adapter::register_test_origin(&server.uri());
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let db = fixture("cae_outcomes").await;
        let app = crate::test_utils::test_app_state(db.clone());
        let row = cae_occurrence(&db, &app, *server.address()).await;
        if status == 429 {
            db.collection::<Document>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"attempts":CAE_MAX_ATTEMPTS-1}},
                )
                .await
                .unwrap();
        }
        crate::services::webhook_delivery_service::with_test_webhook_target(
            *server.address(),
            async {
                sweep(&db, &app.encryption_keys).await.unwrap();
                sweep(&db, &app.encryption_keys).await.unwrap();
            },
        )
        .await;
        assert_eq!(saved_intent(&db, &row).await.state, expected);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        db.drop().await.unwrap();
    }
}

#[test]
fn required_metadata_is_bounded_before_a_source_transaction_starts() {
    assert!(
        audit_service::required::intent(
            None,
            "fixture",
            Some(serde_json::json!({
                "resource_id":"x".repeat(17*1024)
            }))
        )
        .is_err()
    );
}

#[tokio::test]
async fn claim_expiry_uses_database_time_despite_producer_clock_skew() {
    let db = fixture("effect_server_clock").await;
    let mut row = occurrence();
    row.created_at = Utc::now() + chrono::Duration::days(30);
    enqueue(&db, &row).await.unwrap();
    let owned = claim(&db, &row).await.unwrap().unwrap();
    let projection = db
        .collection::<Document>(COLLECTION_NAME)
        .aggregate([
            doc! {"$match":{"_id":&row.id}},
            doc! {"$project":{"remaining":{"$subtract":["$next_attempt_at","$$NOW"]}}},
        ])
        .await
        .unwrap()
        .try_next()
        .await
        .unwrap()
        .unwrap();
    assert!((50_000..=60_000).contains(&projection.get_i64("remaining").unwrap()));
    assert!(begin_send(&db, &owned).await.unwrap());
    db.drop().await.unwrap();
}

mod review_tests;
