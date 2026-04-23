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
        return Ok(Json(CatalogEndpointsListResponse {
            slug,
            openapi_spec_url: None,
            endpoints: vec![],
        }));
    };

    let spec = api_docs_service::fetch_spec_json_scoped(spec_url, &user_endpoint.user_id).await?;
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
    use crate::services::api_docs_service::{cache_test_spec, clear_test_spec_cache};
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
        clear_test_spec_cache();

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
        clear_test_spec_cache();

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
            Path("catalog-api".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.slug, "catalog-api");
        assert_eq!(response.openapi_spec_url.as_deref(), Some(CATALOG_SPEC_URL));
        assert_eq!(response.endpoints.len(), 1);
        assert_eq!(response.endpoints[0].name, "listWidgets");
    }

    #[tokio::test]
    async fn list_catalog_endpoints_allows_org_shared_custom_services() {
        let Some(db) = connect_test_database("catalog_endpoints_org").await else {
            eprintln!("skipping catalog integration test: no local MongoDB available");
            return;
        };
        clear_test_spec_cache();

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
            Path("org-api".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(response.slug, "org-api");
        assert_eq!(response.endpoints.len(), 1);
        assert_eq!(response.endpoints[0].path, "/widgets");
    }
}
