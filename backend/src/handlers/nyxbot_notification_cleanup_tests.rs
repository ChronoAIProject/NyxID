use super::*;
use crate::test_utils::{assert_notification_probe, seed_notification_purge_probe};

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
