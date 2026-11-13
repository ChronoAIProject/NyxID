use super::proposals::{self, Decision};
use super::*;
use crate::models::{
    approval_request::ApprovalRequest,
    assistant_conversation::{AssistantConversation, TurnOrigin},
    assistant_settings::AccessMode,
};
use crate::services::{
    assistant_agent_credential_service as credentials, assistant_nyxagent as engine,
    assistant_settings_service,
};

pub(in crate::services::execution_permission_dispatch) async fn assistant(
    name: &str,
) -> (Fixture, AssistantConversation) {
    assistant_for_cohort(name, false).await
}
pub(in crate::services::execution_permission_dispatch) async fn assistant_for_cohort(
    name: &str,
    global: bool,
) -> (Fixture, AssistantConversation) {
    let mut f = Fixture::new(name).await;
    super::super::phase_b_tests::enroll_cohort(&f.state.db, &f.owner, global).await;
    assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        assistant_settings_service::Update {
            access_mode: Some(AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let request = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: Vec::new(),
        agent_id: None,
        conversation_id: None,
        text: "Check the status".into(),
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&request).into();
    start.execution_enrollment = true;
    let row = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    f.key = row.credential_api_key_id.clone();
    f.raw = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap()
    .raw_key
    .to_string();
    assert!(row.active_turn.as_ref().unwrap().execution_task.is_some());
    (f, row)
}
async fn write_call(f: &Fixture) -> (StatusCode, String) {
    let response = f
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/proxy/s/execution-fixture/status")
                .header("x-api-key", &f.raw)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"Note"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}
async fn proposal(f: &Fixture) -> ApprovalRequest {
    f.state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"execution_proposal.version":{"$in":[1,3]}})
        .await
        .unwrap()
        .unwrap()
}
fn decision(row: &ApprovalRequest, choice: &str) -> Decision {
    let p = proposals::proposal(row).unwrap();
    let c = p.choices.iter().find(|c| c.id == choice).unwrap();
    Decision {
        approved: true,
        expected_revision: p.revision,
        choice_id: Some(c.id.clone()),
        choice_digest: Some(c.digest.clone()),
        idempotency_key: Uuid::new_v4().to_string(),
    }
}
async fn finish(f: &Fixture, row: &AssistantConversation, text: &str) {
    engine::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: text.into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
}
async fn event_turn(f: &Fixture, row: &AssistantConversation) -> AssistantConversation {
    let request = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: Vec::new(),
        agent_id: None,
        conversation_id: Some(row.id.clone()),
        text: String::new(),
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&request).into();
    start.origin = TurnOrigin::Event;
    engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap()
}
async fn resume(f: &Fixture, row: &AssistantConversation) -> String {
    let egress = crate::handlers::assistant_nyxagent::http_egress(
        f.state.clone(),
        test_utils::test_auth_user(&f.owner),
    );
    proposals::resume_turn(&f.state, egress.as_ref(), row)
        .await
        .unwrap()
        .unwrap()
}
async fn count(f: &Fixture, collection: &str) -> u64 {
    f.state
        .db
        .collection::<Document>(collection)
        .count_documents(doc! {})
        .await
        .unwrap()
}

