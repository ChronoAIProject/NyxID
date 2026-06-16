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
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::UserService;
use crate::mw::auth::AuthUser;
use crate::services::{
    api_docs_service, catalog_service, openapi_parser, org_service, user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

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
    tele: TelemetryContext,
    Query(query): Query<CatalogListQuery>,
) -> AppResult<Json<CatalogListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let entries = if query.include_all {
        catalog_service::list_catalog_all(&state.db, &state.encryption_keys, &user_id).await?
    } else {
        catalog_service::list_catalog(&state.db, &state.encryption_keys, &user_id).await?
    };
    let items: Vec<CatalogEntryResponse> =
        entries.into_iter().map(catalog_entry_response).collect();

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
        scope_catalog: entry.scope_catalog,
        scope_removal: entry.scope_removal,
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

async fn find_readable_user_service_by_slug(
    state: &AppState,
    actor_user_id: &str,
    slug: &str,
) -> AppResult<Option<UserService>> {
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
    let svc =
        match catalog_service::get_downstream_service_by_slug(&state.db, &slug, &user_id).await {
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

    let Some(user_service) = find_readable_user_service_by_slug(&state, &user_id, &slug).await?
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
mod tests {
    use super::list_catalog_endpoints;
    use crate::errors::AppError;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgRole};
    use crate::models::user::COLLECTION_NAME as USERS;
    use crate::models::user::UserType;
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::services::api_docs_service::{SpecCacheTestGuard, cache_test_spec};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
        test_user_endpoint, test_user_service,
    };
    use axum::{
        Json,
        extract::{Path, State},
    };
    use uuid::Uuid;

    const CUSTOM_SPEC_URL: &str = "https://example.com/custom-openapi.json";
    const CATALOG_SPEC_URL: &str = "https://example.com/catalog-openapi.json";

    fn openapi_spec() -> serde_json::Value {
        serde_json::json!({
            "openapi": "3.1.0",
            "info": { "title": "Spec", "version": "1.0.0" },
            "paths": {
                "/widgets": {
                    "get": {
                        "operationId": "listWidgets",
                        "summary": "List widgets",
                        "responses": {
                            "200": {
                                "description": "ok"
                            }
                        }
                    }
                }
            }
        })
    }

    fn catalog_service(service_id: &str, slug: &str) -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = service_id.to_string();
        service.slug = slug.to_string();
        service.name = "Catalog Service".to_string();
        service.base_url = "https://catalog.example.com".to_string();
        service.openapi_spec_url = Some(CATALOG_SPEC_URL.to_string());
        service
    }

    #[tokio::test]
    async fn list_catalog_endpoints_returns_custom_service_operations() {
        let Some(db) = connect_test_database("catalog_endpoints_custom").await else {
            eprintln!("skipping catalog integration test: no local MongoDB available");
            return;
        };
        let _cache_guard = SpecCacheTestGuard::acquire();

        let caller_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "Custom API",
            "https://custom.example.com",
            Some(CUSTOM_SPEC_URL),
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "custom-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&caller_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint.clone())
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service)
            .await
            .unwrap();
        cache_test_spec(CUSTOM_SPEC_URL, Some(&caller_id), openapi_spec());

        let state = test_app_state(db);
        let Json(response) = list_catalog_endpoints(
            State(state),
            test_auth_user(&caller_id),
            crate::telemetry::TelemetryContext::default(),
            Path("custom-api".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.slug, "custom-api");
        assert_eq!(response.openapi_spec_url.as_deref(), Some(CUSTOM_SPEC_URL));
        assert_eq!(response.endpoints.len(), 1);
        assert_eq!(response.endpoints[0].method, "GET");
        assert_eq!(response.endpoints[0].path, "/widgets");
    }

    #[tokio::test]
    async fn list_catalog_endpoints_returns_empty_for_custom_service_without_spec() {
        let Some(db) = connect_test_database("catalog_endpoints_empty").await else {
            eprintln!("skipping catalog integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "Custom API",
            "https://custom.example.com",
            None,
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "custom-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&caller_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service)
            .await
            .unwrap();

        let state = test_app_state(db);
        let Json(response) = list_catalog_endpoints(
            State(state),
            test_auth_user(&caller_id),
            crate::telemetry::TelemetryContext::default(),
            Path("custom-api".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.slug, "custom-api");
        assert!(response.openapi_spec_url.is_none());
        assert!(response.endpoints.is_empty());
    }

    #[tokio::test]
    async fn list_catalog_endpoints_hides_other_users_custom_services() {
        let Some(db) = connect_test_database("catalog_endpoints_hidden").await else {
            eprintln!("skipping catalog integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        let owner_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &owner_id,
            "Other API",
            "https://other.example.com",
            Some(CUSTOM_SPEC_URL),
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &owner_id,
            "other-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_many([
                test_user(&caller_id, UserType::Person),
                test_user(&owner_id, UserType::Person),
            ])
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service)
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = list_catalog_endpoints(
            State(state),
            test_auth_user(&caller_id),
            crate::telemetry::TelemetryContext::default(),
            Path("other-api".to_string()),
        )
        .await
        .expect_err("other user's custom service should be hidden");

        assert!(matches!(err, AppError::NotFound(message) if message == "Catalog entry not found"));
    }

    #[tokio::test]
    async fn list_catalog_endpoints_prefers_catalog_entries_when_slug_exists_in_catalog() {
        let Some(db) = connect_test_database("catalog_endpoints_catalog").await else {
            eprintln!("skipping catalog integration test: no local MongoDB available");
            return;
        };
        let _cache_guard = SpecCacheTestGuard::acquire();

        let caller_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&caller_id, UserType::Person))
            .await
            .unwrap();

        let catalog = catalog_service(&Uuid::new_v4().to_string(), "catalog-api");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .unwrap();
        cache_test_spec(CATALOG_SPEC_URL, None, openapi_spec());

        let state = test_app_state(db);
        let Json(response) = list_catalog_endpoints(
            State(state),
            test_auth_user(&caller_id),
            crate::telemetry::TelemetryContext::default(),
            Path("catalog-api".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.slug, "catalog-api");
        assert_eq!(response.openapi_spec_url.as_deref(), Some(CATALOG_SPEC_URL));
        assert_eq!(response.endpoints.len(), 1);
        assert_eq!(response.endpoints[0].name, "listwidgets");
    }

    #[tokio::test]
    async fn list_catalog_endpoints_allows_org_shared_custom_services() {
        let Some(db) = connect_test_database("catalog_endpoints_org").await else {
            eprintln!("skipping catalog integration test: no local MongoDB available");
            return;
        };
        let _cache_guard = SpecCacheTestGuard::acquire();

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &org_id,
            "Org API",
            "https://org.example.com",
            Some(CUSTOM_SPEC_URL),
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &org_id,
            "org-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_many([
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member_id, OrgRole::Member, None))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service)
            .await
            .unwrap();
        cache_test_spec(CUSTOM_SPEC_URL, Some(&org_id), openapi_spec());

        let state = test_app_state(db);
        let Json(response) = list_catalog_endpoints(
            State(state),
            test_auth_user(&member_id),
            crate::telemetry::TelemetryContext::default(),
            Path("org-api".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.slug, "org-api");
        assert_eq!(response.endpoints.len(), 1);
        assert_eq!(response.endpoints[0].path, "/widgets");
    }

    // ── catalog_entry_response: pure mapping tests ──────────────────────

    fn minimal_catalog_entry() -> crate::services::catalog_service::CatalogEntry {
        crate::services::catalog_service::CatalogEntry {
            slug: "openai".to_string(),
            name: "OpenAI".to_string(),
            description: Some("AI API".to_string()),
            base_url: "https://api.openai.com".to_string(),
            auth_method: "bearer".to_string(),
            auth_key_name: "Authorization".to_string(),
            provider_config_id: None,
            provider_type: None,
            requires_gateway_url: false,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            credential_mode: None,
            service_type: "http".to_string(),
            ssh_host: None,
            ssh_port: None,
            ssh_ca_public_key: None,
            ssh_allowed_principals: None,
            ssh_certificate_ttl_minutes: None,
            authorization_url: None,
            token_url: None,
            device_code_url: None,
            device_verification_url: None,
            device_token_url: None,
            default_scopes: None,
            scope_catalog: None,
            scope_removal: None,
            supports_pkce: false,
            device_code_format: None,
            token_endpoint_auth_method: None,
            extra_auth_params: None,
            oauth_client_id: None,
            client_id_param_name: None,
            requires_credential: true,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            token_exchange_credential_fields: None,
            default_request_headers: None,
        }
    }

    #[test]
    fn catalog_entry_response_maps_basic_fields() {
        let entry = minimal_catalog_entry();
        let resp = super::catalog_entry_response(entry);
        assert_eq!(resp.slug, "openai");
        assert_eq!(resp.name, "OpenAI");
        assert_eq!(resp.description.as_deref(), Some("AI API"));
        assert_eq!(resp.base_url, "https://api.openai.com");
        assert_eq!(resp.auth_method, "bearer");
        assert_eq!(resp.auth_key_name, "Authorization");
        assert_eq!(resp.service_type, "http");
        assert!(resp.requires_credential);
    }

    #[test]
    fn catalog_entry_response_supports_pkce_none_when_false() {
        let entry = minimal_catalog_entry();
        let resp = super::catalog_entry_response(entry);
        // When supports_pkce is false, the response field should be None
        // (skip_serializing_if suppresses it in JSON output).
        assert!(resp.supports_pkce.is_none());
    }

    #[test]
    fn catalog_entry_response_supports_pkce_some_when_true() {
        let mut entry = minimal_catalog_entry();
        entry.supports_pkce = true;
        let resp = super::catalog_entry_response(entry);
        assert_eq!(resp.supports_pkce, Some(true));
    }

    #[test]
    fn catalog_entry_response_maps_ssh_fields() {
        let mut entry = minimal_catalog_entry();
        entry.service_type = "ssh".to_string();
        entry.ssh_host = Some("ssh.example.com".to_string());
        entry.ssh_port = Some(22);
        entry.ssh_ca_public_key = Some("ssh-rsa AAAA...".to_string());
        entry.ssh_allowed_principals = Some(vec!["deploy".to_string(), "admin".to_string()]);
        entry.ssh_certificate_ttl_minutes = Some(60);

        let resp = super::catalog_entry_response(entry);
        assert_eq!(resp.service_type, "ssh");
        assert_eq!(resp.ssh_host.as_deref(), Some("ssh.example.com"));
        assert_eq!(resp.ssh_port, Some(22));
        assert_eq!(resp.ssh_ca_public_key.as_deref(), Some("ssh-rsa AAAA..."));
        assert_eq!(
            resp.ssh_allowed_principals,
            Some(vec!["deploy".to_string(), "admin".to_string()])
        );
        assert_eq!(resp.ssh_certificate_ttl_minutes, Some(60));
    }

    #[test]
    fn catalog_entry_response_maps_oauth_fields() {
        let mut entry = minimal_catalog_entry();
        entry.authorization_url = Some("https://auth.example.com/authorize".to_string());
        entry.token_url = Some("https://auth.example.com/token".to_string());
        entry.device_code_url = Some("https://auth.example.com/device".to_string());
        entry.default_scopes = Some(vec!["read".to_string(), "write".to_string()]);
        entry.supports_pkce = true;
        entry.token_endpoint_auth_method = Some("client_secret_post".to_string());

        let resp = super::catalog_entry_response(entry);
        assert_eq!(
            resp.authorization_url.as_deref(),
            Some("https://auth.example.com/authorize")
        );
        assert_eq!(
            resp.token_url.as_deref(),
            Some("https://auth.example.com/token")
        );
        assert_eq!(
            resp.device_code_url.as_deref(),
            Some("https://auth.example.com/device")
        );
        assert_eq!(
            resp.default_scopes,
            Some(vec!["read".to_string(), "write".to_string()])
        );
        assert_eq!(resp.supports_pkce, Some(true));
        assert_eq!(
            resp.token_endpoint_auth_method.as_deref(),
            Some("client_secret_post")
        );
    }

    #[test]
    fn catalog_entry_response_maps_rich_metadata() {
        use crate::models::downstream_service::ServiceCapabilities;

        let mut entry = minimal_catalog_entry();
        entry.homepage_url = Some("https://openai.com".to_string());
        entry.repository_url = Some("https://github.com/openai".to_string());
        entry.issues_url = Some("https://github.com/openai/issues".to_string());
        entry.auth_notes = Some("Use API key from dashboard".to_string());
        entry.known_limitations = Some("Rate limited to 10k RPD".to_string());
        entry.required_permissions = Some(vec!["models:read".to_string()]);
        entry.examples_url = Some("https://docs.openai.com/examples".to_string());
        entry.recommended_skills = Some(vec!["chat".to_string()]);
        entry.capabilities = Some(ServiceCapabilities {
            supports_proxy_read: true,
            supports_proxy_write: true,
            supports_proxy_binary_upload: false,
            supports_direct_downstream_auth: false,
            supports_authoring_via_nyx: false,
            supports_websocket: false,
            supports_streaming: true,
        });

        let resp = super::catalog_entry_response(entry);
        assert_eq!(resp.homepage_url.as_deref(), Some("https://openai.com"));
        assert_eq!(
            resp.repository_url.as_deref(),
            Some("https://github.com/openai")
        );
        assert_eq!(
            resp.auth_notes.as_deref(),
            Some("Use API key from dashboard")
        );
        assert_eq!(
            resp.known_limitations.as_deref(),
            Some("Rate limited to 10k RPD")
        );
        assert_eq!(
            resp.required_permissions,
            Some(vec!["models:read".to_string()])
        );
        let caps = resp.capabilities.expect("capabilities present");
        assert!(caps.supports_proxy_read);
        assert!(caps.supports_streaming);
        assert!(!caps.supports_websocket);
    }

    #[test]
    fn catalog_entry_response_maps_gateway_url_fields() {
        let mut entry = minimal_catalog_entry();
        entry.requires_gateway_url = true;
        entry.provider_config_id = Some("prov-1".to_string());
        entry.provider_type = Some("api_key".to_string());

        let resp = super::catalog_entry_response(entry);
        assert!(resp.requires_gateway_url);
        assert_eq!(resp.provider_config_id.as_deref(), Some("prov-1"));
        assert_eq!(resp.provider_type.as_deref(), Some("api_key"));
    }

    // ── parsed_endpoint_to_response: pure mapping tests ─────────────────

    #[test]
    fn parsed_endpoint_to_response_maps_all_fields() {
        use crate::services::openapi_parser::ParsedEndpoint;

        let parsed = ParsedEndpoint {
            name: "listWidgets".to_string(),
            description: Some("List all widgets".to_string()),
            method: "GET".to_string(),
            path: "/widgets".to_string(),
            parameters: Some(serde_json::json!([{"name": "limit", "in": "query"}])),
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };
        let resp = super::parsed_endpoint_to_response(parsed);
        assert_eq!(resp.name, "listWidgets");
        assert_eq!(resp.description.as_deref(), Some("List all widgets"));
        assert_eq!(resp.method, "GET");
        assert_eq!(resp.path, "/widgets");
        assert!(resp.parameters.is_some());
        assert!(resp.request_body_schema.is_none());
        assert!(resp.request_content_type.is_none());
        assert!(!resp.request_body_required);
    }

    #[test]
    fn parsed_endpoint_to_response_with_request_body() {
        use crate::services::openapi_parser::ParsedEndpoint;

        let parsed = ParsedEndpoint {
            name: "createWidget".to_string(),
            description: None,
            method: "POST".to_string(),
            path: "/widgets".to_string(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({"type": "object"})),
            request_content_type: Some("application/json".to_string()),
            request_body_required: true,
        };
        let resp = super::parsed_endpoint_to_response(parsed);
        assert_eq!(resp.name, "createWidget");
        assert!(resp.description.is_none());
        assert_eq!(resp.method, "POST");
        assert!(resp.request_body_schema.is_some());
        assert_eq!(
            resp.request_content_type.as_deref(),
            Some("application/json")
        );
        assert!(resp.request_body_required);
    }

    // ── CatalogEntryResponse JSON serialization: skip_serializing_if ────

    #[test]
    fn catalog_entry_response_omits_none_fields_in_json() {
        let entry = minimal_catalog_entry();
        let resp = super::catalog_entry_response(entry);
        let json = serde_json::to_value(&resp).unwrap();
        // Fields that are None should not appear in JSON due to skip_serializing_if
        assert!(json.get("description").is_some()); // "AI API" is Some
        assert!(json.get("provider_config_id").is_none()); // None field omitted
        assert!(json.get("ssh_host").is_none());
        assert!(json.get("ssh_port").is_none());
        assert!(json.get("authorization_url").is_none());
        assert!(json.get("supports_pkce").is_none());
        assert!(json.get("capabilities").is_none());
        assert!(json.get("homepage_url").is_none());
        // Required fields are always present
        assert!(json.get("slug").is_some());
        assert!(json.get("name").is_some());
        assert!(json.get("base_url").is_some());
        assert!(json.get("requires_credential").is_some());
    }

    #[test]
    fn catalog_entry_response_includes_present_optional_fields() {
        let mut entry = minimal_catalog_entry();
        entry.homepage_url = Some("https://example.com".to_string());
        entry.supports_pkce = true;

        let resp = super::catalog_entry_response(entry);
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["homepage_url"], "https://example.com");
        assert_eq!(json["supports_pkce"], true);
    }

    // ── CatalogListQuery deserialization ─────────────────────────────────

    #[test]
    fn catalog_list_query_defaults_include_all_to_false() {
        let query: super::CatalogListQuery = serde_json::from_str("{}").unwrap();
        assert!(!query.include_all);
    }

    #[test]
    fn catalog_list_query_include_all_true() {
        let query: super::CatalogListQuery =
            serde_json::from_str(r#"{"include_all": true}"#).unwrap();
        assert!(query.include_all);
    }

    // ── CatalogEndpointResponse serialization ───────────────────────────

    #[test]
    fn catalog_endpoint_response_omits_none_fields() {
        let resp = super::CatalogEndpointResponse {
            name: "getUser".to_string(),
            description: None,
            method: "GET".to_string(),
            path: "/users/{id}".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("description").is_none());
        assert!(json.get("parameters").is_none());
        assert!(json.get("request_body_schema").is_none());
        assert!(json.get("request_content_type").is_none());
        assert_eq!(json["name"], "getUser");
        assert_eq!(json["method"], "GET");
        assert_eq!(json["path"], "/users/{id}");
        assert_eq!(json["request_body_required"], false);
    }

    // ── CatalogEndpointsListResponse serialization ──────────────────────

    #[test]
    fn catalog_endpoints_list_response_serialization() {
        let resp = super::CatalogEndpointsListResponse {
            slug: "my-api".to_string(),
            openapi_spec_url: Some("https://example.com/spec.json".to_string()),
            endpoints: vec![super::CatalogEndpointResponse {
                name: "health".to_string(),
                description: Some("Health check".to_string()),
                method: "GET".to_string(),
                path: "/health".to_string(),
                parameters: None,
                request_body_schema: None,
                request_content_type: None,
                request_body_required: false,
            }],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["slug"], "my-api");
        assert_eq!(json["openapi_spec_url"], "https://example.com/spec.json");
        assert_eq!(json["endpoints"].as_array().unwrap().len(), 1);
        assert_eq!(json["endpoints"][0]["name"], "health");
    }

    #[test]
    fn catalog_endpoints_list_response_omits_none_spec_url() {
        let resp = super::CatalogEndpointsListResponse {
            slug: "api".to_string(),
            openapi_spec_url: None,
            endpoints: vec![],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("openapi_spec_url").is_none());
        assert_eq!(json["endpoints"].as_array().unwrap().len(), 0);
    }
}
