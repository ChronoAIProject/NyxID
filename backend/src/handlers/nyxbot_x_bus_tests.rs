//! The X bus end to end: a signed X Activity DM reaches NyxBot over NyxID's
//! direct relay, and NyxBot's reply leaves through the production
//! `async_reply` path to X's DM endpoint, with delivery diagnostics recorded.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::models::channel_bot::{COLLECTION_NAME as BOTS, ChannelBot};
use crate::services::channel_adapters::x::{REQUIRED_SCOPES, XAdapter};
use crate::services::channel_platform::PlatformAdapter as _;

fn sign(body: &[u8]) -> HeaderMap {
    let mut mac = Hmac::<Sha256>::new_from_slice(b"api-secret").unwrap();
    mac.update(body);
    HeaderMap::from_iter([(
        "x-twitter-webhooks-signature".parse().unwrap(),
        format!("sha256={}", STANDARD.encode(mac.finalize().into_bytes()))
            .parse()
            .unwrap(),
    )])
}

fn dm(bot_id: &str, id: &str, sender: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "data": {"event_type": "dm.received", "tag": format!("nyxid:{bot_id}"), "filter": {"user_id": "10"},
            "payload": {"direct_message_events": [{"id": id, "type": "message_create", "message_create": {
                "sender_id": sender, "target": {"recipient_id": "10"},
                "message_data": {"text": text}}}]}}
    }))
    .unwrap()
}

/// The owner's X connection, the shared X app credentials and a registered X bot.
async fn x_bot(state: &AppState) -> ChannelBot {
    let db = &state.db;
    let provider = Uuid::new_v4().to_string();
    db.collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &provider, "slug": "twitter", "name": "X", "provider_type": "oauth2", "is_active": true,
            "credential_mode": "user", "supports_pkce": true, "created_by": OWNER,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let connection = Uuid::new_v4().to_string();
    let token = state.encryption_keys.encrypt(b"user-token").await.unwrap();
    db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &connection, "user_id": OWNER, "label": "X", "credential_type": "oauth2",
            "credential_source": "platform", "provider_config_id": &provider, "status": "active",
            "token_scopes": REQUIRED_SCOPES.join(" "),
            "access_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: token },
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    crate::services::platform_credential_service::update(
        db,
        &state.encryption_keys,
        &XAdapter::default().platform_credentials().unwrap(),
        OWNER,
        &[
            (
                "consumer_secret".into(),
                Some(zeroize::Zeroizing::new("api-secret".into())),
            ),
            (
                "app_bearer_token".into(),
                Some(zeroize::Zeroizing::new("app-token".into())),
            ),
        ]
        .into(),
        false,
    )
    .await
    .unwrap();
    let bot: ChannelBot = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": OWNER, "platform": "x", "label": "Support",
        "credential_source": "connection", "connection_id": &connection,
        "platform_bot_id": "10", "platform_bot_username": "support", "webhook_registered": true,
        "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] },
        "webhook_secret_hash": "", "status": "active", "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    })
    .unwrap();
    db.collection::<ChannelBot>(BOTS)
        .insert_one(&bot)
        .await
        .unwrap();
    bot
}