#[tokio::test]
async fn activated_proposal_create_and_decide_serve_overlap_and_reject_unsupported() {
    use crate::services::security_readiness_service as readiness;
    let (f, _) = assistant("proposal_roster_overlap").await;
    let sibling = readiness::overlapping_test_replica(&f.state.db).await;
    let first = write_call(&f).await;
    assert_ne!(first.0, StatusCode::SERVICE_UNAVAILABLE, "{}", first.1);
    let request = proposal(&f).await;
    let input = decision(&request, "persistent_private_conversations");
    let approved = proposals::decide(&f.state, &f.owner, &request.id, &input)
        .await
        .unwrap();
    assert_eq!(approved.status, "approved");
    assert_eq!(count(&f, "approval_grants").await, 1);
    readiness::remove_test_replica_capability(&f.state.db, &sibling, proposals::READINESS).await;
    assert!(matches!(
        proposals::decide(&f.state, &f.owner, &request.id, &input).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
    let (f, _) = assistant("proposal_unsupported_overlap").await;
    let sibling = readiness::overlapping_test_replica(&f.state.db).await;
    readiness::remove_test_replica_capability(&f.state.db, &sibling, proposals::READINESS).await;
    let refused = write_call(&f).await;
    assert_eq!(refused.0, StatusCode::SERVICE_UNAVAILABLE, "{}", refused.1);
    assert_eq!(count(&f, "approval_requests").await, 0);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_proxy_card_human_decision_resume_and_receipt_are_exact() {
    for global in [false, true] {
        let (f, row) = assistant_for_cohort("execution_proposal_proxy", global).await;
        let first = write_call(&f).await;
        assert_ne!(first.0, StatusCode::OK, "{}", first.1);
        let request = proposal(&f).await;
        let p = proposals::proposal(&request).unwrap();
        assert_eq!(
            p.choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            [
                "operation",
                "task",
                "window",
                "persistent",
                "persistent_private_conversations"
            ]
        );
        assert!(
            p.contract
                .satisfies
                .contains(&ExecutionFloor::AccessModeRisk)
        );
        assert!(
            p.contract
                .satisfies
                .contains(&ExecutionFloor::PersonalServiceApproval)
        );
        assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
        assert!(f.server.received_requests().await.unwrap().is_empty());
        for _ in 0..3 {
            let _ = write_call(&f).await;
        }
        assert_eq!(count(&f, "approval_requests").await, 1);
        let input = decision(&request, "operation");
        assert!(
            proposals::decide(&f.state, &Uuid::new_v4().to_string(), &request.id, &input)
                .await
                .is_err()
        );
        let key_auth = f.auth().await;
        assert!(
            crate::handlers::approvals::decide_execution_request(
                axum::extract::State(f.state.clone()),
                key_auth,
                axum::extract::Path(request.id.clone()),
                axum::Json(decision(&request, "operation"))
            )
            .await
            .is_err()
        );
        let approved = proposals::decide(&f.state, &f.owner, &request.id, &input)
            .await
            .unwrap();
        assert_eq!(approved.status, "approved");
        let replay = proposals::decide(&f.state, &f.owner, &request.id, &input)
            .await
            .unwrap();
        assert_eq!(
            proposals::proposal(&approved).unwrap().grant_id,
            proposals::proposal(&replay).unwrap().grant_id
        );
        assert_eq!(count(&f, "approval_grants").await, 1);
        let grant_id = proposals::proposal(&approved)
            .unwrap()
            .grant_id
            .clone()
            .unwrap();
        let key_view = crate::handlers::approvals::get_grant_authorization(
            axum::extract::State(f.state.clone()),
            f.auth().await,
            axum::extract::Path(grant_id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert!(key_view.execution.unwrap().review.is_none());
        let human_view = crate::handlers::approvals::get_grant_authorization(
            axum::extract::State(f.state.clone()),
            test_utils::test_auth_user(&f.owner),
            axum::extract::Path(grant_id),
        )
        .await
        .unwrap()
        .0;
        assert!(human_view.execution.unwrap().review.is_some());
        finish(&f, &row, "Waiting for your permission").await;
        let resumed = event_turn(&f, &row).await;
        let text = resume(&f, &resumed).await;
        assert!(text.contains("HTTP 200"), "{text}");
        assert!(text.contains("ok"));
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert_eq!(resume(&f, &resumed).await, text);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            proposals::proposal(&proposal(&f).await).unwrap().state,
            "completed"
        );
        assert_eq!(
            proposal(&f).await.activity_outcome.as_deref(),
            Some("completed")
        );
        finish(&f, &resumed, &text).await;
        assert!(
            proposals::proposal(&proposal(&f).await)
                .unwrap()
                .delivered_turn_id
                .is_some()
        );
        f.cleanup().await;
    }
}

#[tokio::test]
async fn execution_proposal_twenty_duplicate_decisions_and_resumes_send_once_even_persistent() {
    let (f, row) = assistant("execution_proposal_duplicates").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    let input = decision(&request, "persistent");
    let results = futures::future::join_all(
        (0..20).map(|_| proposals::decide(&f.state, &f.owner, &request.id, &input)),
    )
    .await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(count(&f, "approval_grants").await, 1);
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    assert!(
        f.server.received_requests().await.unwrap().is_empty(),
        "approved original cannot send through a model retry"
    );
    finish(&f, &row, "Waiting").await;
    let resumed = event_turn(&f, &row).await;
    let egress = crate::handlers::assistant_nyxagent::http_egress(
        f.state.clone(),
        test_utils::test_auth_user(&f.owner),
    );
    let results = futures::future::join_all(
        (0..20).map(|_| proposals::resume_turn(&f.state, egress.as_ref(), &resumed)),
    )
    .await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_denial_expiry_legacy_guards_and_changed_choice_never_send() {
    let (f, _row) = assistant("execution_proposal_denied").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    assert!(
        crate::services::approval_service::process_decision(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            None,
            None,
            &request.id,
            true,
            None,
            None,
            "web"
        )
        .await
        .is_err()
    );
    assert!(
        crate::services::approval_service::wait_for_decision(&f.state.db, &request.id, 1)
            .await
            .is_err()
    );
    let mut bad = decision(&request, "operation");
    bad.choice_digest = Some("changed".into());
    assert!(
        proposals::decide(&f.state, &f.owner, &request.id, &bad)
            .await
            .is_err()
    );
    assert_eq!(count(&f, "approval_grants").await, 0);
    let denied = Decision {
        approved: false,
        expected_revision: 1,
        choice_id: None,
        choice_digest: None,
        idempotency_key: Uuid::new_v4().to_string(),
    };
    proposals::decide(&f.state, &f.owner, &request.id, &denied)
        .await
        .unwrap();
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(count(&f, "approval_requests").await, 1);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        proposals::proposal(&proposal(&f).await)
            .unwrap()
            .payload
            .is_none()
    );
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "persistent")
        )
        .await
        .is_err()
    );
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_authority_drift_and_task_cancellation_roll_back_decision() {
    let (f, row) = assistant("execution_proposal_fences").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&f.connection},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation")
        )
        .await
        .is_err()
    );
    assert_eq!(count(&f, "approval_grants").await, 0);
    assert_eq!(proposal(&f).await.status, "expired");
    tasks::stop(&f.state.db, &row).await.unwrap();
    assert!(
        proposals::proposal(&proposal(&f).await)
            .unwrap()
            .payload
            .is_none()
    );
    assert!(
        proposals::decide(&f.state, &f.owner, &request.id, &decision(&request, "task"))
            .await
            .is_err()
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_crash_after_barrier_reports_unknown_and_never_retries() {
    let (f, row) = assistant("execution_proposal_unknown").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "window"),
    )
    .await
    .unwrap();
    finish(&f, &row, "Waiting").await;
    let resumed = event_turn(&f, &row).await;
    f.state
        .db
        .collection::<Document>("approval_requests")
        .update_one(
            doc! {"_id":&request.id},
            proposals::v3::AssistantVersion::of(&request)
                .unwrap()
                .update(doc! {"$set":{"execution_proposal.v1.state":"dispatched"}}),
        )
        .await
        .unwrap();
    assert!(resume(&f, &resumed).await.contains("unknown"));
    assert!(resume(&f, &resumed).await.contains("unknown"));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn migration_permission_commit_lost_response_restart_keeps_original_effect() {
    let (mut f, row) = assistant("migration_commit_restart").await;
    let _ = write_call(&f).await;
    let request = proposal(&f).await;
    let input = decision(&request, "persistent");
    let effect = proposals::proposal(&request).unwrap().effect_id.clone();
    // The commit succeeded but the client never received the decision response.
    drop(
        proposals::decide(&f.state, &f.owner, &request.id, &input)
            .await
            .unwrap(),
    );
    finish(&f, &row, "Waiting").await;
    f.state = test_utils::test_app_state(f.state.db.clone());
    proposals::decide(&f.state, &f.owner, &request.id, &input)
        .await
        .unwrap();
    assert_eq!(count(&f, "approval_grants").await, 1);
    let (first, second) = tokio::join!(proposals::sweep(&f.state), proposals::sweep(&f.state));
    first.unwrap();
    second.unwrap();
    let resumed = event_turn(&f, &row).await;
    assert!(resume(&f, &resumed).await.contains("HTTP 200"));
    assert!(resume(&f, &resumed).await.contains("HTTP 200"));
    assert_eq!(
        proposals::proposal(&proposal(&f).await).unwrap().effect_id,
        effect
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
    f.cleanup().await;
}

#[tokio::test]
async fn migration_provider_accepts_then_drops_response_never_repeats_write() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncReadExt;
    let (f, row) = assistant("migration_uncertain_send").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut buf = [0_u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&buf[..n]);
                if received.ends_with(br#"{"title":"Note"}"#) {
                    observed.fetch_add(1, Ordering::SeqCst);
                    break; // Provider accepts the body, but loses the response.
                }
                assert!(received.len() < 16384);
            }
        }
    });
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_many(
            doc! {"user_id":&f.owner},
            doc! {"$set":{"url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    let _ = write_call(&f).await;
    let request = proposal(&f).await;
    proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "operation"),
    )
    .await
    .unwrap();
    finish(&f, &row, "Waiting").await;
    let resumed = event_turn(&f, &row).await;
    let result = resume(&f, &resumed).await;
    assert!(result.contains("unknown"), "{result}");
    assert!(resume(&f, &resumed).await.contains("unknown"));
    proposals::sweep(&f.state).await.unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(count(&f, "approval_requests").await, 1);
    server.abort();
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_mcp_endpoint_generic_and_prepared_resume_the_original_http_effect() {
    for mode in ["endpoint", "prepared", "generic"] {
        let (mut f, row) = assistant(&format!("execution_proposal_mcp_{mode}")).await;
        let args = if mode == "generic" {
            f.state
                .db
                .collection::<Document>("user_services")
                .update_one(
                    doc! {"_id":&f.connection},
                    doc! {"$set":{"catalog_service_id":null}},
                )
                .await
                .unwrap();
            f.state
                .db
                .collection::<Document>("service_approval_configs")
                .update_one(
                    doc! {"service_id":&f.catalog},
                    doc! {"$set":{"service_id":&f.connection}},
                )
                .await
                .unwrap();
            f.catalog = f.connection.clone();
            json!({"method":"GET","path":"/status"})
        } else {
            let endpoint =
                test_utils::test_service_endpoint(&f.catalog, "status", "GET", "/status");
            f.state
                .db
                .collection("service_endpoints")
                .insert_one(endpoint)
                .await
                .unwrap();
            json!({})
        };
        let name = if mode == "generic" {
            "execution-fixture__request"
        } else {
            "execution-fixture__status"
        };
        let params = if mode == "prepared" {
            json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":args.to_string()}})
        } else {
            json!({"name":name,"arguments":args})
        };
        let response = f
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("x-api-key", &f.raw)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            count(&f, "approval_requests").await,
            1,
            "{mode}: {}",
            String::from_utf8_lossy(&body)
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
        let request = proposal(&f).await;
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation"),
        )
        .await
        .unwrap();
        finish(&f, &row, "Waiting").await;
        let resumed = event_turn(&f, &row).await;
        let text = resume(&f, &resumed).await;
        assert!(text.contains("HTTP 200"), "{mode}: {text}");
        let sent = f.server.received_requests().await.unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url.path(), "/status");
        f.cleanup().await;
    }
}

