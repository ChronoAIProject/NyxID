//! External native bots address the receiving bot's existing specialist.
use super::*;

#[test]
fn direct_native_actor_classification_preserves_card_actions_and_channel_posts() {
    use crate::models::channel_thread::ThreadSenderKind::{Bot, Human, Unknown};

    let card_action = json!({
        "header": {"event_type": "card.action.trigger"},
        "event": {"operator": {"open_id": "ou_operator123"}}
    });
    assert_eq!(chats::native_sender_kind("lark", &card_action), Human);
    assert_eq!(chats::native_sender_kind("feishu", &card_action), Human);

    let malformed_card = json!({
        "header": {"event_type": "card.action.trigger"},
        "event": {"operator": {"name": "spoofed"}, "sender": {"sender_type": "app"}}
    });
    assert_eq!(chats::native_sender_kind("lark", &malformed_card), Unknown);

    let channel_post = json!({
        "update_id": 7,
        "channel_post": {
            "message_id": 8,
            "chat": {"id": -100, "type": "channel"},
            "text": "announcement"
        }
    });
    assert_eq!(chats::native_sender_kind("telegram", &channel_post), Human);
    assert!(!chats::native_owner_eligible("telegram", &channel_post));

    let foreign_bot = json!({
        "message": {
            "from": {"id": 9009, "is_bot": true},
            "chat": {"id": -100, "type": "supergroup"},
            "text": "@helper_bot",
            "entities": [{"type": "mention", "offset": 0, "length": 11}]
        }
    });
    assert_eq!(chats::native_sender_kind("telegram", &foreign_bot), Bot);
}

