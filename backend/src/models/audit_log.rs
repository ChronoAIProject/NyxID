use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "audit_log";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditLog {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: Option<String>,
    /// Event type (e.g. "login", "register", "api_key_created")
    pub event_type: String,
    /// Additional event data as JSON
    pub event_data: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "audit_log");
    }

    #[test]
    fn bson_roundtrip() {
        let log = AuditLog {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: Some(uuid::Uuid::new_v4().to_string()),
            event_type: "login".to_string(),
            event_data: Some(serde_json::json!({"ip": "127.0.0.1"})),
            ip_address: Some("127.0.0.1".to_string()),
            user_agent: Some("Mozilla/5.0".to_string()),
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&log).expect("serialize");
        let restored: AuditLog = bson::from_document(doc).expect("deserialize");
        assert_eq!(log.id, restored.id);
        assert_eq!(log.event_type, restored.event_type);
    }

    #[test]
    fn bson_roundtrip_all_none() {
        let log = AuditLog {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: None,
            event_type: "system".to_string(),
            event_data: None,
            ip_address: None,
            user_agent: None,
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&log).expect("serialize");
        let restored: AuditLog = bson::from_document(doc).expect("deserialize");
        assert!(restored.user_id.is_none());
        assert!(restored.event_data.is_none());
    }
}
