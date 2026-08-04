use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "user_service_connections";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserServiceConnection {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub service_id: String,
    /// Per-user encrypted credential for this service.
    /// For "connection" services: required, contains the user's own key/token/password.
    /// For "internal" services: None (master credential used).
    #[serde(with = "crate::models::bson_bytes::optional")]
    pub credential_encrypted: Option<Vec<u8>>,
    /// What kind of credential is stored (e.g., "api_key", "bearer", "basic").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<String>,
    /// Optional user-provided label for the credential (e.g., "Production Key").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_label: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "user_service_connections");
    }

    #[test]
    fn bson_roundtrip() {
        let conn = UserServiceConnection {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            credential_encrypted: Some(vec![10, 20, 30]),
            credential_type: Some("api_key".to_string()),
            credential_label: Some("My Key".to_string()),
            metadata: Some(serde_json::json!({"env": "production"})),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&conn).expect("serialize");
        let restored: UserServiceConnection = bson::from_document(doc).expect("deserialize");
        assert_eq!(conn.id, restored.id);
        assert_eq!(conn.credential_type, restored.credential_type);
    }
}
