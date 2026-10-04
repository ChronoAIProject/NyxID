use super::*;
use crate::models::channel_thread::{ThreadAddress, ThreadSenderKind};
use crate::services::{channel_adapters, provider_token_exchange_service::TokenExchangeCache};
use serde_json::{Value, json};
use std::sync::Arc;

fn bot(platform: &str) -> ChannelBot {
    bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": "org-owner",
        "platform": platform, "label": "Helper", "platform_bot_id": "123",
        "platform_bot_username": "helper_bot", "bot_token_encrypted": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic, bytes: vec![],
        }, "webhook_secret_hash": "hash", "webhook_registered": true,
        "status": "active", "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    })
    .unwrap()
}

fn adapter(platform: &str) -> Box<dyn PlatformAdapter> {
    channel_adapters::resolve_adapter(platform, &Arc::new(TokenExchangeCache::new())).unwrap()
}

async fn parsed(platform: &str, value: Value) -> InboundMessage {
    adapter(platform)
        .parse_inbound(&serde_json::to_vec(&value).unwrap())
        .await
        .unwrap()
        .remove(0)
}

fn telegram_event() -> Value {
    json!({"message": {"message_id": 42, "chat": {"id": -100, "type": "supergroup"},
        "from": {"id": 456, "is_bot": false}, "text": "😀 @helper_bot",
        "entities": [{"type": "mention", "offset": 3, "length": 11}]}})
}

#[tokio::test]
async fn telegram_topics_chains_human_evidence_and_utf16_mentions() {
    for platform in ["telegram", "telegram-new"] {
        let b = bot(platform);
        let a = adapter(platform);
        let input = parsed(platform, telegram_event()).await;
        let facts = a.thread_facts(&input, &b, None).unwrap();
        assert_eq!(facts.root_id.as_deref(), Some("42"));
        assert_eq!(facts.address, ThreadAddress::Mention);
        assert_eq!(facts.sender_kind, ThreadSenderKind::Human);
        let mut raw = telegram_event();
        raw["message"]["reply_to_message"] = json!({"message_id": 7, "from": {"id": 123}});
        let input = parsed(platform, raw.clone()).await;
        let facts = a.thread_facts(&input, &b, None).unwrap();
        assert_eq!(facts.address, ThreadAddress::ReplyToBot);
        assert_eq!(facts.parent_message_id.as_deref(), Some("7"));
        assert!(
            facts.root_id.is_none(),
            "an immediate parent is not a proven root"
        );
        raw["message"]["message_thread_id"] = json!(9);
        let input = parsed(platform, raw.clone()).await;
        assert_eq!(
            a.thread_facts(&input, &b, None).unwrap().kind,
            ThreadKind::ReplyChain,
            "a non-forum thread id does not establish topic-wide scope"
        );
        raw["message"]["is_topic_message"] = json!(true);
        raw["message"]["sender_chat"] = json!({"id": -100});
        let input = parsed(platform, raw).await;
        let facts = a.thread_facts(&input, &b, None).unwrap();
        assert_eq!(facts.kind, ThreadKind::Topic);
        assert_eq!(facts.root_id.as_deref(), Some("9"));
        assert_eq!(facts.sender_kind, ThreadSenderKind::Unknown);
        let mut raw = telegram_event();
        raw["message"]["text"] = json!("😀 @helper_bot_other");
        raw["message"]["entities"][0]["length"] = json!(17);
        let input = parsed(platform, raw).await;
        assert_eq!(
            a.thread_facts(&input, &b, None).unwrap().address,
            ThreadAddress::NotAddressed
        );
    }
}

#[tokio::test]
async fn slack_roots_remain_stable_and_reply_to_root_is_not_a_bot_mention() {
    let a = adapter("slack");
    let mut b = bot("slack");
    b.platform_bot_id = "U123".into();
    let mut raw = json!({"type":"event_callback", "event": {"type":"app_mention",
        "channel":"C123", "user":"U456", "ts":"1700000000.000001", "text":"hello"}});
    let input = parsed("slack", raw.clone()).await;
    let root = a.thread_facts(&input, &b, None).unwrap();
    assert_eq!(root.address, ThreadAddress::Mention);
    raw["event"]["type"] = json!("message");
    raw["event"]["thread_ts"] = json!("1700000000.000001");
    raw["event"]["ts"] = json!("1700000001.000001");
    let input = parsed("slack", raw.clone()).await;
    let followup = a.thread_facts(&input, &b, None).unwrap();
    assert_eq!(root.root_id, followup.root_id);
    assert_eq!(followup.address, ThreadAddress::NotAddressed);
    assert_eq!(input.thread_id.as_deref(), Some("1700000000.000001"));
    b.platform_bot_id = "B123".into();
    raw["event"]["text"] = json!("hello <@U123>");
    let unresolved = parsed("slack", raw.clone()).await;
    assert_eq!(
        a.thread_facts(&unresolved, &b, None).unwrap().address,
        ThreadAddress::Unknown,
        "auth.test bot_id does not identify the bot user in mentions"
    );
    assert_eq!(
        a.thread_facts(&unresolved, &b, Some("U123"))
            .unwrap()
            .address,
        ThreadAddress::Mention
    );
    b.platform_bot_id = "U123".into();
    raw["event"]["user"] = json!("U123");
    let mut echo = input;
    echo.raw_data = raw;
    assert_eq!(
        a.thread_facts(&echo, &b, None).unwrap().sender_kind,
        ThreadSenderKind::Bot
    );
}

