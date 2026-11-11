use super::*;
use crate::services::assistant_acknowledgement_service as acks;

async fn assistant(label: &str, strategy: &str) -> Fixture {
    let mut f = fixture(label, StatusCode::OK, strategy, true).await;
    let owner = f.auth.user_id.to_string();
    // Assistant acknowledgements remain required independently of generic service approval.
    crate::test_utils::set_test_approval_preference(&f.state.db, &owner, false).await;

    let row = crate::services::assistant_authority_tests::new_orchestrator(
        &f.state,
        &owner,
        "Use the pool",
    )
    .await;
    let key =
        crate::services::key_service::get_api_key(&f.state.db, &owner, &row.credential_api_key_id)
            .await
            .unwrap();
    f.auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    f
}
async fn request(f: &Fixture, ack: Option<&str>) -> crate::errors::AppResult<Response> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/v1/proxy/s/review-route/perform")
        .header("content-type", "application/json");
    if let Some(ack) = ack {
        builder = builder.header(
            crate::services::assistant_service_execution::ACK_HEADER,
            ack,
        );
    }
    let mut request = builder.body(Body::from("{\"input\":\"same\"}")).unwrap();
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
    Box::pin(crate::handlers::proxy::proxy_request_by_slug(
        State(f.state.clone()),
        f.auth.clone(),
        Default::default(),
        Path(("review-route".into(), "perform".into())),
        request,
    ))
    .await
}

#[tokio::test]
async fn assistant_pool_post_connect_failure_keeps_configured_safe_failover() {
    let f = assistant("assistant_pool_safe_post", "priority").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! {"label":"review-first"},
            doc! {"$set":{"url":unavailable}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! {"_id":&f.pool_id},
            doc! {"$set":{"failover.retry_on":["connect_error"]}},
        )
        .await
        .unwrap();
    let response = request(&f, None).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(f.first.requests.lock().await.is_empty());
    assert_eq!(f.second.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn assistant_pool_unknown_dispatched_post_never_switches_credentials() {
    use tokio::io::AsyncReadExt;
    let f = assistant("assistant_pool_unknown_post", "priority").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let dispatched = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
    });
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(doc! {"label":"review-first"}, doc! {"$set":{"url":url}})
        .await
        .unwrap();
    f.state.db.collection::<Document>("service_pools").update_one(doc! {"_id":&f.pool_id},doc! {"$set":{"failover.retry_on":["connect_error","transport_error"],"failover.retry_ambiguous_dispatch":true}}).await.unwrap();
    let result = request(&f, None).await;
    assert!(result.is_err() || result.as_ref().is_ok_and(|r| !r.status().is_success()));
    dispatched.await.unwrap();
    assert!(f.second.requests.lock().await.is_empty());
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::services::assistant_service_execution::COLLECTION)
            .count_documents(doc! {"kind":"unavailable_bucket"})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn assistant_pool_approval_pins_member_across_requests_and_balance_ticks() {
    let f = assistant("assistant_pool_durable_pin", "round_robin").await;
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.auth.user_id.to_string(),
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let response = request(&f, None).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let id = body["acknowledgement_id"].as_str().unwrap();
    let card = f
        .state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    let connection = card
        .get_document("arguments")
        .ok()
        .and_then(|a| a.get_str("connection_id").ok())
        .map(str::to_owned)
        .unwrap_or_else(String::new);
    let bound = f
        .state
        .db
        .collection::<Document>(crate::services::assistant_service_execution::COLLECTION)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    let selected = bound.get_str("connection_id").unwrap().to_owned();
    assert!(connection.is_empty() || connection == selected);
    f.state
        .db
        .collection::<Document>("service_pools")
        .update_one(doc! {"_id":&f.pool_id}, doc! {"$inc":{"rr_counter":1_i64}})
        .await
        .unwrap();
    let chat = f.auth.assistant_chat.as_ref().unwrap();
    acks::decide(&f.state.db, &chat.user_id, &chat.conversation_id, id, true)
        .await
        .unwrap();
    let response = request(&f, Some(id)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    to_bytes(response.into_body(), 4096).await.unwrap();
    let first = f.first.requests.lock().await.len();
    let second = f.second.requests.lock().await.len();
    assert_eq!(
        (first, second),
        if selected == f.second_member_id {
            (0, 1)
        } else {
            (1, 0)
        }
    );
}

#[tokio::test]
async fn assistant_pool_approved_predispatch_outage_proposes_and_pins_new_member() {
    let f = assistant("assistant_pool_reconsent", "priority").await;
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.auth.user_id.to_string(),
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    f.state.db.collection::<Document>("service_pools").update_one(
        doc! {"_id":&f.pool_id}, doc! {"$set":{"failover.retry_on":["connect_error"],"failover.cooldown.failures_to_open":2_i64}},
    ).await.unwrap();
    let response = request(&f, None).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let card: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let first_ack = card["acknowledgement_id"].as_str().unwrap();
    let chat = f.auth.assistant_chat.as_ref().unwrap();
    acks::decide(
        &f.state.db,
        &chat.user_id,
        &chat.conversation_id,
        first_ack,
        true,
    )
    .await
    .unwrap();
    // The approved URL/config stays identical, but its listener becomes
    // unavailable before any request bytes can be sent.
    f.first.server.abort();
    let first = request(&f, Some(first_ack)).await;
    assert!(
        first.is_err()
            || first
                .as_ref()
                .is_ok_and(|response| !response.status().is_success())
    );
    assert!(f.second.requests.lock().await.is_empty());
    let response = request(&f, None).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let card: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let second_ack = card["acknowledgement_id"].as_str().unwrap();
    assert_ne!(first_ack, second_ack);
    let binding = f
        .state
        .db
        .collection::<Document>(crate::services::assistant_service_execution::COLLECTION)
        .find_one(doc! {"_id":second_ack})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        binding.get_str("connection_id"),
        Ok(f.second_member_id.as_str())
    );
    assert!(f.second.requests.lock().await.is_empty());
    acks::decide(
        &f.state.db,
        &chat.user_id,
        &chat.conversation_id,
        second_ack,
        true,
    )
    .await
    .unwrap();
    let response = request(&f, Some(second_ack)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(f.second.requests.lock().await.len(), 1);
}
