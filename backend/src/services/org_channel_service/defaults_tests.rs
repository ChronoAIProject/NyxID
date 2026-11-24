use super::super::defaults::{self, ChoiceEvidence, HistoricalRule};
use super::*;
use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use serde_json::json;

async fn reload(f: &mut Fixture) {
    f.channel = management::load(&f.org.state.db, &f.org.admin, &f.channel.id, true)
        .await
        .unwrap();
}
async fn ready(f: &mut Fixture) {
    f.org.readiness.abort();
    f.org.readiness =
        crate::services::security_readiness_service::controlled_ready_test_replica(&f.org.state.db)
            .await;
}
async fn snapshot(db: &Database) -> std::collections::BTreeMap<String, Vec<Document>> {
    let mut result = std::collections::BTreeMap::new();
    for name in db.list_collection_names().await.unwrap() {
        let rows = db
            .collection::<Document>(&name)
            .find(doc! {})
            .sort(doc! {"_id":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        result.insert(name, rows);
    }
    result
}

#[test]
fn org_guest_default_historical_choice_has_one_explicit_selector() {
    let policy = GuestPolicy::default();
    for (rule, evidence, expected) in [
        (
            HistoricalRule::PreserveOff,
            ChoiceEvidence::Untouched,
            false,
        ),
        (
            HistoricalRule::NoAuditedSave,
            ChoiceEvidence::Untouched,
            true,
        ),
        (HistoricalRule::NoAuditedSave, ChoiceEvidence::Chosen, false),
        (
            HistoricalRule::NoAuditedSave,
            ChoiceEvidence::Unknown,
            false,
        ),
    ] {
        assert_eq!(
            defaults::switch_guest_access_on(rule, &policy, evidence),
            expected
        );
    }
    for ready in [false, true] {
        let fresh = new_guest_policy(ready);
        assert!(fresh.enabled);
        assert!(!fresh.revision.is_empty());
        assert_eq!(fresh.follows_agent_grants, ready);
        assert!(!defaults::switch_guest_access_on(
            HistoricalRule::NoAuditedSave,
            &fresh,
            ChoiceEvidence::Untouched
        ));
    }
}

#[tokio::test]
async fn org_guest_defaults_two_replicas_switch_once_and_old_replacements_do_not_duplicate_audit_or_notice()
 {
    let mut f = Fixture::new("guest_defaults_race").await;
    let db = f.org.state.db.clone();
    db.collection::<Document>("nyxbot_channels").update_one(doc! {"_id":&f.channel.id},
        doc! {"$unset":{"private_chats":""},"$set":{"org_specialist.guests.service_ids":["frozen"]}}).await.unwrap();
    let (a, b) = tokio::join!(
        defaults::sweep_with_rule(&db, HistoricalRule::PreserveOff),
        defaults::sweep_with_rule(&db, HistoricalRule::PreserveOff)
    );
    a.unwrap();
    b.unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(f.channel.private_chats.as_deref(), Some("everyone"));
    assert_eq!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .service_ids,
        vec!["frozen"]
    );
    assert!(
        !f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .follows_agent_grants
    );
    defaults::sweep_with_rule(&db, HistoricalRule::PreserveOff)
        .await
        .unwrap();
    let audits = db.collection::<Document>("audit_log");
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"org_channel_guest_defaults_applied"})
            .await
            .unwrap(),
        1
    );
    let audit = audits
        .find_one(doc! {"event_type":"org_channel_guest_defaults_applied"})
        .await
        .unwrap()
        .unwrap();
    for field in ["seq", "prev_hash", "entry_hash"] {
        assert!(audit.contains_key(field));
    }
    // An old replacement can drop additive markers. Completion prevents replay.
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    defaults::sweep_with_rule(&db, HistoricalRule::PreserveOff)
        .await
        .unwrap();
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"org_channel_guest_defaults_applied"})
            .await
            .unwrap(),
        1
    );
    let (tx, mut notices) = tokio::sync::mpsc::unbounded_channel();
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .insert(f.bot.id.clone(), tx);
    let stale_old_writer = f.channel.clone();
    let (a, b) = tokio::join!(
        defaults::apply(&db, &f.channel.id, HistoricalRule::NoAuditedSave),
        defaults::apply(&db, &f.channel.id, HistoricalRule::NoAuditedSave)
    );
    a.unwrap();
    b.unwrap();
    reload(&mut f).await;
    // Lose the additive hint before submission, as an old serde replacement would.
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        defaults::sweep_with_rule(&db, HistoricalRule::NoAuditedSave),
        defaults::sweep_with_rule(&db, HistoricalRule::NoAuditedSave)
    );
    a.unwrap();
    b.unwrap();
    reload(&mut f).await;
    assert!(f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .service_ids,
        vec!["frozen"]
    );
    let (recipient, notice) =
        tokio::time::timeout(std::time::Duration::from_secs(5), notices.recv())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(recipient, f.org.admin);
    assert!(notice.recommended_action_text.contains("Allow guests off"));
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .replace_one(doc! {"_id":&f.channel.id}, &stale_old_writer)
        .await
        .unwrap();
    defaults::sweep_with_rule(&db, HistoricalRule::NoAuditedSave)
        .await
        .unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert!(notices.try_recv().is_err());
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"org_channel_guest_defaults_applied"})
            .await
            .unwrap(),
        2
    );
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .remove(&f.bot.id);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_settings_save_audit_keeps_off_and_explicit_private_owner() {
    let mut f = Fixture::new("guest_defaults_choices").await;
    let db = f.org.state.db.clone();
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"private_chats":"owner"}},
        )
        .await
        .unwrap();
    crate::services::audit_service::log_actor_event(
        db.clone(),
        &crate::services::audit_service::AuditActor {
            user_id: f.org.admin.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "org_channel_guest_policy_changed",
        Some(json!({"channel_id":f.channel.id})),
    )
    .await
    .unwrap();
    let plan = db.run_command(doc! {"explain":{"find":"audit_log","filter":{"event_type":"org_channel_guest_policy_changed","event_data.channel_id":&f.channel.id},"hint":"event_type_1_created_at_-1"},"verbosity":"executionStats"}).await.unwrap();
    assert_eq!(
        plan.get_document("executionStats")
            .unwrap()
            .get_i32("totalDocsExamined")
            .unwrap(),
        1
    );
    defaults::sweep_with_rule(&db, HistoricalRule::NoAuditedSave)
        .await
        .unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(f.channel.private_chats.as_deref(), Some("owner"));
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"org_channel_guest_defaults_applied"})
            .await
            .unwrap(),
        0
    );
    // A predecessor save preserves Off; an absent private audience is still untouched.
    let mut restored = f.channel.clone();
    restored.id = uuid::Uuid::new_v4().to_string();
    restored.private_chats = None;
    restored
        .org_specialist
        .as_mut()
        .unwrap()
        .guests
        .continuity_channel_id = Some(f.channel.id.clone());
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .insert_one(&restored)
        .await
        .unwrap();
    defaults::sweep_with_rule(&db, HistoricalRule::NoAuditedSave)
        .await
        .unwrap();
    let saved = management::load(&db, &f.org.admin, &restored.id, true)
        .await
        .unwrap();
    assert_eq!(saved.private_chats.as_deref(), Some("everyone"));
    assert!(!saved.org_specialist.unwrap().guests.enabled);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_explicit_off_mints_revision_but_omitted_enabled_does_not() {
    let mut f = Fixture::new("guest_default_save_off").await;
    ready(&mut f).await;
    let db = f.org.state.db.clone();
    management::patch(
        &db,
        &f.org.admin,
        &f.channel,
        serde_json::from_value(json!({"daily_credits":"1","expected_revision":""})).unwrap(),
    )
    .await
    .unwrap();
    reload(&mut f).await;
    assert!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .revision
            .is_empty()
    );
    management::patch(
        &db,
        &f.org.admin,
        &f.channel,
        serde_json::from_value(json!({"enabled":false,"expected_revision":""})).unwrap(),
    )
    .await
    .unwrap();
    reload(&mut f).await;
    assert!(
        !f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .revision
            .is_empty()
    );
    defaults::sweep_with_rule(&db, HistoricalRule::NoAuditedSave)
        .await
        .unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_preview_is_metadata_only_and_performs_no_writes() {
    let mut f = Fixture::new("guest_default_preview").await;
    ready(&mut f).await;
    let db = f.org.state.db.clone();
    db.collection::<Document>("nyxbot_channels")
        .delete_many(doc! {})
        .await
        .unwrap();
    crate::services::feature_flag_service::set_platform_override(
        &db,
        crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();
    // Freeze the independent heartbeat writer after establishing readiness.
    f.org.readiness.abort();
    let _ = (&mut f.org.readiness).await;
    let before = snapshot(&db).await;
    let facts = preview::load(
        &db,
        &f.org.state.encryption_keys,
        &f.org.state.config.base_url,
        &f.org.admin,
        &f.bot.id,
        &f.agent.id,
        None,
    )
    .await
    .unwrap();
    assert!(facts.guest_enabled);
    assert_eq!(facts.private_chats, "everyone");
    assert!(facts.summary().contains("organization pays"));
    assert_eq!(facts.health.as_ref().unwrap().action, "enable_billing");
    for actor in [&f.org.member, &f.org.viewer, &f.org.outsider] {
        assert!(
            preview::load(
                &db,
                &f.org.state.encryption_keys,
                &f.org.state.config.base_url,
                actor,
                &f.bot.id,
                &f.agent.id,
                None
            )
            .await
            .is_err()
        );
    }
    let Json(response) = crate::handlers::org_specialist_channels::preview(
        State(f.org.state.clone()),
        crate::test_utils::test_auth_user(&f.org.admin),
        Json(crate::handlers::org_specialist_channels::PreviewRequest {
            bot_id: f.bot.id.clone(),
            agent_id: f.agent.id.clone(),
            channel_id: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(response["guest_enabled"], true);
    use tower::ServiceExt;
    let token = crate::crypto::jwt::generate_access_token(
        &f.org.state.jwt_keys,
        &f.org.state.config,
        &uuid::Uuid::parse_str(&f.org.admin).unwrap(),
        "openid",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, app) = crate::routes::build_router();
    let response = Box::pin(
        app.with_state(f.org.state.clone()).oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/assistant/nyxagent/channels/preview")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"bot_id":f.bot.id,"agent_id":f.agent.id}).to_string(),
                ))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(before, snapshot(&db).await);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_preview_matches_reuse_retarget_and_recovery() {
    let mut f = Fixture::new("guest_default_preview_selection").await;
    ready(&mut f).await;
    let db = f.org.state.db.clone();
    crate::services::feature_flag_service::set_platform_override(
        &db,
        crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();
    let mut failed = f.channel.clone();
    failed.id = uuid::Uuid::new_v4().to_string();
    failed.status = "failed".into();
    failed.created_at += chrono::Duration::seconds(1);
    failed.org_specialist.as_mut().unwrap().guests = new_guest_policy(true);
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .insert_one(&failed)
        .await
        .unwrap();
    // Healthy live Off wins over the newer failed On attempt, as connect does.
    let facts = preview::load(
        &db,
        &f.org.state.encryption_keys,
        &f.org.state.config.base_url,
        &f.org.admin,
        &f.bot.id,
        &f.agent.id,
        None,
    )
    .await
    .unwrap();
    assert!(!facts.guest_enabled);
    assert_eq!(facts.guest_policy_mode, "legacy");
    let mut target = f.agent.clone();
    target.id = uuid::Uuid::new_v4().to_string();
    target.name = "other-specialist".into();
    db.collection::<AssistantAgent>("assistant_agents")
        .insert_one(&target)
        .await
        .unwrap();
    let facts = preview::load(
        &db,
        &f.org.state.encryption_keys,
        &f.org.state.config.base_url,
        &f.org.admin,
        &f.bot.id,
        &target.id,
        Some(&f.channel.id),
    )
    .await
    .unwrap();
    assert!(!facts.guest_enabled);
    assert_eq!(facts.guest_policy_mode, "grants");
    Box::pin(crate::handlers::nyxbot::link(
        &f.org.state,
        &f.org.admin,
        &f.channel.id,
        &target,
    ))
    .await
    .unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .follows_agent_grants
    );
    // With the live transport broken, connect restores from the newest attempt.
    db.collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.channel.route_api_key_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    let facts = preview::load(
        &db,
        &f.org.state.encryption_keys,
        &f.org.state.config.base_url,
        &f.org.admin,
        &f.bot.id,
        &f.agent.id,
        None,
    )
    .await
    .unwrap();
    assert!(facts.guest_enabled);
    assert_eq!(facts.guest_policy_mode, "grants");
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_member_money_and_existing_detail_contracts_are_preserved() {
    let f = Fixture::new("guest_default_member_projection").await;
    let db = &f.org.state.db;
    let admin = management::view(db, &f.org.admin, &f.channel)
        .await
        .unwrap();
    let member = management::view(db, &f.org.member, &f.channel)
        .await
        .unwrap();
    for field in [
        "daily_credits",
        "default_daily_credits",
        "spent_today_credits",
        "reserved_today_credits",
    ] {
        assert!(admin.get(field).is_some(), "{field}");
        assert_eq!(member.get(field), admin.get(field), "{field}");
    }
    assert_eq!(member["can_manage"], false);
    for actor in [&f.org.admin, &f.org.member, &f.org.viewer] {
        let auth = crate::test_utils::test_auth_user(actor);
        let Json(detail) = crate::handlers::channel_bots::get_bot(
            State(f.org.state.clone()),
            auth.clone(),
            Path(f.bot.id.clone()),
        )
        .await
        .unwrap();
        let Json(health) = crate::handlers::channel_bots::health(
            State(f.org.state.clone()),
            auth,
            Path(f.bot.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(health.can_write, actor == &f.org.admin);
        let detail = serde_json::to_value(detail).unwrap();
        assert!(detail.get("setup_instructions").is_some());
        assert!(detail.get("conversations_count").is_some());
    }
    assert!(
        management::load(db, &f.org.viewer, &f.channel.id, false)
            .await
            .is_err()
    );
    crate::services::feature_flag_service::set_platform_override(
        db,
        crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();
    db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id":&f.org.org,"member_user_id":&f.org.admin},
            doc! {"$set":{"role":"viewer"}},
        )
        .await
        .unwrap();
    let Json(own) = crate::handlers::nyxbot::list_channels(
        State(f.org.state.clone()),
        crate::test_utils::test_auth_user(&f.org.admin),
    )
    .await
    .unwrap();
    assert_eq!(own["channel_agents"][0]["id"], f.channel.id);
    assert!(
        own["channel_agents"][0]["org_specialist"]
            .get("daily_credits")
            .is_some(),
        "existing self-owned transport inventory is preserved for Viewer"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_new_binding_needs_no_wallet_and_recovers_saved_off() {
    Box::pin(async {
        let mut f = Fixture::new("guest_default_fresh_billing").await;
        ready(&mut f).await;
        let db = f.org.state.db.clone();
        db.collection::<Document>("nyxbot_channels").delete_many(doc! {}).await.unwrap();
        db.collection::<Document>("channel_conversations").delete_many(doc! {}).await.unwrap();
        let (fresh,_) = Box::pin(crate::handlers::nyxbot::connect(&f.org.state,&f.org.admin,None,&f.bot.id,&f.agent)).await.unwrap();
        assert!(fresh.org_specialist.as_ref().unwrap().guests.enabled);
        assert!(!fresh.org_specialist.as_ref().unwrap().guests.revision.is_empty());
        assert_eq!(fresh.private_chats.as_deref(),Some("everyone"));
        f.channel = fresh;
        db.collection::<Document>("nyxbot_threads").insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"channel_id":&f.channel.id,"user_id":&f.org.admin,"partition":"private","kind":"private","platform_chat_id":"chat","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        let origin = f.guest("visitor").await;
        let mut tx = db.client().start_session().await.unwrap();
        assert!(matches!(validate_in_session(&db,&mut tx,&origin,&f.org.org,&f.agent.id,false).await,Err(AppError::BillingNotConfigured(_))));
        for collection in [crate::models::org_channel::HOLDS,crate::models::usage_meter::COLLECTION_NAME] {
            assert_eq!(db.collection::<Document>(collection).count_documents(doc! {}).await.unwrap(),0);
        }
        let Json(health) = crate::handlers::channel_bots::health(State(f.org.state.clone()),crate::test_utils::test_auth_user(&f.org.admin),Path(f.bot.id.clone())).await.unwrap();
        assert_eq!(health.recommended_action,"enable_billing");
        let mut facts = crate::services::channel_bot_health::HealthFacts::for_bot(&f.bot,chrono::Utc::now(),true);
        facts.org_guest_billing = Some("org_wallet_not_provisioned");
        assert_eq!(crate::services::channel_bot_health::recommend(&facts).action,"enable_billing");
        // Recovery retains On while billing is absent, then the next admission works.
        crate::handlers::nyxbot::disconnect(&f.org.state,&f.org.admin,&f.channel.id).await.unwrap();
        let (rebuilt,_) = Box::pin(crate::handlers::nyxbot::connect(&f.org.state,&f.org.admin,None,&f.bot.id,&f.agent)).await.unwrap();
        assert!(rebuilt.org_specialist.as_ref().unwrap().guests.enabled);
        // Validate the fresh connection independently of recovery's new identity.
        f.channel = rebuilt;
        db.collection::<Document>("nyxbot_threads").insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"channel_id":&f.channel.id,"user_id":&f.org.admin,"partition":"private","kind":"private","platform_chat_id":"chat","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        let origin = f.guest("visitor").await;
        assert!(matches!(validate_in_session(&db,&mut tx,&origin,&f.org.org,&f.agent.id,false).await,Err(AppError::BillingNotConfigured(_))));
        let carried = f.channel.org_specialist.as_ref().unwrap().guests.clone();
        assert!(carried.continuity_channel_id.is_some());
        defaults::sweep(&db).await.unwrap();
        reload(&mut f).await;
        assert_eq!(f.channel.org_specialist.as_ref().unwrap().guests,carried);
        let Json(health) = crate::handlers::channel_bots::health(State(f.org.state.clone()),crate::test_utils::test_auth_user(&f.org.admin),Path(f.bot.id.clone())).await.unwrap();
        assert_eq!(health.recommended_action,"enable_billing");
        f.wallet().await;
        validate_in_session(&db,&mut tx,&origin,&f.org.org,&f.agent.id,false).await.unwrap();
        let threads = db.collection::<Document>("nyxbot_threads");
        threads.update_one(doc! {"channel_id":&f.channel.id,"partition":"private"},doc! {"$set":{"kind":"group","owner_seen":false}}).await.unwrap();
        assert!(validate_in_session(&db,&mut tx,&origin,&f.org.org,&f.agent.id,false).await.is_err());
        threads.update_one(doc! {"channel_id":&f.channel.id,"partition":"private"},doc! {"$set":{"owner_seen":true}}).await.unwrap();
        validate_in_session(&db,&mut tx,&origin,&f.org.org,&f.agent.id,false).await.unwrap();
        management::update(&db,&f.org.admin,&f.channel,serde_json::from_value(json!({"enabled":false})).unwrap()).await.unwrap();
        reload(&mut f).await;
        db.collection::<Document>("billing_wallet").delete_many(doc! {}).await.unwrap();
        assert!(management::update(&db,&f.org.admin,&f.channel,serde_json::from_value(json!({"enabled":true})).unwrap()).await.is_err());
        crate::handlers::nyxbot::disconnect(&f.org.state,&f.org.admin,&f.channel.id).await.unwrap();
        let (restored,_) = Box::pin(crate::handlers::nyxbot::connect(&f.org.state,&f.org.admin,None,&f.bot.id,&f.agent)).await.unwrap();
        assert!(!restored.org_specialist.as_ref().unwrap().guests.enabled);
        db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn org_guest_defaults_member_chats_filter_before_limit_and_single_chat_mutations_stay_denied()
{
    Box::pin(async {
        let f = Fixture::new("guest_default_member_chats").await;
        let db = &f.org.state.db;
        crate::services::feature_flag_service::set_platform_override(
            db,
            crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            &f.org.admin,
        )
        .await
        .unwrap();
        let threads = db.collection::<Document>("nyxbot_threads");
        let base = threads
            .find_one(doc! {"channel_id":&f.channel.id})
            .await
            .unwrap()
            .unwrap();
        let mut hidden = Vec::new();
        for i in 0..160 {
            let mut row = base.clone();
            row.insert("_id", format!("private-{i}"));
            row.insert("partition", format!("private-{i}"));
            if i % 2 == 0 {
                row.remove("kind");
            }
            row.insert("conversation_id", "secret-lane");
            hidden.push(row);
        }
        threads.insert_many(hidden).await.unwrap();
        let mut public = base.clone();
        public.insert("_id", "public-group");
        public.insert("partition", "public-group");
        public.insert("kind", "group");
        public.insert("conversation_id", "secret-group-lane");
        threads.insert_one(public).await.unwrap();
        let auth = crate::test_utils::test_auth_user(&f.org.member);
        let Json(first) = crate::handlers::nyxbot::list_channel_chats(
            State(f.org.state.clone()),
            auth.clone(),
            Path(f.channel.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(first["chats"].as_array().unwrap().len(), 1);
        assert_eq!(first["chats"][0]["id"], "public-group");
        for field in [
            "conversation_id",
            "parent_chat_id",
            "last_message_at",
            "followed_thread_count",
            "has_thread_history",
        ] {
            assert!(first["chats"][0].get(field).is_none(), "{field}");
        }
        threads
            .delete_many(doc! {"kind":{"$nin":["group","channel"]}})
            .await
            .unwrap();
        let Json(second) = crate::handlers::nyxbot::list_channel_chats(
            State(f.org.state.clone()),
            auth.clone(),
            Path(f.channel.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(first, second);
        // Even knowing a hidden id does not grant access through the mutation helper.
        threads.insert_one(base.clone()).await.unwrap();
        for id in ["public-group", base.get_str("_id").unwrap()] {
            assert!(
                crate::handlers::nyxbot::thread_controls::list_threads(
                    State(f.org.state.clone()),
                    auth.clone(),
                    Path((f.channel.id.clone(), id.into())),
                    axum::extract::Query(Default::default())
                )
                .await
                .is_err()
            );
            assert!(
                crate::handlers::nyxbot::update_channel_chat(
                    State(f.org.state.clone()),
                    auth.clone(),
                    Path((f.channel.id.clone(), id.into())),
                    Json(serde_json::from_value(json!({"members":"everyone"})).unwrap())
                )
                .await
                .is_err()
            );
        }
        for actor in [&f.org.viewer, &f.org.outsider] {
            assert!(
                crate::handlers::nyxbot::list_channel_chats(
                    State(f.org.state.clone()),
                    crate::test_utils::test_auth_user(actor),
                    Path(f.channel.id.clone())
                )
                .await
                .is_err()
            );
        }
        let Json(list) = crate::handlers::nyxbot::list_channels(State(f.org.state.clone()), auth)
            .await
            .unwrap();
        let row = &list["channel_agents"][0];
        assert_eq!(row["can_manage"], false);
        assert!(row.get("last_error").is_some());
        assert!(row["owner_linked"].is_boolean());
        for field in ["route_id", "route_api_key_id", "user_id"] {
            assert!(row.get(field).is_none(), "{field}");
        }
        assert!(row["org_specialist"].get("daily_credits").is_some());
        db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn org_guest_defaults_settings_and_preview_reject_nonhuman_credentials() {
    use crate::mw::auth::AuthMethod;
    let f = Fixture::new("guest_default_human_only").await;
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::Delegated,
        AuthMethod::ServiceAccount,
        AuthMethod::Relay,
    ] {
        let mut auth = crate::test_utils::test_auth_user(&f.org.admin);
        auth.auth_method = method;
        assert!(
            crate::handlers::org_specialist_channels::preview(
                State(f.org.state.clone()),
                auth.clone(),
                Json(crate::handlers::org_specialist_channels::PreviewRequest {
                    bot_id: f.bot.id.clone(),
                    agent_id: f.agent.id.clone(),
                    channel_id: None
                })
            )
            .await
            .is_err()
        );
        assert!(
            crate::handlers::org_specialist_channels::get(
                State(f.org.state.clone()),
                auth.clone(),
                Path(f.channel.id.clone())
            )
            .await
            .is_err()
        );
        assert!(
            crate::handlers::nyxbot::list_channel_chats(
                State(f.org.state.clone()),
                auth.clone(),
                Path(f.channel.id.clone())
            )
            .await
            .is_err()
        );
        assert!(
            crate::handlers::nyxbot::list_channels(State(f.org.state.clone()), auth)
                .await
                .is_err()
        );
    }
    let mut auth = crate::test_utils::test_auth_user(&f.org.admin);
    auth.oauth_client_id = Some("third-party".into());
    assert!(
        crate::handlers::org_specialist_channels::preview(
            State(f.org.state.clone()),
            auth,
            Json(crate::handlers::org_specialist_channels::PreviewRequest {
                bot_id: f.bot.id.clone(),
                agent_id: f.agent.id.clone(),
                channel_id: None
            })
        )
        .await
        .is_err()
    );
    f.org.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_mounted_member_reads_and_admin_write_fences() {
    Box::pin(async {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let f = Fixture::new("guest_default_route_permissions").await;
        let db = &f.org.state.db;
        crate::services::feature_flag_service::set_platform_override(
            db,
            crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            &f.org.admin,
        )
        .await
        .unwrap();
        let token = crate::crypto::jwt::generate_access_token(
            &f.org.state.jwt_keys,
            &f.org.state.config,
            &uuid::Uuid::parse_str(&f.org.member).unwrap(),
            "openid",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (_, app) = crate::routes::build_router();
        let app = app.with_state(f.org.state.clone());
        let root = format!("/api/v1/assistant/nyxagent/channels/{}", f.channel.id);
        let bot = format!("/api/v1/channel-bots/{}", f.bot.id);
        let request = |method: &str, path: &str, body: serde_json::Value| {
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        for path in [
            "/api/v1/assistant/nyxagent/channels".to_owned(),
            format!("{root}/org-specialist"),
            format!("{root}/chats"),
            bot.clone(),
            format!("{bot}/health"),
            format!("/api/v1/channel-bots?org_id={}", f.org.org),
        ] {
            let response = Box::pin(app.clone().oneshot(request("GET", &path, json!(null))))
                .await
                .unwrap();
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(
                status.is_success(),
                "{path}: {status} {}",
                String::from_utf8_lossy(&body)
            );
        }
        for (method, path, body) in [
            (
                "POST",
                "/api/v1/assistant/nyxagent/channels".into(),
                json!({"bot_id":f.bot.id,"agent_id":f.agent.id}),
            ),
            (
                "POST",
                "/api/v1/assistant/nyxagent/channels/preview".into(),
                json!({"bot_id":f.bot.id,"agent_id":f.agent.id}),
            ),
            ("PATCH", root.clone(), json!({"private_chats":"everyone"})),
            ("DELETE", root.clone(), json!(null)),
            (
                "PUT",
                format!("{root}/org-specialist"),
                json!({"enabled":true,"expected_revision":""}),
            ),
            ("PATCH", bot.clone(), json!({"label":"Changed"})),
            ("DELETE", bot.clone(), json!(null)),
            ("POST", format!("{bot}/verify"), json!({})),
            ("POST", format!("{bot}/resubscribe"), json!({})),
        ] {
            let response = Box::pin(app.clone().oneshot(request(method, &path, body)))
                .await
                .unwrap();
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "{method} {path}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn org_guest_defaults_idle_pass_is_one_indexed_query() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = commands.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "guest_default_idle",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                observed.lock().unwrap().push(event.command_name);
            }
        }),
    )
    .await
    .unwrap();
    defaults::ensure_indexes(&db).await.unwrap();
    db.collection::<Document>("nyxbot_channels").insert_many((0..500).map(|i| doc! {"_id":i.to_string(),"status":"active","org_specialist":{"version":1},"org_guest_defaults_version":3+i%2})).await.unwrap();
    db.collection::<Document>("nyxbot_channels").insert_many((0..500).map(|i| doc! {"_id":format!("pending-{i}"),"status":"pending","org_specialist":{"version":1}})).await.unwrap();
    commands.lock().unwrap().clear();
    defaults::sweep(&db).await.unwrap();
    assert_eq!(*commands.lock().unwrap(), vec!["find"]);
    let plan = db.run_command(doc! {"explain":{"find":"nyxbot_channels","filter":defaults::pending_filter(HistoricalRule::NoAuditedSave),"sort":{"_id":1},"limit":100},"verbosity":"executionStats"}).await.unwrap();
    let stats = plan.get_document("executionStats").unwrap();
    assert_eq!(stats.get_i32("totalDocsExamined").unwrap(), 0, "{plan:?}");
    assert!(
        !format!(
            "{:?}",
            plan.get_document("queryPlanner")
                .unwrap()
                .get("winningPlan")
        )
        .contains("COLLSCAN")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_old_replica_pending_binding_qualifies_after_activation() {
    let mut f = Fixture::new("guest_defaults_pending_rollout").await;
    let db = f.org.state.db.clone();
    let channels = db.collection::<Document>("nyxbot_channels");
    channels
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"status":"pending"}},
        )
        .await
        .unwrap();
    let pending = channels
        .find_one(doc! {"_id":&f.channel.id})
        .await
        .unwrap()
        .unwrap();
    defaults::sweep(&db).await.unwrap();
    assert_eq!(
        pending,
        channels
            .find_one(doc! {"_id":&f.channel.id})
            .await
            .unwrap()
            .unwrap()
    );
    channels
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"status":"active"}},
        )
        .await
        .unwrap();
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    assert!(f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert!(
        !f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .revision
            .is_empty()
    );
    defaults::sweep(&db).await.unwrap();
    assert_eq!(db.collection::<Document>("audit_log").count_documents(doc! {"event_type":"org_channel_guest_defaults_applied","event_data.channel_id":&f.channel.id}).await.unwrap(),1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_completion_erasure_is_scoped_and_deleted_bots_stay_erased() {
    let f = Fixture::new("guest_defaults_erasure").await;
    let db = &f.org.state.db;
    let migrations = db.collection::<Document>(
        crate::services::assistant_access_policy::MIGRATIONS_COLLECTION_NAME,
    );
    migrations
        .insert_one(doc! {"_id":"unrelated-marker","channel_id":&f.channel.id})
        .await
        .unwrap();
    defaults::sweep(db).await.unwrap();
    assert_eq!(
        migrations
            .count_documents(doc! {"migration":"issue-2482"})
            .await
            .unwrap(),
        1
    );
    db.collection::<Document>("channel_bots")
        .update_one(doc! {"_id":&f.bot.id}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    defaults::erase(db, doc! {"channel_bot_id":&f.bot.id})
        .await
        .unwrap();
    // An old replacement can remove the marker after deletion; it must not recreate completion.
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    defaults::sweep(db).await.unwrap();
    assert_eq!(
        migrations
            .count_documents(doc! {"migration":"issue-2482"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        migrations
            .count_documents(doc! {"_id":"unrelated-marker"})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_untouched_off_switches_once_with_one_audit_and_notice_per_current_admin()
 {
    let mut f = Fixture::new("guest_defaults_untouched").await;
    let db = f.org.state.db.clone();
    db.collection::<Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&f.org.org,"member_user_id":&f.org.member},
            doc! {"$set":{"role":"admin"}},
        )
        .await
        .unwrap();
    let (tx, mut notices) = tokio::sync::mpsc::unbounded_channel();
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .insert(f.bot.id.clone(), tx);
    let original = f.channel.org_specialist.as_ref().unwrap().guests.clone();
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    let policy = &f.channel.org_specialist.as_ref().unwrap().guests;
    assert!(policy.enabled);
    assert!(!policy.revision.is_empty());
    assert_eq!(policy.daily_credits, original.daily_credits);
    assert_eq!(policy.follows_agent_grants, original.follows_agent_grants);
    assert_eq!(policy.service_ids, original.service_ids);
    assert_eq!(policy.service_authority, original.service_authority);
    let mut recipients = Vec::new();
    for _ in 0..2 {
        let (recipient, notice) =
            tokio::time::timeout(std::time::Duration::from_secs(5), notices.recv())
                .await
                .unwrap()
                .unwrap();
        recipients.push(recipient);
        assert_eq!(
            notice.notice_kind,
            crate::services::notification_service::ChannelBotNoticeKind::GuestDefaults
        );
        for text in [
            "now on by default",
            "charged to the organization",
            "model-only chat",
            "Allow guests off",
            "billing wallet and subscription",
        ] {
            assert!(
                notice.recommended_action_text.contains(text),
                "{text}: {}",
                notice.recommended_action_text
            );
        }
    }
    recipients.sort();
    let mut expected = vec![f.org.admin.clone(), f.org.member.clone()];
    expected.sort();
    assert_eq!(recipients, expected);
    defaults::sweep(&db).await.unwrap();
    assert!(notices.try_recv().is_err());
    assert_eq!(db.collection::<Document>("audit_log").count_documents(doc! {"event_type":"org_channel_guest_defaults_applied","event_data.channel_id":&f.channel.id}).await.unwrap(),1);
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .remove(&f.bot.id);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_nonempty_revision_stays_off() {
    let mut f = Fixture::new("guest_defaults_nonempty").await;
    let db = f.org.state.db.clone();
    let saved_revision = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"org_specialist.guests.revision":&saved_revision}},
        )
        .await
        .unwrap();
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(
        f.channel.org_specialist.as_ref().unwrap().guests.revision,
        saved_revision
    );
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"org_channel_guest_defaults_applied"})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_restored_binding_keeps_carried_off_and_private_choice() {
    let mut f = Fixture::new("guest_defaults_restored").await;
    let db = f.org.state.db.clone();
    db.collection::<Document>("nyxbot_channels").update_one(doc! {"_id":&f.channel.id},doc! {"$set":{"org_specialist.guests.continuity_channel_id":"predecessor"},"$unset":{"private_chats":""}}).await.unwrap();
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(f.channel.private_chats, None);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_rest_and_tool_off_saves_have_indexed_audit_evidence() {
    let mut f = Fixture::new("guest_defaults_save_paths").await;
    ready(&mut f).await;
    let db = f.org.state.db.clone();
    let Json(saved) = crate::handlers::org_specialist_channels::update(
        State(f.org.state.clone()),
        crate::test_utils::test_auth_user(&f.org.admin),
        Path(f.channel.id.clone()),
        Json(serde_json::from_value(json!({"enabled":false,"expected_revision":""})).unwrap()),
    )
    .await
    .unwrap();
    assert_ne!(saved["guest_revision"], "");
    let saved = crate::handlers::org_specialist_channels::update_tool(
        &f.org.state,
        &f.org.admin,
        &f.channel.id,
        &json!({"enabled":false,"expected_revision":saved["guest_revision"]}),
    )
    .await
    .unwrap();
    assert!(!saved["guest_enabled"].as_bool().unwrap());
    let filter =
        doc! {"event_type":defaults::SAVE_EVENT,"event_data.channel_id":{"$in":[&f.channel.id]}};
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(filter.clone())
            .await
            .unwrap(),
        2
    );
    // The existing event index excludes unrelated audit traffic; channel id is residual.
    db.collection::<Document>("audit_log").insert_many((0..100).map(|i| doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"event_type":"unrelated_fixture_event",
        "created_at":bson::DateTime::from_millis(i),"event_data":{"channel_id":&f.channel.id}
    })).await.unwrap();
    db.collection::<Document>("audit_log")
        .insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"event_type":defaults::SAVE_EVENT,
            "created_at":bson::DateTime::now(),"event_data":{"channel_id":"another-binding"}
        })
        .await
        .unwrap();
    let plan = db.run_command(doc! {"explain":{"find":"audit_log","filter":filter,"hint":defaults::SAVE_INDEX},"verbosity":"executionStats"}).await.unwrap();
    let stats = plan.get_document("executionStats").unwrap();
    assert_eq!(stats.get_i32("nReturned").unwrap(), 2, "{plan:?}");
    assert_eq!(stats.get_i32("totalDocsExamined").unwrap(), 3, "{plan:?}");
    assert!(stats.get_i32("totalKeysExamined").unwrap() <= 4, "{plan:?}");
    assert!(format!("{plan:?}").contains(defaults::SAVE_INDEX));
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_audit_lookup_error_leaves_off_and_retries() {
    let mut f = Fixture::new("guest_defaults_audit_failure").await;
    let db = f.org.state.db.clone();
    defaults::FAIL_AUDIT_HINT
        .lock()
        .unwrap()
        .insert((db.name().to_owned(), f.channel.id.clone()));
    defaults::sweep(&db).await.unwrap();
    let raw = db
        .collection::<Document>("nyxbot_channels")
        .find_one(doc! {"_id":&f.channel.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!raw.contains_key("org_guest_defaults_version"));
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(
        db.collection::<Document>(
            crate::services::assistant_access_policy::MIGRATIONS_COLLECTION_NAME
        )
        .count_documents(doc! {"migration":"issue-2482"})
        .await
        .unwrap(),
        0
    );
    defaults::FAIL_AUDIT_HINT
        .lock()
        .unwrap()
        .remove(&(db.name().to_owned(), f.channel.id.clone()));
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    assert!(f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_admin_off_wins_concurrent_and_just_completed_migration() {
    for migrate_first in [false, true] {
        let mut f = Fixture::new("guest_defaults_off_race").await;
        ready(&mut f).await;
        let db = f.org.state.db.clone();
        let input =
            serde_json::from_value(json!({"enabled":false,"expected_revision":""})).unwrap();
        if migrate_first {
            defaults::sweep(&db).await.unwrap();
            management::update(&db, &f.org.admin, &f.channel, input)
                .await
                .unwrap();
        } else {
            let (migration, save) = tokio::join!(
                defaults::sweep(&db),
                management::update(&db, &f.org.admin, &f.channel, input)
            );
            migration.unwrap();
            save.unwrap();
        }
        reload(&mut f).await;
        assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
        assert!(
            !f.channel
                .org_specialist
                .as_ref()
                .unwrap()
                .guests
                .revision
                .is_empty()
        );
        defaults::sweep(&db).await.unwrap();
        reload(&mut f).await;
        assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_guest_defaults_unusual_rows_and_nonlive_authority_never_change_settings() {
    let f = Fixture::new("guest_defaults_closed").await;
    let db = &f.org.state.db;
    let channels = db.collection::<Document>("nyxbot_channels");
    let original = channels
        .find_one(doc! {"_id":&f.channel.id})
        .await
        .unwrap()
        .unwrap();
    let mutations = [
        (
            "nyxbot_channels",
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"org_specialist.guests.revision":null}},
        ),
        (
            "nyxbot_channels",
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"org_specialist.guests.revision":123}},
        ),
        (
            "nyxbot_channels",
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"org_specialist.guests.revision":"malformed-revision"},"$unset":{"private_chats":""}},
        ),
        (
            "nyxbot_channels",
            doc! {"_id":&f.channel.id},
            doc! {"$unset":{"org_specialist.guests.revision":""}},
        ),
        (
            "nyxbot_channels",
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"private_chats":"future-audience"}},
        ),
        (
            "nyxbot_channels",
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"status":"pending"}},
        ),
        (
            "users",
            doc! {"_id":&f.org.org},
            doc! {"$set":{"is_active":false}},
        ),
        (
            "users",
            doc! {"_id":&f.org.admin},
            doc! {"$set":{"is_active":false}},
        ),
        (
            "channel_bots",
            doc! {"_id":&f.bot.id},
            doc! {"$set":{"is_active":false}},
        ),
        (
            "assistant_agents",
            doc! {"_id":&f.agent.id},
            doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
        ),
        (
            "org_memberships",
            doc! {"org_user_id":&f.org.org,"member_user_id":&f.org.admin},
            doc! {"$set":{"role":"member"}},
        ),
    ];
    for (name, filter, change) in mutations {
        channels
            .replace_one(doc! {"_id":&f.channel.id}, &original)
            .await
            .unwrap();
        let collection = db.collection::<Document>(name);
        let saved = collection.find_one(filter.clone()).await.unwrap().unwrap();
        collection.update_one(filter.clone(), change).await.unwrap();
        let before = channels
            .find_one(doc! {"_id":&f.channel.id})
            .await
            .unwrap()
            .unwrap();
        defaults::sweep(db).await.unwrap();
        let after = channels
            .find_one(doc! {"_id":&f.channel.id})
            .await
            .unwrap()
            .unwrap();
        for field in [
            "org_specialist",
            "private_chats",
            "execution_policy_revision",
        ] {
            assert_eq!(before.get(field), after.get(field), "{name} {field}");
        }
        collection.replace_one(filter, saved).await.unwrap();
    }
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"org_channel_guest_defaults_applied"})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_notice_lists_effective_frozen_services_and_keeps_the_legacy_cap() {
    let mut f = Fixture::new("guest_defaults_service_notice").await;
    let db = f.org.state.db.clone();
    let service = f.org.service(&f.org.org, "notice-service").await;
    f.agent.grants.service_ids.push(service.clone());
    db.collection::<AssistantAgent>("assistant_agents")
        .replace_one(doc! {"_id":&f.agent.id}, &f.agent)
        .await
        .unwrap();
    let policy = &mut f.channel.org_specialist.as_mut().unwrap().guests;
    policy.service_ids = vec![service.clone()];
    policy.service_authority.insert(
        service.clone(),
        management::service_digest(&f.agent, &service),
    );
    policy.daily_credits = Some(Credits::ZERO);
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    let (tx, mut notices) = tokio::sync::mpsc::unbounded_channel();
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .insert(f.bot.id.clone(), tx);
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    let policy = &f.channel.org_specialist.as_ref().unwrap().guests;
    assert!(policy.enabled);
    assert_eq!(policy.daily_credits, Some(Credits::ZERO));
    assert!(!policy.follows_agent_grants);
    assert_eq!(policy.service_ids, vec![service]);
    let (_, notice) = tokio::time::timeout(std::time::Duration::from_secs(5), notices.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        notice.recommended_action_text.contains("notice-service"),
        "{}",
        notice.recommended_action_text
    );
    f.wallet().await;
    let origin = f.guest("no-cap").await;
    let mut session = db.client().start_session().await.unwrap();
    assert!(
        validate_in_session(&db, &mut session, &origin, &f.org.org, &f.agent.id, false)
            .await
            .is_err(),
        "legacy mode still refuses a zero cap"
    );
    management::update(
        &db,
        &f.org.admin,
        &f.channel,
        serde_json::from_value(json!({"enabled":false})).unwrap(),
    )
    .await
    .unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(
        f.channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .daily_credits,
        Some(Credits::ZERO)
    );
    crate::services::channel_bot_health::TEST_NOTICES
        .lock()
        .unwrap()
        .remove(&f.bot.id);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_off_save_preserves_unavailable_services_and_missing_legacy_cap() {
    let mut f = Fixture::new("guest_defaults_off_unavailable_service").await;
    let db = f.org.state.db.clone();
    let policy = &mut f.channel.org_specialist.as_mut().unwrap().guests;
    policy.service_ids = vec!["removed-service".into()];
    policy
        .service_authority
        .insert("removed-service".into(), "saved-authority".into());
    policy.daily_credits = None;
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    let carried = f.channel.org_specialist.as_ref().unwrap().guests.clone();
    assert!(carried.enabled);
    management::update(
        &db,
        &f.org.admin,
        &f.channel,
        serde_json::from_value(json!({"enabled":false,"expected_revision":carried.revision}))
            .unwrap(),
    )
    .await
    .unwrap();
    reload(&mut f).await;
    let saved = &f.channel.org_specialist.as_ref().unwrap().guests;
    assert!(!saved.enabled);
    assert!(!saved.revision.is_empty());
    assert_ne!(saved.revision, carried.revision);
    assert_eq!(saved.service_ids, carried.service_ids);
    assert_eq!(saved.service_authority, carried.service_authority);
    assert_eq!(saved.daily_credits, None);
    assert!(management::update(&db,&f.org.admin,&f.channel,serde_json::from_value(json!({"enabled":false,"service_ids":["removed-service"],"expected_revision":saved.revision})).unwrap()).await.is_err());
    assert!(matches!(
        management::update(
            &db,
            &f.org.admin,
            &f.channel,
            serde_json::from_value(json!({"enabled":true,"expected_revision":saved.revision}))
                .unwrap()
        )
        .await,
        Err(AppError::BillingNotConfigured(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_no_wallet_switches_and_keeps_billing_refusal_and_health_advice() {
    let mut f = Fixture::new("guest_defaults_no_wallet").await;
    let db = f.org.state.db.clone();
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    assert!(f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    let origin = f.guest("visitor").await;
    let mut session = db.client().start_session().await.unwrap();
    assert!(matches!(
        validate_in_session(&db, &mut session, &origin, &f.org.org, &f.agent.id, false).await,
        Err(AppError::BillingNotConfigured(_))
    ));
    let view = management::view(&db, &f.org.admin, &f.channel)
        .await
        .unwrap();
    assert_eq!(view["health"]["action"], "enable_billing");
    for collection in [
        crate::models::org_channel::HOLDS,
        crate::models::usage_meter::COLLECTION_NAME,
    ] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    f.wallet().await;
    validate_in_session(&db, &mut session, &origin, &f.org.org, &f.agent.id, false)
        .await
        .unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_poisoned_rows_do_not_starve_later_batches_and_retry() {
    let f = Fixture::new("guest_defaults_poisoned_rows").await;
    let db = &f.org.state.db;
    let channels = db.collection::<NyxbotChannel>("nyxbot_channels");
    channels.delete_many(doc! {}).await.unwrap();
    let mut ids = Vec::new();
    for i in 0..102 {
        let mut bot = f.bot.clone();
        bot.id = uuid::Uuid::new_v4().to_string();
        db.collection::<ChannelBot>("channel_bots")
            .insert_one(&bot)
            .await
            .unwrap();
        let mut row = f.channel.clone();
        row.id = format!("00000000-0000-4000-8000-{i:012}");
        row.channel_bot_id = bot.id;
        channels.insert_one(&row).await.unwrap();
        if i < 100 && i != 50 {
            defaults::FAIL_AUDIT_HINT
                .lock()
                .unwrap()
                .insert((db.name().to_owned(), row.id.clone()));
        }
        ids.push(row.id);
    }
    let mut sweep = defaults::Sweep::default();
    sweep.run(db).await.unwrap();
    assert_eq!(
        db.collection::<Document>("nyxbot_channels")
            .count_documents(doc! {"org_specialist.guests.enabled":true})
            .await
            .unwrap(),
        1
    );
    sweep.run(db).await.unwrap();
    assert_eq!(
        db.collection::<Document>("nyxbot_channels")
            .count_documents(doc! {"org_specialist.guests.enabled":true})
            .await
            .unwrap(),
        3
    );
    // Retrying poison leaves completion absent; repaired evidence succeeds next pass.
    for id in &ids[..100] {
        defaults::FAIL_AUDIT_HINT
            .lock()
            .unwrap()
            .remove(&(db.name().to_owned(), id.clone()));
    }
    sweep.run(db).await.unwrap();
    assert_eq!(
        db.collection::<Document>("nyxbot_channels")
            .count_documents(doc! {"org_specialist.guests.enabled":true})
            .await
            .unwrap(),
        102
    );
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"org_channel_guest_defaults_applied"})
            .await
            .unwrap(),
        102
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_bot_list_preserves_credential_gates_and_only_adds_human_members() {
    Box::pin(async {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        let f = Fixture::new("guest_defaults_bot_list_matrix").await;
        let state = &f.org.state;
        let (_, app) = crate::routes::build_router();
        let app = app.with_state(state.clone());
        let mut credentials = Vec::new();
        for (label, actor, allowed) in [
            ("org admin", &f.org.admin, true),
            ("Member", &f.org.member, true),
            ("Viewer", &f.org.viewer, false),
            ("non-member", &f.org.outsider, false),
        ] {
            let token = crate::crypto::jwt::generate_access_token(
                &state.jwt_keys,
                &state.config,
                &uuid::Uuid::parse_str(actor).unwrap(),
                "openid",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            credentials.push((label, format!("Bearer {token}"), allowed, true));
        }
        for (label, owner) in [
            ("personal agent key", &f.org.admin),
            ("org agent key", &f.org.org),
        ] {
            let key = crate::services::key_service::create_api_key(
                &state.db,
                owner,
                "List matrix",
                "read proxy",
                None,
                None,
                Some(&[]),
                Some(&[]),
                Some(false),
                Some(false),
                Some(false),
                None,
                None,
                Some("generic"),
                None,
            )
            .await
            .unwrap();
            credentials.push((label, format!("Bearer {}", key.full_key), false, false));
        }
        let delegated = crate::crypto::jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &uuid::Uuid::parse_str(&f.org.admin).unwrap(),
            "account:read",
            "fixture-client",
            60,
            None,
        )
        .unwrap();
        credentials.push((
            "delegated account:read",
            format!("Bearer {delegated}"),
            false,
            false,
        ));
        let (sa, secret) = crate::services::service_account_service::create_service_account(
            &state.db,
            "List matrix",
            None,
            "account:read",
            &[],
            None,
            &f.org.admin,
        )
        .await
        .unwrap();
        let token = crate::services::service_account_service::authenticate_client_credentials(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &sa.client_id,
            &secret,
            None,
        )
        .await
        .unwrap()
        .access_token;
        credentials.push(("service account", format!("Bearer {token}"), false, false));
        for (label, authorization, org_allowed, human) in credentials {
            for suffix in [
                format!("?org_id={}", f.org.org),
                "?scope=all".into(),
                "?scope=user".into(),
                String::new(),
            ] {
                let request = Request::builder()
                    .uri(format!("/api/v1/channel-bots{suffix}"))
                    .header("authorization", &authorization)
                    .body(Body::empty())
                    .unwrap();
                let response = app.clone().oneshot(request).await.unwrap();
                let status = response.status();
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let allowed = human && (!suffix.starts_with("?org_id=") || org_allowed);
                assert_eq!(
                    status,
                    if allowed {
                        StatusCode::OK
                    } else {
                        StatusCode::FORBIDDEN
                    },
                    "{label} {suffix}: {}",
                    String::from_utf8_lossy(&body)
                );
                if allowed {
                    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let count = usize::from(
                        org_allowed && (suffix.starts_with("?org_id=") || suffix == "?scope=all"),
                    );
                    assert_eq!(body["total"], count, "{label} {suffix}");
                    assert_eq!(
                        body["bots"].as_array().unwrap().len(),
                        count,
                        "{label} {suffix}"
                    );
                }
            }
        }
        // Ordinary third-party OAuth retains main's admin-only organization inventory.
        for (actor, allowed) in [(&f.org.admin, true), (&f.org.member, false)] {
            let mut auth = crate::test_utils::test_auth_user(actor);
            auth.oauth_client_id = Some("third-party".into());
            let result = crate::handlers::channel_bots::list_bots(
                State(state.clone()),
                auth,
                axum::extract::Query(crate::handlers::channel_bots::ChannelBotListQuery {
                    org_id: Some(f.org.org.clone()),
                    scope: None,
                }),
            )
            .await;
            assert_eq!(result.is_ok(), allowed);
        }
        state.db.drop().await.unwrap();
    })
    .await;
}

async fn save_history(db: &Database, actor: &str, event: &str, data: serde_json::Value) {
    crate::services::audit_service::log_actor_event(
        db.clone(),
        &crate::services::audit_service::AuditActor {
            user_id: actor.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        event,
        Some(data),
    )
    .await
    .unwrap();
}

async fn successor(f: &mut Fixture, restore: bool, retarget: bool) -> String {
    let db = &f.org.state.db;
    let predecessor = f.channel.id.clone();
    db.collection::<Document>("nyxbot_channels")
        .update_one(doc! {"_id":&predecessor}, doc! {"$set":{"status":"failed"}})
        .await
        .unwrap();
    let mut next = f.channel.clone();
    next.id = uuid::Uuid::new_v4().to_string();
    next.status = "pending".into();
    next.created_at += chrono::Duration::seconds(1);
    next.private_chats = None;
    next.org_specialist.as_mut().unwrap().guests = GuestPolicy {
        daily_credits: Some(management::default_daily_credits()),
        ..Default::default()
    };
    if retarget {
        let mut agent = f.agent.clone();
        agent.id = uuid::Uuid::new_v4().to_string();
        agent.name = "retarget-defaults".into();
        db.collection::<AssistantAgent>("assistant_agents")
            .insert_one(&agent)
            .await
            .unwrap();
        next.agent_id = Some(agent.id.clone());
        next.org_specialist.as_mut().unwrap().guests = new_guest_policy(true);
        f.agent = agent;
    }
    db.collection::<NyxbotChannel>("nyxbot_channels")
        .insert_one(&next)
        .await
        .unwrap();
    if restore {
        let context = (db.clone(), next.clone());
        crate::services::transaction_service::run_db_only(db, &context, |tx, (db, next)| {
            Box::pin(async move { management::restore_in_session(db, tx, next).await })
        })
        .await
        .unwrap();
    }
    db.collection::<Document>("nyxbot_channels")
        .update_one(doc! {"_id":&next.id}, doc! {"$set":{"status":"active"}})
        .await
        .unwrap();
    f.channel = next;
    reload(f).await;
    predecessor
}

#[tokio::test]
async fn org_guest_defaults_untouched_predecessor_relink_and_restoration_switch_after_deploy() {
    for restored in [false, true] {
        let mut f = Fixture::new("guest_defaults_chain_untouched").await;
        let db = f.org.state.db.clone();
        db.collection::<Document>("nyxbot_channels")
            .update_one(
                doc! {"_id":&f.channel.id},
                doc! {"$set":{"status":"failed"},"$unset":{"private_chats":""}},
            )
            .await
            .unwrap();
        defaults::sweep(&db).await.unwrap();
        reload(&mut f).await;
        let predecessor = successor(&mut f, restored, false).await;
        let before = f.channel.org_specialist.as_ref().unwrap().guests.clone();
        defaults::sweep(&db).await.unwrap();
        reload(&mut f).await;
        let after = &f.channel.org_specialist.as_ref().unwrap().guests;
        assert!(after.enabled);
        assert_ne!(after.revision, before.revision);
        assert_eq!(
            after.continuity_channel_id,
            restored.then_some(predecessor.clone())
        );
        assert_eq!(
            after
                .continuity_channel_id
                .as_deref()
                .unwrap_or(&f.channel.id),
            if restored {
                &predecessor
            } else {
                &f.channel.id
            }
        );
        assert_eq!(after.daily_credits, before.daily_credits);
        assert_eq!(after.service_ids, before.service_ids);
        assert_eq!(after.follows_agent_grants, before.follows_agent_grants);
        assert_eq!(f.channel.private_chats.as_deref(), Some("everyone"));
        defaults::sweep(&db).await.unwrap();
        assert_eq!(
            db.collection::<Document>("audit_log")
                .count_documents(doc! {
                "event_type":"org_channel_guest_defaults_applied"})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_guest_defaults_predecessor_off_audit_or_revision_protects_relinked_and_restored_successor()
 {
    for audited in [false, true] {
        for restored in [false, true] {
            let mut f = Fixture::new("guest_defaults_chain_chosen").await;
            let db = f.org.state.db.clone();
            if audited {
                save_history(
                    &db,
                    &f.org.admin,
                    defaults::SAVE_EVENT,
                    json!({"channel_id":f.channel.id}),
                )
                .await;
            } else {
                db.collection::<Document>("nyxbot_channels").update_one(doc! {"_id":&f.channel.id},
                    doc! {"$set":{"org_specialist.guests.revision":uuid::Uuid::new_v4().to_string()}}).await.unwrap();
                reload(&mut f).await;
            }
            successor(&mut f, restored, false).await;
            defaults::sweep(&db).await.unwrap();
            reload(&mut f).await;
            assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
            db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn org_guest_defaults_restoration_revision_proof_and_legacy_audit_limit() {
    for (recorded_revision, changed_revision, expected_on) in [
        (true, false, true),
        (true, true, false),
        (false, true, true),
    ] {
        let mut f = Fixture::new("guest_defaults_restore_revision").await;
        let db = f.org.state.db.clone();
        let predecessor = successor(&mut f, false, false).await;
        let revision = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>("nyxbot_channels").update_one(doc! {"_id":&f.channel.id},doc! {"$set":{
            "org_specialist.guests.continuity_channel_id":&predecessor,
            "org_specialist.guests.revision":if changed_revision {uuid::Uuid::new_v4().to_string()} else {revision.clone()}
        }}).await.unwrap();
        let mut data = json!({"channel_id":f.channel.id,"predecessor_id":predecessor,"enabled":false,"delegated_services":0});
        if recorded_revision {
            data["guest_revision"] = json!(revision);
        }
        save_history(&db, &f.org.admin, defaults::RESTORE_EVENT, data).await;
        defaults::sweep(&db).await.unwrap();
        reload(&mut f).await;
        assert_eq!(
            f.channel.org_specialist.as_ref().unwrap().guests.enabled,
            expected_on
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_guest_defaults_retarget_restoration_audits_carried_choice_and_exact_revision() {
    let mut f = Fixture::new("guest_defaults_retarget_audit").await;
    let db = f.org.state.db.clone();
    let predecessor = successor(&mut f, true, true).await;
    let policy = &f.channel.org_specialist.as_ref().unwrap().guests;
    assert!(!policy.enabled);
    assert!(policy.follows_agent_grants);
    let audit = db
        .collection::<Document>("audit_log")
        .find_one(doc! {
        "event_type":defaults::RESTORE_EVENT,"event_data.channel_id":&f.channel.id})
        .await
        .unwrap()
        .unwrap();
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("predecessor_id").unwrap(), predecessor);
    assert_eq!(data.get_str("guest_revision").unwrap(), policy.revision);
    assert_eq!(data.len(), 5);
    assert!(audit.contains_key("entry_hash"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_bounded_history_completes_unchanged() {
    let mut f = Fixture::new("guest_defaults_chain_bound").await;
    let db = f.org.state.db.clone();
    for _ in 0..32 {
        let mut row = f.channel.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        row.status = "failed".into();
        db.collection::<NyxbotChannel>("nyxbot_channels")
            .insert_one(row)
            .await
            .unwrap();
    }
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    assert_eq!(
        db.collection::<Document>("nyxbot_channels")
            .count_documents(defaults::pending_filter(HistoricalRule::NoAuditedSave))
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_health_status_is_not_authority_and_reactivation_retries() {
    for status in ["failed", "pending_webhook"] {
        let mut f = Fixture::new("guest_defaults_health_authority").await;
        let db = f.org.state.db.clone();
        db.collection::<Document>("channel_bots")
            .update_one(doc! {"_id":&f.bot.id}, doc! {"$set":{"status":status}})
            .await
            .unwrap();
        db.collection::<Document>("users")
            .update_one(doc! {"_id":&f.org.admin}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        defaults::sweep(&db).await.unwrap();
        assert_eq!(
            db.collection::<Document>("nyxbot_channels")
                .count_documents(defaults::pending_filter(HistoricalRule::NoAuditedSave))
                .await
                .unwrap(),
            1
        );
        db.collection::<Document>("users")
            .update_one(doc! {"_id":&f.org.admin}, doc! {"$set":{"is_active":true}})
            .await
            .unwrap();
        defaults::sweep(&db).await.unwrap();
        reload(&mut f).await;
        assert!(f.channel.org_specialist.as_ref().unwrap().guests.enabled);
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_guest_defaults_already_on_without_wallet_accepts_services_and_noop_but_off_to_on_refuses()
 {
    let mut f = Fixture::new("guest_defaults_wallet_transition").await;
    let db = f.org.state.db.clone();
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    let service = f.org.service(&f.org.org, "legacy-service").await;
    db.collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":&f.agent.id},
            doc! {"$set":{"grants.service_ids":[&service]}},
        )
        .await
        .unwrap();
    for input in [
        json!({"enabled":true,"service_ids":[service]}),
        json!({"enabled":true}),
        json!({"enabled":false}),
    ] {
        management::update(
            &db,
            &f.org.admin,
            &f.channel,
            serde_json::from_value(input).unwrap(),
        )
        .await
        .unwrap();
        reload(&mut f).await;
    }
    let error = management::update(
        &db,
        &f.org.admin,
        &f.channel,
        serde_json::from_value(json!({"enabled":true})).unwrap(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::BillingNotConfigured(_)));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_member_channel_list_preserves_main_shape_and_shared_list_active_limit()
{
    let f = Fixture::new("guest_defaults_list_contract").await;
    let db = &f.org.state.db;
    crate::services::feature_flag_service::set_platform_override(
        db,
        crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        &f.org.admin,
    )
    .await
    .unwrap();
    let Json(result) = crate::handlers::nyxbot::list_channels(
        State(f.org.state.clone()),
        crate::test_utils::test_auth_user(&f.org.member),
    )
    .await
    .unwrap();
    let value = &result["channel_agents"][0];
    let expected = serde_json::to_value(crate::handlers::nyxbot::ChannelAgentResponse::from(
        &f.channel,
    ))
    .unwrap();
    for (key, field) in expected.as_object().unwrap() {
        assert_eq!(value.get(key), Some(field), "{key}");
    }
    assert!(value["owner_linked"].is_boolean());
    assert_eq!(value["can_manage"], false);
    for i in 0..55 {
        let mut row = f.channel.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        row.channel_bot_id = uuid::Uuid::new_v4().to_string();
        if i < 3 {
            row.status = "failed".into();
        }
        db.collection::<NyxbotChannel>("nyxbot_channels")
            .insert_one(row)
            .await
            .unwrap();
    }
    let rows = crate::handlers::nyxbot::list(&f.org.state, &f.org.member)
        .await
        .unwrap();
    assert_eq!(rows.len(), 50);
    assert!(rows.iter().all(|r| r.status == "active"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_guest_defaults_restored_admin_off_wins_concurrent_and_completed_switch() {
    for migrate_first in [false, true] {
        let mut f = Fixture::new("guest_defaults_restored_off_race").await;
        ready(&mut f).await;
        successor(&mut f, true, false).await;
        let db = f.org.state.db.clone();
        let revision = f
            .channel
            .org_specialist
            .as_ref()
            .unwrap()
            .guests
            .revision
            .clone();
        let input =
            serde_json::from_value(json!({"enabled":false,"expected_revision":revision})).unwrap();
        if migrate_first {
            defaults::sweep(&db).await.unwrap();
            management::update(&db, &f.org.admin, &f.channel, input)
                .await
                .unwrap();
        } else {
            let (a, b) = tokio::join!(
                defaults::sweep(&db),
                management::update(&db, &f.org.admin, &f.channel, input)
            );
            a.unwrap();
            b.unwrap();
        }
        reload(&mut f).await;
        assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
        assert!(
            management::update(
                &db,
                &f.org.admin,
                &f.channel,
                serde_json::from_value(json!({"enabled":false,"expected_revision":revision}))
                    .unwrap()
            )
            .await
            .is_err(),
            "a later explicit save is not the migration revision"
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_guest_defaults_legacy_restoration_enabled_change_proves_choice_without_revision_identity()
 {
    let mut f = Fixture::new("guest_defaults_restored_enabled_proof").await;
    let db = f.org.state.db.clone();
    let predecessor = successor(&mut f, false, false).await;
    db.collection::<Document>("nyxbot_channels")
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{
                "org_specialist.guests.continuity_channel_id":&predecessor,
                "org_specialist.guests.revision":uuid::Uuid::new_v4().to_string()
            }},
        )
        .await
        .unwrap();
    save_history(&db,&f.org.admin,defaults::RESTORE_EVENT,
        json!({"channel_id":f.channel.id,"predecessor_id":predecessor,"enabled":true,"delegated_services":0})).await;
    defaults::sweep(&db).await.unwrap();
    reload(&mut f).await;
    assert!(!f.channel.org_specialist.as_ref().unwrap().guests.enabled);
    db.drop().await.unwrap();
}
