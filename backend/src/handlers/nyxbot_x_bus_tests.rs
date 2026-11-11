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

// Request receipt and turn settlement precede outbound diagnostics. The exact
// event's terminal delivery state is written after that send's diagnostics.
async fn x_delivery(
    state: &AppState,
    bot: &ChannelBot,
    channel_id: &str,
    platform_message_id: &str,
    expected_state: &str,
    expected_reason: Option<&str>,
) -> ChannelBot {
    use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};

    let inbound = state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {
            "channel_bot_id": &bot.id,
            "platform": "x",
            "direction": "inbound",
            "platform_message_id": platform_message_id,
        })
        .await
        .unwrap_or_else(|_| {
            panic!("X delivery {platform_message_id}/{expected_state}: inbound lookup failed")
        })
        .unwrap_or_else(|| {
            panic!("X delivery {platform_message_id}/{expected_state}: inbound message missing")
        });
    let mut last = "event not observed".to_owned();
    for _ in 0..400 {
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"channel_id": channel_id, "event_id": &inbound.id})
            .await
            .unwrap_or_else(|_| {
                panic!("X delivery {platform_message_id}/{expected_state}: event lookup failed")
            });
        let current = crate::services::channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "X delivery {platform_message_id}/{expected_state}: diagnostics lookup failed"
                )
            });
        let delivery_state = event
            .as_ref()
            .and_then(|event| event.delivery.as_ref())
            .map(|delivery| delivery.state.as_str());
        let error = current.diagnostics.last_outbound_error.as_ref();
        let reason = error.and_then(|error| error.reason.as_deref());
        let diagnostics_match = match expected_reason {
            Some(expected) => reason == Some(expected),
            None => current.diagnostics.last_outbound_at.is_some() && error.is_none(),
        };
        if delivery_state == Some(expected_state) && diagnostics_match {
            return current;
        }
        last = format!(
            "event_status={:?}, delivery_state={delivery_state:?}, last_outbound_at={:?}, error_reason={reason:?}, error_code={:?}",
            event.as_ref().map(|event| event.status.as_str()),
            current.diagnostics.last_outbound_at,
            error.map(|error| error.code.as_str()),
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!(
        "X delivery {platform_message_id}/{expected_state} on channel {channel_id}, relay_message_id={}: expected_reason={expected_reason:?}; last {last}",
        inbound.id,
    );
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
            .set_delay(std::time::Duration::from_secs(1))
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
    let healthy = x_delivery(&state, &bot, &row.id, "800", "sent", None).await;
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
            .set_delay(std::time::Duration::from_secs(1))
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
    x_delivery(&state, &bot, &row.id, "801", "sent", None).await;

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
    let refused = x_delivery(
        &state,
        &bot,
        &row.id,
        "802",
        "unknown",
        Some("scopes_missing"),
    )
    .await;
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
    let limited = x_delivery(
        &state,
        &bot,
        &row.id,
        "803",
        "unknown",
        Some("rate_limited"),
    )
    .await;
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

#[tokio::test]
async fn org_x_verification_then_personal_specialist_dm_is_answered_and_private_drops_are_visible()
{
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    let (mut state, calls, model) = setup("x_2043_org_personal").await;
    let mut bot = x_bot(&state).await;
    let org = Uuid::new_v4().to_string();
    state
        .db
        .collection(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            OWNER,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    bot.user_id.clone_from(&org);
    state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(doc! {"_id":&bot.id}, doc! {"$set":{"user_id":&org}})
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&bot.connection_id},
            doc! {"$set":{"user_id":&org}},
        )
        .await
        .unwrap();
    let (specialist, _) = crate::services::assistant_team_service::create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        crate::services::assistant_team_service::CreateRequest {
            machines: None,
            logins: None,
            name: "community-nyxbot".into(),
            description: "Community help".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let x = MockServer::start().await;
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            bot.id.clone(),
            Arc::new(XAdapter {
                api_base: Some(x.uri()),
            }),
        );
    mount_send(
        &x,
        "2-10",
        ResponseTemplate::new(201).set_body_json(json!({"data":{"dm_event_id":"900"}})),
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.config.base_url = format!("http://{}", listener.local_addr().unwrap());
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
    let (row, link) = connect(&state, OWNER, None, &bot.id, &specialist)
        .await
        .unwrap();
    assert_eq!(row.bot_owner_id.as_deref(), Some(org.as_str()));
    assert!(row.org_specialist.is_none());
    assert_eq!(row.transport, "direct");
    assert_eq!(
        key_service::get_api_key(&state.db, &org, &row.route_api_key_id)
            .await
            .unwrap()
            .user_id,
        org
    );
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
    deliver("800", "2", &link.code).await;
    let sent = x_requests(&x, "2-10", 1).await;
    assert!(
        sent[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Linked. I'm community-nyxbot")
    );
    // Wait for the exact callback, not only receipt of the provider request.
    wait_x_counter(&state, &bot.id, "x_reply_sent", 1).await;
    let chat = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(chat.owner_chat);
    assert!(chat.owner_seen);
    assert_eq!(chat.title.as_deref(), Some("You"));
    assert!(
        calls.lock().await.is_empty(),
        "verification never reaches the model"
    );
    let linked = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(linked.owner_sender_ids, vec!["2"]);
    // A concurrent request holding the original link snapshot cannot claim success.
    let stale_sender = Sender {
        original_kind: None,
        media: Vec::new(),
        original_message: true,
        kind: crate::models::channel_thread::ThreadSenderKind::Human,
        owner_eligible: true,
        id: "3",
        display_name: None,
    };
    assert!(matches!(
        link_owner(&state, &row, &stale_sender, &link.code, true)
            .await
            .unwrap(),
        Some(Inbound::Silent("refused_link_consumed"))
    ));
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .owner_sender_ids,
        vec!["2"]
    );
    // Identical verification delivery is deduped; a distinct ordinary DM is not.
    deliver("800", "2", &link.code).await;
    deliver("801", "2", "chronoai是做什么的").await;
    let sent = x_requests(&x, "2-10", 2).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["text"], "Here is your answer");
    let ready = x_delivery(&state, &bot, &row.id, "801", "sent", None).await;
    assert!(ready.diagnostics.last_outbound_at.is_some());
    wait_x_counter(&state, &bot.id, "x_reply_sent", 2).await;
    let chat = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id":&chat.id})
        .await
        .unwrap()
        .unwrap();
    let conversation = engine::get(&state.db, OWNER, chat.conversation_id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(
        conversation.agent_id.as_deref(),
        Some(specialist.id.as_str())
    );
    assert!(!conversation.guest_turn);
    // Empty private DM used to be silent and absent from admission diagnostics.
    deliver("802", "2", "").await;
    let inbound = state
        .db
        .collection::<bson::Document>("channel_messages")
        .find_one(doc! {"channel_bot_id":&bot.id,"platform_message_id":"802","direction":"inbound"})
        .await
        .unwrap()
        .unwrap();
    for _ in 0..400 {
        let current = state
            .db
            .collection::<bson::Document>("channel_messages")
            .find_one(doc! {"_id":inbound.get_str("_id").unwrap()})
            .await
            .unwrap()
            .unwrap();
        if current
            .get_document("nyxbot_admission")
            .ok()
            .and_then(|d| d.get_str("disposition").ok())
            == Some("refused_empty_content")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let inbound = state
        .db
        .collection::<bson::Document>("channel_messages")
        .find_one(doc! {"_id":inbound.get_str("_id").unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        inbound
            .get_document("nyxbot_admission")
            .unwrap()
            .get_str("disposition")
            .unwrap(),
        "refused_empty_content"
    );
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let health = crate::services::channel_bot_health::setup_evidence(&state.db, &current, false)
        .await
        .unwrap();
    assert_eq!(
        health["last_inbound_outcome"]["code"],
        "x_content_unavailable"
    );
    assert!(
        health["recommended_action_text"]
            .as_str()
            .unwrap()
            .contains("does not prove encryption")
    );
    // Another sender never inherits the owner's verification, even in the same route.
    mount_send(
        &x,
        "3-10",
        ResponseTemplate::new(201).set_body_json(json!({"data":{"dm_event_id":"901"}})),
    )
    .await;
    deliver("803", "3", "你好").await;
    let refused = x_requests(&x, "3-10", 1).await;
    assert_eq!(refused[0]["text"], PRIVATE_REFUSAL);
    assert_eq!(calls.lock().await.len(), 1);
    wait_x_counter(&state, &bot.id, "x_chat_policy_refused", 1).await;
    // An unreadable authority row is a transient check failure, never a demotion.
    state
        .db
        .collection::<bson::Document>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id":&org,"member_user_id":OWNER},
            doc! {"$set":{"role":"invalid-role"}},
        )
        .await
        .unwrap();
    deliver("805", "2", "authority unavailable").await;
    wait_x_counter(&state, &bot.id, "x_org_access_unavailable", 1).await;
    assert_eq!(
        load_channel(&state, OWNER, &row.id).await.unwrap().status,
        "active"
    );
    assert_eq!(calls.lock().await.len(), 1);
    state
        .db
        .collection::<bson::Document>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id":&org,"member_user_id":OWNER},
            doc! {"$set":{"role":"admin"}},
        )
        .await
        .unwrap();
    // Loss of org-admin authority refuses the next callback before any new turn.
    state
        .db
        .collection::<bson::Document>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id":&org,"member_user_id":OWNER},
            doc! {"$set":{"role":"viewer"}},
        )
        .await
        .unwrap();
    deliver("804", "2", "no authority").await;
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        current.diagnostics.inbound_outcomes["x_org_access_lost"].count,
        1
    );
    assert_eq!(calls.lock().await.len(), 1);
    assert_eq!(
        load_channel(&state, OWNER, &row.id).await.unwrap().status,
        "failed"
    );
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&bot.id);
    nyxid.abort();
    model.abort();
    state.db.drop().await.unwrap();
}

async fn wait_x_counter(state: &AppState, bot: &str, code: &str, count: i64) {
    for _ in 0..400 {
        let current = channel_bot_service::get_bot(&state.db, bot).await.unwrap();
        if current
            .diagnostics
            .inbound_outcomes
            .get(code)
            .is_some_and(|c| c.count >= count)
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("X outcome {code} did not reach {count}");
}
