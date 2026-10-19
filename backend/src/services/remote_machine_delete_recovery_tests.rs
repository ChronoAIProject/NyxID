use super::*;
use crate::models::remote_machine::{
    DeleteStorageDisposition, MachineDeletionBasis, MachineDeletionSource, ResourceState,
};
use chrono::Utc;

fn fixture() -> (ManagedMachine, DeleteRecoveryProgress) {
    let mut machine: ManagedMachine = bson::from_document(bson::doc! {
        "_id":"machine","owner_id":"owner","policy_version":crate::models::remote_machine::POLICY_VERSION,
        "data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,
        "profile":bson::to_bson(&crate::models::remote_machine::MachineProfileSpec::fixed("registry.test/node@sha256:abc")).unwrap(),
        "sandbox_namespace":"nyxid-machine","sandbox_name":"machine-fixture","resource_state":"deleting",
        "billing_state":"free","authorization_state":"revoked","revision":1_i64,"runtime_generation":0_i64,
        "credential_epoch":0_i64,"writer_fence":"original-full-fence","free_slot":true,"cleanup_state":"pending",
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).unwrap();
    machine.resource_state = ResourceState::Deleting;
    machine.active_operation_id = Some("original-delete".into());
    machine.deletion_basis = Some(MachineDeletionBasis {
        owner_id: machine.owner_id.clone(),
        source: MachineDeletionSource::MachineOwner,
        policy_version: machine.data_policy_version.clone(),
        accepted_at: Utc::now(),
    });
    machine.native_uncertainty = Some("predecessor_node_unhealthy_or_stale".into());
    let progress = DeleteRecoveryProgress {
        evidence_version: 1,
        evidence_id: Uuid::new_v4().to_string(),
        operation_id: "original-delete".into(),
        owner_id: machine.owner_id.clone(),
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        writer_fence: machine.writer_fence.clone(),
        deletion_basis_digest: super::basis_digest(&machine).unwrap(),
        evaluated_revision: machine.revision,
        original_uncertainty: machine.native_uncertainty.clone(),
        sandbox_uid: None,
        pod_uid: None,
        node_uid: None,
        pvc_uid: None,
        provisioning_request_id: None,
        provisioning_terminal: None,
        volume: None,
        stop: None,
        storage: DeleteStorageDisposition::NeverRequested,
        creation_terminal: false,
        resources_complete: false,
        blocker: None,
        observed_at: Utc::now(),
    };
    (machine, progress)
}

#[test]
fn original_delete_evidence_binds_current_authorization_and_full_tuple() {
    let (machine, progress) = fixture();
    assert!(binding_matches(&machine, &progress));
    for changed in 0..8 {
        let mut stale = machine.clone();
        match changed {
            0 => stale.writer_fence.push('x'),
            1 => stale.runtime_generation += 1,
            2 => stale.credential_epoch += 1,
            3 => stale.active_operation_id = Some("another-delete".into()),
            4 => stale.owner_id = "another-owner".into(),
            5 => stale.native_uncertainty = Some("known_force_deletion".into()),
            6 => stale.deletion_basis = None,
            _ => stale.resource_state = ResourceState::Ready,
        }
        assert!(
            !binding_matches(&stale, &progress),
            "stale binding case {changed}"
        );
    }
}

#[test]
fn delete_binding_debug_redacts_fence_and_original_identities() {
    let (_, mut progress) = fixture();
    progress.writer_fence = "sensitive-fence".into();
    progress.sandbox_uid = Some("private-sandbox".into());
    let debug = format!("{progress:?}");
    assert!(!debug.contains("sensitive-fence"));
    assert!(!debug.contains("private-sandbox"));
    let bson = bson::to_document(&progress).unwrap();
    assert!(bson.get_datetime("observed_at").is_ok());
    assert_eq!(bson.get_str("writer_fence").unwrap(), "sensitive-fence");
}

#[test]
fn control_plane_delete_stop_never_grants_ordinary_runtime_authority() {
    use crate::models::remote_machine::{DeleteStopEvidence, DeleteStopKind};
    let (mut machine, mut progress) = fixture();
    machine.runtime_generation = 2;
    machine.credential_epoch = 2;
    machine.sandbox_uid = Some("sandbox".into());
    progress.runtime_generation = 2;
    progress.credential_epoch = 2;
    progress.sandbox_uid = machine.sandbox_uid.clone();
    progress.stop = Some(DeleteStopEvidence {
        kind: DeleteStopKind::ControlPlaneSuspended,
        source_operation_id: Some(progress.operation_id.clone()),
        evidence_ref: Uuid::new_v4().to_string(),
        source: "sandbox-kubernetes-control-plane".into(),
        observed_at: super::now(),
        sandbox_generation: 3,
        sandbox_resource_version: "34".into(),
    });
    assert!(stop_is_valid(&machine, &progress));
    assert!(progress.pod_uid.is_none());
    assert!(progress.node_uid.is_none());
    machine.delete_recovery = Some(progress.clone());
    assert!(!crate::services::remote_machine_native::predecessor_stopped(&machine));
    for reason in [
        "known_force_deletion",
        "unexpected_pod_replacement",
        "native_observation_expired",
        "unknown",
    ] {
        machine.native_uncertainty = Some(reason.into());
        progress.original_uncertainty = machine.native_uncertainty.clone();
        assert!(!stop_is_valid(&machine, &progress), "reason {reason}");
    }
}

#[test]
fn control_plane_stop_rejects_wrong_source_operation_version_and_controller_coordinates() {
    use crate::models::remote_machine::{DeleteStopEvidence, DeleteStopKind};
    let (mut machine, mut progress) = fixture();
    machine.sandbox_uid = Some("original-sandbox".into());
    progress.sandbox_uid = machine.sandbox_uid.clone();
    progress.stop = Some(DeleteStopEvidence {
        kind: DeleteStopKind::ControlPlaneSuspended,
        source_operation_id: Some(progress.operation_id.clone()),
        evidence_ref: Uuid::new_v4().to_string(),
        source: "sandbox-kubernetes-control-plane".into(),
        observed_at: super::now(),
        sandbox_generation: 2,
        sandbox_resource_version: "21".into(),
    });
    assert!(stop_is_valid(&machine, &progress));
    for defect in [
        "legacy_version",
        "future_version",
        "source",
        "source_operation",
        "reference",
        "generation",
        "resource_version",
        "future_time",
        "future_revision",
    ] {
        let mut invalid = progress.clone();
        match defect {
            "legacy_version" => invalid.evidence_version = 0,
            "future_version" => invalid.evidence_version = 2,
            "source" => invalid.stop.as_mut().unwrap().source = "runtime-agent".into(),
            "source_operation" => {
                invalid.stop.as_mut().unwrap().source_operation_id = Some("other-delete".into())
            }
            "reference" => invalid.stop.as_mut().unwrap().evidence_ref = "not-a-uuid".into(),
            "generation" => invalid.stop.as_mut().unwrap().sandbox_generation = 0,
            "resource_version" => invalid
                .stop
                .as_mut()
                .unwrap()
                .sandbox_resource_version
                .clear(),
            "future_time" => {
                invalid.stop.as_mut().unwrap().observed_at =
                    super::now() + chrono::Duration::minutes(1)
            }
            _ => invalid.evaluated_revision = machine.revision + 1,
        }
        assert!(!stop_is_valid(&machine, &invalid), "{defect}");
    }
}

#[test]
fn legacy_external_stop_and_unversioned_terminal_milestones_are_not_current_authority() {
    use crate::models::remote_machine::{DeleteStopEvidence, DeleteStopKind};
    let (mut machine, mut progress) = fixture();
    machine.runtime_generation = 1;
    machine.sandbox_uid = Some("sandbox".into());
    machine.pod_uid = Some("pod".into());
    machine.native_node_uid = Some("node".into());
    progress.runtime_generation = 1;
    progress.sandbox_uid = machine.sandbox_uid.clone();
    progress.pod_uid = machine.pod_uid.clone();
    progress.node_uid = machine.native_node_uid.clone();
    progress.stop = Some(DeleteStopEvidence {
        kind: DeleteStopKind::TrustedRuntime,
        source_operation_id: None,
        evidence_ref: "legacy-host-fact".into(),
        source: "trusted-runtime".into(),
        observed_at: super::now(),
        sandbox_generation: 2,
        sandbox_resource_version: "21".into(),
    });
    progress.creation_terminal = true;
    progress.resources_complete = true;
    progress.storage = DeleteStorageDisposition::NeverProvisioned;
    assert!(!stop_is_valid(&machine, &progress));
    assert!(!finality_ready(&machine, &progress));
    progress.evidence_version = 0;
    progress.stop.as_mut().unwrap().kind = DeleteStopKind::ControlPlaneSuspended;
    progress.stop.as_mut().unwrap().source = "sandbox-kubernetes-control-plane".into();
    progress.stop.as_mut().unwrap().evidence_ref = Uuid::new_v4().to_string();
    assert!(!stop_is_valid(&machine, &progress));
    assert!(!finality_ready(&machine, &progress));
    let mut encoded = bson::to_document(&progress).unwrap();
    encoded.remove("evidence_version");
    let legacy: DeleteRecoveryProgress = bson::from_document(encoded).unwrap();
    assert_eq!(legacy.evidence_version, 0);
}

#[test]
fn control_plane_storage_release_requires_complete_inventory_and_no_unresolved_volume_history() {
    use crate::models::remote_machine::{DeleteProvisioningEvidence, DeleteVolumeIdentity};
    let (mut machine, mut progress) = fixture();
    machine.pvc_uid = Some("claim".into());
    progress.pvc_uid = machine.pvc_uid.clone();
    progress.storage = DeleteStorageDisposition::ControlPlaneReleased;
    assert!(!super::storage_is_final(&machine, &progress));
    progress.resources_complete = true;
    assert!(super::storage_is_final(&machine, &progress));
    progress.storage = DeleteStorageDisposition::NeverProvisioned;
    progress.provisioning_terminal = Some(DeleteProvisioningEvidence {
        source: "legacy-csi".into(),
        evidence_ref: "legacy-request-closed".into(),
        never_provisioned: true,
        disk_handles: vec![],
        observed_at: super::now(),
    });
    assert!(!super::storage_is_final(&machine, &progress));
    progress.storage = DeleteStorageDisposition::ControlPlaneReleased;
    progress.volume = Some(DeleteVolumeIdentity {
        pv_name: "original-pv".into(),
        pv_uid: "pv-uid".into(),
        reclaim_policy: "Retain".into(),
        csi_driver: None,
        csi_handle: None,
        immutable_disk_id: None,
    });
    assert!(!super::storage_is_final(&machine, &progress));
    progress.volume = None;
    machine.native_pv_uid = Some("historic-pv".into());
    assert!(!super::storage_is_final(&machine, &progress));
    machine.native_pv_uid = None;
    progress.evidence_version = 0;
    assert!(!super::storage_is_final(&machine, &progress));
}

#[test]
fn legacy_external_disk_hints_do_not_block_complete_kubernetes_storage_release() {
    let (mut machine, mut progress) = fixture();
    machine.native_pv_name = Some("original-pv".into());
    machine.native_pv_uid = Some("original-pv-uid".into());
    machine.native_pv_reclaim_policy = Some("Delete".into());
    progress.volume = Some(crate::models::remote_machine::DeleteVolumeIdentity {
        pv_name: "original-pv".into(),
        pv_uid: "original-pv-uid".into(),
        reclaim_policy: "Delete".into(),
        csi_driver: Some(CSI_DRIVER.into()),
        csi_handle: Some(DISK_HANDLE.into()),
        immutable_disk_id: Some("legacy-cloud-id".into()),
    });
    progress.storage = DeleteStorageDisposition::ControlPlaneReleased;
    progress.resources_complete = true;
    for never_provisioned in [false, true] {
        progress.provisioning_terminal =
            Some(crate::models::remote_machine::DeleteProvisioningEvidence {
                source: "legacy-csi-fixture".into(),
                evidence_ref: "legacy-terminal".into(),
                never_provisioned,
                disk_handles: vec![DISK_HANDLE.into(), EXTRA_DISK_HANDLE.into()],
                observed_at: super::now(),
            });
        assert!(
            super::storage_is_final(&machine, &progress),
            "external disk hints do not change complete Kubernetes release"
        );
    }
    progress.volume.as_mut().unwrap().reclaim_policy = "Retain".into();
    assert!(
        !super::storage_is_final(&machine, &progress),
        "actual original PV reclaim policy still binds release"
    );
}

// These tests run the real owner-acceptance transaction, recovery service,
// SandboxApi and final transaction against a disposable Mongo replica set.
// Only the Kubernetes server is substituted; every step uses production advance.
use crate::{
    models::{
        machine_free_slot::{FreeSlotState, MachineFreeSlot},
        remote_machine::{COLLECTION_NAME as MACHINE_COLLECTION, NativeRuntimeObservation},
        remote_machine_operation::{LifecycleAction, OperationState},
        remote_machine_outbox::{COLLECTION_NAME as OUTBOX_COLLECTION, MachineEffectOutbox},
        user::UserType,
    },
    test_utils::{connect_transaction_test_database, test_user},
};
use nyxid_machine_updater::sandbox::SandboxApi;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use wiremock::{Mock, MockServer, ResponseTemplate};

const CSI_DRIVER: &str = "pd.csi.storage.gke.io";
const DISK_HANDLE: &str = "projects/fixture-project/zones/us-central1-a/disks/original-disk";
const EXTRA_DISK_HANDLE: &str =
    "projects/fixture-project/zones/us-central1-a/disks/late-related-disk";

#[derive(Default)]
struct RecoveryCluster {
    sandbox: Option<Value>,
    pvc: Option<Value>,
    pv: Option<Value>,
    pods: Vec<Value>,
    attachments: Vec<Value>,
    pod_list_error: Option<u16>,
    pv_read_error: Option<u16>,
    owned_pvc_pages: Option<Vec<Value>>,
    requests: Vec<(String, String)>,
    deletes: Vec<(String, Value)>,
    patches: Vec<Value>,
}
impl RecoveryCluster {
    fn list(kind: &str, version: &str, values: Vec<Value>) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({"kind":format!("{kind}List"),"apiVersion":version,"metadata":{"resourceVersion":"500"},"items":values}))
    }
    fn respond(&mut self, request: &wiremock::Request) -> ResponseTemplate {
        let path = request.url.path();
        self.requests
            .push((request.method.to_string(), path.to_string()));
        let sandbox_item = path.contains("/sandboxes/");
        if request.method.as_str() == "DELETE" {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let object = if sandbox_item {
                self.sandbox.as_ref()
            } else if path.contains("/pods/") {
                self.pods
                    .iter()
                    .find(|pod| pod["metadata"]["name"].as_str() == path.rsplit('/').next())
            } else if path.contains("/persistentvolumeclaims/") {
                self.pvc.as_ref()
            } else {
                None
            };
            if object.is_none_or(|o| {
                body["preconditions"]["uid"] != o["metadata"]["uid"]
                    || body["preconditions"]["resourceVersion"] != o["metadata"]["resourceVersion"]
            }) {
                return ResponseTemplate::new(409);
            }
            self.deletes.push((path.into(), body));
            // Accepted mutation intentionally leaves the object present. Tests
            // advance GC explicitly, independently of API acceptance.
            return ResponseTemplate::new(202).set_body_json(json!({"status":"Success"}));
        }
        if request.method.as_str() == "PATCH" {
            let patch: Value = serde_json::from_slice(&request.body).unwrap();
            let Some(object) = self.sandbox.as_mut() else {
                return ResponseTemplate::new(404);
            };
            let previous_spec = object["spec"].clone();
            for op in patch
                .as_array()
                .expect("recovery uses a minimal JSON patch")
            {
                let path = op["path"].as_str().unwrap();
                match op["op"].as_str().unwrap() {
                    "test" if object.pointer(path) != Some(&op["value"]) => {
                        return ResponseTemplate::new(409);
                    }
                    "test" => (),
                    "add" => {
                        *object.pointer_mut(path).expect("existing minimal field") =
                            op["value"].clone()
                    }
                    "remove" => {
                        assert_eq!(path, "/spec/shutdownTime");
                        object["spec"]
                            .as_object_mut()
                            .unwrap()
                            .remove("shutdownTime");
                    }
                    other => panic!("unexpected patch operation {other}"),
                }
            }
            self.patches.push(patch);
            if object["spec"] != previous_spec {
                let generation = object["metadata"]["generation"].as_i64().unwrap();
                object["metadata"]["generation"] = json!(generation + 1);
            }
            object["metadata"]["resourceVersion"] = json!("22");
            return ResponseTemplate::new(200).set_body_json(object.clone());
        }
        if request.method.as_str() != "GET" {
            return ResponseTemplate::new(405);
        }
        if (path == "/api/v1/persistentvolumes" || path.contains("/persistentvolumes/"))
            && let Some(status) = self.pv_read_error
        {
            return ResponseTemplate::new(status);
        }
        if path == "/api/v1/pods" {
            return self
                .pod_list_error
                .map(ResponseTemplate::new)
                .unwrap_or_else(|| Self::list("Pod", "v1", self.pods.clone()));
        }
        if path.ends_with("/pods") {
            return Self::list("Pod", "v1", self.pods.clone());
        }
        if path.ends_with("/persistentvolumeclaims") {
            if let Some(pages) = &self.owned_pvc_pages {
                let page = usize::from(request.url.query_pairs().any(|(key, _)| key == "continue"));
                return ResponseTemplate::new(200).set_body_json(pages[page].clone());
            }
            return Self::list(
                "PersistentVolumeClaim",
                "v1",
                self.pvc.clone().into_iter().collect(),
            );
        }
        if path == "/api/v1/persistentvolumes" {
            return Self::list(
                "PersistentVolume",
                "v1",
                self.pv.clone().into_iter().collect(),
            );
        }
        if path.ends_with("/volumeattachments") {
            return Self::list(
                "VolumeAttachment",
                "storage.k8s.io/v1",
                self.attachments.clone(),
            );
        }
        let object = if sandbox_item {
            self.sandbox.clone()
        } else if path.ends_with("/persistentvolumeclaims/workspace") {
            self.pvc.clone()
        } else if path.ends_with("/persistentvolumes/original-pv") {
            self.pv.clone()
        } else if path.ends_with("/pods/original-pod") {
            self.pods
                .iter()
                .find(|p| p["metadata"]["name"] == "original-pod")
                .cloned()
        } else {
            None
        };
        object
            .map(|o| ResponseTemplate::new(200).set_body_json(o))
            .unwrap_or_else(|| ResponseTemplate::new(404))
    }
}

