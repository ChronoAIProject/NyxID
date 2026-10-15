//! Real manager + transactional Mongo tests. HTTP observations are local
//! Kubernetes API fixtures under the owner's 2026-10-14 revision4 contract.
//! These checks do not certify physical stop, CSI isolation or cloud erasure.
use chrono::Utc;
use mongodb::{
    Database,
    bson::{Document, doc},
};
use nyxid_machine_updater::sandbox::SandboxApi;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

use crate::{
    models::{
        machine_free_slot::{FreeSlotState, MachineFreeSlot},
        remote_machine::{ManagedMachine, ResourceState},
        remote_machine_operation::LifecycleAction,
        remote_machine_outbox::{COLLECTION_NAME as OUTBOX, MachineEffectOutbox},
        user::UserType,
    },
    services::{
        remote_machine_manager::Manager,
        remote_machine_service::{self as lifecycle, CreateMachineInput},
    },
    test_utils::{connect_transaction_test_database, test_user},
};

struct Fixture {
    db: Database,
    server: MockServer,
    manager: Manager,
    machine: ManagedMachine,
    effect: MachineEffectOutbox,
}

async fn fixture(label: &str) -> Fixture {
    fixture_with_rollout(label, false).await
}

async fn fixture_with_rollout(label: &str, enabled: bool) -> Fixture {
    let db = connect_transaction_test_database(label).await;
    lifecycle::ensure_indexes(&db).await.unwrap();
    crate::services::remote_machine_events::ensure_indexes(&db)
        .await
        .unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    if enabled {
        use crate::services::feature_flag_service::{self, FlagTarget, REMOTE_MACHINES_FLAG_KEY};
        feature_flag_service::set_platform_override(
            &db,
            REMOTE_MACHINES_FLAG_KEY,
            &FlagTarget::User(owner.clone()),
            true,
            &owner,
        )
        .await
        .unwrap();
    }
    let mut input = CreateMachineInput::fixed(
        &owner,
        true,
        &format!("registry.test/node@sha256:{}", "a".repeat(64)),
    );
    input.idempotency_key = uuid::Uuid::new_v4().to_string();
    let created = Box::pin(lifecycle::create_machine(&db, input))
        .await
        .unwrap();
    let effect = db
        .collection::<MachineEffectOutbox>(OUTBOX)
        .find_one(doc! {"operation_id": &created.operation.id})
        .await
        .unwrap()
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/apis/storage.k8s.io/v1/storageclasses/local-fixture-csi"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kind":"StorageClass","apiVersion":"storage.k8s.io/v1","metadata":{"name":"local-fixture-csi","uid":"storage-fixture"},"provisioner":"fixture.csi.example"})))
        .with_priority(1).mount(&server).await;
    let manager = Manager {
        db: db.clone(),
        billing: None,
        api: SandboxApi::new(server.uri().parse().unwrap(), None).unwrap(),
        storage_class: Some("local-fixture-csi".into()),
        namespace: "nyxid-machine".into(),
        bootstrap_base_url: "https://nyxid.test".into(),
    };
    Fixture {
        db,
        server,
        manager,
        machine: created.machine,
        effect,
    }
}

async fn disable_rollout(f: &Fixture) {
    use crate::services::feature_flag_service::{self, FlagTarget, REMOTE_MACHINES_FLAG_KEY};
    feature_flag_service::set_platform_override(
        &f.db,
        REMOTE_MACHINES_FLAG_KEY,
        &FlagTarget::User(f.machine.owner_id.clone()),
        false,
        &f.machine.owner_id,
    )
    .await
    .unwrap();
    assert!(
        !feature_flag_service::personal_flag_enabled(
            &f.db,
            &f.machine.owner_id,
            REMOTE_MACHINES_FLAG_KEY
        )
        .await
        .unwrap()
    );
}

fn sandbox_path(machine: &ManagedMachine) -> String {
    format!(
        "/apis/agents.x-k8s.io/v1beta1/namespaces/{}/sandboxes/{}",
        machine.sandbox_namespace, machine.sandbox_name
    )
}

fn sandbox(machine: &ManagedMachine, uid: &str) -> Value {
    let mut object = crate::services::remote_machine_adapter::intent(
        machine,
        machine.active_operation_id.as_deref().unwrap(),
        Some("local-fixture-csi"),
        true,
        "https://nyxid.test",
    )
    .apply_object();
    object["metadata"]["uid"] = json!(uid);
    object["metadata"]["resourceVersion"] = json!("17");
    object["metadata"]["generation"] = json!(1);
    // Match the pinned metav1.Condition schema, including required message,
    // reason and transition time; observedGeneration belongs to each condition.
    let transition = Utc::now().to_rfc3339();
    object["status"] = json!({"conditions":[
        {"type":"Ready","status":"True","observedGeneration":1,
            "reason":"DependenciesReady","message":"All dependent resources are ready","lastTransitionTime":transition},
        {"type":"Suspended","status":"False","reason":"NotSuspended","observedGeneration":1,
            "message":"Sandbox is not suspended","lastTransitionTime":transition}
    ]});
    object
}

async fn current(f: &Fixture) -> ManagedMachine {
    lifecycle::get_machine(&f.db, &f.machine.owner_id, &f.machine.node_id)
        .await
        .unwrap()
        .unwrap()
}

async fn owned_objects(f: &Fixture, machine: &ManagedMachine, sandbox_uid: &str) {
    let mut observed = machine.clone();
    observed.sandbox_uid = Some(sandbox_uid.into());
    observed.pod_uid = Some(uuid::Uuid::new_v4().to_string());
    observed.pvc_uid = Some(uuid::Uuid::new_v4().to_string());
    observed.pvc_name = Some("workspace".into());
    mount_owned_resources(f, &observed, false).await;
}

async fn bind_runtime(f: &Fixture) -> ManagedMachine {
    let machine = current(f).await;
    let machine = lifecycle::advance_runtime_generation(
        &f.db,
        &machine.node_id,
        machine.revision,
        0,
        None,
        None,
    )
    .await
    .unwrap();
    let machine = lifecycle::apply_runtime_observation(
        &f.db,
        lifecycle::RuntimeObservationInput {
            machine_id: machine.node_id,
            sandbox_uid: uuid::Uuid::new_v4().to_string(),
            pod_uid: Some(uuid::Uuid::new_v4().to_string()),
            pvc_name: Some("workspace".into()),
            pvc_uid: Some(uuid::Uuid::new_v4().to_string()),
            ready: true,
            physically_stopped: false,
            source: "local-http-fixture".into(),
            now: Some(Utc::now()),
        },
    )
    .await
    .unwrap();
    f.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&machine.node_id},
            doc! {"$set":{"native_pod_name":"node", "native_node_name":"fixture-node"}},
        )
        .await
        .unwrap();
    crate::services::remote_machine_runtime::install_test_coverage(
        &f.db,
        &machine,
        Utc::now() + chrono::Duration::minutes(2),
    )
    .await
    .unwrap()
}

