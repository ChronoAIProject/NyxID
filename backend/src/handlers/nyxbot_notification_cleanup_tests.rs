use super::*;
use crate::test_utils::{assert_notification_probe, seed_notification_purge_probe};

#[tokio::test]
async fn owner_private_channel_replies_are_chat_updates_on_the_shared_home_thread() {
    use crate::{
        models::assistant_notification::{AssistantNotification, NotificationKind, SourceType},
        services::{assistant_notification_service as feed, feature_flag_service as flags},
    };
    let (state, _, server) = Box::pin(setup("notification_owner_private_chat")).await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    feed::ensure_indexes(&state.db).await.unwrap();
    flags::set_platform_override(
        &state.db,
        feed::FLAG_KEY,
        &flags::FlagTarget::User(OWNER.into()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    feed::reset_flag_cache(&state.db);
    let (connection, key) = Box::pin(channel(&state, "gateway")).await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER)),
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_partition(&state, &key, PARTITION).await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&connection.id},
            doc! {"$set":{"owner_sender_ids":["7"]}},
        )
        .await
        .unwrap();
    let notifications = state.db.collection::<AssistantNotification>(
        crate::models::assistant_notification::COLLECTION_NAME,
    );
    for (event_id, count) in [("private-feed-first", 1), ("private-feed-second", 2)] {
        let incoming = event("What changed?", "7", event_id);
        let answer = body_text(Box::pin(respond(&state, &key, &incoming, event_id)).await).await;
        assert!(answer.contains("Here is your answer"), "{answer}");
        let rows: Vec<_> = notifications
            .find(doc! {"user_id":OWNER})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, NotificationKind::ChatUpdate);
        assert_eq!(rows[0].source.source_type, SourceType::ChannelChat);
        assert_eq!(
            rows[0].source.channel_id.as_deref(),
            Some(connection.id.as_str())
        );
        assert_eq!(rows[0].count, count);
        let conversation = engine::get(&state.db, OWNER, &rows[0].source.id)
            .await
            .unwrap();
        assert!(conversation.channel.is_none());
        assert_eq!(
            conversation
                .reply_channel
                .as_ref()
                .unwrap()
                .nyxbot_channel_id,
            connection.id
        );
        assert_eq!(
            rows[0].source.conversation_id.as_deref(),
            Some(conversation.id.as_str())
        );
        let replay = body_text(Box::pin(respond(&state, &key, &incoming, event_id)).await).await;
        assert!(replay.contains("response.completed"));
        assert_eq!(
            notifications
                .find_one(doc! {"_id":&rows[0].id})
                .await
                .unwrap()
                .unwrap()
                .count,
            count
        );
        let listed = feed::list(&state, OWNER, &feed::ListQuery::default())
            .await
            .unwrap();
        assert_eq!(listed.items.len(), 1);
        assert_eq!(listed.items[0].preview, "Here is your answer");
        assert!(
            listed.items[0]
                .url
                .ends_with(&format!("/assistant?c={}", conversation.id))
        );
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn provider_binding_detach_preserves_pending_decision_until_owner_decides() {
    use crate::services::{
        assistant_acknowledgement_service as acks, feature_flag_service as flags,
    };
    let (state, _, server) = Box::pin(setup("notification_binding_card")).await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    crate::services::assistant_notification_service::ensure_indexes(&state.db)
        .await
        .unwrap();
    flags::set_platform_override(
        &state.db,
        flags::ASSISTANT_NOTIFICATIONS_FLAG_KEY,
        &flags::FlagTarget::User(OWNER.into()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    let (row, key) = Box::pin(channel(&state, "gateway")).await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_confirm".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER)),
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        put_conversation(
            State(state.clone()),
            Path(("bnd_confirm".into(), PARTITION.into())),
            bearer(&key),
        )
        .await
        .status(),
        StatusCode::OK
    );
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"owner_sender_ids":["7"]}},
        )
        .await
        .unwrap();
    assert_eq!(
        Box::pin(respond(
            &state,
            &key,
            &event("Delete an old key", "7", "binding-card"),
            "binding-card",
        ))
        .await
        .status(),
        StatusCode::OK
    );
    let conversation = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"user_id":OWNER,"reply_channel.nyxbot_channel_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    let chat = acks::for_key(&state.db, OWNER, Some(&conversation.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let (card, created) = Box::pin(acks::request_tracked(
        &state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__delete_agent_key"),
            arguments: Some(&json!({"key_id":"old-key"})),
            summary: "Delete an old agent key",
            platform: false,
        },
    ))
    .await
    .unwrap();
    assert!(created);
    assert_eq!(card.status, "pending");
    let notifications = state
        .db
        .collection::<bson::Document>(crate::models::assistant_notification::COLLECTION_NAME);
    let filter = doc! {"user_id":OWNER,"source.card_id":&card.id};
    let pending = notifications
        .find_one(filter.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.get_str("kind").unwrap(), "needs_decision");
    assert_eq!(
        pending
            .get_document("source")
            .unwrap()
            .get_str("channel_id")
            .unwrap(),
        row.id
    );
    assert!(matches!(pending.get("resolved_at"), Some(bson::Bson::Null)));
    assert_eq!(
        delete_binding(
            State(state.clone()),
            Path("bnd_confirm".into()),
            bearer(&key),
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let retained = notifications
        .find_one(filter.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.get("_id"), pending.get("_id"));
    assert!(matches!(
        retained.get("resolved_at"),
        Some(bson::Bson::Null)
    ));
    // The owner can decide from the retained NyxID history while CMA rebinds.
    Box::pin(acks::decide(
        &state.db,
        OWNER,
        &conversation.id,
        &card.id,
        false,
    ))
    .await
    .unwrap();
    let resolved = notifications.find_one(filter).await.unwrap().unwrap();
    assert!(resolved.get_datetime("resolved_at").is_ok());
    assert!(!resolved.get_bool("unread").unwrap());
}

#[tokio::test]
async fn provider_thread_delete_purges_exact_chat_and_lanes_but_retains_history() {
    for native in [false, true] {
        let (state, _, server) = Box::pin(setup("notification_provider_thread_purge")).await;
        let _server = tokio_util::task::AbortOnDropHandle::new(server);
        let (row, key) = if native {
            native_tests::native_channel(&state).await
        } else {
            let (row, key) = Box::pin(channel(&state, "gateway")).await;
            assert_eq!(
                put_binding(
                    State(state.clone()),
                    Path("bnd_chats".into()),
                    bearer(&key),
                    Json(binding_body(&key, OWNER))
                )
                .await
                .status(),
                StatusCode::OK
            );
            ensure_partition(&state, &key, PARTITION).await;
            (load_channel(&state, OWNER, &row.id).await.unwrap(), key)
        };
        let history = Box::pin(engine::begin_turn(
            &state.db,
            OWNER,
            &engine::TurnRequest {
                acknowledgement_id: None,
                attachment_ids: vec![],
                agent_id: None,
                conversation_id: None,
                text: "Retained assistant history".into(),
                model: None,
                access_mode: None,
                retry_of: None,
            },
            &state.encryption_keys,
        ))
        .await
        .unwrap();
        let other_partition = "conv_fedcba9876543210fedcba9876543210";
        ensure_partition(&state, &key, other_partition).await;
        let thread = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one_and_update(
                doc! {"channel_id":&row.id,"partition":PARTITION},
                doc! {"$set":{"conversation_id":&history.id}},
            )
            .return_document(mongodb::options::ReturnDocument::After)
            .await
            .unwrap()
            .unwrap();
        let other_thread = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"channel_id":&row.id,"partition":other_partition})
            .await
            .unwrap()
            .unwrap();
        let lane_conversation = Uuid::new_v4().to_string();
        state
            .db
            .collection::<bson::Document>(crate::models::org_channel::LANES)
            .insert_one(doc! {
                "_id":Uuid::new_v4().to_string(),"channel_id":&row.id,"partition":PARTITION,
                "conversation_id":&lane_conversation,
            })
            .await
            .unwrap();
        let removed = [
            seed_notification_purge_probe(
                &state.db,
                OWNER,
                doc! {"channel_id":&row.id,"conversation_id":&history.id},
            )
            .await,
            seed_notification_purge_probe(
                &state.db,
                OWNER,
                doc! {"channel_id":&row.id,"conversation_id":lane_conversation},
            )
            .await,
            seed_notification_purge_probe(
                &state.db,
                OWNER,
                doc! {"channel_id":&row.id,"thread_id":&thread.id},
            )
            .await,
        ];
        let retained = seed_notification_purge_probe(
            &state.db,
            OWNER,
            doc! {"channel_id":&row.id,"thread_id":&other_thread.id},
        )
        .await;
        let permissions = state.db.collection::<bson::Document>(
            crate::models::execution_permission_channel::COLLECTION_NAME,
        );
        let permission_id = Uuid::new_v4().to_string();
        permissions
            .insert_one(
                doc! {"_id":&permission_id,"task":{"channel_source":{"thread_id":&thread.id}}},
            )
            .await
            .unwrap();
        let responses = state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME);
        let response_id = Uuid::new_v4().to_string();
        let other_response_id = Uuid::new_v4().to_string();
        if native {
            responses.insert_many([
                doc! {"_id":&response_id,"response_id":format!("resp_{response_id}"),"channel_id":&row.id,"partition":PARTITION},
                doc! {"_id":&other_response_id,"response_id":format!("resp_{other_response_id}"),"channel_id":&row.id,"partition":other_partition},
            ]).await.unwrap();
        }
        for _ in 0..2 {
            assert_eq!(
                Box::pin(delete_conversation(
                    State(state.clone()),
                    Path(("bnd_chats".into(), PARTITION.into())),
                    bearer(&key)
                ))
                .await
                .status(),
                StatusCode::NO_CONTENT
            );
            for id in &removed {
                assert_notification_probe(&state.db, id, false).await;
            }
            assert_notification_probe(&state.db, &retained, true).await;
        }
        assert!(
            state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"_id":&thread.id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"_id":&other_thread.id})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .db
                .collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(doc! {"_id":&history.id})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            permissions
                .find_one(doc! {"_id":permission_id})
                .await
                .unwrap()
                .is_none()
        );
        if native {
            assert!(
                responses
                    .find_one(doc! {"_id":response_id})
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                responses
                    .find_one(doc! {"_id":other_response_id})
                    .await
                    .unwrap()
                    .is_some()
            );
        }
    }
}

