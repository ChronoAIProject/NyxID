use super::*;
use crate::models::{
    assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS, TurnOrigin},
    nyxbot_channel::{NyxbotWatch, WATCHES_COLLECTION_NAME as WATCHES},
};
use crate::services::{
    assistant_authority_tests::{Fixture, orchestrator_fixture},
    assistant_nyxagent as engine,
};
use chrono::{Duration, Utc};

async fn watch_fixture(name: &str) -> (Fixture, NyxbotWatch) {
    let f = orchestrator_fixture(name).await;
    let now = Utc::now();
    let source = uuid::Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>(crate::models::connect_link::COLLECTION_NAME).insert_one(doc! {"_id":&source,"user_id":&f.owner,"status":"completed","service_slug":"example","service_id":uuid::Uuid::new_v4().to_string(),"token_hash":"test-hash","created_at":bson::DateTime::from_chrono(now),"expires_at":bson::DateTime::from_chrono(now+Duration::hours(1))}).await.unwrap();
    let watch:NyxbotWatch=bson::from_document(doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.owner,"conversation_id":&f.row.id,"kind":"connect_link","connect_link_id":source,"status":"pending","created_at":bson::DateTime::from_chrono(now),"expires_at":bson::DateTime::from_chrono(now+Duration::hours(1))}).unwrap();
    f.state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .insert_one(&watch)
        .await
        .unwrap();
    (f, watch)
}

fn start(f: &Fixture, origin: TurnOrigin) -> engine::TurnStart {
    let request = engine::TurnRequest {
        conversation_id: Some(f.row.id.clone()),
        agent_id: None,
        text: "continue".into(),
        model: None,
        access_mode: None,
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
    };
    let mut start: engine::TurnStart = (&request).into();
    start.origin = origin;
    start
}

async fn finish(f: &Fixture, row: &AssistantConversation) {
    engine::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Complete".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
}

