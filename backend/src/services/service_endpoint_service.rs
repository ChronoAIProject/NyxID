use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::service_endpoint::{ServiceEndpoint, COLLECTION_NAME};

/// Input for creating or upserting a single endpoint.
pub struct EndpointInput {
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub response_description: Option<String>,
}

/// Fields that can be updated on an existing endpoint.
pub struct EndpointUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub parameters: Option<Option<serde_json::Value>>,
    pub request_body_schema: Option<Option<serde_json::Value>>,
    pub response_description: Option<Option<String>>,
    pub is_active: Option<bool>,
}

/// List all active endpoints for a given service.
pub async fn list_endpoints(
    db: &mongodb::Database,
    service_id: &str,
) -> AppResult<Vec<ServiceEndpoint>> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
    let cursor = coll
        .find(doc! { "service_id": service_id, "is_active": true })
        .await?;
    let endpoints: Vec<ServiceEndpoint> = cursor.try_collect().await?;
    Ok(endpoints)
}

/// Create a new endpoint for a service.
pub async fn create_endpoint(
    db: &mongodb::Database,
    service_id: &str,
    input: EndpointInput,
) -> AppResult<ServiceEndpoint> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
    let now = Utc::now();

    let endpoint = ServiceEndpoint {
        id: Uuid::new_v4().to_string(),
        service_id: service_id.to_string(),
        name: input.name,
        description: input.description,
        method: input.method.to_uppercase(),
        path: input.path,
        parameters: input.parameters,
        request_body_schema: input.request_body_schema,
        response_description: input.response_description,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    coll.insert_one(&endpoint).await?;
    Ok(endpoint)
}

/// Update an existing endpoint by ID.
pub async fn update_endpoint(
    db: &mongodb::Database,
    endpoint_id: &str,
    updates: EndpointUpdate,
) -> AppResult<()> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
    let now = Utc::now();

    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(now),
    };

    if let Some(name) = updates.name {
        set_doc.insert("name", name);
    }
    if let Some(description) = updates.description {
        match description {
            Some(d) => set_doc.insert("description", d),
            None => set_doc.insert("description", bson::Bson::Null),
        };
    }
    if let Some(method) = updates.method {
        set_doc.insert("method", method.to_uppercase());
    }
    if let Some(path) = updates.path {
        set_doc.insert("path", path);
    }
    if let Some(parameters) = updates.parameters {
        match parameters {
            Some(p) => {
                let bson_val = bson::to_bson(&p)
                    .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
                set_doc.insert("parameters", bson_val);
            }
            None => {
                set_doc.insert("parameters", bson::Bson::Null);
            }
        };
    }
    if let Some(request_body_schema) = updates.request_body_schema {
        match request_body_schema {
            Some(s) => {
                let bson_val = bson::to_bson(&s)
                    .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
                set_doc.insert("request_body_schema", bson_val);
            }
            None => {
                set_doc.insert("request_body_schema", bson::Bson::Null);
            }
        };
    }
    if let Some(response_description) = updates.response_description {
        match response_description {
            Some(d) => set_doc.insert("response_description", d),
            None => set_doc.insert("response_description", bson::Bson::Null),
        };
    }
    if let Some(is_active) = updates.is_active {
        set_doc.insert("is_active", is_active);
    }

    let result = coll
        .update_one(doc! { "_id": endpoint_id }, doc! { "$set": set_doc })
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(format!(
            "Endpoint not found: {endpoint_id}"
        )));
    }

    Ok(())
}

/// Delete (hard-delete) an endpoint by ID.
pub async fn delete_endpoint(db: &mongodb::Database, endpoint_id: &str) -> AppResult<()> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);

    let result = coll.delete_one(doc! { "_id": endpoint_id }).await?;

    if result.deleted_count == 0 {
        return Err(AppError::NotFound(format!(
            "Endpoint not found: {endpoint_id}"
        )));
    }

    Ok(())
}

/// Bulk upsert endpoints for a service.
///
/// For each input, matches by (service_id, name). If a matching endpoint exists,
/// it is updated; otherwise a new one is created. Endpoints belonging to this
/// service that are NOT in the input list are soft-deleted (is_active = false).
pub async fn bulk_upsert_endpoints(
    db: &mongodb::Database,
    service_id: &str,
    inputs: Vec<EndpointInput>,
) -> AppResult<Vec<ServiceEndpoint>> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
    let now = Utc::now();

    let mut result_endpoints: Vec<ServiceEndpoint> = Vec::with_capacity(inputs.len());
    let mut upserted_names: Vec<String> = Vec::with_capacity(inputs.len());

    for input in inputs {
        upserted_names.push(input.name.clone());

        let existing = coll
            .find_one(doc! { "service_id": service_id, "name": &input.name })
            .await?;

        if let Some(existing) = existing {
            // Update existing endpoint
            let mut set_doc = doc! {
                "description": input.description.as_deref(),
                "method": input.method.to_uppercase(),
                "path": &input.path,
                "is_active": true,
                "updated_at": bson::DateTime::from_chrono(now),
            };

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

            if let Some(ref desc) = input.response_description {
                set_doc.insert("response_description", desc.as_str());
            } else {
                set_doc.insert("response_description", bson::Bson::Null);
            }

            coll.update_one(
                doc! { "_id": &existing.id },
                doc! { "$set": set_doc },
            )
            .await?;

            // Return the updated version
            let updated = ServiceEndpoint {
                id: existing.id,
                service_id: existing.service_id,
                name: input.name,
                description: input.description,
                method: input.method.to_uppercase(),
                path: input.path,
                parameters: input.parameters,
                request_body_schema: input.request_body_schema,
                response_description: input.response_description,
                is_active: true,
                created_at: existing.created_at,
                updated_at: now,
            };
            result_endpoints.push(updated);
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
                response_description: input.response_description,
                is_active: true,
                created_at: now,
                updated_at: now,
            };
            coll.insert_one(&endpoint).await?;
            result_endpoints.push(endpoint);
        }
    }

    // Soft-delete endpoints for this service that were not in the upsert list
    if !upserted_names.is_empty() {
        coll.update_many(
            doc! {
                "service_id": service_id,
                "name": { "$nin": &upserted_names },
                "is_active": true,
            },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;
    }

    Ok(result_endpoints)
}
