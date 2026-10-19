use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "machine_jobs";

/// Server-issued execution authority. No command, output, or gateway token.
#[derive(Clone, Deserialize, Serialize)]
pub struct MachineJob {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub node_id: String,
    #[serde(default)]
    pub runtime_id: String,
    /// Absent only on pre-migration Assistant jobs. External jobs never use
    /// the legacy chat fields as execution identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<MachineJobBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub conversation_id: String,
    /// Exact assistant turn that issued this job. Legacy rows have no turn
    /// authority and are never selected by transport-only cancellation.
    #[serde(default)]
    pub issuing_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
    pub state: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    /// Request attribution only; neither field proves process or runtime stop.
    #[serde(default)]
    pub cancellation_requested_by: Option<MachineCancellationRequester>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub cancellation_requested_at: Option<DateTime<Utc>>,
    /// Immutable issue-time service identities. Legacy jobs receive no services.
    #[serde(default)]
    pub services: Vec<DeclaredService>,
    #[serde(default)]
    pub report_to: Option<String>,
    #[serde(default)]
    pub report_provenance: u8,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub finished_at: Option<DateTime<Utc>>,
    /// User history is independent of the execution deadline. Raw output is
    /// never persisted in this collection.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub retain_until: Option<DateTime<Utc>>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MachineCancellationRequester {
    HumanOwner {
        user_id: String,
    },
    External {
        principal: super::machine_external_grant::ExternalPrincipalRef,
    },
    System {
        reason: MachineSystemCancellationReason,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MachineSystemCancellationReason {
    SourceRevoked,
}

impl std::fmt::Debug for MachineCancellationRequester {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MachineCancellationRequester { [REDACTED] }")
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MachineJobBinding {
    Assistant {
        version: u32,
        conversation_id: String,
        turn_id: String,
        api_key_id: String,
        agent_id: String,
    },
    External {
        version: u32,
        principal: super::machine_external_grant::ExternalPrincipalRef,
        grant_id: String,
        grant_revision: i64,
        assignment_revision: i64,
        protected_consent: Option<super::consent::ProtectedConsentRef>,
        api_key_state_version_at_approval: Option<i64>,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        authentication_expires_at: DateTime<Utc>,
        /// Issue-time full intersection. A later live grant can only narrow it.
        service_ceiling: Vec<String>,
        lease_id: String,
    },
}

impl std::fmt::Debug for MachineJobBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MachineJobBinding { [REDACTED] }")
    }
}

impl std::fmt::Debug for MachineJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MachineJob { [REDACTED] }")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeclaredService {
    pub id: String,
    pub slug: String,
}
