//! Runtime policy; this singleton shares the platform-settings collection.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SETTINGS_ID: &str = "assistant_upload_retention";
pub const TOMBSTONES: &str = "assistant_attachment_expiry";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub pending_hours: u32,
    pub image_days: u32,
    pub images_delete_after_turn: bool,
    pub document_days: u32,
    /// None keeps tool images for the conversation's lifetime.
    pub tool_image_days: Option<u32>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            pending_hours: 24,
            image_days: 30,
            images_delete_after_turn: false,
            document_days: 30,
            tool_image_days: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(rename = "_id")]
    pub id: String,
    pub revision: i64,
    pub policy: Option<Policy>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub updated_at: Option<DateTime<Utc>>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            id: SETTINGS_ID.into(),
            revision: 0,
            policy: None,
            updated_at: None,
        }
    }
}
