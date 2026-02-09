use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "api_keys";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub name: String,
    /// First 8 characters of the key, used for identification in the UI
    pub key_prefix: String,
    /// SHA-256 hash of the full API key
    #[serde(skip_serializing)]
    pub key_hash: String,
    pub scopes: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}