#[tokio::test]
async fn execution_proposal_transaction_queue_failure_has_no_decision_or_grant() {
    let (f, row) = assistant("execution_proposal_atomic").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    let events = (0..64)
        .map(|_| crate::services::assistant_team_service::event("notice", "unrelated".into(), None))
        .collect::<Vec<_>>();
    f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"pending_events":bson::to_bson(&events).unwrap()}},
        )
        .await
        .unwrap();
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "persistent")
        )
        .await
        .is_err()
    );
    assert_eq!(proposal(&f).await.status, "pending");
    assert_eq!(count(&f, "approval_grants").await, 0);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_recovery_requeues_missing_wake_and_retains_one_effect() {
    let (f, row) = assistant("execution_proposal_recover").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    proposals::decide(&f.state, &f.owner, &request.id, &decision(&request, "task"))
        .await
        .unwrap();
    finish(&f, &row, "Waiting").await;
    f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"pending_events":[]}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("approval_requests")
        .update_one(
            doc! {"_id":&request.id},
            proposals::v3::AssistantVersion::of(&request)
                .unwrap()
                .update(doc! {"$unset":{"execution_proposal.v1.wake_at":""}}),
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(proposals::sweep(&f.state), proposals::sweep(&f.state));
    a.unwrap();
    b.unwrap();
    let current = f
        .state
        .db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current
            .pending_events
            .iter()
            .filter(|e| e.id.starts_with("execution-decision:"))
            .count(),
        1
    );
    let resumed = event_turn(&f, &row).await;
    assert!(resume(&f, &resumed).await.contains("HTTP 200"));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_oversized_and_raw_bodies_keep_legacy_cards() {
    for (kind, body, content_type, path) in [
        (
            "large",
            serde_json::to_vec(&json!({"text":"x".repeat(256*1024)})).unwrap(),
            "application/json",
            "/status",
        ),
        ("raw", b"hello".to_vec(), "text/plain", "/status"),
        (
            "encoded_path",
            b"{}".to_vec(),
            "application/json",
            "/status%20item",
        ),
    ] {
        let (f, _row) = assistant(&format!("execution_proposal_{kind}")).await;
        let response = f
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/proxy/s/execution-fixture{path}"))
                    .header("x-api-key", &f.raw)
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::OK);
        assert_eq!(
            f.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {"execution_proposal.version":{"$exists":true}})
                .await
                .unwrap(),
            0
        );
        assert_eq!(count(&f, "assistant_acknowledgements").await, 1, "{kind}");
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[test]
fn execution_proposal_reusable_shape_rejects_added_body_and_semantic_headers() {
    assert!(proposals::reusable_shape("GET", b"", None, &[]));
    assert!(!proposals::reusable_shape(
        "GET",
        b"{}",
        Some("application/json"),
        &[]
    ));
    assert!(!proposals::reusable_shape(
        "GET",
        b"",
        None,
        &[("x-act-as".into(), "another-person".into())]
    ));
    assert!(proposals::reusable_shape(
        "POST",
        b"{\"title\":\"Note\"}",
        Some("application/json"),
        &[("content-type".into(), "application/json".into())]
    ));
    assert!(!proposals::reusable_shape(
        "POST",
        b"raw",
        Some("text/plain"),
        &[]
    ));
}

