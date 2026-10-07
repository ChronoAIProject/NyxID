use super::*;
use crate::models::assistant_conversation::{COLLECTION_NAME as CONVERSATIONS, RunningResponse};
use crate::services::assistant_steering as steering_service;

struct Fixture {
    state: AppState,
    calls: Captures,
    server: tokio::task::JoinHandle<()>,
    capability_calls: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Semaphore>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture(status: u16, code: &'static str, support: bool) -> Fixture {
    let (state, _, old_server) = setup(None, Duration::ZERO).await;
    old_server.abort();
    let calls: Captures = Arc::new(Mutex::new(vec![]));
    let sink = calls.clone();
    let capability_calls = Arc::new(AtomicUsize::new(0));
    let counter = capability_calls.clone();
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let gate = release.clone();
    let attempt = Arc::new(AtomicUsize::new(0));
    let upstream = Router::new().route("/v1/capabilities", axum::routing::get(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        async move { Json(if support { json!({"steer":{"version":1,"protocol":"nyxagent-steer-v1","input":["text"],"max_chars":32000,"max_per_turn":20}}) } else { json!({}) }) }
    })).route("/v1/conversations/{id}/steer", post(move |uri: Uri, headers: HeaderMap, Json(body): Json<Value>| {
        let sink = sink.clone();
        async move {
            let response_id = body["expected_response_id"].clone();
            sink.lock().await.push(Capture { uri, headers, body });
            tokio::time::sleep(if code == "timeout_test" { Duration::from_secs(16) } else { Duration::from_millis(30) }).await;
            (StatusCode::from_u16(status).unwrap(), Json(if status == 200 {
                json!({"object":"conversation.steer", "conversation_id":SESSION,"response_id":response_id,"status":"accepted"})
            } else { json!({"error":{"code":code,"message":"SECRET upstream detail"}}) }))
        }
    })).route("/v1/responses", post(move || {
        let gate = gate.clone();
        let attempt = attempt.fetch_add(1, Ordering::SeqCst);
        async move {
            let stream = async_stream::stream! {
                let response_id = format!("resp_{}_{}", &SESSION[5..], if attempt == 0 { "a".repeat(32) } else { "b".repeat(32) });
                yield Ok::<_, Infallible>(Event::default().data(json!({"type":"response.created","sequence_number":0,"response":{"id":response_id,"conversation":{"id":SESSION},"status":"in_progress"}}).to_string()));
                gate.acquire().await.unwrap().forget();
                yield Ok::<_, Infallible>(Event::default().data(json!({"type":if attempt == 0 {"response.failed"} else {"response.completed"}, "sequence_number":1,
                    "response":{"id":response_id,"conversation":{"id":SESSION},"status":if attempt == 0 {"failed"} else {"completed"},
                    "error":if attempt == 0 {json!({"code":"turn_timeout"})} else {Value::Null},
                    "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Working"}]}]}}).to_string()));
            };
            Sse::new(stream)
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":base}},
        )
        .await
        .unwrap();
    Fixture {
        state,
        calls,
        server,
        capability_calls,
        release,
    }
}

async fn running(f: &Fixture) -> AssistantConversation {
    let request =
        serde_json::from_value::<engine::TurnRequest>(json!({"text":"Original request"})).unwrap();
    let row = Box::pin(engine::begin_turn(
        &f.state.db,
        OWNER,
        &request,
        &f.state.encryption_keys,
    ))
    .await
    .unwrap();
    steering_service::set_running_response(
        &f.state.db,
        &row,
        Some(&RunningResponse {
            credential_api_key_id: row.credential_api_key_id.clone(),
            response_id: RESPONSE.into(),
            session_id: SESSION.into(),
        }),
    )
    .await
    .unwrap();
    engine::get(&f.state.db, OWNER, &row.id).await.unwrap()
}
fn request(row: &AssistantConversation, id: &str, text: &str) -> Request<Body> {
    let mut request = Request::builder().method("POST").header("cookie","SECRET caller cookie")
        .body(Body::from(json!({"text":text,"turn_id":row.active_turn.as_ref().unwrap().turn_id,"clientRequestId":id}).to_string())).unwrap();
    request.extensions_mut().insert(BillingRoutePolicy::Metered(
        crate::services::billing::BillingIngress::Proxy,
    ));
    request
}
async fn submit(f: &Fixture, row: &AssistantConversation, id: &str, text: &str) -> Response {
    steer(
        State(f.state.clone()),
        test_auth_user(OWNER),
        Path(row.id.clone()),
        request(row, id, text),
    )
    .await
    .unwrap()
}
async fn value(response: Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 32768)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn assistant_steer_accepted_uses_thread_key_and_marks_transcript() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    let id = Uuid::new_v4().to_string();
    let reply = submit(&f, &row, &id, "  Follow this direction  ").await;
    assert_eq!(reply.status(), 200);
    assert_eq!(value(reply).await["outcome"], "applied");
    let calls = f.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].headers["idempotency-key"], id);
    assert_eq!(calls[0].headers["accept"], "application/json");
    assert!(calls[0].headers.get("cookie").is_none());
    let credential =
        credentials::load_for_conversation(&f.state.db, &f.state.encryption_keys, OWNER, &row.id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        calls[0].headers["authorization"],
        format!("Bearer {}", credential.raw_key.as_str())
    );
    assert_eq!(
        calls[0].body,
        json!({"input":"  Follow this direction  ","expected_response_id":RESPONSE})
    );
    let history = engine::messages(&f.state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].role, "user");
    assert_eq!(
        history[1].turn_id,
        row.active_turn.as_ref().unwrap().turn_id
    );
    assert_eq!(history[1].steering.as_ref().unwrap().outcome, "applied");
    assert_eq!(
        engine::get(&f.state.db, OWNER, &row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .turn_id,
        history[0].turn_id
    );
}

