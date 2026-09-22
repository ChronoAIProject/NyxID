use super::{authenticate_mcp, mcp_node_scope, mcp_service_scope};
use crate::{
    AppState,
    crypto::jwt,
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService},
        service_endpoint::COLLECTION_NAME as OPERATIONS,
        user::{COLLECTION_NAME as USERS, User, UserType},
        user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
    services::{
        api_docs_service::{SpecCacheTestGuard, cache_test_spec},
        key_service::{self, CreatedApiKey},
        mcp_service,
    },
    test_utils::{
        connect_test_database, test_app_state, test_auto_connected_catalog_service, test_user,
        test_user_endpoint, test_user_service,
    },
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const CONFIG: &str = "/api/v1/mcp/config";

struct Fixture {
    state: AppState,
    owner: String,
}

impl Fixture {
    async fn new(name: &str) -> Option<Self> {
        let db = connect_test_database(name).await?;
        let owner = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        Some(Self {
            state: test_app_state(db),
            owner,
        })
    }

    async fn key(
        &self,
        scope: &str,
        allowed_service_ids: Option<&[String]>,
        allow_all_services: bool,
        allow_all_nodes: bool,
        platform: Option<&str>,
    ) -> CreatedApiKey {
        key_service::create_api_key(
            &self.state.db,
            &self.owner,
            "MCP discovery",
            scope,
            /* expires_at */ None,
            /* description */ None,
            allowed_service_ids,
            /* allowed_node_ids */ None,
            /* allow_all_services */ Some(allow_all_services),
            /* allow_auto_connected_services */ Some(false),
            /* allow_all_nodes */ Some(allow_all_nodes),
            /* rate_limit_per_second */ None,
            /* rate_limit_burst */ None,
            platform,
            /* callback_url */ None,
        )
        .await
        .unwrap()
    }

    async fn services(&self) -> [UserService; 2] {
        let mut catalog = test_auto_connected_catalog_service();
        catalog.requires_user_credential = true;
        self.state
            .db
            .collection::<DownstreamService>(CATALOG)
            .insert_one(&catalog)
            .await
            .unwrap();
        self.state
            .db
            .collection::<Document>(OPERATIONS)
            .insert_one(doc! {
                "_id": Uuid::new_v4().to_string(), "service_id": &catalog.id,
                "name": "template_op", "method": "GET", "path": "/template",
                "is_active": true, "created_at": bson::DateTime::now(),
                "updated_at": bson::DateTime::now(),
            })
            .await
            .unwrap();
        [
            self.mount("custom", None).await,
            self.mount("override", Some(&catalog.id)).await,
        ]
    }

    async fn mount(&self, slug: &str, catalog_id: Option<&str>) -> UserService {
        let endpoint_id = Uuid::new_v4().to_string();
        let url = format!("https://example.com/{endpoint_id}/openapi.json");
        self.state
            .db
            .collection::<UserEndpoint>(ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &self.owner,
                slug,
                "https://example.com",
                Some(&url),
                catalog_id,
            ))
            .await
            .unwrap();
        let service = test_user_service(
            &Uuid::new_v4().to_string(),
            &self.owner,
            slug,
            &endpoint_id,
            catalog_id,
            None,
        );
        self.state
            .db
            .collection::<UserService>(SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        cache_test_spec(
            &url,
            Some(&self.owner),
            json!({
                "openapi": "3.1.0", "info": {"title": slug, "version": "1.0"},
                "paths": {format!("/{slug}"): {"get": {
                    "operationId": format!("{slug}_op"),
                    "responses": {"200": {"description": "ok", "content": {
                        "application/json": {"schema": {"type": "object"}}
                    }}}
                }}}
            }),
        );
        service
    }
}

fn router(state: &AppState) -> Router {
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    private.with_state(state.clone())
}

async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, Value) {
    let mut req = Request::builder().method(method).uri(path);
    *req.headers_mut().unwrap() = headers.clone();
    let response = router(state)
        .oneshot(
            req.header("content-type", "application/json")
                .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (status, headers, serde_json::from_slice(&bytes).unwrap())
}

fn key_headers(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", key.parse().unwrap());
    headers
}

fn bearer_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    headers
}

