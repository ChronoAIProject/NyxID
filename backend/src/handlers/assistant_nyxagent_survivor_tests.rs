use super::*;
use crate::models::assistant_conversation::{
    COLLECTION_NAME, EngineWorkerInstance, ReattachDeployment,
};
use crate::services::{
    assistant_engine_worker as workers, assistant_turn_restart as restart,
    platform_manager_contract::NyxAgentDestination,
};

#[derive(Clone, Copy)]
enum StopReply {
    Completed,
    Cancelled,
    Unavailable,
}

#[tokio::test]
async fn survivor_recovers_before_created_and_after_checkpoint_with_one_execution() {
    for (mid_answer, lost, trimmed, stop_reply, refusal) in [
        (false, false, false, None, Some("background_disabled")),
        (false, false, false, None, Some("idempotency_key_required")),
        (false, false, false, None, None),
        (true, false, false, None, None),
        (true, true, false, None, None),
        (true, false, true, None, None),
        (true, false, false, Some(StopReply::Completed), None),
        (true, false, false, Some(StopReply::Cancelled), None),
        (true, false, false, Some(StopReply::Unavailable), None),
    ] {
        let db = connect_transaction_test_database("handoff_subscriber").await;
        engine::ensure_indexes(&db).await.unwrap();
        db.collection(USERS)
            .insert_one(test_user(OWNER, UserType::Person))
            .await
            .unwrap();
        let first = test_app_state(db.clone());
        let mut next = test_app_state(db.clone());
        next.encryption_keys = first.encryption_keys.clone();
        let posts = Arc::new(AtomicUsize::new(0));
        let admissions = posts.clone();
        let requests = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let keys = requests.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let served = reads.clone();
        let looked_up = reads.clone();
        let terminal = json!({"type":"response.completed","sequence_number":3,"response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello world"}]}]}});
        let stored = terminal["response"].clone();
        let foreground_reply = terminal.clone();
        let upstream = Router::new().route("/v1/responses",post(move |headers: HeaderMap,Json(body): Json<Value>| {
            let keys = keys.clone(); let admissions = admissions.clone(); let foreground_reply = foreground_reply.clone(); async move {
                if let Some(code) = refusal {
                    if admissions.fetch_add(1, Ordering::SeqCst) == 0 {
                        assert_eq!(body["background"], true);
                        return (StatusCode::BAD_REQUEST, Json(json!({"error":{"code":code}}))).into_response();
                    }
                    assert!(body.get("background").is_none());
                    return ([("content-type", "text/event-stream")], format!("data: {foreground_reply}\n\n")).into_response();
                }
                assert_eq!(body["background"],true); assert_eq!(body["store"],true); assert_eq!(body["stream"],true);
                keys.lock().await.push((headers["idempotency-key"].to_str().unwrap().into(),headers["authorization"].to_str().unwrap().into()));
                admissions.fetch_add(1,Ordering::SeqCst);
                let stream = async_stream::stream! {
                    if mid_answer {
                        let created = json!({"type":"response.created","sequence_number":0,"response":{"id":RESPONSE,"conversation":{"id":SESSION}}});
                        let delta = json!({"type":"response.output_text.delta","sequence_number":1,"delta":"hello"});
                        yield Ok::<_,Infallible>(axum::body::Bytes::from(format!("data: {created}\n\ndata: {delta}\n\n")));
                    }
                    std::future::pending::<()>().await;
                };
                ([("content-type","text/event-stream")],Body::from_stream(stream)).into_response()
            }
        })).route("/v1/responses/lookup",axum::routing::get(move |headers: HeaderMap| {
            let requests = requests.clone(); let looked_up = looked_up.clone(); let stored = stored.clone(); async move {
                let requests = requests.lock().await;
                assert_eq!(headers["idempotency-key"].to_str().unwrap(),requests[0].0);
                assert!(headers["authorization"].to_str().unwrap() == requests[0].1);
                if lost {
                    return Json(json!({"status":"lost","response_id":RESPONSE,"conversation_id":SESSION,"last_sequence_number":2,
                        "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":"failed","error":{"code":"executor_lost","tool_dispatch":"none"},"output":[]}}));
                }
                if trimmed && looked_up.load(Ordering::SeqCst) == 4 {
                    return Json(json!({"status":"completed","response_id":RESPONSE,"conversation_id":SESSION,"last_sequence_number":2,"response":stored}));
                }
                Json(json!({"status":"running","response_id":RESPONSE,"conversation_id":SESSION,"last_sequence_number":2,"response":null}))
            }
        })).route(&format!("/v1/responses/{RESPONSE}/events"),axum::routing::get(move |uri: Uri| {
            let terminal = terminal.clone(); let served = served.clone(); async move {
                let index = served.fetch_add(1, Ordering::SeqCst);
                if trimmed {
                    assert_eq!(uri.query(), Some(if index == 3 { "after=2" } else { "after=1" }));
                    match index {
                        0 | 1 => {
                            let code = if index == 0 {"worker_draining"} else {"subscriber_expired"};
                            let error = json!({"type":"error","error":{"code":code}});
                            return ([("content-type","text/event-stream")],format!("data: {error}\n\n")).into_response();
                        }
                        2 => {
                            let mut snapshot = terminal["response"].clone();
                            snapshot["status"] = json!("in_progress");
                            return (StatusCode::GONE,Json(json!({"error":{"code":"events_not_retained"},"earliest_sequence_number":3,"response":snapshot,"snapshot_sequence_number":2}))).into_response();
                        }
                        3 => return StatusCode::NO_CONTENT.into_response(),
                        _ => panic!("unexpected events request"),
                    }
                }
                assert_eq!(uri.query(),mid_answer.then_some("after=1"));
                let created = json!({"type":"response.created","sequence_number":0,"response":{"id":RESPONSE,"conversation":{"id":SESSION}}});
                let first = json!({"type":"response.output_text.delta","sequence_number":1,"delta":"hello"});
                let second = json!({"type":"response.output_text.delta","sequence_number":2,"delta":" world"});
                ([("content-type","text/event-stream")],format!("data: {created}\n\ndata: {first}\n\ndata: {second}\n\ndata: {terminal}\n\n")).into_response()
            }
        }));
        let cancellations = Arc::new(AtomicUsize::new(0));
        let cancel_count = cancellations.clone();
        let upstream = upstream.route(&format!("/v1/responses/{RESPONSE}/cancel"), post(move || {
            let cancel_count = cancel_count.clone();
            async move {
                cancel_count.fetch_add(1,Ordering::SeqCst);
                let status = match stop_reply {
                    Some(StopReply::Completed) => "completed",
                    Some(StopReply::Cancelled) => "cancelled",
                    _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                };
                Json(json!({"status":status,"conversation_id":SESSION,"response_id":RESPONSE,"last_sequence_number":2,
                    "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":status,"output":[],"error":if status == "cancelled" {json!({"code":"cancelled"})} else {Value::Null}}})).into_response()
            }
        }));
        let server = register_upstream(&first, upstream).await;
        let service = assistant_service::resolve_admin_service_by_slug(&db, engine::SERVICE_SLUG)
            .await
            .unwrap();
        let input: engine::TurnRequest = serde_json::from_value(json!({"text":"hello"})).unwrap();
        let mut start = engine::TurnStart::from(&input);
        start.owner_instruction_source = Some(engine::OwnerInstructionSource::FirstPartyOwner);
        let predecessor = Box::leak(Box::new(EngineWorkerInstance {
            id: Uuid::new_v4().to_string(),
            hostname: None,
        }));
        let claim = workers::TEST_WORKER
            .scope(
                predecessor,
                engine::claim_turn(&db, OWNER, start.clone(), &first.encryption_keys, None),
            )
            .await
            .unwrap();
        let mut row = claim.conversation;
        let input_id = engine::messages(&db, OWNER, &row.id, 10, None)
            .await
            .unwrap()
            .pop()
            .unwrap()
            .id;
        let deployment = ReattachDeployment {
            destination_hash: NyxAgentDestination::for_service(&service.id, &service.base_url).0,
            limits: serde_json::from_value(json!({"protocol":"nyxagent-reattach-v1","credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15},"max_events":64,"recent_events":32,"max_event_bytes":65536,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32})).unwrap(),
        };
        let mut direct = row.clone();
        engine::negotiate_restart(
            &mut direct,
            &start,
            &input_id,
            Some(deployment.clone()),
            Some(InferenceCredential::Platform),
            false,
        );
        let foreground = direct.active_turn.unwrap();
        assert!(foreground.restart_admission.is_none());
        assert_eq!(
            foreground.restart_eligibility.as_deref(),
            Some("platform_credential")
        );
        engine::negotiate_restart(
            &mut row,
            &start,
            &input_id,
            Some(deployment),
            Some(InferenceCredential::Thread),
            false,
        );
        assert_eq!(
            row.active_turn
                .as_ref()
                .unwrap()
                .restart_eligibility
                .as_deref(),
            Some("watched_thread_credential")
        );
        let reply_id = row
            .active_turn
            .as_ref()
            .unwrap()
            .restart_admission
            .as_ref()
            .unwrap()
            .reply_id
            .clone();
        db.collection::<AssistantConversation>(COLLECTION_NAME)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        let permit = first.direct_chat_limiter.admit(OWNER).await.unwrap();
        let (sender, _) = broadcast::channel(256);
        let driver = tokio::spawn(workers::TEST_WORKER.scope(
            predecessor,
            run_turn(
                first.clone(),
                test_auth_user(OWNER),
                row.clone(),
                "hello".into(),
                claim.credential,
                Some(SERVER_TURN_POLICY),
                permit,
                Events {
                    sender,
                    cursor: 0,
                    checkpoint: None,
                    reset_notified: false,
                },
            ),
        ));
        if refusal.is_some() {
            tokio::time::timeout(Duration::from_secs(20), driver)
                .await
                .unwrap()
                .unwrap();
            let messages = engine::messages(&db, OWNER, &row.id, 10, None)
                .await
                .unwrap();
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[1].id, reply_id);
            assert_eq!(messages[1].status, "completed");
            assert_eq!(messages[1].text, "hello world");
            assert_eq!(posts.load(Ordering::SeqCst), 2);
            assert_eq!(reads.load(Ordering::SeqCst), 0);
            server.abort();
            let _ = server.await;
            db.drop().await.unwrap();
            continue;
        }
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if posts.load(Ordering::SeqCst) == 1
                    && (!mid_answer
                        || engine::get(&db, OWNER, &row.id)
                            .await
                            .unwrap()
                            .active_turn
                            .unwrap()
                            .reply
                            .is_some_and(|reply| {
                                reply.upstream.is_some_and(|cursor| cursor.sequence == 1)
                            }))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .expect("background dispatch and optional durable checkpoint");
        if mid_answer {
            let drain = crate::services::shutdown::drain::Drain::begin(
                &first,
                Duration::from_secs(2),
                "SIGTERM",
            )
            .unwrap();
            tokio::select! {
                result = tokio::time::timeout(Duration::from_secs(5), driver) => {
                    result.expect("subscriber yielded before shutdown").unwrap();
                }
                never = drain.progress(&first) => match never {},
            }
        } else {
            driver.abort();
            assert!(driver.await.unwrap_err().is_cancelled());
            let stale = Utc::now()
                - chrono::Duration::seconds(engine::ACTIVE_TURN_HEARTBEAT_STALE_SECS + 1);
            db.collection::<bson::Document>(COLLECTION_NAME)
                .update_one(
                    engine::WorkerFence::capture(&row).unwrap().filter(),
                    doc! {
                        "$set": {"active_turn.heartbeat_at": bson::DateTime::from_chrono(stale)}
                    },
                )
                .await
                .unwrap();
            assert!(
                engine::sweep_orphaned_turn_rows(&db)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        let offered = restart::due(&db, &workers::instance().id)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let offer = offered
            .active_turn
            .as_ref()
            .unwrap()
            .restart_offer
            .as_ref()
            .unwrap();
        assert_eq!(offer.reason, if mid_answer { "shutdown" } else { "sweep" });
        assert!(restart::due(&db, &predecessor.id).await.unwrap().is_empty());
        if let Some(cancel) = stop_reply {
            if matches!(cancel, StopReply::Cancelled) {
                assert!(
                    restart::claim(&db, &offered, workers::instance())
                        .await
                        .unwrap()
                        .is_some()
                );
            }
            let stopped = engine::request_stop_row_target(&db, &offered)
                .await
                .unwrap()
                .unwrap();
            assert!(
                restart::claim(&db, &offered, workers::instance())
                    .await
                    .unwrap()
                    .is_none()
            );
            let before_cancel = engine::get(&db, OWNER, &row.id).await.unwrap();
            assert!(before_cancel.active_turn.is_none() && before_cancel.stopped_window.is_some());
            let messages = engine::messages(&db, OWNER, &row.id, 10, None)
                .await
                .unwrap();
            assert_eq!(messages[1].id, reply_id);
            assert_eq!(messages[1].text, "hello");
            assert_eq!(messages[1].error_code.as_deref(), Some("cancelled"));
            survivor::cancel(&next, &stopped).await;
            let after_cancel = engine::get(&db, OWNER, &row.id).await.unwrap();
            assert!(cancellations.load(Ordering::SeqCst) > 0);
            assert_eq!(
                after_cancel.stopped_window.is_some(),
                matches!(cancel, StopReply::Unavailable)
            );
            assert_eq!(
                after_cancel.nyxagent_session_id.as_deref(),
                matches!(cancel, StopReply::Completed).then_some(SESSION)
            );
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            server.abort();
            let _ = server.await;
            db.drop().await.unwrap();
            continue;
        }
        let (left, right) = tokio::join!(
            survivor::take_one(&next, &offered, workers::instance()),
            survivor::take_one(&next, &offered, workers::instance())
        );
        assert_ne!(left.unwrap(), right.unwrap());
        let saved = settled(&next).await;
        let messages = engine::messages(&db, OWNER, &saved.id, 10, None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].id, reply_id);
        if lost {
            assert_eq!(messages[1].error_code.as_deref(), Some("outcome_unknown"));
            assert_eq!(messages[1].text, "hello");
        } else {
            assert_eq!(messages[1].text, "hello world");
            assert_eq!(messages[1].status, "completed");
        }
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        assert_eq!(
            reads.load(Ordering::SeqCst),
            if lost {
                0
            } else if trimmed {
                4
            } else {
                1
            }
        );
        server.abort();
        let _ = server.await;
        db.drop().await.unwrap();
    }
}
