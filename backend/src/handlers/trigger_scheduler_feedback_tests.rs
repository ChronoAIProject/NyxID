// Focused PR feedback regressions. Included beside the existing scheduler
// fixtures; all database names and upstream endpoints remain fixture-owned.

#[tokio::test]
async fn automation_legacy_flag_off_preserves_create_edit_schedule_webhook_and_group() {
    const CHILD: &str = "NYXID_AUTOMATION_LEGACY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // CI intentionally enables v2 for its existing coverage. Execute the
        // real legacy paths in isolated processes, with no process-global env
        // mutation and no skipped/ignored test entrypoint.
        for value in [None, Some("false")] {
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "handlers::trigger_scheduler::tests::automation_legacy_flag_off_preserves_create_edit_schedule_webhook_and_group",
                "--nocapture", "--test-threads=1"])
                .env(CHILD, "1").env_remove("AUTOMATION_CONTINUATION_V2_ENABLED").kill_on_drop(true);
            if let Some(value) = value {
                command.env("AUTOMATION_CONTINUATION_V2_ENABLED", value);
            }
            let output = tokio::time::timeout(std::time::Duration::from_secs(90), command.output())
                .await
                .unwrap()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "legacy child {value:?}: {stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(stdout.contains("1 passed; 0 failed"));
            assert!(stdout.contains("legacy ingress/group dispatch verified"));
        }
        return;
    }
    Box::pin(feedback_legacy_ingress()).await;
}

