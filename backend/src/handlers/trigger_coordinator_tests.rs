//! Regression cases use real MongoDB transactions and explicitly enroll each
//! v2 fixture. Existing scheduler tests continue exercising the shipping default.
use super::*;
use crate::models::assistant_conversation::TurnOrigin;
use crate::models::assistant_direct_handoff::COLLECTION_NAME as HANDOFFS;
use crate::services::trigger_schedule::{continuation as v2, direct, group as work_groups};
use futures::TryStreamExt;

fn start(thread: &str, text: &str) -> engine::TurnStart {
    engine::TurnStart::from(
        &serde_json::from_value::<engine::TurnRequest>(
            json!({"conversation_id": thread, "text": text, "model": null}),
        )
        .unwrap(),
    )
}

async fn root(
    name: &str,
    specialist_coordinator: bool,
) -> (
    AppState,
    Trigger,
    TriggerRun,
    AssistantConversation,
    tokio::task::JoinHandle<()>,
) {
    let (state, _, server) = setup(name).await;
    assert!(!state.config.automation_continuation_v2_enabled);
    let agent = if specialist_coordinator {
        specialist(&state).await
    } else {
        team::ensure_nyxbot(&state.db, OWNER).await.unwrap()
    };
    let trigger = if specialist_coordinator {
        trigger_service::create(
            &state.db,
            &state.encryption_keys,
            trigger_service::CreateInput {
                source: TriggerSource::Webhook,
                setup_watch_id: None,
                schedule: None,
                overlap: OverlapPolicy::Skip,
                user_id: OWNER.into(),
                label: "Specialist webhook coordinator".into(),
                user_service_id: None,
                verification: TriggerVerification::Token {
                    location: crate::models::trigger::TriggerTokenLocation::Bearer,
                },
                delivery: TriggerDelivery::Assistant {
                    confirmation_policy: Default::default(),
                    agent_id: agent.id.clone(),
                    thread_policy: None,
                    instruction: "Coordinate this request".into(),
                    deliver_to: DeliverTo::Thread,
                },
            },
        )
        .await
        .unwrap()
        .trigger
    } else {
        automation(&state, &agent.id, None).await
    };
    let run = schedules::enqueue_now(&state.db, &trigger).await.unwrap();
    let thread = schedules::run_thread(&state.db, &state.encryption_keys, &run, &trigger)
        .await
        .unwrap();
    let job = schedules::claim(&state.db, &run.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<Document>(USERS)
        .update_one(
            doc! {"_id": OWNER},
            doc! {"$set": {"execution_permission_enrollment": 1_i32}},
        )
        .await
        .unwrap();
    let mut request = start(&thread.id, "Coordinate this automation");
    request.execution_enrollment = true;
    request.origin = TurnOrigin::Trigger;
    request.trigger = Some(schedules::TurnClaim {
        enroll_v2: true,
        run_id: run.id.clone(),
        fence: job.get_str("fence").unwrap().into(),
        trigger_updated_at: trigger.updated_at,
        continuation: false,
    });
    let row = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap();
    let run = state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.coordinator.as_ref().unwrap().protocol, 2);
    assert!(row.active_turn.as_ref().unwrap().execution_task.is_none());
    assert_eq!(row.automation_thread, specialist_coordinator);
    let chat = crate::services::assistant_acknowledgement_service::for_key(
        &state.db,
        OWNER,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(chat.execution_unsupported);
    (state, trigger, run, row, server)
}

async fn save_reply(
    state: &AppState,
    row: &AssistantConversation,
    text: &str,
    error: Option<&str>,
) {
    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        &state.db,
        &state.encryption_keys,
        OWNER,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    engine::finish_turn(
        &state.db,
        row,
        &credential.api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: text.into(),
            session_id: None,
            response_id: None,
            error: error.map(engine::TurnError::new),
        },
    )
    .await
    .unwrap();
}

async fn child(state: &AppState, parent: &AssistantConversation) -> AssistantConversation {
    let agent = specialist(state).await;
    let thread = team::home_thread(&state.db, &state.encryption_keys, &agent)
        .await
        .unwrap();
    let mut request = start(&thread.id, "Investigate");
    request.execution_enrollment = true;
    request.origin = TurnOrigin::Orchestrator;
    request.report_to = Some(parent.id.clone());
    let turn = parent.active_turn.as_ref().unwrap();
    request.automation_parent = Some(direct::Parent {
        thread: parent.id.clone(),
        turn: turn.turn_id.clone(),
        work: turn.automation_work.clone().unwrap(),
    });
    engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap()
}

