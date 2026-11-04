//! Internal deterministic model-Deny work. No model request or new consent.
use super::assistant_conversation::PlatformReplyRouting;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_model_denial_notices";
#[derive(Clone, Serialize, Deserialize)]
pub struct ModelDenialNotice {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub api_key_id: String,
    pub card_arguments_digest: String,
    pub source_input_id: Option<String>,
    pub source_input_binding: Option<String>,
    pub platform_reply: Option<PlatformReplyRouting>,
    /// pending, claimed, settled, or unsupported. Unsupported is not delivery.
    pub state: String,
    pub claimed_turn_id: Option<String>,
    pub reply_id: Option<String>,
    pub reason: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
