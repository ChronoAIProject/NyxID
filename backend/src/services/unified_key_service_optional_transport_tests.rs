use super::*;
use crate::services::billing::route_inventory::{
    BillingIngress, BillingRoutePolicy, enforce_billing_egress_classification,
};
use crate::services::credential_record_service as records;
use crate::test_utils::{test_app_state, test_auth_user, test_user};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode},
    routing::any,
};

struct Fixture {
    state: crate::AppState,
    owner: String,
    provider_id: String,
    catalog: DownstreamService,
    connection: UserService,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture() -> Fixture {
    let db = connect_test_database("optional_transport").await.unwrap();
    let app = Router::new().route(
        "/{*path}",
        any(|headers: HeaderMap| async move {
            axum::Json(serde_json::json!({
                "github": headers.get("X-GitHub-Token").map(|v| v.to_str().unwrap()),
                "authorization": headers.get("Authorization").map(|v| v.to_str().unwrap()),
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let mut llm_provider = multi_conn_provider("api_key");
    llm_provider.slug = "openai".into();
    let mut provider = multi_conn_provider("oauth2");
    provider.slug = "github".into();
    db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .insert_many([&llm_provider, &provider])
        .await
        .unwrap();
    let mut catalog = no_auth_catalog();
    catalog.slug = "llm-openai".into();
    catalog.base_url = base_url;
    catalog.provider_config_id = Some(llm_provider.id);
    catalog.identity_propagation_mode = "none".into();
    catalog.inject_delegation_token = false;
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    // Catalog-backed MCP connections publish registered template operations.
    // Exercise discovery of a real operation rather than the custom-service
    // generic proxy fallback.
    db.collection::<mongodb::bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "service_id": &catalog.id,
            "name": "probe",
            "method": "GET",
            "path": "/probe",
            "is_active": true,
            "created_at": mongodb::bson::DateTime::now(),
            "updated_at": mongodb::bson::DateTime::now(),
        })
        .await
        .unwrap();
    let mut requirement = sample_spr("header", Some("X-GitHub-Token"));
    requirement.id = uuid::Uuid::new_v4().to_string();
    requirement.service_id = catalog.id.clone();
    requirement.provider_config_id = provider.id.clone();
    requirement.required = false;
    db.collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .insert_one(requirement)
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    // Exercise the listing side effect that switches execution from legacy
    // catalog resolution to the automatic UserService route.
    let listed = Box::pin(crate::handlers::keys::list_keys(
        State(state.clone()),
        test_auth_user(&owner),
    ))
    .await
    .unwrap();
    let connection = automatic_service(&db, &owner, &catalog.id).await;
    assert!(listed.0.keys.iter().any(|key| key.id == connection.id));
    Fixture {
        state,
        owner,
        provider_id: provider.id,
        catalog,
        connection,
        server,
    }
}

async fn token_for(f: &Fixture, owner: &str, value: &str) -> String {
    let id = insert_provider_token(&f.state.db, owner, &f.provider_id).await;
    let tokens = f
        .state
        .db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS);
    let mut token = tokens.find_one(doc! { "_id": &id }).await.unwrap().unwrap();
    let (versions, access, refresh) =
        records::oauth_token_fields(&f.state.encryption_keys, &id, owner, None, value, None)
            .await
            .unwrap();
    token.secret_context_versions = versions;
    token.access_token_encrypted = Some(access);
    token.refresh_token_encrypted = refresh;
    tokens
        .replace_one(doc! { "_id": &id }, token)
        .await
        .unwrap();
    id
}

async fn rest_echo(f: &Fixture) -> serde_json::Value {
    rest_echo_at(f, false).await
}

async fn rest_echo_at(f: &Fixture, by_slug: bool) -> serde_json::Value {
    let mut request = Request::builder()
        .uri(if by_slug {
            format!("/api/v1/proxy/s/{}/probe", f.connection.slug)
        } else {
            format!("/api/v1/proxy/{}/probe", f.catalog.id)
        })
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
    let response = if by_slug {
        Box::pin(crate::handlers::proxy::proxy_request_by_slug(
            State(f.state.clone()),
            test_auth_user(&f.owner),
            Default::default(),
            Path((f.connection.slug.clone(), "probe".into())),
            request,
        ))
        .await
    } else {
        Box::pin(crate::handlers::proxy::proxy_request(
            State(f.state.clone()),
            test_auth_user(&f.owner),
            Default::default(),
            Path((f.catalog.id.clone(), "probe".into())),
            request,
        ))
        .await
    }
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap()
}

async fn mcp_echo(f: &Fixture) -> serde_json::Value {
    use crate::services::mcp_service::*;
    let services = Box::pin(load_user_tools(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.owner,
    ))
    .await
    .unwrap();
    let service = services
        .iter()
        .find(|s| s.service_id == f.connection.id)
        .unwrap();
    assert!(matches!(service.source, McpToolSource::UserManaged { .. }));
    let endpoint = service
        .endpoints
        .iter()
        .find(|ep| ep.name == "probe")
        .unwrap();
    assert_eq!(endpoint.path, "/probe");
    let prepared = prepare_proxy_tool_call(service, endpoint, &serde_json::json!({})).unwrap();
    let permit = enforce_billing_egress_classification(
        Some(BillingRoutePolicy::Metered(BillingIngress::Mcp)),
        BillingIngress::Mcp,
    )
    .unwrap();
    let (status, body) = Box::pin(execute_tool(
        &f.state.http_client,
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.node_ws_manager,
        &f.state.billing,
        &f.owner,
        &f.owner,
        service,
        endpoint,
        prepared,
        &f.state.jwt_keys,
        &f.state.config,
        &f.state.connection_expiry_notifier,
        &f.state.token_exchange_cache,
        &f.state.cloud_response_cache,
        &McpExecContext {
            assistant_chat: None,
            response_body_limit: None,
            actor_user_id: None,
            caller_token: None,
            delegation_restrictions: Default::default(),
            attribution: None,
            org_agent_access: None,
            agent_owner: None,
            operation_scopes: None,
            api_key_id: None,
            api_key_credential_id: None,
            allow_all_nodes: true,
            allowed_node_ids: &[],
        },
        permit,
    ))
    .await
    .unwrap();
    assert_eq!(status, 200);
    serde_json::from_str(&body).unwrap()
}

async fn llm_echo(f: &Fixture, gateway: bool) -> serde_json::Value {
    llm_echo_as(f, gateway, test_auth_user(&f.owner)).await
}

async fn llm_echo_as(
    f: &Fixture,
    gateway: bool,
    mut auth: crate::mw::auth::AuthUser,
) -> serde_json::Value {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/v1/llm/openai/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"probe"}]}"#,
        ))
        .unwrap();
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(if gateway {
            BillingIngress::LlmGateway
        } else {
            BillingIngress::LlmProvider
        }));
    auth.scope = "proxy:*".into();
    let response = if gateway {
        Box::pin(crate::handlers::llm_gateway::gateway_request(
            State(f.state.clone()),
            auth,
            Path("chat/completions".into()),
            request,
        ))
        .await
    } else {
        Box::pin(crate::handlers::llm_gateway::llm_proxy_request(
            State(f.state.clone()),
            auth,
            Path(("openai".into(), "chat/completions".into())),
            request,
        ))
        .await
    }
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap()
}