async fn delete_request(f: &Fixture, machine: &ManagedMachine) -> ManagedMachine {
    Box::pin(lifecycle::request_action(
        &f.db,
        lifecycle::LifecycleRequest {
            owner_id: machine.owner_id.clone(),
            machine_id: machine.node_id.clone(),
            action: LifecycleAction::Delete,
            expected_revision: Some(machine.revision),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            request_body: json!({"action":"delete","revision":machine.revision,"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true}),
            now: None,
        },
    ))
    .await
    .unwrap()
    .machine
}

async fn assert_slot_retained(f: &Fixture) {
    let slot =
        f.db.collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
            .find_one(doc! {"_id":&f.machine.owner_id})
            .await
            .unwrap()
            .unwrap();
    assert_ne!(slot.state, FreeSlotState::Released);
    assert_eq!(slot.machine_id.as_deref(), Some(f.machine.node_id.as_str()));
}

#[tokio::test]
async fn manager_lost_create_response_recovers_original_allocation_without_recreating() {
    let f = fixture_with_rollout("mgr_lost_response", true).await;
    disable_rollout(&f).await;
    Mock::given(method("GET"))
        .and(path(sandbox_path(&f.machine)))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(sandbox_path(&f.machine)))
        .respond_with(ResponseTemplate::new(503).set_body_string("provider-secret-in-raw-error"))
        .expect(1)
        .mount(&f.server)
        .await;
    let error = f.manager.apply_effect(&f.effect).await.unwrap_err();
    assert!(!error.to_string().contains("provider-secret"));
    let after_failure = current(&f).await;
    assert_eq!(after_failure.runtime_generation, 1);
    assert!(after_failure.sandbox_uid.is_none());

    // The controller did create the original resource but its response was
    // lost. Retry must recover the same immutable intent by deterministic name.
    f.server.reset().await;
    let uid = uuid::Uuid::new_v4().to_string();
    Mock::given(method("GET"))
        .and(path(sandbox_path(&f.machine)))
        .respond_with(ResponseTemplate::new(200).set_body_json(sandbox(&after_failure, &uid)))
        .mount(&f.server)
        .await;
    owned_objects(&f, &after_failure, &uid).await;
    f.manager.apply_effect(&f.effect).await.unwrap();
    let recovered = current(&f).await;
    assert_eq!(recovered.sandbox_uid.as_deref(), Some(uid.as_str()));
    assert_eq!(recovered.runtime_generation, 1);
    assert_ne!(recovered.resource_state, ResourceState::Ready);
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_uid_replacement_never_receives_a_mutation_or_is_adopted() {
    let f = fixture("mgr_uid_replacement").await;
    let bound = bind_runtime(&f).await;
    let foreign_uid = uuid::Uuid::new_v4().to_string();
    Mock::given(method("GET"))
        .and(path(sandbox_path(&bound)))
        .respond_with(ResponseTemplate::new(200).set_body_json(sandbox(&bound, &foreign_uid)))
        .mount(&f.server)
        .await;
    assert!(f.manager.apply_effect(&f.effect).await.is_err());
    let after = current(&f).await;
    assert_eq!(after.sandbox_uid, bound.sandbox_uid);
    assert_eq!(after.pvc_uid, bound.pvc_uid);
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_missing_sandbox_without_native_stop_cannot_finalize_delete() {
    let f = fixture("mgr_no_stop_proof").await;
    let bound = bind_runtime(&f).await;
    let deleting = delete_request(&f, &bound).await;
    Mock::given(method("GET"))
        .and(path(sandbox_path(&bound)))
        .respond_with(ResponseTemplate::new(404))
        .mount(&f.server)
        .await;
    assert!(f.manager.observe(&deleting).await.is_err());
    let after = current(&f).await;
    assert_eq!(after.resource_state, ResourceState::Deleting);
    assert_eq!(after.pod_uid, bound.pod_uid);
    assert_eq!(after.pvc_uid, bound.pvc_uid);
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_native_stop_without_storage_cleanup_retains_the_free_slot() {
    let f = fixture("mgr_native_stop_pending_cleanup").await;
    let bound = bind_runtime(&f).await;
    let deleting = delete_request(&f, &bound).await;
    mount_bound_resources(&f, &deleting, true).await;
    Mock::given(method("DELETE"))
        .and(path(sandbox_path(&deleting)))
        .respond_with(ResponseTemplate::new(202))
        .mount(&f.server)
        .await;
    assert!(f.manager.observe(&deleting).await.is_err());
    let stopped = current(&f).await;
    assert!(stopped.native_observation.as_ref().unwrap().stop_confirmed);
    assert_eq!(stopped.resource_state, ResourceState::Deleting);
    assert_ne!(stopped.cleanup_state, "complete");
    assert_slot_retained(&f).await;
    // Repeated accepted Sandbox DELETE is still submission, never storage closure.
    assert!(f.manager.observe(&stopped).await.is_err());
    assert_eq!(current(&f).await.pvc_uid, bound.pvc_uid);
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_superseded_create_effect_never_restarts_a_deleting_machine() {
    let f = fixture("mgr_superseded_effect").await;
    let deleting = delete_request(&f, &f.machine).await;
    f.manager.apply_effect(&f.effect).await.unwrap();
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        current(&f).await.active_operation_id,
        deleting.active_operation_id
    );
    assert_eq!(current(&f).await.resource_state, ResourceState::Deleting);
}

#[tokio::test]
async fn manager_orphan_inventory_records_investigation_and_never_deletes_data() {
    let f = fixture("mgr_orphan_inventory").await;
    let mut orphan = f.machine.clone();
    orphan.node_id = uuid::Uuid::new_v4().to_string();
    orphan.runtime_generation = 1;
    orphan.credential_epoch = 1;
    let uid = uuid::Uuid::new_v4().to_string();
    Mock::given(method("GET"))
        .and(path(
            "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes",
        ))
        .and(query_param("labelSelector", "nyxid.dev/managed=true"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"items":[sandbox(&orphan, &uid)]})),
        )
        .mount(&f.server)
        .await;
    for resource in ["pods", "persistentvolumeclaims"] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/namespaces/nyxid-machine/{resource}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[]})))
            .mount(&f.server)
            .await;
    }
    f.manager.inventory().await.unwrap();
    f.manager.inventory().await.unwrap();
    let rows = f.db.collection::<Document>("machine_orphan_inventory");
    assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 1);
    let row = rows
        .find_one(doc! {"sandbox_uid":&uid})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("state").unwrap(), "investigation_required");
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
    assert_slot_retained(&f).await;
}

