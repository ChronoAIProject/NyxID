//! Revision4 native lifecycle tests. The Manager, SandboxApi, lifecycle/ACL
//! transactions and bootstrap gate are production code; Kubernetes and the
//! Agent Sandbox controller are an explicit in-process HTTP/state fixture.
//! PATCH/DELETE acceptance is deliberately separate from controller progress.
//! The Ready transition uses real sealed bootstrap and mark_runtime_online with
//! a local workload-identity verifier and explicit leased Node capability state;
//! it does not test WSS authentication/transport (covered by public journeys).
//! No RuntimeProvider, signed stop receipt or physical erasure assertion exists.
use super::Manager;
use crate::{
    models::{
        machine_free_slot::{FreeSlotState, MachineFreeSlot},
        remote_machine::{
            AuthorizationState, BillingState, COLLECTION_NAME as MACHINES, ManagedMachine,
            ResourceState,
        },
        remote_machine_operation::{
            COLLECTION_NAME as OPERATIONS, CleanupState, LifecycleAction, MachineOperation,
            OperationState,
        },
        remote_machine_outbox::{COLLECTION_NAME as OUTBOX, MachineEffectOutbox},
        user::UserType,
    },
    services::{
        remote_machine_native as native, remote_machine_runtime as runtime,
        remote_machine_service::{self as lifecycle, CreateMachineInput, LifecycleRequest},
    },
    test_utils::{connect_transaction_test_database, test_user},
};
use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use nyxid_machine_updater::sandbox::{SandboxApi, SandboxObservation};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct Cluster {
    sandbox: Option<Value>,
    pod: Option<Value>,
    pvc: Option<Value>,
    pv: Option<Value>,
    sandbox_uid: String,
    node_uid: String,
    revision: i64,
    node_ready: bool,
    node_stale: bool,
    resource_list_pages: std::collections::HashMap<String, Vec<Value>>,
    delete_requested: bool,
}
impl Cluster {
    fn new() -> Self {
        Self {
            sandbox_uid: Uuid::new_v4().to_string(),
            node_uid: Uuid::new_v4().to_string(),
            node_ready: true,
            ..Default::default()
        }
    }
    fn respond(&mut self, request: &wiremock::Request) -> ResponseTemplate {
        let path = request.url.path();
        let sandbox_item = path.contains("/sandboxes/");
        if request.method.as_str() == "PATCH" && sandbox_item {
            let mut next: Value = serde_json::from_slice(&request.body).unwrap();
            if let Some(patch) = next.as_array() {
                let mut original = self
                    .sandbox
                    .clone()
                    .expect("patch requires original Sandbox");
                for operation in patch {
                    let path = operation["path"].as_str().unwrap();
                    match operation["op"].as_str().unwrap() {
                        "test" => {
                            if original.pointer(path) != Some(&operation["value"]) {
                                return ResponseTemplate::new(409);
                            }
                        }
                        "add" => {
                            *original
                                .pointer_mut(path)
                                .expect("known minimal stop patch path") =
                                operation["value"].clone()
                        }
                        "remove" => {
                            assert_eq!(path, "/spec/shutdownTime");
                            original["spec"]
                                .as_object_mut()
                                .unwrap()
                                .remove("shutdownTime");
                        }
                        other => panic!("unexpected stop patch operation {other}"),
                    }
                }
                next = original;
            }
            if self.sandbox.as_ref().is_some_and(|old| {
                next["metadata"]["uid"] != old["metadata"]["uid"]
                    || next["metadata"]["resourceVersion"] != old["metadata"]["resourceVersion"]
            }) {
                return ResponseTemplate::new(409);
            }
            self.revision += 1;
            let old_generation = self
                .sandbox
                .as_ref()
                .and_then(|s| s["metadata"]["generation"].as_i64())
                .unwrap_or(0);
            let unchanged = self
                .sandbox
                .as_ref()
                .is_some_and(|s| s["spec"] == next["spec"]);
            next["metadata"]["uid"] = json!(self.sandbox_uid);
            next["metadata"]["generation"] = json!(old_generation + i64::from(!unchanged));
            next["metadata"]["resourceVersion"] = json!(self.revision.to_string());
            next["status"] = self
                .sandbox
                .as_ref()
                .map(|s| s["status"].clone())
                .unwrap_or(json!({}));
            self.sandbox = Some(next.clone());
            return ResponseTemplate::new(200).set_body_json(next);
        }
        if request.method.as_str() == "DELETE" && sandbox_item {
            let expected: Value = serde_json::from_slice(&request.body).unwrap();
            if expected["preconditions"]["uid"] != self.sandbox_uid {
                return ResponseTemplate::new(409);
            }
            self.delete_requested = true;
            if let Some(sandbox) = &mut self.sandbox {
                sandbox["metadata"]["deletionTimestamp"] = json!(Utc::now().to_rfc3339());
                sandbox["metadata"]["finalizers"] = json!(["fixture.agents.x-k8s.io/cleanup"]);
            }
            return ResponseTemplate::new(202).set_body_json(json!({"status":"Success"}));
        }
        if request.method.as_str() != "GET" {
            return ResponseTemplate::new(405);
        }
        if let Some(pages) = self.resource_list_pages.get(path) {
            let page = usize::from(request.url.query_pairs().any(|(key, _)| key == "continue"));
            return ResponseTemplate::new(200).set_body_json(pages[page].clone());
        }
        if path == "/apis/storage.k8s.io/v1/storageclasses/existing-fixture-storage" {
            return ResponseTemplate::new(200).set_body_json(json!({"kind":"StorageClass","apiVersion":"storage.k8s.io/v1","metadata":{"name":"existing-fixture-storage","uid":"storage-fixture"},"provisioner":"fixture.csi.example"}));
        }
        if path == "/api/v1/pods" {
            return ResponseTemplate::new(403);
        }
        let object = if sandbox_item {
            self.sandbox.clone()
        } else if path.ends_with("/sandboxes") {
            return Self::list(
                "SandboxList",
                "agents.x-k8s.io/v1beta1",
                self.sandbox.clone(),
            );
        } else if path.ends_with("/pods") {
            return Self::list("PodList", "v1", self.pod.clone());
        } else if path.ends_with("/persistentvolumeclaims") {
            return Self::list("PersistentVolumeClaimList", "v1", self.pvc.clone());
        } else if path.ends_with("/pods/native-pod") {
            self.pod.clone()
        } else if path.ends_with("/persistentvolumeclaims/workspace") {
            self.pvc.clone()
        } else if path == "/api/v1/persistentvolumes" {
            return Self::list("PersistentVolumeList", "v1", self.pv.clone());
        } else if path == "/apis/storage.k8s.io/v1/volumeattachments" {
            return Self::list("VolumeAttachmentList", "storage.k8s.io/v1", None);
        } else if path == "/api/v1/persistentvolumes/native-pv" {
            self.pv.clone()
        } else if path == "/api/v1/nodes/native-node" {
            Some(
                json!({"apiVersion":"v1","kind":"Node", "metadata":{"name":"native-node","uid":self.node_uid,"resourceVersion":"1"},
                    "status":{"nodeInfo":{"containerRuntimeVersion":"containerd://fixture-only"},"conditions":[{"type":"Ready","status":if self.node_ready{"True"}else{"Unknown"},"lastHeartbeatTime":(Utc::now()-Duration::seconds(if self.node_stale{600}else{0})).to_rfc3339()}]}}),
            )
        } else {
            None
        };
        match object {
            Some(object) => ResponseTemplate::new(200).set_body_json(object),
            None => ResponseTemplate::new(404),
        }
    }
    fn list(kind: &str, version: &str, mut object: Option<Value>) -> ResponseTemplate {
        if kind != "SandboxList"
            && let Some(value) = &mut object
            && value["kind"].as_str() == kind.strip_suffix("List")
            && value["apiVersion"] == version
        {
            let fields = value.as_object_mut().unwrap();
            fields.remove("kind");
            fields.remove("apiVersion");
        }
        ResponseTemplate::new(200)
            .set_body_json(json!({"kind":kind,"apiVersion":version,"metadata":{"resourceVersion":"complete-fixture-snapshot"},"items":object.into_iter().collect::<Vec<_>>()}))
    }
    /// The fixture controller alone creates/removes Pod/PVC objects. The tested
    /// Manager only writes the Sandbox API, matching the selected ownership.
    fn reconcile(&mut self) {
        let sandbox = self.sandbox.as_mut().expect("accepted Sandbox intent");
        let generation = sandbox["metadata"]["generation"].as_i64().unwrap();
        let suspended = sandbox["spec"]["operatingMode"] == "Suspended";
        let labels = sandbox["spec"]["podTemplate"]["metadata"]["labels"].clone();
        let namespace = sandbox["metadata"]["namespace"].clone();
        let owner = json!([{"apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox","name":sandbox["metadata"]["name"],"uid":self.sandbox_uid,"controller":true}]);
        sandbox["status"] = json!({"conditions":[
            {"type":"Ready","status":if suspended{"False"}else{"True"},"observedGeneration":generation,"lastTransitionTime":Utc::now().to_rfc3339()},
            {"type":"Suspended","status":if suspended{"True"}else{"False"},"observedGeneration":generation,"lastTransitionTime":Utc::now().to_rfc3339()}
        ]});
        if suspended {
            sandbox["status"]["conditions"][1]["reason"] = json!("PodTerminated");
        }
        if self.pvc.is_none() {
            let pvc_uid = Uuid::new_v4().to_string();
            self.pvc = Some(json!({"apiVersion":"v1","kind":"PersistentVolumeClaim",
                "metadata":{"name":"workspace","namespace":namespace,"uid":pvc_uid,"resourceVersion":"1","labels":sandbox["spec"]["volumeClaimTemplates"][0]["metadata"]["labels"],"ownerReferences":owner},
                "spec":{"volumeName":"native-pv","storageClassName":"existing-fixture-storage"},"status":{"phase":"Bound"}}));
            self.pv = Some(
                json!({"apiVersion":"v1","kind":"PersistentVolume", "metadata":{"name":"native-pv","uid":Uuid::new_v4().to_string(),"resourceVersion":"1"},
                "spec":{"claimRef":{"uid":pvc_uid,"namespace":namespace,"name":"workspace"},"persistentVolumeReclaimPolicy":"Delete"},"status":{"phase":"Bound"}}),
            );
        }
        if suspended {
            self.pod = None;
        } else if self.pod.is_none() {
            let mut spec = sandbox["spec"]["podTemplate"]["spec"].clone();
            spec["nodeName"] = json!("native-node");
            spec["volumes"].as_array_mut().unwrap().push(
                json!({"name":"workspace","persistentVolumeClaim":{"claimName":"workspace"}}),
            );
            self.pod = Some(json!({"apiVersion":"v1","kind":"Pod",
                "metadata":{"name":"native-pod","namespace":namespace,"uid":Uuid::new_v4().to_string(),"resourceVersion":generation.to_string(),"labels":labels,"ownerReferences":owner},
                "spec":spec,"status":{"phase":"Running"}}));
        }
    }
    fn finish_delete(&mut self, remove_pv: bool) {
        assert!(
            self.delete_requested,
            "fixture GC only follows actual Sandbox DELETE"
        );
        self.sandbox = None;
        self.pod = None;
        self.pvc = None;
        if remove_pv {
            self.pv = None;
        }
    }
}
struct LocalWorkloadIdentity {
    nonce: String,
    recipient_digest: [u8; 32],
}
#[async_trait::async_trait]
impl crate::services::remote_machine_bootstrap::BootstrapTicketVerifier for LocalWorkloadIdentity {
    async fn verify(
        &self,
        ticket: &Value,
        binding: &nyxid_machine::managed::Binding,
    ) -> crate::errors::AppResult<crate::services::remote_machine_bootstrap::VerifiedBootstrapTicket>
    {
        if ticket["local_workload_nonce"].as_str() != Some(self.nonce.as_str()) {
            return Err(crate::errors::AppError::MachineNotAllowed);
        }
        Ok(crate::services::remote_machine_bootstrap::VerifiedBootstrapTicket::from_provider_verified_claims(
            binding.clone(),self.recipient_digest,Utc::now()+Duration::minutes(5)
        ))
    }
}