struct RecoveryFixture {
    db: Database,
    server: MockServer,
    cluster: Arc<Mutex<RecoveryCluster>>,
    manager: Manager,
    machine: ManagedMachine,
}
impl RecoveryFixture {
    async fn current(&self) -> ManagedMachine {
        lifecycle::get_machine(&self.db, &self.machine.owner_id, &self.machine.node_id)
            .await
            .unwrap()
            .unwrap()
    }
    async fn due(&self) {
        self.db
            .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&self.machine.active_operation_id},
                doc! {"$set":{"next_attempt_at":bson::Bson::Null}},
            )
            .await
            .unwrap();
    }
    async fn step(&self) -> AppResult<()> {
        self.due().await;
        advance(&self.manager, &self.current().await).await
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
        assert_eq!(self.current().await.resource_state, ResourceState::Deleting);
    }
    async fn finish_assertions(&self) {
        let current = self.current().await;
        assert_eq!(current.resource_state, ResourceState::Deleted);
        assert!(!current.writer_admitted);
        assert_eq!(current.native_uncertainty, self.machine.native_uncertainty);
        assert_eq!(
            current.active_operation_id,
            self.machine.active_operation_id
        );
        assert_eq!(current.runtime_generation, self.machine.runtime_generation);
        assert_eq!(current.credential_epoch, self.machine.credential_epoch);
        assert_eq!(current.writer_fence, self.machine.writer_fence);
        assert_eq!(
            operations::get_operation(
                &self.db,
                &current.owner_id,
                current.active_operation_id.as_deref().unwrap()
            )
            .await
            .unwrap()
            .unwrap()
            .state,
            OperationState::Succeeded
        );
        assert_eq!(
            self.db
                .collection::<MachineFreeSlot>(lifecycle::FREE_SLOTS)
                .find_one(doc! {"_id":&current.owner_id})
                .await
                .unwrap()
                .unwrap()
                .state,
            FreeSlotState::Released
        );
    }
    fn set_pv(&self, reclaim: &str) {
        let claim_uid = self
            .cluster
            .lock()
            .unwrap()
            .pvc
            .as_ref()
            .map(|p| p["metadata"]["uid"].clone())
            .unwrap_or_else(|| json!(self.machine.pvc_uid));
        self.cluster.lock().unwrap().pv = Some(
            json!({"apiVersion":"v1","kind":"PersistentVolume","metadata":{"name":"original-pv","uid":Uuid::new_v4().to_string(),"resourceVersion":"31"},"spec":{"claimRef":{"namespace":self.machine.sandbox_namespace,"name":"workspace","uid":claim_uid},"persistentVolumeReclaimPolicy":reclaim,"csi":{"driver":CSI_DRIVER,"volumeHandle":DISK_HANDLE}},"status":{"phase":"Bound"}}),
        );
    }
    fn pod(&self) -> Value {
        let sandbox = self.cluster.lock().unwrap().sandbox.clone().unwrap();
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"original-pod","namespace":self.machine.sandbox_namespace,"uid":self.machine.pod_uid,"resourceVersion":"9","labels":sandbox["spec"]["podTemplate"]["metadata"]["labels"],"ownerReferences":[{"apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox","name":self.machine.sandbox_name,"uid":self.machine.sandbox_uid,"controller":true}]},"spec":{"nodeName":"original-node","containers":[{"name":"node"}],"volumes":[{"name":"workspace","persistentVolumeClaim":{"claimName":"workspace"}}]},"status":{"phase":"Running"}})
    }
}

#[derive(Clone)]
struct DeleteRequestProof {
    path: String,
    machine: ManagedMachine,
    prior_requests: Vec<(String, String)>,
}

#[derive(Clone)]
struct DeleteProofState {
    db: Database,
    machine_id: String,
    upstream: String,
    client: reqwest::Client,
    cluster: Arc<Mutex<RecoveryCluster>>,
    proofs: Arc<Mutex<Vec<DeleteRequestProof>>>,
}

struct DeleteProofProxy {
    task: tokio::task::JoinHandle<()>,
    proofs: Arc<Mutex<Vec<DeleteRequestProof>>>,
}

impl Drop for DeleteProofProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn proxy_with_pre_delete_database_proof(f: &mut RecoveryFixture) -> DeleteProofProxy {
    use axum::{body::Body, extract::State, http::Request, response::Response};

    async fn forward(State(state): State<DeleteProofState>, request: Request<Body>) -> Response {
        let method = request.method().clone();
        let path = request.uri().path().to_string();
        let target = format!("{}{}", state.upstream, request.uri());
        let headers = request.headers().clone();
        let body = axum::body::to_bytes(request.into_body(), 1024 * 1024)
            .await
            .unwrap();
        if method == axum::http::Method::DELETE {
            // This async database read completes before the request reaches
            // the Kubernetes substitute, so acceptance cannot precede proof.
            let machine = state
                .db
                .collection::<ManagedMachine>(MACHINE_COLLECTION)
                .find_one(doc! {"_id": &state.machine_id})
                .await
                .unwrap()
                .unwrap();
            let prior_requests = state.cluster.lock().unwrap().requests.clone();
            state.proofs.lock().unwrap().push(DeleteRequestProof {
                path,
                machine,
                prior_requests,
            });
        }
        let upstream = state
            .client
            .request(method, target)
            .headers(headers)
            .body(body)
            .send()
            .await
            .unwrap();
        let status = upstream.status();
        let headers = upstream.headers().clone();
        let bytes = upstream.bytes().await.unwrap();
        let mut response = Response::builder()
            .status(status)
            .body(Body::from(bytes))
            .unwrap();
        *response.headers_mut() = headers;
        response
    }