#[tokio::test]
async fn provider_thread_delete_survives_unrelated_channel_key_rotation() {
    let (state, _, server) = Box::pin(setup("notification_thread_rotation")).await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (row, key) = Box::pin(channel(&state, "gateway")).await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER)),
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_partition(&state, &key, PARTITION).await;
    let observed = load_channel(&state, OWNER, &row.id).await.unwrap();
    let thread = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&row.id,"partition":PARTITION})
        .await
        .unwrap()
        .unwrap();
    let notification = seed_notification_purge_probe(
        &state.db,
        OWNER,
        doc! {"channel_id":&row.id,"thread_id":&thread.id},
    )
    .await;
    // The gateway authenticated before a concurrent key swap was staged.
    // Only the binding identity governs which provider chat it may delete.
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(
        doc! {"_id":&row.id},
        doc! {"$set":{"pending_agent_api_key_id":Uuid::new_v4().to_string(),"status":"pending"}},
    ).await.unwrap();
    crate::services::nyxbot_chat_settings_service::remove_thread(&state.db, &observed, PARTITION)
        .await
        .unwrap();
    assert!(
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"_id":thread.id})
            .await
            .unwrap()
            .is_none()
    );
    assert_notification_probe(&state.db, &notification, false).await;
}

#[tokio::test]
async fn stale_provider_binding_cannot_purge_current_notifications() {
    let (state, _, server) = Box::pin(setup("notification_stale_binding_purge")).await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (row, key) = Box::pin(channel(&state, "gateway")).await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    let row = load_channel(&state, OWNER, &row.id).await.unwrap();
    let retained =
        seed_notification_purge_probe(&state.db, OWNER, doc! {"channel_id":&row.id}).await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"binding_id":"bnd_successor"}},
        )
        .await
        .unwrap();
    assert!(
        !crate::services::nyxbot_chat_settings_service::remove_binding(
            &state.db,
            &row,
            "bnd_chats"
        )
        .await
        .unwrap()
    );
    assert!(matches!(
        crate::services::nyxbot_chat_settings_service::remove_thread(&state.db, &row, PARTITION)
            .await,
        Err(AppError::Conflict(_))
    ));
    assert_notification_probe(&state.db, &retained, true).await;
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .binding_id
            .as_deref(),
        Some("bnd_successor")
    );
}

