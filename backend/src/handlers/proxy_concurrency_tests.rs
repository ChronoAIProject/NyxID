// Included in proxy::proxy_resolution_integration_tests to reuse REST/node fixtures.
#[tokio::test]
async fn service_concurrency_rest_slug_uuid_mcp_and_stream_share_catalog_capacity() {
    use crate::models::service_concurrency::ServiceConcurrencyPolicy;
    use crate::services::{mcp_service, service_concurrency_service as concurrency};
    let db = crate::test_utils::connect_transaction_test_database("concurrency_dispatch").await;
    concurrency::ensure_indexes(&db).await.unwrap();
    let actor = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&actor, UserType::Person))
        .await
        .unwrap();
    let (base, server) = start_downstream().await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.base_url = base.clone();
    catalog.concurrency_policy = Some(ServiceConcurrencyPolicy {
        service_id: catalog.id.clone(),
        default_limit: Some(1),
        users: vec![],
        orgs: vec![],
    });
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&catalog)
    .await
    .unwrap();
    // Catalog-backed MCP discovery requires a published operation contract.
    let endpoint = crate::models::service_endpoint::ServiceEndpoint {
        id: Uuid::new_v4().to_string(),
        service_id: catalog.id.clone(),
        name: "status".into(),
        description: None,
        method: "GET".into(),
        path: "/status".into(),
        target_id: None,
        parameters: None,
        request_body_schema: None,
        request_content_type: None,
        request_body_required: false,
        response_description: None,
        response: Default::default(),
        risk: Some(crate::models::service_endpoint::EndpointRisk::Read),
        supports_idempotency_key: false,
        is_active: true,
        operation_generation: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.collection(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(endpoint)
        .await
        .unwrap();
    let connection = insert_user_service(&db, &actor, "limited", &base, Some(&catalog.id)).await;
    let state = test_app_state(db.clone());
    let auth = crate::test_utils::test_auth_user(&actor);
    let mut slug = String::new();
    let response = proxy_request_by_slug_inner(
        &state,
        &auth,
        &connection.slug,
        "status",
        proxy_request("/proxy/s/limited/status"),
        &mut slug,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(matches!(
        proxy_request_inner(
            &state,
            &auth,
            &catalog.id,
            "status",
            proxy_request(&format!("/proxy/{}/status", catalog.id)),
            &mut slug
        )
        .await,
        Err(AppError::ServiceConcurrencyLimited)
    ));
    let services = mcp_service::load_user_tools(&db, &state.node_ws_manager, &actor)
        .await
        .unwrap();
    let service = services
        .iter()
        .find(|s| s.service_id == connection.id)
        .expect("discover catalog connection");
    let endpoint = service
        .endpoints
        .first()
        .expect("discover status operation");
    let prepared =
        mcp_service::prepare_proxy_tool_call(service, endpoint, &serde_json::json!({})).unwrap();
    let permit = crate::services::billing::route_inventory::enforce_billing_egress_classification(
        Some(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Mcp,
            ),
        ),
        crate::services::billing::BillingIngress::Mcp,
    )
    .unwrap();
    let ctx = mcp_service::McpExecContext {
        actor_user_id: None,
        caller_token: None,
        delegation_restrictions: Default::default(),
        attribution: None,
        org_agent_access: None,
        agent_owner: None,
        operation_scopes: None,
        api_key_id: None,
        allow_all_nodes: true,
        allowed_node_ids: &[],
    };
    let blocked = Box::pin(mcp_service::execute_tool_response(
        &state.http_client,
        &db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        &actor,
        &actor,
        service,
        endpoint,
        prepared,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &ctx,
        permit,
    ))
    .await;
    assert!(matches!(blocked, Err(AppError::ServiceConcurrencyLimited)));
    // Response-body ownership, not receipt of headers, releases the HTTP slot.
    drop(response);
    concurrency_wait_empty(&db, &catalog.id, &actor).await;
    let prepared =
        mcp_service::prepare_proxy_tool_call(service, endpoint, &serde_json::json!({})).unwrap();
    let completed = Box::pin(mcp_service::execute_tool_response(
        &state.http_client,
        &db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        &actor,
        &actor,
        service,
        endpoint,
        prepared,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &ctx,
        permit,
    ))
    .await
    .unwrap();
    assert_eq!(completed.status, 200);
    drop(completed);
    concurrency_wait_empty(&db, &catalog.id, &actor).await;
    // A node-backed connection shares the catalog scope, not its own ID.
    let node = insert_online_node(&state, &actor, "concurrency-node").await;
    db.collection::<bson::Document>(USER_SERVICES)
        .update_one(
            doc! {"_id":&connection.id},
            doc! {"$set":{"node_id":&node.id}},
        )
        .await
        .unwrap();
    let (node_server, executor) =
        start_node_executor(state.clone(), &node, &connection.slug, &base).await;
    let held = concurrency::acquire(&db, &catalog, &actor).await.unwrap();
    assert!(matches!(
        proxy_request_by_slug_inner(
            &state,
            &auth,
            &connection.slug,
            "status",
            proxy_request("/proxy/s/limited/status"),
            &mut slug
        )
        .await,
        Err(AppError::ServiceConcurrencyLimited)
    ));
    drop(held);
    concurrency_wait_empty(&db, &catalog.id, &actor).await;
    let response = proxy_request_by_slug_inner(
        &state,
        &auth,
        &connection.slug,
        "status",
        proxy_request("/proxy/s/limited/status"),
        &mut slug,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), 1024).await.unwrap();
    concurrency_wait_empty(&db, &catalog.id, &actor).await;
    executor.abort();
    node_server.abort();
    server.abort();
    assert_eq!(
        super::proxy_error_telemetry_fields(&AppError::ServiceConcurrencyLimited),
        (429, 12600)
    );
}

