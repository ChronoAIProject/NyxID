use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantWireLog {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: Option<String>,
    pub payload: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl AssistantWireLog {
    pub const COLLECTION_NAME: &'static str = "assistant_wire_logs";
}
