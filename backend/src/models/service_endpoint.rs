use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_endpoints";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndpointRisk {
    Read,
    Write,
}

fn default_request_body_required() -> bool {
    true
}

/// Normalized success-response metadata for an operation.
///
/// `binary_artifact` is intentionally tri-state. `None` means the source
/// contract did not provide enough information to classify the response, so
/// consumers can fail closed instead of treating an unknown response as text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OperationResponseContract {
    #[serde(default)]
    pub content_types: Vec<String>,
    #[serde(default)]
    pub binary_artifact: Option<bool>,
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
    #[serde(default)]
    pub response: OperationResponseContract,
    /// Explicitly published security classification. Missing on legacy or
    /// unclassified discovered rows, which makes them ineligible for durable
    /// grants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<EndpointRisk>,
    /// Whether the endpoint contract explicitly permits an Idempotency-Key.
    #[serde(default)]
    pub supports_idempotency_key: bool,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl ServiceEndpoint {
    pub fn has_request_body(&self) -> bool {
        self.request_body_schema.is_some() || self.request_content_type.is_some()
    }

    pub fn effective_request_body_required(&self) -> bool {
        self.request_body_required && self.has_request_body()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_endpoint() -> ServiceEndpoint {
        ServiceEndpoint {
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
            response: OperationResponseContract {
                content_types: vec!["application/json".to_string()],
                binary_artifact: Some(false),
            },
            risk: Some(EndpointRisk::Read),
            supports_idempotency_key: false,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "service_endpoints");
    }

    #[test]
    fn bson_roundtrip() {
        let endpoint = make_endpoint();
        let doc = bson::to_document(&endpoint).expect("serialize");
        let restored: ServiceEndpoint = bson::from_document(doc).expect("deserialize");
        assert_eq!(endpoint.id, restored.id);
        assert_eq!(endpoint.method, restored.method);
        assert_eq!(endpoint.request_content_type, restored.request_content_type);
        assert_eq!(
            endpoint.request_body_required,
            restored.request_body_required
        );
        assert_eq!(endpoint.response, restored.response);
    }

    #[test]
    fn legacy_document_defaults_response_contract_to_unknown() {
        let endpoint = make_endpoint();
        let mut doc = bson::to_document(&endpoint).expect("serialize");
        doc.remove("response");
        doc.remove("risk");
        doc.remove("supports_idempotency_key");

        let restored: ServiceEndpoint = bson::from_document(doc).expect("deserialize legacy row");
        assert!(restored.response.content_types.is_empty());
        assert_eq!(restored.response.binary_artifact, None);
        assert_eq!(restored.risk, None);
        assert!(!restored.supports_idempotency_key);
    }

    #[test]
    fn effective_request_body_required_false_without_body_metadata() {
        let mut endpoint = make_endpoint();
        endpoint.request_body_schema = None;
        endpoint.request_content_type = None;

        assert!(!endpoint.has_request_body());
        assert!(!endpoint.effective_request_body_required());
    }

    #[test]
    fn effective_request_body_required_true_with_body_metadata() {
        let endpoint = make_endpoint();

        assert!(endpoint.has_request_body());
        assert!(endpoint.effective_request_body_required());
    }
}
