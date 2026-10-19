use chrono::Utc;
use serde_json::json;

use crate::models::remote_machine::{AuthorizationState, BillingState, ResourceState};
use crate::services::remote_machine_service::{
    CreateMachineInput, RuntimeIdentity, can_write_runtime, canonical_idempotency_digest,
    observation_state, safe_view,
};

#[test]
fn canonical_idempotency_digest_is_stable_and_body_bound() {
    let first = canonical_idempotency_digest(
        "owner-1",
        "create",
        None,
        "key-1",
        &json!({"profile":"fixed","free":true}),
    );
    let same = canonical_idempotency_digest(
        "owner-1",
        "create",
        None,
        "key-1",
        &json!({"free":true,"profile":"fixed"}),
    );
    let changed = canonical_idempotency_digest(
        "owner-1",
        "create",
        None,
        "key-1",
        &json!({"profile":"fixed","free":false}),
    );
    assert_eq!(first, same);
    assert_ne!(first, changed);
    assert_eq!(first.len(), 64);
}

#[test]
fn observations_preserve_pending_lifecycle_states() {
    assert_eq!(
        observation_state(ResourceState::Stopping),
        ResourceState::Stopping
    );
    assert_eq!(
        observation_state(ResourceState::Deleting),
        ResourceState::Deleting
    );
    assert_eq!(
        observation_state(ResourceState::Paused),
        ResourceState::Paused
    );
    assert_eq!(
        observation_state(ResourceState::Ready),
        ResourceState::Reconciling
    );
}

#[test]
fn runtime_writer_requires_exact_generation_epoch_and_ready_state() {
    let current = RuntimeIdentity {
        generation: 3,
        credential_epoch: 8,
        writer_fence: "fence-a",
    };
    assert!(can_write_runtime(
        ResourceState::Ready,
        AuthorizationState::Active,
        current,
        current,
    ));
    for expected in [
        RuntimeIdentity {
            generation: 2,
            ..current
        },
        RuntimeIdentity {
            credential_epoch: 7,
            ..current
        },
        RuntimeIdentity {
            writer_fence: "fence-b",
            ..current
        },
    ] {
        assert!(!can_write_runtime(
            ResourceState::Ready,
            AuthorizationState::Active,
            current,
            expected,
        ));
    }
    assert!(!can_write_runtime(
        ResourceState::Stopping,
        AuthorizationState::Active,
        current,
        current,
    ));
    assert!(!can_write_runtime(
        ResourceState::Ready,
        AuthorizationState::Revoked,
        current,
        current,
    ));
}

#[test]
fn safe_view_contains_states_and_never_secret_material() {
    let input = CreateMachineInput::fixed("owner-1", true, "sha256:image");
    let mut machine = input.preview("node-1", Utc::now());
    machine.resource_state = ResourceState::Ready;
    machine.billing_state = BillingState::Free;
    let view = safe_view(&machine);
    assert_eq!(view.node_id, "node-1");
    assert_eq!(view.resource_state, ResourceState::Ready);
    let encoded = serde_json::to_string(&view).expect("safe view json");
    assert!(!encoded.contains("token"));
    assert!(!encoded.contains("secret"));
}

use crate::{
    models::{
        machine_free_slot::{FreeSlotState, MachineFreeSlot},
        remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine, NativeRuntimeObservation},
        remote_machine_operation::{COLLECTION_NAME as OPERATIONS, LifecycleAction},
        remote_machine_outbox::{COLLECTION_NAME as OUTBOX, MachineEffectOutbox},
        user::UserType,
    },
    services::{
        remote_machine_native as native, remote_machine_operation_service as operations,
        remote_machine_service as lifecycle,
    },
    test_utils::{connect_transaction_test_database, test_user},
};
use mongodb::{
    Database,
    bson::{Document, doc},
};

async fn lifecycle_fixture(label: &str) -> (Database, String) {
    let db = connect_transaction_test_database(label).await;
    lifecycle::ensure_indexes(&db).await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    let mut user = test_user(&owner, UserType::Person);
    user.email_verified = false; // Free qualification adds no email gate.
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(user)
        .await
        .unwrap();
    (db, owner)
}

async fn native_fixture(db: &Database, id: &str, running: bool, stopped: bool) -> ManagedMachine {
    let machine = lifecycle::managed_machine_for_node(db, id)
        .await
        .unwrap()
        .unwrap();
    let observation = NativeRuntimeObservation {
        sandbox_uid: machine.sandbox_uid.clone().unwrap(),
        pod_uid: machine.pod_uid.clone(),
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        writer_fence: machine.writer_fence.clone(),
        operation_id: machine.active_operation_id.clone().unwrap(),
        sandbox_generation: machine
            .native_observation
            .as_ref()
            .map_or(1, |o| o.sandbox_generation + 1),
        sandbox_resource_version: uuid::Uuid::new_v4().to_string(),
        observed_at: chrono::DateTime::from_timestamp_millis(Utc::now().timestamp_millis())
            .unwrap(),
        running,
        suspended: stopped,
        stop_confirmed: stopped,
        run_not_after: machine.funded_until.or(machine.native_run_not_after),
        evidence_ref: format!("native-kubernetes-fixture:{}", uuid::Uuid::new_v4()),
    };
    native::record(db, &machine, &observation, Document::new())
        .await
        .unwrap()
}

fn creation(owner: &str, key: &str) -> CreateMachineInput {
    let mut input =
        CreateMachineInput::fixed(owner, true, "registry.test/nyxid@sha256:0123456789abcdef");
    input.idempotency_key = key.into();
    input
}

