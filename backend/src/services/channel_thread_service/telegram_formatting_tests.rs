use super::components;
use crate::{
    errors::AppError,
    models::channel_thread::{ChannelThreadFacts, TelegramTopic, ThreadKind},
    services::{
        channel_adapters::{telegram::TelegramAdapter, telegram_new::TelegramNewAdapter},
        channel_platform::{
            BotCredentials, MaterializedAttachment, MediaKind, OutboundReply, PlatformAdapter,
        },
        channel_reply_plan,
        channel_thread_service::ThreadReplyTarget,
        telegram_formatting,
    },
};
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

const SOURCE: &str = r"**🦉中文** [link](https://example.test/a_(b)) and §";
const DISPLAY: &str = "🦉中文 link and §";
const SEND: &str = "/botfixture/sendMessage";

fn adapter(platform: &str, base: &str) -> Box<dyn PlatformAdapter> {
    if platform == "telegram" {
        Box::new(TelegramAdapter::media_test_adapter(base))
    } else {
        Box::new(TelegramNewAdapter::media_test_adapter(base))
    }
}

fn target(platform: &str, topic: bool) -> ThreadReplyTarget {
    ThreadReplyTarget::fixture(
        platform,
        ChannelThreadFacts {
            chat_id: "-10022".into(),
            message_id: "51".into(),
            kind: if topic {
                ThreadKind::Topic
            } else {
                ThreadKind::ReplyChain
            },
            root_id: Some("73".into()),
            native_thread_id: topic.then(|| "73".into()),
            telegram_topic: Some(if topic {
                TelegramTopic::Topic { id: "73".into() }
            } else {
                TelegramTopic::Chat
            }),
            ..Default::default()
        },
    )
}

fn part(platform: &str, format: &str, allow_pre_rendered: bool) -> OutboundReply {
    let mut reply = OutboundReply {
        text: Some(SOURCE.into()),
        metadata: Some(json!({
            "text_format":format,"message_thread_id":999,"protect_content":true,
            "reply_markup":{"remove_keyboard":true},
        })),
        attachments: vec![],
        reply_to_platform_message_id: Some("998".into()),
    };
    if format == "telegram_entities" {
        reply.metadata.as_mut().unwrap()["entities"] =
            json!([{"type":"italic","offset":0,"length":6}]);
        reply.metadata.as_mut().unwrap()["parse_mode"] = json!("HTML");
    }
    if allow_pre_rendered {
        reply = channel_reply_plan::assistant_parts(SOURCE, platform, "").remove(0);
    }
    components(&reply, platform, allow_pre_rendered)
        .unwrap()
        .remove(0)
}

fn expected(topic: bool, formatted: bool) -> Value {
    let mut body = if formatted {
        json!({"chat_id":"-10022","text":DISPLAY,"entities":[
            {"type":"bold","offset":0,"length":4},
            {"type":"text_link","url":"https://example.test/a_(b)","offset":5,"length":4},
        ]})
    } else {
        json!({"chat_id":"-10022","text":SOURCE,"parse_mode":"Markdown"})
    };
    body["reply_to_message_id"] = json!(51);
    if topic {
        body["message_thread_id"] = json!(73);
    }
    body
}

fn provider_response(status: u16, code: u16, description: &str) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .set_body_json(json!({"ok":false,"error_code":code,"description":description}))
}

#[tokio::test]
async fn bound_telegram_sends_server_entities_and_keeps_legacy_wire() {
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    for platform in ["telegram", "telegram-new"] {
        for topic in [false, true] {
            let server = MockServer::start().await;
            Mock::given(path(SEND))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "ok":true,"result":{"message_id":100},
                })))
                .expect(3)
                .mount(&server)
                .await;
            let adapter = adapter(platform, &server.uri());
            let target = target(platform, topic);
            for (format, pre_rendered) in [
                ("commonmark", false),
                ("commonmark", true),
                ("telegram_entities", false),
            ] {
                let result = adapter
                    .send_thread_reply(
                        &http,
                        &BotCredentials::from("fixture"),
                        &target,
                        &part(platform, format, pre_rendered),
                    )
                    .await
                    .unwrap();
                assert_eq!(result.as_deref(), Some("100"));
            }
            let requests = server.received_requests().await.unwrap();
            for (index, request) in requests.iter().enumerate() {
                assert_eq!(
                    request.body,
                    serde_json::to_vec(&expected(topic, index < 2)).unwrap()
                );
            }
        }
    }
}

