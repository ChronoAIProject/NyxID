use super::*;
use crate::services::channel_platform::PlatformAdapter;
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
};

#[derive(Clone)]
struct HttpFixture {
    telegram_url: String,
}

async fn create_http(
    State(state): State<crate::AppState>,
    Extension(fixture): Extension<HttpFixture>,
    auth: crate::mw::auth::AuthUser,
    tele: crate::telemetry::TelemetryContext,
    Json(body): Json<crate::handlers::channel_bots::CreateChannelBotRequest>,
) -> crate::errors::AppResult<(
    StatusCode,
    Json<crate::handlers::channel_bots::CreateChannelBotResponse>,
)> {
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
        &fixture.telegram_url,
    );
    crate::handlers::channel_bots::create_bot_with_adapter(
        &state,
        auth,
        tele,
        body,
        &adapter,
        &TelegramApi {
            http: &state.http_client,
            base_url: &fixture.telegram_url,
        },
    )
    .await
}

async fn manager_http(
    State(state): State<crate::AppState>,
    Extension(fixture): Extension<HttpFixture>,
    headers: HeaderMap,
    body: Bytes,
) -> crate::errors::AppResult<StatusCode> {
    crate::handlers::telegram_new::webhook_with_service(
        &state,
        &service(&state, &fixture.telegram_url),
        &headers,
        &body,
    )
    .await
}

async fn reply_http(
    State(state): State<crate::AppState>,
    Extension(fixture): Extension<HttpFixture>,
    headers: HeaderMap,
    Json(body): Json<crate::handlers::channel_relay::AsyncReplyRequest>,
) -> crate::errors::AppResult<Json<crate::handlers::channel_relay::AsyncReplyResponse>> {
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
        &fixture.telegram_url,
    );
    crate::handlers::channel_relay::async_reply_with_test_adapter(&state, &headers, body, &adapter)
        .await
}

async fn start_http(
    state: &crate::AppState,
    server: &MockServer,
) -> (String, tokio::task::JoinHandle<()>) {
    use crate::handlers::{channel_bots, channel_conversations, telegram_new};
    use crate::mw::auth::{
        reject_api_key_tokens, reject_delegated_tokens, reject_relay_tokens,
        reject_service_account_tokens,
    };

    // Match the human-only route boundary; only Telegram's HTTP destination is injected.
    let management = Router::new()
        .route(
            "/api/v1/channel-bots",
            get(channel_bots::list_bots).post(create_http),
        )
        .route(
            "/api/v1/channel-bots/{id}",
            get(channel_bots::get_bot).delete(channel_bots::delete_bot),
        )
        .route(
            "/api/v1/channel-bots/telegram-new",
            post(telegram_new::begin),
        )
        .route(
            "/api/v1/channel-bots/telegram-new/requests/{id}",
            get(telegram_new::get),
        )
        .route(
            "/api/v1/channel-conversations",
            get(channel_conversations::list_conversations)
                .post(channel_conversations::create_conversation),
        )
        .layer(middleware::from_fn(reject_delegated_tokens))
        .layer(middleware::from_fn(reject_api_key_tokens))
        .layer(middleware::from_fn(reject_service_account_tokens))
        .layer(middleware::from_fn(reject_relay_tokens));
    let app = Router::new()
        .merge(management)
        .route(
            "/api/v1/webhooks/channel/telegram-new/manager",
            post(manager_http),
        )
        .route("/api/v1/channel-relay/reply", post(reply_http))
        .layer(Extension(HttpFixture {
            telegram_url: server.uri(),
        }))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, task)
}

fn access_token(state: &crate::AppState, actor: &str) -> String {
    crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &uuid::Uuid::parse_str(actor).unwrap(),
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap()
}

async fn response_json(response: reqwest::Response, expected: StatusCode) -> Value {
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, expected, "{body}");
    serde_json::from_str(&body).unwrap()
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
    manager_channel_http_end_to_end(false).await;
}

#[tokio::test]
async fn telegram_new_manager_org_channel_http_chat_reply_and_creation_end_to_end() {
    manager_channel_http_end_to_end(true).await;
}

