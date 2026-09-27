//! Typed activity metadata. Notification-only records live outside message history.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const NOTIFICATIONS_COLLECTION: &str = "channel_activity_notifications";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityMetadata {
    pub kind: String,
    pub provider_event_type: String,
    pub content_availability: String,
    pub reply_supported: bool,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub occurred_at: Option<DateTime<Utc>>,
}

/// Explicit receiver declaration, bound to this route and exact key revision.
#[derive(Clone, Serialize, Deserialize)]
pub struct ActivityCallback {
    pub version: u8,
    pub kinds: Vec<String>,
    pub agent_api_key_id: String,
    pub callback_url: String,
    pub key_state_version: i64,
    pub enabled: bool,
}

impl std::fmt::Debug for ActivityCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityCallback")
            .field("version", &self.version)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}
