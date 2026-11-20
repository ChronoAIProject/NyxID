#[tokio::test]
async fn refusal_throttle_round2_direct_and_real_node_match_unthrottled_wire_requests() {
    let db = crate::test_utils::connect_transaction_test_database("refusal_wire").await;
    let state = test_app_state(db.clone());
    let user = Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&user, UserType::Person)).await.unwrap();
    crate::test_utils::set_test_approval_preference(&db, &user, false).await;
    let key = crate::services::key_service::create_api_key(
        &db, &user, "refusal parity", "proxy", None, None,
        Some(&[]), Some(&[]), Some(true), Some(true), Some(true), None, None, None, None,
    ).await.unwrap();
    let downstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&downstream).await;
    let service = insert_user_service(&db, &user, "refusal-wire", &downstream.uri(), None).await;
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let current = private.with_state(state.clone());
    // This is the unchanged handler body from main, with real AuthUser extraction
    // and the same billing policy; the route-only ingress marker is absent.
    let original = Router::new()
        .route("/api/v1/proxy/s/{slug}/{*path}", any(super::proxy_request_by_slug))
        .layer(axum::Extension(crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Proxy,
        )))
        .with_state(state.clone());
    const PAYLOAD: &[u8] = b"\x1f\x8b\x08\x00synthetic\x00\xff\r\n\x80payload";
    for via_node in [false, true] {
        let before = downstream.received_requests().await.unwrap().len();
        for app in [&original, &current] {
            let executor = if via_node {
                let node = insert_online_node(&state, &user, "refusal-wire-node").await;
                db.collection::<UserService>(USER_SERVICES)
                    .update_one(doc!{"_id": &service.id}, doc!{"$set":{"node_id": &node.id}})
                    .await.unwrap();
                Some(start_node_executor(state.clone(), &node, &service.slug, &downstream.uri()).await)
            } else { None };
            let chunks = futures::stream::iter([
                Ok::<_, std::io::Error>(Bytes::from_static(&PAYLOAD[..7])),
                Ok(Bytes::from_static(&PAYLOAD[7..])),
            ]);
            let request = Request::builder().method("POST")
                .uri("/api/v1/proxy/s/refusal-wire/items?b=%2f&a=one&a=two")
                .header("x-api-key", &key.full_key)
                .header("content-length", PAYLOAD.len())
                .header("content-type", "application/octet-stream")
                .header("content-encoding", "gzip")
                .header("x-request-id", "synthetic-refusal-parity")
                .header("idempotency-key", "synthetic-retry-key")
                .header("if-match", "\"revision-one\"")
                .body(Body::from_stream(chunks)).unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "node={via_node}");
            to_bytes(response.into_body(), usize::MAX).await.unwrap();
            crate::services::shutdown::process().tasks_stopped().await;
            if let Some((executor, server)) = executor {
                executor.await.unwrap();
                server.abort();
            }
        }
        let received = downstream.received_requests().await.unwrap();
        assert_eq!(received.len(), before + 2);
        let old = &received[before];
        let new = &received[before + 1];
        assert_eq!(old.method, new.method);
        assert_eq!(old.url, new.url);
        assert_eq!(old.body, PAYLOAD);
        assert_eq!(new.body, old.body);
        assert_eq!(new.headers, old.headers, "node={via_node}");
        assert_eq!(new.headers["content-length"], PAYLOAD.len().to_string());
        eprintln!("refusal throttle wire parity: node={via_node}, body_bytes={}, complete_header_map_equal=true, content_encoding={:?}", PAYLOAD.len(), new.headers.get("content-encoding"));
    }
}
