use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "managed_machines";
pub const POLICY_VERSION: &str = "remote-machine-mvp";
pub const DATA_POLICY_VERSION: &str = "remote-machine-data";

pub fn default_name() -> String {
    "my-remote-machine".into()
}
pub fn default_capabilities() -> Vec<String> {
    vec!["shell".into(), "files".into()]
}
fn default_assignment_state() -> String {
    "none".into()
}

/// Owner-selected local capability ceiling; this is not an agent grant.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HostedMachineChoices {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_capabilities")]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub specialist_id: Option<String>,
}

/// The current full Machine profile. Values are persisted so deployment changes
/// cannot silently change an existing machine's allocation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MachineProfileSpec {
    pub profile_version: String,
    pub image_digest: String,
    #[serde(default)]
    pub runtime_class: Option<String>,
    pub cpu_millis: u32,
    pub memory_mib: u32,
    pub workspace_gib: u32,
    /// None means no fixed temporary allocation is declared, never no storage.
    #[serde(default)]
    pub ephemeral_gib: Option<u32>,
}

impl MachineProfileSpec {
    pub fn fixed(image_digest: impl Into<String>) -> Self {
        Self {
            profile_version: nyxid_machine::managed::PROFILE.into(),
            image_digest: image_digest.into(),
            runtime_class: None,
            cpu_millis: nyxid_machine::managed::PROFILE_CPU_MILLIS,
            memory_mib: nyxid_machine::managed::PROFILE_MEMORY_MIB,
            workspace_gib: nyxid_machine::managed::PROFILE_WORKSPACE_GIB,
            ephemeral_gib: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceState {
    Provisioning,
    Ready,
    Degraded,
    Stopping,
    Paused,
    Resuming,
    Deleting,
    Deleted,
    Reconciling,
    Failed,
}

impl ResourceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Stopping => "stopping",
            Self::Paused => "paused",
            Self::Resuming => "resuming",
            Self::Deleting => "deleting",
            Self::Deleted => "deleted",
            Self::Reconciling => "reconciling",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingState {
    Free,
    Pending,
    Ready,
    Suspended,
    Closed,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationState {
    Pending,
    Active,
    Revoked,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeEvidence {
    pub sandbox_uid: String,
    pub pod_uid: Option<String>,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
    pub writer_fence: String,
    pub ready: bool,
    pub physically_stopped: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub observed_at: DateTime<Utc>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub provider_evidence_ref: Option<String>,
}

/// Kubernetes control-plane facts, not physical stop/writer-exclusion proof.
/// Only the Manager records these after exact resource and operation checks.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct NativeRuntimeObservation {
    pub sandbox_uid: String,
    pub pod_uid: Option<String>,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
    pub writer_fence: String,
    pub operation_id: String,
    pub sandbox_generation: i64,
    pub sandbox_resource_version: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub observed_at: DateTime<Utc>,
    pub running: bool,
    pub suspended: bool,
    pub stop_confirmed: bool,
    #[serde(default, with = "bson_datetime::optional")]
    pub run_not_after: Option<DateTime<Utc>>,
    pub evidence_ref: String,
}

/// A requested deadline has no admission authority until the provider returns
/// a signed receipt for the exact current target. Keep the old installed
/// receipt during retry so a lost ACK cannot manufacture extra coverage.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderCoverage {
    pub requested: nyxid_machine::managed::DeadlineControl,
    pub installed: Option<nyxid_machine::managed::DeadlineReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineDeletionBasis {
    pub owner_id: String,
    pub source: MachineDeletionSource,
    pub policy_version: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub accepted_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MachineDeletionSource {
    AccountOwner,
    MachineOwner,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManagedMachine {
    /// Stable public Machine identity. This is also the existing Node `_id`.
    #[serde(rename = "_id")]
    pub node_id: String,
    pub owner_id: String,
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_capabilities")]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub specialist_id: Option<String>,
    #[serde(default = "default_assignment_state")]
    pub specialist_assignment_state: String,
    #[serde(default)]
    pub specialist_assignment_error: Option<String>,
    pub policy_version: String,
    pub data_policy_version: String,
    /// Explicit owner consent survives account removal. Account absence and
    /// administrative deletion are never authority to erase retained data.
    #[serde(default)]
    pub deletion_basis: Option<MachineDeletionBasis>,
    pub profile: MachineProfileSpec,
    pub sandbox_namespace: String,
    pub sandbox_name: String,
    #[serde(default)]
    pub sandbox_uid: Option<String>,
    #[serde(default)]
    pub pvc_name: Option<String>,
    #[serde(default)]
    pub pvc_uid: Option<String>,
    #[serde(default)]
    pub pod_uid: Option<String>,
    pub resource_state: ResourceState,
    pub billing_state: BillingState,
    pub authorization_state: AuthorizationState,
    #[serde(default)]
    pub writer_admitted: bool,
    pub revision: i64,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
    pub writer_fence: String,
    #[serde(default)]
    pub bootstrap_epoch: i64,
    #[serde(default)]
    pub bootstrap_delivery_id: Option<String>,
    #[serde(default)]
    pub bootstrap_digest: Option<String>,
    #[serde(default)]
    pub active_operation_id: Option<String>,
    pub free_slot: bool,
    /// The accepted continuous-run contract; never a fabricated service identity.
    #[serde(default)]
    pub runtime_run_id: Option<String>,
    /// Extended only in the transaction which reserves the next funding window.
    #[serde(default, with = "bson_datetime::optional")]
    pub funded_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub provider_coverage: Option<ProviderCoverage>,
    #[serde(default)]
    pub native_observation: Option<NativeRuntimeObservation>,
    /// Latched uncertainty requires operator investigation; absence alone never clears it.
    #[serde(default)]
    pub native_uncertainty: Option<String>,
    #[serde(default)]
    pub native_pod_name: Option<String>,
    #[serde(default)]
    pub native_node_name: Option<String>,
    #[serde(default)]
    pub native_node_uid: Option<String>,
    #[serde(default)]
    pub native_pv_name: Option<String>,
    #[serde(default)]
    pub native_pv_uid: Option<String>,
    #[serde(default)]
    pub native_pv_reclaim_policy: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub native_run_not_after: Option<DateTime<Utc>>,
    /// Set before the first external Running intent for this generation.
    #[serde(default, with = "bson_datetime::optional")]
    pub runtime_submission_started_at: Option<DateTime<Utc>>,
    pub cleanup_state: String,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub cleanup_evidence_digest: Option<String>,
    #[serde(default)]
    pub last_runtime_evidence: Option<RuntimeEvidence>,
    #[serde(default)]
    pub last_runtime_observation: Option<RuntimeEvidence>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub eligibility_checked_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SafeMachineProfileView {
    pub profile_version: String,
    pub cpu_millis: u32,
    pub memory_mib: u32,
    pub workspace_gib: u32,
    pub ephemeral_gib: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SafeManagedMachineView {
    pub node_id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub specialist_id: Option<String>,
    pub specialist_assignment_state: String,
    pub specialist_assignment_error: Option<String>,
    pub policy_version: String,
    pub data_policy_version: String,
    pub profile: SafeMachineProfileView,
    pub sandbox_namespace: String,
    pub sandbox_name: String,
    pub sandbox_uid: Option<String>,
    pub pvc_name: Option<String>,
    pub pvc_uid: Option<String>,
    pub pod_uid: Option<String>,
    pub resource_state: ResourceState,
    pub billing_state: BillingState,
    pub authorization_state: AuthorizationState,
    pub revision: i64,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
    pub active_operation_id: Option<String>,
    pub cleanup_state: String,
    pub stop_reason: Option<String>,
    pub free_slot: bool,
    pub runtime_run_id: Option<String>,
    pub funded_until: Option<DateTime<Utc>>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn current_profile_bson_preserves_unknown_temporary_capacity() {
        let current = MachineProfileSpec::fixed("image@sha256:abc");
        assert_eq!(current.profile_version, nyxid_machine::managed::PROFILE);
        assert_eq!(
            (
                current.cpu_millis,
                current.memory_mib,
                current.workspace_gib
            ),
            (2000, 2048, 20)
        );
        assert_eq!(current.ephemeral_gib, None);
        let mut document = bson::to_document(&current).unwrap();
        assert!(
            document
                .get("ephemeral_gib")
                .is_some_and(|value| value == &bson::Bson::Null)
        );
        document.remove("ephemeral_gib");
        assert_eq!(
            bson::from_document::<MachineProfileSpec>(document.clone())
                .unwrap()
                .ephemeral_gib,
            None
        );
        assert_eq!(
            serde_json::to_value(current).unwrap()["ephemeral_gib"],
            serde_json::Value::Null
        );
    }
}
