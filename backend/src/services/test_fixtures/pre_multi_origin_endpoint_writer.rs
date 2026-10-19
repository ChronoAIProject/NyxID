// Frozen from 5c0620ea for the rolling-writer compatibility test.
// Only imports and function visibility are adapted to embed the old implementation.
#![allow(dead_code)]
pub(super) mod model {
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

    fn default_operation_generation() -> i64 {
        // Generation predates rolling writers. A missing field means the trusted
        // producer row was created by that legacy writer; its first revision is 1.
        // Explicit zero/negative values remain invalid and fail closed.
        1
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
        /// Producer-owned revision of this operation contract. A field omitted by
        /// a rolling legacy writer is revision 1; explicit non-positive values are
        /// invalid. The startup backfill persists the same canonical default.
        #[serde(default = "default_operation_generation")]
        pub operation_generation: i64,
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
}
use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::services::content_type::normalize_content_type;
use model::{EndpointRisk, OperationResponseContract, ServiceEndpoint};

/// Input for creating or upserting a single endpoint.
#[derive(Clone)]
pub struct EndpointInput {
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub request_content_type: Option<String>,
    pub request_body_required: bool,
    pub response_description: Option<String>,
    pub response: OperationResponseContract,
    pub risk: Option<EndpointRisk>,
    pub supports_idempotency_key: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EndpointSyncActivation {
    /// Admin-initiated reconcile: re-adding an endpoint reactivates it.
    ForceActive,
    /// Background/startup sync: preserve an operator's explicit activation choice.
    PreserveExisting,
}

fn normalize_response(mut response: OperationResponseContract) -> OperationResponseContract {
    response.content_types = response
        .content_types
        .into_iter()
        .map(|content_type| normalize_content_type(&content_type))
        .collect();
    response.content_types.sort_unstable();
    response.content_types.dedup();
    response
}

fn ensure_writable_operation_generation(endpoint: &ServiceEndpoint) -> AppResult<()> {
    if endpoint.operation_generation <= 0 {
        return Err(AppError::Conflict(format!(
            "Endpoint {} has an invalid operation_generation",
            endpoint.id
        )));
    }
    Ok(())
}

fn writable_operation_generation_filter() -> bson::Document {
    doc! {
        "$or": [
            { "operation_generation": { "$exists": false } },
            { "operation_generation": { "$gt": 0 } },
        ]
    }
}

/// Build an aggregation update so a legacy missing generation advances from
/// its canonical value 1 to 2 atomically. `$literal` prevents producer values
/// such as strings beginning with `$` from being interpreted as expressions.
fn semantic_update_pipeline(set_doc: bson::Document) -> Vec<bson::Document> {
    let mut set_expressions = bson::Document::new();
    for (field, value) in set_doc {
        set_expressions.insert(field, doc! { "$literal": value });
    }
    set_expressions.insert(
        "operation_generation",
        doc! {
            "$add": [
                { "$ifNull": ["$operation_generation", 1_i64] },
                1_i64,
            ]
        },
    );
    vec![doc! { "$set": set_expressions }]
}

/// Create or update a single endpoint matched by (service_id, name).
pub(super) async fn upsert_one_endpoint(
    coll: &mongodb::Collection<ServiceEndpoint>,
    service_id: &str,
    input: EndpointInput,
    now: chrono::DateTime<Utc>,
    activation: EndpointSyncActivation,
) -> AppResult<ServiceEndpoint> {
    let existing = coll
        .find_one(doc! { "service_id": service_id, "name": &input.name })
        .await?;

    if let Some(existing) = existing {
        ensure_writable_operation_generation(&existing)?;
        let response = normalize_response(input.response.clone());
        let desired_is_active = match activation {
            EndpointSyncActivation::ForceActive => true,
            EndpointSyncActivation::PreserveExisting => existing.is_active,
        };
        let unchanged = existing.description == input.description
            && existing.method == input.method.to_uppercase()
            && existing.path == input.path
            && existing.parameters == input.parameters
            && existing.request_body_schema == input.request_body_schema
            && existing.request_content_type == input.request_content_type
            && existing.request_body_required == input.request_body_required
            && existing.response_description == input.response_description
            && existing.response == response
            && existing.risk == input.risk
            && existing.supports_idempotency_key == input.supports_idempotency_key
            && existing.is_active == desired_is_active;
        if unchanged {
            return Ok(existing);
        }
        // Update existing endpoint
        let mut set_doc = doc! {
            "description": input.description.as_deref(),
            "method": input.method.to_uppercase(),
            "path": &input.path,
            "updated_at": bson::DateTime::from_chrono(now),
        };
        if activation == EndpointSyncActivation::ForceActive {
            set_doc.insert("is_active", true);
        }

        if let Some(ref params) = input.parameters {
            let bson_val = bson::to_bson(params)
                .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
            set_doc.insert("parameters", bson_val);
        } else {
            set_doc.insert("parameters", bson::Bson::Null);
        }

        if let Some(ref schema) = input.request_body_schema {
            let bson_val = bson::to_bson(schema)
                .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
            set_doc.insert("request_body_schema", bson_val);
        } else {
            set_doc.insert("request_body_schema", bson::Bson::Null);
        }

        if let Some(ref content_type) = input.request_content_type {
            set_doc.insert("request_content_type", content_type.as_str());
        } else {
            set_doc.insert("request_content_type", bson::Bson::Null);
        }
        set_doc.insert("request_body_required", input.request_body_required);

        if let Some(ref desc) = input.response_description {
            set_doc.insert("response_description", desc.as_str());
        } else {
            set_doc.insert("response_description", bson::Bson::Null);
        }
        let response_bson = bson::to_bson(&response)
            .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
        set_doc.insert("response", response_bson);
        match input.risk {
            Some(risk) => {
                set_doc.insert(
                    "risk",
                    bson::to_bson(&risk).map_err(|error| {
                        AppError::Internal(format!("BSON serialization error: {error}"))
                    })?,
                );
            }
            None => {
                set_doc.insert("risk", bson::Bson::Null);
            }
        }
        set_doc.insert("supports_idempotency_key", input.supports_idempotency_key);

        let mut filter = doc! { "_id": &existing.id, "service_id": service_id };
        filter.extend(writable_operation_generation_filter());
        coll.find_one_and_update(filter, semantic_update_pipeline(set_doc))
            .return_document(mongodb::options::ReturnDocument::After)
            .await?
            .ok_or_else(|| {
                AppError::Conflict(format!(
                    "Endpoint {} was deleted or its operation_generation became invalid",
                    existing.id
                ))
            })
    } else {
        // Create new endpoint
        let endpoint = ServiceEndpoint {
            id: Uuid::new_v4().to_string(),
            service_id: service_id.to_string(),
            name: input.name,
            description: input.description,
            method: input.method.to_uppercase(),
            path: input.path,
            parameters: input.parameters,
            request_body_schema: input.request_body_schema,
            request_content_type: input.request_content_type,
            request_body_required: input.request_body_required,
            response_description: input.response_description,
            response: normalize_response(input.response),
            risk: input.risk,
            supports_idempotency_key: input.supports_idempotency_key,
            is_active: true,
            operation_generation: 1,
            created_at: now,
            updated_at: now,
        };
        coll.insert_one(&endpoint).await?;
        Ok(endpoint)
    }
}
