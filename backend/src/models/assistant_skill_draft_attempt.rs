use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_skill_draft_attempts";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantSkillDraftAttempt {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub rejections: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
