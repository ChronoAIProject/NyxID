use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_manager_ingress_receipts";

/// Metadata only. A receipt retains its original audience across consent changes.
#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformManagerIngress {
    #[serde(rename = "_id")]
    pub id: String,
    pub claim_id: String,
    pub profile_id: String,
    pub manager_bot_id: String,
    pub telegram_bot_id: i64,
    pub manager_authority_epoch: i64,
    pub observation_id: String,
    pub sender_id: i64,
    pub chat_id: i64,
    pub update_id: i64,
    pub message_id: i64,
    pub original_private_command: bool,
    pub audience: IngressAudience,
    pub phase: IngressPhase,
    pub execution: Option<IngressExecution>,
    pub decision: Option<IngressDecision>,
    pub reply_id: Option<String>,
    /// Delivery owns retry after its metadata-only row is durable. Legacy
    /// receipts default to pending recovery; terminal tombstones are retained.
    #[serde(default)]
    pub delivery_recovery_queued: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupted_reason: Option<String>,
    pub effect_revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub admitted_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub start_before: DateTime<Utc>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub settled_at: Option<DateTime<Utc>>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IngressAudience {
    Control,
    Personal {
        user_id: String,
        binding_id: String,
        binding_generation: i64,
        personal_control_revision: i64,
    },
    Public {
        public_policy_revision: i64,
    },
    Fixed {
        reason: String,
    },
    PrivateChatHandoff,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressPhase {
    Admitted,
    Executing,
    Deciding,
    Settled,
    Interrupted,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressExecution {
    pub conversation_id: String,
    pub turn_id: String,
    pub public_session_id: Option<String>,
    pub payer_user_id: String,
    pub public_policy_digest: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressDecision {
    pub acknowledgement_id: String,
    pub conversation_id: String,
    pub requested_turn_id: String,
    pub allowed: bool,
    pub voice_request_id: Option<String>,
    pub continuation_receipt_id: Option<String>,
    pub trigger_run_id: Option<String>,
}
