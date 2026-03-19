use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_endpoints";

fn default_request_body_required() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceEndpoint {
    #[serde(rename = "_id")]
    pub id: String,
    pub service_id: String,
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_content_type: Option<String>,
    #[serde(default = "default_request_body_required")]
    pub request_body_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_description: Option<String>,
    pub is_active: bool,
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
        assert_eq!(COLLECTION_NAME, "service_endpoints");
    }

    #[test]
    fn bson_roundtrip() {
        let endpoint = ServiceEndpoint {
            id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            name: "get_users".to_string(),
            description: Some("List users".to_string()),
            method: "GET".to_string(),
            path: "/users".to_string(),
            parameters: Some(serde_json::json!([{"name": "limit", "in": "query"}])),
            request_body_schema: None,
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
            response_description: Some("200 OK".to_string()),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&endpoint).expect("serialize");
        let restored: ServiceEndpoint = bson::from_document(doc).expect("deserialize");
        assert_eq!(endpoint.id, restored.id);
        assert_eq!(endpoint.method, restored.method);
        assert_eq!(endpoint.request_content_type, restored.request_content_type);
        assert_eq!(endpoint.request_body_required, restored.request_body_required);
    }
}