#[tokio::test]
async fn v2_barrier_recovers_recorded_terminal_result_and_next_occurrence() {
    let (state, trigger, run, row, server) = root("v2_terminal_recovery", false).await;
    save_reply(&state, &row, "Final result", None).await;
    schedules::terminal_result(&state.db, &run, None, &state.config.frontend_url)
        .await
        .unwrap();
    // Simulate the worker dying after the durable terminal projection.
    let recorded = state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    assert!(recorded.terminal_result_recorded);
    assert_eq!(
        v2::barrier(&state.db, &recorded).await.unwrap(),
        v2::Barrier::ContinueTerminal
    );
    settle_result(&state, &row, &recorded, "Final result", None)
        .await
        .unwrap();
    settle_result(&state, &row, &recorded, "Final result", None)
        .await
        .unwrap();
    let finished = state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.outcome, RunOutcome::Completed);
    assert_eq!(finished.reason.as_deref(), Some("coordinator_completed"));
    let next = schedules::enqueue_now(&state.db, &trigger).await.unwrap();
    assert_ne!(next.id, run.id);
    let job = schedules::claim(&state.db, &next.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    run_job_once(&state, &job).await.unwrap();
    let next_finished = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if let Some(run) = state
                .db
                .collection::<TriggerRun>(RUNS)
                .find_one(doc! {"_id": &next.id, "outcome": "completed"})
                .await
                .unwrap()
            {
                break run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(next_finished.id, next.id);
    assert!(next_finished.started_at.is_some());
    server.abort();
}

#[tokio::test]
async fn v2_direct_wait_survives_owner_turn_kill_switch_and_lost_callback() {
    let (state, _, run, row, server) = root("v2_direct_recovery", false).await;
    let child = child(&state, &row).await;
    assert!(child.active_turn.as_ref().unwrap().trigger_run_id.is_none());
    assert!(child.active_turn.as_ref().unwrap().execution_task.is_none());
    save_reply(&state, &row, "Waiting for research", None).await;
    let owner = engine::begin_turn(
        &state.db,
        OWNER,
        &start(&row.id, "An unrelated owner question"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(
        v2::barrier(&state.db, &run).await.unwrap(),
        v2::Barrier::Waiting
    );
    let still_owner = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(
        still_owner.active_turn.as_ref().unwrap().turn_id,
        owner.active_turn.as_ref().unwrap().turn_id
    );
    save_reply(&state, &child, "Research result", None).await;
    // Local flag was off throughout: persisted enrollment alone authorizes drain.
    direct::recover(&state.db, &run).await.unwrap();
    direct::recover(&state.db, &run).await.unwrap();
    let thread = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(
        thread
            .pending_events
            .iter()
            .filter(|e| e.kind == "subagent_settled")
            .count(),
        1
    );
    assert!(thread.pending_events[0].automation_work.is_some());
    let note = &thread.pending_events[0].text;
    assert!(note.contains("Specialist schedule-researcher replied"));
    assert!(note.contains(child.agent_id.as_deref().unwrap()));
    assert!(note.contains("nyxid__read_subagent"));
    server.abort();
}

#[tokio::test]
async fn v2_cancel_is_exact_idempotent_and_idle_stop_keeps_waiting_run() {
    let (state, trigger, run, row, server) = root("v2_cancel", false).await;
    let _child = child(&state, &row).await;
    save_reply(&state, &row, "Waiting", None).await;
    assert_eq!(
        v2::barrier(&state.db, &run).await.unwrap(),
        v2::Barrier::Waiting
    );
    assert!(matches!(
        engine::request_stop_target(&state.db, OWNER, &row.id)
            .await
            .unwrap(),
        engine::StopTarget::Scoped(None)
    ));
    assert_eq!(
        state
            .db
            .collection::<TriggerRun>(RUNS)
            .find_one(doc! {"_id": &run.id})
            .await
            .unwrap()
            .unwrap()
            .outcome,
        RunOutcome::Waiting
    );
    assert!(
        v2::cancel(
            &state.db,
            "another-owner",
            &trigger.id,
            &run.id,
            "cancelled",
            ""
        )
        .await
        .is_err()
    );
    assert!(
        v2::cancel(
            &state.db,
            OWNER,
            "another-trigger",
            &run.id,
            "cancelled",
            ""
        )
        .await
        .is_err()
    );
    for _ in 0..2 {
        let cancelled = v2::cancel(&state.db, OWNER, &trigger.id, &run.id, "cancelled", "")
            .await
            .unwrap();
        assert_eq!(cancelled.outcome, RunOutcome::Failed);
        assert_eq!(cancelled.reason.as_deref(), Some("cancelled"));
    }
    assert_eq!(
        state
            .db
            .collection::<Document>(HANDOFFS)
            .count_documents(doc! {"run_id": &run.id, "state": "pending"})
            .await
            .unwrap(),
        0
    );
    let audits: Vec<Document> = state
        .db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type": "trigger_run_cancelled"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(audits.len(), 1);
    for audit in &audits {
        assert!(
            audit.contains_key("seq")
                && audit.contains_key("prev_hash")
                && audit.contains_key("entry_hash")
        );
    }
    server.abort();
}

#[tokio::test]
async fn v2_child_deletion_queues_failure_and_cleans_both_roles_without_marker() {
    let (state, _, run, row, server) = root("v2_delete", false).await;
    let child = child(&state, &row).await;
    let receipts = state.db.collection::<Document>(HANDOFFS);
    let receipt = receipts
        .find_one(doc! {"run_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    let mut terminal_child = receipt.clone();
    terminal_child.insert("_id", Uuid::new_v4().to_string());
    terminal_child.insert("state", "delivered");
    receipts.insert_one(terminal_child).await.unwrap();
    let mut foreign = receipt.clone();
    foreign.insert("_id", Uuid::new_v4().to_string());
    foreign.insert("user_id", "another-owner");
    receipts.insert_one(foreign).await.unwrap();
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &child.id},
            doc! {"$unset": {"automation_v2": ""}},
        )
        .await
        .unwrap();
    engine::delete(&state.db, OWNER, &child.id).await.unwrap();
    let parent = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert!(parent.pending_events.iter().any(|e| {
        e.text.contains("thread_deleted")
            && e.automation_work
                .as_ref()
                .is_some_and(|w| w.run_id == run.id)
    }));
    assert_eq!(
        state
            .db
            .collection::<Document>(HANDOFFS)
            .count_documents(doc! {"user_id": OWNER, "child_thread_id": &child.id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        receipts
            .count_documents(doc! {"user_id": "another-owner", "child_thread_id": &child.id})
            .await
            .unwrap(),
        1
    );
    for status in ["pending", "delivered"] {
        let mut parent_receipt = receipt.clone();
        parent_receipt.insert("_id", Uuid::new_v4().to_string());
        parent_receipt.insert("child_thread_id", Uuid::new_v4().to_string());
        parent_receipt.insert("state", status);
        receipts.insert_one(parent_receipt).await.unwrap();
    }
    engine::delete(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<Document>(HANDOFFS)
            .count_documents(doc! {"user_id": OWNER, "parent_thread_id": &row.id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        receipts
            .count_documents(doc! {"user_id": "another-owner", "parent_thread_id": &row.id})
            .await
            .unwrap(),
        1
    );
    server.abort();
}

#[tokio::test]
async fn v2_continuation_does_not_debit_exhausted_occurrence_budget() {
    let (state, trigger, run, row, server) = root("v2_budget", false).await;
    let child = child(&state, &row).await;
    save_reply(&state, &row, "Waiting", None).await;
    v2::barrier(&state.db, &run).await.unwrap();
    save_reply(&state, &child, "Done", None).await;
    direct::recover(&state.db, &run).await.unwrap();
    let budget = doc! {"_id": OWNER, "hour": Utc::now().timestamp().div_euclid(3600), "day": Utc::now().timestamp().div_euclid(86400), "hourly": 30, "daily": 300};
    state
        .db
        .collection::<Document>(schedules::BUDGETS)
        .replace_one(doc! {"_id": OWNER}, &budget)
        .await
        .unwrap();
    state.db.collection::<Document>(schedules::WORK).update_one(doc! {"_id": &run.id}, doc! {"$set": {"at": schedules::date(Utc::now())}, "$unset": {"admission_lease_until": ""}}).await.unwrap();
    let job = schedules::claim(&state.db, &run.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    let mut request = start(&row.id, "Continue");
    request.origin = TurnOrigin::Trigger;
    request.trigger = Some(schedules::TurnClaim {
        enroll_v2: false,
        run_id: run.id.clone(),
        fence: job.get_str("fence").unwrap().into(),
        trigger_updated_at: trigger.updated_at,
        continuation: true,
    });
    let resumed = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap();
    assert_eq!(resumed.active_turn.as_ref().unwrap().events.len(), 1);
    assert_eq!(
        state
            .db
            .collection::<Document>(schedules::BUDGETS)
            .find_one(doc! {"_id": OWNER})
            .await
            .unwrap()
            .unwrap(),
        budget
    );
    let live = state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.coordinator.unwrap().continuations, 1);
    server.abort();
}

#[tokio::test]
async fn v2_wake_cannot_shorten_admission_lease_or_be_overwritten_by_deferral() {
    let (state, _, run, _, server) = root("v2_wake_fence", false).await;
    state
        .db
        .collection::<Document>(schedules::WORK)
        .update_one(
            doc! {"_id": &run.id},
            doc! {"$set": {"at": schedules::date(Utc::now())}},
        )
        .await
        .unwrap();
    let job = schedules::claim(&state.db, &run.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    // Preparation may consume the original lease. Reserving admission must
    // renew both the durable fence time and the event wake protection.
    state.db.collection::<Document>(schedules::WORK).update_one(
        doc! {"_id": &run.id},
        doc! {"$set": {"admission_lease_until": schedules::date(Utc::now() - Duration::seconds(1))}},
    ).await.unwrap();
    v2::reserve_admission(&state.db, &job).await.unwrap();
    let reserved = state
        .db
        .collection::<Document>(schedules::WORK)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    let lease = *reserved.get_datetime("at").unwrap();
    assert_eq!(
        reserved.get_datetime("admission_lease_until").unwrap(),
        &lease
    );
    assert!(lease.to_chrono() > Utc::now() + Duration::seconds(100));
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    v2::wake_in_session(&state.db, &run.id, &mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    assert_eq!(
        *state
            .db
            .collection::<Document>(schedules::WORK)
            .find_one(doc! {"_id": &run.id})
            .await
            .unwrap()
            .unwrap()
            .get_datetime("at")
            .unwrap(),
        lease
    );
    schedules::reschedule(&state.db, &job, Some(Utc::now() + Duration::minutes(5)))
        .await
        .unwrap();
    let work = state
        .db
        .collection::<Document>(schedules::WORK)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    assert!(work.get_datetime("at").unwrap().to_chrono() <= Utc::now());
    assert_eq!(v2::backstop(0).num_seconds(), 60);
    assert_eq!(v2::backstop(20).num_seconds(), 300);
    server.abort();
}

async fn named_specialist(
    state: &AppState,
    name: &str,
) -> crate::models::assistant_agent::AssistantAgent {
    team::create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: name.into(),
            description: "Bounded group worker".into(),
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
    .0
}

fn authority(
    row: &AssistantConversation,
) -> crate::services::assistant_acknowledgement_service::ChatAuthority {
    let turn = row.active_turn.as_ref().unwrap();
    crate::services::assistant_acknowledgement_service::ChatAuthority {
        automation_work: turn.automation_work.clone(),
        execution_group_id: None,
        execution_unsupported: true,
        execution_task: None,
        execution_permission: None,
        business_system_success: false,
        collaboration_thread: false,
        channel_adapter_origin: None,
        invocation_controlled: false,
        invocation_id: None,
        platform_thread: false,
        platform_origin: None,
        org_agent_access: None,
        turn_id: Some(turn.turn_id.clone()),
        turn_stopped: false,
        turn_live: true,
        machine_node_ids: Vec::new(),
        saved_login_ids: Vec::new(),
        conversation_id: row.id.clone(),
        user_id: OWNER.into(),
        api_key_id: row.credential_api_key_id.clone(),
        role: row.role,
        agent_id: row.agent_id.clone().unwrap(),
        agent_name: "coordinator".into(),
        guest: false,
        confirmation_policy: turn
            .automation_work
            .as_ref()
            .and_then(|w| w.confirmation_policy),
        report_to: None,
        with_nyxbot: false,
    }
}

async fn group_turn(
    state: &AppState,
    group: &crate::models::assistant_group::AssistantGroup,
    agent: &str,
) -> AssistantConversation {
    let work = work_groups::queued(group)
        .into_iter()
        .find(|(_, id)| id == agent)
        .unwrap()
        .0;
    let mut request = start("", "Address this tracked group request");
    request.conversation_id = None;
    request.agent_id = Some(agent.into());
    request.origin = TurnOrigin::Group;
    request.group_id = Some(group.id.clone());
    request.automation_work = Some(work);
    engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap()
}

#[tokio::test]
async fn v2_specialist_webhook_editor_mentions_reviewer_and_nyxbot_then_resumes_coordinator() {
    let (state, trigger, run, row, server) = root("v2_group_mentions", true).await;
    assert!(row.automation_thread);
    assert!(run.confirmation_policy.is_some());
    let editor = named_specialist(&state, "editor").await;
    let reviewer = named_specialist(&state, "nutrition-reviewer").await;
    let nyxbot = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let groups = crate::services::assistant_group_service::create(
        &state.db,
        OWNER,
        "Meal review",
        &[editor.id.clone(), reviewer.id.clone(), nyxbot.id.clone()],
        "user",
    )
    .await
    .unwrap();
    assert!(
        work_groups::post(&state.db, &authority(&row), &groups, "Nobody is addressed")
            .await
            .is_err()
    );
    work_groups::post(
        &state.db,
        &authority(&row),
        &groups,
        "@editor draft a meal plan",
    )
    .await
    .unwrap();
    save_reply(&state, &row, "Waiting for the crew", None).await;
    assert_eq!(
        v2::barrier(&state.db, &run).await.unwrap(),
        v2::Barrier::Waiting
    );
    let group = crate::services::assistant_group_service::get(&state.db, OWNER, &groups.id)
        .await
        .unwrap();
    let editor_turn = group_turn(&state, &group, &editor.id).await;
    assert_eq!(
        editor_turn
            .active_turn
            .as_ref()
            .unwrap()
            .automation_work
            .as_ref()
            .unwrap()
            .confirmation_policy,
        run.confirmation_policy
    );
    save_reply(
        &state,
        &editor_turn,
        "Draft prepared. @nutrition-reviewer check nutrition; @NyxBot review the result.",
        None,
    )
    .await;
    work_groups::recover(&state.db, &group).await.unwrap();
    let group = crate::services::assistant_group_service::get(&state.db, OWNER, &groups.id)
        .await
        .unwrap();
    let queued = work_groups::queued(&group);
    assert_eq!(queued.len(), 2);
    assert!(queued.iter().any(|(_, id)| id == &reviewer.id));
    assert!(queued.iter().any(|(_, id)| id == &nyxbot.id));
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    for agent in [&reviewer.id, &nyxbot.id] {
        let current = crate::services::assistant_group_service::get(&state.db, OWNER, &groups.id)
            .await
            .unwrap();
        let member = group_turn(&state, &current, agent).await;
        save_reply(&state, &member, "Review finished", None).await;
        work_groups::recover(&state.db, &current).await.unwrap();
    }
    let group = crate::services::assistant_group_service::get(&state.db, OWNER, &groups.id)
        .await
        .unwrap();
    assert!(group.followers.is_empty());
    work_groups::recover(&state.db, &group).await.unwrap();
    let coordinator = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(
        coordinator
            .pending_events
            .iter()
            .filter(|e| e.kind == "group_settled")
            .count(),
        1
    );
    state
        .db
        .collection::<Document>(schedules::WORK)
        .update_one(
            doc! {"_id": &run.id},
            doc! {"$set": {"at": schedules::date(Utc::now())}},
        )
        .await
        .unwrap();
    tick(&state).await.unwrap();
    let finished = completed(&state, &trigger).await;
    assert_eq!(finished.outcome, RunOutcome::Completed);
    assert_eq!(finished.thread_id, Some(row.id));
    assert_eq!(finished.coordinator.unwrap().continuations, 1);
    server.abort();
}

#[tokio::test]
async fn v2_deadline_and_dispatch_limits_are_fixed_and_explicit() {
    let (state, trigger, run, row, server) = root("v2_limits", false).await;
    let deadline = run.coordinator.as_ref().unwrap().deadline;
    let job = state
        .db
        .collection::<Document>(schedules::WORK)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    let backstop = job.get_datetime("at").unwrap().to_chrono();
    assert!(backstop <= Utc::now() + Duration::seconds(60));
    assert!(backstop > Utc::now());
    assert_eq!(
        (deadline - run.started_at.unwrap()).num_seconds(),
        v2::COORDINATOR_DEADLINE_SECONDS
    );
    state
        .db
        .collection::<Document>(RUNS)
        .update_one(
            doc! {"_id": &run.id},
            doc! {"$set": {"coordinator.dispatches": 32}},
        )
        .await
        .unwrap();
    let agent = specialist(&state).await;
    let thread = team::home_thread(&state.db, &state.encryption_keys, &agent)
        .await
        .unwrap();
    let mut request = start(&thread.id, "Would exceed the bound");
    request.origin = TurnOrigin::Orchestrator;
    let turn = row.active_turn.as_ref().unwrap();
    request.automation_parent = Some(direct::Parent {
        thread: row.id.clone(),
        turn: turn.turn_id.clone(),
        work: turn.automation_work.clone().unwrap(),
    });
    assert!(
        engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
            .await
            .is_err()
    );
    assert!(
        engine::get(&state.db, OWNER, &thread.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(HANDOFFS)
            .count_documents(doc! {"run_id": &run.id})
            .await
            .unwrap(),
        0
    );
    state.db.collection::<Document>(RUNS).update_one(doc! {"_id": &run.id}, doc! {"$set": {"coordinator.deadline": schedules::date(Utc::now() - Duration::seconds(1))}}).await.unwrap();
    state
        .db
        .collection::<Document>(schedules::WORK)
        .update_one(
            doc! {"_id": &run.id},
            doc! {"$set": {"at": schedules::date(Utc::now())}},
        )
        .await
        .unwrap();
    tick(&state).await.unwrap();
    let expired = completed(&state, &trigger).await;
    assert_eq!(expired.reason.as_deref(), Some("coordinator_timeout"));
    server.abort();
}

#[tokio::test]
async fn v2_result_selection_preserves_ordinary_events_and_one_async_result() {
    let (state, _, run, row, server) = root("v2_event_partition", false).await;
    let child = child(&state, &row).await;
    let work = child
        .active_turn
        .as_ref()
        .unwrap()
        .automation_work
        .clone()
        .unwrap();
    let mut one = team::event(
        crate::services::async_service_operation::EVENT_KIND,
        "first".into(),
        None,
    );
    one.automation_work = Some(work.clone());
    let mut two = team::event(
        crate::services::async_service_operation::EVENT_KIND,
        "second".into(),
        None,
    );
    two.automation_work = Some(work);
    let ordinary = team::event("ordinary", "Owner notice".into(), None);
    let mut root_result = team::event("group_settled", "Coordinator result".into(), None);
    root_result.automation_work = Some(v2::context(&run, &row.id));
    let mut snapshot = child.clone();
    snapshot.pending_events = vec![ordinary, root_result, one, two];
    let selected =
        crate::services::async_service_operation::select_events(&mut snapshot, TurnOrigin::Event);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].text, "first");
    assert_eq!(snapshot.pending_events.len(), 3);
    let ordinary =
        crate::services::async_service_operation::select_events(&mut snapshot, TurnOrigin::User);
    assert_eq!(ordinary.len(), 1);
    assert_eq!(ordinary[0].text, "Owner notice");
    assert_eq!(snapshot.pending_events.len(), 2);
    server.abort();
}

#[tokio::test]
async fn v2_scheduler_enrolls_only_ready_configured_first_admissions() {
    use crate::services::security_readiness_service as readiness;
    let (mut state, _, server) = setup("v2_enrollment").await;
    state.config.automation_continuation_v2_enabled = true;
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    // Configured but unready is successful legacy admission, not an outage.
    let trigger = automation(&state, &agent.id, None).await;
    make_due(&state, &trigger).await;
    tick(&state).await.unwrap();
    assert!(completed(&state, &trigger).await.coordinator.is_none());
    readiness::ready_automation_test_replica(&state.db).await;
    assert!(v2::enrollment_ready(&state.db, true).await);
    assert!(!v2::enrollment_ready(&state.db, false).await);
    let ready_trigger = automation(&state, &agent.id, Some(ThreadPolicy::New)).await;
    make_due(&state, &ready_trigger).await;
    tick(&state).await.unwrap();
    assert_eq!(
        completed(&state, &ready_trigger)
            .await
            .coordinator
            .unwrap()
            .protocol,
        2
    );
    let specialist = named_specialist(&state, "webhook-coordinator").await;
    let webhook = trigger_service::create(
        &state.db,
        &state.encryption_keys,
        trigger_service::CreateInput {
            source: TriggerSource::Webhook,
            setup_watch_id: None,
            schedule: None,
            overlap: OverlapPolicy::Skip,
            user_id: OWNER.into(),
            label: "Webhook run now".into(),
            user_service_id: None,
            verification: TriggerVerification::Token {
                location: crate::models::trigger::TriggerTokenLocation::Bearer,
            },
            delivery: TriggerDelivery::Assistant {
                confirmation_policy: Default::default(),
                agent_id: specialist.id.clone(),
                thread_policy: None,
                instruction: "Coordinate webhook work".into(),
                deliver_to: DeliverTo::Thread,
            },
        },
    )
    .await
    .unwrap()
    .trigger;
    schedules::enqueue_now(&state.db, &webhook).await.unwrap();
    tick(&state).await.unwrap();
    let webhook_run = completed(&state, &webhook).await;
    assert_eq!(webhook_run.coordinator.as_ref().unwrap().protocol, 2);
    let webhook_thread = engine::get(&state.db, OWNER, webhook_run.thread_id.as_deref().unwrap())
        .await
        .unwrap();
    assert!(webhook_thread.automation_thread);
    assert_eq!(
        webhook_thread.agent_id.as_deref(),
        Some(specialist.id.as_str())
    );
    let persisted = state
        .db
        .collection::<Document>(TRIGGERS)
        .find_one(doc! {"_id": &ready_trigger.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!persisted.contains_key("continuation_protocol_version"));
    let off = AppState {
        config: crate::config::AppConfig {
            automation_continuation_v2_enabled: false,
            ..state.config.clone()
        },
        ..state.clone()
    };
    let legacy = automation(&off, &agent.id, None).await;
    make_due(&off, &legacy).await;
    tick(&off).await.unwrap();
    assert!(completed(&off, &legacy).await.coordinator.is_none());
    server.abort();
}

#[tokio::test]
async fn v2_child_async_and_card_sources_prevent_premature_publication() {
    let (state, _, run, row, server) = root("v2_child_sources", false).await;
    let child = child(&state, &row).await;
    let work = child
        .active_turn
        .as_ref()
        .unwrap()
        .automation_work
        .clone()
        .unwrap();
    save_reply(
        &state,
        &child,
        "Waiting for external result and approval",
        None,
    )
    .await;
    let work_bson = mongodb::bson::to_bson(&work).unwrap();
    let cards = state
        .db
        .collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME);
    cards.insert_one(doc! {"_id": "pending-card", "user_id": OWNER, "automation_work": &work_bson, "status": "pending", "expires_at": schedules::date(Utc::now() + Duration::minutes(10))}).await.unwrap();
    let watches = state
        .db
        .collection::<Document>(crate::models::async_service_operation::COLLECTION_NAME);
    watches.insert_one(doc! {"_id": "pending-watch", "user_id": OWNER, "delivery": {"automation_work": work_bson}, "state": "waiting"}).await.unwrap();
    direct::recover(&state.db, &run).await.unwrap();
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    cards
        .delete_one(doc! {"_id": "pending-card"})
        .await
        .unwrap();
    direct::recover(&state.db, &run).await.unwrap();
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    watches
        .delete_one(doc! {"_id": "pending-watch"})
        .await
        .unwrap();
    direct::recover(&state.db, &run).await.unwrap();
    assert_eq!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .len(),
        1
    );
    server.abort();
}

#[tokio::test]
async fn v2_direct_continuation_keeps_assignment_identity_and_latest_source_turn() {
    let (state, _, run, row, server) = root("v2_image_lineage", false).await;
    let child = child(&state, &row).await;
    let initial = child.active_turn.as_ref().unwrap();
    let assignment = initial.delegated_images.assignment.clone().unwrap();
    let mut event = team::event("permission_decided", "Allowed; continue".into(), None);
    event.automation_work = initial.automation_work.clone();
    engine::push_events(&state.db, OWNER, &child.id, vec![event])
        .await
        .unwrap();
    save_reply(&state, &child, "Waiting for decision", None).await;
    direct::recover(&state.db, &run).await.unwrap();
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    let mut request = start(&child.id, "Continue");
    request.origin = TurnOrigin::Event;
    let resumed = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
        .await
        .unwrap();
    let latest = resumed.active_turn.as_ref().unwrap();
    let edge = latest.delegated_images.assignment.as_ref().unwrap();
    assert_eq!(edge.id, assignment.id);
    assert_eq!(edge.source_turn, latest.turn_id);
    assert_eq!(edge.destinations, assignment.destinations);
    assert_eq!(
        edge.recipient_conversation,
        assignment.recipient_conversation
    );
    save_reply(&state, &resumed, "Final answer", None).await;
    direct::recover(&state.db, &run).await.unwrap();
    let parent = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(parent.pending_events.len(), 1);
    assert_eq!(parent.pending_events[0].id, assignment.id);
    assert_eq!(
        parent.pending_events[0]
            .delegated_result
            .as_ref()
            .unwrap()
            .source_turn,
        latest.turn_id
    );
    server.abort();
}

#[tokio::test]
async fn v2_full_result_queue_keeps_receipt_pending_for_retry() {
    let (state, _, run, row, server) = root("v2_queue_retry", false).await;
    let child = child(&state, &row).await;
    save_reply(&state, &child, "Research finished", None).await;
    let full: Vec<_> = (0..engine::MAX_PENDING_EVENTS)
        .map(|i| team::event("ordinary", format!("Notice {i}"), None))
        .collect();
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"pending_events": mongodb::bson::to_bson(&full).unwrap()}},
        )
        .await
        .unwrap();
    assert!(direct::recover(&state.db, &run).await.is_err());
    assert_eq!(
        state
            .db
            .collection::<Document>(HANDOFFS)
            .count_documents(doc! {"run_id": &run.id, "state": "pending"})
            .await
            .unwrap(),
        1
    );
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"pending_events": []}})
        .await
        .unwrap();
    direct::recover(&state.db, &run).await.unwrap();
    assert_eq!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .len(),
        1
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(HANDOFFS)
            .count_documents(doc! {"run_id": &run.id, "state": "pending"})
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
async fn v2_mixed_group_follower_survives_legacy_post_and_deletion_delivers_failure() {
    let (state, _, run, row, server) = root("v2_group_delete", false).await;
    let editor = named_specialist(&state, "editor").await;
    let group = crate::services::assistant_group_service::create(
        &state.db,
        OWNER,
        "Mixed crew",
        &[editor.id],
        "user",
    )
    .await
    .unwrap();
    work_groups::post(&state.db, &authority(&row), &group, "@editor investigate")
        .await
        .unwrap();
    crate::services::assistant_group_service::append_following(
        &state.db,
        OWNER,
        &group.id,
        "user",
        None,
        "An unrelated request",
        &[],
        Vec::new(),
        Some("ordinary-thread"),
    )
    .await
    .unwrap();
    let current = crate::services::assistant_group_service::get(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    assert_eq!(current.followers.len(), 2);
    assert_eq!(
        current
            .followers
            .iter()
            .filter(|f| f.automation.is_some())
            .count(),
        1
    );
    let (legacy, _) = crate::services::assistant_group_service::take_followers_if_quiet(
        &state.db, OWNER, &group.id,
    )
    .await
    .unwrap();
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].conversation_id, "ordinary-thread");
    let current = crate::services::assistant_group_service::get(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    assert_eq!(current.followers.len(), 1);
    assert!(current.followers[0].automation.is_some());
    crate::services::assistant_group_service::delete(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    let parent = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(parent.pending_events.len(), 1);
    assert!(parent.pending_events[0].text.contains("group_deleted"));
    assert_eq!(
        parent.pending_events[0]
            .automation_work
            .as_ref()
            .unwrap()
            .run_id,
        run.id
    );
    server.abort();
}

#[tokio::test]
async fn v2_child_decision_is_durable_without_handler_callback_and_keeps_lineage() {
    use crate::models::assistant_acknowledgement::{
        AssistantAcknowledgement, COLLECTION_NAME as ACKS,
    };
    for allow in [true, false] {
        let (state, _, run, row, server) = root("v2_durable_decision", false).await;
        let child = child(&state, &row).await;
        let turn = child.active_turn.as_ref().unwrap();
        let work = turn.automation_work.as_ref().unwrap();
        let id = Uuid::new_v4().to_string();
        state.db.collection::<Document>(ACKS).insert_one(doc! {
        "_id": &id, "user_id": OWNER, "conversation_id": &child.id,
        "api_key_id": &child.credential_api_key_id, "kind": "action",
        "tool_name": "approved_call", "summary": "Approved action", "status": "pending",
        "requested_turn_id": &turn.turn_id, "automation_work": mongodb::bson::to_bson(work).unwrap(),
        "created_at": schedules::date(Utc::now()),
        "expires_at": schedules::date(Utc::now() + Duration::minutes(10)), "decider": "user",
    }).await.unwrap();
        save_reply(&state, &child, "Please review", None).await;
        crate::services::assistant_acknowledgement_service::decide(
            &state.db, OWNER, &child.id, &id, allow,
        )
        .await
        .unwrap();
        // No HTTP/handler callback ran: the decision and bound event committed together.
        let queued = engine::get(&state.db, OWNER, &child.id).await.unwrap();
        assert_eq!(queued.pending_events.len(), 1);
        assert_eq!(
            queued.pending_events[0].id,
            format!("automation-decision:{id}")
        );
        assert_eq!(
            queued.pending_events[0].automation_work.as_ref(),
            Some(work)
        );
        assert!(v2::event_can_resume(&state.db, &queued).await.unwrap());
        let mut forged = queued.clone();
        forged.pending_events[0]
            .automation_work
            .as_mut()
            .unwrap()
            .work_id = "unclaimed".into();
        assert!(!v2::event_can_resume(&state.db, &forged).await.unwrap());
        forged = queued.clone();
        forged.pending_events[0]
            .automation_work
            .as_mut()
            .unwrap()
            .coordinator_thread_id = "forged-parent".into();
        assert!(!v2::event_can_resume(&state.db, &forged).await.unwrap());
        direct::recover(&state.db, &run).await.unwrap();
        assert!(
            engine::get(&state.db, OWNER, &row.id)
                .await
                .unwrap()
                .pending_events
                .is_empty()
        );
        let mut request = start(&child.id, "Continue approved work");
        request.origin = TurnOrigin::Event;
        let resumed = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
            .await
            .unwrap();
        let latest = resumed.active_turn.as_ref().unwrap();
        assert_eq!(latest.automation_work.as_ref(), Some(work));
        assert!(latest.trigger_run_id.is_none());
        assert!(latest.execution_task.is_none());
        if allow {
            state
                .db
                .collection::<AssistantAcknowledgement>(ACKS)
                .update_one(doc! {"_id": &id}, doc! {"$set": {"status": "used"}})
                .await
                .unwrap();
        }
        save_reply(&state, &resumed, "Approved action finished", None).await;
        direct::recover(&state.db, &run).await.unwrap();
        assert_eq!(
            engine::get(&state.db, OWNER, &row.id)
                .await
                .unwrap()
                .pending_events
                .len(),
            1
        );
        let result = engine::get(&state.db, OWNER, &row.id).await.unwrap();
        if !allow {
            assert!(
                result.pending_events[0]
                    .text
                    .contains("confirmation_denied")
            );
        }
        server.abort();
    }
}

#[tokio::test]
async fn v2_stop_root_targets_descendants_but_stale_stop_cannot_select_waiting_work() {
    let (state, _, run, row, server) = root("v2_stop_descendants", false).await;
    let descendant = child(&state, &row).await;
    assert!(
        v2::stopped_targets(&state.db, &row)
            .await
            .unwrap()
            .is_empty()
    );
    let engine::StopTarget::Scoped(Some(stopped)) =
        engine::request_stop_target(&state.db, OWNER, &row.id)
            .await
            .unwrap()
    else {
        panic!("A successful exact Stop must return its transport target");
    };
    assert!(stopped.active_turn.as_ref().unwrap().stop_requested);
    let targets = v2::stopped_targets(&state.db, &row).await.unwrap();
    assert!(targets.iter().any(|target| target.id == descendant.id));
    assert!(
        targets
            .iter()
            .all(|target| target.active_turn.as_ref().unwrap().stop_requested)
    );
    let cancelled = state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.reason.as_deref(), Some("cancelled"));
    server.abort();

    let (state, _, run, row, server) = root("v2_stale_stop", false).await;
    let _child = child(&state, &row).await;
    save_reply(&state, &row, "Waiting", None).await;
    assert_eq!(
        v2::barrier(&state.db, &run).await.unwrap(),
        v2::Barrier::Waiting
    );
    assert!(
        engine::request_stop_row_target(&state.db, &row)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        v2::stopped_targets(&state.db, &row)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        state
            .db
            .collection::<TriggerRun>(RUNS)
            .find_one(doc! {"_id": &run.id})
            .await
            .unwrap()
            .unwrap()
            .outcome,
        RunOutcome::Waiting
    );
    server.abort();
}

#[tokio::test]
async fn v2_cancel_rejects_nonhuman_credentials_before_mutation() {
    use crate::mw::auth::AuthMethod;
    let (state, trigger, run, _, server) = root("v2_cancel_auth", false).await;
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::Delegated,
        AuthMethod::ServiceAccount,
        AuthMethod::Relay,
    ] {
        let mut auth = crate::test_utils::test_auth_user(OWNER);
        auth.auth_method = method;
        let result = crate::handlers::triggers::cancel_run(
            axum::extract::State(state.clone()),
            auth,
            axum::extract::Path((trigger.id.clone(), run.id.clone())),
        )
        .await;
        assert!(result.is_err());
    }
    assert_eq!(
        state
            .db
            .collection::<TriggerRun>(RUNS)
            .find_one(doc! {"_id": &run.id})
            .await
            .unwrap()
            .unwrap()
            .outcome,
        RunOutcome::Started
    );
    server.abort();
}

#[tokio::test]
async fn v2_readiness_diagnostic_checks_each_indexed_drain_source_and_admin_roles() {
    use crate::services::{role_service, security_readiness_service as readiness};
    let (state, trigger, run, row, server) = root("v2_drain_diagnostic", false).await;
    readiness::ready_automation_test_replica(&state.db).await;
    role_service::seed_system_roles(&state.db).await.unwrap();
    let denied = crate::handlers::health::automation_continuations(
        axum::extract::State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
    )
    .await;
    assert!(matches!(denied, Err(AppError::Forbidden(_))));
    let roles = role_service::get_platform_role_ids(&state.db)
        .await
        .unwrap();
    for role in [&roles.admin, &roles.operator] {
        state
            .db
            .collection::<Document>(USERS)
            .update_one(doc! {"_id": OWNER}, doc! {"$set": {"role_ids": [role]}})
            .await
            .unwrap();
        let Json(report) = crate::handlers::health::automation_continuations(
            axum::extract::State(state.clone()),
            crate::test_utils::test_auth_user(OWNER),
        )
        .await
        .unwrap();
        assert!(!report.rollback_safe);
        assert!(
            report
                .drain_blockers
                .iter()
                .any(|reason| reason == "nonterminal_runs")
        );
    }
    v2::cancel(&state.db, OWNER, &trigger.id, &run.id, "cancelled", "")
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"active_turn": null}})
        .await
        .unwrap();
    assert!(
        v2::diagnostic(&state.db, false)
            .await
            .unwrap()
            .rollback_safe
    );
    for (collection, fields, reason) in [
        (HANDOFFS, doc! {"state": "pending"}, "pending_handoffs"),
        (
            CONVERSATIONS,
            doc! {"active_turn": {"automation_work": {"run_id": "drain"}}},
            "active_children",
        ),
        (
            CONVERSATIONS,
            doc! {"pending_events": [{"automation_work": {"run_id": "drain"}}]},
            "queued_results",
        ),
        (
            crate::models::assistant_group::COLLECTION_NAME,
            doc! {"followers": [{"automation": {"work": {"run_id": "drain"}}}]},
            "group_work",
        ),
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"automation_work": {"run_id": "drain"}, "status": "allowed"},
            "confirmation_cards",
        ),
        (
            crate::models::async_service_operation::COLLECTION_NAME,
            doc! {"delivery": {"automation_work": {"run_id": "drain"}}, "state": "waiting"},
            "async_work",
        ),
    ] {
        let mut fields = fields;
        fields.insert("_id", "diagnostic-fixture");
        state
            .db
            .collection::<Document>(collection)
            .insert_one(fields)
            .await
            .unwrap();
        let report = v2::diagnostic(&state.db, false).await.unwrap();
        assert!(!report.rollback_safe, "{reason}");
        assert_eq!(report.drain_blockers, [reason]);
        state
            .db
            .collection::<Document>(collection)
            .delete_one(doc! {"_id": "diagnostic-fixture"})
            .await
            .unwrap();
    }
    assert!(
        v2::diagnostic(&state.db, false)
            .await
            .unwrap()
            .rollback_safe
    );
    server.abort();
}