fn controlled_allocation(machine: &ManagedMachine, kind: &str, sandbox_uid: &str) -> Value {
    let mut object = sandbox(machine, sandbox_uid);
    let (name, uid) = if kind == "Pod" {
        ("node".to_owned(), machine.pod_uid.clone())
    } else {
        (
            machine
                .pvc_name
                .clone()
                .unwrap_or_else(|| "workspace".into()),
            machine.pvc_uid.clone(),
        )
    };
    object["kind"] = json!(kind);
    object["apiVersion"] = json!("v1");
    object["metadata"]["name"] = json!(name);
    object["metadata"]["uid"] = json!(uid.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()));
    object["metadata"]["ownerReferences"] = json!([{"apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox","controller":true,"uid":sandbox_uid}]);
    // An unrelated secret-bearing field must never enter an investigation DTO.
    object["spec"] = json!({"containers":[{"env":[{"name":"TOKEN","value":"fixture-secret-not-in-diagnostics"}]}]});
    object
}

async fn inventory_fixture(
    server: &MockServer,
    sandboxes: Vec<Value>,
    pods: Vec<Value>,
    pvcs: Vec<Value>,
) {
    for (path, items) in [
        (
            "/apis/agents.x-k8s.io/v1beta1/namespaces/nyxid-machine/sandboxes",
            sandboxes,
        ),
        ("/api/v1/namespaces/nyxid-machine/pods", pods),
        (
            "/api/v1/namespaces/nyxid-machine/persistentvolumeclaims",
            pvcs,
        ),
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .and(query_param("labelSelector", "nyxid.dev/managed=true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":items})))
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn manager_inventory_finds_pod_and_pvc_after_sandbox_disappears_without_cleanup() {
    use crate::services::remote_machine_events as events;
    let f = fixture("mgr_inventory_missing_sandbox").await;
    let bound = bind_runtime(&f).await;
    let sandbox_uid = bound.sandbox_uid.as_deref().unwrap();
    inventory_fixture(
        &f.server,
        vec![],
        vec![controlled_allocation(&bound, "Pod", sandbox_uid)],
        vec![controlled_allocation(
            &bound,
            "PersistentVolumeClaim",
            sandbox_uid,
        )],
    )
    .await;
    f.manager.inventory().await.unwrap();
    let first = events::list_investigations(&f.db, None, None)
        .await
        .unwrap();
    assert_eq!(first.rows.len(), 2);
    for row in &first.rows {
        assert_eq!(row.get_str("reason"), Ok("resource_sandbox_missing"));
        assert_eq!(row.get_str("machine_id"), Ok(bound.node_id.as_str()));
        assert_eq!(row.get_str("sandbox_uid"), Ok(sandbox_uid));
        assert!(
            !serde_json::to_string(row)
                .unwrap()
                .contains("fixture-secret")
        );
    }
    f.manager.inventory().await.unwrap();
    let repeated = events::list_investigations(&f.db, None, None)
        .await
        .unwrap();
    assert_eq!(repeated.rows.len(), 2);
    assert_eq!(
        first
            .rows
            .iter()
            .map(|row| row.get_str("_id").unwrap())
            .collect::<Vec<_>>(),
        repeated
            .rows
            .iter()
            .map(|row| row.get_str("_id").unwrap())
            .collect::<Vec<_>>()
    );
    let retained = current(&f).await;
    assert_eq!(retained.pvc_uid, bound.pvc_uid);
    assert_eq!(retained.resource_state, bound.resource_state);
    assert_slot_retained(&f).await;
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method.as_str() == "GET")
    );
}

#[tokio::test]
async fn manager_inventory_preserves_exact_paused_workspace_without_a_sandbox() {
    let f = fixture("mgr_inventory_paused_pvc").await;
    let bound = bind_runtime(&f).await;
    f.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&bound.node_id},
            doc! {"$set":{"resource_state":"paused","writer_admitted":false}},
        )
        .await
        .unwrap();
    inventory_fixture(
        &f.server,
        vec![],
        vec![],
        vec![controlled_allocation(
            &bound,
            "PersistentVolumeClaim",
            bound.sandbox_uid.as_deref().unwrap(),
        )],
    )
    .await;
    f.manager.inventory().await.unwrap();
    f.manager.inventory().await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(crate::services::remote_machine_events::ORPHAN_INVENTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let retained = current(&f).await;
    assert_eq!(retained.resource_state, ResourceState::Paused);
    assert_eq!(retained.pvc_uid, bound.pvc_uid);
    assert!(!retained.writer_admitted);
    assert_slot_retained(&f).await;
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method.as_str() == "GET")
    );
}

#[tokio::test]
async fn manager_inventory_flags_unbound_and_untrusted_resources_without_adopting_them() {
    use crate::services::remote_machine_events as events;
    let f = fixture("mgr_inventory_unbound_untrusted").await;
    let mut allocation = f.machine.clone();
    allocation.runtime_generation = 1;
    allocation.credential_epoch = 1;
    let uid = uuid::Uuid::new_v4().to_string();
    let unbound = controlled_allocation(&allocation, "Pod", &uid);
    let mut forged = controlled_allocation(&allocation, "Pod", &uid);
    forged["metadata"]["labels"]["nyxid.dev/operation-id"] =
        json!(uuid::Uuid::new_v4().to_string());
    let mut malformed = controlled_allocation(&allocation, "PersistentVolumeClaim", &uid);
    malformed["metadata"]["uid"] = json!("invalid-secret-bearing-uid");
    malformed["metadata"]["labels"]["nyxid.dev/owner-id"] = json!("untrusted-owner");
    inventory_fixture(&f.server, vec![], vec![unbound, forged], vec![malformed]).await;
    f.manager.inventory().await.unwrap();
    f.manager.inventory().await.unwrap();
    let rows = events::list_investigations(&f.db, None, None)
        .await
        .unwrap()
        .rows;
    let reasons = rows
        .iter()
        .map(|row| row.get_str("reason").unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        reasons,
        std::collections::HashSet::from([
            "allocated_resource_unbound",
            "resource_intent_unmatched",
            "resource_metadata_untrusted"
        ])
    );
    assert!(
        !serde_json::to_string(&rows)
            .unwrap()
            .contains("secret-bearing")
    );
    let retained = current(&f).await;
    assert!(
        retained.pod_uid.is_none() && retained.pvc_uid.is_none() && retained.sandbox_uid.is_none()
    );
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_inventory_live_durable_intent_explains_allocation_awaiting_binding() {
    let f = fixture("mgr_inventory_live_unbound").await;
    let mut allocation = f.machine.clone();
    allocation.runtime_generation = 1;
    allocation.credential_epoch = 1;
    let uid = uuid::Uuid::new_v4().to_string();
    inventory_fixture(
        &f.server,
        vec![sandbox(&allocation, &uid)],
        vec![controlled_allocation(&allocation, "Pod", &uid)],
        vec![controlled_allocation(
            &allocation,
            "PersistentVolumeClaim",
            &uid,
        )],
    )
    .await;
    f.manager.inventory().await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(crate::services::remote_machine_events::ORPHAN_INVENTORY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    // Inventory does not adopt the allocation, admit a writer, or replace the
    // ordinary Sandbox effect/observation transaction.
    let retained = current(&f).await;
    assert!(
        retained.sandbox_uid.is_none() && retained.pvc_uid.is_none() && !retained.writer_admitted
    );
}

#[tokio::test]
async fn manager_inventory_rejects_different_owner_and_resource_uids_for_paused_pvc() {
    use crate::services::remote_machine_events as events;
    let f = fixture("mgr_inventory_paused_uid_mismatch").await;
    let bound = bind_runtime(&f).await;
    f.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&bound.node_id},
            doc! {"$set":{"resource_state":"paused","writer_admitted":false}},
        )
        .await
        .unwrap();
    let wrong_owner = controlled_allocation(
        &bound,
        "PersistentVolumeClaim",
        &uuid::Uuid::new_v4().to_string(),
    );
    let mut wrong_resource = controlled_allocation(
        &bound,
        "PersistentVolumeClaim",
        bound.sandbox_uid.as_deref().unwrap(),
    );
    wrong_resource["metadata"]["uid"] = json!(uuid::Uuid::new_v4().to_string());
    inventory_fixture(&f.server, vec![], vec![], vec![wrong_owner, wrong_resource]).await;
    f.manager.inventory().await.unwrap();
    let rows = events::list_investigations(&f.db, None, None)
        .await
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 2);
    let reasons = rows
        .iter()
        .map(|row| row.get_str("reason").unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        reasons,
        std::collections::HashSet::from(["resource_owner_uid_unmatched", "resource_uid_unmatched"])
    );
    assert_eq!(current(&f).await.pvc_uid, bound.pvc_uid);
}

