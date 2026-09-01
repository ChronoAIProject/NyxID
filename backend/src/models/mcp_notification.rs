use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "mcp_session_notifications";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpSessionNotification {
    #[serde(rename = "_id")]
    pub id: String,
    pub session_id: String,
    pub sequence: i64,
    pub payload: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_round_trip_preserves_payload_and_expiry() {
        let notification = McpSessionNotification {
            id: "session:1".to_string(),
            session_id: "session".to_string(),
            sequence: 1,
            payload: serde_json::json!({"jsonrpc": "2.0", "method": "notifications/test"}),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        };

        let document = bson::to_document(&notification).expect("serialize notification");
        let restored: McpSessionNotification =
            bson::from_document(document).expect("deserialize notification");

        assert_eq!(restored.id, notification.id);
        assert_eq!(restored.sequence, 1);
        assert_eq!(restored.payload, notification.payload);
    }
}
