//! Exercise the real HTTP route, credential resolver, and active-turn evidence write.
use crate::{
    services::{assistant_nyxagent as engine, key_service, user_api_key_service},
    test_utils,
};
use axum::{Router, http::StatusCode, routing::get};
use mongodb::{
    bson::{Document, doc},
    event::{EventHandler, command::CommandEvent},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;

#[tokio::test]
async fn real_work_http_chat_key_counts_once_and_ordinary_key_never_writes_signal() {
    let writes = Arc::new(AtomicUsize::new(0));
    let observed = writes.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.command.get_str("update") == Ok("assistant_conversations")
            && event.command.get_array("updates").is_ok_and(|updates| {
                updates.iter().any(|update| {
                    update
                        .as_document()
                        .and_then(|u| u.get_document("u").ok())
                        .and_then(|u| u.get_document("$set").ok())
                        .is_some_and(|set| set.contains_key("active_turn.business_system_success"))
                })
            })
        {
            observed.fetch_add(1, Ordering::SeqCst);
        }
    });
    let db = test_utils::connect_transaction_test_database_with_command_handler(
        "real_work_http",
        handler,
    )
    .await;
    let f = crate::services::org_agent_tests::Fixture::with_db(db.clone()).await;
    let upstream = Router::new().route(
        "/records",
        get(|headers: axum::http::HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer test-business-credential");
            axum::Json(serde_json::json!({"ok":true}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "business-test".into();
    service.base_url = url.clone();
    service.service_category = "connection".into();
    service.inference = None;
    service.auth_method = "bearer".into();
    service.requires_user_credential = true;
    service.visibility = "public".into();
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&service)
    .await
    .unwrap();
    let credential = user_api_key_service::create_api_key(
        &db,
        &f.state.encryption_keys,
        &f.admin,
        user_api_key_service::CreateApiKeyParams {
            label: "Business",
            credential_type: "bearer",
            credential: "test-business-credential",
            access_token: None,
            refresh_token: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: Some("user_created"),
            source_id: None,
        },
    )
    .await
    .unwrap();
    let endpoint = Uuid::new_v4().to_string();
    db.collection::<crate::models::user_endpoint::UserEndpoint>(
        crate::models::user_endpoint::COLLECTION_NAME,
    )
    .insert_one(test_utils::test_user_endpoint(
        &endpoint,
        &f.admin,
        &service.slug,
        &url,
        None,
        Some(&service.id),
    ))
    .await
    .unwrap();
    let mut connection = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.admin,
        &service.slug,
        &endpoint,
        Some(&service.id),
        None,
    );
    connection.api_key_id = Some(credential.id);
    connection.auth_method = "bearer".into();
    db.collection::<crate::models::user_service::UserService>(
        crate::models::user_service::COLLECTION_NAME,
    )
    .insert_one(&connection)
    .await
    .unwrap();
    let input = serde_json::from_value::<engine::TurnRequest>(
        serde_json::json!({"text":"Check my records"}),
    )
    .unwrap();
    let turn = Box::pin(engine::begin_turn(
        &db,
        &f.admin,
        &input,
        &f.state.encryption_keys,
    ))
    .await
    .unwrap();
    let chat_key = crate::services::assistant_agent_credential_service::load_for_conversation(
        &db,
        &f.state.encryption_keys,
        &f.admin,
        &turn.id,
    )
    .await
    .unwrap()
    .unwrap();
    let ordinary = key_service::create_api_key(
        &db,
        &f.admin,
        "ordinary",
        "proxy",
        None,
        None,
        Some(std::slice::from_ref(&connection.id)),
        None,
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
    let path = format!("/api/v1/proxy/s/{}/records", service.slug);
    // The normal key reaches the same upstream but performs zero evidence writes.
    writes.store(0, Ordering::SeqCst);
    let (status, response) = Box::pin(super::super::curation_tests::request(
        &f.state,
        "GET",
        &path,
        &ordinary.full_key,
        None,
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    let conversations = db.collection::<Document>("assistant_conversations");
    assert_eq!(
        conversations
            .find_one(doc! {"_id":&turn.id})
            .await
            .unwrap()
            .unwrap()
            .get_document("active_turn")
            .unwrap()
            .get_bool("business_system_success"),
        Ok(false)
    );
    for _ in 0..2 {
        let (status, response) = Box::pin(super::super::curation_tests::request(
            &f.state,
            "GET",
            &path,
            &chat_key.raw_key,
            None,
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "{response}");
    }
    assert_eq!(
        writes.load(Ordering::SeqCst),
        1,
        "later requests reuse the recorded flag"
    );
    assert_eq!(
        conversations
            .find_one(doc! {"_id":&turn.id})
            .await
            .unwrap()
            .unwrap()
            .get_document("active_turn")
            .unwrap()
            .get_bool("business_system_success"),
        Ok(true)
    );
    server.abort();
    db.drop().await.unwrap();
}
