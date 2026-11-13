use super::tests::{local, row};
use super::*;

async fn fixture(name: &str, required: bool) -> (Database, Runtime) {
    let db = crate::test_utils::connect_transaction_test_database(name).await;
    ensure_indexes(&db).await.unwrap();
    let local = local("a", "a1", &["a"], required);
    let mut duplicate = row(&local, Utc::now());
    duplicate.id = "a2".into();
    duplicate.generation_id = duplicate.id.clone();
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .insert_many([row(&local, Utc::now()), duplicate])
        .await
        .unwrap();
    (db, local)
}
async fn pass(db: &Database, local: &Runtime, capability: &str, now: Instant) {
    record_activations(db, local, &[capability], now)
        .await
        .unwrap();
}
async fn active(db: &Database, local: &Runtime, capability: &str) -> bool {
    report_for(db, local, capability).await.unwrap().activated
}

pub(super) async fn assess_every_ten_seconds(
    db: &Database,
    local: &Runtime,
    capabilities: &[&str],
    start: Instant,
    until: Instant,
) {
    let mut next = start;
    while next < until {
        next = (next + std::time::Duration::from_secs(10)).min(until);
        record_activations(db, local, capabilities, next)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn first_activation_ninety_second_assessment_gap_restarts_countdown_and_requires_full_window()
{
    let (db, local) = fixture("settle_assessment_gap", false).await;
    let cap = CAPABILITIES[0];
    let start = Instant::now();
    pass(&db, &local, cap, start).await;
    let restarted = start + ACTIVATION_SETTLE_WINDOW;
    pass(&db, &local, cap, restarted).await;
    assert!(!active(&db, &local, cap).await);
    assert_eq!(local.pending_seconds(cap, restarted), Some(90));
    assert_eq!(
        report_for(&db, &local, cap)
            .await
            .unwrap()
            .activation_pending_seconds,
        Some(90)
    );

    for elapsed in (10..=90).step_by(10) {
        let now = restarted + std::time::Duration::from_secs(elapsed);
        pass(&db, &local, cap, now).await;
        assert_eq!(active(&db, &local, cap).await, elapsed == 90);
        assert_eq!(
            local.pending_seconds(cap, now),
            (elapsed < 90).then_some(90 - elapsed)
        );
    }
}

#[tokio::test]
async fn first_activation_ten_second_assessments_record_at_ninety_seconds() {
    let (db, local) = fixture("settle_consecutive_heartbeats", false).await;
    let cap = CAPABILITIES[0];
    let start = Instant::now();
    for elapsed in (0..=90).step_by(10) {
        let now = start + std::time::Duration::from_secs(elapsed);
        pass(&db, &local, cap, now).await;
        assert_eq!(active(&db, &local, cap).await, elapsed == 90);
        assert_eq!(
            local.pending_seconds(cap, now),
            (elapsed < 90).then_some(90 - elapsed)
        );
    }
}

#[tokio::test]
async fn first_activation_gap_at_freshness_horizon_continues_but_one_nanosecond_over_restarts() {
    for excess in [
        std::time::Duration::ZERO,
        std::time::Duration::from_nanos(1),
    ] {
        let (db, local) = fixture("settle_gap_boundary", false).await;
        let cap = CAPABILITIES[0];
        let start = Instant::now();
        let horizon = std::time::Duration::from_secs(MAX_AGE_SECS as u64);
        pass(&db, &local, cap, start).await;
        let second = start + horizon + excess;
        pass(&db, &local, cap, second).await;
        assert!(!active(&db, &local, cap).await);
        assert_eq!(
            local.pending_seconds(cap, second),
            Some(if excess.is_zero() { 45 } else { 90 })
        );
        pass(&db, &local, cap, start + ACTIVATION_SETTLE_WINDOW).await;
        assert_eq!(active(&db, &local, cap).await, excess.is_zero());
        if !excess.is_zero() {
            let end = second + ACTIVATION_SETTLE_WINDOW;
            pass(&db, &local, cap, end - excess).await;
            assert!(!active(&db, &local, cap).await);
            assert_eq!(local.pending_seconds(cap, end - excess), Some(1));
            pass(&db, &local, cap, end).await;
            assert!(active(&db, &local, cap).await);
        }
    }
}

#[tokio::test]
async fn first_activation_settles_duplicate_undeclared_and_inconsistent_rosters() {
    for condition in ["duplicate", "undeclared", "inconsistent"] {
        let (db, local) = fixture(condition, false).await;
        let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
        match condition {
            "undeclared" => {
                replicas
                    .update_one(doc! {"_id":"a2"}, doc! {"$set":{"instance_name":"b"}})
                    .await
                    .unwrap();
            }
            "inconsistent" => {
                replicas
                    .update_one(
                        doc! {"_id":"a2"},
                        doc! {"$set":{"instance_name":"b", "expected_replicas":["a","b"]}},
                    )
                    .await
                    .unwrap();
            }
            _ => {}
        }
        let cap = CAPABILITIES[0];
        let start = Instant::now();
        let before = report_for(&db, &local, cap).await.unwrap();
        assert!(serde_json::to_value(before).unwrap()["activation_pending_seconds"].is_null());
        pass(&db, &local, cap, start).await;
        let pending = report_for(&db, &local, cap).await.unwrap();
        assert!(!pending.ready && !pending.serving_ready && !pending.activated);
        assert!(
            pending
                .serving_blockers
                .contains(&"capability_not_activated".into())
        );
        assert!(
            pending
                .activation_pending_seconds
                .is_some_and(|s| s > 0 && s <= 90)
        );
        assess_every_ten_seconds(
            &db,
            &local,
            &[cap],
            start,
            start + ACTIVATION_SETTLE_WINDOW - std::time::Duration::from_secs(1),
        )
        .await;
        assert!(!active(&db, &local, cap).await);
        pass(&db, &local, cap, start + ACTIVATION_SETTLE_WINDOW).await;
        let after = report_for(&db, &local, cap).await.unwrap();
        assert!(!after.ready && after.serving_ready && after.activated);
        assert!(after.activation_pending_seconds.is_none());
    }
}

#[tokio::test]
async fn first_activation_settles_missing_roster_names_empty_rosters_and_undrained_declarations() {
    for roster in [vec!["a", "missing"], vec![]] {
        let (db, mut local) = fixture("advisory_roster", false).await;
        local.config.expected_replicas = roster.into_iter().map(String::from).collect();
        local.config.drained = false;
        db.collection::<SecurityReplica>(COLLECTION_NAME)
            .update_many(doc! {}, doc! {"$set":{"drained":false}})
            .await
            .unwrap();
        let cap = CAPABILITIES[0];
        let start = Instant::now();
        pass(&db, &local, cap, start).await;
        assess_every_ten_seconds(
            &db,
            &local,
            &[cap],
            start,
            start + ACTIVATION_SETTLE_WINDOW - std::time::Duration::from_secs(1),
        )
        .await;
        assert!(!active(&db, &local, cap).await);
        pass(&db, &local, cap, start + ACTIVATION_SETTLE_WINDOW).await;
        let report = report_for(&db, &local, cap).await.unwrap();
        assert!(!report.ready && report.activated && report.serving_ready);
        assert!(
            report
                .roster_warnings
                .contains(&"old_replicas_not_declared_drained".into())
        );
    }
}

#[tokio::test]
async fn first_activation_clean_is_immediate_and_restart_restarts_settle_window() {
    let (db, local) = fixture("settle_restart", false).await;
    let cap = CAPABILITIES[0];
    let start = Instant::now();
    pass(&db, &local, cap, start).await;
    let restarted = Runtime::new(local.identity.clone(), local.config.clone(), false);
    pass(&db, &restarted, cap, start + ACTIVATION_SETTLE_WINDOW).await;
    assert!(!active(&db, &restarted, cap).await);
    assess_every_ten_seconds(
        &db,
        &restarted,
        &[cap],
        start + ACTIVATION_SETTLE_WINDOW,
        start + 2 * ACTIVATION_SETTLE_WINDOW,
    )
    .await;
    assert!(active(&db, &restarted, cap).await);
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .delete_one(doc! {"_id":"a2"})
        .await
        .unwrap();
    pass(&db, &local, CAPABILITIES[1], start).await;
    assert!(active(&db, &local, CAPABILITIES[1]).await);
}

#[tokio::test]
async fn first_activation_window_starts_after_database_and_policy_inventory_complete() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    let seconds = Arc::new(AtomicU64::new(0));
    let delayed_inventory = Arc::new(AtomicBool::new(false));
    let observed_seconds = seconds.clone();
    let delay = delayed_inventory.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "settle_after_inventory",
        EventHandler::callback(move |event| {
            if let CommandEvent::Succeeded(event) = event
                && event.command_name == "aggregate"
                && delay.load(Ordering::SeqCst)
            {
                // Simulate time spent completing a real inventory read; no sleep.
                observed_seconds.store(30, Ordering::SeqCst);
            }
        }),
    )
    .await;
    ensure_indexes(&db).await.unwrap();
    let local = Runtime::new(
        local("a", "a1", &["a"], true).identity,
        RolloutConfig::default(),
        true,
    );
    let capability = PLATFORM_POLICY;
    publish_observation(&db, &local, &[capability])
        .await
        .unwrap();
    let start = Instant::now();
    let clock = || start + std::time::Duration::from_secs(seconds.load(Ordering::SeqCst));
    delayed_inventory.store(true, Ordering::SeqCst);
    record_activations_with_clock(&db, &local, &[capability], clock)
        .await
        .unwrap();
    assert_eq!(seconds.load(Ordering::SeqCst), 30);
    assert_eq!(local.pending_seconds(capability, clock()), Some(90));
    assert!(!active(&db, &local, capability).await);

    delayed_inventory.store(false, Ordering::SeqCst);
    for elapsed in [60, 90, 119, 120] {
        seconds.store(elapsed, Ordering::SeqCst);
        record_activations_with_clock(&db, &local, &[capability], clock)
            .await
            .unwrap();
        assert_eq!(active(&db, &local, capability).await, elapsed == 120);
    }
}

