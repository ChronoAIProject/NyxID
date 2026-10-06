//! Real MCP tools/call and REST proxy wire parity. Tokens stay inside the test
//! collector; neither the downstream response nor diagnostics contain them.
use super::*;
use crate::crypto::jwt::TokenRestrictionClaims;
use crate::models::{
    downstream_service::DownstreamService, user::UserType, user_endpoint::UserEndpoint,
    user_service::UserService,
};
use crate::services::billing::route_inventory::{BillingIngress, BillingRoutePolicy};
use crate::test_utils::*;
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::{any, post},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    owner: String,
    service: String,
    node: String,
    key_id: String,
    key: String,
    headers: Arc<Mutex<Vec<HeaderMap>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let db = connect_test_database("mcp_delegation")
            .await
            .expect("MongoDB required");
        let state = test_app_state(db.clone());
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let headers = Arc::new(Mutex::new(Vec::new()));
        let captured = headers.clone();
        let upstream = Router::new().route(
            "/{*path}",
            any(move |headers: HeaderMap| {
                captured.lock().unwrap().push(headers);
                async { axum::Json(json!({"ok": true})) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.slug = "chrono-sandbox".into();
        catalog.base_url = url.clone();
        catalog.auth_method = "none".into();
        catalog.requires_user_credential = false;
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();
        db.collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": uuid::Uuid::new_v4().to_string(), "service_id": &catalog.id,
                "name":"execute", "method":"POST", "path":"/execute", "is_active":true,
                "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
            })
            .await
            .unwrap();
        let service = uuid::Uuid::new_v4().to_string();
        let endpoint = uuid::Uuid::new_v4().to_string();
        db.collection::<UserEndpoint>(crate::models::user_endpoint::COLLECTION_NAME)
            .insert_one(test_user_endpoint(
                &endpoint,
                &owner,
                "Sandbox",
                &url,
                None,
                Some(&catalog.id),
            ))
            .await
            .unwrap();
        let mut row = test_user_service(
            &service,
            &owner,
            "sandbox-alias",
            &endpoint,
            Some(&catalog.id),
            None,
        );
        row.inject_delegation_token = true;
        row.delegation_token_scope = "proxy:* llm:proxy".into();
        db.collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
            .insert_one(row)
            .await
            .unwrap();
        let (_, registration, _) = crate::services::node_service::create_registration_token(
            &db, &owner, "parity", 10, 600,
        )
        .await
        .unwrap();
        let (node, _, _) = crate::services::node_service::register_node(
            &db,
            &state.encryption_keys,
            &registration,
            None,
        )
        .await
        .unwrap();
        let key = crate::services::key_service::create_api_key(
            &db,
            &owner,
            "restricted",
            "proxy",
            None,
            None,
            Some(std::slice::from_ref(&service)),
            Some(std::slice::from_ref(&node.id)),
            Some(false),
            Some(false),
            Some(false),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        Self {
            state,
            owner,
            service,
            node: node.id,
            key_id: key.id,
            key: key.full_key,
            headers,
            server,
        }
    }
    fn router(&self) -> Router {
        Router::new()
            .merge(
                Router::new()
                    .route("/mcp", post(mcp_post))
                    .layer(Extension(BillingRoutePolicy::Metered(BillingIngress::Mcp))),
            )
            .merge(
                Router::new()
                    .route(
                        "/api/v1/proxy/s/{service_slug}/{*path}",
                        any(crate::handlers::proxy::proxy_request_by_slug),
                    )
                    .layer(Extension(BillingRoutePolicy::Metered(
                        BillingIngress::Proxy,
                    ))),
            )
            .with_state(self.state.clone())
    }
    fn key_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", self.key.parse().unwrap());
        headers
    }
    async fn call(&self, mcp: bool, universal: bool, headers: &HeaderMap) {
        let body = if mcp {
            let arguments = json!({});
            let params = if universal {
                json!({"name":"nyx__call_tool", "arguments":{"tool_name":"sandbox-alias__execute", "arguments_json":arguments.to_string()}})
            } else {
                json!({"name":"sandbox-alias__execute", "arguments":arguments})
            };
            json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":params})
        } else {
            json!({})
        };
        let mut request = Request::builder()
            .method("POST")
            .uri(if mcp {
                "/mcp"
            } else {
                "/api/v1/proxy/s/sandbox-alias/execute"
            })
            .body(Body::from(body.to_string()))
            .unwrap();
        *request.headers_mut() = headers.clone();
        request
            .headers_mut()
            .insert("content-type", "application/json".parse().unwrap());
        let response = Box::pin(self.router().oneshot(request)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
        if mcp {
            assert!(body.get("error").is_none(), "{body}");
            assert_eq!(body["result"]["isError"], false, "{body}");
        }
    }
    fn take_claims(&self) -> Option<Value> {
        let headers = self
            .headers
            .lock()
            .unwrap()
            .pop()
            .expect("one downstream request");
        let token = headers.get("x-nyxid-delegation-token")?;
        let claims = jwt::verify_token(
            &self.state.jwt_keys,
            &self.state.config,
            token.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(claims.exp - claims.iat, jwt::MCP_DELEGATION_TOKEN_TTL_SECS);
        let mut value = serde_json::to_value(claims).unwrap();
        // Per-request nonce and clock are intentionally different; every other
        // claim, including absent optional claims, must be identical.
        for field in ["jti", "iat", "exp"] {
            value.as_object_mut().unwrap().remove(field);
        }
        Some(value)
    }
    async fn assert_pair(&self, headers: &HeaderMap) -> Value {
        self.call(false, false, headers).await;
        let rest = self.take_claims().expect("REST delegation header");
        for universal in [false, true] {
            self.call(true, universal, headers).await;
            assert_eq!(self.take_claims().expect("MCP delegation header"), rest);
        }
        rest
    }
}

#[tokio::test]
async fn mcp_delegation_restricted_agent_matches_rest_and_flag_off_omits_header() {
    let f = Box::pin(Fixture::new()).await;
    let claims = Box::pin(f.assert_pair(&f.key_headers())).await;
    assert_eq!(claims["sub"], f.owner);
    assert_eq!(claims["act"]["sub"], "chrono-sandbox");
    assert_eq!(claims["scope"], "proxy:* llm:proxy");
    assert_eq!(claims["allowed_service_ids"], json!([f.service]));
    assert_eq!(claims["allowed_node_ids"], json!([f.node]));
    assert_eq!(claims["allow_all_services"], false);
    assert_eq!(claims["allow_all_nodes"], false);
    f.state
        .db
        .collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.service},
            doc! {"$set":{"inject_delegation_token":false}},
        )
        .await
        .unwrap();
    for mcp in [false, true] {
        Box::pin(f.call(mcp, false, &f.key_headers())).await;
        assert!(f.take_claims().is_none());
    }
}

