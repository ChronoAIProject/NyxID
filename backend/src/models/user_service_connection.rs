use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "user_service_connections";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserServiceConnection {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub service_id: String,
    /// Per-user encrypted credential for this service (overrides service-level credential)
    #[serde(skip_serializing)]
    pub credential_encrypted: Option<Vec<u8>>,
    pub metadata: Option<serde_json::Value>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
