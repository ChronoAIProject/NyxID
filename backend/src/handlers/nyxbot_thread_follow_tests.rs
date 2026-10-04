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

#[tokio::test]
async fn direct_follow_uses_separate_native_children_and_owner_stop_without_model() {
    let (state, calls, model) = setup("nyxbot_thread_follow_callback").await;
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
