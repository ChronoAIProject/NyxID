use super::*;

async fn thread_row(state: &AppState, channel_id: &str, partition: &str) -> Option<NyxbotThread> {
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id": channel_id, "partition": partition})
        .await
        .unwrap()
}

/// A Telegram group Activity from the owner with the gateway's
/// `conversation.thread_id` and, optionally, version 1 thread facts.
fn telegram_group_event(
    text: &str,
    event_id: &str,
    partition: &str,
    thread_id: Option<&str>,
    facts: Option<Value>,
) -> Value {
    let mut body = group_event(text, "7", "Alice", event_id, partition, true);
    let activity = &mut body["event_context"]["activity"];
    activity["conversation"] = json!({"id": "-100200", "kind": "group"});
    if let Some(thread_id) = thread_id {
        activity["conversation"]["thread_id"] = json!(thread_id);
    }
    if let Some(facts) = facts {
        activity["thread"] = facts;
    }
    body
}

async fn answered(state: &AppState, agent_key: &str, body: &Value, idempotency: &str) {
    let text = body_text(respond(state, agent_key, body, idempotency).await).await;
    assert!(text.contains("Here is your answer"), "{text}");
}

/// The partitions of the channel's conversations, oldest first, each settled
/// so the next Activity into the same thread is admitted.
async fn partitions(state: &AppState, channel_id: &str) -> Vec<String> {
    let mut rows = channel_conversations(state, channel_id).await;
    rows.sort_by_key(|a| a.created_at);
    for row in &rows {
        settle(state, &row.id).await;
    }
    rows.iter()
        .map(|row| row.channel.as_ref().unwrap().partition.clone())
        .collect()
}

#[tokio::test]
async fn gateway_telegram_keeps_existing_inbound_threads_and_separate_outbound_topic_evidence() {
    use crate::models::channel_thread::TelegramTopic;
    let (state, _calls, server) = setup("nyxbot_gateway_topics").await;
    let (row, agent_key) = channel(&state, "gateway").await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"owner_sender_ids": ["7"]}},
        )
        .await
        .unwrap();
    let bound = put_binding(
        State(state.clone()),
        Path("bnd_chats".into()),
        bearer(&agent_key),
        Json(binding_body(&agent_key, OWNER)),
    )
    .await;
    assert_eq!(bound.status(), StatusCode::OK);
    let partition = "conv_00000000000000000000000000000007";
    ensure_partition(&state, &agent_key, partition).await;
    let chain_partition = chats::group_partition("-100200", Some("16"));
    // First create the legacy reply-chain partition without new topic facts.
    answered(
        &state,
        &agent_key,
        &telegram_group_event(
            "@helper_bot before upgrade",
            "evt-old",
            partition,
            Some("16"),
            None,
        ),
        "evt_old",
    )
    .await;
    partitions(&state, &row.id).await;
    let old = thread_row(&state, &row.id, &chain_partition).await.unwrap();
    state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"_id": &old.id},
            doc! {"$set": {"allow_posts": true, "members": "everyone"}},
        )
        .await
        .unwrap();
    let chain_facts = json!({
        "version": 1, "kind": "reply_chain", "chat_id": "-100200",
        "message_id": "17", "parent_message_id": "16",
    });
    answered(
        &state,
        &agent_key,
        &telegram_group_event(
            "@helper_bot after upgrade",
            "evt-new",
            partition,
            Some("16"),
            Some(chain_facts),
        ),
        "evt_new",
    )
    .await;
    assert_eq!(
        partitions(&state, &row.id).await.as_slice(),
        std::slice::from_ref(&chain_partition),
    );
    let current = thread_row(&state, &row.id, &chain_partition).await.unwrap();
    assert_eq!(current.id, old.id);
    assert_eq!(current.conversation_id, old.conversation_id);
    assert_eq!(current.platform_thread_id.as_deref(), Some("16"));
    assert!(current.allow_posts);
    assert_eq!(current.members.as_deref(), Some("everyone"));
    assert_eq!(current.telegram_topic, Some(TelegramTopic::Chat));
    assert_eq!(
        crate::services::channel_image_audience::chat_thread(&current),
        None
    );
    let topic_partition = chats::group_partition("-100200", Some("7"));
    answered(
        &state,
        &agent_key,
        &telegram_group_event(
            "@helper_bot real topic",
            "evt-topic",
            partition,
            Some("7"),
            Some(json!({
                "version": 1,
                "kind": "topic",
                "chat_id": "-100200",
                "message_id": "18",
                "native_thread_id": "7",
                "root_id": "7",
            })),
        ),
        "evt_topic",
    )
    .await;
    partitions(&state, &row.id).await;
    let topic = thread_row(&state, &row.id, &topic_partition).await.unwrap();
    assert_ne!(topic.id, current.id);
    assert_eq!(
        crate::services::channel_image_audience::chat_thread(&topic),
        Some("7")
    );
    // Legacy gateway IDs remain exact inbound keys, including nonnumeric IDs.
    for (i, id) in ["chain-16", "0"].into_iter().enumerate() {
        answered(
            &state,
            &agent_key,
            &telegram_group_event(
                "@helper_bot legacy key",
                &format!("evt-legacy-{i}"),
                partition,
                Some(id),
                None,
            ),
            &format!("evt_legacy_{i}"),
        )
        .await;
        partitions(&state, &row.id).await;
        let thread = thread_row(
            &state,
            &row.id,
            &chats::group_partition("-100200", Some(id)),
        )
        .await
        .unwrap();
        assert_eq!(thread.platform_thread_id.as_deref(), Some(id));
    }
    server.abort();
}
