use super::*;
use crate::models::assistant_conversation::{
    COLLECTION_NAME, EngineWorkerInstance, ReattachDeployment,
};
use crate::services::{
    assistant_engine_worker as workers, assistant_team_service as team,
    assistant_turn_restart as restart, platform_manager_contract::NyxAgentDestination,
};

#[derive(Clone, Copy)]
enum StopReply {
    Completed,
    Cancelled,
    Unavailable,
    Revoke,
    Destroy,
    Rotate,
    Disable,
}

async fn read_granted_service(state: &AppState, auth: &AuthUser) -> StatusCode {
    let mut request = Request::builder()
        .uri("/api/v1/proxy/s/handoff-reader/read")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(
        crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Proxy,
        ),
    );
    match Box::pin(crate::handlers::proxy::proxy_request_by_slug(
        State(state.clone()),
        auth.clone(),
        Default::default(),
        Path(("handoff-reader".into(), "read".into())),
        request,
    ))
    .await
    {
        Ok(response) => response.status(),
        Err(error) => error.status_code(),
    }
}

#[test]
fn only_exact_reattachment_operations_are_metering_only() {
    for (method, path, allowed) in [
        ("GET", "v1/responses/lookup", true),
        ("POST", "v1/responses/cancel", true),
        ("POST", "v1/responses", false),
        ("GET", "v1/models", false),
        ("POST", "v1/responses/lookup", false),
        ("GET", "v1/responses/resp_/events", false),
    ] {
        assert_eq!(super::subscriber_control(method, path), allowed);
    }
    for (method, operation, allowed) in [
        ("GET", "events", true),
        ("POST", "cancel", true),
        ("POST", "events", false),
        ("GET", "events/more", false),
    ] {
        assert_eq!(
            super::subscriber_control(method, &format!("v1/responses/{RESPONSE}/{operation}")),
            allowed
        );
    }
}