#[tokio::test]
async fn manager_inventory_operator_pages_remain_bounded_and_escalation_replays_once() {
    use crate::services::remote_machine_events as events;
    let f = fixture("mgr_inventory_operator_pages").await;
    let rows = f.db.collection::<Document>(events::ORPHAN_INVENTORY);
    let at = mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(25));
    for _ in 0..103 {
        let uid = uuid::Uuid::new_v4().to_string();
        events::record_investigation(
            &f.db,
            events::InvestigationIdentity {
                kind: "Pod",
                namespace: "nyxid-machine".into(),
                name: Some("retained".into()),
                resource_uid: Some(uid.clone()),
                sandbox_uid: None,
                machine_id: None,
                owner_id: None,
                operation_id: None,
                identity_hash: uid,
            },
            "resource_machine_unmatched",
        )
        .await
        .unwrap();
    }
    rows.update_many(doc! {}, doc! {"$set":{"created_at":at}})
        .await
        .unwrap();
    let first = events::list_investigations(&f.db, None, Some(u32::MAX))
        .await
        .unwrap();
    assert_eq!(first.rows.len(), 100);
    assert!(first.next_cursor.is_some());
    rows.update_many(
        doc! {},
        doc! {"$set":{"last_seen_at":mongodb::bson::DateTime::now()}},
    )
    .await
    .unwrap();
    let second = events::list_investigations(&f.db, first.next_cursor.as_deref(), Some(100))
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 3);
    assert!(second.next_cursor.is_none());
    let ids = first
        .rows
        .iter()
        .chain(second.rows.iter())
        .map(|row| row.get_str("_id").unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), 103);
    assert!(
        events::list_investigations(&f.db, Some("not-a-cursor"), None)
            .await
            .is_err()
    );
    let now = Utc::now();
    events::sweep(&f.db, now).await.unwrap();
    assert_eq!(
        rows.count_documents(
            doc! {"operator_escalated_at":mongodb::bson::DateTime::from_chrono(now)}
        )
        .await
        .unwrap(),
        103
    );
    events::sweep(&f.db, now + chrono::Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(
        rows.count_documents(
            doc! {"operator_escalated_at":mongodb::bson::DateTime::from_chrono(now)}
        )
        .await
        .unwrap(),
        103
    );
    assert_eq!(
        rows.count_documents(doc! {"state":"investigation_required"})
            .await
            .unwrap(),
        103
    );
}