#[tokio::test]
async fn assistant_steer_concurrent_replicas_and_late_retries_never_duplicate() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    let id = Uuid::new_v4().to_string();
    let (a, b) = tokio::join!(
        submit(&f, &row, &id, "Guidance"),
        submit(&f, &row, &id, "Guidance")
    );
    assert_eq!(a.status(), 200);
    assert_eq!(b.status(), 200);
    assert_eq!(submit(&f, &row, &id, "Guidance").await.status(), 200);
    assert_eq!(
        submit(&f, &row, &id, "Changed guidance").await.status(),
        409
    );
    engine::finish_turn(
        &f.state.db,
        &row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &TurnResult {
            text: "Done".into(),
            session_id: Some(SESSION.into()),
            response_id: Some(RESPONSE.into()),
            error: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(submit(&f, &row, &id, "Guidance").await.status(), 200);
    assert_eq!(f.calls.lock().await.len(), 1);
    assert_eq!(f.capability_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        engine::messages(&f.state.db, OWNER, &row.id, 10, None)
            .await
            .unwrap()
            .len(),
        3
    );
}

macro_rules! error_case {
    ($name:ident,$status:expr,$code:expr,$outcome:expr) => {
        #[tokio::test]
        async fn $name() {
            let f = fixture($status, $code, true).await;
            let row = running(&f).await;
            let response = submit(&f, &row, "request-1", "Guidance").await;
            assert_eq!(response.status(), $status);
            let body = value(response).await;
            assert_eq!(body["error"], $code);
            assert!(!body.to_string().contains("SECRET"));
            let messages = engine::messages(&f.state.db, OWNER, &row.id, 10, None)
                .await
                .unwrap();
            let receipt = messages.last().unwrap().steering.as_ref().unwrap();
            assert_eq!(receipt.outcome, $outcome);
            assert_eq!(receipt.code.as_deref(), Some($code));
            assert_eq!(
                submit(&f, &row, "request-1", "Guidance").await.status(),
                $status
            );
            assert_eq!(f.calls.lock().await.len(), 1);
        }
    };
}
error_case!(
    assistant_steer_no_active_turn,
    409,
    "no_active_turn",
    "refused"
);
error_case!(
    assistant_steer_response_mismatch,
    409,
    "response_mismatch",
    "refused"
);
error_case!(
    assistant_steer_idempotency_conflict,
    409,
    "idempotency_conflict",
    "refused"
);
error_case!(assistant_steer_limit, 429, "steer_limit", "refused");
error_case!(
    assistant_steer_invalid_request,
    400,
    "invalid_request",
    "refused"
);
error_case!(
    assistant_steer_unknown_conversation,
    404,
    "not_found",
    "refused"
);
error_case!(
    assistant_steer_unavailable_is_uncertain,
    503,
    "steer_unavailable",
    "may_not_have_applied"
);

#[tokio::test]
async fn assistant_steer_starting_is_retryable_without_a_receipt() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    steering_service::set_running_response(&f.state.db, &row, None)
        .await
        .unwrap();
    let response = submit(&f, &row, "same-key", "Guidance").await;
    assert_eq!(response.status(), 409);
    assert_eq!(response.headers()["retry-after"], "1");
    assert_eq!(value(response).await["error"], "starting");
    assert_eq!(
        engine::messages(&f.state.db, OWNER, &row.id, 10, None)
            .await
            .unwrap()
            .len(),
        1
    );
    steering_service::set_running_response(
        &f.state.db,
        &row,
        row.active_turn
            .as_ref()
            .unwrap()
            .running_response
            .as_deref(),
    )
    .await
    .unwrap();
    assert_eq!(submit(&f, &row, "same-key", "Guidance").await.status(), 200);
}

