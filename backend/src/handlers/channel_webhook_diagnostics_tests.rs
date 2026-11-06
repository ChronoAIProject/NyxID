//! Webhook diagnostics: unauthenticated deliveries never write a bot row;
//! authenticated ones record and clear the bot's delivery state.
use super::*;
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
