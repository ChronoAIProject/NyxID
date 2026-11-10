use super::*;
use crate::test_utils::{assert_notification_probe, seed_notification_purge_probe};

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
                doc! {"_id":&response_id,"channel_id":&row.id,"partition":PARTITION},
                doc! {"_id":&other_response_id,"channel_id":&row.id,"partition":other_partition},
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
