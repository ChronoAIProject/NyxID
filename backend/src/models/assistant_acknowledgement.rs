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
    pub tool_name: Option<String>,
    pub arguments_digest: Option<String>,
    pub summary: String,
    pub status: String,
    /// Denial is sticky for the user turn that requested it. A new user turn
    /// may ask again; the model is explicitly instructed not to retry otherwise.
    pub requested_turn_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub decided_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for AssistantAcknowledgement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantAcknowledgement { [REDACTED] }")
    }
}
