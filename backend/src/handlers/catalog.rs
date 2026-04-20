use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::AppState;
use crate::errors::AppResult;
use crate::models::downstream_service::{CredentialFieldSpec, ServiceCapabilities};
use crate::mw::auth::AuthUser;
use crate::services::{api_docs_service, catalog_service, openapi_parser};

#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogEntryResponse {
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_config_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    pub requires_gateway_url: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_mode: Option<String>,
    // SSH fields
    pub service_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_ca_public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_allowed_principals: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_certificate_ttl_minutes: Option<u32>,
    // OAuth config fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_code_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_verification_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_token_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_pkce: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_code_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint_auth_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_auth_params: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id_param_name: Option<String>,
    /// Whether this catalog entry needs credential setup before it can be used
    pub requires_credential: bool,
    // --- Rich metadata for AI agent discovery ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openapi_spec_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asyncapi_spec_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ServiceCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_limitations: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_permissions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_skills: Option<Vec<String>>,
    /// Declared credential fields for `token_exchange` services. Clients
    /// read this to render the correct multi-field credential form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_exchange_credential_fields: Option<Vec<CredentialFieldSpec>>,
    /// Admin-configured default HTTP headers inherited from this catalog
    /// entry (NyxID#356). Read-only on this response; see the admin
    /// `PUT /services/{id}` endpoint to mutate. The per-user AI Services
    /// UI displays these as `(from catalog)` alongside the user's own
    /// entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_request_headers:
        Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogListResponse {
    pub entries: Vec<CatalogEntryResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogEndpointResponse {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_body_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_content_type: Option<String>,
    pub request_body_required: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogEndpointsListResponse {
    pub slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openapi_spec_url: Option<String>,
    pub endpoints: Vec<CatalogEndpointResponse>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct CatalogListQuery {
    /// Include all active services (including system services without auth).
    /// Default: false (only shows services requiring user credential setup).
    #[serde(default)]
    pub include_all: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/catalog",
    params(CatalogListQuery),
    responses(
        (status = 200, description = "List of available service catalog entries", body = CatalogListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "Catalog"
)]
/// GET /api/v1/catalog
pub async fn list_catalog(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<CatalogListQuery>,
) -> AppResult<Json<CatalogListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let entries = if query.include_all {
        catalog_service::list_catalog_all(&state.db, &state.encryption_keys, &user_id).await?
    } else {
        catalog_service::list_catalog(&state.db, &state.encryption_keys, &user_id).await?
    };
    let items = entries.into_iter().map(catalog_entry_response).collect();
    Ok(Json(CatalogListResponse { entries: items }))
}

#[utoipa::path(
    get,
    path = "/api/v1/catalog/{slug}",
    params(
        ("slug" = String, Path, description = "Catalog service slug")
    ),
    responses(
        (status = 200, description = "Catalog entry details", body = CatalogEntryResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Catalog entry not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Catalog"
)]
/// GET /api/v1/catalog/{slug}
pub async fn get_catalog_entry(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> AppResult<Json<CatalogEntryResponse>> {
    // Pass the caller's user_id so the service layer can enforce
    // visibility on private catalog entries — see
    // `catalog_service::get_catalog_entry` for the rules. Without this,
    // any authenticated user who guessed a private slug could read its
    // `default_request_headers` and other metadata.
    let user_id = auth_user.user_id.to_string();
    let entry =
        catalog_service::get_catalog_entry(&state.db, &state.encryption_keys, &user_id, &slug)
            .await?;
    Ok(Json(catalog_entry_response(entry)))
}

fn catalog_entry_response(entry: catalog_service::CatalogEntry) -> CatalogEntryResponse {
    let supports_pkce = if entry.supports_pkce {
        Some(true)
    } else {
        None
    };

    CatalogEntryResponse {
        slug: entry.slug,
        name: entry.name,
        description: entry.description,
        base_url: entry.base_url,
        auth_method: entry.auth_method,
        auth_key_name: entry.auth_key_name,
        provider_config_id: entry.provider_config_id,
        provider_type: entry.provider_type,
        requires_gateway_url: entry.requires_gateway_url,
        api_key_instructions: entry.api_key_instructions,
        api_key_url: entry.api_key_url,
        icon_url: entry.icon_url,
        documentation_url: entry.documentation_url,
        credential_mode: entry.credential_mode,
        service_type: entry.service_type,
        ssh_host: entry.ssh_host,
        ssh_port: entry.ssh_port,
        ssh_ca_public_key: entry.ssh_ca_public_key,
        ssh_allowed_principals: entry.ssh_allowed_principals,
        ssh_certificate_ttl_minutes: entry.ssh_certificate_ttl_minutes,
        authorization_url: entry.authorization_url,
        token_url: entry.token_url,
        device_code_url: entry.device_code_url,
        device_verification_url: entry.device_verification_url,
        device_token_url: entry.device_token_url,
        default_scopes: entry.default_scopes,
        supports_pkce,
        device_code_format: entry.device_code_format,
        token_endpoint_auth_method: entry.token_endpoint_auth_method,
        extra_auth_params: entry.extra_auth_params,
        oauth_client_id: entry.oauth_client_id,
        client_id_param_name: entry.client_id_param_name,
        requires_credential: entry.requires_credential,
        openapi_spec_url: entry.openapi_spec_url,
        asyncapi_spec_url: entry.asyncapi_spec_url,
        homepage_url: entry.homepage_url,
        repository_url: entry.repository_url,
        issues_url: entry.issues_url,
        capabilities: entry.capabilities,
        auth_notes: entry.auth_notes,
        known_limitations: entry.known_limitations,
        required_permissions: entry.required_permissions,
        examples_url: entry.examples_url,
        recommended_skills: entry.recommended_skills,
        token_exchange_credential_fields: entry.token_exchange_credential_fields,
        default_request_headers: crate::models::default_request_header::redact_list_for_response(
            entry.default_request_headers,
        ),
    }
}

fn parsed_endpoint_to_response(p: openapi_parser::ParsedEndpoint) -> CatalogEndpointResponse {
    CatalogEndpointResponse {
        name: p.name,
        description: p.description,
        method: p.method,
        path: p.path,
        parameters: p.parameters,
        request_body_schema: p.request_body_schema,
        request_content_type: p.request_content_type,
        request_body_required: p.request_body_required,
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/catalog/{slug}/endpoints",
    params(
        ("slug" = String, Path, description = "Catalog service slug")
    ),
    responses(
        (status = 200, description = "Parsed API endpoints from the service's OpenAPI spec", body = CatalogEndpointsListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Catalog entry not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Catalog"
)]
/// GET /api/v1/catalog/{slug}/endpoints
pub async fn list_catalog_endpoints(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(slug): Path<String>,
) -> AppResult<Json<CatalogEndpointsListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let svc = catalog_service::get_downstream_service_by_slug(&state.db, &slug, &user_id).await?;

    let Some(ref spec_url) = svc.openapi_spec_url else {
        return Ok(Json(CatalogEndpointsListResponse {
            slug,
            openapi_spec_url: None,
            endpoints: vec![],
        }));
    };

    // Use the hardened fetch path (DNS pinning, 5MB size limit, redirect policy, 60s cache)
    // instead of raw reqwest to prevent SSRF and resource exhaustion.
    let spec = api_docs_service::fetch_spec_json(spec_url).await?;
    let parsed = openapi_parser::parse_openapi_spec_value(&spec)?;
    let endpoints = parsed
        .into_iter()
        .map(parsed_endpoint_to_response)
        .collect();

    Ok(Json(CatalogEndpointsListResponse {
        slug,
        openapi_spec_url: Some(spec_url.clone()),
        endpoints,
    }))
}
