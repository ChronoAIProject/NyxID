use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{CredentialFieldSpec, ServiceCapabilities};
use crate::models::service_billing::ServiceBilling;
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::UserService;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    api_docs_service, catalog_discovery_service, catalog_service, oauth_resource_service,
    openapi_parser, org_service, user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogEntryResponse {
    pub slug: String,
    pub resource_uri: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation: Option<CatalogRevocationResponse>,
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
    pub supports_oauth_scopes: bool,
    /// Curated menu of notable available scopes for this provider (NyxID#917).
    /// Connect UIs render these as selectable pills; defaults are pre-selected
    /// and a free-form field covers anything not listed. `None` for providers
    /// with no curated catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_catalog: Option<Vec<crate::services::scope_catalog::ScopeCatalogEntry>>,
    /// How safely granted scopes can be removed from an existing connection
    /// (`auto` / `manual` / `unsupported`). Drives the Permissions panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_removal: Option<crate::services::scope_catalog::ScopeRemoval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_pkce: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_code_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint_auth_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_request_encoding: Option<String>,
    pub oauth_request_headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_auth_params: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id_param_name: Option<String>,
    /// Platform OAuth app credentials are provisioned for this provider
    /// (ciphertext presence only). With `credential_mode` "both" this drives
    /// the one-click connect path; the BYO form is the fallback when false.
    pub has_platform_oauth_credentials: bool,
    /// Scopes the shared platform OAuth app may request. The picker disables
    /// non-listed scopes on the platform path (they need "your own OAuth app").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_scope_allowlist: Option<Vec<String>>,
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
    pub billing: Option<ServiceBilling>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference: Option<crate::services::inference_service::InferenceView>,
    pub platform_key: crate::services::inference_service::PlatformKeyView,
    pub byok_pricing: Option<crate::services::inference_service::LanePricingView>,
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
    pub recommended_skill_refs: Option<Vec<crate::models::catalog_skill_revision::SkillReference>>,
    pub skills_revision: i64,
    pub skills_manifest_digest: String,
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
pub struct CatalogRevocationResponse {
    pub revokes_grant: bool,
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
    pub response: crate::models::service_endpoint::OperationResponseContract,
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
    tele: TelemetryContext,
    Query(query): Query<CatalogListQuery>,
) -> AppResult<Json<CatalogListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let entries = if query.include_all {
        catalog_service::list_catalog_all(&state.db, &state.encryption_keys, &user_id).await?
    } else {
        catalog_service::list_catalog(&state.db, &state.encryption_keys, &user_id).await?
    };
    let items: Vec<CatalogEntryResponse> = entries
        .into_iter()
        .map(|entry| catalog_entry_response(&state.config, entry))
        .collect();

    // Telemetry: catalog.browsed. `filter` is None today because the list
    // endpoint does not yet accept a search/filter query (only
    // `include_all`); plumb a filter string through here when a search
    // parameter lands.
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::CatalogBrowsed {
            filter: None,
            result_count: items.len() as i64,
        },
    );

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
    tele: TelemetryContext,
    Path(slug): Path<String>,
) -> AppResult<Json<CatalogEntryResponse>> {
    // Pass the caller's user_id so the service layer can enforce
    // visibility on private catalog entries — see
    // `catalog_service::get_catalog_entry` for the rules. Without this,
    // any authenticated user who guessed a private slug could read its
    // `default_request_headers` and other metadata.
    let user_id = auth_user.user_id.to_string();
    if auth_user.auth_method == AuthMethod::ApiKey {
        catalog_discovery_service::get_catalog_service(
            &state.db,
            &user_id,
            &slug,
            auth_user.api_key_service_scope(),
        )
        .await?;
    }
    let entry =
        catalog_service::get_catalog_entry(&state.db, &state.encryption_keys, &user_id, &slug)
            .await?;

    // Telemetry: catalog.entry_viewed.
    let has_openapi_spec = entry.openapi_spec_url.is_some();
    let catalog_slug = entry.slug.clone();
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::CatalogEntryViewed {
            catalog_slug,
            has_openapi_spec,
        },
    );

    Ok(Json(catalog_entry_response(&state.config, entry)))
}

