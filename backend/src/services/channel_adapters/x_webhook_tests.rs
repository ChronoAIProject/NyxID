use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

#[tokio::test]
async fn encrypted_activity_is_account_selected_and_strips_crypto_material() {
    let adapter = XAdapter::default();
    let mut bot: ChannelBot = bson::from_document(bson::doc! {
        "_id": "bot", "user_id": "owner", "platform": "x", "platform_bot_id": "10", "platform_bot_username": "test",
        "label": "test", "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] },
        "webhook_secret_hash": "", "webhook_registered": true, "is_active": true, "status": "active",
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(), "x_events": ["chat"],
    }).unwrap();
    let original = json!({"data": {"event_type": "chat.received", "filter": {"user_id": "10"}, "tag": "nyxid:bot", "payload": {
        "id": "e4f4d3fc-8bbf-4928-92eb-e5058d6bb6f6", "sender_id": "20", "conversation_id": "20:10", "created_at_msec": "1784841183370",
        "conversation_token": "secret", "encoded_event": "ciphertext", "message_event_signature": {"signature": "crypto"}
    }}});
    let bytes = serde_json::to_vec(&original).unwrap();
    let sign = |bytes: &[u8]| {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"api-secret").unwrap();
        mac.update(bytes);
        HeaderMap::from_iter([(
            "x-twitter-webhooks-signature".parse().unwrap(),
            format!("sha256={}", STANDARD.encode(mac.finalize().into_bytes()))
                .parse()
                .unwrap(),
        )])
    };
    adapter
        .verify_webhook(&bot, Some(&secrets()), &sign(&bytes), &bytes)
        .await
        .unwrap();
    let parsed = adapter.parse_inbound(&bytes).await.unwrap();
    assert_eq!(parsed.len(), 1);
    let message = &parsed[0];
    assert_eq!(message.conversation_id, "chat:20:10");
    assert!(message.text.is_none() && message.attachments.is_empty());
    let raw = message.raw_data.to_string();
    for field in ["secret", "ciphertext", "crypto", "conversation_token"] {
        assert!(!raw.contains(field));
    }
    let metadata = adapter.activity_metadata(message).unwrap();
    assert_eq!(metadata.kind, "encrypted_chat");
    assert_eq!(metadata.content_availability, "encrypted");
    assert!(!metadata.reply_supported);
    assert_eq!(
        metadata.occurred_at.unwrap().timestamp_millis(),
        1784841183370
    );
    assert!(
        adapter
            .verify_webhook(&bot, Some(&secrets()), &HeaderMap::new(), &bytes)
            .await
            .is_err()
    );
    for (key, value) in [
        ("tag", json!("nyxid:other")),
        ("filter", json!({"user_id":"999"})),
    ] {
        let mut altered = original.clone();
        altered["data"][key] = value;
        let altered = serde_json::to_vec(&altered).unwrap();
        assert!(
            adapter
                .verify_webhook(&bot, Some(&secrets()), &sign(&altered), &altered)
                .await
                .is_err()
        );
    }
    bot.x_events = Some(vec![XChannelEvent::Dm]);
    assert!(
        adapter
            .verify_webhook(&bot, Some(&secrets()), &sign(&bytes), &bytes)
            .await
            .is_err()
    );
    for (key, value) in [
        ("id", "123"),
        ("id", "../../invalid"),
        ("sender_id", "abc"),
        ("conversation_id", "20:99"),
        ("conversation_id", "https://evil.test"),
    ] {
        let mut altered = original.clone();
        altered["data"]["payload"][key] = json!(value);
        assert!(
            adapter
                .parse_inbound(&serde_json::to_vec(&altered).unwrap())
                .await
                .is_err(),
            "{key}={value}"
        );
    }
    let mut self_sent = original.clone();
    self_sent["data"]["payload"]["sender_id"] = json!("10");
    assert!(
        adapter
            .parse_inbound(&serde_json::to_vec(&self_sent).unwrap())
            .await
            .unwrap()
            .is_empty()
    );
}

