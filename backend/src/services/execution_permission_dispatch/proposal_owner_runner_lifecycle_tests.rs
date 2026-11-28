//! Deterministic late-expiry, Stop-scope and retained-stream authority controls.
use super::*;

pub(super) async fn disable_rollout(f: &Fixture) {
    f.enroll(false).await;
    assert!(
        !crate::services::execution_permission_enrollment::live_enabled(
            &f.state.db,
            &f.owner,
            None
        )
        .await
        .unwrap()
    );
}

pub(super) async fn wait_for_task_expiry(db: &Database, row: &AssistantConversation) {
    let task = row
        .active_turn
        .as_ref()
        .unwrap()
        .execution_task
        .as_ref()
        .unwrap();
    // Wait on the exact persisted Mongo deadline. The polling interval is only
    // backoff; no guessed delay decides whether the fault has been reached.
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if db
                .collection::<Document>("assistant_messages")
                .find_one(doc! {
                    "_id": &task.task.root_id,
                    "execution_task.hard_deadline": bson::DateTime::from_chrono(task.hard_deadline),
                "execution_task.epoch": task.epoch,
                    "$expr": {"$lte": ["$execution_task.hard_deadline", "$$NOW"]},
                })
                .await
                .unwrap()
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

pub(super) async fn run_at_expiry_boundary(
    f: &Fixture,
    row: &AssistantConversation,
    target: &'static str,
    b: &ApprovalRequest,
    a_first: bool,
) {
    let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = reached.clone();
    let b = b.clone();
    let hook: proposals::ResumeBoundaryHook = Arc::new(move |db, row, boundary| {
        let reached = observed.clone();
        let b = b.clone();
        Box::pin(async move {
            if boundary != target || reached.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let task = row
                .active_turn
                .as_ref()
                .unwrap()
                .execution_task
                .as_ref()
                .unwrap();
            assert!(db.collection::<Document>("assistant_messages").find_one(doc! {
                "_id": &task.task.root_id, "$expr": {"$gt": ["$execution_task.hard_deadline", "$$NOW"]},
            }).await.unwrap().is_some(), "boundary must begin before the exact source deadline");
            if boundary == "between_members" && !a_first {
                let current = db
                    .collection::<ApprovalRequest>("approval_requests")
                    .find_one(doc! {"_id": &b.id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(proposals::proposal(&current).unwrap().state, "cancelled");
                let active = engine::get(&db, &row.user_id, &row.id).await.unwrap();
                assert_eq!(
                    active.active_turn.unwrap().execution_local_retirements,
                    Some(vec![b.id.clone()])
                );
                assert!(
                    worker_documents(&db, &b)
                        .await
                        .1
                        .get_bool("revoked")
                        .unwrap()
                );
            }
            wait_for_task_expiry(&db, &row).await;
        })
    });
    proposals::RESUME_BOUNDARY.scope(hook, run(f, row)).await;
    assert!(
        reached.load(std::sync::atomic::Ordering::SeqCst),
        "the exact requested boundary must execute"
    );
}

#[tokio::test]
async fn owner_permission_resume_expiry_crosses_after_check_and_between_members() {
    // Disabling after admission keeps B unsent while the B-first schedule
    // commits an ordinary local cancellation before the later expiry boundary.
    for boundary in ["after_preliminary_check", "between_members"] {
        expired_channel_observer_cases(&["expired_delivered"], Some(boundary), true).await;
    }
    expired_channel_observer_cases(&["live"], None, false).await;
}

#[tokio::test]
async fn owner_permission_resume_disabled_expired_channel_retires_only_unsent() {
    expired_channel_observer_cases(&["expired_undelivered"], None, true).await;
}

pub(super) async fn protected_job(
    f: &Fixture,
    row: &AssistantConversation,
    ordinary: &str,
) -> (String, Document) {
    use crate::models::trigger::Trigger;
    use crate::models::trigger_run::RunOutcome;
    let now = bson::DateTime::now();
    let trigger: Trigger = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": &f.owner, "label": "Protected automation",
        "status": "active", "secret_hash": "schedule-fixture", "verification": {"mode": "schedule"},
        "delivery": {"type": "notification"}, "created_at": now, "updated_at": now,
    })
    .unwrap();
    f.state
        .db
        .collection::<Trigger>("triggers")
        .insert_one(&trigger)
        .await
        .unwrap();
    let mut run = crate::services::trigger_schedule::new_run(
        &trigger,
        now.to_chrono(),
        now.to_chrono() + chrono::Duration::hours(1),
    );
    run.outcome = RunOutcome::Waiting;
    run.coordinator = Some(crate::services::trigger_schedule::continuation::state(
        now.to_chrono(),
        run.deadline,
    ));
    run.thread_id = Some(row.id.clone());
    f.state
        .db
        .collection::<crate::models::trigger_run::TriggerRun>("trigger_runs")
        .insert_one(&run)
        .await
        .unwrap();
    let mut job = f
        .state
        .db
        .collection::<Document>("machine_jobs")
        .find_one(doc! {"_id": ordinary})
        .await
        .unwrap()
        .unwrap();
    let id = Uuid::new_v4().to_string();
    job.insert("_id", &id);
    job.insert("automation_run_id", &run.id);
    job.insert("issuing_turn_id", "protected-automation-turn");
    f.state
        .db
        .collection::<Document>("machine_jobs")
        .insert_one(job.clone())
        .await
        .unwrap();
    (id, job)
}

pub(super) async fn lease_loss_without_stop(
    f: &Fixture,
    row: &AssistantConversation,
    approved: &ApprovalRequest,
    job: &str,
    model: &Model,
    node_requests: &mut tokio::sync::mpsc::Receiver<nyxid_machine::Request>,
) {
    let before = worker_documents(&f.state.db, approved).await;
    let job_before = f
        .state
        .db
        .collection::<Document>("machine_jobs")
        .find_one(doc! {"_id": job})
        .await
        .unwrap()
        .unwrap();
    let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = reached.clone();
    let hook: proposals::ResumeBoundaryHook = Arc::new(move |db, row, boundary| {
        let observed = observed.clone();
        Box::pin(async move {
            if boundary != "after_preliminary_check" {
                return;
            }
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
            db.collection::<Document>("assistant_conversations").update_one(
                engine::WorkerFence::capture(&row).unwrap().filter(),
                doc! {"$set": {"active_turn.lease_expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}},
            ).await.unwrap();
            let current = engine::get(&db, &row.user_id, &row.id).await.unwrap();
            assert!(!current.active_turn.as_ref().unwrap().stop_requested);
            assert!(proposals::retained_identity_matches(&row, &current));
            // The normal parallel watcher, not the effect path, must win.
            std::future::pending::<()>().await;
        })
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        proposals::RESUME_BOUNDARY.scope(hook, run(f, row)),
    )
    .await
    .unwrap();
    assert!(reached.load(std::sync::atomic::Ordering::SeqCst));
    let messages = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
        .await
        .unwrap();
    let reply = messages
        .iter()
        .find(|message| {
            message.role == "assistant"
                && message.turn_id == row.active_turn.as_ref().unwrap().turn_id
        })
        .unwrap();
    assert_eq!(reply.error_code.as_deref(), Some("turn_lost"));
    assert_eq!(reply.error_detail.as_ref().unwrap().stage, "lease_expiry");
    assert!(worker_documents(&f.state.db, approved).await == before);
    assert!(
        f.state
            .db
            .collection::<Document>("machine_jobs")
            .find_one(doc! {"_id": job})
            .await
            .unwrap()
            .unwrap()
            == job_before
    );
    assert!(node_requests.try_recv().is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(model.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn owner_permission_resume_watcher_lease_loss_and_scoped_human_stop() {
    outer_runner_authority_cases(&["lease", "scoped_stop"]).await;
}

#[tokio::test]
async fn owner_permission_resume_stream_checkpoint_binds_admitted_events_and_recovery() {
    for drift in [false, true] {
        let (f, row) = assistant("owner_retained_checkpoint_identity").await;
        set_task(&f, &row).await;
        let _ = sheet(&f, 200).await;
        let control = RetainedStreamControl {
            started: Arc::new(tokio::sync::Notify::new()),
            first: Arc::new(tokio::sync::Semaphore::new(0)),
            second: Arc::new(tokio::sync::Semaphore::new(0)),
            finish: Arc::new(tokio::sync::Semaphore::new(0)),
        };
        let model = controlled_model(&f, false, false, Some(control.clone())).await;
        run(&f, &row).await;
        let request = pending(&f, None).await;
        let approved = proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation"),
        )
        .await
        .unwrap();
        let admitted = event_turn(&f, &row).await;
        let turn = admitted.active_turn.as_ref().unwrap();
        let (writes, mut attempts) = tokio::sync::mpsc::unbounded_channel();
        let watcher = Arc::new(tokio::sync::Semaphore::new(0));
        let driver = crate::services::assistant_stream_checkpoint::CHECKPOINT_WRITES.scope(
            writes,
            runtime::WATCHER_GATE.scope(watcher.clone(), run(&f, &admitted)),
        );
        let observe = async {
            control.started.notified().await;
            let successful = stored(&f, &approved.id).await;
            let p = proposals::proposal(&successful).unwrap();
            assert_eq!(p.state, "completed");
            assert_eq!(
                p.owner_continuation
                    .as_ref()
                    .unwrap()
                    .released_turn_id
                    .as_deref(),
                Some(turn.turn_id.as_str())
            );
            assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
            control.first.add_permits(1);
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(5), attempts.recv())
                    .await
                    .unwrap()
                    .unwrap()
            );
            let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            let first = current
                .active_turn
                .as_ref()
                .unwrap()
                .reply
                .as_ref()
                .unwrap();
            assert_eq!(first.text, "Retained first fragment. ");
            assert_eq!(
                first.retained_authority,
                proposals::checkpoint_identity(&admitted, turn).unwrap()
            );
            if drift {
                f.state.db.collection::<Document>("assistant_conversations").update_one(
                    engine::WorkerFence::capture(&admitted).unwrap().filter(),
                    doc! {"$set": {"active_turn.events.0.text": "A replaced admitted decision proof"}},
                ).await.unwrap();
            }
            control.second.add_permits(1);
            assert_eq!(
                tokio::time::timeout(std::time::Duration::from_secs(5), attempts.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                !drift
            );
            let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            let checkpoint = current
                .active_turn
                .as_ref()
                .unwrap()
                .reply
                .as_ref()
                .unwrap();
            if drift {
                assert_eq!(checkpoint.text, "Retained first fragment. ");
                assert!(!proposals::retained_identity_matches(&admitted, &current));
                assert_ne!(
                    checkpoint.retained_authority,
                    proposals::checkpoint_identity(&current, current.active_turn.as_ref().unwrap())
                        .unwrap()
                );
                watcher.add_permits(1);
            } else {
                assert_eq!(
                    checkpoint.text,
                    "Retained first fragment. Retained second fragment."
                );
                control.finish.add_permits(1);
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            tokio::join!(driver, observe);
        })
        .await
        .unwrap();
        if drift {
            let messages = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
                .await
                .unwrap();
            assert!(
                !messages
                    .iter()
                    .any(|message| message.role == "assistant" && message.turn_id == turn.turn_id)
            );
            let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            // Force genuine current-worker lease settlement through the normal
            // watcher. It must not promote the old legitimate checkpoint now
            // bound to different events, nor any later rejected streamed text.
            let hook: proposals::ResumeBoundaryHook = Arc::new(move |db, row, boundary| {
                Box::pin(async move {
                    if boundary != "after_preliminary_check" {
                        return;
                    }
                    db.collection::<Document>("assistant_conversations").update_one(
                    engine::WorkerFence::capture(&row).unwrap().filter(),
                    doc! {"$set": {"active_turn.lease_expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}},
                ).await.unwrap();
                    std::future::pending::<()>().await;
                })
            });
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                proposals::RESUME_BOUNDARY.scope(hook, run(&f, &current)),
            )
            .await
            .unwrap();
        }
        let messages = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
            .await
            .unwrap();
        let replies: Vec<_> = messages
            .iter()
            .filter(|message| message.role == "assistant" && message.turn_id == turn.turn_id)
            .collect();
        assert_eq!(replies.len(), 1);
        if drift {
            assert_eq!(replies[0].error_code.as_deref(), Some("turn_lost"));
            assert!(replies[0].text.is_empty());
        } else {
            assert_eq!(replies[0].status, "completed");
            assert_eq!(
                replies[0].text,
                "Retained first fragment. Retained second fragment."
            );
            assert_eq!(
                proposals::proposal(&stored(&f, &approved.id).await)
                    .unwrap()
                    .delivered_turn_id
                    .as_deref(),
                Some(turn.turn_id.as_str())
            );
        }
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        drop(model);
        f.cleanup().await;
    }
}
