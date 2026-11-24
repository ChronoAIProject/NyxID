//! Mutating entry points must settle one confirmed Meta assignment atomically.
use super::*;

#[tokio::test]
async fn combined_meta_patch_preserves_assignment_and_closed_or_open_access_on_refusal() {
    for initially_open in [false, true] {
        let (state, _calls, _server) = setup("meta_atomic_patch").await;
        let (_router, _token, _first, row) = assigned(&state, "instagram").await;
        let second = specialist_named(&state, "atomic-second").await;
        if initially_open {
            link_channel(
                State(state.clone()),
                crate::test_utils::test_auth_user(OWNER),
                Path(row.id.clone()),
                Json(LinkRequest {
                    agent_id: None,
                    private_chats: Some("everyone".into()),
                    private_chat_posts: None,
                    apply_to_existing: false,
                }),
            )
            .await
            .unwrap();
        }
        let before = load_channel(&state, OWNER, &row.id).await.unwrap();
        let thread_id = Uuid::new_v4().to_string();
        let threads = state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME);
        threads.insert_one(doc! {"_id":&thread_id,"user_id":OWNER,"channel_id":&row.id,
            "partition":"private-atomic","kind":"private","conversation_id":"retained-private-session",
            "allow_posts":false,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        crate::services::durable_wake_tests::resume_after(
            "meta_relink_before_commit",
            async {
                let error = link_channel(
                    State(state.clone()),
                    crate::test_utils::test_auth_user(OWNER),
                    Path(row.id.clone()),
                    Json(LinkRequest {
                        agent_id: Some(second.id.clone()),
                        private_chats: Some(
                            if initially_open { "owner" } else { "everyone" }.into(),
                        ),
                        private_chat_posts: Some(true),
                        apply_to_existing: true,
                    }),
                )
                .await
                .unwrap_err();
                assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
            },
            async {
                callback(
                    &state,
                    &row.route_api_key_id,
                    "https://callbacks.example.test/drift",
                )
                .await;
            },
        )
        .await;
        let after = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(after.agent_id, before.agent_id);
        assert_eq!(after.private_chats, before.private_chats);
        assert_eq!(after.private_chat_posts, before.private_chat_posts);
        assert_eq!(after.link_code_hash, before.link_code_hash);
        assert_eq!(
            after.execution_policy_revision,
            before.execution_policy_revision
        );
        let untouched = threads
            .find_one(doc! {"_id":&thread_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            untouched.get_str("conversation_id"),
            Ok("retained-private-session")
        );
        assert_eq!(untouched.get_bool("allow_posts"), Ok(false));
        // Closing remains independently available despite the broken callback.
        link_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(row.id.clone()),
            Json(LinkRequest {
                agent_id: None,
                private_chats: Some("owner".into()),
                private_chat_posts: Some(false),
                apply_to_existing: false,
            }),
        )
        .await
        .unwrap();
        let closed = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(closed.private_chats.as_deref(), Some("owner"));
        assert_eq!(closed.agent_id, before.agent_id);
        callback(
            &state,
            &row.route_api_key_id,
            &crate::services::nyxbot_channel_query_service::direct_callback(
                &state.config.base_url,
                &row.id,
            ),
        )
        .await;
        link_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(row.id.clone()),
            Json(LinkRequest {
                agent_id: Some(second.id.clone()),
                private_chats: Some("everyone".into()),
                private_chat_posts: Some(true),
                apply_to_existing: true,
            }),
        )
        .await
        .unwrap();
        let accepted = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(accepted.agent_id.as_deref(), Some(second.id.as_str()));
        assert_eq!(accepted.private_chats.as_deref(), Some("everyone"));
        let changed = threads
            .find_one(doc! {"_id":&thread_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(changed.get("conversation_id"), Some(&bson::Bson::Null));
        assert_eq!(changed.get_bool("allow_posts"), Ok(true));
    }
}

#[tokio::test]
async fn explicit_meta_owner_link_reloads_health_before_changing_the_challenge() {
    let (state, _calls, _server) = setup("meta_owner_live_health").await;
    let (router, token, agent, row) = assigned(&state, "messenger").await;
    crate::services::durable_wake_tests::resume_after(
        "meta_owner_link_before_commit",
        async {
            let (status, body) = action(
                &router,
                &token,
                &row.channel_bot_id,
                &agent.id,
                "owner_link",
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT, "{body}");
        },
        async {
            state
                .db
                .collection::<bson::Document>(CHANNELS)
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"delivery_status":"failing"}},
                )
                .await
                .unwrap();
        },
    )
    .await;
    let after = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(after.link_code_hash, row.link_code_hash);
    assert_eq!(after.link_code_expires_at, row.link_code_expires_at);
    assert_eq!(after.agent_id, row.agent_id);
    assert_eq!(after.delivery_status.as_deref(), Some("failing"));
}

