use super::*;
use crate::services::channel_platform::PlatformAdapter;
use axum::{Json, Router, body::Bytes, extract::State, http::StatusCode, routing::post};

#[derive(Clone)]
struct HttpFixture {
    state: crate::AppState,
    telegram_url: String,
}

async fn manager_http(
    State(fixture): State<HttpFixture>,
    headers: HeaderMap,
    body: Bytes,
) -> crate::errors::AppResult<StatusCode> {
    crate::handlers::telegram_new::webhook_with_service(
        &fixture.state,
        &service(&fixture.state, &fixture.telegram_url),
        &headers,
        &body,
    )
    .await
}

async fn reply_http(
    State(fixture): State<HttpFixture>,
    headers: HeaderMap,
    Json(body): Json<crate::handlers::channel_relay::AsyncReplyRequest>,
) -> crate::errors::AppResult<Json<crate::handlers::channel_relay::AsyncReplyResponse>> {
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
        &fixture.telegram_url,
    );
    crate::handlers::channel_relay::async_reply_with_test_adapter(
        &fixture.state,
        &headers,
        body,
        &adapter,
    )
    .await
}

async fn start_http(
    state: &crate::AppState,
    server: &MockServer,
) -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route(
            "/api/v1/webhooks/channel/telegram-new/manager",
            post(manager_http),
        )
        .route("/api/v1/channel-relay/reply", post(reply_http))
        .with_state(HttpFixture {
            state: state.clone(),
            telegram_url: server.uri(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, task)
}

async fn add_agent(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
    callback: &str,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {
        "_id": &id, "user_id": actor, "name": "Public help", "key_prefix": "nyxid_ag",
        "key_hash": "00".repeat(32), "scopes": "read write", "is_active": true,
        "callback_url": format!("{}{callback}", server.uri()), "created_at": bson::DateTime::now(),
    }).await.unwrap();
    Mock::given(method("POST"))
        .and(path(callback))
        .respond_with(ResponseTemplate::new(202))
        .mount(server)
        .await;
    id
}

async fn wait_for_callbacks(state: &crate::AppState, bot: &ChannelBot, expected: u64) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let count = state
                .db
                .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
                .count_documents(doc! {"channel_bot_id": &bot.id, "callback_status": "delivered"})
                .await
                .unwrap();
            if count == expected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("All agent callbacks must finish");
}

