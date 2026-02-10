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