#[tokio::test]
async fn manager_timeline_owner_filter_and_expiry_do_not_prune_machine_or_internal_operation() {
    use crate::services::{
        remote_machine_events as events, remote_machine_operation_service as operations,
    };
    let f = fixture("mgr_timeline_retention").await;
    let mut operation =
        operations::get_operation(&f.db, &f.machine.owner_id, &f.effect.operation_id)
            .await
            .unwrap()
            .unwrap();
    events::record(
        &f.db,
        &operation,
        "accepted",
        "Lifecycle operation accepted",
        true,
    )
    .await
    .unwrap();
    events::record(
        &f.db,
        &operation,
        "accepted",
        "Lifecycle operation accepted",
        true,
    )
    .await
    .unwrap();
    let own = events::list(&f.db, &f.machine.owner_id, &f.machine.node_id, None)
        .await
        .unwrap();
    assert_eq!(own.len(), 1);
    let safe = serde_json::to_value(&own[0]).unwrap();
    for forbidden in [
        "owner_id",
        "expires_at",
        "auth_token",
        "signing_secret",
        "command",
        "stdout",
        "ciphertext",
    ] {
        assert!(safe.get(forbidden).is_none());
    }
    assert!(
        events::list(
            &f.db,
            &uuid::Uuid::new_v4().to_string(),
            &f.machine.node_id,
            None
        )
        .await
        .unwrap()
        .is_empty()
    );
    let terminal = Utc::now() - chrono::Duration::days(89);
    operation.terminal_at = Some(terminal);
    f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
        .update_one(doc! {"_id": &operation.id}, doc! {"$set":{"terminal_at":mongodb::bson::DateTime::from_chrono(terminal),"state":"succeeded"}})
        .await.unwrap();
    events::sweep(&f.db, Utc::now()).await.unwrap();
    assert!(
        !events::list(&f.db, &f.machine.owner_id, &f.machine.node_id, None)
            .await
            .unwrap()
            .is_empty()
    );
    events::sweep(&f.db, Utc::now() + chrono::Duration::days(2))
        .await
        .unwrap();
    assert_eq!(
        f.db.collection::<Document>(crate::models::remote_machine_event::COLLECTION_NAME)
            .count_documents(doc! {"operation_id":&operation.id})
            .await
            .unwrap(),
        0
    );
    assert!(
        operations::get_operation(&f.db, &f.machine.owner_id, &operation.id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        lifecycle::get_machine(&f.db, &f.machine.owner_id, &f.machine.node_id)
            .await
            .unwrap()
            .is_some()
    );
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_overdue_cleanup_notifies_once_and_preserves_pending_resources() {
    use crate::services::{
        remote_machine_events as events, remote_machine_operation_service as operations,
    };
    let f = fixture("mgr_overdue_cleanup").await;
    let deleting = delete_request(&f, &f.machine).await;
    let operation_id = deleting.active_operation_id.as_deref().unwrap();
    let accepted = Utc::now() - chrono::Duration::hours(25);
    f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":operation_id},
            doc! {"$set":{"created_at":mongodb::bson::DateTime::from_chrono(accepted)}},
        )
        .await
        .unwrap();
    events::sweep(&f.db, Utc::now()).await.unwrap();
    events::sweep(&f.db, Utc::now()).await.unwrap();
    let operation = operations::get_operation(&f.db, &f.machine.owner_id, operation_id)
        .await
        .unwrap()
        .unwrap();
    assert!(operation.terminal_at.is_none());
    assert_eq!(operation.error_code.as_deref(), Some("cleanup_overdue"));
    assert_eq!(f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc!{"event_type":"machine_cleanup_overdue_operator_action","event_data.operation_id":operation_id})
        .await.unwrap(),1);
    let timeline = events::list(&f.db, &f.machine.owner_id, &f.machine.node_id, None)
        .await
        .unwrap();
    assert_eq!(
        timeline
            .iter()
            .filter(|event| event.kind == "cleanup_overdue" && event.notification)
            .count(),
        1
    );
    assert_eq!(current(&f).await.resource_state, ResourceState::Deleting);
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_ready_requires_native_running_and_current_wss_with_fixed_capabilities() {
    use crate::services::remote_machine_manager::mark_runtime_online;
    let f = fixture("mgr_ready_handshake").await;
    let bound = bind_runtime(&f).await;
    let mut stopped = bound.clone();
    let observation = stopped.native_observation.as_mut().unwrap();
    observation.running = false;
    observation.suspended = true;
    observation.stop_confirmed = true;
    assert!(
        crate::services::remote_machine_native::activate(&f.db, &stopped)
            .await
            .is_err()
    );
    crate::services::remote_machine_native::activate(&f.db, &bound)
        .await
        .unwrap();
    assert_ne!(current(&f).await.resource_state, ResourceState::Ready);
    let delivery_id = uuid::Uuid::new_v4().to_string();
    lifecycle::accept_bootstrap_delivery(
        &f.db,
        &bound.node_id,
        bound.runtime_generation,
        bound.credential_epoch,
        &delivery_id,
        "local-sealed-delivery-digest",
    )
    .await
    .unwrap();
    let binding = nyxid_machine::managed::Binding {
        node_id: bound.node_id.clone(),
        sandbox_uid: bound.sandbox_uid.clone().unwrap(),
        pod_uid: bound.pod_uid.clone().unwrap(),
        runtime_generation: bound.runtime_generation as u64,
        credential_epoch: bound.credential_epoch as u64,
        writer_fence: bound.writer_fence.clone(),
        operation_id: bound.active_operation_id.clone().unwrap(),
        delivery_id,
    };
    let connection = uuid::Uuid::new_v4().to_string();
    // A native running observation alone cannot complete provisioning.
    assert!(
        mark_runtime_online(&f.db, &bound.node_id, &binding, &connection)
            .await
            .is_err()
    );
    let profile = nyxid_machine::MachineProfile {
        version: nyxid_machine::PROTOCOL_VERSION,
        runtime_id: uuid::Uuid::new_v4().to_string(),
        shell: true,
        files: true,
        browser: Some(false),
        roots: vec!["/workspace".into()],
        authority_versions: vec![nyxid_machine::authority::VERSION],
        commands_isolated: Some(true),
        os: "linux".into(),
        arch: "aarch64".into(),
        ..Default::default()
    };
    let now = Utc::now();
    let lease = doc! {
        "instance_name":"local-manager-test", "generation_id":uuid::Uuid::new_v4().to_string(),
        "connection_id":&connection,"internal_base_url":"http://127.0.0.1",
        "claimed_at":mongodb::bson::DateTime::from_chrono(now),
        "renewed_at":mongodb::bson::DateTime::from_chrono(now),
        "expires_at":mongodb::bson::DateTime::from_chrono(now+chrono::Duration::minutes(5)),
        "capabilities_resolved":true,
    };
    let nodes =
        f.db.collection::<Document>(crate::models::node::COLLECTION_NAME);
    nodes
        .update_one(
            doc! {"_id":&bound.node_id},
            doc! {"$set":{
                "is_active":true,"status":"online","connection_owner":lease,
                "machine":mongodb::bson::to_bson(&profile).unwrap(),
            }},
        )
        .await
        .unwrap();
    let mut stale = binding.clone();
    stale.credential_epoch += 1;
    assert!(
        mark_runtime_online(&f.db, &bound.node_id, &stale, &connection)
            .await
            .is_err()
    );
    assert!(
        mark_runtime_online(&f.db, &bound.node_id, &binding, "another-connection")
            .await
            .is_err()
    );
    nodes
        .update_one(
            doc! {"_id":&bound.node_id},
            doc! {"$set":{"machine.commands_isolated":false}},
        )
        .await
        .unwrap();
    assert!(
        mark_runtime_online(&f.db, &bound.node_id, &binding, &connection)
            .await
            .is_err()
    );
    nodes
        .update_one(
            doc! {"_id":&bound.node_id},
            doc! {"$set":{"machine.commands_isolated":true}},
        )
        .await
        .unwrap();
    mark_runtime_online(&f.db, &bound.node_id, &binding, &connection)
        .await
        .unwrap();
    assert_eq!(current(&f).await.resource_state, ResourceState::Ready);
    nodes.update_one(doc! {"_id":&bound.node_id}, doc! {"$set":{"connection_owner.expires_at":mongodb::bson::DateTime::from_chrono(now-chrono::Duration::seconds(1))}}).await.unwrap();
    assert!(
        mark_runtime_online(&f.db, &bound.node_id, &binding, &connection)
            .await
            .is_err()
    );
}

async fn action_request(
    f: &Fixture,
    machine: &ManagedMachine,
    action: LifecycleAction,
) -> ManagedMachine {
    Box::pin(lifecycle::request_action(
        &f.db,
        lifecycle::LifecycleRequest {
            owner_id: machine.owner_id.clone(),
            machine_id: machine.node_id.clone(),
            action,
            expected_revision: Some(machine.revision),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            request_body: json!({"action":action.as_str(),"revision":machine.revision,"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true}),
            now: None,
        },
    ))
    .await
    .unwrap()
    .machine
}

async fn mount_bound_resources(f: &Fixture, machine: &ManagedMachine, suspended: bool) {
    mount_bound_resources_with_readiness(f, machine, suspended, true).await;
}

async fn mount_bound_resources_with_readiness(
    f: &Fixture,
    machine: &ManagedMachine,
    suspended: bool,
    ready: bool,
) {
    let uid = machine.sandbox_uid.as_deref().unwrap();
    let mut object = sandbox(machine, uid);
    object["status"]["conditions"][0]["status"] =
        json!(if ready && !suspended { "True" } else { "False" });
    object["status"]["conditions"][0]["reason"] = json!(if suspended {
        "SandboxSuspended"
    } else if ready {
        "DependenciesReady"
    } else {
        "DependenciesNotReady"
    });
    object["status"]["conditions"][1]["status"] = json!(if suspended { "True" } else { "False" });
    object["status"]["conditions"][1]["reason"] = json!(if suspended {
        "PodTerminated"
    } else {
        "NotSuspended"
    });
    object["status"]["conditions"][1]["message"] = json!(if suspended {
        "Pod has been terminated. Sandbox is suspended"
    } else {
        "Sandbox is not suspended"
    });
    object["spec"]["operatingMode"] = json!(if suspended { "Suspended" } else { "Running" });
    if suspended {
        object["spec"]
            .as_object_mut()
            .unwrap()
            .remove("shutdownTime");
    }
    Mock::given(method("GET"))
        .and(path(sandbox_path(machine)))
        .respond_with(ResponseTemplate::new(200).set_body_json(object))
        .mount(&f.server)
        .await;
    mount_owned_resources(f, machine, suspended).await;
}

async fn mount_owned_resources(f: &Fixture, machine: &ManagedMachine, suspended: bool) {
    let uid = machine.sandbox_uid.as_deref().unwrap();
    let node_name = machine
        .native_node_name
        .as_deref()
        .unwrap_or("fixture-node");
    let pod_name = machine.native_pod_name.as_deref().unwrap_or("node");
    let pvc_name = machine.pvc_name.as_deref().unwrap_or("workspace");
    let pv_name = machine.native_pv_name.as_deref().unwrap_or("workspace-pv");
    for (resource, kind, name, resource_uid) in [
        ("pods", "Pod", pod_name, machine.pod_uid.as_deref()),
        (
            "persistentvolumeclaims",
            "PersistentVolumeClaim",
            pvc_name,
            machine.pvc_uid.as_deref(),
        ),
    ] {
        let object = resource_uid.filter(|_| resource != "pods" || !suspended).map(|resource_uid| {
            let mut object = controlled_allocation(machine, kind, uid);
            object["metadata"]["name"] = json!(name);
            object["metadata"]["uid"] = json!(resource_uid);
            object["spec"] = if resource == "pods" {
                json!({"nodeName":node_name,"containers":[{"name":"node"}],
                    "volumes":[{"name":"workspace","persistentVolumeClaim":{"claimName":pvc_name}}]})
            } else { json!({"volumeName":pv_name}) };
            object["status"] = json!({"phase":if resource == "pods" {"Running"} else {"Bound"}});
            object
        });
        let items: Vec<_> = object.iter().cloned().collect();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/namespaces/nyxid-machine/{resource}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":items})))
            .mount(&f.server)
            .await;
        let response = object.map_or_else(
            || ResponseTemplate::new(404),
            |object| ResponseTemplate::new(200).set_body_json(object),
        );
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/namespaces/nyxid-machine/{resource}/{name}"
            )))
            .respond_with(response)
            .mount(&f.server)
            .await;
    }
    Mock::given(method("GET")).and(path(format!("/api/v1/nodes/{node_name}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "metadata":{"name":node_name,"uid":machine.native_node_uid.as_deref().unwrap_or(&machine.node_id),"resourceVersion":"9"},
            "status":{"conditions":[{"type":"Ready","status":"True","lastHeartbeatTime":Utc::now().to_rfc3339()}],
                "nodeInfo":{"containerRuntimeVersion":"containerd://2.0-fixture"}}
        }))).mount(&f.server).await;
    Mock::given(method("GET")).and(path(format!("/api/v1/persistentvolumes/{pv_name}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "metadata":{"name":pv_name,"uid":machine.native_pv_uid.as_deref().unwrap_or(&machine.node_id),"resourceVersion":"13"},
            "spec":{"claimRef":{"uid":machine.pvc_uid,"name":pvc_name,"namespace":machine.sandbox_namespace},
                "persistentVolumeReclaimPolicy":"Delete"},"status":{"phase":"Bound"}
        }))).mount(&f.server).await;
}