#[tokio::test]
async fn execution_proposal_reusable_choices_cover_new_admitted_occurrences_not_changed_inputs() {
    for choice in ["task", "window", "persistent"] {
        let (f, row) = assistant(&format!("execution_proposal_reuse_{choice}")).await;
        let _ = f.call("/proxy/s/execution-fixture/status").await;
        let request = proposal(&f).await;
        proposals::decide(&f.state, &f.owner, &request.id, &decision(&request, choice))
            .await
            .unwrap();
        finish(&f, &row, "Waiting").await;
        let resumed = event_turn(&f, &row).await;
        let result = resume(&f, &resumed).await;
        assert!(result.contains("HTTP 200"), "{result}");
        let mut follow = crate::services::assistant_team_service::event(
            "followup",
            "Continue the admitted task".into(),
            None,
        );
        follow.execution_task = resumed.active_turn.as_ref().unwrap().execution_task.clone();
        f.state
            .db
            .collection::<Document>("assistant_conversations")
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$push":{"pending_events":bson::to_bson(&follow).unwrap()}},
            )
            .await
            .unwrap();
        finish(&f, &resumed, &result).await;
        let next = event_turn(&f, &row).await;
        assert_eq!(
            next.active_turn
                .as_ref()
                .unwrap()
                .execution_task
                .as_ref()
                .unwrap()
                .task,
            resumed
                .active_turn
                .as_ref()
                .unwrap()
                .execution_task
                .as_ref()
                .unwrap()
                .task
        );
        let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
        assert_eq!(status, StatusCode::OK, "{choice}: {body}");
        assert_eq!(count(&f, "approval_requests").await, 1);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
        let _ = f
            .call("/proxy/s/execution-fixture/status?different=resource")
            .await;
        assert_eq!(count(&f, "approval_requests").await, 2);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
        f.cleanup().await;
    }
}