async fn org_meta(
    state: &AppState,
) -> (
    NyxbotChannel,
    crate::models::assistant_agent::AssistantAgent,
    String,
) {
    let (mut row, _, agent, org) = org_channel_tests::org_channel(state, false).await;
    let _ = app(state).await;
    row.platform = "instagram".into();
    row.transport = "direct".into();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"platform":"instagram"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.route_id},
            doc! {"$set":{"platform":"instagram"}},
        )
        .await
        .unwrap();
    callback_for(
        state,
        &org,
        &row.route_api_key_id,
        &crate::services::nyxbot_channel_query_service::direct_callback(
            &state.config.base_url,
            &row.id,
        ),
    )
    .await;
    (row, agent, org)
}

async fn callback_for(state: &AppState, owner: &str, id: &str, url: &str) {
    key_service::update_api_key_scope_with_scope_authorization(
        &state.db,
        owner,
        Some(OWNER),
        id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(Some(url)),
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn personal_meta_guest_enable_rechecks_the_route_key_after_readiness_preflight() {
    for fault in ["callback", "revoked-key"] {
        let (state, _calls, _server) = setup("meta_guest_atomic_personal").await;
        let (router, token, agent, row) = assigned(&state, "messenger").await;
        let (status, body) = action(
            &router,
            &token,
            &row.channel_bot_id,
            &agent.id,
            "prepare_model",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let (status, body) =
                action(&router, &token, &row.channel_bot_id, &agent.id, "check").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            if body["readiness"]["guest_readiness"]["status"] == "ready" {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "{body}");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let before = load_channel(&state, OWNER, &row.id).await.unwrap();
        crate::services::durable_wake_tests::resume_after(
            "channel_access_before_confirmed_update",
            async {
                let (status, body) = action(
                    &router,
                    &token,
                    &row.channel_bot_id,
                    &agent.id,
                    "enable_guests",
                )
                .await;
                assert_eq!(status, StatusCode::CONFLICT, "{fault}: {body}");
            },
            async {
                if fault == "callback" {
                    callback(
                        &state,
                        &row.route_api_key_id,
                        "https://callbacks.example.test/drift",
                    )
                    .await;
                } else {
                    key_service::delete_api_key(&state.db, OWNER, &row.route_api_key_id)
                        .await
                        .unwrap();
                }
            },
        )
        .await;
        let after = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(after.private_chats, before.private_chats);
        assert_eq!(after.agent_id, before.agent_id);
        assert_eq!(after.link_code_hash, before.link_code_hash);
    }
}

#[tokio::test]
async fn org_meta_guest_enable_rechecks_callback_and_live_admin_before_opening_access() {
    for fault in ["callback", "demotion"] {
        let (state, _calls, _server) = setup("meta_guest_atomic_org").await;
        let (row, agent, org) = org_meta(&state).await;
        let (router, token) = app(&state).await;
        let billing = crate::billing_integration_tests::billing_service(
            &state.db,
            Arc::new(crate::billing_integration_tests::FakeLago::default()),
            1000,
        );
        billing.ensure_wallet(&org).await.unwrap();
        let before = load_channel(&state, OWNER, &row.id).await.unwrap();
        crate::services::durable_wake_tests::resume_after(
            "org_guest_before_meta_write",
            async {
                let (status, body) = action(
                    &router,
                    &token,
                    &row.channel_bot_id,
                    &agent.id,
                    "enable_guests",
                )
                .await;
                let expected = if fault == "demotion" {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::CONFLICT
                };
                assert_eq!(status, expected, "{fault}: {body}");
            },
            async {
                if fault == "callback" {
                    callback_for(
                        &state,
                        &org,
                        &row.route_api_key_id,
                        "https://callbacks.example.test/drift",
                    )
                    .await;
                } else {
                    state
                        .db
                        .collection::<bson::Document>("org_memberships")
                        .update_one(
                            doc! {"org_user_id":&org,"member_user_id":OWNER},
                            doc! {"$set":{"role":"viewer"}},
                        )
                        .await
                        .unwrap();
                }
            },
        )
        .await;
        let after = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.private_chats, before.private_chats);
        assert_eq!(
            after.org_specialist.as_ref().unwrap().guests,
            before.org_specialist.as_ref().unwrap().guests
        );
        assert_eq!(after.agent_id, before.agent_id);
        assert_eq!(after.link_code_hash, before.link_code_hash);
    }
}

#[tokio::test]
async fn enabled_org_meta_combined_patch_keeps_the_old_policy_and_assignment_on_refusal() {
    let (state, _calls, _server) = setup("meta_atomic_enabled_org").await;
    let (row, _first, org) = org_meta(&state).await;
    let (second, _) = crate::services::assistant_team_service::create_specialist_for(
        &state.db,
        &state.encryption_keys,
        OWNER,
        &org,
        crate::services::assistant_team_service::CreateRequest {
            name: "atomic-org-second".into(),
            description: "Support".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            machines: None,
            logins: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    crate::handlers::org_specialist_channels::update(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(row.id.clone()),
        Json(
            crate::services::org_channel_service::management::GuestSettings {
                enabled: true,
                service_ids: None,
                daily_credits: None,
                expected_revision: None,
            },
        ),
    )
    .await
    .unwrap();
    let before = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(before.org_specialist.as_ref().unwrap().guests.enabled);
    crate::services::durable_wake_tests::resume_after(
        "meta_relink_before_commit",
        async {
            let error = link_channel(
                State(state.clone()),
                crate::test_utils::test_auth_user(OWNER),
                Path(row.id.clone()),
                Json(LinkRequest {
                    agent_id: Some(second.id.clone()),
                    private_chats: Some("owner".into()),
                    private_chat_posts: Some(false),
                    apply_to_existing: false,
                }),
            )
            .await
            .unwrap_err();
            assert!(matches!(error, AppError::Conflict(_)));
        },
        async {
            callback_for(
                &state,
                &org,
                &row.route_api_key_id,
                "https://callbacks.example.test/drift",
            )
            .await;
        },
    )
    .await;
    let after = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(after.agent_id, before.agent_id);
    assert_eq!(after.private_chats, before.private_chats);
    assert_eq!(
        after.org_specialist.as_ref().unwrap().guests,
        before.org_specialist.as_ref().unwrap().guests
    );
    // Explicit organization guest shutdown retains its independent authority.
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"delivery_status":"failing"}},
        )
        .await
        .unwrap();
    crate::handlers::org_specialist_channels::update(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(row.id.clone()),
        Json(
            crate::services::org_channel_service::management::GuestSettings {
                enabled: false,
                service_ids: None,
                daily_credits: None,
                expected_revision: None,
            },
        ),
    )
    .await
    .unwrap();
    let closed = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(!closed.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(closed.agent_id, before.agent_id);
}