#[tokio::test]
async fn v2_lost_child_lease_delivers_failure_without_waiting_for_run_timeout() {
    let (state, _, run, row, server) = root("v2_lost_child_lease", false).await;
    let child = child(&state, &row).await;
    state.db.collection::<Document>(CONVERSATIONS).update_one(
        doc! {"_id": &child.id},
        doc! {"$set": {"active_turn.lease_expires_at": schedules::date(Utc::now() - Duration::seconds(1))}},
    ).await.unwrap();
    direct::recover(&state.db, &run).await.unwrap();
    direct::recover(&state.db, &run).await.unwrap();
    let parent = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(parent.pending_events.len(), 1);
    assert!(parent.pending_events[0].text.contains("turn_lost"));
    assert_eq!(
        parent.pending_events[0]
            .automation_work
            .as_ref()
            .unwrap()
            .run_id,
        run.id
    );
    server.abort();
}

#[tokio::test]
async fn legacy_turn_stop_and_group_paths_do_not_read_or_persist_v2_state() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = commands.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            sink.lock().unwrap().push(event.command);
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "legacy_completion_parity",
        handler,
    )
    .await;
    engine::ensure_indexes(&db).await.unwrap();
    db.collection(USERS)
        .insert_one(test_user(OWNER, UserType::Person))
        .await
        .unwrap();
    let mut provider = crate::models::downstream_service::test_helpers::dummy_service();
    provider.id = Uuid::new_v4().to_string();
    provider.slug = engine::SERVICE_SLUG.into();
    provider.auth_method = "none".into();
    provider.requires_user_credential = false;
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(provider)
        .await
        .unwrap();
    let state = test_app_state(db);
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let thread = team::home_thread(&state.db, &state.encryption_keys, &agent)
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let row = engine::begin_turn(
        &state.db,
        OWNER,
        &start(&thread.id, "Ordinary owner request"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(!row.automation_v2);
    assert!(row.active_turn.as_ref().unwrap().automation_work.is_none());
    save_reply(&state, &row, "Ordinary reply", None).await;
    assert!(matches!(
        engine::request_stop_target(&state.db, OWNER, &row.id)
            .await
            .unwrap(),
        engine::StopTarget::Legacy
    ));
    let group = crate::services::assistant_group_service::create(
        &state.db,
        OWNER,
        "Ordinary group",
        &[agent.id],
        "user",
    )
    .await
    .unwrap();
    assert!(
        crate::services::assistant_group_service::take_followers_if_quiet(
            &state.db, OWNER, &group.id
        )
        .await
        .unwrap()
        .0
        .is_empty()
    );
    let recorded = commands.lock().unwrap().clone();
    assert!(!recorded.is_empty());
    for command in &recorded {
        for key in [
            "find",
            "aggregate",
            "count",
            "update",
            "findAndModify",
            "insert",
            "delete",
        ] {
            if let Ok(collection) = command.get_str(key) {
                assert!(
                    ![HANDOFFS, RUNS, "security_replicas"].contains(&collection),
                    "unexpected completion command: {command}"
                );
            }
        }
    }
    let stored = state
        .db
        .collection::<Document>(CONVERSATIONS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.contains_key("automation_v2"));
    let messages: Vec<Document> = state
        .db
        .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
        .find(doc! {"conversation_id": &row.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        messages
            .iter()
            .all(|message| !message.contains_key("automation_work"))
    );
}

#[tokio::test]
async fn v2_slow_committed_admission_keeps_its_driver_and_expired_fence_aborts() {
    let (mut state, _, server) = setup("v2_slow_admission").await;
    state.config.automation_continuation_v2_enabled = true;
    crate::services::security_readiness_service::ready_automation_test_replica(&state.db).await;
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let trigger = automation(&state, &agent.id, None).await;
    let run = schedules::enqueue_now(&state.db, &trigger).await.unwrap();
    let job = schedules::claim(&state.db, &run.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    let pause = crate::test_utils::install_admission_pause(&state.db);
    let worker_state = state.clone();
    let worker = tokio::spawn(async move { run_job_once(&worker_state, &job).await });
    tokio::time::timeout(std::time::Duration::from_secs(20), pause.entered.notified())
        .await
        .unwrap();
    // The old 30-second timeout dropped this future after begin_turn committed.
    tokio::time::sleep(std::time::Duration::from_secs(31)).await;
    assert!(
        !worker.is_finished(),
        "Committed admission must retain its future until it spawns the driver"
    );
    pause.resume.notify_one();
    worker.await.unwrap().unwrap();
    assert_eq!(
        completed(&state, &trigger).await.outcome,
        RunOutcome::Completed
    );

    let next = schedules::enqueue_now(&state.db, &trigger).await.unwrap();
    let job = schedules::claim(&state.db, &next.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    let thread = schedules::run_thread(&state.db, &state.encryption_keys, &next, &trigger)
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(schedules::WORK)
        .update_one(
            doc! {"_id": &next.id},
            doc! {"$set": {"at": schedules::date(Utc::now() - Duration::seconds(1))}},
        )
        .await
        .unwrap();
    let mut request = start(&thread.id, "Expired admission");
    request.origin = TurnOrigin::Trigger;
    request.trigger = Some(schedules::TurnClaim {
        enroll_v2: true,
        run_id: next.id,
        fence: job.get_str("fence").unwrap().into(),
        trigger_updated_at: trigger.updated_at,
        continuation: false,
    });
    assert!(
        engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
            .await
            .is_err()
    );
    assert!(
        engine::get(&state.db, OWNER, &thread.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    server.abort();
}

fn stop_job(
    id: &str,
    thread: &str,
    turn: Option<&str>,
    run: Option<&str>,
) -> crate::models::machine_job::MachineJob {
    crate::models::machine_job::MachineJob {
        automation_run_id: run.map(str::to_owned),
        id: id.into(),
        user_id: OWNER.into(),
        node_id: "offline-node".into(),
        runtime_id: "runtime".into(),
        binding: None,
        runtime_generation: None,
        conversation_id: thread.into(),
        issuing_turn_id: turn.map(str::to_owned),
        api_key_id: "test-key".into(),
        agent_id: "test-agent".into(),
        state: "running".into(),
        exit_code: None,
        stop_reason: None,
        duration_ms: None,
        cancellation_requested_by: None,
        cancellation_requested_at: None,
        services: Vec::new(),
        report_to: None,
        report_provenance: 0,
        created_at: Utc::now(),
        expires_at: Utc::now() + Duration::hours(1),
        finished_at: None,
        retain_until: None,
    }
}

#[tokio::test]
async fn v2_marked_idle_and_owner_stop_keep_main_cleanup_and_protect_only_live_runs() {
    use crate::models::async_service_operation::COLLECTION_NAME as WATCHES;
    use crate::models::machine_job::{COLLECTION_NAME as JOBS, MachineJob};
    for marked in [false, true] {
        for active in [false, true] {
            for waiting in [false, true] {
                if waiting && !marked {
                    continue;
                }
                let (state, _, run, row, server) = root("v2_stop_main_cleanup", false).await;
                let _child = child(&state, &row).await;
                save_reply(&state, &row, "Waiting", None).await;
                if waiting {
                    v2::barrier(&state.db, &run).await.unwrap();
                } else {
                    schedules::finish(&state.db, &run.id, RunOutcome::Completed, None, "")
                        .await
                        .unwrap();
                }
                if !marked {
                    state
                        .db
                        .collection::<Document>(CONVERSATIONS)
                        .update_one(
                            doc! {"_id": &row.id},
                            doc! {"$unset": {"automation_v2": ""}},
                        )
                        .await
                        .unwrap();
                }
                let owner = if active {
                    Some(
                        engine::begin_turn(
                            &state.db,
                            OWNER,
                            &start(&row.id, "Ordinary question"),
                            &state.encryption_keys,
                        )
                        .await
                        .unwrap(),
                    )
                } else {
                    None
                };
                let turn = owner
                    .as_ref()
                    .and_then(|r| r.active_turn.as_ref())
                    .map(|t| t.turn_id.as_str());
                for (id, bound) in [
                    ("old-watch", None),
                    ("current-watch", None),
                    ("bound-watch", Some(&run.id)),
                ] {
                    let mut watch = doc! {"_id": id, "user_id": OWNER, "conversation_id": &row.id, "turn_id": turn, "state": "waiting"};
                    if let Some(run) = bound {
                        watch.insert("delivery", doc! {"automation_work": {"run_id": run}});
                    }
                    state
                        .db
                        .collection::<Document>(WATCHES)
                        .insert_one(watch)
                        .await
                        .unwrap();
                }
                state
                    .db
                    .collection::<MachineJob>(JOBS)
                    .insert_many([
                        stop_job("old-job", &row.id, Some("older-turn"), None),
                        stop_job("current-job", &row.id, turn, None),
                        stop_job("bound-job", &row.id, Some("automation-turn"), Some(&run.id)),
                    ])
                    .await
                    .unwrap();
                let target = engine::request_stop_target(&state.db, OWNER, &row.id)
                    .await
                    .unwrap();
                match target {
                    engine::StopTarget::Legacy => {
                        crate::services::machine_service::cancel_conversation(
                            &state, OWNER, &row.id,
                        )
                        .await
                        .unwrap()
                    }
                    engine::StopTarget::Scoped(snapshot) => {
                        crate::services::machine_service::cancel_unrelated(
                            &state,
                            OWNER,
                            &row.id,
                            snapshot.as_deref(),
                        )
                        .await
                        .unwrap()
                    }
                }
                for id in ["old-watch", "current-watch", "bound-watch"] {
                    let watch = state
                        .db
                        .collection::<Document>(WATCHES)
                        .find_one(doc! {"_id": id})
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        watch.get_str("state").unwrap(),
                        if id == "bound-watch" && waiting {
                            "waiting"
                        } else {
                            "cancelling"
                        }
                    );
                }
                for id in ["old-job", "current-job", "bound-job"] {
                    let job = state
                        .db
                        .collection::<MachineJob>(JOBS)
                        .find_one(doc! {"_id": id})
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        job.state,
                        if id == "bound-job" && waiting {
                            "running"
                        } else {
                            "finished"
                        }
                    );
                }
                if active {
                    assert!(
                        engine::get(&state.db, OWNER, &row.id)
                            .await
                            .unwrap()
                            .active_turn
                            .unwrap()
                            .stop_requested
                    );
                }
                if waiting {
                    assert_eq!(
                        state
                            .db
                            .collection::<TriggerRun>(RUNS)
                            .find_one(doc! {"_id": &run.id})
                            .await
                            .unwrap()
                            .unwrap()
                            .outcome,
                        RunOutcome::Waiting
                    );
                }
                server.abort();
            }
        }
    }
}

#[tokio::test]
async fn v2_direct_after_turn_keeps_delivery_and_queued_pool_duties() {
    let (state, _, _, row, server) = root("v2_after_turn_duties", false).await;
    let child = child(&state, &row).await;
    let queued_agent = named_specialist(&state, "queued-researcher").await;
    let queued = team::home_thread(&state.db, &state.encryption_keys, &queued_agent)
        .await
        .unwrap();
    engine::push_events(
        &state.db,
        OWNER,
        &queued.id,
        vec![team::event("test", "Queued work".into(), None)],
    )
    .await
    .unwrap();
    state.db.collection::<Document>(CONVERSATIONS).update_one(doc! {"_id": &child.id}, doc! {"$set": {"deliver_also": [{"nyxbot_channel_id": "missing-channel", "partition": "chat", "platform": "telegram"}]}}).await.unwrap();
    save_reply(&state, &child, "Research finished", None).await;
    crate::handlers::assistant_team::after_turn(&state, &child, "Research finished", None).await;
    assert!(
        engine::get(&state.db, OWNER, &child.id)
            .await
            .unwrap()
            .deliver_also
            .is_empty()
    );
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if state
                .db
                .collection::<Document>(MESSAGES)
                .find_one(doc! {"conversation_id": &queued.id, "role": "assistant"})
                .await
                .unwrap()
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    server.abort();
}

#[tokio::test]
async fn v2_child_fences_are_independent_and_ignore_bson_key_order() {
    let (state, _, run, row, server) = root("v2_parallel_fences", false).await;
    // Persist the same identity in a different BSON key order.
    let work = row
        .active_turn
        .as_ref()
        .unwrap()
        .automation_work
        .as_ref()
        .unwrap();
    let ordered = mongodb::bson::to_document(work).unwrap();
    let reordered: Document = ordered
        .iter()
        .rev()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"active_turn.automation_work": reordered}},
        )
        .await
        .unwrap();
    let mut children = Vec::new();
    for i in 0..6 {
        let agent = named_specialist(&state, &format!("parallel-{i}")).await;
        let thread = team::home_thread(&state.db, &state.encryption_keys, &agent)
            .await
            .unwrap();
        let mut request = start(&thread.id, "Parallel work");
        request.origin = TurnOrigin::Orchestrator;
        request.report_to = Some(row.id.clone());
        request.automation_parent = Some(direct::Parent {
            thread: row.id.clone(),
            turn: row.active_turn.as_ref().unwrap().turn_id.clone(),
            work: work.clone(),
        });
        children.push(
            engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
                .await
                .unwrap(),
        );
    }
    let before = state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id": &run.id})
        .await
        .unwrap()
        .unwrap()
        .coordinator
        .unwrap()
        .version;
    for child in &children {
        let work = child
            .active_turn
            .as_ref()
            .unwrap()
            .automation_work
            .as_ref()
            .unwrap();
        let ordered = mongodb::bson::to_document(work).unwrap();
        let reordered: Document = ordered
            .iter()
            .rev()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        state
            .db
            .collection::<Document>(HANDOFFS)
            .update_one(
                doc! {"_id": &work.work_id},
                doc! {"$set": {"work": reordered}},
            )
            .await
            .unwrap();
        let mut preview = child.clone();
        let mut event = team::event("test", "Resume this child".into(), None);
        event.automation_work = Some(work.clone());
        preview.pending_events = vec![event];
        assert!(v2::event_can_resume(&state.db, &preview).await.unwrap());
    }
    let fences = children.iter().map(|child| {
        let db = state.db.clone();
        let thread = child.id.clone();
        let work = child
            .active_turn
            .as_ref()
            .unwrap()
            .automation_work
            .clone()
            .unwrap();
        async move {
            let context = (db.clone(), thread, work);
            crate::services::transaction_service::run_db_only(
                &db,
                &context,
                |session, (db, thread, work)| {
                    Box::pin(async move { v2::fence_work(db, OWNER, thread, work, session).await })
                },
            )
            .await
        }
    });
    for result in futures::future::join_all(fences).await {
        result.unwrap();
    }
    assert_eq!(
        state
            .db
            .collection::<TriggerRun>(RUNS)
            .find_one(doc! {"_id": &run.id})
            .await
            .unwrap()
            .unwrap()
            .coordinator
            .unwrap()
            .version,
        before
    );
    v2::cancel(&state.db, OWNER, &run.trigger_id, &run.id, "cancelled", "")
        .await
        .unwrap();
    let child = &children[0];
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        v2::fence_work(
            &state.db,
            OWNER,
            &child.id,
            child
                .active_turn
                .as_ref()
                .unwrap()
                .automation_work
                .as_ref()
                .unwrap(),
            &mut session
        )
        .await
        .is_err()
    );
    server.abort();
}