struct Fixture {
    db: Database,
    _server: MockServer,
    cluster: Arc<Mutex<Cluster>>,
    manager: Manager,
    machine: ManagedMachine,
}
impl Fixture {
    async fn current(&self) -> ManagedMachine {
        lifecycle::get_machine(&self.db, &self.machine.owner_id, &self.machine.node_id)
            .await
            .unwrap()
            .unwrap()
    }
    async fn effect(&self) -> MachineEffectOutbox {
        let machine = self.current().await;
        self.db.collection::<MachineEffectOutbox>(OUTBOX).find_one(doc!{"operation_id":&machine.active_operation_id,"kind":{"$ne":"install_provider_deadline"}}).await.unwrap().unwrap()
    }
    async fn action(&self, action: LifecycleAction) -> ManagedMachine {
        let machine = self.current().await;
        Box::pin(lifecycle::request_action(&self.db,LifecycleRequest {
            owner_id:machine.owner_id.clone(),machine_id:machine.node_id.clone(),action,
            expected_revision:Some(machine.revision),idempotency_key:Uuid::new_v4().to_string(),
            request_body:json!({"action":action.as_str(),"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true}),now:None,
        })).await.unwrap().machine
    }
    async fn apply_delete_until_controller_progress(&self) {
        let deleting = self.current().await;
        let operation_id = deleting.active_operation_id.clone().unwrap();
        let original = self.cluster.lock().unwrap().sandbox.clone().unwrap();
        let original_pod = self.cluster.lock().unwrap().pod.clone();
        let result = Box::pin(self.manager.apply_effect(&self.effect().await)).await;
        if deleting.sandbox_uid.is_none() {
            let error = result.unwrap_err();
            assert!(
                matches!(error, crate::errors::AppError::Conflict(ref reason) if reason == "delete_current_suspension_unconfirmed"),
                "lost-create recovery still awaits the current controller generation: {error}"
            );
        } else {
            // Applying the minimal stop intent may succeed before the fixture
            // controller has observed its new generation.
            result.unwrap();
        }
        let suspended = self.cluster.lock().unwrap().sandbox.clone().unwrap();
        assert_eq!(suspended["metadata"]["uid"], original["metadata"]["uid"]);
        assert_eq!(suspended["spec"]["operatingMode"], "Suspended");
        assert_eq!(suspended["status"], original["status"]);
        assert_ne!(
            suspended["status"]["conditions"][1]["observedGeneration"],
            suspended["metadata"]["generation"],
            "PATCH acceptance does not advance the fixture controller"
        );
        assert_eq!(self.cluster.lock().unwrap().pod, original_pod);
        for template in ["podTemplate", "volumeClaimTemplates"] {
            assert_eq!(
                suspended["spec"][template], original["spec"][template],
                "suspension preserves the original {template}"
            );
        }
        self.assert_delete_pending_before_destructive_effects(
            &operation_id,
            deleting.authorization_state,
        )
        .await;
        // Prove the actual pending gate on a fresh due pass. An accepted stop
        // PATCH alone is neither a controller stop nor permission to delete.
        self.make_original_delete_due().await;
        let error = self
            .manager
            .observe(&self.current().await)
            .await
            .unwrap_err();
        assert!(
            matches!(error, crate::errors::AppError::Conflict(ref reason) if reason == "delete_current_suspension_unconfirmed"),
            "the controller has not yet observed the current suspended generation: {error}"
        );
        self.assert_delete_pending_before_destructive_effects(
            &operation_id,
            deleting.authorization_state,
        )
        .await;
    }
    async fn assert_delete_pending_before_destructive_effects(
        &self,
        operation_id: &str,
        accepted_authorization: AuthorizationState,
    ) {
        let machine = self.current().await;
        assert_eq!(machine.resource_state, ResourceState::Deleting);
        assert_eq!(machine.active_operation_id.as_deref(), Some(operation_id));
        assert!(!machine.writer_admitted);
        assert_eq!(machine.authorization_state, accepted_authorization);
        assert!(!lifecycle::runtime_fence_allows_bootstrap(&machine));
        assert!(!lifecycle::has_current_verified_runtime_evidence(&machine));
        assert!(
            !lifecycle::runtime_fence_allows_writer(
                &self.db,
                &machine.node_id,
                machine.runtime_generation,
                machine.credential_epoch,
                &machine.writer_fence,
            )
            .await
            .unwrap()
        );
        let node = self
            .db
            .collection::<Document>(crate::models::node::COLLECTION_NAME)
            .find_one(doc! {"_id":&machine.node_id,"user_id":&machine.owner_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.get_bool("is_active"), Ok(false));
        assert_eq!(node.get_str("status"), Ok("offline"));
        assert_eq!(machine.cleanup_state, "pending");
        assert_ne!(machine.billing_state, BillingState::Closed);
        let operation = self
            .db
            .collection::<MachineOperation>(OPERATIONS)
            .find_one(doc! {"_id":operation_id,"owner_id":&machine.owner_id,"machine_id":&machine.node_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(operation.action, LifecycleAction::Delete);
        assert!(matches!(
            operation.state,
            OperationState::Accepted | OperationState::Applying | OperationState::Reconciling
        ));
        assert_ne!(operation.cleanup_state, CleanupState::Complete);
        assert!(operation.terminal_at.is_none());
        assert!(!self.cluster.lock().unwrap().delete_requested);
        assert!(
            self._server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method.as_str() != "DELETE"),
            "no destructive request precedes confirmed controller and storage observations"
        );
        self.slot_retained().await;
    }
    fn reconcile_delete_suspension(&self) {
        let mut cluster = self.cluster.lock().unwrap();
        cluster.reconcile();
        let sandbox = cluster.sandbox.as_ref().unwrap();
        assert_eq!(sandbox["spec"]["operatingMode"], "Suspended");
        assert_eq!(sandbox["status"]["conditions"][1]["status"], "True");
        assert_eq!(
            sandbox["status"]["conditions"][1]["observedGeneration"],
            sandbox["metadata"]["generation"]
        );
        assert!(cluster.pod.is_none());
        assert!(!cluster.delete_requested);
    }
    fn cleanup_inventory_path(&self, resource: &str) -> String {
        match resource {
            // Recovery also finds original UID/claim users outside owner-label
            // selectors, so Pods are inventoried through the cluster API.
            "pods" => "/api/v1/namespaces/nyxid-machine/pods".into(),
            "persistentvolumeclaims" => format!(
                "/api/v1/namespaces/{}/{resource}",
                self.machine.sandbox_namespace
            ),
            _ => panic!("unsupported cleanup inventory fixture"),
        }
    }
    async fn make_original_delete_due(&self) {
        let machine = self.current().await;
        assert_eq!(machine.resource_state, ResourceState::Deleting);
        // Controller/GC progress is explicit in this fixture. Advance only the
        // accepted original delete's retry time, without changing its evidence.
        let result = self
            .db
            .collection::<Document>(OPERATIONS)
            .update_one(
                doc! {"_id":&machine.active_operation_id,"owner_id":&machine.owner_id,
                "machine_id":&machine.node_id,"action":"delete","terminal_at":bson::Bson::Null},
                doc! {"$set":{"next_attempt_at":bson::Bson::Null}},
            )
            .await
            .unwrap();
        assert_eq!(result.matched_count, 1);
    }
    async fn observation(&self) -> SandboxObservation {
        self.manager
            .api
            .get(&self.machine.sandbox_namespace, &self.machine.sandbox_name)
            .await
            .unwrap()
            .unwrap()
    }
    async fn record(&self) -> crate::errors::AppResult<()> {
        self.manager
            .record_observation(&self.current().await, &self.observation().await)
            .await
    }
    async fn report_online(&self) {
        use crate::services::remote_machine_bootstrap as bootstrap;
        let machine = self.current().await;
        assert!(lifecycle::runtime_fence_allows_bootstrap(&machine));
        let key = nyxid_machine::bootstrap::generate_recipient().unwrap();
        let verifier = LocalWorkloadIdentity {
            nonce: Uuid::new_v4().to_string(),
            recipient_digest: nyxid_machine::bootstrap::recipient_key_digest(
                &key.public_key_b64u().unwrap(),
            )
            .unwrap(),
        };
        let ticket = json!({"local_workload_nonce":verifier.nonce});
        let binding = bootstrap::prepare_workload(
            &self.db,
            &verifier,
            &machine.node_id,
            machine.pod_uid.as_deref().unwrap(),
            &ticket,
            &key.public_key_b64u().unwrap(),
        )
        .await
        .unwrap();
        bootstrap::bootstrap(
            &self.db,
            &crate::test_utils::test_encryption_keys(),
            &verifier,
            bootstrap::BootstrapInput {
                binding: binding.clone(),
                ticket,
                recipient_public_key: key.public_key_b64u().unwrap(),
            },
        )
        .await
        .unwrap();
        let state = crate::test_utils::test_app_state(self.db.clone());
        let (sender, _receiver) = tokio::sync::mpsc::channel(16);
        let connection =
            crate::test_utils::register_test_node_connection(&state, &machine.node_id, sender)
                .await;
        let profile = nyxid_machine::MachineProfile {
            version: nyxid_machine::PROTOCOL_VERSION,
            runtime_id: Uuid::new_v4().to_string(),
            shell: true,
            files: true,
            browser: Some(false),
            roots: vec!["/workspace".into()],
            authority_versions: vec![2, 3],
            commands_isolated: Some(true),
            os: "linux".into(),
            arch: "aarch64".into(),
            ..Default::default()
        };
        // Explicit transport fixture: production managed admission still checks
        // this exact leased owner, capability report, native identity and cutoff.
        self.db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(
            doc!{"_id":&machine.node_id,"connection_owner.connection_id":&connection},
            doc!{"$set":{"status":"online","connection_owner.capabilities_resolved":true,"machine":bson::to_bson(&profile).unwrap()}}
        ).await.unwrap();
        super::mark_runtime_online(&self.db, &machine.node_id, &binding, &connection)
            .await
            .unwrap();
    }

    async fn start(&self) -> ManagedMachine {
        Box::pin(self.manager.apply_effect(&self.effect().await))
            .await
            .unwrap();
        assert!(
            !self.current().await.writer_admitted,
            "accepted PATCH is not bootstrap admission"
        );
        self.cluster.lock().unwrap().reconcile();
        Box::pin(self.manager.observe(&self.current().await))
            .await
            .unwrap();
        self.report_online().await;
        self.current().await
    }
    async fn slot_retained(&self) {
        let slot = self
            .db
            .collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
            .find_one(doc! {"_id":&self.machine.owner_id})
            .await
            .unwrap()
            .unwrap();
        assert_ne!(slot.state, FreeSlotState::Released);
    }
}
async fn fixture(label: &str) -> Fixture {
    let db = connect_transaction_test_database(label).await;
    lifecycle::ensure_indexes(&db).await.unwrap();
    crate::services::remote_machine_bootstrap::ensure_indexes(&db)
        .await
        .unwrap();
    crate::services::remote_machine_events::ensure_indexes(&db)
        .await
        .unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let mut input = CreateMachineInput::fixed(
        &owner,
        true,
        &format!("registry.test/node@sha256:{}", "a".repeat(64)),
    );
    input.idempotency_key = Uuid::new_v4().to_string();
    input.profile.runtime_class = None;
    let created = Box::pin(lifecycle::create_machine(&db, input))
        .await
        .unwrap();
    let server = MockServer::start().await;
    let cluster = Arc::new(Mutex::new(Cluster::new()));
    let state = cluster.clone();
    Mock::given(|_: &wiremock::Request| true)
        .respond_with(move |r: &wiremock::Request| state.lock().unwrap().respond(r))
        .mount(&server)
        .await;
    let manager = Manager {
        db: db.clone(),
        billing: None,
        api: SandboxApi::new(server.uri().parse().unwrap(), None).unwrap(),
        storage_class: Some("existing-fixture-storage".into()),
        namespace: "nyxid-machine".into(),
        bootstrap_base_url: "https://native.local.test".into(),
    };
    Fixture {
        db,
        _server: server,
        cluster,
        manager,
        machine: created.machine,
    }
}

#[tokio::test]
async fn native_free_lifecycle_uses_applied_observations_same_pvc_and_fresh_bootstrap() {
    let mut f = fixture("native_free_lifecycle").await;
    let initial = f.start().await;
    assert!(initial.provider_coverage.is_none());
    assert!(initial.writer_admitted);
    assert!(lifecycle::runtime_fence_allows_bootstrap(&initial));
    assert!(initial.profile.runtime_class.is_none());
    assert!(
        f.cluster.lock().unwrap().sandbox.as_ref().unwrap()["spec"]["podTemplate"]["spec"]
            .get("runtimeClassName")
            .is_none()
    );
    let original_template =
        f.cluster.lock().unwrap().sandbox.as_ref().unwrap()["spec"]["volumeClaimTemplates"].clone();
    let pvc = initial.pvc_uid.clone();
    let pod = initial.pod_uid.clone();
    let generation = initial.runtime_generation;
    f.action(LifecycleAction::Pause).await;
    Box::pin(f.manager.apply_effect(&f.effect().await))
        .await
        .unwrap();
    assert!(!lifecycle::runtime_fence_allows_bootstrap(
        &f.current().await
    ));
    assert!(
        Box::pin(f.manager.observe(&f.current().await))
            .await
            .is_err()
    );
    assert_eq!(f.current().await.resource_state, ResourceState::Stopping);
    f.cluster.lock().unwrap().reconcile();
    Box::pin(f.manager.observe(&f.current().await))
        .await
        .unwrap();
    assert_eq!(f.current().await.resource_state, ResourceState::Paused);
    f.slot_retained().await;
    f.manager.storage_class = Some("changed-and-unavailable-class".into());
    f.action(LifecycleAction::Resume).await;
    Box::pin(f.manager.apply_effect(&f.effect().await))
        .await
        .unwrap();
    assert!(!f.current().await.writer_admitted);
    f.cluster.lock().unwrap().reconcile();
    Box::pin(f.manager.observe(&f.current().await))
        .await
        .unwrap();
    let resumed = f.current().await;
    assert_eq!(resumed.node_id, initial.node_id);
    assert_eq!(resumed.pvc_uid, pvc);
    assert_eq!(
        f.cluster.lock().unwrap().sandbox.as_ref().unwrap()["spec"]["volumeClaimTemplates"],
        original_template
    );
    assert_ne!(resumed.pod_uid, pod);
    assert!(resumed.runtime_generation > generation);
    assert!(lifecycle::runtime_fence_allows_bootstrap(&resumed));
    assert!(
        resumed.bootstrap_digest.is_none(),
        "new generation must retrieve fresh bootstrap credentials"
    );
    f.action(LifecycleAction::Delete).await;
    f.apply_delete_until_controller_progress().await;
    f.reconcile_delete_suspension();
    f.make_original_delete_due().await;
    assert!(
        Box::pin(f.manager.observe(&f.current().await))
            .await
            .is_err()
    );
    assert_eq!(f.current().await.resource_state, ResourceState::Deleting);
    f.slot_retained().await;
    f.cluster.lock().unwrap().finish_delete(true);
    f.make_original_delete_due().await;
    Box::pin(f.manager.observe(&f.current().await))
        .await
        .unwrap();
    assert_eq!(f.current().await.resource_state, ResourceState::Deleted);
    assert_eq!(
        f.current().await.authorization_state,
        AuthorizationState::Revoked
    );
    let slot =
        f.db.collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
            .find_one(doc! {"_id":&f.machine.owner_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(slot.state, FreeSlotState::Released);
    assert!(
        f._server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method.as_str() != "GET")
            .all(|r| r.url.path().contains("/sandboxes/")),
        "only the fixture controller owns Pod/PVC writes"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_pause_needs_current_suspended_condition_and_absent_associated_pod() {
    let f = fixture("native_suspend_applied").await;
    f.start().await;
    f.action(LifecycleAction::Pause).await;
    Box::pin(f.manager.apply_effect(&f.effect().await))
        .await
        .unwrap();
    f.record().await.unwrap();
    assert!(!native::predecessor_stopped(&f.current().await));
    let old_pod = f.cluster.lock().unwrap().pod.clone();
    f.cluster.lock().unwrap().reconcile();
    {
        let mut c = f.cluster.lock().unwrap();
        let sandbox = c.sandbox.as_mut().unwrap();
        let generation = sandbox["metadata"]["generation"].as_i64().unwrap();
        sandbox["status"]["conditions"][1]["observedGeneration"] = json!(generation - 1);
    }
    f.record().await.unwrap();
    assert!(
        !native::predecessor_stopped(&f.current().await),
        "stale applied condition cannot confirm pause"
    );
    f.cluster.lock().unwrap().reconcile();
    f.cluster.lock().unwrap().pod = old_pod;
    f.record().await.unwrap();
    assert!(
        !native::predecessor_stopped(&f.current().await),
        "Suspended with associated Pod still present is pending"
    );
    f.cluster.lock().unwrap().pod = None;
    f.record().await.unwrap();
    assert!(native::predecessor_stopped(&f.current().await));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_observation_rejects_late_operation_generation_and_stale_worker_replay() {
    let f = fixture("native_stale_identity").await;
    let current = f.start().await;
    let original = f.observation().await;
    let mut stale = original.clone();
    stale.operation_id = Some(Uuid::new_v4().to_string());
    assert!(
        f.manager
            .record_observation(&current, &stale)
            .await
            .is_err()
    );
    stale = original.clone();
    stale.runtime_generation = Some((current.runtime_generation + 1).to_string());
    assert!(
        f.manager
            .record_observation(&current, &stale)
            .await
            .is_err()
    );
    let o = current.native_observation.clone().unwrap();
    assert!(
        native::record(&f.db, &current, &o, Document::new())
            .await
            .is_ok()
    );
    assert!(
        native::record(&f.db, &current, &o, Document::new())
            .await
            .is_err(),
        "stale revision cannot replay observation over successor state"
    );
    let mut expired = f.current().await;
    expired.native_observation.as_mut().unwrap().observed_at = Utc::now() - Duration::seconds(61);
    assert!(!lifecycle::runtime_fence_allows_bootstrap(&expired));
    assert!(!runtime::execution_coverage_is_live(&expired, Utc::now()));
    // A fresh HTTP response cannot conceal an already missed native-observation
    // window. The old admitted writer is quarantined before any new admission.
    f.db.collection::<Document>(MACHINES)
        .update_one(
            doc! {"_id":&current.node_id},
            doc! {"$set":{
                "resource_state":"ready",
                "native_observation.observed_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(61))
            }},
        )
        .await
        .unwrap();
    assert!(f.record().await.is_err());
    let quarantined = f.current().await;
    assert!(quarantined.native_uncertainty.is_some());
    assert!(!quarantined.writer_admitted);
    assert!(!lifecycle::runtime_fence_allows_bootstrap(&quarantined));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_force_delete_then_notfound_never_unlocks_old_workspace() {
    let f = fixture("native_force_delete").await;
    f.start().await;
    {
        let mut c = f.cluster.lock().unwrap();
        c.pod.as_mut().unwrap()["metadata"]["deletionTimestamp"] = json!(Utc::now().to_rfc3339());
        c.pod.as_mut().unwrap()["metadata"]["deletionGracePeriodSeconds"] = json!(0);
    }
    assert!(f.record().await.is_err());
    assert_eq!(
        f.current().await.native_uncertainty.as_deref(),
        Some("known_force_deletion")
    );
    f.cluster.lock().unwrap().pod = None;
    f.cluster.lock().unwrap().sandbox = None;
    let _ = Box::pin(f.manager.observe(&f.current().await)).await;
    let quarantined = f.current().await;
    assert!(!quarantined.writer_admitted);
    assert!(!lifecycle::runtime_fence_allows_bootstrap(&quarantined));
    assert!(!native::predecessor_stopped(&quarantined));
    assert!(
        lifecycle::advance_runtime_generation(
            &f.db,
            &quarantined.node_id,
            quarantined.revision,
            quarantined.runtime_generation,
            None,
            None
        )
        .await
        .is_err()
    );
    f.slot_retained().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_node_uncertainty_and_replacement_are_latched_before_new_bootstrap() {
    for (label, change, reason) in [
        ("unhealthy", 0, "predecessor_node_unhealthy_or_stale"),
        ("stale", 1, "predecessor_node_unhealthy_or_stale"),
        ("replacement", 2, "unexpected_pod_replacement"),
        ("ownership", 3, "resource_ownership_conflict"),
    ] {
        let f = fixture(&format!("native_{label}")).await;
        f.start().await;
        {
            let mut c = f.cluster.lock().unwrap();
            match change {
                0 => c.node_ready = false,
                1 => c.node_stale = true,
                2 => c.pod.as_mut().unwrap()["metadata"]["uid"] = json!(Uuid::new_v4().to_string()),
                _ => {
                    c.pod.as_mut().unwrap()["metadata"]["labels"]["nyxid.dev/owner-id"] =
                        json!(Uuid::new_v4().to_string())
                }
            }
        }
        assert!(f.record().await.is_err());
        let current = f.current().await;
        assert_eq!(current.native_uncertainty.as_deref(), Some(reason));
        assert!(!current.writer_admitted);
        assert!(!lifecycle::runtime_fence_allows_bootstrap(&current));
        let first = crate::services::remote_machine_events::list_investigations(&f.db, None, None)
            .await
            .unwrap();
        assert_eq!(first.rows.len(), 1);
        assert_eq!(
            first.rows[0].get_str("machine_id"),
            Ok(current.node_id.as_str())
        );
        assert_eq!(
            first.rows[0].get_str("reason"),
            Ok("native_runtime_uncertainty")
        );
        f.cluster.lock().unwrap().node_ready = true;
        f.cluster.lock().unwrap().node_stale = false;
        assert!(
            f.record().await.is_err(),
            "later healthy reads do not clear quarantine"
        );
        let repeated =
            crate::services::remote_machine_events::list_investigations(&f.db, None, None)
                .await
                .unwrap();
        assert_eq!(repeated.rows.len(), 1);
        assert_eq!(
            repeated.rows[0].get_str("_id"),
            first.rows[0].get_str("_id")
        );
        assert_eq!(
            repeated.rows[0].get_str("reason"),
            Ok("native_runtime_uncertainty")
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn native_delete_waits_for_object_gc_without_observing_retained_pv() {
    let f = fixture("native_delete_retained").await;
    f.start().await;
    {
        let mut c = f.cluster.lock().unwrap();
        c.pv.as_mut().unwrap()["spec"]["persistentVolumeReclaimPolicy"] = json!("Retain");
        c.pv.as_mut().unwrap()["metadata"]["finalizers"] = json!(["kubernetes.io/pv-protection"]);
    }
    f.record().await.unwrap();
    let original_pvc = f.cluster.lock().unwrap().pvc.clone().unwrap();
    let original_pv = f.cluster.lock().unwrap().pv.clone().unwrap();
    // Setup/running observation may GET the known PV. The narrowed contract
    // below applies to the actual delete/retry path, after that setup.
    let requests_before_delete = f._server.received_requests().await.unwrap().len();
    let deleting = f.action(LifecycleAction::Delete).await;
    f.apply_delete_until_controller_progress().await;
    f.reconcile_delete_suspension();
    for _ in 0..2 {
        f.make_original_delete_due().await;
        assert!(matches!(f.manager.observe(&f.current().await).await,
            Err(crate::errors::AppError::Conflict(reason)) if reason == "delete_sandbox_gc_pending"));
        let current = f.current().await;
        assert_eq!(current.active_operation_id, deleting.active_operation_id);
        assert!(!current.writer_admitted);
        assert_eq!(current.authorization_state, deleting.authorization_state);
        assert_eq!(current.cleanup_state, "pending");
        f.slot_retained().await;
        let cluster = f.cluster.lock().unwrap();
        assert!(cluster.delete_requested);
        assert_eq!(cluster.pvc.as_ref(), Some(&original_pvc));
        assert_eq!(cluster.pv.as_ref(), Some(&original_pv));
        assert!(cluster.sandbox.as_ref().unwrap()["metadata"]["deletionTimestamp"].is_string());
    }
    // Independent GC removes Sandbox/Pod/PVC. The retained PV is unqueried and
    // remains present; success describes only the v2 object cascade.
    f.cluster.lock().unwrap().finish_delete(false);
    f.make_original_delete_due().await;
    f.manager.observe(&f.current().await).await.unwrap();
    let deleted = f.current().await;
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    assert_eq!(deleted.billing_state, BillingState::Closed);
    assert_eq!(
        deleted.delete_recovery.as_ref().unwrap().storage,
        crate::models::remote_machine::DeleteStorageDisposition::SandboxCascadeObserved
    );
    assert_eq!(f.cluster.lock().unwrap().pv.as_ref(), Some(&original_pv));
    let requests = f._server.received_requests().await.unwrap();
    assert!(requests.iter().skip(requests_before_delete).all(|request| {
        let path = request.url.path();
        !path.contains("/persistentvolumes")
            && !path.contains("/volumeattachments")
            && path != "/api/v1/pods"
            && (request.method.as_str() != "DELETE" || path.contains("/sandboxes/"))
    }));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_environment_startup_and_readiness_do_not_require_provider_or_runtimeclass() {
    const CHILD: &str = "NYXID_NATIVE_STARTUP_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Child-process env is isolated from concurrent Rust tests. No global
        // test environment mutation and no production configuration change.
        for selection in [
            None,
            Some(""),
            Some("existing-fixture-storage"),
            Some("invalid/class"),
        ] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.arg("--exact").arg("services::remote_machine_manager::native_tests::native_environment_startup_and_readiness_do_not_require_provider_or_runtimeclass").arg("--nocapture").env(CHILD,"1");
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("NYXID_REMOTE_MACHINE_") {
                    command.env_remove(key);
                }
            }
            let token = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(token.path(), "local-native-startup-credential").unwrap();
            command
                .env("NYXID_REMOTE_MACHINE_KUBE_API", "https://127.0.0.1:1")
                .env("NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE", token.path())
                .env(
                    "NYXID_REMOTE_MACHINE_BOOTSTRAP_BASE_URL",
                    "https://nyxid.native.test",
                )
                .env(
                    "NYXID_REMOTE_MACHINE_IMAGE_DIGEST",
                    format!("registry.test/node@sha256:{}", "a".repeat(64)),
                );
            if let Some(selection) = selection {
                command.env("NYXID_REMOTE_MACHINE_STORAGE_CLASS", selection);
            }
            let result = command.output().unwrap();
            assert!(
                result.status.success(),
                "isolated startup failed for {selection:?}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
        }
        return;
    }
    // Closed loopback endpoints keep this child isolated from real services.
    // The production worker tasks are cancelled when this test runtime drops.
    let client =
        mongodb::Client::with_uri_str("mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=100")
            .await
            .unwrap();
    let db = client.database("native_startup_constructor_only");
    assert!(!super::MANAGER_STARTED.load(std::sync::atomic::Ordering::Acquire));
    if std::env::var("NYXID_REMOTE_MACHINE_STORAGE_CLASS").as_deref() == Ok("invalid/class") {
        assert!(Manager::from_environment(db, None, "https://nyxid.native.test").is_err());
        assert!(
            super::platform_readiness("https://nyxid.native.test")
                .blockers
                .contains(&"storage_class_configuration_invalid")
        );
        return;
    }
    let manager = Manager::from_environment(db.clone(), None, "https://nyxid.native.test").unwrap();
    assert_eq!(
        manager.storage_class,
        super::configured_storage_class().unwrap()
    );
    assert!(!super::MANAGER_STARTED.load(std::sync::atomic::Ordering::Acquire));
    super::spawn(crate::test_utils::test_app_state(db.clone()));
    assert!(super::MANAGER_STARTED.load(std::sync::atomic::Ordering::Acquire));
    let readiness = super::platform_readiness("https://nyxid.native.test");
    assert!(readiness.manager_started);
    assert!(
        readiness.configuration_ready,
        "basic configured native startup has no canceled prerequisite"
    );
    assert!(readiness.blockers.is_empty());
    let provisioning =
        super::platform_readiness_for_provisioning("https://nyxid.native.test").await;
    assert!(
        !provisioning.configuration_ready,
        "unverifiable storage is never ready"
    );
    assert!(
        provisioning
            .blockers
            .contains(&if manager.storage_class.is_some() {
                "storage_class_unavailable_or_unverified"
            } else {
                "default_storage_class_unavailable_or_unverified"
            })
    );
    assert_eq!(
        readiness.lifecycle_basis,
        "agent_sandbox_native_observation"
    );
    assert_eq!(
        readiness.physical_acceptance,
        "native_state_only_no_physical_guarantee"
    );
    assert!(
        lifecycle::fixed_profile(format!("registry.test/node@sha256:{}", "a".repeat(64)))
            .runtime_class
            .is_none()
    );
    assert!(
        crate::services::remote_machine_storage_telemetry::KubeletCollector::from_environment()
            .unwrap()
            .is_none()
    );
    let diagnostics = crate::services::remote_machine_controller_health::observe(None).await;
    assert_eq!(
        serde_json::to_value(diagnostics).unwrap()["status"],
        "unconfigured"
    );
}

#[tokio::test]
async fn native_late_quarantine_preserves_committed_owner_pause_and_delete() {
    for action in [LifecycleAction::Pause, LifecycleAction::Delete] {
        let f = fixture(&format!("native_late_quarantine_{}", action.as_str())).await;
        let stale_ready = f.start().await;
        assert_eq!(stale_ready.resource_state, ResourceState::Ready);
        assert!(stale_ready.writer_admitted);
        let accepted = f.action(action).await;
        let expected = if action == LifecycleAction::Delete {
            ResourceState::Deleting
        } else {
            ResourceState::Stopping
        };
        assert_eq!(accepted.runtime_generation, stale_ready.runtime_generation);
        assert_eq!(accepted.credential_epoch, stale_ready.credential_epoch);
        assert_eq!(accepted.writer_fence, stale_ready.writer_fence);
        assert_ne!(
            accepted.active_operation_id,
            stale_ready.active_operation_id
        );
        native::quarantine(&f.db, &stale_ready, "fixture_late_native_uncertainty")
            .await
            .unwrap();
        let current = f.current().await;
        assert_eq!(
            current.resource_state, expected,
            "late Ready observer must use live owner lifecycle state"
        );
        assert_eq!(current.active_operation_id, accepted.active_operation_id);
        assert_eq!(
            current.native_uncertainty.as_deref(),
            Some("fixture_late_native_uncertainty")
        );
        assert_eq!(
            current.stop_reason, accepted.stop_reason,
            "owner stop reason is retained"
        );
        assert!(!current.writer_admitted);
        assert!(!lifecycle::runtime_fence_allows_bootstrap(&current));
        f.slot_retained().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn native_delete_recovers_lost_create_response_only_under_original_allocation_intent() {
    let f = fixture("native_lost_create_owner_delete").await;
    let create_effect = f.effect().await;
    Box::pin(f.manager.apply_effect(&create_effect))
        .await
        .unwrap();
    f.cluster.lock().unwrap().reconcile();
    let allocated = f.cluster.lock().unwrap().sandbox.clone().unwrap();
    let original_uid = allocated["metadata"]["uid"].as_str().unwrap().to_owned();
    // Simulate the lost successful create reply before any durable observation:
    // actual accepted Create operation/outbox and submission marker are retained.
    f.db.collection::<Document>(MACHINES).update_one(doc!{"_id":&f.machine.node_id},doc!{"$set":{
        "sandbox_uid":bson::Bson::Null,"native_observation":bson::Bson::Null,"writer_admitted":false
    }}).await.unwrap();
    let unbound = f.current().await;
    assert!(unbound.sandbox_uid.is_none());
    assert!(unbound.runtime_submission_started_at.is_some());
    assert_eq!(
        unbound.active_operation_id.as_deref(),
        Some(create_effect.operation_id.as_str())
    );
    let deleting = f.action(LifecycleAction::Delete).await;
    let requests_before = f._server.received_requests().await.unwrap().len();
    f.apply_delete_until_controller_progress().await;
    let recovered = f.current().await;
    assert_eq!(
        recovered.sandbox_uid.as_deref(),
        Some(original_uid.as_str())
    );
    assert_eq!(recovered.active_operation_id, deleting.active_operation_id);
    assert_eq!(recovered.runtime_generation, unbound.runtime_generation);
    assert_eq!(recovered.resource_state, ResourceState::Deleting);
    assert!(!recovered.writer_admitted);
    let current_sandbox = f.cluster.lock().unwrap().sandbox.clone().unwrap();
    assert_eq!(current_sandbox["metadata"]["uid"], original_uid);
    assert_eq!(current_sandbox["spec"]["operatingMode"], "Suspended");
    assert_eq!(
        current_sandbox["metadata"]["labels"]["nyxid.dev/operation-id"],
        deleting.active_operation_id.unwrap()
    );
    for request in f
        ._server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .skip(requests_before)
    {
        assert_ne!(
            request.method.as_str(),
            "POST",
            "no replacement allocation is created"
        );
        if request.method.as_str() == "PATCH" {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let patch = body.as_array().expect("minimal JSON Patch suspension");
            assert!(
                patch.contains(&json!({"op":"test","path":"/metadata/uid","value":original_uid})),
                "recovery patches only the original UID"
            );
            assert!(
                patch.contains(&json!({"op":"test","path":"/metadata/resourceVersion","value":allocated["metadata"]["resourceVersion"]})),
                "recovery tests the original resourceVersion"
            );
            assert!(
                patch.contains(
                    &json!({"op":"add","path":"/spec/operatingMode","value":"Suspended"})
                ),
                "lost response cannot resurrect running intent"
            );
            assert!(patch.iter().all(|operation| {
                let path = operation["path"].as_str().unwrap();
                !path.starts_with("/spec/podTemplate")
                    && !path.starts_with("/spec/volumeClaimTemplates")
            }));
        }
    }
    f.slot_retained().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_delete_rejects_incomplete_owned_lists_before_finality() {
    for resource in ["pods", "persistentvolumeclaims"] {
        for defect in ["continue_type", "missing_continue", "malformed_owner"] {
            let f = fixture("native_delete_strict_inventory").await;
            f.start().await;
            f.action(LifecycleAction::Delete).await;
            f.apply_delete_until_controller_progress().await;
            f.reconcile_delete_suspension();
            f.make_original_delete_due().await;
            assert!(f.manager.observe(&f.current().await).await.is_err());
            assert!(f.cluster.lock().unwrap().delete_requested);
            f.cluster.lock().unwrap().finish_delete(true);
            let kind = if resource == "pods" {
                "PodList"
            } else {
                "PersistentVolumeClaimList"
            };
            let inventory_path = f.cleanup_inventory_path(resource);
            let mut page = json!({"kind":kind,"apiVersion":"v1","metadata":{"resourceVersion":"strict-snapshot"},"items":[]});
            match defect {
                "continue_type" => page["metadata"]["continue"] = json!(123),
                "missing_continue" => page["metadata"]["remainingItemCount"] = json!(1),
                _ => {
                    page["items"] = json!([{"kind":kind.trim_end_matches("List"),"apiVersion":"v1",
                    "metadata":{"name":"untrusted","namespace":"nyxid-machine","uid":Uuid::new_v4().to_string(),"resourceVersion":"1","ownerReferences":[{"uid":123}]}}]);
                    if resource == "pods" {
                        page["items"][0]["spec"] = json!({});
                    }
                }
            }
            f.cluster
                .lock()
                .unwrap()
                .resource_list_pages
                .insert(inventory_path.clone(), vec![page]);
            let requests_before_inventory = f._server.received_requests().await.unwrap().len();
            f.make_original_delete_due().await;
            let error = f.manager.observe(&f.current().await).await.unwrap_err();
            let expected = if defect == "malformed_owner" {
                "delete_kubernetes_response_invalid"
            } else {
                "delete_kubernetes_inventory_incomplete"
            };
            assert!(
                matches!(error, crate::errors::AppError::Conflict(ref reason) if reason == expected),
                "{resource}/{defect} must retain its classified inventory error: {error}"
            );
            assert!(
                f._server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .skip(requests_before_inventory)
                    .any(|request| request.method.as_str() == "GET"
                        && request.url.path() == inventory_path.as_str()),
                "{resource}/{defect} reaches its actual post-controller inventory"
            );
            assert_ne!(f.current().await.cleanup_state, "complete");
            f.slot_retained().await;
            f.cluster.lock().unwrap().resource_list_pages.clear();
            f.make_original_delete_due().await;
            f.manager.observe(&f.current().await).await.unwrap();
            assert_eq!(f.current().await.resource_state, ResourceState::Deleted);
            assert_eq!(
                f.current().await.authorization_state,
                AuthorizationState::Revoked
            );
            f.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn native_delete_accepts_complete_multipage_owned_inventories() {
    let f = fixture("native_delete_multipage_inventory").await;
    f.start().await;
    f.action(LifecycleAction::Delete).await;
    f.apply_delete_until_controller_progress().await;
    f.reconcile_delete_suspension();
    f.make_original_delete_due().await;
    assert!(f.manager.observe(&f.current().await).await.is_err());
    f.cluster.lock().unwrap().finish_delete(true);
    let inventories = [
        ("pods", "Pod", f.cleanup_inventory_path("pods")),
        (
            "persistentvolumeclaims",
            "PersistentVolumeClaim",
            f.cleanup_inventory_path("persistentvolumeclaims"),
        ),
    ];
    for (resource, kind, inventory_path) in &inventories {
        let mut unrelated = json!({"kind":kind,"apiVersion":"v1","metadata":{
            "name":"unrelated","namespace":"nyxid-machine","uid":Uuid::new_v4().to_string(),"resourceVersion":"1",
            "ownerReferences":[{"apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox","name":"another","uid":Uuid::new_v4().to_string(),"controller":true}]}});
        if *resource == "pods" {
            unrelated["spec"] = json!({});
        }
        f.cluster.lock().unwrap().resource_list_pages.insert(inventory_path.clone(),vec![
            json!({"kind":format!("{kind}List"),"apiVersion":"v1","metadata":{"resourceVersion":"stable","continue":"second","remainingItemCount":1},"items":[]}),
            json!({"kind":format!("{kind}List"),"apiVersion":"v1","metadata":{"resourceVersion":"stable","remainingItemCount":0},"items":[unrelated]}),
        ]);
    }
    let requests_before_inventory = f._server.received_requests().await.unwrap().len();
    f.make_original_delete_due().await;
    f.manager.observe(&f.current().await).await.unwrap();
    let inventory_requests = f._server.received_requests().await.unwrap();
    for (resource, _, inventory_path) in &inventories {
        for continued in [false, true] {
            assert!(
                inventory_requests
                    .iter()
                    .skip(requests_before_inventory)
                    .any(|request| {
                        let has_continuation = request
                            .url
                            .query_pairs()
                            .any(|(key, value)| key == "continue" && value == "second");
                        request.method.as_str() == "GET"
                            && request.url.path() == inventory_path.as_str()
                            && has_continuation == continued
                    }),
                "the complete {resource} inventory reads its continued={continued} page"
            );
        }
    }
    assert_eq!(f.current().await.resource_state, ResourceState::Deleted);
    assert_eq!(
        f.current().await.authorization_state,
        AuthorizationState::Revoked
    );
    f.db.drop().await.unwrap();
}
