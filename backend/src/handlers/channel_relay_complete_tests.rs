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
        assert_eq!(
            statuses,
            [
                "accepted",
                if lost_response { "uncertain" } else { "failed" },
                "not_attempted"
            ]
        );
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

#[tokio::test]
async fn turn_trace_multipart_relay_command_count_parity() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };
    let mut measured = Vec::new();
    for enabled in [false, true] {
        let (mut fixture, server) = setup().await;
        Mock::given(path("/botbot-token/sendMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok":true,"result":{"message_id":43}})),
            )
            .expect(3)
            .mount(&server)
            .await;
        let counts = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let observed = counts.clone();
        let handler = EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                let collection = event.command.get_str(&event.command_name).unwrap_or("");
                *observed
                    .lock()
                    .unwrap()
                    .entry(format!("{}:{collection}", event.command_name))
                    .or_default() += 1;
            }
        });
        let monitored = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "turn_trace_relay_monitor",
            handler,
        )
        .await;
        fixture.state.db = monitored.client().database(fixture.state.db.name());
        counts.lock().unwrap().clear();
        let trace = enabled.then(|| {
            crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string()).unwrap()
        });
        let (response, records) = crate::turn_trace::capture(crate::turn_trace::scope(
            trace,
            post(
                &fixture,
                &valid_reply_token(&fixture),
                payload(&fixture, &"a".repeat(9000)),
            ),
        ))
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        measured.push(counts.lock().unwrap().clone());
        if enabled {
            assert_eq!(
                records
                    .iter()
                    .filter(|r| r["stage"] == "channel_part"
                        && r["phase"] == "end"
                        && r["outcome"] == "confirmed")
                    .count(),
                3
            );
            assert!(
                records
                    .iter()
                    .any(|r| r["stage"] == "channel" && r["outcome"] == "confirmed")
            );
        }
        server.verify().await;
        cleanup(fixture).await;
        monitored.drop().await.unwrap();
    }
    assert_eq!(measured[0], measured[1]);
    assert!(
        measured[0]
            .keys()
            .any(|key| key.contains("channel_reply_deliveries"))
    );
    eprintln!(
        "turn trace multipart relay command parity: {:?}",
        measured[0]
    );
}

#[tokio::test]
async fn commonmark_multipart_parse_refusal_receipts_fallback_and_suffix_outcomes() {
    for terminal in [200, 400, 500] {
        let (fixture, server) = setup().await;
        let sequence = std::sync::atomic::AtomicUsize::new(0);
        Mock::given(path("/botbot-token/sendMessage")).respond_with(move |_: &wiremock::Request| {
            let n=sequence.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            if n==1 {ResponseTemplate::new(400).set_body_json(serde_json::json!({"ok":false,"error_code":400,"description":"can't parse entities"}))}
            else if n==2 && terminal!=200 {ResponseTemplate::new(terminal).set_body_json(serde_json::json!({"ok":false,"error_code":terminal,"description":"private provider text"}))}
            else {ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true,"result":{"message_id":n+43}}))}
        }).mount(&server).await;
        let source = format!(
            "**{}**\n\n**{}**\n\n**{}**",
            "中文မြန်မာ🦉".repeat(350),
            "English code a_b | ".repeat(250),
            "END".repeat(1500)
        );
        let parts = crate::services::channel_reply_plan::assistant_parts(&source, "telegram", "");
        let body=serde_json::json!({"message_id":fixture.message.id,"reply":{"text":source,"metadata":{"text_format":"commonmark"}}}).to_string();
        let token = valid_reply_token(&fixture);
        let response = post(&fixture, &token, body).await;
        assert_eq!(
            response.status(),
            if terminal == 200 {
                StatusCode::OK
            } else {
                StatusCode::BAD_GATEWAY
            }
        );
        let record = receipt(&fixture).await;
        assert_eq!(record.get_bool("complete").unwrap(), terminal == 200);
        let statuses = record.get_array("parts").unwrap();
        let failed = statuses[1].as_document().unwrap();
        assert_eq!(
            failed.get_str("formatting_fallback").unwrap(),
            "telegram_formatting_fallback"
        );
        assert_eq!(
            failed.get_str("status").unwrap(),
            match terminal {
                200 => "accepted",
                400 => "failed",
                _ => "uncertain",
            }
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            if terminal == 200 { parts.len() + 1 } else { 3 }
        );
        let rejected: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        let fallback: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
        assert_eq!(rejected["text"], fallback["text"]);
        if terminal == 200 {
            let delivered: Vec<String> = requests
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != 1)
                .map(|(_, r)| {
                    serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["text"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect();
            assert_eq!(
                delivered,
                parts
                    .iter()
                    .map(|p| p.text.clone().unwrap())
                    .collect::<Vec<_>>()
            );
        }
        assert!(fallback.get("entities").is_none() && fallback.get("parse_mode").is_none());
        let bot = fixture
            .state
            .db
            .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id":&fixture.bot.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bot.get_document("diagnostics")
                .unwrap()
                .get_document("last_outbound_notice")
                .unwrap()
                .get_str("reason")
                .unwrap(),
            "telegram_formatting_fallback"
        );
        if terminal != 200 {
            assert!(
                statuses[2..]
                    .iter()
                    .all(|p| p.as_document().unwrap().get_str("status") == Ok("not_attempted"))
            );
        }
        cleanup(fixture).await;
    }
}

