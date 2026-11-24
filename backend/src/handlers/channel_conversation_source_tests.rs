use super::*;
use crate::{
    crypto::jwt,
    models::{
        assistant_message, channel_conversation, channel_message, nyxbot_channel,
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    test_utils::{connect_test_database, test_app_state, test_user},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use bson::Document;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn read(app: &Router, token: &str, route: &str, query: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/channel-conversations/{route}/messages{query}"
            ))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    if status == StatusCode::OK && !query.is_empty() {
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn retained_channel_source_joins_exact_inbound_and_reports_turn_and_reply_independently() {
    let db = connect_test_database("channel_source_result")
        .await
        .unwrap();
    let owner = Uuid::new_v4();
    let other = Uuid::new_v4();
    let bot = Uuid::new_v4().to_string();
    let channel = Uuid::new_v4().to_string();
    let route = Uuid::new_v4().to_string();
    let other_route = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    let retained = bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::hours(1));
    for user in [owner, other] {
        db.collection::<User>(USERS)
            .insert_one(test_user(&user.to_string(), UserType::Person))
            .await
            .unwrap();
    }
    for (id, chat) in [(&route, "oc_fixture"), (&other_route, "oc_other")] {
        db.collection::<Document>(channel_conversation::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": id, "user_id": owner.to_string(), "channel_bot_id": &bot,
                "platform": "lark", "platform_conversation_id": chat, "platform_conversation_type": "group",
                "agent_api_key_id": "fixture-route-key", "is_active": true, "created_at": now, "updated_at": now,
            })
            .await
            .unwrap();
    }
    db.collection::<Document>(nyxbot_channel::COLLECTION_NAME)
        .insert_one(doc! {"_id": &channel, "user_id": owner.to_string(),
        "channel_bot_id": &bot, "platform": "lark", "route_id": &route,
        "route_api_key_id": "fixture-route-key"})
        .await
        .unwrap();

    let aliases = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
    let inbound_ids = [
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
    ];
    let events = db.collection::<Document>(nyxbot_channel::EVENTS_COLLECTION_NAME);
    for (index, (alias, inbound)) in aliases.iter().zip(&inbound_ids).enumerate() {
        let turn = format!("fixture-turn-{index}");
        db.collection::<Document>(channel_message::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": inbound, "user_id": owner.to_string(), "channel_bot_id": &bot,
                "conversation_id": &route, "direction":"inbound", "platform": "lark",
                "platform_message_id": format!("om_fixture_{index}"), "sender_platform_id": "ou_fixture",
                "content_type": "text", "created_at": now, "agent_api_key_id": "fixture-route-key",
            })
            .await
            .unwrap();
        events
            .insert_one(doc! {
                "_id": alias, "event_id": inbound, "relay_message_id": inbound,
                "channel_id": &channel, "user_id": owner.to_string(),
                "conversation_id": "assistant-fixture", "turn_id": &turn,
                "expires_at": if index == 2 {now} else {retained},
                "status": "completed",
                "response_read": {"version": 1, "expires_at": retained, "commands": [
                    {"response_id": format!("resp_fixture_{index}"), "request_hash": "internal"},
                    {"response_id": "resp_duplicate_alias", "request_hash":"internal"}
                ]},
                "event_context_ciphertext": "internal",
                "delivery": if index == 0 {
                    doc! {"version": 1, "state": "sent", "gateway_observation": {"status": 200}}
                } else {
                    doc! {"version": 1, "state": "unknown",
                        "preparation": {"version": 1, "gateway_receipt": {"version": 2, "pending": true}}}
                },
            })
            .await
            .unwrap();
        db.collection::<Document>(assistant_message::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": Uuid::new_v4().to_string(), "conversation_id": "assistant-fixture",
                "turn_id": &turn, "user_id": owner.to_string(), "role": "assistant",
                "status": if index == 0 {"failed"} else {"completed"}, "text": "private fixture",
            })
            .await
            .unwrap();
    }
    let state = test_app_state(db.clone());
    let token = |user| {
        jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            user,
            "openid account:read",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let owner_token = token(&owner);
    let other_token = token(&other);
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let app = private.with_state(state);
    let query = |index: usize| format!("?source_event_id={}", aliases[index]);
    let (status, first) = read(&app, &owner_token, &route, &query(0)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        first,
        json!({
            "source_event_id": aliases[0], "inbound_message_id": inbound_ids[0],
            "platform_message_id": "om_fixture_0", "sender_platform_id": "ou_fixture",
            "response_id": "resp_fixture_0", "turn_status": "failed", "reply_status": "confirmed",
            "reply_evidence": "gateway_reply_receipt", "reply_kind": "failed_turn_output",
        })
    );
    let (status, second) = read(&app, &owner_token, &route, &query(1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        second,
        json!({
            "source_event_id": aliases[1], "inbound_message_id": inbound_ids[1],
            "platform_message_id": "om_fixture_1", "sender_platform_id": "ou_fixture",
            "response_id": "resp_fixture_1", "turn_status": "completed", "reply_status": "pending",
            "reply_evidence": null, "reply_kind": null,
        })
    );
    let missing = format!("?source_event_id={}", "f".repeat(64));
    let (status, missing_body) = read(&app, &owner_token, &route, &missing).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        read(&app, &owner_token, &route, &query(2)).await,
        (status, missing_body.clone())
    );
    assert_eq!(
        read(&app, &owner_token, &other_route, &query(0)).await,
        (status, missing_body)
    );
    assert_eq!(
        read(&app, &other_token, &route, &query(0)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        read(&app, &owner_token, &route, "?source_event_id=guess")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    events.update_one(
        doc! {"_id": &aliases[1]},
        doc! {"$set": {"delivery.state": "sent", "delivery.preparation": {
            "version": 1, "total": 2, "receipts": [
                {"platform_message_ids": ["om_part_one"]}, {"platform_message_ids": ["om_part_two"]},
            ]
        }}},
    ).await.unwrap();
    let (_, confirmed) = read(&app, &owner_token, &route, &query(1)).await;
    assert_eq!(confirmed["reply_kind"], "completed_turn_output");
    assert_eq!(confirmed["reply_evidence"], "platform_part_receipts");
    events
        .update_one(
            doc! {"_id": &aliases[1]},
            doc! {"$set": {"delivery.source_route_id": &other_route}},
        )
        .await
        .unwrap();
    assert_eq!(
        read(&app, &owner_token, &route, &query(1)).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, page) = read(&app, &owner_token, &route, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 3);
    assert_eq!(page["messages"].as_array().unwrap().len(), 3);
    assert!(
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value.get("source_event_id").is_none())
    );
    crate::test_utils::drop_test_database(&db).await;
}
