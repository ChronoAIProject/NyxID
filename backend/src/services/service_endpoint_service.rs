use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::service_endpoint::{
    COLLECTION_NAME, EndpointRisk, OperationResponseContract, ServiceEndpoint,
};
use crate::services::content_type::normalize_content_type;

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
    pub response: Option<OperationResponseContract>,
    pub risk: Option<Option<EndpointRisk>>,
    pub supports_idempotency_key: Option<bool>,
    pub is_active: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EndpointSyncActivation {
    /// Admin-initiated reconcile: re-adding an endpoint reactivates it.
    ForceActive,
    /// Background/startup sync: preserve an operator's explicit activation choice.
    PreserveExisting,
}

/// Validate a request content type value for storage on an endpoint.
pub fn validate_request_content_type(content_type: &str) -> AppResult<()> {
    if content_type.trim().is_empty() {
        return Err(AppError::ValidationError(
            "request_content_type must not be empty".to_string(),
        ));
    }

    reqwest::header::HeaderValue::from_str(content_type).map_err(|_| {
        AppError::ValidationError("request_content_type must be a valid HTTP content type".into())
    })?;

    Ok(())
}

/// Validate the response contract's media types for storage on an endpoint.
pub fn validate_response_contract(response: &OperationResponseContract) -> AppResult<()> {
    for content_type in &response.content_types {
        if content_type.trim().is_empty()
            || reqwest::header::HeaderValue::from_str(content_type).is_err()
        {
            return Err(AppError::ValidationError(
                "response.content_types must contain valid HTTP content types".to_string(),
            ));
        }
    }
    Ok(())
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

/// Update an existing endpoint by ID.
pub async fn update_endpoint(
    db: &mongodb::Database,
    service_id: &str,
    endpoint_id: &str,
    updates: EndpointUpdate,
) -> AppResult<()> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
    let now = Utc::now();
    let existing = coll
        .find_one(doc! { "_id": endpoint_id, "service_id": service_id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Endpoint not found: {endpoint_id}")))?;
    ensure_writable_operation_generation(&existing)?;
    let mut set_doc = bson::Document::new();

    if let Some(name) = updates.name
        && existing.name != name
    {
        set_doc.insert("name", name);
    }
    if let Some(description) = updates.description
        && existing.description != description
    {
        match description {
            Some(d) => set_doc.insert("description", d),
            None => set_doc.insert("description", bson::Bson::Null),
        };
    }
    if let Some(method) = updates.method {
        let method = method.to_uppercase();
        if existing.method != method {
            set_doc.insert("method", method);
        }
    }
    if let Some(path) = updates.path
        && existing.path != path
    {
        set_doc.insert("path", path);
    }
    if let Some(parameters) = updates.parameters
        && existing.parameters != parameters
    {
        match parameters {
            Some(p) => {
                let bson_val = bson::to_bson(&p)
                    .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
                set_doc.insert("parameters", bson_val);
            }
            None => {
                set_doc.insert("parameters", bson::Bson::Null);
            }
        }
    }
    if let Some(request_body_schema) = updates.request_body_schema
        && existing.request_body_schema != request_body_schema
    {
        match request_body_schema {
            Some(s) => {
                let bson_val = bson::to_bson(&s)
                    .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
                set_doc.insert("request_body_schema", bson_val);
            }
            None => {
                set_doc.insert("request_body_schema", bson::Bson::Null);
            }
        }
    }
    if let Some(request_content_type) = updates.request_content_type
        && existing.request_content_type != request_content_type
    {
        match request_content_type {
            Some(content_type) => {
                set_doc.insert("request_content_type", content_type);
            }
            None => {
                set_doc.insert("request_content_type", bson::Bson::Null);
            }
        }
    }
    if let Some(request_body_required) = updates.request_body_required
        && existing.request_body_required != request_body_required
    {
        set_doc.insert("request_body_required", request_body_required);
    }
    if let Some(response_description) = updates.response_description
        && existing.response_description != response_description
    {
        match response_description {
            Some(d) => set_doc.insert("response_description", d),
            None => set_doc.insert("response_description", bson::Bson::Null),
        };
    }
    if let Some(response) = updates.response {
        let response = normalize_response(response);
        if existing.response != response {
            let bson_val = bson::to_bson(&response)
                .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
            set_doc.insert("response", bson_val);
        }
    }
    if let Some(risk) = updates.risk
        && existing.risk != risk
    {
        match risk {
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
    }
    if let Some(supports_idempotency_key) = updates.supports_idempotency_key
        && existing.supports_idempotency_key != supports_idempotency_key
    {
        set_doc.insert("supports_idempotency_key", supports_idempotency_key);
    }
    if let Some(is_active) = updates.is_active
        && existing.is_active != is_active
    {
        set_doc.insert("is_active", is_active);
    }

    if set_doc.is_empty() {
        return Ok(());
    }
    set_doc.insert("updated_at", bson::DateTime::from_chrono(now));
    let mut filter = doc! { "_id": endpoint_id, "service_id": service_id };
    filter.extend(writable_operation_generation_filter());
    let result = coll
        .update_one(filter, semantic_update_pipeline(set_doc))
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::Conflict(format!(
            "Endpoint {endpoint_id} was deleted or its operation_generation became invalid"
        )));
    }

    Ok(())
}

/// Delete (hard-delete) an endpoint by ID.
pub async fn delete_endpoint(
    db: &mongodb::Database,
    service_id: &str,
    endpoint_id: &str,
) -> AppResult<()> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);

    let result = coll
        .delete_one(doc! { "_id": endpoint_id, "service_id": service_id })
        .await?;

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
        result_endpoints.push(
            upsert_one_endpoint(
                &coll,
                service_id,
                input,
                now,
                EndpointSyncActivation::ForceActive,
            )
            .await?,
        );
    }

    // Soft-delete endpoints for this service that were not in the authoritative
    // input. `$nin: []` intentionally matches every active endpoint, so an
    // empty discovered contract revokes all previously published operations.
    coll.update_many(
        doc! {
            "service_id": service_id,
            "name": { "$nin": &upserted_names },
            "is_active": true,
        },
        vec![doc! { "$set": {
            "is_active": false,
            "updated_at": bson::DateTime::from_chrono(now),
            "operation_generation": {
                "$switch": {
                    "branches": [
                        {
                            "case": { "$eq": [
                                { "$type": "$operation_generation" },
                                "missing",
                            ]},
                            "then": 2_i64,
                        },
                        {
                            "case": { "$and": [
                                { "$isNumber": "$operation_generation" },
                                { "$gt": ["$operation_generation", 0] },
                            ]},
                            "then": { "$add": ["$operation_generation", 1_i64] },
                        },
                    ],
                    "default": "$operation_generation",
                }
            },
        }}],
    )
    .await?;

    Ok(result_endpoints)
}

