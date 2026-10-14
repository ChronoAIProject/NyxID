use super::*;
use crate::models::user::{COLLECTION_NAME as USERS, UserType};
use crate::services::assistant_agent_credential_service as credentials;
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};

fn request(id: Option<&str>, text: &str) -> TurnRequest {
    TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: Vec::new(),
        agent_id: None,
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
fn insufficient_credits_is_a_stable_terminal_code() {
    let error = TurnError::new("insufficient_credits");
    assert_eq!(error.code, "insufficient_credits");
    assert_eq!(
        error.message,
        "There aren't enough credits to run this turn."
    );
    for (status, bound) in [
        (402, true),
        (402, false),
        (401, true),
        (403, false),
        (409, true),
    ] {
        assert_eq!(
            Recovery::default().decide(status, "insufficient_credits", bound),
            RecoveryAction::Fail
        );
    }
    let mut stream = ResponseStream::default();
    let mut failed = terminal("response.failed", 1, "partial");
    failed["response"]["error"] = json!({"code":"insufficient_credits","message":"SECRET"});
    stream.push(&frame(failed)).unwrap();
    let result = stream.terminal.unwrap();
    let error = result.error.unwrap();
    assert_eq!(error.code, "insufficient_credits");
    assert_eq!(
        error.message,
        "There aren't enough credits to run this turn."
    );
    assert_eq!(result.text, "partial");
    for code in [
        "nyxid_error",
        "payment_required",
        "wallet_suspended",
        "unknown_code",
    ] {
        assert_ne!(TurnError::new(code).code, "insufficient_credits");
    }
    assert_eq!(TurnError::new("nyxid_error").code, "nyxid_error");
    assert_eq!(TurnError::new("unknown_code").code, "assistant_unavailable");
}

#[test]
fn recap_is_labeled_recent_and_bounded_without_splitting_unicode() {
    let messages: Vec<_> = (0..30)
        .map(|i| AssistantMessage {
            continuation_of: None,
            retry_of: None,
            specialist_id: None,
            steering: None,
            voice: None,
            execution_pending: false,
            id: Uuid::new_v4().to_string(),
            conversation_id: "c".into(),
            user_id: "u".into(),
            seq: i,
            turn_id: "t".into(),
            role: "user".into(),
            text: format!("marker{i}:{}", "界".repeat(400)),
            status: "completed".into(),
            error_code: None,
            upstream_error_code: None,
            error_detail: None,
            created_at: Utc::now(),
            activities: Vec::new(),
            attachments: Vec::new(),
            origin: None,
            via: None,
        })
        .collect();
    let row = stale_test_row(Utc::now());
    let context = crate::services::assistant_instruction_context::Prepared::new(
        &[2; 32], &row, None, &messages,
    )
    .unwrap();
    let base = context.instructions(&[], false);
    let prompt = context.instructions(&messages, true);
    assert!(prompt.starts_with(SYSTEM_PROMPT));
    assert!(prompt.contains("Previous context (after reset)"));
    assert!(prompt.contains("marker29"));
    assert!(!prompt.contains("marker0:"));
    assert!(prompt.contains(SCHEDULE_PROMPT));
    assert!(prompt.len() <= base.len() + 16384);
}

#[test]
fn failure_diagnostics_normalize_unknown_upstream_codes() {
    let error = TurnError::new("provider_rate_limit");
    assert_eq!(error.code, "assistant_unavailable");
    assert_eq!(error.upstream_code.as_deref(), Some("provider_rate_limit"));
    let known = TurnError::new("first_byte_timeout");
    assert_eq!(known.code, "first_byte_timeout");
    assert_eq!(known.upstream_code.as_deref(), Some("first_byte_timeout"));
}

#[test]
fn failure_matrix_maps_transport_failures_to_recoverable_classes() {
    assert_eq!(
        TurnError::new("first_byte_timeout").code,
        "first_byte_timeout"
    );
    assert_eq!(TurnError::new("idle_timeout").code, "idle_timeout");
    assert_eq!(TurnError::new("invalid_stream").code, "invalid_stream");
    assert_eq!(TurnError::new("upstream_503").code, "assistant_unavailable");
    assert_eq!(
        TurnError::new("settlement_deadline").code,
        "assistant_unavailable"
    );
}

#[test]
fn recap_preserves_goal_head_and_tail_under_long_last_reply() {
    let rows = vec![
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            specialist_id: None,
            steering: None,
            voice: None,
            execution_pending: false,
            id: "goal".into(),
            conversation_id: "c".into(),
            user_id: "u".into(),
            seq: 1,
            turn_id: "t1".into(),
            role: "user".into(),
            text: "ORIGINATING GOAL".into(),
            status: "completed".into(),
            error_code: None,
            upstream_error_code: None,
            error_detail: None,
            created_at: Utc::now(),
            activities: Vec::new(),
            attachments: Vec::new(),
            origin: None,
            via: None,
        },
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            specialist_id: None,
            steering: None,
            voice: None,
            execution_pending: false,
            id: "last".into(),
            conversation_id: "c".into(),
            user_id: "u".into(),
            seq: 2,
            turn_id: "t2".into(),
            role: "assistant".into(),
            text: format!("HEAD {} TAIL", "x".repeat(6000)),
            status: "completed".into(),
            error_code: None,
            upstream_error_code: None,
            error_detail: None,
            created_at: Utc::now(),
            activities: Vec::new(),
            attachments: Vec::new(),
            origin: None,
            via: None,
        },
    ];
    let recap = bounded_recap(&rows);
    assert!(recap.contains("ORIGINATING GOAL"));
    assert!(recap.contains("HEAD"));
    assert!(recap.contains("TAIL"));
    assert!(recap.len() <= 16384);
    assert!(recap.contains("ORIGINATING GOAL"));
}

#[test]
fn recap_caps_each_activity_list_and_reserves_bounded_total_budget() {
    let row = AssistantMessage {
        continuation_of: None,
        retry_of: None,
        specialist_id: None,
        steering: None,
        voice: None,
        execution_pending: false,
        id: "cap".into(),
        conversation_id: "c".into(),
        user_id: "u".into(),
        seq: 1,
        turn_id: "t".into(),
        role: "assistant".into(),
        text: "goal".into(),
        status: "completed".into(),
        error_code: None,
        upstream_error_code: None,
        error_detail: None,
        created_at: Utc::now(),
        activities: (0..40)
            .map(|i| crate::models::assistant_conversation::TurnActivity {
                machine: None,
                id: i.to_string(),
                digest: None,
                label: format!("tool-{i}"),
                status: "completed".into(),
                started_at: Utc::now(),
                ended_at: None,
            })
            .collect(),
        attachments: Vec::new(),
        origin: None,
        via: None,
    };
    // Thirty such rows with long labels: each list is capped at ten entries
    // and the whole recap stays inside its budget, newest rows first.
    let rows: Vec<_> = (0..30)
        .map(|seq| {
            let mut row = row.clone();
            row.id = format!("cap-{seq}");
            row.seq = seq;
            row.text = format!("row{seq}");
            for activity in &mut row.activities {
                activity.label = format!("{}-{}", activity.label, "x".repeat(100));
            }
            row
        })
        .collect();
    let recap = bounded_recap(&rows);
    assert!(recap.len() <= 16384, "{}", recap.len());
    assert!(recap.contains("row29"));
    assert!(
        !recap.contains("row9 ["),
        "rows beyond the 20-row window are omitted"
    );
    assert!(recap.contains(", +30 more]"));
    assert!(
        !recap.contains("tool-10-"),
        "an action list stops after ten entries"
    );
}

