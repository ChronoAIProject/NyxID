//! Every direct Meta setup validates its default without replacing owner routes.
use super::*;
use crate::services::{channel_routing_service as routes, key_service};

async fn assigned(
    state: &AppState,
    platform: &str,
) -> (
    Router,
    String,
    crate::models::assistant_agent::AssistantAgent,
    NyxbotChannel,
) {
    let agent = specialist(state).await;
    let bot = bot_doc(platform, "Routing contract inbox");
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .insert_one(bot)
        .await
        .unwrap();
    let (router, token) = app(state).await;
    let (status, result) = action(&router, &token, &bot_id, &agent.id, "assign").await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let row = load_channel(state, OWNER, result["channel_id"].as_str().unwrap())
        .await
        .unwrap();
    (router, token, agent, row)
}

async fn callback(state: &AppState, key: &str, url: &str) {
    key_service::update_api_key_scope_with_scope_authorization(
        &state.db,
        OWNER,
        Some(OWNER),
        key,
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
async fn customer_setup_refuses_drifted_defaults_and_repairs_only_its_unclaimed_assignment() {
    for fault in ["key", "default", "callback"] {
        let (state, _calls, _server) = setup("meta_default_drift").await;
        let (router, token, agent, row) = assigned(&state, "instagram").await;
        let other = key(&state, "Explicit conversation callback").await;
        callback(&state, &other.id, "https://callbacks.example.test/custom").await;
        let explicit = routes::create_conversation(
            &state.db,
            OWNER,
            Some(&row.channel_bot_id),
            "instagram",
            "customer-override",
            "private",
            None,
            &other.id,
            false,
            false,
        )
        .await
        .unwrap();
        match fault {
            "key" => {
                routes::update_conversation(
                    &state.db,
                    row.route_id.as_deref().unwrap(),
                    OWNER,
                    Some(&other.id),
                    None,
                    None,
                    None,
                )
                .await
                .unwrap();
            }
            "default" => {
                routes::update_conversation(
                    &state.db,
                    row.route_id.as_deref().unwrap(),
                    OWNER,
                    None,
                    Some(false),
                    None,
                    None,
                )
                .await
                .unwrap();
            }
            _ => {
                callback(
                    &state,
                    &row.route_api_key_id,
                    "https://callbacks.example.test/changed",
                )
                .await
            }
        }
        let (status, result) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "check").await;
        assert_eq!(status, StatusCode::CONFLICT, "{fault}: {result}");
        let (status, result) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "assign").await;
        if fault == "key" {
            assert_eq!(status, StatusCode::CONFLICT, "{result}");
            let current = load_channel(&state, OWNER, &row.id).await.unwrap();
            assert_eq!(current.status, "active");
            assert!(
                key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
                    .await
                    .is_ok()
            );
            assert_eq!(
                routes::resolve_agent(&state.db, &row.channel_bot_id, "new-customer", None, OWNER)
                    .await
                    .unwrap()
                    .unwrap()
                    .api_key_id,
                other.id
            );
        } else {
            assert_eq!(status, StatusCode::OK, "{fault}: {result}");
            let current = load_channel(&state, OWNER, result["channel_id"].as_str().unwrap())
                .await
                .unwrap();
            let default =
                routes::resolve_agent(&state.db, &row.channel_bot_id, "new-customer", None, OWNER)
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(default.api_key_id, current.route_api_key_id);
            assert_eq!(
                default.callback_url,
                crate::services::nyxbot_channel_query_service::direct_callback(
                    &state.config.base_url,
                    &current.id
                )
            );
        }
        let custom = routes::resolve_agent(
            &state.db,
            &row.channel_bot_id,
            "customer-override",
            None,
            OWNER,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(custom.conversation.id, explicit.id);
        assert_eq!(custom.api_key_id, other.id);
    }
}

#[tokio::test]
async fn meta_repair_preserves_an_api_restored_route_and_preexisting_competing_default() {
    for competing in [false, true] {
        let (state, _calls, _server) = setup("meta_repair_live_route").await;
        let (router, token, agent, row) = assigned(&state, "messenger").await;
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
        let other = key(&state, "Owner chosen default").await;
        callback(&state, &other.id, "https://callbacks.example.test/chosen").await;
        let chosen = if competing {
            Some(
                routes::create_conversation(
                    &state.db,
                    OWNER,
                    Some(&row.channel_bot_id),
                    "messenger",
                    "*",
                    "private",
                    None,
                    &other.id,
                    true,
                    false,
                )
                .await
                .unwrap(),
            )
        } else {
            None
        };
        crate::services::durable_wake_tests::resume_after(
            "channel_repair_before_retirement",
            async {
                let (status, result) =
                    action(&router, &token, &row.channel_bot_id, &agent.id, "assign").await;
                assert_eq!(status, StatusCode::CONFLICT, "{result}");
            },
            async {
                if !competing {
                    routes::update_conversation(
                        &state.db,
                        row.route_id.as_deref().unwrap(),
                        OWNER,
                        None,
                        None,
                        Some(true),
                        None,
                    )
                    .await
                    .unwrap();
                }
            },
        )
        .await;
        let current = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(current.status, "active");
        assert_eq!(current.link_code_hash, row.link_code_hash);
        assert!(
            key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
                .await
                .is_ok()
        );
        let target = routes::resolve_agent(
            &state.db,
            &row.channel_bot_id,
            "fresh-customer",
            None,
            OWNER,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            target.conversation.id,
            chosen
                .as_ref()
                .map(|route| route.id.as_str())
                .unwrap_or(row.route_id.as_deref().unwrap())
        );
    }
}

#[tokio::test]
async fn fresh_meta_setup_does_not_replace_a_default_selected_after_preflight() {
    let (state, _calls, _server) = setup("meta_new_default_race").await;
    let agent = specialist(&state).await;
    let bot = bot_doc("instagram", "New inbox");
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .insert_one(bot)
        .await
        .unwrap();
    let (router, token) = app(&state).await;
    let other = key(&state, "Owner selected callback").await;
    callback(&state, &other.id, "https://callbacks.example.test/selected").await;
    crate::services::durable_wake_tests::resume_after(
        "meta_default_before_insert",
        async {
            let (status, result) = action(&router, &token, &bot_id, &agent.id, "assign").await;
            assert_eq!(status, StatusCode::CONFLICT, "{result}");
        },
        async {
            routes::create_conversation(
                &state.db,
                OWNER,
                Some(&bot_id),
                "instagram",
                "*",
                "private",
                None,
                &other.id,
                true,
                false,
            )
            .await
            .unwrap();
        },
    )
    .await;
    let selected = routes::resolve_agent(&state.db, &bot_id, "fresh-customer", None, OWNER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.api_key_id, other.id);
    assert!(
        key_service::get_api_key(&state.db, OWNER, &other.id)
            .await
            .is_ok()
    );
    // The ordinary route API retains its explicit replace-default operation.
    let replacement = key(&state, "Explicit replacement").await;
    callback(
        &state,
        &replacement.id,
        "https://callbacks.example.test/replacement",
    )
    .await;
    let normal = routes::create_conversation(
        &state.db,
        OWNER,
        Some(&bot_id),
        "instagram",
        "*",
        "private",
        None,
        &replacement.id,
        true,
        false,
    )
    .await
    .unwrap();
    let current = routes::resolve_agent(&state.db, &bot_id, "next-customer", None, OWNER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.conversation.id, normal.id);
    assert_eq!(current.api_key_id, replacement.id);
}

#[tokio::test]
async fn meta_repair_and_check_agree_on_revoked_or_expired_stored_keys() {
    for fault in ["revoked-agent", "expired-agent", "expired-route"] {
        let (state, _calls, _server) = setup("meta_repair_key_eligibility").await;
        let (router, token, agent, row) = assigned(&state, "instagram").await;
        let stored = key(&state, "Retained direct provider key").await;
        let target = if fault == "expired-route" {
            &row.route_api_key_id
        } else {
            state
                .db
                .collection::<bson::Document>(CHANNELS)
                .update_one(
                    doc! {"_id": &row.id},
                    doc! {"$set": {"agent_api_key_id": &stored.id}},
                )
                .await
                .unwrap();
            &stored.id
        };
        let change = if fault == "revoked-agent" {
            doc! {"is_active": false}
        } else {
            doc! {"expires_at": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(1))}
        };
        state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .update_one(doc! {"_id": target}, doc! {"$set": change})
            .await
            .unwrap();
        let (status, check) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "check").await;
        assert_eq!(status, StatusCode::CONFLICT, "{fault}: {check}");
        let (status, repaired) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "assign").await;
        assert_eq!(status, StatusCode::OK, "{fault}: {repaired}");
        assert_ne!(repaired["channel_id"], row.id);
        let (status, healthy) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "check").await;
        assert_eq!(status, StatusCode::OK, "{fault}: {healthy}");
        assert_eq!(healthy["channel_id"], repaired["channel_id"]);
    }
}

