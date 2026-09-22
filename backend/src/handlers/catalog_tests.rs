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
        recommended_skill_refs: None,
        skills_revision: 0,
        skills_manifest_digest: String::new(),
        slug: "openai".to_string(),
        name: "OpenAI".to_string(),
        description: Some("AI API".to_string()),
        base_url: "https://api.openai.com".to_string(),
        auth_method: "bearer".to_string(),
        auth_key_name: "Authorization".to_string(),
        provider_config_id: None,
        provider_type: None,
        revokes_grant: None,
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
        supports_oauth_scopes: true,
        scope_catalog: None,
        scope_removal: None,
        supports_pkce: false,
        device_code_format: None,
        token_endpoint_auth_method: None,
        token_request_encoding: None,
        oauth_request_headers: Default::default(),
        extra_auth_params: None,
        oauth_client_id: None,
        client_id_param_name: None,
        has_platform_oauth_credentials: false,
        platform_scope_allowlist: None,
        requires_credential: true,
        openapi_spec_url: None,
        asyncapi_spec_url: None,
        homepage_url: None,
        repository_url: None,
        issues_url: None,
        capabilities: None,
        inference: None,
        billing: None,
        platform_key: Default::default(),
        byok_pricing: None,
        auth_notes: None,
        known_limitations: None,
        required_permissions: None,
        examples_url: None,
        recommended_skills: None,
        token_exchange_credential_fields: None,
        default_request_headers: None,
    }
}

#[tokio::test]
async fn notion_oauth_catalog_response_carries_node_protocol_options() {
    let db = crate::test_utils::connect_test_database("notion_node_catalog")
        .await
        .expect("MongoDB required");
    let enc = crate::test_utils::test_encryption_keys();
    crate::services::provider_service::seed_default_providers(&db, &enc)
        .await
        .unwrap();
    crate::services::provider_service::seed_default_services(&db, &enc)
        .await
        .unwrap();
    let entry = crate::services::catalog_service::get_catalog_entry(
        &db,
        &enc,
        &uuid::Uuid::new_v4().to_string(),
        "api-notion",
    )
    .await
    .unwrap();
    let response = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json["token_request_encoding"], "json");
    assert_eq!(
        json["oauth_request_headers"]["Notion-Version"],
        "2022-06-28"
    );
    assert_eq!(json["supports_oauth_scopes"], false);
    assert_eq!(json["token_endpoint_auth_method"], "client_secret_basic");
    assert_eq!(json["extra_auth_params"]["owner"], "user");
}

#[test]
fn catalog_entry_response_maps_basic_fields() {
    let entry = minimal_catalog_entry();
    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
    assert_eq!(resp.slug, "openai");
    assert_eq!(
        resp.resource_uri,
        "http://localhost:3001/api/v1/proxy/s/openai"
    );
    assert_eq!(resp.name, "OpenAI");
    assert_eq!(resp.description.as_deref(), Some("AI API"));
    assert_eq!(resp.base_url, "https://api.openai.com");
    assert_eq!(resp.auth_method, "bearer");
    assert_eq!(resp.auth_key_name, "Authorization");
    assert_eq!(resp.service_type, "http");
    assert!(resp.requires_credential);
}

#[test]
fn catalog_entry_response_maps_revocation_capability() {
    let mut entry = minimal_catalog_entry();
    entry.revokes_grant = Some(true);

    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
    let revocation = resp.revocation.expect("revocation capability");
    assert!(revocation.revokes_grant);
}

#[test]
fn catalog_entry_response_supports_pkce_none_when_false() {
    let entry = minimal_catalog_entry();
    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
    // When supports_pkce is false, the response field should be None
    // (skip_serializing_if suppresses it in JSON output).
    assert!(resp.supports_pkce.is_none());
}

#[test]
fn catalog_entry_response_supports_pkce_some_when_true() {
    let mut entry = minimal_catalog_entry();
    entry.supports_pkce = true;
    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
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

    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
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

    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
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

    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
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

    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
    assert!(resp.requires_gateway_url);
    assert_eq!(resp.provider_config_id.as_deref(), Some("prov-1"));
    assert_eq!(resp.provider_type.as_deref(), Some("api_key"));
}

// ── parsed_endpoint_to_response: pure mapping tests ─────────────────

#[test]
fn parsed_endpoint_to_response_maps_all_fields() {
    use crate::services::openapi_parser::ParsedEndpoint;

    let parsed = ParsedEndpoint {
        origin: None,
        source_operation_id: Some("listWidgets".to_string()),
        name: "listWidgets".to_string(),
        description: Some("List all widgets".to_string()),
        method: "GET".to_string(),
        path: "/widgets".to_string(),
        parameters: Some(serde_json::json!([{"name": "limit", "in": "query"}])),
        request_body_schema: None,
        request_content_type: None,
        request_body_required: false,
        response: Default::default(),
        risk: None,
        supports_idempotency_key: false,
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
        origin: None,
        source_operation_id: Some("createWidget".to_string()),
        name: "createWidget".to_string(),
        description: None,
        method: "POST".to_string(),
        path: "/widgets".to_string(),
        parameters: None,
        request_body_schema: Some(serde_json::json!({"type": "object"})),
        request_content_type: Some("application/json".to_string()),
        request_body_required: true,
        response: Default::default(),
        risk: None,
        supports_idempotency_key: false,
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
    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
    let json = serde_json::to_value(&resp).unwrap();
    // Fields that are None should not appear in JSON due to skip_serializing_if
    assert!(json.get("description").is_some()); // "AI API" is Some
    assert!(json.get("provider_config_id").is_none()); // None field omitted
    assert!(json.get("revocation").is_none());
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
    entry.revokes_grant = Some(true);

    let resp = super::catalog_entry_response(&crate::test_utils::test_app_config(), entry);
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["homepage_url"], "https://example.com");
    assert_eq!(json["supports_pkce"], true);
    assert_eq!(json["revocation"]["revokes_grant"], true);
}

// ── CatalogListQuery deserialization ─────────────────────────────────

#[test]
fn catalog_list_query_defaults_include_all_to_false() {
    let query: super::CatalogListQuery = serde_json::from_str("{}").unwrap();
    assert!(!query.include_all);
}

#[test]
fn catalog_list_query_include_all_true() {
    let query: super::CatalogListQuery = serde_json::from_str(r#"{"include_all": true}"#).unwrap();
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
        response: Default::default(),
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
            response: Default::default(),
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
