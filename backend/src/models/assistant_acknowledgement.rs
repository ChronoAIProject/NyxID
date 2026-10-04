use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_acknowledgements";

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantAcknowledgement {
    #[serde(rename = "_id")]
    pub id: String,
    pub conversation_id: String,
    pub user_id: String,
    pub api_key_id: String,
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
    #[serde(default)]
    pub skill_selection: Option<super::assistant_agent::SkillSelection>,
    pub summary: String,
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
}

fn default_decider() -> String {
    "user".into()
}

impl std::fmt::Debug for AssistantAcknowledgement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantAcknowledgement { [REDACTED] }")
    }
}
