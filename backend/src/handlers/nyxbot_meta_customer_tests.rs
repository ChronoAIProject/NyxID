//! First-party customer setup uses the same committed channel and guest policy
//! as ordinary NyxBot management, including existing installations.
use super::*;
use crate::handlers::channel_meta;
use crate::services::feature_flag_service as flags;
use tower::ServiceExt;

async fn app(state: &AppState) -> (Router, String) {
    flags::set_platform_override(
        &state.db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        OWNER,
    )
    .await
    .unwrap();
    let token = token_for(state, OWNER);
    (
        Router::new()
            .route(
                "/bots/{id}/customer-setup",
                post(channel_meta::customer_setup),
            )
            .with_state(state.clone()),
        token,
    )
}
fn token_for(state: &AppState, actor: &str) -> String {
    crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(actor).unwrap(),
        "openid",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap()
}
async fn action(
    app: &Router,
    token: &str,
    bot: &str,
    agent: &str,
    action: &str,
) -> (StatusCode, Value) {
    let response = Box::pin(
        app.clone().oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/bots/{bot}/customer-setup"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"agent_id":agent,"action":action}).to_string(),
                ))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    let status = response.status();
    let body = body_text(response).await;
    (status, serde_json::from_str(&body).unwrap())
}
async fn specialist(state: &AppState) -> crate::models::assistant_agent::AssistantAgent {
    specialist_named(state, "customer-support").await
}

