use super::*;
use bson::Document;
use chrono::Utc;

pub(crate) fn channel_and_chat() -> (NyxbotChannel, NyxbotThread) {
    let now = bson::DateTime::now();
    let channel=bson::from_document(doc! {"_id":"channel","user_id":"admin","bot_owner_id":"org",
            "channel_bot_id":"bot","platform":"lark","bot_label":"Community assistant","transport":"gateway","status":"active",
            "route_api_key_id":"route-key","route_id":"route","created_at":now,"updated_at":now}).unwrap();
    let chat=bson::from_document(doc! {"_id":"internal-chat","channel_id":"channel","user_id":"admin",
            "partition":"group:oc_internal","platform_chat_id":"oc_internal","kind":"group","allow_posts":true,
            "created_at":now,"updated_at":now}).unwrap();
    (channel, chat)
}

pub(crate) fn inbound(channel: &NyxbotChannel, chat: &NyxbotThread) -> ChannelMessage {
    super::super::channel_relay_service::inbound_metadata(
        "bot",
        "route",
        "org",
        "lark",
        &super::super::channel_platform::InboundMessage {
            embedded_attachments: vec![],
            embedded_attachments_omitted: 0,
            platform_message_id: "om_host_request".into(),
            conversation_id: chat.platform_chat_id.clone().unwrap(),
            conversation_type: "group".into(),
            sender_platform_id: "ou_host".into(),
            sender_display_name: Some("Host".into()),
            content_type: "text".into(),
            text: Some("Give me the question list".into()),
            attachments: vec![],
            reply_to_platform_message_id: Some("om_previous".into()),
            thread_id: None,
            raw_data: json!({}),
        },
        &channel.route_api_key_id,
        "receipt",
    )
}