async fn manager_channel_http_end_to_end(org_owned: bool) {
    let (state, actor, server) = fixture().await;
    let target_org_id = if org_owned {
        use crate::models::org_membership::{MemberScopeSource, OrgRole};
        let org =
            crate::services::org_service::create_org_user(&state.db, "Manager channel", None, None)
                .await
                .unwrap();
        crate::services::org_service::create_membership(
            &state.db,
            &org.id,
            &actor,
            OrgRole::Admin,
            MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        Some(org.id)
    } else {
        None
    };
    let owner = target_org_id.as_deref().unwrap_or(&actor);
    super::manager_channel::bot_identity_api(&server, MANAGER).await;
    let (url, http_task) = start_http(&state, &server).await;
    let token = access_token(&state, &actor);
    let bots_url = format!("{url}/api/v1/channel-bots");
    let conversations_url = format!("{url}/api/v1/channel-conversations");
    let registration = json!({
        "platform": "telegram", "bot_token": MANAGER, "label": "Public manager",
        "target_org_id": target_org_id,
    });
    for (credential, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("nyxid_ag_denied"), StatusCode::FORBIDDEN),
    ] {
        let mut request = state.http_client.post(&bots_url).json(&registration);
        if let Some(credential) = credential {
            request = request.bearer_auth(credential);
        }
        response_json(request.send().await.unwrap(), expected).await;
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    let created = response_json(
        state
            .http_client
            .post(&bots_url)
            .bearer_auth(&token)
            .json(&registration)
            .send()
            .await
            .unwrap(),
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(created["credential_source"], "telegram_manager");
    assert_eq!(created["status"], "active");
    assert!(created.get("webhook_secret").is_none());
    let channel =
        crate::services::channel_bot_service::get_bot(&state.db, created["id"].as_str().unwrap())
            .await
            .unwrap();
    assert_eq!(channel.user_id, owner);
    assert!(channel.bot_token_encrypted.is_empty());
    assert!(channel.webhook_secret_hash.is_empty());
    let detail_url = format!("{bots_url}/{}", channel.id);
    let public_agent = add_agent(&state, owner, &server, "/public-agent").await;
    let exact_agent = add_agent(&state, owner, &server, "/exact-agent").await;
    let mut route_ids = Vec::new();
    for (chat, agent, default) in [("*", &public_agent, true), ("700", &exact_agent, false)] {
        let route = response_json(
            state
                .http_client
                .post(&conversations_url)
                .bearer_auth(&token)
                .json(&json!({
                    "channel_bot_id": channel.id, "agent_api_key_id": agent,
                    "platform_conversation_id": chat, "default_agent": default,
                    "target_org_id": target_org_id,
                }))
                .send()
                .await
                .unwrap(),
            StatusCode::CREATED,
        )
        .await;
        route_ids.push(route["id"].as_str().unwrap().to_owned());
    }
    Mock::given(method("POST"))
        .and(path(format!("/bot{MANAGER}/getWebhookInfo")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": {"url": ""}})),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    response_json(
        state
            .http_client
            .post(&bots_url)
            .bearer_auth(&token)
            .json(&registration)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let bot_query: Vec<_> = target_org_id.iter().map(|org| ("org_id", org)).collect();
    let failed = response_json(
        state
            .http_client
            .get(&bots_url)
            .bearer_auth(&token)
            .query(&bot_query)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(failed["total"], 1);
    assert_eq!(failed["bots"][0]["id"], channel.id);
    assert_eq!(failed["bots"][0]["status"], "failed");
    assert_eq!(failed["bots"][0]["webhook_registered"], false);

    let outsider = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&outsider, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let outsider_token = access_token(&state, &outsider);
    let denied = state
        .http_client
        .post(&bots_url)
        .bearer_auth(&outsider_token)
        .json(&registration)
        .send()
        .await
        .unwrap();
    assert!(denied.status().is_client_error());
    response_json(
        state
            .http_client
            .get(&detail_url)
            .bearer_auth(&outsider_token)
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
    )
    .await;
    let mut retry = registration.clone();
    retry["label"] = json!("Do not replace saved label");
    let recovered = response_json(
        state
            .http_client
            .post(&bots_url)
            .bearer_auth(&token)
            .json(&retry)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(recovered["id"], channel.id);
    assert_eq!(recovered["status"], "active");
    assert!(recovered.get("webhook_secret").is_none());
    let detail = response_json(
        state
            .http_client
            .get(&detail_url)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(detail["label"], "Public manager");
    assert_eq!(detail["status"], "active");
    assert_eq!(detail["webhook_registered"], true);
    assert_eq!(detail["conversations_count"], 2);
    assert_eq!(detail["user_id"], owner);
    let mut route_query = vec![("bot_id", channel.id.as_str())];
    if let Some(org) = target_org_id.as_deref() {
        route_query.push(("org_id", org));
    }
    let routes = response_json(
        state
            .http_client
            .get(&conversations_url)
            .bearer_auth(&token)
            .query(&route_query)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(routes["total"], 2);
    for route in routes["conversations"].as_array().unwrap() {
        assert!(route_ids.contains(&route["id"].as_str().unwrap().to_owned()));
    }
    let inbound = format!("{url}/api/v1/webhooks/channel/telegram-new/manager");
    let reply_url = format!("{url}/api/v1/channel-relay/reply");
    let base = server.uri();
    let manager = service(&state, &base);
    let launch = response_json(
        state.http_client.post(format!("{bots_url}/telegram-new")).bearer_auth(&token)
            .json(&json!({"label": "Created while chatting", "auto_connect": true, "target_org_id": target_org_id}))
            .send().await.unwrap(), StatusCode::OK,
    ).await;
    let request_id = launch["request"]["id"].as_str().unwrap();
    let challenge = reqwest::Url::parse(launch["launch_url"].as_str().unwrap())
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
    provider_connection(&server, request_id, 200).await;
    manager.complete_pending_creations().await.unwrap();
    let child = response_json(
        state
            .http_client
            .get(format!("{bots_url}/telegram-new/requests/{request_id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(child["status"], "connected");
    assert_eq!(child["owner_user_id"], owner);
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

    assert_eq!(
        state
            .http_client
            .delete(&detail_url)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let routes = response_json(
        state
            .http_client
            .get(&conversations_url)
            .bearer_auth(&token)
            .query(&route_query)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(routes["total"], 0);
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