#[tokio::test]
async fn manager_cancelled_tick_drops_inflight_native_api_observation() {
    use std::time::Duration;
    use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot};

    let mut f = fixture("mgr_native_observation_cancel").await;
    let bound = bind_runtime(&f).await;
    // Skip only fixture effects already represented by its persisted allocation.
    f.db.collection::<Document>(OUTBOX)
        .update_many(
            doc! {"machine_id":&bound.node_id},
            doc! {"$set":{"state":"applied"}},
        )
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    f.manager.api = SandboxApi::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
        None,
    )
    .unwrap();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (closed_tx, closed_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0_u8; 4096];
        let read = stream.read(&mut buffer).await.unwrap();
        let _ = entered_tx.send(String::from_utf8_lossy(&buffer[..read]).to_string());
        // Deliberately return no HTTP response. Dropping the actual reqwest
        // observation future must close this in-flight TCP request.
        let read = stream.read(&mut buffer).await;
        let _ = closed_tx.send(read);
    });
    let manager = f.manager.clone();
    let tick = tokio::spawn(async move { manager.tick().await });
    let entered = tokio::time::timeout(Duration::from_secs(10), entered_rx).await;
    tick.abort();
    let cancelled = tick.await;
    let closed = tokio::time::timeout(Duration::from_secs(5), closed_rx).await;
    server.abort();
    let _ = server.await;
    let request = entered.expect("tick did not reach native API").unwrap();
    assert!(request.starts_with("GET "));
    assert!(request.contains(&sandbox_path(&bound)));
    assert!(cancelled.unwrap_err().is_cancelled());
    assert!(matches!(
        closed
            .expect("cancelled tick retained native HTTP request")
            .unwrap(),
        Ok(0) | Err(_)
    ));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn manager_native_running_can_bootstrap_before_startup_probe_ready() {
    let f = fixture("mgr_bootstrap_before_ready").await;
    let bound = bind_runtime(&f).await;
    mount_bound_resources_with_readiness(&f, &bound, false, false).await;
    // Current applied generation + owned Pod/PVC + healthy Node and exact
    // native cutoff admit controlled bootstrap before the application probe.
    f.manager.observe(&bound).await.unwrap();
    let admitted = current(&f).await;
    assert!(lifecycle::runtime_fence_allows_bootstrap(&admitted));
    assert_ne!(admitted.resource_state, ResourceState::Ready);
    assert!(admitted.native_observation.as_ref().unwrap().running);
    // Commercial Ready still requires the authenticated WSS capability report.
    assert!(
        !lifecycle::runtime_fence_allows_writer(
            &f.db,
            &admitted.node_id,
            admitted.runtime_generation,
            admitted.credential_epoch,
            &admitted.writer_fence,
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn manager_automatic_replacement_is_quarantined_without_adopting_old_jobs() {
    for state in [ResourceState::Ready, ResourceState::Degraded] {
        let f = fixture("mgr_automatic_replacement").await;
        let bound = bind_runtime(&f).await;
        f.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&bound.node_id},
                doc! {"$set":{"resource_state":state.as_str()}},
            )
            .await
            .unwrap();
        let predecessor = current(&f).await;
        let mut replacement = predecessor.clone();
        replacement.pod_uid = Some(uuid::Uuid::new_v4().to_string());
        mount_bound_resources(&f, &replacement, false).await;
        assert!(f.manager.observe(&predecessor).await.is_err());
        assert_eq!(
            current(&f).await.runtime_generation,
            predecessor.runtime_generation
        );
        // Revision4 explicitly forbids automatic successor writers when the
        // old instance became indeterminate. Repeated healthy replacement
        // observations cannot clear the durable uncertainty latch.
        assert!(f.manager.observe(&predecessor).await.is_err());
        let retained = current(&f).await;
        assert_eq!(retained.runtime_generation, predecessor.runtime_generation);
        assert_eq!(retained.credential_epoch, predecessor.credential_epoch);
        assert_eq!(retained.pod_uid, predecessor.pod_uid);
        assert_eq!(retained.pvc_uid, predecessor.pvc_uid);
        assert_eq!(
            retained.native_uncertainty.as_deref(),
            Some("unexpected_pod_replacement")
        );
        assert!(!retained.writer_admitted);
        assert!(!lifecycle::runtime_fence_allows_bootstrap(&retained));
        assert_slot_retained(&f).await;
    }
}

#[tokio::test]
async fn manager_pending_resume_cannot_repair_running_without_predecessor_stop() {
    let f = fixture("mgr_resume_requires_stop").await;
    let bound = bind_runtime(&f).await;
    f.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&bound.node_id},
            doc! {"$set":{"resource_state":"stopping"}},
        )
        .await
        .unwrap();
    let stopping = current(&f).await;
    mount_bound_resources(&f, &stopping, true).await;
    f.manager.observe(&stopping).await.unwrap();
    let paused = current(&f).await;
    assert_eq!(paused.resource_state, ResourceState::Paused);
    let resuming = action_request(&f, &paused, LifecycleAction::Resume).await;
    // A corrupted/legacy missing stop row is not permission to write. This
    // case tests pending recovery, not a claim that missing evidence is normal.
    f.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&resuming.node_id},
            doc! {"$unset":{"native_observation":""}},
        )
        .await
        .unwrap();
    let resuming = current(&f).await;
    f.server.reset().await;
    mount_bound_resources(&f, &resuming, false).await;
    let mut suspended = sandbox(&resuming, resuming.sandbox_uid.as_deref().unwrap());
    suspended["spec"]["operatingMode"] = json!("Suspended");
    suspended["spec"]
        .as_object_mut()
        .unwrap()
        .remove("shutdownTime");
    suspended["status"]["conditions"][0]["status"] = json!("False");
    suspended["status"]["conditions"][0]["reason"] = json!("SandboxSuspended");
    suspended["status"]["conditions"][1]["status"] = json!("False");
    suspended["status"]["conditions"][1]["reason"] = json!("PodTerminating");
    suspended["status"]["conditions"][1]["message"] =
        json!("Pod is terminating. Sandbox is suspending");
    Mock::given(method("PATCH"))
        .and(path(sandbox_path(&resuming)))
        .respond_with(ResponseTemplate::new(200).set_body_json(suspended))
        .expect(1)
        .mount(&f.server)
        .await;
    assert!(f.manager.observe(&resuming).await.is_err());
    let after = current(&f).await;
    assert_eq!(after.runtime_generation, bound.runtime_generation);
    assert_eq!(after.credential_epoch, bound.credential_epoch);
    assert!(!after.writer_admitted);
    let requests = f.server.received_requests().await.unwrap();
    let patches: Vec<_> = requests
        .iter()
        .filter(|r| r.method.as_str() == "PATCH")
        .collect();
    assert_eq!(patches.len(), 1);
    assert_eq!(
        patches[0].body_json::<Value>().unwrap()["spec"]["operatingMode"],
        "Suspended"
    );
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_never_submitted_delete_finishes_without_external_requests() {
    let f = fixture_with_rollout("mgr_unallocated_delete", true).await;
    disable_rollout(&f).await;
    let deleting = delete_request(&f, &f.machine).await;
    let effect =
        f.db.collection::<MachineEffectOutbox>(OUTBOX)
            .find_one(doc! {"operation_id":deleting.active_operation_id.as_deref().unwrap()})
            .await
            .unwrap()
            .unwrap();
    f.manager.apply_effect(&effect).await.unwrap();
    f.manager.observe(&deleting).await.unwrap();
    let deleted = current(&f).await;
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    assert_eq!(deleted.runtime_generation, 0);
    assert!(deleted.cleanup_evidence_digest.is_some());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    let slot =
        f.db.collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
            .find_one(doc! {"_id":&f.machine.owner_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(slot.state, FreeSlotState::Released);
    let audit =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type":"machine_unallocated_delete_proven"})
            .await
            .unwrap()
            .unwrap();
    assert!(audit.get("seq").is_some());
    assert_eq!(
        audit
            .get_document("event_data")
            .unwrap()
            .get_str("proof")
            .unwrap(),
        "generation_zero_never_submitted"
    );
}

#[tokio::test]
async fn manager_historical_unconsented_delete_replay_retains_resources_and_safety_pauses() {
    for owner_state in ["admin_deleted", "absent", "eligible_again"] {
        let f = fixture(&format!("mgr_old_delete_{owner_state}")).await;
        let bound = bind_runtime(&f).await;
        let operation_id = bound.active_operation_id.as_deref().unwrap();
        // Seed the old account-safety shape without creating an owner Delete
        // acceptance audit. Reuse the fixture's durable operation/outbox:
        // bind_runtime intentionally leaves the Machine reconciling, so it
        // must not issue an unrelated owner Pause requiring Ready/Degraded.
        // This is a persisted replay, not new ingress.
        f.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &bound.node_id},
                doc! {"$set":{"resource_state":"deleting","stop_reason":"owner_not_found","cleanup_state":"pending"},"$unset":{"deletion_basis":""}},
            )
            .await
            .unwrap();
        f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":operation_id},
                doc! {"$set":{"action":"delete","cleanup_state":"pending","idempotency_key_hash":"system:historical-account-delete"}},
            )
            .await
            .unwrap();
        f.db.collection::<Document>(OUTBOX)
            .update_one(
                doc! {"operation_id":operation_id},
                doc! {"$set":{"kind":"delete_sandbox","desired_mode":"suspended"}},
            )
            .await
            .unwrap();
        if owner_state == "absent" {
            f.db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                .delete_one(doc! {"_id":&bound.owner_id})
                .await
                .unwrap();
        } else if owner_state == "admin_deleted" {
            f.db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&bound.owner_id},
                    doc! {"$set":{"is_active":false,"deletion_requested_at":mongodb::bson::DateTime::now()}},
                )
                .await
                .unwrap();
        }
        let effect =
            f.db.collection::<MachineEffectOutbox>(OUTBOX)
                .find_one(doc! {"operation_id":operation_id})
                .await
                .unwrap()
                .unwrap();
        for _ in 0..2 {
            let deleting = current(&f).await;
            for error in [
                f.manager.apply_effect(&effect).await.unwrap_err(),
                f.manager.observe(&deleting).await.unwrap_err(),
            ] {
                assert!(
                    error
                        .to_string()
                        .contains("machine_owner_deletion_basis_required")
                );
            }
            assert_slot_retained(&f).await;
            assert_eq!(current(&f).await.pvc_uid, bound.pvc_uid);
        }
        assert!(f.server.received_requests().await.unwrap().is_empty());
        assert_eq!(lifecycle::suspend_ineligible(&f.db).await.unwrap(), 1);
        let retained = current(&f).await;
        assert_eq!(retained.resource_state, ResourceState::Stopping);
        assert_eq!(
            retained.authorization_state,
            crate::models::remote_machine::AuthorizationState::Revoked
        );
        assert!(!retained.writer_admitted);
        assert_eq!(retained.pvc_uid, bound.pvc_uid);
        assert_ne!(retained.active_operation_id.as_deref(), Some(operation_id));
        assert_eq!(lifecycle::suspend_ineligible(&f.db).await.unwrap(), 0);
        f.manager.apply_effect(&effect).await.unwrap();
        assert!(f.server.received_requests().await.unwrap().is_empty());
        assert_slot_retained(&f).await;
    }
}