async fn assert_parity(state: &AppState, headers: &HeaderMap, rest: &Value) {
    let auth = authenticate_mcp(state, headers, false).await.unwrap();
    let catalog = mcp_service::load_operation_catalog(
        &state.db,
        state.node_ws_manager.as_ref(),
        &auth.user_id,
        mcp_node_scope(&auth),
        mcp_service_scope(&auth),
    )
    .await
    .unwrap();
    assert_eq!(
        rest["catalog_digest"],
        mcp_service::operation_catalog_digest(&catalog.services)
    );
    assert_eq!(
        rest["skills_manifest_digest"],
        mcp_service::skills_manifest_digest(&catalog.services)
    );
    assert_eq!(rest["total_services"], catalog.services.len());

    if !auth.is_stateless() {
        return;
    }

    let (status, _, listing) = request(
        state,
        "POST",
        "/mcp",
        headers,
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    let mut services = catalog.services;
    if auth.chat.is_some() {
        services.push(crate::services::assistant_account_tools::virtual_service());
    }
    let expected: Vec<Value> = mcp_service::generate_tool_definitions(&services, None)
        .iter()
        .filter(|t| !(super::is_scoped_api_key(&auth) && super::SSH_META_TOOL_NAMES.contains(&t.name.as_str())))
        .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.input_schema}))
        .collect();
    assert_eq!(listing["result"]["tools"], json!(expected), "{listing}");
}