#[test]
fn recap_v2_keeps_action_status_and_attachment_checkpoint() {
    let row = AssistantMessage {
        continuation_of: None,
        retry_of: None,
        specialist_id: None,
        steering: None,
        voice: None,
        execution_pending: false,
        id: "m".into(),
        conversation_id: "c".into(),
        user_id: "u".into(),
        seq: 1,
        turn_id: "t".into(),
        role: "assistant".into(),
        text: "head goal details tail".into(),
        status: "failed".into(),
        error_code: Some("turn_lost".into()),
        upstream_error_code: Some("upstream_x".into()),
        error_detail: None,
        created_at: Utc::now(),
        activities: vec![crate::models::assistant_conversation::TurnActivity {
            machine: None,
            id: "a".into(),
            digest: None,
            label: "tool.check".into(),
            status: "completed".into(),
            started_at: Utc::now(),
            ended_at: None,
        }],
        attachments: vec![crate::models::assistant_conversation::TurnAttachment {
            image_input: None,
            origin: "tool".into(),
            pages: None,
            id: "att".into(),
            content_type: "text/plain".into(),
            size: 4,
            label: "result.txt".into(),
        }],
        origin: None,
        via: None,
    };
    let recap = bounded_recap(std::slice::from_ref(&row));
    assert!(recap.contains("Previous context (after reset)"));
    assert!(recap.contains("tool.check=completed"));
    assert!(recap.contains("result.txt"));
    assert!(recap.contains("partial, failed"));
}

#[test]
fn guest_recap_omits_owner_tool_metadata() {
    let row = AssistantMessage {
        continuation_of: None,
        retry_of: None,
        specialist_id: None,
        steering: None,
        voice: None,
        execution_pending: false,
        id: "guest-recap".into(),
        conversation_id: "c".into(),
        user_id: "u".into(),
        seq: 1,
        turn_id: "t".into(),
        role: "assistant".into(),
        text: "Visible channel reply".into(),
        status: "completed".into(),
        error_code: Some("turn_lost".into()),
        upstream_error_code: None,
        error_detail: None,
        created_at: Utc::now(),
        activities: vec![crate::models::assistant_conversation::TurnActivity {
            machine: None,
            id: "owner-action".into(),
            digest: None,
            label: "owner.secret_tool".into(),
            status: "completed".into(),
            started_at: Utc::now(),
            ended_at: None,
        }],
        attachments: vec![crate::models::assistant_conversation::TurnAttachment {
            image_input: None,
            origin: "tool".into(),
            pages: None,
            id: "owner-attachment".into(),
            content_type: "text/plain".into(),
            size: 1,
            label: "private.txt".into(),
        }],
        origin: None,
        via: None,
    };
    let recap = bounded_recap_with_checkpoint(std::slice::from_ref(&row), true, None);
    assert!(recap.contains("Visible channel reply"));
    assert!(!recap.contains("owner.secret_tool"));
    assert!(!recap.contains("private.txt"));
    assert!(!recap.contains("turn_lost"));
}

