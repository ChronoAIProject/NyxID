use super::platform_entry_projection::{
    StoredEntryIdentity, StoredEventIdentity, StoredSenderIdentity, checked_identity_record,
    same_version,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_manager_ingress_receipts";

checked_identity_record! {
/// Metadata only. A receipt retains its original audience across consent changes.
#[derive(Clone, Serialize)]
pub struct PlatformManagerIngress, RawPlatformManagerIngress {
    #[serde(rename = "_id")]
    pub id: String,
    pub claim_id: String,
    pub profile_id: String,
    pub manager_bot_id: String,
    #[serde(flatten)]
    pub entry: StoredEntryIdentity,
    pub manager_authority_epoch: i64,
    pub observation_id: String,
    #[serde(flatten, with = "super::platform_entry_projection::ingress_sender")]
    pub sender: StoredSenderIdentity,
    #[serde(flatten)]
    pub event: StoredEventIdentity,
    pub original_private_command: bool,
    pub audience: IngressAudience,
    pub phase: IngressPhase,
    pub execution: Option<IngressExecution>,
    pub decision: Option<IngressDecision>,
    pub reply_id: Option<String>,
    /// Committed control effect, never rendered text, codes or bearer material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_effect: Option<ManagerCommandEffect>,
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
|record| same_version(&record.entry, &record.sender, Some(&record.event)) && super::platform_entry_projection::entry_matches(&record.entry, &record.manager_bot_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatLoginStatus {
    LoggedOut,
    Pending,
    AwaitingConfirmation,
    Expired,
    LoggedIn,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManagerCommandEffect {
    Link {
        intent_id: String,
    },
    Confirmed {
        binding_id: String,
        binding_generation: i64,
    },
    LoggedOut,
    Status {
        state: ChatLoginStatus,
    },
    Help,
    ManagementUnavailable,
    Management {
        binding_id: String,
        binding_generation: i64,
        selection: super::platform_manager_management::ManagerManagementView,
        connect_link_id: Option<String>,
        #[serde(default)]
        selection_ref: Option<String>,
    },
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

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
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
