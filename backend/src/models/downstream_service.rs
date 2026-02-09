use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "downstream_services";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownstreamService {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    /// Base URL of the downstream service (e.g. https://api.example.com)
    pub base_url: String,
    /// How credentials are injected: "header", "query", "body"
    pub auth_method: String,
    /// Header name or query param name for the credential
    pub auth_key_name: String,
    /// Encrypted master credential for this service
    #[serde(skip_serializing)]
    pub credential_encrypted: Vec<u8>,
    pub is_active: bool,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
