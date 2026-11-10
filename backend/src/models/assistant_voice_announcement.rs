use mongodb::bson;
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_voice_announcements";

#[derive(Clone, Serialize, Deserialize)]
pub struct Announcement {
    #[serde(rename = "_id")]
    pub id: String,
    pub request_id: String,
    pub session_id: String,
    pub user_id: String,
    #[serde(default)]
    pub conversation_id: String,
    pub text: String,
    #[serde(default)]
    pub greeting: bool,
    pub state: String,
    pub not_after: bson::DateTime,
    pub expires_at: bson::DateTime,
    pub created_at: bson::DateTime,
}