#[tokio::test]
async fn execution_proposal_disable_or_revoke_between_decision_and_send_has_zero_attempts() {
    for change in ["disable", "revoke", "account"] {
        let (f, row) = assistant(&format!("execution_proposal_pre_send_{change}")).await;
        let _ = f.call("/proxy/s/execution-fixture/status").await;
        let request = proposal(&f).await;
        let approved = proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation"),
        )
        .await
        .unwrap();
        finish(&f, &row, "Waiting").await;
        let resumed = event_turn(&f, &row).await;
        match change {
            "disable" => f.enroll(false).await,
            "revoke" => {
                crate::services::approval_service::revoke_grant(
                    &f.state.db,
                    &f.owner,
                    proposals::proposal(&approved)
                        .unwrap()
                        .grant_id
                        .as_ref()
                        .unwrap(),
                )
                .await
                .unwrap();
            }
            _ => {
                f.state
                    .db
                    .collection::<Document>("user_services")
                    .update_one(
                        doc! {"_id":&f.connection},
                        doc! {"$set":{"api_key_id":Uuid::new_v4().to_string()}},
                    )
                    .await
                    .unwrap();
            }
        }
        let result = resume(&f, &resumed).await;
        assert!(result.contains("not sent"), "{change}: {result}");
        assert!(f.server.received_requests().await.unwrap().is_empty());
        let grant = f
            .state
            .db
            .collection::<ApprovalGrant>("approval_grants")
            .find_one(doc! {"approval_request_id":&request.id})
            .await
            .unwrap()
            .unwrap();
        assert!(grant.revoked);
        assert!(grant.expires_at > Utc::now() + chrono::Duration::days(29));
        f.cleanup().await;
    }
}

