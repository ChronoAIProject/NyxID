use super::*;
use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};
use crate::models::channel_thread::{ThreadAddress, ThreadSenderKind};
use crate::services::channel_platform::{BotCredentials, OutboundReply};
use futures::TryStreamExt;
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

fn credentials() -> BotCredentials<'static> {
    BotCredentials {
        token: "token",
        billing: None,
        platform_bot_id: Some("123"),
        platform_secrets: None,
    }
}

async fn fixture(platform: &str) -> (mongodb::Database, ChannelBot, ChannelMessage) {
    let db =
        crate::test_utils::connect_transaction_test_database("channel_thread_operations").await;
    let bot: ChannelBot=bson::from_document(doc! {
        "_id":"bot", "user_id":"owner", "platform":platform, "label":"Helper",
        "platform_bot_id":"123", "platform_bot_username":"helper", "bot_token_encrypted":bson::Binary {
            subtype:bson::spec::BinarySubtype::Generic, bytes:vec![],
        },"webhook_secret_hash":"hash","webhook_registered":true,"status":"active","is_active":true,
        "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).unwrap();
    db.collection::<ChannelBot>("channel_bots")
        .insert_one(&bot)
        .await
        .unwrap();
    db.collection::<bson::Document>("api_keys").insert_one(doc! {
        "_id":"key","user_id":"owner","name":"route","key_prefix":"prefix","key_hash":"hash",
        "scopes":"channel:reply","is_active":true,"callback_url":"https://example.test/callback",
        "created_at":bson::DateTime::now(),
    }).await.unwrap();
    db.collection::<bson::Document>("channel_conversations").insert_one(doc! {
        "_id":"route","user_id":"owner","channel_bot_id":"bot","platform":platform,
        "platform_conversation_id":"*","platform_conversation_type":"group","agent_api_key_id":"key",
        "default_agent":true,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    db.collection::<bson::Document>(CHANNELS).insert_one(doc! {
        "_id":"link","user_id":"owner","channel_bot_id":"bot","platform":platform,
        "bot_label":"Helper","transport":"direct","status":"active","route_api_key_id":"key",
        "route_id":"route","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    let inbound = InboundMessage {
        platform_message_id: "42".into(),
        conversation_id: "-100".into(),
        conversation_type: "group".into(),
        sender_platform_id: "human".into(),
        sender_display_name: None,
        content_type: "text".into(),
        text: Some("never store input".into()),
        attachments: vec![],
        reply_to_platform_message_id: None,
        thread_id: None,
        raw_data: json!({}),
    };
    let mut source = crate::services::channel_relay_service::inbound_metadata(
        "bot", "route", "owner", platform, &inbound, "key", "source",
    );
    source.thread_context = Some(ChannelThreadFacts {
        version: 1,
        kind: ThreadKind::ReplyChain,
        chat_id: "-100".into(),
        message_id: "42".into(),
        root_id: Some("42".into()),
        sender_kind: ThreadSenderKind::Human,
        address: ThreadAddress::Mention,
        ..Default::default()
    });
    db.collection::<ChannelMessage>(MESSAGES)
        .insert_one(&source)
        .await
        .unwrap();
    (db, bot, source)
}

async fn enable(db: &mongodb::Database) {
    db.collection::<bson::Document>("feature_flag_overrides")
        .insert_one(doc! {
            "_id":"flag","org_user_id":bson::Bson::Null,"flag_key":NYXBOT_THREAD_FOLLOW_FLAG_KEY,
            "target_kind":"user","target_key":"owner","enabled":true,"updated_by":"admin",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn channel_thread_service_binds_source_gates_route_and_records_every_split_reply() {
    let (db, bot, source) = fixture("telegram").await;
    let server = MockServer::start().await;
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
        &server.uri(),
    );
    assert!(
        resolution::resolve(&db, &adapter, &bot, &credentials(), "source")
            .await
            .unwrap()
            .is_none()
    );
    enable(&db).await;
    let target = resolution::resolve(&db, &adapter, &bot, &credentials(), "source")
        .await
        .unwrap()
        .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(43));
    let n = calls.clone();
    Mock::given(path("/bottoken/sendMessage"))
        .and(body_partial_json(
            json!({"chat_id":"-100","reply_to_message_id":42}),
        ))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,
            "result":{"message_id":n.fetch_add(1,std::sync::atomic::Ordering::SeqCst)}}))
        })
        .expect(4)
        .mount(&server)
        .await;
    let n = calls.clone();
    Mock::given(path("/bottoken/sendDocument"))
        .and(body_string_contains(
            "name=\"reply_to_message_id\"\r\n\r\n42",
        ))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,
            "result":{"message_id":n.fetch_add(1,std::sync::atomic::Ordering::SeqCst)}}))
        })
        .expect(1)
        .mount(&server)
        .await;
    let reply = OutboundReply {
        text: Some("界".repeat(1500)),
        attachments: vec![crate::services::channel_platform::MaterializedAttachment {
            kind: crate::services::channel_platform::MediaKind::File,
            bytes: bytes::Bytes::from_static(b"file"),
            filename: Some("test.txt".into()),
            mime_type: Some("text/plain".into()),
            caption: Some("caption".into()),
        }],
        metadata: Some(json!({"message_thread_id":999})),
        reply_to_platform_message_id: Some("999".into()),
    };
    let result = delivery::send_reply(&db, &adapter, &bot, &credentials(), &target, &reply)
        .await
        .unwrap();
    assert!(result.error.is_none());
    assert_eq!(result.message_ids, vec!["43", "44", "45", "46", "47"]);
    let records: Vec<ChannelMessage> = db
        .collection(MESSAGES)
        .find(doc! {"direction":"outbound"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(records.len(), 5);
    for record in records {
        assert_eq!(
            record.thread_context.as_ref().unwrap().root_id.as_deref(),
            Some("42")
        );
        assert_eq!(record.platform_conversation_id.as_deref(), Some("-100"));
        assert!(
            !serde_json::to_string(&bson::to_document(&record).unwrap())
                .unwrap()
                .contains("界")
        );
    }
    let requests = server.received_requests().await.unwrap();
    let reassembled: String = requests
        .iter()
        .filter(|r| r.url.path().ends_with("/sendMessage"))
        .map(|r| {
            r.body_json::<serde_json::Value>().unwrap()["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(reassembled, reply.text.unwrap() + "caption");
    assert!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("/sendMessage"))
            .all(|r| {
                r.body_json::<serde_json::Value>()
                    .unwrap()
                    .get("message_thread_id")
                    .is_none()
            })
    );
    let mut spoof = source.clone();
    spoof.platform_conversation_id = Some("other-chat".into());
    assert!(
        adapter
            .send_bound_reply_outcome(
                &db,
                &reqwest::Client::new(),
                &bot,
                &spoof,
                &credentials(),
                "other-chat",
                &OutboundReply {
                    text: Some("no".into()),
                    attachments: vec![],
                    metadata: None,
                    reply_to_platform_message_id: None,
                },
                Some(&target)
            )
            .await
            .is_err()
    );
    db.collection::<bson::Document>("api_keys")
        .update_one(doc! {"_id":"key"}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        resolution::resolve(&db, &adapter, &bot, &credentials(), "source")
            .await
            .unwrap()
            .is_none()
    );
    let r = delivery::send_reply(
        &db,
        &adapter,
        &bot,
        &credentials(),
        &target,
        &OutboundReply {
            text: Some("no".into()),
            attachments: vec![],
            metadata: None,
            reply_to_platform_message_id: None,
        },
    )
    .await
    .unwrap();
    assert!(r.error.is_some() && r.message_ids.is_empty());
    assert_eq!(server.received_requests().await.unwrap().len(), 5);
    assert_eq!(
        db.collection::<bson::Document>("nyxbot_threads")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn channel_thread_history_timeout_retains_filtered_metadata_and_sends_no_redirect_credentials()
 {
    let (db, mut bot, mut source) = fixture("slack").await;
    bot.platform_bot_id = "Bbot".into();
    let now = chrono::Utc::now();
    let root = format!("{}.000001", now.timestamp() - 60);
    let trigger = format!("{}.000001", now.timestamp() - 1);
    source.platform_conversation_id = Some("C1".into());
    source.platform_message_id = Some(trigger.clone());
    source.thread_context = Some(ChannelThreadFacts {
        version: 1,
        kind: ThreadKind::Native,
        chat_id: "C1".into(),
        message_id: trigger,
        root_id: Some(root.clone()),
        native_thread_id: Some(root.clone()),
        sender_kind: ThreadSenderKind::Human,
        ..Default::default()
    });
    db.collection::<ChannelMessage>(MESSAGES)
        .replace_one(doc! {"_id":"source"}, &source)
        .await
        .unwrap();
    for (id, chat, sender) in [
        ("previous", "C1", "human"),
        ("wrong-chat", "C2", "human"),
        ("ineligible", "C1", "other"),
    ] {
        let mut record = source.clone();
        record.id = id.into();
        record.sender_platform_id = Some(sender.into());
        record.platform_message_id = Some(id.into());
        record.platform_conversation_id = Some(chat.into());
        record.created_at = now - chrono::Duration::seconds(30);
        let f = record.thread_context.as_mut().unwrap();
        f.chat_id = chat.into();
        f.message_id = id.into();
        db.collection::<ChannelMessage>(MESSAGES)
            .insert_one(record)
            .await
            .unwrap();
    }
    enable(&db).await;
    let server = MockServer::start().await;
    let destination = MockServer::start().await;
    let adapter =
        crate::services::channel_adapters::slack::SlackAdapter::media_test_adapter(&server.uri());
    let target = resolution::resolve(&db, &adapter, &bot, &credentials(), "source")
        .await
        .unwrap()
        .unwrap();
    Mock::given(path("/conversations.replies"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/leak", destination.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = history::context(&db, &adapter, &bot, &credentials(), &target, &|id| {
        id == "human"
    })
    .await
    .unwrap();
    assert_eq!(c.metadata.len(), 1);
    assert_eq!(c.metadata[0].message_id, "previous");
    assert!(c.history.partial && c.history.messages.is_empty());
    assert!(destination.received_requests().await.unwrap().is_empty());
    server.reset().await;
    Mock::given(path("/conversations.replies"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(9))
                .set_body_json(json!({"ok":true,"messages":[]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let start = std::time::Instant::now();
    let c = history::context(&db, &adapter, &bot, &credentials(), &target, &|id| {
        id == "human"
    })
    .await
    .unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(9));
    assert_eq!(c.metadata.len(), 1);
    assert!(c.history.partial);
    let rows: Vec<bson::Document> = db
        .collection(MESSAGES)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(rows.iter().all(|r| !r.contains_key("text")
        && !r.contains_key("body")
        && !r.contains_key("raw_data")));
}

#[tokio::test]
async fn channel_thread_partial_send_retains_ids_without_retry_and_gateway_never_resolves() {
    let (db, bot, _) = fixture("telegram").await;
    enable(&db).await;
    let server = MockServer::start().await;
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
        &server.uri(),
    );
    let target = resolution::resolve(&db, &adapter, &bot, &credentials(), "source")
        .await
        .unwrap()
        .unwrap();
    Mock::given(path("/bottoken/sendMessage"))
        .and(body_partial_json(json!({"text":"a".repeat(2000)})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":43}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/bottoken/sendMessage"))
        .and(body_partial_json(json!({"text":"tail"})))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"ok":false,"description":"secret upstream error"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let r = delivery::send_reply(
        &db,
        &adapter,
        &bot,
        &credentials(),
        &target,
        &OutboundReply {
            text: Some("a".repeat(2000) + "tail"),
            attachments: vec![],
            metadata: None,
            reply_to_platform_message_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(r.message_ids, vec!["43"]);
    assert!(!r.error.unwrap().to_string().contains("secret upstream"));
    assert_eq!(
        db.collection::<ChannelMessage>(MESSAGES)
            .count_documents(doc! {"direction":"outbound"})
            .await
            .unwrap(),
        1
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    db.collection::<bson::Document>(CHANNELS)
        .update_one(doc! {"_id":"link"}, doc! {"$set":{"transport":"gateway"}})
        .await
        .unwrap();
    assert!(
        resolution::resolve(&db, &adapter, &bot, &credentials(), "source")
            .await
            .unwrap()
            .is_none()
    );
}
