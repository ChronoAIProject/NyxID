use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "nyxbot_cma_sandboxes";

/// Local admission state for a CMA Sandbox reference. The occupied slot is
/// intentionally separate from this state: a failed or unknown remote
/// operation still occupies the owner's slot until the gateway confirms its
/// outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxAdmissionState {
    /// A local account slot has been reserved before a remote id is known.
    #[default]
    Reserved,
    /// A create request has been submitted and its result is not terminal.
    Creating,
    /// The reference is associated with a remote Sandbox.
    Associated,
    /// Retirement has been requested but is not confirmed yet.
    Retiring,
    /// The remote result is unknown; the slot must remain occupied.
    Unknown,
    /// Retirement was confirmed and the slot is released.
    Retired,
}

impl SandboxAdmissionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Creating => "creating",
            Self::Associated => "associated",
            Self::Retiring => "retiring",
            Self::Unknown => "unknown",
            Self::Retired => "retired",
        }
    }
}

/// Last observed remote lifecycle state. This is an observation, not local
/// authority to release an occupied account slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxObservationState {
    #[default]
    Unknown,
    Provisioning,
    Ready,
    Paused,
    Retiring,
    Retired,
    Failed,
}

impl SandboxObservationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Provisioning => "provisioning",
            Self::Ready => "ready",
            Self::Paused => "paused",
            Self::Retiring => "retiring",
            Self::Retired => "retired",
            Self::Failed => "failed",
        }
    }
}

/// Verified source of the resource capacity saved for this creation identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxCapacitySource {
    ProfileRevision,
    RuntimeAllocation,
}

/// NyxID's durable, owner-scoped reference to a CMA Sandbox.
///
/// The encrypted request body is recovery material only. It is never safe to
/// expose this row directly in an API response; service DTOs must project the
/// identifiers and bounded lifecycle facts they need.
#[derive(Clone, Serialize, Deserialize)]
pub struct NyxbotCmaSandbox {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub cma_service_id: String,
    pub label: String,
    #[serde(default)]
    pub cma_sandbox_id: Option<String>,
    pub occupies_account_slot: bool,
    #[serde(default)]
    pub admission_state: SandboxAdmissionState,
    #[serde(default)]
    pub observation_state: SandboxObservationState,
    pub creation_ref: String,
    pub generation: i64,
    /// Serializes Agent-reference registration with exact receipt association.
    #[serde(default)]
    pub agent_reference_version: i64,
    /// `nyxbot` is server-written creation provenance. Legacy discovered,
    /// claimed and conflict references are outside NyxID's managed allocation.
    pub source: String,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_observed: Option<DateTime<Utc>>,
    #[serde(default)]
    pub machine_included: Option<bool>,
    #[serde(default)]
    pub resource_state: Option<String>,
    #[serde(default)]
    pub cpu_millis: Option<u32>,
    #[serde(default)]
    pub memory_mib: Option<u32>,
    #[serde(default)]
    pub capacity_source: Option<SandboxCapacitySource>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_activity_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub retire_requested_at: Option<DateTime<Utc>>,
    /// The first confirmed retirement intent, scoped to this exact binding.
    #[serde(default)]
    pub retirement_idempotency_key: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub retired_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub request_hash: Option<String>,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub submission_state: String,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub request_ciphertext: Option<Vec<u8>>,
    #[serde(default)]
    pub request_context_version: Option<i32>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for NyxbotCmaSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NyxbotCmaSandbox")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("cma_service_id", &self.cma_service_id)
            .field("cma_sandbox_id", &self.cma_sandbox_id)
            .field("occupies_account_slot", &self.occupies_account_slot)
            .field("admission_state", &self.admission_state)
            .field("observation_state", &self.observation_state)
            .field("generation", &self.generation)
            .field("resource_state", &self.resource_state)
            .field("submission_state", &self.submission_state)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mongodb::bson::{self, Bson};