async fn deliver_signed_channel_post(
    state: &AppState,
    row: &NyxbotChannel,
    message_id: &str,
    update_id: i64,
) {
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": message_id, "channel_bot_id": &row.channel_bot_id, "user_id": OWNER,
            "direction": "inbound", "platform": "telegram", "conversation_id": &row.route_id,
            "agent_api_key_id": &row.route_api_key_id, "sender_platform_id": "",
            "platform_conversation_id": "-100", "content_type": "text",
            "platform_message_id": update_id.to_string(), "native_event_id": update_id.to_string(),
            "created_at": bson::DateTime::now()
        })
        .await
        .unwrap();
    let body = json!({
        "message_id": message_id,
        "correlation_id": format!("jti-{message_id}"),
        "platform": "telegram",
        "agent": {"api_key_id": row.route_api_key_id, "name": "route"},
        "conversation": {"id": row.route_id, "platform_id": "-100", "type": "channel"},
        "sender": {"platform_id": "", "display_name": null},
        "content": {"type": "text", "text": "announcement"},
        "raw_platform_data": {
            "update_id": update_id,
            "channel_post": {"message_id": update_id, "chat": {"id": -100, "type": "channel"}, "text": "announcement"}
        }
    });
    let bytes = serde_json::to_vec(&body).unwrap();
    let token = crate::crypto::jwt::generate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        &format!("jti-{message_id}"),
        &row.route_api_key_id,
        message_id,
        "telegram",
        &sha256_hex(&bytes),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-nyxid-callback-token", token.parse().unwrap());
    let (_watch, done) = watch_relay_callback(&row.id, message_id);
    assert_eq!(
        relay_callback(
            State(state.clone()),
            Path(row.id.clone()),
            headers,
            Bytes::from(bytes),
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(Duration::from_secs(20), done)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn external_signed_telegram_channel_post_keeps_guest_delivery_without_owner_reset() {
    let (state, calls, server) = setup("external_telegram_channel_post").await;
    let (mut row, _) = channel(&state, "direct").await;
    row.platform = "telegram".into();
    row.route_id = Some("route-channel-post".into());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &row.id}, &row)
        .await
        .unwrap();
    let chat = chats::record_chat(
        &state,
        &row,
        &chats::group_partition("-100", None),
        &chats::ChatFacts {
            kind: "channel",
            chat_id: "-100".into(),
            thread_id: None,
            owner: false,
            title: Some("Announcements".into()),
        },
        None,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id": &chat.id},
            doc! {"$set": {"members": "everyone", "reply_mode": "all", "native_bot_hops_remaining": 2}},
        )
        .await
        .unwrap();
    let message_id = "channel-post-1";
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": message_id, "channel_bot_id": &row.channel_bot_id, "user_id": OWNER,
            "direction": "inbound", "platform": "telegram", "conversation_id": &row.route_id,
            "agent_api_key_id": &row.route_api_key_id, "sender_platform_id": "",
            "platform_conversation_id": "-100", "content_type": "text",
            "platform_message_id": "88", "native_event_id": "501", "created_at": bson::DateTime::now()
        })
        .await
        .unwrap();
    let body = json!({
        "message_id": message_id,
        "correlation_id": "jti-channel-post-1",
        "platform": "telegram",
        "agent": {"api_key_id": row.route_api_key_id, "name": "route"},
        "conversation": {"id": row.route_id, "platform_id": "-100", "type": "channel"},
        "sender": {"platform_id": "", "display_name": null},
        "content": {"type": "text", "text": "announcement"},
        "raw_platform_data": {
            "update_id": 501,
            "channel_post": {"message_id": 88, "chat": {"id": -100, "type": "channel"}, "text": "announcement"}
        }
    });
    let bytes = serde_json::to_vec(&body).unwrap();
    let token = crate::crypto::jwt::generate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        "jti-channel-post-1",
        &row.route_api_key_id,
        message_id,
        "telegram",
        &sha256_hex(&bytes),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-nyxid-callback-token", token.parse().unwrap());
    let (_watch, done) = watch_relay_callback(&row.id, message_id);
    assert_eq!(
        relay_callback(
            State(state.clone()),
            Path(row.id.clone()),
            headers,
            Bytes::from(bytes),
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(Duration::from_secs(20), done)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(calls.lock().await.len(), 1);
    settle(&state, &chat.id).await;
    let stored = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id": &chat.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !stored.owner_seen,
        "channel posts cannot establish owner presence"
    );
    assert_eq!(stored.native_bot_hops_remaining, Some(2));

    // Channel posts remain subject to the existing guest consent and
    // addressing policy. They never become owners merely because the
    // provider omitted a user actor.
    let before = calls.lock().await.len();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id": &chat.id},
            doc! {"$set": {"members": "owner", "reply_mode": "all"}},
        )
        .await
        .unwrap();
    deliver_signed_channel_post(&state, &row, "channel-post-owner-only", 502).await;
    assert_eq!(calls.lock().await.len(), before);
    settle(&state, &chat.id).await;

    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id": &chat.id},
            doc! {"$unset": {"members": "", "reply_mode": ""}},
        )
        .await
        .unwrap();
    deliver_signed_channel_post(&state, &row, "channel-post-default", 503).await;
    assert_eq!(calls.lock().await.len(), before);
    settle(&state, &chat.id).await;

    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id": &chat.id},
            doc! {"$set": {"members": "everyone", "reply_mode": "mention"}},
        )
        .await
        .unwrap();
    deliver_signed_channel_post(&state, &row, "channel-post-unaddressed", 504).await;
    assert_eq!(calls.lock().await.len(), before);
    let stored = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id": &chat.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.native_bot_hops_remaining, Some(2));
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_busy_human_group_retry_keeps_one_reset_and_can_start_once() {
    let (state, calls, server) = setup("external_human_busy_retry").await;
    let (mut row, agent_key) = channel(&state, "gateway").await;
    row.owner_sender_ids = vec!["7".into()];
    row.gateway_bot_mentions.enabled = true;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &row.id}, &row)
        .await
        .unwrap();
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&agent_key),
            Json(binding_body(&agent_key, OWNER)),
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_partition(&state, &agent_key, PARTITION).await;
    let mut slots = Vec::new();
    for _ in 0..crate::services::assistant_team_service::CHANNEL_TURNS_PER_OWNER {
        slots.push(
            state
                .direct_chat_limiter
                .try_acquire_pool(
                    "assistant_channel",
                    OWNER,
                    crate::services::assistant_team_service::CHANNEL_TURNS_PER_OWNER,
                )
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let input = group_event(
        "@helper_bot plan lunch",
        "7",
        "Owner",
        "busy-human",
        PARTITION,
        true,
    );
    assert_eq!(
        respond(&state, &agent_key, &input, "busy-human")
            .await
            .status(),
        StatusCode::CONFLICT,
        "no turn has started while the owner pool is full"
    );
    assert!(calls.lock().await.is_empty());
    let chat = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id": &row.id, "kind": "group"})
        .await
        .unwrap()
        .unwrap();
    // An admitted bot attempt while the pool is full still spends one hop.
    let mut bot = group_event(
        "@helper_bot hello",
        "9009",
        "Foreign bot",
        "busy-bot",
        PARTITION,
        true,
    );
    bot["event_context"]["activity"]["actor"]["kind"] = json!("bot");
    let _ = body_text(respond(&state, &agent_key, &bot, "busy-bot").await).await;
    let remaining = || async {
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"_id": &chat.id})
            .await
            .unwrap()
            .unwrap()
            .native_bot_hops_remaining
    };
    assert_eq!(remaining().await, Some(5));
    assert_eq!(
        respond(&state, &agent_key, &input, "busy-human")
            .await
            .status(),
        StatusCode::CONFLICT,
        "a no-effect human busy response remains retryable"
    );
    assert_eq!(remaining().await, Some(5), "retry cannot refill bot hops");
    let mut changed = input.clone();
    changed["input"][0]["content"][0]["text"] = json!("changed retry");
    let _ = body_text(respond(&state, &agent_key, &changed, "busy-human").await).await;
    assert!(calls.lock().await.is_empty());
    drop(slots);
    let retries = futures::future::join_all((0..8).map(|_| async {
        body_text(respond(&state, &agent_key, &input, "busy-human").await).await
    }))
    .await;
    assert_eq!(
        retries
            .iter()
            .filter(|body| body.contains("Here is your answer"))
            .count(),
        1,
        "the original human input executes once after capacity returns"
    );
    assert_eq!(calls.lock().await.len(), 1);
    assert_eq!(
        remaining().await,
        Some(5),
        "successful retry does not reset"
    );
    let _ = body_text(respond(&state, &agent_key, &bot, "busy-bot").await).await;
    assert_eq!(
        calls.lock().await.len(),
        1,
        "busy bot work is never retried"
    );
    assert_eq!(remaining().await, Some(5));
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_bot_mentions_start_only_bounded_guest_turns_without_sender_oauth() {
    use crate::services::assistant_team_service as team;
    for platform in ["telegram", "lark"] {
        let (state, calls, server) = setup(&format!("external_mentions_{platform}")).await;
        let (mut row, agent_key) = channel(&state, "gateway").await;
        let (specialist, _) = team::create_specialist(
            &state.db,
            &state.encryption_keys,
            OWNER,
            team::CreateRequest {
                machines: None,
                logins: None,
                name: "community".into(),
                description: "Community assistant".into(),
                display_name: None,
                persona: None,
                targets: Default::default(),
                account_read: false,
                specialty: None,
                created_by: "user",
            },
        )
        .await
        .unwrap()
        .unwrap();
        row.agent_id = Some(specialist.id.clone());
        row.platform = platform.into();
        row.gateway_bot_id = (platform == "lark").then(|| "ou_receiver".into());
        row.gateway_groups = Some("all".into());
        row.gateway_bot_mentions.supported = true;
        row.gateway_bot_mentions.enabled = true;
        row.gateway_bot_mentions.checked_at = Some(Utc::now());
        // Even a corrupt/legacy owner ID must never turn an app actor into its owner.
        let foreign = if platform == "telegram" {
            "9009"
        } else {
            "ou_foreign"
        };
        row.owner_sender_ids = vec!["human-owner".into(), foreign.into()];
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id": &row.id}, &row)
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &row.channel_bot_id},
                doc! {"$set": {"platform": platform,
                "platform_bot_id": if platform == "lark" {"cli_receiver"} else {"123"}}},
            )
            .await
            .unwrap();
        assert_eq!(
            put_binding(
                State(state.clone()),
                Path("bnd_chats".into()),
                bearer(&agent_key),
                Json(binding_body(&agent_key, OWNER))
            )
            .await
            .status(),
            StatusCode::OK
        );
        ensure_partition(&state, &agent_key, PARTITION).await;
        let chat = chats::record_chat(
            &state,
            &row,
            &chats::group_partition("-100200", Some("42")),
            &chats::ChatFacts {
                kind: "group",
                chat_id: "-100200".into(),
                thread_id: Some("42".into()),
                owner: false,
                title: Some("Existing group".into()),
            },
            None,
        )
        .await
        .unwrap();
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .update_one(
                doc! {"_id": &chat.id},
                doc! {"$set": {"members": "everyone"}},
            )
            .await
            .unwrap();
        let input = |id: &str, actor: &str, kind: &str, addressed: bool| {
            let mut value = group_event(
                "@helper_bot hello",
                actor,
                "Bot (owner)",
                id,
                PARTITION,
                addressed,
            );
            value["event_context"]["activity"]["actor"]["kind"] = json!(kind);
            value["event_context"]["activity"]["conversation"]["thread_id"] = json!("42");
            value
        };
        // No sender account or OAuth was created for this actor.
        for collection in ["oauth_broker_bindings", "notification_channels"] {
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        for (id, actor, kind, mentioned) in [
            ("chatter", foreign, "bot", false),
            ("unknown", foreign, "system", true),
            (
                "self",
                if platform == "lark" {
                    "ou_receiver"
                } else {
                    "123"
                },
                "bot",
                true,
            ),
        ] {
            let text = body_text(
                respond(&state, &agent_key, &input(id, actor, kind, mentioned), id).await,
            )
            .await;
            assert!(!text.contains("Here is your answer"), "{id}: {text}");
        }
        assert!(calls.lock().await.is_empty());
        state
            .db
            .collection::<bson::Document>(THREADS)
            .update_one(doc! {"_id":&chat.id}, doc! {"$unset":{"members":""}})
            .await
            .unwrap();
        assert!(
            !body_text(
                respond(
                    &state,
                    &agent_key,
                    &input("owner-consent-required", foreign, "bot", true),
                    "owner-consent-required"
                )
                .await
            )
            .await
            .contains("Here is your answer")
        );
        state
            .db
            .collection::<bson::Document>(THREADS)
            .update_one(doc! {"_id":&chat.id}, doc! {"$set":{"members":"everyone"}})
            .await
            .unwrap();
        let before_rows = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .count_documents(doc! {"channel_id":&row.id})
            .await
            .unwrap();
        let mut unknown_chat = input("unknown-chat", foreign, "bot", true);
        unknown_chat["event_context"]["activity"]["conversation"]["id"] =
            json!("not-owner-enabled");
        assert!(
            !body_text(respond(&state, &agent_key, &unknown_chat, "unknown-chat").await)
                .await
                .contains("Here is your answer")
        );
        assert_eq!(
            state
                .db
                .collection::<NyxbotThread>(THREADS)
                .count_documents(doc! {"channel_id":&row.id})
                .await
                .unwrap(),
            before_rows,
            "bots cannot allocate settings rows"
        );
        // Each native bot has its own authenticated ingress. It reaches only this
        // channel/chat's existing member, never a name-resolved foreign agent.
        for i in 0..6 {
            let id = format!("bot-{i}");
            let body = input(&id, foreign, "bot", true);
            let text = body_text(respond(&state, &agent_key, &body, &id).await).await;
            assert!(
                text.contains("Here is your answer"),
                "{platform} addressed bot {i}: {text}"
            );
            let conversations = channel_conversations(&state, &row.id).await;
            assert_eq!(conversations.len(), 1);
            let conversation = &conversations[0];
            assert_eq!(
                conversation.agent_id.as_deref(),
                Some(specialist.id.as_str())
            );
            assert!(conversation.guest_turn, "bots are always guests");
            assert_eq!(
                conversation.channel.as_ref().unwrap().partition,
                chat.partition
            );
            settle(&state, &conversation.id).await;
            let replayed = body_text(respond(&state, &agent_key, &body, &id).await).await;
            assert!(
                !replayed.contains("Here is your answer"),
                "a completed replay has no second response effect"
            );
            assert_eq!(calls.lock().await.len(), i + 1, "replay cannot run twice");
            let retry =
                body_text(respond(&state, &agent_key, &body, &format!("different-{id}")).await)
                    .await;
            assert!(
                !retry.contains("Here is your answer"),
                "changing retry keys cannot duplicate a native event"
            );
        }
        let exhausted = body_text(
            respond(
                &state,
                &agent_key,
                &input("exhausted", foreign, "bot", true),
                "exhausted",
            )
            .await,
        )
        .await;
        assert!(!exhausted.contains("Here is your answer"));
        assert_eq!(calls.lock().await.len(), 6);
        // Only an admitted provider-identified human resets the chat budget.
        let human = body_text(
            respond(
                &state,
                &agent_key,
                &input("human-reset", "unlinked-human", "human", true),
                "human-reset",
            )
            .await,
        )
        .await;
        assert!(human.contains("Here is your answer"));
        let conversations = channel_conversations(&state, &row.id).await;
        settle(&state, &conversations[0].id).await;
        assert!(
            body_text(
                respond(
                    &state,
                    &agent_key,
                    &input("after-human", foreign, "bot", true),
                    "after-human"
                )
                .await
            )
            .await
            .contains("Here is your answer")
        );
        // Owner-only policy remains authoritative even for positive mentions.
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .update_one(doc! {"_id": &chat.id}, doc! {"$set": {"members": "owner"}})
            .await
            .unwrap();
        let before = calls.lock().await.len();
        assert!(
            !body_text(
                respond(
                    &state,
                    &agent_key,
                    &input("owner-only", foreign, "bot", true),
                    "owner-only"
                )
                .await
            )
            .await
            .contains("Here is your answer")
        );
        assert_eq!(calls.lock().await.len(), before);
        assert!(
            !state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"_id": &chat.id})
                .await
                .unwrap()
                .unwrap()
                .owner_seen
        );
        server.abort();
        state.db.drop().await.unwrap();
    }
}