fn catalog_entry_response(
    config: &crate::config::AppConfig,
    entry: catalog_service::CatalogEntry,
) -> CatalogEntryResponse {
    let supports_pkce = if entry.supports_pkce {
        Some(true)
    } else {
        None
    };

    // Catalog responses contain template slugs/skills/URLs, never instance
    // overrides or connection flags. Only platform availability and inference
    // binding/status_slug vary by caller, from live grants (not connections).
    let resource_uri = oauth_resource_service::user_service_resource_uri(config, &entry.slug);

    CatalogEntryResponse {
        slug: entry.slug,
        resource_uri,
        name: entry.name,
        description: entry.description,
        base_url: entry.base_url,
        auth_method: entry.auth_method,
        auth_key_name: entry.auth_key_name,
        provider_config_id: entry.provider_config_id,
        provider_type: entry.provider_type,
        revocation: entry
            .revokes_grant
            .map(|revokes_grant| CatalogRevocationResponse { revokes_grant }),
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
        supports_oauth_scopes: entry.supports_oauth_scopes,
        scope_catalog: entry.scope_catalog,
        scope_removal: entry.scope_removal,
        supports_pkce,
        device_code_format: entry.device_code_format,
        token_endpoint_auth_method: entry.token_endpoint_auth_method,
        token_request_encoding: entry.token_request_encoding,
        oauth_request_headers: entry.oauth_request_headers,
        extra_auth_params: entry.extra_auth_params,
        oauth_client_id: entry.oauth_client_id,
        client_id_param_name: entry.client_id_param_name,
        has_platform_oauth_credentials: entry.has_platform_oauth_credentials,
        platform_scope_allowlist: entry.platform_scope_allowlist,
        requires_credential: entry.requires_credential,
        openapi_spec_url: entry.openapi_spec_url,
        asyncapi_spec_url: entry.asyncapi_spec_url,
        homepage_url: entry.homepage_url,
        repository_url: entry.repository_url,
        issues_url: entry.issues_url,
        capabilities: entry.capabilities,
        billing: entry.billing,
        inference: entry.inference,
        platform_key: entry.platform_key,
        byok_pricing: entry.byok_pricing,
        auth_notes: entry.auth_notes,
        known_limitations: entry.known_limitations,
        required_permissions: entry.required_permissions,
        examples_url: entry.examples_url,
        recommended_skills: entry.recommended_skills,
        recommended_skill_refs: entry.recommended_skill_refs,
        skills_revision: entry.skills_revision,
        skills_manifest_digest: entry.skills_manifest_digest,
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
        response: p.response,
    }
}

