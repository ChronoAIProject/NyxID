//! Revision4 contract: Kubernetes observations are explicit fixtures here;
//! Billing, transactions, exact reservations, and lifecycle admission are real.
use super::*;
use crate::models::remote_machine::NativeRuntimeObservation;

fn observed(machine: &ManagedMachine, at: DateTime<Utc>) -> NativeRuntimeObservation {
    NativeRuntimeObservation {
        sandbox_uid: machine.sandbox_uid.clone().unwrap(),
        pod_uid: machine.pod_uid.clone(),
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        writer_fence: machine.writer_fence.clone(),
        operation_id: machine.active_operation_id.clone().unwrap(),
        sandbox_generation: 3,
        sandbox_resource_version: "native-fixture-rv-3".into(),
        observed_at: DateTime::from_timestamp_millis(at.timestamp_millis()).unwrap(),
        running: true,
        suspended: false,
        stop_confirmed: false,
        run_not_after: machine.funded_until,
        evidence_ref: format!("native-fixture:{}", Uuid::new_v4()),
    }
}

async fn persist(
    f: &Fixture,
    machine: &ManagedMachine,
    observation: &NativeRuntimeObservation,
) -> ManagedMachine {
    f.db.collection::<Document>(MACHINES).update_one(doc!{"_id":&machine.node_id},
        doc!{"$set":{"native_observation":bson::to_bson(observation).unwrap(),"provider_coverage":bson::Bson::Null,"native_uncertainty":bson::Bson::Null}})
        .await.unwrap();
    current(f, &machine.node_id).await
}

#[test]
fn native_cutoff_requires_current_fresh_applied_identity_and_bounds_funding() {
    let now = Utc::now();
    let mut machine = CreateMachineInput::fixed(&Uuid::new_v4().to_string(), false, "fixture")
        .preview(&Uuid::new_v4().to_string(), now);
    machine.sandbox_uid = Some(Uuid::new_v4().to_string());
    machine.pod_uid = Some(Uuid::new_v4().to_string());
    machine.runtime_generation = 1;
    machine.credential_epoch = 1;
    machine.active_operation_id = Some(Uuid::new_v4().to_string());
    machine.funded_until = Some(now + Duration::seconds(90));
    let mut observation = observed(&machine, now);
    observation.run_not_after = Some(now + Duration::seconds(120));
    machine.native_observation = Some(observation.clone());
    assert_eq!(effective_execution_cutoff(&machine), machine.funded_until);
    machine.native_uncertainty = Some("node_unavailable".into());
    assert!(effective_execution_cutoff(&machine).is_none());
    machine.native_uncertainty = None;
    for invalid in 0..7 {
        let mut value = observation.clone();
        match invalid {
            0 => value.operation_id = Uuid::new_v4().to_string(),
            1 => value.sandbox_uid = Uuid::new_v4().to_string(),
            2 => value.pod_uid = Some(Uuid::new_v4().to_string()),
            3 => value.observed_at = now - Duration::seconds(61),
            4 => value.suspended = true,
            5 => value.stop_confirmed = true,
            _ => value.sandbox_generation = 0,
        }
        machine.native_observation = Some(value);
        assert!(
            effective_execution_cutoff(&machine).is_none(),
            "invalid case {invalid}"
        );
    }
}