#[test]
fn external_lark_rendering_never_replaces_prefixes_or_conflicting_keys() {
    let raw = json!({"event": {"message": {"mentions": [
        {"key": "@_user_1", "name": "Community"},
        {"key": "@_user_2", "name": "First"},
        {"key": "@_user_2", "name": "Other"}
    ]}}});
    assert_eq!(
        chats::named_mentions("@_user_10 @_user_1 @_user_2", &raw),
        "@_user_10 @Community @_user_2"
    );
}

#[test]
fn external_telegram_addressing_requires_valid_native_ranges() {
    let bot: ChannelBot = bson::from_document(bot_doc("telegram", "Helper bot")).unwrap();
    for (native, expected) in [
        (json!({"text": "mail@helper_bot or @helper_bot"}), false),
        (
            json!({"text": "😀 @helper_bot", "entities": [{"type":"mention","offset":3,"length":11}]}),
            true,
        ),
        (
            json!({"caption": "😀 Helper", "caption_entities": [{"type":"text_mention","offset":3,"length":6,"user":{"id":123}}]}),
            true,
        ),
        (
            json!({"text":"😀 Helper","entities":[{"type":"text_mention","offset":1,"length":1,"user":{"id":123}}]}),
            false,
        ),
        (
            json!({"text":"@helper_bot","entities":[{"type":"mention","offset":0,"length":11},{"type":"text_mention","offset":0,"length":11,"user":{"id":456}}]}),
            false,
        ),
        (
            json!({"text":"@helper_bot","entities":[{"type":"mention","offset":0,"length":11},{"type":"code","offset":0,"length":11}]}),
            false,
        ),
        (
            json!({"text":"/ask@helper_bot","entities":[{"type":"bot_command","offset":0,"length":15}]}),
            true,
        ),
        (
            json!({"text":"hi","caption":"@helper_bot","caption_entities":[{"type":"mention","offset":0,"length":11}]}),
            false,
        ),
    ] {
        assert_eq!(
            chats::raw_addressed(&bot, &json!({"message":native.clone()}), None),
            Some(expected),
            "{native}"
        );
    }
}