#[tokio::test]
async fn execution_proposal_notification_claim_is_once_and_preserves_absent_settings() {
    let (f, _row) = assistant("execution_proposal_notification").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    let (a, b) = tokio::join!(proposals::sweep(&f.state), proposals::sweep(&f.state));
    a.unwrap();
    b.unwrap();
    let first = proposal(&f).await;
    assert!(
        proposals::proposal(&first)
            .unwrap()
            .notification_claimed_at
            .is_some()
    );
    f.state
        .db
        .collection::<Document>("approval_requests")
        .update_one(
            doc! {"_id":&request.id},
            proposals::v3::AssistantVersion::of(&request)
                .unwrap()
                .update(doc! {"$unset":{"execution_proposal.v1.maintenance_at":""}}),
        )
        .await
        .unwrap();
    proposals::sweep(&f.state).await.unwrap();
    assert_eq!(
        proposals::proposal(&proposal(&f).await)
            .unwrap()
            .notification_claimed_at,
        proposals::proposal(&first).unwrap().notification_claimed_at
    );
    assert_eq!(count(&f, "notification_channels").await, 0);
    assert_eq!(count(&f, "approval_requests").await, 1);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_unknown_envelope_and_missing_authoring_readiness_fail_closed() {
    let (f, _row) = assistant("execution_proposal_unknown_envelope").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    f.state
        .db
        .collection::<Document>("approval_requests")
        .update_one(
            doc! {"_id":&request.id},
            doc! {"$set":{"execution_proposal.version":99}},
        )
        .await
        .unwrap();
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation")
        )
        .await
        .is_err()
    );
    assert!(
        crate::services::approval_service::process_decision(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            None,
            None,
            &request.id,
            true,
            None,
            None,
            "web"
        )
        .await
        .is_err()
    );
    let row = f
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"_id":&request.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!proposals::view(&row).unwrap().supported);
    assert_eq!(row.status, "pending");
    assert_eq!(count(&f, "approval_grants").await, 0);
    f.state
        .db
        .collection::<Document>("security_replicas")
        .update_many(
            doc! {},
            doc! {"$pull":{"capabilities":proposals::READINESS}},
        )
        .await
        .unwrap();
    let _ = f.call("/proxy/s/execution-fixture/another-operation").await;
    assert_eq!(count(&f, "approval_requests").await, 1);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_caller_correlation_remains_exact_and_operation_only() {
    let (f, _row) = assistant("execution_proposal_caller_header").await;
    for _ in 0..2 {
        let _ = f
            .router()
            .oneshot(
                Request::builder()
                    .uri("/proxy/s/execution-fixture/status")
                    .header("x-api-key", &f.raw)
                    .header("x-request-id", "caller-owned-correlation")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
    }
    let request = proposal(&f).await;
    let p = proposals::proposal(&request).unwrap();
    assert_eq!(p.choices.len(), 1);
    assert_eq!(p.choices[0].id, "operation");
    assert!(p.review.inputs.contains("caller-owned-correlation"));
    assert_eq!(count(&f, "approval_requests").await, 1);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_expiry_is_visible_and_never_reconstructs_the_effect() {
    let (f, row) = assistant("execution_proposal_expiry").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    f.state.db.collection::<Document>("approval_requests").update_one(doc!{"_id":&request.id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    let expired = proposal(&f).await;
    assert_eq!(proposals::view(&expired).unwrap().state, "expired");
    assert!(!proposals::view(&expired).unwrap().can_decide);
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation")
        )
        .await
        .is_err()
    );
    finish(&f, &row, "Waiting").await;
    proposals::sweep(&f.state).await.unwrap();
    let resumed = event_turn(&f, &row).await;
    assert!(resume(&f, &resumed).await.contains("Nothing was sent"));
    assert_eq!(count(&f, "approval_grants").await, 0);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_org_instruction_remains_separate_and_resumes_once() {
    for (transport, allow, service_review) in [
        ("proxy", true, false),
        ("mcp", true, false),
        ("proxy", false, false),
        ("proxy", true, true),
    ] {
        let (f, row) = assistant(&format!(
            "execution_proposal_org_instruction_{transport}_{allow}_{service_review}"
        ))
        .await;
        let org = Uuid::new_v4().to_string();
        f.state
            .db
            .collection("users")
            .insert_one(test_utils::test_user(&org, UserType::Org))
            .await
            .unwrap();
        f.state
            .db
            .collection("org_memberships")
            .insert_one(test_utils::test_membership(
                &org,
                &f.owner,
                crate::models::org_membership::OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("user_services")
            .update_one(doc! {"_id":&f.connection}, doc! {"$set":{"user_id":&org}})
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("user_endpoints")
            .update_many(doc! {"user_id":&f.owner}, doc! {"$set":{"user_id":&org}})
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("service_approval_configs")
            // Exercise the instruction floor alone, and then with an additional
            // independent service review on the org-owned connection.
            .update_many(doc! {}, doc! {"$set":{"approval_required":service_review}})
            .await
            .unwrap();
        if service_review {
            let channel =
                crate::services::notification_service::get_or_create_channel(&f.state.db, &f.owner)
                    .await
                    .unwrap();
            f.state
                .db
                .collection::<Document>("notification_channels")
                .update_one(
                    doc! {"_id":channel.id},
                    doc! {"$set":{"approval_timeout_secs":1_i32}},
                )
                .await
                .unwrap();
        }
        crate::services::feature_flag_service::set_platform_override(
            &f.state.db,
            crate::services::feature_flag_service::ORG_INSTRUCTION_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::User(f.owner.clone()),
            true,
            &f.owner,
        )
        .await
        .unwrap();
        if transport == "proxy" {
            let _ = write_call(&f).await;
        } else {
            f.state
                .db
                .collection("service_endpoints")
                .insert_one(test_utils::test_service_endpoint(
                    &f.catalog, "status", "POST", "/status",
                ))
                .await
                .unwrap();
            let _=f.router().oneshot(Request::builder().method("POST").uri("/mcp").header("x-api-key",&f.raw).header("content-type","application/json").body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execution-fixture__status","arguments":{}}}).to_string())).unwrap()).await.unwrap();
        }
        let request = proposal(&f).await;
        assert_eq!(
            proposals::proposal(&request).unwrap().contract.satisfies,
            vec![ExecutionFloor::AccessModeRisk]
        );
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation"),
        )
        .await
        .unwrap();
        finish(&f, &row, "Waiting").await;
        let first = event_turn(&f, &row).await;
        let text = resume(&f, &first).await;
        assert!(text.contains("separate required review"), "{text}");
        assert!(f.server.received_requests().await.unwrap().is_empty());
        let waiting = proposal(&f).await;
        let card = proposals::proposal(&waiting)
            .unwrap()
            .independent_card_id
            .clone()
            .unwrap();
        let stored = f
            .state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .find_one(doc! {"_id":&card})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored
                .get_document("confirmation")
                .unwrap()
                .get_str("reason"),
            Ok("org_connection")
        );
        finish(&f, &first, &text).await;
        acks::decide(&f.state.db, &f.owner, &row.id, &card, allow)
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("approval_requests")
            .update_one(
                doc! {"_id":&request.id},
                proposals::v3::AssistantVersion::of(&request)
                    .unwrap()
                    .update(doc! {"$unset":{"execution_proposal.v1.wake_at":""}}),
            )
            .await
            .unwrap();
        proposals::sweep(&f.state).await.unwrap();
        let second = event_turn(&f, &row).await;
        let text = resume(&f, &second).await;
        if service_review {
            assert!(text.contains("not sent"), "{text}");
            assert!(f.server.received_requests().await.unwrap().is_empty());
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>("approval_requests")
                    .count_documents(doc! {"execution_proposal":null})
                    .await
                    .unwrap(),
                1,
                "org connection service review remains independent"
            );
        } else if allow {
            assert!(text.contains("HTTP 200"), "{text}");
            assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>("assistant_acknowledgements")
                    .find_one(doc! {"_id":&card})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_str("status"),
                Ok("used")
            );
        } else {
            assert!(text.contains("not sent"), "{text}");
            assert!(f.server.received_requests().await.unwrap().is_empty());
            assert_eq!(count(&f, "assistant_acknowledgements").await, 1);
        }
        f.cleanup().await;
    }
}

#[tokio::test]
async fn execution_proposal_source_deletion_purges_payload_and_retires_operation() {
    let (f, row) = assistant("execution_proposal_delete").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "operation"),
    )
    .await
    .unwrap();
    engine::delete(&f.state.db, &f.owner, &row.id)
        .await
        .unwrap();
    let terminal = proposal(&f).await;
    let p = proposals::proposal(&terminal).unwrap();
    assert!(p.delivery_abandoned);
    assert!(p.payload.is_none() && p.result.is_none());
    assert_eq!(p.state, "cancelled");
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_grants")
            .count_documents(doc! {"revoked":true})
            .await
            .unwrap(),
        1
    );
    proposals::sweep(&f.state).await.unwrap();
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_invocation_retains_lineage_and_cancel_prevents_send() {
    use crate::services::assistant_invocation_service as invocations;
    for cancelled in [false, true] {
        let mut f = Fixture::new(&format!("execution_proposal_invocation_{cancelled}")).await;
        f.enroll(true).await;
        let request = engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: Vec::new(),
            agent_id: None,
            conversation_id: None,
            text: "Check status".into(),
            model: None,
            access_mode: None,
        };
        let mut start: engine::TurnStart = (&request).into();
        start.execution_enrollment = true;
        let admitted = invocations::admit(
            &f.state.db,
            &f.owner,
            "execution-proposal-invocation",
            "one-effect",
            &start,
            &f.state.encryption_keys,
        )
        .await
        .unwrap();
        let row = admitted.claim.unwrap().conversation;
        f.key = row.credential_api_key_id.clone();
        f.raw = credentials::load_for_conversation(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &row.id,
        )
        .await
        .unwrap()
        .unwrap()
        .raw_key
        .to_string();
        let _ = f.call("/proxy/s/execution-fixture/status").await;
        let request = proposal(&f).await;
        assert_eq!(
            proposals::proposal(&request).unwrap().task.task.root_kind,
            TaskRootKind::Invocation
        );
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation"),
        )
        .await
        .unwrap();
        finish(&f, &row, "Waiting").await;
        if cancelled {
            invocations::cancel(&f.state, &f.owner, "execution-proposal-invocation")
                .await
                .unwrap();
            let terminal = proposal(&f).await;
            assert!(proposals::proposal(&terminal).unwrap().delivery_abandoned);
            assert!(proposals::proposal(&terminal).unwrap().payload.is_none());
            assert!(f.server.received_requests().await.unwrap().is_empty());
        } else {
            let resumed = event_turn(&f, &row).await;
            assert_eq!(
                resumed
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .invocation_id
                    .as_deref(),
                Some(admitted.invocation.id.as_str())
            );
            assert!(resume(&f, &resumed).await.contains("HTTP 200"));
            assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        }
        f.cleanup().await;
    }
}