#[tokio::test]
async fn optional_header_reaches_rest_and_mcp_after_listing() {
    let f = fixture().await;
    let mut unrelated_key = sample_api_key("api_key");
    unrelated_key.id = uuid::Uuid::new_v4().to_string();
    unrelated_key.user_id = f.owner.clone();
    unrelated_key.provider_config_id = Some(f.provider_id.clone());
    unrelated_key.connection_id = Some(uuid::Uuid::new_v4().to_string());
    unrelated_key.credential_encrypted = Some(
        records::encrypt(
            &f.state.encryption_keys,
            USER_API_KEYS,
            &unrelated_key.id,
            &unrelated_key.user_id,
            "credential_encrypted",
            b"unselected-modern-token",
        )
        .await
        .unwrap(),
    );
    records::mark_version(
        &mut unrelated_key.secret_context_versions,
        &f.state.encryption_keys,
        "credential_encrypted",
    );
    f.state
        .db
        .collection::<UserApiKey>(USER_API_KEYS)
        .insert_one(unrelated_key)
        .await
        .unwrap();
    for echo in [
        rest_echo(&f).await,
        rest_echo_at(&f, true).await,
        mcp_echo(&f).await,
    ] {
        assert!(
            echo["github"].is_null(),
            "missing legacy credentials must succeed without selecting an arbitrary modern key"
        );
    }
    token_for(&f, &uuid::Uuid::new_v4().to_string(), "other-owner-token").await;
    for echo in [
        rest_echo(&f).await,
        rest_echo_at(&f, true).await,
        mcp_echo(&f).await,
    ] {
        assert!(
            echo["github"].is_null(),
            "another owner's token must not be selected"
        );
    }
    token_for(&f, &f.owner, "owner-optional-token").await;
    for echo in [
        rest_echo(&f).await,
        rest_echo_at(&f, true).await,
        mcp_echo(&f).await,
    ] {
        assert_eq!(echo["github"], "owner-optional-token");
        assert!(echo["authorization"].is_null());
    }
}