#[tokio::test]
async fn external_direct_lark_bot_mentions_are_guests_and_cannot_link_the_owner() {
    let (state, calls, server) = setup("external_direct_bot_mentions").await;
    let (mut row, _) = channel(&state, "direct").await;
    row.route_id = Some("route".into());
    row.owner_sender_ids = vec!["ou_actor".into()];
    row.link_code_hash = Some(sha256_hex("owner-code"));
    row.link_code_expires_at = Some(Utc::now() + ChronoDuration::minutes(5));
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &row.id}, &row)
        .await
        .unwrap();
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), "ou_receiver".into());
    let chat = chats::record_chat(
        &state,
        &row,
        &chats::group_partition("oc_chat", Some("omt_topic")),
        &chats::ChatFacts {
            kind: "group",
            chat_id: "oc_chat".into(),
            thread_id: Some("omt_topic".into()),
            owner: false,
            title: Some("Trusted group".into()),
        },
        None,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id":&chat.id},
            doc! {"$set":{"members":"everyone","reply_mode":"all"}},
        )
        .await
        .unwrap();
    // Card actions are provider events without a thread_id. Exercise them in
    // a separately consented root chat, matching lark.rs's parser contract.
    let card_chat = chats::record_chat(
        &state,
        &row,
        &chats::group_partition("oc_chat", None),
        &chats::ChatFacts {
            kind: "group",
            chat_id: "oc_chat".into(),
            thread_id: None,
            owner: false,
            title: Some("Card root".into()),
        },
        None,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id": &card_chat.id},
            doc! {"$set": {"members": "everyone", "reply_mode": "all"}},
        )
        .await
        .unwrap();
    for (id, sender, sender_kind, mentions, expected) in [
        (
            "foreign",
            "ou_actor",
            "app",
            json!([{"key":"@_user_1","id":{"open_id":"ou_receiver"},"name":"Community Assistant"}]),
            true,
        ),
        (
            "foreign-redelivery",
            "ou_actor",
            "app",
            json!([{"key":"@_user_1","id":{"open_id":"ou_receiver"},"name":"Community Assistant"}]),
            false,
        ),
        (
            "other",
            "ou_actor",
            "app",
            json!([{"key":"@_user_1","id":{"open_id":"ou_other"},"name":"Community Assistant"}]),
            false,
        ),
        (
            "wrong-namespace",
            "ou_actor",
            "app",
            json!([{"key":"@_user_1","id":{"user_id":"ou_receiver"},"name":"Community Assistant"}]),
            false,
        ),
        (
            "self",
            "ou_receiver",
            "app",
            json!([{"key":"@_user_1","id":{"open_id":"ou_receiver"},"name":"Community Assistant"}]),
            false,
        ),
        (
            "system",
            "ou_actor",
            "system",
            json!([{"key":"@_user_1","id":{"open_id":"ou_receiver"},"name":"Community Assistant"}]),
            false,
        ),
    ] {
        let before = calls.lock().await.len();
        // Authenticated ingress has stored metadata before signing its callback.
        // A native retry can carry a new NyxID UUID for the same platform message.
        state.db.collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc!{"_id":id,"channel_bot_id":&row.channel_bot_id,"user_id":OWNER,
                "direction":"inbound","platform":"lark","conversation_id":"route",
                "agent_api_key_id":&row.route_api_key_id,"sender_platform_id":sender,
                "platform_conversation_id":"oc_chat","content_type":"text","created_at":bson::DateTime::now(),
                "platform_message_id":if id=="foreign-redelivery" {"foreign"} else {id}})
            .await.unwrap();
        let body = json!({"message_id":id,"correlation_id":format!("jti-{id}"),"platform":"lark",
            "agent":{"api_key_id":row.route_api_key_id,"name":"route"},
            "conversation":{"id":"route","platform_id":"oc_chat","type":"group"},
            "sender":{"platform_id":sender,"display_name":"Ada (owner)"},
            "content":{"type":"text","text":"owner-code"},"thread_id":"omt_topic",
            "raw_platform_data":{"event":{"sender":{"sender_type":sender_kind,"sender_id":{"open_id":sender}},"message":{"mentions":mentions}}}});
        let bytes = serde_json::to_vec(&body).unwrap();
        let token = crate::crypto::jwt::generate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            &format!("jti-{id}"),
            &row.route_api_key_id,
            id,
            "lark",
            &sha256_hex(&bytes),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-nyxid-callback-token", token.parse().unwrap());
        let (_observer, completed) = watch_relay_callback(&row.id, id);
        assert_eq!(
            relay_callback(
                State(state.clone()),
                Path(row.id.clone()),
                headers,
                Bytes::from(bytes)
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        if id == "foreign-redelivery" {
            assert_eq!(
                state
                    .db
                    .collection::<NyxbotEvent>(EVENTS)
                    .count_documents(doc! {"channel_id":&row.id,"event_id":id})
                    .await
                    .unwrap(),
                0,
                "native duplicate returns before reserving a second effect"
            );
        } else {
            tokio::time::timeout(Duration::from_secs(20), completed)
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(
            calls.lock().await.len(),
            before + usize::from(expected),
            "{id}"
        );
        let stored = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(
            stored.link_code_hash, row.link_code_hash,
            "bots cannot redeem link codes"
        );
        assert!(
            !state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"_id":&chat.id})
                .await
                .unwrap()
                .unwrap()
                .owner_seen
        );
        if expected {
            let conversations = channel_conversations(&state, &row.id).await;
            assert_eq!(conversations.len(), 1);
            assert!(conversations[0].guest_turn);
            settle(&state, &conversations[0].id).await;
        }
    }

    // Lark/Feishu card actions identify the authenticated human under
    // event.operator rather than event.sender. Preserve the existing direct
    // callback path instead of classifying the action as Unknown.
    let card_id = "operator-card";
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": card_id, "channel_bot_id": &row.channel_bot_id, "user_id": OWNER,
            "direction": "inbound", "platform": "lark", "conversation_id": "route",
            "agent_api_key_id": &row.route_api_key_id, "sender_platform_id": "ou_operator123",
            "platform_conversation_id": "oc_chat", "content_type": "card_action",
            "platform_message_id": card_id, "reply_to_platform_message_id": "om_card",
            "created_at": bson::DateTime::now()
        })
        .await
        .unwrap();
    let card_body = json!({
        "message_id": card_id,
        "correlation_id": format!("jti-{card_id}"),
        "platform": "lark",
        "agent": {"api_key_id": row.route_api_key_id, "name": "route"},
        "conversation": {"id": "route", "platform_id": "oc_chat", "type": "group"},
        "sender": {"platform_id": "ou_operator123", "display_name": "Alice"},
        "content": {"type": "card_action", "text": "{\"tag\":\"button\",\"value\":{\"button_id\":\"approve\"},\"form_value\":null,\"open_message_id\":\"om_card\"}"},
        "reply_to_platform_message_id": "om_card",
        "raw_platform_data": {
            "schema": "2.0",
            "header": {"event_id": card_id, "event_type": "card.action.trigger", "create_time": "1700000002"},
            "event": {"operator": {"open_id": "ou_operator123", "name": "Alice"},
                "action": {"tag": "button", "value": {"button_id": "approve"}},
                "context": {"open_chat_id": "oc_chat", "chat_type": "group", "open_message_id": "om_card"}}
        }
    });
    let card_bytes = serde_json::to_vec(&card_body).unwrap();
    let card_token = crate::crypto::jwt::generate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        &format!("jti-{card_id}"),
        &row.route_api_key_id,
        card_id,
        "lark",
        &sha256_hex(&card_bytes),
    )
    .unwrap();
    let mut card_headers = HeaderMap::new();
    card_headers.insert("x-nyxid-callback-token", card_token.parse().unwrap());
    let (_card_watch, card_done) = watch_relay_callback(&row.id, card_id);
    assert_eq!(
        relay_callback(
            State(state.clone()),
            Path(row.id.clone()),
            card_headers,
            Bytes::from(card_bytes),
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(Duration::from_secs(20), card_done)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        calls.lock().await.len(),
        2,
        "human card action remains deliverable"
    );

    let malformed_id = "malformed-card";
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": malformed_id, "channel_bot_id": &row.channel_bot_id, "user_id": OWNER,
            "direction": "inbound", "platform": "lark", "conversation_id": "route",
            "agent_api_key_id": &row.route_api_key_id, "sender_platform_id": "ou_spoof",
            "platform_conversation_id": "oc_chat", "content_type": "card_action",
            "platform_message_id": malformed_id, "created_at": bson::DateTime::now()
        })
        .await
        .unwrap();
    let malformed_body = json!({
        "message_id": malformed_id,
        "correlation_id": format!("jti-{malformed_id}"),
        "platform": "lark",
        "agent": {"api_key_id": row.route_api_key_id, "name": "route"},
        "conversation": {"id": "route", "platform_id": "oc_chat", "type": "group"},
        "sender": {"platform_id": "ou_spoof", "display_name": "Spoof"},
        "content": {"type": "card_action", "text": "{\"tag\":\"button\",\"value\":null,\"form_value\":null,\"open_message_id\":null}"},
        "raw_platform_data": {
            "schema": "2.0",
            "header": {"event_id": malformed_id, "event_type": "card.action.trigger"},
            "event": {"operator": {"name": "missing-id"},
                "action": {"tag": "button"},
                "context": {"open_chat_id": "oc_chat", "chat_type": "group"}}
        }
    });
    let malformed_bytes = serde_json::to_vec(&malformed_body).unwrap();
    let malformed_token = crate::crypto::jwt::generate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        &format!("jti-{malformed_id}"),
        &row.route_api_key_id,
        malformed_id,
        "lark",
        &sha256_hex(&malformed_bytes),
    )
    .unwrap();
    let mut malformed_headers = HeaderMap::new();
    malformed_headers.insert("x-nyxid-callback-token", malformed_token.parse().unwrap());
    let (_malformed_watch, malformed_done) = watch_relay_callback(&row.id, malformed_id);
    assert_eq!(
        relay_callback(
            State(state.clone()),
            Path(row.id.clone()),
            malformed_headers,
            Bytes::from(malformed_bytes),
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(Duration::from_secs(20), malformed_done)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        calls.lock().await.len(),
        2,
        "malformed card actor stays closed"
    );
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .remove(&row.channel_bot_id);
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_bot_budget_is_atomic_across_clients_and_never_refunds_or_resets_on_replay() {
    use crate::models::channel_thread::ThreadSenderKind::{Bot, Human, Unknown};
    use crate::services::{
        assistant_settings_service as settings, channel_bot_turn_service as budget,
    };
    fn running_event(channel: &str, event: &str) -> NyxbotEvent {
        let mut row = admission(channel, event);
        row.status = "running".into();
        row
    }
    let (state, _, server) = setup("external_bot_budget_race").await;
    let (row, _) = channel(&state, "gateway").await;
    let chat = chats::record_chat(
        &state,
        &row,
        &chats::group_partition("trusted", None),
        &chats::ChatFacts {
            kind: "group",
            chat_id: "trusted".into(),
            thread_id: None,
            owner: false,
            title: Some("Trusted".into()),
        },
        None,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(doc! {"_id":&chat.id}, doc! {"$set":{"members":"everyone"}})
        .await
        .unwrap();
    let dbname = state.db.name().to_owned();
    let uri = std::env::var("NYXID_TEST_DATABASE_URL").expect("explicit real MongoDB required");
    let barrier = Arc::new(tokio::sync::Barrier::new(24));
    let mut tasks = Vec::new();
    for n in 0..24 {
        let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
        let db = client.database(&dbname);
        let event = running_event(&row.id, &format!("race-{n}"));
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .insert_one(&event)
            .await
            .unwrap();
        let (row, chat, barrier) = (row.clone(), chat.clone(), barrier.clone());
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            budget::admit(&db, &row, &chat, Bot, &event.id).await
        }));
    }
    let mut admitted = 0;
    for task in tasks {
        admitted += usize::from(task.await.unwrap().unwrap());
    }
    assert_eq!(admitted, 6, "all replicas share one chat allowance");
    let stored = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id":&chat.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.native_bot_hops_remaining, Some(0));
    assert_eq!(
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .count_documents(doc! {"channel_id":&row.id})
            .await
            .unwrap(),
        1,
        "bot senders allocate no rows"
    );
    let human = running_event(&row.id, "human-reset");
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&human)
        .await
        .unwrap();
    assert!(
        !budget::admit(&state.db, &row, &chat, Unknown, &human.id)
            .await
            .unwrap()
    );
    assert!(
        budget::admit(&state.db, &row, &chat, Human, &human.id)
            .await
            .unwrap()
    );
    assert!(
        !budget::admit(&state.db, &row, &chat, Human, &human.id)
            .await
            .unwrap(),
        "human replays do not refill again"
    );
    let bot = running_event(&row.id, "after-reset");
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&bot)
        .await
        .unwrap();
    assert!(
        budget::admit(&state.db, &row, &chat, Bot, &bot.id)
            .await
            .unwrap()
    );
    assert!(
        !budget::admit(&state.db, &row, &chat, Bot, &bot.id)
            .await
            .unwrap(),
        "uncertain work is not refunded"
    );
    settings::update(
        &state.db,
        OWNER,
        settings::Update {
            max_group_handoffs_per_hour: Some(7),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let denied = running_event(&row.id, "hourly-denied");
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&denied)
        .await
        .unwrap();
    assert!(
        !budget::admit(&state.db, &row, &chat, Bot, &denied.id)
            .await
            .unwrap(),
        "configured hourly handoff cap is shared"
    );
    let stored = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id":&chat.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.native_bot_hops_remaining,
        Some(5),
        "hourly denial rolls back both spend and claim"
    );
    let human2 = running_event(&row.id, "human-at-hourly-cap");
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&human2)
        .await
        .unwrap();
    assert!(
        budget::admit(&state.db, &row, &chat, Human, &human2.id)
            .await
            .unwrap(),
        "bot hourly exhaustion cannot starve humans"
    );
    settings::update(
        &state.db,
        OWNER,
        settings::Update {
            max_group_handoffs: Some(0),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        !budget::admit(&state.db, &row, &chat, Bot, &denied.id)
            .await
            .unwrap()
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_cma_signed_relay_activity_contract_starts_the_existing_agent_as_guest() {
    for (platform, wire) in [
        (
            "telegram",
            include_str!("fixtures/native_bot_mentions/telegram.json"),
        ),
        (
            "lark",
            include_str!("fixtures/native_bot_mentions/lark.json"),
        ),
        (
            "feishu",
            include_str!("fixtures/native_bot_mentions/feishu.json"),
        ),
    ] {
        let activity: Value = serde_json::from_str(wire).unwrap();
        let (state, calls, server) = setup(&format!("external_cma_wire_{platform}")).await;
        let (mut row, key) = channel(&state, "gateway").await;
        row.platform = platform.into();
        row.gateway_bot_id = Some(
            if platform == "telegram" {
                "77"
            } else {
                "ou_receiver"
            }
            .into(),
        );
        row.gateway_bot_mentions.enabled = true;
        row.gateway_bot_mentions.supported = true;
        row.gateway_bot_mentions.checked_at = Some(Utc::now());
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&row.channel_bot_id},doc! {"$set":{"platform":platform,"platform_bot_id":if platform=="telegram" {"77"} else {"cli_receiver"}}}).await.unwrap();
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER)),
        )
        .await;
        ensure_partition(&state, &key, PARTITION).await;
        let chatid = activity["conversation"]["id"].as_str().unwrap();
        let chat = chats::record_chat(
            &state,
            &row,
            &chats::group_partition(chatid, None),
            &chats::ChatFacts {
                kind: "group",
                chat_id: chatid.into(),
                thread_id: None,
                owner: false,
                title: Some("Authorized group".into()),
            },
            None,
        )
        .await
        .unwrap();
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .update_one(doc! {"_id":&chat.id}, doc! {"$set":{"members":"everyone"}})
            .await
            .unwrap();
        let mut request = event(
            activity["kind"]["text"].as_str().unwrap(),
            activity["actor"]["id"].as_str().unwrap(),
            "wire",
        );
        request["event_context"]["activity"] = activity;
        let text = body_text(respond(&state, &key, &request, "wire-probe").await).await;
        assert!(text.contains("Here is your answer"), "{platform}: {text}");
        assert_eq!(calls.lock().await.len(), 1);
        let conversations = channel_conversations(&state, &row.id).await;
        assert_eq!(conversations.len(), 1);
        assert!(conversations[0].guest_turn);
        server.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn external_gateway_bot_policy_is_negotiated_reconciled_and_acknowledged() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};
    let (state, _, server) = setup("external_bot_rollout").await;
    let (mut row, _) = channel(&state, "gateway").await;
    row.gateway_channel_id = Some("cmaeg1.ch.existing".into());
    row.gateway_version = Some(1);
    row.route_id = Some("existing-route".into());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let fence = chats::gateway_local_cas_filter(&row, 1).expect("active gateway fence");
    assert_eq!(
        fence.get_str("gateway_channel_id").unwrap(),
        "cmaeg1.ch.existing"
    );
    assert_eq!(fence.get_str("route_id").unwrap(), "existing-route");
    assert_eq!(
        fence.get_str("route_api_key_id").unwrap(),
        row.route_api_key_id
    );
    assert!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(fence.clone())
            .await
            .unwrap()
            .is_some()
    );
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"transport": "direct"}},
        )
        .await
        .unwrap();
    assert!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(fence)
            .await
            .unwrap()
            .is_none()
    );
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
            doc! {"$set":{"platform_bot_id":"77","platform_bot_username":"helper_bot"}},
        )
        .await
        .unwrap();
    let gateway = MockServer::start().await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "cmaeg".into();
    service.requires_user_credential = false;
    service.service_category = "internal".into();
    service.base_url = gateway.uri();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    // An old gateway returns 404: retain the human-compatible connection, but
    // report that its independent quiet-policy reconciliation is unavailable.
    assert_eq!(
        chats::sync_gateway_groups(&state, &row, true)
            .await
            .unwrap(),
        Some("gateway_unavailable")
    );
    let old = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(old.status, "active");
    assert!(!old.gateway_bot_mentions.enabled);
    assert!(!old.gateway_no_progress);
    assert!(old.gateway_groups_retry_at.is_some());
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let policy = gateway_policy(
        &state,
        &old,
        &bot,
        &["existing-route".into()],
        GATEWAY_GROUPS_DEFAULT,
    );
    assert!(
        policy["sources"][0]["admission"]
            .get("bot_messages")
            .is_none()
    );
    assert!(policy["sources"][0].get("bot_id").is_none());
    gateway.reset().await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$unset":{"gateway_bot_mentions.checked_at":""}},
        )
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/channel-capabilities"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"source_features":["native_bot_mentions_v1"]})),
        )
        .mount(&gateway)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":2})))
        .mount(&gateway)
        .await;
    // The accepted write is learned through an authenticated read; a missing
    // definition advances the local CAS version without enabling the opt-in.
    Mock::given(method("GET"))
        .and(path("/channels/cmaeg1.ch.existing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":2})))
        .mount(&gateway)
        .await;
    // Capability without an echoed policy must not enable bot ingress.
    assert_eq!(
        chats::sync_gateway_groups(&state, &row, true)
            .await
            .unwrap(),
        Some("gateway_native_bot_mentions_unavailable")
    );
    let refused = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(!refused.gateway_bot_mentions.enabled);
    assert!(!refused.gateway_no_progress);
    assert_eq!(refused.status, "active");
    assert_eq!(refused.gateway_version, Some(2));
    assert!(refused.gateway_groups_retry_at.is_some());
    gateway.reset().await;
    Mock::given(method("PUT"))
        .respond_with(|request: &wiremock::Request| {
            let policy: Value = request.body_json().unwrap();
            assert_eq!(policy["expected_version"], 2);
            assert_eq!(policy["reply"]["ack_after_ms"], 0);
            assert_eq!(policy["sources"][0]["bot_id"], "77");
            assert_eq!(policy["sources"][0]["bot_username"], "helper_bot");
            assert_eq!(policy["sources"][0]["route_ids"], json!(["existing-route"]));
            assert_eq!(
                policy["sources"][0]["admission"]["bot_messages"],
                "native_mentions"
            );
            ResponseTemplate::new(200).set_body_json(json!({"version":3,"definition":policy}))
        })
        .expect(1)
        .mount(&gateway)
        .await;
    assert!(
        chats::sync_gateway_groups(&state, &row, true)
            .await
            .unwrap()
            .is_none()
    );
    let enabled = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(enabled.gateway_bot_mentions.enabled);
    assert!(enabled.gateway_no_progress);
    assert_eq!(enabled.gateway_version, Some(3));
    assert_eq!(enabled.agent_api_key_id, row.agent_api_key_id);
    assert_eq!(enabled.route_api_key_id, row.route_api_key_id);
    assert!(enabled.gateway_groups_retry_at.is_none());
    // Stable acknowledged policy needs no further management or ingress I/O.
    assert!(
        chats::sync_gateway_groups(&state, &enabled, false)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(gateway.received_requests().await.unwrap().len(), 1);
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_gateway_conflict_recovery_requires_a_trusted_positive_echo() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

    let (state, _, server) = setup("external_gateway_conflict_recovery").await;
    let gateway = MockServer::start().await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "cmaeg".into();
    service.requires_user_credential = false;
    service.service_category = "internal".into();
    service.base_url = gateway.uri();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/channel-capabilities"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"source_features":["native_bot_mentions_v1"]})),
        )
        .mount(&gateway)
        .await;
    Mock::given(method("PUT"))
        .respond_with(|request: &wiremock::Request| {
            let policy: Value = request.body_json().unwrap();
            assert_eq!(policy["expected_version"], 1);
            ResponseTemplate::new(409)
        })
        .expect(7)
        .mount(&gateway)
        .await;

    let cases = [
        ("matching", Some(2_i64), None),
        ("wrong-identity", Some(3_i64), Some("wrong_identity")),
        ("wrong-admission", Some(4_i64), Some("wrong_admission")),
        ("multiple-sources", Some(5_i64), Some("multiple_sources")),
        ("normalized", Some(6_i64), Some("trust_normalized")),
        ("missing-version", None, None),
        ("negative-version", Some(-1_i64), None),
    ];
    for (name, version, refusal) in cases {
        let (mut row, _) = channel(&state, "gateway").await;
        row.gateway_channel_id = Some(format!("cmaeg1.ch.{name}"));
        row.gateway_version = Some(1);
        row.route_id = Some(format!("route-{name}"));
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id": &row.id}, &row)
            .await
            .unwrap();
        let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
            .await
            .unwrap();
        let mut authored = row.clone();
        authored.gateway_bot_mentions.supported = true;
        let requested = gateway_policy(
            &state,
            &authored,
            &bot,
            &[row.route_id.clone().unwrap()],
            GATEWAY_GROUPS_DEFAULT,
        );
        let definition = match refusal {
            Some("wrong_identity") => {
                let mut changed = requested.clone();
                changed["sources"][0]["bot_id"] = json!("999");
                changed
            }
            Some("wrong_admission") => {
                let mut changed = requested.clone();
                changed["sources"][0]["admission"]["bot_messages"] = json!("all");
                changed
            }
            Some("multiple_sources") => {
                let mut changed = requested.clone();
                let source = changed["sources"][0].clone();
                changed["sources"] = json!([source.clone(), source]);
                changed
            }
            Some("trust_normalized") => {
                let mut changed = requested.clone();
                changed["sources"][0]["trust_normalized"] = json!(true);
                changed
            }
            _ => requested,
        };
        let mut fresh = json!({"definition": definition});
        if let Some(version) = version {
            fresh["version"] = json!(version);
        }
        Mock::given(method("GET"))
            .and(path(format!("/channels/cmaeg1.ch.{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(fresh))
            .mount(&gateway)
            .await;

        let result = chats::sync_gateway_groups(&state, &row, true)
            .await
            .unwrap();
        let current = load_channel(&state, OWNER, &row.id).await.unwrap();
        if name == "matching" {
            assert_eq!(result, None);
            assert!(current.gateway_bot_mentions.enabled);
            assert_eq!(current.gateway_version, Some(2));
        } else {
            assert!(result.is_some(), "unsafe echo must remain refused: {name}");
            assert!(!current.gateway_bot_mentions.enabled);
            assert_ne!(current.gateway_version, Some(0));
            if version.is_some_and(|value| value > 0) {
                assert_eq!(current.gateway_version, version);
            } else {
                assert_eq!(current.gateway_version, Some(1));
            }
        }
    }
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_lark_human_mentions_require_receiver_identity_but_all_mode_still_works() {
    let (state, calls, server) = setup("external_human_lark_identity").await;
    let (mut row, key) = channel(&state, "gateway").await;
    row.platform = "lark".into();
    row.gateway_bot_id = None;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    put_binding(
        State(state.clone()),
        Path("bnd_chats".into()),
        bearer(&key),
        Json(binding_body(&key, OWNER)),
    )
    .await;
    ensure_partition(&state, &key, PARTITION).await;
    let chat = chats::record_chat(
        &state,
        &row,
        &chats::group_partition("-100200", None),
        &chats::ChatFacts {
            kind: "group",
            chat_id: "-100200".into(),
            thread_id: None,
            owner: false,
            title: None,
        },
        None,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(doc! {"_id":&chat.id}, doc! {"$set":{"members":"everyone"}})
        .await
        .unwrap();
    let message = group_event(
        "@other hello",
        "ou_guest",
        "Human",
        "missing-id",
        PARTITION,
        true,
    );
    let response = body_text(respond(&state, &key, &message, "missing-id").await).await;
    assert!(!response.contains("Here is your answer"), "{response}");
    assert!(calls.lock().await.is_empty());
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(doc! {"_id":&chat.id}, doc! {"$set":{"reply_mode":"all"}})
        .await
        .unwrap();
    let message = group_event(
        "hello",
        "ou_guest",
        "Human",
        "explicit-all",
        PARTITION,
        false,
    );
    assert!(
        body_text(respond(&state, &key, &message, "explicit-all").await)
            .await
            .contains("Here is your answer")
    );
    assert_eq!(calls.lock().await.len(), 1);
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_legacy_gateway_recovers_lark_identity_only_after_policy_echo() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};
    let (state, _, server) = setup("external_identity_recovery").await;
    let (mut row, _) = channel(&state, "gateway").await;
    row.platform = "lark".into();
    row.gateway_bot_id = None;
    row.gateway_channel_id = Some("cmaeg1.ch.existing".into());
    row.gateway_version = Some(1);
    row.route_id = Some("existing-route".into());
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
            doc! {"$set":{"platform":"lark","platform_bot_id":"cli_app"}},
        )
        .await
        .unwrap();
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), "ou_recovered".into());
    let gateway = MockServer::start().await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "cmaeg".into();
    service.base_url = gateway.uri();
    service.requires_user_credential = false;
    service.service_category = "internal".into();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":2})))
        .mount(&gateway)
        .await;
    Mock::given(method("GET"))
        .and(path("/channels/cmaeg1.ch.existing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":2})))
        .mount(&gateway)
        .await;
    assert_eq!(
        chats::sync_gateway_groups(&state, &row, true)
            .await
            .unwrap(),
        Some("gateway_receiving_identity_unavailable")
    );
    let refused = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(
        refused.gateway_bot_id.is_none(),
        "unacknowledged identity must not make legacy mentions exact"
    );
    gateway.reset().await;
    // Allow the management identity retry, with an old gateway still returning404 for capabilities.
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$unset":{"gateway_bot_mentions.checked_at":""}},
        )
        .await
        .unwrap();
    Mock::given(method("PUT"))
        .respond_with(|request: &wiremock::Request| {
            let policy: Value = request.body_json().unwrap();
            assert_eq!(policy["expected_version"], 2);
            assert_eq!(policy["sources"][0]["bot_id"], "ou_recovered");
            assert!(
                policy["sources"][0]["admission"]
                    .get("bot_messages")
                    .is_none()
            );
            ResponseTemplate::new(200).set_body_json(json!({"version":3,"definition":policy}))
        })
        .mount(&gateway)
        .await;
    assert!(
        chats::sync_gateway_groups(&state, &row, true)
            .await
            .unwrap()
            .is_none()
    );
    let recovered = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(recovered.gateway_bot_id.as_deref(), Some("ou_recovered"));
    assert!(!recovered.gateway_bot_mentions.enabled);
    assert_eq!(recovered.agent_api_key_id, row.agent_api_key_id);
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .remove(&row.channel_bot_id);
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_native_bot_redelivery_with_a_new_relay_uuid_has_one_turn() {
    native_redelivery_has_one_turn("bot").await;
}

