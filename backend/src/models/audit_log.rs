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
    /// API key ID that made this request (None for non-API-key auth)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    /// Human-readable API key name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_name: Option<String>,
    /// Chain position, strictly increasing from 1. None on pre-chain legacy rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    /// Hex entry_hash of the previous chained entry; 64 zero chars for genesis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    /// Hex HMAC-SHA256 over the canonical encoded audit entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_hash: Option<String>,
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
            api_key_id: None,
            api_key_name: None,
            seq: Some(1),
            prev_hash: Some("0".repeat(64)),
            entry_hash: Some("a".repeat(64)),
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&log).expect("serialize");
        let restored: AuditLog = bson::from_document(doc).expect("deserialize");
        assert_eq!(log.id, restored.id);
        assert_eq!(log.event_type, restored.event_type);
        assert!(restored.api_key_id.is_none());
        assert_eq!(restored.seq, Some(1));
        assert_eq!(
            restored.prev_hash.as_deref(),
            Some("0000000000000000000000000000000000000000000000000000000000000000")
        );
        assert_eq!(
            restored.entry_hash.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
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
            api_key_id: None,
            api_key_name: None,
            seq: None,
            prev_hash: None,
            entry_hash: None,
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&log).expect("serialize");
        let restored: AuditLog = bson::from_document(doc).expect("deserialize");
        assert!(restored.user_id.is_none());
        assert!(restored.event_data.is_none());
        assert!(restored.api_key_id.is_none());
        assert!(restored.api_key_name.is_none());
        assert!(restored.seq.is_none());
        assert!(restored.prev_hash.is_none());
        assert!(restored.entry_hash.is_none());
    }

    #[test]
    fn bson_roundtrip_with_api_key() {
        let log = AuditLog {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: Some(uuid::Uuid::new_v4().to_string()),
            event_type: "proxy_request".to_string(),
            event_data: None,
            ip_address: None,
            user_agent: None,
            api_key_id: Some("key-id-123".to_string()),
            api_key_name: Some("coding-agent".to_string()),
            seq: Some(42),
            prev_hash: Some("1".repeat(64)),
            entry_hash: Some("2".repeat(64)),
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&log).expect("serialize");
        let restored: AuditLog = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.api_key_id.as_deref(), Some("key-id-123"));
        assert_eq!(restored.api_key_name.as_deref(), Some("coding-agent"));
        assert_eq!(restored.seq, Some(42));
    }

    #[test]
    fn bson_backward_compat_missing_api_key_fields() {
        let log = AuditLog {
            id: "test".to_string(),
            user_id: None,
            event_type: "old_event".to_string(),
            event_data: None,
            ip_address: None,
            user_agent: None,
            api_key_id: Some("should-be-skipped".to_string()),
            api_key_name: None,
            seq: Some(99),
            prev_hash: Some("3".repeat(64)),
            entry_hash: Some("4".repeat(64)),
            created_at: Utc::now(),
        };
        let mut doc = bson::to_document(&log).expect("serialize");
        // Simulate old document without api_key fields
        doc.remove("api_key_id");
        doc.remove("api_key_name");
        doc.remove("seq");
        doc.remove("prev_hash");
        doc.remove("entry_hash");
        let restored: AuditLog = bson::from_document(doc).expect("deserialize");
        assert!(restored.api_key_id.is_none());
        assert!(restored.api_key_name.is_none());
        assert!(restored.seq.is_none());
        assert!(restored.prev_hash.is_none());
        assert!(restored.entry_hash.is_none());
    }
}