async fn find_readable_user_service_by_slug(
    state: &AppState,
    actor_user_id: &str,
    slug: &str,
    auth_user: &AuthUser,
) -> AppResult<Option<UserService>> {
    if auth_user.auth_method == AuthMethod::ApiKey {
        return Ok(catalog_discovery_service::agent_services(
            &state.db,
            actor_user_id,
            auth_user.api_key_service_scope(),
        )
        .await?
        .into_iter()
        .find(|service| service.slug == slug));
    }
    if let Some(service) =
        user_service_service::find_by_slug(&state.db, actor_user_id, slug).await?
    {
        return Ok(Some(service));
    }

    let memberships =
        org_service::list_memberships_for_member(&state.db, actor_user_id, false).await?;
    for membership in memberships {
        let Some(service) =
            user_service_service::find_by_slug(&state.db, &membership.org_user_id, slug).await?
        else {
            continue;
        };

        let access =
            org_service::resolve_owner_access(&state.db, actor_user_id, &service.user_id).await?;
        if access.can_read() && access.allows_resource(&service.id) {
            return Ok(Some(service));
        }
    }

    Ok(None)
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
    tele: TelemetryContext,
    Path(slug): Path<String>,
) -> AppResult<Json<CatalogEndpointsListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let resolved = if auth_user.auth_method == AuthMethod::ApiKey {
        catalog_discovery_service::get_catalog_service(
            &state.db,
            &user_id,
            &slug,
            auth_user.api_key_service_scope(),
        )
        .await
    } else {
        catalog_service::get_downstream_service_by_slug(&state.db, &slug, &user_id).await
    };
    let svc = match resolved {
        Ok(service) => Some(service),
        Err(AppError::NotFound(_)) => None,
        Err(error) => return Err(error),
    };

    if let Some(svc) = svc {
        let Some(ref spec_url) = svc.openapi_spec_url else {
            // Telemetry: catalog.endpoints_fetched (no spec configured).
            emit_event(
                state.telemetry.as_deref(),
                &user_id,
                auth_user.api_key_id.as_deref(),
                &tele,
                TelemetryEvent::CatalogEndpointsFetched {
                    catalog_slug: slug.clone(),
                    endpoint_count: 0,
                },
            );
            return Ok(Json(CatalogEndpointsListResponse {
                slug,
                openapi_spec_url: None,
                endpoints: vec![],
            }));
        };

        // Use the hardened fetch path (DNS pinning, 5MB size limit, redirect policy, 60s cache)
        // instead of raw reqwest to prevent SSRF and resource exhaustion.
        // The URL comes only from the admin catalog row. This cache contains
        // public template specs and is shared by URL, never caller credentials.
        let spec = api_docs_service::fetch_spec_json(spec_url).await?;
        let parsed = openapi_parser::parse_openapi_spec_value(&spec)?;
        let endpoints: Vec<CatalogEndpointResponse> = parsed
            .into_iter()
            .map(parsed_endpoint_to_response)
            .collect();

        // Telemetry: catalog.endpoints_fetched (catalog service path).
        emit_event(
            state.telemetry.as_deref(),
            &user_id,
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::CatalogEndpointsFetched {
                catalog_slug: slug.clone(),
                endpoint_count: endpoints.len() as i64,
            },
        );

        return Ok(Json(CatalogEndpointsListResponse {
            slug,
            openapi_spec_url: Some(spec_url.clone()),
            endpoints,
        }));
    }

    let Some(user_service) =
        find_readable_user_service_by_slug(&state, &user_id, &slug, &auth_user).await?
    else {
        return Err(AppError::NotFound("Catalog entry not found".to_string()));
    };

    let user_endpoint = state
        .db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(doc! {
            "_id": &user_service.endpoint_id,
            "user_id": &user_service.user_id,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Catalog entry not found".to_string()))?;

    let Some(ref spec_url) = user_endpoint.openapi_spec_url else {
        // Telemetry: catalog.endpoints_fetched (user-service path, no spec).
        emit_event(
            state.telemetry.as_deref(),
            &user_id,
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::CatalogEndpointsFetched {
                catalog_slug: slug.clone(),
                endpoint_count: 0,
            },
        );
        return Ok(Json(CatalogEndpointsListResponse {
            slug,
            openapi_spec_url: None,
            endpoints: vec![],
        }));
    };

    let spec = api_docs_service::fetch_spec_json_scoped(spec_url, &user_endpoint.user_id).await?;
    let parsed = openapi_parser::parse_openapi_spec_value(&spec)?;
    let endpoints: Vec<CatalogEndpointResponse> = parsed
        .into_iter()
        .map(parsed_endpoint_to_response)
        .collect();

    // Telemetry: catalog.endpoints_fetched (user-service path).
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::CatalogEndpointsFetched {
            catalog_slug: slug.clone(),
            endpoint_count: endpoints.len() as i64,
        },
    );

    Ok(Json(CatalogEndpointsListResponse {
        slug,
        openapi_spec_url: Some(spec_url.clone()),
        endpoints,
    }))
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalog_routes_tests.rs"]
mod catalog_routes_tests;