#[tokio::test]
async fn optional_header_reaches_both_llm_routes_after_listing() {
    let f = fixture().await;
    for gateway in [false, true] {
        assert!(llm_echo(&f, gateway).await["github"].is_null());
    }
    token_for(&f, &f.owner, "owner-optional-token").await;
    for gateway in [false, true] {
        assert_eq!(
            llm_echo(&f, gateway).await["github"],
            "owner-optional-token"
        );
    }
}

#[tokio::test]
async fn no_requirements_preserve_existing_automatic_destination_after_catalog_edit() {
    let f = fixture().await;
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .delete_many(doc! { "service_id": &f.catalog.id })
        .await
        .unwrap();
    f.state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &f.catalog.id },
            doc! { "$set": { "base_url": "https://changed.example" } },
        )
        .await
        .unwrap();
    let listed = Box::pin(crate::handlers::keys::list_keys(
        State(f.state.clone()),
        test_auth_user(&f.owner),
    ))
    .await
    .unwrap();
    assert!(listed.0.keys.iter().any(|key| {
        key.id == f.connection.id
            && key.catalog_service_id.as_deref() == Some(f.catalog.id.as_str())
    }));
    let retained = automatic_service(&f.state.db, &f.owner, &f.catalog.id).await;
    assert_eq!(retained.id, f.connection.id);
    assert_eq!(retained.endpoint_id, f.connection.endpoint_id);
    for echo in [
        rest_echo(&f).await,
        rest_echo_at(&f, true).await,
        mcp_echo(&f).await,
    ] {
        assert!(echo["github"].is_null());
        assert!(echo["authorization"].is_null());
    }
}

#[tokio::test]
async fn optional_header_llm_service_account_uses_resolved_owner() {
    let f = fixture().await;
    let service_account_id = uuid::Uuid::new_v4().to_string();
    token_for(&f, &service_account_id, "sa-subject-token").await;
    let sa_auth = || {
        let mut auth = test_auth_user(&service_account_id);
        auth.auth_method = crate::mw::auth::AuthMethod::ServiceAccount;
        auth.approval_owner_user_id = Some(f.owner.clone());
        auth
    };
    for gateway in [false, true] {
        assert!(llm_echo_as(&f, gateway, sa_auth()).await["github"].is_null());
    }
    token_for(&f, &f.owner, "resolved-owner-token").await;
    for gateway in [false, true] {
        assert_eq!(
            llm_echo_as(&f, gateway, sa_auth()).await["github"],
            "resolved-owner-token"
        );
    }
}

