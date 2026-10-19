use super::*;
use crate::models::channel_thread::{
    ChannelThreadFacts, ThreadAddress, ThreadKind, ThreadSenderKind,
};
use crate::services::channel_platform::*;
use crate::services::channel_thread_service::*;
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

fn credentials() -> BotCredentials<'static> {
    BotCredentials {
        billing: None,
        token: "app:secret",
        platform_bot_id: Some("bot"),
        platform_secrets: None,
    }
}

fn facts(kind: ThreadKind, chat: &str, id: &str, root: Option<&str>) -> ChannelThreadFacts {
    ChannelThreadFacts {
        version: 1,
        kind,
        chat_id: chat.into(),
        message_id: id.into(),
        root_id: root.map(str::to_owned),
        sender_kind: ThreadSenderKind::Human,
        address: ThreadAddress::Mention,
        ..Default::default()
    }
}

fn reply() -> OutboundReply {
    OutboundReply {
        text: Some("safe notice".into()),
        attachments: vec![],
        reply_to_platform_message_id: Some("other-root".into()),
        metadata: Some(json!({"thread_ts":"wrong", "message_thread_id":999,
            "interaction_thread_id":"interaction:app:DO-NOT-SEND"})),
    }
}

fn media() -> OutboundReply {
    OutboundReply {
        text: None,
        reply_to_platform_message_id: None,
        metadata: None,
        attachments: vec![MaterializedAttachment {
            kind: MediaKind::File,
            bytes: bytes::Bytes::from_static(b"attachment"),
            filename: Some("test.txt".into()),
            mime_type: Some("text/plain".into()),
            caption: None,
        }],
    }
}