#[tokio::test]
async fn v2_new_hot_collection_indexes_exclude_unbound_documents() {
    let (state, _, _, _, server) = root("v2_partial_indexes", false).await;
    let mut run_indexes = 0;
    for collection in [
        CONVERSATIONS,
        RUNS,
        crate::models::assistant_group::COLLECTION_NAME,
        crate::models::assistant_acknowledgement::COLLECTION_NAME,
        crate::models::async_service_operation::COLLECTION_NAME,
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
        for index in indexes {
            let options = index.options.unwrap();
            if options
                .name
                .as_deref()
                .is_some_and(|name| name.starts_with("automation_"))
            {
                assert!(
                    options.partial_filter_expression.is_some(),
                    "{collection}: {:?}",
                    options.name
                );
                if collection == RUNS {
                    run_indexes += 1;
                }
            }
        }
    }
    assert_eq!(
        run_indexes, 1,
        "One partial index covers drain and owner/thread lookups"
    );
    // Diagnostic hints execute with exactly the partial-index predicates.
    v2::diagnostic(&state.db, false).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn v2_legacy_queue_eviction_preserves_protected_events() {
    let (state, _, run, row, server) = root("v2_legacy_eviction", false).await;
    let mut protected = team::event("group_settled", "Protected result".into(), None);
    protected.automation_work = Some(v2::context(&run, &row.id));
    let mut events = vec![protected.clone()];
    for i in 1..engine::MAX_PENDING_EVENTS {
        events.push(team::event("ordinary", format!("notice-{i}"), None));
    }
    engine::push_events(&state.db, OWNER, &row.id, events)
        .await
        .unwrap();
    engine::push_events(
        &state.db,
        OWNER,
        &row.id,
        vec![team::event("ordinary", "Newest notice".into(), None)],
    )
    .await
    .unwrap();
    let current = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(current.pending_events.len(), engine::MAX_PENDING_EVENTS);
    assert!(current.pending_events.iter().any(|e| e.id == protected.id));
    assert!(!current.pending_events.iter().any(|e| e.text == "notice-1"));
    assert!(
        current
            .pending_events
            .iter()
            .any(|e| e.text == "Newest notice")
    );
    let protected: Vec<_> = (0..engine::MAX_PENDING_EVENTS)
        .map(|i| {
            let mut event = team::event("group_settled", format!("result-{i}"), None);
            event.automation_work = Some(v2::context(&run, &row.id));
            event
        })
        .collect();
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"pending_events": mongodb::bson::to_bson(&protected).unwrap()}},
        )
        .await
        .unwrap();
    assert!(
        engine::push_events(
            &state.db,
            OWNER,
            &row.id,
            vec![team::event("ordinary", "Cannot fit".into(), None)]
        )
        .await
        .is_err()
    );
    assert_eq!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .len(),
        engine::MAX_PENDING_EVENTS
    );
    server.abort();
}