async fn feedback_legacy_ingress() {
    assert!(!continuation_v2::enabled());
    let (state, calls, server) = setup("automation_legacy_ingress").await;
    state
        .db
        .collection::<Document>(continuation_v2::MIGRATIONS)
        .delete_many(doc! {})
        .await
        .unwrap();
    let agent = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let created = automation(&state, &agent.id, None).await;
    assert_eq!(created.continuation_protocol_version, 0);
    let edited = trigger_service::update(
        &state.db,
        &state.encryption_keys,
        &created,
        trigger_service::UpdateInput {
            label: Some("Legacy edited".into()),
            status: None,
            delivery: None,
            schedule: None,
            overlap: None,
        },
    )
    .await
    .unwrap()
    .trigger;
    assert_eq!(edited.label, "Legacy edited");
    assert_eq!(edited.continuation_protocol_version, 0);
    make_due(&state, &edited).await;
    Box::pin(tick(&state)).await.unwrap();
    let scheduled = completed(&state, &edited).await;
    assert_eq!(scheduled.outcome, RunOutcome::Completed);
    assert_eq!(scheduled.continuation_protocol_version, 0);
    assert_eq!(calls.lock().await.len(), 1);

    let hook = trigger_service::create(
        &state.db,
        &state.encryption_keys,
        trigger_service::CreateInput {
            source: TriggerSource::Webhook,
            setup_watch_id: None,
            schedule: None,
            overlap: OverlapPolicy::Skip,
            user_id: OWNER.into(),
            label: "Legacy webhook".into(),
            user_service_id: None,
            verification: TriggerVerification::Token {
                location: crate::models::trigger::TriggerTokenLocation::Bearer,
            },
            delivery: edited.delivery.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(hook.trigger.continuation_protocol_version, 0);
    for expected in ["accepted", "duplicate"] {
        let request = axum::http::Request::builder()
            .header(
                "authorization",
                format!("Bearer {}", hook.raw_secret.as_str()),
            )
            .body(axum::body::Body::from(
                r#"{"event_id":"legacy-hook","body":"untrusted payload"}"#,
            ))
            .unwrap();
        let Json(response) = crate::handlers::trigger_webhooks::receive_trigger(
            axum::extract::State(state.clone()),
            axum::extract::Path(hook.trigger.id.clone()),
            axum::extract::Query(Default::default()),
            request,
        )
        .await
        .unwrap();
        assert_eq!(response.status, expected);
    }
    Box::pin(tick(&state)).await.unwrap();
    let hooked = completed(&state, &hook.trigger).await;
    assert_eq!(hooked.outcome, RunOutcome::Completed);
    assert_eq!(hooked.continuation_protocol_version, 0);
    assert_eq!(calls.lock().await.len(), 2);

    Box::pin(async {
        // Agent-authored posts follow explicitly mentioned members, not plain
        // notes. Hold that member's real response until the follower is read.
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let signal = entered.clone();
        let gate = release.clone();
        let sink = calls.clone();
        let upstream = Router::new().route(
            "/v1/responses",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let signal = signal.clone();
                let gate = gate.clone();
                let sink = sink.clone();
                async move {
                    let is_member = {
                        let mut calls = sink.lock().await;
                        calls.push(Call {
                            authorization: headers["authorization"].to_str().unwrap().into(),
                            body,
                        });
                        calls.len() == 3 // The schedule and webhook completed first.
                    };
                    if is_member {
                        signal.notify_one();
                        gate.acquire().await.unwrap().forget();
                    }
                    let session = format!("conv_{}", Uuid::new_v4().simple());
                    let response = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
                    let stream = async_stream::stream! {
                        yield Ok::<_, Infallible>(Event::default().data(json!({
                            "type":"response.completed", "sequence_number":1,
                            "response":{"id":response,"conversation":{"id":session},
                            "status":"completed","output":[{"type":"message","role":"assistant",
                            "content":[{"type":"output_text","text":"Legacy group work finished"}]}]}
                        }).to_string()));
                    };
                    Sse::new(stream).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let group_server =
            tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        state.db.collection::<Document>(SERVICES).update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        ).await.unwrap();

        // Keep the real run identity for confirmation/settlement, but preserve
        // legacy group-follow semantics instead of passing it to the v2 fence.
        let (_, run, row) = Box::pin(completion_coordinator(&state)).await;
        assert_eq!(run.continuation_protocol_version, 0);
        assert!(row.coordinator_run_ids.is_empty());
        let member = specialist(&state).await;
        let group = completion_groups::create(
            &state.db,
            OWNER,
            "Legacy group",
            &[agent.id.clone(), member.id.clone()],
            "user",
        )
        .await
        .unwrap();
        let chat = crate::services::assistant_acknowledgement_service::for_key(
            &state.db,
            OWNER,
            Some(&row.credential_api_key_id),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(chat.trigger_run_id.as_deref(), Some(run.id.as_str()));
        let (posted, error) = Box::pin(super::super::assistant_team::execute_tool(
            &state,
            &chat,
            "nyxid__post_to_group",
            &json!({"group":group.id,"text":format!("@{} Legacy group note", member.name)}),
        ))
        .await;
        assert!(!error, "{posted}");
        assert!(posted["posted"].is_i64());
        assert_eq!(posted["addressed_agent_ids"], json!([member.id]));
        tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
            .await
            .expect("mentioned specialist reached the actual upstream");
        let group = completion_groups::get(&state.db, OWNER, &group.id)
            .await
            .unwrap();
        assert_eq!(group.followers.len(), 1);
        assert_eq!(group.followers[0].conversation_id, row.id);
        assert!(group.followers[0].trigger_run_id.is_none());
        Box::pin(completion_reply(&state, &row, &run)).await;
        assert_eq!(
            completion_load(&state, &run.id).await.outcome,
            RunOutcome::Completed
        );
        release.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let group = completion_groups::get(&state.db, OWNER, &group.id)
                    .await
                    .unwrap();
                let call_count = calls.lock().await.len();
                let home = engine::get(&state.db, OWNER, &row.id).await.unwrap();
                if group.followers.is_empty()
                    && call_count == 4
                    && home.active_turn.is_none()
                    && home.pending_events.is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("legacy group reply and follower wake both completed");
        let messages = completion_groups::messages(&state.db, OWNER, &group.id, 100, None)
            .await
            .unwrap();
        assert!(messages.iter().any(|message| {
            message.agent_id.as_deref() == Some(member.id.as_str())
                && message.text == "Legacy group work finished"
        }));
        group_server.abort();
    })
    .await;
    let persisted = state
        .db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! {"_id":&edited.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.continuation_protocol_version, 0);
    state
        .db
        .collection::<Document>(continuation_v2::MIGRATIONS)
        .insert_one(
            doc! {"_id":continuation_v2::MIGRATION_ID,"completed_at":schedules::date(Utc::now())},
        )
        .await
        .unwrap();
    let pending = schedules::enqueue_now(&state.db, &persisted).await.unwrap();
    state
        .db
        .collection::<Document>(RUNS)
        .update_one(
            doc! {"_id":&pending.id},
            doc! {"$set":{"continuation_protocol_version":2}},
        )
        .await
        .unwrap();
    let home = schedules::run_thread(&state.db, &state.encryption_keys, &pending, &persisted)
        .await
        .unwrap();
    let claim = schedules::claim(&state.db, &pending.id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(Box::pin(engine::begin_turn(&state.db, OWNER,
        completion_start(&home.id, TurnOrigin::Trigger, Some(schedules::TurnClaim {
            run_id: pending.id.clone(), fence: claim.get_str("fence").unwrap().into(),
            trigger_updated_at: persisted.updated_at, continuation: false,
        })), &state.encryption_keys)).await,
        Err(AppError::Conflict(reason)) if reason == "automation_continuation_v2_not_ready"));
    assert_eq!(
        completion_load(&state, &pending.id).await.outcome,
        RunOutcome::Pending
    );
    assert!(
        engine::get(&state.db, OWNER, &home.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    // Disabling does not admit persisted v2 triggers or runs, even with a
    // completed migration marker. No accidental downgrade of saved authority.
    let mut v2 = persisted.clone();
    v2.continuation_protocol_version = 2;
    state
        .db
        .collection::<Trigger>(TRIGGERS)
        .replace_one(doc! {"_id":&v2.id}, &v2)
        .await
        .unwrap();
    assert!(schedules::enqueue_now(&state.db, &v2).await.is_err());
    assert!(
        trigger_service::update(
            &state.db,
            &state.encryption_keys,
            &v2,
            trigger_service::UpdateInput {
                label: Some("Cannot bypass readiness".into()),
                status: None,
                delivery: None,
                schedule: None,
                overlap: None
            }
        )
        .await
        .is_err()
    );
    println!("legacy ingress/group dispatch verified");
    server.abort();
}

#[tokio::test]
async fn automation_direct_tools_wait_retry_resume_once_with_independent_allowance() {
    for tool in ["message_subagent", "spawn_subagent"] {
        Box::pin(feedback_direct_tool(tool)).await;
    }
}

async fn feedback_direct_tool(tool: &str) {
    use crate::models::assistant_direct_handoff::{COLLECTION_NAME as HANDOFFS, DirectHandoff};
    let (state, calls, old_server) = setup(&format!("automation_direct_{tool}")).await;
    old_server.abort();
    state.db.collection::<Document>(crate::models::assistant_settings::COLLECTION_NAME)
        .update_one(doc! {"_id":OWNER},doc! {"$set":{"max_auto_continuations":0},"$setOnInsert":{"updated_at":schedules::date(Utc::now())}}).upsert(true).await.unwrap();
    let (_, run, row) = Box::pin(completion_coordinator(&state)).await;
    assert_eq!(
        run.continuation_limit,
        continuation_v2::AUTOMATION_CONTINUATION_LIMIT
    );
    assert!(run.continuation_limit > 0);
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let upstream_state = state.clone();
    let signal = entered.clone();
    let gate = release.clone();
    let sink = calls.clone();
    let upstream = Router::new().route("/v1/responses",post(move |headers:HeaderMap,Json(body):Json<Value>| {
        let state=upstream_state.clone(); let signal=signal.clone(); let gate=gate.clone(); let sink=sink.clone();
        async move {
            sink.lock().await.push(Call { authorization:headers["authorization"].to_str().unwrap().into(),body });
            let active = state.db.collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(doc! {"user_id":OWNER,"role":"subagent","active_turn":{"$ne":null}}).await.unwrap();
            if active.is_some() { signal.notify_one(); gate.acquire().await.unwrap().forget(); }
            let session=format!("conv_{}",Uuid::new_v4().simple());
            let response=format!("resp_{}_{}",&session[5..],Uuid::new_v4().simple());
            let stream=async_stream::stream! { yield Ok::<_,Infallible>(Event::default().data(json!({
                "type":"response.completed","sequence_number":1,"response":{"id":response,
                "conversation":{"id":session},"status":"completed","output":[{"type":"message",
                "role":"assistant","content":[{"type":"output_text","text":"Direct specialist completed"}]}]}
            }).to_string())); };
            Sse::new(stream).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    state
        .db
        .collection::<Document>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    let chat = crate::services::assistant_acknowledgement_service::for_key(
        &state.db,
        OWNER,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let args = if tool == "spawn_subagent" {
        json!({"name":"directhelper","description":"Direct lifecycle fixture","services":[],"task":"Do the exact task"})
    } else {
        let agent = specialist(&state).await;
        json!({"subagent":agent.id,"text":"Do the exact task"})
    };
    let (started, error) = Box::pin(super::super::assistant_team::execute_tool(
        &state,
        &chat,
        &format!("nyxid__{tool}"),
        &args,
    ))
    .await;
    assert!(!error, "{started}");
    let started = if tool == "spawn_subagent" {
        &started["task"]
    } else {
        &started
    };
    assert_eq!(started["status"], "started");
    tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
        .await
        .unwrap();
    let receipt = state
        .db
        .collection::<DirectHandoff>(HANDOFFS)
        .find_one(doc! {"run_id":&run.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.parent_thread_id, row.id);
    assert_eq!(
        receipt.parent_turn_id,
        row.active_turn.as_ref().unwrap().turn_id
    );
    assert_eq!(started["turn_id"], receipt.child_turn_id);
    let child = engine::get(&state.db, OWNER, &receipt.child_thread_id)
        .await
        .unwrap();
    assert!(
        child.active_turn.as_ref().unwrap().trigger_run_id.is_none(),
        "notification is not coordinator execution authority"
    );
    Box::pin(completion_reply(&state, &row, &run)).await;
    let waiting = completion_load(&state, &run.id).await;
    assert_eq!(waiting.outcome, RunOutcome::Waiting);
    assert_eq!(
        waiting.coordinator_wait.as_ref().unwrap().source,
        "specialist"
    );
    assert!(!waiting.terminal_result_recorded);
    assert_eq!(waiting.continuation_count, 0);
    // Force the real callback's queue write to fail. The saved reply and
    // pending receipt must suffice for a subsequent sweep after restart.
    let filler: Vec<_> = (0..engine::MAX_PENDING_EVENTS)
        .map(|_| team::event("fixture_capacity", "capacity".into(), None))
        .collect();
    engine::push_events(&state.db, OWNER, &row.id, filler)
        .await
        .unwrap();
    let pool_limit = super::super::assistant_team::team_pool_limit(&state, OWNER).await + 1;
    let mut parked = Vec::new();
    for _ in 0..pool_limit {
        parked.push(
            state
                .direct_chat_limiter
                .try_acquire_pool("assistant_team", OWNER, pool_limit + 1)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    release.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let child = engine::get(&state.db, OWNER, &receipt.child_thread_id)
                .await
                .unwrap();
            if child.active_turn.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Let the callback run through the same failed transactional delivery;
    // this call is also a deterministic check that capacity refusal is real.
    assert!(
        schedules::direct::settled(
            &state.db,
            OWNER,
            &receipt.child_thread_id,
            &receipt.child_turn_id
        )
        .await
        .is_err()
    );
    assert_eq!(
        completion_load(&state, &run.id).await.outcome,
        RunOutcome::Waiting
    );
    state.db.collection::<Document>(CONVERSATIONS).update_one(doc! {"_id":&row.id},
        doc! {"$pull":{"pending_events":{"kind":"fixture_capacity"}},"$set":{"event_streak":100}}).await.unwrap();
    // No sleeps or new model call are needed to reconstruct the exact notice.
    schedules::direct::recover(&state.db).await.unwrap();
    schedules::direct::recover(&state.db).await.unwrap();
    schedules::direct::settled(
        &state.db,
        OWNER,
        &receipt.child_thread_id,
        &receipt.child_turn_id,
    )
    .await
    .unwrap();
    let queued = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(queued.pending_events.len(), 1);
    assert_eq!(
        queued.pending_events[0].trigger_run_id.as_deref(),
        Some(run.id.as_str())
    );
    assert_eq!(queued.pending_events[0].continuation_protocol_version, 2);
    assert_eq!(
        queued.pending_events[0].agent_id.as_deref(),
        Some(receipt.agent_id.as_str())
    );
    assert!(
        queued.pending_events[0]
            .text
            .contains("Direct specialist completed")
    );
    assert_eq!(
        state
            .db
            .collection::<DirectHandoff>(HANDOFFS)
            .find_one(doc! {"_id":&receipt.id})
            .await
            .unwrap()
            .unwrap()
            .state,
        "delivered"
    );
    drop(parked);
    Box::pin(super::super::assistant_team::wake(&state, OWNER, &row.id)).await;
    let finished = completed(&state, &automation_trigger(&state, &run.trigger_id).await).await;
    assert_eq!(finished.outcome, RunOutcome::Completed);
    assert_eq!(finished.continuation_count, 1);
    assert!(finished.terminal_result_recorded);
    assert!(finished.result_claimed);
    schedules::direct::recover(&state.db).await.unwrap();
    schedules::direct::settled(
        &state.db,
        OWNER,
        &receipt.child_thread_id,
        &receipt.child_turn_id,
    )
    .await
    .unwrap();
    Box::pin(super::super::assistant_team::wake(&state, OWNER, &row.id)).await;
    assert_eq!(
        calls.lock().await.len(),
        2,
        "one child and one coordinator continuation"
    );
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    // completed() observes the terminal run and trigger projection before
    // finish_record() appends its audit. Wait for that actual write as well.
    let outcomes = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let count = state
                .db
                .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"user_id":OWNER,"event_type":"trigger_run_outcome","event_data.run_id":&run.id})
                .await
                .unwrap();
            if count != 0 {
                break count;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("terminal outcome audit was published");
    assert_eq!(outcomes, 1);
    server.abort();
}

async fn automation_trigger(state: &AppState, id: &str) -> Trigger {
    state
        .db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn automation_direct_admission_is_atomic_with_barrier_stop_and_terminal_cleanup() {
    for winner in ["registration", "barrier", "stop", "stale_parent"] {
        Box::pin(feedback_direct_admission(winner)).await;
    }
}

async fn feedback_direct_admission(winner: &str) {
    use crate::models::assistant_direct_handoff::{COLLECTION_NAME as HANDOFFS, DirectHandoff};
    let (state, _, server) = setup(&format!("automation_direct_fence_{winner}")).await;
    let (_, run, row) = Box::pin(completion_coordinator(&state)).await;
    let agent = specialist(&state).await;
    let home = team::home_thread_for(&state.db, &state.encryption_keys, OWNER, &agent)
        .await
        .unwrap();
    let mut start = completion_start(&home.id, TurnOrigin::Orchestrator, None);
    start.report_to = Some(row.id.clone());
    start.automation_parent = Some(schedules::direct::Parent {
        run_id: run.id.clone(),
        thread_id: row.id.clone(),
        turn_id: if winner == "stale_parent" {
            Uuid::new_v4().to_string()
        } else {
            row.active_turn.as_ref().unwrap().turn_id.clone()
        },
    });
    if winner == "barrier" {
        assert!(
            !continuation_v2::mark_coordinator_waiting(&state.db, &run)
                .await
                .unwrap()
        );
    } else if winner == "stop" {
        engine::request_stop(&state.db, OWNER, &row.id)
            .await
            .unwrap();
    }
    if winner != "registration" {
        assert!(
            Box::pin(engine::begin_turn(
                &state.db,
                OWNER,
                &start,
                &state.encryption_keys
            ))
            .await
            .is_err()
        );
        assert!(
            engine::get(&state.db, OWNER, &home.id)
                .await
                .unwrap()
                .active_turn
                .is_none()
        );
        assert_eq!(
            state
                .db
                .collection::<DirectHandoff>(HANDOFFS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        server.abort();
        return;
    }
    let mut snapshot = state.db.client().start_session().await.unwrap();
    snapshot
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await
        .unwrap();
    assert!(
        continuation_v2::source(&state.db, &run, &mut snapshot)
            .await
            .unwrap()
            .is_none()
    );
    let child = Box::pin(engine::begin_turn(
        &state.db,
        OWNER,
        &start,
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    assert!(
        continuation_v2::mark_coordinator_waiting_in_session(&state.db, &run, &mut snapshot)
            .await
            .is_err(),
        "stale no-work snapshot must conflict with committed registration"
    );
    snapshot.abort_transaction().await.unwrap();
    assert!(!continuation_v2::rollback_ready(&state.db).await.unwrap());
    assert!(
        continuation_v2::mark_coordinator_waiting(&state.db, &run)
            .await
            .unwrap()
    );
    engine::request_stop(&state.db, OWNER, &row.id)
        .await
        .unwrap();
    let saved = completion_load(&state, &run.id).await;
    assert_eq!(saved.outcome, RunOutcome::Failed);
    assert_eq!(saved.reason.as_deref(), Some("cancelled"));
    let receipt = state
        .db
        .collection::<DirectHandoff>(HANDOFFS)
        .find_one(doc! {"run_id":&run.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.state, "cancelled");
    // A late durable child reply cannot resurrect a cancelled source or
    // fall back to an ordinary notification after terminal cleanup.
    engine::finish_turn(
        &state.db,
        &child,
        &child.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Late specialist reply".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    assert!(
        schedules::direct::settled(
            &state.db,
            OWNER,
            &child.id,
            &child.active_turn.as_ref().unwrap().turn_id
        )
        .await
        .unwrap()
        .is_some()
    );
    schedules::direct::recover(&state.db).await.unwrap();
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    assert!(continuation_v2::rollback_ready(&state.db).await.unwrap());
    // The rollback check must independently account for a pending direct
    // receipt even when no run/event/follower remains nonterminal.
    state
        .db
        .collection::<Document>(HANDOFFS)
        .update_one(doc! {"_id":&receipt.id}, doc! {"$set":{"state":"pending"}})
        .await
        .unwrap();
    assert!(!continuation_v2::rollback_ready(&state.db).await.unwrap());
    schedules::direct::recover(&state.db).await.unwrap();
    assert!(continuation_v2::rollback_ready(&state.db).await.unwrap());
    server.abort();
}

#[tokio::test]
async fn automation_stop_preserves_unrelated_turn_and_rereads_idle_membership() {
    let (state, calls, server) = setup("automation_stop_owner").await;
    let (_, run, row) = Box::pin(completion_coordinator(&state)).await;
    completion_queue(&state, &row, &run).await;
    Box::pin(completion_reply(&state, &row, &run)).await;
    let idle = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    let unrelated = Box::pin(engine::begin_turn(
        &state.db,
        OWNER,
        completion_start(&row.id, TurnOrigin::User, None),
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    // A Stop that read idle before this unrelated admission must not act on
    // its stale membership snapshot once an active owner exists.
    continuation_v2::cancel_thread(&state.db, &idle)
        .await
        .unwrap();
    engine::request_stop(&state.db, OWNER, &row.id)
        .await
        .unwrap();
    let stopped = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert!(stopped.active_turn.as_ref().unwrap().stop_requested);
    assert_eq!(
        stopped.active_turn.as_ref().unwrap().turn_id,
        unrelated.active_turn.as_ref().unwrap().turn_id
    );
    assert_eq!(
        completion_load(&state, &run.id).await.outcome,
        RunOutcome::Waiting
    );
    assert!(stopped.coordinator_run_ids.contains(&run.id));
    assert_eq!(stopped.pending_events.len(), 1);
    engine::force_stop(
        &state.db,
        OWNER,
        &row.id,
        &unrelated.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap();
    let replacement = Box::pin(engine::begin_turn(
        &state.db,
        OWNER,
        completion_start(&row.id, TurnOrigin::User, None),
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    use crate::models::machine_job::{
        COLLECTION_NAME as MACHINE_JOBS, MachineJob, MachineJobBinding,
    };
    let job = |turn: &str| MachineJob {
        id: Uuid::new_v4().to_string(),
        user_id: OWNER.into(),
        node_id: "offline-stop-fixture-node".into(),
        runtime_id: String::new(),
        binding: Some(MachineJobBinding::Assistant {
            version: 1,
            conversation_id: row.id.clone(),
            turn_id: turn.into(),
            api_key_id: row.credential_api_key_id.clone(),
            agent_id: row.agent_id.clone().unwrap(),
        }),
        runtime_generation: None,
        conversation_id: row.id.clone(),
        issuing_turn_id: Some(turn.into()),
        api_key_id: row.credential_api_key_id.clone(),
        agent_id: row.agent_id.clone().unwrap(),
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
    };
    let old_job = job(&unrelated.active_turn.as_ref().unwrap().turn_id);
    let replacement_job = job(&replacement.active_turn.as_ref().unwrap().turn_id);
    state
        .db
        .collection::<MachineJob>(MACHINE_JOBS)
        .insert_many([&old_job, &replacement_job])
        .await
        .unwrap();
    let replacement_before = state
        .db
        .collection::<Document>(MACHINE_JOBS)
        .find_one(doc! {"_id":&replacement_job.id})
        .await
        .unwrap()
        .unwrap();
    // Both an old active snapshot and an old idle snapshot must leave a new
    // active turn's durable stop fence intact during machine cleanup.
    Box::pin(crate::services::machine_service::cancel_stopped_turn(
        &state, &stopped,
    ))
    .await
    .unwrap();
    let old_after = state
        .db
        .collection::<MachineJob>(MACHINE_JOBS)
        .find_one(doc! {"_id":&old_job.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old_after.state, "finished");
    assert!(old_after.finished_at.is_some());
    assert_eq!(
        state
            .db
            .collection::<Document>(MACHINE_JOBS)
            .find_one(doc! {"_id":&replacement_job.id})
            .await
            .unwrap()
            .unwrap(),
        replacement_before
    );
    Box::pin(crate::services::machine_service::cancel_stopped_turn(
        &state, &idle,
    ))
    .await
    .unwrap();
    assert_eq!(
        state
            .db
            .collection::<Document>(MACHINE_JOBS)
            .find_one(doc! {"_id":&replacement_job.id})
            .await
            .unwrap()
            .unwrap(),
        replacement_before,
        "stale idle cleanup must not finish the replacement turn's running job"
    );
    let current = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(
        current.active_turn.as_ref().unwrap().turn_id,
        replacement.active_turn.as_ref().unwrap().turn_id
    );
    assert!(!current.active_turn.as_ref().unwrap().stop_requested);
    engine::force_stop(
        &state.db,
        OWNER,
        &row.id,
        &replacement.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap();
    // An idle Stop must re-read the registered set in its transaction even
    // when its caller's saved idle membership was empty.
    let mut stale_idle = idle;
    stale_idle.coordinator_run_ids.clear();
    continuation_v2::cancel_thread(&state.db, &stale_idle)
        .await
        .unwrap();
    let cancelled = completion_load(&state, &run.id).await;
    assert_eq!(cancelled.outcome, RunOutcome::Failed);
    assert_eq!(cancelled.reason.as_deref(), Some("cancelled"));
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    assert!(calls.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn automation_idle_stop_cleanup_failure_cannot_acknowledge_resumable_work() {
    let (state, calls, server) = setup("automation_idle_stop_cleanup_error").await;
    let (_, run, row) = Box::pin(completion_coordinator(&state)).await;
    completion_queue(&state, &row, &run).await;
    Box::pin(completion_reply(&state, &row, &run)).await;
    let idle = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert!(idle.active_turn.is_none());
    assert!(idle.coordinator_run_ids.contains(&run.id));
    assert_eq!(
        completion_load(&state, &run.id).await.outcome,
        RunOutcome::Waiting
    );
    // Reject the real terminal write, just as the active-turn failure control
    // below does. No production failpoint replaces the Stop implementation.
    state
        .db
        .run_command(doc! {
            "collMod": RUNS,
            "validator": {"outcome": {"$ne": "failed"}},
            "validationLevel": "strict", "validationAction": "error"
        })
        .await
        .unwrap();
    let stop = engine::request_stop_target(&state.db, OWNER, &row.id).await;
    // Recovery cannot revive work that Stop acknowledged successfully.
    state
        .db
        .run_command(doc! {"collMod": RUNS, "validator": {}})
        .await
        .unwrap();
    let claim = continuation_v2::claim_event(&state.db, OWNER, &row.id, &run.id)
        .await
        .unwrap();
    let resumed = if let Some(claim) = claim {
        Box::pin(engine::begin_turn(
            &state.db,
            OWNER,
            completion_start(&row.id, TurnOrigin::Event, Some(claim)),
            &state.encryption_keys,
        ))
        .await
        .is_ok()
    } else {
        false
    };
    assert!(
        calls.lock().await.is_empty(),
        "admission probe must not invoke provider"
    );
    server.abort();
    assert!(
        stop.is_err() || !resumed,
        "idle Stop returned success but its run admitted a fresh coordinator turn after cleanup recovered"
    );
}

#[tokio::test]
async fn automation_stop_cleanup_failure_cannot_block_actual_turn_stop() {
    let (state, _, server) = setup("automation_stop_cleanup_error").await;
    let (_, run, row) = Box::pin(completion_coordinator(&state)).await;
    // A real Mongo validation error in terminal cleanup, without a production
    // failpoint or replacing the Stop implementation under test.
    state
        .db
        .run_command(
            doc! {"collMod":RUNS,"validator":{"outcome":{"$ne":"failed"}},
            "validationLevel":"strict","validationAction":"error"},
        )
        .await
        .unwrap();
    engine::request_stop(&state.db, OWNER, &row.id)
        .await
        .unwrap();
    let stopped = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    assert!(stopped.active_turn.as_ref().unwrap().stop_requested);
    assert_eq!(
        completion_load(&state, &run.id).await.outcome,
        RunOutcome::Started,
        "cleanup really refused the terminal write"
    );
    assert!(
        engine::force_stop(
            &state.db,
            OWNER,
            &row.id,
            &row.active_turn.as_ref().unwrap().turn_id
        )
        .await
        .unwrap()
    );
    let messages = engine::messages(&state.db, OWNER, &row.id, 100, None)
        .await
        .unwrap();
    assert!(
        messages
            .iter()
            .any(|reply| reply.role == "assistant"
                && reply.error_code.as_deref() == Some("cancelled"))
    );
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    state
        .db
        .run_command(doc! {"collMod":RUNS,"validator":{}})
        .await
        .unwrap();
    server.abort();
}
