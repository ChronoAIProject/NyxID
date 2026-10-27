use super::*;

fn discord_event(
    row: &NyxbotChannel,
    id: &str,
    sender: &str,
    kind: &str,
    chat: &str,
    thread: Option<&str>,
    mention: bool,
) -> Value {
    let mut body = event("Discord input", sender, id);
    let context = &mut body["event_context"];
    context["channel_id"] = json!(row.gateway_channel_id);
    let activity = &mut context["activity"];
    activity.as_object_mut().unwrap().remove("telegram_input");
    activity["source"] = json!({"type":"nyxid_relay","platform":"discord","route_id":row.route_id});
    activity["conversation"] = json!({"id":chat,"kind":kind,"thread_id":thread});
    activity["kind"]["mentions_bot"] = json!(mention);
    body
}

async fn discord_channel(state: &AppState) -> (NyxbotChannel, String) {
    let (mut row, key) = channel(state, "gateway").await;
    row.platform = "discord".into();
    row.gateway_channel_id = Some("cmaeg1.ch.test".into());
    row.gateway_bot_id = Some("100".into());
    row.owner_sender_ids = vec!["7".into()];
    row.gateway_groups = Some("mention".into());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"platform":"discord","platform_bot_id":"100"}},
        )
        .await
        .unwrap();
    let response = put_binding(
        State(state.clone()),
        Path("bnd_chats".into()),
        bearer(&key),
        Json(binding_body(&key, OWNER)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    ensure_partition(state, &key, PARTITION).await;
    (load_channel(state, OWNER, &row.id).await.unwrap(), key)
}

#[tokio::test]
async fn discord_provider_service_selection_reads_original_owner_question_in_groups_and_threads() {
    let (state, calls, server) = setup("discord_provider_selection").await;
    let (row, key) = discord_channel(&state).await;
    for (id, separator, thread) in [("inline", " ", None), ("multiline", "\n", Some("201"))] {
        let mut body = discord_event(&row, id, "7", "group", "200", thread, true);
        body["input"][0]["content"][0]["text"] = json!(format!(
            "/service {}{separator}Continue the search <@100>",
            Uuid::new_v4()
        ));
        let output = body_text(respond(&state, &key, &body, id).await).await;
        assert!(output.contains("connection_unavailable"), "{output}");
        assert!(
            output.contains("Reconnect it or restore access"),
            "{output}"
        );
        assert!(output.contains("response.completed"), "{output}");
    }
    assert!(
        calls.lock().await.is_empty(),
        "invalid selection must stop before model dispatch"
    );
    // An admitted guest's identical text cannot install an owner selection.
    let mut guest = discord_event(&row, "guest-selection", "9", "group", "200", None, true);
    guest["input"][0]["content"][0]["text"] =
        json!(format!("/service {} Continue the search", Uuid::new_v4()));
    let output = body_text(respond(&state, &key, &guest, "guest-selection").await).await;
    assert!(output.contains("Here is your answer"), "{output}");
    assert!(!output.contains("connection_unavailable"), "{output}");
    assert_eq!(calls.lock().await.len(), 1);
    server.abort();
}

#[tokio::test]
async fn discord_relay_service_selection_preserves_gateway_message_text_before_attribution() {
    let (state, calls, server) = setup("discord_relay_selection").await;
    let (mut row, _) = discord_channel(&state).await;
    row.transport = "direct".into();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    for (id, chat_id, thread_id) in [("500", "200", None), ("501", "201", Some("201"))] {
        let raw = json!({"id":id,"channel_id":chat_id,"guild_id":"300","type":0,
            "author":{"id":"7","username":"Alice","bot":false},"mentions":[{"id":"100"}],
            "content":format!("/service {}\nContinue the search <@100>", Uuid::new_v4())});
        let inbound =
            crate::services::channel_adapters::discord::parse_gateway_message(&raw).unwrap();
        let chat = chats::record_chat(
            &state,
            &row,
            &chats::group_partition(chat_id, thread_id),
            &chats::ChatFacts {
                kind: "group",
                chat_id: chat_id.into(),
                thread_id: thread_id.map(str::to_owned),
                owner: true,
                title: Some("Discord channel".into()),
            },
            None,
        )
        .await
        .unwrap();
        let mut admitted = admission(&row.id, id);
        admitted.status = "running".into();
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .insert_one(&admitted)
            .await
            .unwrap();
        let sender = Sender {
            original_message: true,
            media: Vec::new(),
            kind: chats::native_sender_kind("discord", &raw),
            owner_eligible: chats::native_owner_eligible("discord", &raw),
            id: &inbound.sender_platform_id,
            display_name: inbound.sender_display_name.as_deref(),
        };
        let result = Box::pin(inbound_message(
            &state,
            &row,
            &chat,
            &sender,
            inbound.text.as_deref().unwrap(),
            chats::raw_addressed(&bot, &raw, Some("100")),
            &admitted.id,
        ))
        .await
        .unwrap();
        let Inbound::Reply(output) = result else {
            panic!("selection must refuse before starting a turn")
        };
        assert!(output.contains("connection_unavailable"), "{output}");
        assert!(
            output.contains("Reconnect it or restore access"),
            "{output}"
        );
    }
    assert!(calls.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn discord_provider_rejects_misbound_identity_edits_and_private_threads() {
    let (state, calls, server) = setup("discord_provider_validation").await;
    let (row, key) = discord_channel(&state).await;
    let body = discord_event(&row, "evt", "7", "private", "200", None, false);
    assert!(discord_provider_context(&row, &body["event_context"]));
    let mut pending = row.clone();
    pending.transport = "direct".into();
    assert!(!discord_provider_context(&pending, &body["event_context"]));
    for (pointer, value) in [
        ("/event_context/channel_id", json!("foreign")),
        ("/event_context/event_ref", Value::Null),
        ("/event_context/activity/source/platform", json!("lark")),
        ("/event_context/activity/source/route_id", json!("foreign")),
        ("/event_context/activity/conversation/id", json!("#general")),
        ("/event_context/activity/conversation/kind", Value::Null),
        (
            "/event_context/activity/conversation/thread_id",
            json!("300"),
        ),
        ("/event_context/activity/actor/id", json!("Alice")),
        ("/event_context/activity/kind/type", json!("message_update")),
    ] {
        let mut invalid = body.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        let response = respond(&state, &key, &invalid, "invalid").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{pointer}");
    }
    assert!(calls.lock().await.is_empty());
    assert_eq!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
async fn discord_provider_preserves_dm_group_thread_identity_and_live_member_policy() {
    let (state, calls, server) = setup("discord_provider_policy").await;
    let (row, key) = discord_channel(&state).await;
    // Text and display names never establish a mention; even an admitted
    // gateway event must pass the local reply-mode check.
    let mut body = discord_event(&row, "plain", "7", "group", "200", None, false);
    body["input"][0]["content"][0]["text"] = json!("@helper_bot <@100>");
    body_text(respond(&state, &key, &body, "plain").await).await;
    assert!(calls.lock().await.is_empty());
    for (id, sender, thread) in [
        ("owner", "7", None),
        ("guest", "9", None),
        ("thread", "7", Some("201")),
    ] {
        let body = discord_event(&row, id, sender, "group", "200", thread, true);
        let output = body_text(respond(&state, &key, &body, id).await).await;
        assert!(output.contains("Here is your answer"), "{output}");
        let partition = chats::group_partition(thread.unwrap_or("200"), thread);
        let chat = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"channel_id":&row.id,"partition":&partition})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            chat.platform_chat_id.as_deref(),
            Some(thread.unwrap_or("200"))
        );
        let conversation = engine::get(&state.db, OWNER, chat.conversation_id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(conversation.guest_turn, sender == "9");
        settle(&state, &conversation.id).await;
    }
    assert_eq!(calls.lock().await.len(), 3);
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"channel_id":&row.id,"partition":chats::group_partition("200",None)},
            doc! {"$set":{"members":"owner","reply_mode":"all"}},
        )
        .await
        .unwrap();
    for (id, sender, actor) in [
        ("blocked", "9", "human"),
        ("unknown", "7", "unknown"),
        ("self", "100", "human"),
        ("bot", "9", "bot"),
    ] {
        let mut body = discord_event(&row, id, sender, "group", "200", None, true);
        body["event_context"]["activity"]["actor"]["kind"] = json!(actor);
        body_text(respond(&state, &key, &body, id).await).await;
    }
    assert_eq!(calls.lock().await.len(), 3);
    let all = discord_event(&row, "all", "7", "group", "200", None, false);
    body_text(respond(&state, &key, &all, "all").await).await;
    assert_eq!(calls.lock().await.len(), 4);
    let dm = discord_event(&row, "dm", "7", "private", "400", None, false);
    body_text(respond(&state, &key, &dm, "dm").await).await;
    let chat = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&row.id,"partition":PARTITION})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chat.kind.as_deref(), Some("private"));
    assert_eq!(
        chat.relay_partition.as_deref(),
        Some(chats::direct_partition("400", "7", None).as_str())
    );
    assert_eq!(calls.lock().await.len(), 5);
    server.abort();
}

