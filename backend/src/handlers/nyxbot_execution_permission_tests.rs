use super::*;

#[tokio::test]
async fn migration_channel_actual_ingress_enrolls_only_verified_original_owner() {
    for (original, stale_off) in [(true, false), (false, false), (true, true)] {
        let (state, _calls, server) = Box::pin(setup("migration_channel_ingress")).await;
        let _upstream = tokio_util::task::AbortOnDropHandle::new(server);
        crate::services::execution_permission_dispatch::phase_b_tests::enroll_global(
            &state.db, OWNER,
        )
        .await;
        let (row, key) = Box::pin(channel(&state, "gateway")).await;
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
            Box::pin(put_binding(
                State(state.clone()),
                Path("bnd_execution".into()),
                bearer(&key),
                Json(binding_body(&key, OWNER))
            ))
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            Box::pin(put_conversation(
                State(state.clone()),
                Path(("bnd_execution".into(), PARTITION.into())),
                bearer(&key)
            ))
            .await
            .status(),
            StatusCode::OK
        );
        if stale_off {
            state
                .db
                .collection::<bson::Document>("feature_flag_overrides")
                .update_many(
                    doc! {"flag_key":"permissions:execution-v1"},
                    doc! {"$set":{"enabled":false}},
                )
                .await
                .unwrap();
        }
        let mut input = event("Inspect my sheet", "7", "source-execution");
        input["event_context"]["activity"]["telegram_input"]["original"] = json!(original);
        let response = Box::pin(respond(&state, &key, &input, "execution-ingress")).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        assert!(body.contains("response.completed"), "{body}");
        let root = state
            .db
            .collection::<crate::models::assistant_message::AssistantMessage>("assistant_messages")
            .find_one(doc! {"user_id":OWNER,"role":"user"})
            .sort(doc! {"created_at":-1})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(root.execution_task.is_some(), original && !stale_off);
        if let Some(task) = root.execution_task {
            let source = task.channel_source.unwrap();
            assert_eq!(source.actor_user_id, OWNER);
            assert!(matches!(
                task.audience,
                crate::models::execution_permission::Audience::Channel { .. }
            ));
            let stored = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"_id":&source.event_id})
                .await
                .unwrap()
                .unwrap();
            assert!(stored.delivery.unwrap().execution_owner_verified);
            assert_eq!(source.conversation_id, root.conversation_id);
        }
    }
}

#[tokio::test]
async fn migration_channel_native_ingress_binds_exact_event_and_short_deadline() {
    let (state, _calls, server) = Box::pin(setup("migration_channel_native_ingress")).await;
    let _upstream = tokio_util::task::AbortOnDropHandle::new(server);
    crate::services::execution_permission_dispatch::phase_b_tests::enroll_global(&state.db, OWNER)
        .await;
    let (row, key) = native_tests::native_channel(&state).await;
    let input = native_tests::native_event(&row, "550", "Check the sheet");
    let response = respond(&state, &key, &input, "native-execution-source").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("response.completed"), "{body}");
    let root = state
        .db
        .collection::<crate::models::assistant_message::AssistantMessage>("assistant_messages")
        .find_one(doc! {"user_id":OWNER,"role":"user"})
        .await
        .unwrap()
        .unwrap();
    let source = root.execution_task.unwrap().channel_source.unwrap();
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&source.event_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event
            .delivery
            .as_ref()
            .unwrap()
            .origin
            .native_event_id
            .as_deref(),
        Some(source.event_id.as_str())
    );
    assert!(source.expires_at <= event.created_at + ChronoDuration::minutes(29));
    assert_eq!(event.delivery.unwrap().state, "replay");
}

#[tokio::test]
async fn migration_channel_owner_proof_excludes_guest_unknown_bot_forwarded_and_org() {
    use crate::models::channel_thread::ThreadSenderKind as Kind;
    let (state, _calls, server) = setup("migration_channel_proof").await;
    let _upstream = tokio_util::task::AbortOnDropHandle::new(server);
    let (mut row, _) = channel(&state, "gateway").await;
    for kind in [Kind::Human, Kind::Bot, Kind::Unknown] {
        for owner_eligible in [false, true] {
            for original in [false, true] {
                for guest in [false, true] {
                    assert_eq!(
                        crate::services::execution_permission_channel::verified_owner(
                            &row,
                            kind,
                            owner_eligible,
                            original,
                            guest
                        ),
                        kind == Kind::Human && owner_eligible && original && !guest
                    );
                }
            }
        }
    }
    for raw_kind in ["human", "bot", "unknown"] {
        let activity = json!({"actor":{"kind":raw_kind,"id":"7"}});
        let sender = native_responses::native_sender(&activity);
        assert_eq!(
            sender.kind,
            Kind::Human,
            "legacy native-chat classification is unchanged"
        );
        assert_eq!(
            crate::services::execution_permission_channel::verified_owner(
                &row,
                sender.execution_kind(),
                sender.owner_eligible,
                sender.original_message,
                false
            ),
            raw_kind == "human"
        );
    }
    row.bot_owner_id = Some("organization".into());
    assert!(
        !crate::services::execution_permission_channel::verified_owner(
            &row,
            Kind::Human,
            true,
            true,
            false
        )
    );
}
