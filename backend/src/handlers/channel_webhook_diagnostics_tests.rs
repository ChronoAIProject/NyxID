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
