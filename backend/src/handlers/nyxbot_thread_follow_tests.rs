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
        raw_data: json!({}),
    };
    let facts = ChannelThreadFacts {
        version: 1,
        kind: ThreadKind::Topic,
        chat_id: "-100".into(),
        message_id: n.to_string(),
        root_id: Some(topic.into()),
        native_thread_id: Some(topic.into()),
        sender_kind: ThreadSenderKind::Human,
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
        "content":{"type":"text","text":text},"raw_platform_data":{}});
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

async fn native_fixture(
    name: &str,
) -> (
    AppState,
    Calls,
    tokio::task::JoinHandle<()>,
    NyxbotChannel,
    MockServer,
    AdapterGuard,
) {
    let (state, calls, model) = setup(name).await;
    let (mut row, _) = channel(&state, "direct").await;
    row.platform = "telegram".into();
    row.owner_sender_ids = vec!["human".into()];
    row.route_id = Some(Uuid::new_v4().to_string());
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
        .insert_one(bot)
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
    state.db.collection::<bson::Document>("channel_conversations").insert_one(doc! {"_id":&row.route_id,"user_id":OWNER,"channel_bot_id":&row.channel_bot_id,
        "platform":"telegram","platform_conversation_id":"*","platform_conversation_type":"group","agent_api_key_id":&row.route_api_key_id,
        "default_agent":true,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
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
    let guard = AdapterGuard(row.channel_bot_id.clone());
    (state, calls, model, row, provider, guard)
}

#[tokio::test]
async fn direct_follow_uses_separate_native_children_and_owner_stop_without_model() {
    let (state, calls, model, row, provider, _adapter) =
        native_fixture("nyxbot_thread_follow_callback").await;
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
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(requests.len(), 5);
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
    // A selected tool image follows the same native topic as the ordinary text.
    Mock::given(path("/bottoken/sendPhoto"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"result":{"message_id":2000}})),
        )
        .mount(&provider)
        .await;
    let mut reply = engine::messages(&state.db, OWNER, id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|m| m.role == "assistant")
        .unwrap();
    state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":id},doc! {"$set":{"active_turn":{
        "turn_id":&reply.turn_id,"started_at":bson::DateTime::now(),"origin":"channel","stop_requested":false,
    }}}).await.unwrap();
    let image = engine::attach_image(
        &state.db,
        &state.encryption_keys,
        OWNER,
        id,
        "camera__snapshot",
        "image/png",
        b"\x89PNG\r\n\x1a\nNATIVE_IMAGE",
    )
    .await
    .unwrap()
    .unwrap();
    reply.reply_image_refs = vec![image.id.clone()];
    reply.attachments.push(image);
    state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME).update_one(doc! {"_id":&reply.id},doc! {"$set":{"reply_image_refs":&reply.reply_image_refs,"attachments":bson::to_bson(&reply.attachments).unwrap()}}).await.unwrap();
    state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id":id}, doc! {"$unset":{"active_turn":""}})
        .await
        .unwrap();
    let report = thread_follow::send_selected(
        &state,
        OWNER,
        &owner_origin,
        id,
        "Selected native image",
        Some(&reply),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(report.status(), "confirmed");
    let sent = provider.received_requests().await.unwrap();
    assert_eq!(sent.len(), requests.len() + 2);
    assert_eq!(
        sent[sent.len() - 2].body_json::<Value>().unwrap()["message_thread_id"],
        7
    );
    let photo = String::from_utf8_lossy(&sent.last().unwrap().body);
    assert!(
        photo.contains("NATIVE_IMAGE") && photo.contains("name=\"message_thread_id\"\r\n\r\n7")
    );
    Mock::given(path("/bottoken/sendPhoto"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":false})))
        .with_priority(1)
        .expect(1)
        .mount(&provider)
        .await;
    let report = thread_follow::send_selected(
        &state,
        OWNER,
        &owner_origin,
        id,
        "Unconfirmed native image",
        Some(&reply),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(report.status(), "not_confirmed");
    assert_eq!(
        provider.received_requests().await.unwrap().len(),
        sent.len() + 2
    );
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
    event.reply_to = vec![owner_origin];
    assert!(
        crate::services::channel_thread_follow_service::enqueue(&state.db, OWNER, id, event)
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

async fn binding_lookup_failure_preserves_send(with_image: bool) {
    use crate::models::{assistant_message::AssistantMessage, nyxbot_channel::ChannelTurnDelivery};
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (mut state, _, model, row, provider, _adapter) =
        native_fixture("native_binding_failure").await;
    Mock::given(path("/bottoken/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"result":{"message_id":1000}})),
        )
        .mount(&provider)
        .await;
    callback(&state, &row, 101, "7", "human", true, "Request").await;
    let child = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&row.id,"record_scope":"platform_thread"})
        .await
        .unwrap()
        .unwrap();
    let conversation = engine::get(&state.db, OWNER, child.conversation_id.as_deref().unwrap())
        .await
        .unwrap();
    let origin = conversation.channel.as_ref().unwrap();
    let mut message = engine::messages(&state.db, OWNER, &conversation.id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|m| m.role == "assistant")
        .unwrap();
    if with_image {
        let id = Uuid::new_v4().to_string();
        let bytes = b"\x89PNG\r\n\x1a\nSELECTED_IMAGE";
        state
            .db
            .collection::<crate::models::assistant_attachment::AssistantAttachment>(
                crate::models::assistant_attachment::COLLECTION_NAME,
            )
            .insert_one(crate::models::assistant_attachment::AssistantAttachment {
                id: id.clone(),
                origin: "tool".into(),
                user_id: OWNER.into(),
                conversation_id: conversation.id.clone(),
                turn_id: message.turn_id.clone(),
                content_type: "image/png".into(),
                size: bytes.len() as i64,
                data_encrypted: state.encryption_keys.encrypt(bytes).await.unwrap(),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        message.reply_image_refs = vec![id];
        state
            .db
            .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
            .replace_one(doc! {"_id":&message.id}, &message)
            .await
            .unwrap();
    }
    let mut event = admission(&row.id, "binding-failure");
    event.event_id = origin.thread.as_ref().unwrap().source_message_id.clone();
    event.conversation_id = Some(conversation.id.clone());
    event.turn_id = Some(message.turn_id.clone());
    event.delivery = Some(ChannelTurnDelivery {
        version: 1,
        state: "pending".into(),
        origin: origin.clone(),
        sender_id: "human".into(),
        guest: false,
        addressed: true,
        transport: "direct".into(),
        stream_deadline: Utc::now(),
        checked_at: Utc::now(),
        claim_id: None,
        target_ciphertext: None,
        target_expires_at: None,
    });
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&event)
        .await
        .unwrap();

    let expected_sends = if with_image { 2 } else { 1 };
    let sends = Arc::new(AtomicUsize::new(0));
    let sent = sends.clone();
    let original_db = state.db.clone();
    let db = original_db.clone();
    let source_id = event.event_id.clone();
    // Corrupt only after the last provider effect, before it returns success.
    // All authority checks and the existing send pipeline still run normally.
    let transport = Router::new().route(
        "/bottoken/{operation}",
        post(move || {
            let db = db.clone();
            let source_id = source_id.clone();
            let sent = sent.clone();
            async move {
                let n = sent.fetch_add(1, Ordering::SeqCst) + 1;
                if n == expected_sends {
                    db.collection::<bson::Document>(
                        crate::models::channel_message::COLLECTION_NAME,
                    )
                    .update_one(doc! {"_id":source_id}, doc! {"$set":{"content_type":42}})
                    .await
                    .unwrap();
                }
                Json(json!({"ok":true,"result":{"message_id":2000+n}}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, transport).await.unwrap();
    });
    thread_follow::TEST_ADAPTERS.lock().unwrap().insert(
        row.channel_bot_id.clone(),
        Arc::new(
            crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
                &format!("http://{address}"),
            ),
        ),
    );
    let binding_reads = Arc::new(AtomicUsize::new(0));
    let reads = binding_reads.clone();
    let sent = sends.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.command.get_str("find") == Ok(crate::models::channel_message::COLLECTION_NAME)
            && sent.load(Ordering::SeqCst) >= expected_sends
        {
            reads.fetch_add(1, Ordering::SeqCst);
        }
    }));
    state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(original_db.name());

    late_delivery::process(&state, &event.id).await;
    let saved = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.delivery.unwrap().state,
        "sent",
        "optional binding must not replace a successful send"
    );
    let audit: crate::models::audit_log::AuditLog = state
        .db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find_one(
            doc! {"event_type":"nyxbot_channel_answer_delivery","event_data.event_id":&event.id},
        )
        .await
        .unwrap()
        .unwrap();
    let data = audit.event_data.unwrap();
    assert_eq!(data["outcome"], "sent");
    if with_image {
        assert_eq!(data["delivery"]["status"], "confirmed");
    } else {
        assert!(data["delivery"].is_null());
    }
    assert_eq!(
        binding_reads.load(Ordering::SeqCst),
        usize::from(with_image)
    );
    assert!(
        original_db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .find_one(doc! {"_id":&event.event_id})
            .await
            .is_err(),
        "binding lookup must actually fail"
    );
    late_delivery::process(&state, &event.id).await;
    assert_eq!(
        sends.load(Ordering::SeqCst),
        expected_sends,
        "settled sends must not be retried"
    );
    server.abort();
    model.abort();
}

#[tokio::test]
async fn native_image_send_stays_confirmed_when_binding_lookup_fails_without_retry() {
    binding_lookup_failure_preserves_send(true).await;
}

#[tokio::test]
async fn native_text_send_skips_failing_binding_lookup_without_retry() {
    binding_lookup_failure_preserves_send(false).await;
}
