//! Webhook diagnostics: unauthenticated deliveries never write a bot row;
//! authenticated ones record and clear the bot's delivery state.
use super::*;
use crate::services::channel_platform::PlatformAdapter;
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use mongodb::bson::{self, doc};
use sha2::Sha256;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

use crate::services::channel_bot_health::{list_webhook_rejections, reset_rejection_throttle};
use crate::services::channel_x_tests::{fixture, insert_bot, webhooks::credentials};

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

fn dm(bot_id: &str, id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "data": {"event_type": "dm.received", "tag": format!("nyxid:{bot_id}"), "filter": {"user_id": "10"},
            "payload": {"direct_message_events": [{"id": id, "type": "message_create", "message_create": {
                "sender_id": "2", "target": {"recipient_id": "10"},
                "message_data": {"text": "diagnostics probe"}}}]}}
    }))
    .unwrap()
}

async fn raw_bot(state: &AppState, id: &str) -> bson::Document {
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
}

async fn rejections(state: &AppState, key: &str) -> i64 {
    list_webhook_rejections(&state.db)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.id == key)
        .map_or(0, |row| row.count)
}

#[tokio::test]
async fn only_authenticated_deliveries_write_bot_diagnostics() {
    let (state, adapter, server, owner, connection) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let bot = insert_bot(&state, &owner, &connection).await;
    let body = dm(&bot.id, "700");
    // The typed insert stores default diagnostics; nothing below may change them
    // until an authenticated delivery arrives.
    let untouched = raw_bot(&state, &bot.id).await.get("diagnostics").cloned();
    assert!(untouched.is_some());

    // 1. Unsigned platform delivery: counted per platform, bot row untouched.
    reset_rejection_throttle();
    assert!(
        dispatch_platform_webhook(&state, "x", &HeaderMap::new(), &body)
            .await
            .is_err()
    );
    assert_eq!(
        raw_bot(&state, &bot.id).await.get("diagnostics").cloned(),
        untouched
    );
    assert_eq!(rejections(&state, "x:signature").await, 1);

    // 2. A per-bot route for the wrong platform is refused before verification.
    reset_rejection_throttle();
    assert!(
        handle_webhook_inner(&state, &bot.id, "telegram", &sign(&body), &body)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        raw_bot(&state, &bot.id).await.get("diagnostics").cloned(),
        untouched
    );
    assert_eq!(rejections(&state, "telegram:platform_mismatch").await, 1);

    // 3. A per-bot forgery is counted, never written to the bot.
    reset_rejection_throttle();
    assert!(
        handle_webhook_inner(&state, &bot.id, "x", &HeaderMap::new(), &body)
            .await
            .is_err()
    );
    assert_eq!(
        raw_bot(&state, &bot.id).await.get("diagnostics").cloned(),
        untouched
    );
    assert_eq!(rejections(&state, "x:signature").await, 2);

    // 4. An authenticated delivery for an unregistered subscription is recorded.
    dispatch_platform_webhook(&state, "x", &sign(&body), &body)
        .await
        .unwrap();
    let row = raw_bot(&state, &bot.id).await;
    let diagnostics = row.get_document("diagnostics").unwrap();
    assert_eq!(
        diagnostics
            .get_document("last_webhook_error")
            .unwrap()
            .get_str("code")
            .unwrap(),
        "unregistered"
    );
    assert_eq!(
        diagnostics.get_str("failure_reason").unwrap(),
        "unregistered"
    );

    // 5. Once registered, a delivered message clears what it disproves.
    let agent = uuid::Uuid::new_v4().to_string();
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {
        "_id": &agent, "user_id": &owner, "name": "agent", "key_prefix": "nyxid_ag", "key_hash": "deadbeef".repeat(8),
        "scopes": "read write", "is_active": true, "callback_url": format!("{}/callback", server.uri()),
        "created_at": bson::DateTime::now(),
    }).await.unwrap();
    state.db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": &owner, "channel_bot_id": &bot.id, "platform": "x",
        "platform_conversation_id": "2-10", "platform_conversation_type": "private", "agent_api_key_id": &agent,
        "default_agent": false, "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {"webhook_registered": true}},
        )
        .await
        .unwrap();
    let delivered = dm(&bot.id, "701");
    dispatch_platform_webhook(&state, "x", &sign(&delivered), &delivered)
        .await
        .unwrap();
    let row = raw_bot(&state, &bot.id).await;
    let diagnostics = row.get_document("diagnostics").unwrap();
    assert_eq!(
        diagnostics.get("last_webhook_error"),
        Some(&bson::Bson::Null)
    );
    assert_eq!(diagnostics.get("failure_reason"), Some(&bson::Bson::Null));
    assert_eq!(
        diagnostics.get_str("last_inbound_source").unwrap(),
        "webhook"
    );
    assert!(diagnostics.get_datetime("last_inbound_at").is_ok());
    assert!(diagnostics.get_datetime("last_webhook_at").is_ok());
    let callbacks = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.url.path() == "/callback")
        .count();
    assert_eq!(
        callbacks, 1,
        "only the registered delivery reached the agent"
    );
}

