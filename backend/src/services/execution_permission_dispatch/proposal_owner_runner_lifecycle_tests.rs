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
    let enrolled_bind = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let terminal_bind = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed_enrolled = enrolled_bind.clone();
    let observed_terminal = terminal_bind.clone();
    let b = b.clone();
    let hook: proposals::ResumeBoundaryHook = Arc::new(move |db, row, boundary| {
        let reached = observed.clone();
        let b = b.clone();
        let enrolled = observed_enrolled.clone();
        let terminal = observed_terminal.clone();
        Box::pin(async move {
            if boundary == "binding_live_root" {
                let enabled = crate::services::execution_permission_enrollment::live_enabled(
                    &db,
                    &row.user_id,
                    None,
                )
                .await;
                assert!(matches!(enabled, Ok(true)));
                enrolled.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if boundary == "binding_terminal_root" {
                assert!(enrolled.load(std::sync::atomic::Ordering::SeqCst));
                terminal.store(true, std::sync::atomic::Ordering::SeqCst);
            }
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
                let enabled = crate::services::execution_permission_enrollment::live_enabled(
                    &db,
                    &row.user_id,
                    None,
                )
                .await
                .unwrap();
                assert_eq!(
                    proposals::proposal(&current).unwrap().state,
                    if enabled { "not_sent" } else { "cancelled" }
                );
                let active = engine::get(&db, &row.user_id, &row.id).await.unwrap();
                assert!(!active.active_turn.as_ref().unwrap().execution_local_only);
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
    if matches!(target, "binding_live_root" | "between_members")
        && crate::services::execution_permission_enrollment::live_enabled(
            &f.state.db,
            &f.owner,
            None,
        )
        .await
        .unwrap()
    {
        assert!(enrolled_bind.load(std::sync::atomic::Ordering::SeqCst));
        if target == "binding_live_root" {
            assert!(terminal_bind.load(std::sync::atomic::Ordering::SeqCst));
        }
    }
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

async fn owner_history(f: &Fixture, id: &str) -> Value {
    let token = crate::crypto::jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&f.owner).unwrap(),
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let router = Router::new()
        .route(
            "/api/v1/assistant/nyxagent/conversations/{id}",
            axum::routing::get(crate::handlers::assistant_nyxagent::history),
        )
        .with_state(f.state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/assistant/nyxagent/conversations/{id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn owner_permission_resume_stream_checkpoint_binds_admitted_events_and_recovery() {
    for drift in [false, true] {
        let (f, row) = assistant("owner_retained_checkpoint_identity").await;
        set_task(&f, &row).await;
        let _ = sheet(&f, 200).await;
        let control = RetainedStreamControl::new(false);
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
            let history = owner_history(&f, &row.id).await;
            let projected = &history["conversation"]["active_turn"]["reply"];
            assert_eq!(projected["text"], first.text);
            assert_eq!(projected["cursor"], first.cursor);
            assert_eq!(projected["message_id"], first.message_id);
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
                let history = owner_history(&f, &row.id).await;
                let active = &history["conversation"]["active_turn"];
                assert_eq!(active["turn_id"], turn.turn_id);
                assert!(
                    active.get("reply").is_none(),
                    "history must withhold obsolete text and cursor together"
                );
                assert!(!history.to_string().contains("Retained first fragment."));
                assert!(!history.to_string().contains("Retained second fragment."));
                watcher.add_permits(1);
            } else {
                assert_eq!(
                    checkpoint.text,
                    "Retained first fragment. Retained second fragment."
                );
                let history = owner_history(&f, &row.id).await;
                assert_eq!(
                    history["conversation"]["active_turn"]["reply"]["text"],
                    checkpoint.text
                );
                assert_eq!(
                    history["conversation"]["active_turn"]["reply"]["cursor"],
                    checkpoint.cursor
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
        let history = owner_history(&f, &row.id).await;
        assert!(history["conversation"]["active_turn"].is_null());
        let rendered = history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["turn_id"] == turn.turn_id && message["role"] == "assistant")
            .unwrap();
        assert_eq!(rendered["text"], replies[0].text);
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        drop(model);
        f.cleanup().await;
    }
}

pub(super) async fn live_mixed_unsent(
    f: &Fixture,
    admitted: &AssistantConversation,
    a: &ApprovalRequest,
    b: &ApprovalRequest,
    model: &Model,
    relay: &ReplyRelay,
    a_first: bool,
) {
    let turn = admitted.active_turn.as_ref().unwrap();
    let before_a = stored(f, &a.id).await;
    let old_a = proposals::proposal(&before_a).unwrap();
    let expected_delivery = old_a
        .delivered_turn_id
        .as_deref()
        .unwrap_or(turn.turn_id.as_str());
    let model_count = 1 + usize::from(old_a.delivered_turn_id.is_some());
    let a_notice = proposals::result_for_human(&f.state, &before_a)
        .await
        .unwrap()
        .unwrap();
    let expected_b =
        "The operation was not sent because its authority or source is no longer available.";
    let expected = if a_first {
        format!("{a_notice}\n\n{expected_b}")
    } else {
        format!("{expected_b}\n\n{a_notice}")
    };
    let b_id = b.id.clone();
    let hook: proposals::ResumeBoundaryHook = Arc::new(move |db, row, boundary| {
        let b_id = b_id.clone();
        Box::pin(async move {
            if boundary == "between_members" && !a_first {
                let current = engine::get(&db, &row.user_id, &admitted.id).await.unwrap();
                let active = current.active_turn.unwrap();
                assert_eq!(active.execution_local_retirements, Some(vec![b_id]));
                assert!(
                    !active.execution_local_only,
                    "live member retirement is provisional"
                );
            }
        })
    });
    proposals::RESUME_BOUNDARY
        .scope(hook, run(f, admitted))
        .await;
    let messages = engine::messages(&f.state.db, &f.owner, &admitted.id, 30, None)
        .await
        .unwrap();
    let replies: Vec<_> = messages
        .iter()
        .filter(|m| m.role == "assistant" && m.turn_id == turn.turn_id)
        .collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].status, "completed");
    assert_eq!(replies[0].text, expected);
    confirmed_reply(f, &turn.turn_id, &replies[0].id, relay, &expected).await;
    let after_a = stored(f, &a.id).await;
    let new_a = proposals::proposal(&after_a).unwrap();
    assert_eq!(new_a.effect_id, old_a.effect_id);
    assert_eq!(new_a.claim_id, old_a.claim_id);
    assert_eq!(
        new_a.owner_continuation.as_ref().unwrap().released_turn_id,
        old_a.owner_continuation.as_ref().unwrap().released_turn_id
    );
    assert_eq!(new_a.delivered_turn_id.as_deref(), Some(expected_delivery));
    let after_b = stored(f, &b.id).await;
    let new_b = proposals::proposal(&after_b).unwrap();
    assert_eq!(new_b.state, "not_sent");
    assert_eq!(
        new_b.delivered_turn_id.as_deref(),
        Some(turn.turn_id.as_str())
    );
    assert!(new_b.owner_continuation.as_ref().unwrap().blocked);
    assert!(
        new_b
            .owner_continuation
            .as_ref()
            .unwrap()
            .released_turn_id
            .is_none()
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert_eq!(model.calls.lock().unwrap().len(), model_count);
    let wire_count = relay.receipts.lock().unwrap().len();
    run(f, admitted).await;
    proposals::sweep(&f.state).await.unwrap();
    assert_eq!(
        proposals::proposal(&stored(f, &a.id).await)
            .unwrap()
            .delivered_turn_id,
        new_a.delivered_turn_id
    );
    assert_eq!(
        proposals::proposal(&stored(f, &b.id).await)
            .unwrap()
            .delivered_turn_id,
        new_b.delivered_turn_id
    );
    let messages = engine::messages(&f.state.db, &f.owner, &admitted.id, 30, None)
        .await
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.role == "assistant" && m.turn_id == turn.turn_id)
            .count(),
        1
    );
    confirmed_reply(f, &turn.turn_id, &replies[0].id, relay, &expected).await;
    assert_eq!(relay.receipts.lock().unwrap().len(), wire_count);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert_eq!(model.calls.lock().unwrap().len(), model_count);
}

#[tokio::test]
async fn owner_permission_resume_live_mixed_unsent_delivers_both_receipts_once() {
    expired_channel_observer_cases(&["live_unsent", "live_unsent_delivered"], None, false).await;
}

#[tokio::test]
async fn owner_permission_resume_enrolled_bind_expiry_preserves_batch_receipt() {
    expired_channel_observer_cases(&["expired_delivered"], Some("binding_live_root"), false).await;
    for boundary in ["after_preliminary_check", "between_members"] {
        expired_channel_observer_cases(&["expired_delivered"], Some(boundary), false).await;
    }
    expired_channel_observer_cases(&["live"], None, false).await;
}

#[tokio::test]
async fn owner_permission_resume_live_rollout_off_retirement_requires_verified_actor() {
    for channel in [false, true] {
        for drift in ["valid", "relationship", "raw_key", "key", "account"] {
            let (f, row) = if channel {
                let (f, row, _) =
                    channel_turn_transport("owner_live_disabled_actor", "direct").await;
                (f, row)
            } else {
                assistant("owner_live_disabled_actor").await
            };
            set_task(&f, &row).await;
            let _ = sheet(&f, 200).await;
            let model = model(&f, false).await;
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
            let mut credential = load_credential(&f, &admitted).await;
            disable_rollout(&f).await;
            let before = worker_documents(&f.state.db, &approved).await;
            let relationship = f
                .state
                .db
                .collection::<Document>("assistant_agent_credentials")
                .find_one(doc! {"conversation_id": &row.id, "user_id": &f.owner})
                .await
                .unwrap()
                .unwrap();
            let key = f
                .state
                .db
                .collection::<Document>("api_keys")
                .find_one(doc! {"_id": &f.key})
                .await
                .unwrap()
                .unwrap();
            let account = f
                .state
                .db
                .collection::<Document>("users")
                .find_one(doc! {"_id": &f.owner})
                .await
                .unwrap()
                .unwrap();
            match drift {
                "relationship" => {
                    f.state
                        .db
                        .collection::<Document>("assistant_agent_credentials")
                        .delete_one(doc! {"_id": relationship.get("_id").unwrap()})
                        .await
                        .unwrap();
                }
                "raw_key" => {
                    credential.raw_key = zeroize::Zeroizing::new("fixture-invalid-key".into())
                }
                "key" => {
                    f.state
                        .db
                        .collection::<Document>("api_keys")
                        .update_one(doc! {"_id": &f.key}, doc! {"$set": {"is_active": false}})
                        .await
                        .unwrap();
                }
                "account" => {
                    f.state
                        .db
                        .collection::<Document>("users")
                        .update_one(doc! {"_id": &f.owner}, doc! {"$set": {"is_active": false}})
                        .await
                        .unwrap();
                }
                _ => {}
            }
            if drift != "valid" {
                let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let observed = reached.clone();
                let hook: proposals::ResumeBoundaryHook = Arc::new(move |_, _, boundary| {
                    let observed = observed.clone();
                    Box::pin(async move {
                        if boundary == "before_unsent_retirement" {
                            observed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    })
                });
                let egress = crate::handlers::assistant_nyxagent::http_egress(
                    f.state.clone(),
                    test_utils::test_auth_user(&f.owner),
                );
                let error = proposals::RESUME_BOUNDARY
                    .scope(
                        hook,
                        execute_using_egress(&f, &admitted, &mut credential, egress.as_ref()),
                    )
                    .await
                    .unwrap_err();
                assert!(reached.load(std::sync::atomic::Ordering::SeqCst));
                assert_eq!(
                    (error.code, error.stage),
                    ("cancelled", "permission_resume")
                );
                assert!(worker_documents(&f.state.db, &approved).await == before);
                assert!(f.server.received_requests().await.unwrap().is_empty());
                assert_eq!(model.calls.lock().unwrap().len(), 1);
                f.state
                    .db
                    .collection::<Document>("assistant_agent_credentials")
                    .replace_one(doc! {"_id": relationship.get("_id").unwrap()}, relationship)
                    .upsert(true)
                    .await
                    .unwrap();
                f.state
                    .db
                    .collection::<Document>("api_keys")
                    .replace_one(doc! {"_id": &f.key}, key)
                    .await
                    .unwrap();
                f.state
                    .db
                    .collection::<Document>("users")
                    .replace_one(doc! {"_id": &f.owner}, account)
                    .await
                    .unwrap();
            }
            // The intact current worker settles through the real outer runner;
            // disabled rollout alone does not discard an authorized local receipt.
            run(&f, &admitted).await;
            let saved = stored(&f, &approved.id).await;
            let p = proposals::proposal(&saved).unwrap();
            assert_eq!(p.state, "cancelled");
            assert_eq!(
                p.delivered_turn_id.as_deref(),
                Some(admitted.active_turn.as_ref().unwrap().turn_id.as_str())
            );
            assert!(p.owner_continuation.as_ref().unwrap().blocked);
            assert!(
                worker_documents(&f.state.db, &approved)
                    .await
                    .1
                    .get_bool("revoked")
                    .unwrap()
            );
            let history = owner_history(&f, &row.id).await;
            let reply = history["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|message| {
                    message["role"] == "assistant"
                        && message["turn_id"] == admitted.active_turn.as_ref().unwrap().turn_id
                })
                .unwrap();
            assert_eq!(reply["status"], "completed");
            assert_eq!(
                reply["text"],
                "The operation was not sent because its authority or task is no longer available."
            );
            assert!(f.server.received_requests().await.unwrap().is_empty());
            assert_eq!(model.calls.lock().unwrap().len(), 1);
            drop(model);
            f.cleanup().await;
        }
    }
}

#[tokio::test]
async fn owner_history_ordinary_checkpoint_preserves_text_and_cursor() {
    let (f, row) = assistant("owner_history_ordinary_checkpoint").await;
    set_task(&f, &row).await;
    let control = RetainedStreamControl::new(true);
    let model = controlled_model(&f, false, false, Some(control.clone())).await;
    let (writes, mut attempts) = tokio::sync::mpsc::unbounded_channel();
    let driver = crate::services::assistant_stream_checkpoint::CHECKPOINT_WRITES
        .scope(writes, run(&f, &row));
    let observe = async {
        control.started.notified().await;
        control.first.add_permits(1);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), attempts.recv())
                .await
                .unwrap()
                .unwrap()
        );
        let saved = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
        let checkpoint = saved.active_turn.as_ref().unwrap().reply.as_ref().unwrap();
        assert!(checkpoint.retained_authority.is_none());
        let history = owner_history(&f, &row.id).await;
        assert_eq!(
            history["conversation"]["active_turn"]["reply"]["text"],
            checkpoint.text
        );
        assert_eq!(
            history["conversation"]["active_turn"]["reply"]["cursor"],
            checkpoint.cursor
        );
        control.second.add_permits(1);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), attempts.recv())
                .await
                .unwrap()
                .unwrap()
        );
        control.finish.add_permits(1);
    };
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        tokio::join!(driver, observe);
    })
    .await
    .unwrap();
    let history = owner_history(&f, &row.id).await;
    assert!(history["conversation"]["active_turn"].is_null());
    assert!(
        history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "assistant"
                && m["text"] == "Retained first fragment. Retained second fragment.")
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(model.calls.lock().unwrap().len(), 1);
    drop(model);
    f.cleanup().await;
}