#[tokio::test]
async fn source_provenance_comes_from_the_exact_admitted_receipt_and_survives_roundtrip() {
    let db =
        crate::test_utils::connect_transaction_test_database("channel_source_provenance").await;
    let (channel, chat) = channel_and_chat();
    let inbound = inbound(&channel, &chat);
    db.collection::<ChannelMessage>(MESSAGES)
        .insert_one(&inbound)
        .await
        .unwrap();
    db.collection::<bson::Document>(EVENTS).insert_one(doc! {"_id":"event","channel_id":"channel","user_id":"admin",
            "partition":&chat.partition,"event_id":"arbitrary_gateway_event_id","relay_message_id":&inbound.id,"status":"running",
            "created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))}).await.unwrap();
    let source = source_message(&db, &channel, &chat, "ou_host", "event")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.receipt_id, inbound.id);
    assert_eq!(source.platform_message_id, "om_host_request");
    assert_eq!(source.platform_chat_id, "oc_internal");
    assert_eq!(source.nyxid_chat_id, "internal-chat");
    assert_eq!(
        source.reply_to_platform_message_id.as_deref(),
        Some("om_previous")
    );
    let encoded = bson::to_document(&source).unwrap();
    assert!(encoded.get_datetime("received_at").is_ok());
    let restored: ChannelSourceMessage = bson::from_document(encoded).unwrap();
    assert_eq!(restored.platform_message_id, source.platform_message_id);
    for (field, value) in [
        ("user_id", "someone-else"),
        ("channel_bot_id", "other-bot"),
        ("agent_api_key_id", "other-route-key"),
        ("conversation_id", "other-route"),
        ("platform_conversation_id", "oc_guest"),
        ("sender_platform_id", "ou_other"),
    ] {
        db.collection::<bson::Document>(MESSAGES)
            .update_one(doc! {"_id":&inbound.id}, doc! {"$set":{field:value}})
            .await
            .unwrap();
        assert!(
            source_message(&db, &channel, &chat, "ou_host", "event")
                .await
                .unwrap()
                .is_none(),
            "unrelated {field} must not become source provenance"
        );
        db.collection::<ChannelMessage>(MESSAGES)
            .replace_one(doc! {"_id":&inbound.id}, &inbound)
            .await
            .unwrap();
    }
    assert!(
        source_message(&db, &channel, &chat, "ou_host", "missing-event")
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn delivery_reads_only_exact_source_receipts_and_never_sends_or_writes() {
    let db = crate::test_utils::connect_transaction_test_database("channel_delivery_receipt_scope")
        .await;
    let (channel, chat) = channel_and_chat();
    let inbound = inbound(&channel, &chat);
    db.collection::<ChannelMessage>(MESSAGES)
        .insert_one(&inbound)
        .await
        .unwrap();
    let args = json!({"chat_id":chat.id,"source_message_id":inbound.id});
    let read = super::super::channel_delivery_receipts::read;
    let before = read(&db, &channel, &chat, "admin", Some("agent"), &args)
        .await
        .unwrap();
    assert_eq!(before["state"], "unconfirmed");
    let mut wrong = chat.clone();
    wrong.platform_chat_id = Some("oc_other".into());
    assert!(
        read(&db, &channel, &wrong, "admin", Some("agent"), &args)
            .await
            .is_err()
    );
    let outbound = super::super::channel_relay_service::store_outbound_message(
        &db,
        "other-bot",
        "route",
        "org",
        "lark",
        "route-key",
        Some(&inbound.id),
        Some("om_unrelated"),
        Some("oc_internal"),
        "text",
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        read(&db, &channel, &chat, "admin", Some("agent"), &args)
            .await
            .unwrap()["state"],
        "unconfirmed"
    );
    db.collection::<bson::Document>(MESSAGES)
        .update_one(
            doc! {"_id":&outbound.id},
            doc! {"$set":{"channel_bot_id":"bot"}},
        )
        .await
        .unwrap();
    for _ in 0..2 {
        let result = read(&db, &channel, &chat, "admin", Some("agent"), &args)
            .await
            .unwrap();
        assert_eq!(result["state"], "confirmed");
        assert_eq!(result["receipts"][0]["platform_message_id"], "om_unrelated");
        assert!(result.get("text").is_none());
    }
    db.collection::<Document>(EVENTS)
        .insert_one(
            doc! {"_id":"event","channel_id":&channel.id,"user_id":&channel.user_id,"partition":&chat.partition,
            "relay_message_id":&inbound.id,"status":"failed","created_at":bson::DateTime::now()},
        )
        .await
        .unwrap();
    let result = read(&db, &channel, &chat, "admin", Some("agent"), &args)
        .await
        .unwrap();
    assert_eq!(result["state"], "confirmed");
    assert_eq!(
        result["answer_succeeded"], false,
        "a delivered failure notice is not a successful answer"
    );
    db.collection::<Document>(EVENTS)
        .update_one(doc! {"_id":"event"}, doc! {"$set":{"status":"completed"}})
        .await
        .unwrap();
    assert_eq!(
        read(&db, &channel, &chat, "admin", Some("agent"), &args)
            .await
            .unwrap()["answer_succeeded"],
        true
    );
    assert_eq!(
        db.collection::<ChannelMessage>(MESSAGES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    let now = bson::DateTime::now();
    db.collection::<bson::Document>("trigger_runs").insert_one(doc! {"_id":"run","trigger_id":"reminder","user_id":"admin",
            "agent_id":"agent","scheduled_at":now,"deadline":now,"outcome":"started","label":"Reminder","fence":"fence",
            "lease_until":now,"expires_at":now,"result_claimed":true,"deliver_to":{"type":"chat","chat_id":&chat.id}}).await.unwrap();
    let schedule_args = json!({"chat_id":chat.id,"schedule_id":"reminder"});
    assert_eq!(
        read(&db, &channel, &chat, "admin", Some("agent"), &schedule_args)
            .await
            .unwrap()["state"],
        "unknown"
    );
    let key = super::super::mcp_service::canonical_sha256(json!([
        "nyxbot-post-v1",
        "admin",
        chat.id,
        "automation:run"
    ]));
    let mut delivery = super::super::channel_text_delivery::begin(
        &db,
        &key,
        "admin",
        "automation:run",
        "fingerprint",
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        read(&db, &channel, &chat, "admin", Some("agent"), &schedule_args)
            .await
            .unwrap()["state"],
        "pending"
    );
    delivery.parts[0].state = crate::models::channel_text_delivery::TextPartState::Sent;
    delivery.parts[0].platform_message_id = Some("om_reminder".into());
    delivery.parts[0].outbound_message_id = Some("outbound-reminder".into());
    db.collection::<crate::models::channel_text_delivery::ChannelTextDelivery>(
        "channel_text_deliveries",
    )
    .replace_one(doc! {"_id":&delivery.id}, &delivery)
    .await
    .unwrap();
    assert_eq!(
        read(&db, &channel, &chat, "admin", Some("agent"), &schedule_args)
            .await
            .unwrap()["state"],
        "confirmed"
    );
    assert!(
        read(
            &db,
            &channel,
            &chat,
            "another-admin",
            Some("agent"),
            &schedule_args
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &db,
            &channel,
            &chat,
            "admin",
            Some("another-agent"),
            &schedule_args
        )
        .await
        .is_err()
    );
    assert!(
        read(
            &db,
            &channel,
            &chat,
            "admin",
            Some("agent"),
            &json!({"source_message_id":inbound.id,"schedule_id":"reminder"})
        )
        .await
        .is_err()
    );
    db.drop().await.unwrap();
}