#[tokio::test]
async fn manager_missing_owner_basis_cannot_be_reconstructed_from_audit() {
    assert_missing_owner_basis_rejected(false).await;
}

#[tokio::test]
async fn manager_missing_owner_basis_with_pending_progress_stays_closed() {
    assert_missing_owner_basis_rejected(true).await;
}

async fn assert_missing_owner_basis_rejected(record_pending_progress: bool) {
    let f = fixture("mgr_legacy_owner_delete").await;
    let deleting = delete_request(&f, &f.machine).await;
    if record_pending_progress {
        f.manager
            .progress(
                deleting.active_operation_id.as_deref().unwrap(),
                "runtime_evidence_pending",
            )
            .await
            .unwrap();
    }
    f.db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&deleting.node_id},
            doc! {"$unset":{"deletion_basis":""}},
        )
        .await
        .unwrap();
    let effect =
        f.db.collection::<MachineEffectOutbox>(OUTBOX)
            .find_one(doc! {"operation_id":deleting.active_operation_id.as_deref().unwrap()})
            .await
            .unwrap()
            .unwrap();
    assert!(f.manager.apply_effect(&effect).await.is_err());
    let stored = current(&f).await;
    assert!(f.manager.observe(&stored).await.is_err());
    assert_eq!(stored.resource_state, ResourceState::Deleting);
    assert!(stored.deletion_basis.is_none());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_slot_retained(&f).await;
}