#[tokio::test]
async fn telegram_new_manager_public_channel_http_chat_reply_and_creation_end_to_end() {
    let (state, actor, server) = fixture().await;
    let channel = register_manager_channel(&state, &actor, &server).await;
    let public_agent = add_agent(&state, &actor, &server, "/public-agent").await;
    let exact_agent = add_agent(&state, &actor, &server, "/exact-agent").await;
    for (chat, agent, default) in [("*", &public_agent, true), ("700", &exact_agent, false)] {
        crate::services::channel_routing_service::create_conversation(
            &state.db,
            &actor,
            Some(&channel.id),
            "telegram",
            chat,
            "private",
            None,
            agent,
            default,
            false,
        )
        .await
        .unwrap();
    }
    let (url, http_task) = start_http(&state, &server).await;
    let inbound = format!("{url}/api/v1/webhooks/channel/telegram-new/manager");
    let reply_url = format!("{url}/api/v1/channel-relay/reply");
    let base = server.uri();
    let manager = service(&state, &base);
    let (request, link) = manager
        .begin(&actor, &actor, "Created while chatting", true)
        .await
        .unwrap();
    let challenge = reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(name, _)| name == "start")
        .unwrap()
        .1
        .into_owned();
    let mut start = message(json!({"text": format!("/start {challenge}")}));
    start["update_id"] = json!(10);
    assert_eq!(
        state
            .http_client
            .post(&inbound)
            .headers(headers(&state).await)
            .json(&start)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    for (index, chat, sender, kind) in [
        (11, 700, 700, "private"),
        (12, 701, 701, "private"),
        (13, -400, 702, "group"),
    ] {
        let update = json!({"update_id": index, "message": {
            "message_id": index, "from": {"id": sender, "is_bot": false}, "chat": {"id": chat, "type": kind},
            "date": Utc::now().timestamp(), "text": format!("Public help from {sender}"),
        }});
        assert_eq!(
            state
                .http_client
                .post(&inbound)
                .headers(headers(&state).await)
                .json(&update)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        if index == 12 {
            let management = json!({"update_id": 14, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": bot()}});
            assert_eq!(
                state
                    .http_client
                    .post(&inbound)
                    .headers(headers(&state).await)
                    .json(&management)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
        }
    }
    let mut created = message(json!({"managed_bot_created": {"bot": bot()}}));
    created["update_id"] = json!(15);
    assert_eq!(
        state
            .http_client
            .post(&inbound)
            .headers(headers(&state).await)
            .json(&created)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    provider_connection(&server, &request.id, 200).await;
    manager.complete_pending_creations().await.unwrap();
    assert_eq!(
        manager.get(&actor, &request.id).await.unwrap().status,
        Status::Connected
    );
    wait_for_callbacks(&state, &channel, 3).await;

    let callbacks: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| matches!(request.url.path(), "/public-agent" | "/exact-agent"))
        .collect();
    assert_eq!(callbacks.len(), 3, "Setup updates must not reach agents");
    let mut payloads = Vec::new();
    for callback in &callbacks {
        assert!(!callback.headers.contains_key("x-nyxid-user-token"));
        assert!(callback.headers.contains_key("x-nyxid-callback-token"));
        let payload = callback.body_json::<Value>().unwrap();
        let claims = crate::crypto::jwt::validate_relay_reply_token(
            &state.jwt_keys,
            &state.config,
            payload["reply_token"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(claims.inbound_message_id, payload["message_id"]);
        assert_eq!(
            payload["agent"]["api_key_id"].as_str().unwrap(),
            if callback.url.path() == "/exact-agent" {
                exact_agent.as_str()
            } else {
                public_agent.as_str()
            }
        );
        payloads.push(payload);
    }
    assert_eq!(
        callbacks
            .iter()
            .filter(|r| r.url.path() == "/exact-agent")
            .count(),
        1
    );

    // A reply token cannot target another sender's message, and a rejected
    // attempt must not consume the token for its legitimate message.
    let wrong = state
        .http_client
        .post(&reply_url)
        .bearer_auth(payloads[0]["reply_token"].as_str().unwrap())
        .json(
            &json!({"message_id": payloads[1]["message_id"], "reply": {"text": "wrong recipient"}}),
        )
        .send()
        .await
        .unwrap();
    assert!(wrong.status().is_client_error());
    for payload in payloads.iter().take(2) {
        let response = state.http_client.post(&reply_url).bearer_auth(payload["reply_token"].as_str().unwrap())
            .json(&json!({"message_id": payload["message_id"], "reply": {"text": "Public assistant response"}})).send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let sent = server.received_requests().await.unwrap();
    let replies: Vec<_> = sent
        .iter()
        .filter(|r| r.url.path() == format!("/bot{MANAGER}/sendMessage"))
        .filter_map(|r| r.body_json::<Value>().ok())
        .filter(|v| v["text"] == "Public assistant response")
        .collect();
    assert_eq!(replies.len(), 2);
    for payload in payloads.iter().take(2) {
        assert!(
            replies
                .iter()
                .any(|r| r["chat_id"].to_string().trim_matches('"')
                    == payload["conversation"]["platform_id"].as_str().unwrap())
        );
    }
    assert!(
        !sent
            .iter()
            .any(|r| r.url.path() == format!("/bot{MANAGER}/setWebhook"))
    );

    // The unchanged normal Telegram path still supplies scoped owner authority.
    let mut ordinary = channel.clone();
    ordinary.credential_source = "user".into();
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::new();
    let messages = adapter
        .parse_inbound(&serde_json::to_vec(&message(json!({"text": "Ordinary bot"}))).unwrap())
        .await
        .unwrap();
    crate::services::channel_inbound_service::process_inbound_messages(
        (&state).into(),
        &ordinary,
        &adapter,
        &messages,
    )
    .await
    .unwrap();
    assert!(server.received_requests().await.unwrap().iter().any(|r| r.url.path() == "/exact-agent" && r.headers.contains_key("x-nyxid-user-token")));

    crate::services::channel_bot_service::delete_bot(
        &state.db,
        &state.config,
        &state.http_client,
        &state.encryption_keys,
        &adapter,
        &channel.id,
        &actor,
    )
    .await
    .unwrap();
    let rejected = state
        .http_client
        .post(&reply_url)
        .bearer_auth(payloads[2]["reply_token"].as_str().unwrap())
        .json(
            &json!({"message_id": payloads[2]["message_id"], "reply": {"text": "after deletion"}}),
        )
        .send()
        .await
        .unwrap();
    assert!(rejected.status().is_client_error());
    assert!(manager.manager().await.is_ok());
    let mut recover = message(json!({"text": "/recover @CustomerBot"}));
    recover["update_id"] = json!(16);
    assert_eq!(
        state
            .http_client
            .post(&inbound)
            .headers(headers(&state).await)
            .json(&recover)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(
        !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path() == format!("/bot{MANAGER}/deleteWebhook"))
    );
    http_task.abort();
}

#[tokio::test]
async fn telegram_new_manager_ordinary_lookup_failure_isolated_from_control_failure() {
    let (state, actor, server) = fixture().await;
    let channel = register_manager_channel(&state, &actor, &server).await;
    let (url, http_task) = start_http(&state, &server).await;
    let inbound = format!("{url}/api/v1/webhooks/channel/telegram-new/manager");
    state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(
            doc! {"_id": &channel.id},
            doc! {"$set": {"label": ["invalid model field"]}},
        )
        .await
        .unwrap();
    let response = state
        .http_client
        .post(&inbound)
        .headers(headers(&state).await)
        .json(&message(json!({"text": "hello"})))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let invalid_auth = state
        .http_client
        .post(&inbound)
        .json(&message(json!({"text": "hello"})))
        .send()
        .await
        .unwrap();
    assert!(invalid_auth.status().is_client_error());
    state.db.collection::<bson::Document>(crate::models::telegram_bot_request::MANAGED_BOTS).insert_one(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "manager_bot_id": 100_i64, "telegram_bot_id": 900_i64, "revision": "invalid", "update_ids": [],
    }).await.unwrap();
    let control = state
        .http_client
        .post(&inbound)
        .headers(headers(&state).await)
        .json(&json!({"update_id": 99, "managed_bot": {"user": {"id": 700}, "bot": bot()}}))
        .send()
        .await
        .unwrap();
    assert!(control.status().is_server_error());
    http_task.abort();
}
