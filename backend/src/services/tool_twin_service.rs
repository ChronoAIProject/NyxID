use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{COLLECTION_NAME, DownstreamService},
        service_endpoint::{PublicationState, ServiceEndpoint},
    },
};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde_json::{Value, json};

pub async fn prepare_create(db: &Database, mut body: Value) -> AppResult<Value> {
    let Some(reference) = body.get("twin_of_service_id").and_then(Value::as_str) else {
        return Ok(body);
    };
    let source = db
        .collection::<DownstreamService>(COLLECTION_NAME)
        .find_one(doc! {"$or":[{"_id":reference},{"slug":reference}],"is_active":true})
        .await?
        .ok_or_else(|| AppError::NotFound("Twin source service not found".into()))?;
    super::retired_service_service::require_available(&source)?;
    if source.service_type != "http"
        || !matches!(
            source.auth_method.as_str(),
            "none" | "bearer" | "header" | "query" | "query_param" | "basic"
        )
    {
        return Err(AppError::ValidationError(
            "Tool twins require an HTTP source with supported credential injection".into(),
        ));
    }
    if body
        .get("slug")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(AppError::ValidationError(
            "slug is required for a tool twin".into(),
        ));
    }
    for (field, expected) in [
        ("offering_kind", "tool"),
        ("service_category", "internal"),
        ("service_type", "http"),
    ] {
        if body.get(field).is_some_and(|value| value != expected) {
            return Err(AppError::ValidationError(format!(
                "Tool twin {field} must be {expected}"
            )));
        }
        body[field] = expected.into();
    }
    if body.get("provider_config_id").is_some_and(|v| !v.is_null())
        || body.get("credential").is_some_and(|v| !v.is_null())
    {
        return Err(AppError::ValidationError(
            "Tool twins cannot receive a provider link or credential at creation".into(),
        ));
    }
    body["provider_config_id"] = Value::Null;
    body["credential"] = Value::Null;
    body["twin_of_service_id"] = source.id.clone().into();
    let inherited = json!({
        "name":source.name, "base_url":source.base_url,
        "destination_targets":source.destination_targets, "auth_method":source.auth_method,
        "auth_key_name":source.auth_key_name, "openapi_spec_url":source.openapi_spec_url,
        "asyncapi_spec_url":source.asyncapi_spec_url, "default_request_headers":source.default_request_headers,
        "custom_user_agent":source.custom_user_agent, "capabilities":source.capabilities,
        "homepage_url":source.homepage_url, "repository_url":source.repository_url,
        "issues_url":source.issues_url, "examples_url":source.examples_url,
        "auth_notes":source.auth_notes, "known_limitations":source.known_limitations,
        "required_permissions":source.required_permissions, "description":source.description,
        "platform_key":{"enabled":true,"audience":"public","allowed_owner_ids":[]},
        "import_source":{"kind":"catalog_twin","reference":source.slug,"version":source.updated_at.to_rfc3339(),"imported_at":chrono::Utc::now().to_rfc3339()}
    });
    let target = body
        .as_object_mut()
        .ok_or_else(|| AppError::ValidationError("Expected service object".into()))?;
    for (key, value) in inherited.as_object().unwrap() {
        target.entry(key.clone()).or_insert_with(|| value.clone());
    }
    target.insert("import_source".into(), inherited["import_source"].clone());
    Ok(body)
}

pub async fn clone_endpoints(
    db: &Database,
    source_id: &str,
    target_id: &str,
) -> AppResult<Vec<ServiceEndpoint>> {
    let mut endpoints: Vec<ServiceEndpoint> = db
        .collection::<ServiceEndpoint>(crate::models::service_endpoint::COLLECTION_NAME)
        .find(doc! {"service_id":source_id})
        .await?
        .try_collect()
        .await?;
    let now = chrono::Utc::now();
    for endpoint in &mut endpoints {
        endpoint.id = uuid::Uuid::new_v4().to_string();
        endpoint.service_id = target_id.into();
        endpoint.publication = PublicationState::Draft;
        endpoint.is_active = false;
        endpoint.operation_generation = 1;
        endpoint.created_at = now;
        endpoint.updated_at = now;
    }
    Ok(endpoints)
}
