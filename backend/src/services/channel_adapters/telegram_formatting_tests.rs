use super::*;
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

fn reply() -> OutboundReply {
    crate::services::channel_reply_plan::assistant_parts(
        "**🦉中文** `C:\\qa\\menu_01.txt` [D1-B-ZH] | မြန်မာ",
        "telegram",
        "",
    )
    .remove(0)
}

#[tokio::test]
async fn formatting_rejection_falls_back_once_with_identical_display_text_and_context() {
    for final_status in [200, 400, 500] {
        let server = MockServer::start().await;
        let count = std::sync::atomic::AtomicUsize::new(0);
        Mock::given(path("/botfixture/sendMessage")).respond_with(move |_: &wiremock::Request| {
            let first=count.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0;
            ResponseTemplate::new(if first {400}else{final_status}).set_body_json(if first || final_status!=200 {
                json!({"ok":false,"error_code":if first {400}else{final_status},"description":"Bad Request: can't parse entities: SECRET_SENTINEL"})
            }else{json!({"ok":true,"result":{"message_id":42}})})
        }).expect(2).mount(&server).await;
        let mut reply = reply();
        reply.reply_to_platform_message_id = Some("7".into());
        reply.metadata.as_mut().unwrap()["message_thread_id"] = 9.into();
        reply.metadata.as_mut().unwrap()["reply_markup"] =
            json!({"inline_keyboard":[[{"text":"Review","callback_data":"action"}]]});
        let result = TelegramAdapter::for_api_base(&server.uri())
            .send_reply(&reqwest::Client::new(), &"fixture".into(), "chat", &reply)
            .await;
        assert_eq!(result.is_ok(), final_status == 200);
        if let Err(error) = result {
            assert!(!error.to_string().contains("SECRET_SENTINEL"));
        }
        let bodies: Vec<serde_json::Value> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        assert_eq!(bodies[0]["text"], bodies[1]["text"]);
        assert!(bodies[0].get("entities").is_some());
        assert_eq!(bodies[0]["message_thread_id"], 9);
        assert_eq!(bodies[0]["reply_to_message_id"], 7);
        assert!(bodies[0]["reply_markup"]["inline_keyboard"].is_array());
        assert!(bodies[1].get("entities").is_none() && bodies[1].get("parse_mode").is_none());
        for key in ["reply_to_message_id", "message_thread_id", "reply_markup"] {
            assert_eq!(bodies[0][key], bodies[1][key]);
        }
    }
}

#[tokio::test]
async fn server_error_malformed_response_and_external_parse_mode_are_never_format_retried() {
    for case in 0..3 {
        let server = MockServer::start().await;
        let response = match case {
            0 => ResponseTemplate::new(500).set_body_json(
                json!({"ok":false,"error_code":400,"description":"can't parse entities"}),
            ),
            1 => ResponseTemplate::new(200).set_body_string("incomplete"),
            _ => ResponseTemplate::new(400).set_body_json(
                json!({"ok":false,"error_code":400,"description":"can't parse entities"}),
            ),
        };
        Mock::given(path("/botfixture/sendMessage"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let mut part = reply();
        if case == 2 {
            part.text = Some("*external* \\|".into());
            part.metadata = Some(json!({"parse_mode":"MarkdownV2"}));
        }
        assert!(
            TelegramAdapter::for_api_base(&server.uri())
                .send_reply(&reqwest::Client::new(), &"fixture".into(), "chat", &part)
                .await
                .is_err()
        );
        let bodies = server.received_requests().await.unwrap();
        if case == 2 {
            let body: serde_json::Value = serde_json::from_slice(&bodies[0].body).unwrap();
            assert_eq!(body["text"], "*external* \\|");
            assert_eq!(body["parse_mode"], "Markdown");
        }
    }
}

#[tokio::test]
async fn response_timeout_is_uncertain_and_never_retried() {
    let server = MockServer::start().await;
    Mock::given(path("/botfixture/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(200))
                .set_body_json(json!({"ok":true,"result":{"message_id":42}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let http = reqwest::Client::builder()
        .read_timeout(std::time::Duration::from_millis(30))
        .build()
        .unwrap();
    let error = TelegramAdapter::for_api_base(&server.uri())
        .send_reply(&http, &"fixture".into(), "chat", &reply())
        .await
        .unwrap_err();
    assert!(!crate::services::telegram_formatting::definite_refusal(
        &error
    ));
}

#[tokio::test]
async fn commonmark_edit_and_media_caption_keep_entities_then_fall_back_cleanly() {
    for caption in [false, true] {
        let server = MockServer::start().await;
        if caption {
            Mock::given(path("/botfixture/editMessageText")).respond_with(ResponseTemplate::new(400).set_body_json(json!({"ok":false,"error_code":400,"description":"Bad Request: there is no text in the message to edit"}))).expect(1).mount(&server).await;
        }
        let count = std::sync::atomic::AtomicUsize::new(0);
        Mock::given(path(if caption {
            "/botfixture/editMessageCaption"
        } else {
            "/botfixture/editMessageText"
        }))
        .respond_with(move |_: &wiremock::Request| {
            if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                ResponseTemplate::new(400).set_body_json(
                    json!({"ok":false,"error_code":400,"description":"can't parse entities"}),
                )
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"ok":true}))
            }
        })
        .expect(2)
        .mount(&server)
        .await;
        TelegramAdapter::for_api_base(&server.uri())
            .edit_reply(
                &reqwest::Client::new(),
                &"fixture".into(),
                "chat",
                "42",
                &OutboundEdit {
                    text: Some("# 🦉标题\n\n`a|b`".into()),
                    metadata: Some(json!({"text_format":"commonmark"})),
                },
            )
            .await
            .unwrap();
        let bodies: Vec<serde_json::Value> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        let last = bodies.last().unwrap();
        let previous = &bodies[bodies.len() - 2];
        let (text, entities) = if caption {
            ("caption", "caption_entities")
        } else {
            ("text", "entities")
        };
        assert_eq!(last[text], previous[text]);
        assert_eq!(last[text], "🦉标题\n\na|b");
        assert!(previous.get(entities).is_some());
        assert!(last.get(entities).is_none() && last.get("parse_mode").is_none());
    }
}
