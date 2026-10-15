use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_acknowledgements";

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantAcknowledgement {
    #[serde(default)]
    pub authored_skill: Option<AuthoredSkillReview>,
    #[serde(rename = "_id")]
    pub id: String,
    pub conversation_id: String,
    pub user_id: String,
    pub api_key_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester_agent_id: Option<String>,
    /// Why an action card was raised and how risky the action is, shown on
    /// the card. Absent on permission requests and rows from older replicas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<ActionConfirmation>,
    pub kind: String,
    pub service_id: Option<String>,
    pub service_slug: Option<String>,
    pub service_name: Option<String>,
    /// The service is a platform-provided catalog entry (DownstreamService ID)
    /// granted through the key's `allowed_platform_service_ids`.
    #[serde(default)]
    pub platform: bool,
    pub tool_name: Option<String>,
    pub arguments_digest: Option<String>,
    /// Operation permission proposal; IDs and templates only, never call arguments.
    #[serde(default)]
    pub operation_selection: Option<super::agent_operation_scope::OperationSelection>,
    /// Compiled contract the request was raised against. Kept beside the
    /// selection, whose strict schema older replicas still read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_contract_digest: Option<String>,
    #[serde(default)]
    pub skill_selection: Option<super::assistant_agent::SkillSelection>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub status: String,
    /// Denial is sticky for the user turn that requested it. A new user turn
    /// may ask again; the model is explicitly instructed not to retry otherwise.
    pub requested_turn_id: Option<String>,
    #[serde(default)]
    pub voice_request_id: Option<String>,
    #[serde(default)]
    pub continuation_receipt_id: Option<String>,
    #[serde(default)]
    pub trigger_run_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub decided_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    /// `user` (default) or `orchestrator`: a specialist's permission request is
    /// decided by the owner's NyxBot (or by the owner on the card).
    #[serde(default = "default_decider")]
    pub decider: String,
    /// Bounded excerpt of the text that started the requesting turn, so the
    /// orchestrator can judge the request against what was actually asked.
    #[serde(default)]
    pub request_excerpt: Option<String>,
    /// `user` or `orchestrator` once decided.
    #[serde(default)]
    pub decided_by: Option<String>,
    /// The orchestrator's bounded reason for its decision.
    #[serde(default)]
    pub reason: Option<String>,
    /// A graphical machine-context request. The bounded selection is applied
    /// only after the owner allows this action card.
    #[serde(default)]
    pub machine_context: Option<super::machine_access::HumanContextAction>,
}

/// The server's classification of an action card: its risk class, the
/// owner's access mode when the card was raised, and which policy asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionConfirmation {
    /// `read`, `change`, `sensitive` or `destructive`.
    pub risk: String,
    /// The effective access mode for the acting agent; absent when only a
    /// policy floor (webhook, machine or login confirmation) asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_mode: Option<super::assistant_settings::AccessMode>,
    /// `access_mode`, `webhook` or `policy`.
    pub reason: String,
}

fn default_decider() -> String {
    "user".into()
}

impl std::fmt::Debug for AssistantAcknowledgement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantAcknowledgement { [REDACTED] }")
    }
}

/// Metadata only; content is read from the encrypted review proposal by humans.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredSkillReview {
    pub agent_id: String,
    pub proposal_id: String,
    pub revision: i64,
    pub skills_revision: i64,
}