#[tokio::test]
async fn discord_provider_unknown_actors_cannot_verify_owner_and_media_has_no_fetch_claim() {
    let (state, calls, server) = setup("discord_provider_owner_media").await;
    let (row, key) = discord_channel(&state).await;
    let link = issue_owner_link(&state, &row).await;
    let mut body = discord_event(&row, "unknown", "9", "private", "400", None, false);
    body["input"][0]["content"][0]["text"] = json!(link.code);
    body["event_context"]["activity"]["actor"]["kind"] = json!("unknown");
    body_text(respond(&state, &key, &body, "unknown").await).await;
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .owner_sender_ids,
        vec!["7"]
    );
    body["event_context"]["activity"]["actor"]["kind"] = json!("human");
    body["event_context"]["activity"]["event_id"] = json!("verified");
    let output = body_text(respond(&state, &key, &body, "verified").await).await;
    assert!(output.contains("Linked."), "{output}");
    assert!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .owner_sender_ids
            .contains(&"9".to_string())
    );
    assert!(calls.lock().await.is_empty());
    let mut media = discord_event(&row, "media", "7", "private", "400", None, false);
    media["event_context"]["activity"]["kind"]["attachments"] =
        json!([{"url":"http://127.0.0.1/private"}]);
    body_text(respond(&state, &key, &media, "media").await).await;
    let stored = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"channel_id":&row.id,"event_id":"media"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.source_media_declared, Some(1));
    assert!(stored.media.is_none());
    server.abort();
}