#[tokio::test]
async fn gateway_commonmark_route_converts_once_and_keeps_legacy_route_separate() {
    let (fixture, server) = setup().await;
    Mock::given(path("/botbot-token/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok":true,"result":{"message_id":42}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let router = axum::Router::new()
        .route(
            "/reply/commonmark",
            axum::routing::post(crate::handlers::channel_relay_request::commonmark_reply),
        )
        .with_state(fixture.state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reply/commonmark")
                .header(
                    "authorization",
                    format!("Bearer {}", valid_reply_token(&fixture)),
                )
                .header("content-type", "application/json")
                .body(Body::from(payload(
                    &fixture,
                    "**中文 🦉** `C:\\qa\\menu_01.txt` [D1-B-ZH] | မြန်မာ",
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["text"],
        "中文 🦉 C:\\qa\\menu_01.txt [D1-B-ZH] | မြန်မာ"
    );
    assert!(body.get("parse_mode").is_none());
    assert_eq!(body["entities"].as_array().unwrap().len(), 2);
    cleanup(fixture).await;
}

#[tokio::test]
async fn commonmark_edit_retains_authorized_row_and_fallback_health() {
    let (mut fixture, server) = setup().await;
    fixture.outbound_message.platform_message_id = Some("42".into());
    fixture
        .state
        .db
        .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
        .replace_one(
            doc! {"_id": &fixture.outbound_message.id},
            &fixture.outbound_message,
        )
        .await
        .unwrap();
    let claims = valid_reply_claims(&fixture);
    insert_reply_token_use(&fixture, &claims).await;
    let token = encode_reply_claims(&fixture.state, &claims);
    let mut request = UpdateReplyRequest {
        message_id: "42".into(),
        outbound_message_id: Some(fixture.outbound_message.id.clone()),
        reply: body(Some("# 中文 🦉\n\n`a_b|c` မြန်မာ"), None),
    };
    crate::handlers::channel_relay_request::mark_commonmark(&mut request.reply).unwrap();
    let headers = bearer_headers(&token);
    let context = resolve_edit_request_context(&fixture.state, &headers, None, &request)
        .await
        .unwrap();
    let sequence = std::sync::atomic::AtomicUsize::new(0);
    Mock::given(path("/botbot-token/editMessageText")).respond_with(move |_: &wiremock::Request| {
        if sequence.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            ResponseTemplate::new(400).set_body_json(serde_json::json!({"ok":false,"error_code":400,"description":"can't parse entities"}))
        } else { ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})) }
    }).expect(2).mount(&server).await;
    let adapter =
        crate::services::channel_adapters::telegram::TelegramAdapter::for_api_base(&server.uri());
    let _ = edit_resolved_reply(&fixture.state, &headers, context, request, &adapter)
        .await
        .unwrap();
    let saved = fixture
        .state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {"_id":&fixture.outbound_message.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.get_str("formatting_fallback").unwrap(),
        "telegram_formatting_fallback"
    );
    assert!(saved.get_datetime("updated_at").is_ok());
    let requests = server.received_requests().await.unwrap();
    let first: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let plain: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(first["text"], "中文 🦉\n\na_b|c မြန်မာ");
    assert_eq!(plain["text"], first["text"]);
    assert_eq!(plain["message_id"], 42);
    assert!(plain.get("entities").is_none() && plain.get("parse_mode").is_none());
    let bot = fixture
        .state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":&fixture.bot.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bot.get_document("diagnostics")
            .unwrap()
            .get_document("last_outbound_notice")
            .unwrap()
            .get_str("reason")
            .unwrap(),
        "telegram_formatting_fallback"
    );
    cleanup(fixture).await;
}