#[tokio::test]
async fn owner_permission_resume_survivor_transfers_only_proven_checkpoint_and_window() {
    use crate::handlers::assistant_nyxagent::survivor;
    use crate::models::assistant_conversation::{
        ChatRestartSource, EngineWorkerInstance, ReattachDeployment,
    };
    use crate::services::{assistant_engine_worker as workers, assistant_turn_restart as restart};

    for drift in [false, true] {
        let (f, row, _) = channel_turn_transport("owner_retained_survivor_proof", "direct").await;
        set_task(&f, &row).await;
        let (relay, _) = reply_transport(&f, &row).await;
        let _ = sheet(&f, 200).await;
        let control = RetainedStreamControl::new(false);
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
        let predecessor = Box::leak(Box::new(EngineWorkerInstance {
            id: Uuid::new_v4().to_string(),
            hostname: None,
        }));
        let mut admitted = workers::TEST_WORKER
            .scope(predecessor, event_turn(&f, &row))
            .await;
        crate::services::security_readiness_service::observe_test_handoff_readiness(
            &f.state.db,
            true,
        )
        .await;
        assert!(crate::services::security_readiness_service::chat_restart_enrollment_ready());
        let turn_id = admitted.active_turn.as_ref().unwrap().turn_id.clone();
        let initiating = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
            .await
            .unwrap()
            .into_iter()
            .find(|message| message.turn_id == turn_id && message.role == "event")
            .unwrap();
        let service = f
            .state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                "downstream_services",
            )
            .find_one(doc! {"slug": engine::SERVICE_SLUG})
            .await
            .unwrap()
            .unwrap();
        let start = engine::TurnStart {
            origin: TurnOrigin::Event,
            ..engine::TurnStart::from(&engine::TurnRequest {
                conversation_id: Some(row.id.clone()),
                text: String::new(),
                acknowledgement_id: None,
                retry_of: None,
                attachment_ids: vec![],
                agent_id: None,
                model: None,
                access_mode: None,
            })
        };
        engine::enroll_restart(&mut admitted, &start, &initiating.id, ReattachDeployment {
            destination_hash: crate::services::platform_manager_contract::NyxAgentDestination::for_service(&service.id, &service.base_url).0,
            limits: serde_json::from_value(json!({"protocol":"nyxagent-reattach-v1","credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15},"max_events":64,"recent_events":32,"max_event_bytes":65536,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32})).unwrap(),
        }, false);
        let source = admitted
            .active_turn
            .as_ref()
            .unwrap()
            .restart_admission
            .as_ref()
            .unwrap();
        assert_eq!(source.version, 4);
        assert!(matches!(
            source.chat_source.as_deref(),
            Some(ChatRestartSource::Wake { .. })
        ));
        f.state
            .db
            .collection::<AssistantConversation>("assistant_conversations")
            .replace_one(doc! {"_id": &row.id}, &admitted)
            .await
            .unwrap();
        let state = f.state.clone();
        let running = admitted.clone();
        let credential = load_credential(&f, &admitted).await;
        let permit = f
            .state
            .direct_chat_limiter
            .try_acquire(&f.owner)
            .await
            .unwrap();
        let auth = test_utils::test_auth_user(&f.owner);
        let (writes, mut attempts) = tokio::sync::mpsc::unbounded_channel();
        let (sender, _) = tokio::sync::broadcast::channel(256);
        let driver = tokio::spawn(workers::TEST_WORKER.scope(
            predecessor,
            crate::services::assistant_stream_checkpoint::CHECKPOINT_WRITES.scope(
                writes,
                crate::handlers::assistant_nyxagent::run_turn(
                    state,
                    auth,
                    running,
                    TASK.into(),
                    credential,
                    Some(crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY),
                    permit,
                    runtime::Events {
                        sender,
                        cursor: 0,
                        reset_notified: false,
                        checkpoint: None,
                    },
                ),
            ),
        ));
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            control.started.notified(),
        )
        .await
        .unwrap();
        control.first.add_permits(1);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), attempts.recv())
                .await
                .unwrap()
                .unwrap()
        );
        let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
        let checkpoint = current
            .active_turn
            .as_ref()
            .unwrap()
            .reply
            .as_ref()
            .unwrap()
            .clone();
        assert_eq!(checkpoint.text, "Retained first fragment. ");
        assert!(checkpoint.upstream.is_some());
        let active = current.active_turn.as_ref().unwrap();
        let window = active
            .restart_admission
            .as_ref()
            .unwrap()
            .window
            .as_ref()
            .unwrap();
        assert!(window.input.is_some());
        assert_eq!(
            window.retained_authority,
            proposals::window_identity(&current, active).unwrap()
        );
        assert!(window.retained_authority.is_some());
        assert_eq!(checkpoint.upstream.as_ref().unwrap().window_key, window.key);
        assert_eq!(
            owner_history(&f, &row.id).await["conversation"]["active_turn"]["reply"]["text"],
            checkpoint.text
        );
        let success = stored(&f, &approved.id).await;
        let p = proposals::proposal(&success).unwrap();
        assert_eq!(p.state, "completed");
        assert_eq!(
            p.owner_continuation
                .as_ref()
                .unwrap()
                .released_turn_id
                .as_deref(),
            Some(turn_id.as_str())
        );
        assert!(p.delivered_turn_id.is_none());
        assert!(
            restart::yield_on_shutdown(&f.state.db, &current, &predecessor.id)
                .await
                .unwrap()
        );
        driver.abort();
        let _ = driver.await;
        let offered = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
        let offered_turn = offered.active_turn.as_ref().unwrap();
        assert!(
            crate::services::assistant_stream_checkpoint::window_matches_snapshot(
                &offered,
                offered_turn
            )
        );
        let transferred = crate::services::assistant_stream_checkpoint::reply_for_snapshot(
            &offered,
            offered_turn,
        )
        .unwrap();
        assert_eq!(transferred.text, checkpoint.text);
        assert_eq!(
            serde_json::to_value(&transferred.upstream).unwrap(),
            serde_json::to_value(&checkpoint.upstream).unwrap()
        );
        assert_ne!(
            transferred.retained_authority,
            checkpoint.retained_authority
        );
        let successor = workers::instance();
        assert_ne!(successor.id, predecessor.id);
        if drift {
            let before = worker_documents(&f.state.db, &approved).await;
            let proof = offered_turn.events[0].text.clone();
            f.state.db.collection::<Document>("assistant_conversations").update_one(
                engine::WorkerFence::capture(&offered).unwrap().filter(),
                doc! {"$set": {"active_turn.events.0.text": "Changed same-ID decision after the real release"}},
            ).await.unwrap();
            assert!(
                survivor::take_one_for_test(&f.state, &offered, successor)
                    .await
                    .is_err()
            );
            let changed = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert!(
                survivor::take_one_for_test(&f.state, &changed, successor)
                    .await
                    .is_err()
            );
            let history = owner_history(&f, &row.id).await;
            assert!(
                history["conversation"]["active_turn"]
                    .get("reply")
                    .is_none()
            );
            assert!(!history.to_string().contains("Retained first fragment."));
            // Even without a checkpoint, the encrypted replay window retains
            // the original authority. Same IDs are insufficient to replay it.
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    engine::WorkerFence::capture(&changed).unwrap().filter(),
                    doc! {"$unset": {"active_turn.reply": ""}},
                )
                .await
                .unwrap();
            let no_reply = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert!(
                survivor::take_one_for_test(&f.state, &no_reply, successor)
                    .await
                    .is_err()
            );
            let after = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert_eq!(
                after.active_turn.as_ref().unwrap().execution_attempt,
                offered_turn.execution_attempt
            );
            assert_eq!(
                after.active_turn.as_ref().unwrap().worker_instance,
                offered_turn.worker_instance
            );
            assert!(
                after
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .restart_offer
                    .as_ref()
                    .unwrap()
                    .claimed_by
                    .is_none()
            );
            assert!(worker_documents(&f.state.db, &approved).await == before);
            assert_eq!(control.lookups.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert_eq!(control.cancels.load(std::sync::atomic::Ordering::SeqCst), 0);
            // Restore the exact genuine proof and exercise the real terminal
            // claim-deadline branch, including its cancel/finish/after_turn.
            f.state.db.collection::<Document>("assistant_conversations").update_one(
                engine::WorkerFence::capture(&after).unwrap().filter(), doc! {"$set": {
                    "active_turn.events.0.text": proof,
                    "active_turn.reply": bson::to_bson(transferred).unwrap(),
                    "active_turn.queue_deadline": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1)),
                }},
            ).await.unwrap();
            let expired = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert_eq!(runtime::expired_handoff(&expired).unwrap().stage, "queue");
            assert!(
                survivor::take_one_for_test(&f.state, &expired, successor)
                    .await
                    .unwrap()
            );
            assert_eq!(control.cancels.load(std::sync::atomic::Ordering::SeqCst), 1);
        } else {
            assert!(
                survivor::take_one_for_test(&f.state, &offered, successor)
                    .await
                    .unwrap()
            );
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                control.lookup_started.notified(),
            )
            .await
            .unwrap();
            let claimed = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            let active = claimed.active_turn.as_ref().unwrap();
            assert_eq!(active.worker_instance.as_ref().unwrap().id, successor.id);
            assert_ne!(active.execution_attempt, offered_turn.execution_attempt);
            let recovered =
                crate::services::assistant_stream_checkpoint::reply_for_snapshot(&claimed, active)
                    .unwrap();
            assert_eq!(recovered.text, checkpoint.text);
            assert_eq!(
                serde_json::to_value(&recovered.upstream).unwrap(),
                serde_json::to_value(&checkpoint.upstream).unwrap()
            );
            assert_eq!(recovered.cursor, checkpoint.cursor);
            assert_ne!(recovered.retained_authority, transferred.retained_authority);
            let history = owner_history(&f, &row.id).await;
            assert_eq!(
                history["conversation"]["active_turn"]["reply"]["text"],
                checkpoint.text
            );
            assert_eq!(
                history["conversation"]["active_turn"]["reply"]["cursor"],
                checkpoint.cursor
            );
            assert!(
                crate::services::assistant_stream_checkpoint::window_matches_snapshot(
                    &claimed, active
                )
            );
            control.lookup_gate.add_permits(1);
        }
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                if engine::get(&f.state.db, &f.owner, &row.id)
                    .await
                    .unwrap()
                    .active_turn
                    .is_none()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let history = owner_history(&f, &row.id).await;
        assert!(history["conversation"]["active_turn"].is_null());
        let messages = engine::messages(&f.state.db, &f.owner, &row.id, 30, None)
            .await
            .unwrap();
        let finals: Vec<_> = messages
            .iter()
            .filter(|message| message.turn_id == turn_id && message.role == "assistant")
            .collect();
        assert_eq!(finals.len(), 1);
        let final_reply = finals[0];
        let expected = if drift {
            "Retained first fragment. "
        } else {
            "Retained first fragment. Retained second fragment."
        };
        assert_eq!(final_reply.text, expected);
        if drift {
            assert_eq!(final_reply.status, "failed");
            assert_eq!(final_reply.error_code.as_deref(), Some("outcome_unknown"));
            assert_eq!(
                final_reply.error_detail.as_ref().unwrap().stage,
                "claim_deadline"
            );
        } else {
            assert_eq!(final_reply.status, "completed");
            assert_eq!(control.lookups.load(std::sync::atomic::Ordering::SeqCst), 1);
            confirmed_reply(&f, &turn_id, &final_reply.id, &relay, expected).await;
        }
        let projected = history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["id"] == final_reply.id)
            .unwrap();
        assert_eq!(projected["text"], expected);
        let result = stored(&f, &approved.id).await;
        let final_p = proposals::proposal(&result).unwrap();
        assert_eq!(final_p.effect_id, p.effect_id);
        assert_eq!(final_p.claim_id, p.claim_id);
        assert_eq!(
            final_p
                .owner_continuation
                .as_ref()
                .unwrap()
                .released_turn_id,
            p.owner_continuation.as_ref().unwrap().released_turn_id
        );
        assert_eq!(
            final_p.delivered_turn_id.as_deref(),
            if drift { None } else { Some(turn_id.as_str()) }
        );
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert!(
            !survivor::take_one_for_test(&f.state, &offered, successor)
                .await
                .unwrap()
        );
        assert_eq!(model.calls.lock().unwrap().len(), 2);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        drop(model);
        f.cleanup().await;
    }
}