#[tokio::test]
async fn bound_telegram_keeps_refusals_definite_and_ambiguous_delivery_uncertain() {
    let http = reqwest::Client::builder()
        .no_proxy()
        .read_timeout(Duration::from_millis(250))
        .build()
        .unwrap();
    for platform in ["telegram", "telegram-new"] {
        for topic in [false, true] {
            for (format_refused, terminal, definite) in [
                (
                    true,
                    provider_response(400, 400, "private diagnostic fixture"),
                    true,
                ),
                (
                    false,
                    provider_response(503, 400, "can't parse entities"),
                    false,
                ),
                (
                    false,
                    ResponseTemplate::new(200).set_body_string("malformed"),
                    false,
                ),
                (
                    false,
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_secs(2))
                        .set_body_json(json!({"ok":true,"result":{"message_id":100}})),
                    false,
                ),
            ] {
                let server = MockServer::start().await;
                let attempts = AtomicUsize::new(0);
                Mock::given(path(SEND))
                    .respond_with(move |_: &wiremock::Request| {
                        if attempts.fetch_add(1, Ordering::SeqCst) == 0 && format_refused {
                            provider_response(400, 400, "can't parse entities")
                        } else {
                            terminal.clone()
                        }
                    })
                    .expect(if format_refused { 2 } else { 1 })
                    .mount(&server)
                    .await;
                let error = adapter(platform, &server.uri())
                    .send_thread_reply(
                        &http,
                        &BotCredentials::from("fixture"),
                        &target(platform, topic),
                        &part(platform, "commonmark", false),
                    )
                    .await
                    .unwrap_err();
                assert_eq!(telegram_formatting::definite_refusal(&error), definite);
                assert!(
                    matches!(error, AppError::ChannelPlatformError(message) if message
                    == if definite { telegram_formatting::REJECTED } else { crate::services::channel_media_service::DELIVERY_UNCERTAIN })
                );
                let requests = server.received_requests().await.unwrap();
                assert_eq!(
                    requests[0].body,
                    serde_json::to_vec(&expected(topic, true)).unwrap()
                );
                if format_refused {
                    let mut plain = expected(topic, true);
                    plain.as_object_mut().unwrap().remove("entities");
                    assert_eq!(
                        serde_json::from_slice::<Value>(&requests[1].body).unwrap(),
                        plain
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn bound_telegram_refuses_unresolved_target_before_sending() {
    for platform in ["telegram", "telegram-new"] {
        let server = MockServer::start().await;
        let invalid = ThreadReplyTarget::fixture(
            platform,
            ChannelThreadFacts {
                kind: ThreadKind::Topic,
                chat_id: "-10022".into(),
                message_id: "51".into(),
                ..Default::default()
            },
        );
        let error = adapter(platform, &server.uri())
            .send_thread_reply(
                &reqwest::Client::new(),
                &BotCredentials::from("fixture"),
                &invalid,
                &part(platform, "commonmark", false),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AppError::ChannelConversationNotReachable(_)
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[test]
fn bound_telegram_attachment_only_does_not_post_the_formatting_marker() {
    let commonmark = json!({"text_format":"commonmark","message_thread_id":999});
    let forged = json!({
        "text_format":"telegram_entities","entities":[{"type":"bold","offset":0,"length":1}],
        "parse_mode":"HTML","reply_markup":{"remove_keyboard":true},
    });
    let mut reply = OutboundReply {
        text: None,
        metadata: Some(commonmark.clone()),
        reply_to_platform_message_id: None,
        attachments: vec![MaterializedAttachment {
            kind: MediaKind::Image,
            bytes: Default::default(),
            filename: None,
            mime_type: None,
            caption: None,
        }],
    };
    for platform in ["telegram", "telegram-new"] {
        for text in [None, Some(String::new())] {
            reply.text = text;
            reply.metadata = Some(commonmark.clone());
            let parts = components(&reply, platform, false).unwrap();
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].attachments.len(), 1);
            assert!(parts[0].text.is_none() && parts[0].metadata.is_none());

            reply.metadata = Some(forged.clone());
            let parts = components(&reply, platform, false).unwrap();
            let mut legacy_metadata = forged.clone();
            legacy_metadata
                .as_object_mut()
                .unwrap()
                .remove("text_format");
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0].metadata.as_ref(), Some(&legacy_metadata));
            assert_eq!(parts[1].attachments.len(), 1);
        }
    }
    reply.attachments.clear();
    reply.metadata = Some(commonmark);
    assert!(components(&reply, "telegram", false).is_err());

    let card = json!({"card":{"elements":[]}});
    reply.metadata = Some(card.clone());
    let parts = components(&reply, "lark", false).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].metadata.as_ref(), Some(&card));
}
