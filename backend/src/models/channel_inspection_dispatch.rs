use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_inspection_dispatches";

/// Metadata-only correlation record for one allowlisted synthetic inspection.
///
/// Destination identifiers stay internal and are never serialized directly by
/// an HTTP handler. Message content is fixed in code and is not persisted here.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChannelInspectionDispatch {
    #[serde(rename = "_id")]
    pub id: String,
    pub protocol_version: u8,
    pub case_id: String,
    pub run_id: String,
    pub event_id: String,
    pub platform: String,
    pub channel_bot_id: String,
    pub conversation_id: String,
    pub agent_api_key_id: String,
    pub inbound_message_id: String,
    pub state: String,
    #[serde(default)]
    pub failure_code: Option<String>,
    #[serde(default)]
    pub callback_http_status: Option<u16>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub purge_at: DateTime<Utc>,
}

impl std::fmt::Debug for ChannelInspectionDispatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelInspectionDispatch")
            .field("id", &self.id)
            .field("case_id", &self.case_id)
            .field("state", &self.state)
            .field("routing", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_internal_routing_identifiers() {
        let now = Utc::now();
        let row = ChannelInspectionDispatch {
            id: uuid::Uuid::new_v4().to_string(),
            protocol_version: 1,
            case_id: "telegram_private".into(),
            run_id: "run-1".into(),
            event_id: uuid::Uuid::new_v4().to_string(),
            platform: "telegram".into(),
            channel_bot_id: "secret-bot-id".into(),
            conversation_id: "secret-conversation-id".into(),
            agent_api_key_id: "secret-agent-id".into(),
            inbound_message_id: "secret-message-id".into(),
            state: "preparing".into(),
            failure_code: None,
            callback_http_status: None,
            created_at: now,
            updated_at: now,
            deadline_at: now,
            purge_at: now,
        };
        let rendered = format!("{row:?}");
        assert!(!rendered.contains("secret-bot-id"));
        assert!(!rendered.contains("secret-conversation-id"));
        assert!(!rendered.contains("secret-agent-id"));
        assert!(!rendered.contains("secret-message-id"));
    }
}
