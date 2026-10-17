use super::platform_entry_projection::{
    StoredEntryIdentity, StoredSenderIdentity, checked_identity_record, same_version,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_manager_deliveries";

checked_identity_record! {
/// Metadata only. Private bodies and command bearers are never queued.

#[derive(Clone, Serialize)]
pub struct PlatformManagerDelivery, RawPlatformManagerDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub dedupe_key: String,
    pub profile_id: String,
    pub manager_bot_id: String,
    #[serde(flatten)]
    pub entry: StoredEntryIdentity,
    pub manager_authority_epoch: i64,
    #[serde(flatten, with = "super::platform_entry_projection::ingress_sender")]
    pub sender: StoredSenderIdentity,
    pub ingress_id: Option<String>,
    pub ingress_claim_id: Option<String>,
    pub origin: Option<super::assistant_conversation::PlatformTurnOrigin>,
    /// The original delivery turn, which may precede this reply's execution.
    #[serde(default)]
    pub origin_turn_id: Option<String>,
    pub reference: DeliveryReference,
    pub state: DeliveryState,
    pub claim_id: Option<String>,
    pub telegram_message_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platform_message_ids: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
|record| same_version(&record.entry, &record.sender, None) && super::platform_entry_projection::entry_matches(&record.entry, &record.manager_bot_id)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliveryReference {
    Reply {
        conversation_id: String,
        turn_id: String,
        reply_id: String,
    },
    PublicReply {
        public_session_id: String,
        conversation_id: String,
        turn_id: String,
        reply_id: String,
        public_policy_revision: i64,
    },
    Fixed {
        reason: String,
    },
    Handoff,
    /// One ephemeral original-private command attempt. Never recover its body.
    Command {
        /// Legacy ephemeral command rows decode but cannot acquire a new send.
        #[serde(default)]
        effect: Option<super::platform_manager_ingress::ManagerCommandEffect>,
    },
    /// One ephemeral public-turn proof attempt. Never recover its bearer body.
    PublicSignup {
        public_session_id: String,
        conversation_id: String,
        turn_id: String,
        intent_id: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    Dispatching,
    Sent,
    Refused,
    Uncertain,
}
