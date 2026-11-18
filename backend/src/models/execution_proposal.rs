//! Optional approval-request extension. It is a review and effect receipt, never authority.
use super::execution_permission::{ExecutionContract, TaskAdmission};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct ExecutionProposal {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v1: Option<Proposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v2: Option<ProposalV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v3: Option<Proposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v4: Option<Proposal>,
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
    /// Only the v3 assistant envelope can author a different conversation scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_scope: Option<super::execution_permission::ConversationScope>,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct ProposalV2 {
    #[serde(default)]
    pub background_allowed: bool,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub waiter_until: Option<DateTime<Utc>>,
    pub source: ProposalSource,
    pub locator: String,
    pub source_digest: String,
    pub revision: i64,
    pub gap_key: String,
    pub effect_id: String,
    pub prepared_digest: String,
    pub authority_digest: String,
    pub key_id: String,
    pub conversation_id: Option<String>,
    pub source_turn_id: Option<String>,
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
    pub task: Option<TaskAdmission>,
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

/// Closed sources, with Machine authoring behind its own reader readiness.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposalSource {
    Machine {
        binding: Box<MachineSource>,
    },
    External {
        #[serde(default)]
        pool: Option<ProposalPool>,
    },
    Native {
        policy_id: String,
        revision: i64,
    },
    Watch {
        watch_id: String,
        operation: String,
        operation_id: String,
    },
    Voice {
        request_id: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProposalPool {
    pub id: String,
    pub owner_id: String,
    pub revision: i64,
}

/// Encrypted adapter input; HTTP contains no caller authentication material.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparedAdapter {
    Machine {
        binding: Box<MachineSource>,
        call_id: String,
        verified_digest: String,
    },
    Unsupported,
    Http,
    Pool {
        binding: ProposalPool,
    },
    Llm {
        provider: Option<String>,
        path: String,
        method: String,
        query: Option<String>,
        body: String,
        headers: Vec<(String, String)>,
    },
    Native {
        uri: String,
        method: String,
        body: String,
        content_type: Option<String>,
        mcp: bool,
    },
    Watch {
        operation: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RetainedEffect {
    pub effect: PreparedEffect,
    pub adapter: PreparedAdapter,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub partial: bool,
}

/// Immutable, server-verified job identity. No commands, output or credentials.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineSource {
    pub node_owner_id: String,
    pub agent_owner_id: String,
    pub job_id: String,
    pub node_id: String,
    pub runtime_id: String,
    pub runtime_generation: Option<u64>,
    pub actor_id: String,
    pub key_id: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub context_id: String,
    pub generation: u64,
    pub revision: i64,
    pub lease_id: String,
    pub declared_id: String,
}