async fn create_notification_chat(
    state: &AppState,
    row: &NyxbotChannel,
    key: &str,
    partition: &str,
    destination: &NyxbotThread,
    path: usize,
) -> NyxbotThread {
    match path {
        0 => thread_conversation(state, row, partition).await.unwrap().0,
        1 => {
            assert_eq!(
                put_conversation(
                    State(state.clone()),
                    Path(("bnd_chats".into(), partition.into())),
                    bearer(key),
                )
                .await
                .status(),
                StatusCode::OK,
            );
            state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"channel_id": &row.id, "partition": partition})
                .await
                .unwrap()
                .unwrap()
        }
        2 => chats::record_chat(
            state,
            row,
            partition,
            &chats::ChatFacts {
                kind: "private",
                chat_id: partition.into(),
                thread_id: None,
                owner: false,
                title: Some("New chat".into()),
            },
            None,
        )
        .await
        .unwrap(),
        3 => custom_event_thread(state, row, partition, destination)
            .await
            .unwrap(),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn notification_new_chats_use_atomic_upserts_without_channel_writes_or_transactions() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let (mut state, _, server) = Box::pin(setup("notification_new_chat_no_fanout")).await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (connection, key) = Box::pin(channel(&state, "gateway")).await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    let connection = load_channel(&state, OWNER, &connection.id).await.unwrap();
    let destination = thread_conversation(&state, &connection, PARTITION)
        .await
        .unwrap()
        .0;
    assert!(
        !crate::services::assistant_notification_service::enabled(&state.db, OWNER)
            .await
            .unwrap()
    );
    // Monitor the same test-owned database without repeating setup or touching
    // any other session's databases on the shared replica set.
    let commands = Arc::new(std::sync::Mutex::new(Vec::<bson::Document>::new()));
    let captured = commands.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            captured.lock().unwrap().push(event.command);
        }
    }));
    state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(state.db.name());
    let channels = state.db.collection::<bson::Document>(CHANNELS);
    let before = channels
        .find_one(doc! {"_id": &connection.id})
        .await
        .unwrap()
        .unwrap();
    commands.lock().unwrap().clear();
    for path in 0..4 {
        let partition = format!("conv_{}", Uuid::new_v4().simple());
        let thread =
            create_notification_chat(&state, &connection, &key, &partition, &destination, path)
                .await;
        let repeated =
            create_notification_chat(&state, &connection, &key, &partition, &destination, path)
                .await;
        assert_eq!(repeated.id, thread.id);
        assert_eq!(
            channels
                .find_one(doc! {"_id": &connection.id})
                .await
                .unwrap()
                .unwrap(),
            before,
            "creation path {path} must leave the shared channel unchanged"
        );
    }
    let observed = commands.lock().unwrap().clone();
    assert!(!observed.is_empty());
    for command in observed {
        assert!(
            !command.contains_key("autocommit") && !command.contains_key("startTransaction"),
            "chat admission must not start a transaction: {command:?}"
        );
        assert_ne!(command.get_str("update").ok(), Some(CHANNELS));
        assert_ne!(command.get_str("findAndModify").ok(), Some(CHANNELS));
    }
    // Retirement must be independent of retained chat count. All plain chat
    // documents, including existing notification fences, stay byte-identical.
    let threads = state.db.collection::<bson::Document>(THREADS);
    let now = bson::DateTime::now();
    threads.insert_many((0..1000).map(|index| doc! {
        "_id": Uuid::new_v4().to_string(), "channel_id": &connection.id,
        "user_id": OWNER, "partition": format!("retained-{index}"), "title": "Retained",
        "kind": "private", "notification_fence": i64::from(index), "created_at": now, "updated_at": now,
    })).await.unwrap();
    let before: Vec<_> = threads
        .find(doc! {"channel_id": &connection.id})
        .sort(doc! {"_id": 1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let probe =
        seed_notification_purge_probe(&state.db, OWNER, doc! {"channel_id": &connection.id}).await;
    releases::retire_connection(
        &state,
        &connection,
        ReleaseReason::Disconnect,
        "disconnected",
        None,
    )
    .await
    .unwrap();
    let after: Vec<_> = threads
        .find(doc! {"channel_id": &connection.id})
        .sort(doc! {"_id": 1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "retirement must not fan notification writes out over chats"
    );
    assert_notification_probe(&state.db, &probe, false).await;
    assert!(
        load_channel(&state, OWNER, &connection.id)
            .await
            .unwrap()
            .notification_epoch
            > connection.notification_epoch
    );
}
