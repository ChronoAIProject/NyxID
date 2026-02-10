use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "oauth_clients";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OauthClient {
    #[serde(rename = "_id")]
    pub id: String,
    pub client_name: String,
    /// Hashed client secret (SHA-256)
    pub client_secret_hash: String,
    /// Allowed redirect URIs
    pub redirect_uris: Vec<String>,
    /// Space-separated allowed scopes
    pub allowed_scopes: String,
    /// "authorization_code", "client_credentials", etc.
    pub grant_types: String,
    /// "confidential" or "public"
    pub client_type: String,
    pub is_active: bool,
    pub created_by: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
