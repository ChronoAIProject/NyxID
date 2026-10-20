use super::*;
use crate::services::{mcp_service, proxy_service};
use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

pub(super) async fn call(
    f: &Fixture,
    state: &AppState,
    conversation_key: bool,
) -> AppResult<mcp_service::ToolResponse> {
    let services =
        mcp_service::load_user_tools_all(&state.db, &state.node_ws_manager, &f.subject).await?;
    let service = services
        .iter()
        .find(|service| service.service_id == f.connection)
        .unwrap();
    let endpoint = service
        .endpoints
        .iter()
        .find(|endpoint| endpoint.name == "list_agents")
        .expect("catalog publishes the declared list-agents GET operation");
    assert!(!service.is_generic_proxy);
    let prepared =
        mcp_service::prepare_proxy_tool_call(service, endpoint, &serde_json::json!({"limit":1}))?;
    let key_id = conversation_key.then_some(f.key.id.as_str());
    let chat =
        crate::services::assistant_acknowledgement_service::for_key(&state.db, &f.subject, key_id)
            .await?;
    let context = mcp_service::McpExecContext {
        assistant_chat: chat.as_ref(),
        response_body_limit: None,
        actor_user_id: Some(&f.subject),
        caller_token: None,
        delegation_restrictions: Default::default(),
        attribution: None,
        org_agent_access: None,
        agent_owner: None,
        operation_scopes: None,
        api_key_id: key_id,
        api_key_credential_id: None,
        allow_all_nodes: true,
        allowed_node_ids: &[],
    };
    mcp_service::execute_tool_response(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        &f.subject,
        &f.subject,
        service,
        endpoint,
        prepared,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &context,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    )
    .await
}

async fn rest(f: &Fixture) -> AppResult<axum::response::Response> {
    rest_path(f, "/api/v1/agents?limit=1").await
}

pub(super) async fn rest_path(
    f: &Fixture,
    path_and_query: &str,
) -> AppResult<axum::response::Response> {
    let (path, query) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    let separator = if query.is_empty() { "" } else { "&" };
    let key = f
        .state
        .db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id":&f.key.id})
        .await?
        .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None).await?;
    let mut request = axum::http::Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/proxy/{}{}?{}{}_nyxid_via={}",
            f.catalog, path, query, separator, f.connection
        ))
        .header(HEADER, "caller-spoof")
        .body(axum::body::Body::empty())
        .unwrap();
    request.extensions_mut().insert(
        crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Proxy,
        ),
    );
    crate::handlers::proxy::proxy_request(
        axum::extract::State(f.state.clone()),
        auth,
        Default::default(),
        axum::extract::Path((f.catalog.clone(), path.trim_start_matches('/').into())),
        request,
    )
    .await
}

#[tokio::test]
async fn actual_nyxbot_mcp_and_rest_egress_mint_exact_proofs_without_redirects() {
    let f = Fixture::new().await;
    let captured = Arc::new(Mutex::new(None::<HeaderMap>));
    let redirects = Arc::new(AtomicUsize::new(0));
    let seen = captured.clone();
    let followed = redirects.clone();
    let router = Router::new()
        .route(
            "/api/v1/agents",
            get(move |headers: HeaderMap| {
                let seen = seen.clone();
                async move {
                    *seen.lock().unwrap() = Some(headers);
                    (StatusCode::FOUND, [("location", "/other")], "read redirect")
                }
            }),
        )
        .route(
            "/other",
            get(move || {
                let followed = followed.clone();
                async move {
                    followed.fetch_add(1, Ordering::SeqCst);
                    "followed"
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    f.state
        .db
        .collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id":&f.catalog},
            doc! {"$set":{"base_url":&base_url}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(ENDPOINTS)
        .update_one(doc! {"user_id":&f.subject}, doc! {"$set":{"url":&base_url}})
        .await
        .unwrap();

    let response = call(&f, &f.state, true).await.unwrap();
    assert_eq!(response.status, 302, "{}", response.text);
    assert_eq!(redirects.load(Ordering::SeqCst), 0);
    let headers = captured.lock().unwrap().take().unwrap();
    assert_eq!(headers.get_all(HEADER).iter().count(), 1);
    let token = headers[HEADER].to_str().unwrap().to_owned();
    let descriptor =
        ReadDescriptor::from_request(&request("GET", "/api/v1/agents?limit=1")).unwrap();
    assert!(
        introspect(&f.state, &f.client, &f.catalog, &token, descriptor.clone())
            .await
            .is_ok()
    );
    let response = rest(&f).await.unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(redirects.load(Ordering::SeqCst), 0);
    let headers = captured.lock().unwrap().take().unwrap();
    assert_eq!(headers.get_all(HEADER).iter().count(), 1);
    assert!(
        introspect(
            &f.state,
            &f.client,
            &f.catalog,
            headers[HEADER].to_str().unwrap(),
            ReadDescriptor::from_request(&request("GET", "/api/v1/agents?limit=1")).unwrap()
        )
        .await
        .is_ok()
    );

    // Human/ordinary proxy calls retain their identity JWT and do not acquire
    // request authority. Disabling the feature keeps the prior transport path.
    for (conversation_key, enabled) in [(false, true), (true, false)] {
        let mut state = f.state.clone();
        state.config.resource_request_proof_enabled = enabled;
        let response = call(&f, &state, conversation_key).await.unwrap();
        assert_eq!(response.status, 200, "{}", response.text);
        let headers = captured.lock().unwrap().take().unwrap();
        assert!(!headers.contains_key(HEADER));
        assert!(headers.contains_key("x-nyxid-identity-token"));
    }
    assert_eq!(redirects.load(Ordering::SeqCst), 2);

    // These rows are keyed by catalog, never by the connection selected by
    // MCP. Revoking policy after issuance must invalidate the same exact read
    // and prevent a new MCP dispatch, even though the parent key still works.
    let policies = f
        .state
        .db
        .collection::<Document>(crate::models::service_approval_config::COLLECTION_NAME);
    let policy = Uuid::new_v4().to_string();
    for effect in ["deny", "require_approval"] {
        let now = mongodb::bson::DateTime::now();
        policies.update_one(doc! {"_id":&policy},doc! {"$set":{
            "user_id":&f.subject,"service_id":&f.catalog,"service_name":"CMA",
            "approval_required":false,"default_effect":effect,"created_at":now,"updated_at":now,
        }}).upsert(true).await.unwrap();
        assert!(
            introspect(&f.state, &f.client, &f.catalog, &token, descriptor.clone())
                .await
                .is_err()
        );
        assert!(call(&f, &f.state, true).await.is_err());
        assert!(
            captured.lock().unwrap().is_none(),
            "policy rejection must precede HTTP dispatch"
        );
        assert_eq!(redirects.load(Ordering::SeqCst), 2);
    }
    server.abort();
}

#[tokio::test]
async fn all_header_layers_cannot_spoof_resource_request_authority() {
    let f = Fixture::new().await;
    let mut target = f.target().await;
    target.catalog_default_headers = vec![
        crate::models::default_request_header::ResolvedDefaultRequestHeader {
            name: HEADER.into(),
            value: "catalog-spoof".to_owned().into(),
            overridable: false,
            sensitive: false,
        },
    ];
    target.user_service_default_headers = target.catalog_default_headers.clone();
    let spoof = vec![(HEADER.to_owned(), "caller-spoof".into())];
    let headers = proxy_service::build_effective_outbound_headers(
        &target,
        spoof.clone(),
        &spoof,
        &spoof,
        &spoof,
    );
    assert!(
        !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(HEADER))
    );
}
