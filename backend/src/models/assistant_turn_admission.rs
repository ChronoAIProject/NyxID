use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_turn_admissions";

/// Durable receipt for one caller-supplied direct-turn idempotency key.
/// The request body is represented only by its canonical digest.
#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantTurnAdmission {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub client_request_id: String,
    /// Durable negative recovery fence. When true, this request ID can never
    /// later admit a turn; the execution fields remain empty sentinels.
    #[serde(default)]
    pub not_admitted: bool,
    pub payload_fingerprint: String,
    pub conversation_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub conversation_deleted: bool,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub conversation_deleted_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for AssistantTurnAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantTurnAdmission")
            .field("client_request_id", &self.client_request_id)
            .field("not_admitted", &self.not_admitted)
            .field("conversation_id", &self.conversation_id)
            .field("turn_id", &self.turn_id)
            .finish_non_exhaustive()
    }
}