#[test]
fn own_posts_are_distinct_metadata_notifications_without_reply_authority() {
    let event = json!({"data": {"event_type": "post.create", "filter": {"user_id": "10"}, "payload": {
        "id": "400", "author_id": "10", "conversation_id": "300", "text": "own content", "created_at": "2026-09-25T01:00:00Z"
    }}});
    let parsed = webhooks::parse(&serde_json::to_vec(&event).unwrap()).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].conversation_id, "post:300");
    assert!(parsed[0].text.is_none());
    let activity = XAdapter::default().activity_metadata(&parsed[0]).unwrap();
    assert_eq!(activity.kind, "post");
    assert!(!activity.reply_supported);
    let mut other = event;
    other["data"]["payload"]["author_id"] = json!("20");
    assert!(
        webhooks::parse(&serde_json::to_vec(&other).unwrap())
            .unwrap()
            .is_empty()
    );
}

fn secrets() -> PlatformVerifySecrets {
    [
        ("consumer_secret", "api-secret"),
        ("app_bearer_token", "app-token"),
    ]
    .into()
}

pub(crate) fn event() -> Value {
    json!({"data": {"event_type": "dm.received", "event_uuid": "delivery-1",
        "filter": {"user_id": "10"}, "payload": {
            "users": {"2": {"data": {"name": "Sender"}}},
            "direct_message_events": [{"type": "message_create", "id": "500", "message_create": {
                "sender_id": "2", "target": {"recipient_id": "10"}, "message_data": {"text": "hello"}
            }}]
        }
    }})
}

#[test]
fn x_crc_and_signatures_use_api_secret_and_exact_body() {
    let query = [("crc_token".into(), "challenge".into())].into();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"api-secret").unwrap();
    mac.update(b"challenge");
    let expected = format!("sha256={}", STANDARD.encode(mac.finalize().into_bytes()));
    let response: Value =
        serde_json::from_str(&webhooks::handshake(&secrets(), &query).unwrap()).unwrap();
    assert_eq!(response["response_token"], expected);
    assert!(webhooks::handshake(&[("client_secret", "api-secret")].into(), &query).is_err());
    assert!(webhooks::crc_token(&[("crc_token".into(), "a".repeat(257))].into()).is_err());

    let body = serde_json::to_vec(&event()).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"api-secret").unwrap();
    mac.update(&body);
    let headers = HeaderMap::from_iter([(
        "x-twitter-webhooks-signature".parse().unwrap(),
        format!("sha256={}", STANDARD.encode(mac.finalize().into_bytes()))
            .parse()
            .unwrap(),
    )]);
    assert!(webhooks::verify(&secrets(), &headers, &body).is_ok());
    assert!(webhooks::verify(&secrets(), &headers, b"different body").is_err());
    assert!(webhooks::verify(&secrets(), &HeaderMap::new(), &body).is_err());
}

#[test]
fn x_activity_normalizes_recipient_bound_messages_and_media() {
    let mut payload = event();
    let message = &mut payload["data"]["payload"]["direct_message_events"][0];
    message["message_create"]["message_data"]["attachment"] = json!({"media": {
        "id_str": "700", "type": "video", "media_url_https": "https://pbs.twimg.com/preview",
        "video_info": {"variants": [{"content_type": "video/mp4", "bitrate": 10, "url": "https://video.twimg.com/movie"}]}
    }});
    let messages = webhooks::parse(&serde_json::to_vec(&payload).unwrap()).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].conversation_id, "2-10");
    assert_eq!(messages[0].sender_display_name.as_deref(), Some("Sender"));
    assert_eq!(messages[0].text.as_deref(), Some("hello"));
    assert_eq!(
        messages[0].attachments[0].url,
        "https://video.twimg.com/movie"
    );
    payload["data"]["payload"]["direct_message_events"][0]["message_create"]["target"]["recipient_id"] =
        json!("999");
    assert!(
        webhooks::parse(&serde_json::to_vec(&payload).unwrap())
            .unwrap()
            .is_empty()
    );
    payload["data"]["event_type"] = json!("chat.sent");
    assert!(
        webhooks::parse(&serde_json::to_vec(&payload).unwrap())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        webhooks::target(&serde_json::to_vec(&payload).unwrap()).unwrap(),
        None
    );
}