async fn expire(f: &Fixture, watch: &NyxbotWatch) {
    let past = bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1));
    f.state
        .db
        .collection::<bson::Document>(WATCHES)
        .update_one(
            doc! {"_id":&watch.id},
            doc! {"$set":{"wake_claim.expires_at":past}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"pending_events.0.wake.ready_at":past}},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn durable_wake_claim_crash_survives_ttl_and_two_replicas_admit_once() {
    let (f, watch) = watch_fixture("wake_claim_ttl").await;
    let claimed = watch::claim(&f.state.db, &watch, doc! {})
        .await
        .unwrap()
        .unwrap();
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    assert_eq!(row.pending_events.len(), 1);
    assert!(row.pending_events[0].text.is_empty());
    assert!(
        super::super::assistant_team_service::queued(&f.state.db, Some(&f.owner))
            .await
            .unwrap()
            .is_empty()
    );
    // No settlement callback runs; a successor sees only the committed intent.
    expire(&f, &claimed).await;
    f.state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .delete_one(doc! {"_id":&claimed.id})
        .await
        .unwrap();
    finish(&f, &f.row).await;
    let (a, b) = tokio::join!(
        engine::begin_turn(
            &f.state.db,
            &f.owner,
            start(&f, TurnOrigin::Event),
            &f.state.encryption_keys
        ),
        engine::begin_turn(
            &f.state.db,
            &f.owner,
            start(&f, TurnOrigin::Event),
            &f.state.encryption_keys
        ),
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let admitted = a.or(b).unwrap();
    assert!(admitted.pending_events.is_empty());
    assert_eq!(admitted.active_turn.as_ref().unwrap().events.len(), 1);
    assert!(
        admitted.active_turn.as_ref().unwrap().events[0]
            .text
            .contains("finished connecting")
    );
    assert!(
        !watch::settle(&f.state.db, &claimed, "connection_finished", None, None)
            .await
            .unwrap()
    );
    finish(&f, &admitted).await;
    assert!(
        engine::begin_turn(
            &f.state.db,
            &f.owner,
            start(&f, TurnOrigin::Event),
            &f.state.encryption_keys
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_expired_claim_fences_the_original_settler() {
    let (f, watch) = watch_fixture("wake_claim_fence").await;
    let claimed = watch::claim(&f.state.db, &watch, doc! {})
        .await
        .unwrap()
        .unwrap();
    expire(&f, &claimed).await;
    finish(&f, &f.row).await;
    let admitted = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start(&f, TurnOrigin::Event),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(
        !watch::settle(&f.state.db, &claimed, "connection_finished", None, None)
            .await
            .unwrap()
    );
    assert_eq!(
        f.state
            .db
            .collection::<NyxbotWatch>(WATCHES)
            .find_one(doc! {"_id":&watch.id})
            .await
            .unwrap()
            .unwrap()
            .last_error
            .as_deref(),
        Some("wake_preparation_interrupted")
    );
    finish(&f, &admitted).await;
    assert!(
        engine::get(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_unfinished_preparation_does_not_block_a_browser_turn() {
    let (f, watch) = watch_fixture("wake_preparing_browser").await;
    let claimed = watch::claim(&f.state.db, &watch, doc! {})
        .await
        .unwrap()
        .unwrap();
    finish(&f, &f.row).await;
    let browser = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start(&f, TurnOrigin::User),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(browser.pending_events.len(), 1);
    assert!(browser.active_turn.as_ref().unwrap().events.is_empty());
    assert!(
        watch::settle(&f.state.db, &claimed, "connection_finished", None, None)
            .await
            .unwrap()
    );
    finish(&f, &browser).await;
    let admitted = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start(&f, TurnOrigin::Event),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(admitted.active_turn.as_ref().unwrap().events.len(), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_queue_full_rolls_back_the_preparation_claim() {
    let (f, watch) = watch_fixture("wake_claim_full").await;
    let events: Vec<_> = (0..engine::MAX_PENDING_EVENTS)
        .map(|_| {
            event(
                "group_settled",
                WakeReference::GroupFollower {
                    group_id: "group".into(),
                    since_seq: 0,
                    upper_seq: 1,
                },
            )
        })
        .collect();
    engine::push_events(&f.state.db, &f.owner, &f.row.id, events)
        .await
        .unwrap();
    assert!(matches!(
        watch::claim(&f.state.db, &watch, doc! {}).await,
        Err(AppError::Conflict(_))
    ));
    let current = f
        .state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .find_one(doc! {"_id":&watch.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.status, "pending");
    assert!(current.wake_claim.is_none());
    assert_eq!(
        engine::get(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .pending_events
            .len(),
        engine::MAX_PENDING_EVENTS
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_idle_selectors_each_use_one_indexed_read() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let commands = Arc::new(Mutex::new(Vec::new()));
    let observed = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "wake_idle_reads",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                observed
                    .lock()
                    .unwrap()
                    .push((event.command_name, event.command));
            }
        }),
    )
    .await;
    engine::ensure_indexes(&db).await.unwrap();
    let state = crate::test_utils::test_app_state(db.clone());
    commands.lock().unwrap().clear();
    crate::handlers::nyxbot::process_watches(&state)
        .await
        .unwrap();
    assert_eq!(commands.lock().unwrap().len(), 1);
    super::super::assistant_group_service::with_pending(&db)
        .await
        .unwrap();
    assert_eq!(commands.lock().unwrap().len(), 2);
    super::super::assistant_team_service::queued(&db, None)
        .await
        .unwrap();
    assert_eq!(commands.lock().unwrap().len(), 3);
    super::super::collaboration_group_service::requests::pending(&db, None)
        .await
        .unwrap();
    let reads = std::mem::take(&mut *commands.lock().unwrap());
    assert_eq!(reads.len(), 4);
    for (name, command) in reads {
        assert_eq!(name, "find");
        let mut find = doc! {"find":command.get_str("find").unwrap(),"filter":command.get_document("filter").unwrap().clone()};
        for field in ["sort", "hint", "limit"] {
            if let Some(value) = command.get(field) {
                find.insert(field, value.clone());
            }
        }
        let explained = db
            .run_command(doc! {"explain":find,"verbosity":"queryPlanner"})
            .await
            .unwrap();
        let plan = explained
            .get_document("queryPlanner")
            .unwrap()
            .get_document("winningPlan")
            .unwrap()
            .to_string();
        assert!(!plan.contains("COLLSCAN"), "{plan}");
        eprintln!(
            "durable wake idle {}: 1 find; plan={plan}",
            command.get_str("find").unwrap()
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_mixed_fleet_keeps_legacy_events_until_all_readers_support_references() {
    let (f, _) = watch_fixture("wake_mixed_fleet").await;
    let old = super::super::security_readiness_service::overlapping_test_replica(&f.state.db).await;
    super::super::security_readiness_service::remove_test_replica_capability(
        &f.state.db,
        &old,
        READINESS,
    )
    .await;
    assert!(!enabled(&f.state.db).await.unwrap());
    crate::handlers::nyxbot::process_watches(&f.state)
        .await
        .unwrap();
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    assert_eq!(row.pending_events.len(), 1);
    assert!(row.pending_events[0].wake.is_none());
    assert!(row.pending_events[0].text.contains("finished connecting"));
    let mut encoded = bson::to_document(&row.pending_events[0]).unwrap();
    encoded.remove("wake");
    let legacy: crate::models::assistant_conversation::AgentEvent =
        bson::from_document(encoded).unwrap();
    assert!(legacy.wake.is_none());
    let mut references = vec![event(
        "group_settled",
        WakeReference::GroupFollower {
            group_id: "group".into(),
            since_seq: 0,
            upper_seq: 1,
        },
    )];
    let mut tx = f.state.db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    assert!(matches!(
        materialize(&f.state.db, &f.owner, &mut references, &mut tx).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    tx.abort_transaction().await.unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_expired_channel_preparation_never_claims_success_after_source_ttl() {
    let (f, mut watch) = watch_fixture("wake_channel_ttl").await;
    watch.kind = "channel_bot".into();
    f.state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .replace_one(doc! {"_id":&watch.id}, &watch)
        .await
        .unwrap();
    let claimed = watch::claim(&f.state.db, &watch, doc! {})
        .await
        .unwrap()
        .unwrap();
    expire(&f, &claimed).await;
    f.state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .delete_one(doc! {"_id":&watch.id})
        .await
        .unwrap();
    finish(&f, &f.row).await;
    let admitted = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start(&f, TurnOrigin::Event),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let text = &admitted.active_turn.as_ref().unwrap().events[0].text;
    assert!(text.contains("setup was interrupted"));
    assert!(!text.contains("is now linked"));
    assert!(admitted.pending_events.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_health_queue_pressure_rolls_back_throttle_and_two_replicas_enqueue_once() {
    use crate::models::nyxbot_channel::{COLLECTION_NAME as CHANNELS, NyxbotChannel};
    let (f, _) = watch_fixture("wake_health_pressure").await;
    let now = bson::DateTime::now();
    let channel: NyxbotChannel = bson::from_document(doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.owner,
        "channel_bot_id":uuid::Uuid::new_v4().to_string(),"platform":"lark",
        "bot_label":"Example","transport":"gateway","status":"active",
        "route_api_key_id":"test-route","created_at":now,"updated_at":now,
    })
    .unwrap();
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .insert_one(&channel)
        .await
        .unwrap();
    let events = (0..engine::MAX_PENDING_EVENTS)
        .map(|_| {
            event(
                "group_settled",
                WakeReference::GroupFollower {
                    group_id: "group".into(),
                    since_seq: 0,
                    upper_seq: 1,
                },
            )
        })
        .collect();
    engine::push_events(&f.state.db, &f.owner, &f.row.id, events)
        .await
        .unwrap();
    assert!(matches!(
        health(
            &f.state.db,
            &channel,
            &f.row.id,
            "agent",
            "routed_elsewhere",
            Utc::now()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let retained = f
        .state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id":&channel.id})
        .await
        .unwrap()
        .unwrap();
    assert!(retained.delivery_notified_at.is_none());
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"pending_events":[]}})
        .await
        .unwrap();
    let at = Utc::now();
    let (a, b) = tokio::join!(
        health(
            &f.state.db,
            &channel,
            &f.row.id,
            "agent",
            "routed_elsewhere",
            at
        ),
        health(
            &f.state.db,
            &channel,
            &f.row.id,
            "agent",
            "routed_elsewhere",
            at
        )
    );
    assert_ne!(a.unwrap(), b.unwrap());
    assert_eq!(
        engine::get(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .pending_events
            .len(),
        1
    );
    finish(&f, &f.row).await;
    let admitted = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start(&f, TurnOrigin::Event),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(admitted.active_turn.as_ref().unwrap().events.len(), 1);
    assert!(
        !health(
            &f.state.db,
            &channel,
            &f.row.id,
            "agent",
            "routed_elsewhere",
            at
        )
        .await
        .unwrap()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_owner_step_hash_commits_with_admission_and_relink_refuses_old_source() {
    use crate::models::nyxbot_channel::{COLLECTION_NAME as CHANNELS, NyxbotChannel};
    use sha2::{Digest, Sha256};
    let (f, mut watch) = watch_fixture("wake_owner_step").await;
    let bot = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    let agent = f.row.agent_id.as_deref().unwrap();
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(doc! {"_id":&bot,"user_id":&f.owner,"platform":"telegram","label":"Example"})
        .await
        .unwrap();
    let row: NyxbotChannel = bson::from_document(doc! {
        "_id":&channel,"user_id":&f.owner,"channel_bot_id":&bot,"agent_id":agent,
        "platform":"telegram","bot_label":"Example","bot_username":"example_bot",
        "transport":"gateway","status":"active","route_api_key_id":"test-route",
        "created_at":now,"updated_at":now,
    })
    .unwrap();
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .insert_one(row)
        .await
        .unwrap();
    watch.kind = "channel_bot".into();
    watch.channel_bot_id = Some(bot.clone());
    watch.agent_id = Some(agent.into());
    f.state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .replace_one(doc! {"_id":&watch.id}, &watch)
        .await
        .unwrap();
    let claimed = watch::claim(&f.state.db, &watch, doc! {})
        .await
        .unwrap()
        .unwrap();
    watch::settle(&f.state.db, &claimed, "channel_bot_linked", None, None)
        .await
        .unwrap();
    let pending = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    assert!(
        !bson::to_document(&pending.pending_events[0])
            .unwrap()
            .to_string()
            .contains("nyxlink_")
    );
    let mut tx = f.state.db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    watch::verification(&f.state.db, &f.owner, &channel, &bot, agent, &mut tx, None)
        .await
        .unwrap();
    tx.abort_transaction().await.unwrap();
    assert!(
        f.state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id":&channel})
            .await
            .unwrap()
            .unwrap()
            .link_code_hash
            .is_none()
    );
    finish(&f, &f.row).await;
    let admitted = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start(&f, TurnOrigin::Event),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let text = &admitted.active_turn.as_ref().unwrap().events[0].text;
    let code = text
        .split("?start=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .trim_end_matches('.');
    let stored = f
        .state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id":&channel})
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored.link_code_hash.as_deref()
            == Some(hex::encode(Sha256::digest(code.as_bytes())).as_str())
    );
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&channel},
            doc! {"$set":{"agent_id":uuid::Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    tx.start_transaction().await.unwrap();
    let result = watch::verification(&f.state.db, &f.owner, &channel, &bot, agent, &mut tx, None)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
    assert!(result.contains("link changed"));
    let current = f
        .state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id":&channel})
        .await
        .unwrap()
        .unwrap();
    assert!(current.link_code_hash == stored.link_code_hash);
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(doc! {"_id":&channel}, doc! {"$set":{"agent_id":agent}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&bot},
            doc! {"$set":{"platform_profile_id":"profile"}},
        )
        .await
        .unwrap();
    tx.start_transaction().await.unwrap();
    let result = watch::verification(&f.state.db, &f.owner, &channel, &bot, agent, &mut tx, None)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
    assert!(result.contains("authority changed"));
    let retained = f
        .state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id":&channel})
        .await
        .unwrap()
        .unwrap();
    assert!(retained.link_code_hash == stored.link_code_hash);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn hosted_meta_wake_rechecks_its_trusted_callback_before_issuing_an_owner_code() {
    use crate::models::nyxbot_channel::{COLLECTION_NAME as CHANNELS, NyxbotChannel};
    for (trusted, drift) in [(true, true), (true, false), (false, false)] {
        let (f, mut watch) = watch_fixture("meta_wake_live_route").await;
        let bot = uuid::Uuid::new_v4().to_string();
        let now = bson::DateTime::now();
        f.state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! {"_id":&bot,"user_id":&f.owner,"platform":"instagram","label":"Hosted inbox",
                "credential_source":"user","bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},
                "platform_bot_id":"222","webhook_registered":false,"webhook_secret_hash":"",
                "is_active":true,"status":"active","created_at":now,"updated_at":now}).await.unwrap();
        let agent = crate::services::assistant_team_service::agent(
            &f.state.db,
            &f.owner,
            f.row.agent_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
        let (channel, _) = crate::services::nyxbot_channel_setup_service::connect_meta(
            &f.state, &f.owner, None, &bot, &agent, true, None,
        )
        .await
        .unwrap();
        watch.kind = "channel_bot".into();
        watch.channel_bot_id = Some(bot);
        watch.agent_id = Some(agent.id);
        f.state
            .db
            .collection::<NyxbotWatch>(WATCHES)
            .replace_one(doc! {"_id":&watch.id}, &watch)
            .await
            .unwrap();
        let claimed = watch::claim(&f.state.db, &watch, doc! {})
            .await
            .unwrap()
            .unwrap();
        watch::settle(
            &f.state.db,
            &claimed,
            "channel_bot_linked",
            None,
            trusted.then_some(f.state.config.base_url.as_str()),
        )
        .await
        .unwrap();
        if drift {
            crate::services::key_service::update_api_key_scope_with_scope_authorization(
                &f.state.db,
                &f.owner,
                Some(&f.owner),
                &channel.route_api_key_id,
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
        }
        finish(&f, &f.row).await;
        let admitted = engine::begin_turn(
            &f.state.db,
            &f.owner,
            start(&f, TurnOrigin::Event),
            &f.state.encryption_keys,
        )
        .await
        .unwrap();
        let events = &admitted.active_turn.as_ref().unwrap().events;
        let after = f
            .state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .find_one(doc! {"_id":&channel.id})
            .await
            .unwrap()
            .unwrap();
        if trusted && !drift {
            assert!(events.iter().any(|event| event.text.contains("nyxlink_")));
            assert_ne!(after.link_code_hash, channel.link_code_hash);
        } else {
            assert!(events.iter().any(|event| event.text.contains("repair")));
            assert!(events.iter().all(|event| !event.text.contains("nyxlink_")));
            assert_eq!(after.link_code_hash, channel.link_code_hash);
            assert_eq!(after.link_code_expires_at, channel.link_code_expires_at);
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn durable_wake_review_fast_replica_leaves_live_preparation_and_consumes_other_events() {
    let (f, watch) = watch_fixture("wake_clock_skew").await;
    let claimed = watch::claim(&f.state.db, &watch, doc! {})
        .await
        .unwrap()
        .unwrap();
    // A fast replica has already selected this event. Move only its mirror into
    // the past: the source claim is still live according to MongoDB.
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"pending_events.0.wake.ready_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    engine::push_events(
        &f.state.db,
        &f.owner,
        &f.row.id,
        vec![super::super::assistant_team_service::event(
            "other",
            "Other work".into(),
            None,
        )],
    )
    .await
    .unwrap();
    finish(&f, &f.row).await;
    let admitted = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start(&f, TurnOrigin::Event),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(
        admitted.pending_events.len(),
        1,
        "unexpired preparation must remain queued"
    );
    assert_eq!(admitted.active_turn.as_ref().unwrap().events.len(), 1);
    assert_eq!(
        admitted.active_turn.as_ref().unwrap().events[0].kind,
        "other"
    );
    assert!(
        watch::settle(&f.state.db, &claimed, "connection_finished", None, None)
            .await
            .unwrap()
    );
    finish(&f, &admitted).await;
    let real = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start(&f, TurnOrigin::Event),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(real.pending_events.is_empty());
    let events = &real.active_turn.as_ref().unwrap().events;
    assert_eq!(events.len(), 1);
    assert!(events[0].text.contains("finished connecting"));
    finish(&f, &real).await;
    assert!(
        super::super::assistant_team_service::queued(&f.state.db, Some(&f.owner))
            .await
            .unwrap()
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_review_database_renewal_mirrors_lease_and_cannot_revive_expired_claim() {
    let (f, watch) = watch_fixture("wake_renewal_clock").await;
    let claimed = watch::claim(&f.state.db, &watch, doc! {})
        .await
        .unwrap()
        .unwrap();
    let initial = claimed.wake_claim.as_ref().unwrap().expires_at;
    watch::renew(&f.state.db, &claimed).await.unwrap();
    let renewed = f
        .state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .find_one(doc! {"_id":&watch.id})
        .await
        .unwrap()
        .unwrap();
    let until = renewed.wake_claim.as_ref().unwrap().expires_at;
    assert!(until >= initial);
    let row = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    let Some(WakeReference::Watch(reference)) = &row.pending_events[0].wake else {
        panic!("missing watch")
    };
    assert_eq!(reference.ready_at, until);
    expire(&f, &claimed).await;
    watch::renew(&f.state.db, &claimed).await.unwrap();
    let expired = f
        .state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .find_one(doc! {"_id":&watch.id})
        .await
        .unwrap()
        .unwrap();
    assert!(expired.wake_claim.as_ref().unwrap().expires_at < initial);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn durable_wake_review_command_costs_for_group_publication_and_watch_admission() {
    use super::super::durable_wake_tests::{monitored, take};
    use crate::models::assistant_group::{AssistantGroup, COLLECTION_NAME as GROUPS};
    let (f, watch) = watch_fixture("wake_command_costs").await;
    let claimed = watch::claim(&f.state.db, &watch, doc! {})
        .await
        .unwrap()
        .unwrap();
    watch::settle(&f.state.db, &claimed, "connection_finished", None, None)
        .await
        .unwrap();
    finish(&f, &f.row).await;
    let (db, counts) = monitored(&f.state.db).await;
    let admitted = engine::begin_turn(
        &db,
        &f.owner,
        start(&f, TurnOrigin::Event),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(admitted.active_turn.as_ref().unwrap().events.len(), 1);
    eprintln!("watch reference admission commands: {:?}", take(&counts));
    let now = bson::DateTime::now();
    let group: AssistantGroup = bson::from_document(
        doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.owner,
        "name":"Group","member_agent_ids":[&f.row.agent_id],"lead_agent_id":&f.row.agent_id,
        "created_by":"user","message_count":1_i64,"pending_agent_ids":[&f.row.agent_id],"created_at":now,"updated_at":now},
    )
    .unwrap();
    db.collection::<AssistantGroup>(GROUPS)
        .insert_one(&group)
        .await
        .unwrap();
    let mut member = admitted.clone();
    member.group_id = Some(group.id);
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    take(&counts);
    publication::bind(&db, &mut member, &mut tx).await.unwrap();
    assert!(
        member
            .active_turn
            .as_ref()
            .unwrap()
            .group_reply_in_settlement
    );
    eprintln!("group publication bind added commands: {:?}", take(&counts));
    group::with_member(
        group::Member {
            group_id: member.group_id.clone().unwrap(),
            agent_id: member.agent_id.clone().unwrap(),
            upper_seq: 1,
        },
        group::admit(&db, &mut member, &mut tx),
    )
    .await
    .unwrap();
    assert_eq!(member.group_seen_seq, 1);
    eprintln!(
        "group member consumption added commands: {:?}",
        take(&counts)
    );
    let reply = engine::messages(&f.state.db, &f.owner, &f.row.id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.role == "assistant")
        .unwrap();
    publication::publish(
        &db,
        &member,
        member.active_turn.as_ref().unwrap(),
        &reply,
        &mut tx,
    )
    .await
    .unwrap();
    eprintln!(
        "group publication without handoff added commands: {:?}",
        take(&counts)
    );
    tx.abort_transaction().await.unwrap();
    tx.start_transaction().await.unwrap();
    take(&counts);
    let events = admitted.active_turn.as_ref().unwrap().events.clone();
    let (mut eligible, preparing) = partition_in_session(&db, &f.owner, &f.row.id, events, &mut tx)
        .await
        .unwrap();
    materialize(&db, &f.owner, &mut eligible, &mut tx)
        .await
        .unwrap();
    assert!(preparing.is_empty());
    assert_eq!(eligible.len(), 1);
    eprintln!(
        "watch reference partition and materialization added commands: {:?}",
        take(&counts)
    );
    tx.abort_transaction().await.unwrap();
    db.drop().await.unwrap();
}