#[tokio::test]
async fn first_activation_settle_resets_for_every_hard_blocker_and_assessment_error() {
    for condition in [
        "unsupported",
        "protocol",
        "executing_missing",
        "future",
        "malformed_observation",
        "malformed_evidence",
        "bound",
    ] {
        let (db, mut local) = fixture(condition, false).await;
        let cap = CAPABILITIES[0];
        let start = Instant::now();
        pass(&db, &local, cap, start).await;
        let replicas = db.collection::<bson::Document>(COLLECTION_NAME);
        let originals: Vec<SecurityReplica> = db
            .collection(COLLECTION_NAME)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        match condition {
            "unsupported" => {
                replicas
                    .update_one(doc! {"_id":"a2"}, doc! {"$pull":{"capabilities":cap}})
                    .await
                    .unwrap();
            }
            "protocol" => {
                replicas
                    .update_one(doc! {"_id":"a2"}, doc! {"$set":{"protocol":2}})
                    .await
                    .unwrap();
            }
            "executing_missing" => {
                replicas.delete_one(doc! {"_id":"a1"}).await.unwrap();
            }
            "future" => {
                replicas.update_one(doc! {"_id":"a2"}, doc! {"$set":{"observed_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(30))}}).await.unwrap();
            }
            "malformed_observation" => {
                replicas
                    .insert_one(doc! {"_id":"bad", "observed_at":bson::DateTime::now()})
                    .await
                    .unwrap();
            }
            "malformed_evidence" => {
                db.collection::<bson::Document>(ROLLOUTS)
                    .insert_one(doc! {"_id":"bad", "record_kind":RECORD_KIND,"capability":cap})
                    .await
                    .unwrap();
            }
            "bound" => {
                replicas
                    .insert_many((0..255).map(|i| {
                        let mut r = row(&local, Utc::now());
                        r.id = format!("extra-{i}");
                        r.generation_id = r.id.clone();
                        bson::to_document(&r).unwrap()
                    }))
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let result = record_activations(
            &db,
            &local,
            &[cap],
            start + std::time::Duration::from_secs(45),
        )
        .await;
        assert_eq!(
            result.is_err(),
            matches!(
                condition,
                "malformed_observation" | "malformed_evidence" | "bound"
            ),
            "{condition}"
        );
        assert!(local.pending_seconds(cap, start).is_none(), "{condition}");
        local.config = RolloutConfig::parse("a", true).unwrap();
        replicas.delete_many(doc! {}).await.unwrap();
        db.collection::<SecurityReplica>(COLLECTION_NAME)
            .insert_many(originals)
            .await
            .unwrap();
        db.collection::<bson::Document>(ROLLOUTS)
            .delete_one(doc! {"_id":"bad"})
            .await
            .unwrap();
        pass(&db, &local, cap, start + ACTIVATION_SETTLE_WINDOW).await;
        assert!(!active(&db, &local, cap).await, "{condition}");
        assess_every_ten_seconds(
            &db,
            &local,
            &[cap],
            start + ACTIVATION_SETTLE_WINDOW,
            start + 2 * ACTIVATION_SETTLE_WINDOW,
        )
        .await;
        assert!(active(&db, &local, cap).await, "{condition}");
    }
}

#[tokio::test]
async fn first_activation_platform_policy_inventory_and_sticky_local_false_remain_hard() {
    for condition in [
        "local_false",
        "sibling_false",
        "all_false",
        "missing_policy",
    ] {
        let (db, mut local) = fixture(condition, true).await;
        let start = Instant::now();
        pass(&db, &local, PLATFORM_POLICY, start).await;
        match condition {
            "local_false" => local.platform_policy_required = false,
            "sibling_false" => {
                db.collection::<bson::Document>(COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":"a2"},
                        doc! {"$set":{"platform_policy_required":false}},
                    )
                    .await
                    .unwrap();
            }
            "all_false" => {
                local.platform_policy_required = false;
                db.collection::<bson::Document>(COLLECTION_NAME)
                    .update_many(doc! {}, doc! {"$set":{"platform_policy_required":false}})
                    .await
                    .unwrap();
            }
            "missing_policy" => {
                db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME).insert_one(doc! {"_id":"catalog", "is_active":true,"auth_method":"bearer","requires_user_credential":false}).await.unwrap();
            }
            _ => unreachable!(),
        }
        pass(
            &db,
            &local,
            PLATFORM_POLICY,
            start + ACTIVATION_SETTLE_WINDOW,
        )
        .await;
        assert!(!active(&db, &local, PLATFORM_POLICY).await);
        assert!(local.pending_seconds(PLATFORM_POLICY, start).is_none());
    }
}

#[tokio::test]
async fn first_activation_rollout_expires_old_writer_settles_then_refuses_regression() {
    let (db, local) = fixture("settle_rollout", false).await;
    let cap = CAPABILITIES[0];
    let mut old = row(&local, Utc::now());
    old.id = "old".into();
    old.generation_id = "old".into();
    old.capabilities.retain(|c| c != cap);
    let replicas = db.collection::<SecurityReplica>(COLLECTION_NAME);
    replicas.insert_one(&old).await.unwrap();
    let start = Instant::now();
    pass(&db, &local, cap, start).await;
    assert!(!report_for(&db, &local, cap).await.unwrap().serving_ready);
    old.observed_at = Utc::now() - Duration::seconds(MAX_AGE_SECS + 1);
    replicas
        .replace_one(doc! {"_id":"old"}, &old)
        .await
        .unwrap();
    pass(&db, &local, cap, start + ACTIVATION_SETTLE_WINDOW).await;
    assert!(!active(&db, &local, cap).await);
    assess_every_ten_seconds(
        &db,
        &local,
        &[cap],
        start + ACTIVATION_SETTLE_WINDOW,
        start + 2 * ACTIVATION_SETTLE_WINDOW,
    )
    .await;
    assert!(report_for(&db, &local, cap).await.unwrap().serving_ready);
    old.observed_at = Utc::now();
    replicas
        .replace_one(doc! {"_id":"old"}, &old)
        .await
        .unwrap();
    assert!(!report_for(&db, &local, cap).await.unwrap().serving_ready);
}

#[tokio::test]
async fn first_activation_racing_processes_insert_once_and_audit_only_winning_pass() {
    let (db, a) = fixture("settle_race_audit", false).await;
    let mut b = Runtime::new(a.identity.clone(), a.config.clone(), false);
    b.identity.generation_id = "a2".into();
    let caps = &CAPABILITIES[..2];
    let start = Instant::now();
    record_activations(&db, &a, caps, start).await.unwrap();
    record_activations(&db, &b, caps, start).await.unwrap();
    let before_window = start + ACTIVATION_SETTLE_WINDOW - std::time::Duration::from_secs(10);
    assess_every_ten_seconds(&db, &a, caps, start, before_window).await;
    assess_every_ten_seconds(&db, &b, caps, start, before_window).await;
    let (left, right) = tokio::join!(
        record_activations(&db, &a, caps, start + ACTIVATION_SETTLE_WINDOW),
        record_activations(&db, &b, caps, start + ACTIVATION_SETTLE_WINDOW)
    );
    left.unwrap();
    right.unwrap();
    let (_, before) = rollout_state(&db, caps).await.unwrap();
    assert_eq!(before.len(), 2);
    record_activations(&db, &a, caps, start + 2 * ACTIVATION_SETTLE_WINDOW)
        .await
        .unwrap();
    assert_eq!(rollout_state(&db, caps).await.unwrap().1, before);
    let audits: Vec<crate::models::audit_log::AuditLog> = db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":"security_capability_activated"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(!audits.is_empty() && audits.len() <= 2);
    let mut inserted = vec![];
    for audit in audits {
        assert!(audit.seq.is_some() && audit.entry_hash.is_some());
        let data = audit.event_data.unwrap();
        assert_eq!(data["roster_clean"], false);
        assert_eq!(
            data["roster_warnings"],
            serde_json::json!(["missing_or_duplicate_replica:a"])
        );
        assert_eq!(data["live_generations"], 2);
        assert_eq!(data["expected_replicas"], serde_json::json!(["a"]));
        inserted.extend(
            data["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_str().unwrap().to_owned()),
        );
    }
    inserted.sort();
    let mut expected = caps.to_vec();
    expected.sort();
    assert_eq!(inserted, expected);
}

#[tokio::test]
async fn first_activation_clean_pass_has_one_audit_and_failed_audit_never_fails_observation() {
    let (db, local) = fixture("activation_audit_failure", false).await;
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .delete_one(doc! {"_id":"a2"})
        .await
        .unwrap();
    record_activations(&db, &local, &CAPABILITIES[..2], Instant::now())
        .await
        .unwrap();
    let audits = db.collection::<crate::models::audit_log::AuditLog>(
        crate::models::audit_log::COLLECTION_NAME,
    );
    let audit = audits
        .find_one(doc! {"event_type":"security_capability_activated"})
        .await
        .unwrap()
        .unwrap();
    let data = audit.event_data.unwrap();
    assert_eq!(data["roster_clean"], true);
    assert_eq!(data["capabilities"].as_array().unwrap().len(), 2);
    record_activations(&db, &local, &CAPABILITIES[..2], Instant::now())
        .await
        .unwrap();
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"security_capability_activated"})
            .await
            .unwrap(),
        1
    );
    db.run_command(doc! {"collMod":crate::models::audit_log::COLLECTION_NAME,"validator":{"event_type":{"$ne":"security_capability_activated"}}}).await.unwrap();
    observe_for(&db, &local, &CAPABILITIES[..3]).await.unwrap();
    assert!(active(&db, &local, CAPABILITIES[2]).await);
    assert_eq!(
        audits
            .count_documents(doc! {"event_type":"security_capability_activated"})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn first_activation_database_read_error_resets_window_without_failing_observation() {
    let (db, local) = fixture("activation_read_error", false).await;
    let cap = CAPABILITIES[0];
    let start = Instant::now();
    pass(&db, &local, cap, start).await;
    let bad = db.client().database_with_options(
        db.name(),
        mongodb::options::DatabaseOptions::builder()
            .read_concern(mongodb::options::ReadConcern::custom(
                "invalid-test-read-concern",
            ))
            .build(),
    );
    // A real server command error, scoped to this handle; no global failpoint.
    assert!(matches!(
        record_activations(
            &bad,
            &local,
            &[cap],
            start + std::time::Duration::from_secs(45)
        )
        .await,
        Err(AppError::DatabaseError(_))
    ));
    assert!(local.pending_seconds(cap, start).is_none());
    observe_for(&bad, &local, &[cap]).await.unwrap();
    pass(&db, &local, cap, start + ACTIVATION_SETTLE_WINDOW).await;
    assert!(!active(&db, &local, cap).await);
    assess_every_ten_seconds(
        &db,
        &local,
        &[cap],
        start + ACTIVATION_SETTLE_WINDOW,
        start + 2 * ACTIVATION_SETTLE_WINDOW,
    )
    .await;
    assert!(active(&db, &local, cap).await);
}

#[tokio::test]
async fn heartbeat_command_counts_keep_steady_state_unchanged_and_add_only_insertion_audit() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(Mutex::new(Vec::new()));
    let captured = commands.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "activation_heartbeat_commands",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                captured.lock().unwrap().push(event.command_name);
            }
        }),
    )
    .await
    .unwrap();
    ensure_indexes(&db).await.unwrap();
    let local = local("a", "a1", &["a"], false);
    let cap = CAPABILITIES[0];
    // #2422's ordinary heartbeat: publish, live observations, rollout evidence,
    // then one upsert per new clean capability. No activation audit existed.
    commands.lock().unwrap().clear();
    publish_observation(&db, &local, &[cap]).await.unwrap();
    let live = rows(&db).await.unwrap();
    let (cutover, active) = rollout_state(&db, &[cap]).await.unwrap();
    assert!(active.is_empty() && assess(&live, &local, cap, cutover, Utc::now()).ready);
    insert_activation(&db, cap).await.unwrap();
    let before_new = commands.lock().unwrap().clone();
    assert_eq!(before_new, ["update", "find", "find", "update"]);
    db.collection::<bson::Document>(ROLLOUTS)
        .delete_many(doc! {"record_kind":RECORD_KIND})
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    observe_for(&db, &local, &[cap]).await.unwrap();
    let after_new = commands.lock().unwrap().clone();
    assert_eq!(
        after_new,
        ["update", "find", "find", "update", "find", "insert"]
    );
    commands.lock().unwrap().clear();
    publish_observation(&db, &local, &[cap]).await.unwrap();
    rows(&db).await.unwrap();
    rollout_state(&db, &[cap]).await.unwrap();
    let before_active = commands.lock().unwrap().clone();
    commands.lock().unwrap().clear();
    observe_for(&db, &local, &[cap]).await.unwrap();
    let after_active = commands.lock().unwrap().clone();
    assert_eq!(after_active, before_active);
    assert_eq!(after_active, ["update", "find", "find"]);
    eprintln!(
        "heartbeat new capability: production={before_new:?}, followup={after_new:?}; active: production={before_active:?}, followup={after_active:?}"
    );
    let mut duplicate = row(&local, Utc::now());
    duplicate.id = "a2".into();
    duplicate.generation_id = "a2".into();
    db.collection::<SecurityReplica>(COLLECTION_NAME)
        .insert_one(duplicate)
        .await
        .unwrap();
    db.collection::<bson::Document>(ROLLOUTS)
        .delete_many(doc! {"record_kind":RECORD_KIND})
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    observe_for(&db, &local, &[cap]).await.unwrap();
    assert_eq!(*commands.lock().unwrap(), ["update", "find", "find"]);
}

