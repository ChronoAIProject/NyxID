use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::service_endpoint_service::{EndpointInput, EndpointUpdate};
use crate::services::{openapi_parser, service_endpoint_service};

use super::services_helpers::{fetch_service, require_admin_or_creator};

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct CreateEndpointRequest {
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub response_description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEndpointRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub parameters: Option<Option<serde_json::Value>>,
    pub request_body_schema: Option<Option<serde_json::Value>>,
    pub response_description: Option<Option<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct EndpointResponse {
    pub id: String,
    pub service_id: String,
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub response_description: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct EndpointListResponse {
    pub endpoints: Vec<EndpointResponse>,
}

#[derive(Debug, Serialize)]
pub struct DeleteEndpointResponse {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct DiscoverEndpointsResponse {
    pub endpoints: Vec<EndpointResponse>,
    pub message: String,
}

// --- Validation helpers ---

const VALID_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH"];

fn validate_endpoint_name(name: &str) -> AppResult<()> {
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::ValidationError(
            "name must be between 1 and 100 characters".to_string(),
        ));
    }

    let valid = name.chars().enumerate().all(|(i, c)| {
        if i == 0 {
            c.is_ascii_lowercase()
        } else {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'
        }
    });

    if !valid {
        return Err(AppError::ValidationError(
            "name must match ^[a-z][a-z0-9_]*$ (valid MCP tool name)".to_string(),
        ));
    }

    Ok(())
}

fn validate_method(method: &str) -> AppResult<()> {
    let upper = method.to_uppercase();
    if !VALID_METHODS.contains(&upper.as_str()) {
        return Err(AppError::ValidationError(format!(
            "method must be one of: {}",
            VALID_METHODS.join(", ")
        )));
    }
    Ok(())
}

fn validate_path(path: &str) -> AppResult<()> {
    if !path.starts_with('/') {
        return Err(AppError::ValidationError(
            "path must start with /".to_string(),
        ));
    }
    if path.len() > 2048 {
        return Err(AppError::ValidationError(
            "path must not exceed 2048 characters".to_string(),
        ));
    }
    Ok(())
}

fn endpoint_to_response(e: crate::models::service_endpoint::ServiceEndpoint) -> EndpointResponse {
    EndpointResponse {
        id: e.id,
        service_id: e.service_id,
        name: e.name,
        description: e.description,
        method: e.method,
        path: e.path,
        parameters: e.parameters,
        request_body_schema: e.request_body_schema,
        response_description: e.response_description,
        is_active: e.is_active,
        created_at: e.created_at.to_rfc3339(),
        updated_at: e.updated_at.to_rfc3339(),
    }
}

// --- Handlers ---

/// GET /api/v1/services/{service_id}/endpoints
///
/// List all active endpoints for a service. Any authenticated user.
pub async fn list_endpoints(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<EndpointListResponse>> {
    // Verify service exists
    let _service = fetch_service(&state, &service_id).await?;

    let endpoints = service_endpoint_service::list_endpoints(&state.db, &service_id).await?;
    let items: Vec<EndpointResponse> = endpoints.into_iter().map(endpoint_to_response).collect();

    Ok(Json(EndpointListResponse { endpoints: items }))
}

/// POST /api/v1/services/{service_id}/endpoints
///
/// Create a new endpoint. Admin or service creator.
pub async fn create_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<CreateEndpointRequest>,
) -> AppResult<Json<EndpointResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    validate_endpoint_name(&body.name)?;
    validate_method(&body.method)?;
    validate_path(&body.path)?;

    let input = EndpointInput {
        name: body.name,
        description: body.description,
        method: body.method,
        path: body.path,
        parameters: body.parameters,
        request_body_schema: body.request_body_schema,
        response_description: body.response_description,
    };

    let endpoint = service_endpoint_service::create_endpoint(&state.db, &service_id, input).await?;

    tracing::info!(
        endpoint_id = %endpoint.id,
        service_id = %service_id,
        created_by = %auth_user.user_id,
        "Endpoint created"
    );

    Ok(Json(endpoint_to_response(endpoint)))
}

/// PUT /api/v1/services/{service_id}/endpoints/{endpoint_id}
///
/// Update an existing endpoint. Admin or service creator.
pub async fn update_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, endpoint_id)): Path<(String, String)>,
    Json(body): Json<UpdateEndpointRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    if let Some(ref name) = body.name {
        validate_endpoint_name(name)?;
    }
    if let Some(ref method) = body.method {
        validate_method(method)?;
    }
    if let Some(ref path) = body.path {
        validate_path(path)?;
    }

    let updates = EndpointUpdate {
        name: body.name,
        description: body.description,
        method: body.method,
        path: body.path,
        parameters: body.parameters,
        request_body_schema: body.request_body_schema,
        response_description: body.response_description,
        is_active: body.is_active,
    };

    service_endpoint_service::update_endpoint(&state.db, &endpoint_id, updates).await?;

    tracing::info!(
        endpoint_id = %endpoint_id,
        service_id = %service_id,
        updated_by = %auth_user.user_id,
        "Endpoint updated"
    );

    Ok(Json(serde_json::json!({ "message": "Endpoint updated" })))
}

/// DELETE /api/v1/services/{service_id}/endpoints/{endpoint_id}
///
/// Delete an endpoint. Admin or service creator.
pub async fn delete_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, endpoint_id)): Path<(String, String)>,
) -> AppResult<Json<DeleteEndpointResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    service_endpoint_service::delete_endpoint(&state.db, &endpoint_id).await?;

    tracing::info!(
        endpoint_id = %endpoint_id,
        service_id = %service_id,
        deleted_by = %auth_user.user_id,
        "Endpoint deleted"
    );

    Ok(Json(DeleteEndpointResponse {
        message: "Endpoint deleted".to_string(),
    }))
}

/// POST /api/v1/services/{service_id}/discover-endpoints
///
/// Fetch the service's api_spec_url, parse the OpenAPI/Swagger spec,
/// and bulk upsert discovered endpoints. Admin or service creator.
pub async fn discover_endpoints(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<DiscoverEndpointsResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let api_spec_url = service.api_spec_url.ok_or_else(|| {
        AppError::BadRequest("Service has no api_spec_url configured".to_string())
    })?;

    let parsed = openapi_parser::parse_openapi_spec(&state.http_client, &api_spec_url).await?;

    let inputs: Vec<EndpointInput> = parsed
        .into_iter()
        .map(|p| EndpointInput {
            name: p.name,
            description: p.description,
            method: p.method,
            path: p.path,
            parameters: p.parameters,
            request_body_schema: p.request_body_schema,
            response_description: None,
        })
        .collect();

    let count = inputs.len();
    let endpoints =
        service_endpoint_service::bulk_upsert_endpoints(&state.db, &service_id, inputs).await?;

    tracing::info!(
        service_id = %service_id,
        endpoint_count = count,
        discovered_by = %auth_user.user_id,
        "Endpoints discovered from OpenAPI spec"
    );

    let items: Vec<EndpointResponse> = endpoints.into_iter().map(endpoint_to_response).collect();

    Ok(Json(DiscoverEndpointsResponse {
        message: format!("{count} endpoints discovered and synced"),
        endpoints: items,
    }))
}
