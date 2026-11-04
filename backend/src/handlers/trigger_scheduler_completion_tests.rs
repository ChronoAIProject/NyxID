// Included in trigger_scheduler::tests so all fixtures use the existing fake
// NyxAgent, transaction topology guard and unique, managed test database names.
use crate::models::assistant_conversation::{AgentEvent, TurnOrigin};
use crate::models::trigger_run::RunWaitKind;
use crate::services::{
    assistant_group_service as completion_groups, trigger_schedule::continuation as continuation_v2,
};
use futures::TryStreamExt;

fn completion_start(
    thread: &str,
    origin: TurnOrigin,
    claim: Option<schedules::TurnClaim>,
) -> engine::TurnStart {
    let request: engine::TurnRequest = serde_json::from_value(
        json!({"conversation_id":thread,"text":"automation completion fixture"}),
    )
    .unwrap();
    let mut start = engine::TurnStart::from(&request);
    start.origin = origin;
    start.trigger = claim;
    start
}

async fn completion_load(state: &AppState, id: &str) -> TriggerRun {
    state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

async fn completion_coordinator(state: &AppState) -> (Trigger, TriggerRun, AssistantConversation) {
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let trigger = automation(state, &agent.id, None).await;
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
    (trigger, completion_load(state, &run.id).await, row)
}

async fn completion_reply(state: &AppState, row: &AssistantConversation, run: &TriggerRun) {
    engine::finish_turn(
        &state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Work delegated to group".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    settled(state, row, &run.id, "Work delegated to group", None).await;
}

async fn completion_group(
    state: &AppState,
    row: &AssistantConversation,
    run: &TriggerRun,
) -> crate::models::assistant_group::AssistantGroup {
    let group = completion_groups::create(
        &state.db,
        OWNER,
        "Completion 2040 fixture",
        &[row.agent_id.clone().unwrap()],
        "nyxbot",
    )
    .await
    .unwrap();
    completion_groups::append_following(
        &state.db,
        OWNER,
        &group.id,
        "agent",
        None,
        "Research stage",
        &[],
        Vec::new(),
        Some((&row.id, Some(&run.id))),
    )
    .await
    .unwrap();
    completion_groups::get(&state.db, OWNER, &group.id)
        .await
        .unwrap()
}

async fn completion_queue(
    state: &AppState,
    row: &AssistantConversation,
    run: &TriggerRun,
) -> AgentEvent {
    let event = team::bound_event(
        "group_settled",
        "Local fixture group result".into(),
        None,
        None,
    )
    .for_trigger(Some(&run.id));
    engine::push_events(&state.db, OWNER, &row.id, vec![event.clone()])
        .await
        .unwrap();
    event
}

#[tokio::test]
async fn automation_v2_completion_barrier_claim_restart_and_selective_drain() {
    let (state, _, server) = setup("auto2040_barrier").await;
    // The ordinary event-loop limit must not disable one real automation
    // group wait/resume. Its finite allowance belongs to the persisted run.
    state.db.collection::<Document>(crate::models::assistant_settings::COLLECTION_NAME)
        .update_one(doc! {"_id":OWNER},doc! {"$set":{"max_auto_continuations":0},"$setOnInsert":{"updated_at":schedules::date(Utc::now())}})
        .upsert(true).await.unwrap();
    let (trigger, run, row) = completion_coordinator(&state).await;
    assert_eq!(
        run.continuation_limit,
        continuation_v2::AUTOMATION_CONTINUATION_LIMIT
    );
    let group = completion_group(&state, &row, &run).await;
    // Queue before the coordinator has settled: this is the historical race.
    let (claims, _) = completion_groups::claim_followers_if_quiet(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    let first = claims[0].clone();
    let mut event = team::bound_event(
        "group_settled",
        "First specialist result".into(),
        None,
        None,
    )
    .for_trigger(Some(&run.id));
    event.id = first.notification.as_ref().unwrap().event_id.clone();
    assert!(
        completion_groups::deliver_follower(&state.db, OWNER, &group.id, &first, &event)
            .await
            .unwrap()
    );
    assert!(
        !completion_groups::deliver_follower(&state.db, OWNER, &group.id, &first, &event)
            .await
            .unwrap()
    );
    let ordinary = team::event("message", "Unrelated owner notice".into(), None);
    let other = team::event("group_settled", "Another run".into(), None)
        .for_trigger(Some("automation-2040-other-run"));
    engine::push_events(
        &state.db,
        OWNER,
        &row.id,
        vec![ordinary.clone(), other.clone(), event.clone()],
    )
    .await
    .unwrap();
    completion_reply(&state, &row, &run).await;
    let waiting = completion_load(&state, &run.id).await;
    assert_eq!(waiting.outcome, RunOutcome::Waiting);
    assert_eq!(waiting.wait_kind, Some(RunWaitKind::Coordinator));
    assert_eq!(waiting.continuation_count, 0);
    assert!(!waiting.terminal_result_recorded);
    assert!(!waiting.result_claimed);
    assert!(waiting.finished_at.is_none());
    assert!(
        continuation_v2::claim_event(&state.db, OWNER, "wrong-thread", &run.id)
            .await
            .unwrap()
            .is_none()
    );
    let claim = continuation_v2::claim_event(&state.db, OWNER, &row.id, &run.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        continuation_v2::claim_event(&state.db, OWNER, &row.id, &run.id)
            .await
            .unwrap()
            .is_none()
    );
    let row = engine::begin_turn(
        &state.db,
        OWNER,
        completion_start(&row.id, TurnOrigin::Event, Some(claim.clone())),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let turn = row.active_turn.as_ref().unwrap();
    assert_eq!(turn.trigger_run_id.as_deref(), Some(run.id.as_str()));
    assert!(turn.trigger_continuation);
    assert_eq!(turn.events.len(), 1);
    assert_eq!(turn.events[0].id, event.id);
    assert_eq!(row.event_streak, 0);
    assert_eq!(
        row.pending_events
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![ordinary.id, other.id]
    );
    let admitted = completion_load(&state, &run.id).await;
    assert_eq!(admitted.continuation_count, 1);
    assert_eq!(admitted.continuation_limit, waiting.continuation_limit);
    assert!(
        engine::begin_turn(
            &state.db,
            OWNER,
            completion_start(&row.id, TurnOrigin::Event, Some(claim)),
            &state.encryption_keys
        )
        .await
        .is_err()
    );
    completion_reply(&state, &row, &admitted).await;
    let final_run = completion_load(&state, &run.id).await;
    assert_eq!(final_run.outcome, RunOutcome::Completed);
    assert_eq!(final_run.trigger_id, trigger.id);
    assert_eq!(final_run.continuation_count, 1);
    let budget = state
        .db
        .collection::<Document>(schedules::BUDGETS)
        .find_one(doc! {"_id":OWNER})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(budget.get_i32("hourly").unwrap(), 2);
    assert_eq!(budget.get_i32("daily").unwrap(), 2);
    server.abort();
}

/// Hold an actual empty-source snapshot across registration, and separately
/// recheck committed Stop/terminal state. No ordering relies on sleeps.
#[tokio::test]
async fn automation_v2_completion_barrier_fences_registration_and_stop() {
    for winner in ["registration", "barrier", "stop", "terminal"] {
        let (state, calls, server) = setup(&format!("auto2040_f4_{winner}")).await;
        let (_, run, row) = completion_coordinator(&state).await;
        let group = completion_groups::create(
            &state.db,
            OWNER,
            "Completion F4 race fixture",
            &[row.agent_id.clone().unwrap()],
            "nyxbot",
        )
        .await
        .unwrap();

        if winner == "barrier" {
            // The empty-source decision commits first. Registration must also
            // fail in the gap before the run's separate terminal projection.
            assert!(
                !continuation_v2::mark_coordinator_waiting(&state.db, &run)
                    .await
                    .unwrap()
            );
            let saved = completion_load(&state, &run.id).await;
            assert_eq!(saved.outcome, RunOutcome::Started);
            assert!(!saved.terminal_result_recorded);
        } else {
            let mut session = state.db.client().start_session().await.unwrap();
            session
                .start_transaction()
                .read_concern(mongodb::options::ReadConcern::snapshot())
                .await
                .unwrap();
            assert!(
                continuation_v2::source(&state.db, &run, &mut session)
                    .await
                    .unwrap()
                    .is_none()
            );
            if winner == "registration" {
                completion_groups::append_following(
                    &state.db,
                    OWNER,
                    &group.id,
                    "agent",
                    None,
                    "Handoff committed after the completion snapshot",
                    &[],
                    Vec::new(),
                    Some((&row.id, Some(&run.id))),
                )
                .await
                .unwrap();
            } else if winner == "stop" {
                engine::request_stop(&state.db, OWNER, &row.id)
                    .await
                    .unwrap();
            } else {
                schedules::finish(
                    &state.db,
                    &run.id,
                    RunOutcome::Completed,
                    None,
                    &state.config.frontend_url,
                )
                .await
                .unwrap();
            }
            if winner == "registration" {
                let error = continuation_v2::mark_coordinator_waiting_in_session(
                    &state.db,
                    &run,
                    &mut session,
                )
                .await
                .expect_err("a stale registration snapshot must not commit a barrier decision");
                assert!(
                    matches!(&error, AppError::DatabaseError(error)
                    if error.contains_label("TransientTransactionError")),
                    "{error}"
                );
                let _ = session.abort_transaction().await;
            } else {
                // Stop/terminal cleanup has committed. A fresh transaction
                // must suppress settlement at the live-run predicate; no
                // transient error is needed to prove this terminal ordering.
                session.abort_transaction().await.unwrap();
                session.start_transaction().await.unwrap();
                assert_eq!(
                    continuation_v2::mark_coordinator_waiting_in_session(
                        &state.db,
                        &run,
                        &mut session,
                    )
                    .await
                    .unwrap(),
                    (true, false)
                );
                session.commit_transaction().await.unwrap();
            }
        }

        if winner == "registration" {
            // Fresh production settlement rechecks the committed follower and
            // waits without terminal projection or outbound delivery.
            completion_reply(&state, &row, &run).await;
            let waiting = completion_load(&state, &run.id).await;
            assert_eq!(waiting.outcome, RunOutcome::Waiting);
            assert_eq!(waiting.wait_kind, Some(RunWaitKind::Coordinator));
            assert!(!waiting.terminal_result_recorded);
            assert!(!waiting.result_claimed);
            assert!(waiting.finished_at.is_none());
            assert_eq!(
                completion_groups::get(&state.db, OWNER, &group.id)
                    .await
                    .unwrap()
                    .followers
                    .len(),
                1
            );
            // The persisted seal must reopen only with the next admitted turn.
            completion_queue(&state, &row, &run).await;
            let claim = continuation_v2::claim_event(&state.db, OWNER, &row.id, &run.id)
                .await
                .unwrap()
                .unwrap();
            let next = engine::begin_turn(
                &state.db,
                OWNER,
                completion_start(&row.id, TurnOrigin::Event, Some(claim)),
                &state.encryption_keys,
            )
            .await
            .unwrap();
            completion_groups::append_following(
                &state.db,
                OWNER,
                &group.id,
                "agent",
                None,
                "The next exact turn may delegate again",
                &[],
                Vec::new(),
                Some((&next.id, Some(&run.id))),
            )
            .await
            .unwrap();
            // The opposite Stop ordering: the next barrier commits waiting
            // first, then Stop removes both its follower and queued event.
            let next_run = completion_load(&state, &run.id).await;
            let (followers, _) =
                completion_groups::claim_followers_if_quiet(&state.db, OWNER, &group.id)
                    .await
                    .unwrap();
            let follower = followers.into_iter().next().unwrap();
            let mut event = team::bound_event(
                "group_settled",
                "Queued result before Stop".into(),
                None,
                None,
            )
            .for_trigger(Some(&run.id));
            event.id = follower.notification.as_ref().unwrap().event_id.clone();
            engine::push_events(&state.db, OWNER, &next.id, vec![event.clone()])
                .await
                .unwrap();
            assert!(
                continuation_v2::mark_coordinator_waiting(&state.db, &next_run)
                    .await
                    .unwrap()
            );
            engine::request_stop(&state.db, OWNER, &next.id)
                .await
                .unwrap();
            assert!(
                !completion_groups::deliver_follower(
                    &state.db, OWNER, &group.id, &follower, &event,
                )
                .await
                .unwrap(),
                "a follower claim captured before Stop cannot requeue work"
            );
            settled(
                &state,
                &next,
                &next_run.id,
                "Late completion after Stop",
                None,
            )
            .await;
            let stopped = completion_load(&state, &run.id).await;
            assert_eq!(stopped.outcome, RunOutcome::Failed);
            assert_eq!(stopped.reason.as_deref(), Some("cancelled"));
            assert!(stopped.terminal_result_recorded);
            assert!(
                completion_groups::get(&state.db, OWNER, &group.id)
                    .await
                    .unwrap()
                    .followers
                    .is_empty()
            );
            let thread = engine::get(&state.db, OWNER, &next.id).await.unwrap();
            assert!(!thread.coordinator_run_ids.contains(&run.id));
            assert!(
                thread
                    .pending_events
                    .iter()
                    .all(|event| event.trigger_run_id.as_deref() != Some(run.id.as_str()))
            );
        } else {
            let before = completion_groups::get(&state.db, OWNER, &group.id)
                .await
                .unwrap();
            assert!(
                completion_groups::append_following(
                    &state.db,
                    OWNER,
                    &group.id,
                    "agent",
                    None,
                    "A late handoff must roll back",
                    &[],
                    Vec::new(),
                    Some((&row.id, Some(&run.id))),
                )
                .await
                .is_err()
            );
            let after = completion_groups::get(&state.db, OWNER, &group.id)
                .await
                .unwrap();
            assert_eq!(after.message_count, before.message_count);
            assert!(after.followers.is_empty());
            if winner == "barrier" {
                completion_reply(&state, &row, &run).await;
            }
            let terminal = completion_load(&state, &run.id).await;
            assert_eq!(
                terminal.outcome,
                if winner == "stop" {
                    RunOutcome::Failed
                } else {
                    RunOutcome::Completed
                }
            );
            assert!(terminal.terminal_result_recorded);
            if winner == "stop" {
                assert_eq!(terminal.reason.as_deref(), Some("cancelled"));
            }
            assert!(
                continuation_v2::mark_coordinator_waiting(&state.db, &run)
                    .await
                    .unwrap(),
                "late settlement must not revive a terminal run"
            );
            assert_eq!(
                completion_load(&state, &run.id).await.outcome,
                terminal.outcome
            );
            let thread = engine::get(&state.db, OWNER, &row.id).await.unwrap();
            assert!(!thread.coordinator_run_ids.contains(&run.id));
            assert!(
                thread
                    .pending_events
                    .iter()
                    .all(|event| event.trigger_run_id.as_deref() != Some(run.id.as_str()))
            );
        }
        assert!(calls.lock().await.is_empty());
        server.abort();
    }
}

#[tokio::test]
async fn automation_v2_trigger_update_persists_protocol_and_preserves_other_states() {
    let (state, calls, server) = setup("auto2040_f5_update").await;
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    for (status, assistant, previous, expected) in [
        (TriggerStatus::Active, true, 0, 2),
        (TriggerStatus::Disabled, true, 0, 0),
        (TriggerStatus::Disabled, true, 2, 2),
        (TriggerStatus::Active, false, 0, 0),
        (TriggerStatus::Disabled, false, 0, 0),
        (TriggerStatus::Active, false, 2, 2),
    ] {
        let trigger = automation(&state, &agent.id, None).await;
        if previous == 0 {
            // Legacy documents lack this field. Read the serde-defaulted row
            // back so update exercises its real exact-document predicate.
            state
                .db
                .collection::<Document>(TRIGGERS)
                .update_one(
                    doc! {"_id": &trigger.id},
                    doc! {"$unset": {"continuation_protocol_version": ""}},
                )
                .await
                .unwrap();
        }
        let current = state
            .db
            .collection::<Trigger>(TRIGGERS)
            .find_one(doc! {"_id": &trigger.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.continuation_protocol_version, previous);
        let delivery = if assistant {
            current.delivery.clone()
        } else {
            TriggerDelivery::Notification
        };
        let updated = trigger_service::update(
            &state.db,
            &state.encryption_keys,
            &current,
            trigger_service::UpdateInput {
                label: Some("Protocol update F5 fixture".into()),
                status: Some(status),
                delivery: Some(delivery.clone()),
                schedule: None,
                overlap: None,
            },
        )
        .await
        .unwrap()
        .trigger;
        let stored = state
            .db
            .collection::<Document>(TRIGGERS)
            .find_one(doc! {"_id": &trigger.id})
            .await
            .unwrap()
            .unwrap();
        assert!(stored.contains_key("continuation_protocol_version"));
        let reloaded: Trigger = bson::from_document(stored).unwrap();
        assert_eq!(updated.continuation_protocol_version, expected);
        assert_eq!(reloaded.continuation_protocol_version, expected);
        assert_eq!(reloaded.status, status);
        assert_eq!(reloaded.delivery, delivery);
        assert_eq!(reloaded.updated_at, updated.updated_at);
        assert!(reloaded.updated_at > current.updated_at);
        assert!(matches!(
            trigger_service::update(
                &state.db,
                &state.encryption_keys,
                &current,
                trigger_service::UpdateInput {
                    label: Some("Stale generation must be rejected".into()),
                    status: None,
                    delivery: None,
                    schedule: None,
                    overlap: None,
                },
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        trigger_service::delete(&state.db, &reloaded).await.unwrap();
    }
    assert!(calls.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn automation_v2_follower_lost_claim_queue_full_expiry_and_cross_run_ids() {
    let (state, _, server) = setup("auto2040_follower").await;
    let (_, run, row) = completion_coordinator(&state).await;
    let group = completion_group(&state, &row, &run).await;
    let (claims, _) = completion_groups::claim_followers_if_quiet(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    let old = claims[0].clone();
    let mut later = old.clone();
    later.trigger_run_id = Some("another-run".into());
    assert_ne!(
        completion_groups::follower_event_id(&group.id, &old, 2),
        completion_groups::follower_event_id(&group.id, &later, 2)
    );
    assert!(
        completion_groups::claim_followers_if_quiet(&state.db, OWNER, &group.id)
            .await
            .unwrap()
            .0
            .is_empty()
    );
    // Simulate process loss: the durable snapshot stays, only its lease expires.
    state.db.collection::<Document>(crate::models::assistant_group::COLLECTION_NAME).update_one(doc!{"_id":&group.id},doc!{"$set":{"followers.0.notification.lease_until":schedules::date(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    completion_groups::append(
        &state.db,
        OWNER,
        &group.id,
        "agent",
        None,
        "A later reply outside the frozen snapshot",
        Vec::new(),
    )
    .await
    .unwrap();
    let (claims, _) = completion_groups::claim_followers_if_quiet(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    let current = &claims[0];
    assert_eq!(
        current.notification.as_ref().unwrap().upper_seq,
        old.notification.as_ref().unwrap().upper_seq
    );
    assert_eq!(
        current.notification.as_ref().unwrap().event_id,
        old.notification.as_ref().unwrap().event_id
    );
    assert_ne!(
        current.notification.as_ref().unwrap().fence,
        old.notification.as_ref().unwrap().fence
    );
    let mut event = team::event("group_settled", "Crash recovery fixture".into(), None)
        .for_trigger(Some(&run.id));
    event.id = current.notification.as_ref().unwrap().event_id.clone();
    assert!(
        !completion_groups::deliver_follower(&state.db, OWNER, &group.id, &old, &event)
            .await
            .unwrap()
    );
    let full: Vec<_> = (0..engine::MAX_PENDING_EVENTS)
        .map(|n| {
            team::event("message", format!("fixture {n}"), None).for_trigger(Some("other-run"))
        })
        .collect();
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"pending_events":bson::to_bson(&full).unwrap()}},
        )
        .await
        .unwrap();
    assert!(
        completion_groups::deliver_follower(&state.db, OWNER, &group.id, current, &event)
            .await
            .is_err()
    );
    assert_eq!(
        completion_groups::get(&state.db, OWNER, &group.id)
            .await
            .unwrap()
            .followers
            .len(),
        1
    );
    assert!(
        engine::push_events(
            &state.db,
            OWNER,
            &row.id,
            vec![team::event(
                "message",
                "ordinary cannot evict a trigger".into(),
                None
            )]
        )
        .await
        .is_err()
    );
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"pending_events":[]}})
        .await
        .unwrap();
    assert!(
        completion_groups::deliver_follower(&state.db, OWNER, &group.id, current, &event)
            .await
            .unwrap()
    );
    assert!(
        completion_groups::get(&state.db, OWNER, &group.id)
            .await
            .unwrap()
            .followers
            .is_empty()
    );
    server.abort();
}

#[tokio::test]
async fn automation_v2_deadline_limit_and_stop() {
    for reason in [
        "coordinator_timeout",
        "continuation_limit_exceeded",
        "cancelled",
        "thread_deleted",
    ] {
        let (state, calls, server) = setup(&format!("auto2040_{reason}")).await;
        let (_, run, row) = completion_coordinator(&state).await;
        completion_queue(&state, &row, &run).await;
        completion_reply(&state, &row, &run).await;
        let budget_before = state
            .db
            .collection::<Document>(schedules::BUDGETS)
            .find_one(doc! {"_id": OWNER})
            .await
            .unwrap()
            .unwrap();
        match reason {
            "cancelled" => {
                engine::request_stop(&state.db, OWNER, &row.id)
                    .await
                    .unwrap();
            }
            "thread_deleted" => {
                engine::delete(&state.db, OWNER, &row.id).await.unwrap();
                Box::pin(tick(&state)).await.unwrap();
            }
            _ => {
                let set = if reason == "coordinator_timeout" {
                    doc! {"coordinator_deadline":schedules::date(Utc::now()-Duration::seconds(1))}
                } else {
                    doc! {"continuation_count":run.continuation_limit}
                };
                state
                    .db
                    .collection::<Document>(RUNS)
                    .update_one(doc! {"_id":&run.id}, doc! {"$set":set})
                    .await
                    .unwrap();
                Box::pin(tick(&state)).await.unwrap();
            }
        }
        let saved = completion_load(&state, &run.id).await;
        assert_eq!(saved.outcome, RunOutcome::Failed, "{reason}");
        assert_eq!(saved.reason.as_deref(), Some(reason));
        assert!(saved.terminal_result_recorded);
        assert!(saved.finished_at.is_some());
        assert_eq!(
            saved.continuation_count,
            if reason == "continuation_limit_exceeded" {
                run.continuation_limit
            } else {
                0
            }
        );
        let budget_after = state
            .db
            .collection::<Document>(schedules::BUDGETS)
            .find_one(doc! {"_id": OWNER})
            .await
            .unwrap()
            .unwrap();
        for field in ["hour", "day", "hourly", "daily"] {
            assert_eq!(
                budget_after.get(field),
                budget_before.get(field),
                "{reason}"
            );
        }
        assert!(calls.lock().await.is_empty());
        server.abort();
    }
}

#[tokio::test]
async fn automation_v2_migration_readiness_real_leases_confirmation_and_ambiguity() {
    let (state, _, server) = setup("auto2040_migrate").await;
    let (trigger, run, row) = completion_coordinator(&state).await;
    engine::finish_turn(
        &state.db,
        &row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Legacy fixture".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let mut ambiguous = run.clone();
    ambiguous.id = Uuid::new_v4().to_string();
    ambiguous.outcome = RunOutcome::Waiting;
    ambiguous.continuation_protocol_version = 0;
    ambiguous.scheduled_at = run.scheduled_at + Duration::milliseconds(1);
    state
        .db
        .collection::<TriggerRun>(RUNS)
        .insert_one(&ambiguous)
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(schedules::WORK)
        .insert_one(
            doc! {"_id": &ambiguous.id, "kind": "run", "trigger_id": &trigger.id,
            "user_id": OWNER, "at": schedules::date(Utc::now()), "fence": ""},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(TRIGGERS)
        .update_one(
            doc! {"_id": &trigger.id},
            doc! {"$set": {"history_count": 2_i64}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$addToSet": {"coordinator_run_ids": &ambiguous.id}},
        )
        .await
        .unwrap();
    state.db.collection::<Document>(RUNS).update_one(doc!{"_id":&run.id},doc!{"$set":{"outcome":"waiting","waiting_for":["preserved-confirmation"],"continuation_protocol_version":0},"$unset":{"continuation_limit":"","continuation_count":"","wait_kind":""}}).await.unwrap();
    state
        .db
        .collection::<Document>(continuation_v2::MIGRATIONS)
        .delete_one(doc! {"_id":continuation_v2::MIGRATION_ID})
        .await
        .unwrap();
    assert!(continuation_v2::ready(&state.db, true).await.is_err());
    assert!(continuation_v2::ready(&state.db, false).await.is_err());
    let drained = Utc::now() - Duration::seconds(2 * schedules::LEASE_SECONDS + 1);
    assert!(
        continuation_v2::migrate_checked(&state.db, Utc::now(), false)
            .await
            .is_err()
    );
    assert!(
        continuation_v2::migrate_checked(&state.db, drained, true)
            .await
            .is_err()
    );
    assert!(
        continuation_v2::migrate_checked(&state.db, drained, false)
            .await
            .is_err()
    );
    state
        .db
        .collection::<Document>(schedules::WORK)
        .update_many(
            doc! {},
            doc! {"$set":{"at":schedules::date(Utc::now()-Duration::seconds(1))}},
        )
        .await
        .unwrap();
    let marker = continuation_v2::migrate_checked(&state.db, drained, false)
        .await
        .unwrap();
    assert!(marker.contains_key("completed_at"));
    let migrated = completion_load(&state, &run.id).await;
    assert_eq!(migrated.wait_kind, Some(RunWaitKind::Confirmation));
    assert_eq!(migrated.waiting_for, vec!["preserved-confirmation"]);
    assert_eq!(migrated.continuation_protocol_version, 2);
    let failed = completion_load(&state, &ambiguous.id).await;
    assert_eq!(failed.reason.as_deref(), Some("legacy_wait_unclassified"));
    assert_eq!(failed.outcome, RunOutcome::Failed);
    assert!(failed.terminal_result_recorded);
    assert!(failed.finished_at.is_some());
    assert!(failed.duration_ms.is_some());
    assert!(
        state
            .db
            .collection::<Document>(schedules::WORK)
            .find_one(doc! {"_id": &ambiguous.id})
            .await
            .unwrap()
            .is_none()
    );
    let history = schedules::history(&state.db, &trigger, None, None)
        .await
        .unwrap();
    assert_eq!(history[0].id, ambiguous.id);
    assert!(history[0].terminal_result_recorded);
    let projected = state
        .db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! {"_id": &trigger.id})
        .await
        .unwrap()
        .unwrap();
    let last = projected.schedule_state.last_run.as_ref().unwrap();
    assert_eq!(last.id, ambiguous.id);
    assert_eq!(last.outcome, RunOutcome::Failed);
    assert!(last.terminal_result_recorded);
    assert!(
        !engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .coordinator_run_ids
            .contains(&ambiguous.id)
    );
    let outcomes = state.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"user_id": OWNER, "event_type": "trigger_run_outcome", "event_data.run_id": &ambiguous.id})
        .await.unwrap().try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(outcomes.len(), 1);
    let metadata = outcomes[0].get_document("event_data").unwrap();
    assert_eq!(metadata.get_str("outcome").unwrap(), "failed");
    assert_eq!(
        metadata.get_str("reason").unwrap(),
        "legacy_wait_unclassified"
    );
    assert_eq!(
        marker.get_array("ambiguous").unwrap()[0],
        bson::Bson::String(ambiguous.id.clone())
    );
    let again = continuation_v2::migrate_checked(&state.db, drained, false)
        .await
        .unwrap();
    assert_eq!(
        again.get_i64("classified").unwrap(),
        marker.get_i64("classified").unwrap()
    );
    assert_eq!(again.get_i64("ambiguous_count").unwrap(), 1);
    let replayed = completion_load(&state, &ambiguous.id).await;
    assert_eq!(replayed.finished_at, failed.finished_at);
    assert_eq!(replayed.duration_ms, failed.duration_ms);
    assert_eq!(
        state
            .db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(
                doc! {"event_type": "trigger_run_outcome", "event_data.run_id": &ambiguous.id}
            )
            .await
            .unwrap(),
        1
    );
    assert!(!continuation_v2::rollback_ready(&state.db).await.unwrap());
    schedules::finish(
        &state.db,
        &run.id,
        RunOutcome::Failed,
        Some("cancelled"),
        &state.config.frontend_url,
    )
    .await
    .unwrap();
    assert!(continuation_v2::rollback_ready(&state.db).await.unwrap());
    let stored = state
        .db
        .collection::<Document>(TRIGGERS)
        .find_one(doc! {"_id":&trigger.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.get_i64("continuation_protocol_version").unwrap(), 2);
    server.abort();
}

#[tokio::test]
async fn automation_v2_migration_terminalization_is_atomic_and_generation_fenced() {
    for (stale, webhook) in [(false, false), (true, false), (false, true), (true, true)] {
        let (state, calls, server) = setup(&format!("auto2040_f7_atomic_{stale}_{webhook}")).await;
        let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
        let trigger = if webhook {
            acceptance::assistant_webhook(&state, &agent.id).await
        } else {
            automation(&state, &agent.id, None).await
        };
        let trigger = trigger_service::get_for_actor(&state.db, OWNER, &trigger.id)
            .await
            .unwrap();
        let now = Utc::now();
        let mut legacy = schedules::new_run(&trigger, now, now + Duration::minutes(5));
        legacy.outcome = RunOutcome::Waiting;
        legacy.started_at = Some(now - Duration::minutes(1));
        legacy.trigger_generation = Some(if stale {
            trigger.updated_at - Duration::milliseconds(1)
        } else {
            trigger.updated_at
        });
        state
            .db
            .collection::<TriggerRun>(RUNS)
            .insert_one(&legacy)
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(schedules::WORK)
            .insert_one(
                doc! {"_id": &legacy.id, "kind": "run", "trigger_id": &trigger.id,
                "user_id": OWNER, "at": schedules::date(now), "fence": ""},
            )
            .await
            .unwrap();
        let old_count = if stale { 0 } else { schedules::HISTORY_LIMIT };
        if old_count > 0 {
            let history: Vec<_> = (1..=old_count)
                .map(|n| {
                    let at = now - Duration::minutes(n as i64);
                    let mut old = schedules::new_run(&trigger, at, at);
                    old.outcome = RunOutcome::Completed;
                    old
                })
                .collect();
            state
                .db
                .collection::<TriggerRun>(RUNS)
                .insert_many(history)
                .await
                .unwrap();
        }
        state
            .db
            .collection::<Document>(TRIGGERS)
            .update_one(
                doc! {"_id": &trigger.id},
                doc! {"$set": {
                    "history_count": (old_count + 1) as i64,
                    "schedule_state.terminal_failure_code": "credential_invalid",
                    "schedule_state.terminal_failure_count": 2_i64,
                }},
            )
            .await
            .unwrap();
        let fence = Uuid::new_v4().to_string();
        state
            .db
            .collection::<Document>(continuation_v2::MIGRATIONS)
            .replace_one(
                doc! {"_id": continuation_v2::MIGRATION_ID},
                doc! {
                    "_id": continuation_v2::MIGRATION_ID, "classified": 0_i64,
                    "triggers": 0_i64, "ambiguous": [], "ambiguous_count": 0_i64,
                    "fence": &fence, "lease_until": schedules::date(now + Duration::minutes(5)),
                },
            )
            .await
            .unwrap();

        // Execute the production body, verify its terminal write is visible
        // in this real session, then abort as a process loss before commit.
        let mut session = state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        continuation_v2::migrate_run_in_session(&state.db, &legacy.id, &fence, &mut session)
            .await
            .unwrap();
        let provisional = state
            .db
            .collection::<TriggerRun>(RUNS)
            .find_one(doc! {"_id": &legacy.id})
            .session(&mut session)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(provisional.outcome, RunOutcome::Failed);
        assert!(provisional.terminal_result_recorded);
        session.abort_transaction().await.unwrap();
        let unchanged = completion_load(&state, &legacy.id).await;
        assert_eq!(unchanged.outcome, RunOutcome::Waiting);
        assert_eq!(unchanged.continuation_protocol_version, 0);
        assert!(!unchanged.terminal_result_recorded);
        assert!(
            state
                .db
                .collection::<Document>(schedules::WORK)
                .find_one(doc! {"_id": &legacy.id})
                .await
                .unwrap()
                .is_some()
        );
        let before = state
            .db
            .collection::<Trigger>(TRIGGERS)
            .find_one(doc! {"_id": &trigger.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.schedule_state.terminal_failure_count, 2);
        assert!(before.schedule_state.last_run.is_none());
        assert_eq!(
            state
                .db
                .collection::<Document>(RUNS)
                .count_documents(doc! {"trigger_id": &trigger.id})
                .await
                .unwrap(),
            old_count + 1
        );
        assert_eq!(
            state
                .db
                .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(
                    doc! {"event_type": "trigger_run_outcome", "event_data.run_id": &legacy.id}
                )
                .await
                .unwrap(),
            0
        );
        let progress = state
            .db
            .collection::<Document>(continuation_v2::MIGRATIONS)
            .find_one(doc! {"_id": continuation_v2::MIGRATION_ID})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(progress.get_i64("classified").unwrap(), 0);
        assert_eq!(progress.get_i64("ambiguous_count").unwrap(), 0);
        assert!(
            state
                .db
                .collection::<Document>(continuation_v2::MIGRATIONS)
                .find_one(doc! {"_id": format!("continuation-v2:ambiguous:{}", legacy.id)})
                .await
                .unwrap()
                .is_none()
        );

        // Expire only this fixture's abandoned migration lease, then retry the
        // full operator path. No sleep or mock replaces transaction behavior.
        state
            .db
            .collection::<Document>(continuation_v2::MIGRATIONS)
            .update_one(
                doc! {"_id": continuation_v2::MIGRATION_ID},
                doc! {"$set": {"lease_until": schedules::date(now - Duration::seconds(1))}},
            )
            .await
            .unwrap();
        let drained = now - Duration::seconds(2 * schedules::LEASE_SECONDS + 1);
        let marker = continuation_v2::migrate_checked(&state.db, drained, false)
            .await
            .unwrap();
        assert!(marker.contains_key("completed_at"));
        assert_eq!(marker.get_i64("ambiguous_count").unwrap(), 1);
        let failed = completion_load(&state, &legacy.id).await;
        assert_eq!(failed.outcome, RunOutcome::Failed);
        assert_eq!(failed.reason.as_deref(), Some("legacy_wait_unclassified"));
        assert!(failed.terminal_result_recorded);
        assert!(failed.finished_at.is_some());
        assert!(failed.duration_ms.is_some_and(|duration| duration > 0));
        assert_eq!(failed.trigger_generation, legacy.trigger_generation);
        let projected = state
            .db
            .collection::<Trigger>(TRIGGERS)
            .find_one(doc! {"_id": &trigger.id})
            .await
            .unwrap()
            .unwrap();
        let last = projected.schedule_state.last_run.as_ref().unwrap();
        assert_eq!(last.id, legacy.id);
        assert_eq!(last.outcome, RunOutcome::Failed);
        assert_eq!(last.reason.as_deref(), Some("legacy_wait_unclassified"));
        assert!(last.terminal_result_recorded);
        assert_eq!(projected.status, TriggerStatus::Active);
        assert!(projected.schedule_state.pause_reason.is_none());
        assert_eq!(
            projected.schedule_state.terminal_failure_count,
            if stale { 2 } else { 0 }
        );
        assert_eq!(
            projected.schedule_state.terminal_failure_code.as_deref(),
            if stale {
                Some("credential_invalid")
            } else {
                None
            }
        );
        let expected_history = (old_count + 1).min(schedules::HISTORY_LIMIT);
        assert_eq!(
            state
                .db
                .collection::<Document>(RUNS)
                .count_documents(doc! {"trigger_id": &trigger.id})
                .await
                .unwrap(),
            expected_history
        );
        let raw = state
            .db
            .collection::<Document>(TRIGGERS)
            .find_one(doc! {"_id": &trigger.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            raw.get_i64("history_count").unwrap(),
            expected_history as i64
        );
        assert!(
            state
                .db
                .collection::<Document>(schedules::WORK)
                .find_one(doc! {"_id": &legacy.id})
                .await
                .unwrap()
                .is_none()
        );

        // A completed migration replay must neither duplicate terminal audit
        // nor evaluate a stale generation or trim another history page.
        let replay = continuation_v2::migrate_checked(&state.db, drained, false)
            .await
            .unwrap();
        assert_eq!(
            replay.get_i64("classified").unwrap(),
            marker.get_i64("classified").unwrap()
        );
        assert_eq!(replay.get_i64("ambiguous_count").unwrap(), 1);
        schedules::finish(
            &state.db,
            &legacy.id,
            RunOutcome::Completed,
            None,
            &state.config.frontend_url,
        )
        .await
        .unwrap();
        assert_eq!(
            state
                .db
                .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(
                    doc! {"event_type": "trigger_run_outcome", "event_data.run_id": &legacy.id}
                )
                .await
                .unwrap(),
            1
        );
        let final_run = completion_load(&state, &legacy.id).await;
        assert_eq!(final_run.outcome, RunOutcome::Failed);
        assert_eq!(
            final_run.reason.as_deref(),
            Some("legacy_wait_unclassified")
        );
        assert_eq!(final_run.finished_at, failed.finished_at);
        assert_eq!(
            state
                .db
                .collection::<Document>(RUNS)
                .count_documents(doc! {"trigger_id": &trigger.id})
                .await
                .unwrap(),
            expected_history
        );
        assert!(calls.lock().await.is_empty());
        server.abort();
    }
}

#[test]
fn automation_v2_legacy_bson_and_unbound_event_guards() {
    let at = schedules::date(Utc::now());
    let legacy:TriggerRun=bson::from_document(doc!{"_id":"legacy","trigger_id":"trigger","user_id":"owner","scheduled_at":at,"deadline":at,"outcome":"waiting","fence":"legacy","lease_until":at,"expires_at":at}).unwrap();
    assert_eq!(legacy.continuation_protocol_version, 0);
    assert_eq!(legacy.continuation_count, 0);
    assert_eq!(legacy.continuation_limit, 0);
    assert!(legacy.wait_kind.is_none());
    let follower: crate::models::assistant_group::GroupFollower =
        bson::from_document(doc! {"conversation_id":"thread","since_seq":1_i64,"created_at":at})
            .unwrap();
    assert!(follower.trigger_run_id.is_none());
    assert!(follower.notification.is_none());
    let event: AgentEvent = bson::from_document(
        doc! {"id":"legacy-event","kind":"message","text":"legacy","created_at":at},
    )
    .unwrap();
    assert!(event.trigger_run_id.is_none());
    assert_eq!(event.continuation_protocol_version, 0);
    assert!(
        serde_json::from_value::<engine::TurnRequest>(
            json!({"text":"untrusted","trigger_run_id":"forged"})
        )
        .is_err()
    );
    let mut row:AssistantConversation=bson::from_document(doc!{"_id":"nyxa-fixture","user_id":"owner","title":"fixture","model":"fixture","credential_api_key_id":"fixture","message_count":0_i64,"active_turn":null,"created_at":at,"updated_at":at}).unwrap();
    let bound = team::bound_event("group_settled", "Run-bound result".into(), None, None)
        .for_trigger(Some("exact-run"));
    row.pending_events = vec![bound.clone(), event.clone()];
    let ordinary =
        crate::services::async_service_operation::select_events(&mut row, TurnOrigin::User);
    assert_eq!(ordinary.len(), 1);
    assert_eq!(ordinary[0].id, event.id);
    assert_eq!(row.pending_events[0].id, bound.id);
    assert!(
        crate::services::async_service_operation::select_events(&mut row, TurnOrigin::Event)
            .is_empty()
    );
    assert_eq!(row.pending_events.len(), 1);
}

#[tokio::test]
async fn automation_v2_follower_capacity_never_evicts_or_completes() {
    let (state, _, server) = setup("auto2040_capacity").await;
    let (_, run, row) = completion_coordinator(&state).await;
    let agent = team::agent_for_conversation(&state.db, &row).await.unwrap();
    let group = completion_groups::create(
        &state.db,
        OWNER,
        "Capacity 2040 fixture",
        std::slice::from_ref(&agent.id),
        "nyxbot",
    )
    .await
    .unwrap();
    let followers:Vec<_>=(0..crate::models::assistant_group::MAX_FOLLOWERS).map(|n|doc!{"conversation_id":&row.id,"trigger_run_id":format!("other-fixture-run-{n}"),"since_seq":n as i64,"created_at":schedules::date(Utc::now()),"expires_at":schedules::date(Utc::now()+Duration::hours(1))}).collect();
    state
        .db
        .collection::<Document>(crate::models::assistant_group::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&group.id},
            doc! {"$set":{"followers":&followers}},
        )
        .await
        .unwrap();
    let before = completion_groups::get(&state.db, OWNER, &group.id)
        .await
        .unwrap()
        .message_count;
    assert!(
        matches!(completion_groups::append_following(&state.db,OWNER,&group.id,"agent",Some(&agent),"capacity fixture",&[],Vec::new(),Some((&row.id,Some(&run.id)))).await,Err(AppError::Conflict(reason)) if reason=="group_follower_capacity")
    );
    let saved = completion_groups::get(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    assert_eq!(saved.message_count, before);
    assert_eq!(saved.followers.len(), followers.len());
    assert!(
        saved
            .followers
            .iter()
            .all(|f| f.trigger_run_id.as_deref() != Some(run.id.as_str()))
    );
    continuation_v2::defer_group_capacity(&state.db, OWNER, &row.id, &run.id)
        .await
        .unwrap();
    completion_reply(&state, &row, &run).await;
    let waiting = completion_load(&state, &run.id).await;
    assert_eq!(waiting.outcome, RunOutcome::Waiting);
    assert!(!waiting.terminal_result_recorded);
    continuation_v2::queue_capacity_retry(&state.db, &waiting)
        .await
        .unwrap();
    let queued = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(queued.pending_events.len(), 1);
    assert_eq!(
        queued.pending_events[0].trigger_run_id.as_deref(),
        Some(run.id.as_str())
    );
    server.abort();
}

/// Full local workflow: the fake model calls the real native group tool using
/// ChatAuthority re-derived from the live coordinator key. Member responses go
/// through the normal group settlement/sweep and trigger event wake paths.
#[tokio::test]
async fn automation_v2_fake_agent_research_edit_review_fixture_publish_readback() {
    use axum::routing::get;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn assert_stage_barrier(finishes: &[String], stage: usize) {
        assert_eq!(finishes.len(), [0, 2, 3, 4][stage]);
        if stage >= 1 {
            // Both research members must finish before editing can start;
            // their relative completion order is not a runtime contract.
            let mut research: Vec<_> = finishes[..2].iter().map(String::as_str).collect();
            research.sort_unstable();
            assert_eq!(research, ["researchfast", "researchslow"]);
        }
        if stage >= 2 {
            assert_eq!(finishes[2], "editor2040");
        }
        if stage >= 3 {
            assert_eq!(finishes[3], "reviewer2040");
        }
    }
    let (state, _, old_server) = setup("auto2040_workflow").await;
    old_server.abort();
    let nyxbot = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let mut member_ids = Vec::new();
    for name in ["researchslow", "researchfast", "editor2040", "reviewer2040"] {
        let agent = team::create_specialist(
            &state.db,
            &state.encryption_keys,
            OWNER,
            team::CreateRequest {
                machines: None,
                logins: None,
                name: name.into(),
                description: "Local automation acceptance fixture".into(),
                display_name: None,
                persona: None,
                targets: Default::default(),
                account_read: false,
                specialty: None,
                created_by: "user",
            },
        )
        .await
        .unwrap()
        .unwrap()
        .0;
        member_ids.push(agent.id);
    }
    let group = completion_groups::create(
        &state.db,
        OWNER,
        "Workflow 2040 fixture",
        &member_ids,
        "nyxbot",
    )
    .await
    .unwrap();
    let stages = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let finishes = Arc::new(Mutex::new(Vec::<String>::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fixture_state = state.clone();
    let fixture_stages = stages.clone();
    let fixture_finishes = finishes.clone();
    let fixture_group = group.id.clone();
    let publish_count = writes.clone();
    let read_count = writes.clone();
    let upstream=Router::new().route("/fixture/publish",post(move || {let count=publish_count.clone();async move {count.fetch_add(1,Ordering::SeqCst);Json(json!({"published":"automation-2040-local"}))}}))
        .route("/fixture/publish",get(move || {let count=read_count.clone();async move {Json(json!({"published":count.load(Ordering::SeqCst)}))}}))
        .route("/v1/responses",post(move |Json(body):Json<Value>| {
            let state=fixture_state.clone();let stages=fixture_stages.clone();let finishes=fixture_finishes.clone();let group=fixture_group.clone();
            async move {
                let input=body["input"].as_str().unwrap_or_default();
                let member=["researchslow","researchfast","editor2040","reviewer2040"].into_iter().find(|name|input.contains(&format!("You are {name} in the group chat")));
                let text=if let Some(member)=member {
                    let delay=if member=="researchslow" {350} else if member=="researchfast" {25} else {150};
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    finishes.lock().await.push(member.into());format!("{member} result ready for the next stage")
                } else {
                    let stage=stages.fetch_add(1,Ordering::SeqCst);
                    assert_stage_barrier(&finishes.lock().await,stage);
                    let run=state.db.collection::<TriggerRun>(RUNS).find_one(doc!{"user_id":OWNER,"outcome":"started"}).await.unwrap().unwrap();
                    let row=engine::get(&state.db,OWNER,run.thread_id.as_deref().unwrap()).await.unwrap();
                    assert_eq!(row.active_turn.as_ref().unwrap().trigger_run_id.as_deref(),Some(run.id.as_str()));
                    let chat=crate::services::assistant_acknowledgement_service::for_key(&state.db,OWNER,Some(&row.credential_api_key_id)).await.unwrap().unwrap();
                    assert_eq!(chat.trigger_run_id.as_deref(),Some(run.id.as_str()));
                    if stage<3 {
                        let text=match stage {0=>"@researchslow @researchfast research the local fixture",1=>"@editor2040 edit the researchers' result",_=>"@reviewer2040 independently review the edited result"};
                        let (result,error)=super::super::assistant_team::execute_tool(&state,&chat,"nyxid__post_to_group",&json!({"group":group,"text":text})).await;
                        assert!(!error,"group tool refused: {result}");assert!(result["posted"].is_i64());
                        "Delegated this stage; waiting for the members' result".to_owned()
                    } else {
                        assert_eq!(stage,3,"duplicate coordinator continuation");
                        assert_eq!(run.continuation_count,3);assert!(!run.terminal_result_recorded);
                        let published=state.http_client.post(format!("http://{address}/fixture/publish")).send().await.unwrap().json::<Value>().await.unwrap();
                        assert_eq!(published["published"],"automation-2040-local");
                        let readback=state.http_client.get(format!("http://{address}/fixture/publish")).send().await.unwrap().json::<Value>().await.unwrap();assert_eq!(readback["published"],1);
                        "Research, edit, independent review and local fixture publication/readback finished".to_owned()
                    }
                };
                let session=format!("conv_{}",Uuid::new_v4().simple());let response=format!("resp_{}_{}",&session[5..],Uuid::new_v4().simple());
                let stream=async_stream::stream!{yield Ok::<_,Infallible>(Event::default().data(json!({"type":"response.completed","sequence_number":1,"response":{"id":response,"conversation":{"id":session},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]}}).to_string()));};
                Sse::new(stream).into_response()
            }
        }));
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    let trigger = automation(&state, &nyxbot.id, None).await;
    make_due(&state, &trigger).await;
    Box::pin(tick(&state)).await.unwrap();
    let mut waiting_turns = std::collections::HashSet::new();
    let run = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let current = state
                .db
                .collection::<TriggerRun>(RUNS)
                .find_one(doc! {"trigger_id":&trigger.id})
                .await
                .unwrap()
                .unwrap();
            if current.outcome == RunOutcome::Waiting {
                assert_eq!(current.wait_kind, Some(RunWaitKind::Coordinator));
                assert!(!current.result_claimed);
                assert!(!current.terminal_result_recorded);
                waiting_turns.insert(current.turn_id.clone().unwrap());
            }
            if current.outcome == RunOutcome::Completed {
                break current;
            }
            assert_ne!(current.outcome, RunOutcome::Failed, "{:?}", current.reason);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(run.continuation_count, 3);
    assert_eq!(stages.load(Ordering::SeqCst), 4);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(waiting_turns.len(), 3, "every stage waits durably");
    assert_stage_barrier(&finishes.lock().await, 3);
    let history = engine::messages(
        &state.db,
        OWNER,
        run.thread_id.as_deref().unwrap(),
        100,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|m| m.role == "event" && m.origin == Some(TurnOrigin::Event))
            .count(),
        3
    );
    assert!(
        history
            .iter()
            .any(|m| m.text.contains("publication/readback finished"))
    );
    let audits: Vec<Document> = state
        .db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"user_id":OWNER,"event_type":"trigger_run_waiting"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        audits.len(),
        3,
        "each coordinator wait has a metadata audit"
    );
    for audit in audits {
        let encoded = serde_json::to_string(&audit).unwrap();
        for forbidden in [
            "research the local fixture",
            "First specialist result",
            "fence",
            "Bearer ",
        ] {
            assert!(!encoded.contains(forbidden));
        }
    }
    server.abort();
}