#[tokio::test]
async fn native_running_and_stop_settle_exactly_once_without_unknown_tail_charge() {
    let f = fixture("native_runtime_exact_stop").await;
    let machine = start_ready(&f).await;
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let runs = f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION);
    let before = runs
        .find_one(doc! {"_id":&machine.runtime_run_id})
        .await
        .unwrap()
        .unwrap();
    let observation = observed(&machine, Utc::now());
    let machine = persist(&f, &machine, &observation).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &machine, &observation)
        .await
        .unwrap();
    let running = runs
        .find_one(doc! {"_id":&machine.runtime_run_id})
        .await
        .unwrap()
        .unwrap();
    assert!(running.proven_until_ns >= before.proven_until_ns);
    let paused = action(
        &f,
        &machine,
        LifecycleAction::Pause,
        json!({"action":"pause"}),
    )
    .await
    .unwrap()
    .machine;
    let mut stop = observed(&paused, Utc::now());
    stop.running = false;
    stop.suspended = true;
    stop.stop_confirmed = true;
    let paused = persist(&f, &paused, &stop).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &paused, &stop)
        .await
        .unwrap();
    let settled = runs
        .find_one(doc! {"_id":&machine.runtime_run_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.state, MachineRuntimeRunState::Closed);
    assert_eq!(settled.proven_until_ns, running.proven_until_ns);
    assert!(settled.stop_is_upper_bound);
    let balance = wallet(&f).await.balance_credits;
    native_observe_and_settle(&f.db, f.billing.clone(), &paused, &stop)
        .await
        .unwrap();
    assert_eq!(wallet(&f).await.balance_credits, balance);
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_unknown_or_stale_observation_cannot_release_funding_or_open_execution() {
    let f = fixture("native_runtime_unknown").await;
    let machine = start_ready(&f).await;
    let held = wallet(&f).await.reserved_credits;
    let mut observation = observed(&machine, Utc::now());
    observation.running = false;
    let persisted = persist(&f, &machine, &observation).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &persisted, &observation)
        .await
        .unwrap();
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert!(effective_execution_cutoff(&persisted).is_none());
    observation.stop_confirmed = true;
    observation.suspended = true;
    assert!(
        native_observe_and_settle(&f.db, f.billing.clone(), &persisted, &observation)
            .await
            .is_err()
    );
    let current = persist(&f, &machine, &observation).await;
    f.db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id":&machine.node_id},
            doc! {"$set":{"native_uncertainty":"known_force_delete"}},
        )
        .await
        .unwrap();
    assert!(
        native_observe_and_settle(&f.db, f.billing.clone(), &current, &observation)
            .await
            .is_err()
    );
    assert_eq!(wallet(&f).await.reserved_credits, held);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_deadline_renewals_share_one_lifecycle_operation_without_index_collision() {
    use crate::models::remote_machine_outbox::{
        COLLECTION_NAME as OUTBOX, EffectKind, MachineEffectOutbox,
    };
    let f = fixture("native_runtime_outbox_renewals").await;
    // Ensure the real lifecycle partial unique index is installed explicitly;
    // a renewal must coexist with CreateSandbox and subsequent renewals.
    crate::services::remote_machine_operation_service::ensure_indexes(&f.db)
        .await
        .unwrap();
    let mut input = CreateMachineInput::fixed(&f.owner, true, "fixture-image");
    input.idempotency_key = Uuid::new_v4().to_string();
    let created = lifecycle::create_machine(&f.db, input)
        .await
        .unwrap()
        .machine;
    let machine = lifecycle::advance_runtime_generation(
        &f.db,
        &created.node_id,
        created.revision,
        0,
        None,
        None,
    )
    .await
    .unwrap();
    f.db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id":&machine.node_id},
            doc! {"$set":{
                "sandbox_uid":Uuid::new_v4().to_string(),"pod_uid":Uuid::new_v4().to_string(),
                "pvc_uid":Uuid::new_v4().to_string(),"resource_state":"ready","writer_admitted":true
            }},
        )
        .await
        .unwrap();
    let machine = current(&f, &machine.node_id).await;
    let mut machine = install_test_coverage(
        &f.db,
        &machine,
        Utc::now() + Duration::seconds(window_seconds()),
    )
    .await
    .unwrap();
    let operation = machine.active_operation_id.clone().unwrap();
    for _ in 0..3 {
        // The queue accepts an explicit clock so the test can advance free
        // safety windows without sleeping. The controller observation remains
        // an explicitly declared fixture; no signed receipt is involved.
        let now =
            machine.native_run_not_after.unwrap() - Duration::seconds(renewal_margin_seconds());
        let mut session = f.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            queue_native_deadline_in_session(&f.db, &mut session, &machine, false, now)
                .await
                .unwrap()
        );
        session.commit_transaction().await.unwrap();
        machine = current(&f, &machine.node_id).await;
        let effects = f.db.collection::<MachineEffectOutbox>(OUTBOX);
        let effect=effects.find_one(doc!{"operation_id":&operation,"kind":"update_sandbox_deadline",
            "effect_digest":format!("native-shutdown:{}:{}:false:{}",machine.runtime_generation,machine.credential_epoch,
                machine.native_run_not_after.unwrap().timestamp_millis())}).await.unwrap().unwrap();
        assert_eq!(effect.kind, EffectKind::UpdateSandboxDeadline);
        assert_eq!(effect.desired_mode, "running");
        let mut replay = f.db.client().start_session().await.unwrap();
        replay.start_transaction().await.unwrap();
        assert!(
            !queue_native_deadline_in_session(&f.db, &mut replay, &machine, false, now)
                .await
                .unwrap()
        );
        replay.commit_transaction().await.unwrap();
        let cutoff = machine.native_run_not_after.unwrap();
        machine = install_test_coverage(&f.db, &machine, cutoff)
            .await
            .unwrap();
    }
    assert_eq!(
        f.db.collection::<Document>(OUTBOX)
            .count_documents(doc! {"operation_id":&operation,"kind":"create_sandbox"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.collection::<Document>(OUTBOX)
            .count_documents(doc! {"operation_id":&operation,"kind":"update_sandbox_deadline"})
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        f.db.collection::<Document>(OUTBOX)
            .count_documents(doc! {"operation_id":&operation,"kind":"resume_sandbox"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_delete_records_resource_cleanup_while_accounting_waits_then_settles_once() {
    use crate::services::remote_machine_manager::Manager;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    let f = fixture("native_cleanup_accounting_separation").await;
    let machine = start_ready(&f).await;
    // Establish a nonzero observed running interval using the real clock and
    // Billing core. Only the Kubernetes facts are fixture inputs.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (machine, running) = native_fixture(&f, &machine, true, false).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &machine, &running)
        .await
        .unwrap();
    let held = wallet(&f).await.reserved_credits;
    assert!(held > Credits::ZERO);
    let deleting = action(
        &f,
        &machine,
        LifecycleAction::Delete,
        json!({"action":"delete"}),
    )
    .await
    .unwrap()
    .machine;
    assert!(
        deleting.deletion_basis.is_some(),
        "real owner deletion acceptance is retained"
    );
    f.db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id":&deleting.node_id},
            doc! {"$set":{
                "native_pod_name":"node","native_pv_name":"native-cleanup-pv",
                "native_pv_uid":Uuid::new_v4().to_string(),"native_pv_reclaim_policy":"Delete"
            }},
        )
        .await
        .unwrap();
    let deleting = current(&f, &deleting.node_id).await;
    // This test starts at an already accepted native stop observation. The
    // separate Manager journeys cover acquisition of that observation from
    // Suspended condition + Pod/Node identity. No physical receipt is supplied.
    let (deleting, _) = native_fixture(&f, &deleting, false, true).await;
    let kube = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(|request: &wiremock::Request| {
            let path = request.url.path();
            assert!(!path.contains("/persistentvolumes"));
            assert!(!path.contains("/volumeattachments"));
            assert_ne!(path, "/api/v1/pods");
            if path.ends_with("/pods") || path.ends_with("/persistentvolumeclaims") {
                let kind = if path.ends_with("/pods") {
                    "PodList"
                } else {
                    "PersistentVolumeClaimList"
                };
                ResponseTemplate::new(200).set_body_json(json!({
                    "kind":kind,"apiVersion":"v1",
                    "metadata":{"resourceVersion":"complete-native-cleanup-fixture"},"items":[]
                }))
            } else {
                // Original Sandbox and known namespace children are absent.
                ResponseTemplate::new(404)
            }
        })
        .mount(&kube)
        .await;
    let mut manager = Manager {
        db: f.db.clone(),
        billing: None,
        api: nyxid_machine_updater::sandbox::SandboxApi::new(kube.uri().parse().unwrap(), None)
            .unwrap(),
        storage_class: Some("local-fixture-csi".into()),
        namespace: "nyxid-machine".into(),
        bootstrap_base_url: "https://nyxid.test".into(),
    };
    assert!(Box::pin(manager.observe(&deleting)).await.is_err());
    let pending = current(&f, &deleting.node_id).await;
    assert_eq!(pending.cleanup_state, "complete");
    let resource_evidence = pending.delete_recovery.as_ref().unwrap();
    assert!(resource_evidence.resources_complete);
    assert!(!resource_evidence.evidence_id.is_empty());
    assert_eq!(
        resource_evidence.evidence_version,
        crate::services::remote_machine_delete_recovery::EVIDENCE_VERSION
    );
    assert_eq!(
        resource_evidence.storage,
        crate::models::remote_machine::DeleteStorageDisposition::SandboxCascadeObserved
    );
    assert_eq!(
        pending.cleanup_evidence_digest.as_deref(),
        Some(resource_evidence.evidence_id.as_str()),
        "resource cleanup remains durably visible before accounting finality"
    );
    assert_eq!(pending.resource_state, ResourceState::Deleting);
    // An unavailable accounting consumer preserves the accepted billing state;
    // require_accounting_final below proves that the run still cannot finalize.
    assert_eq!(pending.billing_state, deleting.billing_state);
    assert!(!pending.writer_admitted);
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert!(require_accounting_final(&f.db, &pending).await.is_err());
    assert_eq!(
        lifecycle::list_machines(&f.db, &f.owner)
            .await
            .unwrap()
            .len(),
        1,
        "pending accounting retains the live Machine record"
    );
    let pending_operation =
        f.db.collection::<crate::models::remote_machine_operation::MachineOperation>(OPERATIONS)
            .find_one(doc! {"_id":&pending.active_operation_id})
            .await
            .unwrap()
            .unwrap();
    assert!(pending_operation.terminal_at.is_none());
    let node =
        f.db.collection::<Node>(crate::models::node::COLLECTION_NAME)
            .find_one(doc! {"_id":&pending.node_id})
            .await
            .unwrap()
            .unwrap();
    assert!(!node.is_active);
    assert!(node.signing_secret_encrypted.is_none());
    let requests = kube.received_requests().await.unwrap();
    for suffix in [
        format!("/sandboxes/{}", pending.sandbox_name),
        "/pods".into(),
        "/pods/node".into(),
        "/persistentvolumeclaims".into(),
        "/persistentvolumeclaims/workspace".into(),
    ] {
        assert!(
            requests
                .iter()
                .any(|request| request.url.path().ends_with(&suffix)),
            "cleanup must observe {suffix}"
        );
    }

    manager.billing = Some(f.billing.clone());
    // The accounting dependency recovered. Advance only this original accepted
    // delete's retry clock; resource evidence and settlement identity stay fixed.
    let retry =
        f.db.collection::<Document>(OPERATIONS)
            .update_one(
                doc! {"_id":&pending.active_operation_id,"owner_id":&pending.owner_id,
                "machine_id":&pending.node_id,"action":"delete","terminal_at":bson::Bson::Null},
                doc! {"$set":{"next_attempt_at":bson::Bson::Null}},
            )
            .await
            .unwrap();
    assert_eq!(retry.matched_count, 1);
    Box::pin(manager.observe(&pending)).await.unwrap();
    let deleted = current(&f, &pending.node_id).await;
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    assert_eq!(deleted.billing_state, BillingState::Closed);
    assert_eq!(deleted.cleanup_state, "complete");
    assert_eq!(
        deleted.cleanup_evidence_digest,
        pending.cleanup_evidence_digest
    );
    assert_eq!(deleted.active_operation_id, pending.active_operation_id);
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert!(
        lifecycle::list_machines(&f.db, &f.owner)
            .await
            .unwrap()
            .is_empty()
    );
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&deleted.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.state, MachineRuntimeRunState::Closed);
    assert!(run.cumulative_seconds >= 1);
    assert!(run.stop_is_upper_bound);
    let settled_wallet = wallet(&f).await;
    let ledger_count =
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap();
    assert!(ledger_count > 0);
    Box::pin(manager.observe(&deleted)).await.unwrap();
    let replayed_wallet = wallet(&f).await;
    assert_eq!(
        replayed_wallet.balance_credits,
        settled_wallet.balance_credits
    );
    assert_eq!(
        replayed_wallet.pending_lago_debits,
        settled_wallet.pending_lago_debits
    );
    assert_eq!(replayed_wallet.reserved_credits, Credits::ZERO);
    assert_eq!(
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        ledger_count
    );
    f.db.drop().await.unwrap();
}
