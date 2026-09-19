use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

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
    payload["data"]["event_type"] = json!("chat.received");
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
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
            "channel",
            url,
        )
        .await
        .unwrap();
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
            "channel",
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
                    "channel",
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
                "channel",
                url,
            )
            .await;
        assert_eq!(result.is_ok(), confirmed);
        if let Err(error) = result {
            assert!(!error.to_string().contains("private upstream error"));
        }
    }
}
