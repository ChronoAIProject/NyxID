use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

async fn setup() -> (ReplyTokenFixture, MockServer) {
    let mut fixture = setup_reply_token_fixture("complete_reply")
        .await
        .expect("MongoDB required");
    fixture.message.platform_message_id = Some("42".into());
    fixture.message.thread_id = Some("123".into());
    fixture
        .state
        .db
        .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
        .replace_one(doc! {"_id": &fixture.message.id}, &fixture.message)
        .await
        .unwrap();
    let server = MockServer::start().await;
    TEST_REPLY_ADAPTERS.lock().unwrap().insert(
        fixture.bot.id.clone(),
        std::sync::Arc::new(
            crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
                &server.uri(),
            ),
        ),
    );
    (fixture, server)
}

async fn post(fixture: &ReplyTokenFixture, token: &str, body: String) -> axum::response::Response {
    // Match production's outer cap and route override, including the new wire extractor.
    axum::Router::new()
        .route(
            "/reply",
            axum::routing::post(crate::handlers::channel_relay_request::reply).layer(
                axum::extract::DefaultBodyLimit::max(
                    crate::handlers::channel_relay_request::request_body_limit(0),
                ),
            ),
        )
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .with_state(fixture.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reply")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn payload(fixture: &ReplyTokenFixture, text: &str) -> String {
    serde_json::json!({"message_id":fixture.message.id,"reply":{"text":text,"metadata":null}})
        .to_string()
}

async fn receipt(fixture: &ReplyTokenFixture) -> bson::Document {
    fixture
        .state
        .db
        .collection::<bson::Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
        .find_one(doc! {"inbound_message_id": &fixture.message.id})
        .await
        .unwrap()
        .unwrap()
}

async fn cleanup(fixture: ReplyTokenFixture) {
    TEST_REPLY_ADAPTERS.lock().unwrap().remove(&fixture.bot.id);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn complete_reply_token_covers_all_ordered_parts_and_success_means_all_accepted() {
    let (fixture, server) = setup().await;
    let sequence = std::sync::atomic::AtomicUsize::new(43);
    Mock::given(path("/botbot-token/sendMessage"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true,"result":{
                "message_id":sequence.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            }}))
        })
        .mount(&server)
        .await;
    let text = format!(
        "{}\n\nhttps://example.com/long-link\n```\ncode\n```",
        "界🦉\\* paragraph\n\n".repeat(700)
    );
    let token = valid_reply_token(&fixture);
    let response = post(&fixture, &token, payload(&fixture, &text)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(requests.len() > 2);
    let mut reconstructed = String::new();
    for request in &requests {
        let part: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(part["chat_id"], "chat_123");
        assert_eq!(part["reply_to_message_id"], 42);
        assert_eq!(part["message_thread_id"], 123);
        reconstructed.push_str(part["text"].as_str().unwrap());
        assert!(part["text"].as_str().unwrap().encode_utf16().count() <= 4096);
    }
    assert_eq!(reconstructed, text);
    let record = receipt(&fixture).await;
    assert!(record.get_bool("complete").unwrap());
    assert_eq!(
        record
            .get_i64("expected_parts")
            .or_else(|_| record.get_i32("expected_parts").map(i64::from))
            .unwrap() as usize,
        requests.len()
    );
    let parts = record.get_array("parts").unwrap();
    assert!(
        parts
            .iter()
            .all(|p| p.as_document().unwrap().get_str("status").unwrap() == "accepted")
    );
    assert_eq!(
        parts
            .last()
            .unwrap()
            .as_document()
            .unwrap()
            .get_str("outbound_message_id")
            .unwrap(),
        body["message_id"].as_str().unwrap()
    );
    assert!(!record.to_string().contains("paragraph"));
    assert_eq!(
        post(&fixture, &token, payload(&fixture, "replay"))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        requests.len()
    );
    cleanup(fixture).await;
}

#[tokio::test]
async fn complete_reply_partial_failure_and_lost_response_never_return_success_or_replay() {
    for lost_response in [false, true] {
        let (fixture, server) = setup().await;
        let sequence = std::sync::atomic::AtomicUsize::new(0);
        Mock::given(path("/botbot-token/sendMessage"))
            .respond_with(move |_: &wiremock::Request| {
                if sequence.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"ok":true,"result":{"message_id":43}}))
                } else if lost_response {
                    ResponseTemplate::new(200).set_body_string("{incomplete receipt")
                } else {
                    ResponseTemplate::new(400).set_body_json(
                        serde_json::json!({"ok":false,"description":"provider refused"}),
                    )
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let token = valid_reply_token(&fixture);
        assert_eq!(
            post(&fixture, &token, payload(&fixture, &"a".repeat(9000)))
                .await
                .status(),
            StatusCode::BAD_GATEWAY
        );
        let record = receipt(&fixture).await;
        assert!(!record.get_bool("complete").unwrap());
        let statuses: Vec<_> = record
            .get_array("parts")
            .unwrap()
            .iter()
            .map(|p| p.as_document().unwrap().get_str("status").unwrap())
            .collect();
        assert_eq!(statuses, ["accepted", "uncertain", "not_attempted"]);
        assert_eq!(
            post(&fixture, &token, payload(&fixture, "replay"))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        server.verify().await;
        cleanup(fixture).await;
    }
}

#[tokio::test]
async fn complete_reply_preflights_part_count_before_any_send() {
    let (fixture, server) = setup().await;
    let response = post(
        &fixture,
        &valid_reply_token(&fixture),
        payload(&fixture, &"a".repeat(4096 * 64 + 1)),
    )
    .await;
    assert!(!response.status().is_success());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        fixture
            .state
            .db
            .collection::<bson::Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    cleanup(fixture).await;
}

#[tokio::test]
async fn complete_reply_wire_limit_counts_json_bytes_and_precedes_token_consumption() {
    let (fixture, server) = setup().await;
    let token = valid_reply_token(&fixture);
    let claims =
        jwt::validate_relay_reply_token(&fixture.state.jwt_keys, &fixture.state.config, &token)
            .unwrap();
    let max = crate::services::channel_reply_delivery::MAX_REPLY_REQUEST_BYTES;
    let mut exact = payload(&fixture, "hello");
    exact.extend(std::iter::repeat_n(' ', max - exact.len()));
    assert_eq!(exact.len(), max);
    let response = post(&fixture, &token, format!("{exact} ")).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        !reply_token_has_been_consumed(&fixture.state, &claims.jti)
            .await
            .unwrap()
    );
    Mock::given(path("/botbot-token/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"result":{"message_id":43}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(post(&fixture, &token, exact).await.status(), StatusCode::OK);
    server.verify().await;
    cleanup(fixture).await;
}

#[tokio::test]
async fn complete_reply_waits_for_all_acceptance_even_beyond_cma_five_second_deadline() {
    let (fixture, server) = setup().await;
    let sequence = std::sync::atomic::AtomicUsize::new(43);
    Mock::given(path("/botbot-token/sendMessage"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(2))
                .set_body_json(serde_json::json!({"ok":true,"result":{
                    "message_id":sequence.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                }}))
        })
        .expect(3)
        .mount(&server)
        .await;
    let token = valid_reply_token(&fixture);
    let mut send = Box::pin(post(&fixture, &token, payload(&fixture, &"a".repeat(9000))));
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), &mut send)
            .await
            .is_err()
    );
    assert!(!receipt(&fixture).await.get_bool("complete").unwrap());
    assert_eq!(send.await.status(), StatusCode::OK);
    assert!(receipt(&fixture).await.get_bool("complete").unwrap());
    server.verify().await;
    cleanup(fixture).await;
}