#[tokio::test]
async fn discord_native_channels_need_evidence_and_interactions_never_become_threads() {
    let a = adapter("discord");
    let b = bot("discord");
    let mut raw = json!({"d": {"id":"10", "channel_id":"20", "author":{"id":"456"},
        "content":"hi", "mentions":[{"id":"123"}]}});
    let input = parsed("discord", raw.clone()).await;
    let facts = a.thread_facts(&input, &b, None).unwrap();
    assert_eq!(facts.kind, ThreadKind::Unknown);
    assert!(facts.root_id.is_none());
    assert_eq!(facts.address, ThreadAddress::Mention);
    assert_eq!(facts.sender_kind, ThreadSenderKind::Human);
    let mut missing_author = input;
    missing_author.raw_data["d"]["author"]["bot"] = json!("false");
    assert_eq!(
        a.thread_facts(&missing_author, &b, None)
            .unwrap()
            .sender_kind,
        ThreadSenderKind::Unknown,
        "a malformed bot marker is not human evidence"
    );
    missing_author.raw_data["d"]["author"] = json!({});
    assert_eq!(
        a.thread_facts(&missing_author, &b, None)
            .unwrap()
            .sender_kind,
        ThreadSenderKind::Unknown
    );
    missing_author.raw_data["d"]["author"] = json!({"id":"123"});
    assert_eq!(
        a.thread_facts(&missing_author, &b, None)
            .unwrap()
            .sender_kind,
        ThreadSenderKind::Bot
    );
    raw["d"]["channel_type"] = json!(11);
    let input = parsed("discord", raw.clone()).await;
    let facts = a.thread_facts(&input, &b, None).unwrap();
    assert_eq!(facts.root_id.as_deref(), Some("20"));
    raw["d"]["channel_type"] = json!(0);
    raw["d"]["message_reference"] = json!({"message_id":"5"});
    raw["d"]["webhook_id"] = json!("webhook");
    let input = parsed("discord", raw).await;
    let facts = a.thread_facts(&input, &b, None).unwrap();
    assert_eq!(facts.kind, ThreadKind::ReplyChain);
    assert!(facts.root_id.is_none());
    assert_eq!(facts.sender_kind, ThreadSenderKind::Bot);
    let mut interaction = input;
    interaction.raw_data = json!({"type":2, "token":"secret"});
    interaction.thread_id = Some("interaction:app:secret".into());
    assert!(a.thread_facts(&interaction, &b, None).is_none());
}

#[tokio::test]
async fn lark_family_keeps_root_and_alias_distinct_and_never_guesses_bot_identity() {
    for platform in ["lark", "feishu"] {
        let raw = json!({"header":{"event_type":"im.message.receive_v1"},"event":{
            "sender":{"sender_type":"user", "sender_id":{"open_id":"ou_human"}},
            "message":{"message_id":"om_child", "chat_id":"oc_chat", "chat_type":"group",
                "message_type":"text", "content":"{\"text\":\"hello\"}",
                "root_id":"om_root", "parent_id":"om_parent", "thread_id":"omt_thread",
                "mentions":[{"id":{"open_id":"ou_bot"}}]}}});
        let input = parsed(platform, raw).await;
        let a = adapter(platform);
        let b = bot(platform);
        let facts = a.thread_facts(&input, &b, None).unwrap();
        assert_eq!(facts.root_id.as_deref(), Some("om_root"));
        assert_eq!(facts.native_thread_id.as_deref(), Some("omt_thread"));
        assert_eq!(facts.parent_message_id.as_deref(), Some("om_parent"));
        assert_eq!(facts.address, ThreadAddress::Unknown);
        assert_eq!(
            a.thread_facts(&input, &b, Some("ou_bot")).unwrap().address,
            ThreadAddress::Mention
        );
        assert_eq!(
            a.thread_facts(&input, &b, Some("another_bot"))
                .unwrap()
                .address,
            ThreadAddress::NotAddressed
        );
        let mut root = input.clone();
        for field in ["root_id", "parent_id", "thread_id"] {
            root.raw_data["event"]["message"][field] = json!("");
        }
        root.reply_to_platform_message_id = Some(String::new());
        root.thread_id = Some(String::new());
        let facts = a.thread_facts(&root, &b, Some("ou_bot")).unwrap();
        assert_eq!(facts.root_id.as_deref(), Some("om_child"));
        assert!(facts.parent_message_id.is_none() && facts.native_thread_id.is_none());
        assert!(valid_facts(&facts, &root));
    }
}

