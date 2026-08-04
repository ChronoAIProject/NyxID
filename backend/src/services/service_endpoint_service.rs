use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::service_endpoint::{COLLECTION_NAME, ServiceEndpoint};

/// Input for creating or upserting a single endpoint.
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
}

/// Fields that can be updated on an existing endpoint.
pub struct EndpointUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub parameters: Option<Option<serde_json::Value>>,
    pub request_body_schema: Option<Option<serde_json::Value>>,
    pub request_content_type: Option<Option<String>>,
    pub request_body_required: Option<bool>,
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
        request_content_type: input.request_content_type,
        request_body_required: input.request_body_required,
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
    if let Some(request_content_type) = updates.request_content_type {
        match request_content_type {
            Some(content_type) => {
                set_doc.insert("request_content_type", content_type);
            }
            None => {
                set_doc.insert("request_content_type", bson::Bson::Null);
            }
        };
    }
    if let Some(request_body_required) = updates.request_body_required {
        set_doc.insert("request_body_required", request_body_required);
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

            coll.update_one(doc! { "_id": &existing.id }, doc! { "$set": set_doc })
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
                request_content_type: input.request_content_type,
                request_body_required: input.request_body_required,
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
                request_content_type: input.request_content_type,
                request_body_required: input.request_body_required,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    fn make_input(name: &str, method: &str, path: &str) -> EndpointInput {
        EndpointInput {
            name: name.to_string(),
            description: Some(format!("{name} endpoint")),
            method: method.to_string(),
            path: path.to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: None,
        }
    }

    #[tokio::test]
    async fn test_create_endpoint() {
        let Some(db) = connect_test_database("svc_endpoint").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();

        let ep = create_endpoint(&db, &service_id, make_input("list_users", "get", "/users"))
            .await
            .unwrap();

        assert_eq!(ep.service_id, service_id);
        assert_eq!(ep.name, "list_users");
        assert_eq!(ep.method, "GET");
        assert_eq!(ep.path, "/users");
        assert!(ep.is_active);
    }

    #[tokio::test]
    async fn test_list_endpoints_filters_inactive() {
        let Some(db) = connect_test_database("svc_endpoint").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();

        create_endpoint(&db, &service_id, make_input("active_ep", "get", "/a"))
            .await
            .unwrap();
        let inactive = create_endpoint(&db, &service_id, make_input("inactive_ep", "post", "/b"))
            .await
            .unwrap();

        db.collection::<ServiceEndpoint>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &inactive.id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let endpoints = list_endpoints(&db, &service_id).await.unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].name, "active_ep");
    }

    #[tokio::test]
    async fn test_update_endpoint_partial() {
        let Some(db) = connect_test_database("svc_endpoint").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();

        let ep = create_endpoint(&db, &service_id, make_input("ep1", "get", "/old"))
            .await
            .unwrap();

        update_endpoint(
            &db,
            &ep.id,
            EndpointUpdate {
                name: Some("ep1_renamed".to_string()),
                description: None,
                method: Some("post".to_string()),
                path: Some("/new".to_string()),
                parameters: None,
                request_body_schema: None,
                request_content_type: None,
                request_body_required: None,
                response_description: None,
                is_active: None,
            },
        )
        .await
        .unwrap();

        let updated = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find_one(doc! { "_id": &ep.id })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.name, "ep1_renamed");
        assert_eq!(updated.method, "POST");
        assert_eq!(updated.path, "/new");
    }

    #[tokio::test]
    async fn test_update_endpoint_not_found() {
        let Some(db) = connect_test_database("svc_endpoint").await else {
            return;
        };

        let result = update_endpoint(
            &db,
            "nonexistent-id",
            EndpointUpdate {
                name: Some("x".to_string()),
                description: None,
                method: None,
                path: None,
                parameters: None,
                request_body_schema: None,
                request_content_type: None,
                request_body_required: None,
                response_description: None,
                is_active: None,
            },
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_endpoint() {
        let Some(db) = connect_test_database("svc_endpoint").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();

        let ep = create_endpoint(&db, &service_id, make_input("to_delete", "delete", "/x"))
            .await
            .unwrap();
        delete_endpoint(&db, &ep.id).await.unwrap();

        let count = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .count_documents(doc! { "_id": &ep.id })
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_delete_endpoint_not_found() {
        let Some(db) = connect_test_database("svc_endpoint").await else {
            return;
        };

        let result = delete_endpoint(&db, "nonexistent-id").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bulk_upsert_endpoints() {
        let Some(db) = connect_test_database("svc_endpoint").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();

        create_endpoint(&db, &service_id, make_input("ep_a", "get", "/a"))
            .await
            .unwrap();
        create_endpoint(&db, &service_id, make_input("ep_b", "get", "/b"))
            .await
            .unwrap();
        create_endpoint(&db, &service_id, make_input("ep_c", "get", "/c"))
            .await
            .unwrap();

        let inputs = vec![
            make_input("ep_a", "put", "/a_updated"),
            make_input("ep_d", "post", "/d_new"),
        ];
        let results = bulk_upsert_endpoints(&db, &service_id, inputs)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "ep_a");
        assert_eq!(results[0].method, "PUT");
        assert_eq!(results[0].path, "/a_updated");
        assert_eq!(results[1].name, "ep_d");

        let active = list_endpoints(&db, &service_id).await.unwrap();
        let active_names: Vec<&str> = active.iter().map(|e| e.name.as_str()).collect();
        assert!(active_names.contains(&"ep_a"));
        assert!(active_names.contains(&"ep_d"));
        assert!(!active_names.contains(&"ep_b"));
        assert!(!active_names.contains(&"ep_c"));
    }
}