#[tokio::test]
async fn supplemental_local_reply_preserves_token_final_parts_and_edit_authority() {
    Box::pin(async {
        for commonmark in [false, true] {
            let (fixture, server) = setup().await;
            let sequence = std::sync::atomic::AtomicUsize::new(100);
            Mock::given(path("/botbot-token/sendMessage")).respond_with(move |_: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true,"result":{"message_id":sequence.fetch_add(1,std::sync::atomic::Ordering::SeqCst)}}))
            }).mount(&server).await;
            Mock::given(path("/botbot-token/editMessageText")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true}))).mount(&server).await;
            let token = valid_reply_token(&fixture);
            let claims = jwt::validate_relay_reply_token(&fixture.state.jwt_keys, &fixture.state.config, &token).unwrap();
            let before: Vec<bson::Document> = fixture.state.db.collection::<bson::Document>(REPLY_TOKEN_USES).find(doc!{}).await.unwrap().try_collect().await.unwrap();
            let local = async_reply(State(fixture.state.clone()), HeaderMap::new(), OptionalAuthUser(Some(api_key_auth_user(&fixture.api_key))), Json(reply_request(&fixture.message.id))).await.unwrap().0;
            assert!(!reply_token_has_been_consumed(&fixture.state, &claims.jti).await.unwrap());
            let after: Vec<bson::Document> = fixture.state.db.collection::<bson::Document>(REPLY_TOKEN_USES).find(doc!{}).await.unwrap().try_collect().await.unwrap();
            assert_eq!(before, after);
            // Route-key sends cannot authorize token edits before the token's use.
            let local_edit = || UpdateReplyRequest { message_id:local.platform_message_id.clone().unwrap(), outbound_message_id:Some(local.message_id.clone()), reply:body(Some("edited notice"),None) };
            assert!(update_reply(State(fixture.state.clone()),bearer_headers(&token),OptionalAuthUser(None),Json(local_edit())).await.is_err());
            let endpoint = if commonmark {"/reply/commonmark"} else {"/reply"};
            let route = if commonmark {axum::routing::post(crate::handlers::channel_relay_request::commonmark_reply)} else {axum::routing::post(crate::handlers::channel_relay_request::reply)};
            let final_text = "中文 English မြန်မာစာ\n".repeat(1200);
            let response = axum::Router::new().route(endpoint, route).with_state(fixture.state.clone()).oneshot(Request::builder().method("POST").uri(endpoint).header("authorization",format!("Bearer {token}")).header("content-type","application/json").body(Body::from(payload(&fixture,&final_text))).unwrap()).await.unwrap();
            assert_eq!(response.status(),StatusCode::OK);
            let result: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap();
            let final_id=result["message_id"].as_str().unwrap();
            let receipts: Vec<bson::Document> = fixture.state.db.collection::<bson::Document>(crate::models::channel_reply_delivery::COLLECTION_NAME).find(doc!{"inbound_message_id":&fixture.message.id}).await.unwrap().try_collect().await.unwrap();
            assert_eq!(receipts.len(),2);
            let final_receipt=receipts.iter().find(|d|d.get_array("parts").unwrap().iter().any(|p|p.as_document().unwrap().get_str("outbound_message_id")==Ok(final_id))).unwrap();
            assert!(final_receipt.get_bool("complete").unwrap());
            assert!(final_receipt.get_array("parts").unwrap().iter().all(|p| p.as_document().unwrap().get_str("outbound_message_id")!=Ok(local.message_id.as_str())));
            let requests=server.received_requests().await.unwrap();
            let parts:Vec<serde_json::Value>=requests.iter().map(|r|serde_json::from_slice(&r.body).unwrap()).collect();
            assert!(parts.len()>3);
            assert_eq!(parts.iter().skip(1).map(|p|p["text"].as_str().unwrap()).collect::<String>(),if commonmark { final_text.trim_end() } else { &final_text });
            for part in parts { assert_eq!(part["reply_to_message_id"],42);assert_eq!(part["message_thread_id"],123); }
            // Existing edit authority is the same inbound/route, not a JTI-to-row
            // binding: after final use both of its authorized outbound rows edit.
            let _ = update_reply(State(fixture.state.clone()),bearer_headers(&token),OptionalAuthUser(None),Json(local_edit())).await.unwrap();
            let _ = update_reply(State(fixture.state.clone()),bearer_headers(&token),OptionalAuthUser(None),Json(UpdateReplyRequest {message_id:result["platform_message_id"].as_str().unwrap().into(),outbound_message_id:Some(final_id.into()),reply:body(Some("edited final"),None)})).await.unwrap();
            assert!(update_reply(State(fixture.state.clone()),bearer_headers(&token),OptionalAuthUser(None),Json(UpdateReplyRequest {message_id:"999".into(),outbound_message_id:Some(uuid::Uuid::new_v4().to_string()),reply:body(Some("refused"),None)})).await.is_err());
            assert_eq!(fixture.state.db.collection::<bson::Document>(REPLY_TOKEN_USES).count_documents(doc!{}).await.unwrap(),before.len() as u64+1);
            cleanup(fixture).await;
        }
    }).await;
}