#[tokio::test]
async fn ordinary_and_assistant_meta_connect_preserve_competing_routes_and_assignments() {
    for entry in ["human", "assistant"] {
        let (state, _calls, _server) = setup("meta_all_connect_entries").await;
        let (router, token, agent, row) = assigned(&state, "messenger").await;
        let chosen = key(&state, "Owner callback").await;
        callback(&state, &chosen.id, "https://callbacks.example.test/owner").await;
        routes::update_conversation(
            &state.db,
            row.route_id.as_deref().unwrap(),
            OWNER,
            Some(&chosen.id),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let before = load_channel(&state, OWNER, &row.id).await.unwrap();
        let error = if entry == "human" {
            connect_channel(
                State(state.clone()),
                crate::test_utils::test_auth_user(OWNER),
                Json(ConnectRequest {
                    bot_id: row.channel_bot_id.clone(),
                    agent_id: Some(agent.id.clone()),
                }),
            )
            .await
            .err()
            .unwrap()
        } else {
            connect_tool(&state, OWNER, "source-chat", &row.channel_bot_id, &agent)
                .await
                .err()
                .unwrap()
        };
        assert!(matches!(error, AppError::Conflict(_)), "{entry}: {error:?}");
        let after = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(after.status, "active");
        assert_eq!(after.link_code_hash, before.link_code_hash);
        assert!(
            key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
                .await
                .is_ok()
        );
        let (status, _) = action(&router, &token, &row.channel_bot_id, &agent.id, "check").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            routes::resolve_agent(&state.db, &row.channel_bot_id, "new", None, OWNER)
                .await
                .unwrap()
                .unwrap()
                .api_key_id,
            chosen.id
        );
    }
}