async fn x_requests(server: &MockServer, conversation: &str, count: usize) -> Vec<Value> {
    let path = format!("/2/dm_conversations/{conversation}/messages");
    for _ in 0..400 {
        let bodies: Vec<Value> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.url.path() == path)
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect();
        if bodies.len() >= count {
            return bodies;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("X never received {count} replies on {conversation}");
}

async fn outbound_outcome(state: &AppState, bot: &str, expected: Option<&str>) -> ChannelBot {
    for _ in 0..400 {
        let current = crate::services::channel_bot_service::get_bot(&state.db, bot)
            .await
            .unwrap();
        let reason = current
            .diagnostics
            .last_outbound_error
            .as_ref()
            .and_then(|error| error.reason.as_deref());
        if reason == expected
            && (expected.is_some()
                || (current.diagnostics.last_outbound_at.is_some()
                    && current.diagnostics.last_outbound_error.is_none()))
        {
            return current;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("outbound outcome {expected:?} was never recorded");
}

async fn idle(state: &AppState, conversation: &str) {
    for _ in 0..400 {
        let thread = crate::services::assistant_nyxagent::get(&state.db, OWNER, conversation)
            .await
            .unwrap();
        if thread.active_turn.is_none() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("turn never settled");
}

async fn mount_send(server: &MockServer, conversation: &str, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(format!("/2/dm_conversations/{conversation}/messages")))
        .respond_with(response)
        .mount(server)
        .await;
}

#[tokio::test]
async fn x_dm_reaches_nyxbot_and_the_reply_reaches_x_with_diagnostics() {
    let (state, calls, model) = setup("nyxbot_x_bus").await;
    let bot = x_bot(&state).await;
    let x = MockServer::start().await;
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            bot.id.clone(),
            std::sync::Arc::new(XAdapter {
                api_base: Some(x.uri()),
            }),
        );
    // NyxID itself, so the relay callback is a real HTTP request.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (_, router) = crate::routes::build_router_with_state(state.clone());
    let app = router.with_state(state.clone());
    let nyxid = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    // The NyxBot link exactly as `connect_direct` leaves it for an X bot.
    let (row, _) = channel(&state, "direct").await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"platform": "x", "channel_bot_id": &bot.id, "owner_sender_ids": ["2"]}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.route_api_key_id},
            doc! {"$set": {"callback_url": format!("http://{address}/api/v1/nyxbot/relay/{}", row.id)}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": OWNER, "channel_bot_id": &bot.id,
            "platform": "x", "platform_conversation_id": "*", "platform_conversation_type": "private",
            "agent_api_key_id": &row.route_api_key_id, "default_agent": true, "is_active": true,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let deliver = |id: &str, sender: &str, text: &str| {
        let body = dm(&bot.id, id, sender, text);
        let state = state.clone();
        async move {
            crate::handlers::channel_webhooks::dispatch_platform_webhook(
                &state,
                "x",
                &sign(&body),
                &body,
            )
            .await
            .unwrap();
        }
    };

    // 1. The owner's DM lands in NyxBot's own (home) thread; the reply is an X DM.
    mount_send(
        &x,
        "2-10",
        ResponseTemplate::new(201)
            .set_body_json(json!({"data": {"dm_event_id": "900", "dm_conversation_id": "2-10"}})),
    )
    .await;
    deliver("800", "2", "hello from X").await;
    let sent = x_requests(&x, "2-10", 1).await;
    assert_eq!(sent[0]["text"], "Here is your answer");
    let nyxbot = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = nyxbot.home_conversation_id.clone().unwrap();
    idle(&state, &home).await;
    let messages = engine::messages(&state.db, OWNER, &home, 100, None)
        .await
        .unwrap();
    let asked = messages
        .iter()
        .find(|message| message.role == "user")
        .expect("the DM is in the home thread");
    assert!(asked.text.ends_with("hello from X"), "{}", asked.text);
    assert_eq!(asked.via.as_deref(), Some("x"));
    assert!(
        messages
            .iter()
            .any(|message| message.role == "assistant" && message.text == "Here is your answer")
    );
    // Receiving the provider request precedes reply-part persistence and diagnostics.
    let healthy = outbound_outcome(&state, &bot.id, None).await;
    assert!(healthy.diagnostics.last_inbound_at.is_some());
    assert_eq!(
        healthy.diagnostics.last_inbound_source.as_deref(),
        Some("webhook")
    );
    assert!(healthy.diagnostics.last_outbound_at.is_some());
    assert!(healthy.diagnostics.last_outbound_error.is_none());
    assert_eq!(
        crate::handlers::channel_bots::health_for(&healthy, false, true).recommended_action,
        "none"
    );

    // 2. Another X user is a guest: own thread, no tools, reply to their DM.
    chats::set_private_chats(&state, OWNER, &row.id, "everyone")
        .await
        .unwrap();
    mount_send(
        &x,
        "3-10",
        ResponseTemplate::new(201)
            .set_body_json(json!({"data": {"dm_event_id": "901", "dm_conversation_id": "3-10"}})),
    )
    .await;
    deliver("801", "3", "who are you?").await;
    x_requests(&x, "3-10", 1).await;
    let guests = channel_conversations(&state, &row.id).await;
    assert_eq!(guests.len(), 1);
    assert!(guests[0].guest_turn);
    assert_ne!(guests[0].id, home);
    {
        let calls = calls.lock().await;
        let instructions = calls.last().unwrap()["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("you use no tools or services"),
            "{instructions}"
        );
    }
    idle(&state, &guests[0].id).await;

    // 3. X refuses the reply for a missing scope: recorded, and Health says reconnect.
    x.reset().await;
    mount_send(
        &x,
        "2-10",
        ResponseTemplate::new(403).set_body_json(
            json!({"title": "Forbidden", "detail": "Missing required OAuth2 scope dm.write"}),
        ),
    )
    .await;
    deliver("802", "2", "are you there?").await;
    let refused = outbound_outcome(&state, &bot.id, Some("scopes_missing")).await;
    assert_eq!(
        refused
            .diagnostics
            .last_outbound_error
            .as_ref()
            .unwrap()
            .code,
        crate::errors::AppError::ChannelPlatformError(String::new())
            .error_code()
            .to_string()
    );
    assert_eq!(
        crate::handlers::channel_bots::health_for(&refused, false, true).recommended_action,
        "reconnect"
    );
    idle(&state, &home).await;

    // 4. Then X rate-limits it: the newer error replaces the older one.
    x.reset().await;
    mount_send(
        &x,
        "2-10",
        ResponseTemplate::new(429).insert_header("retry-after", "30"),
    )
    .await;
    deliver("803", "2", "still there?").await;
    let limited = outbound_outcome(&state, &bot.id, Some("rate_limited")).await;
    assert_eq!(
        crate::handlers::channel_bots::health_for(&limited, false, true).recommended_action,
        "wait_rate_limit"
    );

    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&bot.id);
    nyxid.abort();
    model.abort();
}