async fn concurrency_wait_empty(db: &mongodb::Database, service: &str, actor: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let row = db
                .collection::<bson::Document>(crate::models::service_concurrency::COLLECTION_NAME)
                .find_one(doc! {"service_id":service,"person_id":actor})
                .await
                .unwrap()
                .unwrap();
            if row.get_array("leases").unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn service_concurrency_sse_disconnect_and_websocket_close_release_capacity() {
    use crate::models::service_concurrency::ServiceConcurrencyPolicy;
    use crate::services::service_concurrency_service as concurrency;
    let db =
        crate::test_utils::connect_transaction_test_database("concurrency_long_transports").await;
    concurrency::ensure_indexes(&db).await.unwrap();
    crate::services::coordination_service::ensure_indexes(&db)
        .await
        .unwrap();
    let app = Router::new()
        .route(
            "/events",
            get(|| async {
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(
                        futures::stream::once(async {
                            Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"data: hello\n\n"))
                        })
                        .chain(futures::stream::pending()),
                    ))
                    .unwrap()
            }),
        )
        .route(
            "/socket",
            get(|ws: axum::extract::WebSocketUpgrade| async {
                ws.on_upgrade(|mut socket| async move { while socket.recv().await.is_some() {} })
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let downstream = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let actor = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&actor, UserType::Person))
        .await
        .unwrap();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.base_url = base.clone();
    catalog.streaming_supported = true;
    catalog.concurrency_policy = Some(ServiceConcurrencyPolicy {
        service_id: catalog.id.clone(),
        default_limit: Some(1),
        users: vec![],
        orgs: vec![],
    });
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&catalog)
    .await
    .unwrap();
    let connection =
        insert_user_service(&db, &actor, "limited-stream", &base, Some(&catalog.id)).await;
    let state = test_app_state(db.clone());
    let auth = crate::test_utils::test_auth_user(&actor);
    let mut slug = String::new();
    let response = proxy_request_by_slug_inner(
        &state,
        &auth,
        &connection.slug,
        "events",
        proxy_request("/proxy/s/limited-stream/events"),
        &mut slug,
    )
    .await
    .unwrap();
    let mut body = response.into_body();
    use http_body_util::BodyExt;
    assert!(body.frame().await.unwrap().unwrap().is_data());
    assert!(matches!(
        concurrency::acquire(&db, &catalog, &actor).await,
        Err(AppError::ServiceConcurrencyLimited)
    ));
    drop(body);
    concurrency_wait_empty(&db, &catalog.id, &actor).await;
    let proxy = Router::new()
        .route("/proxy/s/{slug}/{*path}", get(ws_proxy_test_route))
        .route_layer(axum::Extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        ))
        .with_state((state, auth));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "ws://{}/proxy/s/limited-stream/socket",
        listener.local_addr().unwrap()
    );
    let proxy_server = tokio::spawn(async move { axum::serve(listener, proxy).await.unwrap() });
    let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    assert!(matches!(
        concurrency::acquire(&db, &catalog, &actor).await,
        Err(AppError::ServiceConcurrencyLimited)
    ));
    let refused = tokio_tungstenite::connect_async(&url).await.unwrap_err();
    match refused {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), 429);
            assert_eq!(response.headers()["retry-after"], "1");
        }
        other => panic!("unexpected {other}"),
    }
    socket.close(None).await.unwrap();
    drop(socket);
    concurrency_wait_empty(&db, &catalog.id, &actor).await;
    proxy_server.abort();
    downstream.abort();
}
