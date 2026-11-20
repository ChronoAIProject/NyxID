use super::*;
use crate::models::assistant_group::{
    AssistantGroup, GroupMessage, MESSAGES_COLLECTION_NAME as GROUP_MESSAGES,
};
use crate::services::assistant_group_service as groups;

async fn queued_editor(
    state: &AppState,
    coordinator: &AssistantConversation,
) -> (AssistantGroup, String) {
    let editor = named_specialist(state, "editor").await;
    let group = groups::create(
        &state.db,
        OWNER,
        "Draft crew",
        std::slice::from_ref(&editor.id),
        "user",
    )
    .await
    .unwrap();
    work_groups::post(&state.db, &authority(coordinator), &group, "@editor draft")
        .await
        .unwrap();
    (
        groups::get(&state.db, OWNER, &group.id).await.unwrap(),
        editor.id,
    )
}

#[tokio::test]
async fn delayed_initial_dispatch_cannot_replace_a_finished_member_turn() {
    let (state, _, run, coordinator, server) = root("workflow_duplicate", false).await;
    let (queued, editor) = queued_editor(&state, &coordinator).await;
    let first = group_turn(&state, &queued, &editor).await;
    let mut delayed = group_request(&queued, &editor);
    delayed.conversation_id = Some(first.id.clone());
    let turn = first.active_turn.as_ref().unwrap();
    assert!(matches!(
        engine::begin_turn(&state.db, OWNER, &delayed, &state.encryption_keys).await,
        Err(AppError::AssistantTurnActive)
    ));
    save_reply(&state, &first, "Durable editor draft", None).await;

    // The first worker finished persistence and lost its group callback.
    // The other worker had already selected the queued identity.
    let delayed_result =
        engine::begin_turn(&state.db, OWNER, &delayed, &state.encryption_keys).await;
    assert!(
        matches!(delayed_result, Err(AppError::AssistantTurnRequired)),
        "stale dispatcher should be refused after durable finish, got {:?}",
        delayed_result
            .as_ref()
            .map(|row| row.active_turn.as_ref().unwrap().turn_id.as_str())
    );
    let group = groups::get(&state.db, OWNER, &queued.id).await.unwrap();
    let dispatch = &group.followers[0].automation.as_ref().unwrap().dispatches[0];
    assert_eq!(dispatch.turn_id.as_deref(), Some(turn.turn_id.as_str()));
    assert!(
        engine::get(&state.db, OWNER, &first.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );

    save_reply(&state, &coordinator, "Waiting", None).await;
    assert_eq!(
        v2::barrier(&state.db, &run).await.unwrap(),
        v2::Barrier::Waiting
    );
    let (one, two) = tokio::join!(
        work_groups::recover(&state.db, &group),
        work_groups::recover(&state.db, &group)
    );
    one.unwrap();
    two.unwrap();
    let thread = engine::get(&state.db, OWNER, &coordinator.id)
        .await
        .unwrap();
    assert_eq!(thread.pending_events.len(), 1);
    assert!(
        thread.pending_events[0]
            .text
            .contains("Durable editor draft")
    );
    assert_eq!(
        groups::get(&state.db, OWNER, &group.id)
            .await
            .unwrap()
            .message_count,
        group.message_count + 1
    );
    server.abort();
}

#[tokio::test]
async fn recovery_interleaved_with_member_finish_keeps_the_durable_reply() {
    let (state, _, _, coordinator, server) = root("workflow_recover_race", false).await;
    let (queued, editor) = queued_editor(&state, &coordinator).await;
    let member = group_turn(&state, &queued, &editor).await;
    let group = groups::get(&state.db, OWNER, &queued.id).await.unwrap();

    crate::services::durable_wake_tests::resume_after(
        "automation_group_member_read",
        async { work_groups::recover(&state.db, &group).await.unwrap() },
        async {
            save_reply(
                &state,
                &member,
                "Editor result committed during recovery",
                None,
            )
            .await;
        },
    )
    .await;

    work_groups::recover(&state.db, &group).await.unwrap();
    let coordinator = engine::get(&state.db, OWNER, &coordinator.id)
        .await
        .unwrap();
    assert_eq!(coordinator.pending_events.len(), 1);
    let summary = &coordinator.pending_events[0].text;
    assert!(
        summary.contains("Editor result committed during recovery"),
        "{summary}"
    );
    let replies: Vec<GroupMessage> = state
        .db
        .collection(GROUP_MESSAGES)
        .find(doc! {"group_id": &group.id, "agent_id": &editor})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].role, "agent");
    server.abort();
}

