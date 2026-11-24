use super::*;
use crate::{
    handlers::channel_bots,
    models::nyxbot_channel::COLLECTION_NAME as CHANNELS,
    services::assistant_authority_tests::{Fixture, orchestrator_fixture},
    test_utils::test_auth_user,
};
use axum::extract::{Path, Query, State};
use mongodb::bson::{self, Document};
use serde_json::{Value, json};
use uuid::Uuid;

async fn bot(f: &Fixture, platform: &str) -> ChannelBot {
    let bot: ChannelBot = bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"user_id":&f.owner,"platform":platform,
        "label":"Local Telegram health","platform_bot_id":"123","platform_bot_username":"local_bot",
        "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
        "webhook_secret_hash":"PRIVATE-SECRET","webhook_registered":true,
        "status":"active","is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).unwrap();
    f.state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    bot
}

async fn binding(f: &Fixture, bot: &ChannelBot, status: &str, code: &str, at: i64) -> String {
    let id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(CHANNELS)
        .insert_one(doc! {
            "_id":&id,"user_id":&f.owner,"channel_bot_id":&bot.id,"platform":"telegram",
            "transport":"gateway","status":status,"last_error":code,
            "created_at":bson::DateTime::from_millis(at),
            // Projection must discard these, including on unknown legacy errors.
            "agent_key_ciphertext":"PRIVATE-CIPHERTEXT","owner_sender_ids":["PRIVATE-SENDER"],
        })
        .await
        .unwrap();
    id
}

