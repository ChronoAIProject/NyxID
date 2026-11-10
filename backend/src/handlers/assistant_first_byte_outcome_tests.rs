//! Real turn -> authenticated MCP callback -> schedule write -> failure/retry.
use super::*;

async fn tool_first_fixture(
    label: &str,
    delay_headers: bool,
    finish: bool,
) -> (
    AppState,
    Arc<Mutex<Vec<Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let db = connect_transaction_test_database(label).await;
    engine::ensure_indexes(&db).await.unwrap();
    db.collection(USERS)
        .insert_one(test_user(OWNER, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db);
    crate::services::assistant_settings_service::update(
        &state.db,
        OWNER,
        crate::services::assistant_settings_service::Update {
            timezone: Some("Asia/Shanghai".into()),
            access_mode: Some(crate::models::assistant_settings::AccessMode::Autopilot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let results = Arc::new(Mutex::new(Vec::new()));
    let saved = results.clone();
    let callback_state = state.clone();
    let server = register_upstream(&state, Router::new().route("/v1/responses", post(
        move |headers: HeaderMap| {
            let state = callback_state.clone();
            let results = saved.clone();
            async move {
                // NyxAgent receives the thread key as Bearer, then uses
                // x-api-key for its stateless MCP callback (Bearer here is JWT).
                let retry = !results.lock().await.is_empty();
                let mut arguments = json!({
                    "label":"iOS QA Automation", "instruction":"Post only the QA check. Do not run now.",
                    "schedule":{"kind":"at", "at":"2099-12-31T23:00:00+08:00"}
                });
                let params = if retry {
                    json!({"name":"nyx__call_tool", "arguments":{
                        "tool_name":"nyxid__create_schedule", "arguments_json":arguments.to_string()}})
                } else {
                    // Full access needs no card, but a stale approval handle must
                    // not make an otherwise identical retry a different write.
                    arguments["acknowledgement_id"] = json!("previous-confirmation");
                    json!({"name":"nyxid__create_schedule", "arguments":arguments})
                };
                let request = Request::builder().method("POST").uri("/mcp")
                    .header("x-api-key", headers.get("authorization").unwrap().to_str().unwrap().strip_prefix("Bearer ").unwrap())
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                        "params":params}).to_string())).unwrap();
                let response = Box::pin(crate::handlers::mcp_transport::mcp_post(
                    State(state), axum::Extension(BillingRoutePolicy::Metered(
                        crate::services::billing::BillingIngress::Mcp)), request)).await;
                let bytes = axum::body::to_bytes(response.into_body(), 1_000_000).await.unwrap();
                results.lock().await.push(serde_json::from_slice(&bytes).unwrap());
                let delay = if finish { Duration::from_millis(850) } else { Duration::from_secs(10) };
                if delay_headers { tokio::time::sleep(delay).await; }
                let stream = async_stream::stream! {
                    if !delay_headers { tokio::time::sleep(delay).await; }
                    yield Ok::<_, Infallible>(axum::body::Bytes::from(format!("data: {}\n\n", json!({
                        "type":"response.completed","sequence_number":0,
                        "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":"completed",
                            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}]}
                    }))));
                };
                ([("content-type", "text/event-stream")], Body::from_stream(stream)).into_response()
            }
        }
    ))).await;
    (state, results, server)
}

fn tool_first_bounds() {
    TEST_STREAM_BOUNDS.with(|bounds| {
        bounds.set(Some(StreamBounds {
            first_byte: Duration::from_millis(700),
            no_progress: Duration::from_secs(3),
            transport_idle: Duration::from_secs(2),
            turn_cap: Duration::from_secs(30),
        }))
    });
}