#[tokio::test]
async fn thread_run_recovers_a_claimed_result_from_its_saved_reply() {
    for (enroll_v2, error, expected, reason) in [
        (
            true,
            None,
            RunOutcome::Completed,
            Some("coordinator_completed"),
        ),
        (
            true,
            Some("model_unavailable"),
            RunOutcome::Failed,
            Some("model_unavailable"),
        ),
        (false, None, RunOutcome::Completed, None),
    ] {
        let (state, trigger, run, row, server) =
            root_with_enrollment("workflow_claimed_thread", false, enroll_v2).await;
        save_reply(&state, &row, "Durable final reply in thread", error).await;
        let mut stale = run.clone();
        stale.turn_id = Some(Uuid::new_v4().to_string());
        settle_result(&state, &row, &stale, "", Some("turn_lost"))
            .await
            .unwrap();
        if enroll_v2 {
            schedules::terminal_result(&state.db, &run, error, &state.config.frontend_url)
                .await
                .unwrap();
        }
        // Old thread result settlement could die after claiming the send slot.
        state
            .db
            .collection::<Document>(RUNS)
            .update_one(
                doc! {"_id": &run.id, "outcome": "started"},
                doc! {"$set": {"result_claimed": true}},
            )
            .await
            .map(|updated| assert_eq!(updated.modified_count, 1))
            .unwrap();
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
        assert_eq!(finished.id, run.id);
        assert_eq!(finished.outcome, expected, "reason: {:?}", finished.reason);
        assert_eq!(finished.reason.as_deref(), reason);
        assert_eq!(finished.execution_outcome, Some(expected));
        assert_eq!(finished.thread_id, Some(row.id));
        server.abort();
    }
}

#[tokio::test]
async fn group_member_can_resume_from_its_bound_event_or_allowed_card() {
    use crate::models::assistant_acknowledgement::COLLECTION_NAME as ACKS;
    use crate::services::assistant_acknowledgement_service as acks;

    for use_event in [true, false] {
        let (state, _, _, coordinator, server) = root("workflow_group_successor", false).await;
        let (queued, editor) = queued_editor(&state, &coordinator).await;
        let member = group_turn(&state, &queued, &editor).await;
        let first_turn = member.active_turn.as_ref().unwrap();
        let work = first_turn.automation_work.as_ref().unwrap();
        let card = Uuid::new_v4().to_string();
        state.db.collection::<Document>(ACKS).insert_one(doc! {
            "_id": &card, "user_id": OWNER, "conversation_id": &member.id,
            "api_key_id": &member.credential_api_key_id, "kind": "action",
            "tool_name": "approved_call", "summary": "Approved action", "status": "pending",
            "requested_turn_id": &first_turn.turn_id, "automation_work": mongodb::bson::to_bson(work).unwrap(),
            "created_at": schedules::date(Utc::now()),
            "expires_at": schedules::date(Utc::now() + Duration::minutes(10)), "decider": "user",
        }).await.unwrap();
        save_reply(&state, &member, "Waiting for approval", None).await;
        acks::decide(&state.db, OWNER, &member.id, &card, true)
            .await
            .unwrap();
        let group = groups::get(&state.db, OWNER, &queued.id).await.unwrap();
        work_groups::recover(&state.db, &group).await.unwrap();
        assert!(
            engine::get(&state.db, OWNER, &coordinator.id)
                .await
                .unwrap()
                .pending_events
                .is_empty()
        );

        let mut request = start(&member.id, "Continue approved work");
        if use_event {
            let ordinary = team::event("ordinary", "Unbound update".into(), None);
            engine::push_events(&state.db, OWNER, &member.id, vec![ordinary])
                .await
                .unwrap();
            request.origin = TurnOrigin::Event;
        } else {
            request.acknowledgement_id = Some(card.clone());
        }
        let resumed = engine::begin_turn(&state.db, OWNER, &request, &state.encryption_keys)
            .await
            .unwrap();
        let latest = resumed.active_turn.as_ref().unwrap();
        assert_eq!(latest.automation_work.as_ref(), Some(work));
        assert_ne!(latest.turn_id, first_turn.turn_id);
        let group_after_admission = groups::get(&state.db, OWNER, &group.id).await.unwrap();
        let dispatch = &group_after_admission.followers[0]
            .automation
            .as_ref()
            .unwrap()
            .dispatches[0];
        assert_eq!(dispatch.turn_id.as_deref(), Some(latest.turn_id.as_str()));
        assert_eq!(dispatch.thread_id.as_deref(), Some(member.id.as_str()));

        // An older snapshot can see the first saved reply while the successor is live.
        work_groups::recover(&state.db, &group).await.unwrap();
        assert!(
            engine::get(&state.db, OWNER, &coordinator.id)
                .await
                .unwrap()
                .pending_events
                .is_empty()
        );
        if use_event {
            assert_eq!(resumed.pending_events.len(), 1);
            assert_eq!(resumed.pending_events[0].kind, "ordinary");
            assert!(
                latest
                    .events
                    .iter()
                    .any(|event| event.id == format!("automation-decision:{card}"))
            );
            state
                .db
                .collection::<Document>(ACKS)
                .update_one(doc! {"_id": &card}, doc! {"$set": {"status": "used"}})
                .await
                .unwrap();
            save_reply(&state, &resumed, "Latest approved result", None).await;
            work_groups::recover(&state.db, &group_after_admission)
                .await
                .unwrap();
            let thread = engine::get(&state.db, OWNER, &coordinator.id)
                .await
                .unwrap();
            assert_eq!(thread.pending_events.len(), 1);
            assert!(
                thread.pending_events[0]
                    .text
                    .contains("Latest approved result")
            );
        }
        server.abort();
    }
}

