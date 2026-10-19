use super::*;
use crate::{
    models::{
        channel_message::ChannelMessage,
        channel_thread::{ChannelThreadFacts, ThreadAddress, ThreadKind, ThreadSenderKind},
    },
    services::channel_platform::InboundMessage,
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

struct AdapterGuard(String);
impl Drop for AdapterGuard {
    fn drop(&mut self) {
        thread_follow::TEST_ADAPTERS.lock().unwrap().remove(&self.0);
    }
}

#[tokio::test]
async fn private_telegram_with_follow_off_is_a_zero_read_legacy_fast_path() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::Mutex;

    let commands = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            observed.lock().unwrap().push(event.command_name.clone());
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "nyxbot_thread_follow_private_fast_path",
        handler,
    )
    .await;
    let state = crate::test_utils::test_app_state(db);
    let mut row: NyxbotChannel = bson::from_document(doc! {
        "_id":"private-link", "user_id":OWNER, "channel_bot_id":"private-bot",
        "platform":"telegram", "bot_label":"Private bot", "transport":"direct",
        "status":"active", "route_api_key_id":"route", "created_at":bson::DateTime::now(),
        "updated_at":bson::DateTime::now(),
    })
    .unwrap();
    assert!(!thread_follow::enabled(&state, OWNER).await.unwrap());
    commands.lock().unwrap().clear();

    for platform in [
        "telegram",
        "telegram-new",
        "slack",
        "discord",
        "lark",
        "feishu",
        "x",
        "whatsapp",
    ] {
        row.platform = platform.into();
        assert!(
            !Box::pin(thread_follow::inbound(
                &state,
                &row,
                &json!({"conversation": {"type": "private"}}),
                "message-that-does-not-exist",
                "legacy DM",
                "unused-event-key",
            ))
            .await
            .unwrap()
        );
    }
    // The capability gate must honor the bot-specific test override too.
    row.platform = "aurinko".into();
    thread_follow::TEST_ADAPTERS.lock().unwrap().insert(
        row.channel_bot_id.clone(),
        Arc::new(crate::services::channel_adapters::telegram::TelegramAdapter::default()),
    );
    let _adapter = AdapterGuard(row.channel_bot_id.clone());
    assert!(
        !Box::pin(thread_follow::inbound(
            &state,
            &row,
            &json!({"conversation": {"type": "private"}}),
            "message-that-does-not-exist",
            "legacy DM",
            "unused-event-key",
        ))
        .await
        .unwrap()
    );
    assert!(
        commands.lock().unwrap().is_empty(),
        "private Telegram follow fast path issued MongoDB commands"
    );
}