#[tokio::test]
async fn meta_org_preview_uses_the_same_live_callback_proof_for_both_lookup_modes() {
    let (state, _calls, _server) = setup("meta_org_preview_callback").await;
    let (mut row, _, agent, org) = org_channel_tests::org_channel(&state, false).await;
    let _ = app(&state).await;
    row.platform = "instagram".into();
    row.transport = "direct".into();
    row.org_specialist
        .as_mut()
        .unwrap()
        .guests
        .follows_agent_grants = true;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &row.id}, &row)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id": &row.channel_bot_id},
            doc! {"$set": {"platform": "instagram"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.route_id},
            doc! {"$set": {"platform": "instagram"}},
        )
        .await
        .unwrap();
    let callback = crate::services::nyxbot_channel_query_service::direct_callback(
        &state.config.base_url,
        &row.id,
    );
    let keys = state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
    keys.update_one(
        doc! {"_id": &row.route_api_key_id},
        doc! {"$set": {"callback_url": callback}},
    )
    .await
    .unwrap();
    for explicit in [None, Some(row.id.as_str())] {
        let preview = crate::services::org_channel_service::preview::load(
            &state.db,
            &state.encryption_keys,
            &state.config.base_url,
            OWNER,
            &row.channel_bot_id,
            &agent.id,
            explicit,
        )
        .await
        .unwrap();
        assert_eq!(preview.guest_policy_mode, "grants");
    }
    crate::services::key_service::update_api_key_scope_with_scope_authorization(
        &state.db,
        &org,
        Some(OWNER),
        &row.route_api_key_id,
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
        Some(Some("https://callbacks.example.test/drift")),
        None,
    )
    .await
    .unwrap();
    for explicit in [None, Some(row.id.as_str())] {
        let preview = crate::services::org_channel_service::preview::load(
            &state.db,
            &state.encryption_keys,
            &state.config.base_url,
            OWNER,
            &row.channel_bot_id,
            &agent.id,
            explicit,
        )
        .await
        .unwrap();
        assert_eq!(preview.guest_policy_mode, "grants");
    }
    let retained = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(retained.status, "active");
    assert_eq!(retained.link_code_hash, row.link_code_hash);
    let (second, _) = crate::services::assistant_team_service::create_specialist_for(
        &state.db,
        &state.encryption_keys,
        OWNER,
        &org,
        crate::services::assistant_team_service::CreateRequest {
            name: "second-org-specialist".into(),
            description: "Organization support".into(),
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
    let error = link_channel(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(row.id.clone()),
        Json(LinkRequest {
            agent_id: Some(second.id),
            private_chats: None,
            private_chat_posts: None,
            apply_to_existing: false,
        }),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::Conflict(ref message) if message.contains("repair")));
    let retained = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(retained.agent_id, row.agent_id);
    assert_eq!(retained.link_code_hash, row.link_code_hash);
    assert!(
        retained
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .follows_agent_grants
    );
    // Ordinary Connect repairs the same specialist; its deliberately saved
    // grants policy survives both preview and actual restoration.
    let Json(connected) = connect_channel(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Json(ConnectRequest {
            bot_id: row.channel_bot_id.clone(),
            agent_id: Some(agent.id.clone()),
        }),
    )
    .await
    .unwrap();
    let repaired = load_channel(
        &state,
        OWNER,
        connected["channel_agent"]["id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    assert_ne!(repaired.id, row.id);
    assert_eq!(repaired.agent_id.as_deref(), Some(agent.id.as_str()));
    assert!(
        repaired
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .follows_agent_grants
    );
    assert!(
        crate::services::nyxbot_channel_query_service::reusable_meta(
            &state.db,
            OWNER,
            &org,
            &repaired,
            &state.config.base_url
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn meta_reused_connect_and_owner_link_refuse_callback_drift_before_committing() {
    for entry in ["patch", "human", "assistant", "owner-code", "owner-action"] {
        let (state, _calls, _server) = setup("meta_reuse_route_race").await;
        let (router, token, agent, row) = assigned(&state, "instagram").await;
        let second = specialist_named(&state, "second-personal-specialist").await;
        let site = match entry {
            "patch" => "meta_relink_before_commit",
            "owner-code" | "owner-action" => "meta_owner_link_before_commit",
            _ => "meta_reuse_before_link",
        };
        crate::services::durable_wake_tests::resume_after(
            site,
            async {
                let error = match entry {
                    "patch" => link_channel(
                        State(state.clone()),
                        crate::test_utils::test_auth_user(OWNER),
                        Path(row.id.clone()),
                        Json(LinkRequest {
                            agent_id: Some(second.id.clone()),
                            private_chats: None,
                            private_chat_posts: None,
                            apply_to_existing: false,
                        }),
                    )
                    .await
                    .unwrap_err(),
                    "assistant" => {
                        connect_tool(&state, OWNER, "source-chat", &row.channel_bot_id, &second)
                            .await
                            .unwrap_err()
                    }
                    "owner-action" => {
                        let (status, body) = action(
                            &router,
                            &token,
                            &row.channel_bot_id,
                            &agent.id,
                            "owner_link",
                        )
                        .await;
                        assert_eq!(status, StatusCode::CONFLICT, "{body}");
                        return;
                    }
                    _ => connect_channel(
                        State(state.clone()),
                        crate::test_utils::test_auth_user(OWNER),
                        Json(ConnectRequest {
                            bot_id: row.channel_bot_id.clone(),
                            agent_id: Some(if entry == "owner-code" {
                                agent.id.clone()
                            } else {
                                second.id.clone()
                            }),
                        }),
                    )
                    .await
                    .unwrap_err(),
                };
                assert!(
                    matches!(error, AppError::Conflict(ref message) if message.contains("repair")),
                    "{entry}: {error:?}"
                );
            },
            async {
                callback(
                    &state,
                    &row.route_api_key_id,
                    "https://callbacks.example.test/drifted",
                )
                .await;
            },
        )
        .await;
        let retained = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(retained.agent_id, row.agent_id, "{entry}");
        assert_eq!(retained.link_code_hash, row.link_code_hash, "{entry}");
        assert_eq!(
            retained.execution_policy_revision, row.execution_policy_revision,
            "{entry}"
        );
        assert!(
            key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
                .await
                .is_ok()
        );
    }
}

#[tokio::test]
async fn meta_activation_rechecks_default_callback_and_fallback_uniqueness() {
    for fault in ["default", "callback", "competing"] {
        let (state, _calls, _server) = setup("meta_activation_route_race").await;
        let agent = specialist(&state).await;
        let bot = bot_doc("instagram", "Activation inbox");
        let bot_id = bot.get_str("_id").unwrap().to_owned();
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .insert_one(bot)
            .await
            .unwrap();
        let (router, token) = app(&state).await;
        let other = key(&state, "Concurrent default").await;
        callback(&state, &other.id, "https://callbacks.example.test/other").await;
        crate::services::durable_wake_tests::resume_after(
            "meta_default_before_activation",
            async {
                let (status, body) = action(&router, &token, &bot_id, &agent.id, "assign").await;
                assert_eq!(status, StatusCode::CONFLICT, "{fault}: {body}");
            },
            async {
                let route = state
                    .db
                    .collection::<crate::models::channel_conversation::ChannelConversation>(
                        crate::models::channel_conversation::COLLECTION_NAME,
                    )
                    .find_one(doc! {"channel_bot_id": &bot_id, "is_active": true})
                    .await
                    .unwrap()
                    .unwrap();
                match fault {
                    "default" => {
                        routes::update_conversation(
                            &state.db,
                            &route.id,
                            OWNER,
                            None,
                            Some(false),
                            None,
                            None,
                        )
                        .await
                        .unwrap();
                    }
                    "callback" => {
                        callback(
                            &state,
                            &route.agent_api_key_id,
                            "https://callbacks.example.test/changed",
                        )
                        .await
                    }
                    _ => {
                        routes::create_conversation(
                            &state.db,
                            OWNER,
                            Some(&bot_id),
                            "instagram",
                            "specific-default",
                            "private",
                            None,
                            &other.id,
                            true,
                            false,
                        )
                        .await
                        .unwrap();
                    }
                }
            },
        )
        .await;
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(CHANNELS)
                .count_documents(doc! {"channel_bot_id": &bot_id, "status": "active"})
                .await
                .unwrap(),
            0
        );
        if fault == "competing" {
            assert_eq!(
                routes::resolve_agent(&state.db, &bot_id, "new", None, OWNER)
                    .await
                    .unwrap()
                    .unwrap()
                    .api_key_id,
                other.id
            );
            assert!(
                key_service::get_api_key(&state.db, OWNER, &other.id)
                    .await
                    .is_ok()
            );
        }
    }
}

#[tokio::test]
async fn meta_default_reactivation_reconciles_defaults_and_check_rejects_legacy_ambiguity() {
    let (state, _calls, _server) = setup("meta_reactivate_defaults").await;
    let (router, token, agent, row) = assigned(&state, "instagram").await;
    let other = key(&state, "Replacement default").await;
    callback(
        &state,
        &other.id,
        "https://callbacks.example.test/replacement",
    )
    .await;
    let replacement = routes::create_conversation(
        &state.db,
        OWNER,
        Some(&row.channel_bot_id),
        "instagram",
        "different-default",
        "private",
        None,
        &other.id,
        true,
        false,
    )
    .await
    .unwrap();
    // Explicit default replacement deactivated the older default, retaining its flag.
    routes::update_conversation(
        &state.db,
        row.route_id.as_deref().unwrap(),
        OWNER,
        None,
        None,
        Some(true),
        None,
    )
    .await
    .unwrap();
    let defaults = state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME);
    assert_eq!(
        defaults
            .count_documents(doc! {"channel_bot_id": &row.channel_bot_id,
            "is_active": true, "default_agent": true})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        routes::resolve_agent(&state.db, &row.channel_bot_id, "new", None, OWNER)
            .await
            .unwrap()
            .unwrap()
            .api_key_id,
        row.route_api_key_id
    );
    assert_eq!(
        action(&router, &token, &row.channel_bot_id, &agent.id, "check")
            .await
            .0,
        StatusCode::OK
    );
    // Model older data written before the default reconciliation guard.
    defaults
        .update_one(
            doc! {"_id": &replacement.id},
            doc! {"$set": {"default_agent": true, "is_active": true}},
        )
        .await
        .unwrap();
    assert_eq!(
        action(&router, &token, &row.channel_bot_id, &agent.id, "check")
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        action(&router, &token, &row.channel_bot_id, &agent.id, "assign")
            .await
            .0,
        StatusCode::CONFLICT
    );
    let retained = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(retained.link_code_hash, row.link_code_hash);
    assert!(
        key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn meta_repair_preserves_rebound_nondefault_and_foreign_wildcards() {
    for fault in ["rebound", "other", "foreign"] {
        let (state, _calls, _server) = setup("meta_wildcard_competitor").await;
        let (router, token, agent, row) = assigned(&state, "messenger").await;
        let other = key(&state, "Unscoped owner route").await;
        callback(&state, &other.id, "https://callbacks.example.test/wildcard").await;
        if fault == "rebound" {
            routes::update_conversation(
                &state.db,
                row.route_id.as_deref().unwrap(),
                OWNER,
                Some(&other.id),
                Some(false),
                None,
                None,
            )
            .await
            .unwrap();
        } else {
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
            let competitor = routes::create_conversation(
                &state.db,
                OWNER,
                Some(&row.channel_bot_id),
                "messenger",
                "*",
                "private",
                None,
                &other.id,
                false,
                false,
            )
            .await
            .unwrap();
            if fault == "foreign" {
                // Retained foreign-owner tuple is still protected by the bot's unique route index.
                state
                    .db
                    .collection::<bson::Document>(
                        crate::models::channel_conversation::COLLECTION_NAME,
                    )
                    .update_one(
                        doc! {"_id": &competitor.id},
                        doc! {"$set": {"user_id": "former-owner"}},
                    )
                    .await
                    .unwrap();
            }
        }
        let (status, body) =
            action(&router, &token, &row.channel_bot_id, &agent.id, "assign").await;
        assert_eq!(status, StatusCode::CONFLICT, "{fault}: {body}");
        let retained = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(retained.status, "active");
        assert_eq!(retained.link_code_hash, row.link_code_hash);
        assert!(
            key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
                .await
                .is_ok()
        );
        assert!(
            key_service::get_api_key(&state.db, OWNER, &other.id)
                .await
                .is_ok()
        );
    }
}

#[tokio::test]
async fn meta_route_creation_preserves_storage_and_credential_error_categories() {
    for fault in ["revoked", "callback", "storage"] {
        let (state, _calls, _server) = setup("meta_route_error_category").await;
        let agent = specialist(&state).await;
        let bot = bot_doc("messenger", "Error category inbox");
        let bot_id = bot.get_str("_id").unwrap().to_owned();
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .insert_one(bot)
            .await
            .unwrap();
        let _ = app(&state).await;
        crate::services::durable_wake_tests::resume_after(
            "meta_default_before_insert",
            async {
                let error = connect_channel(State(state.clone()), crate::test_utils::test_auth_user(OWNER),
                    Json(ConnectRequest {bot_id: bot_id.clone(), agent_id: Some(agent.id.clone())}))
                    .await.err().unwrap();
                match fault {
                    "revoked" => assert!(matches!(error, AppError::NotFound(_)), "{error:?}"),
                    "callback" => assert!(matches!(error, AppError::Conflict(ref message) if message.contains("callback")), "{error:?}"),
                    _ => assert!(matches!(error, AppError::DatabaseError(_)), "{error:?}"),
                }
            },
            async {
                let pending = state.db.collection::<NyxbotChannel>(CHANNELS)
                    .find_one(doc! {"channel_bot_id": &bot_id, "status": "pending"}).await.unwrap().unwrap();
                match fault {
                    "callback" => callback(&state, &pending.route_api_key_id,
                        "https://callbacks.example.test/changed").await,
                    _ => { let patch = if fault == "revoked" {doc! {"is_active": false}}
                        else {doc! {"expires_at": "malformed historical date"}};
                        state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                            .update_one(doc! {"_id": &pending.route_api_key_id}, doc! {"$set": patch}).await.unwrap(); },
                }
            },
        ).await;
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(CHANNELS)
                .count_documents(doc! {"channel_bot_id": &bot_id, "status": "active"})
                .await
                .unwrap(),
            0
        );
    }
}