#[tokio::test]
async fn expired_group_loss_is_revalidated_after_renewal_or_finish() {
    for finish in [false, true] {
        let (state, _, _, coordinator, server) = root("workflow_late_recovery", false).await;
        let (queued, editor) = queued_editor(&state, &coordinator).await;
        let member = group_turn(&state, &queued, &editor).await;
        let turn = member.active_turn.as_ref().unwrap();
        state.db.collection::<Document>(CONVERSATIONS).update_one(
            doc! {"_id": &member.id, "user_id": OWNER, "active_turn.turn_id": &turn.turn_id},
            doc! {"$set": {"active_turn.lease_expires_at": schedules::date(Utc::now() - Duration::seconds(1))}},
        ).await.unwrap();
        let group = groups::get(&state.db, OWNER, &queued.id).await.unwrap();
        crate::services::durable_wake_tests::resume_after(
            "automation_group_loss_observed",
            async { work_groups::recover(&state.db, &group).await.unwrap() },
            async {
                if finish {
                    save_reply(
                        &state,
                        &member,
                        "Member finished after loss observation",
                        None,
                    )
                    .await;
                } else {
                    let now = Utc::now();
                    assert!(
                        engine::mark_queued(&state.db, &member, now, now + Duration::seconds(30))
                            .await
                            .unwrap()
                    );
                }
            },
        )
        .await;

        let thread = engine::get(&state.db, OWNER, &coordinator.id)
            .await
            .unwrap();
        if finish {
            assert_eq!(thread.pending_events.len(), 1);
            let summary = &thread.pending_events[0].text;
            assert!(
                summary.contains("Member finished after loss observation"),
                "{summary}"
            );
        } else {
            let renewed = engine::get(&state.db, OWNER, &member.id).await.unwrap();
            assert_eq!(
                engine::live_turn(&renewed, Utc::now()).unwrap().turn_id,
                turn.turn_id
            );
            assert!(
                thread.pending_events.is_empty(),
                "{:?}",
                thread
                    .pending_events
                    .iter()
                    .map(|e| &e.text)
                    .collect::<Vec<_>>()
            );
            // An unrenewed, expired turn with no saved reply still settles once.
            state.db.collection::<Document>(CONVERSATIONS).update_one(
                doc! {"_id": &member.id, "user_id": OWNER, "active_turn.turn_id": &turn.turn_id},
                doc! {"$set": {"active_turn.lease_expires_at": schedules::date(Utc::now() - Duration::seconds(1))}},
            ).await.unwrap();
            work_groups::recover(&state.db, &group).await.unwrap();
            work_groups::recover(&state.db, &group).await.unwrap();
            let thread = engine::get(&state.db, OWNER, &coordinator.id)
                .await
                .unwrap();
            assert_eq!(thread.pending_events.len(), 1);
            assert!(
                thread.pending_events[0]
                    .text
                    .contains("editor failed (turn_lost)")
            );
            let now = Utc::now();
            assert!(
                !engine::mark_queued(&state.db, &member, now, now + Duration::seconds(30))
                    .await
                    .unwrap(),
                "a turn that has been reported lost could still renew its lease"
            );
        }
        server.abort();
    }
}
