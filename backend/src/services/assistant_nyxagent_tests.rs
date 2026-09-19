use super::*;
use crate::models::user::{COLLECTION_NAME as USERS, UserType};
use crate::services::assistant_agent_credential_service as credentials;
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};

fn request(id: Option<&str>, text: &str) -> TurnRequest {
    TurnRequest {
        conversation_id: id.map(str::to_owned),
        text: text.into(),
        model: None,
        access_mode: None,
    }
}
fn terminal(kind: &str, seq: u64, text: &str) -> Value {
    let sid = "1234567890abcdef1234567890abcdef";
    json!({"type":kind,"sequence_number":seq,"response":{
        "id":format!("resp_{sid}_1234567890abcdef1234567890abcdef"),
        "conversation":{"id":format!("conv_{sid}")},
        "status":if kind == "response.completed" {"completed"} else {"failed"},
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text}],
        }],
        "error":{"code":"turn_timeout","message":"SECRET upstream detail"}
    }})
}
fn frame(value: Value) -> Vec<u8> {
    format!("event: ignored\r\ndata: {value}\r\n\r\n").into_bytes()
}

#[test]
fn closed_request_grammar_and_unicode_limit() {
    for value in [
        json!({"text":""}),
        json!({"text":"  "}),
        json!({"text":"hello","user_id":"other"}),
        json!({"text":"hello","conversation_id":"chatc-123"}),
        json!({"text":"hello","model":"gpt-5"}),
        json!({"text":"hello","access_mode":"unrestricted"}),
        json!({
            "text": "hello",
            "conversation_id": "nyxa-1234567890abcdef1234567890abcdef",
            "access_mode": "full",
        }),
        json!({"text":"hello","conversation_id":"nyxa-ABCDEF1234567890abcdef1234567890"}),
    ] {
        assert!(
            parse_turn(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{value}"
        );
    }
    assert!(
        parse_turn(&serde_json::to_vec(&json!({"text":"界".repeat(MAX_MESSAGE_CHARS)})).unwrap())
            .is_ok()
    );
    assert!(
        parse_turn(&serde_json::to_vec(&json!({"text":"界".repeat(MAX_MESSAGE_CHARS+1)})).unwrap())
            .is_err()
    );
    assert!(parse_turn(br#"{"text":"hello","model":"nyxagent/research"}"#).is_ok());
    for mode in ["ask", "full"] {
        let parsed = parse_turn(
            &serde_json::to_vec(&json!({"text": "hello", "access_mode": mode})).unwrap(),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(parsed.access_mode).unwrap(), mode);
    }
}

#[test]
fn upstream_body_is_exact_closed_contract_and_never_injects_history_as_input() {
    let body = upstream_body(DEFAULT_MODEL, "new question", Some("conv_id"), "recap");
    assert_eq!(
        body,
        json!({
            "input": "new question",
            "model": "nyxagent/chat",
            "instructions": "recap",
            "conversation": "conv_id",
            "stream": true,
            "store": true,
        })
    );
    assert!(
        upstream_body(DEFAULT_MODEL, "x", None, SYSTEM_PROMPT)
            .get("conversation")
            .is_none()
    );
}

#[test]
fn fragmented_sse_unicode_keepalive_unknown_events_and_terminal_replay() {
    let mut bytes = b": keepalive\n\n".to_vec();
    bytes.extend(frame(json!({"type":"future.event","new":"field"})));
    bytes.extend(frame(
        json!({"type":"response.output_text.delta","sequence_number":2,"delta":"Hello 界"}),
    ));
    bytes.extend(frame(terminal("response.completed", 9, "Hello 界!")));
    let mut stream = ResponseStream::default();
    for byte in bytes {
        stream.push(&[byte]).unwrap();
    }
    assert_eq!(stream.terminal.unwrap().text, "Hello 界!");
    let mut replay = ResponseStream::default();
    replay
        .push(&frame(terminal("response.completed", 0, "cached output")))
        .unwrap();
    let result = replay.terminal.unwrap();
    assert_eq!(result.text, "cached output");
    assert_eq!(
        result.session_id.as_deref(),
        Some("conv_1234567890abcdef1234567890abcdef")
    );
}

#[test]
fn mixed_sse_line_endings_in_one_chunk_preserve_frame_order() {
    let mut bytes =
        frame(json!({"type":"response.output_text.delta","sequence_number":0,"delta":"hello"}));
    bytes.extend(format!("data: {}\n\n", terminal("response.completed", 1, "hello")).as_bytes());
    let mut stream = ResponseStream::default();
    stream.push(&bytes).unwrap();
    assert_eq!(stream.terminal.unwrap().text, "hello");
}

#[test]
fn stream_rejects_invalid_sequences_json_identifiers_and_bounds() {
    let mut stream = ResponseStream::default();
    let delta =
        frame(json!({"type":"response.output_text.delta","sequence_number":4,"delta":"partial"}));
    stream.push(&delta).unwrap();
    assert_eq!(stream.push(&delta).unwrap_err().code, "invalid_stream");
    assert_eq!(stream.text, "partial");
    assert!(ResponseStream::default().push(b"data: broken\n\n").is_err());
    let mut wrong = terminal("response.completed", 1, "hi");
    wrong["response"]["conversation"]["id"] = json!("../../arbitrary");
    assert!(ResponseStream::default().push(&frame(wrong)).is_err());
    let mut oversized = ResponseStream::default();
    let too_large = frame(json!({
        "type": "response.output_text.delta",
        "sequence_number": 0,
        "delta": "x".repeat(MAX_OUTPUT_BYTES + 1),
    }));
    assert_eq!(
        oversized.push(&too_large).unwrap_err().code,
        "output_too_large"
    );
    assert!(oversized.text.is_empty());
}

#[test]
fn failed_sse_preserves_partial_but_never_forwards_raw_error() {
    let mut stream = ResponseStream::default();
    stream
        .push(&frame(terminal("response.failed", 1, "partial")))
        .unwrap();
    let result = stream.terminal.unwrap();
    assert_eq!(result.text, "partial");
    assert_eq!(result.error.as_ref().unwrap().code, "turn_timeout");
    assert!(
        !serde_json::to_string(&result.error)
            .unwrap()
            .contains("SECRET")
    );
    assert_eq!(
        TurnError::new("secret provider message").code,
        "assistant_unavailable"
    );
}

#[test]
fn recovery_is_bounded_and_never_retries_uncertain_effects() {
    let mut recovery = Recovery::default();
    assert_eq!(
        recovery.decide(404, "not_found", true),
        RecoveryAction::Rebind
    );
    assert_eq!(
        recovery.decide(404, "not_found", true),
        RecoveryAction::Fail
    );
    assert_eq!(
        Recovery::default().decide(404, "not_found", false),
        RecoveryAction::Fail
    );
    assert_eq!(
        recovery.decide(401, "", false),
        RecoveryAction::ReplaceCredential
    );
    assert_eq!(
        recovery.decide(403, "agent_key_required", false),
        RecoveryAction::Fail
    );
    for code in ["session_busy", "capacity_exceeded"] {
        let mut retry = Recovery::default();
        for _ in 0..4 {
            assert_eq!(retry.decide(409, code, true), RecoveryAction::Backoff);
        }
        assert_eq!(retry.decide(409, code, true), RecoveryAction::Fail);
    }
    for code in [
        "outcome_unknown",
        "stale_response",
        "idempotency_conflict",
        "lease_lost",
        "turn_timeout",
        "client_disconnected",
        "session_too_large",
    ] {
        assert_eq!(
            Recovery::default().decide(409, code, true),
            RecoveryAction::Fail
        );
    }
}

#[test]
fn recap_is_labeled_recent_and_bounded_without_splitting_unicode() {
    let messages: Vec<_> = (0..30)
        .map(|i| AssistantMessage {
            id: Uuid::new_v4().to_string(),
            conversation_id: "c".into(),
            user_id: "u".into(),
            seq: i,
            turn_id: "t".into(),
            role: "user".into(),
            text: format!("marker{i}:{}", "界".repeat(400)),
            status: "completed".into(),
            error_code: None,
            created_at: Utc::now(),
            activities: Vec::new(),
        })
        .collect();
    let prompt = instructions(&messages);
    assert!(prompt.starts_with(SYSTEM_PROMPT));
    assert!(prompt.contains("Prior conversation history"));
    assert!(prompt.contains("marker29"));
    assert!(!prompt.contains("marker0:"));
    assert!(prompt.len() <= SYSTEM_PROMPT.len() + 8192);
}

#[tokio::test]
async fn persistence_fences_concurrent_turns_scopes_owners_paginates_and_deletes() {
    let db = connect_transaction_test_database("nyxa_persist").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4();
    db.collection(USERS)
        .insert_one(test_user(&user.to_string(), UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let owner = user.to_string();
    let first = begin_turn(
        &db,
        &owner,
        &request(None, "First question"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let credential =
        credentials::load_for_conversation(&db, &state.encryption_keys, &owner, &first.id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        messages(&db, &owner, &first.id, 100, None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        begin_turn(
            &db,
            &owner,
            &request(Some(&first.id), "overlap"),
            &state.encryption_keys
        )
        .await,
        Err(AppError::AssistantTurnActive)
    ));
    for result in [
        get(&db, "other", &first.id).await,
        rename(&db, "other", &first.id, "No").await,
        delete(&db, "other", &first.id).await,
    ] {
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }
    assert!(matches!(
        request_stop(&db, "other", &first.id).await,
        Err(AppError::NotFound(_))
    ));
    let success = TurnResult {
        text: "Answer".into(),
        session_id: Some("returned-session".into()),
        response_id: Some("returned-response".into()),
        error: None,
    };
    finish_turn(
        &db,
        &first,
        &credential.api_key_id,
        &Uuid::new_v4().to_string(),
        &success,
    )
    .await
    .unwrap();
    let row = get(&db, &owner, &first.id).await.unwrap();
    assert_eq!(row.nyxagent_session_id.as_deref(), Some("returned-session"));
    assert_eq!(row.message_count, 2);
    assert!(row.active_turn.is_none());
    let req = request(Some(&row.id), "next");
    let (a, b) = tokio::join!(
        begin_turn(&db, &owner, &req, &state.encryption_keys),
        begin_turn(&db, &owner, &req, &state.encryption_keys)
    );
    let claimed = match (a, b) {
        (Ok(row), Err(AppError::AssistantTurnActive))
        | (Err(AppError::AssistantTurnActive), Ok(row)) => row,
        other => panic!("unexpected concurrent admission {other:?}"),
    };
    request_stop(&db, &owner, &row.id).await.unwrap();
    let cancelled = finish_turn(
        &db,
        &claimed,
        &credential.api_key_id,
        &Uuid::new_v4().to_string(),
        &success,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(cancelled.code, "cancelled");
    let row = get(&db, &owner, &row.id).await.unwrap();
    assert!(row.nyxagent_session_id.is_none());
    assert_eq!(row.context_reset_reason.as_deref(), Some("turn_failed"));
    let latest = messages(&db, &owner, &row.id, 2, None).await.unwrap();
    assert_eq!(latest.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![3, 4]);
    assert_eq!(latest[1].error_code.as_deref(), Some("cancelled"));
    assert_eq!(
        messages(&db, &owner, &row.id, 2, Some(3))
            .await
            .unwrap()
            .len(),
        2
    );
    request_stop(&db, &owner, &row.id).await.unwrap();
    assert_eq!(
        rename(&db, &owner, &row.id, "Renamed").await.unwrap().title,
        "Renamed"
    );
    assert_eq!(list(&db, &owner, 1, None).await.unwrap().len(), 1);
    assert!(
        list(&db, &owner, 1, Some(&index_cursor(&row)))
            .await
            .unwrap()
            .is_empty()
    );
    delete(&db, &owner, &row.id).await.unwrap();
    assert_eq!(
        db.collection::<bson::Document>(MESSAGES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn every_failed_turn_clears_binding_and_retains_partial_reply() {
    let db = connect_transaction_test_database("nyxa_failures").await;
    ensure_indexes(&db).await.unwrap();
    for code in [
        "client_disconnected",
        "turn_timeout",
        "lease_lost",
        "session_too_large",
        "idempotency_conflict",
        "agent_error",
        "outcome_unknown",
        "stale_response",
        "cancelled",
    ] {
        let row = begin_turn(
            &db,
            "owner",
            &request(None, code),
            &test_app_state(db.clone()).encryption_keys,
        )
        .await
        .unwrap();
        db.collection::<AssistantConversation>(CONVERSATIONS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"nyxagent_session_id":"old"}},
            )
            .await
            .unwrap();
        finish_turn(
            &db,
            &row,
            "key",
            &Uuid::new_v4().to_string(),
            &TurnResult {
                text: "partial".into(),
                session_id: None,
                response_id: None,
                error: Some(TurnError::new(code)),
            },
        )
        .await
        .unwrap();
        let saved = get(&db, "owner", &row.id).await.unwrap();
        assert!(saved.nyxagent_session_id.is_none(), "{code}");
        assert_eq!(saved.context_reset_reason.as_deref(), Some("turn_failed"));
        let messages = messages(&db, "owner", &row.id, 2, None).await.unwrap();
        assert_eq!(messages[1].text, "partial");
        assert_eq!(messages[1].status, "failed");
        assert_eq!(Some(messages[1].created_at), saved.context_reset_at);
    }
}

#[test]
fn live_turn_expires_at_the_exact_ttl_boundary() {
    let now = Utc::now();
    let mut row = stale_test_row(now);
    assert!(live_turn(&row, now).is_none());
    row.active_turn.as_mut().unwrap().started_at += chrono::Duration::milliseconds(1);
    assert!(live_turn(&row, now).is_some());
    assert_eq!(ACTIVE_TURN_TTL_SECS, 2100);
}

fn stale_test_row(now: DateTime<Utc>) -> AssistantConversation {
    AssistantConversation {
        id: format!("nyxa-{}", Uuid::new_v4().simple()),
        user_id: "owner".into(),
        title: "Interrupted turn".into(),
        model: DEFAULT_MODEL.into(),
        access_mode: Default::default(),
        nyxagent_session_id: Some("old-session".into()),
        nyxagent_last_response_id: None,
        credential_api_key_id: "key".into(),
        message_count: 0,
        active_turn: Some(ActiveTurn {
            activities: Vec::new(),
            turn_id: Uuid::new_v4().to_string(),
            started_at: now - chrono::Duration::seconds(ACTIVE_TURN_TTL_SECS),
            stop_requested: false,
        }),
        context_reset_at: None,
        context_reset_reason: None,
        created_at: now,
        updated_at: now,
    }
}

async fn expire_turn(db: &Database, row: &AssistantConversation) {
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {
                "active_turn.started_at": bson::DateTime::from_chrono(
                    Utc::now() - chrono::Duration::seconds(ACTIVE_TURN_TTL_SECS),
                ),
                "nyxagent_session_id": "old-session",
            }},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn stale_fence_is_reclaimed_with_lost_reply_and_late_settlement_is_a_noop() {
    let db = connect_transaction_test_database("nyxa_reclaim").await;
    ensure_indexes(&db).await.unwrap();
    let old = begin_turn(
        &db,
        "owner",
        &request(None, "lost question"),
        &test_app_state(db.clone()).encryption_keys,
    )
    .await
    .unwrap();
    expire_turn(&db, &old).await;
    let next = begin_turn(
        &db,
        "owner",
        &request(Some(&old.id), "continue"),
        &test_app_state(db.clone()).encryption_keys,
    )
    .await
    .unwrap();
    assert_ne!(
        next.active_turn.as_ref().unwrap().turn_id,
        old.active_turn.as_ref().unwrap().turn_id,
    );
    assert!(next.nyxagent_session_id.is_none());
    assert_eq!(next.context_reset_reason.as_deref(), Some("turn_failed"));
    let transcript = messages(&db, "owner", &old.id, 100, None).await.unwrap();
    assert_eq!(transcript.len(), 3);
    let lost = &transcript[1];
    assert_eq!(lost.turn_id, old.active_turn.as_ref().unwrap().turn_id);
    assert_eq!(lost.role, "assistant");
    assert_eq!(lost.status, "failed");
    assert_eq!(lost.error_code.as_deref(), Some("turn_lost"));
    assert!(lost.text.is_empty());
    assert_eq!(
        lost.created_at.timestamp_millis(),
        next.context_reset_at.unwrap().timestamp_millis()
    );
    assert!(transcript[2].created_at > lost.created_at);
    let late = finish_turn(
        &db,
        &old,
        "key",
        &Uuid::new_v4().to_string(),
        &TurnResult {
            text: "late response".into(),
            session_id: Some("stale-session".into()),
            response_id: None,
            error: None,
        },
    )
    .await;
    assert!(matches!(late, Err(AppError::NotFound(_))));
    let saved = get(&db, "owner", &old.id).await.unwrap();
    assert_eq!(saved.message_count, 3);
    assert_eq!(
        saved.active_turn.unwrap().turn_id,
        next.active_turn.unwrap().turn_id
    );
    assert!(saved.nyxagent_session_id.is_none());
}

#[tokio::test]
async fn stale_fences_allow_rename_delete_and_stop_is_a_noop() {
    let db = connect_transaction_test_database("nyxa_stale_mutations").await;
    let row = begin_turn(
        &db,
        "owner",
        &request(None, "question"),
        &test_app_state(db.clone()).encryption_keys,
    )
    .await
    .unwrap();
    expire_turn(&db, &row).await;
    request_stop(&db, "owner", &row.id).await.unwrap();
    let stale = get(&db, "owner", &row.id).await.unwrap();
    assert!(!stale.active_turn.as_ref().unwrap().stop_requested);
    assert!(
        stop_requested(&db, "owner", &row.id, &row.active_turn.unwrap().turn_id)
            .await
            .unwrap()
    );
    assert_eq!(
        rename(&db, "owner", &row.id, "Renamed")
            .await
            .unwrap()
            .title,
        "Renamed"
    );
    delete(&db, "owner", &row.id).await.unwrap();
    assert!(matches!(
        get(&db, "owner", &row.id).await,
        Err(AppError::NotFound(_))
    ));
    assert!(messages(&db, "owner", &row.id, 100, None).await.is_err());
}

#[test]
fn row_contract_ignores_a_stored_credential_when_auth_is_none() {
    let mut row = crate::models::downstream_service::test_helpers::dummy_service();
    row.slug = SERVICE_SLUG.into();
    row.is_active = true;
    row.auth_method = "none".into();
    row.requires_user_credential = false;
    row.forward_access_token = true;
    row.inject_delegation_token = false;
    row.credential_encrypted = vec![1, 2, 3];
    for configured in [Some(true), Some(false), None] {
        let contract = row_contract(Some(&row), configured);
        assert_eq!(contract.master_credential_configured, configured);
        assert!(
            contract.valid(),
            "a never-injected credential must not take the assistant down"
        );
    }
    row.auth_method = "bearer".into();
    assert!(!row_contract(Some(&row), Some(false)).valid());
    row.auth_method = "none".into();
    row.forward_access_token = false;
    assert!(!row_contract(Some(&row), Some(false)).valid());
    assert!(!row_contract(None, None).valid());
}

#[tokio::test]
async fn catalog_contract_reports_decrypted_credential_presence() {
    let db = connect_transaction_test_database("nyxa_row_contract").await;
    let state = test_app_state(db.clone());
    let mut row = crate::models::downstream_service::test_helpers::dummy_service();
    row.id = Uuid::new_v4().to_string();
    row.slug = SERVICE_SLUG.into();
    row.is_active = true;
    row.auth_method = "none".into();
    row.requires_user_credential = false;
    row.forward_access_token = true;
    row.inject_delegation_token = false;
    // Legacy create paths encrypted an absent credential; that is "not configured".
    row.credential_encrypted = state.encryption_keys.encrypt(b"").await.unwrap();
    let services = db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    );
    services.insert_one(&row).await.unwrap();
    let contract = catalog_contract(&db, &state.encryption_keys).await.unwrap();
    assert_eq!(contract.master_credential_configured, Some(false));
    assert!(contract.valid());
    services
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"credential_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: state.encryption_keys.encrypt(b"real-secret").await.unwrap(),
            }}},
        )
        .await
        .unwrap();
    let contract = catalog_contract(&db, &state.encryption_keys).await.unwrap();
    assert_eq!(contract.master_credential_configured, Some(true));
    assert!(contract.valid(), "auth none never injects it");
}

#[tokio::test]
async fn turn_activities_are_metadata_only_bounded_and_retained_on_the_reply() {
    let db = connect_transaction_test_database("nyxa_activity").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4();
    db.collection(USERS)
        .insert_one(test_user(&user.to_string(), UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let owner = user.to_string();
    // No live turn: nothing is recorded.
    assert_eq!(
        activity_started(&db, &owner, "nyxa-00000000000000000000000000000000", "x")
            .await
            .unwrap(),
        None
    );
    let row = begin_turn(
        &db,
        &owner,
        &request(None, "List issues"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let credential =
        credentials::load_for_conversation(&db, &state.encryption_keys, &owner, &row.id)
            .await
            .unwrap()
            .unwrap();
    let long_label = "l".repeat(500);
    let first = activity_started(&db, &owner, &row.id, &long_label)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        activity_started(&db, "other", &row.id, "x").await.unwrap(),
        None
    );
    activity_finished(&db, &owner, &row.id, &first, true)
        .await
        .unwrap();
    for i in 0..MAX_TURN_ACTIVITIES {
        activity_started(&db, &owner, &row.id, &format!("github__tool_{i}"))
            .await
            .unwrap()
            .unwrap();
    }
    let live = get(&db, &owner, &row.id).await.unwrap();
    let activities = &live.active_turn.as_ref().unwrap().activities;
    assert_eq!(
        activities.len() as i64,
        MAX_TURN_ACTIVITIES,
        "oldest entries are evicted"
    );
    assert!(
        activities.iter().all(|a| a.id != first),
        "the settled first entry was evicted"
    );
    assert_eq!(activities[0].label, "github__tool_0");
    assert!(
        activities
            .iter()
            .all(|a| a.status == "running" && a.ended_at.is_none())
    );
    let last = activities.last().unwrap().id.clone();
    activity_finished(&db, &owner, &row.id, &last, false)
        .await
        .unwrap();
    // Unknown IDs and wrong owners are no-ops.
    activity_finished(&db, "other", &row.id, &last, true)
        .await
        .unwrap();
    activity_finished(&db, &owner, &row.id, "missing", true)
        .await
        .unwrap();
    let live = get(&db, &owner, &row.id).await.unwrap();
    let settled = live
        .active_turn
        .as_ref()
        .unwrap()
        .activities
        .last()
        .unwrap();
    assert_eq!(settled.status, "error");
    assert!(settled.ended_at.is_some());
    let message_id = Uuid::new_v4().to_string();
    finish_turn(
        &db,
        &row,
        &credential.api_key_id,
        &message_id,
        &TurnResult {
            text: "Done".into(),
            session_id: Some("s".into()),
            response_id: Some("r".into()),
            error: None,
        },
    )
    .await
    .unwrap();
    let reply = messages(&db, &owner, &row.id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.id == message_id)
        .unwrap();
    assert_eq!(reply.activities.len() as i64, MAX_TURN_ACTIVITIES);
    assert_eq!(
        reply.activities[0].label.chars().count(),
        "github__tool_0".len()
    );
    assert!(
        reply.activities[..MAX_TURN_ACTIVITIES as usize - 1]
            .iter()
            .all(|a| a.status == "completed" && a.ended_at.is_some()),
        "in-flight calls share the settled turn's outcome"
    );
    assert_eq!(reply.activities.last().unwrap().status, "error");
    let user_row = messages(&db, &owner, &row.id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.role == "user")
        .unwrap();
    assert!(user_row.activities.is_empty());
    // Label bound applies at insert.
    let row = begin_turn(
        &db,
        &owner,
        &request(Some(&row.id), "again"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    activity_started(&db, &owner, &row.id, &long_label)
        .await
        .unwrap()
        .unwrap();
    let live = get(&db, &owner, &row.id).await.unwrap();
    assert_eq!(
        live.active_turn.unwrap().activities[0]
            .label
            .chars()
            .count(),
        MAX_ACTIVITY_LABEL_CHARS
    );
}