#[tokio::test]
async fn unsupported_surfaces_and_follow_admission_stay_off() {
    let mut input = parsed("telegram", telegram_event()).await;
    for a in channel_adapters::registered_adapters(&Arc::new(TokenExchangeCache::new())) {
        let flags = a.thread_capabilities();
        assert!(!flags.thread_follow, "{}", a.platform_id());
        assert_eq!(
            flags.thread_history,
            matches!(a.platform_id(), "slack" | "discord" | "lark" | "feishu")
        );
        if matches!(a.platform_id(), "whatsapp" | "x" | "openclaw" | "aurinko") {
            assert!(!flags.thread_reply);
            assert!(
                a.thread_facts(&input, &bot(a.platform_id()), None)
                    .is_none()
            );
        }
        input.conversation_type = "private".into();
        assert!(
            a.thread_facts(&input, &bot(a.platform_id()), None)
                .is_none()
        );
    }
    let absent: crate::services::channel_platform::ThreadCapabilities =
        serde_json::from_value(json!({})).unwrap();
    assert_eq!(absent, Default::default());
    assert!(
        !feature_flag_service::find_flag(NYXBOT_THREAD_FOLLOW_FLAG_KEY)
            .unwrap()
            .default_enabled
    );
}

#[tokio::test]
async fn facts_are_metadata_only_backward_compatible_and_bound_to_source() {
    let input = parsed("telegram", telegram_event()).await;
    let facts = adapter("telegram")
        .thread_facts(&input, &bot("telegram"), None)
        .unwrap();
    assert!(valid_facts(&facts, &input));
    for field in ["chat_id", "message_id", "root_id", "native_thread_id"] {
        let mut json = serde_json::to_value(&facts).unwrap();
        json[field] = json!("interaction:app:secret");
        let changed = serde_json::from_value(json).unwrap();
        assert!(!valid_facts(&changed, &input));
    }
    let mut row = crate::services::channel_relay_service::inbound_metadata(
        "bot", "route", "owner", "telegram", &input, "key", "message",
    );
    row.thread_context = Some(facts.clone());
    let mut stored = bson::to_document(&row).unwrap();
    let encoded = serde_json::to_string(&stored).unwrap();
    assert!(!encoded.contains("helper_bot"));
    for field in ["text", "raw_platform_data", "body", "content"] {
        assert!(!stored.contains_key(field));
    }
    assert!(!format!("{facts:?}").contains("42"));
    assert_eq!(
        bson::from_document::<crate::models::channel_message::ChannelMessage>(stored.clone())
            .unwrap()
            .thread_context,
        Some(facts)
    );
    stored.remove("thread_context");
    assert!(
        bson::from_document::<crate::models::channel_message::ChannelMessage>(stored)
            .unwrap()
            .thread_context
            .is_none()
    );
    let future: ChannelThreadFacts = serde_json::from_value(
        json!({"kind":"future", "address":"future", "sender_kind":"future"}),
    )
    .unwrap();
    assert_eq!(future.address, ThreadAddress::Unknown);
    assert_eq!(future.sender_kind, ThreadSenderKind::Unknown);
    assert!(!valid_facts(&future, &input));
}

#[tokio::test]
async fn metadata_requires_flagged_direct_link_and_never_changes_gateway_or_routes() {
    let db = crate::test_utils::connect_transaction_test_database("thread_facts_gate").await;
    let input = parsed("telegram", telegram_event()).await;
    let b = bot("telegram");
    let a = adapter("telegram");
    assert!(
        inbound_facts(&db, &b, "route-key", a.as_ref(), &input)
            .await
            .unwrap()
            .is_none()
    );
    db.collection::<bson::Document>(CHANNELS)
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "user_id": "person-owner",
            "bot_owner_id": "org-owner", "channel_bot_id": &b.id,
            "platform":"telegram", "bot_label":"Helper", "transport":"direct",
            "status":"active", "route_api_key_id":"route-key",
            "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    assert!(
        inbound_facts(&db, &b, "route-key", a.as_ref(), &input)
            .await
            .unwrap()
            .is_none()
    );
    // Enable for the linked person only, not the organization that owns the bot.
    db.collection::<bson::Document>("feature_flag_overrides")
        .insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(), "org_user_id":bson::Bson::Null,
            "flag_key":NYXBOT_THREAD_FOLLOW_FLAG_KEY, "target_kind":"user",
            "target_key":"person-owner", "enabled":true, "updated_by":"admin",
            "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    assert!(
        inbound_facts(&db, &b, "route-key", a.as_ref(), &input)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        inbound_facts(&db, &b, "other-key", a.as_ref(), &input)
            .await
            .unwrap()
            .is_none()
    );
    db.collection::<bson::Document>(CHANNELS)
        .update_one(doc! {}, doc! {"$set":{"transport":"gateway"}})
        .await
        .unwrap();
    assert!(
        inbound_facts(&db, &b, "route-key", a.as_ref(), &input)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.collection::<bson::Document>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
