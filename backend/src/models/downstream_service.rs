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
    pub credential_encrypted: Vec<u8>,
    /// Original auth type as selected by the admin (e.g., "api_key", "oauth2", "oidc", "basic", "bearer").
    /// Preserves the user's intent, while `auth_method` is the resolved injection method.
    #[serde(default)]
    pub auth_type: Option<String>,
    /// URL to an OpenAPI / Swagger spec describing this service's API
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_spec_url: Option<String>,
    /// Associated OAuth client ID (set when auth_method is "oidc")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    pub is_active: bool,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