#[tokio::test]
async fn external_native_human_redelivery_cannot_refill_bot_hops_or_run_twice() {
    native_redelivery_has_one_turn("human").await;
}

#[tokio::test]
async fn external_signed_telegram_edit_keeps_message_id_distinct_from_redelivery() {
    let (state, calls, server) = setup("external_telegram_edit_identity").await;
    let (mut row, _key) = channel(&state, "direct").await;
    row.platform = "telegram".into();
    row.route_id = Some("route-telegram-edit".into());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &row.id}, &row)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.channel_bot_id},
            doc! {"$set": {"platform": "telegram", "platform_bot_id": "123", "platform_bot_username": "helper_bot"}},
        )
        .await
        .unwrap();
    let chat = chats::record_chat(
        &state,
        &row,
        &chats::group_partition("-100", None),
        &chats::ChatFacts {
            kind: "group",
            chat_id: "-100".into(),
            thread_id: None,
            owner: false,
            title: Some("Edit identity group".into()),
        },
        None,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"_id": &chat.id},
            doc! {"$set": {"members": "everyone", "reply_mode": "all"}},
        )
        .await
        .unwrap();

    async fn deliver(
        state: &AppState,
        row: &NyxbotChannel,
        relay_id: &str,
        update_id: i64,
        edited: bool,
        wait_for_completion: bool,
    ) {
        let message = json!({
            "message_id": 77,
            "from": {"id": 9009, "is_bot": false, "first_name": "Ada"},
            "chat": {"id": -100, "type": "supergroup"},
            "text": "@helper_bot hello",
            "entities": [{"type": "mention", "offset": 0, "length": 11}]
        });
        let raw = if edited {
            json!({"update_id": update_id, "edited_message": message})
        } else {
            json!({"update_id": update_id, "message": message})
        };
        state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": relay_id, "channel_bot_id": &row.channel_bot_id,
                "user_id": OWNER, "direction": "inbound", "platform": "telegram",
                "conversation_id": &row.route_id, "agent_api_key_id": &row.route_api_key_id,
                "sender_platform_id": "9009", "platform_conversation_id": "-100",
                "platform_message_id": "77", "content_type": "text",
                "native_event_id": update_id.to_string(),
                "created_at": bson::DateTime::now()
            })
            .await
            .unwrap();
        let body = json!({
            "message_id": relay_id,
            "correlation_id": format!("jti-{relay_id}"),
            "platform": "telegram",
            "agent": {"api_key_id": row.route_api_key_id, "name": "route"},
            "conversation": {"id": row.route_id, "platform_id": "-100", "type": "group"},
            "sender": {"platform_id": "9009", "display_name": "Foreign"},
            "content": {"type": "text", "text": "@helper_bot hello"},
            "raw_platform_data": raw
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let token = crate::crypto::jwt::generate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            &format!("jti-{relay_id}"),
            &row.route_api_key_id,
            relay_id,
            "telegram",
            &sha256_hex(&bytes),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-nyxid-callback-token", token.parse().unwrap());
        let completion = wait_for_completion.then(|| watch_relay_callback(&row.id, relay_id));
        assert_eq!(
            relay_callback(
                State(state.clone()),
                Path(row.id.clone()),
                headers,
                Bytes::from(bytes),
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        if let Some((_watcher, done)) = completion {
            tokio::time::timeout(Duration::from_secs(20), done)
                .await
                .unwrap()
                .unwrap();
        }
    }

    // Telegram edits retain message_id 77, but authenticated update_id 101
    // makes the edit a new event. A fresh relay UUID for the same edit must
    // collide with the persisted update identity and produce no third turn.
    deliver(&state, &row, "relay-original", 100, false, true).await;
    assert_eq!(calls.lock().await.len(), 1);
    deliver(&state, &row, "relay-edit", 101, true, true).await;
    assert_eq!(calls.lock().await.len(), 2);
    deliver(&state, &row, "relay-edit-redelivery", 101, true, false).await;
    assert_eq!(calls.lock().await.len(), 2);
    assert_eq!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {"channel_id": &row.id})
            .await
            .unwrap(),
        2,
        "original and edit are distinct; duplicate edit is one event"
    );
    server.abort();
    state.db.drop().await.unwrap();
}

