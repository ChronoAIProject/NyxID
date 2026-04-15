use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "user_endpoints";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserEndpoint {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub label: String,
    /// Target URL (e.g., "https://api.openai.com/v1" or "http://localhost:18789")
    pub url: String,
    /// Optional: populated when auto-provisioned from catalog
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_service_id: Option<String>,
    /// Optional: user-provided OpenAPI spec URL for endpoint discovery. When
    /// set, agent-facing surfaces (MCP, `/endpoints/{id}/openapi-endpoints`)
    /// fetch and parse this spec so AI tools can call specific operations
    /// instead of only the generic proxy tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openapi_spec_url: Option<String>,
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
        assert_eq!(COLLECTION_NAME, "user_endpoints");
    }

    #[test]
    fn bson_roundtrip() {
        let ep = UserEndpoint {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            label: "OpenAI".to_string(),
            url: "https://api.openai.com/v1".to_string(),
            catalog_service_id: Some("llm-openai".to_string()),
            openapi_spec_url: Some("https://api.example.com/openapi.json".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&ep).expect("serialize");
        let restored: UserEndpoint = bson::from_document(doc).expect("deserialize");
        assert_eq!(ep.id, restored.id);
        assert_eq!(ep.url, restored.url);
        assert_eq!(ep.catalog_service_id, restored.catalog_service_id);
        assert_eq!(ep.openapi_spec_url, restored.openapi_spec_url);
    }

    #[test]
    fn bson_roundtrip_no_catalog() {
        let ep = UserEndpoint {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            label: "Custom".to_string(),
            url: "http://localhost:8080".to_string(),
            catalog_service_id: None,
            openapi_spec_url: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&ep).expect("serialize");
        assert!(doc.get("catalog_service_id").is_none());
        assert!(doc.get("openapi_spec_url").is_none());
        let restored: UserEndpoint = bson::from_document(doc).expect("deserialize");
        assert!(restored.catalog_service_id.is_none());
        assert!(restored.openapi_spec_url.is_none());
    }

    #[test]
    fn deserialize_legacy_document_without_openapi_field() {
        // Pre-existing Mongo docs won't have the new openapi_spec_url field;
        // they must still deserialize into the current struct shape.
        let doc = bson::doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "user_id": uuid::Uuid::new_v4().to_string(),
            "label": "Legacy",
            "url": "https://api.example.com",
            "created_at": bson::DateTime::from_chrono(Utc::now()),
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        };
        let ep: UserEndpoint = bson::from_document(doc).expect("deserialize legacy");
        assert!(ep.openapi_spec_url.is_none());
        assert!(ep.catalog_service_id.is_none());
    }
}