/// Additively upsert endpoints for a service.
///
/// Like `bulk_upsert_endpoints`, matches by (service_id, name) and creates or
/// updates each input -- but endpoints with other names are left untouched
/// (nothing is soft-deleted). Used by the seeded catalog spec sync so
/// admin-added endpoints on a system service survive restarts.
pub async fn upsert_endpoints_additive(
    db: &mongodb::Database,
    service_id: &str,
    inputs: Vec<EndpointInput>,
) -> AppResult<Vec<ServiceEndpoint>> {
    let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
    let now = Utc::now();

    let mut result_endpoints: Vec<ServiceEndpoint> = Vec::with_capacity(inputs.len());
    for input in inputs {
        result_endpoints.push(
            upsert_one_endpoint(
                &coll,
                service_id,
                input,
                now,
                EndpointSyncActivation::PreserveExisting,
            )
            .await?,
        );
    }
    Ok(result_endpoints)
}

/// Create or update a single endpoint matched by (service_id, name).
async fn upsert_one_endpoint(
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
            response: OperationResponseContract::default(),
            risk: None,
            supports_idempotency_key: false,
        }
    }

    fn empty_update() -> EndpointUpdate {
        EndpointUpdate {
            name: None,
            description: None,
            method: None,
            path: None,
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: None,
            response_description: None,
            response: None,
            risk: None,
            supports_idempotency_key: None,
            is_active: None,
        }
    }

    async fn load_endpoint(db: &mongodb::Database, endpoint_id: &str) -> ServiceEndpoint {
        db.collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find_one(doc! { "_id": endpoint_id })
            .await
            .unwrap()
            .unwrap()
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
        assert_eq!(ep.operation_generation, 1);
    }

    #[tokio::test]
    async fn operation_generation_tracks_only_semantic_endpoint_changes() {
        let Some(db) = connect_test_database("svc_endpoint_generation_lifecycle").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        let endpoint = create_endpoint(&db, &service_id, make_input("generation", "get", "/items"))
            .await
            .unwrap();
        assert_eq!(endpoint.operation_generation, 1);

        let mut no_op = empty_update();
        no_op.path = Some("/items".to_string());
        update_endpoint(&db, &service_id, &endpoint.id, no_op)
            .await
            .unwrap();
        assert_eq!(
            load_endpoint(&db, &endpoint.id).await.operation_generation,
            1
        );

        let mut semantic = empty_update();
        semantic.supports_idempotency_key = Some(true);
        update_endpoint(&db, &service_id, &endpoint.id, semantic)
            .await
            .unwrap();
        assert_eq!(
            load_endpoint(&db, &endpoint.id).await.operation_generation,
            2
        );

        let mut deactivate = empty_update();
        deactivate.is_active = Some(false);
        update_endpoint(&db, &service_id, &endpoint.id, deactivate)
            .await
            .unwrap();
        assert_eq!(
            load_endpoint(&db, &endpoint.id).await.operation_generation,
            3
        );

        let mut reactivate = empty_update();
        reactivate.is_active = Some(true);
        update_endpoint(&db, &service_id, &endpoint.id, reactivate)
            .await
            .unwrap();
        assert_eq!(
            load_endpoint(&db, &endpoint.id).await.operation_generation,
            4
        );

        let mut repeated_reactivation = empty_update();
        repeated_reactivation.is_active = Some(true);
        update_endpoint(&db, &service_id, &endpoint.id, repeated_reactivation)
            .await
            .unwrap();
        assert_eq!(
            load_endpoint(&db, &endpoint.id).await.operation_generation,
            4
        );
    }

    #[tokio::test]
    async fn legacy_missing_generation_advances_from_one_to_two_on_semantic_writes() {
        let Some(db) = connect_test_database("svc_endpoint_legacy_generation_write").await else {
            return;
        };
        let direct_service_id = Uuid::new_v4().to_string();
        let direct = create_endpoint(
            &db,
            &direct_service_id,
            make_input("direct_legacy", "get", "/direct"),
        )
        .await
        .unwrap();
        let coll = db.collection::<ServiceEndpoint>(COLLECTION_NAME);
        coll.update_one(
            doc! { "_id": &direct.id },
            doc! { "$unset": { "operation_generation": "" } },
        )
        .await
        .unwrap();

        let mut direct_update = empty_update();
        direct_update.description = Some(Some("$direct-literal".to_string()));
        update_endpoint(&db, &direct_service_id, &direct.id, direct_update)
            .await
            .unwrap();
        let direct = load_endpoint(&db, &direct.id).await;
        assert_eq!(direct.operation_generation, 2);
        assert_eq!(direct.description.as_deref(), Some("$direct-literal"));

        let bulk_service_id = Uuid::new_v4().to_string();
        let original = make_input("bulk_legacy", "get", "/bulk");
        let bulk = create_endpoint(&db, &bulk_service_id, original.clone())
            .await
            .unwrap();
        coll.update_one(
            doc! { "_id": &bulk.id },
            doc! { "$unset": { "operation_generation": "" } },
        )
        .await
        .unwrap();
        let mut changed = original;
        changed.description = Some("$bulk-literal".to_string());
        changed.path = "/bulk-v2".to_string();

        let updated = bulk_upsert_endpoints(&db, &bulk_service_id, vec![changed])
            .await
            .unwrap()
            .remove(0);
        assert_eq!(updated.operation_generation, 2);
        assert_eq!(updated.description.as_deref(), Some("$bulk-literal"));
        assert_eq!(updated.path, "/bulk-v2");
    }

    #[tokio::test]
    async fn explicit_invalid_generation_is_never_ratified_by_endpoint_writes() {
        let Some(db) = connect_test_database("svc_endpoint_invalid_generation_write").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        let endpoint = create_endpoint(
            &db,
            &service_id,
            make_input("invalid_generation", "get", "/original"),
        )
        .await
        .unwrap();
        db.collection::<ServiceEndpoint>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &endpoint.id },
                doc! { "$set": { "operation_generation": 0_i64 } },
            )
            .await
            .unwrap();

        let mut update = empty_update();
        update.path = Some("/must-not-apply".to_string());
        let error = update_endpoint(&db, &service_id, &endpoint.id, update)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
        let unchanged = load_endpoint(&db, &endpoint.id).await;
        assert_eq!(unchanged.path, "/original");
        assert_eq!(unchanged.operation_generation, 0);

        bulk_upsert_endpoints(&db, &service_id, Vec::new())
            .await
            .unwrap();
        let deactivated = load_endpoint(&db, &endpoint.id).await;
        assert!(!deactivated.is_active);
        assert_eq!(deactivated.operation_generation, 0);
    }

    #[tokio::test]
    async fn operation_generation_is_stable_across_bulk_and_additive_reconciliation() {
        let Some(db) = connect_test_database("svc_endpoint_generation_reconcile").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        let original = make_input("generation", "get", "/items");
        let created = bulk_upsert_endpoints(&db, &service_id, vec![original.clone()])
            .await
            .unwrap()
            .remove(0);
        assert_eq!(created.operation_generation, 1);

        let unchanged = bulk_upsert_endpoints(&db, &service_id, vec![original.clone()])
            .await
            .unwrap()
            .remove(0);
        assert_eq!(unchanged.operation_generation, 1);

        let mut changed = original.clone();
        changed.path = "/items-v2".to_string();
        let updated = bulk_upsert_endpoints(&db, &service_id, vec![changed.clone()])
            .await
            .unwrap()
            .remove(0);
        assert_eq!(updated.operation_generation, 2);
        let stable = upsert_endpoints_additive(&db, &service_id, vec![changed.clone()])
            .await
            .unwrap()
            .remove(0);
        assert_eq!(stable.operation_generation, 2);

        bulk_upsert_endpoints(&db, &service_id, Vec::new())
            .await
            .unwrap();
        bulk_upsert_endpoints(&db, &service_id, Vec::new())
            .await
            .unwrap();
        let inactive = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find_one(doc! { "_id": &created.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!inactive.is_active);
        assert_eq!(inactive.operation_generation, 3);

        let reactivated = bulk_upsert_endpoints(&db, &service_id, vec![changed.clone()])
            .await
            .unwrap()
            .remove(0);
        assert!(reactivated.is_active);
        assert_eq!(reactivated.operation_generation, 4);
        let stable = upsert_endpoints_additive(&db, &service_id, vec![changed])
            .await
            .unwrap()
            .remove(0);
        assert_eq!(stable.operation_generation, 4);
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
            &service_id,
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
                response: None,
                risk: None,
                supports_idempotency_key: None,
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
            "service-alpha",
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
                response: None,
                risk: None,
                supports_idempotency_key: None,
                is_active: None,
            },
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_endpoint_rejects_endpoint_owned_by_another_service() {
        let Some(db) = connect_test_database("svc_endpoint_update_owner").await else {
            return;
        };
        let owner_service_id = Uuid::new_v4().to_string();
        let other_service_id = Uuid::new_v4().to_string();
        let endpoint = create_endpoint(
            &db,
            &other_service_id,
            make_input("other_endpoint", "get", "/original"),
        )
        .await
        .unwrap();

        let result = update_endpoint(
            &db,
            &owner_service_id,
            &endpoint.id,
            EndpointUpdate {
                name: None,
                description: None,
                method: None,
                path: Some("/unauthorized".to_string()),
                parameters: None,
                request_body_schema: None,
                request_content_type: None,
                request_body_required: None,
                response_description: None,
                response: None,
                risk: None,
                supports_idempotency_key: None,
                is_active: None,
            },
        )
        .await;

        assert!(matches!(result, Err(AppError::NotFound(_))));
        let persisted = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find_one(doc! { "_id": &endpoint.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(persisted.path, "/original");
        assert_eq!(persisted.operation_generation, 1);
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
        delete_endpoint(&db, &service_id, &ep.id).await.unwrap();

        let count = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .count_documents(doc! { "_id": &ep.id })
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn delete_and_recreate_gets_a_new_producer_identity() {
        let Some(db) = connect_test_database("svc_endpoint_generation_recreate").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        let first = create_endpoint(&db, &service_id, make_input("recreated", "get", "/items"))
            .await
            .unwrap();
        delete_endpoint(&db, &service_id, &first.id).await.unwrap();
        let second = create_endpoint(&db, &service_id, make_input("recreated", "get", "/items"))
            .await
            .unwrap();

        assert_ne!(first.id, second.id, "delete/recreate must not form an ABA");
        assert_eq!(first.operation_generation, 1);
        assert_eq!(second.operation_generation, 1);
    }

    #[tokio::test]
    async fn test_delete_endpoint_not_found() {
        let Some(db) = connect_test_database("svc_endpoint").await else {
            return;
        };

        let result = delete_endpoint(&db, "service-alpha", "nonexistent-id").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_endpoint_rejects_endpoint_owned_by_another_service() {
        let Some(db) = connect_test_database("svc_endpoint_delete_owner").await else {
            return;
        };
        let owner_service_id = Uuid::new_v4().to_string();
        let other_service_id = Uuid::new_v4().to_string();
        let endpoint = create_endpoint(
            &db,
            &other_service_id,
            make_input("other_endpoint", "get", "/original"),
        )
        .await
        .unwrap();

        let result = delete_endpoint(&db, &owner_service_id, &endpoint.id).await;

        assert!(matches!(result, Err(AppError::NotFound(_))));
        assert_eq!(
            db.collection::<ServiceEndpoint>(COLLECTION_NAME)
                .count_documents(doc! { "_id": &endpoint.id })
                .await
                .unwrap(),
            1
        );
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

    #[tokio::test]
    async fn empty_bulk_reconcile_deactivates_all_endpoints_and_bumps_generation_once() {
        let Some(db) = connect_test_database("svc_endpoint_bulk_empty").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        let first = create_endpoint(&db, &service_id, make_input("ep_a", "get", "/a"))
            .await
            .unwrap();
        let second = create_endpoint(&db, &service_id, make_input("ep_b", "post", "/b"))
            .await
            .unwrap();

        let result = bulk_upsert_endpoints(&db, &service_id, Vec::new())
            .await
            .unwrap();

        assert!(result.is_empty());
        let mut persisted = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find(doc! { "_id": { "$in": [&first.id, &second.id] } })
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        persisted.sort_by(|left, right| left.id.cmp(&right.id));
        assert_eq!(persisted.len(), 2);
        assert!(persisted.iter().all(|endpoint| !endpoint.is_active));
        assert!(
            persisted
                .iter()
                .all(|endpoint| endpoint.operation_generation == 2)
        );

        bulk_upsert_endpoints(&db, &service_id, Vec::new())
            .await
            .unwrap();
        let generations = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find(doc! { "_id": { "$in": [&first.id, &second.id] } })
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert!(
            generations
                .iter()
                .all(|endpoint| endpoint.operation_generation == 2),
            "already inactive rows must not churn generation on repeat reconcile"
        );
    }

    #[tokio::test]
    async fn bulk_upsert_reactivates_on_admin_reconcile() {
        let Some(db) = connect_test_database("svc_endpoint_bulk_reactivate").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        let endpoint = create_endpoint(&db, &service_id, make_input("reactivate", "get", "/old"))
            .await
            .unwrap();

        db.collection::<ServiceEndpoint>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &endpoint.id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let results = bulk_upsert_endpoints(
            &db,
            &service_id,
            vec![make_input("reactivate", "get", "/updated")],
        )
        .await
        .unwrap();
        assert!(results[0].is_active);

        let stored = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find_one(doc! { "_id": &endpoint.id })
            .await
            .unwrap()
            .unwrap();
        assert!(stored.is_active);
    }

    #[tokio::test]
    async fn preserve_existing_returns_stored_activation() {
        let Some(db) = connect_test_database("svc_endpoint_preserve_activation").await else {
            return;
        };
        let service_id = Uuid::new_v4().to_string();
        let endpoint = create_endpoint(&db, &service_id, make_input("preserve", "get", "/old"))
            .await
            .unwrap();

        db.collection::<ServiceEndpoint>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &endpoint.id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let results = upsert_endpoints_additive(
            &db,
            &service_id,
            vec![make_input("preserve", "get", "/refreshed")],
        )
        .await
        .unwrap();
        assert!(!results[0].is_active);

        let stored = db
            .collection::<ServiceEndpoint>(COLLECTION_NAME)
            .find_one(doc! { "_id": &endpoint.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.is_active);
        assert_eq!(stored.path, "/refreshed");
    }
}
