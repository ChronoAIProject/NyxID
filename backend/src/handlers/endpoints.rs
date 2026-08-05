use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::service_endpoint::{EndpointRisk, OperationResponseContract};
use crate::mw::auth::AuthUser;
use crate::services::service_endpoint_service::{
    EndpointInput, EndpointUpdate, validate_request_content_type, validate_response_contract,
};
use crate::services::{openapi_parser, service_endpoint_service};

use super::services_helpers::{fetch_service, require_admin_or_creator, require_http_service};

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct CreateEndpointRequest {
    pub name: String,
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    pub parameters: Option<serde_json::Value>,
    pub request_body_schema: Option<serde_json::Value>,
    pub request_content_type: Option<String>,
    pub request_body_required: Option<bool>,
    pub response_description: Option<String>,
    pub response: Option<OperationResponseContract>,
    pub risk: Option<EndpointRisk>,
    pub supports_idempotency_key: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEndpointRequest {
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
    pub request_content_type: Option<String>,
    pub request_body_required: bool,
    pub response_description: Option<String>,
    pub response: OperationResponseContract,
    pub risk: Option<EndpointRisk>,
    pub supports_idempotency_key: bool,
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
    let request_body_required = e.effective_request_body_required();

    EndpointResponse {
        id: e.id,
        service_id: e.service_id,
        name: e.name,
        description: e.description,
        method: e.method,
        path: e.path,
        parameters: e.parameters,
        request_body_schema: e.request_body_schema,
        request_content_type: e.request_content_type,
        request_body_required,
        response_description: e.response_description,
        response: e.response,
        risk: e.risk,
        supports_idempotency_key: e.supports_idempotency_key,
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
    let service = fetch_service(&state, &service_id).await?;
    require_http_service(&service)?;

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
    require_http_service(&service)?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    validate_endpoint_name(&body.name)?;
    validate_method(&body.method)?;
    validate_path(&body.path)?;
    if let Some(content_type) = body.request_content_type.as_deref() {
        validate_request_content_type(content_type)?;
    }
    if let Some(response) = body.response.as_ref() {
        validate_response_contract(response)?;
    }

    let input = EndpointInput {
        request_body_required: body
            .request_body_required
            .unwrap_or(body.request_body_schema.is_some() || body.request_content_type.is_some()),
        name: body.name,
        description: body.description,
        method: body.method,
        path: body.path,
        parameters: body.parameters,
        request_body_schema: body.request_body_schema,
        request_content_type: body.request_content_type,
        response_description: body.response_description,
        response: body.response.unwrap_or_default(),
        risk: body.risk,
        supports_idempotency_key: body.supports_idempotency_key.unwrap_or(false),
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
    require_http_service(&service)?;
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
    if let Some(Some(content_type)) = body.request_content_type.as_ref() {
        validate_request_content_type(content_type)?;
    }
    if let Some(response) = body.response.as_ref() {
        validate_response_contract(response)?;
    }

    let updates = EndpointUpdate {
        name: body.name,
        description: body.description,
        method: body.method,
        path: body.path,
        parameters: body.parameters,
        request_body_schema: body.request_body_schema,
        request_content_type: body.request_content_type,
        request_body_required: body.request_body_required,
        response_description: body.response_description,
        response: body.response,
        risk: body.risk,
        supports_idempotency_key: body.supports_idempotency_key,
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
    require_http_service(&service)?;
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
/// Fetch the service's OpenAPI spec URL, parse the OpenAPI/Swagger spec,
/// and bulk upsert discovered endpoints. Admin or service creator.
pub async fn discover_endpoints(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<DiscoverEndpointsResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_http_service(&service)?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let api_spec_url = service.openapi_spec_url.ok_or_else(|| {
        AppError::BadRequest("Service has no openapi_spec_url configured".to_string())
    })?;

    let parsed = openapi_parser::parse_openapi_spec(&state.http_client, &api_spec_url).await?;

    for endpoint in &parsed {
        if let Some(content_type) = endpoint.request_content_type.as_deref() {
            validate_request_content_type(content_type)?;
        }
        validate_response_contract(&endpoint.response)?;
    }

    let inputs: Vec<EndpointInput> = parsed
        .into_iter()
        .map(|p| EndpointInput {
            name: p.name,
            description: p.description,
            method: p.method,
            path: p.path,
            parameters: p.parameters,
            request_body_schema: p.request_body_schema,
            request_content_type: p.request_content_type,
            request_body_required: p.request_body_required,
            response_description: None,
            response: p.response,
            risk: p.risk,
            supports_idempotency_key: p.supports_idempotency_key,
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

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{endpoint_to_response, validate_request_content_type, validate_response_contract};
    use crate::errors::AppError;
    use crate::models::service_endpoint::ServiceEndpoint;

    #[test]
    fn validate_request_content_type_accepts_valid_values() {
        validate_request_content_type("application/zip").expect("zip should be valid");
        validate_request_content_type("application/json; charset=utf-8")
            .expect("parameterized content type should be valid");
        validate_request_content_type("*/*").expect("wildcard content type should be valid");
    }

    #[test]
    fn validate_request_content_type_rejects_invalid_values() {
        let empty = validate_request_content_type("   ")
            .expect_err("empty content types should be rejected");
        assert!(
            matches!(empty, AppError::ValidationError(message) if message.contains("must not be empty"))
        );

        let invalid = validate_request_content_type("application/json\nx-bad: nope")
            .expect_err("invalid header values should be rejected");
        assert!(
            matches!(invalid, AppError::ValidationError(message) if message.contains("valid HTTP content type"))
        );
    }

    #[test]
    fn validate_response_contract_rejects_invalid_media_types() {
        let response = crate::models::service_endpoint::OperationResponseContract {
            content_types: vec!["application/json\nx-invalid: true".to_string()],
            binary_artifact: Some(false),
        };
        assert!(validate_response_contract(&response).is_err());
    }

    #[test]
    fn endpoint_to_response_uses_effective_request_body_required() {
        let endpoint = ServiceEndpoint {
            id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            name: "list_users".to_string(),
            description: Some("List users".to_string()),
            method: "GET".to_string(),
            path: "/users".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: true,
            response_description: None,
            response: Default::default(),
            risk: None,
            supports_idempotency_key: false,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let response = endpoint_to_response(endpoint);
        assert!(!response.request_body_required);
    }
}
