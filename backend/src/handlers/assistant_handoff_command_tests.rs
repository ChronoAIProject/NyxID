use super::*;
use crate::services::{
    assistant_nyxagent_runtime as runtime, feature_flag_service as flags,
    security_readiness_service as readiness,
};
use mongodb::event::{EventHandler, command::CommandEvent};

async fn check_cost(enabled: bool, fleet_ready: bool) {
    let (original, calls, server) = setup(None, Duration::from_millis(5)).await;
    let commands = Arc::new(AtomicUsize::new(0));
    let observed = commands.clone();
    let monitor = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "handoff_commands",
        EventHandler::callback(move |event| {
            if matches!(event, CommandEvent::Started(_)) {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }),
    )
    .await;
    let state = test_app_state(monitor.client().database(original.db.name()));
    readiness::observe_test_handoff_readiness(&state.db, true).await;
    if enabled {
        flags::set_platform_override(
            &state.db,
            flags::TURN_HANDOFF_FLAG_KEY,
            &flags::FlagTarget::User(OWNER.into()),
            true,
            OWNER,
        )
        .await
        .unwrap();
    }
    if enabled && fleet_ready {
        runtime::fetch_capabilities(&state, runtime::CapabilityLookup::CurrentCatalog, |_| async {
            Ok(Json(json!({"reattach":{"protocol":"nyxagent-reattach-v1","credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15},"recent_events":32,"max_events":64,"max_event_bytes":65536,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32}})).into_response())
        }).await.unwrap();
        MODELS.lock().await.insert(
            format!("{}:{OWNER}", state.db.name()),
            (
                Instant::now(),
                Some(vec![ModelInfo::from(
                    model_response(
                        &json!({"id":engine::DEFAULT_MODEL,"inference_credential":"thread"}),
                    )
                    .unwrap(),
                )]),
            ),
        );
    }
    if !fleet_ready {
        readiness::observe_test_handoff_readiness(&state.db, false).await;
    }
    commands.store(0, Ordering::SeqCst);
    let response = turns(
        State(state.clone()),
        test_auth_user(OWNER),
        turn_request(None),
    )
    .await
    .unwrap();
    assert!(drain(response).await.contains("turn.completed"));
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stream_commands = commands.load(Ordering::SeqCst);
    let captured = calls.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].body["background"] == true,
        enabled && fleet_ready
    );
    drop(captured);
    let saved = settled(&state).await;
    let mut input = engine::parse_turn(br#"{"text":"second"}"#).unwrap();
    input.conversation_id = Some(saved.id);
    let ordinary = engine::begin_turn(&state.db, OWNER, &input, &state.encryption_keys)
        .await
        .unwrap();
    let active = ordinary.active_turn.unwrap();
    assert!(active.restart_admission.is_none());
    assert_eq!(
        active.restart_eligibility.as_deref(),
        (enabled && fleet_ready).then_some("discovery_skipped")
    );
    commands.store(0, Ordering::SeqCst);
    assert_eq!(
        stop(
            State(state.clone()),
            test_auth_user(OWNER),
            Path(ordinary.id)
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    let stop_commands = commands.load(Ordering::SeqCst);
    server.abort();
    let _ = server.await;
    state.db.drop().await.unwrap();
    monitor.drop().await.unwrap();
    // Provisioning the conversation's Agent Key inserts it through the personal
    // erasure fence (find + upsert on personal_owner_write_fences, then a
    // service_accounts lookup), added with #2859.
    assert_eq!((stream_commands, stop_commands), (130, 17));
}

#[tokio::test]
async fn handoff_disabled_ordinary_turn_command_counts() {
    check_cost(false, true).await;
}

#[tokio::test]
async fn handoff_enabled_ordinary_turn_command_counts() {
    check_cost(true, true).await;
}

#[tokio::test]
async fn handoff_mixed_fleet_ordinary_turn_command_counts() {
    check_cost(true, false).await;
}

#[tokio::test]
async fn model_listing_waits_for_a_contended_cache_entry() {
    let (state, calls, server) = setup(None, Duration::from_millis(5)).await;
    let auth = test_auth_user(OWNER);
    let mut cache = MODELS.lock().await;
    let listing = live_models(&state, &auth, None);
    tokio::pin!(listing);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut listing)
            .await
            .is_err()
    );
    let profile = model_response(&json!({"id":"nyxagent/test-cached-profile"})).unwrap();
    cache.insert(
        format!("{}:{OWNER}", state.db.name()),
        (Instant::now(), Some(vec![ModelInfo::from(profile)])),
    );
    drop(cache);
    assert_eq!(listing.await.unwrap()[0].id, "nyxagent/test-cached-profile");
    assert!(calls.lock().await.is_empty());
    server.abort();
    let _ = server.await;
    state.db.drop().await.unwrap();
}