#[tokio::test]
async fn lark_challenge_is_not_inbound_and_first_webhook_without_route_is_observed() {
    use crate::models::{channel_bot::ChannelBot, user::UserType};
    let db = crate::test_utils::connect_transaction_test_database("lark_setup_evidence").await;
    let state = crate::test_utils::test_app_state(db.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(crate::test_utils::test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let encrypted = state
        .encryption_keys
        .encrypt(b"verification-token")
        .await
        .unwrap();
    let bot: ChannelBot = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id":owner,"platform":"lark","label":"Lark",
        "platform_bot_id":"cli_app", "platform_bot_username":"bot", "bot_token_encrypted":bson::Binary {
            subtype:bson::spec::BinarySubtype::Generic, bytes: vec![] },
        "lark_verification_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
        "webhook_secret_hash":"hash","webhook_registered":false,"status":"pending_webhook","is_active":true,
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).unwrap();
    db.collection::<ChannelBot>("channel_bots")
        .insert_one(&bot)
        .await
        .unwrap();
    let challenge = |token| {
        serde_json::to_vec(&serde_json::json!({"type":"url_verification","token":token,"challenge":"callback-probe"})).unwrap()
    };
    assert!(
        handle_webhook_inner(
            &state,
            &bot.id,
            "lark",
            &HeaderMap::new(),
            &challenge("wrong")
        )
        .await
        .is_err()
    );
    let read = || channel_bot_service::get_bot(&db, &bot.id);
    assert!(
        read()
            .await
            .unwrap()
            .diagnostics
            .last_callback_challenge_at
            .is_none()
    );
    assert_eq!(
        handle_webhook_inner(
            &state,
            &bot.id,
            "lark",
            &HeaderMap::new(),
            &challenge("verification-token")
        )
        .await
        .unwrap(),
        Some(serde_json::json!({"challenge":"callback-probe"}))
    );
    let observed = read().await.unwrap();
    assert_eq!(observed.status, "pending_webhook");
    assert!(!observed.webhook_registered);
    assert!(observed.diagnostics.last_callback_challenge_at.is_some());
    assert!(observed.diagnostics.last_inbound_at.is_none());
    assert!(observed.diagnostics.last_webhook_at.is_none());
    assert!(observed.diagnostics.last_route_result.is_none());
    assert!(observed.last_verification.is_none());
    let event = serde_json::to_vec(&serde_json::json!({"schema":"2.0",
        "header":{"event_id":"event","event_type":"im.message.receive_v1","token":"verification-token"},
        "event":{"sender":{"sender_id":{"open_id":"ou_sender"}},"message":{"message_id":"om_message",
            "chat_id":"oc_chat","chat_type":"p2p","message_type":"text","content":"{\"text\":\"Hello\"}"}}
    })).unwrap();
    handle_webhook_inner(&state, &bot.id, "lark", &HeaderMap::new(), &event)
        .await
        .unwrap();
    let received = read().await.unwrap();
    assert_eq!(received.status, "active");
    assert!(received.webhook_registered);
    assert_eq!(
        received.diagnostics.last_inbound_source.as_deref(),
        Some("webhook")
    );
    assert!(received.diagnostics.last_inbound_at.is_some());
    assert!(received.diagnostics.last_webhook_at.is_some());
    assert_eq!(
        received.diagnostics.last_route_result.as_deref(),
        Some("no_route")
    );
    assert!(received.diagnostics.last_route_at.is_some());
    assert_eq!(
        db.collection::<bson::Document>("channel_messages")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    // Conversely, an actual poll must not be mislabeled by a registered snapshot.
    channel_bot_service::record_inbound_success(
        &db,
        &received,
        crate::services::channel_inbound_service::InboundSource::Poll,
    )
    .await;
    assert_eq!(
        read()
            .await
            .unwrap()
            .diagnostics
            .last_inbound_source
            .as_deref(),
        Some("poll")
    );
    db.drop().await.unwrap();
}

async fn assert_x_outcome(state: &AppState, bot: &str, code: &str) {
    let bot = channel_bot_service::get_bot(&state.db, bot).await.unwrap();
    let counter = bot
        .diagnostics
        .inbound_outcomes
        .get(code)
        .unwrap_or_else(|| panic!("missing {code}"));
    assert!(counter.count > 0);
    assert!(counter.at <= chrono::Utc::now());
    let health = crate::services::channel_bot_health::setup_evidence(&state.db, &bot, false)
        .await
        .unwrap();
    assert_eq!(health["inbound_outcomes"][code]["count"], counter.count);
    assert!(health["inbound_outcomes"][code]["at"].is_string());
    assert_ne!(health["recommended_action_text"], "Healthy.");
    let http = crate::handlers::channel_bots::health_for(&bot, false, true);
    assert_eq!(http.diagnostics.inbound_outcomes[code].count, counter.count);
    assert_eq!(
        http.recommended_action_text,
        health["recommended_action_text"]
    );
}

#[tokio::test]
async fn x_2043_authenticated_filtered_and_invalid_events_have_bounded_metadata() {
    use serde_json::{Value, json};
    let (state, adapter, _server, owner, connection) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let bot = insert_bot(&state, &owner, &connection).await;
    let body = dm(&bot.id, "700");
    handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
        .await
        .unwrap();
    assert_x_outcome(&state, &bot.id, "x_subscription_unregistered").await;
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"webhook_registered":true}},
        )
        .await
        .unwrap();
    let base: Value = serde_json::from_slice(&body).unwrap();
    let edits = [
        (
            "/data/payload/direct_message_events",
            json!([]),
            "x_empty_batch",
        ),
        (
            "/data/payload/direct_message_events/0/type",
            json!("participants_join"),
            "x_event_unsupported",
        ),
        (
            "/data/payload/direct_message_events/0/message_create/sender_id",
            json!("10"),
            "x_self_event",
        ),
        (
            "/data/payload/direct_message_events/0/message_create/target/recipient_id",
            json!("99"),
            "x_recipient_mismatch",
        ),
        (
            "/data/payload/direct_message_events/0/id",
            json!("invalid"),
            "x_payload_invalid",
        ),
        (
            "/data/event_type",
            json!("chat.received"),
            "x_event_not_selected",
        ),
        (
            "/data/event_type",
            json!("chat.sent"),
            "x_event_unsupported",
        ),
    ];
    for (pointer, value, code) in edits {
        let mut payload = base.clone();
        *payload.pointer_mut(pointer).unwrap() = value;
        let bytes = serde_json::to_vec(&payload).unwrap();
        let _ = handle_webhook_inner(&state, &bot.id, "x", &sign(&bytes), &bytes).await;
        assert_x_outcome(&state, &bot.id, code).await;
    }
    handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
        .await
        .unwrap();
    assert_x_outcome(&state, &bot.id, "x_route_missing").await;
    // A valid app signature is insufficient when account/tag binding differs.
    let before = raw_bot(&state, &bot.id).await.get("diagnostics").cloned();
    for pointer in ["/data/filter/user_id", "/data/tag"] {
        let mut payload = base.clone();
        *payload.pointer_mut(pointer).unwrap() = json!("99");
        let bytes = serde_json::to_vec(&payload).unwrap();
        assert!(
            handle_webhook_inner(&state, &bot.id, "x", &sign(&bytes), &bytes)
                .await
                .is_err()
        );
    }
    assert_eq!(
        before,
        raw_bot(&state, &bot.id).await.get("diagnostics").cloned()
    );
    // Locally revoked OAuth refuses before routing, without a provider request.
    state
        .db
        .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .update_one(doc! {"_id":&connection}, doc! {"$set":{"status":"revoked"}})
        .await
        .unwrap();
    assert!(
        handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
            .await
            .is_err()
    );
    assert_x_outcome(&state, &bot.id, "x_connection_unavailable").await;
    let serialized =
        serde_json::to_string(&raw_bot(&state, &bot.id).await.get("diagnostics")).unwrap();
    assert!(!serialized.contains("diagnostics probe"));
    assert!(!serialized.contains("api-secret"));
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn x_2043_missing_event_type_is_diagnosed_on_both_webhook_paths() {
    use serde_json::{Value, json};
    let (state, adapter, server, owner, connection) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let bot = insert_bot(&state, &owner, &connection).await;
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"webhook_registered":true}},
        )
        .await
        .unwrap();
    let base: Value = serde_json::from_slice(&dm(&bot.id, "701")).unwrap();
    let mut count = 0;
    for event_type in [Some(Value::Null), Some(json!(123)), None] {
        let mut payload = base.clone();
        if let Some(value) = event_type {
            payload["data"]["event_type"] = value;
        } else {
            payload["data"]
                .as_object_mut()
                .unwrap()
                .remove("event_type");
        }
        let bytes = serde_json::to_vec(&payload).unwrap();
        for shared in [true, false] {
            if shared {
                dispatch_platform_webhook(&state, "x", &sign(&bytes), &bytes)
                    .await
                    .unwrap();
            } else {
                handle_webhook_inner(&state, &bot.id, "x", &sign(&bytes), &bytes)
                    .await
                    .unwrap();
            }
            count += 1;
            assert_x_outcome(&state, &bot.id, "x_payload_invalid").await;
            let current = channel_bot_service::get_bot(&state.db, &bot.id)
                .await
                .unwrap();
            assert_eq!(
                current.diagnostics.inbound_outcomes["x_payload_invalid"].count,
                count
            );
        }
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("channel_messages")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn x_2043_both_official_encrypted_shapes_are_metadata_only_and_notice_is_bounded() {
    use crate::models::channel_bot::{ChannelBot, XChannelEvent};
    use serde_json::json;
    let (state, adapter, server, owner, connection) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let mut bot = insert_bot(&state, &owner, &connection).await;
    bot.webhook_registered = true;
    bot.x_events = Some(vec![XChannelEvent::Dm, XChannelEvent::Chat]);
    state
        .db
        .collection::<ChannelBot>("channel_bots")
        .replace_one(doc! {"_id":&bot.id}, &bot)
        .await
        .unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>("api_keys").insert_one(doc!{
        "_id":&key,"user_id":&owner,"name":"legacy","key_prefix":"nyxid_ag","key_hash":"deadbeef".repeat(8),
        "scopes":"read write","is_active":true,"callback_url":format!("{}/callback",server.uri()),"created_at":bson::DateTime::now()
    }).await.unwrap();
    state.db.collection::<bson::Document>("channel_conversations").insert_one(doc!{
        "_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"channel_bot_id":&bot.id,"platform":"x",
        "platform_conversation_id":"*","platform_conversation_type":"private","agent_api_key_id":key,
        "default_agent":true,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).await.unwrap();
    let (notice, mut notices) = tokio::sync::mpsc::unbounded_channel();
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .insert(bot.id.clone(), notice);
    for modern in [false, true] {
        let id = uuid::Uuid::new_v4().to_string();
        let mut payload = json!({"data":{"event_type":"chat.received","filter":{"user_id":"10"},"tag":format!("nyxid:{}",bot.id),
            "payload":{"sender_id":"2","conversation_id":if modern {"2-10"} else {"2:10"},
                "encoded_event":"ciphertext-must-not-escape","conversation_key_change_event":"private-key-change"}}});
        if modern {
            payload["data"]["event_uuid"] = json!(id);
        } else {
            payload["data"]["payload"]["id"] = json!(id);
        }
        let bytes = serde_json::to_vec(&payload).unwrap();
        for _ in 0..2 {
            handle_webhook_inner(&state, &bot.id, "x", &sign(&bytes), &bytes)
                .await
                .unwrap();
        }
    }
    assert_x_outcome(&state, &bot.id, "x_encrypted_chat_unsupported").await;
    assert_x_outcome(&state, &bot.id, "x_inbound_duplicate").await;
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("channel_messages")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let notifications = state
        .db
        .collection::<bson::Document>(crate::models::channel_activity::NOTIFICATIONS_COLLECTION)
        .find(doc! {})
        .await
        .unwrap();
    use futures::TryStreamExt;
    let notifications = notifications.try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(notifications.len(), 2);
    assert!(
        notifications
            .iter()
            .all(|n| n.get_str("callback_status") == Ok("not_enabled"))
    );
    let raw = serde_json::to_string(&notifications).unwrap();
    assert!(!raw.contains("ciphertext-must-not-escape"));
    assert!(!raw.contains("private-key-change"));
    let (recipient, notice) =
        tokio::time::timeout(std::time::Duration::from_secs(5), notices.recv())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(recipient, owner);
    assert!(
        notice
            .recommended_action_text
            .contains("cannot read or reply")
    );
    assert!(notices.try_recv().is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .remove(&bot.id);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn x_2043_callback_failure_duplicate_and_owner_fence_are_visible_without_replay() {
    let (state, adapter, server, owner, connection) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let mut bot = insert_bot(&state, &owner, &connection).await;
    bot.webhook_registered = true;
    state
        .db
        .collection::<crate::models::channel_bot::ChannelBot>("channel_bots")
        .replace_one(doc! {"_id":&bot.id}, &bot)
        .await
        .unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>("api_keys").insert_one(doc!{
        "_id":&key,"user_id":&owner,"name":"agent","key_prefix":"nyxid_ag","key_hash":"deadbeef".repeat(8),
        "scopes":"read write","is_active":true,"callback_url":format!("{}/callback",server.uri()),"created_at":bson::DateTime::now()
    }).await.unwrap();
    state.db.collection::<bson::Document>("channel_conversations").insert_one(doc!{
        "_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"channel_bot_id":&bot.id,"platform":"x",
        "platform_conversation_id":"*","platform_conversation_type":"private","agent_api_key_id":&key,
        "default_agent":true,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).await.unwrap();
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(503).set_body_string("private-upstream-content"))
        .expect(1)
        .mount(&server)
        .await;
    let body = dm(&bot.id, "900");
    for _ in 0..2 {
        handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
            .await
            .unwrap();
    }
    assert_x_outcome(&state, &bot.id, "x_callback_failed").await;
    assert_x_outcome(&state, &bot.id, "x_inbound_duplicate").await;
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        current.diagnostics.last_inbound_outcome.unwrap().code,
        "x_callback_failed"
    );
    let admitted = state
        .db
        .collection::<bson::Document>("channel_messages")
        .find_one(doc! {"channel_bot_id":&bot.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admitted.get_str("callback_status").unwrap(), "failed");
    assert!(
        !serde_json::to_string(&current.diagnostics.inbound_outcomes)
            .unwrap()
            .contains("private-upstream-content")
    );
    server.verify().await;
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(
            ResponseTemplate::new(202)
                .set_body_json(serde_json::json!({"status":"refused","reason":"sender_denied"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let refused = dm(&bot.id, "902");
    handle_webhook_inner(&state, &bot.id, "x", &sign(&refused), &refused)
        .await
        .unwrap();
    assert_x_outcome(&state, &bot.id, "x_callback_refused").await;
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        current.diagnostics.last_inbound_outcome.unwrap().code,
        "x_callback_refused"
    );
    let message = state
        .db
        .collection::<bson::Document>("channel_messages")
        .find_one(doc! {"channel_bot_id":&bot.id,"platform_message_id":"902"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        message
            .get_document("callback_disposition")
            .unwrap()
            .get_str("reason")
            .unwrap(),
        "sender_denied"
    );
    state
        .db
        .collection::<bson::Document>("users")
        .update_one(doc! {"_id":&owner}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    // Direct invocation of the post-parse pipeline exercises the admission fence
    // after authentication raced with owner deactivation. The callback is never retried.
    let inbound = adapter.parse_inbound(&dm(&bot.id, "901")).await.unwrap();
    let _ = crate::services::channel_inbound_service::process_inbound_messages(
        crate::services::channel_inbound_service::InboundSource::Webhook,
        crate::services::channel_inbound_service::InboundDeps::from(&state),
        &bot,
        &adapter,
        &inbound,
    )
    .await;
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let code = &current
        .diagnostics
        .last_inbound_outcome
        .as_ref()
        .unwrap()
        .code;
    assert!(matches!(
        code.as_str(),
        "x_authority_changed" | "x_admission_refused"
    ));
    assert_x_outcome(&state, &bot.id, code).await;
    server.verify().await;
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn x_2043_disabled_and_native_authority_refusals_are_bound_to_the_signed_bot() {
    let (state, adapter, server, owner, connection) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let bot = insert_bot(&state, &owner, &connection).await;
    let body = dm(&bot.id, "950");
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"is_active":false,"status":"failed"}},
        )
        .await
        .unwrap();
    // Both the shared app URL and the per-bot URL diagnose, without reviving it.
    dispatch_platform_webhook(&state, "x", &sign(&body), &body)
        .await
        .unwrap();
    handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
        .await
        .unwrap();
    assert_x_outcome(&state, &bot.id, "x_bot_inactive").await;
    let before = raw_bot(&state, &bot.id).await;
    assert!(!before.get_bool("is_active").unwrap());
    let snapshot = before.get("diagnostics").cloned();
    let _ = handle_webhook_inner(&state, &bot.id, "x", &HeaderMap::new(), &body).await;
    assert_eq!(
        raw_bot(&state, &bot.id).await.get("diagnostics").cloned(),
        snapshot
    );
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"is_active":true,"status":"active","platform_entry_pending":true}},
        )
        .await
        .unwrap();
    assert!(
        handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
            .await
            .is_err()
    );
    assert_x_outcome(&state, &bot.id, "x_authority_changed").await;
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("channel_messages")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn x_2043_route_key_and_storage_refusals_never_dispatch_content() {
    let (state, adapter, server, owner, connection) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let bot = insert_bot(&state, &owner, &connection).await;
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"webhook_registered":true}},
        )
        .await
        .unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>("api_keys").insert_one(doc! {
        "_id":&key,"user_id":&owner,"name":"agent","key_prefix":"nyxid_ag","key_hash":"deadbeef".repeat(8),
        "scopes":"read write","is_active":true,"callback_url":format!("{}/callback",server.uri()),"created_at":bson::DateTime::now()
    }).await.unwrap();
    let route = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>("channel_conversations").insert_one(doc! {
        "_id":&route,"user_id":&owner,"channel_bot_id":&bot.id,"platform":"x",
        "platform_conversation_id":"*","platform_conversation_type":"private","agent_api_key_id":&key,
        "default_agent":true,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":"invalid-date"
    }).await.unwrap();
    let body = dm(&bot.id, "960");
    handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
        .await
        .unwrap();
    assert_x_outcome(&state, &bot.id, "x_route_unavailable").await;
    // A structurally valid route with the wrong platform must not sign a callback.
    state
        .db
        .collection::<bson::Document>("channel_conversations")
        .update_one(
            doc! {"_id":&route},
            doc! {"$set":{"updated_at":bson::DateTime::now(),"platform":"telegram"}},
        )
        .await
        .unwrap();
    let body = dm(&bot.id, "961");
    handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
        .await
        .unwrap();
    assert_x_outcome(&state, &bot.id, "x_callback_key_unavailable").await;
    state
        .db
        .collection::<bson::Document>("channel_conversations")
        .update_one(doc! {"_id":&route}, doc! {"$set":{"platform":"x"}})
        .await
        .unwrap();
    // A real Mongo validator refuses admission, while diagnostics stay writable.
    state.db.run_command(doc! {"collMod":"channel_messages","validator":{"$jsonSchema":{"required":["test_refuse_admission"]}}}).await.unwrap();
    let body = dm(&bot.id, "962");
    handle_webhook_inner(&state, &bot.id, "x", &sign(&body), &body)
        .await
        .unwrap();
    assert_x_outcome(&state, &bot.id, "x_storage_unavailable").await;
    assert!(
        state
            .db
            .collection::<bson::Document>("channel_messages")
            .find_one(doc! {"platform_message_id":"962"})
            .await
            .unwrap()
            .is_none()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    let bot = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(bot.diagnostics.last_outbound_at.is_none());
    state.db.drop().await.unwrap();
}
