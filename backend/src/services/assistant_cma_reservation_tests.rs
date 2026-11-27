use crate::services::{assistant_cma_reservation_service as admin, audit_service::AuditActor};
use mongodb::bson::Document;
use serde_json::json;
const PARENTS: &str = crate::models::assistant_cma_sandbox::COLLECTION_NAME;
const BODY: &[u8] = br#"{"name":"private merchant request"}"#;
fn actor(f: &Fixture) -> AuditActor {
    AuditActor {
        user_id: format!("admin-{}", f.owner),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}
async fn pending(f: &Fixture, route: &str) -> AssistantCmaCreation {
    let permit = f
        .admit(&f.target, route, "private-key", BODY)
        .await
        .unwrap();
    permit
        .before_send(&f.request(route, "private-key", BODY))
        .await
        .unwrap();
    f.saved(&permit.creation.as_ref().unwrap().id).await
}
async fn parent(f: &Fixture, creation: &AssistantCmaCreation) -> AssistantCmaSandbox {
    sandboxes::get_owned(&f.db, &f.owner, &creation.sandbox_ref_id)
        .await
        .unwrap()
}
fn request(row: &admin::ReservationRow, action: admin::ResolveAction) -> admin::ResolveRequest {
    admin::ResolveRequest {
        generation: row.generation,
        revision: row.revision,
        creation_revision: row.creation_revision,
        action,
        remote_sandbox_id: Some("sandbox_1".into()),
    }
}
async fn assert_gets_only(f: &Fixture) {
    let requests = f.server.received_requests().await.unwrap();
    assert!(
        requests.iter().all(|r| r.method == "GET"),
        "admin must never replay, create or mutate CMA"
    );
}

async fn assert_admin_reads_unmetered(f: &Fixture) {
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"service_request"})
            .await
            .unwrap(),
        0
    );
    for collection in [
        crate::models::usage_meter::COLLECTION_NAME,
        crate::models::billing_ledger::COLLECTION_NAME,
        crate::models::billing_wallet::COLLECTION_NAME,
        crate::models::usage_allowance_period::COLLECTION_NAME,
    ] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
}