async fn resolved_target(f: &Fixture) -> crate::services::proxy_service::ProxyTarget {
    crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.connection.id,
        None,
        Some(&f.catalog.id),
        crate::services::proxy_service::ProxyExecutionContext::new(
            Some(&f.state.connection_expiry_notifier),
            f.state.platform_user_rate_limit,
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .target
}

async fn optional_credentials(
    f: &Fixture,
    owner: &str,
    catalog_id: &str,
    target: &crate::services::proxy_service::ProxyTarget,
) -> AppResult<Vec<crate::services::delegation_service::DelegatedCredential>> {
    crate::services::delegation_service::resolve_optional_auto_provision_credentials(
        &f.state.db,
        &f.state.encryption_keys,
        owner,
        &f.connection.id,
        catalog_id,
        target,
        Some(&f.state.connection_expiry_notifier),
    )
    .await
}

#[tokio::test]
async fn optional_helper_rejects_connection_owner_key_node_and_target_mismatches() {
    let f = fixture().await;
    let target = resolved_target(&f).await;
    let token_id = token_for(&f, &f.owner, "owner-optional-token").await;
    assert!(
        optional_credentials(
            &f,
            &uuid::Uuid::new_v4().to_string(),
            &f.catalog.id,
            &target
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert!(
        optional_credentials(&f, &f.owner, &uuid::Uuid::new_v4().to_string(), &target)
            .await
            .unwrap()
            .is_empty()
    );
    let services = f.state.db.collection::<UserService>(USER_SERVICES);
    for change in [
        doc! { "source": "manual" },
        doc! { "auth_method": "bearer" },
        doc! { "api_key_id": uuid::Uuid::new_v4().to_string() },
        doc! { "node_id": uuid::Uuid::new_v4().to_string() },
        doc! { "is_active": false },
        doc! { "credential_binding": "platform" },
    ] {
        services
            .update_one(doc! { "_id": &f.connection.id }, doc! { "$set": change })
            .await
            .unwrap();
        assert!(
            optional_credentials(&f, &f.owner, &f.catalog.id, &target)
                .await
                .unwrap()
                .is_empty()
        );
        services
            .replace_one(doc! { "_id": &f.connection.id }, &f.connection)
            .await
            .unwrap();
    }
    for mismatch in [
        "url",
        "target_id",
        "catalog",
        "slug",
        "service_url",
        "auth",
        "connection",
        "credential",
    ] {
        let mut target = resolved_target(&f).await;
        match mismatch {
            "url" => target.base_url = "https://unrelated.example".into(),
            "target_id" => target.target_id = Some("unrelated-target".into()),
            "catalog" => target.service.id = uuid::Uuid::new_v4().to_string(),
            "slug" => target.service.slug = "unrelated-service".into(),
            "service_url" => target.service.base_url = "https://unrelated.example".into(),
            "auth" => target.auth_method = "bearer".into(),
            "connection" => target.connection_id = Some(uuid::Uuid::new_v4().to_string()),
            "credential" => target.credential = "selected-modern-token".to_string().into(),
            _ => unreachable!(),
        }
        let result = optional_credentials(&f, &f.owner, &f.catalog.id, &target).await;
        assert!(
            result.is_err() || result.unwrap().is_empty(),
            "{mismatch} must not inherit provider credentials"
        );
    }
    let catalogs = f
        .state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    catalogs
        .update_one(
            doc! { "_id": &f.catalog.id },
            doc! { "$set": { "base_url": "https://changed.example" } },
        )
        .await
        .unwrap();
    assert!(
        optional_credentials(&f, &f.owner, &f.catalog.id, &target)
            .await
            .is_err(),
        "an actual optional-provider route must retain strict live catalog destination binding"
    );
    catalogs
        .replace_one(doc! { "_id": &f.catalog.id }, &f.catalog)
        .await
        .unwrap();
    let endpoints = f.state.db.collection::<UserEndpoint>(USER_ENDPOINTS);
    let endpoint = endpoints
        .find_one(doc! { "_id": &f.connection.endpoint_id })
        .await
        .unwrap()
        .unwrap();
    for change in [
        doc! { "user_id": uuid::Uuid::new_v4().to_string() },
        doc! { "catalog_service_id": uuid::Uuid::new_v4().to_string() },
        doc! { "url": "https://unrelated.example" },
    ] {
        endpoints
            .update_one(
                doc! { "_id": &f.connection.endpoint_id },
                doc! { "$set": change },
            )
            .await
            .unwrap();
        assert!(
            optional_credentials(&f, &f.owner, &f.catalog.id, &target)
                .await
                .is_err()
        );
        endpoints
            .replace_one(doc! { "_id": &f.connection.endpoint_id }, &endpoint)
            .await
            .unwrap();
    }
    let token = f
        .state
        .db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find_one(doc! { "_id": token_id })
        .await
        .unwrap()
        .unwrap();
    assert!(
        token.last_used_at.is_none(),
        "rejected candidates must not materialize tokens"
    );
}

#[tokio::test]
async fn optional_helper_rejects_requirement_made_mandatory_after_target_resolution() {
    let f = fixture().await;
    let target = resolved_target(&f).await;
    let token_id = token_for(&f, &f.owner, "owner-optional-token").await;
    f.state
        .db
        .collection::<ServiceProviderRequirement>(REQUIREMENTS)
        .update_one(
            doc! { "service_id": &f.catalog.id },
            doc! { "$set": { "required": true } },
        )
        .await
        .unwrap();
    assert!(matches!(
        optional_credentials(&f, &f.owner, &f.catalog.id, &target).await,
        Err(AppError::NotFound(_))
    ));
    let token = f
        .state
        .db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find_one(doc! { "_id": token_id })
        .await
        .unwrap()
        .unwrap();
    assert!(token.last_used_at.is_none());
}

#[tokio::test]
async fn optional_manual_and_modern_connections_do_not_inherit_legacy_tokens() {
    let f = fixture().await;
    token_for(&f, &f.owner, "owner-optional-token").await;
    let services = f.state.db.collection::<UserService>(USER_SERVICES);
    services
        .update_one(
            doc! { "_id": &f.connection.id },
            doc! { "$set": { "source": "manual" } },
        )
        .await
        .unwrap();
    for echo in [rest_echo(&f).await, mcp_echo(&f).await] {
        assert!(echo["github"].is_null());
    }
    let mut key = sample_api_key("api_key");
    key.id = uuid::Uuid::new_v4().to_string();
    key.user_id = f.owner.clone();
    key.connection_id = Some(uuid::Uuid::new_v4().to_string());
    key.provider_config_id = Some(f.provider_id.clone());
    key.credential_encrypted = Some(
        records::encrypt(
            &f.state.encryption_keys,
            USER_API_KEYS,
            &key.id,
            &key.user_id,
            "credential_encrypted",
            b"selected-modern-token",
        )
        .await
        .unwrap(),
    );
    records::mark_version(
        &mut key.secret_context_versions,
        &f.state.encryption_keys,
        "credential_encrypted",
    );
    f.state
        .db
        .collection::<UserApiKey>(USER_API_KEYS)
        .insert_one(&key)
        .await
        .unwrap();
    services.update_one(doc! { "_id": &f.connection.id }, doc! { "$set": { "api_key_id": &key.id, "auth_method": "bearer", "auth_key_name": "Authorization" } }).await.unwrap();
    for echo in [rest_echo(&f).await, mcp_echo(&f).await] {
        assert!(echo["github"].is_null());
        assert_eq!(echo["authorization"], "Bearer selected-modern-token");
    }
}
