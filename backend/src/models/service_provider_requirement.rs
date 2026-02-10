use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_provider_requirements";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceProviderRequirement {
    #[serde(rename = "_id")]
    pub id: String,
    pub service_id: String,
    pub provider_config_id: String,
    /// Whether this provider is required (vs optional) to use the service
    pub required: bool,
    /// Specific scopes this service needs from the provider
    pub scopes: Option<Vec<String>>,
    /// How to inject the provider token: "bearer" | "header" | "query"
    pub injection_method: String,
    /// Header name or query param name (e.g., "Authorization", "X-API-Key")
    pub injection_key: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