#[tokio::test]
async fn first_byte_outcome_schedule_created_before_timeout_and_retry_is_fenced() {
    tool_first_bounds();
    let (state, results, server) = tool_first_fixture("first_byte_schedule", true, false).await;
    let failed = run_turn_to_reply(&state).await;
    assert_eq!(failed.error_code.as_deref(), Some("first_byte_timeout"));
    assert_eq!(failed.error_detail.as_ref().unwrap().stage, "first_byte");
    assert_eq!(failed.turn_metadata.as_ref().unwrap().attempts, Some(1));
    assert!(failed.text.is_empty());
    assert_eq!(failed.activity_count, Some(1), "{:?}", results.lock().await);
    assert!(failed.activities[0].retry_identity.is_some());
    assert_ne!(
        failed.activities[0].digest,
        failed.activities[0].retry_identity
    );
    assert_eq!(failed.activities[0].label, "nyxid__create_schedule");
    assert_eq!(
        failed.activities[0].status,
        "completed",
        "{:?}",
        results.lock().await
    );
    let outcome = serde_json::to_value(MessageResponse::from(failed.clone())).unwrap();
    assert_eq!(outcome["action_outcome"]["complete"], true);
    assert_eq!(
        outcome["action_outcome"]["actions"][0]["summary"],
        "Created a scheduled automation"
    );
    assert!(
        outcome["action_outcome"]["retry_warning"]
            .as_str()
            .unwrap()
            .contains("Retrying may repeat it")
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::trigger::COLLECTION_NAME)
            .count_documents(doc! {"user_id": OWNER})
            .await
            .unwrap(),
        1
    );

    let mut request = turn_request(Some(&failed.conversation_id));
    *request.body_mut() = Body::from(
        json!({"conversation_id":failed.conversation_id,
        "text":"hello", "retry_of":failed.turn_id})
        .to_string(),
    );
    let retry = turns(State(state.clone()), test_auth_user(OWNER), request)
        .await
        .unwrap();
    axum::body::to_bytes(retry.into_body(), 1_000_000)
        .await
        .unwrap();
    settled(&state).await;
    let results = results.lock().await;
    assert_eq!(results.len(), 2, "no automatic retry of ambiguous timeout");
    assert_eq!(results[0]["result"]["isError"], false, "{}", results[0]);
    assert_eq!(results[1]["result"]["isError"], true, "{}", results[1]);
    assert!(results[1].to_string().contains("retry_duplicate_action"));
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::trigger::COLLECTION_NAME)
            .count_documents(doc! {"user_id": OWNER})
            .await
            .unwrap(),
        1
    );
    let retried = engine::messages(&state.db, OWNER, &failed.conversation_id, 1, None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let retry_outcome = serde_json::to_value(MessageResponse::from(retried)).unwrap();
    assert!(
        retry_outcome["action_outcome"]["actions"][0]["summary"]
            .as_str()
            .unwrap()
            .starts_with("Previously completed; not repeated")
    );
    server.abort();
}

#[tokio::test]
async fn first_byte_tool_first_extends_same_header_and_body_request() {
    tool_first_bounds();
    for delay_headers in [true, false] {
        let (state, results, server) =
            tool_first_fixture("tool_first_progress", delay_headers, true).await;
        let reply = run_turn_to_reply(&state).await;
        assert_eq!(reply.status, "completed", "{:?}", reply.error_code);
        assert_eq!(reply.text, "Done");
        assert_eq!(results.lock().await.len(), 1);
        server.abort();
    }
}

#[tokio::test]
async fn first_byte_old_continuation_activity_does_not_extend_deadline() {
    let f = Box::pin(
        crate::services::assistant_authority_tests::orchestrator_fixture(
            "old_continuation_deadline",
        ),
    )
    .await;
    let turn = f.row.active_turn.as_ref().unwrap();
    engine::activity_started_with_digest(
        &f.state.db,
        &f.owner,
        &f.row.id,
        &turn.turn_id,
        "nyxid__create_schedule",
        Some("digest"),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    let mut deadline = crate::services::assistant_start_retry::FirstByteDeadline::new(
        Duration::from_millis(25),
        Duration::from_secs(5),
    );
    let start = tokio::time::Instant::now();
    let result = deadline
        .wait(
            &f.state.db,
            &f.row,
            tokio::time::sleep(Duration::from_secs(1)),
        )
        .await;
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn first_byte_outcome_legacy_uncertain_and_truncated_evidence() {
    let mut row = bson::from_document::<AssistantMessage>(doc! {
        "_id":"old", "conversation_id":"c", "user_id":OWNER, "seq":1_i64,
        "turn_id":"t", "role":"assistant", "text":"", "status":"failed",
        "created_at":bson::DateTime::now(), "activities":[], "attachments":[]
    })
    .unwrap();
    assert!(ActionOutcomeResponse::from_message(&row).is_none());
    row.activity_count = Some(0);
    assert_eq!(
        ActionOutcomeResponse::from_message(&row).unwrap().summary,
        "No actions were taken."
    );
    row.error_code = Some("outcome_unknown".into());
    let unknown = ActionOutcomeResponse::from_message(&row).unwrap();
    assert!(!unknown.complete);
    assert!(unknown.retry_warning.is_some());
    assert!(!unknown.summary.contains("No actions"));
    row.error_code = None;
    row.activity_count = Some(1);
    let incomplete = ActionOutcomeResponse::from_message(&row).unwrap();
    assert!(!incomplete.complete);
    assert!(incomplete.retry_warning.is_some());
    row.activities
        .push(crate::models::assistant_conversation::TurnActivity {
            decision: None,
            retry_identity: None,
            id: "a".into(),
            label: "nyxid__create_schedule".into(),
            status: "error".into(),
            digest: Some("digest".into()),
            machine: None,
            failure: None,
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
        });
    let uncertain = ActionOutcomeResponse::from_message(&row).unwrap();
    assert_eq!(uncertain.actions[0].status, "uncertain");
    assert!(!uncertain.actions[0].summary.contains("Created"));
    row.activity_count = Some(41);
    row.activities[0].status = "completed".into();
    assert!(!ActionOutcomeResponse::from_message(&row).unwrap().complete);
    row.status = "completed".into();
    assert!(ActionOutcomeResponse::from_message(&row).is_none());
}