async fn native_redelivery_has_one_turn(actor: &str) {
    let (state, calls, server) = setup(&format!("external_native_redelivery_{actor}")).await;
    let (mut row, key) = channel(&state, "gateway").await;
    row.route_id = Some("route-1".into());
    row.gateway_bot_mentions.supported = true;
    row.gateway_bot_mentions.enabled = true;
    row.gateway_bot_mentions.checked_at = Some(Utc::now());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    put_binding(
        State(state.clone()),
        Path("bnd_chats".into()),
        bearer(&key),
        Json(binding_body(&key, OWNER)),
    )
    .await;
    ensure_partition(&state, &key, PARTITION).await;
    let chat = chats::record_chat(
        &state,
        &row,
        &chats::group_partition("-100200", Some("42")),
        &chats::ChatFacts {
            kind: "group",
            chat_id: "-100200".into(),
            thread_id: Some("42".into()),
            owner: false,
            title: Some("Enabled group".into()),
        },
        None,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(doc! {"_id":&chat.id}, doc! {"$set":{"members":"everyone"}})
        .await
        .unwrap();
    for n in 0..2 {
        let relay = Uuid::new_v4().to_string();
        state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc! {"_id":&relay,"channel_bot_id":&row.channel_bot_id,
            "user_id":OWNER,"direction":"inbound","platform":&row.platform,
            "conversation_id":&row.route_id,"agent_api_key_id":&row.route_api_key_id,
            "sender_platform_id":"9009","platform_conversation_id":"-100200",
            "platform_message_id":"107", "native_event_id":"107"})
            .await
            .unwrap();
        let mut request = group_event(
            "@helper_bot hello",
            "9009",
            "Foreign bot",
            &relay,
            PARTITION,
            true,
        );
        request["event_context"]["activity"]["actor"]["kind"] = json!(actor);
        request["event_context"]["activity"]["conversation"]["thread_id"] = json!("42");
        let output =
            body_text(respond(&state, &key, &request, &format!("delivery-{n}")).await).await;
        assert_eq!(
            output.contains("Here is your answer"),
            n == 0,
            "native {actor} redelivery {n}: {output}"
        );
        let conversations = channel_conversations(&state, &row.id).await;
        settle(&state, &conversations[0].id).await;
        if n == 0 {
            // Represent other admitted automation spending the allowance
            // between the first delivery and a provider redelivery.
            state
                .db
                .collection::<NyxbotThread>(THREADS)
                .update_one(
                    doc! {"_id": &chat.id},
                    doc! {"$set": {"native_bot_hops_remaining": 2}},
                )
                .await
                .unwrap();
        }
    }
    assert_eq!(
        calls.lock().await.len(),
        1,
        "one native message has one effect"
    );
    assert_eq!(
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"_id": &chat.id})
            .await
            .unwrap()
            .unwrap()
            .native_bot_hops_remaining,
        Some(2),
        "a provider redelivery cannot reset or spend again"
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn previously_acknowledged_quiet_policy_is_revoked_by_unconfirmed_replacement() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};
    for status in [200, 409] {
        for ack in [Some(1000_u64), None] {
            let (state, _, server) = setup("external_quiet_proof_replacement").await;
            let (mut row, _) = channel(&state, "gateway").await;
            row.gateway_channel_id = Some("cmaeg1.ch.quiet".into());
            row.gateway_version = Some(1);
            row.gateway_no_progress = true;
            row.gateway_groups = Some("all".into()); // desired policy has changed
            row.gateway_bot_mentions.checked_at = Some(Utc::now());
            row.gateway_threads.checked_at = Some(Utc::now());
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .replace_one(doc! {"_id": &row.id}, &row)
                .await
                .unwrap();
            let gateway = MockServer::start().await;
            let mut service = crate::models::downstream_service::test_helpers::dummy_service();
            service.id = Uuid::new_v4().to_string();
            service.slug = GATEWAY_SLUG.into();
            service.requires_user_credential = false;
            service.service_category = "internal".into();
            service.base_url = gateway.uri();
            state
                .db
                .collection::<DownstreamService>(SERVICES)
                .insert_one(service)
                .await
                .unwrap();
            let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
                .await
                .unwrap();
            let mut unsafe_policy = gateway_policy(
                &state,
                &row,
                &bot,
                &[row.route_id.clone().unwrap()],
                GATEWAY_GROUPS_DEFAULT,
            );
            match ack {
                Some(value) => unsafe_policy["reply"]["ack_after_ms"] = json!(value),
                None => {
                    unsafe_policy["reply"]
                        .as_object_mut()
                        .unwrap()
                        .remove("ack_after_ms");
                }
            }
            let fresh = json!({"version": 2, "definition": unsafe_policy});
            Mock::given(method("PUT"))
                .respond_with(ResponseTemplate::new(status).set_body_json(fresh.clone()))
                .expect(1)
                .mount(&gateway)
                .await;
            Mock::given(method("GET"))
                .and(path("/channels/cmaeg1.ch.quiet"))
                .respond_with(ResponseTemplate::new(200).set_body_json(fresh))
                .expect(1)
                .mount(&gateway)
                .await;
            assert!(
                chats::sync_gateway_groups(&state, &row, true)
                    .await
                    .unwrap()
                    .is_some()
            );
            let unconfirmed = load_channel(&state, OWNER, &row.id).await.unwrap();
            assert!(
                !unconfirmed.gateway_no_progress,
                "status={status}, ack={ack:?}"
            );
            assert_eq!(unconfirmed.gateway_version, Some(2));
            gateway.verify().await;
            gateway.reset().await;
            // Missing proof must prevent the early return even if other policy
            // fields happen to match. A trustworthy echoed replacement restores it.
            Mock::given(method("PUT"))
                .respond_with(|request: &wiremock::Request| {
                    let policy: Value = request.body_json().unwrap();
                    assert_eq!(policy["expected_version"], 2);
                    assert_eq!(policy["reply"]["ack_after_ms"], 0);
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"version":3, "definition":policy}))
                })
                .expect(1)
                .mount(&gateway)
                .await;
            assert_eq!(
                chats::sync_gateway_groups(&state, &unconfirmed, true)
                    .await
                    .unwrap(),
                None
            );
            assert!(
                load_channel(&state, OWNER, &row.id)
                    .await
                    .unwrap()
                    .gateway_no_progress
            );
            server.abort();
            state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn binding_quiet_proof_is_idempotent_but_resets_on_replacement_and_reports_write_failure() {
    let (state, _, server) = setup("external_binding_quiet_proof").await;
    let (mut row, key) = channel(&state, "gateway").await;
    row.binding_id = Some("quiet-binding".into());
    row.gateway_no_progress = true;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &row.id}, &row)
        .await
        .unwrap();
    let put = || {
        put_binding(
            State(state.clone()),
            Path("quiet-binding".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER)),
        )
    };
    assert_eq!(put().await.status(), StatusCode::OK);
    assert!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_no_progress
    );
    // A failed delete is not acknowledged and does not remove the old binding.
    state
        .db
        .run_command(doc! {"collMod": CHANNELS, "validationLevel": "strict",
        "validator": {"binding_id": "quiet-binding"}})
        .await
        .unwrap();
    assert_eq!(
        delete_binding(
            State(state.clone()),
            Path("quiet-binding".into()),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .binding_id
            .as_deref(),
        Some("quiet-binding")
    );
    state
        .db
        .run_command(doc! {"collMod": CHANNELS, "validator": {}})
        .await
        .unwrap();
    assert_eq!(
        delete_binding(
            State(state.clone()),
            Path("quiet-binding".into()),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(
        !load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_no_progress
    );
    state
        .db
        .run_command(doc! {"collMod": CHANNELS, "validationLevel": "strict",
        "validator": {"binding_id": bson::Bson::Null}})
        .await
        .unwrap();
    assert_eq!(put().await.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .binding_id
            .is_none()
    );
    state
        .db
        .run_command(doc! {"collMod": CHANNELS, "validator": {}})
        .await
        .unwrap();
    assert_eq!(put().await.status(), StatusCode::OK);
    assert!(
        !load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_no_progress
    );
    server.abort();
    state.db.drop().await.unwrap();
}