async fn surfaces(f: &Fixture, bot: &ChannelBot) -> (Value, Value) {
    let owner = channel_bots::health(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Path(bot.id.clone()),
    )
    .await
    .unwrap()
    .0;
    let owner = serde_json::to_value(owner).unwrap();
    let admin = channel_bots::admin_health(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Path(bot.id.clone()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(
        serde_json::to_value(admin).unwrap()["recommended_action_text"],
        owner["recommended_action_text"]
    );
    let page = channel_bots::admin_list(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Query(channel_bots::AdminChannelBotQuery::default()),
    )
    .await
    .unwrap()
    .0;
    let listed = page.into_iter().find(|row| row.id == bot.id).unwrap();
    assert_eq!(
        serde_json::to_value(listed).unwrap()["recommended_action_text"],
        owner["recommended_action_text"]
    );
    let result = f
        .tools()
        .execute(&f.auth, "nyxid__get_channel_bot", &json!({"bot_id":bot.id}))
        .await;
    assert!(!result.is_error, "{:?}", result.value);
    let tool = result.value["health"].clone();
    assert_eq!(tool["recommended_action"], owner["recommended_action"]);
    assert_eq!(
        tool["recommended_action_text"],
        owner["recommended_action_text"]
    );
    assert_eq!(tool["authorization"], "not_probed");
    assert_eq!(tool["automatic_reply"], "not_verified");
    assert!(!tool.to_string().contains("PRIVATE-"));
    (owner, tool)
}

async fn fixture() -> Fixture {
    let f = orchestrator_fixture("telegram_binding_health").await;
    crate::services::nyxbot_gateway_release_service::ensure_indexes(&f.state.db)
        .await
        .unwrap();
    crate::services::role_service::seed_system_roles(&f.state.db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.owner},
            doc! {"$set":{"role_ids":[roles.admin]}},
        )
        .await
        .unwrap();
    f
}

#[test]
fn telegram_failed_gateway_health_matches_owner_admin_and_account_tool() {
    crate::test_utils::run_native_journey_test(
        "telegram_failed_gateway_health_matches_owner_admin_and_account_tool",
        || async {
            let f = fixture().await;
            for platform in ["telegram-new", "telegram"] {
                let bot = bot(&f, platform).await;
                assert_eq!(
                    surfaces(&f, &bot).await.0["recommended_action_text"],
                    "Healthy."
                );
                for (at, code, action) in [
                    (1, "gateway_not_authenticated", "reconnect"),
                    (2, "gateway_permission_required", "contact_admin"),
                    (3, "gateway_configuration_required", "contact_admin"),
                    (4, "gateway_unavailable", "reconnect"),
                    (41, "gateway_rejected_request", "contact_admin"),
                    (42, "gateway_provider_not_registered", "contact_admin"),
                ] {
                    binding(&f, &bot, "failed", code, at).await;
                    let (owner, tool) = surfaces(&f, &bot).await;
                    assert_eq!(owner["status"], "active");
                    assert_eq!(owner["webhook_registered"], true);
                    assert!(owner["diagnostics"]["failure_reason"].is_null());
                    assert_eq!(owner["nyxbot_status"], "failed");
                    assert_eq!(owner["nyxbot_last_error"], code);
                    assert_eq!(tool["nyxbot_last_error"], code);
                    assert_eq!(owner["recommended_action"], action);
                    assert_eq!(owner["recommendation_cause"], code);
                    assert_eq!(tool["recommendation_cause"], code);
                    assert_eq!(
                        owner["recommended_action_text"],
                        format!(
                            "{code}: Last connection attempt: {} Retry after correcting the issue; health does not probe the gateway.",
                            crate::services::assistant_wake_service::text::gateway_error_guidance(
                                code
                            )
                        )
                    );
                }
                // Unknown/free-text history retains the existing fallback and never
                // copies that prose into the assistant's health projection.
                binding(&f, &bot, "failed", "PRIVATE-UPSTREAM-PROSE", 43).await;
                assert_eq!(
                    surfaces(&f, &bot).await.0["recommended_action_text"],
                    "Healthy."
                );
                // REST's established field keeps all selected stable setup failures;
                // the assistant's new field remains restricted to its closed codes.
                for (at, code) in [
                    (51, "gateway_invalid_response"),
                    (53, "route_create_failed"),
                ] {
                    let legacy = binding(&f, &bot, "failed", code, at).await;
                    let (owner, tool) = surfaces(&f, &bot).await;
                    assert_eq!(owner["nyxbot_last_error"], code);
                    assert!(tool["nyxbot_last_error"].is_null());
                    f.state
                        .db
                        .collection::<Document>(CHANNELS)
                        .delete_one(doc! {"_id":legacy})
                        .await
                        .unwrap();
                }
                binding(&f, &bot, "failed", "gateway_permission_required", 106).await;
                let live = binding(&f, &bot, "active", "gateway_not_authenticated", 107).await;
                // Live-first selection suppresses even a newer failed attempt.
                binding(&f, &bot, "failed", "gateway_permission_required", 108).await;
                let (owner, tool) = surfaces(&f, &bot).await;
                assert_eq!(owner["nyxbot_status"], "active");
                assert!(tool["nyxbot_last_error"].is_null());
                assert_eq!(owner["recommended_action"], "gateway");
                f.state
                    .db
                    .collection::<Document>(CHANNELS)
                    .update_one(doc! {"_id":&live}, doc! {"$set":{"status":"disconnected"}})
                    .await
                    .unwrap();
                binding(&f, &bot, "disconnected", "gateway_permission_required", 109).await;
                let (owner, tool) = surfaces(&f, &bot).await;
                assert_eq!(owner["nyxbot_status"], "disconnected");
                assert!(tool["nyxbot_last_error"].is_null());
                assert_eq!(owner["recommended_action_text"], "Healthy.");
                let pending = binding(&f, &bot, "pending", "gateway_not_authenticated", 110).await;
                assert_eq!(surfaces(&f, &bot).await.0["nyxbot_status"], "pending");
                f.state
                    .db
                    .collection::<Document>(CHANNELS)
                    .delete_one(doc! {"_id":pending})
                    .await
                    .unwrap();
                // Another bot's failure and a foreign owner cannot supply this bot's diagnosis.
                let foreign = binding(&f, &bot, "failed", "gateway_not_authenticated", 111).await;
                f.state
                    .db
                    .collection::<Document>(CHANNELS)
                    .update_one(
                        doc! {"_id":foreign},
                        doc! {"$set":{"user_id":Uuid::new_v4().to_string()}},
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    surfaces(&f, &bot).await.0["recommended_action_text"],
                    "Healthy."
                );
            }
            f.state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn telegram_failed_gateway_health_preserves_actionable_bot_faults_only() {
    crate::test_utils::run_native_journey_test(
        "telegram_failed_gateway_health_preserves_actionable_bot_faults_only",
        || async {
            let f = fixture().await;
            let bot = bot(&f, "telegram-new").await;
            binding(&f, &bot, "failed", "gateway_permission_required", 1).await;
            let bots = f
                .state
                .db
                .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
            for (reason, action, gateway) in [
                ("provider_unavailable", "contact_admin", true),
                ("telegram_text_rejected", "contact_admin", true),
                ("unknown_legacy_reason", "contact_admin", true),
            ] {
                bots.update_one(
                    doc! {"_id":&bot.id},
                    doc! {"$set":{"diagnostics.last_outbound_error":{
                    "code":"provider_error","reason":reason,"at":bson::DateTime::now()}}},
                )
                .await
                .unwrap();
                let (owner, _) = surfaces(&f, &bot).await;
                assert_eq!(owner["recommended_action"], action);
                assert_eq!(
                    owner["recommended_action_text"]
                        .as_str()
                        .unwrap()
                        .contains("gateway_permission_required"),
                    gateway
                );
            }
            bots.update_one(
                doc! {"_id":&bot.id},
                doc! {"$set":{"diagnostics.failure_reason":"auth_revoked"}},
            )
            .await
            .unwrap();
            let urgent = surfaces(&f, &bot).await;
            assert_eq!(urgent.0["recommended_action"], "reconnect");
            assert!(urgent.0["recommendation_cause"].is_null());
            assert!(urgent.1["recommendation_cause"].is_null());
            bots.update_one(doc! {"_id":&bot.id},doc! {"$unset":{"diagnostics.failure_reason":"","diagnostics.last_outbound_error":""},
        "$set":{"diagnostics.last_outbound_notice":{"code":"telegram_formatting_fallback","reason":"telegram_formatting_fallback","at":bson::DateTime::now()}}}).await.unwrap();
            assert!(
                surfaces(&f, &bot).await.0["recommended_action_text"]
                    .as_str()
                    .unwrap()
                    .contains("gateway_permission_required")
            );
            bots.update_one(
                doc! {"_id":&bot.id},
                doc! {"$set":{"is_active":false,"status":"suspended"}},
            )
            .await
            .unwrap();
            assert!(
                surfaces(&f, &bot).await.0["recommended_action_text"]
                    .as_str()
                    .unwrap()
                    .starts_with("gateway_permission_required:")
            );
            // A failed bot retains its existing credential recommendation.
            bots.update_one(doc! {"_id":&bot.id}, doc! {"$set":{"status":"failed"}})
                .await
                .unwrap();
            assert_eq!(
                surfaces(&f, &bot).await.0["recommended_action"],
                "reconnect"
            );
            f.state.db.drop().await.unwrap();
        },
    );
}

#[tokio::test]
async fn telegram_setup_evidence_without_health_index_filters_foreign_owner() {
    // Existing account-tool fixtures need not run NyxBot's index migration.
    let db = crate::test_utils::connect_transaction_test_database("telegram_health_no_index").await;
    let mut bot: ChannelBot = bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"user_id":Uuid::new_v4().to_string(),"platform":"telegram-new",
        "label":"Local bot","platform_bot_id":"123","platform_bot_username":"local_bot",
        "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
        "webhook_secret_hash":"PRIVATE-SECRET","webhook_registered":true,
        "status":"active","is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).unwrap();
    assert_eq!(
        super::super::setup_evidence(&db, &bot, false)
            .await
            .unwrap()["recommended_action_text"],
        "Healthy."
    );
    let rows = db.collection::<Document>(CHANNELS);
    for (owner, at, code) in [
        (bot.user_id.clone(), 1, "gateway_permission_required"),
        (Uuid::new_v4().to_string(), 2, "gateway_not_authenticated"),
    ] {
        rows.insert_one(
            doc! {"_id":Uuid::new_v4().to_string(),"user_id":owner,"channel_bot_id":&bot.id,
            "platform":"telegram","transport":"gateway","status":"failed","last_error":code,
            "created_at":bson::DateTime::from_millis(at)},
        )
        .await
        .unwrap();
    }
    let evidence = super::super::setup_evidence(&db, &bot, false)
        .await
        .unwrap();
    assert_eq!(evidence["nyxbot_last_error"], "gateway_permission_required");
    let indexes: Vec<_> = rows
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        indexes.len(),
        1,
        "discovery never creates or requires the health index"
    );
    // Older projected rows can omit platform/transport. The loaded bot owns
    // those facts, and a live legacy connection still suppresses failed history.
    rows.update_many(
        doc! {"channel_bot_id":&bot.id},
        doc! {"$unset":{"platform":"","transport":""}},
    )
    .await
    .unwrap();
    assert_eq!(
        super::super::setup_evidence(&db, &bot, false)
            .await
            .unwrap()["nyxbot_last_error"],
        "gateway_permission_required"
    );
    rows.insert_one(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&bot.user_id,"channel_bot_id":&bot.id,
        "status":"active","last_error":"gateway_not_authenticated","created_at":bson::DateTime::from_millis(0)}).await.unwrap();
    let recovered = super::super::setup_evidence(&db, &bot, false)
        .await
        .unwrap();
    assert_eq!(recovered["nyxbot_status"], "active");
    assert!(recovered["nyxbot_last_error"].is_null());
    bot.platform = "lark".into();
    assert_eq!(
        super::super::setup_evidence(&db, &bot, false)
            .await
            .unwrap()["recommended_action_text"],
        "Healthy."
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_health_query_keeps_indexed_sort_and_bounded_projection() {
    let commands = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = commands.clone();
    let handler = mongodb::event::EventHandler::callback(move |event| {
        if let mongodb::event::command::CommandEvent::Started(event) = event
            && event.command_name == "aggregate"
            && event.command.get_str("aggregate").ok() == Some(CHANNELS)
        {
            captured.lock().unwrap().push(event.command);
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "telegram_health_plan",
        handler,
    )
    .await;
    crate::services::nyxbot_gateway_release_service::ensure_indexes(&db)
        .await
        .unwrap();
    let mut bot: ChannelBot = bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"user_id":Uuid::new_v4().to_string(),"platform":"telegram-new",
        "label":"Local bot","platform_bot_id":"123","platform_bot_username":"local_bot",
        "bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
        "webhook_secret_hash":"PRIVATE-SECRET","webhook_registered":true,
        "status":"active","is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).unwrap();
    let rows = db.collection::<Document>(CHANNELS);
    for i in 0..33 {
        rows.insert_one(doc! {
            "_id":Uuid::new_v4().to_string(),
            "channel_bot_id": if i < 3 {bot.id.clone()} else {Uuid::new_v4().to_string()},
            "user_id": if i == 2 {Uuid::new_v4().to_string()} else {bot.user_id.clone()},
            "platform":"telegram","transport":"gateway","status": if i == 0 {"active"}else{"failed"},
            "last_error":"gateway_permission_required","created_at":bson::DateTime::from_millis(i),
            "agent_key_ciphertext":"PRIVATE-CIPHERTEXT","owner_sender_ids":["PRIVATE-SENDER"],
        }).await.unwrap();
    }
    for platform in ["telegram-new", "lark"] {
        bot.platform = platform.into();
        let loaded = latest_nyxbot_rows(&db, std::slice::from_ref(&bot))
            .await
            .unwrap();
        if platform == "telegram-new" {
            assert_eq!(loaded[&bot.id].status, "active");
        }
        let recorded = commands.lock().unwrap().last().unwrap().clone();
        let mut aggregate = Document::new();
        for field in ["aggregate", "pipeline", "cursor"] {
            aggregate.insert(field, recorded.get(field).unwrap().clone());
        }
        if platform == "lark" {
            assert!(
                aggregate.get_array("pipeline").unwrap()[0]
                    .as_document()
                    .unwrap()
                    .get_document("$match")
                    .unwrap()
                    .get("$or")
                    .is_none()
            );
        }
        let explain = db
            .run_command(doc! {"explain":aggregate,"verbosity":"executionStats"})
            .await
            .unwrap();
        let stages = explain.get_array("stages").unwrap();
        let cursor = stages[0]
            .as_document()
            .unwrap()
            .get_document("$cursor")
            .unwrap();
        let plan = cursor
            .get_document("queryPlanner")
            .unwrap()
            .get_document("winningPlan")
            .unwrap();
        let encoded = serde_json::to_string(plan).unwrap();
        assert!(encoded.contains("nyxbot_health_latest"), "{encoded}");
        assert!(!encoded.contains("COLLSCAN"), "{encoded}");
        assert!(!encoded.contains("\"SORT\""), "{encoded}");
        assert!(
            cursor
                .get_document("executionStats")
                .unwrap()
                .get_i32("totalDocsExamined")
                .unwrap()
                <= 3
        );
        let projection = recorded.get_array("pipeline").unwrap()[1]
            .as_document()
            .unwrap()
            .get_document("$project")
            .unwrap();
        assert!(!projection.contains_key("agent_key_ciphertext"));
        assert!(!projection.contains_key("owner_sender_ids"));
    }
    // The real maximum admin page: one bounded owner map and one indexed scan,
    // not an OR arm per bot. Include history and foreign rows under each bot.
    let mut page: Vec<_> = (0..500)
        .map(|_| {
            let mut b = bot.clone();
            b.platform = "telegram".into();
            b.id = Uuid::new_v4().to_string();
            b
        })
        .collect();
    let mut history = Vec::new();
    for b in &page {
        for i in 0..20 {
            history.push(doc!{"_id":Uuid::new_v4().to_string(),"channel_bot_id":&b.id,
        "user_id":if i==19 {"foreign"} else {&b.user_id},"status":"failed","platform":"telegram","transport":"gateway",
        "last_error":"gateway_rejected_request","created_at":bson::DateTime::from_millis(i)});
        }
    }
    rows.insert_many(history).await.unwrap();
    let started = std::time::Instant::now();
    assert_eq!(latest_nyxbot_rows(&db, &page).await.unwrap().len(), 500);
    eprintln!(
        "500 bots / 10,000 rows owner-scoped health read: {:?}",
        started.elapsed()
    );
    let recorded = commands.lock().unwrap().last().unwrap().clone();
    let pipeline = recorded.get_array("pipeline").unwrap();
    let filter = pipeline[0]
        .as_document()
        .unwrap()
        .get_document("$match")
        .unwrap();
    assert!(filter.get_document("$expr").is_ok());
    assert!(!filter.contains_key("$or"));
    let explain=db.run_command(doc!{"explain":{"aggregate":CHANNELS,"pipeline":pipeline,"cursor":{}},"verbosity":"executionStats"}).await.unwrap();
    let encoded = serde_json::to_string(&explain).unwrap();
    assert!(encoded.contains("nyxbot_health_latest"), "{encoded}");
    assert!(!encoded.contains("COLLSCAN"), "{encoded}");
    assert!(!encoded.contains("\"stage\":\"SORT\""), "{encoded}");
    eprintln!("500 bot query uses nyxbot_health_latest, no collection scan or blocking sort");
    for b in &mut page {
        b.ownership_version = 2;
    }
    let ids: Vec<_> = page.iter().map(|b| &b.id).collect();
    rows.update_many(
        doc! {"channel_bot_id":{"$in":ids}},
        doc! {"$set":{"bot_ownership_version":2_i64}},
    )
    .await
    .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(latest_nyxbot_rows(&db, &page).await.unwrap().len(), 500);
    eprintln!(
        "500 transferred bots / 10,000 rows health read: {:?}",
        started.elapsed()
    );
    let recorded = commands.lock().unwrap().last().unwrap().clone();
    let explain=db.run_command(doc!{"explain":{"aggregate":CHANNELS,"pipeline":recorded.get_array("pipeline").unwrap(),"cursor":{}},"verbosity":"executionStats"}).await.unwrap();
    let encoded = serde_json::to_string(&explain).unwrap();
    assert!(encoded.contains("nyxbot_health_latest"));
    assert!(!encoded.contains("COLLSCAN"), "{encoded}");
    assert!(!encoded.contains("\"stage\":\"SORT\""), "{encoded}");

    db.drop().await.unwrap();
}

#[test]
fn telegram_org_health_scopes_owner_transfer_and_legacy_active_routes() {
    crate::test_utils::run_native_journey_test("telegram_org_health_owner", || async {
        let f = fixture().await;
        let mut bot = bot(&f, "telegram").await;
        let org = Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &org,
                crate::models::user::UserType::Org,
            ))
            .await
            .unwrap();
        f.state
            .db
            .collection::<crate::models::org_membership::OrgMembership>(
                crate::models::org_membership::COLLECTION_NAME,
            )
            .insert_one(crate::test_utils::test_membership(
                &org,
                &f.owner,
                crate::models::org_membership::OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        bot.user_id = org.clone();
        f.state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id":&bot.id}, &bot)
            .await
            .unwrap();
        let rows = f.state.db.collection::<Document>(CHANNELS);
        let ours = binding(&f, &bot, "failed", "gateway_rejected_request", 1).await;
        rows.update_one(doc!{"_id":&ours},doc!{"$set":{"bot_owner_id":&org,"org_specialist":{"version":1,"org_id":&org,"revision":"local"}}}).await.unwrap();
        let foreign = binding(&f, &bot, "failed", "gateway_not_authenticated", 2).await;
        rows.update_one(
            doc! {"_id":&foreign},
            doc! {"$set":{"bot_owner_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
        let (owner, tool) = surfaces(&f, &bot).await;
        assert_eq!(owner["nyxbot_last_error"], "gateway_rejected_request");
        assert_eq!(tool["nyxbot_last_error"], "gateway_rejected_request");
        rows.update_one(doc! {"_id":&ours}, doc! {"$unset":{"bot_owner_id":""}})
            .await
            .unwrap();
        assert_eq!(
            surfaces(&f, &bot).await.0["nyxbot_last_error"],
            "gateway_rejected_request"
        );
        rows.update_one(doc! {"_id":&ours}, doc! {"$set":{"bot_owner_id":&org}})
            .await
            .unwrap();
        // Current org viewers see diagnostics but cannot run recovery actions.
        f.state
            .db
            .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(
                doc! {"org_user_id":&org,"member_user_id":&f.owner},
                doc! {"$set":{"role":"viewer"}},
            )
            .await
            .unwrap();
        let viewer = channel_bots::health(
            State(f.state.clone()),
            test_auth_user(&f.owner),
            Path(bot.id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert!(!viewer.can_write);
        assert_eq!(
            viewer.nyxbot_last_error.as_deref(),
            Some("gateway_rejected_request")
        );
        let denied = f
            .tools()
            .execute(&f.auth, "nyxid__get_channel_bot", &json!({"bot_id":bot.id}))
            .await;
        assert!(denied.is_error);
        assert_eq!(denied.value["error"], "channel_bot_not_found");
        f.state
            .db
            .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(
                doc! {"org_user_id":&org,"member_user_id":&f.owner},
                doc! {"$set":{"role":"admin"}},
            )
            .await
            .unwrap();
        // A transfer away and back must not revive failed history from the old generation.
        bot.ownership_version = 2;
        f.state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id":&bot.id}, &bot)
            .await
            .unwrap();
        assert!(surfaces(&f, &bot).await.0["nyxbot_status"].is_null());
        assert_eq!(surfaces(&f, &bot).await.0["recommended_action"], "none");
        assert_eq!(surfaces(&f, &bot).await.1["gateway"], false);

        rows.update_one(
            doc! {"_id":&ours},
            doc! {"$set":{"bot_ownership_version":0_i64}},
        )
        .await
        .unwrap();
        assert!(surfaces(&f, &bot).await.0["nyxbot_status"].is_null());
        // An old active connection created after transfer has no new version.
        // Its still-live owned route is the compatibility evidence.
        let route = Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .insert_one(
                doc! {"_id":&route,"user_id":&org,"channel_bot_id":&bot.id,"is_active":true},
            )
            .await
            .unwrap();
        let legacy = binding(&f, &bot, "active", "", 3).await;
        rows.update_one(
            doc! {"_id":&legacy},
            doc! {"$set":{"bot_owner_id":&org,"route_id":&route}},
        )
        .await
        .unwrap();
        assert_eq!(surfaces(&f, &bot).await.0["nyxbot_status"], "active");
        f.state
            .db
            .collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&route},
                doc! {"$set":{"is_active":false,"retired_by_transfer":true}},
            )
            .await
            .unwrap();
        assert!(surfaces(&f, &bot).await.0["nyxbot_status"].is_null());
        rows.update_one(
            doc! {"_id":&ours},
            doc! {"$set":{"bot_ownership_version":2_i64}},
        )
        .await
        .unwrap();
        assert_eq!(
            surfaces(&f, &bot).await.0["nyxbot_last_error"],
            "gateway_rejected_request"
        );
        // A row cannot name the current bot owner but a different specialist org.
        rows.update_one(
            doc! {"_id":&ours},
            doc! {"$set":{"org_specialist.org_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
        assert!(surfaces(&f, &bot).await.0["nyxbot_status"].is_null());
        // Public owner route rejects a user with no remaining membership.
        f.state
            .db
            .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .delete_many(doc! {"org_user_id":&org,"member_user_id":&f.owner})
            .await
            .unwrap();
        assert!(
            channel_bots::health(
                State(f.state.clone()),
                test_auth_user(&f.owner),
                Path(bot.id.clone())
            )
            .await
            .is_err()
        );
        // Platform admins retain their separate explicit inspection endpoint.
        assert!(
            channel_bots::admin_health(
                State(f.state.clone()),
                test_auth_user(&f.owner),
                Path(bot.id.clone())
            )
            .await
            .is_ok()
        );
        f.state.db.drop().await.unwrap();
    });
}