async fn tenant(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"tenant_access_token":"tenant","expire":7200})),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn channel_thread_bound_text_and_media_never_use_conflicting_targets() {
    let server = MockServer::start().await;
    let http = reqwest::Client::new();
    let creds = credentials();
    for adapter in [
        Box::new(telegram::TelegramAdapter::media_test_adapter(&server.uri()))
            as Box<dyn PlatformAdapter>,
        Box::new(telegram_new::TelegramNewAdapter::media_test_adapter(
            &server.uri(),
        )),
    ] {
        server.reset().await;
        let mut f = facts(ThreadKind::Topic, "-100", "42", Some("7"));
        f.native_thread_id = Some("7".into());
        let target = ThreadReplyTarget::fixture(adapter.platform_id(), f);
        Mock::given(method("POST"))
            .and(path("/botapp:secret/sendMessage"))
            .and(body_json(
                json!({"chat_id":"-100","text":"safe notice","parse_mode":"Markdown",
                "reply_to_message_id":42,"message_thread_id":7}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":43}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/botapp:secret/sendDocument"))
            .and(body_string_contains("name=\"message_thread_id\"\r\n\r\n7"))
            .and(body_string_contains(
                "name=\"reply_to_message_id\"\r\n\r\n42",
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":44}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            adapter
                .send_thread_reply(&http, &creds, &target, &reply())
                .await
                .unwrap()
                .as_deref(),
            Some("43")
        );
        assert_eq!(
            adapter
                .send_thread_reply(&http, &creds, &target, &media())
                .await
                .unwrap()
                .as_deref(),
            Some("44")
        );
        server.verify().await;
    }
    server.reset().await;
    let adapter = discord::DiscordAdapter::media_test_adapter(&server.uri());
    let target =
        ThreadReplyTarget::fixture("discord", facts(ThreadKind::Native, "20", "42", Some("20")));
    Mock::given(method("POST"))
        .and(path("/channels/20/messages"))
        .and(header("authorization", "Bot app:secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"43"})))
        .expect(2)
        .mount(&server)
        .await;
    adapter
        .send_thread_reply(&http, &creds, &target, &reply())
        .await
        .unwrap();
    adapter
        .send_thread_reply(&http, &creds, &target, &media())
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let text: Value = requests[0].body_json().unwrap();
    assert_eq!(
        text["message_reference"],
        json!({"message_id":"42","channel_id":"20","fail_if_not_exists":true})
    );
    assert_eq!(text["allowed_mentions"]["replied_user"], false);
    let upload = String::from_utf8_lossy(&requests[1].body);
    assert!(upload.contains("\"message_id\":\"42\""));
    assert!(upload.contains("\"fail_if_not_exists\":true"));
    assert!(!upload.contains("DO-NOT-SEND"));
    server.verify().await;
}

#[tokio::test]
async fn channel_thread_lark_family_uses_native_reply_for_text_card_and_file() {
    let server = MockServer::start().await;
    let http = reqwest::Client::new();
    for platform in ["lark", "feishu"] {
        server.reset().await;
        tenant(&server).await;
        let adapter = lark::LarkFamilyAdapter::media_test_adapter(&server.uri(), platform);
        let target = ThreadReplyTarget::fixture(
            platform,
            facts(ThreadKind::Native, "oc_chat", "om_current", Some("om_root")),
        );
        Mock::given(method("POST"))
            .and(path("/open-apis/im/v1/files"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"file_key":"file"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/open-apis/im/v1/messages/om_root/reply"))
            .and(body_partial_json(json!({"reply_in_thread":true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"code":0,"data":{"message_id":"om_sent","thread_id":"omt_alias"}}),
            ))
            .expect(3)
            .mount(&server)
            .await;
        adapter
            .send_thread_reply(&http, &credentials(), &target, &reply())
            .await
            .unwrap();
        adapter
            .send_thread_reply(&http, &credentials(), &target, &media())
            .await
            .unwrap();
        let mut card = reply();
        card.text = None;
        card.metadata = Some(json!({"card":{"elements":[]}}));
        adapter
            .send_thread_reply(&http, &credentials(), &target, &card)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let bodies: Vec<Value> = requests
            .iter()
            .filter(|r| r.url.path().ends_with("/reply"))
            .map(|r| r.body_json().unwrap())
            .collect();
        assert_eq!(
            bodies
                .iter()
                .map(|b| b["msg_type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["text", "file", "interactive"]
        );
        assert!(bodies.iter().all(|b| b.get("receive_id").is_none()));
        server.verify().await;
    }
}

#[tokio::test]
async fn channel_thread_slack_file_completion_and_text_use_root() {
    let server = MockServer::start().await;
    let adapter = slack::SlackAdapter::media_test_adapter(&server.uri());
    let target = ThreadReplyTarget::fixture(
        "slack",
        facts(
            ThreadKind::Native,
            "C1",
            "1700000010.000001",
            Some("1700000000.000001"),
        ),
    );
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .and(body_partial_json(
            json!({"channel":"C1","thread_ts":"1700000000.000001"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"1700000020.000001"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/files.getUploadURLExternal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"ok":true,"file_id":"F1","upload_url":format!("{}/upload",server.uri())}),
        ))
        .mount(&server)
        .await;
    Mock::given(path("/upload"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(path("/files.completeUploadExternal"))
        .and(body_partial_json(
            json!({"channel_id":"C1","thread_ts":"1700000000.000001"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/files.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,
        "file":{"shares":{"public":{"C1":[{"ts":"1700000021.000001"}]}}}})))
        .mount(&server)
        .await;
    adapter
        .send_thread_reply(&reqwest::Client::new(), &credentials(), &target, &reply())
        .await
        .unwrap();
    adapter
        .send_thread_reply(&reqwest::Client::new(), &credentials(), &target, &media())
        .await
        .unwrap();
}

#[tokio::test]
async fn channel_thread_discord_resolves_actual_channel_and_rejects_ambiguous_ancestry() {
    let server = MockServer::start().await;
    let adapter = discord::DiscordAdapter::media_test_adapter(&server.uri());
    let http = reqwest::Client::new();
    let mut f = facts(ThreadKind::Unknown, "20", "42", None);
    for channel in [
        json!({"id":"20","type":11,"parent_id":"10"}),
        json!({"id":"20","type":11,"parent_id":"10","thread_metadata":{"archived":true}}),
        json!({"id":"99","type":11,"parent_id":"10"}),
        json!({"id":"20","type":1}),
    ] {
        server.reset().await;
        Mock::given(path("/channels/20"))
            .respond_with(ResponseTemplate::new(200).set_body_json(channel.clone()))
            .expect(1)
            .mount(&server)
            .await;
        let resolved = adapter
            .resolve_thread(&http, &credentials(), &f, &[])
            .await
            .unwrap();
        if channel["id"] == "20" && channel["type"] == 11 && channel["thread_metadata"].is_null() {
            let r = resolved.unwrap();
            assert_eq!(r.root_id.as_deref(), Some("20"));
            assert_eq!(r.parent_chat_id.as_deref(), Some("10"));
        } else {
            assert!(resolved.is_none());
        }
    }
    server.reset().await;
    Mock::given(path("/channels/20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"20","type":0})))
        .mount(&server)
        .await;
    f.parent_message_id = Some("41".into());
    Mock::given(path("/channels/20/messages/41"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"41","channel_id":"20","message_reference":{"message_id":"42","channel_id":"20"}})))
        .expect(1).mount(&server).await;
    assert!(
        adapter
            .resolve_thread(&http, &credentials(), &f, &[])
            .await
            .unwrap()
            .is_none()
    );
    f.chat_id = "interaction:app:secret".into();
    assert!(
        adapter
            .resolve_thread(&http, &credentials(), &f, &[])
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn channel_thread_telegram_ancestry_is_scoped_and_unknown_parents_do_not_guess() {
    let adapter = telegram::TelegramAdapter::default();
    let mut child = facts(ThreadKind::ReplyChain, "-100", "42", None);
    child.parent_message_id = Some("41".into());
    let mut parent = facts(ThreadKind::ReplyChain, "-100", "41", Some("7"));
    let http = reqwest::Client::new();
    assert!(
        adapter
            .resolve_thread(&http, &credentials(), &child, &[])
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        adapter
            .resolve_thread(&http, &credentials(), &child, &[parent.clone()])
            .await
            .unwrap()
            .unwrap()
            .root_id
            .as_deref(),
        Some("7")
    );
    parent.chat_id = "-999".into();
    assert!(
        adapter
            .resolve_thread(&http, &credentials(), &child, &[parent])
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn channel_thread_slack_history_bounds_scope_denials_and_cursor_budget() {
    let server = MockServer::start().await;
    let adapter = slack::SlackAdapter::media_test_adapter(&server.uri());
    let http = reqwest::Client::new();
    let target = ThreadReplyTarget::fixture(
        "slack",
        facts(
            ThreadKind::Native,
            "C1",
            "1700000100.000001",
            Some("1700000000.000001"),
        ),
    );
    let before = Utc.timestamp_opt(1700000100, 1000).unwrap();
    let mut messages: Vec<_> = (1..=30)
        .map(|i| {
            json!({"ts":format!("17000000{i:02}.000001"),
        "thread_ts":"1700000000.000001","user":"human","text":"界".repeat(2000)})
        })
        .collect();
    messages.insert(
        0,
        json!({"ts":"1700000000.000001","user":"root","text":"root text"}),
    );
    messages.insert(
        1,
        json!({"ts":"1700000001.000009","thread_ts":"other","user":"leak","text":"WRONG THREAD"}),
    );
    messages.insert(
        2,
        json!({"ts":"1700000002.000009","thread_ts":"1700000000.000001",
            "subtype":"bot_message","user":"U_OTHER_BOT","text":"bot history"}),
    );
    Mock::given(path("/conversations.replies"))
        .and(query_param("ts", "1700000000.000001"))
        .and(query_param("latest", "1700000100.000001"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"messages":messages})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let history = adapter
        .thread_history(&http, &credentials(), &target, before)
        .await
        .unwrap();
    assert!(history.partial);
    assert!(history.messages.len() <= HISTORY_MESSAGES);
    assert!(
        history
            .messages
            .iter()
            .all(|m| m.text.len() <= HISTORY_MESSAGE_BYTES)
    );
    assert!(history.messages.iter().map(|m| m.text.len()).sum::<usize>() <= HISTORY_BYTES);
    assert!(
        history
            .messages
            .iter()
            .all(|m| !m.text.contains("WRONG THREAD"))
    );
    assert!(history.messages.iter().any(|m| m.text == "root text"));
    assert!(
        history
            .messages
            .iter()
            .any(|m| { m.sender_id == "U_OTHER_BOT" && m.sender_kind == ThreadSenderKind::Bot })
    );
    assert!(!format!("{history:?}").contains("root text"));
    server.reset().await;
    for (cursor, next) in [("", "one"), ("one", "two"), ("two", "three")] {
        Mock::given(path("/conversations.replies")).and(query_param("cursor",cursor))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"messages":[],"has_more":true,"response_metadata":{"next_cursor":next}})))
            .expect(1).mount(&server).await;
    }
    assert!(
        adapter
            .thread_history(&http, &credentials(), &target, before)
            .await
            .unwrap()
            .partial
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    for response in [
        ResponseTemplate::new(429),
        ResponseTemplate::new(200).set_body_json(json!({"ok":false,"error":"missing_scope"})),
        ResponseTemplate::new(200).set_body_string("x".repeat(HISTORY_RESPONSE_BYTES as usize + 1)),
    ] {
        server.reset().await;
        Mock::given(path("/conversations.replies"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let h = adapter
            .thread_history(&http, &credentials(), &target, before)
            .await
            .unwrap();
        assert!(h.partial && h.messages.is_empty());
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "no retries or scope escalation"
        );
    }
}

#[tokio::test]
async fn channel_thread_discord_history_filters_channel_trigger_system_and_bot_events() {
    let server = MockServer::start().await;
    let adapter = discord::DiscordAdapter::media_test_adapter(&server.uri());
    let target = ThreadReplyTarget::fixture(
        "discord",
        facts(ThreadKind::Native, "20", "100", Some("20")),
    );
    let before = Utc::now();
    let time = (before - chrono::Duration::minutes(1)).to_rfc3339();
    let msg = |id: &str, chat: &str, kind: u8| {
        json!({"id":id,"channel_id":chat,"type":kind,
        "author":{"id":"human"},"content":"history","timestamp":time})
    };
    Mock::given(path("/channels/20/messages"))
        .and(query_param("before", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            msg("99", "20", 0),
            msg("98", "21", 0),
            msg("100", "20", 0),
            msg("97", "20", 7)
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let h = adapter
        .thread_history(&reqwest::Client::new(), &credentials(), &target, before)
        .await
        .unwrap();
    assert_eq!(h.messages.len(), 1);
    assert_eq!(h.messages[0].message_id, "99");
}

#[tokio::test]
async fn channel_thread_lark_alias_resolution_and_known_message_history_are_scoped() {
    let server = MockServer::start().await;
    let http = reqwest::Client::new();
    for platform in ["lark", "feishu"] {
        server.reset().await;
        tenant(&server).await;
        let adapter = lark::LarkFamilyAdapter::media_test_adapter(&server.uri(), platform);
        let mut f = facts(ThreadKind::Native, "oc_chat", "om_trigger", None);
        f.parent_message_id = Some("om_root".into());
        f.native_thread_id = Some("omt_alias".into());
        let now = Utc::now();
        let trigger = json!({"message_id":"om_trigger","chat_id":"oc_chat","root_id":"om_root",
            "thread_id":"omt_alias","create_time":now.timestamp_millis().to_string()});
        Mock::given(path("/open-apis/im/v1/messages/om_trigger"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"items":[trigger]}})),
            )
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(path("/open-apis/im/v1/messages/om_root"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[{
                "message_id":"om_root","chat_id":"oc_chat","msg_type":"text",
                "create_time":(now-chrono::Duration::minutes(1)).timestamp_millis().to_string(),
                "sender":{"id":"ou_human","sender_type":"user"},"body":{"content":"{\"text\":\"thread root\"}"}
            }]}}))).expect(1).mount(&server).await;
        let resolved = adapter
            .resolve_thread(&http, &credentials(), &f, &[])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.root_id.as_deref(), Some("om_root"));
        assert_eq!(resolved.native_thread_id.as_deref(), Some("omt_alias"));
        let target = ThreadReplyTarget::fixture(platform, resolved);
        let h = adapter
            .thread_history(&http, &credentials(), &target, now)
            .await
            .unwrap();
        assert!(h.partial);
        assert_eq!(h.messages.len(), 1);
        assert_eq!(h.messages[0].text, "thread root");
        server.verify().await;
    }
}

#[tokio::test]
async fn channel_thread_reply_failure_never_falls_back_to_parent_or_legacy_send() {
    let server = MockServer::start().await;
    let adapter = discord::DiscordAdapter::media_test_adapter(&server.uri());
    let target =
        ThreadReplyTarget::fixture("discord", facts(ThreadKind::Native, "20", "42", Some("20")));
    Mock::given(path("/channels/20/messages"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"message":"provider private detail","code":50013})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let error = adapter
        .send_thread_reply(&reqwest::Client::new(), &credentials(), &target, &reply())
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("provider private detail"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn channel_thread_lark_alias_only_response_cannot_invent_a_root_or_path() {
    let server = MockServer::start().await;
    tenant(&server).await;
    let adapter = lark::LarkFamilyAdapter::media_test_adapter(&server.uri(), "lark");
    let mut f = facts(ThreadKind::Native, "oc_chat", "om_child", None);
    f.native_thread_id = Some("omt_alias".into());
    Mock::given(path("/open-apis/im/v1/messages/om_child"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[{
                "message_id":"om_child","chat_id":"oc_chat","thread_id":"omt_alias"
            }]}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        adapter
            .resolve_thread(&reqwest::Client::new(), &credentials(), &f, &[])
            .await
            .unwrap()
            .is_none()
    );
    f.root_id = Some("..".into());
    assert!(
        adapter
            .resolve_thread(&reqwest::Client::new(), &credentials(), &f, &[])
            .await
            .unwrap()
            .is_none()
    );
}
