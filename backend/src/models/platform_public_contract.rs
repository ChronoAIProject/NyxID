//! Metadata-only public deployment evidence. Never contains bearer material.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagerBinding {
    pub profile_id: String,
    pub manager_bot_id: String,
    pub telegram_bot_id: i64,
    pub manager_authority_epoch: i64,
    pub public_policy_revision: i64,
    pub sponsor_owner_user_id: String,
    pub candidate_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOperation {
    pub service_id: String,
    pub operation: String,
    pub method: String,
    pub canonical_path: String,
    pub protocol: String,
    pub execution_authority_digest: String,
    pub endpoint_generation: i64,
    pub endpoint_set_digest: String,
    pub model_id: String,
    pub max_input_bytes: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_response_bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    pub max_duration_secs: u32,
    pub max_model_calls: u32,
    pub max_tool_calls: u32,
    pub max_continuations: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_input_bytes: u32,
    pub max_context_bytes: u32,
    pub max_output_bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicProfile {
    pub id: String,
    pub label: String,
    pub profile_digest: String,
    pub model_operations: Vec<ModelOperation>,
    pub limits: ExecutionLimits,
}

/// Exact semantic contract, including the immutable runtime binary. Assertions
/// are accepted only over the configured no-redirect catalog transport.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub contract: String,
    pub build: String,
    pub runtime_digest: String,
    pub model_contract: String,
    pub cleanup_contract: String,
    pub tool_contract: String,
    pub credential_bound: bool,
    pub callback_only: bool,
    pub bounded_compaction: bool,
    pub absolute_expiry: bool,
    pub cleanup_after_revocation: bool,
    pub native_tools_disabled: bool,
    pub provider_tools_disabled: bool,
    pub profiles: Vec<PublicProfile>,
    pub default_profile: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentEvidence {
    pub binding: ManagerBinding,
    pub manifest: RuntimeManifest,
    pub selected_profile: String,
    pub nyxagent_service_id: String,
    pub nyxagent_catalog_digest: String,
    pub policy_digest: String,
    pub evidence_digest: String,
    pub probe_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

pub const PROBES: &str = "platform_public_readiness_probes";
#[derive(Clone, Serialize, Deserialize)]
pub struct PublicProbe {
    #[serde(rename = "_id")]
    pub id: String,
    pub binding: ManagerBinding,
    #[serde(default)]
    pub activation_digest: Option<String>,
    pub selected_profile: String,
    pub manifest: RuntimeManifest,
    pub nyxagent_service_id: String,
    pub nyxagent_catalog_digest: String,
    pub operation: ModelOperation,
    pub request_digest: String,
    pub route_digest: String,
    pub pricing_digest: String,
    pub meter_row_ids: Vec<String>,
    pub bucket_id: String,
    pub state: String,
    #[serde(with = "super::credits::whole")]
    pub maximum: super::credits::Credits,
    pub exact_model_observed: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for PublicProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublicProbe { [REDACTED] }")
    }
}

/// Short-lived consent for a human-reviewed candidate that is not enabled yet.
/// Refreshes of an enabled policy use live controls instead.
pub const PROBE_CONSENTS: &str = "platform_public_probe_consents";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProbeConsent {
    #[serde(rename = "_id")]
    pub id: String,
    pub binding: ManagerBinding,
    pub actor_user_id: String,
    pub profile_generation: i64,
    pub controls_id: Option<String>,
    pub manager_authority_epoch: i64,
    pub personal_control_revision: i64,
    pub public_policy_revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

/// Separate from legacy unavailable-readiness rows for rolling startup safety.
pub const DEPLOYMENT_RECORDS: &str = "platform_public_deployment_proofs";
#[derive(Clone, Serialize, Deserialize)]
pub struct PublicDeploymentRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub profile_id: String,
    pub public_policy_revision: i64,
    pub candidate_digest: String,
    pub status: String,
    pub proof_gaps: Vec<String>,
    pub evidence: Option<DeploymentEvidence>,
    pub effect_revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub checked_at: DateTime<Utc>,
}

/// Human activation selects stable execution identity, independently of refresh.
pub const ACTIVATIONS: &str = "platform_public_deployment_activations";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicDeploymentActivation {
    #[serde(rename = "_id")]
    pub profile_id: String,
    pub binding: ManagerBinding,
    pub policy_digest: String,
    pub activated_by: String,
    pub effect_revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub activated_at: DateTime<Utc>,
}