async fn specialist_named(
    state: &AppState,
    name: &str,
) -> crate::models::assistant_agent::AssistantAgent {
    crate::services::assistant_team_service::create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        crate::services::assistant_team_service::CreateRequest {
            name: name.into(),
            description: "Customer support".into(),
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
    .unwrap()
    .0
}

async fn signed_customer_message(state: &AppState, channel_id: &str, sender: &str, text: &str) {
    let row = load_channel(state, OWNER, channel_id).await.unwrap();
    let id = Uuid::new_v4().to_string();
    let body = json!({
        "message_id":id, "correlation_id":format!("jti-{id}"), "platform":row.platform,
        "agent":{"api_key_id":row.route_api_key_id,"name":"support"},
        "conversation":{"id":"route","platform_id":sender,"type":"private"},
        "sender":{"platform_id":sender}, "content":{"type":"text","text":text},
        "timestamp":Utc::now().to_rfc3339(),
        "raw_platform_data":{"asset_id":"222","sender":{"id":sender.rsplit(':').next().unwrap()}},
    });
    let bytes = serde_json::to_vec(&body).unwrap();
    let proof = crate::crypto::jwt::generate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        &format!("jti-{id}"),
        &row.route_api_key_id,
        &id,
        &row.platform,
        &sha256_hex(&bytes),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-nyxid-callback-token", proof.parse().unwrap());
    let (_guard, completed) = watch_relay_callback(channel_id, &id);
    assert_eq!(
        relay_callback(
            State(state.clone()),
            Path(channel_id.to_owned()),
            headers,
            Bytes::from(bytes)
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    tokio::time::timeout(Duration::from_secs(20), completed)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn meta_personal_customer_setup_prepares_then_enables_without_rotating_on_readiness_checks() {
    for platform in ["messenger", "instagram"] {
        let (state, calls, server) = setup("meta_customer_personal").await;
        let agent = specialist(&state).await;
        let bot = bot_doc(platform, "Customer inbox");
        let bot_id = bot.get_str("_id").unwrap().to_owned();
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .insert_one(bot)
            .await
            .unwrap();
        let (router, token) = app(&state).await;
        let (status, assigned) = action(&router, &token, &bot_id, &agent.id, "assign").await;
        assert_eq!(status, StatusCode::OK, "{assigned}");
        assert_eq!(assigned["guests_enabled"], false);
        assert!(
            assigned["owner_link_code"]
                .as_str()
                .is_some_and(|code| !code.is_empty())
        );
        let channel_id = assigned["channel_id"].as_str().unwrap();
        let sender = format!("{platform}:{bot_id}:222:333");
        signed_customer_message(&state, channel_id, &sender, "ordinary first DM").await;
        assert!(
            load_channel(&state, OWNER, channel_id)
                .await
                .unwrap()
                .owner_sender_ids
                .is_empty()
        );
        let before = state
            .db
            .collection::<bson::Document>(CHANNELS)
            .find_one(doc! {"_id":channel_id})
            .await
            .unwrap()
            .unwrap();
        let (status, checked) = action(&router, &token, &bot_id, &agent.id, "check").await;
        assert_eq!(status, StatusCode::OK, "{checked}");
        assert!(checked["owner_link_code"].is_null());
        let after = state
            .db
            .collection::<bson::Document>(CHANNELS)
            .find_one(doc! {"_id":channel_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            before, after,
            "readiness must not mutate routing or owner verification"
        );
        let (status, failed) = action(&router, &token, &bot_id, &agent.id, "enable_guests").await;
        assert!(!status.is_success(), "{failed}");
        assert_eq!(
            load_channel(&state, OWNER, channel_id)
                .await
                .unwrap()
                .private_chats
                .as_deref()
                .unwrap_or("owner"),
            "owner"
        );
        let (status, prepared) = action(&router, &token, &bot_id, &agent.id, "prepare_model").await;
        assert_eq!(status, StatusCode::OK, "{prepared}");
        // PrepareModel schedules the existing hidden owner turn. Readiness
        // becomes ready only after its authenticated model callback commits a
        // usable route and the preparation job records that terminal result.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let (status, progress) = action(&router, &token, &bot_id, &agent.id, "check").await;
            assert_eq!(status, StatusCode::OK, "{progress}");
            if progress["readiness"]["guest_readiness"]["status"] == "ready" {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "model preparation did not commit readiness: {progress}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let (status, enabled) = action(&router, &token, &bot_id, &agent.id, "enable_guests").await;
        assert_eq!(status, StatusCode::OK, "{enabled}");
        assert_eq!(enabled["guests_enabled"], true);
        for visitor in ["444", "555"] {
            let customer = format!("{platform}:{bot_id}:222:{visitor}");
            signed_customer_message(&state, channel_id, &customer, "A private support question")
                .await;
        }
        let turns = calls.lock().await;
        assert_eq!(turns.len(), 2);
        for turn in turns.iter() {
            let contexts = turn_context_tests::contexts(turn);
            assert_eq!(contexts.len(), 1);
            assert_eq!(contexts[0]["identity"]["platform"], platform);
            assert_eq!(contexts[0]["sender"]["classification"], "guest");
            assert_eq!(turn["_test_channel_context"]["guest"], true);
        }
        drop(turns);
        let threads = channel_conversations(&state, channel_id).await;
        assert_eq!(threads.len(), 2);
        assert!(threads.iter().all(|thread| thread.guest_turn));
        assert_ne!(
            threads[0].channel.as_ref().unwrap().partition,
            threads[1].channel.as_ref().unwrap().partition
        );
        let (status, repeated) = action(&router, &token, &bot_id, &agent.id, "assign").await;
        assert_eq!(status, StatusCode::OK, "{repeated}");
        assert_eq!(repeated["guests_enabled"], true);
        assert!(repeated["owner_link_code"].is_null());
        assert_eq!(repeated["channel_id"], assigned["channel_id"]);
        let (status, renewed) = action(&router, &token, &bot_id, &agent.id, "owner_link").await;
        assert_eq!(status, StatusCode::OK, "{renewed}");
        assert_ne!(renewed["owner_link_code"], assigned["owner_link_code"]);
        assert_eq!(
            renewed["guests_enabled"], true,
            "owner verification must preserve public access"
        );
        signed_customer_message(
            &state,
            channel_id,
            &sender,
            renewed["owner_link_code"].as_str().unwrap(),
        )
        .await;
        let verified = load_channel(&state, OWNER, channel_id).await.unwrap();
        assert_eq!(verified.owner_sender_ids, [sender]);
        assert!(
            verified.link_code_hash.is_none(),
            "owner code is consumed once"
        );
        server.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn meta_org_customer_checks_preserve_committed_exposure_and_failed_enable_is_atomic() {
    for platform in ["messenger", "instagram"] {
        let (state, _, server) = setup("meta_customer_org").await;
        let (row, _, agent, org_id) = org_channel_tests::org_channel(&state, false).await;
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .update_one(
                doc! {"_id":&row.channel_bot_id},
                doc! {"$set":{"platform":platform}},
            )
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"platform":platform,"private_chats":"owner"}},
            )
            .await
            .unwrap();
        let (router, _) = app(&state).await;
        let other_admin = Uuid::new_v4().to_string();
        state
            .db
            .collection(USERS)
            .insert_one(test_user(&other_admin, UserType::Person))
            .await
            .unwrap();
        state
            .db
            .collection("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &org_id,
                &other_admin,
                crate::models::org_membership::OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        let token = token_for(&state, &other_admin);
        let before = state
            .db
            .collection::<bson::Document>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        for name in ["check", "assign"] {
            let (status, result) =
                action(&router, &token, &row.channel_bot_id, &agent.id, name).await;
            assert_eq!(status, StatusCode::OK, "{result}");
            assert_eq!(result["guests_enabled"], false);
            assert!(result["owner_link_code"].is_null());
        }
        let after = state
            .db
            .collection::<bson::Document>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before, after);
        let (status, result) = action(
            &router,
            &token,
            &row.channel_bot_id,
            &agent.id,
            "enable_guests",
        )
        .await;
        assert!(
            !status.is_success(),
            "unfunded org must remain closed: {result}"
        );
        let live = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(live.private_chats.as_deref(), Some("owner"));
        assert!(!live.org_specialist.as_ref().unwrap().guests.enabled);
        assert!(
            !action(
                &router,
                &token,
                &row.channel_bot_id,
                &agent.id,
                "owner_link"
            )
            .await
            .0
            .is_success()
        );
        let billing = crate::billing_integration_tests::billing_service(
            &state.db,
            Arc::new(crate::billing_integration_tests::FakeLago::default()),
            1000,
        );
        billing.ensure_wallet(&org_id).await.unwrap();
        let mut fresh = bot_doc(platform, "Fresh organization inbox");
        fresh.insert("user_id", &org_id);
        let fresh_id = fresh.get_str("_id").unwrap().to_owned();
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .insert_one(fresh)
            .await
            .unwrap();
        let preview = crate::services::org_channel_service::preview::load(
            &state.db,
            &state.encryption_keys,
            &other_admin,
            &fresh_id,
            &agent.id,
            None,
        )
        .await
        .unwrap();
        assert!(!preview.guest_enabled);
        assert_eq!(preview.private_chats, "owner");
        let (status, assigned) = action(&router, &token, &fresh_id, &agent.id, "assign").await;
        assert_eq!(status, StatusCode::OK, "{assigned}");
        assert_eq!(assigned["guests_enabled"], false);
        assert!(assigned["owner_link_code"].is_null());
        let fresh_channel = assigned["channel_id"].as_str().unwrap();
        // An already funded organization cannot accept customers through Assign.
        // The versioned ordinary defaults worker must preserve this explicit choice.
        crate::services::org_channel_service::defaults::apply(
            &state.db,
            fresh_channel,
            crate::services::org_channel_service::defaults::HistoricalRule::NoAuditedSave,
        )
        .await
        .unwrap();
        let closed = load_channel(&state, OWNER, fresh_channel).await.unwrap();
        assert_eq!(closed.private_chats.as_deref(), Some("owner"));
        assert!(!closed.org_specialist.as_ref().unwrap().guests.enabled);
        assert!(
            crate::services::org_channel_service::sender(
                &state.db,
                &closed,
                "unverified-customer",
                false,
                &agent.id,
            )
            .await
            .is_err()
        );
        for bot_id in [&row.channel_bot_id, &fresh_id] {
            let (status, enabled) =
                action(&router, &token, bot_id, &agent.id, "enable_guests").await;
            assert_eq!(status, StatusCode::OK, "{enabled}");
            assert_eq!(enabled["guests_enabled"], true);
            let (status, checked) = action(&router, &token, bot_id, &agent.id, "check").await;
            assert_eq!(status, StatusCode::OK, "{checked}");
            assert_eq!(checked["guests_enabled"], true);
            let committed = load_channel(&state, OWNER, checked["channel_id"].as_str().unwrap())
                .await
                .unwrap();
            assert_eq!(committed.private_chats.as_deref(), Some("everyone"));
            assert!(committed.org_specialist.as_ref().unwrap().guests.enabled);
            let before_repeat = committed
                .org_specialist
                .as_ref()
                .unwrap()
                .guests
                .revision
                .clone();
            let (status, repeated) =
                action(&router, &token, bot_id, &agent.id, "enable_guests").await;
            assert_eq!(status, StatusCode::OK, "{repeated}");
            assert_eq!(repeated["guests_enabled"], true);
            let after_repeat =
                load_channel(&state, OWNER, repeated["channel_id"].as_str().unwrap())
                    .await
                    .unwrap();
            assert_eq!(
                after_repeat
                    .org_specialist
                    .as_ref()
                    .unwrap()
                    .guests
                    .revision,
                before_repeat
            );
        }
        server.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn personal_meta_enable_refuses_assignment_changed_after_readiness_before_the_access_commit()
{
    for restore_first in [false, true] {
        let (state, _calls, _server) = setup("meta_enable_assignment_race").await;
        let first = specialist(&state).await;
        let next = specialist_named(&state, "replacement-support").await;
        let bot = bot_doc("instagram", "Customer inbox");
        let bot_id = bot.get_str("_id").unwrap().to_owned();
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .insert_one(bot)
            .await
            .unwrap();
        let (router, token) = app(&state).await;
        let (status, assigned) = action(&router, &token, &bot_id, &first.id, "assign").await;
        assert_eq!(status, StatusCode::OK, "{assigned}");
        let id = assigned["channel_id"].as_str().unwrap();
        let confirmed = load_channel(&state, OWNER, id).await.unwrap();
        let (status, prepared) = action(&router, &token, &bot_id, &first.id, "prepare_model").await;
        assert_eq!(status, StatusCode::OK, "{prepared}");
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let (status, progress) = action(&router, &token, &bot_id, &first.id, "check").await;
                assert_eq!(status, StatusCode::OK, "{progress}");
                if progress["readiness"]["guest_readiness"]["status"] == "ready" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        crate::services::durable_wake_tests::resume_after(
            "channel_access_before_confirmed_update",
            async {
                let (status, result) =
                    action(&router, &token, &bot_id, &first.id, "enable_guests").await;
                assert_eq!(status, StatusCode::CONFLICT, "{result}");
            },
            async {
                let (status, changed) = action(&router, &token, &bot_id, &next.id, "assign").await;
                assert_eq!(status, StatusCode::OK, "{changed}");
                if restore_first {
                    let (status, changed) =
                        action(&router, &token, &bot_id, &first.id, "assign").await;
                    assert_eq!(status, StatusCode::OK, "{changed}");
                }
            },
        )
        .await;
        let current = load_channel(&state, OWNER, id).await.unwrap();
        assert_eq!(
            current.agent_id.as_deref(),
            Some(if restore_first {
                first.id.as_str()
            } else {
                next.id.as_str()
            })
        );
        assert_eq!(current.private_chats.as_deref().unwrap_or("owner"), "owner");
        assert!(!current.private_chat_posts.unwrap_or(false));
        // Ordinary callers can still narrow access using a stale assignment
        // snapshot; only the explicit specialist-confirming setup action adds CAS.
        crate::services::nyxbot_chat_settings_service::update_as(
            &state.db,
            &confirmed,
            OWNER,
            &crate::services::nyxbot_chat_settings_service::AccessSettings {
                private_chats: Some("owner".into()),
                private_chat_posts: None,
                apply_to_existing: false,
            },
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn meta_assign_repairs_incomplete_routes_and_keys_then_preserves_healthy_retry_owner_codes() {
    for platform in ["messenger", "instagram"] {
        for fault in ["pending", "missing_route", "revoked_key"] {
            let (state, _calls, _server) = setup("meta_assignment_repair").await;
            let agent = specialist(&state).await;
            let bot = bot_doc(platform, "Repairable inbox");
            let bot_id = bot.get_str("_id").unwrap().to_owned();
            state
                .db
                .collection::<bson::Document>("channel_bots")
                .insert_one(bot)
                .await
                .unwrap();
            let (router, token) = app(&state).await;
            let (status, assigned) = action(&router, &token, &bot_id, &agent.id, "assign").await;
            assert_eq!(status, StatusCode::OK, "{assigned}");
            let prior = load_channel(&state, OWNER, assigned["channel_id"].as_str().unwrap())
                .await
                .unwrap();
            match fault {
                "pending" => {
                    state
                        .db
                        .collection::<bson::Document>(CHANNELS)
                        .update_one(
                            doc! {"_id": &prior.id},
                            doc! {"$set": {"status": "pending", "route_id": bson::Bson::Null}},
                        )
                        .await
                        .unwrap();
                }
                "missing_route" => {
                    state
                        .db
                        .collection::<bson::Document>(
                            crate::models::channel_conversation::COLLECTION_NAME,
                        )
                        .delete_one(doc! {"_id": prior.route_id.as_ref().unwrap()})
                        .await
                        .unwrap();
                }
                _ => {
                    state
                        .db
                        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                        .update_one(
                            doc! {"_id": &prior.route_api_key_id},
                            doc! {"$set": {"is_active": false}},
                        )
                        .await
                        .unwrap();
                }
            }
            let (status, repaired) = action(&router, &token, &bot_id, &agent.id, "assign").await;
            assert_eq!(status, StatusCode::OK, "{platform}/{fault}: {repaired}");
            let live = load_channel(&state, OWNER, repaired["channel_id"].as_str().unwrap())
                .await
                .unwrap();
            assert_ne!(live.id, prior.id);
            assert_eq!(live.agent_id.as_deref(), Some(agent.id.as_str()));
            assert!(
                repaired["owner_link_code"]
                    .as_str()
                    .is_some_and(|code| !code.is_empty())
            );
            assert!(
                crate::services::nyxbot_channel_query_service::reusable(
                    &state.db, OWNER, OWNER, &live
                )
                .await
                .unwrap()
            );
            assert_eq!(
                load_channel(&state, OWNER, &prior.id).await.unwrap().status,
                "disconnected"
            );
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(
                        crate::models::channel_conversation::COLLECTION_NAME
                    )
                    .count_documents(
                        doc! {"channel_bot_id": &bot_id, "is_active": true, "default_agent": true}
                    )
                    .await
                    .unwrap(),
                1
            );
            let hash = live.link_code_hash.clone();
            let (status, repeated) = action(&router, &token, &bot_id, &agent.id, "assign").await;
            assert_eq!(status, StatusCode::OK, "{repeated}");
            assert_eq!(repeated["channel_id"], live.id);
            assert!(repeated["owner_link_code"].is_null());
            assert_eq!(
                load_channel(&state, OWNER, &live.id)
                    .await
                    .unwrap()
                    .link_code_hash,
                hash
            );
        }
    }
}

#[tokio::test]
async fn meta_assignment_key_storage_error_preserves_existing_route_and_owner_link() {
    let (state, _calls, _server) = setup("meta_repair_key_storage_error").await;
    let agent = specialist(&state).await;
    let bot = bot_doc("instagram", "Healthy inbox");
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .insert_one(bot)
        .await
        .unwrap();
    let (router, token) = app(&state).await;
    let (status, assigned) = action(&router, &token, &bot_id, &agent.id, "assign").await;
    assert_eq!(status, StatusCode::OK, "{assigned}");
    let prior = load_channel(&state, OWNER, assigned["channel_id"].as_str().unwrap())
        .await
        .unwrap();
    let keys = state.db.collection::<bson::Document>("api_keys");
    let key = keys
        .find_one(doc! {"_id": &prior.route_api_key_id})
        .await
        .unwrap()
        .unwrap();
    keys.update_one(
        doc! {"_id": &prior.route_api_key_id},
        doc! {"$set": {"created_at": "invalid-storage-date"}},
    )
    .await
    .unwrap();
    for name in ["check", "assign"] {
        let (status, result) = action(&router, &token, &bot_id, &agent.id, name).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{result}");
        let current = load_channel(&state, OWNER, &prior.id).await.unwrap();
        assert_eq!(current.status, "active");
        assert_eq!(current.route_id, prior.route_id);
        assert_eq!(current.link_code_hash, prior.link_code_hash);
        assert!(
            keys.find_one(doc! {"_id": &prior.route_api_key_id, "is_active": true})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .db
                .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .find_one(doc! {"_id": &prior.route_id, "is_active": true})
                .await
                .unwrap()
                .is_some()
        );
    }
    keys.replace_one(doc! {"_id": &prior.route_api_key_id}, key)
        .await
        .unwrap();
    let (status, retried) = action(&router, &token, &bot_id, &agent.id, "assign").await;
    assert_eq!(status, StatusCode::OK, "{retried}");
    assert_eq!(retried["channel_id"], prior.id);
    assert!(retried["owner_link_code"].is_null());
    assert_eq!(
        load_channel(&state, OWNER, &prior.id)
            .await
            .unwrap()
            .link_code_hash,
        prior.link_code_hash
    );
}

#[tokio::test]
async fn meta_assignment_repair_fences_activation_and_acting_org_admin_at_retirement() {
    for demotion in [false, true] {
        let (state, _calls, _server) = setup("meta_repair_retirement_race").await;
        let (prior, _, agent, org) = org_channel_tests::org_channel(&state, false).await;
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .update_one(
                doc! {"_id": &prior.channel_bot_id},
                doc! {"$set": {"platform": "instagram"}},
            )
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id": &prior.id},
                doc! {"$set": {"platform": "instagram", "status": "pending"}},
            )
            .await
            .unwrap();
        let second = Uuid::new_v4().to_string();
        state
            .db
            .collection(USERS)
            .insert_one(test_user(&second, UserType::Person))
            .await
            .unwrap();
        let membership = crate::test_utils::test_membership(
            &org,
            &second,
            crate::models::org_membership::OrgRole::Admin,
            None,
        );
        state
            .db
            .collection::<crate::models::org_membership::OrgMembership>("org_memberships")
            .insert_one(&membership)
            .await
            .unwrap();
        let (router, _) = app(&state).await;
        let token = token_for(&state, &second);
        crate::services::durable_wake_tests::resume_after(
            "channel_repair_before_retirement",
            async {
                let (status, result) =
                    action(&router, &token, &prior.channel_bot_id, &agent.id, "assign").await;
                assert_eq!(
                    status,
                    if demotion {
                        StatusCode::FORBIDDEN
                    } else {
                        StatusCode::CONFLICT
                    },
                    "{result}"
                );
            },
            async {
                if demotion {
                    crate::services::org_service::update_membership(
                        &state.db,
                        &membership.id,
                        Some(crate::models::org_membership::OrgRole::Member),
                        None,
                        None,
                    )
                    .await
                    .unwrap();
                } else {
                    crate::services::nyxbot_channel_continuity_service::activate(
                        &state.db,
                        OWNER,
                        &prior.id,
                        prior.route_id.as_deref().unwrap(),
                        None,
                        false,
                        "activated-owner-code",
                    )
                    .await
                    .unwrap();
                }
            },
        )
        .await;
        let current = load_channel(&state, OWNER, &prior.id).await.unwrap();
        assert_eq!(current.status, if demotion { "pending" } else { "active" });
        assert_eq!(current.route_api_key_id, prior.route_api_key_id);
        assert_eq!(current.route_id, prior.route_id);
        assert!(
            state
                .db
                .collection::<bson::Document>("api_keys")
                .find_one(doc! {"_id": &prior.route_api_key_id, "is_active": true})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .db
                .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .find_one(doc! {"_id": &prior.route_id, "is_active": true})
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(CHANNELS)
                .count_documents(doc! {"channel_bot_id": &prior.channel_bot_id})
                .await
                .unwrap(),
            1
        );
    }
}
