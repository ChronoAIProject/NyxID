use super::*;
use crate::models::channel_activity::NOTIFICATIONS_COLLECTION;
use crate::services::{
    channel_activity_callback_service as callbacks, channel_activity_service as activities,
    channel_x_tests,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::{Digest, Sha256};
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

fn chat(bot: &ChannelBot, id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"data": {
        "event_type": "chat.received", "filter": {"user_id": "10"}, "tag": format!("nyxid:{}", bot.id),
        "payload": {"id": id, "sender_id": "20", "conversation_id": "20:10", "created_at_msec": "1784841183370",
            "conversation_token": "must-not-escape", "encoded_event": "private-encrypted-bytes", "message_event_signature": {"signature": "private-signature"}}
    }})).unwrap()
}

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

async fn dispatch(state: &AppState, body: &[u8]) {
    // The duplicate-delivery case polls two complete webhook pipelines together.
    Box::pin(
        crate::handlers::channel_webhooks::dispatch_platform_webhook(state, "x", &sign(body), body),
    )
    .await
    .unwrap();
}

async fn route(state: &AppState, id: &str) -> ChannelConversation {
    state
        .db
        .collection::<ChannelConversation>(CONVERSATIONS)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn typed_x_notifications_are_durable_gated_signed_deduplicated_and_never_replyable() {
    for metered in [false, true] {
        let (mut state, adapter, server, owner, connection) = channel_x_tests::fixture().await;
        if metered {
            channel_x_tests::billing::enable_billing(&mut state, &owner).await;
        }
        channel_x_tests::webhooks::credentials(&state, &adapter, &owner).await;
        activities::ensure_indexes(&state.db).await.unwrap();
        let mut bot = channel_x_tests::insert_bot(&state, &owner, &connection).await;
        bot.x_events = Some(vec![
            crate::models::channel_bot::XChannelEvent::Chat,
            crate::models::channel_bot::XChannelEvent::Posts,
        ]);
        bot.webhook_registered = true;
        state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id": &bot.id}, &bot)
            .await
            .unwrap();
        let agent = uuid::Uuid::new_v4().to_string();
        state.db.collection::<bson::Document>(API_KEYS).insert_one(doc! {
            "_id": &agent, "user_id": &owner, "name": "agent", "key_prefix": "nyxid_ag", "key_hash": "deadbeef".repeat(8),
            "scopes": "read write", "is_active": true, "callback_url": format!("{}/callback", server.uri()), "created_at": bson::DateTime::now(), "state_version": 1_i64,
        }).await.unwrap();
        let assigned = crate::services::channel_routing_service::create_conversation(
            &state.db,
            &owner,
            Some(&bot.id),
            "x",
            "*",
            "private",
            None,
            &agent,
            true,
            false,
        )
        .await
        .unwrap();
        let first = chat(&bot, "e4f4d3fc-8bbf-4928-92eb-e5058d6bb6f6");
        dispatch(&state, &first).await;
        let before = activities::list(&state.db, &owner, Some(&bot.id), None, None, 1, 20)
            .await
            .unwrap();
        assert_eq!(before.total, 1);
        assert_eq!(before.routes[0].count, 1);
        assert_eq!(
            before.activities[0].callback_status.as_deref(),
            Some("not_enabled")
        );
        assert!(route(&state, &assigned.id).await.last_message_at.is_none());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.url.path() != "/callback")
        );
        assert!(
            callbacks::set_enabled(&state.db, &assigned, true)
                .await
                .is_err()
        );

        callbacks::declare(
            &state.db,
            &assigned.id,
            &owner,
            &agent,
            callbacks::Declaration {
                version: 1,
                kinds: vec!["encrypted_chat".into(), "post".into()],
            },
            &adapter,
        )
        .await
        .unwrap();
        let human = crate::test_utils::test_auth_user(&owner);
        let declaration = || callbacks::Declaration {
            version: 1,
            kinds: vec!["post".into(), "encrypted_chat".into()],
        };
        assert!(
            crate::handlers::channel_activities::declare_callback(
                State(state.clone()),
                human.clone(),
                Path(assigned.id.clone()),
                Json(declaration())
            )
            .await
            .is_err()
        );
        let mut agent_auth = human.clone();
        agent_auth.auth_method = AuthMethod::ApiKey;
        agent_auth.api_key_id = Some(agent.clone());
        assert!(
            crate::handlers::channel_activities::enable_callback(
                State(state.clone()),
                agent_auth.clone(),
                Path(assigned.id.clone()),
                Json(crate::handlers::channel_activities::EnableRequest { enabled: true })
            )
            .await
            .is_err()
        );
        crate::handlers::channel_activities::declare_callback(
            State(state.clone()),
            agent_auth.clone(),
            Path(assigned.id.clone()),
            Json(declaration()),
        )
        .await
        .unwrap();
        let declared = route(&state, &assigned.id).await;
        assert!(
            !callbacks::status(&state.db, &declared)
                .await
                .unwrap()
                .enabled
        );
        callbacks::set_enabled(&state.db, &declared, true)
            .await
            .unwrap();
        crate::handlers::channel_activities::declare_callback(
            State(state.clone()),
            agent_auth,
            Path(assigned.id.clone()),
            Json(declaration()),
        )
        .await
        .unwrap();
        assert!(
            callbacks::status(&state.db, &route(&state, &assigned.id).await)
                .await
                .unwrap()
                .enabled,
            "identical declarations preserve consent"
        );
        Mock::given(method("POST"))
            .and(path("/callback"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;

        // Observation before consent is not replayed after consent.
        dispatch(&state, &first).await;
        let second = chat(&bot, "8e75466b-56ec-4e88-bddf-b18f7cc6030e");
        tokio::join!(dispatch(&state, &second), dispatch(&state, &second));
        dispatch(&state, &first).await;
        let requests = server.received_requests().await.unwrap();
        let delivered: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path() == "/callback")
            .collect();
        assert_eq!(delivered.len(), 1);
        let request = delivered[0];
        let body: serde_json::Value = request.body_json().unwrap();
        assert_eq!(body["activity"]["kind"], "encrypted_chat");
        assert_eq!(body["activity"]["version"], 1);
        assert_eq!(body["activity"]["reply_supported"], false);
        assert_eq!(body["content"], json!({"type": "unknown"}));
        assert_eq!(body["conversation"]["platform_id"], "chat:20:10");
        for field in [
            "raw_platform_data",
            "reply_token",
            "thread_id",
            "reply_to_platform_message_id",
        ] {
            assert!(body.get(field).is_none(), "{field}");
        }
        assert!(!request.headers.contains_key("x-nyxid-user-token"));
        let token = request.headers["x-nyxid-callback-token"].to_str().unwrap();
        let claims = crate::crypto::jwt::validate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            token,
        )
        .unwrap();
        assert_eq!(
            claims.body_sha256,
            hex::encode(Sha256::digest(&request.body))
        );
        let after = activities::list(&state.db, &owner, Some(&bot.id), None, None, 1, 20)
            .await
            .unwrap();
        assert_eq!(after.total, 2);
        assert_eq!(
            after.routes[0].last.platform_message_id.as_deref(),
            Some("8e75466b-56ec-4e88-bddf-b18f7cc6030e")
        );
        let (_, total) = crate::services::channel_relay_service::list_messages(
            &state.db,
            &assigned.id,
            &owner,
            1,
            20,
        )
        .await
        .unwrap();
        assert_eq!(total, 0);
        let stored = state
            .db
            .collection::<bson::Document>(NOTIFICATIONS_COLLECTION)
            .find_one(doc! {"_id": body["message_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        for secret in [
            "must-not-escape",
            "private-encrypted-bytes",
            "private-signature",
        ] {
            assert!(!stored.to_string().contains(secret));
            assert!(!String::from_utf8_lossy(&request.body).contains(secret));
        }
        let mut auth = crate::test_utils::test_auth_user(&owner);
        auth.auth_method = AuthMethod::ApiKey;
        auth.api_key_id = Some(agent.clone());
        let reply = AsyncReplyRequest {
            message_id: body["message_id"].as_str().unwrap().into(),
            reply: AsyncReplyBody {
                text: Some("must not send".into()),
                attachments: vec![],
                metadata: None,
            },
        };
        assert!(
            resolve_reply_request_context(&state, &HeaderMap::new(), Some(&auth), &reply)
                .await
                .is_err()
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| !request.url.path().starts_with("/2/dm_conversations"))
        );
        let usage = channel_x_tests::billing::settled(&state).await;
        assert_eq!(usage.len(), if metered { 2 } else { 0 });
        assert!(
            usage
                .iter()
                .all(|row| { row.billing_request_id.starts_with("x-chat-received:") })
        );

        // Callback failure still advances admitted activity, with honest status.
        Mock::given(method("POST"))
            .and(path("/callback"))
            .respond_with(ResponseTemplate::new(503))
            .with_priority(1)
            .mount(&server)
            .await;
        dispatch(&state, &chat(&bot, "12345678-1234-4234-8234-123456789abc")).await;
        let failed = activities::list(&state.db, &owner, Some(&bot.id), None, None, 1, 20)
            .await
            .unwrap();
        assert_eq!(failed.total, 3);
        assert_eq!(
            failed.activities[0].callback_status.as_deref(),
            Some("failed")
        );
        assert!(route(&state, &assigned.id).await.last_message_at.is_none());

        // A key/endpoint change invalidates the declaration, even at the same URL.
        state
            .db
            .collection::<ApiKey>(API_KEYS)
            .update_one(
                doc! {"_id": &agent},
                doc! {"$inc": {"state_version": 1_i64}},
            )
            .await
            .unwrap();
        dispatch(&state, &chat(&bot, "22345678-1234-4234-8234-123456789abc")).await;
        let current = route(&state, &assigned.id).await;
        assert!(
            !callbacks::status(&state.db, &current)
                .await
                .unwrap()
                .declared
        );
        let status = activities::list(&state.db, &owner, Some(&bot.id), None, None, 1, 20)
            .await
            .unwrap();
        assert_eq!(
            status.activities[0].callback_status.as_deref(),
            Some("not_enabled")
        );
        assert_eq!(
            activities::list(&state.db, "another-owner", Some(&bot.id), None, None, 1, 20)
                .await
                .unwrap()
                .total,
            0
        );
        assert_eq!(
            activities::list(&state.db, &owner, Some(&bot.id), None, Some("dm"), 1, 20)
                .await
                .unwrap()
                .total,
            0
        );

        let own_post = serde_json::to_vec(&json!({"data": {"event_type": "post.create", "filter": {"user_id": "10"}, "tag": format!("nyxid:{}", bot.id), "payload": {"id":"500", "author_id":"10", "conversation_id":"500", "text":"own post content"}}})).unwrap();
        dispatch(&state, &own_post).await;
        dispatch(&state, &own_post).await;
        assert_eq!(
            activities::list(&state.db, &owner, Some(&bot.id), None, Some("post"), 1, 20)
                .await
                .unwrap()
                .total,
            1
        );
        callbacks::declare(
            &state.db,
            &assigned.id,
            &owner,
            &agent,
            declaration(),
            &adapter,
        )
        .await
        .unwrap();
        callbacks::set_enabled(&state.db, &route(&state, &assigned.id).await, true)
            .await
            .unwrap();
        let switched = crate::services::channel_routing_service::update_conversation(
            &state.db,
            &assigned.id,
            &owner,
            Some(&agent),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            switched.activity_callback.is_none(),
            "reassignment clears the old receiver contract"
        );
        let total = activities::list(&state.db, &owner, Some(&bot.id), None, None, 1, 20)
            .await
            .unwrap()
            .total;
        state
            .db
            .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
            .delete_one(doc! {"_id": &connection})
            .await
            .unwrap();
        dispatch(&state, &chat(&bot, "32345678-1234-4234-8234-123456789abc")).await;
        assert_eq!(
            activities::list(&state.db, &owner, Some(&bot.id), None, None, 1, 20)
                .await
                .unwrap()
                .total,
            total,
            "disconnected accounts cannot admit chat"
        );
    }
}