#[tokio::test]
async fn general_keys_read_mounted_specs_and_match_stateless_tools_list() {
    let Some(f) = Fixture::new("mcp_config_specs").await else {
        eprintln!("skipping MCP config specs: MongoDB unavailable");
        return;
    };
    let _cache_guard = SpecCacheTestGuard::acquire();
    let services = f.services().await;
    for (prefix, allow_all_nodes, platform) in
        [("nyx_", true, None), ("nyxid_ag_", false, Some("codex"))]
    {
        let key = f.key("proxy", None, true, allow_all_nodes, platform).await;
        assert!(key.full_key.starts_with(prefix));
        let headers = key_headers(&key.full_key);
        let (status, _, body) = request(&f.state, "GET", CONFIG, &headers, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["total_services"], 2);
        for service in &services {
            let row = body["services"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["service_id"] == service.id)
                .unwrap();
            assert_eq!(row["is_generic_proxy"], false);
            assert_eq!(row["endpoints"].as_array().unwrap().len(), 1);
            assert_eq!(row["endpoints"][0]["name"], format!("{}_op", service.slug));
            assert_eq!(row["endpoints"][0]["path"], format!("/{}", service.slug));
        }
        assert!(!body.to_string().contains("template_op"));
        assert_parity(&f.state, &headers, &body).await;
        let (status, _, bearer) = request(
            &f.state,
            "GET",
            CONFIG,
            &bearer_headers(&key.full_key),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{bearer}");
        assert_eq!(body["catalog_digest"], bearer["catalog_digest"]);

        for path in [
            "/api/v1/keys".into(),
            format!("/api/v1/keys/{}", services[0].id),
            format!("/api/v1/keys/{}/authorization", services[0].id),
            format!(
                "/api/v1/endpoints/{}/openapi-endpoints",
                services[0].endpoint_id
            ),
        ] {
            let (status, _, inventory) = request(&f.state, "GET", &path, &headers, None).await;
            assert_eq!(status, StatusCode::OK, "{path}: {inventory}");
        }
        let (status, _, denied) =
            request(&f.state, "POST", "/api/v1/keys", &headers, Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            denied["message"],
            "Forbidden: API keys cannot access this endpoint"
        );
    }
}

#[tokio::test]
async fn restricted_and_auto_connected_scopes_match_mcp_without_provisioning() {
    let Some(f) = Fixture::new("mcp_config_scope").await else {
        eprintln!("skipping MCP config scopes: MongoDB unavailable");
        return;
    };
    let _cache_guard = SpecCacheTestGuard::acquire();
    let [custom, auto] = f.services().await;
    f.state
        .db
        .collection::<UserService>(SERVICES)
        .update_one(
            doc! {"_id": &auto.id},
            doc! {"$set": {"source": crate::models::user_service::AUTO_PROVISION_SOURCE}},
        )
        .await
        .unwrap();
    let key = f
        .key(
            "proxy",
            Some(std::slice::from_ref(&custom.id)),
            false,
            false,
            Some("codex"),
        )
        .await;
    // A platform ID on an ordinary key must not grant it platform discovery.
    f.state
        .db
        .collection::<ApiKey>(KEYS)
        .update_one(
            doc! {"_id": &key.id},
            doc! {"$set": {
                "allowed_platform_service_ids": [auto.catalog_service_id.as_ref().unwrap()],
            }},
        )
        .await
        .unwrap();
    for allow_auto in [false, true] {
        f.state
            .db
            .collection::<ApiKey>(KEYS)
            .update_one(
                doc! {"_id": &key.id},
                doc! {"$set": {"allow_auto_connected_services": allow_auto}},
            )
            .await
            .unwrap();
        for headers in [key_headers(&key.full_key), bearer_headers(&key.full_key)] {
            let (status, _, body) = request(&f.state, "GET", CONFIG, &headers, None).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["diagnostics"]["service_scope_restricted"], true);
            assert_eq!(body["diagnostics"]["node_scope_restricted"], true);
            let ids: Vec<&str> = body["services"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["service_id"].as_str().unwrap())
                .collect();
            assert!(ids.contains(&custom.id.as_str()));
            assert_eq!(ids.contains(&auto.id.as_str()), allow_auto);
            assert_eq!(ids.len(), if allow_auto { 2 } else { 1 });
            assert_parity(&f.state, &key_headers(&key.full_key), &body).await;
        }
    }
    assert_eq!(
        f.state
            .db
            .collection::<UserService>(SERVICES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn keys_without_proxy_scope_and_scheduled_keys_are_rejected() {
    let Some(f) = Fixture::new("mcp_config_denied_keys").await else {
        eprintln!("skipping MCP config denied keys: MongoDB unavailable");
        return;
    };
    let read = f.key("read", None, true, false, Some("codex")).await;
    let scheduled = f.key("proxy", None, true, false, Some("codex")).await;
    f.state
        .db
        .collection::<ApiKey>(KEYS)
        .update_one(
            doc! {"_id": &scheduled.id},
            doc! {"$set": {"purpose": "scheduled_invocation", "scheduled_write_enabled": true}},
        )
        .await
        .unwrap();
    for (key, message) in [
        (&read, "Missing required scope for proxy access"),
        (&scheduled, "scheduled_invocation API keys are restricted"),
    ] {
        for headers in [key_headers(&key.full_key), bearer_headers(&key.full_key)] {
            let (status, _, body) = request(&f.state, "GET", CONFIG, &headers, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            assert!(
                body["message"].as_str().unwrap().contains(message),
                "{body}"
            );
        }
    }
}

#[tokio::test]
async fn human_access_remains_allowed_but_delegated_and_relay_are_rejected() {
    let Some(f) = Fixture::new("mcp_config_tokens").await else {
        eprintln!("skipping MCP config tokens: MongoDB unavailable");
        return;
    };
    let owner = Uuid::parse_str(&f.owner).unwrap();
    let human = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &owner,
        "proxy",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (status, _, body) = request(&f.state, "GET", CONFIG, &bearer_headers(&human), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let key = f.key("proxy", None, true, false, Some("codex")).await;
    let relay = jwt::generate_relay_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &owner,
        "proxy account:read",
        None,
        &jwt::RelayAgentScope {
            api_key_id: key.id,
            api_key_name: key.name,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
        },
    )
    .unwrap();
    let delegated = jwt::generate_delegated_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &owner,
        "proxy account:read",
        "mcp-reader",
        60,
        None,
    )
    .unwrap();
    for (token, message) in [
        (relay, "Relay tokens cannot access this endpoint"),
        (delegated, "Delegated tokens cannot access this endpoint"),
    ] {
        let (status, _, body) =
            request(&f.state, "GET", CONFIG, &bearer_headers(&token), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["message"], format!("Forbidden: {message}"));
    }
}

#[tokio::test]
async fn service_account_purpose_scope_and_revocation_match_mcp() {
    let Some(f) = Fixture::new("mcp_config_sa").await else {
        eprintln!("skipping MCP config service accounts: MongoDB unavailable");
        return;
    };
    use crate::models::service_account::COLLECTION_NAME as ACCOUNTS;
    use crate::services::service_account_service as accounts;
    let (sa, secret) = accounts::create_service_account(
        &f.state.db,
        "Discovery",
        None,
        "proxy read",
        &[],
        None,
        &f.owner,
    )
    .await
    .unwrap();
    let token = accounts::authenticate_client_credentials(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &sa.client_id,
        &secret,
        Some("proxy"),
    )
    .await
    .unwrap()
    .access_token;
    let headers = bearer_headers(&token);
    let (status, _, body) = request(&f.state, "GET", CONFIG, &headers, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user_id"], sa.id);
    assert_parity(&f.state, &headers, &body).await;
    let read = accounts::authenticate_client_credentials(
        &f.state.db,
        &f.state.config,
        &f.state.jwt_keys,
        &sa.client_id,
        &secret,
        Some("read"),
    )
    .await
    .unwrap()
    .access_token;
    assert_eq!(
        request(&f.state, "GET", CONFIG, &bearer_headers(&read), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        authenticate_mcp(&f.state, &bearer_headers(&read), false)
            .await
            .unwrap_err()
            .status(),
        StatusCode::FORBIDDEN
    );

    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(
            doc! {"_id": &sa.id},
            doc! {"$set": {
                "purpose": "curation", "platform_protected": true,
                "curation_grant": {
                    "id": Uuid::new_v4().to_string(), "service_ids": [],
                    "issued_by": &f.owner, "issued_at": bson::DateTime::now(),
                    "max_writes": 10_i64, "window_seconds": 3600_i64,
                    "window_started_at": bson::DateTime::now(), "writes_used": 0_i64,
                },
            }},
        )
        .await
        .unwrap();
    let (status, _, denied) = request(&f.state, "GET", CONFIG, &headers, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert!(
        denied["message"]
            .as_str()
            .unwrap()
            .contains("Curation accounts can only curate")
    );
    assert_eq!(
        authenticate_mcp(&f.state, &headers, false)
            .await
            .unwrap_err()
            .status(),
        StatusCode::FORBIDDEN
    );
    f.state
        .db
        .collection::<Document>(ACCOUNTS)
        .update_one(doc! {"_id": &sa.id}, doc! {"$set": {"purpose": "general"}})
        .await
        .unwrap();
    accounts::rotate_secret(&f.state.db, &sa.id, true)
        .await
        .unwrap();
    assert_eq!(
        request(&f.state, "GET", CONFIG, &headers, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        authenticate_mcp(&f.state, &headers, false)
            .await
            .unwrap_err()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn chat_catalog_matches_mcp_before_and_after_acknowledgement() {
    use crate::services::{
        assistant_agent_credential_service as credentials, assistant_authority_tests,
    };
    let f = assistant_authority_tests::fixture("mcp_config_chat").await;
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let service_id = assistant_authority_tests::connected(
        &f.state.db,
        &f.owner,
        "chat-custom",
        "https://example.com",
    )
    .await;
    let mut catalog = test_auto_connected_catalog_service();
    catalog.service_category = "internal".into();
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .insert_one(&catalog)
        .await
        .unwrap();
    f.state.db.collection::<Document>(OPERATIONS).insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "service_id": &catalog.id,
        "name": "platform_op", "method": "GET", "path": "/platform",
        "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    let headers = key_headers(&key.raw_key);
    let mut previous_digest = Value::Null;
    for acknowledged in [false, true] {
        if acknowledged {
            f.state
                .db
                .collection::<ApiKey>(KEYS)
                .update_one(
                    doc! {"_id": &f.row.credential_api_key_id},
                    doc! {"$set": {"allowed_platform_service_ids": [&catalog.id],
                    "allowed_service_ids": [&service_id], "scopes": "proxy assistant:account"}},
                )
                .await
                .unwrap();
        }
        let auth = authenticate_mcp(&f.state, &headers, false).await.unwrap();
        assert!(auth.chat.is_some());
        assert_eq!(auth.account_acknowledged, acknowledged);
        assert_eq!(
            auth.allowed_platform_service_ids.contains(&catalog.id),
            acknowledged
        );
        let (status, _, body) = request(&f.state, "GET", CONFIG, &headers, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["total_services"], 2, "{body}");
        assert_parity(&f.state, &headers, &body).await;
        if acknowledged {
            assert_eq!(body["catalog_digest"], previous_digest);
        }
        previous_digest = body["catalog_digest"].clone();
        assert_eq!(
            request(&f.state, "POST", "/api/v1/keys", &headers, Some(json!({})))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0,
        "discovery must not create acknowledgement cards"
    );
}

#[tokio::test]
async fn dangling_chat_key_is_unauthorized_on_rest_and_mcp() {
    use crate::services::{
        assistant_agent_credential_service as credentials, assistant_authority_tests,
    };
    let f = assistant_authority_tests::fixture("mcp_config_dangling_chat").await;
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let deleted = f
        .state
        .db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .delete_one(doc! {"_id": &f.row.id})
        .await
        .unwrap();
    assert_eq!(deleted.deleted_count, 1);
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
            .count_documents(doc! {"api_key_id": &f.row.credential_api_key_id})
            .await
            .unwrap(),
        1
    );

    for headers in [key_headers(&key.raw_key), bearer_headers(&key.raw_key)] {
        let (status, _, body) = request(&f.state, "GET", CONFIG, &headers, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(
            body["message"],
            "Unauthorized: Invalid authentication credentials"
        );
    }
    assert_eq!(
        authenticate_mcp(&f.state, &key_headers(&key.raw_key), false)
            .await
            .unwrap_err()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn discovery_keeps_inventory_attribution_and_rate_limit_behavior() {
    let Some(f) = Fixture::new("mcp_config_attribution").await else {
        eprintln!("skipping MCP config attribution: MongoDB unavailable");
        return;
    };
    let key = f.key("proxy", None, true, false, Some("codex")).await;
    f.state
        .db
        .collection::<ApiKey>(KEYS)
        .update_one(
            doc! {"_id": &key.id},
            doc! {"$set": {"rate_limit_per_second": 1, "rate_limit_burst": 1}},
        )
        .await
        .unwrap();
    let mut headers = key_headers(&key.full_key);
    headers.insert("user-agent", "mcp-config-router-test".parse().unwrap());
    use axum::extract::FromRequestParts;
    let (mut parts, ()) = Request::builder()
        .uri(CONFIG)
        .body(())
        .unwrap()
        .into_parts();
    parts.headers = headers.clone();
    let auth = crate::mw::auth::AuthUser::from_request_parts(&mut parts, &f.state)
        .await
        .unwrap();
    assert_eq!(auth.api_key_id.as_deref(), Some(key.id.as_str()));
    assert_eq!(auth.api_key_name.as_deref(), Some(key.name.as_str()));
    assert_eq!(auth.user_agent.as_deref(), Some("mcp-config-router-test"));
    assert_eq!(auth.rate_limit_per_second, Some(1));
    assert_eq!(auth.rate_limit_burst, Some(1));
    // Discovery GETs do not invoke the execution limiter or add proxy headers/audits.
    for path in [CONFIG, "/api/v1/keys", CONFIG, "/api/v1/keys"] {
        let (status, headers, body) = request(&f.state, "GET", path, &headers, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(!headers.contains_key("x-nyxid-agent-id"));
    }
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::coordination::TOKEN_BUCKET_COLLECTION_NAME)
            .count_documents(doc! {"namespace": "agent"})
            .await
            .unwrap(),
        0
    );
}
