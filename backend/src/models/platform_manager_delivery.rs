use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_manager_deliveries";

/// Metadata only. Private bodies and command bearers are never queued.
#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformManagerDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub dedupe_key: String,
    pub profile_id: String,
    pub manager_bot_id: String,
    pub telegram_bot_id: i64,
    pub manager_authority_epoch: i64,
    pub sender_id: i64,
    pub chat_id: i64,
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
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
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
    Command,
    /// One ephemeral public-turn proof attempt. Never recover its bearer body.
    PublicSignup {
        public_session_id: String,
        conversation_id: String,
        turn_id: String,
        intent_id: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    Dispatching,
    Sent,
    Refused,
    Uncertain,
}
