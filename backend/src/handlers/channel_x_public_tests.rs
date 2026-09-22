use super::*;
use crate::services::{
    channel_adapters::x::PUBLIC_SCOPES, channel_platform::PlatformAdapter, channel_x_tests,
};
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

#[tokio::test]
async fn x_public_webhook_to_agent_to_bound_reply_is_deduplicated_and_metered() {
    for billing_enabled in [false, true] {
        let (mut state, adapter, server, owner, connection) = channel_x_tests::fixture().await;
        if billing_enabled {
            channel_x_tests::billing::enable_billing(&mut state, &owner).await;
        }
        channel_x_tests::webhooks::credentials(&state, &adapter, &owner).await;
        state
            .db
            .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &connection},
                doc! {"$set": {"token_scopes": PUBLIC_SCOPES.join(" ")}},
            )
            .await
            .unwrap();
        let mut bot = channel_x_tests::insert_bot(&state, &owner, &connection).await;
        bot.x_events = Some(vec![
            crate::models::channel_bot::XChannelEvent::Mentions,
            crate::models::channel_bot::XChannelEvent::Replies,
        ]);
        bot.webhook_registered = true;
        state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id": &bot.id}, &bot)
            .await
            .unwrap();
        let agent = uuid::Uuid::new_v4().to_string();
        let conversation_id = uuid::Uuid::new_v4().to_string();
        state.db.collection::<bson::Document>(API_KEYS).insert_one(doc! {
        "_id": &agent, "user_id": &owner, "name": "agent", "key_prefix": "nyxid_ag",
        "key_hash": "deadbeef".repeat(8), "scopes": "read write", "is_active": true,
        "callback_url": format!("{}/callback", server.uri()), "created_at": bson::DateTime::now(),
    }).await.unwrap();
        state.db.collection::<bson::Document>(CONVERSATIONS).insert_one(doc! {
        "_id": &conversation_id, "user_id": &owner, "channel_bot_id": &bot.id,
        "platform": "x", "platform_conversation_id": "*", "platform_conversation_type": "channel",
        "agent_api_key_id": &agent, "default_agent": true, "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
        Mock::given(method("POST"))
            .and(path("/callback"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        let event = |kind: &str| {
            serde_json::to_vec(&json!({"data": {
        "event_type": kind, "event_uuid": kind, "tag": format!("nyxid:{}", bot.id), "filter": {"user_id": "10"},
        "payload": {"id": "600", "author_id": "2", "conversation_id": "550", "text": "@support public question",
            "in_reply_to_user_id": "10", "in_reply_to_tweet_id": "550", "entities": {"mentions": [{"id": "10"}]}},
        "includes": {}
    }})).unwrap()
        };
        let sign = |body: &[u8]| {
            let mut mac = Hmac::<Sha256>::new_from_slice(b"api-secret").unwrap();
            mac.update(body);
            HeaderMap::from_iter([(
                "x-twitter-webhooks-signature".parse().unwrap(),
                format!("sha256={}", STANDARD.encode(mac.finalize().into_bytes()))
                    .parse()
                    .unwrap(),
            )])
        };
        let mention = event("post.mention.create");
        let reply = event("post.reply.create");
        let mention_headers = sign(&mention);
        let reply_headers = sign(&reply);
        let (a, b) = tokio::join!(
            crate::handlers::channel_webhooks::dispatch_platform_webhook(
                &state,
                "x",
                &mention_headers,
                &mention
            ),
            crate::handlers::channel_webhooks::dispatch_platform_webhook(
                &state,
                "x",
                &reply_headers,
                &reply
            ),
        );
        a.unwrap();
        b.unwrap();
        // Exercise redelivery after the overlapping receive charges have settled.
        crate::handlers::channel_webhooks::dispatch_platform_webhook(
            &state,
            "x",
            &reply_headers,
            &reply,
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        let callbacks: Vec<_> = requests
            .iter()
            .filter(|r| r.url.path() == "/callback")
            .collect();
        assert_eq!(callbacks.len(), 1);
        assert!(!callbacks[0].headers.contains_key("x-nyxid-user-token"));
        assert!(callbacks[0].headers.contains_key("x-nyxid-callback-token"));
        let callback: serde_json::Value = callbacks[0].body_json().unwrap();
        assert_eq!(callback["content"]["text"], "@support public question");
        assert_eq!(callback["thread_id"], "550");
        assert_eq!(callback["reply_to_platform_message_id"], "550");
        let message_id = callback["message_id"].as_str().unwrap();
        let messages = state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME);
        let stored = messages
            .find_one(doc! {"_id": message_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.get_str("platform_conversation_id").unwrap(),
            "post:550"
        );
        assert!(!stored.to_string().contains("public question"));
        assert!(!stored.contains_key("raw_data"));
        let usage = channel_x_tests::billing::settled(&state).await;
        assert_eq!(usage.len(), usize::from(billing_enabled));
        assert!(usage.iter().all(|row| row.quantity == Some(1)));

        Mock::given(method("POST"))
            .and(path("/2/tweets"))
            .and(header("authorization", "Bearer live-access-token"))
            .and(body_json(
                json!({"text": "Thanks for asking", "reply": {"in_reply_to_tweet_id": "600"}}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"data": {"id": "700"}})))
            .expect(1)
            .mount(&server)
            .await;
        let headers = HeaderMap::from_iter([(
            "authorization".parse().unwrap(),
            format!("Bearer {}", callback["reply_token"].as_str().unwrap())
                .parse()
                .unwrap(),
        )]);
        let body = || AsyncReplyRequest {
            message_id: message_id.into(),
            reply: AsyncReplyBody {
                text: Some("Thanks for asking".into()),
                attachments: vec![],
                metadata: Some(json!({"reply_to": "999", "conversation_id": "post:999"})),
            },
        };
        let context = resolve_reply_request_context(&state, &headers, None, &body())
            .await
            .unwrap();
        let response = deliver_async_reply(&state, &headers, context, body(), &adapter)
            .await
            .unwrap()
            .0;
        assert_eq!(response.platform_message_id.as_deref(), Some("700"));
        assert!(
            resolve_reply_request_context(&state, &headers, None, &body())
                .await
                .is_err()
        );
        assert_eq!(messages.count_documents(doc! {}).await.unwrap(), 2);
        let usage = channel_x_tests::billing::settled(&state).await;
        assert_eq!(usage.len(), if billing_enabled { 2 } else { 0 });
        assert!(usage.iter().all(|row| row.quantity == Some(1)));
        if billing_enabled {
            assert!(
                usage
                    .iter()
                    .any(|row| row.api_key_id.as_deref() == Some(&agent))
            );
        }
        let caps = crate::services::channel_adapters::conversation_capabilities(
            "x",
            "post:550",
            &state.token_exchange_cache,
        );
        assert!(!caps.outbound.initiated_send);
        assert!(caps.outbound.reply_to);
        assert!(caps.media.outbound.is_empty());
        assert!(
            adapter
                .send_reply(
                    &state.http_client,
                    &crate::services::channel_platform::BotCredentials {
                        billing: None,
                        token: "unused",
                        platform_bot_id: Some("10"),
                        platform_secrets: None,
                    },
                    "post:550",
                    &OutboundReply {
                        text: Some("unsolicited".into()),
                        attachments: vec![],
                        metadata: None,
                        reply_to_platform_message_id: None
                    }
                )
                .await
                .is_err()
        );
    }
}