#[tokio::test]
async fn assistant_steer_stop_and_stale_heartbeat_refuse_without_dispatch() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    engine::request_stop(&f.state.db, OWNER, &row.id)
        .await
        .unwrap();
    assert_eq!(
        value(submit(&f, &row, "stop", "Guidance").await).await["error"],
        "stop_pending"
    );
    assert!(
        !steering_service::set_running_response(
            &f.state.db,
            &row,
            row.active_turn
                .as_ref()
                .unwrap()
                .running_response
                .as_deref()
        )
        .await
        .unwrap()
    );
    f.state.db.collection::<AssistantConversation>(CONVERSATIONS).update_one(doc! {"_id":&row.id},doc! {"$set":{"active_turn.stop_requested":false,"active_turn.heartbeat_at":mongodb::bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(76))}}).await.unwrap();
    assert_eq!(
        value(submit(&f, &row, "stale", "Guidance").await).await["error"],
        "no_active_turn"
    );
    assert!(f.calls.lock().await.is_empty());
}

#[tokio::test]
async fn assistant_steer_requires_first_party_owner_and_web_turn() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    let mut auth = test_auth_user(OWNER);
    auth.api_key_id = Some("key".into());
    assert!(matches!(
        steer(
            State(f.state.clone()),
            auth,
            Path(row.id.clone()),
            request(&row, "key", "Guidance")
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    let mut auth = test_auth_user(OWNER);
    auth.oauth_client_id = Some("oauth".into());
    assert!(matches!(
        steer(
            State(f.state.clone()),
            auth,
            Path(row.id.clone()),
            request(&row, "oauth", "Guidance")
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        steer(
            State(f.state.clone()),
            test_auth_user("22345678-1234-4123-8123-123456789012"),
            Path(row.id.clone()),
            request(&row, "other", "Guidance")
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    for change in [
        doc! {"guest_turn":true},
        doc! {"guest_turn":false,"group_id":"group"},
        doc! {"group_id":mongodb::bson::Bson::Null,"voice_parent_conversation_id":"voice"},
        doc! {"voice_parent_conversation_id":mongodb::bson::Bson::Null,"active_turn.origin":"channel"},
    ] {
        f.state
            .db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .update_one(doc! {"_id":&row.id}, doc! {"$set":change})
            .await
            .unwrap();
        assert_eq!(
            value(submit(&f, &row, "unsupported", "Guidance").await).await["error"],
            "steer_unsupported"
        );
    }
    assert!(f.calls.lock().await.is_empty());
}

#[tokio::test]
async fn assistant_steer_no_capability_keeps_turn_active_behavior() {
    let f = fixture(200, "", false).await;
    let row = running(&f).await;
    assert!(matches!(
        steer(
            State(f.state.clone()),
            test_auth_user(OWNER),
            Path(row.id.clone()),
            request(&row, "none", "Guidance")
        )
        .await,
        Err(AppError::AssistantTurnActive)
    ));
    assert!(f.calls.lock().await.is_empty());
    assert_eq!(
        engine::messages(&f.state.db, OWNER, &row.id, 10, None)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn assistant_steer_input_is_bounded_text_only() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    for body in [
        json!({"text":"","turn_id":row.active_turn.as_ref().unwrap().turn_id,"clientRequestId":"id"}),
        json!({"text":"ok","turn_id":row.active_turn.as_ref().unwrap().turn_id,"clientRequestId":"id","images":[]}),
        json!({"text":[{"type":"input_image"}],"turn_id":row.active_turn.as_ref().unwrap().turn_id,"clientRequestId":"id"}),
        json!({"text":"界".repeat(32001),"turn_id":row.active_turn.as_ref().unwrap().turn_id,"clientRequestId":"id"}),
    ] {
        let response = steer(
            State(f.state.clone()),
            test_auth_user(OWNER),
            Path(row.id.clone()),
            Request::new(Body::from(body.to_string())),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 400);
    }
    assert_eq!(
        submit(&f, &row, "unicode", "界".repeat(32000).as_str())
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn assistant_steer_excluded_from_anchors_and_only_labelled_in_recap() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    for i in 0..22 {
        submit(&f, &row, &format!("guidance-{i}"), "NEW GUIDANCE").await;
    }
    let previous = engine::previous_user_message(&f.state.db, OWNER, &row.id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(previous.text, "Original request");
    let history = engine::messages(&f.state.db, OWNER, &row.id, 100, None)
        .await
        .unwrap();
    let prepared = crate::services::assistant_instruction_context::Prepared::new(
        b"test key",
        &row,
        None,
        &history,
    )
    .unwrap();
    assert!(
        !prepared
            .instructions(&history, false)
            .contains("NEW GUIDANCE")
    );
    assert!(!prepared.input("", "Next request").contains("NEW GUIDANCE"));
    assert!(
        prepared
            .instructions(&history, true)
            .contains("user (steering, applied): NEW GUIDANCE")
    );
}

async fn wait_response(f: &Fixture, suffix: char) -> AssistantConversation {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let rows = engine::list(&f.state.db, OWNER, 1, None, None)
                .await
                .unwrap();
            if let Some(row) = rows.into_iter().next()
                && row
                    .active_turn
                    .as_ref()
                    .and_then(|t| t.running_response.as_deref())
                    .is_some_and(|r| r.response_id.ends_with(suffix))
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn assistant_steer_response_created_is_persisted_and_updated_across_continuations() {
    let f = fixture(200, "", true).await;
    let response = turns(
        State(f.state.clone()),
        test_auth_user(OWNER),
        turn_request(None),
    )
    .await
    .unwrap();
    drop(response);
    let first = wait_response(&f, 'a').await;
    assert_eq!(
        f.capability_calls.load(Ordering::SeqCst),
        0,
        "ordinary text turns add no capability calls"
    );
    assert!(first.nyxagent_last_response_id.is_none());
    assert_eq!(submit(&f, &first, "first", "Guidance").await.status(), 200);
    f.release.add_permits(1);
    let next = wait_response(&f, 'b').await;
    assert_eq!(
        first.active_turn.as_ref().unwrap().turn_id,
        next.active_turn.as_ref().unwrap().turn_id
    );
    assert_eq!(next.active_turn.as_ref().unwrap().continuations, 1);
    assert_eq!(
        submit(&f, &next, "second", "More guidance").await.status(),
        200
    );
    let calls = f.calls.lock().await;
    assert!(
        calls[0].body["expected_response_id"]
            .as_str()
            .unwrap()
            .ends_with('a')
    );
    assert!(
        calls[1].body["expected_response_id"]
            .as_str()
            .unwrap()
            .ends_with('b')
    );
    drop(calls);
    f.release.add_permits(1);
    let final_row = settled(&f.state).await;
    assert!(final_row.nyxagent_last_response_id.unwrap().ends_with('b'));
    assert!(
        !steering_service::set_running_response(
            &f.state.db,
            &first,
            first
                .active_turn
                .as_ref()
                .unwrap()
                .running_response
                .as_deref()
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn assistant_steer_delivery_timeout_is_durable_and_never_retried() {
    let f = fixture(200, "timeout_test", true).await;
    let row = running(&f).await;
    let response = submit(&f, &row, "timeout", "Guidance").await;
    assert_eq!(response.status(), 503);
    assert_eq!(value(response).await["error"], "steer_unavailable");
    assert_eq!(submit(&f, &row, "timeout", "Guidance").await.status(), 503);
    assert_eq!(f.calls.lock().await.len(), 1);
    let messages = engine::messages(&f.state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(
        messages.last().unwrap().steering.as_ref().unwrap().outcome,
        "may_not_have_applied"
    );
}

#[tokio::test]
async fn assistant_steer_credential_generation_fences_old_response() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    f.state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"credential_api_key_id": "rotated-successor"}},
        )
        .await
        .unwrap();
    assert!(
        !steering_service::set_running_response(
            &f.state.db,
            &row,
            row.active_turn
                .as_ref()
                .unwrap()
                .running_response
                .as_deref()
        )
        .await
        .unwrap()
    );
    let current = engine::get(&f.state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(
        steering_service::unavailable(&current, &row.active_turn.as_ref().unwrap().turn_id),
        Some("steer_unavailable")
    );
    assert!(f.calls.lock().await.is_empty());
}

#[tokio::test]
async fn assistant_steer_web_event_keeps_original_turn_origin_and_loop_budget() {
    let f = fixture(200, "", true).await;
    let row = running(&f).await;
    f.state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"active_turn.origin": "event", "event_streak": 2}},
        )
        .await
        .unwrap();
    assert_eq!(
        submit(&f, &row, "event-guidance", "Guidance")
            .await
            .status(),
        200
    );
    let current = engine::get(&f.state.db, OWNER, &row.id).await.unwrap();
    assert_eq!(
        current.active_turn.unwrap().origin,
        crate::models::assistant_conversation::TurnOrigin::Event
    );
    let raw = f
        .state
        .db
        .collection::<mongodb::bson::Document>(CONVERSATIONS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(raw.get_i32("event_streak").unwrap(), 2);
}