#[tokio::test]
async fn manager_staged_rekey_advances_once_and_successor_wss_cannot_skip_admission() {
    let f = fixture("mgr_staged_rekey").await;
    let bound = bind_runtime(&f).await;
    let rekeying = action_request(&f, &bound, LifecycleAction::Rekey).await;
    assert_eq!(rekeying.resource_state, ResourceState::Stopping);
    mount_bound_resources(&f, &rekeying, true).await;
    assert!(f.manager.observe(&rekeying).await.is_err());
    let successor = current(&f).await;
    assert_eq!(successor.runtime_generation, bound.runtime_generation + 1);
    assert_eq!(successor.credential_epoch, bound.credential_epoch + 1);
    assert_ne!(successor.writer_fence, bound.writer_fence);
    assert_eq!(successor.node_id, bound.node_id);
    assert_eq!(successor.sandbox_uid, bound.sandbox_uid);
    assert_eq!(successor.pvc_uid, bound.pvc_uid);
    assert!(successor.pod_uid.is_none());
    assert!(!successor.writer_admitted);
    let mut observed_successor = successor.clone();
    observed_successor.pod_uid = Some(uuid::Uuid::new_v4().to_string());
    // Repeating reconciliation cannot consume the old stop a second time.
    // The native successor first receives a finite desired runtime window.
    let successor = crate::services::remote_machine_runtime::mark_submission(&f.db, &successor)
        .await
        .unwrap();
    f.server.reset().await;
    observed_successor.native_run_not_after = successor.native_run_not_after;
    mount_bound_resources(&f, &observed_successor, false).await;
    f.manager.observe(&successor).await.unwrap();
    let after = current(&f).await;
    assert_eq!(after.runtime_generation, successor.runtime_generation);
    assert_eq!(after.credential_epoch, successor.credential_epoch);
    let binding = nyxid_machine::managed::Binding {
        node_id: after.node_id.clone(),
        sandbox_uid: after.sandbox_uid.clone().unwrap(),
        pod_uid: after.pod_uid.clone().unwrap(),
        runtime_generation: after.runtime_generation as u64,
        credential_epoch: after.credential_epoch as u64,
        writer_fence: after.writer_fence.clone(),
        operation_id: after.active_operation_id.clone().unwrap(),
        delivery_id: uuid::Uuid::new_v4().to_string(),
    };
    assert!(
        crate::services::remote_machine_manager::mark_runtime_online(
            &f.db,
            &after.node_id,
            &binding,
            &uuid::Uuid::new_v4().to_string()
        )
        .await
        .is_err()
    );
    assert_ne!(current(&f).await.resource_state, ResourceState::Ready);
    assert_slot_retained(&f).await;
}

#[test]
fn hosted_profile_accepts_only_the_selected_capability_ceiling() {
    let mut machine =
        lifecycle::CreateMachineInput::fixed("owner", true, "image").preview("machine", Utc::now());
    let mut profile = nyxid_machine::MachineProfile {
        version: nyxid_machine::PROTOCOL_VERSION,
        runtime_id: uuid::Uuid::new_v4().to_string(),
        roots: vec!["/workspace".into()],
        authority_versions: vec![2],
        commands_isolated: Some(true),
        shell: true,
        files: true,
        browser: Some(false),
        ..Default::default()
    };
    assert!(super::accepts_selected_profile(&machine, &profile));
    profile.browser = Some(true);
    assert!(!super::accepts_selected_profile(&machine, &profile));
    machine.capabilities = vec!["browser".into(), "computer".into()];
    profile.shell = false;
    profile.files = false;
    profile.computer = true;
    profile.browser_tools = true;
    assert!(super::accepts_selected_profile(&machine, &profile));
    machine.capabilities = vec!["browser".into()];
    profile.computer = false;
    profile.computer_ready = true;
    profile.computer_tools = vec!["screenshot".into()];
    assert!(
        super::accepts_selected_profile(&machine, &profile),
        "browser-only uses cua without granting Computer"
    );
    profile.computer = true;
    assert!(!super::accepts_selected_profile(&machine, &profile));
    machine.capabilities.push("computer".into());
    profile.browser = Some(false);
    assert!(!super::accepts_selected_profile(&machine, &profile));
    profile.browser = Some(true);
    profile.commands_isolated = Some(false);
    assert!(!super::accepts_selected_profile(&machine, &profile));
}
