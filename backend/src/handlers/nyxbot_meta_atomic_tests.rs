//! Mutating entry points must settle one confirmed Meta assignment atomically.
use super::*;

#[tokio::test]
async fn combined_meta_patch_preserves_assignment_and_closed_or_open_access_on_refusal() {
    for initially_open in [false, true] {
        let (state, _calls, _server) = setup("meta_atomic_patch").await;
        let (_router, _token, _first, row) = assigned(&state, "instagram").await;
        let second = specialist_named(&state, "atomic-second").await;
        if initially_open {
            let Json(result) = link_channel(
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
            assert_eq!(result["private_chats"], "everyone");
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
        let Json(result) = link_channel(
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
        assert_eq!(result["private_chats"], "owner");
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
        let Json(result) = link_channel(
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
        assert_eq!(result["private_chats"], "everyone");
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

async fn provision_org_wallet(state: &AppState, org: &str) {
    let billing = crate::billing_integration_tests::billing_service(
        &state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        1000,
    );
    billing.ensure_wallet(org).await.unwrap();
}

#[tokio::test]
async fn org_meta_guest_enable_rechecks_callback_and_live_admin_before_opening_access() {
    for fault in ["callback", "demotion"] {
        let (state, _calls, _server) = setup("meta_guest_atomic_org").await;
        let (row, agent, org) = org_meta(&state).await;
        let (router, token) = app(&state).await;
        provision_org_wallet(&state, &org).await;
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
    provision_org_wallet(&state, &org).await;
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
    let Json(result) = crate::handlers::org_specialist_channels::update(
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
    assert_eq!(result["guest_enabled"], true);
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
    let Json(result) = crate::handlers::org_specialist_channels::update(
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
    assert_eq!(result["guest_enabled"], false);
    let closed = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(!closed.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(closed.agent_id, before.agent_id);
}

#[tokio::test]
async fn reused_meta_connect_returns_its_committed_assignment_and_challenge_during_rotation() {
    let (state, _calls, _server) = setup("meta_reuse_committed_response").await;
    let (_, _, first, row) = assigned(&state, "instagram").await;
    let second = specialist_named(&state, "concurrent-specialist").await;
    let committed = std::sync::Mutex::new(None);
    crate::services::durable_wake_tests::resume_after(
        "meta_reuse_after_commit",
        async {
            let Json(response) = connect_channel(
                State(state.clone()),
                crate::test_utils::test_auth_user(OWNER),
                Json(ConnectRequest {
                    bot_id: row.channel_bot_id.clone(),
                    agent_id: Some(first.id.clone()),
                }),
            )
            .await
            .unwrap();
            let snapshot: NyxbotChannel = committed.lock().unwrap().clone().unwrap();
            assert_eq!(
                response["channel_agent"]["agent_id"],
                json!(snapshot.agent_id)
            );
            assert_eq!(
                response["link"]["expires_at"],
                json!(snapshot.link_code_expires_at.unwrap())
            );
            assert_eq!(
                snapshot.link_code_hash.as_deref(),
                Some(
                    crate::crypto::token::hash_token(response["link"]["code"].as_str().unwrap())
                        .as_str()
                )
            );
            let current = load_channel(&state, OWNER, &row.id).await.unwrap();
            assert_eq!(current.agent_id.as_deref(), Some(second.id.as_str()));
            assert_ne!(current.link_code_hash, snapshot.link_code_hash);
        },
        async {
            *committed.lock().unwrap() = Some(load_channel(&state, OWNER, &row.id).await.unwrap());
            // Separate request/task: it must not inherit the first request's pause hook.
            let state = state.clone();
            let bot = row.channel_bot_id.clone();
            let agent = second.id.clone();
            tokio::spawn(async move {
                let Json(response) = connect_channel(
                    State(state),
                    crate::test_utils::test_auth_user(OWNER),
                    Json(ConnectRequest {
                        bot_id: bot,
                        agent_id: Some(agent.clone()),
                    }),
                )
                .await
                .unwrap();
                assert_eq!(response["channel_agent"]["agent_id"], agent);
            })
            .await
            .unwrap();
        },
    )
    .await;
}

#[tokio::test]
async fn legacy_org_meta_guest_reductions_remain_available_on_callback_failure() {
    use crate::services::{org_channel_service as org, security_readiness_service as readiness};
    let (state, _calls, _server) = setup("meta_legacy_org_narrowing").await;
    let (mut row, mut agent, org_id) = org_meta(&state).await;
    let replica = readiness::overlapping_test_replica(&state.db).await;
    readiness::remove_test_replica_capability(&state.db, &replica, org::GUEST_GRANTS_READINESS)
        .await;
    assert!(!org::guest_grants_ready(&state.db).await.unwrap());
    let service_ids: Vec<_> = (0..2).map(|_| Uuid::new_v4().to_string()).collect();
    for (i, id) in service_ids.iter().enumerate() {
        state
            .db
            .collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(crate::test_utils::test_user_service(
                id,
                &org_id,
                &format!("narrowing-{i}"),
                &Uuid::new_v4().to_string(),
                None,
                None,
            ))
            .await
            .unwrap();
    }
    agent.grants.service_ids = service_ids.clone();
    state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>("assistant_agents")
        .replace_one(doc! {"_id":&agent.id}, &agent)
        .await
        .unwrap();
    let policy = &mut row.org_specialist.as_mut().unwrap().guests;
    policy.enabled = true;
    policy.revision = Uuid::new_v4().to_string();
    policy.daily_credits = Some(crate::models::credits::Credits::from_whole(10));
    policy.service_ids = service_ids.clone();
    policy.service_authority = service_ids
        .iter()
        .map(|id| (id.clone(), org::management::service_digest(&agent, id)))
        .collect();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    callback_for(
        &state,
        &org_id,
        &row.route_api_key_id,
        "https://callbacks.example.test/drift",
    )
    .await;
    let revision = || async {
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .org_specialist
            .unwrap()
            .guests
            .revision
    };
    let patch = |value: Value| {
        let state = &state;
        let id = &row.id;
        async move {
            crate::handlers::org_specialist_channels::update_tool(state, OWNER, id, &value).await
        }
    };
    patch(json!({"daily_credits":"5","expected_revision":revision().await}))
        .await
        .unwrap();
    let lower = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(
        lower.org_specialist.as_ref().unwrap().guests.daily_credits,
        Some(crate::models::credits::Credits::from_whole(5))
    );
    assert!(matches!(
        patch(json!({"daily_credits":"6","expected_revision":revision().await})).await,
        Err(AppError::Conflict(_))
    ));
    patch(json!({"service_ids":[&service_ids[0]],"expected_revision":revision().await}))
        .await
        .unwrap();
    let narrower = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(
        narrower.org_specialist.as_ref().unwrap().guests.service_ids,
        vec![service_ids[0].clone()]
    );
    assert!(matches!(
        patch(json!({"service_ids":service_ids,"expected_revision":revision().await})).await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(patch(json!({"daily_credits":"4","expected_revision":lower.org_specialist.as_ref().unwrap().guests.revision})).await,Err(AppError::Conflict(_))));
    callback_for(
        &state,
        &org_id,
        &row.route_api_key_id,
        &crate::services::nyxbot_channel_query_service::direct_callback(
            &state.config.base_url,
            &row.id,
        ),
    )
    .await;
    // A live route permits this legacy expansion, but the fleet switches to
    // Grants before the write. Its previously narrowed service list must survive.
    crate::services::durable_wake_tests::resume_after(
        "org_guest_before_meta_write",
        async {
            assert!(matches!(
                patch(json!({"service_ids":service_ids,"expected_revision":revision().await}))
                    .await,
                Err(AppError::Conflict(_))
            ));
        },
        async {
            state
                .db
                .collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME)
                .delete_one(doc! {"_id":&replica.id})
                .await
                .unwrap();
            assert!(org::guest_grants_ready(&state.db).await.unwrap());
        },
    )
    .await;
    let frozen = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(
        frozen.org_specialist.as_ref().unwrap().guests,
        narrower.org_specialist.as_ref().unwrap().guests
    );
    state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&org_id,"member_user_id":OWNER},
            doc! {"$set":{"role":"viewer"}},
        )
        .await
        .unwrap();
    assert!(matches!(patch(json!({"daily_credits":"4","expected_revision":narrower.org_specialist.as_ref().unwrap().guests.revision})).await,Err(AppError::Forbidden(_))));
}

#[tokio::test]
async fn combined_org_meta_patch_shares_authority_and_preserves_old_and_new_binding_checks() {
    for enabled in [false, true] {
        let (state, _calls, _server) = setup("meta_org_shared_patch_authority").await;
        let (mut row, _, org_id) = org_meta(&state).await;
        row.org_specialist.as_mut().unwrap().guests.enabled = enabled;
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        let (second, _) = crate::services::assistant_team_service::create_specialist_for(
            &state.db,
            &state.encryption_keys,
            OWNER,
            &org_id,
            crate::services::assistant_team_service::CreateRequest {
                name: "shared-authority-second".into(),
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
        let users = state.db.collection::<bson::Document>("users");
        let before = users
            .find_one(doc! {"_id":&org_id})
            .await
            .unwrap()
            .unwrap()
            .get_i64("org_channel_fence")
            .unwrap_or(0);
        let channel_fence_before = state
            .db
            .collection::<bson::Document>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap()
            .get_i64("org_channel_fence")
            .unwrap_or(0);
        let Json(response) = link_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(row.id.clone()),
            Json(LinkRequest {
                agent_id: Some(second.id.clone()),
                private_chats: Some("everyone".into()),
                private_chat_posts: Some(false),
                apply_to_existing: false,
            }),
        )
        .await
        .unwrap();
        assert_eq!(response["private_chats"], "everyone");
        assert_eq!(
            response["audience_readiness"]["configuration"]["instructions_present"],
            true
        );
        assert_ne!(response["guest_readiness"]["reason"], "agent_unavailable");
        assert!(
            response["guest_access_description"]
                .as_str()
                .unwrap()
                .contains("organization")
        );
        let channel_fence_after = state
            .db
            .collection::<bson::Document>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap()
            .get_i64("org_channel_fence")
            .unwrap();
        assert_eq!(
            channel_fence_after - channel_fence_before,
            2,
            "old binding once, new binding once"
        );
        let after = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(after.agent_id.as_deref(), Some(second.id.as_str()));
        assert_eq!(after.private_chats.as_deref(), Some("everyone"));
        assert_ne!(
            after.org_specialist.as_ref().unwrap().revision,
            row.org_specialist.as_ref().unwrap().revision
        );
        assert_eq!(
            after.org_specialist.as_ref().unwrap().guests.enabled,
            enabled
        );
        let written = users
            .find_one(doc! {"_id":&org_id})
            .await
            .unwrap()
            .unwrap()
            .get_i64("org_channel_fence")
            .unwrap();
        assert_eq!(
            written - before,
            1,
            "one actor/org authority write across relink and access"
        );
        assert!(matches!(
            crate::services::nyxbot_channel_setup_service::mutate_meta_assignment(
                &state,
                OWNER,
                &row,
                None,
                Some(
                    &crate::services::nyxbot_chat_settings_service::AccessSettings {
                        private_chats: Some("everyone".into()),
                        private_chat_posts: None,
                        apply_to_existing: false,
                    }
                ),
                None
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
}

#[tokio::test]
async fn legacy_meta_policy_cannot_lift_execution_refusal_without_a_live_route() {
    use crate::services::{org_channel_service as org, security_readiness_service as readiness};
    for fault in ["missing-cap", "stale-digest"] {
        let (state, _calls, _server) = setup("meta_legacy_execution_gate").await;
        let (mut row, _, org_id) = org_meta(&state).await;
        let replica = readiness::overlapping_test_replica(&state.db).await;
        readiness::remove_test_replica_capability(&state.db, &replica, org::GUEST_GRANTS_READINESS)
            .await;
        let policy = &mut row.org_specialist.as_mut().unwrap().guests;
        policy.enabled = true;
        policy.revision = Uuid::new_v4().to_string();
        policy.daily_credits = if fault == "missing-cap" {
            None
        } else {
            Some(crate::models::credits::Credits::from_whole(10))
        };
        if fault == "stale-digest" {
            policy.service_ids = vec!["prior-service".into()];
            policy
                .service_authority
                .insert("prior-service".into(), "stale".into());
        }
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        callback_for(
            &state,
            &org_id,
            &row.route_api_key_id,
            "https://callbacks.example.test/drift",
        )
        .await;
        let before = row.org_specialist.as_ref().unwrap().guests.clone();
        let edit = if fault == "missing-cap" {
            json!({"daily_credits":"1","expected_revision":before.revision})
        } else {
            json!({"service_ids":[],"expected_revision":before.revision})
        };
        assert!(matches!(
            crate::handlers::org_specialist_channels::update_tool(&state, OWNER, &row.id, &edit)
                .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            load_channel(&state, OWNER, &row.id)
                .await
                .unwrap()
                .org_specialist
                .unwrap()
                .guests,
            before
        );
        if fault == "stale-digest" {
            // Narrow the budget without reauthorizing the unrelated stale delegation.
            crate::handlers::org_specialist_channels::update_tool(
                &state,
                OWNER,
                &row.id,
                &json!({"daily_credits":"5","expected_revision":before.revision}),
            )
            .await
            .unwrap();
            let saved = load_channel(&state, OWNER, &row.id)
                .await
                .unwrap()
                .org_specialist
                .unwrap()
                .guests;
            assert_eq!(saved.service_authority, before.service_authority);
            assert_eq!(saved.service_ids, before.service_ids);
        }
        callback_for(
            &state,
            &org_id,
            &row.route_api_key_id,
            &crate::services::nyxbot_channel_query_service::direct_callback(
                &state.config.base_url,
                &row.id,
            ),
        )
        .await;
        let current = load_channel(&state, OWNER, &row.id).await.unwrap();
        let edit = if fault == "missing-cap" {
            json!({"daily_credits":"1","expected_revision":current.org_specialist.as_ref().unwrap().guests.revision})
        } else {
            json!({"service_ids":[],"expected_revision":current.org_specialist.as_ref().unwrap().guests.revision})
        };
        crate::handlers::org_specialist_channels::update_tool(&state, OWNER, &row.id, &edit)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn meta_org_enable_cannot_fall_back_to_legacy_after_readiness_preflight() {
    use crate::services::{org_channel_service as org, security_readiness_service as readiness};
    let (state, _calls, _server) = setup("meta_org_enable_readiness_boundary").await;
    let (row, agent, org_id) = org_meta(&state).await;
    provision_org_wallet(&state, &org_id).await;
    let (router, token) = app(&state).await;
    let replica = readiness::overlapping_test_replica(&state.db).await;
    assert!(org::guest_grants_ready(&state.db).await.unwrap());
    crate::services::durable_wake_tests::resume_after(
        "org_meta_before_guest_mode",
        async {
            let (status, body) = action(
                &router,
                &token,
                &row.channel_bot_id,
                &agent.id,
                "enable_guests",
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        },
        async {
            readiness::remove_test_replica_capability(
                &state.db,
                &replica,
                org::GUEST_GRANTS_READINESS,
            )
            .await;
        },
    )
    .await;
    let closed = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(closed.private_chats, row.private_chats);
    assert_eq!(
        closed.org_specialist.as_ref().unwrap().guests,
        row.org_specialist.as_ref().unwrap().guests
    );
    state
        .db
        .collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME)
        .delete_one(doc! {"_id":replica.id})
        .await
        .unwrap();
    let (status, body) = action(
        &router,
        &token,
        &row.channel_bot_id,
        &agent.id,
        "enable_guests",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .org_specialist
            .unwrap()
            .guests
            .enabled
    );
}

#[tokio::test]
async fn first_and_repaired_meta_connect_return_their_committed_owner_code() {
    for (repair, customer) in [(false, false), (true, false), (false, true), (true, true)] {
        let (state, _calls, _server) = setup("meta_activation_response_snapshot").await;
        let (router, token) = app(&state).await;
        let first = specialist_named(&state, "activation-first").await;
        let second = specialist_named(&state, "activation-second").await;
        let bot_id = if repair {
            let (_, _, _, row) = assigned(&state, "instagram").await;
            routes::update_conversation(
                &state.db,
                row.route_id.as_deref().unwrap(),
                OWNER,
                None,
                None,
                Some(false),
                None,
            )
            .await
            .unwrap();
            row.channel_bot_id
        } else {
            let bot = bot_doc("instagram", "Fresh response inbox");
            let id = bot.get_str("_id").unwrap().to_owned();
            state
                .db
                .collection::<bson::Document>("channel_bots")
                .insert_one(bot)
                .await
                .unwrap();
            id
        };
        let snapshot = std::sync::Mutex::new(None);
        crate::services::durable_wake_tests::resume_after(
            "meta_new_connect_after_commit",
            async {
                let (assigned, code, expires) = if customer {
                    let (status, result) =
                        action(&router, &token, &bot_id, &first.id, "assign").await;
                    assert_eq!(status, StatusCode::OK, "{result}");
                    (
                        result["agent_id"].clone(),
                        result["owner_link_code"].clone(),
                        result["owner_link_expires_at"].clone(),
                    )
                } else {
                    let Json(result) = connect_channel(
                        State(state.clone()),
                        crate::test_utils::test_auth_user(OWNER),
                        Json(ConnectRequest {
                            bot_id: bot_id.clone(),
                            agent_id: Some(first.id.clone()),
                        }),
                    )
                    .await
                    .unwrap();
                    (
                        result["channel_agent"]["agent_id"].clone(),
                        result["link"]["code"].clone(),
                        result["link"]["expires_at"].clone(),
                    )
                };
                let committed: NyxbotChannel = snapshot.lock().unwrap().clone().unwrap();
                assert_eq!(assigned, first.id);
                assert_eq!(
                    expires
                        .as_str()
                        .unwrap()
                        .parse::<chrono::DateTime<chrono::Utc>>()
                        .unwrap(),
                    committed.link_code_expires_at.unwrap()
                );
                assert_eq!(
                    committed.link_code_hash,
                    Some(crate::crypto::token::hash_token(code.as_str().unwrap()))
                );
                let current = load_channel(&state, OWNER, &committed.id).await.unwrap();
                assert_eq!(current.agent_id.as_deref(), Some(second.id.as_str()));
                assert_ne!(current.link_code_hash, committed.link_code_hash);
            },
            async {
                let current = state
                    .db
                    .collection::<NyxbotChannel>(CHANNELS)
                    .find_one(doc! {"channel_bot_id":&bot_id,"status":"active"})
                    .await
                    .unwrap()
                    .unwrap();
                *snapshot.lock().unwrap() = Some(current);
                let Json(response) = connect_channel(
                    State(state.clone()),
                    crate::test_utils::test_auth_user(OWNER),
                    Json(ConnectRequest {
                        bot_id: bot_id.clone(),
                        agent_id: Some(second.id.clone()),
                    }),
                )
                .await
                .unwrap();
                assert_eq!(response["channel_agent"]["agent_id"], second.id);
            },
        )
        .await;
    }
}

#[tokio::test]
async fn meta_connect_with_lost_activation_ack_rejects_a_superseding_patch() {
    use crate::services::nyxbot_channel_connection_service::{
        META_ACTIVATION_ACK_LOST, RESTORE_CONNECT_FAIL_STAGE,
    };
    for relink in [false, true] {
        let (state, _calls, _server) = setup("meta_connect_uncertain_activation").await;
        let _ = app(&state).await;
        let first = specialist_named(&state, "first-assignment").await;
        let second = specialist_named(&state, "second-assignment").await;
        let bot = bot_doc("messenger", "Support");
        let bot_id = bot.get_str("_id").unwrap().to_string();
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .insert_one(bot)
            .await
            .unwrap();
        let connect = connect_channel(
            State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Json(ConnectRequest {
                bot_id: bot_id.clone(),
                agent_id: Some(first.id.clone()),
            }),
        );
        crate::services::durable_wake_tests::resume_after(
            "meta_activation_ack_lost",
            async {
                let result = RESTORE_CONNECT_FAIL_STAGE
                    .scope(META_ACTIVATION_ACK_LOST, connect)
                    .await;
                if relink {
                    assert!(matches!(result, Err(AppError::Conflict(_))));
                } else {
                    assert_eq!(result.unwrap().0["channel_agent"]["agent_id"], first.id);
                }
            },
            async {
                if relink {
                    let current = state
                        .db
                        .collection::<NyxbotChannel>(CHANNELS)
                        .find_one(doc! {"channel_bot_id":&bot_id,"status":"active"})
                        .await
                        .unwrap()
                        .unwrap();
                    let Json(updated) = link_channel(
                        State(state.clone()),
                        crate::test_utils::test_auth_user(OWNER),
                        Path(current.id.clone()),
                        Json(LinkRequest {
                            agent_id: Some(second.id.clone()),
                            private_chats: None,
                            private_chat_posts: None,
                            apply_to_existing: false,
                        }),
                    )
                    .await
                    .unwrap();
                    assert_eq!(updated["changed"], true);
                    let saved = load_channel(&state, OWNER, &current.id).await.unwrap();
                    assert_eq!(saved.link_code_hash, current.link_code_hash);
                    assert_eq!(saved.agent_id.as_deref(), Some(second.id.as_str()));
                }
            },
        )
        .await;
    }
}

#[tokio::test]
async fn combined_meta_patch_keeps_readiness_for_its_committed_target_during_a_second_relink() {
    let (state, _calls, _server) = setup("meta_committed_audience_result").await;
    let (_, _, first, row) = assigned(&state, "instagram").await;
    let second = specialist_named(&state, "committed-readiness-second").await;
    state
        .db
        .collection::<bson::Document>("assistant_agents")
        .update_one(doc! {"_id":&first.id}, doc! {"$set":{"description":""}})
        .await
        .unwrap();
    crate::services::durable_wake_tests::resume_after(
        "meta_combined_after_commit",
        async {
            let Json(response) = link_channel(
                State(state.clone()),
                crate::test_utils::test_auth_user(OWNER),
                Path(row.id.clone()),
                Json(LinkRequest {
                    agent_id: Some(second.id.clone()),
                    private_chats: Some("everyone".into()),
                    private_chat_posts: None,
                    apply_to_existing: false,
                }),
            )
            .await
            .unwrap();
            assert_eq!(response["agent"], second.name);
            assert_eq!(
                response["audience_readiness"]["configuration"]["instructions_present"],
                true
            );
            assert_eq!(
                load_channel(&state, OWNER, &row.id)
                    .await
                    .unwrap()
                    .agent_id
                    .as_deref(),
                Some(first.id.as_str())
            );
        },
        async {
            let Json(response) = link_channel(
                State(state.clone()),
                crate::test_utils::test_auth_user(OWNER),
                Path(row.id.clone()),
                Json(LinkRequest {
                    agent_id: Some(first.id.clone()),
                    private_chats: None,
                    private_chat_posts: None,
                    apply_to_existing: false,
                }),
            )
            .await
            .unwrap();
            assert_eq!(response["agent"], first.name);
        },
    )
    .await;
}