#[tokio::test]
async fn first_activation_settles_despite_missing_or_empty_sibling_declarations() {
    for declaration in [vec!["a", "missing"], vec![]] {
        let (db, local) = fixture("sibling_advisory_declaration", false).await;
        let cap = CAPABILITIES[0];
        let start = Instant::now();
        pass(&db, &local, cap, start).await;
        db.collection::<SecurityReplica>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":"a2"},
                doc! {"$set":{"expected_replicas":declaration}},
            )
            .await
            .unwrap();
        assess_every_ten_seconds(&db, &local, &[cap], start, start + ACTIVATION_SETTLE_WINDOW)
            .await;
        let report = report_for(&db, &local, cap).await.unwrap();
        assert!(
            report.activated && report.serving_ready && report.activation_pending_seconds.is_none()
        );
        assert_eq!(
            report.blockers,
            vec![
                "inconsistent_serving_roster",
                "missing_or_duplicate_replica:a"
            ]
        );
    }
}

#[tokio::test]
async fn malformed_capability_evidence_does_not_restart_another_capabilitys_settle_window() {
    let (db, local) = fixture("isolated_settle_failure", false).await;
    let broken = CAPABILITIES[0];
    let healthy = CAPABILITIES[1];
    db.collection::<bson::Document>(ROLLOUTS)
        .insert_one(doc! {"_id":"malformed", "record_kind":RECORD_KIND, "capability":broken, "active_since":"invalid"})
        .await.unwrap();
    let start = Instant::now();
    for elapsed in [0, 45, 89, 90] {
        assert!(matches!(
            record_activations(
                &db,
                &local,
                &[broken, healthy],
                start + std::time::Duration::from_secs(elapsed)
            )
            .await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        assert!(local.pending_seconds(broken, start).is_none());
        assert_eq!(active(&db, &local, healthy).await, elapsed == 90);
    }
    let state = report_for(&db, &local, healthy).await.unwrap();
    assert!(!state.ready && state.serving_ready);
    assert!(report_for(&db, &local, broken).await.is_err());
    // Publishing remains successful even when this capability's evidence is still bad.
    observe_for(&db, &local, &[broken, healthy]).await.unwrap();
    assert!(active(&db, &local, healthy).await);
}