#[tokio::test]
async fn transferring_capacity_preserves_running_slots_and_fifo_while_saturated() {
    use crate::mw::rate_limit::{DIRECT_CHAT_MAX_IN_FLIGHT, DIRECT_CHAT_QUEUE_NAMESPACE};
    use crate::services::coordination_service::SlotStore;
    let db = connect_transaction_test_database("handoff_capacity").await;
    let first = test_app_state(db.clone());
    let next = test_app_state(db.clone());
    let mut running = Vec::new();
    for _ in 0..DIRECT_CHAT_MAX_IN_FLIGHT {
        running.push(first.direct_chat_limiter.admit(OWNER).await.unwrap());
    }
    let predecessor = first.direct_chat_limiter.admit(OWNER).await.unwrap();
    let later = first.direct_chat_limiter.admit(OWNER).await.unwrap();
    let team = first
        .direct_chat_limiter
        .try_acquire_pool("assistant_team", OWNER, 1)
        .await
        .unwrap()
        .unwrap();
    for (guard, limit) in [
        (predecessor, None),
        (team, Some(1)),
        (running.pop().unwrap(), None),
    ] {
        let old = guard.token().unwrap().clone();
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let grant = next
            .direct_chat_limiter
            .transfer_capacity(&mut session, &old, OWNER, limit)
            .await
            .unwrap()
            .unwrap();
        session.commit_transaction().await.unwrap();
        let transferred = grant.token.clone();
        assert_eq!(old.id, transferred.id);
        assert_eq!(old.acquired_at, transferred.acquired_at);
        assert_eq!(old.order_id, transferred.order_id);
        assert_ne!(old.lease_id, transferred.lease_id);
        let moved = next.direct_chat_limiter.transferred_permit(OWNER, grant);
        assert!(!SlotStore::release(&db, &old).await.unwrap());
        drop(guard);
        if old.namespace == DIRECT_CHAT_QUEUE_NAMESPACE {
            assert!(SlotStore::is_oldest_live(&db, &transferred).await.unwrap());
            assert!(
                !SlotStore::is_oldest_live(&db, later.token().unwrap())
                    .await
                    .unwrap()
            );
        }
        if limit.is_some() {
            assert!(
                next.direct_chat_limiter
                    .try_acquire_pool("assistant_team", OWNER, 1)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert!(SlotStore::release(&db, &transferred).await.unwrap());
        drop(moved);
    }
    drop((running, later));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn ambiguous_image_window_fences_credential_when_preserving_base_reply() {
    for (image, unconfirmed) in [
        ("invalid_stream", true),
        ("invalid_stream_confirmed", false),
    ] {
        let (saved, reply, keys) = scripted_windows(vec!["", image], 1).await;
        assert_eq!(keys.len(), 2);
        assert!(keys[1].ends_with(":images:1"));
        assert_eq!(reply.text, "Partial answer completed");
        assert!(reply.error_code.is_none());
        assert_eq!(
            saved.stopped_window.map(|window| window.window_key),
            unconfirmed.then(|| keys[1].clone())
        );
    }
}

#[tokio::test]
async fn deadline_before_created_fences_the_dispatched_window() {
    short_bounds();
    let (saved, reply, keys) = scripted_windows(vec!["http_delayed_accept"], 0).await;
    assert_eq!(reply.error_code.as_deref(), Some("first_byte_timeout"));
    assert_eq!(keys.len(), 1);
    assert_eq!(saved.stopped_window.unwrap().window_key, keys[0]);
}

#[tokio::test]
async fn continuation_give_up_fences_only_the_unconfirmed_window() {
    for (failure, fence) in [("http_budget", true), ("tool_budget_exhausted", false)] {
        let (saved, reply, keys) = scripted_windows(vec![failure], 0).await;
        assert_eq!(reply.error_code.as_deref(), Some("tool_budget_exhausted"));
        assert_eq!(saved.stopped_window.is_some(), fence);
        assert!(!keys.is_empty());
    }
}

#[tokio::test]
async fn budget_status_resolves_the_window_before_original_and_survivor_continuations() {
    for first in ["tool_budget_exhausted", "held_budget"] {
        let (saved, reply, keys) =
            scripted_windows_with_limit(vec![first, "http_budget_absent", "lookup_success"], 0, 2)
                .await;
        assert_eq!(reply.status, "completed", "{first}: {:?}", reply.error_code);
        assert_eq!(keys.len(), 3);
        assert!(keys[1].ends_with(":continuation:1"));
        assert!(keys[2].ends_with(":continuation:2"));
        assert!(saved.stopped_window.is_none());
    }
}

#[tokio::test]
async fn survivor_follow_up_retries_capacity_with_the_same_key_and_surfaces_permanent_refusal() {
    let (_, reply, keys) =
        scripted_windows_with_limit(vec!["held_budget", "http_capacity", "lookup_success"], 0, 1)
            .await;
    assert_eq!(reply.status, "completed");
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[1], keys[2]);
    for script in [
        vec!["http_model_unavailable"],
        vec!["held_budget", "http_model_unavailable"],
    ] {
        let expected_posts = script.len();
        let (row, reply, keys) = scripted_windows_with_limit(script, 0, 1).await;
        assert_eq!(reply.error_code.as_deref(), Some("model_unavailable"));
        assert_eq!(row.context_reset_reason.as_deref(), Some("turn_failed"));
        assert_eq!(keys.len(), expected_posts);
    }
}

#[tokio::test]
async fn survivor_credential_refusal_settles_after_confirmed_cancel_by_key() {
    let (row, reply, keys) = scripted_windows_with_limit(
        vec!["held_budget", "http_credential_confirmed_absent"],
        0,
        1,
    )
    .await;
    assert_eq!(reply.error_code.as_deref(), Some("credential_invalid"));
    assert_eq!(
        reply.upstream_error_code.as_deref(),
        Some("agent_key_required")
    );
    assert!(row.stopped_window.is_none());
    assert_eq!(keys.len(), 2);
}

async fn scripted_windows(
    failures: Vec<&'static str>,
    images: usize,
) -> (AssistantConversation, AssistantMessage, Vec<String>) {
    scripted_windows_with_limit(failures, images, 0).await
}

#[tokio::test]
async fn resumed_intermediate_continuation_and_image_windows_preserve_the_answer() {
    for held in [1, 2] {
        let mut script = vec!["lookup_budget", "lookup_success", "lookup_success"];
        script[held] = "held_success";
        let (saved, reply, keys) = scripted_windows_with_limit(script, 1, 1).await;
        assert_eq!(reply.text, "window 0\n\nwindow 1\n\nwindow 2");
        assert_eq!(reply.status, "completed");
        assert!(reply.turn_metadata.unwrap().continued_after_restart);
        assert_eq!(keys.len(), 3);
        assert!(keys[1].ends_with(":continuation:1"));
        assert!(keys[2].ends_with(":images:1"));
        assert!(saved.stopped_window.is_none());
    }
}

#[tokio::test]
async fn pdf_rejection_is_request_scoped_in_reconnect_and_restart_windows() {
    for script in [
        vec!["lookup_pdf_rejected", ""],
        vec!["held_pdf_rejected", ""],
        vec!["", "lookup_pdf_rejected"],
        vec!["", "held_pdf_rejected"],
    ] {
        let (_, reply, keys) = scripted_windows_with_limit(script, 1, 0).await;
        assert_eq!(reply.status, "completed");
        assert_eq!(keys.len(), 2);
    }
}

async fn scripted_windows_with_limit(
    failures: Vec<&'static str>,
    images: usize,
    maximum: i32,
) -> (AssistantConversation, AssistantMessage, Vec<String>) {
    let pdf_rejection = failures
        .iter()
        .position(|name| name.ends_with("pdf_rejected"));
    let _budget = if pdf_rejection.is_some() {
        Some(crate::services::assistant_upload_service::byte_budget::test_guard().await)
    } else {
        None
    };
    let held = failures
        .iter()
        .position(|kind| matches!(*kind, "held_success" | "held_budget" | "held_pdf_rejected"));
    let capacity = failures.iter().position(|kind| *kind == "http_capacity");
    let (state, calls, server) = setup_options_with_tools(
        None,
        Duration::ZERO,
        failures,
        true,
        images,
        Arc::new(AtomicUsize::new(0)),
    )
    .await;
    let service = assistant_service::resolve_admin_service_by_slug(&state.db, engine::SERVICE_SLUG)
        .await
        .unwrap();
    let mut request: engine::TurnRequest =
        serde_json::from_value(json!({"text":"view image"})).unwrap();
    let pdf_id = if pdf_rejection.is_some() {
        let uploads = &crate::handlers::assistant_uploads::draft(
            State(state.clone()),
            test_auth_user(OWNER),
            Json(crate::handlers::assistant_uploads::Draft { agent_id: None }),
        )
        .await
        .unwrap()
        .0;
        let id = uploads["id"].as_str().unwrap();
        crate::services::assistant_upload_service::pdf_vision::ensure_indexes(&state.db)
            .await
            .unwrap();
        state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {
            "_id":Uuid::new_v4().to_string(), "flag_key":"assistant:pdf-vision", "enabled":true,
            "target_kind":"global", "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
            "updated_by":OWNER,
        }).await.unwrap();
        let item = crate::services::assistant_upload_service::upload(
            &state.db,
            &state.encryption_keys,
            OWNER,
            id,
            "survivor.pdf",
            crate::services::attachment_extraction::pdf_fixture(&[None, None], 0),
        )
        .await
        .unwrap();
        request = serde_json::from_value(
            json!({"text":"view image","conversation_id":id,"attachment_ids":[item.id]}),
        )
        .unwrap();
        Some(item.id)
    } else {
        None
    };
    let mut start = engine::TurnStart::from(&request);
    start.owner_instruction_source = Some(engine::OwnerInstructionSource::FirstPartyOwner);
    let permit = state.direct_chat_limiter.admit(OWNER).await.unwrap();
    start.capacity = permit.token().cloned();
    let predecessor = Box::leak(Box::new(EngineWorkerInstance {
        id: Uuid::new_v4().to_string(),
        hostname: None,
    }));
    let claim = workers::TEST_WORKER
        .scope(
            predecessor,
            engine::claim_turn(
                &state.db,
                OWNER,
                start.clone(),
                &state.encryption_keys,
                None,
                None,
            ),
        )
        .await
        .unwrap();
    let mut row = claim.conversation;
    let initiating = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    engine::enroll_restart(
        &mut row,
        &start,
        &initiating.id,
        ReattachDeployment {
            destination_hash: NyxAgentDestination::for_service(&service.id, &service.base_url).0,
            limits: serde_json::from_value(json!({"protocol":"nyxagent-reattach-v1","credential_watch":{"interval_seconds":30,"model_request_max_age_seconds":15},"max_events":64,"recent_events":32,"max_event_bytes":65536,"retention_seconds":3600,"max_subscribers_per_response":8,"max_subscribers_per_owner":32})).unwrap(),
        },
        false,
    );
    state
        .db
        .collection::<AssistantConversation>(COLLECTION_NAME)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let (sender, _) = broadcast::channel(256);
    crate::services::assistant_settings_service::update(
        &state.db,
        OWNER,
        crate::services::assistant_settings_service::Update {
            max_auto_continuations: Some(maximum),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let conversation_id = row.id.clone();
    let admitted_credential = claim.credential.api_key_id.clone();
    let driver = tokio::spawn(workers::TEST_WORKER.scope(
        predecessor,
        run_turn(
            state.clone(),
            test_auth_user(OWNER),
            row,
            initiating.text,
            claim.credential,
            Some(SERVER_TURN_POLICY),
            permit,
            Events {
                sender,
                cursor: 0,
                reset_notified: false,
                checkpoint: None,
            },
        ),
    ));
    if let Some(held) = held {
        tokio::time::timeout(Duration::from_secs(20), async {
            while calls.lock().await.len() <= held {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("intermediate window dispatched");
        let current = engine::get(&state.db, OWNER, &conversation_id)
            .await
            .unwrap();
        assert!(
            restart::offer(
                &state.db,
                &current,
                restart::Loss::Shutdown(&predecessor.id)
            )
            .await
            .unwrap()
        );
        driver.abort();
        let _ = driver.await;
        let mut next = test_app_state(state.db.clone());
        next.encryption_keys = state.encryption_keys.clone();
        let offered = restart::due(&state.db, &workers::instance().id)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert!(
            survivor::take_one(&next, &offered, workers::instance())
                .await
                .unwrap()
        );
    } else {
        driver.await.unwrap();
    }
    let saved = settled(&state).await;
    assert!(saved.credential_api_key_id == admitted_credential);
    let reply = engine::messages(&state.db, OWNER, &saved.id, 10, None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let calls = calls.lock().await;
    assert!(calls.iter().all(|call| call.body["background"] == true));
    if let Some(index) = capacity {
        assert!(
            calls[index + 1]
                .received_at
                .duration_since(calls[index].received_at)
                >= Duration::from_secs(1)
        );
    }
    if let (Some(rejection), Some(pdf_id)) = (pdf_rejection, pdf_id) {
        let messages = engine::messages(&state.db, OWNER, &saved.id, 10, None)
            .await
            .unwrap();
        let pdf = messages
            .iter()
            .flat_map(|message| &message.attachments)
            .find(|attachment| attachment.id == pdf_id)
            .unwrap()
            .pdf
            .as_ref()
            .unwrap();
        let first = calls[0].body["input"][0]["content"].as_array().unwrap();
        assert_eq!(
            first
                .iter()
                .filter(|part| part["type"] == "input_image")
                .count(),
            2
        );
        let follow_up = calls[1].body["input"].to_string();
        assert!(
            follow_up.contains(r#"image_pages_in_this_request\":[]"#),
            "{follow_up}"
        );
        if rejection == 0 {
            assert!(pdf.image_pages.is_empty());
            assert_eq!(pdf.unread_pages, [1, 2]);
            assert_eq!(pdf.reason.as_deref(), Some("image_input_unsupported"));
            assert!(
                follow_up.contains(r#"unavailable_pages\":[1,2]"#),
                "{follow_up}"
            );
        } else {
            assert_eq!(pdf.image_pages, [1, 2]);
            assert!(pdf.unread_pages.is_empty());
            assert!(
                follow_up.contains(r#"image_pages_accepted_in_prior_requests\":[1,2]"#),
                "{follow_up}"
            );
        }
    }
    let keys = calls
        .iter()
        .map(|call| call.headers["idempotency-key"].to_str().unwrap().to_owned())
        .collect();
    server.abort();
    let _ = server.await;
    state.db.drop().await.unwrap();
    (saved, reply, keys)
}

async fn wait_for_phase<F: std::future::Future>(event: &'static str, future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(90), future)
        .await
        .expect(event)
}

// Subscribe first, then check durable state. This also catches a write that
// commits between finding no match and awaiting the next change.
async fn wait_for_turn_state(
    db: &mongodb::Database,
    filter: bson::Document,
) -> AssistantConversation {
    let conversations = db.collection::<AssistantConversation>(COLLECTION_NAME);
    let mut changes = conversations.watch().await.unwrap();
    loop {
        if let Some(row) = conversations.find_one(filter.clone()).await.unwrap() {
            return row;
        }
        changes
            .next()
            .await
            .expect("conversation change stream ended")
            .unwrap();
    }
}

async fn wait_for_checkpoint(db: &mongodb::Database, sequence: usize, expected: &str) {
    let row = wait_for_phase(
        "durable replay checkpoint",
        wait_for_turn_state(
            db,
            doc! {
                "user_id": OWNER, "active_turn.reply.upstream.response_id": RESPONSE,
                "active_turn.reply.upstream.sequence": sequence as i64,
            },
        ),
    )
    .await;
    let checkpoint = row.active_turn.unwrap().reply.unwrap();
    assert_eq!(checkpoint.text, expected);
    assert_eq!(checkpoint.upstream.unwrap().window_text, expected);
}

async fn released_usage(state: &AppState) -> Vec<crate::models::usage_meter::UsageMeterRow> {
    let mut changes = state
        .db
        .collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
        .watch()
        .await
        .unwrap();
    loop {
        let rows = crate::services::channel_x_tests::billing::rows(state).await;
        if !rows.is_empty() && rows.iter().all(|row| row.released) {
            return rows;
        }
        changes
            .next()
            .await
            .expect("usage change stream ended")
            .unwrap();
    }
}

#[tokio::test]
async fn survivor_recovers_before_created_and_after_checkpoint_with_one_execution() {
    for ((mid_answer, review, trimmed, stop_reply, refusal), discard_cursor) in [
        (false, None, false, None, Some("background_disabled")),
        (false, None, false, None, Some("idempotency_key_required")),
        (false, None, false, None, None),
        (true, None, false, None, None),
        (true, Some("lost"), false, None, None),
        (false, Some("expired"), false, None, None),
        (false, Some("not_found"), false, None, None),
        (true, Some("capacity_mismatch"), false, None, None),
        (true, None, true, None, None),
        (true, None, false, Some(StopReply::Completed), None),
        (true, None, false, Some(StopReply::Cancelled), None),
        (true, None, false, Some(StopReply::Unavailable), None),
        (true, None, false, Some(StopReply::Revoke), None),
        (true, None, false, Some(StopReply::Rotate), None),
        (true, None, false, Some(StopReply::Disable), None),
        (true, None, false, Some(StopReply::Destroy), None),
    ]
    .into_iter()
    .map(|case| (case, None))
    .chain(
        [BEFORE_DISCARD, AFTER_DISCARD]
            .map(|cursor| ((true, None, false, None, None), Some(cursor))),
    ) {
        let db = connect_transaction_test_database("handoff_subscriber").await;
        engine::ensure_indexes(&db).await.unwrap();
        db.collection(USERS)
            .insert_one(test_user(OWNER, UserType::Person))
            .await
            .unwrap();
        let mut first = test_app_state(db.clone());
        let mut next = test_app_state(db.clone());
        next.encryption_keys = first.encryption_keys.clone();
        let posts = Arc::new(AtomicUsize::new(0));
        let admissions = posts.clone();
        let dispatched = Arc::new(tokio::sync::Notify::new());
        let post_received = dispatched.clone();
        let requests = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let keys = requests.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let served = reads.clone();
        let looked_up = reads.clone();
        let terminal = json!({"type":"response.completed","sequence_number":3,"response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello world"}]}]}});
        let stored = terminal["response"].clone();
        let replay_db = db.clone();
        let foreground_reply = terminal.clone();
        let upstream = Router::new().route("/v1/responses",post(move |headers: HeaderMap,Json(body): Json<Value>| {
            let keys = keys.clone(); let admissions = admissions.clone(); let post_received = post_received.clone(); let foreground_reply = foreground_reply.clone(); async move {
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
                post_received.notify_one();
                let stream = async_stream::stream! {
                    if let Some(cursor) = discard_cursor {
                        for frame in &discarded_attempt_frames()[..=cursor] {
                            yield Ok::<_,Infallible>(axum::body::Bytes::from(format!("data: {frame}\n\n")));
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    } else if mid_answer {
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
                if review == Some("lost") {
                    return Json(json!({"status":"lost","response_id":RESPONSE,"conversation_id":SESSION,"last_sequence_number":2,
                        "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":"failed","error":{"code":"executor_lost","tool_dispatch":"none"},"output":[]}}));
                }
                if review == Some("expired") {
                    return Json(json!({"status":"expired","response_id":RESPONSE,"conversation_id":SESSION,"last_sequence_number":null,"response":null}));
                }
                if let Some(status) = review {
                    return Json(json!({"status":status,"response_id":null,"conversation_id":null,"last_sequence_number":null,"response":null}));
                }
                if trimmed && looked_up.load(Ordering::SeqCst) == 4 {
                    return Json(json!({"status":"completed","response_id":RESPONSE,"conversation_id":SESSION,"last_sequence_number":2,"response":stored}));
                }
                Json(json!({"status":"running","response_id":RESPONSE,"conversation_id":SESSION,"last_sequence_number":2,"response":null}))
            }
        })).route(&format!("/v1/responses/{RESPONSE}/events"),axum::routing::get(move |uri: Uri, headers: HeaderMap| {
            let terminal = terminal.clone(); let served = served.clone(); let replay_db = replay_db.clone(); async move {
                let index = served.fetch_add(1, Ordering::SeqCst);
                if let Some(cursor) = discard_cursor {
                    // Force a second connection just after the replacement delta. Its
                    // checkpoint must agree with the browser before a terminal is sent.
                    let after = if index == 0 { cursor } else { RETRY_DELTA };
                    assert_eq!(uri.query(), Some(format!("after={after}").as_str()));
                    if index == 1 {
                        wait_for_checkpoint(&replay_db, RETRY_DELTA, RETRIED_ANSWER).await;
                    }
                    let frames = discarded_attempt_frames();
                    let end = if index == 0 { RETRY_DELTA } else { frames.len()-1 };
                    let stream = frames[after+1..=end].iter().map(|frame| format!("data: {frame}\n\n")).collect::<String>();
                    return ([("content-type","text/event-stream")], stream).into_response();
                }
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
                            let secret = headers["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
                            snapshot["output"][0]["content"][0]["text"] = json!(format!("hello {}", &secret[..12]));
                            return (StatusCode::GONE,Json(json!({"error":{"code":"events_not_retained"},"earliest_sequence_number":3,"response":snapshot,"snapshot_sequence_number":2}))).into_response();
                        }
                        3 => {
                            let current = replay_db.collection::<AssistantConversation>(COLLECTION_NAME).find_one(doc! {"user_id":OWNER}).await.unwrap().unwrap();
                            let checkpoint = current.active_turn.unwrap().reply.unwrap();
                            assert!(checkpoint.text == "hello");
                            assert!(checkpoint.upstream.is_some_and(|cursor| cursor.sequence == 1 && cursor.window_text == "hello"));
                            return StatusCode::NO_CONTENT.into_response();
                        }
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
        let steered = Arc::new(AtomicUsize::new(0));
        let steer_count = steered.clone();
        let upstream = upstream.route("/v1/capabilities", axum::routing::get(|| async {
            Json(json!({"steer":{"version":1,"protocol":"nyxagent-steer-v1","input":["text"],"max_chars":32000,"max_per_turn":20}}))
        })).route(&format!("/v1/conversations/{SESSION}/steer"), post(move |Json(body): Json<Value>| {
            let steer_count = steer_count.clone();
            async move {
                assert_eq!(body["expected_response_id"],RESPONSE);
                steer_count.fetch_add(1,Ordering::SeqCst);
                Json(json!({"object":"conversation.steer","conversation_id":SESSION,"response_id":RESPONSE,"status":"accepted"}))
            }
        }));
        let cancel_count = cancellations.clone();
        let cancel_db = db.clone();
        let upstream = upstream.route(&format!("/v1/responses/{RESPONSE}/cancel"), post(move |headers: HeaderMap| {
            let cancel_count = cancel_count.clone();
            let db = cancel_db.clone();
            async move {
                if matches!(stop_reply, Some(StopReply::Revoke | StopReply::Destroy | StopReply::Rotate | StopReply::Disable)) {
                    let raw = headers["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
                    assert!(crate::services::key_service::validate_api_key(&db,raw).await.is_ok());
                }
                cancel_count.fetch_add(1,Ordering::SeqCst);
                if review == Some("lost") {
                    return Json(json!({"status":"lost","response_id":RESPONSE,"conversation_id":SESSION,"last_sequence_number":2,
                        "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":"failed","error":{"code":"executor_lost","tool_dispatch":"none"},"output":[]}})).into_response();
                }
                let status = match stop_reply {
                    Some(StopReply::Completed) => "completed",
                    Some(StopReply::Cancelled | StopReply::Revoke | StopReply::Destroy | StopReply::Rotate | StopReply::Disable) => "cancelled",
                    _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                };
                Json(json!({"status":status,"conversation_id":SESSION,"response_id":RESPONSE,"last_sequence_number":2,
                    "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":status,"output":[],"error":if status == "cancelled" {json!({"code":"cancelled"})} else {Value::Null}}})).into_response()
            }
        }));
        let server = register_upstream(&first, upstream).await;
        let funded = !mid_answer && review.is_none() && refusal.is_none();
        if funded {
            crate::services::channel_x_tests::billing::enable_billing_with_entitlement(
                &mut first, OWNER, "llm-nyx",
            )
            .await;
            next.config.billing_enabled = true;
            next.billing = first.billing.clone();
            db.collection::<bson::Document>("billing_rate_cache").insert_one(doc! {
                "_id":"platform_requests:*", "lago_metric_code":"platform_requests",
                "credits_per_unit_pico": 1_000_000_000_000_i64, "credits_per_unit_micros": 1_000_000_i64,
                "synced_at":bson::DateTime::now(),
            }).await.unwrap();
            db.collection::<bson::Document>(SERVICES).update_one(doc! {"slug":engine::SERVICE_SLUG},doc! {"$set":{
                "billing": {"platform_billable":true,"platform_charge_nyxid_credentials_only":false,"platform_metric":"requests"}
            }}).await.unwrap();
        }
        let service = assistant_service::resolve_admin_service_by_slug(&db, engine::SERVICE_SLUG)
            .await
            .unwrap();
        let input: engine::TurnRequest = serde_json::from_value(json!({"text":"hello"})).unwrap();
        let mut start = engine::TurnStart::from(&input);
        if matches!(stop_reply, Some(StopReply::Destroy)) {
            let (agent, _) = team::create_specialist(
                &db,
                &first.encryption_keys,
                OWNER,
                team::CreateRequest {
                    name: "handoff".into(),
                    description: "Handoff fixture".into(),
                    display_name: None,
                    persona: None,
                    voice_instructions: None,
                    targets: Default::default(),
                    account_read: false,
                    specialty: None,
                    machines: None,
                    logins: None,
                    created_by: "user",
                },
            )
            .await
            .unwrap()
            .unwrap();
            start.agent_id = Some(agent.id);
        }
        let permit = first.direct_chat_limiter.admit(OWNER).await.unwrap();
        start.capacity = permit.token().cloned();
        start.owner_instruction_source = Some(engine::OwnerInstructionSource::FirstPartyOwner);
        let predecessor = Box::leak(Box::new(EngineWorkerInstance {
            id: Uuid::new_v4().to_string(),
            hostname: None,
        }));
        let claim = workers::TEST_WORKER
            .scope(
                predecessor,
                engine::claim_turn(
                    &db,
                    OWNER,
                    start.clone(),
                    &first.encryption_keys,
                    None,
                    None,
                ),
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
        let (sender, _) = broadcast::channel(256);
        let pending_auth = if review == Some("lost") {
            crate::services::security_readiness_service::ready_test_replica(&db).await;
            let channel = crate::services::notification_service::get_or_create_channel(&db, OWNER)
                .await
                .unwrap();
            db.collection::<bson::Document>(crate::models::notification_channel::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":channel.id},
                    doc! {"$set":{"approval_required":false}},
                )
                .await
                .unwrap();
            let provider = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .respond_with(wiremock::ResponseTemplate::new(200))
                .mount(&provider)
                .await;
            let connection = crate::services::assistant_authority_tests::connected(
                &db,
                OWNER,
                "handoff-reader",
                &provider.uri(),
            )
            .await;
            db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&row.credential_api_key_id},
                    doc! {"$addToSet":{"allowed_service_ids":&connection}},
                )
                .await
                .unwrap();
            let (_, key, _) =
                crate::services::key_service::validate_api_key(&db, &claim.credential.raw_key)
                    .await
                    .unwrap();
            let auth = Box::pin(crate::mw::auth::api_key_auth_user(
                &db, &key, None, None, None,
            ))
            .await
            .unwrap();
            assert_eq!(read_granted_service(&first, &auth).await, StatusCode::OK);
            assert_eq!(provider.received_requests().await.unwrap().len(), 1);
            Some((auth, provider))
        } else {
            None
        };
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
            assert_eq!(
                messages[1].text,
                if discard_cursor.is_some() {
                    RETRIED_ANSWER
                } else {
                    "hello world"
                }
            );
            assert_eq!(posts.load(Ordering::SeqCst), 2);
            assert_eq!(reads.load(Ordering::SeqCst), 0);
            server.abort();
            let _ = server.await;
            db.drop().await.unwrap();
            continue;
        }
        wait_for_phase("predecessor response POST", dispatched.notified()).await;
        if mid_answer {
            let expected = match discard_cursor {
                Some(BEFORE_DISCARD) => "abandoned attempt.",
                Some(AFTER_DISCARD) => "",
                _ => "hello",
            };
            wait_for_checkpoint(&db, discard_cursor.unwrap_or(1), expected).await;
        } else if funded {
            // The proxy owns settlement once the predecessor has the response
            // headers. The mock still withholds every response.created frame.
            let accepted = wait_for_phase(
                "predecessor accepted the response headers",
                wait_for_turn_state(
                    &db,
                    doc! {
                        "_id": &row.id, "active_turn.restart_admission.window.accepted": true,
                    },
                ),
            )
            .await;
            assert!(accepted.active_turn.unwrap().running_response.is_none());
        }
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        let approval = if trimmed {
            let card_id = Uuid::new_v4().to_string();
            db.collection::<bson::Document>("assistant_acknowledgements").insert_one(doc! {
                "_id":&card_id,"user_id":OWNER,"conversation_id":&row.id,"api_key_id":&row.credential_api_key_id,
                "requested_turn_id":&row.active_turn.as_ref().unwrap().turn_id,"tool_name":"nyxid.service_execution",
                "status":"pending","expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(5)),"summary":"review the connection",
            }).await.unwrap();
            let markers = db.collection::<bson::Document>(
                crate::services::assistant_service_execution::COLLECTION,
            );
            assert_eq!(
                markers
                    .update_one(
                        doc! {"_id":&reply_id,"active":true},
                        doc! {"$set":{"refused_card_id":card_id}}
                    )
                    .await
                    .unwrap()
                    .matched_count,
                1
            );
            assert_eq!(
                crate::services::assistant_service_execution::finish_model_attempt(
                    &db, &reply_id, &row
                )
                .await
                .unwrap()
                .as_deref(),
                Some("review the connection")
            );
            let request = Box::pin(
                crate::services::approval_service::create_tool_approval_request(
                    &db,
                    &first.config,
                    &first.http_client,
                    None,
                    None,
                    OWNER,
                    "handoff_tool",
                    Some("handoff-call"),
                    None,
                    true,
                    "api_key",
                    &row.credential_api_key_id,
                    None,
                    Default::default(),
                ),
            )
            .await
            .unwrap();
            assert_eq!(request.status, "pending");
            Some(request.id)
        } else {
            None
        };
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
        if review == Some("not_found") {
            db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc! {"_id":&row.id},doc! {"$set":{"active_turn.restart_admission.window.absence_retry_before":bson::DateTime::from_millis(0)}}).await.unwrap();
        }
        let offered = restart::due(&db, &workers::instance().id)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let turn = offered.active_turn.as_ref().unwrap();
        if review == Some("capacity_mismatch") {
            let rows = db.collection::<bson::Document>(COLLECTION_NAME);
            let filter = doc! {"_id":&row.id};
            let cap = crate::services::assistant_nyxagent_runtime::stream_bounds().turn_cap;
            let start = Utc::now() - chrono::Duration::seconds(120);
            rows.update_one(
                filter.clone(),
                doc! {"$set":{
                    "active_turn.restart_admission.capacity.namespace":"mismatch",
                    "active_turn.started_at":bson::DateTime::from_chrono(start),
                    "active_turn.timing.queue_wait_ms":120_000_i64,
                }},
            )
            .await
            .unwrap();
            let queued = engine::get(&db, OWNER, &row.id).await.unwrap();
            assert!(
                crate::services::assistant_nyxagent_runtime::turn_remaining(&queued)
                    > cap - Duration::from_secs(2)
            );
            assert!(
                survivor::take_one(&next, &queued, workers::instance())
                    .await
                    .is_err()
            );
            let overdue = start - chrono::Duration::from_std(cap).unwrap();
            rows.update_one(
                filter,
                doc! {"$set":{"active_turn.started_at":bson::DateTime::from_chrono(overdue)}},
            )
            .await
            .unwrap();
            let expired = engine::get(&db, OWNER, &row.id).await.unwrap();
            assert!(
                survivor::take_one(&next, &expired, workers::instance())
                    .await
                    .unwrap()
            );
            let done = engine::get(&db, OWNER, &row.id).await.unwrap();
            assert!(done.active_turn.is_none() && done.stopped_window.is_some());
            assert_eq!(posts.load(Ordering::SeqCst), 1);
            assert_eq!(
                engine::messages(&db, OWNER, &row.id, 5, None)
                    .await
                    .unwrap()
                    .last()
                    .unwrap()
                    .error_code
                    .as_deref(),
                Some("outcome_unknown")
            );
            server.abort();
            let _ = server.await;
            db.drop().await.unwrap();
            continue;
        }
        let mut completed_tool = None;
        if mid_answer && review.is_some() {
            let receipt = engine::activity_started_for_turn(
                &db,
                OWNER,
                &row.id,
                &turn.turn_id,
                "handoff_test_tool",
            )
            .await
            .unwrap()
            .unwrap();
            engine::activity_finished_for_turn(
                &db,
                OWNER,
                &row.id,
                &turn.turn_id,
                &receipt,
                true,
                None,
            )
            .await
            .unwrap();
            completed_tool = Some(receipt);
        }
        if trimmed {
            let client = Uuid::new_v4().to_string();
            for _ in 0..2 {
                let mut request = Request::builder().method("POST").body(Body::from(json!({"text":"Use the shorter answer","turn_id":&turn.turn_id,"clientRequestId":&client}).to_string())).unwrap();
                request.extensions_mut().insert(SERVER_TURN_POLICY);
                let mut bearer = test_auth_user(OWNER);
                bearer.auth_method = crate::mw::auth::AuthMethod::AccessToken;
                let response = steer(State(next.clone()), bearer, Path(row.id.clone()), request)
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
            }
            assert_eq!(steered.load(Ordering::SeqCst), 1);
        }
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
            if matches!(
                cancel,
                StopReply::Revoke | StopReply::Destroy | StopReply::Rotate | StopReply::Disable
            ) {
                let credential =
                    credentials::load_for_conversation(&db, &next.encryption_keys, OWNER, &row.id)
                        .await
                        .unwrap()
                        .unwrap();
                if matches!(cancel, StopReply::Revoke) {
                    let _ = crate::handlers::api_keys::delete_key(
                        State(next.clone()),
                        test_auth_user(OWNER),
                        Default::default(),
                        Path(credential.api_key_id.clone()),
                    )
                    .await
                    .unwrap();
                } else if matches!(cancel, StopReply::Rotate) {
                    let _ = crate::handlers::api_keys::rotate_key(
                        State(next.clone()),
                        test_auth_user(OWNER),
                        Default::default(),
                        Path(credential.api_key_id.clone()),
                    )
                    .await
                    .unwrap();
                } else if matches!(cancel, StopReply::Disable) {
                    use crate::services::role_service;
                    role_service::seed_system_roles(&db).await.unwrap();
                    let admin = Uuid::new_v4().to_string();
                    let mut user = test_user(&admin, UserType::Person);
                    user.role_ids.push(
                        role_service::get_platform_role_ids(&db)
                            .await
                            .unwrap()
                            .admin,
                    );
                    db.collection(USERS).insert_one(user).await.unwrap();
                    let _ = crate::handlers::admin::set_user_status(
                        State(next.clone()),
                        test_auth_user(&admin),
                        Default::default(),
                        HeaderMap::new(),
                        Path(OWNER.into()),
                        Json(crate::handlers::admin::SetStatusRequest { is_active: false }),
                    )
                    .await
                    .unwrap();
                } else {
                    crate::handlers::assistant_team::destroy_agent(
                        &next,
                        OWNER,
                        row.agent_id.as_deref().unwrap(),
                    )
                    .await
                    .unwrap();
                }
                assert!(cancellations.load(Ordering::SeqCst) > 0);
                assert!(
                    crate::services::key_service::validate_api_key(&db, &credential.raw_key)
                        .await
                        .is_err()
                );
                assert_eq!(posts.load(Ordering::SeqCst), 1);
                server.abort();
                let _ = server.await;
                db.drop().await.unwrap();
                continue;
            }
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
        let previous = offer.clone();
        let offered = if trimmed {
            let intermediate = EngineWorkerInstance {
                id: Uuid::new_v4().to_string(),
                hostname: None,
            };
            let claimed = restart::claim(&db, &offered, &intermediate)
                .await
                .unwrap()
                .unwrap();
            assert!(
                crate::services::assistant_service_execution::finish_model_attempt(
                    &db, &reply_id, &row
                )
                .await
                .unwrap()
                .is_none()
            );
            assert_eq!(
                crate::services::assistant_service_execution::finish_model_attempt(
                    &db, &reply_id, &claimed
                )
                .await
                .unwrap()
                .as_deref(),
                Some("review the connection")
            );
            assert!(
                restart::offer(&db, &claimed, restart::Loss::Shutdown(&intermediate.id))
                    .await
                    .unwrap()
            );
            restart::due(&db, &workers::instance().id)
                .await
                .unwrap()
                .pop()
                .unwrap()
        } else {
            offered
        };
        let offer = offered
            .active_turn
            .as_ref()
            .unwrap()
            .restart_offer
            .as_ref()
            .unwrap();
        let (left, right) = tokio::join!(
            survivor::take_one(&next, &offered, workers::instance()),
            survivor::take_one(&next, &offered, workers::instance())
        );
        assert_ne!(left.unwrap(), right.unwrap());
        wait_for_phase("survivor runner completed", next.shutdown.idle()).await;
        let saved = engine::get(&db, OWNER, &offered.id).await.unwrap();
        assert!(saved.active_turn.is_none());
        if funded {
            use crate::models::{billing_wallet::BillingWallet, credits::Credits};
            let usage = wait_for_phase("durable usage releases", released_usage(&next)).await;
            let controls: Vec<_> = usage
                .iter()
                .filter(|record| record.service_id.as_deref() == Some(service.id.as_str()))
                .collect();
            assert!(controls.len() >= 3);
            assert!(
                controls
                    .iter()
                    .all(|record| record.billing_owner_id == OWNER && record.released)
            );
            let funded: Vec<_> = controls
                .iter()
                .filter_map(|record| record.funding.as_ref()?.wallet_funded)
                .filter(|amount| *amount != Credits::ZERO)
                .collect();
            assert_eq!(funded, vec![Credits::from_whole(1)]);
            let wallet = db
                .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
                .find_one(doc! {"owner_id":OWNER})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(wallet.reserved_credits, Credits::ZERO);
        }
        assert_eq!(saved.stopped_window.is_some(), review.is_some());
        if let Some(request_id) = approval {
            let requests =
                db.collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME);
            let filter =
                doc! {"requester_id": &row.credential_api_key_id, "tool_call_id": "handoff-call"};
            assert_eq!(requests.count_documents(filter.clone()).await.unwrap(), 1);
            let pending = requests.find_one(filter).await.unwrap().unwrap();
            assert_eq!(pending.get_str("_id"), Ok(request_id.as_str()));
            assert_eq!(pending.get_str("status"), Ok("pending"));
        }
        let mut messages = engine::messages(&db, OWNER, &saved.id, 10, None)
            .await
            .unwrap();
        if trimmed {
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message
                        .steering
                        .as_ref()
                        .is_some_and(|receipt| receipt.outcome == "applied"))
                    .count(),
                1
            );
            messages.retain(|message| message.steering.is_none());
        }
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].id, reply_id);
        if let Some(receipt) = completed_tool {
            assert!(
                messages[1]
                    .activities
                    .iter()
                    .any(|activity| activity.id == receipt && activity.status == "completed")
            );
            assert!(cancellations.load(Ordering::SeqCst) > 0);
        }
        assert!(
            messages[1]
                .turn_metadata
                .as_ref()
                .unwrap()
                .continued_after_restart
        );
        for transition in ["offered", "claimed", "settled"] {
            let count = db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME).count_documents(doc! {
                "event_type":"assistant_turn_handoff", "event_data.turn_id":&messages[1].turn_id,
                "event_data.generation":&offer.generation,"event_data.from_worker":&offer.from_worker,"event_data.transition":transition
            }).await.unwrap();
            assert_eq!(count, 1, "{transition}");
        }
        if trimmed {
            for (transition, expected) in [("offered", 1), ("claimed", 1), ("settled", 0)] {
                let count = db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME).count_documents(doc! {
                    "event_type":"assistant_turn_handoff", "event_data.turn_id":&messages[1].turn_id,
                    "event_data.generation":&previous.generation, "event_data.from_worker":&previous.from_worker,
                    "event_data.transition":transition
                }).await.unwrap();
                assert_eq!(count, expected);
            }
        }
        if review.is_some() {
            assert_eq!(messages[1].error_code.as_deref(), Some("outcome_unknown"));
            assert_eq!(messages[1].text, if mid_answer { "hello" } else { "" });
            if let Some((in_flight, provider)) = pending_auth {
                let mut successor = start.clone();
                successor.conversation_id = Some(row.id.clone());
                let claimed =
                    engine::claim_turn(&db, OWNER, successor, &next.encryption_keys, None, None)
                        .await
                        .unwrap();
                assert!(
                    claimed.conversation.credential_version_floor()
                        > in_flight.thread_key_version()
                );
                assert_eq!(
                    read_granted_service(&next, &in_flight).await,
                    StatusCode::UNAUTHORIZED
                );
                assert_eq!(provider.received_requests().await.unwrap().len(), 1);
                let result = Box::pin(crate::handlers::mcp::get_mcp_config(
                    State(next.clone()),
                    in_flight,
                ))
                .await;
                assert!(matches!(result, Err(AppError::Unauthorized(_))));
            }
        } else {
            assert_eq!(
                messages[1].text,
                if discard_cursor.is_some() {
                    RETRIED_ANSWER
                } else {
                    "hello world"
                }
            );
            assert_eq!(messages[1].status, "completed");
        }
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        assert_eq!(
            reads.load(Ordering::SeqCst),
            if review.is_some() {
                0
            } else if discard_cursor.is_some() {
                2
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