async fn callback(
    state: &AppState,
    row: &NyxbotChannel,
    n: u32,
    topic: &str,
    sender: &str,
    addressed: bool,
    text: &str,
) {
    callback_as(
        state,
        row,
        n,
        topic,
        sender,
        addressed,
        text,
        ThreadSenderKind::Human,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn callback_as(
    state: &AppState,
    row: &NyxbotChannel,
    n: u32,
    topic: &str,
    sender: &str,
    addressed: bool,
    text: &str,
    kind: ThreadSenderKind,
) {
    let raw = if kind == ThreadSenderKind::Bot {
        json!({"message":{"message_id":n,"message_thread_id":topic.parse::<i64>().unwrap(),"is_topic_message":true,
            "from":{"id":sender.parse::<i64>().unwrap(),"is_bot":true},
            "chat":{"id":-100,"type":"supergroup"},"text":text,
            "entities":[{"type":"mention","offset":0,"length":11}]}})
    } else {
        json!({"message":{"from":{"is_bot":false}}})
    };
    let inbound = InboundMessage {
        platform_message_id: n.to_string(),
        conversation_id: "-100".into(),
        conversation_type: "group".into(),
        sender_platform_id: sender.into(),
        sender_display_name: Some("Sender".into()),
        content_type: "text".into(),
        text: Some(text.into()),
        attachments: Vec::new(),
        reply_to_platform_message_id: None,
        thread_id: Some(topic.into()),
        raw_data: raw.clone(),
    };
    let facts = ChannelThreadFacts {
        version: 1,
        kind: ThreadKind::Topic,
        chat_id: "-100".into(),
        message_id: n.to_string(),
        root_id: Some(topic.into()),
        native_thread_id: Some(topic.into()),
        sender_kind: kind,
        address: if addressed {
            ThreadAddress::Mention
        } else {
            ThreadAddress::NotAddressed
        },
        ..Default::default()
    };
    let id = format!("source-{n}");
    let mut source = crate::services::channel_relay_service::inbound_metadata(
        &row.channel_bot_id,
        row.route_id.as_deref().unwrap(),
        OWNER,
        "telegram",
        &inbound,
        &row.route_api_key_id,
        &id,
    );
    source.thread_context = Some(facts.clone());
    state
        .db
        .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(&source)
        .await
        .unwrap();
    let body = json!({"message_id":id,"correlation_id":format!("jti-{n}"),"platform":"telegram","thread_context":facts,"thread_id":topic,
        "conversation":{"id":row.route_id,"platform_id":"-100","type":"group"},"sender":{"platform_id":sender,"display_name":"Sender"},
        "content":{"type":"text","text":text},"raw_platform_data":raw});
    let bytes = serde_json::to_vec(&body).unwrap();
    let token = crate::crypto::jwt::generate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        &format!("jti-{n}"),
        &row.route_api_key_id,
        &id,
        "telegram",
        &sha256_hex(&bytes),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-nyxid-callback-token", token.parse().unwrap());
    let (_guard, done) = watch_relay_callback(&row.id, &id);
    assert_eq!(
        Box::pin(relay_callback(
            State(state.clone()),
            Path(row.id.clone()),
            headers,
            Bytes::from(bytes)
        ))
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(Duration::from_secs(30), done)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn direct_follow_uses_separate_native_children_and_owner_stop_without_model() {
    let (state, calls, model) = setup("nyxbot_thread_follow_callback").await;
    let (mut row, _) = channel(&state, "direct").await;
    row.platform = "telegram".into();
    row.owner_sender_ids = vec!["human".into()];
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let mut bot = bot_doc("telegram", "Helper");
    bot.insert("_id", &row.channel_bot_id);
    bot.insert(
        "bot_token_encrypted",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: state.encryption_keys.encrypt(b"token").await.unwrap(),
        },
    );
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .replace_one(
            doc! {"_id": bot.get_str("_id").unwrap().to_owned()},
            bot.clone(),
        )
        .upsert(true)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("api_keys")
        .update_one(
            doc! {"_id":&row.route_api_key_id},
            doc! {"$set":{"callback_url":"https://example.test/callback"}},
        )
        .await
        .unwrap();
    assert_eq!(state.db.collection::<bson::Document>("channel_conversations").replace_one(
        doc! {"_id": &row.route_id},
        doc! {"_id":&row.route_id,"user_id":OWNER,"channel_bot_id":&row.channel_bot_id,
        "platform":"telegram","platform_conversation_id":"*","platform_conversation_type":"group","agent_api_key_id":&row.route_api_key_id,
        "default_agent":true,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}
    ).await.unwrap().matched_count, 1);
    state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {"_id":"follow-flag","org_user_id":bson::Bson::Null,
        "flag_key":crate::services::feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,"target_kind":"user","target_key":OWNER,
        "enabled":true,"updated_by":"admin","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let provider = MockServer::start().await;
    thread_follow::TEST_ADAPTERS.lock().unwrap().insert(
        row.channel_bot_id.clone(),
        Arc::new(
            crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
                &provider.uri(),
            ),
        ),
    );
    let _adapter = AdapterGuard(row.channel_bot_id.clone());
    let counter = std::sync::atomic::AtomicU32::new(1000);
    Mock::given(path("/bottoken/sendMessage")).and(body_partial_json(json!({"chat_id":"-100"})))
        .respond_with(move |_:&wiremock::Request|ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":counter.fetch_add(1,std::sync::atomic::Ordering::SeqCst)}})))
        .mount(&provider).await;
    callback(
        &state,
        &row,
        100,
        "7",
        "human",
        false,
        "History body must never be retained",
    )
    .await;
    assert!(calls.lock().await.is_empty());
    callback(&state, &row, 101, "7", "human", true, "First request").await;
    callback(
        &state,
        &row,
        102,
        "7",
        "guest",
        false,
        "Follow-up from a member",
    )
    .await;
    let thread_rows = state.db.collection::<NyxbotThread>(THREADS);
    let children =
        || thread_rows.find(doc! {"channel_id":&row.id,"record_scope":"platform_thread"});
    let first = children()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].follow.follow_state.as_deref(), Some("active"));
    assert_eq!(calls.lock().await.len(), 2);
    let conversation = first[0].conversation_id.clone();
    assert!(calls.lock().await[0].to_string().contains("body_available"));
    let messages = engine::messages(
        &state.db,
        OWNER,
        conversation.as_deref().unwrap(),
        100,
        None,
    )
    .await
    .unwrap();
    assert!(messages.iter().all(|m| !m.text.contains("body_available")
        && !m.text.contains("History body must never be retained")));
    callback(&state, &row, 103, "7", "guest", true, "stop following").await;
    assert_eq!(calls.lock().await.len(), 2);
    callback(&state, &row, 104, "7", "human", false, "stop following").await;
    assert_eq!(calls.lock().await.len(), 2);
    callback(&state, &row, 105, "7", "human", false, "Ignored after stop").await;
    assert_eq!(calls.lock().await.len(), 2);
    callback(&state, &row, 106, "7", "human", true, "Reactivate").await;
    assert_eq!(calls.lock().await.len(), 3);
    let same = children()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(same[0].conversation_id, conversation);
    callback(&state, &row, 107, "8", "human", true, "Different topic").await;
    assert_eq!(calls.lock().await.len(), 4);
    assert_eq!(
        children()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .len(),
        2
    );
    let before_bot = children()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .find(|child| child.follow.thread_root_id.as_deref() == Some("8"))
        .unwrap();
    callback_as(
        &state,
        &row,
        108,
        "8",
        "9009",
        true,
        "@helper_bot Bot follow-up",
        ThreadSenderKind::Bot,
    )
    .await;
    assert_eq!(calls.lock().await.len(), 5);
    let after_bot = thread_rows
        .find_one(doc! {"_id": &before_bot.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_bot.follow.follow_expires_at,
        before_bot.follow.follow_expires_at
    );
    assert!(
        engine::get(
            &state.db,
            OWNER,
            after_bot.conversation_id.as_deref().unwrap()
        )
        .await
        .unwrap()
        .guest_turn
    );
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        6,
        "both human and bot native answers are delivered"
    );
    for request in &requests[..4] {
        assert_eq!(
            request.body_json::<Value>().unwrap()["message_thread_id"],
            7
        );
    }
    assert_eq!(
        requests[4].body_json::<Value>().unwrap()["message_thread_id"],
        8
    );
    assert_eq!(
        requests[5].body_json::<Value>().unwrap()["message_thread_id"],
        8
    );
    let parents = chats::list_chats(&state, OWNER, Some(&row.id))
        .await
        .unwrap();
    let dto = serde_json::to_value(parents).unwrap();
    assert_eq!(dto.as_array().unwrap().len(), 2);
    assert!(
        dto.as_array()
            .unwrap()
            .iter()
            .all(|p| p["followed_thread_count"] == 1)
    );
    // Queue execution uses the queued owner's source, even if the previous
    // successfully admitted guest has since lost access to the chat.
    let id = conversation.as_deref().unwrap();
    let current = engine::get(&state.db, OWNER, id).await.unwrap();
    let owner_origin = current.channel.clone().unwrap();
    let mut guest_origin = owner_origin.clone();
    let guest_binding = guest_origin.thread.as_mut().unwrap();
    guest_binding.source_message_id = "source-102".into();
    guest_binding.sender_id = "guest".into();
    guest_binding.guest = true;
    let mut event = crate::services::assistant_team_service::event(
        "message",
        "Queued owner request".into(),
        None,
    );
    event.reply_to = vec![owner_origin.clone()];
    let source_id = Uuid::new_v4().to_string();
    event.source_event_ids = vec![source_id.clone()];
    let mut source_event = admission(&row.id, "queued-native-source");
    source_event.status = "running".into();
    source_event.partition = owner_origin.partition.clone();
    source_event.id = source_id.clone();
    source_event.relay_message_id = Some(
        owner_origin
            .thread
            .as_ref()
            .unwrap()
            .source_message_id
            .clone(),
    );
    source_event.delivery = Some(crate::models::nyxbot_channel::ChannelTurnDelivery {
        version: 1,
        state: "pending".into(),
        origin: owner_origin,
        sender_id: "human".into(),
        sender_kind: Some(ThreadSenderKind::Human),
        guest: false,
        addressed: true,
        transport: "direct".into(),
        source_route_id: None,
        source_route_api_key_id: None,
        stream_deadline: Utc::now(),
        checked_at: Utc::now(),
        claim_id: None,
        target_ciphertext: None,
        target_expires_at: None,
    });
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&source_event)
        .await
        .unwrap();
    assert!(
        crate::services::channel_turn_delivery::enqueue(&state.db, OWNER, id, event)
            .await
            .unwrap()
    );
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"channel":bson::to_bson(&guest_origin).unwrap()}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id":&first[0].follow.settings_chat_id},
            doc! {"$set":{"members":"owner"}},
        )
        .await
        .unwrap();
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: Some(id.into()),
        agent_id: None,
        text: String::new(),
        model: None,
        access_mode: None,
    });
    start.origin = TurnOrigin::Event;
    let claimed = Box::pin(engine::begin_turn(
        &state.db,
        OWNER,
        &start,
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    assert!(!claimed.guest_turn);
    let turn = claimed.active_turn.as_ref().unwrap();
    assert_eq!(turn.channel_event_id.as_deref(), Some(source_id.as_str()));
    let saved = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&source_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.turn_id.as_deref(), Some(turn.turn_id.as_str()));
    engine::finish_turn(
        &state.db,
        &claimed,
        &claimed.credential_api_key_id,
        "native-queued-response",
        &engine::TurnResult {
            text: "Native queued answer".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    late_delivery::process(&state, &source_id).await;
    late_delivery::process(&state, &source_id).await;
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(requests.len(), 7, "one source-bound native answer attempt");
    let last = requests.last().unwrap().body_json::<Value>().unwrap();
    assert_eq!(last["text"], "Native queued answer");
    assert_eq!(last["message_thread_id"], 7);
    let saved = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&source_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.delivery.unwrap().state, "sent");
    assert_eq!(claimed.active_turn.as_ref().unwrap().events.len(), 1);
    assert!(
        !claimed
            .channel
            .as_ref()
            .unwrap()
            .thread
            .as_ref()
            .unwrap()
            .guest
    );

    assert_eq!(
        thread_controls::list_tool(&state, OWNER, &json!({}))
            .await
            .unwrap()["threads"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // The link can still be active before the org-loss sweep catches it.
    // Discovery and stop must recheck live org access in that interval.
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"bot_owner_id":"revoked-org"}},
        )
        .await
        .unwrap();
    assert!(
        thread_controls::list_tool(&state, OWNER, &json!({}))
            .await
            .unwrap()["threads"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        thread_controls::list_tool(&state, OWNER, &json!({"channel_agent_id":row.id}))
            .await
            .is_err()
    );
    assert!(
        thread_controls::stop(
            &state,
            OWNER,
            &row.id,
            first[0].follow.parent_chat_id.as_deref().unwrap(),
            &first[0].id
        )
        .await
        .is_err()
    );
    model.abort();
}
