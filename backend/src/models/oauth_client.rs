use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "oauth_clients";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OauthClient {
    #[serde(rename = "_id")]
    pub id: String,
    pub client_name: String,
    /// Hashed client secret (SHA-256)
    #[serde(skip_serializing)]
    pub client_secret_hash: String,
    /// JSON array of allowed redirect URIs
    pub redirect_uris: serde_json::Value,
    /// Space-separated allowed scopes
    pub allowed_scopes: String,
    /// "authorization_code", "client_credentials", etc.
    pub grant_types: String,
    /// "confidential" or "public"
    pub client_type: String,
    pub is_active: bool,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