fn action(
    machine: &ManagedMachine,
    kind: LifecycleAction,
    key: &str,
) -> lifecycle::LifecycleRequest {
    lifecycle::LifecycleRequest {
        owner_id: machine.owner_id.clone(),
        machine_id: machine.node_id.clone(),
        action: kind,
        expected_revision: Some(machine.revision),
        idempotency_key: key.into(),
        request_body: json!({"action":kind.as_str(),"expected_revision":machine.revision,"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true}),
        now: None,
    }
}

#[test]
fn fixed_profile_rejects_custom_allocations() {
    let mut profile = lifecycle::fixed_profile("image@sha256:abc");
    lifecycle::validate_fixed_profile(&profile).unwrap();
    for invalid in ["", ".", "-bad", "bad-", "a..b", "UPPER"] {
        let mut changed = profile.clone();
        changed.runtime_class = Some(invalid.into());
        assert!(lifecycle::validate_fixed_profile(&changed).is_err());
    }
    let mut saved = profile.clone();
    saved.runtime_class = Some("saved-runtime-a".into());
    lifecycle::validate_fixed_profile(&saved).unwrap();
    profile.workspace_gib = 40;
    assert!(lifecycle::validate_fixed_profile(&profile).is_err());
    profile = lifecycle::fixed_profile("image@sha256:abc");
    profile.profile_version = "unapproved".into();
    assert!(lifecycle::validate_fixed_profile(&profile).is_err());
}

#[tokio::test]
async fn remote_machine_create_is_atomic_replayable_and_body_bound() {
    let (db, owner) = lifecycle_fixture("remote_create_replay").await;
    let input = creation(&owner, "same-intent");
    let created = Box::pin(lifecycle::create_machine(&db, input.clone()))
        .await
        .unwrap();
    let replay = Box::pin(lifecycle::create_machine(&db, input.clone()))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(created.machine.node_id, replay.machine.node_id);
    assert_eq!(created.operation.id, replay.operation.id);
    let mut unsafe_metadata = created.operation.clone();
    unsafe_metadata.error_code = Some("raw/provider/error".into());
    unsafe_metadata.error_message = Some("sensitive-provider-error-body".into());
    unsafe_metadata.evidence_digest = Some("sensitive-provider-reference".into());
    let encoded = serde_json::to_string(&lifecycle::safe_operation_view(&unsafe_metadata)).unwrap();
    assert!(!encoded.contains("sensitive"));
    assert!(!encoded.contains("raw/provider"));
    let node = db
        .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id": &created.machine.node_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(node.user_id, owner);
    assert!(!node.is_active);
    assert!(node.signing_secret_encrypted.is_none());
    assert_eq!(
        db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(OPERATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(OUTBOX)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let mut changed = input;
    changed.request_body["name"] = json!("changed-name");
    assert!(
        Box::pin(lifecycle::create_machine(&db, changed))
            .await
            .unwrap_err()
            .to_string()
            .contains("idempotency_key_reused")
    );
}

#[tokio::test]
async fn remote_machine_concurrent_free_creates_occupy_one_slot() {
    let (db, owner) = lifecycle_fixture("remote_free_race").await;
    let first = creation(&owner, "first");
    let second = creation(&owner, "second");
    let (a, b) = tokio::join!(
        Box::pin(lifecycle::create_machine(&db, first)),
        Box::pin(lifecycle::create_machine(&db, second))
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(crate::models::node::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(OUTBOX)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let slot = db
        .collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
        .find_one(doc! {"_id": &owner})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(slot.state, FreeSlotState::Reserved);
    assert!(
        !lifecycle::entitlement(&db, &owner)
            .await
            .unwrap()
            .free_slot_available
    );
}

#[tokio::test]
async fn remote_machine_concurrent_same_intent_returns_same_operation() {
    let (db, owner) = lifecycle_fixture("remote_idempotency_race").await;
    let input = creation(&owner, "shared-intent");
    let (a, b) = tokio::join!(
        Box::pin(lifecycle::create_machine(&db, input.clone())),
        Box::pin(lifecycle::create_machine(&db, input))
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.machine.node_id, b.machine.node_id);
    assert_eq!(a.operation.id, b.operation.id);
    assert_ne!(a.replayed, b.replayed);
}

#[tokio::test]
async fn remote_machine_account_exception_pauses_once_without_releasing_storage() {
    let (db, owner) = lifecycle_fixture("remote_account_safety").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    db.collection::<Document>(MACHINES).update_one(doc! {"_id": &created.machine.node_id}, doc! {"$set":{"resource_state":"ready","pvc_uid":"retained-workspace","writer_admitted":true}}).await.unwrap();
    db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": &owner}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert_eq!(
        Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
        1
    );
    assert_eq!(
        Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
        0
    );
    let machine = lifecycle::get_machine(&db, &owner, &created.machine.node_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(machine.resource_state, ResourceState::Stopping);
    assert_eq!(machine.authorization_state, AuthorizationState::Revoked);
    assert!(!machine.writer_admitted);
    assert_eq!(machine.pvc_uid.as_deref(), Some("retained-workspace"));
    assert_eq!(machine.billing_state, BillingState::Free);
    let slot = db
        .collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
        .find_one(doc! {"_id": &owner})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(slot.state, FreeSlotState::Released);
    let audit = db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"machine_account_safety_pause"})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        audit.get("user_id"),
        Some(mongodb::bson::Bson::Null)
    ));
    assert!(audit.get_i64("seq").unwrap() > 0);
    db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(doc! {"_id": &owner}, doc! {"$set":{"is_active":true}})
        .await
        .unwrap();
    assert_eq!(
        Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
        0
    );
    assert_eq!(
        lifecycle::get_machine(&db, &owner, &machine.node_id)
            .await
            .unwrap()
            .unwrap()
            .resource_state,
        ResourceState::Stopping
    );
}

#[tokio::test]
async fn administrative_or_missing_owner_never_authorizes_workspace_erasure() {
    for admin_delete in [false, true] {
        for paused in [false, true] {
            let (db, owner) = lifecycle_fixture("remote_unconsented_owner_removal").await;
            let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
                .await
                .unwrap();
            let id = &created.machine.node_id;
            db.collection::<Document>(MACHINES)
                .update_one(
                    doc! {"_id":id},
                    doc! {"$set":{
                        "resource_state":if paused {"paused"} else {"ready"},
                        "authorization_state":"active", "writer_admitted":!paused,
                        "pvc_uid":"retained-workspace", "sandbox_uid":"retained-sandbox"
                    }},
                )
                .await
                .unwrap();
            if admin_delete {
                crate::services::admin_user_service::delete_user_cascade(
                    &db,
                    &uuid::Uuid::new_v4().to_string(),
                    &owner,
                )
                .await
                .unwrap();
            } else {
                db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                    .delete_one(doc! {"_id":&owner})
                    .await
                    .unwrap();
            }
            Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap();
            assert_eq!(
                Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
                0
            );
            let retained = lifecycle::managed_machine_for_node(&db, id)
                .await
                .unwrap()
                .unwrap();
            assert_ne!(
                retained.resource_state,
                ResourceState::Deleting,
                "admin deletion or owner absence is not consent"
            );
            assert_eq!(retained.authorization_state, AuthorizationState::Revoked);
            assert!(!retained.writer_admitted);
            assert_eq!(retained.pvc_uid.as_deref(), Some("retained-workspace"));
            assert_eq!(
                db.collection::<Document>(OUTBOX)
                    .count_documents(doc! {"machine_id":id,"kind":"delete_sandbox"})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                db.collection::<Document>(OPERATIONS)
                    .count_documents(doc! {"machine_id":id,"action":"delete"})
                    .await
                    .unwrap(),
                0
            );
            let slot = db
                .collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap()
                .unwrap();
            assert_ne!(slot.state, FreeSlotState::Released);
            db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn previously_queued_unconsented_delete_cannot_finalize_or_release_slot() {
    let (db, owner) = lifecycle_fixture("remote_old_unconsented_delete").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let deleted = Box::pin(lifecycle::request_action(
        &db,
        lifecycle::LifecycleRequest {
            owner_id: owner.clone(),
            machine_id: created.machine.node_id.clone(),
            action: LifecycleAction::Delete,
            expected_revision: Some(created.machine.revision),
            idempotency_key: "old-delete".into(),
            request_body: json!({"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true}),
            now: None,
        },
    ))
    .await
    .unwrap();
    let id = &created.machine.node_id;
    db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id":id},
            doc! {"$unset":{"deletion_basis":""},"$set":{"stop_reason":"owner_not_found"}},
        )
        .await
        .unwrap();
    db.collection::<Document>(OPERATIONS).update_one(doc! {"_id":&deleted.operation.id},doc! {"$set":{"idempotency_key_hash":"system:old-account-safety","error_code":"owner_not_found"}}).await.unwrap();
    db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .delete_one(doc! {"_id":&owner})
        .await
        .unwrap();
    let result = lifecycle::finalize_unallocated_delete(&db, id).await;
    let outcome = match &result {
        Ok(Some(_)) => "finalized",
        Ok(None) => "pending",
        Err(crate::errors::AppError::Conflict(code)) => code,
        Err(_) => "other_error",
    };
    assert!(
        matches!(&result, Err(crate::errors::AppError::Conflict(code)) if code=="machine_owner_deletion_basis_required"),
        "unconsented deletion must report its authorization failure: {outcome}"
    );
    assert_eq!(
        Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
        1
    );
    assert_eq!(
        Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
        0
    );
    let retained = lifecycle::managed_machine_for_node(&db, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.resource_state, ResourceState::Stopping);
    assert_eq!(retained.authorization_state, AuthorizationState::Revoked);
    assert!(retained.deletion_basis.is_none());
    let slot = db
        .collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
        .find_one(doc! {"_id":&owner})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(slot.state, FreeSlotState::Released);
    assert_eq!(
        db.collection::<Document>(OPERATIONS)
            .count_documents(doc! {"machine_id":id,"action":"pause"})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn wrong_owner_or_unrecognized_deletion_basis_retains_workspace() {
    for wrong_owner in [false, true] {
        let (db, owner) = lifecycle_fixture("remote_invalid_deletion_basis").await;
        let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
            .await
            .unwrap();
        let id = &created.machine.node_id;
        db.collection::<Document>(MACHINES).update_one(doc! {"_id":id},doc! {"$set":{
            "resource_state":"paused","pvc_uid":"retained-workspace",
            "deletion_basis":{"owner_id":if wrong_owner {uuid::Uuid::new_v4().to_string()} else {owner.clone()},
                "source":"account_owner","policy_version":if wrong_owner {"machine-data"} else {"unrecognized"},"accepted_at":bson::DateTime::now()}
        }}).await.unwrap();
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .delete_one(doc! {"_id":&owner})
            .await
            .unwrap();
        assert_eq!(
            Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
            1
        );
        assert_eq!(
            Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
            0
        );
        let retained = lifecycle::managed_machine_for_node(&db, id)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(retained.resource_state, ResourceState::Deleting);
        assert_eq!(retained.authorization_state, AuthorizationState::Revoked);
        assert_eq!(retained.pvc_uid.as_deref(), Some("retained-workspace"));
        assert_eq!(
            db.collection::<Document>(OPERATIONS)
                .count_documents(doc! {"machine_id":id,"action":"delete"})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn account_deletion_cascade_queues_cleanup_for_an_already_paused_workspace() {
    let (db, owner) = lifecycle_fixture("remote_deleted_owner").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let machine_id = &created.machine.node_id;
    db.collection::<Document>(MACHINES).update_one(
        doc! {"_id":machine_id},
        doc! {"$set":{"resource_state":"paused","pvc_uid":"retained-workspace","sandbox_uid":"existing-sandbox","runtime_generation":1_i64,"writer_admitted":false}},
    ).await.unwrap();
    let job_id = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>(crate::models::machine_job::COLLECTION_NAME)
        .insert_one(doc! {"_id":&job_id,"user_id":&owner,"node_id":machine_id,"status":"running"})
        .await
        .unwrap();
    crate::services::admin_user_service::delete_current_user_cascade(
        &db,
        &owner,
        crate::services::admin_user_service::ACCOUNT_DELETION_POLICY_VERSION,
    )
    .await
    .unwrap();
    assert!(
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.collection::<Document>(crate::models::machine_job::COLLECTION_NAME)
            .find_one(doc! {"_id":&job_id})
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
        1
    );
    assert_eq!(
        Box::pin(lifecycle::suspend_ineligible(&db)).await.unwrap(),
        0
    );
    let machine = lifecycle::managed_machine_for_node(&db, machine_id)
        .await
        .unwrap()
        .unwrap();
    let basis = machine
        .deletion_basis
        .as_ref()
        .expect("surviving explicit owner consent");
    assert_eq!(basis.owner_id, owner);
    assert_eq!(
        basis.source,
        crate::models::remote_machine::MachineDeletionSource::AccountOwner
    );
    assert_eq!(
        basis.policy_version,
        crate::services::admin_user_service::ACCOUNT_DELETION_POLICY_VERSION
    );
    assert_eq!(machine.resource_state, ResourceState::Deleting);
    assert_eq!(machine.authorization_state, AuthorizationState::Revoked);
    assert_eq!(machine.pvc_uid.as_deref(), Some("retained-workspace"));
    assert!(
        lifecycle::finalize_unallocated_delete(&db, machine_id)
            .await
            .unwrap()
            .is_none()
    );
    let slot = db
        .collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
        .find_one(doc! {"_id":&owner})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(slot.state, FreeSlotState::Released);
    let operation = db
        .collection::<Document>(OPERATIONS)
        .find_one(doc! {"_id":&machine.active_operation_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(operation.get_str("action").unwrap(), "delete");
    let effect = db
        .collection::<Document>(OUTBOX)
        .find_one(doc! {"operation_id":&machine.active_operation_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(effect.get_str("kind").unwrap(), "delete_sandbox");
    db.drop().await.unwrap();
}

#[tokio::test]
async fn remote_machine_native_observation_allows_bootstrap_before_ready_only() {
    let (db, owner) = lifecycle_fixture("remote_bootstrap_admission").await;
    let sandbox = uuid::Uuid::new_v4().to_string();
    let pod = uuid::Uuid::new_v4().to_string();
    let workspace = uuid::Uuid::new_v4().to_string();
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let machine = lifecycle::advance_runtime_generation(
        &db,
        &created.machine.node_id,
        0,
        0,
        Some(pod.as_str()),
        None,
    )
    .await
    .unwrap();
    db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id": &machine.node_id},
            doc! {"$set":{"sandbox_uid":&sandbox,"pvc_uid":&workspace}},
        )
        .await
        .unwrap();
    let machine = lifecycle::managed_machine_for_node(&db, &machine.node_id)
        .await
        .unwrap()
        .unwrap();
    let machine = crate::services::remote_machine_runtime::install_test_coverage(
        &db,
        &machine,
        Utc::now() + chrono::Duration::minutes(2),
    )
    .await
    .unwrap();
    let mut stopped = machine.clone();
    let observation = stopped.native_observation.as_mut().unwrap();
    observation.running = false;
    observation.suspended = true;
    observation.stop_confirmed = true;
    assert!(native::activate(&db, &stopped).await.is_err());
    let admitted = native::activate(&db, &machine).await.unwrap();
    assert!(lifecycle::runtime_fence_allows_bootstrap(&admitted));
    assert_ne!(admitted.resource_state, ResourceState::Ready);
    assert!(
        !lifecycle::runtime_fence_allows_writer(
            &db,
            &machine.node_id,
            machine.runtime_generation,
            machine.credential_epoch,
            &machine.writer_fence
        )
        .await
        .unwrap()
    );
    let rekeyed = Box::pin(lifecycle::rekey_bootstrap(
        &db,
        &admitted.node_id,
        admitted.revision,
        None,
    ))
    .await
    .unwrap();
    assert!(!lifecycle::runtime_fence_allows_bootstrap(&rekeyed));
    assert!(rekeyed.last_runtime_evidence.is_none());
    assert_ne!(rekeyed.credential_epoch, admitted.credential_epoch);
    let node = db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id": &machine.node_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!node.get_bool("is_active").unwrap());
    assert!(node.get_i64("machine_authority_admission_fence").unwrap() > 0);
}

#[tokio::test]
async fn remote_machine_ineligible_owner_and_paid_creation_leave_no_resources() {
    let (db, owner) = lifecycle_fixture("remote_qualification").await;
    let users = db.collection::<Document>(crate::models::user::COLLECTION_NAME);
    users
        .update_one(doc! {"_id": &owner}, doc! {"$set": {"is_active":false}})
        .await
        .unwrap();
    assert!(
        Box::pin(lifecycle::create_machine(&db, creation(&owner, "inactive")))
            .await
            .is_err()
    );
    users
        .update_one(
            doc! {"_id": &owner},
            doc! {"$set": {"is_active":true,"user_type":"org"}},
        )
        .await
        .unwrap();
    assert!(
        Box::pin(lifecycle::create_machine(&db, creation(&owner, "org")))
            .await
            .is_err()
    );
    users
        .update_one(
            doc! {"_id": &owner},
            doc! {"$set": {"is_active":true,"user_type":"person"}},
        )
        .await
        .unwrap();
    assert!(lifecycle::entitlement(&db, &owner).await.unwrap().eligible);
    let mut paid = creation(&owner, "paid-missing-quote");
    paid.free_slot = false;
    paid.profile.image_digest = format!("registry.test/nyxid@sha256:{}", "a".repeat(64));
    paid.request_body = json!({"free_slot":false,"accept_account_balance":true,"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true});
    assert!(matches!(
        Box::pin(lifecycle::create_machine(&db, paid.clone())).await,
        Err(crate::errors::AppError::ValidationError(_))
    ));
    let quote_id = uuid::Uuid::new_v4().to_string();
    paid.idempotency_key = "paid-missing-acceptance".into();
    paid.request_body = json!({"free_slot":false,"quote_id":&quote_id,"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true});
    assert!(matches!(
        Box::pin(lifecycle::create_machine(&db, paid.clone())).await,
        Err(crate::errors::AppError::ValidationError(_))
    ));
    paid.idempotency_key = "paid-missing-billing".into();
    paid.request_body = json!({"free_slot":false,"quote_id":quote_id,"accept_account_balance":true,"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true});
    assert!(matches!(
        Box::pin(lifecycle::create_machine(&db, paid)).await,
        Err(crate::errors::AppError::BillingProviderUnavailable(_))
    ));
    for collection in [
        MACHINES,
        lifecycle::FREE_SLOTS,
        OPERATIONS,
        OUTBOX,
        crate::models::node::COLLECTION_NAME,
        crate::models::billing_wallet::COLLECTION_NAME,
        crate::models::usage_meter::COLLECTION_NAME,
        crate::models::machine_runtime_billing::QUOTE_COLLECTION,
        crate::models::machine_runtime_billing::RUN_COLLECTION,
        crate::models::machine_runtime_billing::INTERVAL_COLLECTION,
        crate::models::machine_runtime_billing::HEAD_COLLECTION,
    ] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "rejected admission must leave no rows in {collection}"
        );
    }
}

#[tokio::test]
async fn remote_machine_actions_use_owner_revision_and_replay_cas() {
    let (db, owner) = lifecycle_fixture("remote_action_cas").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let deletion = action(&created.machine, LifecycleAction::Delete, "delete");
    let accepted = Box::pin(lifecycle::request_action(&db, deletion.clone()))
        .await
        .unwrap();
    assert_eq!(accepted.machine.resource_state, ResourceState::Deleting);
    let replay = Box::pin(lifecycle::request_action(&db, deletion))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(accepted.operation.id, replay.operation.id);
    assert!(
        Box::pin(lifecycle::request_action(
            &db,
            action(&created.machine, LifecycleAction::Delete, "stale")
        ))
        .await
        .unwrap_err()
        .to_string()
        .contains("revision_conflict")
    );
    let mut foreign = action(&accepted.machine, LifecycleAction::Delete, "foreign");
    foreign.owner_id = uuid::Uuid::new_v4().to_string();
    assert!(
        Box::pin(lifecycle::request_action(&db, foreign))
            .await
            .is_err()
    );
    assert!(
        !lifecycle::entitlement(&db, &owner)
            .await
            .unwrap()
            .free_slot_available
    );
}

#[tokio::test]
async fn remote_machine_outbox_reclaims_expired_lease_and_rejects_stale_ack() {
    let (db, owner) = lifecycle_fixture("remote_effect_claim").await;
    Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let now = Utc::now();
    let first = operations::claim_effect(&db, now, std::time::Duration::from_secs(1))
        .await
        .unwrap()
        .unwrap();
    assert!(
        operations::claim_effect(&db, now, std::time::Duration::from_secs(1))
            .await
            .unwrap()
            .is_none()
    );
    let second = operations::claim_effect(
        &db,
        now + chrono::Duration::seconds(2),
        std::time::Duration::from_secs(1),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.id, second.id);
    assert_ne!(first.claim_id, second.claim_id);
    assert!(
        !operations::acknowledge_effect(
            &db,
            &first.id,
            first.claim_id.as_deref().unwrap(),
            true,
            None,
            now
        )
        .await
        .unwrap()
    );
    assert!(
        operations::acknowledge_effect(
            &db,
            &second.id,
            second.claim_id.as_deref().unwrap(),
            true,
            None,
            now
        )
        .await
        .unwrap()
    );
    let stored = db
        .collection::<MachineEffectOutbox>(OUTBOX)
        .find_one(doc! {"_id": &second.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.state,
        crate::models::remote_machine_outbox::EffectState::Applied
    );
}

#[tokio::test]
async fn remote_machine_observation_preserves_identity_and_cannot_admit_writer() {
    let (db, owner) = lifecycle_fixture("remote_observation").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let observation = lifecycle::RuntimeObservationInput {
        machine_id: created.machine.node_id.clone(),
        sandbox_uid: "sandbox-a".into(),
        pod_uid: Some("pod-a".into()),
        pvc_name: Some("workspace".into()),
        pvc_uid: Some("pvc-a".into()),
        ready: true,
        physically_stopped: false,
        source: "sandbox".into(),
        now: None,
    };
    let observed = lifecycle::apply_runtime_observation(&db, observation.clone())
        .await
        .unwrap();
    assert_eq!(observed.resource_state, ResourceState::Reconciling);
    assert!(
        !lifecycle::runtime_fence_allows_writer(
            &db,
            &observed.node_id,
            observed.runtime_generation,
            observed.credential_epoch,
            &observed.writer_fence
        )
        .await
        .unwrap()
    );
    let mut missing = observation.clone();
    missing.pod_uid = None;
    missing.pvc_name = None;
    missing.pvc_uid = None;
    let remembered = lifecycle::apply_runtime_observation(&db, missing)
        .await
        .unwrap();
    assert_eq!(remembered.pod_uid.as_deref(), Some("pod-a"));
    assert_eq!(remembered.pvc_uid.as_deref(), Some("pvc-a"));
    let mut drifted = observation.clone();
    drifted.pvc_uid = Some("pvc-b".into());
    assert!(
        lifecycle::apply_runtime_observation(&db, drifted)
            .await
            .unwrap_err()
            .to_string()
            .contains("pvc_uid_mismatch")
    );
    let mut drifted = observation;
    drifted.sandbox_uid = "replacement".into();
    assert!(
        lifecycle::apply_runtime_observation(&db, drifted)
            .await
            .unwrap_err()
            .to_string()
            .contains("sandbox_uid_mismatch")
    );
}

#[tokio::test]
async fn remote_machine_bootstrap_replay_and_generation_are_fenced() {
    let (db, owner) = lifecycle_fixture("remote_bootstrap_fence").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let id = &created.machine.node_id;
    assert_eq!(
        lifecycle::accept_bootstrap_delivery(&db, id, 0, 0, "delivery-1", "digest-1")
            .await
            .unwrap(),
        lifecycle::BootstrapDeliveryResult::Accepted
    );
    assert_eq!(
        lifecycle::accept_bootstrap_delivery(&db, id, 0, 0, "delivery-1", "digest-1")
            .await
            .unwrap(),
        lifecycle::BootstrapDeliveryResult::Replay
    );
    assert_eq!(
        lifecycle::accept_bootstrap_delivery(&db, id, 0, 0, "delivery-2", "digest-2")
            .await
            .unwrap(),
        lifecycle::BootstrapDeliveryResult::RekeyRequired
    );
    let current = lifecycle::get_machine(&db, &owner, id)
        .await
        .unwrap()
        .unwrap();
    let next =
        lifecycle::advance_runtime_generation(&db, id, current.revision, 0, Some("new-pod"), None)
            .await
            .unwrap();
    assert!(next.bootstrap_delivery_id.is_none());
    assert_eq!(
        lifecycle::accept_bootstrap_delivery(&db, id, 0, 0, "delivery-1", "digest-1")
            .await
            .unwrap(),
        lifecycle::BootstrapDeliveryResult::RekeyRequired
    );
    assert!(
        lifecycle::advance_runtime_generation(&db, id, next.revision, 1, Some("another-pod"), None)
            .await
            .unwrap_err()
            .to_string()
            .contains("native_predecessor_stop_required")
    );
}

#[tokio::test]
async fn remote_machine_delete_requires_bound_complete_cleanup_and_releases_atomically() {
    let (db, owner) = lifecycle_fixture("remote_cleanup_finality").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let allocated = lifecycle::advance_runtime_generation(
        &db,
        &created.machine.node_id,
        0,
        0,
        Some("pod"),
        None,
    )
    .await
    .unwrap();
    let deleting = Box::pin(lifecycle::request_action(
        &db,
        action(&allocated, LifecycleAction::Delete, "delete"),
    ))
    .await
    .unwrap()
    .machine;
    db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id": &deleting.node_id},
            doc! {"$set":{"sandbox_uid":"sandbox","pod_uid":"pod","pvc_uid":"pvc"}},
        )
        .await
        .unwrap();
    let not_stopped = native_fixture(&db, &deleting.node_id, false, false).await;
    assert!(
        native::finalize_stop(&db, &not_stopped, None)
            .await
            .is_err()
    );
    let stopped = native_fixture(&db, &deleting.node_id, false, true).await;
    assert!(native::finalize_stop(&db, &stopped, None).await.is_err());
    assert!(
        native::finalize_stop(&db, &stopped, Some(""))
            .await
            .is_err()
    );
    let mut wrong_binding = stopped.clone();
    wrong_binding
        .native_observation
        .as_mut()
        .unwrap()
        .credential_epoch += 1;
    assert!(
        native::finalize_stop(&db, &wrong_binding, Some("native-cleanup-fixture"))
            .await
            .is_err()
    );
    assert!(
        !lifecycle::entitlement(&db, &owner)
            .await
            .unwrap()
            .free_slot_available
    );
    let mut wrong_pvc = stopped.clone();
    wrong_pvc.pvc_uid = Some("replacement-pvc".into());
    assert!(
        native::finalize_stop(&db, &wrong_pvc, Some("native-cleanup-fixture"))
            .await
            .is_err()
    );
    let deleted = native::finalize_stop(&db, &stopped, Some("native-cleanup-fixture"))
        .await
        .unwrap();
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    assert_eq!(deleted.billing_state, BillingState::Closed);
    assert!(
        lifecycle::entitlement(&db, &owner)
            .await
            .unwrap()
            .free_slot_available
    );
    let slot = db
        .collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
        .find_one(doc! {"_id": &owner})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(slot.state, FreeSlotState::Released);
    assert_eq!(
        slot.release_evidence_digest,
        deleted.cleanup_evidence_digest
    );
}

#[tokio::test]
async fn remote_machine_pause_retains_workspace_slot_and_managed_mutation_guard() {
    let (db, owner) = lifecycle_fixture("remote_pause_retention").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let allocated = lifecycle::advance_runtime_generation(
        &db,
        &created.machine.node_id,
        0,
        0,
        Some("pod"),
        None,
    )
    .await
    .unwrap();
    db.collection::<Document>(MACHINES).update_one(doc! {"_id": &created.machine.node_id}, doc! {"$set":{"sandbox_uid":"sandbox","pod_uid":"pod","pvc_uid":"workspace-retained","resource_state":"stopping","cleanup_state":"pending"}}).await.unwrap();
    let stopped = native_fixture(&db, &allocated.node_id, false, true).await;
    let paused = native::finalize_stop(&db, &stopped, None).await.unwrap();
    assert_eq!(paused.resource_state, ResourceState::Paused);
    assert_eq!(paused.cleanup_state, "not_required");
    assert_eq!(paused.pvc_uid.as_deref(), Some("workspace-retained"));
    assert!(
        !lifecycle::entitlement(&db, &owner)
            .await
            .unwrap()
            .free_slot_available
    );
    for operation in ["delete", "rotate", "transfer", "update"] {
        assert!(
            lifecycle::guard_legacy_node_mutation(&db, &paused.node_id, operation)
                .await
                .is_err()
        );
    }
    db.collection::<Document>(MACHINES)
        .delete_one(doc! {"_id": &paused.node_id})
        .await
        .unwrap();
    assert!(
        lifecycle::guard_legacy_node_mutation(&db, &paused.node_id, "delete")
            .await
            .unwrap_err()
            .to_string()
            .contains("mapping_missing")
    );
}

#[tokio::test]
async fn remote_machine_rekey_is_idempotent_and_waits_for_old_writer_stop() {
    let (db, owner) = lifecycle_fixture("remote_rekey_operation").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let allocated = lifecycle::advance_runtime_generation(
        &db,
        &created.machine.node_id,
        0,
        0,
        Some("pod"),
        None,
    )
    .await
    .unwrap();
    db.collection::<Document>(MACHINES).update_one(doc! {"_id": &allocated.node_id}, doc! {"$set":{"resource_state":"ready","writer_admitted":true,"sandbox_uid":"sandbox","pvc_uid":"persistent-workspace","bootstrap_delivery_id":"old-delivery","bootstrap_digest":"old-digest"}}).await.unwrap();
    let current = lifecycle::get_machine(&db, &owner, &allocated.node_id)
        .await
        .unwrap()
        .unwrap();
    let request = action(&current, LifecycleAction::Rekey, "rekey-retry");
    let (first, second) = tokio::join!(
        Box::pin(lifecycle::request_action(&db, request.clone())),
        Box::pin(lifecycle::request_action(&db, request))
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.operation.id, second.operation.id);
    assert_eq!(first.machine.node_id, current.node_id);
    assert_eq!(first.machine.pvc_uid, current.pvc_uid);
    assert_eq!(first.machine.runtime_generation, current.runtime_generation);
    assert_eq!(first.machine.credential_epoch, current.credential_epoch);
    assert_eq!(
        first.operation.desired_runtime_generation,
        Some(current.runtime_generation + 1)
    );
    assert!(!first.machine.writer_admitted);
    assert!(first.machine.bootstrap_delivery_id.is_none());
    assert_eq!(first.machine.resource_state, ResourceState::Stopping);
    assert!(
        lifecycle::advance_runtime_generation(
            &db,
            &current.node_id,
            first.machine.revision,
            current.runtime_generation,
            Some("replacement-pod"),
            None
        )
        .await
        .is_err()
    );
    let stopped = native_fixture(&db, &current.node_id, false, true).await;
    let advanced = lifecycle::advance_runtime_generation(
        &db,
        &current.node_id,
        stopped.revision,
        current.runtime_generation,
        Some("replacement-pod"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(advanced.node_id, current.node_id);
    assert_eq!(advanced.pvc_uid, current.pvc_uid);
    assert_eq!(advanced.runtime_generation, current.runtime_generation + 1);
    assert!(advanced.credential_epoch > current.credential_epoch);
    let predecessor = operations::get_operation(&db, &owner, &created.operation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        predecessor.state,
        crate::models::remote_machine_operation::OperationState::Failed
    );
    assert_eq!(
        predecessor.error_code.as_deref(),
        Some("operation_superseded")
    );
}

#[tokio::test]
async fn remote_machine_never_submitted_delete_finalizes_without_provider_assumptions() {
    let (db, owner) = lifecycle_fixture("remote_unallocated_delete").await;
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    assert!(
        Box::pin(lifecycle::finalize_unallocated_delete(
            &db,
            &created.machine.node_id
        ))
        .await
        .unwrap()
        .is_none()
    );
    let deletion = Box::pin(lifecycle::request_action(
        &db,
        action(&created.machine, LifecycleAction::Delete, "delete"),
    ))
    .await
    .unwrap();
    let deleted = Box::pin(lifecycle::finalize_unallocated_delete(
        &db,
        &created.machine.node_id,
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    assert!(
        lifecycle::entitlement(&db, &owner)
            .await
            .unwrap()
            .free_slot_available
    );
    assert!(
        lifecycle::advance_runtime_generation(
            &db,
            &deleted.node_id,
            deleted.revision,
            0,
            None,
            None
        )
        .await
        .is_err()
    );
    let operation = operations::get_operation(&db, &owner, &deletion.operation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        operation.state,
        crate::models::remote_machine_operation::OperationState::Succeeded
    );
    let predecessor = operations::get_operation(&db, &owner, &created.operation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        predecessor.error_code.as_deref(),
        Some("operation_superseded")
    );
    let second = Box::pin(lifecycle::create_machine(&db, creation(&owner, "second")))
        .await
        .unwrap();
    let allocated = lifecycle::advance_runtime_generation(
        &db,
        &second.machine.node_id,
        0,
        0,
        Some("pod"),
        None,
    )
    .await
    .unwrap();
    Box::pin(lifecycle::request_action(
        &db,
        action(&allocated, LifecycleAction::Delete, "delete-allocated"),
    ))
    .await
    .unwrap();
    assert!(
        Box::pin(lifecycle::finalize_unallocated_delete(
            &db,
            &allocated.node_id
        ))
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        !lifecycle::entitlement(&db, &owner)
            .await
            .unwrap()
            .free_slot_available
    );
}

#[tokio::test]
async fn remote_machine_unallocated_delete_rejects_unresolved_submission_responsibility() {
    let (db, owner) = lifecycle_fixture("delete_unresolved_submission").await;
    let created = lifecycle::create_machine(&db, creation(&owner, "create"))
        .await
        .unwrap();
    let deletion = lifecycle::request_action(
        &db,
        action(&created.machine, LifecycleAction::Delete, "delete"),
    )
    .await
    .unwrap();
    db.collection::<mongodb::bson::Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &created.machine.node_id},
            doc! {"$set": {"runtime_submission_started_at": mongodb::bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert!(
        lifecycle::finalize_unallocated_delete(&db, &created.machine.node_id)
            .await
            .unwrap()
            .is_none(),
        "generation zero must not erase a persisted in-flight submission obligation"
    );
    let stored = lifecycle::get_machine(&db, &owner, &created.machine.node_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.resource_state, ResourceState::Deleting);
    assert_eq!(
        stored.active_operation_id.as_deref(),
        Some(deletion.operation.id.as_str())
    );
    assert!(
        !lifecycle::entitlement(&db, &owner)
            .await
            .unwrap()
            .free_slot_available
    );
}

#[tokio::test]
async fn remote_machine_lost_ready_observation_closes_admission_but_keeps_identity() {
    let (db, owner) = lifecycle_fixture("remote_lost_ready").await;
    let sandbox = uuid::Uuid::new_v4().to_string();
    let pod = uuid::Uuid::new_v4().to_string();
    let workspace = uuid::Uuid::new_v4().to_string();
    let created = Box::pin(lifecycle::create_machine(&db, creation(&owner, "create")))
        .await
        .unwrap();
    let current = lifecycle::advance_runtime_generation(
        &db,
        &created.machine.node_id,
        0,
        0,
        Some(pod.as_str()),
        None,
    )
    .await
    .unwrap();
    db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id": &current.node_id},
            doc! {"$set":{"sandbox_uid":&sandbox,"pvc_uid":&workspace}},
        )
        .await
        .unwrap();
    let current = lifecycle::managed_machine_for_node(&db, &current.node_id)
        .await
        .unwrap()
        .unwrap();
    let current = crate::services::remote_machine_runtime::install_test_coverage(
        &db,
        &current,
        Utc::now() + chrono::Duration::minutes(2),
    )
    .await
    .unwrap();
    native::activate(&db, &current).await.unwrap();
    db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id": &current.node_id},
            doc! {"$set":{"resource_state":"ready"}},
        )
        .await
        .unwrap();
    let observed = lifecycle::apply_runtime_observation(
        &db,
        lifecycle::RuntimeObservationInput {
            machine_id: current.node_id.clone(),
            sandbox_uid: sandbox.clone(),
            pod_uid: None,
            pvc_name: None,
            pvc_uid: None,
            ready: false,
            physically_stopped: false,
            source: "sandbox".into(),
            now: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(observed.resource_state, ResourceState::Reconciling);
    assert!(!observed.writer_admitted);
    assert_eq!(observed.pod_uid.as_deref(), Some(pod.as_str()));
    assert_eq!(observed.pvc_uid.as_deref(), Some(workspace.as_str()));
    assert!(
        observed
            .native_observation
            .as_ref()
            .is_some_and(|o| !o.evidence_ref.is_empty())
    );
}

#[tokio::test]
async fn hosted_creation_persists_display_name_and_capabilities_and_binds_retries() {
    let (db, owner) = lifecycle_fixture("hosted_choices").await;
    let mut input = creation(&owner, "hosted-config-request");
    input.request_body["name"] = json!("my-browser-machine");
    input.request_body["capabilities"] = json!(["computer", "files", "browser", "shell"]);
    let first = lifecycle::create_machine(&db, input.clone()).await.unwrap();
    let view = serde_json::to_value(safe_view(&first.machine)).unwrap();
    assert_eq!(view["name"], "my-browser-machine");
    assert_eq!(
        view["capabilities"],
        json!(["browser", "computer", "files", "shell"])
    );
    assert_eq!(view["specialist_assignment_state"], "none");
    let node = db
        .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .find_one(doc! {"_id":&first.machine.node_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(node.name, "my-browser-machine");
    assert_ne!(node.name, first.machine.sandbox_name);
    assert!(
        !node.is_active,
        "creation metadata is not runtime authorization"
    );
    input.request_body["capabilities"] = json!(["shell", "files", "browser", "computer", "files"]);
    let replay = lifecycle::create_machine(&db, input.clone()).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(first.machine.node_id, replay.machine.node_id);
    input.request_body["name"] = json!("another-machine");
    assert!(lifecycle::create_machine(&db, input).await.is_err());
    assert_eq!(
        db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn hosted_creation_rejects_invalid_configuration_before_reserving_resources() {
    let (db, owner) = lifecycle_fixture("hosted_invalid").await;
    for (index, body) in [
        json!({"name":"Upper Case"}),
        json!({"name":""}),
        json!({"capabilities":[]}),
        json!({"capabilities":["computer"]}),
        json!({"capabilities":["shell","developer_browser"]}),
        json!({"specialist_id":uuid::Uuid::new_v4().to_string()}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = creation(&owner, &format!("invalid-{index}"));
        for (key, value) in body.as_object().unwrap() {
            input.request_body[key] = value.clone();
        }
        assert!(
            lifecycle::create_machine(&db, input).await.is_err(),
            "invalid hosted config {index}"
        );
    }
    assert_eq!(
        db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(lifecycle::FREE_SLOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn hosted_defaults_reject_stripped_historical_digest_and_changed_choices() {
    use sha2::{Digest, Sha256};
    let (db, owner) = lifecycle_fixture("hosted_legacy_replay").await;
    let input = creation(&owner, "legacy-config-request");
    let created = lifecycle::create_machine(&db, input.clone()).await.unwrap();
    // The old persisted contract canonicalized object keys, including when
    // serde_json's preserve_order feature is enabled by another dependency.
    // A raw insertion-ordered fixture would manufacture a never-valid digest.
    let legacy_body: std::collections::BTreeMap<_, _> =
        input.request_body.as_object().unwrap().iter().collect();
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(&legacy_body).unwrap()));
    db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&created.operation.id},
            doc! {"$set":{"request_digest":digest}},
        )
        .await
        .unwrap();
    let mut replay = input.clone();
    replay.request_body["name"] = json!("my-remote-machine");
    replay.request_body["capabilities"] = json!(["files", "shell"]);
    replay.request_body["specialist_id"] = serde_json::Value::Null;
    assert!(
        lifecycle::has_recorded_request(
            &db,
            &owner,
            None,
            LifecycleAction::Create,
            &replay.idempotency_key,
            &replay.request_body
        )
        .await
        .is_err()
    );
    assert!(
        lifecycle::create_machine(&db, replay.clone())
            .await
            .is_err()
    );
    for choices in [
        json!({"name":"changed"}),
        json!({"capabilities":["files"]}),
        json!({"specialist_id":uuid::Uuid::new_v4().to_string()}),
    ] {
        let mut changed = replay.clone();
        for (key, value) in choices.as_object().unwrap() {
            changed.request_body[key] = value.clone();
        }
        assert!(
            lifecycle::has_recorded_request(
                &db,
                &owner,
                None,
                LifecycleAction::Create,
                &changed.idempotency_key,
                &changed.request_body
            )
            .await
            .is_err()
        );
        assert!(lifecycle::create_machine(&db, changed).await.is_err());
    }
    assert_eq!(
        db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn hosted_specialist_destroyed_before_readiness_is_reported_without_grant() {
    let f = crate::services::assistant_authority_tests::fixture("hosted_assignment_failure").await;
    lifecycle::ensure_indexes(&f.state.db).await.unwrap();
    let mut input = creation(&f.owner, "assignment-failure");
    input.request_body["specialist_id"] = json!(f.chat.agent_id);
    let created = lifecycle::create_machine(&f.state.db, input).await.unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"destroyed_at":mongodb::bson::DateTime::now()}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id":&created.machine.node_id},
            doc! {"$set":{"resource_state":"ready","writer_admitted":true}},
        )
        .await
        .unwrap();
    crate::services::remote_machine_configuration::assign_specialist(
        &f.state.db,
        &created.machine.node_id,
    )
    .await
    .unwrap();
    let machine = lifecycle::managed_machine_for_node(&f.state.db, &created.machine.node_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(machine.specialist_assignment_state, "failed");
    assert_eq!(
        machine.specialist_assignment_error.as_deref(),
        Some("specialist_assignment_unavailable")
    );
    let agent = f
        .state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!agent.machine_node_ids.contains(&created.machine.node_id));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn hosted_specialist_assignment_uses_existing_defaults_and_never_regrants_revocation() {
    use crate::services::{
        assistant_team_service as team, machine_access_service as access,
        remote_machine_configuration as config,
    };
    for editor_enabled in [false, true] {
        let f = crate::services::assistant_authority_tests::fixture("hosted_specialist").await;
        lifecycle::ensure_indexes(&f.state.db).await.unwrap();
        access::ensure_indexes(&f.state.db).await.unwrap();
        crate::services::feature_flag_service::set_platform_override(
            &f.state.db,
            access::FLAG,
            &crate::services::feature_flag_service::FlagTarget::Global,
            editor_enabled,
            &f.owner,
        )
        .await
        .unwrap();
        let mut input = creation(&f.owner, "specialist-config");
        input.request_body["specialist_id"] = json!(f.chat.agent_id);
        input.request_body["capabilities"] = json!(["shell", "files", "browser", "computer"]);
        let created = lifecycle::create_machine(&f.state.db, input.clone())
            .await
            .unwrap();
        assert_eq!(created.machine.specialist_assignment_state, "pending");
        config::assign_specialist(&f.state.db, &created.machine.node_id)
            .await
            .unwrap();
        let before = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
        assert!(!before.machine_node_ids.contains(&created.machine.node_id));
        // Explicit local runtime-readiness fixture; it is not WSS acceptance.
        let profile = nyxid_machine::MachineProfile {
            shell: true,
            files: true,
            browser: Some(true),
            computer: true,
            authority_versions: vec![2],
            roots: vec!["/workspace".into()],
            runtime_id: uuid::Uuid::new_v4().to_string(),
            commands_isolated: Some(true),
            ..Default::default()
        };
        f.state.db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(
            doc!{"_id":&created.machine.node_id},doc!{"$set":{"machine":mongodb::bson::to_bson(&profile).unwrap(),"is_active":true}}).await.unwrap();
        f.state
            .db
            .collection::<Document>(MACHINES)
            .update_one(
                doc! {"_id":&created.machine.node_id},
                doc! {"$set":{"resource_state":"ready","writer_admitted":true}},
            )
            .await
            .unwrap();
        config::assign_specialist(&f.state.db, &created.machine.node_id)
            .await
            .unwrap();
        let assigned = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
        assert!(assigned.machine_node_ids.contains(&created.machine.node_id));
        let policy = access::policy(&f.state.db, &f.chat.agent_id).await.unwrap();
        let grant = &policy.assignments[&created.machine.node_id];
        assert_eq!(grant.capabilities.shell, !editor_enabled);
        assert_eq!(grant.capabilities.browser, !editor_enabled);
        assert_eq!(grant.capabilities.computer, !editor_enabled);
        let machine = lifecycle::managed_machine_for_node(&f.state.db, &created.machine.node_id)
            .await
            .unwrap()
            .unwrap();
        let view = config::safe_view(&f.state.db, &machine).await.unwrap();
        assert_eq!(
            view.specialist_assignment_state,
            if editor_enabled {
                "permissions_required"
            } else {
                "assigned"
            }
        );
        team::set_grants(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            team::GrantChange::Machine {
                base: Box::new(team::GrantChange::Add(Default::default())),
                machines: Some(vec![created.machine.node_id.clone()]),
                logins: None,
                mode: team::MachineGrantMode::Remove,
            },
        )
        .await
        .unwrap();
        config::assign_specialist(&f.state.db, &created.machine.node_id)
            .await
            .unwrap();
        assert!(
            lifecycle::create_machine(&f.state.db, input)
                .await
                .unwrap()
                .replayed
        );
        let revoked = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
        assert!(!revoked.machine_node_ids.contains(&created.machine.node_id));
        assert_eq!(
            config::safe_view(&f.state.db, &machine)
                .await
                .unwrap()
                .specialist_assignment_state,
            "permissions_required"
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn current_replay_retains_saved_image_after_default_change_but_rejects_obsolete_rows() {
    use crate::models::remote_machine::{DATA_POLICY_VERSION, POLICY_VERSION};
    let (db, owner) = lifecycle_fixture("single_current_replay").await;
    let input = creation(&owner, "current-exact");
    let created = lifecycle::create_machine(&db, input.clone()).await.unwrap();
    let mut retry = input.clone();
    retry.profile.image_digest = format!("deployment-b@sha256:{}", "b".repeat(64));
    let replay = lifecycle::create_machine(&db, retry.clone()).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.machine.profile, created.machine.profile);
    let mut wrong_shape = retry.clone();
    wrong_shape.profile.workspace_gib = 40;
    assert!(lifecycle::create_machine(&db, wrong_shape).await.is_err());
    // Even a matching accepted request digest cannot revive an obsolete saved contract.
    for (field, obsolete, current) in [
        (
            "profile.profile_version",
            "remote-shell-files-v1",
            nyxid_machine::managed::PROFILE,
        ),
        (
            "profile.profile_version",
            "remote-shell-files-v2",
            nyxid_machine::managed::PROFILE,
        ),
        ("policy_version", "remote-machine-mvp-v1", POLICY_VERSION),
        ("policy_version", "remote-machine-mvp-v2", POLICY_VERSION),
        (
            "data_policy_version",
            "remote-machine-data-v1",
            DATA_POLICY_VERSION,
        ),
    ] {
        db.collection::<Document>(MACHINES)
            .update_one(
                doc! {"_id":&created.machine.node_id},
                doc! {"$set":{field:obsolete}},
            )
            .await
            .unwrap();
        assert!(lifecycle::create_machine(&db, retry.clone()).await.is_err());
        db.collection::<Document>(MACHINES)
            .update_one(
                doc! {"_id":&created.machine.node_id},
                doc! {"$set":{field:current}},
            )
            .await
            .unwrap();
    }
    for (field, value) in [
        ("profile", "remote-shell-files-v1"),
        ("profile", "remote-shell-files-v2"),
        ("policy_version", "remote-machine-mvp-v1"),
        ("policy_version", "remote-machine-mvp-v2"),
        ("data_policy_version", "remote-machine-data-v1"),
    ] {
        let mut rejected = retry.clone();
        rejected.request_body[field] = json!(value);
        assert!(lifecycle::create_machine(&db, rejected).await.is_err());
    }
    let mut changed = retry.clone();
    changed.request_body["name"] = json!("different-name");
    assert!(lifecycle::create_machine(&db, changed).await.is_err());
    assert_eq!(
        db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(OPERATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(OUTBOX)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn individual_delete_requires_current_owner_ack_and_preserves_original_basis_on_retry() {
    let (db, owner) = lifecycle_fixture("single_current_delete").await;
    let created = lifecycle::create_machine(&db, creation(&owner, "delete-ack-create"))
        .await
        .unwrap();
    let accepted = action(&created.machine, LifecycleAction::Delete, "owner-delete");
    for patch in [
        json!({"data_policy_version":null}),
        json!({"data_policy_version":"remote-machine-data-v1"}),
        json!({"data_policy_version":"machine-data"}),
        json!({"acknowledge_data_loss":false}),
    ] {
        let mut denied = accepted.clone();
        for (key, value) in patch.as_object().unwrap() {
            denied.request_body[key] = value.clone();
        }
        assert!(lifecycle::request_action(&db, denied).await.is_err());
        let stored = lifecycle::get_machine(&db, &owner, &created.machine.node_id)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.deletion_basis.is_none());
        assert_eq!(stored.resource_state, ResourceState::Provisioning);
    }
    let result = lifecycle::request_action(&db, accepted.clone())
        .await
        .unwrap();
    let basis =
        mongodb::bson::to_document(result.machine.deletion_basis.as_ref().unwrap()).unwrap();
    assert_eq!(basis.get_str("source").unwrap(), "machine_owner");
    assert_eq!(
        basis.get_str("policy_version").unwrap(),
        crate::models::remote_machine::DATA_POLICY_VERSION
    );
    let replay = lifecycle::request_action(&db, accepted.clone())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(
        mongodb::bson::to_document(replay.machine.deletion_basis.as_ref().unwrap()).unwrap(),
        basis
    );
    let mut obsolete = accepted;
    obsolete.request_body["data_policy_version"] = json!("remote-machine-data-v1");
    assert!(lifecycle::request_action(&db, obsolete).await.is_err());
    assert_eq!(
        db.collection::<Document>(OPERATIONS)
            .count_documents(doc! {"action":"delete"})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn profile_binding_rejects_body_tampering_before_machine_or_outbox() {
    let (db, owner) = lifecycle_fixture("profile_binding_body").await;
    for (field, value) in [
        ("profile_version", json!("test-foreign-profile")),
        ("profile_version", serde_json::Value::Null),
        ("cpu_millis", json!(4000)),
        ("memory_mib", json!(8192)),
        ("workspace_gib", json!(50)),
    ] {
        let mut input = creation(&owner, &format!("invalid-{field}-{value}"));
        input.request_body[field] = value;
        assert!(
            matches!(
                lifecycle::create_machine(&db, input).await,
                Err(crate::errors::AppError::ValidationError(_))
            ),
            "untrusted {field} must fail before durable allocation"
        );
        assert_eq!(
            db.collection::<Document>(MACHINES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(OUTBOX)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    db.drop().await.unwrap();
}

#[test]
fn controlled_profile_has_stable_resource_meaning_and_rejects_aliases() {
    let profile = lifecycle::controlled_profile("remote-machine").unwrap();
    assert_eq!(
        (
            profile.cpu_millis,
            profile.memory_mib,
            profile.workspace_gib,
            profile.ephemeral_gib
        ),
        (2000, 2048, 20, None)
    );
    for invalid in [
        "",
        "remote-machine-4c8g",
        "remote-shell-files-v1",
        "remote-shell-files-v2",
    ] {
        assert!(lifecycle::controlled_profile(invalid).is_err());
    }
    let mut accepted = lifecycle::resolve_profile("remote-machine", "accepted-image").unwrap();
    accepted.runtime_class = Some("retained-runtime".into());
    lifecycle::validate_fixed_profile(&accepted).unwrap();
    accepted.memory_mib = 8192;
    assert!(lifecycle::validate_fixed_profile(&accepted).is_err());
}

#[tokio::test]
async fn profile_binding_normalizes_omitted_selection_and_rejects_changed_replay() {
    let (db, owner) = lifecycle_fixture("profile_binding_replay").await;
    let input = creation(&owner, "profile-replay");
    let created = lifecycle::create_machine(&db, input.clone()).await.unwrap();
    let mut explicit = input;
    explicit.request_body["profile_version"] = json!("remote-machine");
    let replay = lifecycle::create_machine(&db, explicit.clone())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(created.machine.node_id, replay.machine.node_id);
    explicit.request_body["profile_version"] = json!("other-profile");
    assert!(lifecycle::create_machine(&db, explicit).await.is_err());
    assert_eq!(
        db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>(OUTBOX)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}