#[tokio::test]
async fn mcp_delegation_relay_and_delegated_restrictions_match_rest() {
    let f = Box::pin(Fixture::new()).await;
    let subject = uuid::Uuid::parse_str(&f.owner).unwrap();
    let relay = jwt::generate_relay_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &subject,
        "proxy",
        None,
        &jwt::RelayAgentScope {
            api_key_id: f.key_id.clone(),
            api_key_name: "restricted".into(),
            allowed_service_ids: vec![f.service.clone()],
            allowed_node_ids: vec![f.node.clone()],
            allow_all_services: false,
            allow_all_nodes: false,
        },
    )
    .unwrap();
    let restrictions = TokenRestrictionClaims {
        resources: Some(vec![format!("{}/mcp", f.state.config.base_url)]),
        allowed_service_ids: Some(vec![f.service.clone()]),
        allowed_node_ids: Some(vec![f.node.clone()]),
        allow_all_services: Some(false),
        allow_all_nodes: Some(false),
    };
    let delegated = jwt::generate_delegated_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &subject,
        "proxy:*",
        "downstream",
        300,
        Some(&restrictions),
    )
    .unwrap();
    for (token, expected_resources) in [(relay, None), (delegated, restrictions.resources)] {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        let claims = Box::pin(f.assert_pair(&headers)).await;
        assert_eq!(
            claims.get("resources").cloned(),
            expected_resources.map(|value| json!(value))
        );
    }
}

#[tokio::test]
async fn mcp_delegation_scheduled_key_cannot_bypass_durable_admission() {
    let f = Box::pin(Fixture::new()).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.key_id},
            doc! {"$set": {"purpose": "scheduled_invocation"}},
        )
        .await
        .unwrap();
    // Neither route may send a request or token without durable admission.
    // MCP has no such protocol; REST requires the durable-grant headers.
    for mcp in [false, true] {
        let mut request = Request::builder()
            .method("POST")
            .uri(if mcp {
                "/mcp"
            } else {
                "/api/v1/proxy/s/sandbox-alias/execute"
            })
            .body(Body::from(if mcp {
                json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": "sandbox-alias__execute", "arguments": {}}})
                .to_string()
            } else {
                "{}".into()
            }))
            .unwrap();
        *request.headers_mut() = f.key_headers();
        request
            .headers_mut()
            .insert("content-type", "application/json".parse().unwrap());
        let response = Box::pin(f.router().oneshot(request)).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
        assert_eq!(body["error_code"], if mcp { 9009 } else { 9008 });
        assert!(f.headers.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn mcp_delegation_node_headers_match_rest() {
    use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse};
    let f = Box::pin(Fixture::new()).await;
    f.state
        .db
        .collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.service}, doc! {"$set":{"node_id":&f.node}})
        .await
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    register_test_node_connection(&f.state, &f.node, tx).await;
    let captured = f.headers.clone();
    let manager = f.state.node_ws_manager.clone();
    let node = f.node.clone();
    let responder = tokio::spawn(async move {
        for _ in 0..3 {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let NodeOutboundMessage::Text(text) = frame else {
                panic!("expected proxy frame")
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            let mut headers = HeaderMap::new();
            for (name, value) in value["headers"].as_object().unwrap() {
                headers.insert(
                    axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    value.as_str().unwrap().parse().unwrap(),
                );
            }
            captured.lock().unwrap().push(headers);
            manager.deliver_proxy_response(
                &node,
                NodeProxyResponse {
                    request_id: value["request_id"].as_str().unwrap().into(),
                    status: 200,
                    headers: vec![],
                    body: br#"{"ok":true}"#.to_vec(),
                },
            );
        }
    });
    Box::pin(f.assert_pair(&f.key_headers())).await;
    responder.await.unwrap();
}