#[tokio::test]
async fn execution_proposal_recovers_a_matching_operation_reservation_before_send() {
    let (f, row) = assistant("execution_proposal_reserved_recovery").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    let approved = proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "operation"),
    )
    .await
    .unwrap();
    let p = proposals::proposal(&approved).unwrap();
    finish(&f, &row, "Waiting").await;
    let resumed = event_turn(&f, &row).await;
    let old = Utc::now() - chrono::Duration::minutes(3);
    f.state.db.collection::<Document>("approval_requests").update_one(doc!{"_id":&request.id},proposals::v3::AssistantVersion::of(&request).unwrap().update(doc!{"$set":{
        "execution_proposal.v1.state":"preparing","execution_proposal.v1.claim_id":"lost-preparer",
        "execution_proposal.v1.claimed_at":bson::DateTime::from_chrono(old)
    }})).await.unwrap();
    f.state.db.collection::<Document>("approval_grants").update_one(doc!{"_id":&p.grant_id},doc!{"$set":{"execution_use":bson::to_bson(&ExecutionUse{
        version:VERSION,operation_id:p.effect_id.clone(),prepared_digest:p.prepared_digest.clone(),claim_id:"lost-reservation".into(),
        state:ExecutionUseState::Claimed,claimed_at:old,dispatched_at:None,completed_at:None,
    }).unwrap()}}).await.unwrap();
    let text = resume(&f, &resumed).await;
    assert!(text.contains("HTTP 200"), "{text}");
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert_eq!(resume(&f, &resumed).await, text);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn execution_proposal_personal_policy_coverage_never_satisfies_an_org_or_other_actor_review()
{
    use crate::models::service_approval_config::{ApprovalEffect, ApprovalMode};
    use crate::services::approval_service::{ApprovalOutcome, ApprovalResolution, PendingApproval};
    let f = Fixture::new("execution_proposal_policy_decider").await;
    f.enroll(true).await;
    f.store(&f.grant().await).await;
    let context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let target = f.target().await;
    let _permit = f.prepare(&context, &target).await;
    let pending = |actor: &str, org: bool| {
        ApprovalOutcome::NeedsApproval(PendingApproval {
            options: Default::default(),
            resolution: ApprovalResolution {
                required: true,
                mode: ApprovalMode::PerRequest,
                effect: ApprovalEffect::RequireApproval,
                grant_scope: None,
                primary_owner_user_id: actor.into(),
                from_org_policy: org,
            },
            primary_owner_user_id: actor.into(),
            requester_type: "api_key".into(),
            requester_id: f.key.clone(),
        })
    };
    assert!(matches!(
        approval_outcome(
            Some(&context),
            &f.owner,
            Some(&f.connection),
            pending(&f.owner, false)
        ),
        ApprovalOutcome::Allowed { required: false }
    ));
    for outcome in [pending(&f.owner, true), pending("another-person", false)] {
        assert!(matches!(
            approval_outcome(Some(&context), &f.owner, Some(&f.connection), outcome),
            ApprovalOutcome::NeedsApproval(_)
        ));
    }
    assert!(matches!(
        approval_outcome(
            Some(&context),
            "different-connection-owner",
            Some(&f.connection),
            pending(&f.owner, false)
        ),
        ApprovalOutcome::NeedsApproval(_)
    ));
    f.no_prompts().await;
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[path = "proposal_private_conversations_tests.rs"]
mod private_conversations;

#[path = "proposal_channel_tests.rs"]
mod channel;

/// #2354: six genuinely different effects must not look like six deliveries
/// of one failed request. Repeated calls still join their exact receipt.
#[tokio::test]
async fn retained_review_batch_reports_distinct_denials_and_expiries_once_after_reload() {
    use futures::TryStreamExt;
    let (f, row) = assistant("retained_review_batch_2354").await;
    let mut ids = Vec::new();
    for index in 0..6 {
        let path = format!("/proxy/s/execution-fixture/status?item={index}");
        for _ in 0..2 {
            let (status, _) = f.call(&path).await;
            assert_ne!(status, StatusCode::OK);
        }
        assert_eq!(count(&f, "approval_requests").await, index + 1);
        let request = f
            .state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {"_id":{"$nin":&ids}})
            .await
            .unwrap()
            .unwrap();
        ids.push(request.id.clone());
        if index < 3 {
            proposals::decide(
                &f.state,
                &f.owner,
                &request.id,
                &Decision {
                    approved: false,
                    expected_revision: 1,
                    choice_id: None,
                    choice_digest: None,
                    idempotency_key: Uuid::new_v4().to_string(),
                },
            )
            .await
            .unwrap();
            // Denial is a decision on this exact effect, not a new card.
            let _ = f.call(&path).await;
            assert_eq!(count(&f, "approval_requests").await, index + 1);
        }
    }
    finish(&f, &row, "Waiting for the decisions").await;
    f.state.db.collection::<Document>("approval_requests").update_many(
        doc! {"status":"pending"}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}},
    ).await.unwrap();
    let (a, b) = tokio::join!(proposals::sweep(&f.state), proposals::sweep(&f.state));
    a.unwrap();
    b.unwrap();
    // Assert the durable contract before presentation: request denials use
    // "rejected", while the retained proposal itself uses "denied".
    for status in ["rejected", "expired"] {
        assert_eq!(
            f.state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .count_documents(doc! {"_id":{"$in":&ids},"status":status})
                .await
                .unwrap(),
            3,
            "unexpected durable request status: {status}",
        );
    }
    let resumed = event_turn(&f, &row).await;
    assert_eq!(resumed.active_turn.as_ref().unwrap().events.len(), 6);
    let event_text = engine::events_text(&resumed.active_turn.as_ref().unwrap().events);
    assert_eq!(event_text.matches("\n- ").count(), 1);
    assert!(event_text.contains("6 distinct permission requests"));
    // Reload a live turn, as recovery/history polling would: durable identities
    // and state, rather than client-side deduplication, determine the result.
    let reloaded = f
        .state
        .db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    let text = resume(&f, &reloaded).await;
    assert_eq!(
        text.matches("Permission requests denied: 3.").count(),
        1,
        "{text}"
    );
    assert_eq!(
        text.matches("Permission requests expired: 3.").count(),
        1,
        "{text}"
    );
    for id in &ids {
        assert_eq!(text.matches(id).count(), 1);
    }
    assert!(!text.contains("not approved"));
    assert!(!text.contains("item="));
    assert_eq!(resume(&f, &reloaded).await, text);
    let mut replayed = reloaded.clone();
    let events = &mut replayed.active_turn.as_mut().unwrap().events;
    events.push(events[0].clone());
    assert_eq!(
        resume(&f, &replayed).await,
        text,
        "a repeated wake ID is one receipt"
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    finish(&f, &reloaded, &text).await;
    let requests: Vec<ApprovalRequest> = f
        .state
        .db
        .collection("approval_requests")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(requests.len(), 6);
    for request in &requests {
        assert_eq!(
            proposals::proposal(request)
                .unwrap()
                .delivered_turn_id
                .as_deref(),
            Some(reloaded.active_turn.as_ref().unwrap().turn_id.as_str())
        );
        let version = proposals::v3::AssistantVersion::of(request).unwrap();
        f.state.db.collection::<Document>("approval_requests").update_one(
            doc! {"_id":&request.id}, version.update(doc! {"$unset":{"execution_proposal.v1.maintenance_at":"","execution_proposal.v1.wake_at":""}}),
        ).await.unwrap();
    }
    proposals::sweep(&f.state).await.unwrap();
    let current = f
        .state
        .db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        current.pending_events.is_empty(),
        "delivered receipts must not wake again"
    );
    let replies: Vec<Document> = f.state.db.collection("assistant_messages")
        .find(doc! {"conversation_id":&row.id,"turn_id":&reloaded.active_turn.as_ref().unwrap().turn_id,"role":"assistant"})
        .await.unwrap().try_collect().await.unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].get_str("text").unwrap(), text);
    f.cleanup().await;
}

#[tokio::test]
async fn expired_retained_request_is_not_reraised_by_same_turn_retry() {
    let (f, row) = assistant("retained_expiry_retry_2354").await;
    let _ = write_call(&f).await;
    let request = proposal(&f).await;
    f.state.db.collection::<Document>("approval_requests").update_one(
        doc! {"_id":&request.id},
        doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}},
    ).await.unwrap();
    for _ in 0..2 {
        let (status, _) = write_call(&f).await;
        assert_ne!(status, StatusCode::OK);
    }
    assert_eq!(count(&f, "approval_requests").await, 1);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    finish(&f, &row, "The request expired").await;
    // A later explicit owner request starts a new task and may ask again.
    let request = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: Vec::new(),
        agent_id: None,
        conversation_id: Some(row.id.clone()),
        text: "Try the operation again".into(),
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&request).into();
    start.execution_enrollment = true;
    let next = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_ne!(
        next.active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .task,
        row.active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .task
    );
    let _ = write_call(&f).await;
    assert_eq!(count(&f, "approval_requests").await, 2);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}
