use super::*;
use crate::handlers::assistant_nyxagent::{SERVER_TURN_POLICY, start_turn};
use crate::services::{feature_flag_service as flags, security_readiness_service as readiness};
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "assistant_team_handoff_tests.rs"]
mod team_handoff;

#[derive(Default, Clone)]
struct Requests {
    capabilities: Arc<AtomicUsize>,
    models: Arc<AtomicUsize>,
}

impl Requests {
    fn counts(&self) -> (usize, usize) {
        (
            self.capabilities.load(Ordering::SeqCst),
            self.models.load(Ordering::SeqCst),
        )
    }

    fn router(&self, fail_models: bool) -> Router {
        let capabilities = self.capabilities.clone();
        let models = self.models.clone();
        Router::new().route("/v1/capabilities", axum::routing::get(move |headers: HeaderMap| async move {
            assert!(headers["authorization"].to_str().unwrap().starts_with("Bearer nyx"));
            capabilities.fetch_add(1, Ordering::SeqCst);
            Json(json!({"reattach":{"protocol":"nyxagent-reattach-v1","credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15},"recent_events":32,"max_events":64,"max_event_bytes":65536,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32}}))
        })).route("/v1/models", axum::routing::get(move |headers: HeaderMap, uri: axum::http::Uri| async move {
            assert_eq!(uri.query(), Some("include_capabilities=true"));
            assert!(headers["authorization"].to_str().unwrap().starts_with("Bearer nyx"));
            models.fetch_add(1, Ordering::SeqCst);
            if fail_models { return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(); }
            Json(json!({"data":[{"id":engine::DEFAULT_MODEL,"inference_credential":"thread"}]})).into_response()
        }))
    }
}

async fn set_flag(state: &AppState) {
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

#[tokio::test]
async fn cold_schedule_dispatch_fetches_metadata_and_enrolls() {
    let requests = Requests::default();
    let (state, calls, server) =
        setup_with_discovery("schedule_discovery", "done", requests.router(false)).await;
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let trigger = automation(&state, &agent.id, Some(ThreadPolicy::Home)).await;
    make_due(&state, &trigger).await;
    readiness::observe_test_handoff_readiness(&state.db, true).await;
    set_flag(&state).await;
    tick(&state).await.unwrap();
    assert_eq!(
        completed(&state, &trigger).await.outcome,
        RunOutcome::Completed
    );
    assert_eq!(requests.counts(), (1, 1));
    let calls = calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].body["background"], true);
    server.abort();
    let _ = server.await;
    state.db.drop().await.unwrap();
}

async fn owner_turn(state: &AppState, id: &str) -> Option<String> {
    engine::require_enabled(&state.db, OWNER).await.unwrap();
    let mut input = engine::parse_turn(br#"{"text":"hello"}"#).unwrap();
    input.conversation_id = Some(id.into());
    let mut start = engine::TurnStart::from(&input);
    start.owner_instruction_source = Some(engine::OwnerInstructionSource::FirstPartyOwner);
    let permit = state.direct_chat_limiter.admit(OWNER).await.unwrap();
    let (row, mut events) = Box::pin(start_turn(
        state,
        crate::test_utils::test_auth_user(OWNER),
        &start,
        Some(SERVER_TURN_POLICY),
        permit,
    ))
    .await
    .unwrap();
    let active = row.active_turn.unwrap();
    assert_eq!(
        active.restart_admission.is_some(),
        active.restart_eligibility.as_deref() == Some("watched_thread_credential")
    );
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if events.recv().await.unwrap()["event"] == "turn.completed" {
                break;
            }
        }
    })
    .await
    .unwrap();
    active.restart_eligibility
}

#[tokio::test]
async fn cold_owner_discovery_honors_gates_and_reuses_results() {
    let requests = Requests::default();
    let (state, calls, server) =
        setup_with_discovery("owner_discovery", "done", requests.router(false)).await;
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let home = team::home_thread(&state.db, &state.encryption_keys, &agent)
        .await
        .unwrap();
    readiness::observe_test_handoff_readiness(&state.db, true).await;
    assert_eq!(owner_turn(&state, &home.id).await, None);
    assert_eq!(requests.counts(), (0, 0));
    set_flag(&state).await;
    for _ in 0..2 {
        assert_eq!(
            owner_turn(&state, &home.id).await.as_deref(),
            Some("watched_thread_credential")
        );
        assert_eq!(requests.counts(), (1, 1));
    }
    assert_eq!(calls.lock().await.len(), 3);
    server.abort();
    let _ = server.await;
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn cold_owner_failed_profile_fetch_records_reason_and_runs_foreground() {
    let requests = Requests::default();
    let (state, calls, server) =
        setup_with_discovery("owner_profile_failure", "done", requests.router(true)).await;
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let home = team::home_thread(&state.db, &state.encryption_keys, &agent)
        .await
        .unwrap();
    readiness::observe_test_handoff_readiness(&state.db, true).await;
    set_flag(&state).await;
    assert_eq!(
        owner_turn(&state, &home.id).await.as_deref(),
        Some("profile_unknown")
    );
    assert_eq!(requests.counts(), (1, 1));
    assert_ne!(calls.lock().await[0].body["background"], true);
    server.abort();
    let _ = server.await;
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn automation_group_admission_flags_match_personal_resolution() {
    use crate::models::{assistant_group, feature_flag_override, org_membership};
    use crate::services::{
        assistant_group_service as groups, assistant_notification_service as notifications,
    };
    use bson::{DateTime, Document, doc};

    let db = connect_transaction_test_database("group_dispatch_flags").await;
    readiness::observe_test_handoff_readiness(&db, true).await;
    let group_id = format!("nyxg-{}", Uuid::new_v4().simple());
    let now = DateTime::now();
    db.collection::<Document>(assistant_group::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &group_id, "user_id": OWNER, "name": "Test group", "member_agent_ids": [],
            "lead_agent_id": "lead", "created_by": "user", "message_count": 0,
            "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    let org = Uuid::new_v4().to_string();
    let memberships = db.collection::<Document>(org_membership::COLLECTION_NAME);
    memberships
        .insert_one(doc! {"_id": "membership", "org_user_id": &org,
        "member_user_id": OWNER, "role": "viewer", "created_at": now})
        .await
        .unwrap();
    let keys = [notifications::FLAG_KEY, flags::TURN_HANDOFF_FLAG_KEY];
    let compare = |expected: [bool; 2]| {
        let (db, group_id) = (&db, &group_id);
        async move {
            let canonical = flags::resolve_personal_features(db, OWNER).await.unwrap();
            assert_eq!(
                keys.map(|key| canonical.iter().any(|flag| flag == key)),
                expected
            );
            notifications::reset_flag_cache(db);
            groups::get_for_automation_dispatch(db, OWNER, group_id)
                .await
                .unwrap();
            assert!(notifications::has_admission_features(db, OWNER));
            assert_eq!(
                [
                    notifications::enabled(db, OWNER).await.unwrap(),
                    notifications::turn_handoff_enabled(db, OWNER)
                ],
                expected
            );
        }
    };
    let overrides = db.collection::<Document>(feature_flag_override::COLLECTION_NAME);
    for (scope, kind, target, definition, values, expected) in [
        (None, "global", None, None, [true, false], [true, false]),
        (
            Some(org.as_str()),
            "org",
            None,
            None,
            [false, true],
            [false, true],
        ),
        (
            Some(org.as_str()),
            "role",
            Some("viewer"),
            None,
            [true, false],
            [true, false],
        ),
        (
            Some(org.as_str()),
            "user",
            Some(OWNER),
            None,
            [false, true],
            [false, true],
        ),
        (
            None,
            "user",
            Some(OWNER),
            Some("runtime-definition"),
            [true, false],
            [false, true],
        ),
    ] {
        for (key, enabled) in keys.into_iter().zip(values) {
            overrides.insert_one(doc! {"_id": Uuid::new_v4().to_string(), "org_user_id": scope,
                "flag_key": key, "target_kind": kind, "target_key": target, "definition_id": definition,
                "enabled": enabled, "created_at": now, "updated_at": now, "updated_by": OWNER}).await.unwrap();
        }
        compare(expected).await;
    }
    memberships
        .update_one(doc! {"_id":"membership"}, doc! {"$set":{"revoked_at": now}})
        .await
        .unwrap();
    compare([true, false]).await;
    for (key, enabled) in keys.into_iter().zip([false, true]) {
        overrides
            .insert_one(doc! {"_id": Uuid::new_v4().to_string(), "flag_key": key,
            "target_kind": "user", "target_key": OWNER, "enabled": enabled, "created_at": now,
            "updated_at": now, "updated_by": OWNER})
            .await
            .unwrap();
    }
    compare([false, true]).await;
    db.drop().await.unwrap();
}