    #[test]
    fn bson_serialization_keeps_dates_and_encrypted_request_bytes() {
        let now = Utc::now();
        let row = NyxbotCmaSandbox {
            id: "sandbox-row".into(),
            user_id: "user".into(),
            cma_service_id: "service".into(),
            label: "Sandbox".into(),
            cma_sandbox_id: Some("remote-sandbox".into()),
            occupies_account_slot: true,
            admission_state: SandboxAdmissionState::Creating,
            observation_state: SandboxObservationState::Provisioning,
            creation_ref: "creation-ref".into(),
            generation: 1,
            agent_reference_version: 0,
            source: "nyxbot".into(),
            last_observed: Some(now),
            machine_included: Some(false),
            resource_state: Some("provisioning".into()),
            cpu_millis: Some(2_000),
            memory_mib: Some(2_048),
            capacity_source: None,
            last_activity_at: Some(now),
            retire_requested_at: Some(now),
            retirement_idempotency_key: Some("private-retirement-key".into()),
            retired_at: Some(now),
            idempotency_key: Some("idem".into()),
            request_hash: Some("hash".into()),
            operation_id: Some("operation".into()),
            submission_state: "submitted".into(),
            request_ciphertext: Some(vec![1, 2, 3]),
            request_context_version: Some(1),
            created_at: now,
            updated_at: now,
        };

        let document = bson::to_document(&row).expect("serialize sandbox");
        for field in [
            "created_at",
            "updated_at",
            "last_observed",
            "last_activity_at",
            "retire_requested_at",
            "retired_at",
        ] {
            assert!(
                matches!(document.get(field), Some(Bson::DateTime(_))),
                "{field} must be a BSON datetime"
            );
        }
        assert!(matches!(
            document.get("created_at"),
            Some(Bson::DateTime(_))
        ));
        assert!(matches!(
            document.get("last_observed"),
            Some(Bson::DateTime(_))
        ));
        assert!(matches!(
            document.get("last_activity_at"),
            Some(Bson::DateTime(_))
        ));
        assert!(matches!(
            document.get("request_ciphertext"),
            Some(Bson::Binary(_))
        ));
    }

    #[test]
    fn optional_dates_are_stored_as_null_and_encrypted_bytes_are_not_debugged() {
        let now = Utc::now();
        let row = NyxbotCmaSandbox {
            id: "sandbox-row".into(),
            user_id: "user".into(),
            cma_service_id: "service".into(),
            label: "Sandbox".into(),
            cma_sandbox_id: None,
            occupies_account_slot: true,
            admission_state: SandboxAdmissionState::Reserved,
            observation_state: SandboxObservationState::Unknown,
            creation_ref: "creation-ref".into(),
            generation: 1,
            agent_reference_version: 0,
            source: "nyxbot".into(),
            last_observed: None,
            machine_included: None,
            resource_state: None,
            cpu_millis: None,
            memory_mib: None,
            capacity_source: None,
            last_activity_at: None,
            retire_requested_at: None,
            retirement_idempotency_key: Some("private-retirement-key".into()),
            retired_at: None,
            idempotency_key: None,
            request_hash: None,
            operation_id: None,
            submission_state: "pending".into(),
            request_ciphertext: Some(vec![0xde, 0xad, 0xbe, 0xef]),
            request_context_version: None,
            created_at: now,
            updated_at: now,
        };

        let document = bson::to_document(&row).expect("serialize sandbox");
        assert_eq!(document.get("last_observed"), Some(&Bson::Null));
        assert_eq!(document.get("retired_at"), Some(&Bson::Null));
        let debug = format!("{row:?}");
        assert!(!debug.contains("deadbeef"));
        assert!(!debug.contains("request_ciphertext"));
        assert!(!debug.contains("private-retirement-key"));
        assert_eq!(
            document.get_str("retirement_idempotency_key").unwrap(),
            "private-retirement-key"
        );
    }
}