#[tokio::test]
async fn x_webhook_setup_uses_app_token_for_management_and_user_token_for_private_events() {
    let server = MockServer::start().await;
    let adapter = XAdapter {
        api_base: Some(server.uri()),
    };
    let url = "https://nyx.example/api/v1/webhooks/channel/x/platform";
    Mock::given(method("GET"))
        .and(path("/2/webhooks"))
        .and(header("authorization", "Bearer app-token"))
        .and(query_param("webhook_config.fields", "id,url,valid"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"meta": {"result_count": 0}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/webhooks"))
        .and(header("authorization", "Bearer app-token"))
        .and(body_json(json!({"url": url})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data": {"id": "100", "valid": true}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/activity/subscriptions"))
        .and(header("authorization", "Bearer app-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [], "errors": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/2/activity/subscriptions")).and(header("authorization", "Bearer user-token"))
        .and(body_json(json!({"event_type": "dm.received", "filter": {"user_id": "10"}, "webhook_id": "100", "tag": "nyxid:channel"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"subscription": {"subscription_id": "200"}}}))).expect(1).mount(&server).await;
    adapter
        .setup_connection_webhook(
            &reqwest::Client::new(),
            &BotCredentials {
                billing: None,
                token: "user-token",
                platform_bot_id: Some("10"),
                platform_secrets: Some(&secrets()),
            },
            &bot(),
            url,
        )
        .await
        .unwrap();
}

#[test]
fn x_webhook_lists_require_explicit_empty_results_and_reject_partial_errors() {
    for body in [
        json!({}),
        json!({"data": null, "meta": {"result_count": 0}}),
        json!({"meta": {"result_count": 1}}),
        json!({"data": [], "errors": [{"detail": "partial failure"}]}),
        json!({"meta": {"result_count": 0}, "errors": {}}),
    ] {
        assert!(webhooks::list_data(&body).is_err(), "{body}");
    }
}

#[tokio::test]
async fn x_webhook_repair_reuses_subscriptions_and_delete_preserves_other_channels() {
    let server = MockServer::start().await;
    let adapter = XAdapter {
        api_base: Some(server.uri()),
    };
    let url = "https://nyx.example/api/v1/webhooks/channel/x/platform";
    Mock::given(method("GET"))
        .and(path("/2/webhooks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": [{"id": "100", "url": url, "valid": true}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/2/activity/subscriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
            {"subscription_id": "200", "event_type": "dm.received", "filter": {"user_id": "10"}, "webhook_id": "100", "tag": "nyxid:channel"},
            {"subscription_id": "201", "event_type": "dm.received", "filter": {"user_id": "11"}, "webhook_id": "100", "tag": "nyxid:other"}
        ]}))).mount(&server).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/200"))
        .and(header("authorization", "Bearer app-token"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/201"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    adapter
        .setup_connection_webhook(
            &reqwest::Client::new(),
            &BotCredentials {
                billing: None,
                token: "user-token",
                platform_bot_id: Some("10"),
                platform_secrets: Some(&secrets()),
            },
            &bot(),
            url,
        )
        .await
        .unwrap();
    adapter
        .remove_connection_webhook(&reqwest::Client::new(), &secrets(), "channel")
        .await
        .unwrap();
}

#[tokio::test]
async fn x_webhooks_reject_unsafe_callback_urls_before_provider_effects() {
    let adapter = XAdapter::default();
    for url in ["http://nyx.example/hook", "https://nyx.example:8443/hook"] {
        assert!(
            adapter
                .setup_connection_webhook(
                    &reqwest::Client::new(),
                    &BotCredentials {
                        billing: None,
                        token: "user-token",
                        platform_bot_id: Some("10"),
                        platform_secrets: Some(&secrets()),
                    },
                    &bot(),
                    url
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn x_webhook_repoint_requires_provider_confirmation() {
    for confirmed in [true, false] {
        let server = MockServer::start().await;
        let adapter = XAdapter {
            api_base: Some(server.uri()),
        };
        let url = "https://nyx.example/api/v1/webhooks/channel/x/platform";
        Mock::given(method("GET"))
            .and(path("/2/webhooks"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": [{"id": "100", "url": url, "valid": true}]})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/2/activity/subscriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{
                "subscription_id": "200", "event_type": "dm.received", "filter": {"user_id": "10"},
                "webhook_id": "99", "tag": "nyxid:channel"
            }]})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/2/activity/subscriptions/200"))
            .and(header("authorization", "Bearer user-token"))
            .and(body_json(
                json!({"webhook_id": "100", "tag": "nyxid:channel"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(if confirmed {
                json!({"data": {"subscription": {"subscription_id": "200", "webhook_id": "100"}}})
            } else {
                json!({"errors": [{"detail": "private upstream error"}]})
            }))
            .expect(1)
            .mount(&server)
            .await;
        let result = adapter
            .setup_connection_webhook(
                &reqwest::Client::new(),
                &BotCredentials {
                    billing: None,
                    token: "user-token",
                    platform_bot_id: Some("10"),
                    platform_secrets: Some(&secrets()),
                },
                &bot(),
                url,
            )
            .await;
        assert_eq!(result.is_ok(), confirmed);
        if let Err(error) = result {
            assert!(!error.to_string().contains("private upstream error"));
        }
    }
}

fn bot() -> ChannelBot {
    serde_json::from_value(json!({
        "_id": "channel", "user_id": "owner", "platform": "x", "label": "X",
        "credential_source": "connection", "bot_token_encrypted": [], "platform_bot_id": "10",
        "platform_bot_username": "test", "webhook_registered": true, "webhook_secret_hash": "",
        "status": "active", "is_active": true,
        "created_at": {"$date": {"$numberLong": "0"}}, "updated_at": {"$date": {"$numberLong": "0"}}
    }))
    .unwrap()
}

fn post_event(event_type: &str) -> Value {
    json!({"data": {"event_type": event_type, "event_uuid": "delivery-2", "tag": "nyxid:channel",
        "filter": {"user_id": "10"},
        "payload": {"id": "600", "author_id": "2", "conversation_id": "550", "text": "@test hello",
            "in_reply_to_user_id": "10", "in_reply_to_tweet_id": "550",
            "entities": {"mentions": [{"id": "10", "username": "test"}]}},
        "includes": {"users": [{"id": "2", "name": "Reader"}]}
    }})
}

#[test]
fn public_events_preserve_post_identity_and_thread_without_becoming_dms() {
    for event_type in ["post.mention.create", "post.reply.create"] {
        let mut event = post_event(event_type);
        let parsed = webhooks::parse(&serde_json::to_vec(&event).unwrap()).unwrap();
        assert_eq!(parsed.len(), 1);
        let message = &parsed[0];
        assert_eq!(message.platform_message_id, "600");
        assert_eq!(message.conversation_id, "post:550");
        assert_eq!(message.conversation_type, "channel");
        assert_eq!(message.sender_display_name.as_deref(), Some("Reader"));
        assert_eq!(message.thread_id.as_deref(), Some("550"));
        assert_eq!(message.reply_to_platform_message_id.as_deref(), Some("550"));
        event["data"]["payload"]["author_id"] = json!("10");
        assert!(
            webhooks::parse(&serde_json::to_vec(&event).unwrap())
                .unwrap()
                .is_empty()
        );
        event["data"]["payload"]["author_id"] = json!("2");
        event["data"]["payload"]["in_reply_to_user_id"] = json!("99");
        event["data"]["payload"]["entities"]["mentions"] = json!([]);
        assert!(
            webhooks::parse(&serde_json::to_vec(&event).unwrap())
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn public_webhooks_require_live_event_opt_in_and_matching_signature() {
    let adapter = XAdapter::default();
    let mut bot = bot();
    let body = serde_json::to_vec(&post_event("post.mention.create")).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"api-secret").unwrap();
    mac.update(&body);
    let headers = HeaderMap::from_iter([(
        "x-twitter-webhooks-signature".parse().unwrap(),
        format!("sha256={}", STANDARD.encode(mac.finalize().into_bytes()))
            .parse()
            .unwrap(),
    )]);
    assert!(
        adapter
            .verify_webhook(&bot, Some(&secrets()), &headers, &body)
            .await
            .is_err()
    );
    bot.x_events = Some(vec![XChannelEvent::Mentions]);
    adapter
        .verify_webhook(&bot, Some(&secrets()), &headers, &body)
        .await
        .unwrap();
    assert!(
        adapter
            .verify_webhook(&bot, Some(&secrets()), &HeaderMap::new(), &body)
            .await
            .is_err()
    );
    bot.x_events = Some(vec![XChannelEvent::Replies]);
    assert!(
        adapter
            .verify_webhook(&bot, Some(&secrets()), &headers, &body)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn event_selection_reconciles_subscriptions_without_touching_other_channels() {
    let server = MockServer::start().await;
    let adapter = XAdapter {
        api_base: Some(server.uri()),
    };
    let url = "https://nyx.example/api/v1/webhooks/channel/x/platform";
    Mock::given(method("GET"))
        .and(path("/2/webhooks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": [{"id":"100", "valid":true, "url":url}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/2/activity/subscriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
            {"subscription_id":"200", "event_type":"dm.received", "filter":{"user_id":"10"}, "webhook_id":"100", "tag":"nyxid:channel"},
            {"subscription_id":"204", "event_type":"post.mention.create", "filter":{"user_id":"10"}, "webhook_id":"999", "tag":"nyxid:channel"},
            {"subscription_id":"201", "event_type":"post.mention.create", "filter":{"user_id":"10"}, "webhook_id":"100", "tag":"nyxid:channel"},
            {"subscription_id":"202", "event_type":"dm.received", "filter":{"user_id":"20"}, "webhook_id":"100", "tag":"nyxid:other"}
        ]}))).mount(&server).await;
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/200"))
        .and(header("authorization", "Bearer app-token"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/204"))
        .and(header("authorization", "Bearer app-token"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/2/activity/subscriptions"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_json(json!({"event_type":"post.reply.create", "filter":{"user_id":"10"}, "webhook_id":"100", "tag":"nyxid:channel"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"subscription_id":"203"}})))
        .expect(1).mount(&server).await;
    for (name, id) in [("chat.received", "205"), ("post.create", "206")] {
        Mock::given(method("POST")).and(path("/2/activity/subscriptions"))
            .and(header("authorization", "Bearer user-token"))
            .and(body_json(json!({"event_type":name, "filter":{"user_id":"10"}, "webhook_id":"100", "tag":"nyxid:channel"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"subscription_id":id}})))
            .expect(1).mount(&server).await;
    }
    let mut bot = bot();
    bot.x_events = Some(vec![
        XChannelEvent::Mentions,
        XChannelEvent::Replies,
        XChannelEvent::Chat,
        XChannelEvent::Posts,
    ]);
    adapter
        .setup_connection_webhook(
            &reqwest::Client::new(),
            &BotCredentials {
                billing: None,
                token: "user-token",
                platform_bot_id: Some("10"),
                platform_secrets: Some(&secrets()),
            },
            &bot,
            url,
        )
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 7);
}

#[tokio::test]
async fn public_reply_uses_the_persisted_incoming_post_as_its_target() {
    let server = MockServer::start().await;
    let adapter = XAdapter {
        api_base: Some(server.uri()),
    };
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_json(
            json!({"text":"Thanks for asking", "reply":{"in_reply_to_tweet_id":"600"}}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"data":{"id":"700"}})))
        .expect(1)
        .mount(&server)
        .await;
    let db = mongodb::Client::with_uri_str("mongodb://127.0.0.1:27017")
        .await
        .unwrap()
        .database("unused_x_adapter_test");
    let mut bot = bot();
    bot.x_events = Some(vec![XChannelEvent::Mentions]);
    let incoming =
        webhooks::parse(&serde_json::to_vec(&post_event("post.mention.create")).unwrap())
            .unwrap()
            .remove(0);
    let original = crate::services::channel_relay_service::inbound_metadata(
        &bot.id,
        "route",
        &bot.user_id,
        "x",
        &incoming,
        "agent",
        "message",
    );
    let reply = OutboundReply {
        text: Some("Thanks for asking".into()),
        attachments: vec![],
        reply_to_platform_message_id: Some("999".into()),
        metadata: Some(json!({"reply":{"in_reply_to_tweet_id":"999"}})),
    };
    let credentials = BotCredentials::from("user-token");
    assert!(
        adapter
            .send_reply(&reqwest::Client::new(), &credentials, "post:550", &reply)
            .await
            .is_err()
    );
    let id = adapter
        .send_bound_reply(
            &db,
            &reqwest::Client::new(),
            &bot,
            &original,
            &credentials,
            "post:550",
            &reply,
        )
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("700"));
    bot.x_events = None;
    assert!(
        adapter
            .send_bound_reply(
                &db,
                &reqwest::Client::new(),
                &bot,
                &original,
                &credentials,
                "post:550",
                &reply
            )
            .await
            .is_err()
    );
}
