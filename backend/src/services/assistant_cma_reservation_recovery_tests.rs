use crate::services::{
    assistant_cma_reservation_recovery as recovery, assistant_cma_reservation_service as admin,
};
use mongodb::bson::Document;
use serde_json::json;
const PARENTS: &str = crate::models::assistant_cma_sandbox::COLLECTION_NAME;

async fn setup(route: &str, receipt: Option<CreationReceipt>) -> (Fixture, AssistantCmaCreation) {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let f = Fixture::with_management(false).await;
    let body = br#"{"name":"Recovery fixture"}"#;
    let permit = f
        .admit(&f.target, route, "recovery-original-key", body)
        .await
        .unwrap();
    permit
        .before_send(&f.request(route, "recovery-original-key", body))
        .await
        .unwrap();
    let mut creation = f.saved(&permit.creation.as_ref().unwrap().id).await;
    if let Some(receipt) = receipt {
        creation = change_creation(&f.db, &creation, CreationChange::Receipt(receipt))
            .await
            .unwrap();
    }
    (f, creation)
}
async fn parent(f: &Fixture, c: &AssistantCmaCreation) -> AssistantCmaSandbox {
    sandboxes::get_owned(&f.db, &f.owner, &c.sandbox_ref_id)
        .await
        .unwrap()
}
fn state(f: &Fixture) -> crate::AppState {
    let mut state = crate::test_utils::test_app_state(f.db.clone());
    state.config = f.config.clone();
    state.jwt_keys = f.jwt.clone();
    state
}
async fn all_gets(f: &Fixture) {
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET")
    );
    assert_eq!(
        f.db.collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
async fn retiring(f: &Fixture, c: &AssistantCmaCreation) -> AssistantCmaSandbox {
    f.db.collection::<Document>(PARENTS).update_one(doc! {"_id":&c.sandbox_ref_id}, doc! {"$set":{
        "cma_sandbox_id":"sandbox_1","admission_state":"retiring","retire_requested_at":bson::DateTime::now()
    }}).await.unwrap();
    parent(f, c).await
}
async fn retired_response(f: &Fixture) {
    Mock::given(method("GET"))
        .and(path("/api/v1/sandboxes/sandbox_1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"sandbox_1","owner":f.owner,"status":"retired"})),
        )
        .mount(&f.server)
        .await;
}

#[tokio::test]
async fn cma_recovery_sweep_settles_submitted_lifecycle_without_owner_activity_or_feature() {
    let (f, c) = setup(
        "/api/v1/agents",
        Some(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await;
    f.observations().await;
    recovery::sweep(&state(&f)).await.unwrap();
    let after = parent(&f, &c).await;
    assert_eq!(after.cma_sandbox_id.as_deref(), Some("sandbox_1"));
    assert_eq!(after.admission_state, SandboxAdmissionState::Associated);
    let saved = f.saved(&c.id).await;
    assert_eq!(saved.submission_state, "confirmed");
    assert!(saved.request_ciphertext.is_none());
    assert!(saved.request_context_version.is_none());
    assert_eq!(saved.cma_agent_id.as_deref(), Some("agent_1"));
    let count = f.server.received_requests().await.unwrap().len();
    recovery::sweep(&state(&f)).await.unwrap();
    assert_eq!(f.server.received_requests().await.unwrap().len(), count);
    assert_eq!(
        recovery::recover_one(&f.db, &f.gateway(), &after)
            .await
            .unwrap(),
        "unchanged"
    );
    all_gets(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn cma_recovery_retirement_is_exact_atomic_audited_and_idempotent() {
    let (f, c) = setup(
        "/api/v1/sandboxes",
        Some(CreationReceipt::Sandbox("sandbox_1".into())),
    )
    .await;
    retiring(&f, &c).await;
    retired_response(&f).await;
    recovery::sweep(&state(&f)).await.unwrap();
    let after = parent(&f, &c).await;
    assert_eq!(after.admission_state, SandboxAdmissionState::Retired);
    assert!(!after.occupies_account_slot);
    assert!(after.request_ciphertext.is_none());
    assert!(f.saved(&c.id).await.request_ciphertext.is_none());
    assert_eq!(
        recovery::recover_one(&f.db, &f.gateway(), &after)
            .await
            .unwrap(),
        "unchanged"
    );
    let audits =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type":"cma_sandbox_reservation_recovered"})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
    assert_eq!(audits.len(), 1);
    assert!(audits[0].get("seq").is_some());
    assert!(!format!("{:?}", audits).contains("recovery-original-key"));
    all_gets(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn cma_recovery_operation_status_never_proves_absence_or_replays() {
    for (status, reason) in [
        ("queued", "operation_in_progress_without_resource"),
        ("running", "operation_in_progress_without_resource"),
        ("waiting_for_connection", "operation_waiting_for_connection"),
        ("failed", "operation_failed_without_resource"),
        ("succeeded", "operation_without_resource"),
    ] {
        let (f, c) = setup(
            "/api/v1/agents",
            Some(CreationReceipt::Lifecycle("op_1".into())),
        )
        .await;
        Mock::given(method("GET")).and(path("/api/v1/agent-lifecycle/op_1")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"op_1","status":status,"agent_id":null,"error":"private provider details"}))).mount(&f.server).await;
        let before = parent(&f, &c).await;
        assert_eq!(
            recovery::recover_one(&f.db, &f.gateway(), &before)
                .await
                .unwrap(),
            reason
        );
        let actor = crate::services::audit_service::AuditActor {
            user_id: "admin".into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        let inspect = admin::inspect(&f.db, &f.gateway(), &before.id, &actor)
            .await
            .unwrap();
        assert!(
            matches!(inspect.outcome,admin::InspectionOutcome::Unknown{reason:actual} if actual==reason)
        );
        assert!(inspect.next_step.is_some());
        let after = parent(&f, &c).await;
        assert!(after.occupies_account_slot);
        assert!(after.cma_sandbox_id.is_none());
        assert!(f.saved(&c.id).await.request_ciphertext.is_some());
        all_gets(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn cma_recovery_no_receipt_and_external_rows_are_unchanged() {
    let (f, c) = setup("/api/v1/sandboxes", None).await;
    let before = parent(&f, &c).await;
    assert_eq!(
        recovery::recover_one(&f.db, &f.gateway(), &before)
            .await
            .unwrap(),
        "unchanged"
    );
    let mut external = before.clone();
    external.source = "discovered".into();
    assert_eq!(
        recovery::recover_one(&f.db, &f.gateway(), &external)
            .await
            .unwrap(),
        "unchanged"
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        bson::to_document(&parent(&f, &c).await).unwrap(),
        bson::to_document(&before).unwrap()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn cma_recovery_unavailable_missing_or_live_retirement_keeps_slot() {
    for status in [404, 503, 200] {
        let (f, c) = setup(
            "/api/v1/sandboxes",
            Some(CreationReceipt::Sandbox("sandbox_1".into())),
        )
        .await;
        let before = retiring(&f, &c).await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sandboxes/sandbox_1"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"id":"sandbox_1","owner":f.owner,"status":"ready"})),
            )
            .mount(&f.server)
            .await;
        let result = recovery::recover_one(&f.db, &f.gateway(), &before).await;
        if status == 200 {
            assert_eq!(result.unwrap(), "retirement_not_confirmed");
        } else {
            assert!(result.is_err());
        }
        assert_eq!(
            bson::to_document(&parent(&f, &c).await).unwrap(),
            bson::to_document(&before).unwrap()
        );
        all_gets(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn cma_recovery_retirement_fences_parent_generation_revision_creation_and_erasure() {
    for kind in [
        "generation",
        "parent_version",
        "creation_revision",
        "erasure",
    ] {
        let (f, c) = setup(
            "/api/v1/sandboxes",
            Some(CreationReceipt::Sandbox("sandbox_1".into())),
        )
        .await;
        let captured = retiring(&f, &c).await;
        retired_response(&f).await;
        let remote = f
            .gateway()
            .get_sandbox_authority(&f.owner, "sandbox_1")
            .await
            .unwrap();
        match kind {
            "generation" => {
                f.db.collection::<Document>(PARENTS)
                    .update_one(
                        doc! {"_id":&captured.id},
                        doc! {"$inc":{"generation":1_i64}},
                    )
                    .await
                    .unwrap();
            }
            "parent_version" => {
                f.db.collection::<Document>(PARENTS)
                    .update_one(
                        doc! {"_id":&captured.id},
                        doc! {"$inc":{"agent_reference_version":1_i64}},
                    )
                    .await
                    .unwrap();
            }
            "creation_revision" => {
                f.db.collection::<Document>(COLLECTION_NAME)
                    .update_one(doc! {"_id":&c.id}, doc! {"$inc":{"revision":1_i64}})
                    .await
                    .unwrap();
            }
            _ => {
                f.db.collection::<Document>(crate::services::erasure_admission::COLLECTION)
                    .update_one(
                        doc! {"_id":&f.owner},
                        doc! {"$set":{"erasure_accepted":true}},
                    )
                    .upsert(true)
                    .await
                    .unwrap();
            }
        }
        assert!(
            reservation_admin::commit_recovery(
                &f.db,
                &captured,
                Some(&c),
                reservation_admin::Change::ConfirmRetirement(remote),
                json!({})
            )
            .await
            .is_err()
        );
        assert!(parent(&f, &c).await.occupies_account_slot);
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn cma_recovery_admin_can_confirm_exact_retirement_including_legacy_bound_row() {
    for legacy in [false, true] {
        let (f, c) = setup(
            "/api/v1/sandboxes",
            Some(CreationReceipt::Sandbox("sandbox_1".into())),
        )
        .await;
        let before = retiring(&f, &c).await;
        if legacy {
            f.db.collection::<Document>(COLLECTION_NAME)
                .delete_one(doc! {"_id":&c.id})
                .await
                .unwrap();
        }
        retired_response(&f).await;
        let actor = crate::services::audit_service::AuditActor {
            user_id: "admin".into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        let inspect = admin::inspect(&f.db, &f.gateway(), &before.id, &actor)
            .await
            .unwrap();
        assert!(
            parent(&f, &c).await.occupies_account_slot,
            "inspection is audit-only"
        );
        let req = admin::ResolveRequest {
            generation: inspect.reservation.generation,
            revision: inspect.reservation.revision,
            creation_revision: inspect.reservation.creation_revision,
            action: admin::ResolveAction::ConfirmRetirement,
            remote_sandbox_id: Some("sandbox_1".into()),
        };
        let after = admin::resolve(&f.db, &f.gateway(), &before.id, &req, &actor)
            .await
            .unwrap();
        assert!(!after.occupies_account_slot);
        assert_eq!(after.admission_state, SandboxAdmissionState::Retired);
        assert!(
            admin::resolve(&f.db, &f.gateway(), &before.id, &req, &actor)
                .await
                .is_err()
        );
        all_gets(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn cma_recovery_sweep_is_fair_resumable_leased_and_diagnostics_are_bounded() {
    use crate::services::coordination_service::{LeaseStore, cluster_lease_runtime};
    let (f, c) = setup(
        "/api/v1/agents",
        Some(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await;
    f.observations().await;
    let original = parent(&f, &c).await;
    // First full page has valid legacy rows with unavailable owners. None can
    // block the receipt on the next page, including after a new app instance.
    for i in 0..16 {
        let mut broken = original.clone();
        broken.id = format!("00000000-0000-0000-0000-{i:012}");
        broken.user_id = format!("absent-owner-{i}");
        broken.cma_sandbox_id = Some("legacy_sandbox".into());
        broken.retire_requested_at = Some(Utc::now());
        broken.admission_state = SandboxAdmissionState::Retiring;
        f.db.collection::<AssistantCmaSandbox>(PARENTS)
            .insert_one(broken)
            .await
            .unwrap();
    }
    let db = &f.db;
    let token = cluster_lease_runtime()
        .acquire(db, "assistant:cma-reservation-recovery")
        .await
        .unwrap()
        .unwrap();
    recovery::sweep(&state(&f)).await.unwrap();
    assert!(f.server.received_requests().await.unwrap().is_empty());
    LeaseStore::release(db, &token).await.unwrap();
    recovery::sweep(&state(&f)).await.unwrap();
    assert_eq!(f.saved(&c.id).await.submission_state, "submitted");
    recovery::sweep(&state(&f)).await.unwrap();
    assert_eq!(f.saved(&c.id).await.submission_state, "confirmed");
    let audits = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"cma_sandbox_reservation_recovery_deferred"})
            .await
            .unwrap(),
        16
    );
    // Start another complete pass; unchanged failures produce no audit flood.
    db.collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
        .update_one(
            doc! {"_id":"assistant:cma-reservation-recovery"},
            doc! {"$unset":{"checkpoint":""}},
        )
        .await
        .unwrap();
    recovery::sweep(&state(&f)).await.unwrap();
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"cma_sandbox_reservation_recovery_deferred"})
            .await
            .unwrap(),
        16
    );
    all_gets(&f).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn cma_recovery_receipts_reject_changed_deployment_generation_and_foreign_owner() {
    for kind in ["deployment", "generation", "foreign_owner"] {
        let (f, c) = setup(
            "/api/v1/agents",
            Some(CreationReceipt::Lifecycle("op_1".into())),
        )
        .await;
        f.observations().await;
        let before = parent(&f, &c).await;
        match kind {
            "deployment" => {
                f.db.collection::<DownstreamService>(
                    crate::models::downstream_service::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&f.catalog.id},
                    doc! {"$set":{"base_url":"https://invalid.example"}},
                )
                .await
                .unwrap();
            }
            "generation" => {
                f.db.collection::<Document>(PARENTS)
                    .update_one(
                        doc! {"_id":&c.sandbox_ref_id},
                        doc! {"$inc":{"generation":1_i64}},
                    )
                    .await
                    .unwrap();
            }
            _ => {
                Mock::given(method("GET")).and(path("/api/v1/agents/agent_1")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"agent_1","owner":"someone-else","sandbox_id":"sandbox_1","status":"ready"}))).with_priority(1).mount(&f.server).await;
            }
        }
        assert!(
            recovery::recover_one(&f.db, &f.gateway(), &before)
                .await
                .is_err()
        );
        let after = parent(&f, &c).await;
        assert!(after.occupies_account_slot);
        assert!(after.cma_sandbox_id.is_none());
        assert_eq!(f.saved(&c.id).await.submission_state, "submitted");
        all_gets(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn cma_recovery_saved_sandbox_and_agent_receipts_complete_without_replay() {
    for receipt in [
        CreationReceipt::Sandbox("sandbox_1".into()),
        CreationReceipt::Agent("agent_1".into()),
    ] {
        let route = if matches!(receipt, CreationReceipt::Sandbox(_)) {
            "/api/v1/sandboxes"
        } else {
            "/api/v1/agents"
        };
        let (f, c) = setup(route, Some(receipt)).await;
        f.observations().await;
        assert_eq!(
            recovery::recover_one(&f.db, &f.gateway(), &parent(&f, &c).await)
                .await
                .unwrap(),
            "creation_confirmed"
        );
        assert_eq!(f.saved(&c.id).await.submission_state, "confirmed");
        assert!(parent(&f, &c).await.occupies_account_slot);
        all_gets(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn cma_recovery_admin_retirement_refuses_live_unknown_and_mismatched_resource() {
    for status in [200, 404, 503] {
        let (f, c) = setup(
            "/api/v1/sandboxes",
            Some(CreationReceipt::Sandbox("sandbox_1".into())),
        )
        .await;
        let before = retiring(&f, &c).await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sandboxes/sandbox_1"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(json!({"id":"sandbox_1","owner":f.owner,"status":"ready"})),
            )
            .mount(&f.server)
            .await;
        let actor = crate::services::audit_service::AuditActor {
            user_id: "admin".into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        let mut request = admin::ResolveRequest {
            generation: before.generation,
            revision: before.agent_reference_version,
            creation_revision: Some(c.revision),
            action: admin::ResolveAction::ConfirmRetirement,
            remote_sandbox_id: Some("sandbox_1".into()),
        };
        request.remote_sandbox_id = Some("different_sandbox".into());
        assert!(
            admin::resolve(&f.db, &f.gateway(), &before.id, &request, &actor)
                .await
                .is_err()
        );
        request.remote_sandbox_id = Some("sandbox_1".into());
        assert!(
            admin::resolve(&f.db, &f.gateway(), &before.id, &request, &actor)
                .await
                .is_err()
        );
        assert!(parent(&f, &c).await.occupies_account_slot);
        all_gets(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn cma_recovery_sweep_timeout_preserves_receipt_and_advances_cursor() {
    let (f, c) = setup(
        "/api/v1/agents",
        Some(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/agent-lifecycle/op_1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(20))
                .set_body_json(json!({"id":"op_1","status":"running"})),
        )
        .mount(&f.server)
        .await;
    recovery::sweep(&state(&f)).await.unwrap();
    assert!(parent(&f, &c).await.occupies_account_slot);
    assert!(f.saved(&c.id).await.request_ciphertext.is_some());
    let audit =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type":"cma_sandbox_reservation_recovery_deferred"})
            .await
            .unwrap()
            .unwrap();
    assert!(format!("{audit:?}").contains("observation_timeout"));
    let count = f.server.received_requests().await.unwrap().len();
    recovery::sweep(&state(&f)).await.unwrap();
    assert_eq!(
        f.server.received_requests().await.unwrap().len(),
        count,
        "cursor completed pass and rests, instead of hammering slow owner"
    );
    all_gets(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn cma_recovery_associated_parent_still_finishes_submitted_agent_journal() {
    let (f, c) = setup(
        "/api/v1/agents",
        Some(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await;
    f.observations().await;
    let before = parent(&f, &c).await;
    let actor = crate::services::audit_service::AuditActor {
        user_id: "admin".into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor)
        .await
        .unwrap();
    let request = admin::ResolveRequest {
        generation: inspected.reservation.generation,
        revision: inspected.reservation.revision,
        creation_revision: inspected.reservation.creation_revision,
        action: admin::ResolveAction::Associate,
        remote_sandbox_id: Some("sandbox_1".into()),
    };
    let associated = admin::resolve(&f.db, &f.gateway(), &before.id, &request, &actor)
        .await
        .unwrap();
    assert_eq!(associated.holding, "live_sandbox");
    assert_eq!(
        associated.admission_state,
        SandboxAdmissionState::Associated
    );
    assert_eq!(f.saved(&c.id).await.submission_state, "submitted");
    assert!(f.saved(&c.id).await.request_ciphertext.is_some());
    recovery::sweep(&state(&f)).await.unwrap();
    let saved = f.saved(&c.id).await;
    assert_eq!(saved.submission_state, "confirmed");
    assert_eq!(saved.cma_agent_id.as_deref(), Some("agent_1"));
    assert!(saved.request_ciphertext.is_none());
    assert!(parent(&f, &c).await.occupies_account_slot);
    all_gets(&f).await;
    f.db.drop().await.unwrap();
}

async fn covered_setup(route: &str, operation: Option<&str>) -> (Fixture, AssistantCmaCreation) {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let mut f = Fixture::with_management(true).await;
    f.config.cma_creation_outcome_epoch = Some("e".repeat(32));
    let body = br#"{"name":"Covered fixture"}"#;
    let permit = f.admit(&f.target, route, "covered-owner-key", body).await.unwrap();
    let mut request = f.request(route, "covered-owner-key", body);
    let creation = permit.creation.as_ref().unwrap();
    let upstream = creation.covered_key.clone().unwrap();
    assert_ne!(upstream, creation.idempotency_key);
    permit.prepare_request(&mut request).await.unwrap();
    assert_eq!(request.headers()["idempotency-key"], upstream.as_str());
    let mut saved = f.saved(&creation.id).await;
    if let Some(operation)=operation {
        saved=change_creation(&f.db,&saved,CreationChange::Receipt(CreationReceipt::Lifecycle(operation.into()))).await.unwrap();
    }
    (f,saved)
}
async fn covered_reply(f: &Fixture, c: &AssistantCmaCreation, result: Value) {
    Mock::given(method("POST")).and(path("/api/v1/sandbox-creation-outcomes/lookup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(creation_outcome::fixture_response(c,result)))
        .mount(&f.server).await;
}
fn absence(operation: Option<&str>, code: &str) -> Value {
    json!({"kind":"refused","code":code,"operation_id":operation,"proof_id":"a".repeat(64)})
}

#[tokio::test]
async fn covered_creation_without_response_id_settles_and_allows_a_new_key() {
    let (f,c) = covered_setup("/api/v1/sandboxes",None).await;
    let before=parent(&f,&c).await;
    covered_reply(&f,&c,absence(None,"creation_expired")).await;
    assert_eq!(recovery::recover_one(&f.db,&f.gateway(),&before).await.unwrap(),"creation_confirmed_not_created");
    let after=parent(&f,&c).await;
    assert!(!after.occupies_account_slot);
    assert_eq!(f.saved(&c.id).await.submission_state,"confirmed_not_created");
    assert!(f.admit(&f.target,"/api/v1/sandboxes","covered-owner-key",br#"{"name":"Covered fixture"}"#).await.is_err());
    assert!(f.admit(&f.target,"/api/v1/sandboxes","fresh-owner-key",br#"{"name":"Fresh fixture"}"#).await.is_ok());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn covered_cancellation_waits_for_terminal_evidence_and_replays_idempotently() {
    let op="1".repeat(32);
    let (f,c)=covered_setup("/api/v1/agents",Some(&op)).await;
    let before=parent(&f,&c).await;
    covered_reply(&f,&c,json!({"kind":"in_progress","operation_id":op})).await;
    assert_eq!(recovery::recover_one(&f.db,&f.gateway(),&before).await.unwrap(),"creation_in_progress");
    assert!(parent(&f,&c).await.occupies_account_slot);
    f.server.reset().await;
    Mock::given(method("POST")).and(path(format!("/api/v1/agent-lifecycle/{op}/cancel")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":op,"status":"failed"}))).mount(&f.server).await;
    covered_reply(&f,&c,absence(Some(&op),"creation_cancelled")).await;
    let gateway=f.gateway();
    let result=reservation_admin::cancel_covered(&f.db,&gateway,&f.owner,&before.id,before.generation).await.unwrap();
    assert!(!result.pending);
    assert!(!parent(&f,&c).await.occupies_account_slot);
    assert_eq!(f.saved(&c.id).await.operation_id.as_deref(),Some(op.as_str()));
    let count=f.server.received_requests().await.unwrap().len();
    assert!(!reservation_admin::cancel_covered(&f.db,&gateway,&f.owner,&before.id,before.generation).await.unwrap().pending);
    assert_eq!(f.server.received_requests().await.unwrap().len(),count);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn covered_created_receipt_retains_live_slot_and_releases_exact_retired_association() {
    for status in ["active","retired"] {
        let (f,c)=covered_setup("/api/v1/sandboxes",None).await;
        let before=parent(&f,&c).await;
        covered_reply(&f,&c,json!({"kind":"created","sandbox_id":"sandbox_1","proof_id":"b".repeat(64)})).await;
        Mock::given(method("GET")).and(path("/api/v1/sandboxes/sandbox_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"sandbox_1","owner":f.owner,"status":status}))).mount(&f.server).await;
        recovery::recover_one(&f.db,&f.gateway(),&before).await.unwrap();
        let after=parent(&f,&c).await;
        assert_eq!(after.cma_sandbox_id.as_deref(),Some("sandbox_1"));
        assert_eq!(after.occupies_account_slot,status!="retired");
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn covered_unknown_or_mismatched_reply_cannot_release() {
    let (f,c)=covered_setup("/api/v1/sandboxes",None).await;
    let before=parent(&f,&c).await;
    covered_reply(&f,&c,json!({"kind":"unknown","reason":"recovery_required"})).await;
    assert_eq!(recovery::recover_one(&f.db,&f.gateway(),&before).await.unwrap(),"creation_outcome_unknown");
    f.server.reset().await;
    let mut bad=creation_outcome::fixture_response(&c,absence(None,"creation_expired"));
    bad["identity_digest"]=json!("0".repeat(64));
    Mock::given(method("POST")).and(path("/api/v1/sandbox-creation-outcomes/lookup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(bad)).mount(&f.server).await;
    assert!(recovery::recover_one(&f.db,&f.gateway(),&before).await.is_err());
    assert!(parent(&f,&c).await.occupies_account_slot);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn covered_recovery_two_replicas_and_stale_generations() {
    let (f,c)=covered_setup("/api/v1/sandboxes",None).await;
    let before=parent(&f,&c).await;
    covered_reply(&f,&c,absence(None,"creation_expired")).await;
    let left=f.gateway();let right=f.gateway();
    let (a,b)=tokio::join!(recovery::recover_one(&f.db,&left,&before),recovery::recover_one(&f.db,&right,&before));
    assert!(a.is_ok() || b.is_ok());
    assert!(!parent(&f,&c).await.occupies_account_slot);
    let count=f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME).count_documents(doc!{"event_type":"cma_sandbox_reservation_recovered"}).await.unwrap();
    assert_eq!(count,1);
    f.db.drop().await.unwrap();

    for field in ["generation","agent_reference_version","submission_generation","revision"] {
        let (f,c)=covered_setup("/api/v1/sandboxes",None).await;
        let before=parent(&f,&c).await;
        covered_reply(&f,&c,absence(None,"creation_expired")).await;
        let (collection,id)=if field=="generation" || field=="agent_reference_version" {(PARENTS,&before.id)} else {(COLLECTION_NAME,&c.id)};
        f.db.collection::<Document>(collection).update_one(doc!{"_id":id},doc!{"$inc":{field:1_i64}}).await.unwrap();
        assert!(reservation_admin::recover_covered(&f.db,&f.gateway(),&before,&c).await.is_err());
        assert!(parent(&f,&c).await.occupies_account_slot);
        f.db.drop().await.unwrap();
    }
}

// Drop the caller's acknowledgement after the exact two-row settlement. A
// restarted observer with the old snapshot can positively read back the winner.
#[tokio::test]
async fn covered_recovery_readback_after_lost_settlement_acknowledgement() {
    let (f,c)=covered_setup("/api/v1/sandboxes",None).await;
    let before=parent(&f,&c).await;
    reservation_admin::commit_recovery(&f.db,&before,Some(&c),reservation_admin::Change::CoveredAbsent(None),json!({"result":"terminal"})).await.unwrap();
    covered_reply(&f,&c,absence(None,"creation_expired")).await;
    assert_eq!(reservation_admin::recover_covered(&f.db,&f.gateway(),&before,&c).await.unwrap(),"creation_confirmed_not_created");
    assert!(!parent(&f,&c).await.occupies_account_slot);
    assert_eq!(f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME).count_documents(doc!{"event_type":"cma_sandbox_reservation_recovered"}).await.unwrap(),1);
    f.db.drop().await.unwrap();
}
