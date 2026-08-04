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
    /// How to inject the provider token: "bearer" | "header" | "query" | "path"
    pub injection_method: String,
    /// Header name, query param name, or path prefix (e.g., "Authorization",
    /// "X-API-Key", or "bot" for Telegram Bot API).
    pub injection_key: Option<String>,
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
        assert_eq!(COLLECTION_NAME, "service_provider_requirements");
    }

    #[test]
    fn bson_roundtrip() {
        let req = ServiceProviderRequirement {
            id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            provider_config_id: uuid::Uuid::new_v4().to_string(),
            required: true,
            scopes: Some(vec!["read".to_string(), "write".to_string()]),
            injection_method: "bearer".to_string(),
            injection_key: Some("Authorization".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&req).expect("serialize");
        let restored: ServiceProviderRequirement = bson::from_document(doc).expect("deserialize");
        assert_eq!(req.id, restored.id);
        assert!(restored.required);
        assert_eq!(restored.scopes.unwrap().len(), 2);
    }
}