#[test]
fn retry_recap_lists_completed_activity_ids_for_no_duplicate_replay() {
    let row = AssistantMessage {
        continuation_of: None,
        retry_of: None,
        specialist_id: None,
        steering: None,
        voice: None,
        execution_pending: false,
        id: "retry-recap".into(),
        conversation_id: "c".into(),
        user_id: "u".into(),
        seq: 1,
        turn_id: "t".into(),
        role: "assistant".into(),
        text: "partial".into(),
        status: "failed".into(),
        error_code: Some("turn_lost".into()),
        upstream_error_code: None,
        error_detail: None,
        created_at: Utc::now(),
        activities: vec![crate::models::assistant_conversation::TurnActivity {
            machine: None,
            id: "activity-42".into(),
            digest: None,
            label: "mail.send".into(),
            status: "completed".into(),
            started_at: Utc::now(),
            ended_at: None,
        }],
        attachments: Vec::new(),
        origin: None,
        via: None,
    };
    let recap = bounded_recap(std::slice::from_ref(&row));
    assert!(
        recap.contains("assistant (partial, failed): partial"),
        "{recap}"
    );
    assert!(recap.contains("[actions: mail.send=completed]"), "{recap}");
    assert!(recap.contains("[error: turn_lost]"), "{recap}");
    // Activity ids are internal; the model sees labels and statuses only.
    assert!(!recap.contains("activity-42"));
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
        delete(&db, "other", &first.id)
            .await
            .map(|mut rows| rows.remove(0)),
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
    assert_eq!(row.context_reset_reason.as_deref(), Some("stopped"));
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
    assert_eq!(list(&db, &owner, 1, None, None).await.unwrap().len(), 1);
    assert!(
        list(&db, &owner, 1, Some(&index_cursor(&row)), None)
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
        // Stop records its own reason so the reset notice can say so.
        let reason = if code == "cancelled" {
            "stopped"
        } else {
            "turn_failed"
        };
        assert_eq!(
            saved.context_reset_reason.as_deref(),
            Some(reason),
            "{code}"
        );
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
        machine_previews: false,
        platform_cleanup_pending: false,
        platform_cleanup_id: None,
        title_source: Default::default(),
        automation_thread: false,
        voice_parent_conversation_id: None,
        agent_owner_id: None,
        id: format!("nyxa-{}", Uuid::new_v4().simple()),
        user_id: "owner".into(),
        title: "Interrupted turn".into(),
        model: DEFAULT_MODEL.into(),
        access_mode: Default::default(),
        nyxagent_session_id: Some("old-session".into()),
        nyxagent_instruction_binding: None,
        nyxagent_last_response_id: None,
        credential_api_key_id: "key".into(),
        message_count: 0,
        active_turn: Some(ActiveTurn {
            retry_digests: Vec::new(),
            retry_of: None,
            running_response: None,
            channel_event_id: None,
            initiating_message_seq: None,
            voice_request_id: None,
            machine_node_ids: Vec::new(),
            continuations: 0,
            tool_progress: Default::default(),
            lease_expires_at: None,
            heartbeat_at: None,
            trigger_run_id: None,
            activities: Vec::new(),
            attachments: Vec::new(),
            turn_id: Uuid::new_v4().to_string(),
            origin: Default::default(),
            started_at: now - chrono::Duration::seconds(ACTIVE_TURN_TTL_SECS),
            stop_requested: false,
            events: Vec::new(),
            note: None,
            question_key: None,
            question: None,
            asked_from: None,
            also_deliver: Vec::new(),
        }),
        context_reset_at: None,
        context_reset_reason: None,
        created_at: now,
        updated_at: now,
        role: Default::default(),
        agent_id: None,
        learning_epoch: None,
        report_to: None,
        pending_events: Vec::new(),
        event_streak: 0,
        channel: None,
        group_id: None,
        group_request_id: None,
        group_seen_seq: 0,
        guest_turn: false,
        reply_channel: None,
        deliver_also: Vec::new(),
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
                "active_turn.heartbeat_at": bson::Bson::Null,
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
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &old.id},
            doc! {"$set": {"active_turn.activities": [{"id": "act-1", "label": "nyxid__check", "status": "running", "started_at": bson::DateTime::now(), "ended_at": bson::Bson::Null}], "active_turn.attachments": [{"id": "att-1", "content_type": "text/plain", "size": 4i64, "label": "result.txt", "origin": "tool"}]}},
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
    assert_eq!(lost.activities[0].status, "error");
    assert_eq!(lost.attachments[0].id, "att-1");
    let detail = lost.error_detail.as_ref().expect("reclaim diagnostics");
    assert_eq!(detail.stage, "lease_expiry");
    assert_eq!(lost.upstream_error_code, None);
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

#[tokio::test]
async fn legacy_stop_keeps_the_existing_lease_fence() {
    let db = connect_transaction_test_database("nyxa_legacy_stop").await;
    let state = test_app_state(db.clone());
    let row = begin_turn(
        &db,
        "owner",
        &request(None, "legacy question"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$unset": {"active_turn.heartbeat_at": ""}},
        )
        .await
        .unwrap();
    request_stop(&db, "owner", &row.id).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let saved = get(&db, "owner", &row.id).await.unwrap();
    assert!(saved.active_turn.is_some());
    assert!(saved.active_turn.unwrap().stop_requested);
}

#[tokio::test]
async fn dead_worker_stop_is_forced_within_grace_and_late_settlement_is_fenced() {
    let db = connect_transaction_test_database("nyxa_forced_stop").await;
    let state = test_app_state(db.clone());
    let row = begin_turn(
        &db,
        "owner",
        &request(None, "question"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let credential =
        credentials::load_for_conversation(&db, &state.encryption_keys, "owner", &row.id)
            .await
            .unwrap()
            .unwrap();
    request_stop(&db, "owner", &row.id).await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(STOP_SETTLEMENT_GRACE_SECS + 2),
        async {
            loop {
                if get(&db, "owner", &row.id)
                    .await
                    .unwrap()
                    .active_turn
                    .is_none()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        },
    )
    .await
    .expect("forced stop did not settle within grace");
    let settled = get(&db, "owner", &row.id).await.unwrap();
    assert_eq!(settled.nyxagent_session_id, None);
    assert_eq!(
        messages(&db, "owner", &row.id, 10, None)
            .await
            .unwrap()
            .last()
            .and_then(|message| message.error_code.as_deref()),
        Some("cancelled")
    );
    let late = finish_turn(
        &db,
        &row,
        &credential.api_key_id,
        &Uuid::new_v4().to_string(),
        &TurnResult {
            text: "late response".into(),
            session_id: Some("late-session".into()),
            response_id: None,
            error: None,
        },
    )
    .await;
    assert!(matches!(late, Err(AppError::NotFound(_))));
}

#[tokio::test]
async fn orphan_sweep_reclaims_stale_heartbeat_but_keeps_fresh_and_legacy_turns() {
    let db = connect_transaction_test_database("nyxa_heartbeat_sweep").await;
    let state = test_app_state(db.clone());
    let stale = begin_turn(
        &db,
        "owner",
        &request(None, "stale"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let fresh = begin_turn(
        &db,
        "owner",
        &request(None, "fresh"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let legacy = begin_turn(
        &db,
        "owner",
        &request(None, "legacy"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let old = bson::DateTime::from_chrono(
        Utc::now() - chrono::Duration::seconds(ACTIVE_TURN_HEARTBEAT_STALE_SECS + 1),
    );
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &stale.id},
            doc! {"$set": {"active_turn.heartbeat_at": old}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &legacy.id},
            doc! {"$unset": {"active_turn.heartbeat_at": ""}},
        )
        .await
        .unwrap();
    assert_eq!(sweep_orphaned_turns(&db).await.unwrap(), 1);
    assert!(
        get(&db, "owner", &stale.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    let reclaimed = messages(&db, "owner", &stale.id, 1, None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(reclaimed.error_code.as_deref(), Some("turn_lost"));
    assert_eq!(
        reclaimed.upstream_error_code, None,
        "a NyxID class, not an upstream code"
    );
    assert_eq!(reclaimed.error_detail.as_ref().unwrap().stage, "sweep");
    let saved = checkpoint(&db, "owner", &stale.id).await.unwrap().unwrap();
    assert_eq!(
        (saved.status.as_str(), saved.error_code.as_deref()),
        ("failed", Some("turn_lost"))
    );
    assert!(
        get(&db, "owner", &fresh.id)
            .await
            .unwrap()
            .active_turn
            .is_some()
    );
    assert!(
        get(&db, "owner", &legacy.id)
            .await
            .unwrap()
            .active_turn
            .is_some()
    );
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

async fn settle(
    db: &Database,
    keys: &std::sync::Arc<crate::crypto::aes::EncryptionKeys>,
    owner: &str,
    row: &AssistantConversation,
    text: &str,
    error: Option<&str>,
) {
    let credential = credentials::load_for_conversation(db, keys, owner, &row.id)
        .await
        .unwrap()
        .unwrap();
    finish_turn(
        db,
        row,
        &credential.api_key_id,
        &Uuid::new_v4().to_string(),
        &TurnResult {
            text: text.into(),
            session_id: None,
            response_id: None,
            error: error.map(TurnError::new),
        },
    )
    .await
    .unwrap();
}

fn retry(id: &str, text: &str, retry_of: &str) -> TurnRequest {
    TurnRequest {
        acknowledgement_id: None,
        retry_of: Some(retry_of.into()),
        ..request(Some(id), text)
    }
}

#[tokio::test]
async fn b4_retry_fences_completed_calls_carries_attachments_and_chains() {
    let db = connect_transaction_test_database("nyxa_retry_fence").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let keys = &state.encryption_keys;
    let first = begin_turn(&db, &user, &request(None, "hello"), keys)
        .await
        .unwrap();
    settle(&db, keys, &user, &first, "hi", None).await;
    let upload = crate::services::assistant_upload_service::upload(
        &db,
        keys,
        &user,
        &first.id,
        "brief.txt",
        b"the brief".to_vec(),
    )
    .await
    .unwrap();
    let failed = begin_turn(
        &db,
        &user,
        &TurnRequest {
            attachment_ids: vec![upload.id.clone()],
            ..request(Some(&first.id), "send the brief")
        },
        keys,
    )
    .await
    .unwrap();
    let failed_turn = failed.active_turn.as_ref().unwrap().turn_id.clone();
    let sent =
        activity_started_with_digest(&db, &user, &first.id, "mail__send", Some("digest-send"))
            .await
            .unwrap()
            .unwrap();
    activity_finished(&db, &user, &first.id, &sent, true)
        .await
        .unwrap();
    let broken = activity_started_with_digest(
        &db,
        &user,
        &first.id,
        "drive__upload",
        Some("digest-upload"),
    )
    .await
    .unwrap()
    .unwrap();
    activity_finished(&db, &user, &first.id, &broken, false)
        .await
        .unwrap();
    settle(&db, keys, &user, &failed, "partial", Some("idle_timeout")).await;

    // Only a failed owner turn of this conversation can be retried.
    for target in [
        first.active_turn.as_ref().unwrap().turn_id.as_str(),
        "missing-turn",
    ] {
        let error = begin_turn(
            &db,
            &user,
            &retry(&first.id, "send the brief", target),
            keys,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&error, AppError::BadRequest(message) if message == RETRY_INVALID),
            "{error:?}"
        );
    }
    // The retry re-sends no upload id: the bound upload is carried over.
    let retried = begin_turn(
        &db,
        &user,
        &retry(&first.id, "send the brief", &failed_turn),
        keys,
    )
    .await
    .unwrap();
    let turn = retried.active_turn.as_ref().unwrap();
    assert_eq!(turn.retry_of.as_deref(), Some(failed_turn.as_str()));
    assert_eq!(turn.retry_digests, ["digest-send"]);
    let note = turn.note.as_deref().unwrap();
    assert_eq!(
        note,
        format!(
            "[NyxID retry context: the owner is retrying turn {failed_turn}, which failed (idle_timeout). These actions already completed there; use their results. NyxID refuses repeating any of them with the same arguments: mail__send. These did not complete; check their effect before running them again: drive__upload. The original message's attachments are attached again.]"
        )
    );
    let input = messages(&db, &user, &first.id, 1, None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(input.turn_id, turn.turn_id);
    assert_eq!(
        input
            .attachments
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        [upload.id.as_str()]
    );
    assert!(
        retry_duplicate(&db, &user, &first.id, "digest-send")
            .await
            .unwrap()
    );
    assert!(
        !retry_duplicate(&db, &user, &first.id, "digest-upload")
            .await
            .unwrap()
    );
    assert!(
        !retry_duplicate(&db, &user, &first.id, "digest-other")
            .await
            .unwrap()
    );
    // The refused repeat is recorded as completed, so a retry of this retry
    // inherits the fence even though the call did not run again here.
    record_retry_duplicate(&db, &user, &first.id, "mail__send", "digest-send")
        .await
        .unwrap();
    let retried_turn = turn.turn_id.clone();
    settle(
        &db,
        keys,
        &user,
        &retried,
        "",
        Some("assistant_unavailable"),
    )
    .await;
    let chained = begin_turn(
        &db,
        &user,
        &retry(&first.id, "send the brief", &retried_turn),
        keys,
    )
    .await
    .unwrap();
    assert_eq!(
        chained.active_turn.as_ref().unwrap().retry_digests,
        ["digest-send"]
    );
    assert!(
        retry_duplicate(&db, &user, &first.id, "digest-send")
            .await
            .unwrap()
    );
    // An ordinary turn is never fenced.
    settle(&db, keys, &user, &chained, "done", None).await;
    let ordinary = begin_turn(&db, &user, &request(Some(&first.id), "again"), keys)
        .await
        .unwrap();
    assert!(
        ordinary
            .active_turn
            .as_ref()
            .unwrap()
            .retry_digests
            .is_empty()
    );
    assert!(
        !retry_duplicate(&db, &user, &first.id, "digest-send")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn durable_checkpoint_accumulates_failures_resets_on_success_and_is_purged() {
    let db = connect_transaction_test_database("nyxa_checkpoint").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let keys = &state.encryption_keys;
    let first = begin_turn(&db, &user, &request(None, "Plan the launch"), keys)
        .await
        .unwrap();
    let id = first.id.clone();
    assert!(checkpoint(&db, &user, &id).await.unwrap().is_none());
    settle(&db, keys, &user, &first, "Here is the plan", None).await;
    let saved = checkpoint(&db, &user, &id).await.unwrap().unwrap();
    let first_turn = first.active_turn.as_ref().unwrap().turn_id.clone();
    assert_eq!(saved.status, "completed");
    assert_eq!(saved.goal.as_deref(), Some("Plan the launch"));
    assert_eq!(saved.output.as_deref(), Some("Here is the plan"));
    assert_eq!(
        saved.last_successful_turn_id.as_deref(),
        Some(first_turn.as_str())
    );

    let failing = begin_turn(&db, &user, &request(Some(&id), "Send the invites"), keys)
        .await
        .unwrap();
    let call = activity_started_with_digest(&db, &user, &id, "mail__send", None)
        .await
        .unwrap()
        .unwrap();
    activity_finished(&db, &user, &id, &call, true)
        .await
        .unwrap();
    activity_started_with_digest(&db, &user, &id, "calendar__create", None)
        .await
        .unwrap()
        .unwrap();
    settle(&db, keys, &user, &failing, "", Some("idle_timeout")).await;
    let failed_turn = failing.active_turn.as_ref().unwrap().turn_id.clone();
    let saved = checkpoint(&db, &user, &id).await.unwrap().unwrap();
    assert_eq!(saved.status, "failed");
    assert_eq!(saved.turn_id, failed_turn);
    assert_eq!(saved.error_code.as_deref(), Some("idle_timeout"));
    assert_eq!(saved.goal.as_deref(), Some("Send the invites"));
    assert_eq!(
        saved.completed_actions,
        [CheckpointAction {
            label: "mail__send".into(),
            turn_id: failed_turn.clone()
        }]
    );
    assert_eq!(
        saved.failed_actions,
        [CheckpointAction {
            label: "calendar__create".into(),
            turn_id: failed_turn.clone()
        }]
    );
    assert_eq!(
        saved.last_successful_turn_id.as_deref(),
        Some(first_turn.as_str())
    );
    assert!(saved.output.is_none());

    // A second failure keeps the open window's actions.
    let again = begin_turn(
        &db,
        &user,
        &request(Some(&id), "Try the invites again"),
        keys,
    )
    .await
    .unwrap();
    let call = activity_started_with_digest(&db, &user, &id, "slack__post", None)
        .await
        .unwrap()
        .unwrap();
    activity_finished(&db, &user, &id, &call, true)
        .await
        .unwrap();
    settle(&db, keys, &user, &again, "", Some("assistant_unavailable")).await;
    let saved = checkpoint(&db, &user, &id).await.unwrap().unwrap();
    assert_eq!(
        saved
            .completed_actions
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>(),
        ["mail__send", "slack__post"]
    );
    assert_eq!(saved.failed_actions.len(), 1);

    // The checkpoint leads the recap and names its scope, even though the
    // goal message itself is outside the recap window.
    let summary = checkpoint_summary(&db, &user, &id).await.unwrap();
    let recap = bounded_recap_with_checkpoint(&[], false, summary.checkpoint.as_ref());
    assert!(recap.contains(&format!("Checkpoint (turn {}, failed: assistant_unavailable; last successful turn {first_turn})", again.active_turn.as_ref().unwrap().turn_id)), "{recap}");
    assert!(recap.contains("- goal: Try the invites again"), "{recap}");
    assert!(
        recap.contains("- completed: mail__send, slack__post"),
        "{recap}"
    );
    assert!(
        recap.contains("- failed or interrupted (check before rerunning): calendar__create"),
        "{recap}"
    );
    // A guest audience never sees the owner's checkpoint.
    let guest = bounded_recap_with_checkpoint(&[], true, summary.checkpoint.as_ref());
    assert!(!guest.contains("mail__send"));

    // Success closes the window.
    let done = begin_turn(&db, &user, &request(Some(&id), "Thanks"), keys)
        .await
        .unwrap();
    settle(&db, keys, &user, &done, "All sent", None).await;
    let saved = checkpoint(&db, &user, &id).await.unwrap().unwrap();
    assert_eq!(saved.status, "completed");
    assert!(saved.completed_actions.is_empty() && saved.failed_actions.is_empty());
    assert_eq!(saved.goal.as_deref(), Some("Thanks"));

    // Another owner cannot read it; deleting the conversation purges it.
    assert!(checkpoint_summary(&db, "someone-else", &id).await.is_err());
    delete(&db, &user, &id).await.unwrap();
    assert_eq!(
        db.collection::<bson::Document>(CHECKPOINTS)
            .count_documents(doc! {"_id": &id})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn reset_context_is_fenced_and_drops_the_checkpoint() {
    let db = connect_transaction_test_database("nyxa_reset_fence").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let keys = &state.encryption_keys;
    let row = begin_turn(&db, &user, &request(None, "goal"), keys)
        .await
        .unwrap();
    // A live turn wins: nothing is reset.
    assert!(matches!(
        reset_context(&db, &user, &row.id).await,
        Err(AppError::AssistantTurnActive)
    ));
    settle(&db, keys, &user, &row, "answer", None).await;
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"nyxagent_session_id": "conv_bound"}},
        )
        .await
        .unwrap();
    assert!(checkpoint(&db, &user, &row.id).await.unwrap().is_some());
    reset_context(&db, &user, &row.id).await.unwrap();
    let reset = get(&db, &user, &row.id).await.unwrap();
    assert!(reset.nyxagent_session_id.is_none());
    assert_eq!(reset.context_reset_reason.as_deref(), Some("user_reset"));
    assert!(checkpoint(&db, &user, &row.id).await.unwrap().is_none());
    // Unknown and foreign conversations are not found.
    assert!(matches!(
        reset_context(&db, "someone-else", &row.id).await,
        Err(AppError::NotFound(_))
    ));
}

#[test]
fn recap_reserves_the_goal_before_long_recent_rows() {
    let row = |seq: i64, role: &str, text: String| AssistantMessage {
        continuation_of: None,
        retry_of: None,
        specialist_id: None,
        steering: None,
        voice: None,
        execution_pending: false,
        id: format!("m{seq}"),
        conversation_id: "c".into(),
        user_id: "u".into(),
        seq,
        turn_id: format!("t{seq}"),
        role: role.into(),
        text,
        status: "completed".into(),
        error_code: None,
        upstream_error_code: None,
        error_detail: None,
        created_at: Utc::now(),
        activities: Vec::new(),
        attachments: Vec::new(),
        origin: None,
        via: None,
    };
    // The goal plus 19 rows of 2 KiB: newest-first, the goal would be the
    // 20th row and no budget would remain for it.
    let mut history = vec![row(1, "user", "ORIGINATING GOAL".into())];
    history
        .extend((2..=20).map(|seq| row(seq, "assistant", format!("r{seq}:{}", "x".repeat(2048)))));
    let recap = bounded_recap(&history);
    assert!(recap.len() <= 16384);
    assert!(recap.contains("\nuser: ORIGINATING GOAL"), "goal kept");
    assert!(recap.contains("r20:"), "newest row kept");
    // The goal leads the recap.
    assert!(recap.find("ORIGINATING GOAL").unwrap() < recap.find("r20:").unwrap());
}

#[tokio::test]
async fn turn_images_are_bounded_copied_to_the_reply_and_deleted_with_the_chat() {
    use crate::models::assistant_attachment::COLLECTION_NAME as ATTACHMENTS;
    let db = connect_transaction_test_database("nyxa_images").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4();
    db.collection(USERS)
        .insert_one(test_user(&user.to_string(), UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let owner = user.to_string();
    let keys = &state.encryption_keys;
    let png = b"\x89PNG\r\n\x1a\nimage".to_vec();
    // No live turn: nothing is stored.
    assert!(
        attach_image(
            &db,
            keys,
            &owner,
            "nyxa-00000000000000000000000000000000",
            "t",
            "image/png",
            &png
        )
        .await
        .unwrap()
        .is_none()
    );
    let row = begin_turn(&db, &owner, &request(None, "show the lobby"), keys)
        .await
        .unwrap();
    let credential = credentials::load_for_conversation(&db, keys, &owner, &row.id)
        .await
        .unwrap()
        .unwrap();
    for i in 0..MAX_TURN_ATTACHMENTS {
        attach_image(
            &db,
            keys,
            &owner,
            &row.id,
            &format!("cam__snapshot_{i}"),
            "image/png",
            &png,
        )
        .await
        .unwrap()
        .unwrap();
    }
    assert!(
        attach_image(
            &db,
            keys,
            &owner,
            &row.id,
            "cam__overflow",
            "image/png",
            &png
        )
        .await
        .unwrap()
        .is_none(),
        "per-turn limit"
    );
    assert!(
        attach_image(&db, keys, "someone-else", &row.id, "cam", "image/png", &png)
            .await
            .unwrap()
            .is_none()
    );
    let count = || async {
        db.collection::<bson::Document>(ATTACHMENTS)
            .count_documents(doc! {"conversation_id": &row.id})
            .await
            .unwrap()
    };
    assert_eq!(count().await, MAX_TURN_ATTACHMENTS as u64);
    let message_id = Uuid::new_v4().to_string();
    finish_turn(
        &db,
        &row,
        &credential.api_key_id,
        &message_id,
        &TurnResult {
            text: "Here is the lobby.".into(),
            session_id: Some("s".into()),
            response_id: Some("r".into()),
            error: None,
        },
    )
    .await
    .unwrap();
    let reply = messages(&db, &owner, &row.id, 10, None)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.id == message_id)
        .unwrap();
    assert_eq!(reply.attachments.len(), MAX_TURN_ATTACHMENTS);
    assert_eq!(reply.attachments[0].label, "cam__snapshot_0");
    let (content_type, bytes) =
        read_attachment(&db, keys, &owner, &row.id, &reply.attachments[0].id)
            .await
            .unwrap();
    assert_eq!((content_type.as_str(), bytes), ("image/png", png.clone()));
    assert!(matches!(
        read_attachment(&db, keys, "someone-else", &row.id, &reply.attachments[0].id).await,
        Err(AppError::NotFound(_))
    ));
    // Settled: a late tool response cannot attach to a finished turn.
    assert!(
        attach_image(&db, keys, &owner, &row.id, "late", "image/png", &png)
            .await
            .unwrap()
            .is_none()
    );
    delete(&db, &owner, &row.id).await.unwrap();
    assert_eq!(count().await, 0, "deleted with the conversation");
}

#[test]
fn upstream_codes_are_bounded_identifiers_and_budget_errors_preserve_the_session() {
    for code in [
        "tool_budget_exhausted",
        "turn_timeout",
        "continuation_no_progress",
    ] {
        let error = TurnError::new(code);
        assert_eq!(error.code, code);
        assert!(error.preserves_session());
    }
    assert_eq!(
        TurnError::new("new_upstream_error")
            .upstream_code
            .as_deref(),
        Some("new_upstream_error")
    );
    for code in [
        "secret provider message",
        "Bearer xxx",
        "nyx_secret",
        "sk_secret",
        "new\nline",
        &"a".repeat(65),
    ] {
        let error = TurnError::new(code);
        assert!(error.upstream_code.is_none());
        assert!(!serde_json::to_string(&error).unwrap().contains(code));
    }
}

#[tokio::test]
async fn voice_queue_claims_one_message_and_preserves_initiating_identity() {
    use crate::services::feature_flag_service::FlagTarget;
    use crate::services::{assistant_voice as voice, feature_flag_service as flags};
    let db = connect_transaction_test_database("voice_admission").await;
    ensure_indexes(&db).await.unwrap();
    voice::ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    flags::set_platform_override(
        &db,
        flags::ASSISTANT_VOICE_FLAG_KEY,
        &FlagTarget::Global,
        true,
        &user,
    )
    .await
    .unwrap();
    let first = Box::pin(begin_turn(
        &db,
        &user,
        &request(None, "original request"),
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    let session = Uuid::new_v4().to_string();
    let a = voice::enqueue(
        &db,
        &user,
        &first.id,
        &session,
        "call-1",
        "first voice task",
    )
    .await
    .unwrap();
    let replay = voice::enqueue(
        &db,
        &user,
        &first.id,
        &session,
        "call-1",
        "ignored duplicate",
    )
    .await
    .unwrap();
    assert_eq!(a.id, replay.id);
    let b = voice::enqueue(
        &db,
        &user,
        &first.id,
        &session,
        "call-2",
        "later voice task",
    )
    .await
    .unwrap();
    assert_eq!(
        first.active_turn.as_ref().unwrap().initiating_message_seq,
        Some(1)
    );
    assert!(matches!(
        Box::pin(begin_turn_with_voice(
            &db,
            &user,
            &request(Some(&first.id), "first voice task"),
            &state.encryption_keys,
            Some(&a.id)
        ))
        .await,
        Err(AppError::AssistantTurnActive)
    ));
    let credential =
        credentials::load_for_conversation(&db, &state.encryption_keys, &user, &first.id)
            .await
            .unwrap()
            .unwrap();
    Box::pin(finish_turn(
        &db,
        &first,
        &credential.api_key_id,
        &Uuid::new_v4().to_string(),
        &TurnResult {
            text: "done".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    ))
    .await
    .unwrap();
    let claimed = Box::pin(begin_turn_with_voice(
        &db,
        &user,
        &request(Some(&first.id), "first voice task"),
        &state.encryption_keys,
        Some(&a.id),
    ))
    .await
    .unwrap();
    assert_eq!(claimed.active_turn.as_ref().unwrap().turn_id, a.turn_id);
    assert_eq!(
        claimed.active_turn.as_ref().unwrap().initiating_message_seq,
        Some(a.message_seq)
    );
    let history = messages(&db, &user, &first.id, 100, None).await.unwrap();
    assert_eq!(history.iter().filter(|m| m.id == a.message_id).count(), 1);
    assert!(
        !history
            .iter()
            .find(|m| m.id == a.message_id)
            .unwrap()
            .execution_pending
    );
    assert!(
        history
            .iter()
            .find(|m| m.id == b.message_id)
            .unwrap()
            .execution_pending
    );
    use crate::services::assistant_acknowledgement_service as acks;
    let chat = acks::for_key(&db, &user, Some(&credential.api_key_id))
        .await
        .unwrap()
        .unwrap();
    let card = Box::pin(acks::request(
        &db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("test_action"),
            arguments: Some(&json!({"id":"x"})),
            summary: "Delete the selected object",
            platform: false,
        },
    ))
    .await
    .unwrap();
    assert_eq!(card.requested_turn_id.as_deref(), Some(a.turn_id.as_str()));
    assert_eq!(card.voice_request_id.as_deref(), Some(a.id.as_str()));
    assert!(acks::refusal(&card).get("confirm_phrase").is_none());
    // Simulate a still-pending card created before the voice association existed.
    db.collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&card.id},
            doc! {"$unset":{"voice_request_id":""}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
        .update_one(
            doc! {"_id":&a.id},
            doc! {"$set":{"state":"claimed","pending_acknowledgement_ids":[]}},
        )
        .await
        .unwrap();
    let reused = Box::pin(acks::request(
        &db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("test_action"),
            arguments: Some(&json!({"id":"x"})),
            summary: "A replacement summary must not win",
            platform: false,
        },
    ))
    .await
    .unwrap();
    assert_eq!(reused.id, card.id);
    assert_eq!(reused.summary, card.summary);
    assert_eq!(reused.arguments_digest, card.arguments_digest);
    assert_eq!(reused.voice_request_id.as_deref(), Some(a.id.as_str()));
    let pending = voice::get(&db, &user, &first.id, &a.id).await.unwrap();
    assert_eq!(
        pending.state,
        crate::models::assistant_voice::RequestState::AwaitingConfirmation
    );
    assert_eq!(pending.pending_acknowledgement_ids, vec![card.id.clone()]);
    // Click and a future authoritative voice decision share this transaction.
    let (click, voice_decision) = tokio::join!(
        Box::pin(acks::decide(&db, &user, &first.id, &card.id, true)),
        Box::pin(acks::decide(&db, &user, &first.id, &card.id, false))
    );
    assert_ne!(click.is_ok(), voice_decision.is_ok());
    let decision = click.or(voice_decision).unwrap();
    let continuation = voice::get(
        &db,
        &user,
        &first.id,
        decision.continuation_receipt_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        continuation.parent_turn_id.as_deref(),
        Some(a.turn_id.as_str())
    );
    assert_eq!(
        db.collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
            .count_documents(doc! {"source_id":format!("ack:{}",card.id)})
            .await
            .unwrap(),
        1
    );
    assert!(
        Box::pin(acks::decide(&db, &user, &first.id, &card.id, true))
            .await
            .is_err()
    );
    voice::cancel(&db, &user, &first.id, &b.id).await.unwrap();
    assert!(
        !stop_requested(&db, &user, &first.id, &a.turn_id)
            .await
            .unwrap()
    );
    voice::cancel(&db, &user, &first.id, &a.id).await.unwrap();
    assert!(
        stop_requested(&db, &user, &first.id, &a.turn_id)
            .await
            .unwrap()
    );
    Box::pin(finish_turn(
        &db,
        &claimed,
        &credential.api_key_id,
        &Uuid::new_v4().to_string(),
        &TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(TurnError::new("cancelled")),
        },
    ))
    .await
    .unwrap();
    let input = messages(&db, &user, &first.id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.id == continuation.message_id)
        .unwrap()
        .text;
    assert!(matches!(
        Box::pin(begin_turn_with_voice(
            &db,
            &user,
            &request(Some(&first.id), &input),
            &state.encryption_keys,
            Some(&continuation.id)
        ))
        .await,
        Err(AppError::Conflict(_))
    ));
}

#[tokio::test]
async fn voice_queue_bounds_concurrent_admission_and_never_replays_lost_claims() {
    Box::pin(async {
        use crate::models::assistant_voice::{REQUESTS, RequestState, VoiceRequest};
        use crate::services::{assistant_voice as voice, feature_flag_service as flags};
        let db = connect_transaction_test_database("voice_queue_races").await;
        ensure_indexes(&db).await.unwrap();
        voice::ensure_indexes(&db).await.unwrap();
        let user = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&user, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        flags::set_platform_override(
            &db,
            flags::ASSISTANT_VOICE_FLAG_KEY,
            &flags::FlagTarget::Global,
            true,
            &user,
        )
        .await
        .unwrap();
        let first = Box::pin(begin_turn(
            &db,
            &user,
            &request(None, "original"),
            &state.encryption_keys,
        ))
        .await
        .unwrap();
        let session = Uuid::new_v4().to_string();
        let admissions = futures::future::join_all((0..11).map(|i| {
            let (db, user, conversation, session) = (&db, &user, &first.id, &session);
            async move {
                voice::enqueue(
                    db,
                    user,
                    conversation,
                    session,
                    &format!("source-{i}"),
                    "queued",
                )
                .await
            }
        }))
        .await;
        assert_eq!(admissions.iter().filter(|r| r.is_ok()).count(), 10);
        assert_eq!(
            admissions
                .iter()
                .filter(|r| matches!(r, Err(AppError::VoiceQueueFull)))
                .count(),
            1
        );
        let credential =
            credentials::load_for_conversation(&db, &state.encryption_keys, &user, &first.id)
                .await
                .unwrap()
                .unwrap();
        Box::pin(finish_turn(
            &db,
            &first,
            &credential.api_key_id,
            &Uuid::new_v4().to_string(),
            &TurnResult {
                text: "done".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        ))
        .await
        .unwrap();
        let next = db
            .collection::<VoiceRequest>(REQUESTS)
            .find_one(doc! {"state":"queued"})
            .sort(doc! {"message_seq":1})
            .await
            .unwrap()
            .unwrap();
        let later = db
            .collection::<VoiceRequest>(REQUESTS)
            .find_one(doc! {"state":"queued","message_seq":{"$gt":next.message_seq}})
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            Box::pin(begin_turn_with_voice(
                &db,
                &user,
                &request(Some(&first.id), "queued"),
                &state.encryption_keys,
                Some(&later.id)
            ))
            .await,
            Err(AppError::AssistantTurnActive)
        ));
        assert_eq!(
            voice::get(&db, &user, &first.id, &later.id)
                .await
                .unwrap()
                .state,
            RequestState::Queued
        );
        assert!(matches!(
            Box::pin(begin_turn_with_voice(
                &db,
                &user,
                &request(Some(&first.id), "substituted input"),
                &state.encryption_keys,
                Some(&next.id)
            ))
            .await,
            Err(AppError::Conflict(_))
        ));
        let start = request(Some(&first.id), "queued");
        let (one, two) = tokio::join!(
            Box::pin(begin_turn_with_voice(
                &db,
                &user,
                &start,
                &state.encryption_keys,
                Some(&next.id)
            )),
            Box::pin(begin_turn_with_voice(
                &db,
                &user,
                &start,
                &state.encryption_keys,
                Some(&next.id)
            ))
        );
        assert_ne!(one.is_ok(), two.is_ok());
        let claimed = one.or(two).unwrap();
        let old = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(2));
        db.collection::<bson::Document>(REQUESTS)
            .update_one(doc! {"_id":&next.id}, doc! {"$set":{"created_at":old}})
            .await
            .unwrap();
        db.collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":&claimed.id},
                doc! {"$set":{"active_turn.started_at":old}},
            )
            .await
            .unwrap();
        voice::recover(&db).await.unwrap();
        voice::recover(&db).await.unwrap();
        let recovered = voice::get(&db, &user, &first.id, &next.id).await.unwrap();
        assert_eq!(recovered.state, RequestState::Cancelled);
        assert!(matches!(
            Box::pin(begin_turn_with_voice(
                &db,
                &user,
                &start,
                &state.encryption_keys,
                Some(&next.id)
            ))
            .await,
            Err(AppError::Conflict(_))
        ));
    })
    .await;
}

#[test]
fn instruction_fingerprint_covers_configuration_and_audience_without_database_access() {
    use crate::models::{
        assistant_agent::{AssistantAgent, MemoryNote},
        catalog_skill_revision::SkillReference,
    };
    use crate::services::assistant_instruction_context::Prepared;
    let now = Utc::now();
    let mut row = stale_test_row(now);
    row.role = AgentRole::Subagent;
    row.nyxagent_session_id = None;
    let mut agent: AssistantAgent = bson::from_document(doc! {
        "_id":"agent", "user_id":&row.user_id, "kind":"specialist", "name":"helper",
        "description":"Track releases", "created_by":"user", "model":DEFAULT_MODEL,
        "created_at":bson::DateTime::from_chrono(now), "updated_at":bson::DateTime::from_chrono(now)
    })
    .unwrap();
    let initial = Prepared::new(&[2; 32], &row, Some(&agent), &[]).unwrap();
    let mut binding = initial.binding.clone();
    binding.session_id = "bound-session".into();
    row.nyxagent_session_id = Some(binding.session_id.clone());
    row.nyxagent_instruction_binding = Some(binding);
    // All preparation below is synchronous with no database argument: no
    // fingerprint-specific reads can be introduced on the hot path.
    let prepare = |r: &AssistantConversation, a: &AssistantAgent| {
        Prepared::new(&[2; 32], r, Some(a), &[]).unwrap()
    };
    row.active_turn.as_mut().unwrap().note = Some("A different sender/context".into());
    row.active_turn
        .as_mut()
        .unwrap()
        .events
        .push(crate::services::assistant_team_service::event(
            "message",
            "Current news".into(),
            None,
        ));
    agent.updated_at = now + chrono::Duration::hours(1);
    agent.grants.service_ids.push("another-service".into());
    assert!(!prepare(&row, &agent).reset);
    for field in [
        "description",
        "persona",
        "display_name",
        "name",
        "memory",
        "skills",
        "machines",
        "owner",
    ] {
        let mut changed = agent.clone();
        match field {
            "description" => changed.description = "A different role".into(),
            "persona" => changed.persona = Some("A different tone".into()),
            "display_name" => changed.display_name = Some("Different display".into()),
            "name" => changed.name = "helper-renamed".into(),
            "memory" => changed.memory.push(MemoryNote {
                id: "note".into(),
                text: "Current preferences".into(),
                created_at: now,
                updated_at: now,
            }),
            "skills" => changed.skills.push(SkillReference {
                source: "ornn".into(),
                skill_id: "skill".into(),
                name: "Release planning".into(),
                version: "1.0.0".into(),
                sha256: "a".repeat(64),
                dependencies: vec![],
                disabled: false,
            }),
            "machines" => changed.machine_node_ids.push("machine".into()),
            "owner" => changed.user_id = "organization".into(),
            _ => unreachable!(),
        }
        let prepared = prepare(&row, &changed);
        if field == "memory" {
            assert!(
                !prepared.reset,
                "memory changes are per-turn input, not stable instructions"
            );
            assert!(
                prepared
                    .input_with_memory("Current preferences", "continue", true)
                    .contains("Current preferences")
            );
            continue;
        }
        assert!(
            prepared.reset,
            "stable field {field} must invalidate the binding"
        );
        assert!(prepared.binding.marker != initial.binding.marker);
    }
    row.guest_turn = true;
    assert!(prepare(&row, &agent).reset);
    row.guest_turn = false;
    // An old replica may bind another session while retaining stale metadata.
    // Do not mistake the old marker for that session's initial instructions.
    row.nyxagent_session_id = Some("old-replica-replacement".into());
    // The replacement was established after this profile update.
    row.context_reset_at = Some(agent.updated_at);
    let adopted = prepare(&row, &agent);
    assert!(!adopted.reset && adopted.binding.marker.is_none());
    row.context_reset_at = None;
    // With no establishment clock, conversation creation is the conservative
    // lower bound: repair pre-upgrade profile edits for either audience once.
    for guest in [false, true] {
        row.guest_turn = guest;
        assert!(prepare(&row, &agent).reset);
        row.context_reset_at = Some(agent.updated_at);
        assert!(!prepare(&row, &agent).reset);
        row.context_reset_at = Some(agent.updated_at - chrono::Duration::seconds(1));
        assert!(prepare(&row, &agent).reset);
        row.context_reset_at = None;
    }
}

#[test]
fn model_validation_and_unavailable_copy_are_stable() {
    assert!(super::valid_model("nyxagent/chat"));
    assert!(super::valid_model("nyxagent/reasoning_v2"));
    assert!(!super::valid_model("openai/gpt-4"));
    assert!(!super::valid_model("nyxagent/"));
}

#[test]
fn memory_remembered_in_one_thread_keeps_another_threads_binding() {
    use crate::models::assistant_agent::{AssistantAgent, MemoryNote};
    use crate::services::assistant_instruction_context::Prepared;
    let now = Utc::now();
    let mut agent: AssistantAgent = bson::from_document(doc! {
        "_id":"agent", "user_id":"owner", "kind":"nyxbot", "name":"nyxbot",
        "created_by":"user", "model":DEFAULT_MODEL,
        "created_at":bson::DateTime::from_chrono(now), "updated_at":bson::DateTime::from_chrono(now)
    })
    .unwrap();
    // Two bound threads of the same agent.
    let bind = |agent: &AssistantAgent| {
        let mut row = stale_test_row(now);
        row.nyxagent_session_id = None;
        let mut binding = Prepared::new(&[2; 32], &row, Some(agent), &[])
            .unwrap()
            .binding;
        binding.session_id = format!("conv_{}", row.id);
        row.nyxagent_session_id = Some(binding.session_id.clone());
        row.nyxagent_instruction_binding = Some(binding);
        row
    };
    let _thread_a = bind(&agent);
    let thread_b = bind(&agent);
    // `nyxid__remember` in thread A adds a note to the agent.
    agent.memory.push(MemoryNote {
        id: "note".into(),
        text: "Prefers metric units".into(),
        created_at: now,
        updated_at: now,
    });
    let prepared = Prepared::new(&[2; 32], &thread_b, Some(&agent), &[]).unwrap();
    assert!(!prepared.reset, "thread B keeps its live context");
    assert_eq!(
        prepared.binding.session_id,
        thread_b.nyxagent_session_id.clone().unwrap()
    );
    assert!(
        prepared
            .input_with_memory("", "next question", true)
            .contains("Prefers metric units"),
        "thread B's next turn sees the new note in its input"
    );
    assert!(
        !prepared
            .instructions(&[], false)
            .contains("Prefers metric units")
    );
}

#[test]
fn channel_review_links_and_skill_selections_survive_prompt_formatting() {
    let mut row = stale_test_row(Utc::now());
    row.channel = Some(crate::models::assistant_conversation::ChannelOrigin {
        nyxbot_channel_id: "bot".into(),
        platform: "telegram".into(),
        partition: "chat".into(),
        thread: None,
    });
    let note = channel_note(&row);
    assert!(note.contains("review_url as a full URL"));
    assert!(note.contains("chat replies cannot approve"));
    let selection = "pin ".repeat(500);
    let event = crate::services::assistant_team_service::event(
        "skill_request_allowed",
        format!("{selection} Attach it now."),
        None,
    );
    let note = events_text(&[event]);
    assert!(note.ends_with("Attach it now."));
}

#[test]
fn upstream_model_rejections_report_model_unavailable_with_guidance() {
    for code in [
        "model_not_found",
        "model_unavailable",
        "unknown_model",
        "invalid_model",
    ] {
        let error = super::TurnError::new(code);
        assert_eq!(error.code, "model_unavailable", "{code}");
        assert!(error.message.contains("model selector"), "{code}");
        assert_eq!(error.upstream_code.as_deref(), Some(code));
    }
    assert!(
        super::TurnError::new("model_not_configured")
            .message
            .contains("model selector")
    );
}