#[tokio::test]
async fn v2_group_posts_use_sticky_then_lead_recipients_and_bind_summary() {
    let (state, _, _, row, server) = root("v2_group_recipients", false).await;
    let editor = named_specialist(&state, "editor").await;
    let reviewer = named_specialist(&state, "reviewer").await;
    let group = crate::services::assistant_group_service::create(
        &state.db,
        OWNER,
        "Recipients",
        &[editor.id.clone(), reviewer.id.clone()],
        "user",
    )
    .await
    .unwrap();
    let (_, ids) = work_groups::post(&state.db, &authority(&row), &group, "Investigate this")
        .await
        .unwrap();
    assert_eq!(ids, [group.lead_agent_id.clone()]);
    crate::services::assistant_group_service::persist_last_recipients(
        &state.db,
        OWNER,
        &group.id,
        std::slice::from_ref(&reviewer.id),
    )
    .await
    .unwrap();
    let group = crate::services::assistant_group_service::get(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    let (_, ids) = work_groups::post(&state.db, &authority(&row), &group, "Follow up")
        .await
        .unwrap();
    assert_eq!(ids, [reviewer.id]);
    let group = crate::services::assistant_group_service::get(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    for (_, agent) in work_groups::queued(&group) {
        let child = group_turn(&state, &group, &agent).await;
        save_reply(&state, &child, "Finished", None).await;
    }
    work_groups::recover(&state.db, &group).await.unwrap();
    let current = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    let summary = current
        .pending_events
        .iter()
        .find(|e| e.kind == "group_settled")
        .unwrap();
    assert_eq!(summary.report_to.as_deref(), Some(row.id.as_str()));
    server.abort();
}

#[tokio::test]
async fn v2_delegation_tracks_usable_org_and_shared_specialists_and_shared_group_members() {
    use crate::models::{
        assistant_sharing::{ShareRole, ShareSubject},
        org_membership::OrgRole,
    };
    use crate::services::{feature_flag_service as flags, shared_agent_service as sharing};
    let (state, _, run, row, server) = root("v2_foreign_specialists", false).await;
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    sharing::ensure_indexes(&state.db).await.unwrap();
    for flag in ["assistant:org-agents", sharing::FLAG] {
        flags::set_platform_override(&state.db, flag, &flags::FlagTarget::Global, true, OWNER)
            .await
            .unwrap();
    }
    let mut shared_agent = None;
    for org in [true, false] {
        let owner = Uuid::new_v4().to_string();
        state
            .db
            .collection(USERS)
            .insert_one(test_user(
                &owner,
                if org { UserType::Org } else { UserType::Person },
            ))
            .await
            .unwrap();
        if org {
            state
                .db
                .collection(crate::models::org_membership::COLLECTION_NAME)
                .insert_one(crate::test_utils::test_membership(
                    &owner,
                    OWNER,
                    OrgRole::Admin,
                    None,
                ))
                .await
                .unwrap();
        }
        let request = team::CreateRequest {
            machines: None,
            logins: None,
            name: if org {
                "org-researcher"
            } else {
                "shared-researcher"
            }
            .into(),
            description: "Delegated research".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        };
        let (agent, _) = team::create_specialist_for(
            &state.db,
            &state.encryption_keys,
            if org { OWNER } else { &owner },
            &owner,
            request,
        )
        .await
        .unwrap()
        .unwrap();
        if !org {
            sharing::update(
                &state.db,
                &owner,
                &agent.id,
                sharing::SharingUpdate {
                    expected_version: 0,
                    enabled: true,
                    shared_access: Default::default(),
                    audience: vec![sharing::Audience {
                        subject_kind: ShareSubject::Person,
                        subject_id: OWNER.into(),
                        role: ShareRole::User,
                    }],
                },
                true,
            )
            .await
            .unwrap();
            shared_agent = Some(agent.clone());
        }
        let admitted = team::delegable_specialist(&state.db, OWNER, &agent.id)
            .await
            .unwrap();
        let thread = team::home_thread_for(&state.db, &state.encryption_keys, OWNER, &admitted)
            .await
            .unwrap();
        let mut request = start(&thread.id, "Research for this automation");
        request.origin = TurnOrigin::Orchestrator;
        request.report_to = Some(row.id.clone());
        request.automation_parent = Some(direct::Parent {
            thread: row.id.clone(),
            turn: row.active_turn.as_ref().unwrap().turn_id.clone(),
            work: row
                .active_turn
                .as_ref()
                .unwrap()
                .automation_work
                .clone()
                .unwrap(),
        });
        let child = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
            .await
            .unwrap();
        assert_eq!(child.user_id, OWNER);
        assert_eq!(child.agent_owner_id.as_deref(), Some(owner.as_str()));
        assert_eq!(child.shared_agent_thread, !org);
        assert!(child.active_turn.as_ref().unwrap().execution_task.is_none());
        save_reply(&state, &child, "Foreign specialist result", None).await;
        direct::recover(&state.db, &run).await.unwrap();
        let current = engine::get(&state.db, OWNER, &row.id).await.unwrap();
        assert!(current.pending_events.iter().any(|e| {
            e.agent_id.as_deref() == Some(agent.id.as_str())
                && e.automation_work
                    .as_ref()
                    .is_some_and(|w| w.run_id == run.id)
        }));
    }
    let shared = shared_agent.unwrap();
    let group = crate::services::assistant_group_service::create(
        &state.db,
        OWNER,
        "Shared crew",
        std::slice::from_ref(&shared.id),
        "user",
    )
    .await
    .unwrap();
    let (_, addressed) = work_groups::post(
        &state.db,
        &authority(&row),
        &group,
        "@shared-researcher Review this",
    )
    .await
    .unwrap();
    assert_eq!(addressed, [shared.id.clone()]);
    let current = crate::services::assistant_group_service::get(&state.db, OWNER, &group.id)
        .await
        .unwrap();
    let member = group_turn(&state, &current, &shared.id).await;
    assert!(member.shared_agent_thread);
    save_reply(&state, &member, "Shared group result", None).await;
    crate::handlers::assistant_team::after_turn(&state, &member, "Shared group result", None).await;
    let parent = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert!(
        parent
            .pending_events
            .iter()
            .any(|e| e.kind == "group_settled" && e.text.contains("Shared group result"))
    );
    server.abort();
}
