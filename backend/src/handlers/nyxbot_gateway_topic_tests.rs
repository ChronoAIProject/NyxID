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
async fn gateway_telegram_reply_chains_stay_in_the_chat_and_topics_get_their_own_thread() {
    let (state, _calls, server) = setup("nyxbot_gateway_topics").await;
    let (row, agent_key) = channel(&state, "gateway").await;
    assert_eq!(row.platform, "telegram");
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
    let chat_partition = chats::group_partition("-100200", None);
    let topic_partition = chats::group_partition("-100200", Some("7"));

    // A reply inside the plain group: the gateway sends the reply-chain root
    // as `conversation.thread_id`, and its version 1 facts say reply chain.
    // The message lands in the chat's own thread with no topic recorded.
    let chain_facts = json!({
        "version": 1, "kind": "reply_chain", "chat_id": "-100200",
        "message_id": "17", "parent_message_id": "16",
    });
    answered(
        &state,
        &agent_key,
        &telegram_group_event(
            "@helper_bot plan lunch",
            "evt-chain",
            partition,
            Some("16"),
            Some(chain_facts.clone()),
        ),
        "evt_chain",
    )
    .await;
    assert_eq!(
        partitions(&state, &row.id).await.as_slice(),
        std::slice::from_ref(&chat_partition)
    );
    let chat = thread_row(&state, &row.id, &chat_partition).await.unwrap();
    assert_eq!(chat.platform_chat_id.as_deref(), Some("-100200"));
    assert_eq!(chat.platform_thread_id, None);
    assert!(
        thread_row(
            &state,
            &row.id,
            &chats::group_partition("-100200", Some("16"))
        )
        .await
        .is_none()
    );

    // A real forum topic (version 1 facts: topic 7) is its own thread with
    // the decimal topic recorded, separate from the chat's thread.
    let topic_facts = json!({
        "version": 1, "kind": "topic", "chat_id": "-100200",
        "message_id": "18", "native_thread_id": "7", "root_id": "7",
    });
    answered(
        &state,
        &agent_key,
        &telegram_group_event(
            "@helper_bot in the topic",
            "evt-topic",
            partition,
            Some("7"),
            Some(topic_facts),
        ),
        "evt_topic",
    )
    .await;
    assert_eq!(
        partitions(&state, &row.id).await,
        [chat_partition.clone(), topic_partition.clone()]
    );
    let topic = thread_row(&state, &row.id, &topic_partition).await.unwrap();
    assert_eq!(topic.platform_chat_id.as_deref(), Some("-100200"));
    assert_eq!(topic.platform_thread_id.as_deref(), Some("7"));
    assert_ne!(topic.id, chat.id);

    // Without facts a positive decimal id keeps the existing gateway
    // contract: the same topic thread, no new conversation.
    answered(
        &state,
        &agent_key,
        &telegram_group_event(
            "@helper_bot again in the topic",
            "evt-topic-2",
            partition,
            Some("7"),
            None,
        ),
        "evt_topic_2",
    )
    .await;
    assert_eq!(
        partitions(&state, &row.id).await,
        [chat_partition.clone(), topic_partition.clone()]
    );
    assert_eq!(
        thread_row(&state, &row.id, &topic_partition)
            .await
            .unwrap()
            .platform_thread_id
            .as_deref(),
        Some("7")
    );

    // Without facts anything that is not a positive integer is chat-level,
    // and a reply chain declared by facts stays chat-level however many ids
    // the gateway sends: the chat's thread is reused, nothing new is created.
    for (event_id, thread_id, facts) in [
        ("evt-chain-2", Some("chain-16"), None),
        ("evt-chain-3", Some("0"), None),
        ("evt-chain-4", Some("23"), Some(chain_facts.clone())),
        ("evt-chain-5", None, Some(chain_facts.clone())),
    ] {
        answered(
            &state,
            &agent_key,
            &telegram_group_event(
                "@helper_bot once more",
                event_id,
                partition,
                thread_id,
                facts,
            ),
            &event_id.replace('-', "_"),
        )
        .await;
        assert_eq!(
            partitions(&state, &row.id).await,
            [chat_partition.clone(), topic_partition.clone()],
            "{event_id}"
        );
        assert_eq!(
            thread_row(&state, &row.id, &chat_partition)
                .await
                .unwrap()
                .platform_thread_id,
            None
        );
    }
    server.abort();
}
