// Real Mongo acceptance boundaries for #2040. setup requires transactions;
// missing/standalone Mongo fails the fixture instead of skipping assertions.
use crate::models::assistant_conversation::TurnOrigin;
use crate::models::trigger_run::RunWaitKind;
use crate::services::{assistant_group_service as groups, trigger_schedule::continuation as v2};
use futures::TryStreamExt;

pub(super) async fn assistant_webhook(state: &AppState, agent: &str) -> Trigger {
    trigger_service::create(
        &state.db,
        &state.encryption_keys,
        trigger_service::CreateInput {
            source: TriggerSource::Webhook,
            setup_watch_id: None,
            schedule: None,
            overlap: OverlapPolicy::Skip,
            user_id: OWNER.into(),
            label: "2040 webhook fixture".into(),
            user_service_id: None,
            verification: TriggerVerification::Token {
                location: crate::models::trigger::TriggerTokenLocation::Bearer,
            },
            delivery: TriggerDelivery::Assistant {
                confirmation_policy: Default::default(),
                agent_id: agent.into(),
                thread_policy: None,
                instruction: "Local acceptance fixture".into(),
                deliver_to: DeliverTo::Thread,
            },
        },
    )
    .await
    .unwrap()
    .trigger
}

async fn waiting(state: &AppState) -> (Trigger, TriggerRun, AssistantConversation) {
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let trigger = automation(state, &agent.id, Some(ThreadPolicy::Dedicated)).await;
    let run = schedules::enqueue_now(&state.db, &trigger).await.unwrap();
    let row = schedules::run_thread(&state.db, &state.encryption_keys, &run, &trigger)
        .await
        .unwrap();
    let job = schedules::claim(&state.db, &run.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    let claim = schedules::TurnClaim {
        run_id: run.id.clone(),
        fence: job.get_str("fence").unwrap().into(),
        trigger_updated_at: trigger.updated_at,
        continuation: false,
    };
    let row = engine::begin_turn(
        &state.db,
        OWNER,
        completion_start(&row.id, TurnOrigin::Trigger, Some(claim)),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let run = completion_load(state, &run.id).await;
    completion_queue(state, &row, &run).await;
    completion_reply(state, &row, &run).await;
    (trigger, completion_load(state, &run.id).await, row)
}

async fn owner_budget(state: &AppState) -> Document {
    state
        .db
        .collection::<Document>(schedules::BUDGETS)
        .find_one(doc! {"_id": OWNER})
        .await
        .unwrap()
        .unwrap()
}

async fn budget_limits(state: &AppState, hourly: i32, daily: i32) {
    use crate::models::assistant_settings::{AssistantSettings, COLLECTION_NAME};
    let mut settings = AssistantSettings::defaults(OWNER);
    settings.trigger_runs_per_hour = hourly;
    settings.trigger_runs_per_day = daily;
    state
        .db
        .collection::<AssistantSettings>(COLLECTION_NAME)
        .replace_one(doc! {"_id": OWNER}, settings)
        .upsert(true)
        .await
        .unwrap();
}

async fn due_now(state: &AppState, id: &str) {
    state
        .db
        .collection::<Document>(schedules::WORK)
        .update_one(
            doc! {"_id": id},
            doc! {"$set": {"at": schedules::date(Utc::now())}},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn automation_v2_owner_windows_exhaust_and_reset_independently() {
    for window in ["hour", "day"] {
        let (state, calls, server) = setup(&format!("auto2040_windows_{window}")).await;
        let (_, run, row) = waiting(&state).await;
        budget_limits(&state, 4, 8).await;
        let now = Utc::now();
        let hour = now.timestamp().div_euclid(3600);
        let day = now.timestamp().div_euclid(86400);
        let (hourly, daily) = if window == "hour" { (4, 5) } else { (1, 8) };
        state
            .db
            .collection::<Document>(schedules::BUDGETS)
            .update_one(
                doc! {"_id": OWNER},
                doc! {"$set": {
                    "hour": hour, "day": day, "hourly": hourly, "daily": daily,
                }},
            )
            .await
            .unwrap();
        let claim = v2::claim_event(&state.db, OWNER, &row.id, &run.id)
            .await
            .unwrap()
            .unwrap();
        let rejected = engine::begin_turn(
            &state.db,
            OWNER,
            completion_start(&row.id, TurnOrigin::Event, Some(claim)),
            &state.encryption_keys,
        )
        .await;
        assert!(matches!(rejected, Err(AppError::TriggerRateLimited)));
        let unchanged = owner_budget(&state).await;
        assert_eq!(unchanged.get_i32("hourly").unwrap(), hourly);
        assert_eq!(unchanged.get_i32("daily").unwrap(), daily);
        let saved = completion_load(&state, &run.id).await;
        assert_eq!(saved.outcome, RunOutcome::Waiting);
        assert_eq!(saved.reason.as_deref(), Some("coordinator_pending"));
        assert_eq!(saved.continuation_count, 0);
        assert!(!saved.terminal_result_recorded);
        assert_eq!(
            engine::get(&state.db, OWNER, &row.id)
                .await
                .unwrap()
                .pending_events
                .len(),
            1
        );

        let job = state
            .db
            .collection::<Document>(schedules::WORK)
            .find_one(doc! {"_id": &run.id})
            .await
            .unwrap()
            .unwrap();
        defer(&state, &job, &saved, "budget_exhausted")
            .await
            .unwrap();
        let reset = chrono::DateTime::from_timestamp(
            if window == "hour" {
                (hour + 1) * 3600
            } else {
                (day + 1) * 86400
            },
            0,
        )
        .unwrap();
        let deferred = state
            .db
            .collection::<Document>(schedules::WORK)
            .find_one(doc! {"_id": &run.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            deferred.get_datetime("at").unwrap().to_chrono(),
            reset.min(saved.coordinator_deadline.unwrap())
        );
        assert!(
            schedules::claim(&state.db, &run.id, now)
                .await
                .unwrap()
                .is_none()
        );

        // Move just the exhausted persisted window into the previous UTC
        // period. The other period/counter stays live and must not reset.
        state
            .db
            .collection::<Document>(schedules::BUDGETS)
            .update_one(
                doc! {"_id": OWNER},
                doc! {"$set": {
                    window: if window == "hour" { hour - 1 } else { day - 1 },
                }},
            )
            .await
            .unwrap();
        due_now(&state, &run.id).await;
        let claim = v2::claim_event(&state.db, OWNER, &row.id, &run.id)
            .await
            .unwrap()
            .unwrap();
        let admitted = engine::begin_turn(
            &state.db,
            OWNER,
            completion_start(&row.id, TurnOrigin::Event, Some(claim)),
            &state.encryption_keys,
        )
        .await
        .unwrap();
        let budget = owner_budget(&state).await;
        assert_eq!(
            budget.get_i64("hour").unwrap(),
            Utc::now().timestamp().div_euclid(3600)
        );
        assert_eq!(
            budget.get_i64("day").unwrap(),
            Utc::now().timestamp().div_euclid(86400)
        );
        assert_eq!(
            budget.get_i32("hourly").unwrap(),
            if window == "hour" { 1 } else { hourly + 1 }
        );
        assert_eq!(
            budget.get_i32("daily").unwrap(),
            if window == "day" { 1 } else { daily + 1 }
        );
        assert_eq!(completion_load(&state, &run.id).await.continuation_count, 1);
        completion_reply(&state, &admitted, &run).await;
        let final_run = completion_load(&state, &run.id).await;
        assert_eq!(final_run.outcome, RunOutcome::Completed);
        assert!(final_run.terminal_result_recorded);
        assert!(calls.lock().await.is_empty());
        server.abort();
    }
}

#[tokio::test]
async fn automation_v2_concurrent_work_claims_and_admission_charge_once() {
    let (state, calls, server) = setup("auto2040_work_race").await;
    let (_, run, row) = waiting(&state).await;
    let before = owner_budget(&state).await;
    let (a, b) = tokio::join!(
        v2::claim_event(&state.db, OWNER, &row.id, &run.id),
        v2::claim_event(&state.db, OWNER, &row.id, &run.id)
    );
    let claims: Vec<_> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(claims.len(), 1);
    let stale = claims[0].clone();
    // Reclaim an expired lease using the production claim path, without sleeps.
    due_now(&state, &run.id).await;
    let claim = v2::claim_event(&state.db, OWNER, &row.id, &run.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(stale.fence, claim.fence);
    assert!(matches!(engine::begin_turn(&state.db, OWNER,
        completion_start(&row.id, TurnOrigin::Event, Some(stale)), &state.encryption_keys).await,
        Err(AppError::Conflict(reason)) if reason == "trigger_lease_lost"));
    let mut wrong_generation = claim.clone();
    wrong_generation.trigger_updated_at -= Duration::milliseconds(1);
    assert!(matches!(engine::begin_turn(&state.db, OWNER,
        completion_start(&row.id, TurnOrigin::Event, Some(wrong_generation)), &state.encryption_keys).await,
        Err(AppError::Conflict(reason)) if reason == "trigger_continuation_identity_mismatch"));
    let (a, b) = tokio::join!(
        engine::begin_turn(
            &state.db,
            OWNER,
            completion_start(&row.id, TurnOrigin::Event, Some(claim.clone())),
            &state.encryption_keys
        ),
        engine::begin_turn(
            &state.db,
            OWNER,
            completion_start(&row.id, TurnOrigin::Event, Some(claim)),
            &state.encryption_keys
        )
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let admitted = match (a, b) {
        (Ok(row), Err(_)) | (Err(_), Ok(row)) => row,
        _ => unreachable!(),
    };
    let budget = owner_budget(&state).await;
    assert_eq!(
        budget.get_i32("hourly").unwrap(),
        before.get_i32("hourly").unwrap() + 1
    );
    assert_eq!(
        budget.get_i32("daily").unwrap(),
        before.get_i32("daily").unwrap() + 1
    );
    assert_eq!(completion_load(&state, &run.id).await.continuation_count, 1);
    assert_eq!(admitted.active_turn.as_ref().unwrap().events.len(), 1);
    assert!(admitted.pending_events.is_empty());
    completion_reply(&state, &admitted, &run).await;
    assert_eq!(
        completion_load(&state, &run.id).await.outcome,
        RunOutcome::Completed
    );
    assert!(calls.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn automation_v2_concurrent_runs_share_the_last_owner_budget_slot() {
    let (state, calls, server) = setup("auto2040_owner_slot_race").await;
    let (_, first, a) = waiting(&state).await;
    let (_, second, b) = waiting(&state).await;
    assert_ne!(first.id, second.id);
    assert_ne!(a.id, b.id);
    budget_limits(&state, 4, 8).await;
    state
        .db
        .collection::<Document>(schedules::BUDGETS)
        .update_one(
            doc! {"_id": OWNER},
            doc! {"$set": {"hourly": 3_i32, "daily": 5_i32}},
        )
        .await
        .unwrap();
    let a_claim = v2::claim_event(&state.db, OWNER, &a.id, &first.id)
        .await
        .unwrap()
        .unwrap();
    let b_claim = v2::claim_event(&state.db, OWNER, &b.id, &second.id)
        .await
        .unwrap()
        .unwrap();
    let (a_result, b_result) = tokio::join!(
        engine::begin_turn(
            &state.db,
            OWNER,
            completion_start(&a.id, TurnOrigin::Event, Some(a_claim)),
            &state.encryption_keys
        ),
        engine::begin_turn(
            &state.db,
            OWNER,
            completion_start(&b.id, TurnOrigin::Event, Some(b_claim)),
            &state.encryption_keys
        )
    );
    let (winner, loser) = match (a_result, b_result) {
        (Ok(_), Err(AppError::TriggerRateLimited)) => (&first, &second),
        (Err(AppError::TriggerRateLimited), Ok(_)) => (&second, &first),
        _ => panic!("exactly one owner-budget admission must commit"),
    };
    assert_eq!(
        completion_load(&state, &winner.id).await.continuation_count,
        1
    );
    let waiting = completion_load(&state, &loser.id).await;
    assert_eq!(waiting.continuation_count, 0);
    assert_eq!(waiting.outcome, RunOutcome::Waiting);
    assert_eq!(waiting.reason.as_deref(), Some("coordinator_pending"));
    assert!(!waiting.terminal_result_recorded);
    let budget = owner_budget(&state).await;
    assert_eq!(budget.get_i32("hourly").unwrap(), 4);
    assert_eq!(budget.get_i32("daily").unwrap(), 6);
    let job = state
        .db
        .collection::<Document>(schedules::WORK)
        .find_one(doc! {"_id": &loser.id})
        .await
        .unwrap()
        .unwrap();
    defer(&state, &job, &waiting, "budget_exhausted")
        .await
        .unwrap();
    assert!(
        engine::get(&state.db, OWNER, waiting.thread_id.as_ref().unwrap())
            .await
            .unwrap()
            .pending_events
            .iter()
            .any(|event| event.trigger_run_id.as_deref() == Some(loser.id.as_str()))
    );
    assert!(calls.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn automation_v2_concurrent_follower_claims_commit_one_exact_event() {
    let (state, calls, server) = setup("auto2040_follower_race").await;
    let (_, run, row) = completion_coordinator(&state).await;
    let group = completion_group(&state, &row, &run).await;
    let (a, b) = tokio::join!(
        groups::claim_followers_if_quiet(&state.db, OWNER, &group.id),
        groups::claim_followers_if_quiet(&state.db, OWNER, &group.id)
    );
    let claims: Vec<_> = a.unwrap().0.into_iter().chain(b.unwrap().0).collect();
    assert_eq!(claims.len(), 1);
    let old = claims[0].clone();
    state.db.collection::<Document>(crate::models::assistant_group::COLLECTION_NAME).update_one(
        doc! {"_id": &group.id}, doc! {"$set": {
            "followers.0.notification.lease_until": schedules::date(Utc::now() - Duration::seconds(1)),
        }}).await.unwrap();
    let (claims, _) = groups::claim_followers_if_quiet(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    let current = &claims[0];
    let old_notice = old.notification.as_ref().unwrap();
    let notice = current.notification.as_ref().unwrap();
    assert_ne!(old_notice.fence, notice.fence);
    assert_eq!(old_notice.event_id, notice.event_id);
    let mut event = team::bound_event("group_settled", "Concurrent handoff".into(), None, None)
        .for_trigger(Some(&run.id));
    event.id = notice.event_id.clone();
    assert!(
        !groups::deliver_follower(&state.db, OWNER, &group.id, &old, &event)
            .await
            .unwrap()
    );
    let (a, b) = tokio::join!(
        groups::deliver_follower(&state.db, OWNER, &group.id, current, &event),
        groups::deliver_follower(&state.db, OWNER, &group.id, current, &event)
    );
    assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
    assert!(
        groups::get(&state.db, OWNER, &group.id)
            .await
            .unwrap()
            .followers
            .is_empty()
    );
    let queued = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(queued.pending_events.len(), 1);
    assert_eq!(queued.pending_events[0].id, event.id);
    assert_eq!(
        queued.pending_events[0].trigger_run_id.as_deref(),
        Some(run.id.as_str())
    );
    completion_reply(&state, &row, &run).await;
    let claim = v2::claim_event(&state.db, OWNER, &row.id, &run.id)
        .await
        .unwrap()
        .unwrap();
    let admitted = engine::begin_turn(
        &state.db,
        OWNER,
        completion_start(&row.id, TurnOrigin::Event, Some(claim)),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(
        admitted.active_turn.as_ref().unwrap().events[0].id,
        event.id
    );
    assert_eq!(completion_load(&state, &run.id).await.continuation_count, 1);
    assert!(calls.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn automation_v2_two_real_runs_cannot_consume_each_others_thread_events() {
    let (state, calls, server) = setup("auto2040_two_runs").await;
    let (_, first, a) = waiting(&state).await;
    let (_, second, b) = waiting(&state).await;
    assert_ne!(first.id, second.id);
    assert_ne!(a.id, b.id);
    assert!(
        v2::claim_event(&state.db, OWNER, &b.id, &first.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        v2::claim_event(&state.db, OWNER, &a.id, &second.id)
            .await
            .unwrap()
            .is_none()
    );
    let claim = v2::claim_event(&state.db, OWNER, &a.id, &first.id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(engine::begin_turn(&state.db, OWNER,
        completion_start(&b.id, TurnOrigin::Event, Some(claim.clone())), &state.encryption_keys).await,
        Err(AppError::Conflict(reason)) if reason == "trigger_continuation_identity_mismatch"));
    let foreign = team::bound_event("group_settled", "Second run".into(), None, None)
        .for_trigger(Some(&second.id));
    engine::push_events(&state.db, OWNER, &a.id, vec![foreign.clone()])
        .await
        .unwrap();
    let admitted = engine::begin_turn(
        &state.db,
        OWNER,
        completion_start(&a.id, TurnOrigin::Event, Some(claim)),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(
        admitted
            .active_turn
            .as_ref()
            .unwrap()
            .events
            .iter()
            .all(|event| event.trigger_run_id.as_deref() == Some(first.id.as_str()))
    );
    assert_eq!(admitted.pending_events.len(), 1);
    assert_eq!(admitted.pending_events[0].id, foreign.id);
    let second_thread = engine::get(&state.db, OWNER, &b.id).await.unwrap();
    assert_eq!(second_thread.pending_events.len(), 1);
    assert_eq!(
        second_thread.pending_events[0].trigger_run_id.as_deref(),
        Some(second.id.as_str())
    );
    assert_eq!(
        completion_load(&state, &second.id).await.continuation_count,
        0
    );
    assert!(calls.lock().await.is_empty());
    server.abort();
}

async fn migration_marker(state: &AppState) -> Document {
    state
        .db
        .collection::<Document>(v2::MIGRATIONS)
        .find_one(doc! {"_id": v2::MIGRATION_ID})
        .await
        .unwrap()
        .unwrap()
}

async fn abandon_marker(state: &AppState) {
    state
        .db
        .collection::<Document>(v2::MIGRATIONS)
        .update_one(
            doc! {"_id": v2::MIGRATION_ID},
            doc! {"$unset": {"completed_at": ""}, "$set": {
                "lease_until": schedules::date(Utc::now() - Duration::seconds(1)),
                "classified": 0_i64, "triggers": 0_i64, "ambiguous": [], "ambiguous_count": 0_i64,
            }},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn automation_v2_migration_schedule_and_webhook_classify_real_waits_and_drain() {
    for webhook in [false, true] {
        for kind in [
            "coordinator",
            "coordinator_group",
            "confirmation",
            "missing",
            "malformed",
            "dangling",
            "confirmation_missing",
        ] {
            let (state, calls, server) = setup(&format!("auto2040_migrate_{webhook}_{kind}")).await;
            let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
            let trigger = if webhook {
                assistant_webhook(&state, &agent.id).await
            } else {
                automation(&state, &agent.id, None).await
            };
            let mut run =
                schedules::new_run(&trigger, Utc::now(), Utc::now() + Duration::minutes(5));
            state
                .db
                .collection::<TriggerRun>(RUNS)
                .insert_one(&run)
                .await
                .unwrap();
            let row = schedules::run_thread(&state.db, &state.encryption_keys, &run, &trigger)
                .await
                .unwrap();
            run.outcome = RunOutcome::Waiting;
            run.started_at = Some(Utc::now() - Duration::seconds(10));
            run.thread_id = Some(row.id.clone());
            run.turn_id = Some(Uuid::new_v4().to_string());
            if kind.starts_with("confirmation") {
                run.waiting_for = vec![Uuid::new_v4().to_string()];
            }
            if kind == "missing" {
                run.thread_id = None;
            }
            if kind == "malformed" {
                run.thread_id = Some("malformed-thread-2040".into());
            }
            state
                .db
                .collection::<TriggerRun>(RUNS)
                .replace_one(doc! {"_id": &run.id}, &run)
                .await
                .unwrap();
            state
                .db
                .collection::<Document>(schedules::WORK)
                .insert_one(doc! {
                    "_id": &run.id, "kind": "run", "trigger_id": &trigger.id, "user_id": OWNER,
                    "at": schedules::date(Utc::now()), "fence": "",
                })
                .await
                .unwrap();
            if kind == "coordinator" {
                completion_queue(&state, &row, &run).await;
            }
            if matches!(kind, "coordinator_group" | "dangling") {
                let group = groups::create(
                    &state.db,
                    OWNER,
                    "Legacy source fixture",
                    std::slice::from_ref(&agent.id),
                    "nyxbot",
                )
                .await
                .unwrap();
                let follower = crate::models::assistant_group::GroupFollower {
                    trigger_run_id: Some(run.id.clone()),
                    notification: None,
                    conversation_id: row.id.clone(),
                    since_seq: 0,
                    created_at: Utc::now(),
                    expires_at: Some(Utc::now() + Duration::hours(1)),
                };
                state
                    .db
                    .collection::<Document>(crate::models::assistant_group::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id": &group.id},
                        doc! {"$set": {
                            "followers": [bson::to_document(&follower).unwrap()],
                        }},
                    )
                    .await
                    .unwrap();
            }
            let confirmation = if kind.starts_with("confirmation") {
                let card = doc! {
                    "_id": &run.waiting_for[0], "conversation_id": &row.id, "user_id": OWNER,
                    "api_key_id": &row.credential_api_key_id, "kind": "action", "summary": "Local migration card",
                    "status": "pending", "requested_turn_id": &run.turn_id, "trigger_run_id": &run.id,
                    "created_at": schedules::date(Utc::now()),
                    "expires_at": schedules::date(Utc::now() + Duration::minutes(30)), "decider": "user",
                };
                bson::from_document::<
                    crate::models::assistant_acknowledgement::AssistantAcknowledgement,
                >(card.clone())
                .expect("fixture must be a valid persisted action card");
                state
                    .db
                    .collection::<Document>(
                        crate::models::assistant_acknowledgement::COLLECTION_NAME,
                    )
                    .insert_one(card.clone())
                    .await
                    .unwrap();
                Some(card)
            } else {
                None
            };
            if matches!(kind, "dangling" | "confirmation_missing") {
                state
                    .db
                    .collection::<Document>(CONVERSATIONS)
                    .delete_one(doc! {"_id": &row.id})
                    .await
                    .unwrap();
                // A stale capacity flag must not make a deleted thread resumable.
                state
                    .db
                    .collection::<Document>(RUNS)
                    .update_one(
                        doc! {"_id": &run.id},
                        doc! {"$set": {"coordinator_capacity_retry": true}},
                    )
                    .await
                    .unwrap();
            }
            state.db.collection::<Document>(TRIGGERS).update_one(doc! {"_id": &trigger.id},
                doc! {"$unset": {"continuation_protocol_version": ""}, "$set": {"history_count": 1_i64}})
                .await.unwrap();
            if webhook {
                // Pre-source-field webhook rows must retain the serde default.
                state
                    .db
                    .collection::<Document>(TRIGGERS)
                    .update_one(doc! {"_id": &trigger.id}, doc! {"$unset": {"source": ""}})
                    .await
                    .unwrap();
            }
            abandon_marker(&state).await;
            assert!(v2::ready(&state.db, true).await.is_err());
            let drained = Utc::now() - Duration::seconds(2 * schedules::LEASE_SECONDS + 1);
            assert!(v2::migrate_checked(&state.db, drained, true).await.is_err());
            let fence = Uuid::new_v4().to_string();
            state.db.collection::<Document>(v2::MIGRATIONS).update_one(doc! {"_id": v2::MIGRATION_ID},
                doc! {"$set": {"fence": &fence, "lease_until": schedules::date(Utc::now() + Duration::minutes(5))}})
                .await.unwrap();
            let mut session = state.db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            v2::migrate_run_in_session(&state.db, &run.id, &fence, &mut session)
                .await
                .unwrap();
            let provisional = state
                .db
                .collection::<TriggerRun>(RUNS)
                .find_one(doc! {"_id": &run.id})
                .session(&mut session)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(provisional.continuation_protocol_version, 2);
            session.abort_transaction().await.unwrap();
            let rolled_back = completion_load(&state, &run.id).await;
            assert_eq!(rolled_back.continuation_protocol_version, 0);
            assert_eq!(rolled_back.outcome, RunOutcome::Waiting);
            assert_eq!(rolled_back.waiting_for, run.waiting_for);
            assert!(!rolled_back.terminal_result_recorded);
            assert_eq!(
                migration_marker(&state)
                    .await
                    .get_i64("classified")
                    .unwrap(),
                0
            );
            assert!(
                state
                    .db
                    .collection::<Document>(schedules::WORK)
                    .find_one(doc! {"_id": &run.id})
                    .await
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                state
                    .db
                    .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                    .count_documents(
                        doc! {"event_type": "trigger_run_outcome", "event_data.run_id": &run.id}
                    )
                    .await
                    .unwrap(),
                0
            );
            abandon_marker(&state).await;
            let marker = v2::migrate_checked(&state.db, drained, false)
                .await
                .unwrap();
            assert!(marker.contains_key("completed_at"));
            assert_eq!(marker.get_i64("classified").unwrap(), 1);
            assert_eq!(marker.get_i64("triggers").unwrap(), 1);
            // A fresh client/session must observe durable readiness, not a
            // rollback-prone in-memory classification flag.
            let durable = crate::services::transaction_service::durable_database(&state.db);
            v2::ready(&durable, true).await.unwrap();
            assert!(v2::ready(&durable, false).await.is_err());
            let migrated = completion_load(&state, &run.id).await;
            assert_eq!(migrated.continuation_protocol_version, 2);
            let target = trigger_service::get_for_actor(&state.db, OWNER, &trigger.id)
                .await
                .unwrap();
            assert_eq!(
                target.source,
                if webhook {
                    TriggerSource::Webhook
                } else {
                    TriggerSource::Schedule
                }
            );
            assert_eq!(target.continuation_protocol_version, 2);
            let terminal = !matches!(kind, "coordinator" | "coordinator_group" | "confirmation");
            assert_eq!(migrated.terminal_result_recorded, terminal);
            if !terminal {
                assert_eq!(migrated.outcome, RunOutcome::Waiting);
                assert_eq!(
                    migrated.wait_kind,
                    Some(if kind.starts_with("coordinator") {
                        RunWaitKind::Coordinator
                    } else {
                        RunWaitKind::Confirmation
                    })
                );
                assert_eq!(migrated.waiting_for, run.waiting_for);
                if kind.starts_with("coordinator") {
                    assert_eq!(
                        migrated.coordinator_wait.as_ref().unwrap().thread_id,
                        row.id
                    );
                    assert_eq!(
                        migrated.coordinator_wait.as_ref().unwrap().source,
                        if kind == "coordinator_group" {
                            "group"
                        } else {
                            "event"
                        }
                    );
                    assert!(migrated.waiting_for.is_empty());
                }
                assert!(!v2::rollback_ready(&state.db).await.unwrap());
            } else {
                let reason = if kind == "confirmation_missing" {
                    "thread_deleted"
                } else {
                    "legacy_wait_unclassified"
                };
                assert_eq!(migrated.outcome, RunOutcome::Failed);
                assert_eq!(migrated.reason.as_deref(), Some(reason));
                assert!(migrated.finished_at.is_some());
                assert!(
                    state
                        .db
                        .collection::<Document>(schedules::WORK)
                        .find_one(doc! {"_id": &run.id})
                        .await
                        .unwrap()
                        .is_none()
                );
                assert_eq!(target.schedule_state.last_run.as_ref().unwrap().id, run.id);
            }
            let audit_filter =
                doc! {"event_type": "trigger_run_outcome", "event_data.run_id": &run.id};
            assert_eq!(
                state
                    .db
                    .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                    .count_documents(audit_filter.clone())
                    .await
                    .unwrap(),
                u64::from(terminal)
            );
            let replay = v2::migrate_checked(&state.db, drained, false)
                .await
                .unwrap();
            assert_eq!(replay.get_i64("classified").unwrap(), 1);
            assert_eq!(
                completion_load(&state, &run.id).await.finished_at,
                migrated.finished_at
            );
            if let Some(card) = confirmation {
                let persisted = state
                    .db
                    .collection::<Document>(
                        crate::models::assistant_acknowledgement::COLLECTION_NAME,
                    )
                    .find_one(doc! {"_id": &run.waiting_for[0]})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    persisted, card,
                    "migration and aborted classification preserve the exact card"
                );
            }
            if !terminal {
                schedules::finish(
                    &state.db,
                    &run.id,
                    RunOutcome::Failed,
                    Some("cancelled"),
                    &state.config.frontend_url,
                )
                .await
                .unwrap();
            }
            assert!(v2::rollback_ready(&state.db).await.unwrap());
            assert_eq!(
                state
                    .db
                    .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                    .count_documents(audit_filter)
                    .await
                    .unwrap(),
                1
            );
            assert!(calls.lock().await.is_empty());
            server.abort();
        }
    }
}

#[tokio::test]
async fn automation_v2_migration_missing_or_changed_target_cannot_seal_readiness() {
    for kind in [
        "missing",
        "wrong_owner",
        "changed_delivery",
        "unsupported_protocol",
    ] {
        let (state, calls, server) = setup(&format!("auto2040_migration_target_{kind}")).await;
        let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
        let trigger = assistant_webhook(&state, &agent.id).await;
        let mut run = schedules::new_run(&trigger, Utc::now(), Utc::now() + Duration::minutes(5));
        run.outcome = RunOutcome::Waiting;
        state
            .db
            .collection::<TriggerRun>(RUNS)
            .insert_one(&run)
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(schedules::WORK)
            .insert_one(doc! {
                "_id": &run.id, "kind": "run", "trigger_id": &trigger.id, "user_id": OWNER,
                "at": schedules::date(Utc::now()), "fence": "",
            })
            .await
            .unwrap();
        match kind {
            "missing" => {
                state
                    .db
                    .collection::<Document>(TRIGGERS)
                    .delete_one(doc! {"_id": &trigger.id})
                    .await
                    .unwrap();
            }
            "wrong_owner" => {
                state
                    .db
                    .collection::<Document>(TRIGGERS)
                    .update_one(
                        doc! {"_id": &trigger.id},
                        doc! {"$set": {"user_id": Uuid::new_v4().to_string()}},
                    )
                    .await
                    .unwrap();
            }
            "unsupported_protocol" => {
                state
                    .db
                    .collection::<Document>(RUNS)
                    .update_one(
                        doc! {"_id": &run.id},
                        doc! {"$set": {"continuation_protocol_version": 3}},
                    )
                    .await
                    .unwrap();
            }
            _ => {
                state
                    .db
                    .collection::<Document>(TRIGGERS)
                    .update_one(
                        doc! {"_id": &trigger.id},
                        doc! {"$set": {"delivery": {"type": "notification"}}},
                    )
                    .await
                    .unwrap();
            }
        }
        abandon_marker(&state).await;
        let drained = Utc::now() - Duration::seconds(2 * schedules::LEASE_SECONDS + 1);
        assert!(
            matches!(v2::migrate_checked(&state.db, drained, false).await,
            Err(AppError::Conflict(reason)) if reason == if kind == "unsupported_protocol" {
                "automation_migration_protocol_unsupported"
            } else { "automation_migration_trigger_unavailable" })
        );
        let progress = migration_marker(&state).await;
        assert!(!progress.contains_key("completed_at"));
        assert_eq!(progress.get_i64("classified").unwrap(), 0);
        assert_eq!(progress.get_i64("ambiguous_count").unwrap(), 0);
        assert!(v2::ready(&state.db, true).await.is_err());
        let legacy = completion_load(&state, &run.id).await;
        assert_eq!(
            legacy.continuation_protocol_version,
            if kind == "unsupported_protocol" { 3 } else { 0 }
        );
        assert_eq!(legacy.outcome, RunOutcome::Waiting);
        assert!(!legacy.terminal_result_recorded);
        assert!(
            state
                .db
                .collection::<Document>(schedules::WORK)
                .find_one(doc! {"_id": &run.id})
                .await
                .unwrap()
                .is_some()
        );
        // Reconcile only the fixture target, preserving its exact owner and
        // authored generation, and expire the failed operator's lease.
        state
            .db
            .collection::<Trigger>(TRIGGERS)
            .replace_one(doc! {"_id": &trigger.id}, &trigger)
            .upsert(true)
            .await
            .unwrap();
        if kind == "unsupported_protocol" {
            state
                .db
                .collection::<Document>(RUNS)
                .update_one(
                    doc! {"_id": &run.id},
                    doc! {"$set": {"continuation_protocol_version": 0}},
                )
                .await
                .unwrap();
        }
        abandon_marker(&state).await;
        let marker = v2::migrate_checked(&state.db, drained, false)
            .await
            .unwrap();
        assert_eq!(marker.get_i64("classified").unwrap(), 1);
        assert!(marker.contains_key("completed_at"));
        assert_eq!(
            completion_load(&state, &run.id).await.outcome,
            RunOutcome::Failed
        );
        v2::ready(&state.db, true).await.unwrap();
        assert!(v2::rollback_ready(&state.db).await.unwrap());
        assert!(calls.lock().await.is_empty());
        server.abort();
    }
}

#[tokio::test]
async fn automation_v2_migration_preserves_reachable_non_assistant_pending_work() {
    let (state, calls, server) = setup("auto2040_non_assistant_migration").await;
    let trigger = trigger_service::create(
        &state.db,
        &state.encryption_keys,
        trigger_service::CreateInput {
            source: TriggerSource::Webhook,
            setup_watch_id: None,
            schedule: None,
            overlap: OverlapPolicy::Skip,
            user_id: OWNER.into(),
            label: "2040 notification fixture".into(),
            user_service_id: None,
            verification: TriggerVerification::Token {
                location: crate::models::trigger::TriggerTokenLocation::Bearer,
            },
            delivery: TriggerDelivery::Notification,
        },
    )
    .await
    .unwrap()
    .trigger;
    let run = schedules::enqueue_now(&state.db, &trigger).await.unwrap();
    abandon_marker(&state).await;
    let marker = v2::migrate_checked(
        &state.db,
        Utc::now() - Duration::seconds(2 * schedules::LEASE_SECONDS + 1),
        false,
    )
    .await
    .unwrap();
    assert!(marker.contains_key("completed_at"));
    assert_eq!(marker.get_i64("classified").unwrap(), 0);
    let preserved = completion_load(&state, &run.id).await;
    assert_eq!(preserved.outcome, RunOutcome::Pending);
    assert_eq!(preserved.continuation_protocol_version, 0);
    assert!(
        schedules::claim(&state.db, &run.id, Utc::now())
            .await
            .unwrap()
            .is_some(),
        "ordinary pending delivery keeps its scheduler work"
    );
    assert!(calls.lock().await.is_empty());
    server.abort();
}

async fn ordinary_budget(state: &AppState) -> i64 {
    state
        .db
        .collection::<Document>(crate::models::coordination::RATE_WINDOW_COLLECTION_NAME)
        .find_one(doc! {"namespace": "assistant_event_turns"})
        .await
        .unwrap()
        .map_or(0, |row| row.get_i64("count").unwrap())
}

#[tokio::test]
async fn automation_v2_wake_preserves_ordinary_hourly_and_streak_guards() {
    use crate::services::coordination_service::{RateWindowStore, TEST_RATE_WINDOW_NOW};
    for guard in ["hourly", "streak"] {
        let reads = MongoReads::default();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            &format!("auto2040_event_guard_{guard}"),
            reads.handler(),
        )
        .await;
        let (state, calls, server) = setup_with_database(db).await;
        let (trigger, run, row) = waiting(&state).await;
        let ordinary = team::event(
            "message",
            "Ordinary event remains owned by its guard".into(),
            None,
        );
        // Hold only the ordinary event while testing ordinary wake eligibility.
        let bound = engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events;
        state
            .db
            .collection::<Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"pending_events": bson::to_bson(&vec![ordinary.clone()]).unwrap(),
                "event_streak": if guard == "streak" { team::MAX_EVENT_STREAK } else { 0 }}},
            )
            .await
            .unwrap();
        if guard == "hourly" {
            for _ in 0..team::EVENT_TURNS_PER_HOUR {
                assert!(
                    RateWindowStore::admit(
                        &state.db,
                        "assistant_event_turns",
                        OWNER,
                        team::EVENT_TURNS_PER_HOUR,
                        std::time::Duration::from_secs(3600)
                    )
                    .await
                    .unwrap()
                    .allowed
                );
            }
        }
        let ordinary_before = ordinary_budget(&state).await;
        let trigger_before = owner_budget(&state).await;
        Box::pin(crate::handlers::assistant_team::wake(
            &state, OWNER, &row.id,
        ))
        .await;
        let blocked = engine::get(&state.db, OWNER, &row.id).await.unwrap();
        assert!(blocked.active_turn.is_none());
        assert_eq!(blocked.pending_events.len(), 1);
        assert_eq!(blocked.pending_events[0].id, ordinary.id);
        assert_eq!(ordinary_budget(&state).await, ordinary_before);
        assert!(calls.lock().await.is_empty());
        engine::push_events(&state.db, OWNER, &row.id, bound)
            .await
            .unwrap();
        reads.clear();
        Box::pin(crate::handlers::assistant_team::wake(
            &state, OWNER, &row.id,
        ))
        .await;
        let finished = completed(&state, &trigger).await;
        assert_eq!(finished.outcome, RunOutcome::Completed);
        assert_eq!(finished.continuation_count, 1);
        // Terminal projection precedes after_turn's own wake. Observe its
        // subsequent queued() query before resetting the ordinary guard, so
        // this fixture cannot race that wake with its explicit admission.
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                if reads.finds().iter().any(|command| {
                    command.get_str("find") == Ok(CONVERSATIONS)
                        && command
                            .get_document("filter")
                            .is_ok_and(|filter| filter.contains_key("pending_events.created_at"))
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let after = engine::get(&state.db, OWNER, &row.id).await.unwrap();
        assert_eq!(after.event_streak, blocked.event_streak);
        assert_eq!(after.pending_events.len(), 1);
        assert_eq!(after.pending_events[0].id, ordinary.id);
        assert_eq!(ordinary_budget(&state).await, ordinary_before);
        let trigger_after = owner_budget(&state).await;
        assert_eq!(
            trigger_after.get_i32("hourly").unwrap(),
            trigger_before.get_i32("hourly").unwrap() + 1
        );
        assert_eq!(
            trigger_after.get_i32("daily").unwrap(),
            trigger_before.get_i32("daily").unwrap() + 1
        );
        Box::pin(crate::handlers::assistant_team::wake(
            &state, OWNER, &row.id,
        ))
        .await;
        assert_eq!(
            engine::get(&state.db, OWNER, &row.id)
                .await
                .unwrap()
                .pending_events[0]
                .id,
            ordinary.id
        );
        assert_eq!(ordinary_budget(&state).await, ordinary_before);
        // Cross the exact hourly boundary through the pipeline's task-local
        // clock, or reset only the streak. Never rewrite window_start to an
        // unaligned client timestamp or change the trigger budget.
        let reset_at = if guard == "hourly" {
            let budget = state
                .db
                .collection::<Document>(crate::models::coordination::RATE_WINDOW_COLLECTION_NAME)
                .find_one(doc! {"namespace": "assistant_event_turns"})
                .await
                .unwrap()
                .unwrap();
            Some(bson::DateTime::from_millis(
                budget
                    .get_datetime("window_start")
                    .unwrap()
                    .timestamp_millis()
                    + 3_600_000,
            ))
        } else {
            state
                .db
                .collection::<Document>(CONVERSATIONS)
                .update_one(doc! {"_id": &row.id}, doc! {"$set": {"event_streak": 0}})
                .await
                .unwrap();
            None
        };
        let count = calls.lock().await.len();
        let wake = Box::pin(crate::handlers::assistant_team::wake(
            &state, OWNER, &row.id,
        ));
        if let Some(reset_at) = reset_at {
            TEST_RATE_WINDOW_NOW
                .scope(std::cell::Cell::new(reset_at), wake)
                .await;
        } else {
            wake.await;
        }
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let current = engine::get(&state.db, OWNER, &row.id).await.unwrap();
                if current.active_turn.is_none()
                    && current.pending_events.is_empty()
                    && calls.lock().await.len() > count
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(ordinary_budget(&state).await, 1, "{guard}");
        assert_eq!(calls.lock().await.len(), count + 1, "one ordinary turn");
        if let Some(reset_at) = reset_at {
            let budget = state
                .db
                .collection::<Document>(crate::models::coordination::RATE_WINDOW_COLLECTION_NAME)
                .find_one(doc! {"namespace": "assistant_event_turns"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(budget.get_datetime("window_start").unwrap(), &reset_at);
        }
        assert_eq!(
            engine::get(&state.db, OWNER, &row.id)
                .await
                .unwrap()
                .event_streak,
            1
        );
        let final_budget = owner_budget(&state).await;
        assert_eq!(
            final_budget.get_i32("hourly").unwrap(),
            trigger_after.get_i32("hourly").unwrap()
        );
        assert_eq!(
            final_budget.get_i32("daily").unwrap(),
            trigger_after.get_i32("daily").unwrap()
        );
        assert_eq!(completion_load(&state, &run.id).await.continuation_count, 1);
        server.abort();
    }
}

#[derive(Clone, Default)]
struct MongoReads {
    commands: Arc<std::sync::Mutex<Vec<Document>>>,
    batches: Arc<std::sync::Mutex<Vec<usize>>>,
}

impl MongoReads {
    fn handler(&self) -> mongodb::event::EventHandler<mongodb::event::command::CommandEvent> {
        use mongodb::event::command::CommandEvent;
        let monitor = self.clone();
        mongodb::event::EventHandler::callback(move |event| match event {
            CommandEvent::Started(event) => monitor.commands.lock().unwrap().push(event.command),
            CommandEvent::Succeeded(event) => {
                if let Ok(cursor) = event.reply.get_document("cursor") {
                    for name in ["firstBatch", "nextBatch"] {
                        if let Ok(batch) = cursor.get_array(name) {
                            monitor.batches.lock().unwrap().push(batch.len());
                        }
                    }
                }
            }
            _ => {}
        })
    }

    fn clear(&self) {
        self.commands.lock().unwrap().clear();
        self.batches.lock().unwrap().clear();
    }

    fn finds(&self) -> Vec<Document> {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| command.contains_key("find"))
            .cloned()
            .collect()
    }
}

fn mongo_number(document: &Document, key: &str) -> i64 {
    document
        .get_i64(key)
        .unwrap_or_else(|_| i64::from(document.get_i32(key).unwrap()))
}

async fn explain_find(
    db: &mongodb::Database,
    command: &Document,
    max_docs: i64,
    max_keys: i64,
) -> Document {
    // Replay the real boundary's filter/projection/sort/limit, dropping session
    // and driver transport fields that explain's nested find does not accept.
    let mut find = Document::new();
    for key in ["find", "filter", "projection", "sort", "limit", "hint"] {
        if let Some(value) = command.get(key) {
            find.insert(key, value.clone());
        }
    }
    let report = db
        .run_command(doc! {"explain": find, "verbosity": "executionStats"})
        .await
        .expect("real Mongo explain must succeed");
    let stats = report.get_document("executionStats").unwrap();
    assert!(
        mongo_number(stats, "totalDocsExamined") <= max_docs,
        "{report:?}"
    );
    assert!(
        mongo_number(stats, "totalKeysExamined") <= max_keys,
        "{report:?}"
    );
    let plan = winning_plan(&report);
    assert!(!plan.contains("COLLSCAN"), "{plan}");
    assert!(
        !plan.contains("\"stage\":\"SORT\""),
        "unbounded blocking sort: {plan}"
    );
    eprintln!(
        "automation production find {:?}: {}",
        command.get_str("find").unwrap(),
        stats
    );
    report
}

fn winning_plan(report: &Document) -> String {
    serde_json::to_string(
        report
            .get_document("queryPlanner")
            .unwrap()
            .get_document("winningPlan")
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn automation_v2_mongo_source_work_history_and_readiness_query_bounds() {
    use crate::models::assistant_direct_handoff::{COLLECTION_NAME as HANDOFFS, DirectHandoff};

    let reads = MongoReads::default();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "auto2040_query_bounds",
        reads.handler(),
    )
    .await;
    let (state, calls, server) = setup_with_database(db).await;
    let (_, run, row) = completion_coordinator(&state).await;
    let group = completion_group(&state, &row, &run).await;
    // Unrelated groups/receipts/runs/work must not inflate the exact-source lookup.
    let mut group_rows = Vec::new();
    let mut handoff_rows = Vec::new();
    for n in 0..256 {
        let mut other = group.clone();
        other.id = format!("nyxg-{}", Uuid::new_v4().simple());
        other.followers[0].trigger_run_id = Some(Uuid::new_v4().to_string());
        other.followers[0].conversation_id = format!("nyxa-noise-{n}");
        group_rows.push(other);
        // Neither pending receipts for other runs nor terminal receipts for
        // this run are a live source. Both index keys must bound this lookup.
        let terminal = n % 2 == 0;
        handoff_rows.push(DirectHandoff {
            id: Uuid::new_v4().to_string(),
            user_id: OWNER.into(),
            run_id: if terminal {
                run.id.clone()
            } else {
                Uuid::new_v4().to_string()
            },
            parent_thread_id: row.id.clone(),
            parent_turn_id: run.turn_id.clone().unwrap(),
            child_thread_id: format!("nyxa-receipt-noise-{n}"),
            child_turn_id: Uuid::new_v4().to_string(),
            agent_id: row.agent_id.clone().unwrap(),
            invocation_id: None,
            state: if terminal { "delivered" } else { "pending" }.into(),
            retry_at: run.expires_at,
            expires_at: run.expires_at,
        });
    }
    state
        .db
        .collection::<crate::models::assistant_group::AssistantGroup>(
            crate::models::assistant_group::COLLECTION_NAME,
        )
        .insert_many(group_rows)
        .await
        .unwrap();
    state
        .db
        .collection::<DirectHandoff>(HANDOFFS)
        .insert_many(handoff_rows)
        .await
        .unwrap();
    let mut work = Vec::new();
    for n in 0..256 {
        work.push(doc! {"_id": format!("auto2040-noise-{n}"), "kind": "run",
            "trigger_id": Uuid::new_v4().to_string(), "at": schedules::date(Utc::now() + Duration::days(1)),
            "fence": ""});
    }
    state
        .db
        .collection::<Document>(schedules::WORK)
        .insert_many(work)
        .await
        .unwrap();
    for (collection, name, keys) in [
        (
            HANDOFFS,
            "direct_handoff_run",
            doc! {"run_id": 1, "state": 1},
        ),
        (
            schedules::WORK,
            "trigger_work_due",
            doc! {"at": 1, "_id": 1},
        ),
        (
            schedules::WORK,
            "trigger_work_parent",
            doc! {"trigger_id": 1, "_id": 1},
        ),
        (
            RUNS,
            "trigger_run_history",
            doc! {"trigger_id": 1, "scheduled_at": -1, "_id": -1},
        ),
        (
            RUNS,
            "trigger_run_overlap",
            doc! {"trigger_id": 1, "outcome": 1},
        ),
        (
            RUNS,
            "trigger_run_continuation_state",
            doc! {"continuation_protocol_version": 1, "outcome": 1, "_id": 1},
        ),
    ] {
        let indexes: Vec<_> = state
            .db
            .collection::<Document>(collection)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let index = indexes
            .iter()
            .find(|index| index.options.as_ref().and_then(|o| o.name.as_deref()) == Some(name))
            .unwrap();
        assert_eq!(index.keys, keys);
    }
    let indexes: Vec<_> = state
        .db
        .collection::<Document>(crate::models::assistant_group::COLLECTION_NAME)
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(indexes.iter().any(|index| index.keys
        == doc! {
        "user_id": 1, "followers.trigger_run_id": 1, "followers.conversation_id": 1}));

    reads.clear();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert_eq!(
        v2::source(&state.db, &run, &mut session).await.unwrap(),
        Some("group")
    );
    session.abort_transaction().await.unwrap();
    let finds = reads.finds();
    assert_eq!(
        finds
            .iter()
            .map(|q| q.get_str("find").unwrap())
            .collect::<Vec<_>>(),
        [
            CONVERSATIONS,
            RUNS,
            HANDOFFS,
            crate::models::assistant_group::COLLECTION_NAME
        ],
        "exact thread, capacity retry, pending direct receipt, then group follower"
    );
    let direct_query = &finds[2];
    assert_eq!(
        direct_query.get_document("filter").unwrap(),
        &doc! {"user_id": OWNER, "run_id": &run.id, "parent_thread_id": &row.id, "state": "pending"}
    );
    assert_eq!(
        direct_query.get_document("projection").unwrap(),
        &doc! {"_id": 1}
    );
    assert!(
        reads
            .batches
            .lock()
            .unwrap()
            .iter()
            .all(|count| *count <= 1)
    );
    for command in &finds {
        explain_find(&state.db, command, 1, 4).await;
    }
    let direct_report = explain_find(&state.db, direct_query, 0, 4).await;
    assert!(winning_plan(&direct_report).contains("direct_handoff_run"));
    let group_query = finds
        .iter()
        .find(|q| q.get_str("find") == Ok(crate::models::assistant_group::COLLECTION_NAME))
        .unwrap();
    let report = explain_find(&state.db, group_query, 1, 4).await;
    let plan = winning_plan(&report);
    assert!(plan.contains("followers.trigger_run_id"));
    for version in [bson::Bson::Int32(2), bson::Bson::Int64(2)] {
        let event = team::bound_event("group_settled", "Source lookup".into(), None, None)
            .for_trigger(Some(&run.id));
        let mut event = bson::to_document(&event).unwrap();
        event.insert("continuation_protocol_version", version);
        state
            .db
            .collection::<Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"pending_events": [event]}},
            )
            .await
            .unwrap();
        reads.clear();
        session.start_transaction().await.unwrap();
        assert_eq!(
            v2::source(&state.db, &run, &mut session).await.unwrap(),
            Some("event")
        );
        session.abort_transaction().await.unwrap();
        assert_eq!(reads.finds().len(), 2);
        assert!(
            reads
                .batches
                .lock()
                .unwrap()
                .iter()
                .all(|count| *count <= 1)
        );
    }
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"pending_events": []}})
        .await
        .unwrap();
    let mut no_automation = run.clone();
    no_automation.thread_id = None;
    reads.clear();
    session.start_transaction().await.unwrap();
    assert!(
        v2::source(&state.db, &no_automation, &mut session)
            .await
            .unwrap()
            .is_none()
    );
    session.abort_transaction().await.unwrap();
    assert!(
        reads.finds().is_empty(),
        "no automation thread adds no source reads"
    );

    // Public history, owner/trigger overlap and parent work removal use their
    // existing indexes; explains include actual execution, not index presence.
    for command in [
        doc! {"find": RUNS, "filter": {"trigger_id": &run.trigger_id, "user_id": OWNER},
        "sort": {"scheduled_at": -1, "_id": -1}, "limit": 100_i64},
        doc! {"find": RUNS, "filter": {"trigger_id": &run.trigger_id,
        "outcome": {"$in": ["started", "waiting"]}}},
        doc! {"find": schedules::WORK, "filter": {"trigger_id": &run.trigger_id}, "projection": {"_id": 1}},
    ] {
        explain_find(&state.db, &command, 2, 6).await;
    }
    due_now(&state, &run.id).await;
    reads.clear();
    assert_eq!(
        schedules::due(&state.db, Utc::now()).await.unwrap().len(),
        1
    );
    let due = reads.finds();
    assert_eq!(due.len(), 1);
    assert_eq!(mongo_number(&due[0], "limit"), 100);
    let report = explain_find(&state.db, &due[0], 0, 4).await;
    assert_eq!(
        mongo_number(report.get_document("executionStats").unwrap(), "nReturned"),
        1
    );
    let job = schedules::claim(&state.db, &run.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    schedules::reschedule(&state.db, &job, Some(Utc::now() + Duration::minutes(1)))
        .await
        .unwrap();
    reads.clear();
    assert!(
        schedules::due(&state.db, Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(reads.finds().len(), 1);
    assert!(
        reads
            .batches
            .lock()
            .unwrap()
            .iter()
            .all(|count| *count == 0)
    );
    reads.clear();
    v2::ready(&state.db, true).await.unwrap();
    assert_eq!(reads.finds().len(), 1);
    assert!(
        reads
            .batches
            .lock()
            .unwrap()
            .iter()
            .all(|count| *count <= 1)
    );
    explain_find(&state.db, &reads.finds()[0], 1, 1).await;
    reads.clear();
    assert!(
        v2::claim_event(&state.db, OWNER, "nyxa-wrong-thread", &run.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads.finds().len(),
        2,
        "readiness + exact run lookup; no owner-wide scan"
    );
    for command in reads.finds() {
        explain_find(&state.db, &command, 1, 1).await;
    }
    completion_reply(&state, &row, &run).await;
    reads.clear();
    let claim = v2::claim_event(&state.db, OWNER, &row.id, &run.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reads.finds().len(),
        3,
        "readiness, exact run, exact trigger"
    );
    assert_eq!(
        reads
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|q| q.contains_key("findAndModify"))
            .count(),
        1
    );
    for command in reads.finds() {
        explain_find(&state.db, &command, 1, 1).await;
    }
    reads.clear();
    assert!(
        v2::claim_event(&state.db, OWNER, &row.id, &run.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(reads.finds().len(), 3, "busy lease retry stays bounded");
    assert_eq!(
        reads
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|q| q.contains_key("findAndModify"))
            .count(),
        1
    );
    assert!(
        reads
            .batches
            .lock()
            .unwrap()
            .iter()
            .all(|count| *count <= 1)
    );
    // The wake entry point also leaves a leased run/event alone. This measures
    // the production owner checks/queue path and forbids unbounded discovery.
    completion_queue(&state, &row, &run).await;
    reads.clear();
    Box::pin(crate::handlers::assistant_team::wake(
        &state, OWNER, &row.id,
    ))
    .await;
    assert!(
        reads.finds().len() <= 24,
        "bounded wake reads: {}",
        reads.finds().len()
    );
    assert!(
        reads
            .finds()
            .iter()
            .filter(|q| q.get_str("find") == Ok(RUNS))
            .all(|q| q.get_document("filter").unwrap().get_str("_id") == Ok(run.id.as_str()))
    );
    let queued = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert!(queued.active_turn.is_none());
    assert_eq!(queued.pending_events.len(), 1);
    assert_eq!(completion_load(&state, &run.id).await.continuation_count, 0);
    assert!(
        state
            .db
            .collection::<Document>(schedules::WORK)
            .find_one(doc! {"_id": &run.id, "fence": &claim.fence})
            .await
            .unwrap()
            .is_some()
    );
    assert!(calls.lock().await.is_empty());
    assert_eq!(
        engine::get(&state.db, OWNER, &row.id).await.unwrap().id,
        row.id
    );
    server.abort();
}

#[tokio::test]
async fn automation_v2_mongo_migration_pages_and_durable_readiness_are_bounded() {
    let reads = MongoReads::default();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "auto2040_migration_bounds",
        reads.handler(),
    )
    .await;
    let (state, calls, server) = setup_with_database(db).await;
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let trigger = assistant_webhook(&state, &agent.id).await;
    let now = Utc::now();
    let mut runs = Vec::new();
    for n in 0..705 {
        let at = now - Duration::seconds(n);
        let mut run = schedules::new_run(&trigger, at, at + Duration::hours(1));
        if n >= 205 {
            run.outcome = RunOutcome::Completed;
        }
        runs.push(run);
    }
    state
        .db
        .collection::<TriggerRun>(RUNS)
        .insert_many(runs)
        .await
        .unwrap();
    abandon_marker(&state).await;
    // Capture the production discovery before migration and explain its exact
    // shape against terminal-history noise, then run all three real pages.
    let selector = doc! {"find": RUNS, "filter": {"_id": {"$gt": ""},
    "outcome": {"$in": ["pending", "started", "waiting"]},
    "continuation_protocol_version": {"$in": [null, 0]}},
    "projection": {"_id": 1}, "sort": {"_id": 1}, "limit": 100_i64};
    let report = explain_find(&state.db, &selector, 100, 120).await;
    assert!(winning_plan(&report).contains("trigger_run_continuation_state"));
    reads.clear();
    let marker = v2::migrate_checked(
        &state.db,
        now - Duration::seconds(2 * schedules::LEASE_SECONDS + 1),
        false,
    )
    .await
    .unwrap();
    assert_eq!(marker.get_i64("classified").unwrap(), 205);
    let finds = reads.finds();
    let pages: Vec<_> = finds
        .iter()
        .filter(|q| {
            q.get_str("find") == Ok(RUNS)
                && q.get_document("filter")
                    .is_ok_and(|f| f.get_document("_id").is_ok())
        })
        .collect();
    assert_eq!(pages.len(), 4, "100 + 100 + 5 + empty discovery");
    assert_eq!(
        pages[0].get_document("filter").unwrap(),
        selector.get_document("filter").unwrap(),
        "the explained selector must be the production migration selector"
    );
    assert!(
        pages.iter().all(|q| mongo_number(q, "limit") == 100
            && q.get_document("projection") == Ok(&doc! {"_id": 1}))
    );
    assert!(
        reads
            .batches
            .lock()
            .unwrap()
            .iter()
            .all(|count| *count <= 100)
    );
    assert!(
        finds.len() <= 4 + 205 * 3 + 5,
        "bounded point reads per pending run: {}",
        finds.len()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(RUNS)
            .count_documents(doc! {
            "outcome": "pending", "continuation_protocol_version": 2})
            .await
            .unwrap(),
        205
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(RUNS)
            .count_documents(doc! {
            "outcome": "completed", "continuation_protocol_version": 0})
            .await
            .unwrap(),
        500
    );
    // Keep a completed migration marker's proof observable on a fresh durable
    // handle; readiness never enumerates those 705 historical/current rows.
    reads.clear();
    v2::ready(
        &crate::services::transaction_service::durable_database(&state.db),
        true,
    )
    .await
    .unwrap();
    assert_eq!(reads.finds().len(), 1);
    assert_eq!(reads.finds()[0].get_str("find"), Ok(v2::MIGRATIONS));
    assert!(
        reads
            .batches
            .lock()
            .unwrap()
            .iter()
            .all(|count| *count <= 1)
    );
    assert!(!v2::rollback_ready(&state.db).await.unwrap());
    assert!(calls.lock().await.is_empty());
    server.abort();
}