#[test]
fn mcp_delegation_projection_matches_auth_user_for_all_caller_kinds() {
    for kind in [
        AuthMethod::ApiKey,
        AuthMethod::AccessToken,
        AuthMethod::Session,
        AuthMethod::ServiceAccount,
        AuthMethod::Delegated,
        AuthMethod::Relay,
    ] {
        let owner = uuid::Uuid::new_v4().to_string();
        let mut rest = test_auth_user(&owner);
        rest.auth_method = kind.clone();
        rest.scope = format!("proxy {}", auth::MCP_CATALOG_READ_SCOPE);
        rest.allow_all_services = false;
        rest.allow_all_nodes = false;
        rest.allowed_service_ids = vec![uuid::Uuid::new_v4().to_string()];
        rest.allowed_node_ids = vec![uuid::Uuid::new_v4().to_string()];
        let mut mcp = McpAuthContext::user(owner, kind);
        mcp.scope = rest.scope.clone();
        mcp.allow_all_services = rest.allow_all_services;
        mcp.allow_all_nodes = rest.allow_all_nodes;
        mcp.allowed_service_ids = rest.allowed_service_ids.clone();
        mcp.allowed_node_ids = rest.allowed_node_ids.clone();
        for resources in [None, Some(vec!["https://nyx.example/mcp".into()])] {
            rest.resource_uris = resources.clone();
            mcp.resource_uris = resources;
            let actual = mcp_exec_context(&mcp).delegation_restrictions;
            let expected = TokenRestrictionClaims::from_auth_user(&rest);
            assert_eq!(actual.resources, expected.resources);
            assert_eq!(actual.allowed_service_ids, expected.allowed_service_ids);
            assert_eq!(actual.allow_all_services, expected.allow_all_services);
            assert_eq!(actual.allowed_node_ids, expected.allowed_node_ids);
            assert_eq!(actual.allow_all_nodes, expected.allow_all_nodes);
        }
    }
}

#[tokio::test]
async fn mcp_delegation_service_account_keeps_its_subject_not_its_owner() {
    use crate::services::service_account_service as accounts;
    let f = Box::pin(Fixture::new()).await;
    let (sa, secret) = accounts::create_service_account(
        &f.state.db,
        "Parity caller",
        None,
        "proxy",
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
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    // Authentication and execution retain the SA subject; its owner is only
    // the credential/billing principal, exactly as on REST.
    let sid = f
        .state
        .mcp_sessions
        .create_with_proxy_access(&sa.id, true)
        .await
        .unwrap()
        .unwrap();
    headers.insert("mcp-session-id", sid.parse().unwrap());
    Box::pin(f.call(false, false, &headers)).await;
    let rest = f.take_claims().unwrap();
    assert_eq!(rest["sub"], sa.id);
    assert_eq!(rest["allow_all_services"], true);
    let auth = authenticate_mcp(&f.state, &headers, false)
        .await
        .ok()
        .unwrap();
    let expected = crate::services::identity_service::generate_proxy_delegation_token(
        &f.state.jwt_keys,
        &f.state.config,
        &uuid::Uuid::parse_str(&auth.user_id).unwrap(),
        "proxy:* llm:proxy",
        "sandbox-alias",
        Some("chrono-sandbox"),
        Some(&mcp_exec_context(&auth).delegation_restrictions),
    )
    .unwrap();
    let claims = jwt::verify_token(&f.state.jwt_keys, &f.state.config, &expected).unwrap();
    assert_eq!(claims.sub, sa.id);
    assert_eq!(claims.scope, rest["scope"]);
    assert_eq!(claims.allow_all_services, Some(true));
    assert_eq!(claims.allow_all_nodes, Some(true));
}
