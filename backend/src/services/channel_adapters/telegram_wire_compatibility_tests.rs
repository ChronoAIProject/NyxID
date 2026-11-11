//! Frozen request JSON from pre-#2336 b3367c63c. Equality includes absent keys
//! and numeric/string types; provider mocks capture the actual serialized body.
use super::*;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

fn context() -> Value {
    json!({"message_thread_id":"7","protect_content":true,
        "reply_to_message_id":999,"reply_parameters":{"message_id":888},
        "link_preview_options":{"is_disabled":true},
        "reply_markup":{"inline_keyboard":[[{"text":"Review","callback_data":"review"}]]},
        "parse_mode":"HTML"})
}

async fn send_capture(reply: &OutboundReply, chat: &str) -> Vec<u8> {
    let server = MockServer::start().await;
    Mock::given(path("/botfixture/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":42}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    TelegramAdapter::for_api_base(&server.uri())
        .send_reply(&reqwest::Client::new(), &"fixture".into(), chat, reply)
        .await
        .unwrap();
    server.received_requests().await.unwrap().remove(0).body
}

#[tokio::test]
async fn legacy_external_and_internal_plain_requests_are_byte_identical_to_pre_pr_json() {
    for chat in ["700", "-100700", "@public_channel"] {
        // Existing internal and external callers with no formatting opt-in.
        let mut reply = OutboundReply {
            text: Some("plain reply".into()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            metadata: None,
        };
        let before = json!({"chat_id":chat,"text":"plain reply","parse_mode":"Markdown"});
        assert_eq!(
            send_capture(&reply, chat).await,
            serde_json::to_vec(&before).unwrap()
        );
        reply.reply_to_platform_message_id = Some("42".into());
        reply.metadata = Some(context());
        let before = json!({"chat_id":chat,"text":"plain reply","parse_mode":"Markdown",
            "protect_content":true,"reply_to_message_id":42,"message_thread_id":7});
        assert_eq!(
            send_capture(&reply, chat).await,
            serde_json::to_vec(&before).unwrap()
        );
        // Legacy explicit entities already replaced the default parse mode.
        reply.metadata.as_mut().unwrap()["entities"] = json!([]);
        let before = json!({"chat_id":chat,"text":"plain reply","entities":[],
            "protect_content":true,"reply_to_message_id":42,"message_thread_id":7});
        assert_eq!(
            send_capture(&reply, chat).await,
            serde_json::to_vec(&before).unwrap()
        );
    }
}

#[tokio::test]
async fn commonmark_direct_and_proactive_plain_requests_only_change_formatting() {
    let mut metadata = context();
    metadata.as_object_mut().unwrap().remove("parse_mode");
    metadata["text_format"] = "commonmark".into();
    let reply = OutboundReply {
        text: Some("plain reply".into()),
        attachments: vec![],
        reply_to_platform_message_id: Some("42".into()),
        metadata: Some(metadata),
    };
    // The only difference from the legacy direct snapshot is parse_mode -> entities.
    let expected = json!({"chat_id":"700","text":"plain reply","entities":[],
        "protect_content":true,"reply_to_message_id":42,"message_thread_id":7});
    assert_eq!(
        send_capture(&reply, "700").await,
        serde_json::to_vec(&expected).unwrap()
    );

    let adapter = TelegramAdapter::default();
    let mut part =
        crate::services::channel_reply_plan::assistant_parts("plain reply", "telegram", "")
            .remove(0);
    // This is the same context hook used by proactive posts.
    adapter.reply_context(Some("7"), chrono::Utc::now(), &mut part.metadata);
    let before =
        json!({"chat_id":"-100700","text":"plain reply","entities":[],"message_thread_id":7});
    assert_eq!(
        send_capture(&part, "-100700").await,
        serde_json::to_vec(&before).unwrap()
    );
}

#[tokio::test]
async fn legacy_and_commonmark_text_and_caption_edits_preserve_their_wire_shape() {
    for commonmark in [false, true] {
        for caption in [false, true] {
            let server = MockServer::start().await;
            Mock::given(path("/botfixture/editMessageText"))
                .respond_with(if caption {
                    ResponseTemplate::new(400).set_body_json(json!({"ok":false,"error_code":400,
                        "description":"Bad Request: there is no text in the message to edit"}))
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({"ok":true}))
                })
                .expect(1)
                .mount(&server)
                .await;
            if caption {
                Mock::given(path("/botfixture/editMessageCaption"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
                    .expect(1)
                    .mount(&server)
                    .await;
            }
            let mut metadata = context();
            if commonmark {
                metadata.as_object_mut().unwrap().remove("parse_mode");
                metadata["text_format"] = "commonmark".into();
            } else {
                // Legacy edits ignored entities as well as send-only metadata.
                metadata["entities"] = json!([{"type":"bold","offset":0,"length":5}]);
            }
            TelegramAdapter::for_api_base(&server.uri())
                .edit_reply(
                    &reqwest::Client::new(),
                    &"fixture".into(),
                    "700",
                    "42",
                    &OutboundEdit {
                        text: Some("plain reply".into()),
                        metadata: Some(metadata),
                    },
                )
                .await
                .unwrap();
            let requests = server.received_requests().await.unwrap();
            for (index, request) in requests.iter().enumerate() {
                let mut before = if index == 0 {
                    json!({"chat_id":"700","message_id":42,"text":"plain reply","parse_mode":"Markdown"})
                } else {
                    json!({"chat_id":"700","message_id":42,"caption":"plain reply","parse_mode":"Markdown"})
                };
                if commonmark {
                    before.as_object_mut().unwrap().remove("parse_mode");
                    before[if index == 0 {
                        "entities"
                    } else {
                        "caption_entities"
                    }] = json!([]);
                }
                assert_eq!(request.body, serde_json::to_vec(&before).unwrap());
            }
        }
    }
}