#[tokio::test]
async fn admin_cma_list_redacts_recovery_material_and_pages_every_holding_class() {
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let mut row = parent(&f, &creation).await;
    let mut ids = Vec::new();
    for (state, remote, retire, expected) in [
        (
            SandboxAdmissionState::Unknown,
            None,
            None,
            "creation_unknown",
        ),
        (
            SandboxAdmissionState::Reserved,
            None,
            None,
            "creation_in_progress",
        ),
        (
            SandboxAdmissionState::Associated,
            Some("known"),
            None,
            "live_sandbox",
        ),
        (
            SandboxAdmissionState::Retiring,
            Some("known2"),
            Some(Utc::now()),
            "retiring",
        ),
    ] {
        row.admission_state = state;
        row.cma_sandbox_id = remote.map(str::to_owned);
        row.retire_requested_at = retire;
        row.submission_state = if state == SandboxAdmissionState::Unknown {
            "unknown"
        } else {
            "pending"
        }
        .into();
        f.db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&creation.id},
                doc! {"$set":{"submission_state":&row.submission_state}},
            )
            .await
            .unwrap();
        f.db.collection::<AssistantCmaSandbox>(PARENTS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        let items = admin::list(
            &f.db,
            Some(&f.owner),
            Some(&[state.as_str().into()]),
            None,
            1,
        )
        .await
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].holding, expected);
        assert_eq!(items[0].creation_revision, Some(creation.revision));
        let encoded = serde_json::to_string(&items).unwrap();
        for secret in [
            "private merchant request",
            "private-key",
            "request_ciphertext",
            "request_hash",
            "creation_ref",
        ] {
            assert!(!encoded.contains(secret));
        }
        assert!(
            admin::list(&f.db, None, None, Some(&row.id), 1)
                .await
                .unwrap()
                .is_empty()
        );
        ids.push(items[0].id.clone());
    }
    assert!(ids.iter().all(|id| id == &row.id));
    assert!(
        admin::list(&f.db, None, Some(&["unrecognized".into()]), None, 10)
            .await
            .is_err()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_unknown_inspection_never_replays_and_does_not_invent_absence() {
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let before = parent(&f, &creation).await;
    let first = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(matches!(
        first.outcome,
        admin::InspectionOutcome::Unknown {
            reason: "owner_retry_will_recover"
        }
    ));
    assert!(
        first
            .next_step
            .unwrap()
            .contains("owner's next allocating create")
    );
    f.db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&creation.id},
            doc! {"$unset":{"first_dispatched_at":""}},
        )
        .await
        .unwrap();
    let legacy = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(matches!(
        legacy.outcome,
        admin::InspectionOutcome::Unknown {
            reason: "creation_dispatch_time_unknown"
        }
    ));
    f.db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&creation.id},doc! {"$set":{"first_dispatched_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(4))}}).await.unwrap();
    let second = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(matches!(
        second.outcome,
        admin::InspectionOutcome::Unknown {
            reason: "idempotency_window_passed"
        }
    ));
    assert!(
        admin::resolve(
            &f.db,
            &f.gateway(),
            &before.id,
            &request(&second.reservation, admin::ResolveAction::Release),
            &actor(&f)
        )
        .await
        .is_err()
    );
    let after = parent(&f, &creation).await;
    assert!(after.occupies_account_slot);
    assert_eq!(after.admission_state, before.admission_state);
    assert_eq!(after.request_ciphertext, before.request_ciphertext);
    assert_gets_only(&f).await;
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_lifecycle_inspection_keeps_parent_until_explicit_association() {
    let f = Fixture::with_management(false).await;
    f.observations().await;
    let creation = pending(&f, "/api/v1/agents").await;
    let creation = change_creation(
        &f.db,
        &creation,
        CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await
    .unwrap();
    let before = parent(&f, &creation).await;
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(
        matches!(inspected.outcome,admin::InspectionOutcome::Exists{ref remote_sandbox_id,..} if remote_sandbox_id=="sandbox_1")
    );
    assert!(inspected.reservation.remote_sandbox_id.is_none());
    assert!(parent(&f, &creation).await.cma_sandbox_id.is_none());
    let associated = admin::resolve(
        &f.db,
        &f.gateway(),
        &before.id,
        &request(&inspected.reservation, admin::ResolveAction::Associate),
        &actor(&f),
    )
    .await
    .unwrap();
    assert_eq!(associated.remote_sandbox_id.as_deref(), Some("sandbox_1"));
    assert!(associated.occupies_account_slot);
    assert!(
        f.saved(&creation.id).await.request_ciphertext.is_some(),
        "parent existence does not complete Agent creation"
    );
    let error = admin::resolve(
        &f.db,
        &f.gateway(),
        &before.id,
        &request(&associated, admin::ResolveAction::Release),
        &actor(&f),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error,AppError::Conflict(ref message) if message.contains("workspace") && message.contains("owner"))
    );
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_known_sandbox_observation_never_retires_local_state() {
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let creation = change_creation(
        &f.db,
        &creation,
        CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into())),
    )
    .await
    .unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/sandboxes/sandbox_1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"sandbox_1","owner":f.owner,"status":"retired"})),
        )
        .mount(&f.server)
        .await;
    let before = parent(&f, &creation).await;
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(matches!(
        inspected.outcome,
        admin::InspectionOutcome::Exists {
            state: SandboxObservationState::Retired,
            ..
        }
    ));
    let after = parent(&f, &creation).await;
    assert!(after.occupies_account_slot);
    assert_eq!(after.admission_state, before.admission_state);
    assert_eq!(after.retired_at, before.retired_at);
    assert!(
        admin::resolve(
            &f.db,
            &f.gateway(),
            &before.id,
            &request(&inspected.reservation, admin::ResolveAction::Associate),
            &actor(&f)
        )
        .await
        .is_err()
    );
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_release_requires_durable_proof_and_clears_both_records_atomically() {
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let before = parent(&f, &creation).await;
    let unknown = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(
        admin::resolve(
            &f.db,
            &f.gateway(),
            &before.id,
            &request(&unknown.reservation, admin::ResolveAction::Release),
            &actor(&f)
        )
        .await
        .is_err()
    );
    // Seed an authoritative durable proof with an unfurled parent projection.
    // Admin never accepts proof text supplied by its caller.
    f.db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&creation.id},
            doc! {"$set":{"submission_state":"confirmed_not_created"}},
        )
        .await
        .unwrap();
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(matches!(
        inspected.outcome,
        admin::InspectionOutcome::ProvenAbsent {
            evidence: "confirmed_not_created"
        }
    ));
    assert!(
        parent(&f, &creation).await.occupies_account_slot,
        "inspect must not release"
    );
    let released = admin::resolve(
        &f.db,
        &f.gateway(),
        &before.id,
        &request(&inspected.reservation, admin::ResolveAction::Release),
        &actor(&f),
    )
    .await
    .unwrap();
    assert!(!released.occupies_account_slot);
    assert_eq!(released.holding, "released");
    assert!(parent(&f, &creation).await.request_ciphertext.is_none());
    assert!(f.saved(&creation.id).await.request_ciphertext.is_none());
    let again = admin::resolve(
        &f.db,
        &f.gateway(),
        &before.id,
        &request(&released, admin::ResolveAction::Release),
        &actor(&f),
    )
    .await
    .unwrap();
    assert!(!again.occupies_account_slot);
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"admin_cma_sandbox_reservation_resolved"})
            .await
            .unwrap(),
        2
    );
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_resolve_fences_generation_revision_and_erasure_in_the_transaction() {
    let f = Fixture::with_management(false).await;
    f.observations().await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let creation = change_creation(
        &f.db,
        &creation,
        CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into())),
    )
    .await
    .unwrap();
    let before = parent(&f, &creation).await;
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    for field in ["generation", "revision", "creation_revision"] {
        let mut body = request(&inspected.reservation, admin::ResolveAction::Associate);
        match field {
            "generation" => body.generation += 1,
            "revision" => body.revision += 1,
            _ => body.creation_revision = body.creation_revision.map(|revision| revision + 1),
        };
        assert!(
            admin::resolve(&f.db, &f.gateway(), &before.id, &body, &actor(&f))
                .await
                .is_err()
        );
    }
    let captured = parent(&f, &creation).await;
    let captured_creation = f.saved(&creation.id).await;
    change_creation(&f.db, &captured_creation, CreationChange::Checked)
        .await
        .unwrap();
    let remote = f
        .gateway()
        .get_sandbox_authority(&f.owner, "sandbox_1")
        .await
        .unwrap();
    assert!(
        reservation_admin::commit(
            &f.db,
            &captured,
            Some(&captured_creation),
            reservation_admin::Change::Associate(remote),
            &actor(&f),
            json!({})
        )
        .await
        .is_err()
    );
    assert!(parent(&f, &creation).await.cma_sandbox_id.is_none());
    let latest = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    f.db.collection::<Document>(crate::services::erasure_admission::COLLECTION)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"erasure_accepted":true}},
        )
        .upsert(true)
        .await
        .unwrap();
    assert!(
        admin::resolve(
            &f.db,
            &f.gateway(),
            &before.id,
            &request(&latest.reservation, admin::ResolveAction::Associate),
            &actor(&f)
        )
        .await
        .is_err()
    );
    assert!(
        admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
            .await
            .is_err()
    );
    assert!(parent(&f, &creation).await.cma_sandbox_id.is_none());
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_reads_refuse_changed_deployment_and_foreign_remote_owner() {
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let creation = change_creation(
        &f.db,
        &creation,
        CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into())),
    )
    .await
    .unwrap();
    let before = parent(&f, &creation).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/sandboxes/sandbox_1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"sandbox_1","owner":"foreign","status":"active"})),
        )
        .mount(&f.server)
        .await;
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(matches!(
        inspected.outcome,
        admin::InspectionOutcome::Unknown { .. }
    ));
    f.db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.catalog.id},
            doc! {"$set":{"identity_include_email":true}},
        )
        .await
        .unwrap();
    let count = f.server.received_requests().await.unwrap().len();
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(matches!(
        inspected.outcome,
        admin::InspectionOutcome::Unknown {
            reason: "cma_configuration_changed"
        }
    ));
    assert_eq!(f.server.received_requests().await.unwrap().len(), count);
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_occupied_audit_is_atomic_hourly_and_shared_between_app_states() {
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/agents").await;
    let held = parent(&f, &creation).await;
    let one = crate::test_utils::test_app_state(f.db.clone());
    let two = crate::test_utils::test_app_state(f.db.clone());
    use crate::services::assistant_cma_slot_audit::{self as throttle, ReplicaCache};
    one.db
        .collection::<Document>(throttle::COLLECTION)
        .create_indexes(throttle::indexes())
        .await
        .unwrap();
    let cache_one = ReplicaCache::default();
    let cache_two = ReplicaCache::default();
    let (a, b) = tokio::join!(
        throttle::record_with_cache(&one.db, &held, &cache_one),
        throttle::record_with_cache(&two.db, &held, &cache_two)
    );
    a.unwrap();
    b.unwrap();
    let err = f
        .admit(&f.target, "/api/v1/agents", "different", BODY)
        .await
        .err()
        .unwrap();
    assert!(
        err.response_body().message == "Conflict: cma_sandbox_account_slot_occupied"
    );
    let logs =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(
        logs.count_documents(doc! {"event_type":"cma_sandbox_slot_occupied"})
            .await
            .unwrap(),
        1
    );
    let log = logs
        .find_one(doc! {"event_type":"cma_sandbox_slot_occupied"})
        .await
        .unwrap()
        .unwrap();
    let encoded = serde_json::to_string(&log).unwrap();
    for secret in [
        "private-key",
        "private merchant",
        "request_ciphertext",
        "request_hash",
    ] {
        assert!(!encoded.contains(secret));
    }
    f.db.collection::<Document>(throttle::COLLECTION).update_one(doc! {"owner_id":&held.user_id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(2))}}).await.unwrap();
    // Independent replica after the server-side hour has elapsed.
    throttle::record_with_cache(&one.db, &held, &ReplicaCache::default())
        .await
        .unwrap();
    assert_eq!(
        logs.count_documents(doc! {"event_type":"cma_sandbox_slot_occupied"})
            .await
            .unwrap(),
        2
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_direct_association_confirms_only_the_exact_owner_sandbox() {
    let f = Fixture::with_management(false).await;
    f.observations().await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let before = parent(&f, &creation).await;
    let unproven = admin::list(&f.db, Some(&f.owner), None, None, 1)
        .await
        .unwrap()
        .remove(0);
    let refused = admin::resolve(
        &f.db,
        &f.gateway(),
        &before.id,
        &request(&unproven, admin::ResolveAction::Associate),
        &actor(&f),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(refused, AppError::Conflict(ref message) if message.starts_with("cma_creation_provenance_required:"))
    );
    assert!(
        f.server.received_requests().await.unwrap().is_empty(),
        "arbitrary same-owner sandbox is not a receipt"
    );
    let creation = change_creation(
        &f.db,
        &creation,
        CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into())),
    )
    .await
    .unwrap();
    let before = parent(&f, &creation).await;
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    let associated = admin::resolve(
        &f.db,
        &f.gateway(),
        &before.id,
        &request(&inspected.reservation, admin::ResolveAction::Associate),
        &actor(&f),
    )
    .await
    .unwrap();
    assert_eq!(associated.holding, "live_sandbox");
    assert_eq!(associated.remote_sandbox_id.as_deref(), Some("sandbox_1"));
    assert_eq!(f.saved(&creation.id).await.submission_state, "confirmed");
    assert!(f.saved(&creation.id).await.request_ciphertext.is_none());
    assert_eq!(
        parent(&f, &creation).await.request_ciphertext,
        before.request_ciphertext
    );
    assert_admin_reads_unmetered(&f).await;
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"admin_cma_sandbox_reservation_resolved","user_id":&actor(&f).user_id})
            .await
            .unwrap(),
        1
    );
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_missing_or_unavailable_operation_never_proves_absence() {
    for status in [404, 503] {
        let f = Fixture::with_management(false).await;
        let creation = pending(&f, "/api/v1/agents").await;
        let creation = change_creation(
            &f.db,
            &creation,
            CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
        )
        .await
        .unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v1/agent-lifecycle/op_1"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(json!({"private":"must not appear"})),
            )
            .expect(1)
            .mount(&f.server)
            .await;
        let before = parent(&f, &creation).await;
        let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
            .await
            .unwrap();
        assert!(matches!(
            inspected.outcome,
            admin::InspectionOutcome::Unknown { .. }
        ));
        assert!(
            !serde_json::to_string(&inspected)
                .unwrap()
                .contains("must not appear")
        );
        assert!(
            admin::resolve(
                &f.db,
                &f.gateway(),
                &before.id,
                &request(&inspected.reservation, admin::ResolveAction::Release),
                &actor(&f)
            )
            .await
            .is_err()
        );
        assert!(parent(&f, &creation).await.occupies_account_slot);
        assert_eq!(
            f.saved(&creation.id).await.request_ciphertext,
            creation.request_ciphertext
        );
        assert_gets_only(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn admin_cma_release_respects_erasure_and_no_resource_receipt_fence() {
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let before = parent(&f, &creation).await;
    for receipt in ["operation_id", "cma_agent_id", "cma_sandbox_id"] {
        let mut set = doc! {"submission_state":"confirmed_not_created"};
        set.insert(receipt, "inconsistent-receipt");
        f.db.collection::<Document>(COLLECTION_NAME)
            .update_one(doc! {"_id":&creation.id}, doc! {"$set":set})
            .await
            .unwrap();
        let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
            .await
            .unwrap();
        assert!(!matches!(
            inspected.outcome,
            admin::InspectionOutcome::ProvenAbsent { .. }
        ));
        let parent_before = bson::to_document(&parent(&f, &creation).await).unwrap();
        let creation_before = bson::to_document(&f.saved(&creation.id).await).unwrap();
        let error = admin::resolve(
            &f.db,
            &f.gateway(),
            &before.id,
            &request(&inspected.reservation, admin::ResolveAction::Release),
            &actor(&f),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, AppError::Conflict(ref message)
            if message.starts_with("cma_creation_absence_proof_required:")),
            "{error}"
        );
        assert_eq!(
            bson::to_document(&parent(&f, &creation).await).unwrap(),
            parent_before
        );
        assert_eq!(
            bson::to_document(&f.saved(&creation.id).await).unwrap(),
            creation_before
        );
        let mut clear = Document::new();
        clear.insert(receipt, bson::Bson::Null);
        f.db.collection::<Document>(COLLECTION_NAME)
            .update_one(doc! {"_id":&creation.id}, doc! {"$set":clear})
            .await
            .unwrap();
    }
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    f.db.collection::<Document>(crate::services::erasure_admission::COLLECTION)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"erasure_accepted":true}},
        )
        .upsert(true)
        .await
        .unwrap();
    let requests_before = f.server.received_requests().await.unwrap().len();
    let error = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(ref message)
        if message == "Account erasure is in progress for this owner; inspection is unavailable"));
    assert_eq!(
        f.server.received_requests().await.unwrap().len(),
        requests_before
    );
    assert!(
        admin::resolve(
            &f.db,
            &f.gateway(),
            &before.id,
            &request(&inspected.reservation, admin::ResolveAction::Release),
            &actor(&f)
        )
        .await
        .is_err()
    );
    assert!(parent(&f, &creation).await.occupies_account_slot);
    assert!(f.saved(&creation.id).await.request_ciphertext.is_some());
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_association_uses_existing_metadata_fold_without_changing_inspect() {
    let f = Fixture::with_management(false).await;
    f.observations().await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let creation = change_creation(
        &f.db,
        &creation,
        CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into())),
    )
    .await
    .unwrap();
    let before = parent(&f, &creation).await;
    let mut discovery = before.clone();
    discovery.id = uuid::Uuid::new_v4().to_string();
    discovery.source = "discovered".into();
    discovery.cma_sandbox_id = Some("sandbox_1".into());
    discovery.occupies_account_slot = false;
    discovery.generation = 8;
    discovery.idempotency_key = None;
    discovery.request_hash = None;
    discovery.request_ciphertext = None;
    discovery.request_context_version = None;
    discovery.retire_requested_at = Some(Utc::now());
    f.db.collection::<AssistantCmaSandbox>(PARENTS)
        .insert_one(&discovery)
        .await
        .unwrap();
    let agents =
        f.db.collection::<Document>(crate::models::assistant_cma_agent::COLLECTION_NAME);
    agents.insert_one(doc! {"_id":"dependent-agent","user_id":&f.owner,"cma_service_id":&before.cma_service_id,"cma_sandbox_id":"sandbox_1","sandbox_generation":discovery.generation}).await.unwrap();
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert_eq!(
        inspected.reservation.remote_sandbox_id.as_deref(),
        Some("sandbox_1")
    );
    assert!(!inspected.reservation.associated);
    assert!(
        f.db.collection::<Document>(PARENTS)
            .find_one(doc! {"_id":&discovery.id})
            .await
            .unwrap()
            .is_some(),
        "inspection cannot fold/delete a reference"
    );
    let associated = admin::resolve(
        &f.db,
        &f.gateway(),
        &before.id,
        &request(&inspected.reservation, admin::ResolveAction::Associate),
        &actor(&f),
    )
    .await
    .unwrap();
    assert!(associated.associated);
    assert_eq!(associated.holding, "retiring");
    assert!(
        f.db.collection::<Document>(PARENTS)
            .find_one(doc! {"_id":&discovery.id})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        agents
            .find_one(doc! {"_id":"dependent-agent"})
            .await
            .unwrap()
            .unwrap()
            .get_i64("sandbox_generation")
            .unwrap(),
        before.generation
    );
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_association_never_folds_a_distinct_creation_intent() {
    let f = Fixture::with_management(false).await;
    f.observations().await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let creation = change_creation(
        &f.db,
        &creation,
        CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into())),
    )
    .await
    .unwrap();
    let before = parent(&f, &creation).await;
    let mut other = before.clone();
    other.id = uuid::Uuid::new_v4().to_string();
    other.cma_sandbox_id = Some("sandbox_1".into());
    other.occupies_account_slot = false;
    other.idempotency_key = Some("other-private-key".into());
    f.db.collection::<AssistantCmaSandbox>(PARENTS)
        .insert_one(&other)
        .await
        .unwrap();
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(
        admin::resolve(
            &f.db,
            &f.gateway(),
            &before.id,
            &request(&inspected.reservation, admin::ResolveAction::Associate),
            &actor(&f)
        )
        .await
        .is_err()
    );
    assert!(parent(&f, &creation).await.cma_sandbox_id.is_none());
    assert!(
        f.db.collection::<Document>(PARENTS)
            .find_one(doc! {"_id":&other.id})
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        f.saved(&creation.id).await.request_ciphertext,
        creation.request_ciphertext
    );
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_legacy_live_parent_refuses_release_with_workspace_guidance() {
    let f = Fixture::with_management(false).await;
    f.observations().await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let before = parent(&f, &creation).await;
    f.db.collection::<Document>(COLLECTION_NAME)
        .delete_one(doc! {"_id":&creation.id})
        .await
        .unwrap();
    f.db.collection::<Document>(PARENTS).update_one(doc! {"_id":&before.id},doc! {"$set":{"cma_sandbox_id":"sandbox_1","admission_state":"associated","source":"claimed"}}).await.unwrap();
    let inspected = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
        .await
        .unwrap();
    assert!(inspected.reservation.creation_revision.is_none());
    assert!(inspected.reservation.associated);
    let err = admin::resolve(
        &f.db,
        &f.gateway(),
        &before.id,
        &request(&inspected.reservation, admin::ResolveAction::Release),
        &actor(&f),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err,AppError::Conflict(ref message) if message.contains("workspace") && message.contains("owner"))
    );
    assert!(
        admin::get(&f.db, &before.id)
            .await
            .unwrap()
            .occupies_account_slot
    );
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_occupied_audit_survives_same_reservation_reacquisition() {
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let held = parent(&f, &creation).await;
    sandboxes::audit_occupied_slot(&f.db, &held).await.unwrap();
    change_creation(&f.db, &creation, CreationChange::NoResource)
        .await
        .unwrap();
    let retry =
        sandboxes::begin_creation(&f.db, &f.keys, &f.gateway(), &f.owner, "private-key", BODY)
            .await
            .unwrap();
    assert_eq!(retry.id, held.id);
    assert!(retry.generation > held.generation);
    sandboxes::audit_occupied_slot(&f.db, &retry).await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"cma_sandbox_slot_occupied"})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_inspection_is_audit_only_on_success_and_failure_and_preserves_owner_cas() {
    let f = Fixture::with_management(false).await;
    f.observations().await;
    let creation = pending(&f, "/api/v1/sandboxes").await;
    let creation = change_creation(
        &f.db,
        &creation,
        CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into())),
    )
    .await
    .unwrap();
    let before = parent(&f, &creation).await;
    let parents = f.db.collection::<Document>(PARENTS);
    let creations = f.db.collection::<Document>(COLLECTION_NAME);
    let parent_bson = parents
        .find_one(doc! {"_id":&before.id})
        .await
        .unwrap()
        .unwrap();
    let creation_bson = creations
        .find_one(doc! {"_id":&creation.id})
        .await
        .unwrap()
        .unwrap();
    for failing in [false, true] {
        if failing {
            Mock::given(method("GET"))
                .and(path("/api/v1/sandboxes/sandbox_1"))
                .respond_with(ResponseTemplate::new(503))
                .with_priority(1)
                .mount(&f.server)
                .await;
        }
        let result = admin::inspect(&f.db, &f.gateway(), &before.id, &actor(&f))
            .await
            .unwrap();
        assert_eq!(
            matches!(result.outcome, admin::InspectionOutcome::Unknown { .. }),
            failing
        );
        assert_eq!(
            parents
                .find_one(doc! {"_id":&before.id})
                .await
                .unwrap()
                .unwrap(),
            parent_bson
        );
        assert_eq!(
            creations
                .find_one(doc! {"_id":&creation.id})
                .await
                .unwrap()
                .unwrap(),
            creation_bson
        );
        assert_eq!(result.reservation.revision, before.agent_reference_version);
        assert_eq!(
            result.reservation.creation_revision,
            Some(creation.revision)
        );
    }
    // Owner snapshot captured before either inspection still owns its CAS.
    let matched = parents
        .update_one(
            sandboxes::observation_filter(&before),
            doc! {"$inc":{"agent_reference_version":1_i64}},
        )
        .await
        .unwrap();
    assert_eq!(matched.matched_count, 1);
    let logs =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
    assert_eq!(logs.count_documents(doc! {"event_type":"admin_cma_sandbox_reservation_inspected","user_id":&actor(&f).user_id}).await.unwrap(),2);
    assert_admin_reads_unmetered(&f).await;
    assert_gets_only(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_old_parent_throttle_conflicts_but_isolated_claim_cannot_invalidate_owner_transaction()
 {
    use crate::services::assistant_cma_slot_audit::{self as throttle, ReplicaCache};
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/agents").await;
    let held = parent(&f, &creation).await;
    f.db.collection::<Document>(throttle::COLLECTION)
        .create_indexes(throttle::indexes())
        .await
        .unwrap();
    let parents = f.db.collection::<Document>(PARENTS);
    let mut old_owner = f.db.client().start_session().await.unwrap();
    old_owner.start_transaction().await.unwrap();
    parents
        .find_one(doc! {"_id":&held.id})
        .session(&mut old_owner)
        .await
        .unwrap()
        .unwrap();
    // The reviewed implementation wrote this field in a separate transaction.
    let mut old_audit = f.db.client().start_session().await.unwrap();
    old_audit.start_transaction().await.unwrap();
    parents
        .update_one(
            doc! {"_id":&held.id},
            doc! {"$set":{"occupied_slot_audited_at":bson::DateTime::now()}},
        )
        .session(&mut old_audit)
        .await
        .unwrap();
    old_audit.commit_transaction().await.unwrap();
    let conflict = parents
        .update_one(
            doc! {"_id":&held.id},
            doc! {"$inc":{"agent_reference_version":1_i64}},
        )
        .session(&mut old_owner)
        .await
        .unwrap_err();
    assert!(conflict.contains_label("TransientTransactionError"));
    let _ = old_owner.abort_transaction().await;
    parents
        .update_one(
            doc! {"_id":&held.id},
            doc! {"$unset":{"occupied_slot_audited_at":""}},
        )
        .await
        .unwrap();
    let mut owner = f.db.client().start_session().await.unwrap();
    owner.start_transaction().await.unwrap();
    let captured = parents
        .find_one(doc! {"_id":&held.id})
        .session(&mut owner)
        .await
        .unwrap()
        .unwrap();
    throttle::record_with_cache(&f.db, &held, &ReplicaCache::default())
        .await
        .unwrap();
    assert_eq!(
        parents
            .find_one(doc! {"_id":&held.id})
            .await
            .unwrap()
            .unwrap(),
        captured
    );
    parents
        .update_one(
            doc! {"_id":&held.id},
            doc! {"$inc":{"agent_reference_version":1_i64}},
        )
        .session(&mut owner)
        .await
        .unwrap();
    owner.commit_transaction().await.unwrap();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_cma_slot_audit_loop_adds_no_commands_and_failed_audit_rolls_back_claim() {
    use crate::services::assistant_cma_slot_audit::{self as throttle, ReplicaCache};
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let commands = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "slot_audit_commands",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                seen.lock().unwrap().push(event.command_name);
            }
        }),
    )
    .await;
    let f = Fixture::with_management(false).await;
    let creation = pending(&f, "/api/v1/agents").await;
    let held = parent(&f, &creation).await;
    db.collection::<Document>(throttle::COLLECTION)
        .create_indexes(throttle::indexes())
        .await
        .unwrap();
    db.create_collection(crate::models::audit_log::COLLECTION_NAME)
        .await
        .unwrap();
    db.run_command(doc! {"collMod":crate::models::audit_log::COLLECTION_NAME,"validator":{"event_type":{"$ne":"cma_sandbox_slot_occupied"}},"validationAction":"error"}).await.unwrap();
    let failed_cache = ReplicaCache::default();
    assert!(
        throttle::record_with_cache(&db, &held, &failed_cache)
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<Document>(throttle::COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    commands.lock().unwrap().clear();
    for _ in 0..100 {
        throttle::record_with_cache(&db, &held, &failed_cache)
            .await
            .unwrap();
    }
    assert!(
        commands.lock().unwrap().is_empty(),
        "failures also suppress retry-loop load"
    );
    db.run_command(doc! {"collMod":crate::models::audit_log::COLLECTION_NAME,"validator":{}})
        .await
        .unwrap();
    let cache = ReplicaCache::default();
    throttle::record_with_cache(&db, &held, &cache)
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    for _ in 0..1000 {
        throttle::record_with_cache(&db, &held, &cache)
            .await
            .unwrap();
    }
    assert!(commands.lock().unwrap().is_empty());
    let other_replica = ReplicaCache::default();
    throttle::record_with_cache(&db, &held, &other_replica)
        .await
        .unwrap();
    assert_eq!(
        commands
            .lock()
            .unwrap()
            .iter()
            .filter(|name| name.as_str() == "update")
            .count(),
        1,
        "another replica makes one conditional claim"
    );
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"cma_sandbox_slot_occupied"})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
    f.db.drop().await.unwrap();
}