    let proofs = Arc::new(Mutex::new(Vec::new()));
    let state = DeleteProofState {
        db: f.db.clone(),
        machine_id: f.machine.node_id.clone(),
        upstream: f.server.uri(),
        client: reqwest::Client::new(),
        cluster: f.cluster.clone(),
        proofs: proofs.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = axum::Router::new().fallback(forward).with_state(state);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    f.manager.api = SandboxApi::new(format!("http://{address}").parse().unwrap(), None).unwrap();
    DeleteProofProxy { task, proofs }
}

async fn collection_documents(db: &Database, name: &str) -> Vec<Document> {
    let mut cursor = db
        .collection::<Document>(name)
        .find(doc! {})
        .sort(doc! {"_id": 1})
        .await
        .unwrap();
    let mut documents = Vec::new();
    while cursor.advance().await.unwrap() {
        documents.push(cursor.deserialize_current().unwrap());
    }
    documents
}

struct PendingDeleteInvariants {
    machine: ManagedMachine,
    operation: Document,
    slot: Document,
    accounting: Vec<(&'static str, Vec<Document>)>,
    operation_count: u64,
    outbox_count: u64,
}

impl PendingDeleteInvariants {
    async fn capture(f: &RecoveryFixture) -> Self {
        let machine = f.current().await;
        let mut accounting = Vec::new();
        for collection in [
            crate::models::machine_runtime_billing::RUN_COLLECTION,
            crate::models::machine_runtime_billing::INTERVAL_COLLECTION,
            crate::models::machine_runtime_billing::HEAD_COLLECTION,
            crate::models::billing_wallet::COLLECTION_NAME,
            crate::models::billing_ledger::COLLECTION_NAME,
            crate::models::usage_meter::COLLECTION_NAME,
        ] {
            accounting.push((collection, collection_documents(&f.db, collection).await));
        }
        Self {
            operation: f
                .db
                .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .find_one(doc! {"_id": &machine.active_operation_id})
                .await
                .unwrap()
                .unwrap(),
            slot: f
                .db
                .collection::<Document>(lifecycle::FREE_SLOTS)
                .find_one(doc! {"_id": &machine.owner_id})
                .await
                .unwrap()
                .unwrap(),
            operation_count: f
                .db
                .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            outbox_count: f
                .db
                .collection::<Document>(OUTBOX_COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            accounting,
            machine,
        }
    }

    async fn assert_retained(&self, f: &RecoveryFixture) {
        let current = f.current().await;
        assert_eq!(current.resource_state, ResourceState::Deleting);
        assert!(!current.writer_admitted);
        assert_eq!(current.owner_id, self.machine.owner_id);
        assert_eq!(
            current.active_operation_id,
            self.machine.active_operation_id
        );
        assert_eq!(current.runtime_generation, self.machine.runtime_generation);
        assert_eq!(current.credential_epoch, self.machine.credential_epoch);
        assert!(current.writer_fence == self.machine.writer_fence);
        assert_eq!(basis_digest(&current), basis_digest(&self.machine));
        assert_eq!(
            current.authorization_state,
            self.machine.authorization_state
        );
        assert_eq!(current.billing_state, self.machine.billing_state);
        assert_eq!(current.runtime_run_id, self.machine.runtime_run_id);
        assert_eq!(current.native_uncertainty, self.machine.native_uncertainty);
        assert_eq!(current.native_observation, self.machine.native_observation);
        assert_eq!(current.sandbox_uid, self.machine.sandbox_uid);
        assert_eq!(current.pod_uid, self.machine.pod_uid);
        assert_eq!(current.native_pod_name, self.machine.native_pod_name);
        assert_eq!(current.native_node_uid, self.machine.native_node_uid);
        assert_eq!(current.native_node_name, self.machine.native_node_name);
        assert_eq!(current.pvc_name, self.machine.pvc_name);
        assert_eq!(current.pvc_uid, self.machine.pvc_uid);
        assert_eq!(current.native_pv_name, self.machine.native_pv_name);
        assert_eq!(current.native_pv_uid, self.machine.native_pv_uid);
        assert_eq!(
            current.native_pv_reclaim_policy,
            self.machine.native_pv_reclaim_policy
        );
        let operation = operations::get_operation(
            &f.db,
            &current.owner_id,
            current.active_operation_id.as_deref().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(
            operation.state,
            OperationState::Accepted | OperationState::Applying | OperationState::Reconciling
        ));
        assert!(operation.terminal_at.is_none());
        let encoded = bson::to_document(&operation).unwrap();
        for field in [
            "_id",
            "owner_id",
            "machine_id",
            "action",
            "idempotency_key_hash",
            "request_digest",
            "expected_revision",
            "desired_runtime_generation",
            "effect_id",
            "created_at",
            "deadline_at",
        ] {
            assert_eq!(encoded.get(field), self.operation.get(field), "{field}");
        }
        assert_eq!(
            f.db.collection::<Document>(lifecycle::FREE_SLOTS)
                .find_one(doc! {"_id": &current.owner_id})
                .await
                .unwrap()
                .unwrap(),
            self.slot,
            "pending storage must retain the same slot unchanged"
        );
        for (collection, before) in &self.accounting {
            assert_eq!(
                &collection_documents(&f.db, collection).await,
                before,
                "{collection}"
            );
        }
        assert_eq!(
            f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            self.operation_count
        );
        assert_eq!(
            f.db.collection::<Document>(OUTBOX_COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            self.outbox_count
        );
        f.slot_retained().await;
    }
}

async fn recovery_fixture(label: &str, with_claim: bool) -> RecoveryFixture {
    let db = connect_transaction_test_database(label).await;
    lifecycle::ensure_indexes(&db).await.unwrap();
    crate::services::remote_machine_events::ensure_indexes(&db)
        .await
        .unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let mut input = lifecycle::CreateMachineInput::fixed(
        &owner,
        true,
        &format!("registry.test/node@sha256:{}", "a".repeat(64)),
    );
    input.idempotency_key = Uuid::new_v4().to_string();
    let created = Box::pin(lifecycle::create_machine(&db, input))
        .await
        .unwrap();
    let sandbox_uid = Uuid::new_v4().to_string();
    let pod_uid = Uuid::new_v4().to_string();
    let node_uid = Uuid::new_v4().to_string();
    let claim_uid = Uuid::new_v4().to_string();
    let mut set = doc! {"resource_state":"reconciling","runtime_generation":1_i64,"credential_epoch":1_i64,"sandbox_uid":&sandbox_uid,"pod_uid":&pod_uid,"native_pod_name":"original-pod","native_node_name":"original-node","native_node_uid":&node_uid,"native_uncertainty":"predecessor_node_unhealthy_or_stale","writer_admitted":false,"runtime_submission_started_at":bson::DateTime::now()};
    if with_claim {
        set.insert("pvc_name", "workspace");
        set.insert("pvc_uid", &claim_uid);
    }
    db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id":&created.machine.node_id},
            doc! {"$set":set,"$inc":{"revision":1_i64}},
        )
        .await
        .unwrap();
    db.collection::<Document>(OUTBOX_COLLECTION)
        .update_many(
            doc! {"operation_id":&created.operation.id},
            vec![doc! {"$set":{"state":"applied","attempts":1_i64,
                "submission":{"request_id":"$_id", "runtime_generation":1_i64,"credential_epoch":1_i64,"writer_fence":&created.machine.writer_fence,
                    "started_at":bson::DateTime::now(),"completed_at":bson::DateTime::now(),"cancelled_before_dispatch":false,
                    "result_sandbox_uid":&sandbox_uid,"result_resource_version":"21","dispatches":[]}}}],
        )
        .await
        .unwrap();
    let current = lifecycle::get_machine(&db, &owner, &created.machine.node_id)
        .await
        .unwrap()
        .unwrap();
    let machine=Box::pin(lifecycle::request_action(&db,lifecycle::LifecycleRequest {
        owner_id:owner,machine_id:current.node_id.clone(),action:LifecycleAction::Delete,expected_revision:Some(current.revision),idempotency_key:Uuid::new_v4().to_string(),
        request_body:json!({"action":"delete","revision":current.revision,"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true}),now:None,
    })).await.unwrap().machine;
    let intent = crate::services::remote_machine_adapter::intent(
        &machine,
        machine.active_operation_id.as_deref().unwrap(),
        Some("existing-storage"),
        false,
        "https://fixture.test",
    );
    let mut sandbox = intent.apply_object();
    sandbox["metadata"]["uid"] = json!(sandbox_uid);
    sandbox["metadata"]["resourceVersion"] = json!("21");
    sandbox["metadata"]["generation"] = json!(2);
    sandbox["status"] = json!({"conditions":[{"type":"Suspended","status":"True","observedGeneration":2,"reason":"PodTerminated","lastTransitionTime":Utc::now().to_rfc3339()}]});
    let pvc=with_claim.then(||json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"namespace":machine.sandbox_namespace,"name":"workspace","uid":claim_uid,"resourceVersion":"11","labels":sandbox["spec"]["volumeClaimTemplates"][0]["metadata"]["labels"],"ownerReferences":[{"apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox","name":machine.sandbox_name,"uid":sandbox_uid,"controller":true}]},"spec":{"storageClassName":"existing-storage"},"status":{"phase":"Pending"}}));
    let cluster = Arc::new(Mutex::new(RecoveryCluster {
        sandbox: Some(sandbox),
        pvc,
        ..Default::default()
    }));
    let server = MockServer::start().await;
    let state = cluster.clone();
    Mock::given(|_: &wiremock::Request| true)
        .respond_with(move |r: &wiremock::Request| state.lock().unwrap().respond(r))
        .mount(&server)
        .await;
    let manager = Manager {
        db: db.clone(),
        billing: None,
        api: SandboxApi::new(server.uri().parse().unwrap(), None).unwrap(),
        storage_class: None,
        namespace: "nyxid-machine".into(),
        bootstrap_base_url: "https://fixture.test".into(),
    };
    RecoveryFixture {
        db,
        server,
        cluster,
        manager,
        machine,
    }
}

#[tokio::test]
async fn pending_claim_control_plane_release_survives_restart_and_preserves_quarantine() {
    use crate::models::remote_machine::DeleteStopKind;
    let f = recovery_fixture("delete_pending_control_plane", true).await;
    assert!(f.step().await.is_err());
    f.slot_retained().await;
    let saved = f.current().await.delete_recovery.unwrap();
    assert_eq!(saved.evidence_version, 1);
    assert_ne!(saved.storage, DeleteStorageDisposition::NeverProvisioned);
    let stop = saved.stop.as_ref().unwrap();
    assert_eq!(stop.kind, DeleteStopKind::ControlPlaneSuspended);
    assert_eq!(stop.source, "sandbox-kubernetes-control-plane");
    assert!(Uuid::parse_str(&stop.evidence_ref).is_ok());
    assert_eq!(stop.sandbox_generation, 2);
    assert_eq!(stop.sandbox_resource_version, "21");
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    assert!(saved.volume.is_none());
    assert!(saved.provisioning_terminal.is_none());
    assert!(f.current().await.native_pv_uid.is_none());
    {
        let mut c = f.cluster.lock().unwrap();
        c.sandbox = None;
        c.pvc = None;
    }
    // A fresh manager resumes only the persisted current control-plane stop.
    f.due().await;
    let restarted = f.manager.clone();
    advance(&restarted, &f.current().await).await.unwrap();
    f.finish_assertions().await;
    let final_progress = f.current().await.delete_recovery.unwrap();
    assert_eq!(
        final_progress.storage,
        DeleteStorageDisposition::ControlPlaneReleased
    );
    assert_eq!(final_progress.evidence_id, saved.evidence_id);
    assert_eq!(final_progress.stop, saved.stop);
    assert!(final_progress.provisioning_terminal.is_none());
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| { !r.url.path().contains("/nodes/") && !r.url.path().contains("/leases/") })
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn original_named_claim_uid_can_be_captured_once_before_safe_cleanup() {
    let f = recovery_fixture("delete_original_claim_uid_capture", true).await;
    let original_uid = f.current().await.pvc_uid.unwrap();
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id":&f.machine.node_id},
            doc! {"$set":{"pvc_uid":bson::Bson::Null}},
        )
        .await
        .unwrap();
    assert!(f.step().await.is_err());
    let captured = f.current().await;
    assert_eq!(captured.pvc_uid.as_deref(), Some(original_uid.as_str()));
    let progress = captured.delete_recovery.as_ref().unwrap();
    assert_eq!(progress.pvc_uid.as_deref(), Some(original_uid.as_str()));
    assert!(progress.provisioning_request_id.is_none());
    assert!(progress.provisioning_terminal.is_none());
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    f.slot_retained().await;
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
    }
    f.step().await.unwrap();
    f.finish_assertions().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn claim_list_identity_survives_following_exact_404_with_late_pv() {
    let f = recovery_fixture("delete_claim_list_exact_absent", true).await;
    f.set_pv("Delete");
    let original_uid = f.machine.pvc_uid.clone().unwrap();
    {
        let mut cluster = f.cluster.lock().unwrap();
        let mut claim = cluster.pvc.take().unwrap();
        claim["spec"]["volumeName"] = json!("original-pv");
        cluster.owned_pvc_pages = Some(vec![json!({
            "apiVersion":"v1","kind":"PersistentVolumeClaimList",
            "metadata":{"resourceVersion":"listed-before-gc"},"items":[claim]
        })]);
        cluster.sandbox = None;
        cluster.attachments = vec![json!({
            "apiVersion":"storage.k8s.io/v1","kind":"VolumeAttachment",
            "metadata":{"name":"original-attachment","uid":"attachment-uid","resourceVersion":"32"},
            "spec":{"source":{"persistentVolumeName":"original-pv"},"nodeName":"original-node","attacher":CSI_DRIVER}
        })];
    }
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id":&f.machine.node_id},
            doc! {"$set":{"pvc_uid":bson::Bson::Null}},
        )
        .await
        .unwrap();
    let blocker = match f.step().await {
        Err(AppError::Conflict(code)) => Some(code),
        Err(_) => Some("non_conflict_failure".into()),
        Ok(()) => None,
    };
    assert_eq!(
        blocker.as_deref(),
        Some("delete_volume_attachment_present"),
        "a list-observed original claim must retain its residual volume obligation after exact GET404"
    );
    let current = f.current().await;
    assert_eq!(current.pvc_uid.as_deref(), Some(original_uid.as_str()));
    assert_eq!(current.native_pv_name.as_deref(), Some("original-pv"));
    let progress = current.delete_recovery.as_ref().unwrap();
    assert_eq!(progress.pvc_uid.as_deref(), Some(original_uid.as_str()));
    assert_eq!(progress.volume.as_ref().unwrap().pv_name, "original-pv");
    assert!(!progress.resources_complete);
    f.slot_retained().await;
    {
        let cluster = f.cluster.lock().unwrap();
        assert!(
            cluster.deletes.is_empty(),
            "exact404 skips the claim DELETE"
        );
        for path in [
            "/api/v1/persistentvolumes",
            "/api/v1/persistentvolumes/original-pv",
            "/apis/storage.k8s.io/v1/volumeattachments",
        ] {
            assert!(
                cluster
                    .requests
                    .iter()
                    .any(|(method, seen)| method == "GET" && seen == path),
                "missing {path}"
            );
        }
    }
    // The next pass cannot forget the anchor when the earlier list becomes empty.
    f.cluster.lock().unwrap().owned_pvc_pages = None;
    assert!(
        matches!(f.step().await, Err(AppError::Conflict(code)) if code == "delete_volume_attachment_present")
    );
    f.slot_retained().await;
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.attachments.clear();
        cluster.pv = None;
    }
    f.step().await.unwrap();
    f.finish_assertions().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn named_claim_without_captured_uid_discovers_pv_conflicts_before_and_after_finality() {
    let f = recovery_fixture("delete_named_claim_missing_uid", true).await;
    f.set_pv("Delete");
    let original_pv = {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
        cluster.pv.clone().unwrap()
    };
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id":&f.machine.node_id},
            doc! {"$set":{"pvc_uid":bson::Bson::Null}},
        )
        .await
        .unwrap();

    assert!(
        matches!(f.step().await, Err(AppError::Conflict(code)) if code == "delete_pv_claim_identity_conflict"),
        "a same-name PV must block completion when the original claim UID was never captured"
    );
    let current = f.current().await;
    assert!(current.pvc_uid.is_none());
    assert!(current.native_pv_uid.is_none());
    assert!(current.native_pv_name.is_none());
    let progress = current.delete_recovery.as_ref().unwrap();
    assert!(progress.pvc_uid.is_none());
    assert!(progress.volume.is_none());
    assert!(!progress.resources_complete);
    f.slot_retained().await;
    {
        let cluster = f.cluster.lock().unwrap();
        assert!(
            cluster
                .requests
                .iter()
                .any(|(method, path)| { method == "GET" && path == "/api/v1/persistentvolumes" })
        );
        assert!(cluster.deletes.is_empty());
        assert!(cluster.patches.is_empty());
    }

    // A missing identity must not hide a failed inventory, even after PV GC.
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.pv = None;
        cluster.pv_read_error = Some(403);
    }
    assert!(f.step().await.is_err());
    assert!(
        !f.current()
            .await
            .delete_recovery
            .unwrap()
            .resources_complete
    );
    f.slot_retained().await;
    f.cluster.lock().unwrap().pv_read_error = None;
    f.step().await.unwrap();
    f.finish_assertions().await;
    assert!(f.current().await.pvc_uid.is_none());

    // Finalized scans use the same discovery and preserve money/slot/operation.
    let invariants = FinalizedInvariants::capture(&f).await;
    f.cluster.lock().unwrap().pv = Some(original_pv);
    assert!(
        matches!(advance(&f.manager, &f.current().await).await, Err(AppError::Conflict(code)) if code == "delete_pv_claim_identity_conflict")
    );
    assert!(
        !f.current()
            .await
            .delete_recovery
            .unwrap()
            .resources_complete
    );
    invariants.assert_unchanged(&f).await;
    {
        let mut cluster = f.cluster.lock().unwrap();
        assert!(cluster.deletes.is_empty());
        assert!(cluster.patches.is_empty());
        cluster.pv = None;
        cluster.pv_read_error = Some(403);
    }
    assert!(advance(&f.manager, &f.current().await).await.is_err());
    assert!(
        !f.current()
            .await
            .delete_recovery
            .unwrap()
            .resources_complete
    );
    invariants.assert_unchanged(&f).await;
    f.cluster.lock().unwrap().pv_read_error = None;
    advance(&f.manager, &f.current().await).await.unwrap();
    assert!(
        f.current()
            .await
            .delete_recovery
            .unwrap()
            .resources_complete
    );
    invariants.assert_unchanged(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn trusted_never_submitted_generation_zero_finishes_without_any_remote_fact_or_resource() {
    let f = recovery_fixture("delete_never_submitted", false).await;
    // This fixture represents the old accepted delete before its original
    // create outbox was ever claimed. It has no remote request/result history.
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id": &f.machine.node_id},
            doc! {"$set": {
                "runtime_generation": 0_i64, "credential_epoch": 0_i64,
                "sandbox_uid": bson::Bson::Null, "pod_uid": bson::Bson::Null,
                "native_pod_name": bson::Bson::Null, "native_node_name": bson::Bson::Null,
                "native_node_uid": bson::Bson::Null, "native_uncertainty": bson::Bson::Null,
                "runtime_submission_started_at": bson::Bson::Null,
            }},
        )
        .await
        .unwrap();
    f.db.collection::<Document>(OUTBOX_COLLECTION)
        .update_one(
            doc! {"machine_id": &f.machine.node_id, "kind": "create_sandbox"},
            doc! {"$set": {"state": "pending", "attempts": 0_i64, "submission":bson::Bson::Null}},
        )
        .await
        .unwrap();
    f.step().await.unwrap();
    let current = f.current().await;
    assert_eq!(current.resource_state, ResourceState::Deleted);
    assert_eq!(current.runtime_generation, 0);
    assert!(current.sandbox_uid.is_none());
    assert!(current.pod_uid.is_none());
    assert!(current.native_observation.is_none());
    assert!(current.native_pv_uid.is_none());
    assert!(current.delete_recovery.is_none());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn missing_or_stale_controller_suspension_keeps_original_pending_claim_and_slot() {
    for defect in ["missing", "false", "stale_generation"] {
        let f = recovery_fixture("delete_missing_controller_stop", true).await;
        {
            let mut c = f.cluster.lock().unwrap();
            let sandbox = c.sandbox.as_mut().unwrap();
            match defect {
                "missing" => sandbox["status"]["conditions"] = json!([]),
                "false" => sandbox["status"]["conditions"][0]["status"] = json!("False"),
                _ => sandbox["status"]["conditions"][0]["observedGeneration"] = json!(1),
            }
        }
        assert!(f.step().await.is_err(), "{defect}");
        let current = f.current().await;
        let progress = current.delete_recovery.unwrap();
        assert_eq!(
            progress.blocker.as_deref(),
            Some("delete_current_suspension_unconfirmed")
        );
        assert_eq!(progress.pvc_uid, current.pvc_uid);
        assert!(progress.stop.is_none());
        assert!(f.cluster.lock().unwrap().deletes.is_empty());
        f.slot_retained().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn initially_absent_original_workload_completes_without_historical_stop_or_controller_claim()
{
    use crate::models::remote_machine::DeleteStopKind;
    let f = recovery_fixture("delete_initially_absent", true).await;
    let original = f.current().await;
    assert!(original.delete_recovery.is_none());
    assert!(original.native_observation.is_none());
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
    }
    let query_started = super::now();
    f.step().await.unwrap();
    let query_finished = super::now();
    f.finish_assertions().await;
    let deleted = f.current().await;
    let progress = deleted.delete_recovery.as_ref().unwrap();
    let stop = progress.stop.as_ref().unwrap();
    assert_eq!(stop.kind, DeleteStopKind::ControlPlaneWorkloadAbsent);
    assert_eq!(stop.source, "sandbox-kubernetes-control-plane");
    assert_eq!(stop.source_operation_id, original.active_operation_id);
    assert!(Uuid::parse_str(&stop.evidence_ref).is_ok());
    assert!(stop.observed_at >= query_started && stop.observed_at <= query_finished);
    assert_eq!(stop.sandbox_generation, 0);
    assert!(stop.sandbox_resource_version.is_empty());
    assert_eq!(progress.sandbox_uid, original.sandbox_uid);
    assert_eq!(progress.pod_uid, original.pod_uid);
    assert_eq!(progress.node_uid, original.native_node_uid);
    assert_eq!(progress.pvc_uid, original.pvc_uid);
    assert!(progress.creation_terminal && progress.resources_complete);
    assert_eq!(
        progress.storage,
        DeleteStorageDisposition::ControlPlaneReleased
    );
    assert!(deleted.native_observation.is_none());
    assert!(!crate::services::remote_machine_native::predecessor_stopped(&deleted));
    assert!(!lifecycle::runtime_fence_allows_bootstrap(&deleted));
    assert!(f.cluster.lock().unwrap().deletes.is_empty());
    assert!(f.cluster.lock().unwrap().patches.is_empty());
    let retained = stop.clone();
    advance(&f.manager, &deleted).await.unwrap();
    assert_eq!(
        f.current().await.delete_recovery.unwrap().stop.as_ref(),
        Some(&retained)
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_claim_late_volume_keeps_original_delete_and_records_kubernetes_identity() {
    let f = recovery_fixture("delete_late_volume", true).await;
    assert!(f.step().await.is_err());
    f.set_pv("Delete");
    {
        let mut c = f.cluster.lock().unwrap();
        c.sandbox = None;
        c.pvc = None;
    }
    assert!(f.step().await.is_err());
    let captured = f.current().await;
    let volume = captured
        .delete_recovery
        .as_ref()
        .unwrap()
        .volume
        .as_ref()
        .unwrap();
    assert_eq!(volume.csi_handle.as_deref(), Some(DISK_HANDLE));
    assert!(volume.immutable_disk_id.is_none());
    assert_eq!(
        captured.native_pv_uid.as_deref(),
        Some(volume.pv_uid.as_str())
    );
    assert_eq!(captured.active_operation_id, f.machine.active_operation_id);
    f.slot_retained().await;
    f.cluster.lock().unwrap().pv = None;
    f.step().await.unwrap();
    f.finish_assertions().await;
    assert_eq!(
        f.current().await.delete_recovery.unwrap().storage,
        DeleteStorageDisposition::ControlPlaneReleased
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_external_disk_hints_remain_readable_but_do_not_block_restart_after_kubernetes_gc() {
    let f = recovery_fixture("delete_inert_external_hints", true).await;
    f.set_pv("Delete");
    assert!(f.step().await.is_err());
    let initial = f.current().await;
    let initial_progress = initial.delete_recovery.as_ref().unwrap();
    let retained = crate::models::remote_machine::DeleteProvisioningEvidence {
        source: "legacy-csi-fixture".into(),
        evidence_ref: "original-request-terminal".into(),
        never_provisioned: false,
        disk_handles: vec![DISK_HANDLE.to_string(), EXTRA_DISK_HANDLE.to_string()],
        observed_at: super::now(),
    };
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &f.machine.node_id},
        doc! {"$set": {"delete_recovery.provisioning_terminal": bson::to_bson(&retained).unwrap()}},
    ).await.unwrap();
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
        cluster.pv = None;
    }
    f.due().await;
    let restarted = f.manager.clone();
    advance(&restarted, &f.current().await).await.unwrap();
    f.finish_assertions().await;
    let current = f.current().await;
    let progress = current.delete_recovery.as_ref().unwrap();
    assert_eq!(progress.provisioning_terminal.as_ref(), Some(&retained));
    assert_eq!(progress.evidence_id, initial_progress.evidence_id);
    assert_eq!(progress.stop, initial_progress.stop);
    assert_eq!(
        progress.storage,
        DeleteStorageDisposition::ControlPlaneReleased
    );
    assert!(progress.resources_complete);
    assert!(progress.blocker.is_none());
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    advance(&restarted, &current).await.unwrap();
    assert_eq!(
        f.current()
            .await
            .delete_recovery
            .unwrap()
            .provisioning_terminal
            .as_ref(),
        Some(&retained)
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn runtime_or_resource_conflicts_never_mutate_replacement_resources() {
    for conflict in [
        "sandbox_uid",
        "sandbox_owner",
        "sandbox_generation_tuple",
        "sandbox_fence",
        "pvc_uid",
        "pod_present",
        "pod_list_forbidden",
        "pod_list_unavailable",
    ] {
        let f = recovery_fixture(&format!("delete_conflict_{conflict}"), true).await;
        match conflict {
            "sandbox_uid" => {
                f.cluster.lock().unwrap().sandbox.as_mut().unwrap()["metadata"]["uid"] =
                    json!(Uuid::new_v4().to_string())
            }
            "sandbox_owner" => {
                f.cluster.lock().unwrap().sandbox.as_mut().unwrap()["metadata"]["labels"]["nyxid.dev/owner-id"] =
                    json!(Uuid::new_v4().to_string())
            }
            "sandbox_generation_tuple" => {
                f.cluster.lock().unwrap().sandbox.as_mut().unwrap()["metadata"]["labels"]["nyxid.dev/runtime-generation"] =
                    json!("999")
            }
            "sandbox_fence" => {
                f.cluster.lock().unwrap().sandbox.as_mut().unwrap()["metadata"]["labels"]["nyxid.dev/writer-fence"] =
                    json!("another-fence")
            }
            "pvc_uid" => {
                f.cluster.lock().unwrap().pvc.as_mut().unwrap()["metadata"]["uid"] =
                    json!(Uuid::new_v4().to_string())
            }
            "pod_present" => {
                let pod = f.pod();
                f.cluster.lock().unwrap().pods.push(pod);
            }
            "pod_list_forbidden" => f.cluster.lock().unwrap().pod_list_error = Some(403),
            "pod_list_unavailable" => f.cluster.lock().unwrap().pod_list_error = Some(503),
            _ => unreachable!(),
        }
        assert!(f.step().await.is_err(), "{conflict}");
        assert!(f.cluster.lock().unwrap().deletes.is_empty(), "{conflict}");
        assert!(f.cluster.lock().unwrap().patches.is_empty(), "{conflict}");
        assert_eq!(
            f.current().await.native_uncertainty,
            f.machine.native_uncertainty
        );
        f.slot_retained().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn retained_or_attached_volume_stays_pending_with_durable_identity() {
    for conflict in ["Retain", "attachment"] {
        let f = recovery_fixture(&format!("delete_volume_{conflict}"), true).await;
        f.set_pv(if conflict == "Retain" {
            "Retain"
        } else {
            "Delete"
        });
        if conflict == "attachment" {
            f.cluster.lock().unwrap().attachments.push(json!({"kind":"VolumeAttachment","apiVersion":"storage.k8s.io/v1","metadata":{"name":"attachment","uid":Uuid::new_v4().to_string(),"resourceVersion":"1"},"spec":{"attacher":CSI_DRIVER,"nodeName":"original-node","source":{"persistentVolumeName":"original-pv"}},"status":{"attached":false}}));
        }
        assert!(f.step().await.is_err(), "{conflict}");
        let current = f.current().await;
        let p = current.delete_recovery.unwrap();
        assert!(
            p.volume.is_some(),
            "{conflict}: persist exact identity before cleanup"
        );
        assert_eq!(current.native_pv_name.as_deref(), Some("original-pv"));
        assert_eq!(
            p.volume.as_ref().unwrap().reclaim_policy,
            if conflict == "Retain" {
                "Retain"
            } else {
                "Delete"
            }
        );
        assert!(p.volume.as_ref().unwrap().immutable_disk_id.is_none());
        assert!(f.cluster.lock().unwrap().deletes.is_empty(), "{conflict}");
        f.slot_retained().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn legacy_terminal_milestones_refresh_from_complete_current_absence_without_external_authority()
 {
    use crate::models::remote_machine::{
        DeleteProvisioningEvidence, DeleteStopEvidence, DeleteStopKind,
    };
    for storage in [
        DeleteStorageDisposition::NeverProvisioned,
        DeleteStorageDisposition::Reclaimed,
        DeleteStorageDisposition::ControlPlaneReleased,
    ] {
        let f = recovery_fixture("delete_legacy_terminal_absent", true).await;
        let current = f.current().await;
        let mut legacy = initial_progress(&current).unwrap();
        legacy.evidence_version = 0;
        legacy.storage = storage;
        legacy.creation_terminal = true;
        legacy.resources_complete = true;
        legacy.stop = Some(DeleteStopEvidence {
            kind: DeleteStopKind::TrustedRuntime,
            source_operation_id: None,
            evidence_ref: "legacy-host-stop".into(),
            source: "legacy-runtime".into(),
            observed_at: super::now(),
            sandbox_generation: 2,
            sandbox_resource_version: "21".into(),
        });
        legacy.provisioning_terminal = Some(DeleteProvisioningEvidence {
            source: "legacy-csi".into(),
            evidence_ref: "legacy-terminal".into(),
            never_provisioned: true,
            disk_handles: vec![EXTRA_DISK_HANDLE.into()],
            observed_at: super::now(),
        });
        let mut legacy_doc = bson::to_document(&legacy).unwrap();
        legacy_doc.remove("evidence_version");
        f.db.collection::<Document>(MACHINE_COLLECTION)
            .update_one(
                doc! {"_id": &current.node_id},
                doc! {"$set": {"delete_recovery": legacy_doc}},
            )
            .await
            .unwrap();
        {
            let mut cluster = f.cluster.lock().unwrap();
            cluster.sandbox = None;
            cluster.pvc = None;
        }
        let query_started = super::now();
        f.step().await.unwrap();
        f.finish_assertions().await;
        let deleted = f.current().await;
        let progress = deleted.delete_recovery.as_ref().unwrap();
        assert_eq!(progress.evidence_version, 1);
        assert_eq!(progress.pvc_uid, current.pvc_uid);
        assert_eq!(progress.sandbox_uid, current.sandbox_uid);
        let stop = progress.stop.as_ref().unwrap();
        assert_eq!(stop.kind, DeleteStopKind::ControlPlaneWorkloadAbsent);
        assert_ne!(stop.evidence_ref, "legacy-host-stop");
        assert!(stop.observed_at >= query_started);
        assert_eq!(stop.sandbox_generation, 0);
        assert!(stop.sandbox_resource_version.is_empty());
        assert!(Uuid::parse_str(&stop.evidence_ref).is_ok());
        assert!(f.cluster.lock().unwrap().deletes.is_empty());
        assert!(!f.server.received_requests().await.unwrap().is_empty());
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn legacy_evidence_refreshes_current_control_plane_stop_without_losing_original_identities() {
    use crate::models::remote_machine::{DeleteStopEvidence, DeleteStopKind};
    let f = recovery_fixture("delete_legacy_refresh", true).await;
    f.set_pv("Delete");
    assert!(f.step().await.is_err());
    let captured = f.current().await;
    let mut legacy = captured.delete_recovery.clone().unwrap();
    let volume = legacy.volume.clone().unwrap();
    legacy.evidence_version = 0;
    legacy.creation_terminal = true;
    legacy.resources_complete = true;
    legacy.storage = DeleteStorageDisposition::Reclaimed;
    legacy.stop = Some(DeleteStopEvidence {
        kind: DeleteStopKind::TrustedRuntime,
        source_operation_id: None,
        evidence_ref: "legacy-host-stop".into(),
        source: "legacy-runtime".into(),
        observed_at: super::now(),
        sandbox_generation: 2,
        sandbox_resource_version: "21".into(),
    });
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id": &captured.node_id},
            doc! {"$set": {"delete_recovery": bson::to_bson(&legacy).unwrap()}},
        )
        .await
        .unwrap();
    assert!(f.step().await.is_err());
    let refreshed = f.current().await;
    let progress = refreshed.delete_recovery.as_ref().unwrap();
    assert_eq!(progress.evidence_version, 1);
    assert_eq!(progress.volume.as_ref(), Some(&volume));
    assert_eq!(progress.pvc_uid, legacy.pvc_uid);
    assert_eq!(progress.sandbox_uid, legacy.sandbox_uid);
    assert_eq!(progress.pod_uid, legacy.pod_uid);
    assert_eq!(progress.node_uid, legacy.node_uid);
    assert!(!progress.resources_complete);
    let stop = progress.stop.as_ref().unwrap();
    assert_eq!(stop.kind, DeleteStopKind::ControlPlaneSuspended);
    assert_ne!(stop.evidence_ref, "legacy-host-stop");
    assert!(Uuid::parse_str(&stop.evidence_ref).is_ok());
    {
        let mut c = f.cluster.lock().unwrap();
        c.sandbox = None;
        c.pvc = None;
        c.pv = None;
    }
    f.step().await.unwrap();
    f.finish_assertions().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn disallowed_quarantine_retains_investigation_without_querying_resources() {
    for reason in [
        "known_force_deletion",
        "unexpected_pod_replacement",
        "unknown",
    ] {
        let f = recovery_fixture("delete_disallowed", true).await;
        f.db.collection::<Document>(MACHINE_COLLECTION)
            .update_one(
                doc! {"_id":&f.machine.node_id},
                doc! {"$set":{"native_uncertainty":reason}},
            )
            .await
            .unwrap();
        assert!(f.step().await.is_err());
        assert!(f.server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            f.current().await.native_uncertainty.as_deref(),
            Some(reason)
        );
        assert_eq!(
            f.current()
                .await
                .delete_recovery
                .unwrap()
                .blocker
                .as_deref(),
            Some("delete_uncertainty_reason_not_allowed")
        );
        f.slot_retained().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn stale_evaluation_cannot_commit_after_any_original_delete_fence_changes() {
    for changed in [
        "revision",
        "active_operation_id",
        "writer_fence",
        "runtime_generation",
        "credential_epoch",
        "owner_id",
    ] {
        let f = recovery_fixture("delete_evidence_cas", true).await;
        let original = f.current().await;
        let mut progress = initial_progress(&original).unwrap();
        let mut changed_value = Document::new();
        changed_value.insert(
            changed,
            if changed == "revision"
                || changed == "runtime_generation"
                || changed == "credential_epoch"
            {
                bson::Bson::Int64(999)
            } else {
                bson::Bson::String(Uuid::new_v4().to_string())
            },
        );
        f.db.collection::<Document>(MACHINE_COLLECTION)
            .update_one(doc! {"_id":&original.node_id}, doc! {"$set":changed_value})
            .await
            .unwrap();
        assert!(
            save(&f.db, &original, &mut progress, Document::new())
                .await
                .is_err(),
            "{changed}"
        );
        let pending =
            f.db.collection::<ManagedMachine>(MACHINE_COLLECTION)
                .find_one(doc! {"_id": &original.node_id})
                .await
                .unwrap()
                .unwrap();
        assert!(pending.delete_recovery.is_none(), "{changed}");
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn unresolved_original_create_survives_empty_inventory_and_expired_worker_lease() {
    let f = recovery_fixture("delete_unresolved_create", true).await;
    let effect =
        f.db.collection::<MachineEffectOutbox>(OUTBOX_COLLECTION)
            .find_one(doc! {"machine_id":&f.machine.node_id,"kind":"create_sandbox"})
            .await
            .unwrap()
            .unwrap();
    f.db.collection::<Document>(OUTBOX_COLLECTION).update_one(doc!{"_id":&effect.id},doc!{"$set":{"state":"retry","attempts":1_i64,"claimed_until":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(1)),"submission":{"request_id":&effect.id,"runtime_generation":1_i64,"credential_epoch":1_i64,"writer_fence":&f.machine.writer_fence,"started_at":bson::DateTime::now(),"completed_at":bson::Bson::Null,"cancelled_before_dispatch":false,"result_sandbox_uid":bson::Bson::Null,"result_resource_version":bson::Bson::Null}}}).await.unwrap();
    assert!(f.step().await.is_err());
    {
        let mut c = f.cluster.lock().unwrap();
        c.sandbox = None;
        c.pvc = None;
    }
    assert!(f.step().await.is_err());
    assert_eq!(
        f.current()
            .await
            .delete_recovery
            .unwrap()
            .blocker
            .as_deref(),
        Some("creation_result_unknown")
    );
    f.slot_retained().await;
    // The original response closes the same responsibility; no new request,
    // operation, fence or resource identity is invented by the retry.
    f.db.collection::<Document>(OUTBOX_COLLECTION).update_one(doc!{"_id":&effect.id},doc!{"$set":{"submission.completed_at":bson::DateTime::now(),"submission.result_sandbox_uid":&f.machine.sandbox_uid,"submission.result_resource_version":"21"}}).await.unwrap();
    f.step().await.unwrap();
    f.finish_assertions().await;
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn ordinary_native_stop_with_unobserved_template_claim_uses_complete_control_plane_inventory()
{
    use crate::models::remote_machine::DeleteStopKind;
    let f = recovery_fixture("delete_ordinary_no_claim", false).await;
    let current = f.current().await;
    let observation = NativeRuntimeObservation {
        sandbox_uid: current.sandbox_uid.clone().unwrap(),
        pod_uid: current.pod_uid.clone(),
        runtime_generation: current.runtime_generation,
        credential_epoch: current.credential_epoch,
        writer_fence: current.writer_fence.clone(),
        operation_id: current.active_operation_id.clone().unwrap(),
        sandbox_generation: 2,
        sandbox_resource_version: "21".into(),
        observed_at: super::now(),
        running: false,
        suspended: true,
        stop_confirmed: true,
        run_not_after: None,
        evidence_ref: "ordinary-native-stop".into(),
    };
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &current.node_id},
        doc! {"$set": {"native_uncertainty": bson::Bson::Null, "native_observation": bson::to_bson(&observation).unwrap()}},
    ).await.unwrap();
    assert!(f.step().await.is_err());
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    f.cluster.lock().unwrap().sandbox = None;
    f.step().await.unwrap();
    let deleted = f.current().await;
    let progress = deleted.delete_recovery.as_ref().unwrap();
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    assert_eq!(
        progress.storage,
        DeleteStorageDisposition::ControlPlaneReleased
    );
    assert!(progress.resources_complete);
    assert!(progress.provisioning_terminal.is_none());
    assert!(deleted.pvc_uid.is_none());
    assert!(deleted.native_pv_uid.is_none());
    assert_eq!(deleted.native_observation.as_ref(), Some(&observation));
    assert_eq!(
        progress.stop.as_ref().unwrap().kind,
        DeleteStopKind::OrdinaryNative
    );
    assert_eq!(
        progress.stop.as_ref().unwrap().evidence_ref,
        observation.evidence_ref
    );
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| { !r.url.path().contains("/nodes/") && !r.url.path().contains("/leases/") })
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn prior_pause_stop_is_preserved_when_delete_only_relabels_the_same_suspended_generation() {
    let f = recovery_fixture("delete_prior_pause_bound_claim", true).await;
    f.set_pv("Delete");
    let pv_uid = f.cluster.lock().unwrap().pv.as_ref().unwrap()["metadata"]["uid"]
        .as_str()
        .unwrap()
        .to_string();
    {
        let mut cluster = f.cluster.lock().unwrap();
        let pvc = cluster.pvc.as_mut().unwrap();
        pvc["spec"]["volumeName"] = json!("original-pv");
        pvc["status"]["phase"] = json!("Bound");
    }
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(doc! {"_id": &f.machine.node_id}, doc! {"$set": {
        "native_pv_name": "original-pv", "native_pv_uid": &pv_uid, "native_pv_reclaim_policy": "Delete",
    }}).await.unwrap();
    let current = f.current().await;
    let mut pause = operations::get_operation(
        &f.db,
        &current.owner_id,
        current.active_operation_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    pause.id = Uuid::new_v4().to_string();
    pause.action = LifecycleAction::Pause;
    pause.state = OperationState::Succeeded;
    pause.terminal_at = Some(super::now());
    f.db.collection::<crate::models::remote_machine_operation::MachineOperation>(
        crate::models::remote_machine_operation::COLLECTION_NAME,
    )
    .insert_one(&pause)
    .await
    .unwrap();
    let original_stop = NativeRuntimeObservation {
        sandbox_uid: current.sandbox_uid.clone().unwrap(),
        pod_uid: current.pod_uid.clone(),
        runtime_generation: current.runtime_generation,
        credential_epoch: current.credential_epoch,
        writer_fence: current.writer_fence.clone(),
        operation_id: pause.id.clone(),
        sandbox_generation: 2,
        sandbox_resource_version: "21".into(),
        observed_at: super::now() - chrono::Duration::seconds(120),
        running: false,
        suspended: true,
        stop_confirmed: true,
        run_not_after: None,
        evidence_ref: "original-pause-native-stop".into(),
    };
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(doc! {"_id": &current.node_id}, doc! {"$set": {
        "native_uncertainty": bson::Bson::Null, "native_observation": bson::to_bson(&original_stop).unwrap(),
    }}).await.unwrap();
    f.cluster.lock().unwrap().sandbox.as_mut().unwrap()["metadata"]["labels"]["nyxid.dev/operation-id"] =
        json!(pause.id);
    // Kubernetes increments resourceVersion for the metadata patch, but does
    // not increment generation or manufacture a new controller condition.
    f.step().await.unwrap();
    {
        let cluster = f.cluster.lock().unwrap();
        assert_eq!(cluster.patches.len(), 1);
        assert!(cluster.deletes.is_empty());
        assert_eq!(
            cluster.sandbox.as_ref().unwrap()["metadata"]["generation"],
            json!(2)
        );
    }
    assert_eq!(
        f.current().await.native_observation.as_ref(),
        Some(&original_stop)
    );
    assert!(f.step().await.is_err());
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    let rebound = f.current().await;
    assert!(
        Utc::now() - rebound.native_observation.as_ref().unwrap().observed_at
            > chrono::Duration::seconds(60),
        "persisted first stop is older than the ordinary running freshness window"
    );
    assert_eq!(rebound.native_observation.as_ref(), Some(&original_stop));
    let stop = rebound
        .delete_recovery
        .as_ref()
        .unwrap()
        .stop
        .as_ref()
        .unwrap();
    assert_eq!(
        stop.source_operation_id.as_deref(),
        Some(original_stop.operation_id.as_str())
    );
    assert_eq!(stop.observed_at, original_stop.observed_at);
    assert_eq!(stop.evidence_ref, original_stop.evidence_ref);
    assert_eq!(rebound.active_operation_id, current.active_operation_id);
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
    }
    assert!(
        f.step().await.is_err(),
        "the actual original PV remains a cleanup obligation"
    );
    f.slot_retained().await;
    f.cluster.lock().unwrap().pv = None;
    f.due().await;
    let restarted = f.manager.clone();
    let aged_persisted = f.current().await;
    assert_eq!(
        aged_persisted
            .native_observation
            .as_ref()
            .unwrap()
            .observed_at,
        original_stop.observed_at
    );
    assert!(!crate::services::remote_machine_native::fresh(
        aged_persisted.native_observation.as_ref().unwrap(),
        Utc::now()
    ));
    super::advance(&restarted, &aged_persisted).await.unwrap();
    let deleted = f.current().await;
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    assert_eq!(deleted.native_observation.as_ref(), Some(&original_stop));
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r.url.path().contains("/nodes/") && !r.url.path().contains("/leases/"))
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn original_volume_missing_before_identity_capture_is_unknown_without_destructive_effects() {
    let f = recovery_fixture("delete_pv_history_missing", true).await;
    let pv_uid = Uuid::new_v4().to_string();
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id": &f.machine.node_id},
            doc! {"$set": {
                "native_pv_name": "original-pv", "native_pv_uid": &pv_uid,
                "native_pv_reclaim_policy": bson::Bson::Null,
            }},
        )
        .await
        .unwrap();
    let invariants = PendingDeleteInvariants::capture(&f).await;
    assert!(
        matches!(f.step().await, Err(AppError::Conflict(code)) if code == "delete_original_volume_chain_missing")
    );
    let current = f.current().await;
    let progress = current.delete_recovery.as_ref().unwrap();
    assert_eq!(
        progress.blocker.as_deref(),
        Some("delete_original_volume_chain_missing")
    );
    assert_ne!(progress.storage, DeleteStorageDisposition::NeverProvisioned);
    assert!(!progress.resources_complete);
    assert!(progress.volume.is_none());
    assert_eq!(current.native_pv_name.as_deref(), Some("original-pv"));
    assert_eq!(current.native_pv_uid.as_deref(), Some(pv_uid.as_str()));
    assert!(current.native_pv_reclaim_policy.is_none());
    assert!(f.cluster.lock().unwrap().deletes.is_empty());
    assert!(f.cluster.lock().unwrap().patches.is_empty());
    invariants.assert_retained(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_original_pv_complete_persisted_anchor_survives_absence_and_explicit_gc() {
    let f = recovery_fixture("delete_complete_persisted_pv", true).await;
    let pv_uid = Uuid::new_v4().to_string();
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &f.machine.node_id},
        doc! {"$set": {"native_pv_name": "original-pv", "native_pv_uid": &pv_uid, "native_pv_reclaim_policy": "Delete"}},
    ).await.unwrap();
    let invariants = PendingDeleteInvariants::capture(&f).await;
    assert!(
        matches!(f.step().await, Err(AppError::Conflict(code)) if code == "delete_sandbox_gc_pending")
    );
    let captured = f.current().await;
    let progress = captured.delete_recovery.as_ref().unwrap();
    let volume = progress.volume.clone().unwrap();
    assert_eq!(volume.pv_name, "original-pv");
    assert_eq!(volume.pv_uid, pv_uid);
    assert_eq!(volume.reclaim_policy, "Delete");
    assert!(volume.csi_driver.is_none());
    assert!(volume.csi_handle.is_none());
    assert!(volume.immutable_disk_id.is_none());
    assert_eq!(progress.storage, DeleteStorageDisposition::Provisioned);
    assert!(!progress.resources_complete);
    assert!(progress.provisioning_terminal.is_none());
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    invariants.assert_retained(&f).await;
    f.cluster.lock().unwrap().sandbox = None;
    assert!(
        matches!(f.step().await, Err(AppError::Conflict(code)) if code == "delete_pvc_gc_pending")
    );
    let waiting = f.current().await;
    let waiting_progress = waiting.delete_recovery.as_ref().unwrap();
    assert_eq!(waiting_progress.volume.as_ref(), Some(&volume));
    assert_eq!(
        waiting_progress.storage,
        DeleteStorageDisposition::Provisioned
    );
    assert!(!waiting_progress.resources_complete);
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 2);
    invariants.assert_retained(&f).await;
    f.cluster.lock().unwrap().pvc = None;
    f.step().await.unwrap();
    f.finish_assertions().await;
    let deleted = f.current().await;
    let final_progress = deleted.delete_recovery.as_ref().unwrap();
    assert_eq!(final_progress.volume.as_ref(), Some(&volume));
    assert_eq!(
        final_progress.storage,
        DeleteStorageDisposition::ControlPlaneReleased
    );
    assert_ne!(
        final_progress.storage,
        DeleteStorageDisposition::NeverProvisioned
    );
    assert!(final_progress.provisioning_terminal.is_none());
    assert!(final_progress.resources_complete);
    assert_eq!(final_progress.evidence_id, progress.evidence_id);
    assert_eq!(final_progress.stop, progress.stop);
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 2);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_original_pv_selector_escape_conflicts_before_additional_delete() {
    for defect in [
        "claim_ref_removed",
        "claim_ref_reassigned",
        "reassigned_retain",
        "reassigned_missing_policy",
        "reassigned_csi_driver",
        "reassigned_csi_handle",
        "reassigned_csi_removed",
    ] {
        let f = recovery_fixture(&format!("delete_exact_pv_{defect}"), true).await;
        f.set_pv("Delete");
        assert!(
            matches!(f.step().await, Err(AppError::Conflict(code)) if code == "delete_sandbox_gc_pending")
        );
        let captured = f.current().await;
        let old_progress = captured.delete_recovery.as_ref().unwrap().clone();
        let old_volume = old_progress.volume.as_ref().unwrap();
        assert_eq!(old_volume.csi_driver.as_deref(), Some(CSI_DRIVER));
        assert_eq!(old_volume.csi_handle.as_deref(), Some(DISK_HANDLE));
        let invariants = PendingDeleteInvariants::capture(&f).await;
        let deletes_before = f.cluster.lock().unwrap().deletes.clone();
        {
            let mut cluster = f.cluster.lock().unwrap();
            cluster.sandbox = None;
            assert!(
                cluster.pvc.is_some(),
                "leave original PVC for the unsafe fallback to target"
            );
            let pv = cluster.pv.as_mut().unwrap();
            assert_eq!(pv["metadata"]["uid"], json!(old_volume.pv_uid));
            pv["metadata"]["resourceVersion"] = json!("32");
            if defect == "claim_ref_removed" {
                pv["spec"].as_object_mut().unwrap().remove("claimRef");
            } else {
                pv["spec"]["claimRef"] = json!({"namespace":"another-namespace","name":"another-claim","uid":Uuid::new_v4().to_string()});
            }
            match defect {
                "reassigned_retain" => {
                    pv["spec"]["persistentVolumeReclaimPolicy"] = json!("Retain")
                }
                "reassigned_missing_policy" => {
                    pv["spec"]
                        .as_object_mut()
                        .unwrap()
                        .remove("persistentVolumeReclaimPolicy");
                }
                "reassigned_csi_driver" => {
                    pv["spec"]["csi"]["driver"] = json!("another.csi.example")
                }
                "reassigned_csi_handle" => {
                    pv["spec"]["csi"]["volumeHandle"] = json!(EXTRA_DISK_HANDLE)
                }
                "reassigned_csi_removed" => {
                    pv["spec"].as_object_mut().unwrap().remove("csi");
                }
                _ => (),
            }
        }
        assert!(
            f.manager
                .api
                .persistent_volumes_for_claim(
                    &captured.sandbox_namespace,
                    captured.pvc_name.as_deref().unwrap(),
                    Some(captured.pvc_uid.as_deref().unwrap()),
                )
                .await
                .unwrap()
                .is_empty(),
            "{defect}: exercise the real selector-escaping exact-read fallback"
        );
        let error = f.step().await.unwrap_err();
        assert_eq!(
            f.cluster.lock().unwrap().deletes,
            deletes_before,
            "{defect}: no additional DELETE may precede durable conflict"
        );
        assert!(
            matches!(&error, AppError::Conflict(code) if code == "delete_pv_claim_identity_conflict"),
            "{defect}: expected current exact-PV claim conflict"
        );
        let current = f.current().await;
        let progress = current.delete_recovery.as_ref().unwrap();
        assert_eq!(
            progress.blocker.as_deref(),
            Some("delete_pv_claim_identity_conflict"),
            "{defect}"
        );
        assert!(!progress.resources_complete);
        assert_eq!(
            progress.volume, old_progress.volume,
            "{defect}: retain original PV/CSI identity"
        );
        assert_eq!(progress.evidence_id, old_progress.evidence_id);
        assert_eq!(progress.stop, old_progress.stop);
        assert_eq!(progress.storage, old_progress.storage);
        assert!(f.cluster.lock().unwrap().patches.is_empty());
        invariants.assert_retained(&f).await;
        f.db.drop().await.unwrap();
    }
}

async fn seed_ordinary_native_storage(f: &RecoveryFixture) -> NativeRuntimeObservation {
    let first_stop_at = super::now() - chrono::Duration::seconds(120);
    f.set_pv("Delete");
    let pv_uid = f.cluster.lock().unwrap().pv.as_ref().unwrap()["metadata"]["uid"]
        .as_str()
        .unwrap()
        .to_string();
    {
        let mut cluster = f.cluster.lock().unwrap();
        let pvc = cluster.pvc.as_mut().unwrap();
        pvc["spec"]["volumeName"] = json!("original-pv");
        pvc["status"]["phase"] = json!("Bound");
    }
    let current = f.current().await;
    let mut pause = operations::get_operation(
        &f.db,
        &current.owner_id,
        current.active_operation_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    pause.id = Uuid::new_v4().to_string();
    pause.action = LifecycleAction::Pause;
    pause.state = OperationState::Succeeded;
    pause.created_at = first_stop_at - chrono::Duration::seconds(10);
    pause.updated_at = first_stop_at;
    pause.terminal_at = Some(first_stop_at);
    f.db.collection::<crate::models::remote_machine_operation::MachineOperation>(
        crate::models::remote_machine_operation::COLLECTION_NAME,
    )
    .insert_one(&pause)
    .await
    .unwrap();
    let observation = NativeRuntimeObservation {
        sandbox_uid: current.sandbox_uid.clone().unwrap(),
        pod_uid: current.pod_uid.clone(),
        runtime_generation: current.runtime_generation,
        credential_epoch: current.credential_epoch,
        writer_fence: current.writer_fence.clone(),
        operation_id: pause.id,
        sandbox_generation: 2,
        sandbox_resource_version: "21".into(),
        observed_at: first_stop_at,
        running: false,
        suspended: true,
        stop_confirmed: true,
        run_not_after: None,
        evidence_ref: "ordinary-storage-original-pause-stop".into(),
    };
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &current.node_id},
        doc! {"$set": {"native_uncertainty": bson::Bson::Null, "native_observation": bson::to_bson(&observation).unwrap(),
            "native_pv_name": "original-pv", "native_pv_uid": &pv_uid, "native_pv_reclaim_policy": "Delete"}},
    ).await.unwrap();
    assert!(crate::services::remote_machine_native::predecessor_stopped(
        &f.current().await
    ));
    assert!(!crate::services::remote_machine_native::fresh(
        &observation,
        Utc::now()
    ));
    observation
}

#[tokio::test]
async fn ordinary_native_storage_identity_is_persisted_before_first_delete() {
    use crate::models::remote_machine::DeleteStopKind;
    let mut f = recovery_fixture("delete_ordinary_storage_first_effect", true).await;
    let original_stop = seed_ordinary_native_storage(&f).await;
    let invariants = PendingDeleteInvariants::capture(&f).await;
    let proxy = proxy_with_pre_delete_database_proof(&mut f).await;
    assert!(f.step().await.is_err());
    let proofs = proxy.proofs.lock().unwrap().clone();
    assert_eq!(
        proofs.len(),
        1,
        "exercise the first actual destructive request"
    );
    assert!(
        proofs[0]
            .path
            .ends_with(&format!("/sandboxes/{}", f.machine.sandbox_name))
    );
    let saved = &proofs[0].machine;
    let progress = saved.delete_recovery.as_ref().unwrap();
    let volume = progress.volume.as_ref().unwrap();
    assert_eq!(saved.pvc_name.as_deref(), Some("workspace"));
    assert_eq!(saved.pvc_uid, invariants.machine.pvc_uid);
    assert_eq!(progress.pvc_uid, saved.pvc_uid);
    assert_eq!(volume.pv_name, "original-pv");
    assert_eq!(Some(volume.pv_uid.as_str()), saved.native_pv_uid.as_deref());
    assert_eq!(volume.reclaim_policy, "Delete");
    assert_eq!(
        volume.csi_driver.as_deref(),
        Some(CSI_DRIVER),
        "current CSI driver must be durable before first DELETE"
    );
    assert_eq!(
        volume.csi_handle.as_deref(),
        Some(DISK_HANDLE),
        "current CSI handle must be durable before first DELETE"
    );
    assert!(volume.immutable_disk_id.is_none());
    assert_eq!(progress.storage, DeleteStorageDisposition::Provisioned);
    assert!(!progress.resources_complete);
    let stop = progress.stop.as_ref().unwrap();
    assert_eq!(stop.kind, DeleteStopKind::OrdinaryNative);
    assert_eq!(stop.source, "kubernetes-native");
    assert_eq!(
        stop.source_operation_id.as_deref(),
        Some(original_stop.operation_id.as_str())
    );
    assert_eq!(stop.observed_at, original_stop.observed_at);
    assert_eq!(stop.evidence_ref, original_stop.evidence_ref);
    assert_eq!(stop.sandbox_generation, original_stop.sandbox_generation);
    assert_eq!(
        stop.sandbox_resource_version,
        original_stop.sandbox_resource_version
    );
    assert_eq!(saved.native_observation.as_ref(), Some(&original_stop));
    for path in [
        format!(
            "/api/v1/namespaces/{}/persistentvolumeclaims/workspace",
            saved.sandbox_namespace
        ),
        "/api/v1/persistentvolumes".into(),
        "/api/v1/persistentvolumes/original-pv".into(),
    ] {
        assert!(
            proofs[0]
                .prior_requests
                .iter()
                .any(|(method, seen)| method == "GET" && seen == &path),
            "verified current storage read must precede DELETE: {path}"
        );
    }
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    assert!(f.cluster.lock().unwrap().patches.is_empty());
    invariants.assert_retained(&f).await;
    drop(proxy);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn ordinary_native_storage_read_failure_retain_and_claim_conflict_prevent_delete() {
    for defect in [
        "pv_forbidden",
        "pv_unavailable",
        "Retain",
        "claim_uid",
        "claim_ref_reassigned",
    ] {
        let f = recovery_fixture(&format!("delete_ordinary_storage_{defect}"), true).await;
        let original_stop = seed_ordinary_native_storage(&f).await;
        let invariants = PendingDeleteInvariants::capture(&f).await;
        {
            let mut cluster = f.cluster.lock().unwrap();
            match defect {
                "pv_forbidden" => cluster.pv_read_error = Some(403),
                "pv_unavailable" => cluster.pv_read_error = Some(503),
                "Retain" => {
                    cluster.pv.as_mut().unwrap()["spec"]["persistentVolumeReclaimPolicy"] =
                        json!("Retain")
                }
                "claim_uid" => {
                    cluster.pv.as_mut().unwrap()["spec"]["claimRef"]["uid"] =
                        json!(Uuid::new_v4().to_string())
                }
                "claim_ref_reassigned" => {
                    cluster.pv.as_mut().unwrap()["spec"]["claimRef"] = json!({"namespace":"another-namespace","name":"another-claim","uid":Uuid::new_v4().to_string()})
                }
                _ => unreachable!(),
            }
        }
        assert!(f.step().await.is_err(), "{defect}");
        assert!(
            f.cluster.lock().unwrap().deletes.is_empty(),
            "{defect}: verify current PV before any DELETE"
        );
        assert!(f.cluster.lock().unwrap().patches.is_empty(), "{defect}");
        let current = f.current().await;
        let expected_blocker = match defect {
            "pv_forbidden" | "pv_unavailable" => "delete_resource_observation_unknown",
            "Retain" => "delete_volume_identity_conflict",
            "claim_uid" | "claim_ref_reassigned" => "delete_pv_claim_identity_conflict",
            _ => unreachable!(),
        };
        let progress = current.delete_recovery.as_ref().unwrap();
        assert_eq!(
            progress.blocker.as_deref(),
            Some(expected_blocker),
            "{defect}"
        );
        assert!(!progress.resources_complete, "{defect}");
        assert_eq!(current.native_observation.as_ref(), Some(&original_stop));
        invariants.assert_retained(&f).await;
        f.db.drop().await.unwrap();
    }
}

async fn reopen_original_submission(f: &RecoveryFixture) -> (ManagedMachine, MachineEffectOutbox) {
    let mut effect =
        f.db.collection::<MachineEffectOutbox>(OUTBOX_COLLECTION)
            .find_one(doc! {"machine_id": &f.machine.node_id, "kind": "create_sandbox"})
            .await
            .unwrap()
            .unwrap();
    effect.submission = None;
    f.db.collection::<Document>(OUTBOX_COLLECTION)
        .update_one(
            doc! {"_id":&effect.id},
            doc! {"$set":{"submission":bson::Bson::Null}},
        )
        .await
        .unwrap();
    // Construct the state immediately before the owner delete, keeping its
    // original request and tuple. The test accepts no network result here.
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &f.machine.node_id},
        doc! {"$set": {"active_operation_id": &effect.operation_id, "resource_state": "provisioning"}},
    ).await.unwrap();
    (f.current().await, effect)
}

async fn restore_original_delete(f: &RecoveryFixture) {
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &f.machine.node_id},
        doc! {"$set": {"active_operation_id": &f.machine.active_operation_id, "resource_state": "deleting"}},
    ).await.unwrap();
}

#[tokio::test]
async fn distinct_redispatch_results_cannot_hide_an_earlier_unresolved_create_request() {
    let f = recovery_fixture("delete_redispatch_journal", true).await;
    let (running, effect) = reopen_original_submission(&f).await;
    let earlier = operations::begin_submission(&f.db, &running, &effect)
        .await
        .unwrap();
    let later = operations::begin_submission(&f.db, &running, &effect)
        .await
        .unwrap();
    let cancelled = operations::begin_submission(&f.db, &running, &effect)
        .await
        .unwrap();
    assert_ne!(earlier, later);
    assert_ne!(cancelled, later);
    let observation = nyxid_machine_updater::sandbox::SandboxObservation::from_object(
        f.cluster.lock().unwrap().sandbox.as_ref().unwrap(),
    )
    .unwrap();
    operations::finish_submission(&f.db, &effect.id, &later, Some(&observation))
        .await
        .unwrap();
    operations::finish_submission(&f.db, &effect.id, &cancelled, None)
        .await
        .unwrap();
    restore_original_delete(&f).await;
    assert!(
        !operations::creation_responsibilities_terminal(&f.db, &f.current().await)
            .await
            .unwrap()
    );
    assert!(f.step().await.is_err());
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
    }
    assert!(f.step().await.is_err());
    assert_eq!(
        f.current()
            .await
            .delete_recovery
            .unwrap()
            .blocker
            .as_deref(),
        Some("creation_result_unknown")
    );
    f.slot_retained().await;
    operations::finish_submission(&f.db, &effect.id, &earlier, Some(&observation))
        .await
        .unwrap();
    let completed =
        f.db.collection::<MachineEffectOutbox>(OUTBOX_COLLECTION)
            .find_one(doc! {"_id": &effect.id})
            .await
            .unwrap()
            .unwrap();
    let completed_at = completed
        .submission
        .unwrap()
        .dispatches
        .into_iter()
        .find(|d| d.id == earlier)
        .unwrap()
        .completed_at;
    operations::finish_submission(&f.db, &effect.id, &earlier, Some(&observation))
        .await
        .unwrap();
    let replay =
        f.db.collection::<MachineEffectOutbox>(OUTBOX_COLLECTION)
            .find_one(doc! {"_id": &effect.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        replay
            .submission
            .unwrap()
            .dispatches
            .into_iter()
            .find(|d| d.id == earlier)
            .unwrap()
            .completed_at,
        completed_at
    );
    assert!(
        operations::finish_submission(&f.db, &effect.id, &earlier, None)
            .await
            .is_err()
    );
    f.step().await.unwrap();
    f.finish_assertions().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn unresolved_suspended_rekey_submission_remains_a_creation_responsibility() {
    use crate::models::remote_machine_outbox::{EffectKind, EffectState};
    let f = recovery_fixture("delete_rekey_request", true).await;
    let mut running = f.current().await;
    let original_create =
        f.db.collection::<MachineEffectOutbox>(OUTBOX_COLLECTION)
            .find_one(doc! {"machine_id": &f.machine.node_id, "kind": "create_sandbox"})
            .await
            .unwrap()
            .unwrap();
    let original_submission = bson::to_bson(&original_create.submission).unwrap();
    assert!(
        operations::creation_responsibilities_terminal(&f.db, &running)
            .await
            .unwrap()
    );
    // This is a distinct rekey request. Keep the original create's completed
    // result intact so only the rekey dispatch introduces an open obligation.
    let mut rekey = original_create.clone();
    rekey.id = Uuid::new_v4().to_string();
    rekey.operation_id = Uuid::new_v4().to_string();
    rekey.kind = EffectKind::RekeySandbox;
    rekey.desired_mode = "suspended".into();
    rekey.state = EffectState::Claimed;
    rekey.attempts = 1;
    rekey.submission = None;
    running.active_operation_id = Some(rekey.operation_id.clone());
    running.resource_state = ResourceState::Provisioning;
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id": &running.node_id},
            doc! {"$set": {"active_operation_id": &rekey.operation_id, "resource_state": "provisioning"}},
        )
        .await
        .unwrap();
    f.db.collection::<MachineEffectOutbox>(OUTBOX_COLLECTION)
        .insert_one(&rekey)
        .await
        .unwrap();
    let dispatch = operations::begin_submission(&f.db, &running, &rekey)
        .await
        .unwrap();
    restore_original_delete(&f).await;
    assert!(
        !operations::creation_responsibilities_terminal(&f.db, &f.current().await)
            .await
            .unwrap()
    );
    operations::finish_submission(&f.db, &rekey.id, &dispatch, None)
        .await
        .unwrap();
    assert!(
        operations::creation_responsibilities_terminal(&f.db, &f.current().await)
            .await
            .unwrap()
    );
    let retained_create =
        f.db.collection::<MachineEffectOutbox>(OUTBOX_COLLECTION)
            .find_one(doc! {"_id": &original_create.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        bson::to_bson(&retained_create.submission).unwrap(),
        original_submission
    );
    f.db.drop().await.unwrap();
}

async fn insert_unready_runtime_run(f: &RecoveryFixture, run_id: &str) {
    use crate::models::{
        credits::Credits,
        machine_runtime_billing::*,
        service_billing::{BillingMetric, PricingSyncStatus},
    };
    let current = f.current().await;
    let at = super::now() - chrono::Duration::seconds(5);
    let ns = at.timestamp_nanos_opt().unwrap();
    let run = MachineRuntimeRun {
        id: run_id.into(),
        node_id: current.node_id.clone(),
        owner_id: current.owner_id.clone(),
        quote_id: Uuid::new_v4().to_string(),
        operation_id: current.active_operation_id.clone().unwrap(),
        rate: MachineRuntimeRate {
            id: Uuid::new_v4().to_string(),
            profile_version: current.profile.profile_version.clone(),
            metric: BillingMetric::MachineRunningSeconds,
            lago_metric_code: "fixture-runtime".into(),
            plan_code: "fixture-plan".into(),
            credits_per_second: Credits::from_whole(1),
            sync_status: PricingSyncStatus::Synced,
            retired: false,
            verified_at: Some(at),
            created_at: at,
        },
        terms: crate::services::remote_machine_runtime::terms(
            &current.profile,
            "remote-machine-mvp",
        )
        .unwrap(),
        state: MachineRuntimeRunState::Accepted,
        runtime_generation: current.runtime_generation,
        revision: 0,
        generation_admitted_ns: ns,
        ready_ns: None,
        proven_until_ns: None,
        proven_evidence_id: None,
        replacement_requested: false,
        stop_ns: None,
        stop_is_upper_bound: false,
        ready_evidence_id: None,
        stop_evidence_id: None,
        covered_until_ns: ns,
        last_window_from_ns: ns,
        accounted_until_ns: ns,
        cumulative_covered_ns: 0,
        cumulative_seconds: 0,
        generation_covered_start_ns: 0,
        platform_residual_ns: 0,
        platform_residual_upper_bound_ns: 0,
        generation_residual_recorded: false,
        generation_floor_ns: None,
        created_at: at,
        updated_at: at,
    };
    f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .insert_one(run)
        .await
        .unwrap();
}

#[tokio::test]
async fn private_paid_delete_consumer_preserves_uncertainty_and_first_stop_upper_bound() {
    use crate::models::machine_runtime_billing::{
        MachineRuntimeRun, MachineRuntimeRunState, RUN_COLLECTION,
    };
    use crate::services::remote_machine_runtime as runtime;
    let f = recovery_fixture("delete_paid_private_consumer", true).await;
    assert!(f.step().await.is_err());
    let run_id = Uuid::new_v4().to_string();
    insert_unready_runtime_run(&f, &run_id).await;
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &f.machine.node_id}, doc! {"$set": {"free_slot": false, "runtime_run_id": &run_id, "billing_state": "pending"}},
    ).await.unwrap();
    let config = crate::config::AppConfig {
        billing_enabled: true,
        lago_api_url: None,
        lago_api_key: None,
        ..crate::test_utils::test_app_config()
    };
    let billing = Arc::new(crate::services::billing::BillingService::new(
        f.db.clone(),
        Arc::new(config),
    ));
    let machine = f.current().await;
    let stop = machine
        .delete_recovery
        .as_ref()
        .unwrap()
        .stop
        .clone()
        .unwrap();
    runtime::settle_persisted_delete(&f.db, billing.clone(), &machine)
        .await
        .unwrap();
    let first =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id": &run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(first.state, MachineRuntimeRunState::Closed);
    assert_eq!(first.stop_ns, stop.observed_at.timestamp_nanos_opt());
    assert!(first.stop_is_upper_bound);
    assert_eq!(
        first.stop_evidence_id.as_deref(),
        Some(stop.evidence_ref.as_str())
    );
    assert_eq!(first.cumulative_seconds, 0);
    runtime::settle_persisted_delete(&f.db, billing, &f.current().await)
        .await
        .unwrap();
    let replay =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id": &run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(replay.stop_ns, first.stop_ns);
    assert_eq!(replay.stop_evidence_id, first.stop_evidence_id);
    assert_eq!(replay.cumulative_seconds, first.cumulative_seconds);
    assert_eq!(replay.revision, first.revision);
    let current = f.current().await;
    assert_eq!(current.native_uncertainty, f.machine.native_uncertainty);
    assert!(!current.writer_admitted);
    assert!(!crate::services::remote_machine_native::predecessor_stopped(&current));
    assert!(!lifecycle::runtime_fence_allows_bootstrap(&current));
    assert!(
        crate::services::remote_machine_native::activate(&f.db, &current)
            .await
            .is_err()
    );
    assert_eq!(
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn absent_accounting_service_does_not_block_safe_cleanup_or_release_slot_early() {
    let f = recovery_fixture("delete_accounting_pending", true).await;
    let run_id = Uuid::new_v4().to_string();
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &f.machine.node_id}, doc! {"$set": {"free_slot": false, "runtime_run_id": &run_id, "billing_state": "pending"}},
    ).await.unwrap();
    assert!(f.step().await.is_err());
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    {
        let mut c = f.cluster.lock().unwrap();
        c.sandbox = None;
        c.pvc = None;
    }
    assert!(f.step().await.is_err());
    assert!(
        f.current()
            .await
            .delete_recovery
            .unwrap()
            .resources_complete
    );
    f.slot_retained().await;
    let operation = operations::get_operation(
        &f.db,
        &f.machine.owner_id,
        f.machine.active_operation_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(operation.state, OperationState::Succeeded);
    insert_unready_runtime_run(&f, &run_id).await;
    f.db.collection::<Document>(crate::models::machine_runtime_billing::RUN_COLLECTION)
        .update_one(doc! {"_id": &run_id}, doc! {"$set": {"state": "closed"}})
        .await
        .unwrap();
    f.step().await.unwrap();
    f.finish_assertions().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn malformed_foreign_pod_owner_blocks_cleanup_and_absence_finality() {
    for workload_present in [true, false] {
        let f = recovery_fixture("delete_foreign_owner_inventory", true).await;
        {
            let mut cluster = f.cluster.lock().unwrap();
            cluster.pods = vec![json!({
                "apiVersion":"v1","kind":"Pod",
                "metadata":{"name":"foreign-pod","namespace":"other-namespace",
                    "uid":"foreign-pod-uid","resourceVersion":"2",
                    "ownerReferences":[{"apiVersion":"apps/v1","kind":"ReplicaSet",
                        "name":"foreign","uid":"foreign-owner","controller":"true"}]},
                "spec":{"containers":[{"name":"foreign"}],"volumes":[]}
            })];
            if !workload_present {
                cluster.sandbox = None;
                cluster.pvc = None;
            }
        }
        let invariants = PendingDeleteInvariants::capture(&f).await;
        assert!(
            matches!(f.step().await, Err(AppError::Conflict(code)) if code == "delete_resource_observation_unknown"),
            "malformed nonmatching Pod ownership must block both cleanup and workload absence"
        );
        let current = f.current().await;
        assert_ne!(current.cleanup_state, "complete");
        assert!(
            current
                .delete_recovery
                .as_ref()
                .is_none_or(|p| !p.resources_complete)
        );
        assert!(f.cluster.lock().unwrap().deletes.is_empty());
        assert!(
            f.cluster
                .lock()
                .unwrap()
                .requests
                .iter()
                .any(|(method, path)| method == "GET" && path == "/api/v1/pods")
        );
        invariants.assert_retained(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn partial_delete_pvc_discovery_never_discards_malformed_or_incomplete_inventory() {
    for defect in ["continue_type", "missing_continue", "malformed_owner"] {
        let f = recovery_fixture("delete_strict_claim_discovery", true).await;
        let mut page = json!({"kind":"PersistentVolumeClaimList","apiVersion":"v1",
            "metadata":{"resourceVersion":"strict-snapshot"},"items":[]});
        match defect {
            "continue_type" => page["metadata"]["continue"] = json!(123),
            "missing_continue" => page["metadata"]["remainingItemCount"] = json!(1),
            _ => {
                let mut claim = f.cluster.lock().unwrap().pvc.clone().unwrap();
                claim["metadata"]["ownerReferences"] = json!([{"uid":123}]);
                page["items"] = json!([claim]);
            }
        }
        f.cluster.lock().unwrap().owned_pvc_pages = Some(vec![page]);
        assert!(f.step().await.is_err(), "{defect}");
        let current = f.current().await;
        assert_ne!(current.cleanup_state, "complete");
        assert!(
            current
                .delete_recovery
                .as_ref()
                .is_none_or(|p| !p.resources_complete)
        );
        assert!(f.cluster.lock().unwrap().deletes.is_empty());
        f.slot_retained().await;
        // A complete second page with the original claim must become visible
        // to discovery and retain the exact original UID before cleanup.
        let claim = f.cluster.lock().unwrap().pvc.clone().unwrap();
        let uid = claim["metadata"]["uid"].as_str().unwrap().to_string();
        f.cluster.lock().unwrap().owned_pvc_pages = Some(vec![
            json!({"kind":"PersistentVolumeClaimList","apiVersion":"v1","metadata":{"resourceVersion":"stable","continue":"second","remainingItemCount":1},"items":[]}),
            json!({"kind":"PersistentVolumeClaimList","apiVersion":"v1","metadata":{"resourceVersion":"stable","remainingItemCount":0},"items":[claim]}),
        ]);
        let _ = f.step().await;
        assert_eq!(
            f.current()
                .await
                .delete_recovery
                .as_ref()
                .and_then(|p| p.pvc_uid.as_deref()),
            Some(uid.as_str())
        );
        assert!(
            f.cluster.lock().unwrap().deletes.len() == 1,
            "complete multipage discovery must reach exact original cleanup"
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn partially_created_sandbox_needs_no_fabricated_pod_node_or_csi_fact() {
    use crate::models::remote_machine::DeleteStopKind;
    let f = recovery_fixture("delete_partial_no_pod_node", false).await;
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id": &f.machine.node_id},
            doc! {"$set": {
                "pod_uid": bson::Bson::Null, "native_pod_name": bson::Bson::Null,
                "native_node_uid": bson::Bson::Null, "native_node_name": bson::Bson::Null,
            }},
        )
        .await
        .unwrap();
    assert!(f.step().await.is_err());
    let current = f.current().await;
    let progress = current.delete_recovery.as_ref().unwrap();
    assert_eq!(
        progress.stop.as_ref().unwrap().kind,
        DeleteStopKind::ControlPlaneSuspended
    );
    assert!(progress.pod_uid.is_none());
    assert!(progress.node_uid.is_none());
    assert!(current.pod_uid.is_none());
    assert!(current.native_node_uid.is_none());
    assert!(current.native_observation.is_none());
    assert!(progress.provisioning_terminal.is_none());
    assert!(!crate::services::remote_machine_native::predecessor_stopped(&current));
    assert!(!lifecycle::runtime_fence_allows_bootstrap(&current));
    f.cluster.lock().unwrap().sandbox = None;
    f.step().await.unwrap();
    f.finish_assertions().await;
    assert_eq!(
        f.current().await.delete_recovery.unwrap().storage,
        DeleteStorageDisposition::ControlPlaneReleased
    );
    assert!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| { !r.url.path().contains("/nodes/") && !r.url.path().contains("/leases/") })
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn current_persisted_control_plane_stop_survives_manual_retry_after_lifetime_limit() {
    let f = recovery_fixture("delete_aged_control_plane_stop", true).await;
    assert!(f.step().await.is_err());
    let original = f.current().await;
    let mut retained = original.delete_recovery.clone().unwrap();
    let first_stop = super::now() - chrono::Duration::days(120);
    retained.stop.as_mut().unwrap().observed_at = first_stop;
    retained.observed_at = first_stop;
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id": &original.node_id},
            doc! {"$set": {"delete_recovery": bson::to_bson(&retained).unwrap()}},
        )
        .await
        .unwrap();
    f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME).update_one(
        doc! {"_id": &original.active_operation_id},
        doc! {"$set": {"retry_count": 100_000_i64, "delete_retry":bson::Bson::Null, "deadline_at": bson::DateTime::from_chrono(first_stop), "created_at": bson::DateTime::from_chrono(first_stop)}},
    ).await.unwrap();
    let exhausted = f.current().await;
    super::super::remote_machine_delete_retry::retry(
        &f.db,
        &exhausted.owner_id,
        exhausted.active_operation_id.as_deref().unwrap(),
        exhausted.revision,
        100_000,
        "renew-aged-delete",
    )
    .await
    .unwrap();
    {
        let mut c = f.cluster.lock().unwrap();
        c.sandbox = None;
        c.pvc = None;
    }
    f.step().await.unwrap();
    f.finish_assertions().await;
    let final_progress = f.current().await.delete_recovery.unwrap();
    assert_eq!(
        final_progress.stop.as_ref().unwrap().observed_at,
        first_stop
    );
    assert_eq!(
        final_progress.stop.as_ref().unwrap().evidence_ref,
        retained.stop.as_ref().unwrap().evidence_ref
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn delete_retry_limit_stops_both_entry_points_and_retains_cleanup() {
    let f = recovery_fixture("delete_retry_limit", true).await;
    f.set_pv("Retain");
    for _ in 0..10 {
        assert!(f.step().await.is_err());
    }
    let before = f.cluster.lock().unwrap().requests.len();
    f.due().await;
    let machine = f.current().await;
    assert!(advance(&f.manager, &machine).await.is_ok());
    assert!(f.manager.observe(&machine).await.is_ok());
    assert_eq!(f.cluster.lock().unwrap().requests.len(), before);
    f.slot_retained().await;
    let op = operations::get_operation(
        &f.db,
        &machine.owner_id,
        machine.active_operation_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(op.retry_count, 10);
    assert!(op.terminal_at.is_none());
    assert_eq!(
        op.cleanup_state,
        crate::models::remote_machine_operation::CleanupState::Pending
    );
    let view = serde_json::to_value(lifecycle::safe_operation_view(&op)).unwrap();
    assert_eq!(view["delete_retry_stopped"], true);
    assert_eq!(view["next_attempt_at"], Value::Null);
    assert!(!f.current().await.writer_admitted);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn delete_retry_limit_reserves_one_minute_without_progress_double_counting() {
    let f = recovery_fixture("delete_retry_spacing", true).await;
    f.set_pv("Retain");
    assert!(f.step().await.is_err());
    let machine = f.current().await;
    let op_id = machine.active_operation_id.as_deref().unwrap();
    let op = operations::get_operation(&f.db, &machine.owner_id, op_id)
        .await
        .unwrap()
        .unwrap();
    assert!(op.next_attempt_at.unwrap() >= Utc::now() + chrono::Duration::seconds(55));
    let before = f.cluster.lock().unwrap().requests.len();
    assert!(f.manager.observe(&machine).await.is_ok());
    f.manager
        .progress(op_id, "runtime_evidence_pending")
        .await
        .unwrap();
    let after = operations::get_operation(&f.db, &machine.owner_id, op_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.retry_count, 1);
    assert_eq!(after.next_attempt_at, op.next_attempt_at);
    assert_eq!(f.cluster.lock().unwrap().requests.len(), before);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn delete_retry_limit_stops_legacy_exhausted_operations_before_kubernetes_io() {
    let f = recovery_fixture("delete_legacy_retry_limit", true).await;
    let machine = f.current().await;
    f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &machine.active_operation_id},
            doc! {"$set": {"retry_count": 15_639_i64}},
        )
        .await
        .unwrap();
    assert!(f.step().await.is_ok());
    assert!(f.cluster.lock().unwrap().requests.is_empty());
    let op = operations::get_operation(
        &f.db,
        &machine.owner_id,
        machine.active_operation_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(op.retry_count, 15_639);
    assert_eq!(
        serde_json::to_value(lifecycle::safe_operation_view(&op)).unwrap()["delete_retry_stopped"],
        true
    );
    f.slot_retained().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn delete_retry_limit_manual_reset_is_owner_bound_idempotent_and_reuses_original_delete() {
    use super::super::remote_machine_delete_retry as retry;
    let f = recovery_fixture("delete_manual_retry", true).await;
    f.set_pv("Retain");
    assert!(f.step().await.is_err());
    let machine = f.current().await;
    let op_id = machine.active_operation_id.as_deref().unwrap();
    let collection =
        f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME);
    collection
        .update_one(
            doc! {"_id":op_id},
            doc! {"$set":{"retry_count":15_639_i64,"delete_retry":bson::Bson::Null}},
        )
        .await
        .unwrap();
    let before = operations::get_operation(&f.db, &machine.owner_id, op_id)
        .await
        .unwrap()
        .unwrap();
    let count = collection.count_documents(doc! {}).await.unwrap();
    for (owner, revision, total) in [
        (Uuid::new_v4().to_string(), machine.revision, 15_639),
        (machine.owner_id.clone(), machine.revision - 1, 15_639),
        (machine.owner_id.clone(), machine.revision, 15_638),
    ] {
        assert!(
            retry::retry(&f.db, &owner, op_id, revision, total, "manual-delete")
                .await
                .is_err()
        );
    }
    let resumed = retry::retry(
        &f.db,
        &machine.owner_id,
        op_id,
        machine.revision,
        15_639,
        "manual-delete",
    )
    .await
    .unwrap();
    assert_eq!(resumed.id, before.id);
    assert_eq!(resumed.retry_count, 15_639);
    assert_eq!(retry::attempts(&resumed), 0);
    assert_eq!(resumed.error_code, before.error_code);
    assert_eq!(resumed.evidence_digest, before.evidence_digest);
    assert_eq!(resumed.cleanup_state, before.cleanup_state);
    assert!(resumed.next_attempt_at.unwrap() <= Utc::now());
    let mut expected = serde_json::to_value(&machine).unwrap();
    let mut actual = serde_json::to_value(f.current().await).unwrap();
    for key in ["revision", "updated_at"] {
        expected.as_object_mut().unwrap().remove(key);
        actual.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(
        actual, expected,
        "manual retry only changes scheduling and the revision fence"
    );
    assert_eq!(f.current().await.revision, machine.revision + 1);
    f.slot_retained().await;
    assert!(f.step().await.is_err());
    let replay = retry::retry(
        &f.db,
        &machine.owner_id,
        op_id,
        machine.revision,
        15_639,
        "manual-delete",
    )
    .await
    .unwrap();
    assert_eq!(replay.retry_count, 15_640);
    assert_eq!(
        retry::attempts(&replay),
        1,
        "lost response replay cannot renew the budget again"
    );
    assert!(
        retry::retry(
            &f.db,
            &machine.owner_id,
            op_id,
            machine.revision + 1,
            15_639,
            "manual-delete"
        )
        .await
        .is_err()
    );
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), count);
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"remote_machine_delete_retried"})
            .await
            .unwrap(),
        1
    );
    f.slot_retained().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn delete_retry_limit_concurrent_entries_reserve_one_attempt_and_expired_claim_recovers() {
    use super::super::remote_machine_delete_retry as retry;
    let f = recovery_fixture("delete_retry_claim", true).await;
    let machine = f.current().await;
    let (a, b) = tokio::join!(retry::claim(&f.db, &machine), retry::claim(&f.db, &machine));
    let claims = [a.unwrap(), b.unwrap()];
    assert_eq!(claims.iter().filter(|c| c.is_some()).count(), 1);
    let op_id = machine.active_operation_id.as_deref().unwrap();
    let op = operations::get_operation(&f.db, &machine.owner_id, op_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(op.retry_count, 1);
    f.due().await;
    assert!(advance(&f.manager, &machine).await.is_ok());
    assert!(f.cluster.lock().unwrap().requests.is_empty());
    let collection =
        f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME);
    collection.update_one(doc! {"_id":op_id}, doc! {"$set":{
        "delete_retry.claim_until":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))
    }}).await.unwrap();
    let successor = retry::claim(&f.db, &machine).await.unwrap().unwrap();
    let old_claim = claims.iter().flatten().next().unwrap();
    retry::finish(&f.db, op_id, old_claim).await.unwrap();
    let claimed = operations::get_operation(&f.db, &machine.owner_id, op_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.retry_count, 2);
    assert_eq!(
        claimed.delete_retry.unwrap().claim_id.as_deref(),
        Some(successor.as_str())
    );
    retry::finish(&f.db, op_id, &successor).await.unwrap();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn delete_retry_limit_manual_reset_rejects_active_claim_and_wrong_lifecycle() {
    use super::super::remote_machine_delete_retry as retry;
    let f = recovery_fixture("delete_retry_authority", true).await;
    let machine = f.current().await;
    let op_id = machine.active_operation_id.as_deref().unwrap();
    let collection =
        f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME);
    for change in [
        doc! {"retry_count":10_i64,"delete_retry":{"attempts":10_i64,"claim_id":"active",
        "claim_until":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(1))}},
        doc! {"delete_retry":bson::Bson::Null,"action":"pause"},
        doc! {"action":"delete","state":"succeeded","terminal_at":bson::DateTime::now()},
    ] {
        collection
            .update_one(doc! {"_id":op_id}, doc! {"$set":change})
            .await
            .unwrap();
        assert!(
            retry::retry(
                &f.db,
                &machine.owner_id,
                op_id,
                machine.revision,
                10,
                "no-reset"
            )
            .await
            .is_err()
        );
    }
    collection
        .update_one(
            doc! {"_id":op_id},
            doc! {"$set":{"state":"reconciling","terminal_at":bson::Bson::Null}},
        )
        .await
        .unwrap();
    f.db.collection::<Document>(MACHINE_COLLECTION)
        .update_one(
            doc! {"_id":&machine.node_id},
            doc! {"$set":{"writer_admitted":true}},
        )
        .await
        .unwrap();
    assert!(
        retry::retry(
            &f.db,
            &machine.owner_id,
            op_id,
            machine.revision,
            10,
            "no-reset"
        )
        .await
        .is_err()
    );
    assert_eq!(f.current().await.revision, machine.revision);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn replacement_or_mutated_original_pv_never_reuses_the_first_volume_identity() {
    for conflict in ["pv_uid", "claim_uid", "csi_handle", "reclaim_policy"] {
        let f = recovery_fixture("delete_original_pv_conflict", true).await;
        f.set_pv("Delete");
        assert!(f.step().await.is_err());
        let original = f.current().await;
        let identity = original.delete_recovery.as_ref().unwrap().volume.clone();
        {
            let mut c = f.cluster.lock().unwrap();
            let pv = c.pv.as_mut().unwrap();
            match conflict {
                "pv_uid" => pv["metadata"]["uid"] = json!(Uuid::new_v4().to_string()),
                "claim_uid" => pv["spec"]["claimRef"]["uid"] = json!(Uuid::new_v4().to_string()),
                "csi_handle" => pv["spec"]["csi"]["volumeHandle"] = json!(EXTRA_DISK_HANDLE),
                _ => pv["spec"]["persistentVolumeReclaimPolicy"] = json!("Retain"),
            }
        }
        assert!(f.step().await.is_err(), "{conflict}");
        assert_eq!(
            f.current()
                .await
                .delete_recovery
                .as_ref()
                .unwrap()
                .volume
                .as_ref(),
            identity.as_ref(),
            "{conflict}"
        );
        assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1, "{conflict}");
        f.slot_retained().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn retained_stop_never_overrides_a_present_sandbox_with_unconfirmed_current_suspension() {
    for defect in ["missing", "false", "stale_generation"] {
        let f = recovery_fixture("delete_retained_stop_current_condition", true).await;
        assert!(f.step().await.is_err());
        let first = f.current().await.delete_recovery.unwrap().stop.unwrap();
        {
            let mut c = f.cluster.lock().unwrap();
            let sandbox = c.sandbox.as_mut().unwrap();
            match defect {
                "missing" => sandbox["status"]["conditions"] = json!([]),
                "false" => sandbox["status"]["conditions"][0]["status"] = json!("False"),
                _ => sandbox["status"]["conditions"][0]["observedGeneration"] = json!(1),
            }
        }
        assert!(f.step().await.is_err(), "{defect}");
        let current = f.current().await;
        let progress = current.delete_recovery.as_ref().unwrap();
        assert_eq!(progress.stop.as_ref(), Some(&first));
        assert_eq!(
            progress.blocker.as_deref(),
            Some("delete_current_suspension_unconfirmed")
        );
        assert!(!progress.resources_complete);
        assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
        f.slot_retained().await;
        f.db.drop().await.unwrap();
    }
}

struct FinalizedResources {
    sandbox: Value,
    pvc: Value,
    pod: Value,
}

struct FinalizedInvariants {
    machine: ManagedMachine,
    operation: Document,
    slot: Document,
    node: Document,
    operation_count: u64,
    outbox_count: u64,
    runtime_count: u64,
    ledger_count: u64,
}

impl FinalizedInvariants {
    async fn capture(f: &RecoveryFixture) -> Self {
        let machine = f.current().await;
        Self {
            operation: f
                .db
                .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .find_one(doc! {"_id": &machine.active_operation_id})
                .await
                .unwrap()
                .unwrap(),
            slot: f
                .db
                .collection::<Document>(lifecycle::FREE_SLOTS)
                .find_one(doc! {"_id": &machine.owner_id})
                .await
                .unwrap()
                .unwrap(),
            node: f
                .db
                .collection::<Document>(crate::models::node::COLLECTION_NAME)
                .find_one(doc! {"_id": &machine.node_id})
                .await
                .unwrap()
                .unwrap(),
            operation_count: f
                .db
                .collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            outbox_count: f
                .db
                .collection::<Document>(OUTBOX_COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            runtime_count: f
                .db
                .collection::<Document>(crate::models::machine_runtime_billing::RUN_COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            ledger_count: f
                .db
                .collection::<Document>("billing_ledger")
                .count_documents(doc! {})
                .await
                .unwrap(),
            machine,
        }
    }

    async fn assert_unchanged(&self, f: &RecoveryFixture) {
        use crate::models::remote_machine::{AuthorizationState, BillingState};
        let current = f.current().await;
        assert_eq!(current.resource_state, ResourceState::Deleted);
        assert_eq!(current.billing_state, BillingState::Closed);
        assert_eq!(current.authorization_state, AuthorizationState::Revoked);
        assert!(matches!(
            current.cleanup_state.as_str(),
            "pending" | "complete"
        ));
        assert!(!current.writer_admitted);
        assert_eq!(
            current.active_operation_id,
            self.machine.active_operation_id
        );
        assert_eq!(current.runtime_generation, self.machine.runtime_generation);
        assert_eq!(current.credential_epoch, self.machine.credential_epoch);
        assert!(current.writer_fence == self.machine.writer_fence);
        assert_eq!(
            super::basis_digest(&current),
            super::basis_digest(&self.machine)
        );
        assert_eq!(current.runtime_run_id, self.machine.runtime_run_id);
        assert_eq!(current.native_uncertainty, self.machine.native_uncertainty);
        assert_eq!(current.native_observation, self.machine.native_observation);
        assert_eq!(
            current.delete_recovery.as_ref().unwrap().evidence_id,
            self.machine.delete_recovery.as_ref().unwrap().evidence_id
        );
        assert_eq!(
            current.delete_recovery.as_ref().unwrap().stop,
            self.machine.delete_recovery.as_ref().unwrap().stop
        );
        assert!(!crate::services::remote_machine_native::predecessor_stopped(&current));
        assert!(!lifecycle::runtime_fence_allows_bootstrap(&current));
        let operation =
            f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .find_one(doc! {"_id": &current.active_operation_id})
                .await
                .unwrap()
                .unwrap();
        assert!(
            operation == self.operation,
            "continual cleanup must retain the original succeeded operation unchanged"
        );
        let slot =
            f.db.collection::<Document>(lifecycle::FREE_SLOTS)
                .find_one(doc! {"_id": &current.owner_id})
                .await
                .unwrap()
                .unwrap();
        assert!(
            slot == self.slot,
            "continual cleanup must not release or mutate the free slot again"
        );
        let node =
            f.db.collection::<Document>(crate::models::node::COLLECTION_NAME)
                .find_one(doc! {"_id": &current.node_id})
                .await
                .unwrap()
                .unwrap();
        assert!(
            node == self.node,
            "continual cleanup must not rotate credentials or change the disabled authority"
        );
        assert_eq!(
            f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            self.operation_count
        );
        assert_eq!(
            f.db.collection::<Document>(OUTBOX_COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            self.outbox_count
        );
        assert_eq!(
            f.db.collection::<Document>(crate::models::machine_runtime_billing::RUN_COLLECTION)
                .count_documents(doc! {})
                .await
                .unwrap(),
            self.runtime_count
        );
        assert_eq!(
            f.db.collection::<Document>("billing_ledger")
                .count_documents(doc! {})
                .await
                .unwrap(),
            self.ledger_count
        );
    }
}

async fn finalized_recovery_fixture(
    label: &str,
) -> (RecoveryFixture, FinalizedResources, FinalizedInvariants) {
    let f = recovery_fixture(label, true).await;
    let (sandbox, pvc) = {
        let cluster = f.cluster.lock().unwrap();
        (
            cluster.sandbox.clone().unwrap(),
            cluster.pvc.clone().unwrap(),
        )
    };
    let resources = FinalizedResources {
        sandbox,
        pvc,
        pod: f.pod(),
    };
    assert!(f.step().await.is_err());
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
    }
    f.step().await.unwrap();
    f.finish_assertions().await;
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.deletes.clear();
        cluster.patches.clear();
    }
    let invariants = FinalizedInvariants::capture(&f).await;
    (f, resources, invariants)
}

#[tokio::test]
async fn finalized_delete_continues_safe_exact_late_pvc_cleanup_under_original_authorization() {
    let (f, resources, invariants) = finalized_recovery_fixture("delete_final_late_pvc").await;
    let uid = resources.pvc["metadata"]["uid"].clone();
    let rv = resources.pvc["metadata"]["resourceVersion"].clone();
    f.cluster.lock().unwrap().pvc = Some(resources.pvc);
    assert!(advance(&f.manager, &f.current().await).await.is_err());
    {
        let cluster = f.cluster.lock().unwrap();
        assert_eq!(cluster.deletes.len(), 1);
        assert!(
            cluster.deletes[0]
                .0
                .ends_with("/persistentvolumeclaims/workspace")
        );
        assert_eq!(cluster.deletes[0].1["preconditions"]["uid"], uid);
        assert_eq!(cluster.deletes[0].1["preconditions"]["resourceVersion"], rv);
        assert!(cluster.patches.is_empty());
    }
    invariants.assert_unchanged(&f).await;
    f.cluster.lock().unwrap().pvc = None;
    advance(&f.manager, &f.current().await).await.unwrap();
    assert_eq!(f.current().await.cleanup_state, "complete");
    invariants.assert_unchanged(&f).await;
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn finalized_delete_blocks_conflicting_late_resources_without_mutation() {
    for conflict in [
        "pvc_uid",
        "pvc_owner",
        "sandbox_uid",
        "sandbox_owner",
        "pod_uid",
        "claim_user",
        "Retain",
        "attachment",
    ] {
        let (f, resources, invariants) =
            finalized_recovery_fixture("delete_final_late_conflict").await;
        match conflict {
            "pvc_uid" => {
                let mut pvc = resources.pvc;
                pvc["metadata"]["uid"] = json!(Uuid::new_v4().to_string());
                f.cluster.lock().unwrap().pvc = Some(pvc);
            }
            "pvc_owner" => {
                let mut pvc = resources.pvc;
                pvc["metadata"]["labels"]["nyxid.dev/owner-id"] = json!(Uuid::new_v4().to_string());
                f.cluster.lock().unwrap().pvc = Some(pvc);
            }
            "sandbox_uid" | "sandbox_owner" => {
                let mut sandbox = resources.sandbox;
                if conflict == "sandbox_uid" {
                    sandbox["metadata"]["uid"] = json!(Uuid::new_v4().to_string());
                } else {
                    sandbox["metadata"]["labels"]["nyxid.dev/owner-id"] =
                        json!(Uuid::new_v4().to_string());
                }
                f.cluster.lock().unwrap().sandbox = Some(sandbox);
            }
            "pod_uid" | "claim_user" => {
                let mut pod = resources.pod;
                pod["metadata"]["uid"] = json!(Uuid::new_v4().to_string());
                if conflict == "claim_user" {
                    pod["metadata"]["ownerReferences"] = json!([]);
                    pod["metadata"]["labels"] = json!({});
                    f.cluster.lock().unwrap().pvc = Some(resources.pvc);
                }
                f.cluster.lock().unwrap().pods.push(pod);
            }
            _ => {
                f.cluster.lock().unwrap().pvc = Some(resources.pvc);
                f.set_pv(if conflict == "Retain" {
                    "Retain"
                } else {
                    "Delete"
                });
                if conflict == "attachment" {
                    f.cluster.lock().unwrap().attachments.push(json!({"kind":"VolumeAttachment","apiVersion":"storage.k8s.io/v1","metadata":{"name":"late-attachment","uid":Uuid::new_v4().to_string(),"resourceVersion":"8"},"spec":{"attacher":CSI_DRIVER,"nodeName":"original-node","source":{"persistentVolumeName":"original-pv"}},"status":{"attached":true}}));
                }
            }
        }
        assert!(
            advance(&f.manager, &f.current().await).await.is_err(),
            "{conflict}"
        );
        assert!(f.cluster.lock().unwrap().deletes.is_empty(), "{conflict}");
        assert!(f.cluster.lock().unwrap().patches.is_empty(), "{conflict}");
        invariants.assert_unchanged(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn finalized_delete_records_late_original_pv_before_claim_cleanup_and_waits_for_csi_gc() {
    let (f, resources, invariants) = finalized_recovery_fixture("delete_final_late_pv").await;
    f.cluster.lock().unwrap().pvc = Some(resources.pvc);
    f.set_pv("Delete");
    let pv_uid = f.cluster.lock().unwrap().pv.as_ref().unwrap()["metadata"]["uid"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(advance(&f.manager, &f.current().await).await.is_err());
    let current = f.current().await;
    assert_eq!(current.native_pv_uid.as_deref(), Some(pv_uid.as_str()));
    let volume = current
        .delete_recovery
        .as_ref()
        .unwrap()
        .volume
        .as_ref()
        .unwrap();
    assert_eq!(volume.pv_uid, pv_uid);
    assert_eq!(volume.csi_handle.as_deref(), Some(DISK_HANDLE));
    assert_eq!(volume.reclaim_policy, "Delete");
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    assert!(
        f.cluster.lock().unwrap().deletes[0]
            .0
            .ends_with("/persistentvolumeclaims/workspace")
    );
    invariants.assert_unchanged(&f).await;
    f.cluster.lock().unwrap().pvc = None;
    assert!(advance(&f.manager, &f.current().await).await.is_err());
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 1);
    invariants.assert_unchanged(&f).await;
    f.cluster.lock().unwrap().pv = None;
    advance(&f.manager, &f.current().await).await.unwrap();
    assert_eq!(f.current().await.cleanup_state, "complete");
    invariants.assert_unchanged(&f).await;
    assert!(f.server.received_requests().await.unwrap().iter().all(|r| {
        r.method.as_str() != "DELETE" || !r.url.path().contains("/persistentvolumes/")
    }));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn finalized_delete_blocks_failed_or_incomplete_inventory_before_late_claim_cleanup() {
    for defect in ["forbidden", "unavailable", "incomplete_claims"] {
        let (f, resources, invariants) =
            finalized_recovery_fixture("delete_final_inventory_blocked").await;
        f.cluster.lock().unwrap().pvc = Some(resources.pvc);
        match defect {
            "forbidden" => f.cluster.lock().unwrap().pod_list_error = Some(403),
            "unavailable" => f.cluster.lock().unwrap().pod_list_error = Some(503),
            _ => {
                f.cluster.lock().unwrap().owned_pvc_pages = Some(vec![json!({
                    "kind":"PersistentVolumeClaimList","apiVersion":"v1",
                    "metadata":{"resourceVersion":"incomplete","remainingItemCount":1},"items":[]
                })])
            }
        }
        assert!(
            advance(&f.manager, &f.current().await).await.is_err(),
            "{defect}"
        );
        assert!(f.cluster.lock().unwrap().deletes.is_empty(), "{defect}");
        assert!(f.cluster.lock().unwrap().patches.is_empty(), "{defect}");
        invariants.assert_unchanged(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn finalized_delete_removes_only_exact_original_late_pod_before_its_claim() {
    let (f, resources, invariants) = finalized_recovery_fixture("delete_final_late_pod").await;
    let uid = resources.pod["metadata"]["uid"].clone();
    let rv = resources.pod["metadata"]["resourceVersion"].clone();
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.pods.push(resources.pod);
        cluster.pvc = Some(resources.pvc);
    }
    assert!(advance(&f.manager, &f.current().await).await.is_err());
    {
        let cluster = f.cluster.lock().unwrap();
        assert_eq!(cluster.deletes.len(), 1);
        assert!(cluster.deletes[0].0.ends_with("/pods/original-pod"));
        assert_eq!(cluster.deletes[0].1["preconditions"]["uid"], uid);
        assert_eq!(cluster.deletes[0].1["preconditions"]["resourceVersion"], rv);
    }
    invariants.assert_unchanged(&f).await;
    f.cluster.lock().unwrap().pods.clear();
    assert!(advance(&f.manager, &f.current().await).await.is_err());
    assert_eq!(f.cluster.lock().unwrap().deletes.len(), 2);
    assert!(
        f.cluster.lock().unwrap().deletes[1]
            .0
            .ends_with("/persistentvolumeclaims/workspace")
    );
    f.cluster.lock().unwrap().pvc = None;
    advance(&f.manager, &f.current().await).await.unwrap();
    assert_eq!(f.current().await.cleanup_state, "complete");
    invariants.assert_unchanged(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn finalized_delete_suspends_late_original_sandbox_then_waits_for_current_controller() {
    let (f, resources, invariants) = finalized_recovery_fixture("delete_final_late_sandbox").await;
    let mut sandbox = resources.sandbox;
    sandbox["spec"]["operatingMode"] = json!("Running");
    sandbox["status"]["conditions"][0]["status"] = json!("False");
    f.cluster.lock().unwrap().sandbox = Some(sandbox);
    advance(&f.manager, &f.current().await).await.unwrap();
    {
        let cluster = f.cluster.lock().unwrap();
        assert_eq!(cluster.patches.len(), 1);
        assert!(cluster.deletes.is_empty());
        let patch = cluster.patches[0].as_array().unwrap();
        assert!(
            patch
                .iter()
                .any(|op| op["op"] == "test" && op["path"] == "/metadata/uid")
        );
        assert!(
            patch
                .iter()
                .any(|op| op["op"] == "test" && op["path"] == "/metadata/resourceVersion")
        );
        assert!(patch.iter().any(|op| op["op"] == "add"
            && op["path"] == "/spec/operatingMode"
            && op["value"] == "Suspended"));
        assert_eq!(
            cluster.sandbox.as_ref().unwrap()["metadata"]["generation"],
            json!(3)
        );
    }
    invariants.assert_unchanged(&f).await;
    assert!(advance(&f.manager, &f.current().await).await.is_err());
    assert!(f.cluster.lock().unwrap().deletes.is_empty());
    invariants.assert_unchanged(&f).await;
    {
        let mut cluster = f.cluster.lock().unwrap();
        let sandbox = cluster.sandbox.as_mut().unwrap();
        sandbox["status"]["conditions"][0]["status"] = json!("True");
        sandbox["status"]["conditions"][0]["observedGeneration"] = json!(3);
        sandbox["metadata"]["resourceVersion"] = json!("23");
    }
    assert!(advance(&f.manager, &f.current().await).await.is_err());
    {
        let cluster = f.cluster.lock().unwrap();
        assert_eq!(cluster.deletes.len(), 1);
        assert!(cluster.deletes[0].0.contains("/sandboxes/"));
        assert_eq!(
            cluster.deletes[0].1["preconditions"]["resourceVersion"],
            json!("23")
        );
    }
    invariants.assert_unchanged(&f).await;
    f.cluster.lock().unwrap().sandbox = None;
    advance(&f.manager, &f.current().await).await.unwrap();
    assert_eq!(f.current().await.cleanup_state, "complete");
    invariants.assert_unchanged(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn finalized_delete_rejects_changed_original_authorization_before_remote_effects() {
    for changed in [
        "owner",
        "operation",
        "writer_admitted",
        "credential_epoch",
        "deletion_basis",
        "operation_terminal",
    ] {
        let (f, resources, _) = finalized_recovery_fixture("delete_final_authority_fenced").await;
        f.cluster.lock().unwrap().pvc = Some(resources.pvc);
        let before = f.server.received_requests().await.unwrap().len();
        if changed == "operation_terminal" {
            f.db.collection::<Document>(crate::models::remote_machine_operation::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &f.machine.active_operation_id},
                    doc! {"$set": {"state": "reconciling", "terminal_at": bson::Bson::Null}},
                )
                .await
                .unwrap();
        } else {
            let mutation = match changed {
                "owner" => doc! {"owner_id": Uuid::new_v4().to_string()},
                "operation" => doc! {"active_operation_id": Uuid::new_v4().to_string()},
                "writer_admitted" => doc! {"writer_admitted": true},
                "credential_epoch" => doc! {"credential_epoch": 999_i64},
                _ => doc! {"deletion_basis": bson::Bson::Null},
            };
            f.db.collection::<Document>(MACHINE_COLLECTION)
                .update_one(doc! {"_id": &f.machine.node_id}, doc! {"$set": mutation})
                .await
                .unwrap();
        }
        assert!(advance(&f.manager, &f.machine).await.is_err(), "{changed}");
        assert_eq!(
            f.server.received_requests().await.unwrap().len(),
            before,
            "{changed}"
        );
        assert!(f.cluster.lock().unwrap().deletes.is_empty(), "{changed}");
        assert!(f.cluster.lock().unwrap().patches.is_empty(), "{changed}");
        f.db.drop().await.unwrap();
    }
}

#[test]
fn workload_absent_evidence_requires_terminal_owned_work_without_fabricated_controller_coordinates()
{
    use crate::models::remote_machine::{DeleteStopEvidence, DeleteStopKind};
    let (mut machine, mut progress) = fixture();
    machine.runtime_generation = 1;
    machine.sandbox_uid = Some("original-sandbox".into());
    machine.pod_uid = Some("original-pod".into());
    progress.runtime_generation = machine.runtime_generation;
    progress.sandbox_uid = machine.sandbox_uid.clone();
    progress.pod_uid = machine.pod_uid.clone();
    progress.creation_terminal = true;
    progress.resources_complete = true;
    progress.storage = DeleteStorageDisposition::ControlPlaneReleased;
    progress.stop = Some(DeleteStopEvidence {
        kind: DeleteStopKind::ControlPlaneWorkloadAbsent,
        source_operation_id: Some(progress.operation_id.clone()),
        evidence_ref: Uuid::new_v4().to_string(),
        source: "sandbox-kubernetes-control-plane".into(),
        observed_at: super::now(),
        sandbox_generation: 0,
        sandbox_resource_version: String::new(),
    });
    assert!(stop_is_valid(&machine, &progress));
    assert!(finality_ready(&machine, &progress));
    assert!(!crate::services::remote_machine_native::predecessor_stopped(&machine));
    assert!(!lifecycle::runtime_fence_allows_bootstrap(&machine));
    for defect in [
        "owned_work_unresolved",
        "fake_generation",
        "fake_resource_version",
        "source_operation",
        "reference",
        "old_version",
    ] {
        let mut invalid = progress.clone();
        match defect {
            "owned_work_unresolved" => invalid.creation_terminal = false,
            "fake_generation" => invalid.stop.as_mut().unwrap().sandbox_generation = 2,
            "fake_resource_version" => {
                invalid.stop.as_mut().unwrap().sandbox_resource_version = "21".into()
            }
            "source_operation" => {
                invalid.stop.as_mut().unwrap().source_operation_id = Some("different-delete".into())
            }
            "reference" => invalid.stop.as_mut().unwrap().evidence_ref = "not-a-uuid".into(),
            _ => invalid.evidence_version = 0,
        }
        assert!(!stop_is_valid(&machine, &invalid), "{defect}");
        assert!(!finality_ready(&machine, &invalid), "{defect}");
    }
}

#[tokio::test]
async fn initially_absent_workload_keeps_query_failures_replacements_and_old_503_work_pending() {
    for defect in [
        "forbidden",
        "unavailable",
        "incomplete_claims",
        "pod_replacement",
        "old_503_create",
    ] {
        let f = recovery_fixture("delete_absent_incomplete_or_unknown", true).await;
        let mut replacement = f.pod();
        replacement["metadata"]["uid"] = json!(Uuid::new_v4().to_string());
        {
            let mut cluster = f.cluster.lock().unwrap();
            cluster.sandbox = None;
            cluster.pvc = None;
            match defect {
                "forbidden" => cluster.pod_list_error = Some(403),
                "unavailable" => cluster.pod_list_error = Some(503),
                "pod_replacement" => cluster.pods.push(replacement),
                "incomplete_claims" => {
                    cluster.owned_pvc_pages = Some(vec![json!({
                        "kind":"PersistentVolumeClaimList","apiVersion":"v1",
                        "metadata":{"resourceVersion":"incomplete","remainingItemCount":1},"items":[]
                    })])
                }
                _ => (),
            }
        }
        if defect == "old_503_create" {
            f.db.collection::<Document>(OUTBOX_COLLECTION).update_one(
                doc! {"machine_id": &f.machine.node_id, "kind": "create_sandbox"},
                doc! {"$set": {
                    "state":"retry", "attempts": 1_i64, "last_error":"sandbox API response 503",
                    "claimed_until": bson::DateTime::from_chrono(super::now() - chrono::Duration::days(30)),
                    "submission.completed_at": bson::Bson::Null,
                    "submission.result_sandbox_uid": bson::Bson::Null,
                    "submission.result_resource_version": bson::Bson::Null,
                    "submission.cancelled_before_dispatch": false,
                }},
            ).await.unwrap();
        }
        assert!(f.step().await.is_err(), "{defect}");
        let pending = f.current().await;
        assert!(pending.native_observation.is_none());
        assert!(
            pending
                .delete_recovery
                .as_ref()
                .is_none_or(|progress| progress.stop.is_none()),
            "{defect}"
        );
        assert!(f.cluster.lock().unwrap().deletes.is_empty(), "{defect}");
        assert!(f.cluster.lock().unwrap().patches.is_empty(), "{defect}");
        if defect == "old_503_create" {
            assert!(
                !operations::creation_responsibilities_terminal(&f.db, &pending)
                    .await
                    .unwrap()
            );
            assert_eq!(
                pending.delete_recovery.as_ref().unwrap().blocker.as_deref(),
                Some("creation_result_unknown")
            );
        }
        f.slot_retained().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn initially_absent_paid_unready_workload_closes_zero_usage_from_real_query_upper_bound() {
    use crate::models::{
        machine_runtime_billing::{MachineRuntimeRun, MachineRuntimeRunState, RUN_COLLECTION},
        remote_machine::DeleteStopKind,
    };
    let mut f = recovery_fixture("delete_paid_initially_absent", true).await;
    let run_id = Uuid::new_v4().to_string();
    insert_unready_runtime_run(&f, &run_id).await;
    f.db.collection::<Document>(MACHINE_COLLECTION).update_one(
        doc! {"_id": &f.machine.node_id},
        doc! {"$set": {"free_slot": false, "runtime_run_id": &run_id, "billing_state": "pending"}},
    ).await.unwrap();
    let config = crate::config::AppConfig {
        billing_enabled: true,
        lago_api_url: None,
        lago_api_key: None,
        ..crate::test_utils::test_app_config()
    };
    f.manager.billing = Some(Arc::new(crate::services::billing::BillingService::new(
        f.db.clone(),
        Arc::new(config),
    )));
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
    }
    let query_started = super::now();
    f.step().await.unwrap();
    f.finish_assertions().await;
    let deleted = f.current().await;
    let stop = deleted
        .delete_recovery
        .as_ref()
        .unwrap()
        .stop
        .as_ref()
        .unwrap();
    assert_eq!(stop.kind, DeleteStopKind::ControlPlaneWorkloadAbsent);
    assert!(stop.observed_at >= query_started);
    assert_eq!(stop.sandbox_generation, 0);
    assert!(stop.sandbox_resource_version.is_empty());
    assert!(deleted.native_observation.is_none());
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id": &run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.state, MachineRuntimeRunState::Closed);
    assert_eq!(run.stop_ns, stop.observed_at.timestamp_nanos_opt());
    assert_eq!(
        run.stop_evidence_id.as_deref(),
        Some(stop.evidence_ref.as_str())
    );
    assert!(run.stop_is_upper_bound);
    assert_eq!(run.cumulative_seconds, 0);
    assert_eq!(
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    advance(&f.manager, &deleted).await.unwrap();
    let replay =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id": &run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(replay.revision, run.revision);
    assert_eq!(replay.stop_ns, run.stop_ns);
    assert_eq!(replay.stop_evidence_id, run.stop_evidence_id);
    assert_eq!(replay.cumulative_seconds, run.cumulative_seconds);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn finalized_absence_stop_survives_new_unresolved_original_creation_work_and_recovers() {
    use crate::models::{
        remote_machine::DeleteStopKind, remote_machine_outbox::MachineSubmissionDispatch,
    };
    let f = recovery_fixture("delete_final_absence_new_creation_work", true).await;
    let original_sandbox = f.cluster.lock().unwrap().sandbox.clone().unwrap();
    {
        let mut cluster = f.cluster.lock().unwrap();
        cluster.sandbox = None;
        cluster.pvc = None;
    }
    f.step().await.unwrap();
    f.finish_assertions().await;
    let finalized = f.current().await;
    let original_progress = finalized.delete_recovery.as_ref().unwrap();
    let original_stop = original_progress.stop.clone().unwrap();
    assert_eq!(
        original_stop.kind,
        DeleteStopKind::ControlPlaneWorkloadAbsent
    );
    assert!(original_progress.creation_terminal && original_progress.resources_complete);
    let invariants = FinalizedInvariants::capture(&f).await;
    let effect =
        f.db.collection::<MachineEffectOutbox>(OUTBOX_COLLECTION)
            .find_one(doc! {"machine_id": &finalized.node_id, "kind": "create_sandbox"})
            .await
            .unwrap()
            .unwrap();
    let submission = effect.submission.as_ref().unwrap();
    let dispatch_id = Uuid::new_v4().to_string();
    // Recover a previously dispatched original request whose result was not
    // journaled. This retains the original request and full runtime tuple;
    // it grants no authority to submit new work after deletion.
    let unresolved = MachineSubmissionDispatch {
        id: dispatch_id.clone(),
        started_at: submission.started_at,
        completed_at: None,
        cancelled_before_dispatch: false,
        result_sandbox_uid: None,
        result_resource_version: None,
    };
    f.db.collection::<Document>(OUTBOX_COLLECTION)
        .update_one(
            doc! {"_id": &effect.id, "submission.request_id": &effect.id},
            doc! {"$push": {"submission.dispatches": bson::to_bson(&unresolved).unwrap()}},
        )
        .await
        .unwrap();
    assert!(
        !operations::creation_responsibilities_terminal(&f.db, &finalized)
            .await
            .unwrap()
    );
    for _ in 0..2 {
        assert!(advance(&f.manager, &f.current().await).await.is_err());
        let pending = f.current().await;
        let progress = pending.delete_recovery.as_ref().unwrap();
        assert_eq!(pending.cleanup_state, "pending");
        assert!(!progress.resources_complete);
        assert!(
            progress.creation_terminal,
            "the first complete-query authority must remain bound to its original terminal-work evidence"
        );
        assert_eq!(progress.stop.as_ref(), Some(&original_stop));
        assert_eq!(progress.evidence_id, original_progress.evidence_id);
        assert_eq!(progress.blocker.as_deref(), Some("creation_result_unknown"));
        let mut projected = pending.clone();
        projected.resource_state = ResourceState::Deleting;
        assert!(super::stop_is_valid(&projected, progress));
        invariants.assert_unchanged(&f).await;
        assert!(f.cluster.lock().unwrap().deletes.is_empty());
        assert!(f.cluster.lock().unwrap().patches.is_empty());
    }
    let original_result =
        nyxid_machine_updater::sandbox::SandboxObservation::from_object(&original_sandbox).unwrap();
    operations::finish_submission(&f.db, &effect.id, &dispatch_id, Some(&original_result))
        .await
        .unwrap();
    assert!(
        operations::creation_responsibilities_terminal(&f.db, &f.current().await)
            .await
            .unwrap()
    );
    advance(&f.manager, &f.current().await).await.unwrap();
    let recovered = f.current().await;
    let progress = recovered.delete_recovery.as_ref().unwrap();
    assert_eq!(recovered.cleanup_state, "complete");
    assert!(progress.creation_terminal && progress.resources_complete);
    assert!(progress.blocker.is_none());
    assert_eq!(progress.stop.as_ref(), Some(&original_stop));
    assert_eq!(progress.evidence_id, original_progress.evidence_id);
    invariants.assert_unchanged(&f).await;
    f.db.drop().await.unwrap();
}
