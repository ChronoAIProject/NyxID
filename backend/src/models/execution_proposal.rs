//! Optional approval-request extension. It is a review and effect receipt, never authority.
use super::execution_permission::{ExecutionContract, TaskAdmission};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct ExecutionProposal {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v1: Option<Proposal>,
}
impl std::fmt::Debug for ExecutionProposal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionProposal")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub revision: i64,
    pub gap_key: String,
    pub effect_id: String,
    pub prepared_digest: String,
    pub authority_digest: String,
    pub key_id: String,
    pub conversation_id: String,
    pub source_turn_id: String,
    #[serde(default)]
    pub source_credential_id: Option<String>,
    #[serde(default)]
    pub endpoint_digest: Option<String>,
    #[serde(default)]
    pub selected_choice_digest: Option<String>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub notification_claimed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub wake_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub maintenance_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub delivery_abandoned: bool,
    #[serde(default)]
    pub independent_request_id: Option<String>,
    #[serde(default)]
    pub independent_card_id: Option<String>,
    #[serde(default)]
    pub delivered_turn_id: Option<String>,
    pub task: TaskAdmission,
    pub report_to: Option<String>,
    pub invocation_id: Option<String>,
    pub contract: ExecutionContract,
    pub choices: Vec<Choice>,
    pub review: Review,
    /// Envelope encryption with request-ID AAD. Never returned or logged.
    pub payload: Option<bson::Binary>,
    pub state: String,
    pub grant_id: Option<String>,
    pub selected_choice: Option<String>,
    pub claim_id: Option<String>,
    pub resume_turn_id: Option<String>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub claimed_at: Option<DateTime<Utc>>,
    pub result: Option<bson::Binary>,
    pub failure_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub digest: String,
    pub validity: super::execution_permission::Validity,
    pub label: String,
}

/// Safe structured owner review; raw HTTP inputs and credentials are never embedded.
#[derive(Clone, Serialize, Deserialize)]
pub struct Review {
    pub agent: String,
    pub connection: String,
    pub operation: String,
    pub audience: String,
    pub scope: String,
    #[serde(default)]
    pub inputs: String,
    #[serde(default)]
    pub existing_permission: String,
    #[serde(default)]
    pub added_permission: String,
    pub reusable_unavailable: Option<String>,
}

/// Encrypted retention payload, not a public request type.
#[derive(Clone, Serialize, Deserialize)]
pub struct PreparedEffect {
    #[serde(default)]
    pub generated_correlation: Option<String>,
    pub connection: String,
    pub catalog: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub body: String,
    pub headers: Vec<(String, String)>,
    pub endpoint: Option<String>,
}

impl std::fmt::Debug for Review {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Review { [REDACTED] }")
    }
}
