use super::*;
use crate::models::assistant_attachment::COLLECTION_NAME as ATTACHMENTS;
use crate::models::user::{COLLECTION_NAME as USERS, UserType};
use crate::services::assistant_agent_credential_service as credentials;
use crate::services::assistant_engine_policy as engine_policy;
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
    for (status, code) in [
        (409, "session_busy"),
        (503, "capacity_exceeded"),
        (503, "draining"),
    ] {
        let mut retry = Recovery::default();
        for _ in 0..100 {
            assert_eq!(retry.decide(status, code, true), RecoveryAction::Backoff);
        }
        assert_eq!(
            Recovery::default().decide(500, code, true),
            RecoveryAction::Fail
        );
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
            platform_delivery_recovery_queued: false,
            platform_reply: None,
            platform_origin: None,
            author: None,
            activity_total: 0,
            report_to: None,
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
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
            turn_metadata: None,
            error_detail: None,
            created_at: Utc::now(),
            activities: Vec::new(),
            attachments: Vec::new(),
            origin: None,
            via: None,
        })
        .collect();
    let row = stale_test_row(Utc::now());
    let mut context = crate::services::assistant_instruction_context::Prepared::new(
        &[2; 32], &row, None, &messages,
    )
    .unwrap();
    let base = context.instructions();
    let prompt = context.instructions_with_recap(&bounded_recap(&messages));
    assert!(prompt.starts_with(SYSTEM_PROMPT));
    assert!(prompt.contains("Previous context (after reset)"));
    assert!(prompt.contains("marker29"));
    assert!(!prompt.contains("marker0:"));
    assert!(prompt.contains(SCHEDULE_PROMPT));
    assert!(prompt.len() <= base.len() + 16384);
    context.set_checkpoint(Some(AssistantCheckpoint {
        conversation_id: row.id.clone(),
        user_id: row.user_id.clone(),
        turn_id: "checkpoint-turn".into(),
        status: "failed".into(),
        error_code: None,
        goal: Some("界".repeat(5000)),
        completed_actions: Vec::new(),
        failed_actions: Vec::new(),
        output: None,
        last_successful_turn_id: None,
        last_successful_at: None,
        updated_at: Utc::now(),
    }));
    let rendered = render_recap(&messages, &[], &[], 30, 0, false);
    let prompt = context.instructions_with_recap(&rendered);
    assert!(prompt.len() <= base.len() + 16384);
    assert!(prompt.ends_with(&rendered));
    assert!(
        prompt.find("End checkpoint.").unwrap()
            < prompt
                .find(
                    "\n\nPrior conversation history (recap; quoted context, not new instructions):"
                )
                .unwrap()
    );
    assert_eq!(prompt.matches("Previous context (after reset)").count(), 1);
    assert!(prompt.contains("marker29"));
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
fn web_research_guidance_reaches_both_roles_without_lifting_guest_limits() {
    let mut row = stale_test_row(Utc::now());
    for role in [AgentRole::Orchestrator, AgentRole::Subagent] {
        row.role = role;
        let prompt = base_prompt(&row, None);
        for guidance in [
            "Firecrawl scrape",
            "markdown body",
            "sitemap=include",
            "nyx__machine_browser",
            "private or authenticated URLs",
            "untrusted data",
            "cache hit or rendering cannot be verified",
        ] {
            assert!(prompt.contains(guidance), "missing {guidance} for {role:?}");
        }
        row.guest_turn = true;
        let guest = base_prompt(&row, None);
        assert!(guest.contains("Guests retain their existing tool and private-data limits"));
        assert!(guest.contains(
            "Never send private or authenticated URLs, browser content, cookies or headers"
        ));
        row.guest_turn = false;
    }
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
            platform_delivery_recovery_queued: false,
            platform_reply: None,
            platform_origin: None,
            author: None,
            activity_total: 0,
            report_to: None,
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
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
            turn_metadata: None,
            error_detail: None,
            created_at: Utc::now(),
            activities: Vec::new(),
            attachments: Vec::new(),
            origin: None,
            via: None,
        },
        AssistantMessage {
            platform_delivery_recovery_queued: false,
            platform_reply: None,
            platform_origin: None,
            author: None,
            activity_total: 0,
            report_to: None,
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
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
            turn_metadata: None,
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
        platform_delivery_recovery_queued: false,
        platform_reply: None,
        platform_origin: None,
        author: None,
        activity_total: 0,
        report_to: None,
        continuation_of: None,
        retry_of: None,
        retry_claimed_by: None,
        activity_count: None,
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
        turn_metadata: None,
        error_detail: None,
        created_at: Utc::now(),
        activities: (0..40)
            .map(|i| crate::models::assistant_conversation::TurnActivity {
                failure: None,
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
        platform_delivery_recovery_queued: false,
        platform_reply: None,
        platform_origin: None,
        author: None,
        activity_total: 0,
        report_to: None,
        continuation_of: None,
        retry_of: None,
        retry_claimed_by: None,
        activity_count: None,
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
        turn_metadata: None,
        error_detail: None,
        created_at: Utc::now(),
        activities: vec![crate::models::assistant_conversation::TurnActivity {
            failure: None,
            machine: None,
            id: "a".into(),
            digest: None,
            label: "tool.check".into(),
            status: "completed".into(),
            started_at: Utc::now(),
            ended_at: None,
        }],
        attachments: vec![crate::models::assistant_conversation::TurnAttachment {
            chat_delivery: None,
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
        platform_delivery_recovery_queued: false,
        platform_reply: None,
        platform_origin: None,
        author: None,
        activity_total: 0,
        report_to: None,
        continuation_of: None,
        retry_of: None,
        retry_claimed_by: None,
        activity_count: None,
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
        turn_metadata: None,
        error_detail: None,
        created_at: Utc::now(),
        activities: vec![crate::models::assistant_conversation::TurnActivity {
            failure: None,
            machine: None,
            id: "owner-action".into(),
            digest: None,
            label: "owner.secret_tool".into(),
            status: "completed".into(),
            started_at: Utc::now(),
            ended_at: None,
        }],
        attachments: vec![crate::models::assistant_conversation::TurnAttachment {
            chat_delivery: None,
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
    let recap = bounded_recap_for_audience(std::slice::from_ref(&row), true);
    assert!(recap.contains("Visible channel reply"));
    assert!(!recap.contains("owner.secret_tool"));
    assert!(!recap.contains("private.txt"));
    assert!(!recap.contains("turn_lost"));
}

#[test]
fn retry_recap_lists_completed_activity_ids_for_no_duplicate_replay() {
    let row = AssistantMessage {
        platform_delivery_recovery_queued: false,
        platform_reply: None,
        platform_origin: None,
        author: None,
        activity_total: 0,
        report_to: None,
        continuation_of: None,
        retry_of: None,
        retry_claimed_by: None,
        activity_count: None,
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
        turn_metadata: None,
        error_detail: None,
        created_at: Utc::now(),
        activities: vec![crate::models::assistant_conversation::TurnActivity {
            failure: None,
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
        platform_thread: None,
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
            platform_reply: None,
            execution_started: false,
            machine_scopes: Vec::new(),
            platform_origin: None,
            activity_total: 0,
            worker_instance: None,
            model_attempt_id: None,
            model_requests_active: 0,
            model_attribution_uncertain: false,
            model_refusal: None,
            model_stream_fault: None,
            model_stream_cancellation: None,
            waiting_for_capacity: false,
            capacity_wait_note_origin: None,
            engine_request_started_at: None,
            engine_attempts: 0,
            capacity_wait_started_at: None,
            capacity_wait_ms: 0,
            capacity_wait_deadline: None,
            capacity_accepted_at: None,
            capacity_notice_claimed: false,
            retry_digests: Vec::new(),
            retry_of: None,
            activity_count: None,
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
            report_to: None,
            with_nyxbot: false,
            trigger_continuation: false,
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

fn platform_test_origin(
    row: &AssistantConversation,
) -> crate::models::assistant_conversation::PlatformTurnOrigin {
    use crate::models::assistant_conversation::{
        PlatformOriginSource, PlatformThread, PlatformTurnOrigin,
    };
    PlatformTurnOrigin {
        thread: PlatformThread {
            platform: "telegram".into(),
            profile_id: "manager-profile".into(),
            manager_bot_id: "manager".into(),
            telegram_bot_id: 123,
            binding_id: "personal-binding".into(),
            binding_generation: 7,
            user_id: row.user_id.clone(),
            nyxbot_agent_id: row.agent_id.clone().unwrap_or_else(|| "agent".into()),
        },
        manager_authority_epoch: 3,
        personal_control_revision: 11,
        sender_id: 456,
        chat_id: 789,
        source: PlatformOriginSource::Ingress {
            receipt_id: "receipt".into(),
            claim_id: "claim".into(),
        },
    }
}

async fn seed_platform_execution(
    db: &Database,
    row: &AssistantConversation,
    proof: &crate::models::assistant_conversation::PlatformTurnOrigin,
) {
    use crate::models::assistant_conversation::PlatformOriginSource;
    let turn = row.active_turn.as_ref().unwrap();
    let PlatformOriginSource::Ingress {
        receipt_id,
        claim_id,
    } = &proof.source
    else {
        panic!("ingress fixture");
    };
    db.collection::<bson::Document>(crate::models::platform_personal_binding::COLLECTION_NAME).insert_one(doc! {
        "_id":&proof.thread.binding_id, "user_id":&row.user_id, "generation":proof.thread.binding_generation,
        "state":"revoked", "conversation_id":&row.id, "nyxbot_agent_id":&proof.thread.nyxbot_agent_id,
        "active_turn":{"conversation_id":&row.id,"turn_id":&turn.turn_id},
    }).await.unwrap();
    db.collection::<bson::Document>(crate::models::platform_manager_ingress::COLLECTION_NAME).insert_one(doc! {
        "_id":receipt_id, "claim_id":claim_id, "phase":"executing", "reply_id":bson::Bson::Null,"decision":bson::Bson::Null,
        "profile_id":&proof.thread.profile_id,"manager_bot_id":&proof.thread.manager_bot_id,
        "telegram_bot_id":proof.thread.telegram_bot_id,"manager_authority_epoch":proof.manager_authority_epoch,
        "sender_id":proof.sender_id,"chat_id":proof.chat_id,
        "audience":{"kind":"personal","user_id":&row.user_id,"binding_id":&proof.thread.binding_id,
            "binding_generation":proof.thread.binding_generation,"personal_control_revision":proof.personal_control_revision},
        "execution":{"conversation_id":&row.id,"turn_id":&turn.turn_id,"payer_user_id":&row.user_id},
    }).await.unwrap();
}

async fn assert_platform_settled(
    db: &Database,
    proof: &crate::models::assistant_conversation::PlatformTurnOrigin,
    reply_id: &str,
) {
    let binding = db
        .collection::<bson::Document>(crate::models::platform_personal_binding::COLLECTION_NAME)
        .find_one(doc! {"_id":&proof.thread.binding_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.get("active_turn"), Some(&bson::Bson::Null));
    let receipt = db
        .collection::<bson::Document>(crate::models::platform_manager_ingress::COLLECTION_NAME)
        .find_one(doc! {"audience.binding_id":&proof.thread.binding_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.get_str("phase").unwrap(), "settled");
    assert_eq!(receipt.get_str("reply_id").unwrap(), reply_id);
}

#[test]
fn platform_provenance_round_trips_and_legacy_rows_remain_origin_free() {
    let mut row = stale_test_row(Utc::now());
    let mut legacy = bson::to_document(&row).unwrap();
    legacy.remove("platform_thread");
    legacy
        .get_document_mut("active_turn")
        .unwrap()
        .remove("platform_origin");
    let restored: AssistantConversation = bson::from_document(legacy).unwrap();
    assert!(restored.platform_thread.is_none());
    assert!(restored.active_turn.unwrap().platform_origin.is_none());
    let proof = platform_test_origin(&row);
    row.platform_thread = Some(proof.thread.clone());
    row.active_turn.as_mut().unwrap().platform_origin = Some(proof.clone());
    let restored: AssistantConversation =
        bson::from_document(bson::to_document(&row).unwrap()).unwrap();
    assert_eq!(restored.platform_thread.as_ref(), Some(&proof.thread));
    assert_eq!(
        restored.active_turn.unwrap().platform_origin,
        Some(proof.clone())
    );
    let debug = format!("{proof:?}");
    for internal in ["receipt", "claim", "456", "789", "personal-binding"] {
        assert!(!debug.contains(internal));
    }
}

#[tokio::test]
async fn platform_thread_is_excluded_from_home_validation_fallback_repair_and_web_adoption() {
    let db = connect_transaction_test_database("nyxa_platform_home").await;
    ensure_indexes(&db).await.unwrap();
    let state = test_app_state(db.clone());
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let ordinary = begin_turn(
        &db,
        &owner,
        &request(None, "ordinary"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let result = TurnResult {
        text: "reply".into(),
        session_id: None,
        response_id: None,
        error: None,
    };
    finish_turn(
        &db,
        &ordinary,
        &ordinary.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &result,
    )
    .await
    .unwrap();
    let platform = begin_turn(
        &db,
        &owner,
        &request(None, "personal"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    finish_turn(
        &db,
        &platform,
        &platform.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &result,
    )
    .await
    .unwrap();
    let proof = platform_test_origin(&platform);
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &platform.id},
            doc! {"$set": {"platform_thread": bson::to_bson(&proof.thread).unwrap()}},
        )
        .await
        .unwrap();
    let agents = db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME);
    let agent_id = platform.agent_id.as_deref().unwrap();
    agents
        .update_one(
            doc! {"_id": agent_id},
            doc! {"$set": {"home_conversation_id": &platform.id}},
        )
        .await
        .unwrap();
    let team = crate::services::assistant_team_service::agent(&db, &owner, agent_id)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread_for(
        &db,
        &state.encryption_keys,
        &owner,
        &team,
    )
    .await
    .unwrap();
    assert_eq!(home.id, ordinary.id);
    agents
        .update_one(
            doc! {"_id": agent_id},
            doc! {"$set": {"home_conversation_id": &platform.id}},
        )
        .await
        .unwrap();
    assert_eq!(repair_channel_homes(&db).await.unwrap(), 1);
    let web = begin_turn(
        &db,
        &owner,
        &request(Some(&platform.id), "web after unlink"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(web.platform_thread.is_some());
    assert!(web.active_turn.as_ref().unwrap().platform_origin.is_none());
    let team = crate::services::assistant_team_service::agent(&db, &owner, agent_id)
        .await
        .unwrap();
    assert!(team.home_conversation_id.is_none());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn shared_owner_admission_commits_proof_input_and_key_together_without_home_adoption() {
    let db = connect_transaction_test_database("nyxa_platform_admission").await;
    ensure_indexes(&db).await.unwrap();
    let state = test_app_state(db.clone());
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&db, &owner)
        .await
        .unwrap();
    let mut template = stale_test_row(Utc::now());
    template.user_id = owner.clone();
    template.agent_id = Some(agent.id.clone());
    let proof = platform_test_origin(&template);
    let ids = OwnerTurnIds {
        conversation_id: template.id,
        turn_id: Uuid::new_v4().to_string(),
        message_id: Uuid::new_v4().to_string(),
    };
    let start = TurnStart::from(&request(None, "personal input"));
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let aborted = begin_owner_turn_in_session(
        &db,
        &mut session,
        &state.encryption_keys,
        &owner,
        &start,
        &ids,
        &agent,
        Some(99),
        Some(&proof),
        None,
    )
    .await
    .unwrap()
    .into_started()
    .unwrap();
    session.abort_transaction().await.unwrap();
    assert!(get(&db, &owner, &ids.conversation_id).await.is_err());
    assert!(
        db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! {"_id": &aborted.credential.api_key_id})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.collection::<AssistantMessage>(MESSAGES)
            .count_documents(doc! {"conversation_id": &ids.conversation_id})
            .await
            .unwrap(),
        0
    );
    session.start_transaction().await.unwrap();
    let committed = begin_owner_turn_in_session(
        &db,
        &mut session,
        &state.encryption_keys,
        &owner,
        &start,
        &ids,
        &agent,
        Some(99),
        Some(&proof),
        None,
    )
    .await
    .unwrap()
    .into_started()
    .unwrap();
    session.commit_transaction().await.unwrap();
    let saved = get(&db, &owner, &ids.conversation_id).await.unwrap();
    assert_eq!(saved.credential_api_key_id, committed.credential.api_key_id);
    assert!(saved.learning_epoch.is_none());
    assert_eq!(saved.platform_thread.as_ref(), Some(&proof.thread));
    assert_eq!(
        saved.active_turn.as_ref().unwrap().platform_origin.as_ref(),
        Some(&proof)
    );
    let input = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"_id": &ids.message_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(input.platform_origin.as_ref(), Some(&proof));
    assert_eq!(input.turn_id, ids.turn_id);
    assert!(
        crate::services::assistant_team_service::agent(&db, &owner, &agent.id)
            .await
            .unwrap()
            .home_conversation_id
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn settlement_retains_platform_origin_after_active_turn_is_cleared() {
    let db = connect_transaction_test_database("nyxa_platform_settlement").await;
    ensure_indexes(&db).await.unwrap();
    let state = test_app_state(db.clone());
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let mut row = begin_turn(
        &db,
        &owner,
        &request(None, "personal"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let proof = platform_test_origin(&row);
    row.platform_thread = Some(proof.thread.clone());
    row.active_turn.as_mut().unwrap().platform_origin = Some(proof.clone());
    let destination = crate::models::assistant_conversation::PlatformReplyRouting::Captured {
        origin: Box::new(proof.clone()),
        turn_id: row.active_turn.as_ref().unwrap().turn_id.clone(),
        expires_at: bson::DateTime::from_chrono(Utc::now() + chrono::Duration::minutes(30))
            .to_chrono(),
    };
    row.active_turn.as_mut().unwrap().platform_reply = Some(destination.clone());
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .replace_one(doc! {"_id": &row.id}, &row)
        .await
        .unwrap();
    seed_platform_execution(&db, &row, &proof).await;
    let message_id = Uuid::new_v4().to_string();
    finish_turn(
        &db,
        &row,
        &row.credential_api_key_id,
        &message_id,
        &TurnResult {
            text: "personal reply".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    assert!(
        get(&db, &owner, &row.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    let reply = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"_id": &message_id})
        .await
        .unwrap()
        .unwrap();
    assert_platform_settled(&db, &proof, &reply.id).await;
    assert_eq!(reply.platform_origin, Some(proof));
    assert_eq!(reply.platform_reply, Some(destination));
    let mut legacy = bson::to_document(&reply).unwrap();
    legacy.remove("platform_origin");
    assert!(
        bson::from_document::<AssistantMessage>(legacy)
            .unwrap()
            .platform_origin
            .is_none()
    );
    db.drop().await.unwrap();
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
    let proof = platform_test_origin(&old);
    let destination = crate::models::assistant_conversation::PlatformReplyRouting::Captured {
        origin: Box::new(proof.clone()),
        turn_id: old.active_turn.as_ref().unwrap().turn_id.clone(),
        expires_at: bson::DateTime::from_chrono(Utc::now() + chrono::Duration::minutes(30))
            .to_chrono(),
    };
    seed_platform_execution(&db, &old, &proof).await;
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &old.id},
            doc! {"$set": {
                "platform_thread": bson::to_bson(&proof.thread).unwrap(),
                "active_turn.platform_origin": bson::to_bson(&proof).unwrap(),
                "active_turn.platform_reply": bson::to_bson(&destination).unwrap(),
            }},
        )
        .await
        .unwrap();
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &old.id},
            doc! {"$set": {"active_turn.waiting_for_capacity": true, "active_turn.activities": [{"id": "act-1", "label": "nyxid__check", "status": "running", "started_at": bson::DateTime::now(), "ended_at": bson::Bson::Null}], "active_turn.attachments": [{"id": "att-1", "content_type": "text/plain", "size": 4i64, "label": "result.txt", "origin": "tool"}]}},
        )
        .await
        .unwrap();
    expire_turn(&db, &old).await;
    db.collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME)
        .insert_one(doc! {"_id": "reclaimed-run", "user_id": "owner",
            "turn_id": &old.active_turn.as_ref().unwrap().turn_id, "attempts": null, "capacity_wait_ms": null})
        .await.unwrap();
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id": &old.id}, doc! {"$set": {
            "active_turn.trigger_run_id": "reclaimed-run", "active_turn.engine_attempts": 2,
            "active_turn.capacity_wait_started_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(20)),
        }}).await.unwrap();
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
    assert_eq!(lost.platform_origin.as_ref(), Some(&proof));
    assert_eq!(lost.platform_reply.as_ref(), Some(&destination));
    assert_platform_settled(&db, &proof, &lost.id).await;
    assert!(next.active_turn.as_ref().unwrap().platform_origin.is_none());
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
    let run = db
        .collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME)
        .find_one(doc! {"_id": "reclaimed-run"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.get_i64("attempts").unwrap(), 2);
    assert!(run.get_i64("capacity_wait_ms").unwrap() >= 20_000);
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
async fn reclaim_preserves_lost_turn_audience_across_guest_transitions() {
    for (old_guest, next_guest) in [(false, true), (true, false)] {
        let db = connect_transaction_test_database("nyxa_reclaim_guest").await;
        ensure_indexes(&db).await.unwrap();
        let user = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&user, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let keys = &state.encryption_keys;
        let mut initial: TurnStart = (&request(None, "old request")).into();
        initial.guest = old_guest;
        let old = begin_turn(&db, &user, initial, keys).await.unwrap();
        expire_turn(&db, &old).await;
        let mut next: TurnStart = (&request(Some(&old.id), "next request")).into();
        next.guest = next_guest;
        begin_turn(&db, &user, next, keys).await.unwrap();
        let lost = db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {
                "conversation_id": &old.id,
                "turn_id": &old.active_turn.as_ref().unwrap().turn_id,
                "role": "assistant",
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lost.origin, Some(TurnOrigin::User));
        let input = db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {
                "conversation_id": &old.id, "turn_id": &lost.turn_id, "role": "user",
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retry_evidence(
                &RetryEvidence::from(&lost),
                Some(&input),
                &Default::default()
            )
            .is_ok(),
            !old_guest,
            "reclamation must use the original author's authority, not the next audience"
        );
    }
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
    use tracing::instrument::WithSubscriber;
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
    assert_eq!(
        stale.active_turn.as_ref().unwrap().worker_instance.as_ref(),
        Some(crate::services::assistant_engine_worker::instance())
    );
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
            doc! {"$set": {"active_turn.heartbeat_at": old,
            "active_turn.worker_instance": {"id": "old-process-id", "hostname": "nyxid-old-pod"}}},
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
    let capture = tempfile::NamedTempFile::new().unwrap();
    let writer = capture.reopen().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.try_clone().unwrap())
        .finish();
    assert_eq!(
        sweep_orphaned_turns(&db)
            .with_subscriber(subscriber)
            .await
            .unwrap(),
        1
    );
    let logs = std::fs::read_to_string(capture.path()).unwrap();
    assert!(logs.contains("nyxid::assistant_engine"), "{logs}");
    assert!(logs.contains("old-process-id"), "{logs}");
    assert!(logs.contains("nyxid-old-pod"), "{logs}");
    assert!(logs.contains("heartbeat_age_ms="), "{logs}");
    let audits = db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    let filter = doc! {"event_type": "assistant_turn_failed", "event_data.turn_id": &stale.active_turn.as_ref().unwrap().turn_id};
    let audit = audits.find_one(filter.clone()).await.unwrap().unwrap();
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("code").unwrap(), "turn_lost");
    assert_eq!(data.get_str("stage").unwrap(), "sweep");
    let worker = data.get_document("worker_instance").unwrap();
    assert_eq!(worker.get_str("id").unwrap(), "old-process-id");
    assert_eq!(worker.get_str("hostname").unwrap(), "nyxid-old-pod");
    assert_eq!(
        data.get_document("settled_by_worker_instance")
            .unwrap()
            .get_str("id")
            .unwrap(),
        crate::services::assistant_engine_worker::instance().id
    );
    assert!(data.get_i64("heartbeat_age_ms").unwrap() >= ACTIVE_TURN_HEARTBEAT_STALE_SECS * 1_000);
    assert_eq!(sweep_orphaned_turns(&db).await.unwrap(), 0);
    assert_eq!(audits.count_documents(filter).await.unwrap(), 1);
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

#[tokio::test]
async fn stopped_orphan_sweep_retains_heartbeat_diagnostics_for_legacy_worker() {
    let db = connect_transaction_test_database("nyxa_stopped_orphan").await;
    let state = test_app_state(db.clone());
    let row = begin_turn(
        &db,
        "owner",
        &request(None, "private stopped turn"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let turn_id = &row.active_turn.as_ref().unwrap().turn_id;
    let old = bson::DateTime::from_chrono(
        Utc::now() - chrono::Duration::seconds(ACTIVE_TURN_HEARTBEAT_STALE_SECS + 1),
    );
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {
                "$set": {"active_turn.stop_requested": true, "active_turn.heartbeat_at": old},
                "$unset": {"active_turn.worker_instance": ""},
            },
        )
        .await
        .unwrap();
    assert!(
        get(&db, "owner", &row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .worker_instance
            .is_none()
    );
    assert_eq!(sweep_orphaned_turns(&db).await.unwrap(), 1);
    let audit = db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type": "assistant_turn_failed", "event_data.turn_id": turn_id})
        .await
        .unwrap()
        .unwrap();
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("code").unwrap(), "cancelled");
    assert_eq!(data.get_str("stage").unwrap(), "stop");
    assert_eq!(data.get("worker_instance"), Some(&bson::Bson::Null));
    assert!(data.get_i64("heartbeat_age_ms").unwrap() >= ACTIVE_TURN_HEARTBEAT_STALE_SECS * 1_000);
    assert!(
        !serde_json::to_string(&audit)
            .unwrap()
            .contains("private stopped turn")
    );
    assert!(
        get(&db, "owner", &row.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
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
        activity_started_for_turn(
            &db,
            &owner,
            "nyxa-00000000000000000000000000000000",
            "stale",
            "x"
        )
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
    let first = activity_started_for_turn(
        &db,
        &owner,
        &row.id,
        &row.active_turn.as_ref().unwrap().turn_id,
        &long_label,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        activity_started_for_turn(
            &db,
            "other",
            &row.id,
            &row.active_turn.as_ref().unwrap().turn_id,
            "x"
        )
        .await
        .unwrap(),
        None
    );
    activity_finished_for_turn(
        &db,
        &owner,
        &row.id,
        &row.active_turn.as_ref().unwrap().turn_id,
        &first,
        true,
        None,
    )
    .await
    .unwrap();
    for i in 0..MAX_TURN_ACTIVITIES {
        activity_started_for_turn(
            &db,
            &owner,
            &row.id,
            &row.active_turn.as_ref().unwrap().turn_id,
            &format!("github__tool_{i}"),
        )
        .await
        .unwrap()
        .unwrap();
    }
    let live = get(&db, &owner, &row.id).await.unwrap();
    assert_eq!(
        live.active_turn.as_ref().unwrap().activity_total,
        MAX_TURN_ACTIVITIES + 1
    );
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
    activity_finished_for_turn(
        &db,
        &owner,
        &row.id,
        &row.active_turn.as_ref().unwrap().turn_id,
        &last,
        false,
        None,
    )
    .await
    .unwrap();
    // Unknown IDs and wrong owners are no-ops.
    activity_finished_for_turn(
        &db,
        "other",
        &row.id,
        &row.active_turn.as_ref().unwrap().turn_id,
        &last,
        true,
        None,
    )
    .await
    .unwrap();
    activity_finished_for_turn(
        &db,
        &owner,
        &row.id,
        &row.active_turn.as_ref().unwrap().turn_id,
        "missing",
        true,
        None,
    )
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
    assert_eq!(reply.activity_total, MAX_TURN_ACTIVITIES + 1);
    assert_eq!(reply.activity_count, Some(MAX_TURN_ACTIVITIES + 1));
    let old_turn_id = row.active_turn.as_ref().unwrap().turn_id.clone();
    // Label bound applies at insert.
    let row = begin_turn(
        &db,
        &owner,
        &request(Some(&row.id), "again"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    activity_started_for_turn(
        &db,
        &owner,
        &row.id,
        &row.active_turn.as_ref().unwrap().turn_id,
        &long_label,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        activity_started_for_turn(&db, &owner, &row.id, &old_turn_id, "private stale label")
            .await
            .unwrap()
            .is_none()
    );
    activity_label_for_turn(
        &db,
        &owner,
        &row.id,
        &old_turn_id,
        &last,
        "private stale label",
    )
    .await
    .unwrap();
    activity_finished_for_turn(&db, &owner, &row.id, &old_turn_id, &last, false, None)
        .await
        .unwrap();
    // A refused duplicate from a previous turn must not contaminate the new
    // turn's display total or retry evidence.
    record_retry_duplicate(
        &db,
        &owner,
        &row.id,
        &old_turn_id,
        "stale__call",
        "stale-digest",
    )
    .await
    .unwrap();
    let live = get(&db, &owner, &row.id).await.unwrap();
    let turn = live.active_turn.as_ref().unwrap();
    assert_eq!(turn.activity_total, 1);
    assert_eq!(turn.activity_count, Some(1));
    assert_eq!(turn.activities.len(), 1);
    record_retry_duplicate(&db, &owner, &row.id, &turn.turn_id, "$$ROOT", "$digest")
        .await
        .unwrap();
    let after_duplicate = get(&db, &owner, &row.id)
        .await
        .unwrap()
        .active_turn
        .unwrap();
    assert_eq!(after_duplicate.activity_total, 2);
    assert_eq!(after_duplicate.activity_count, Some(2));
    assert_eq!(after_duplicate.activities[1].label, "$$ROOT");
    assert_eq!(
        after_duplicate.activities[1].digest.as_deref(),
        Some("$digest")
    );
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
        test_activity_started_with_digest(&db, &user, &first.id, "mail__send", Some("digest-send"))
            .await
            .unwrap()
            .unwrap();
    test_activity_finished(&db, &user, &first.id, &sent, true)
        .await
        .unwrap();
    settle(&db, keys, &user, &failed, "partial", Some("idle_timeout")).await;
    let failed_reply = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "conversation_id": &first.id, "turn_id": &failed_turn, "role": "assistant",
        })
        .await
        .unwrap()
        .unwrap();
    let preview = recovery_for_message(&db, &user, &first.id, &failed_reply.id)
        .await
        .unwrap();
    assert_eq!(preview.status, "available");
    assert_eq!(preview.text.as_deref(), Some("send the brief"));
    assert_eq!(preview.attachments[0].id, upload.id);
    let row = get(&db, &user, &first.id).await.unwrap();
    let page = recovery_previews(
        &db,
        &user,
        &first.id,
        &row,
        std::slice::from_ref(&failed_reply),
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        page.get(&failed_reply.id).unwrap().text.as_deref(),
        Some("send the brief")
    );
    assert!(
        recovery_for_message(&db, "another-owner", &first.id, &failed_reply.id)
            .await
            .is_err()
    );
    let busy = begin_turn(&db, &user, &request(Some(&first.id), "other work"), keys)
        .await
        .unwrap();
    assert_eq!(
        recovery_for_message(&db, &user, &first.id, &failed_reply.id)
            .await
            .unwrap()
            .reason,
        "turn_active"
    );
    settle(&db, keys, &user, &busy, "done", None).await;
    assert_eq!(
        recovery_for_message(&db, &user, &first.id, &failed_reply.id)
            .await
            .unwrap()
            .status,
        "available"
    );

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
            "[NyxID retry context: the owner is retrying turn {failed_turn}, which failed (idle_timeout). These actions already completed there; use their results. NyxID refuses repeating any of them with the same arguments: mail__send. The original message's attachments are attached again.]"
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
        retry_duplicate(
            &db,
            &user,
            &first.id,
            &get(&db, &user, &first.id)
                .await
                .unwrap()
                .active_turn
                .unwrap()
                .turn_id,
            "digest-send"
        )
        .await
        .unwrap()
    );
    assert!(
        !retry_duplicate(
            &db,
            &user,
            &first.id,
            &get(&db, &user, &first.id)
                .await
                .unwrap()
                .active_turn
                .unwrap()
                .turn_id,
            "digest-other"
        )
        .await
        .unwrap()
    );
    // A retry of this retry inherits the first turn's fence even when no
    // duplicate call was attempted in the intermediate turn.
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
    let consumed = recovery_for_message(&db, &user, &first.id, &failed_reply.id)
        .await
        .unwrap();
    assert_eq!(consumed.reason, "already_retried");
    let old_source = begin_turn(
        &db,
        &user,
        &retry(&first.id, "send the brief", &failed_turn),
        keys,
    )
    .await
    .unwrap_err();
    assert!(matches!(old_source, AppError::Conflict(_)));
    let reply = messages(&db, &user, &first.id, 2, None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(reply.retry_of.as_deref(), Some(failed_turn.as_str()));
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
        retry_duplicate(
            &db,
            &user,
            &first.id,
            &get(&db, &user, &first.id)
                .await
                .unwrap()
                .active_turn
                .unwrap()
                .turn_id,
            "digest-send"
        )
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
        !retry_duplicate(
            &db,
            &user,
            &first.id,
            &get(&db, &user, &first.id)
                .await
                .unwrap()
                .active_turn
                .unwrap()
                .turn_id,
            "digest-send"
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn retry_rejects_unknown_and_incomplete_effect_evidence() {
    let db = connect_transaction_test_database("nyxa_retry_uncertain").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let keys = &state.encryption_keys;
    let first = begin_turn(&db, &user, &request(None, "first"), keys)
        .await
        .unwrap();
    settle(&db, keys, &user, &first, "done", None).await;

    let unknown = begin_turn(&db, &user, &request(Some(&first.id), "unknown"), keys)
        .await
        .unwrap();
    let unknown_id = unknown.active_turn.as_ref().unwrap().turn_id.clone();
    settle(&db, keys, &user, &unknown, "", Some("outcome_unknown")).await;
    let unknown_reply = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "conversation_id": &first.id, "turn_id": &unknown_id, "role": "assistant",
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recovery_for_message(&db, &user, &first.id, &unknown_reply.id)
            .await
            .unwrap()
            .reason,
        "activity_outcome_uncertain"
    );
    let unknown_input = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "conversation_id": &first.id, "turn_id": &unknown_id, "role": "user",
        })
        .await
        .unwrap()
        .unwrap();
    let mut long_head = RetryEvidence::from(&unknown_reply);
    long_head.retry_of = Some("ancestor-0".into());
    let mut ancestors = std::collections::HashMap::new();
    for index in 0..32 {
        let mut ancestor = RetryEvidence::from(&unknown_reply);
        ancestor.turn_id = format!("ancestor-{index}");
        ancestor.retry_of = Some(format!("ancestor-{}", index + 1));
        ancestors.insert(ancestor.turn_id.clone(), ancestor);
    }
    assert!(matches!(
        retry_evidence(&long_head, Some(&unknown_input), &ancestors),
        Err("lineage_incomplete")
    ));
    let result = begin_turn(&db, &user, &retry(&first.id, "unknown", &unknown_id), keys).await;
    assert!(
        matches!(result, Err(AppError::Conflict(_))),
        "unknown outcome must need review"
    );

    let incomplete = begin_turn(&db, &user, &request(Some(&first.id), "incomplete"), keys)
        .await
        .unwrap();
    let incomplete_id = incomplete.active_turn.as_ref().unwrap().turn_id.clone();
    let activity =
        test_activity_started_with_digest(&db, &user, &first.id, "$$ROOT", Some("$digest"))
            .await
            .unwrap()
            .unwrap();
    test_activity_finished(&db, &user, &first.id, &activity, true)
        .await
        .unwrap();
    settle(&db, keys, &user, &incomplete, "", Some("idle_timeout")).await;
    let reply = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "conversation_id": &first.id, "turn_id": &incomplete_id, "role": "assistant",
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.activities[0].label, "$$ROOT");
    assert_eq!(reply.activities[0].digest.as_deref(), Some("$digest"));
    let accepted = begin_turn(
        &db,
        &user,
        &retry(&first.id, "incomplete", &incomplete_id),
        keys,
    )
    .await
    .unwrap();
    assert_eq!(
        accepted.active_turn.as_ref().unwrap().retry_digests,
        ["$digest"]
    );
    settle(&db, keys, &user, &accepted, "done", None).await;

    let truncated = begin_turn(&db, &user, &request(Some(&first.id), "truncated"), keys)
        .await
        .unwrap();
    let truncated_id = truncated.active_turn.as_ref().unwrap().turn_id.clone();
    let activity = test_activity_started_with_digest(&db, &user, &first.id, "call", Some("digest"))
        .await
        .unwrap()
        .unwrap();
    test_activity_finished(&db, &user, &first.id, &activity, true)
        .await
        .unwrap();
    settle(&db, keys, &user, &truncated, "", Some("idle_timeout")).await;
    db.collection::<bson::Document>(MESSAGES)
        .update_one(
            doc! {
                "conversation_id": &first.id, "turn_id": &truncated_id, "role": "assistant",
            },
            doc! {"$set": {"activity_count": 2}},
        )
        .await
        .unwrap();
    let result = begin_turn(
        &db,
        &user,
        &retry(&first.id, "truncated", &truncated_id),
        keys,
    )
    .await;
    assert!(
        matches!(result, Err(AppError::Conflict(_))),
        "truncated history must need review"
    );
    let replies = db.collection::<bson::Document>(MESSAGES);
    let filter = doc! {"conversation_id": &first.id, "turn_id": &truncated_id, "role": "assistant"};
    replies
        .update_one(
            filter.clone(),
            doc! {"$set": {"activity_count": bson::Bson::Null}},
        )
        .await
        .unwrap();
    assert!(
        matches!(
            begin_turn(
                &db,
                &user,
                &retry(&first.id, "truncated", &truncated_id),
                keys
            )
            .await,
            Err(AppError::Conflict(_))
        ),
        "legacy unknown count must need review"
    );
    replies
        .update_one(
            filter.clone(),
            doc! {"$set": {"activity_count": 1, "activities.0.status": "error"}},
        )
        .await
        .unwrap();
    assert!(
        matches!(
            begin_turn(
                &db,
                &user,
                &retry(&first.id, "truncated", &truncated_id),
                keys
            )
            .await,
            Err(AppError::Conflict(_))
        ),
        "failed activity may have taken effect"
    );
    replies
        .update_one(
            filter.clone(),
            doc! {"$set": {"activities.0.status": "completed",
            "activities.0.digest": bson::Bson::Null}},
        )
        .await
        .unwrap();
    assert!(
        matches!(
            begin_turn(
                &db,
                &user,
                &retry(&first.id, "truncated", &truncated_id),
                keys
            )
            .await,
            Err(AppError::Conflict(_))
        ),
        "digest-less action cannot be fenced"
    );
    replies
        .update_one(
            filter,
            doc! {"$set": {"activities.0.digest": "digest",
            "retry_of": "missing-ancestor"}},
        )
        .await
        .unwrap();
    assert!(
        matches!(
            begin_turn(
                &db,
                &user,
                &retry(&first.id, "truncated", &truncated_id),
                keys
            )
            .await,
            Err(AppError::Conflict(_))
        ),
        "missing ancestry must need review"
    );
    replies
        .update_one(
            doc! {"conversation_id": &first.id, "turn_id": &truncated_id,
            "role": "assistant"},
            doc! {"$set": {"retry_of": &truncated_id}},
        )
        .await
        .unwrap();
    assert!(
        matches!(
            begin_turn(
                &db,
                &user,
                &retry(&first.id, "truncated", &truncated_id),
                keys
            )
            .await,
            Err(AppError::Conflict(_))
        ),
        "cyclic ancestry must need review"
    );

    let capped = begin_turn(&db, &user, &request(Some(&first.id), "many calls"), keys)
        .await
        .unwrap();
    let capped_id = capped.active_turn.as_ref().unwrap().turn_id.clone();
    for index in 0..=MAX_TURN_ACTIVITIES {
        let activity = test_activity_started_with_digest(
            &db,
            &user,
            &first.id,
            "read",
            Some(&format!("digest-{index}")),
        )
        .await
        .unwrap()
        .unwrap();
        test_activity_finished(&db, &user, &first.id, &activity, true)
            .await
            .unwrap();
    }
    settle(&db, keys, &user, &capped, "", Some("idle_timeout")).await;
    let reply = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "conversation_id": &first.id, "turn_id": &capped_id, "role": "assistant",
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.activity_count, Some(MAX_TURN_ACTIVITIES + 1));
    assert_eq!(reply.activities.len() as i64, MAX_TURN_ACTIVITIES);
    assert!(
        matches!(
            begin_turn(
                &db,
                &user,
                &retry(&first.id, "many calls", &capped_id),
                keys
            )
            .await,
            Err(AppError::Conflict(_))
        ),
        "capped activity history must need review"
    );
    db.collection::<bson::Document>(MESSAGES)
        .update_one(
            doc! {
                "conversation_id": &first.id, "turn_id": &capped_id, "role": "assistant",
            },
            doc! {"$set": {"activity_count": MAX_TURN_ACTIVITIES}},
        )
        .await
        .unwrap();
    assert!(
        matches!(
            begin_turn(
                &db,
                &user,
                &retry(&first.id, "many calls", &capped_id),
                keys
            )
            .await,
            Err(AppError::Conflict(_))
        ),
        "a full retained list is ambiguous during rollout"
    );
}

#[tokio::test]
async fn guest_failure_cannot_be_retried_as_owner_turn() {
    let db = connect_transaction_test_database("nyxa_retry_guest").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let keys = &state.encryption_keys;
    for origin in [TurnOrigin::User, TurnOrigin::Channel] {
        let mut start: TurnStart = (&request(None, "guest request")).into();
        start.guest = true;
        start.origin = origin;
        let guest = begin_turn(&db, &user, start, keys).await.unwrap();
        let guest_turn = guest.active_turn.as_ref().unwrap().turn_id.clone();
        settle(&db, keys, &user, &guest, "", Some("idle_timeout")).await;
        let history = messages(&db, &user, &guest.id, 20, None).await.unwrap();
        let input = history
            .iter()
            .find(|message| message.role == "user")
            .unwrap();
        let reply = history
            .iter()
            .find(|message| message.role == "assistant")
            .unwrap();
        assert_eq!(input.origin, Some(origin));
        assert_eq!(reply.origin, Some(origin));
        assert_eq!(
            input.author,
            Some(crate::models::assistant_message::MessageAuthor::Guest)
        );
        let preview = recovery_for_message(&db, &user, &guest.id, &reply.id)
            .await
            .unwrap();
        assert_eq!(preview.status, "review");
        let result = begin_turn(
            &db,
            &user,
            &retry(&guest.id, "guest request", &guest_turn),
            keys,
        )
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}

#[tokio::test]
async fn retry_rejects_missing_original_upload_and_concurrent_claim() {
    let db = connect_transaction_test_database("nyxa_retry_claim").await;
    ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let keys = &state.encryption_keys;
    let first = begin_turn(&db, &user, &request(None, "first"), keys)
        .await
        .unwrap();
    settle(&db, keys, &user, &first, "done", None).await;
    let upload = crate::services::assistant_upload_service::upload(
        &db,
        keys,
        &user,
        &first.id,
        "scan.txt",
        b"scan".to_vec(),
    )
    .await
    .unwrap();
    let failed = begin_turn(
        &db,
        &user,
        &TurnRequest {
            attachment_ids: vec![upload.id.clone()],
            ..request(Some(&first.id), "inspect")
        },
        keys,
    )
    .await
    .unwrap();
    let turn_id = failed.active_turn.as_ref().unwrap().turn_id.clone();
    settle(&db, keys, &user, &failed, "", Some("idle_timeout")).await;
    let attachments = db.collection::<bson::Document>(ATTACHMENTS);
    attachments
        .update_one(
            doc! {"_id": &upload.id},
            doc! {"$set": {
                "bound_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::days(400)),
            }},
        )
        .await
        .unwrap();
    let expired = begin_turn(&db, &user, &retry(&first.id, "inspect", &turn_id), keys).await;
    assert!(matches!(expired, Err(AppError::AssistantAttachmentExpired)));
    attachments
        .update_one(
            doc! {"_id": &upload.id},
            doc! {"$set": {
                "bound_at": bson::DateTime::now(),
            }},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(ATTACHMENTS)
        .delete_one(doc! {"_id": &upload.id})
        .await
        .unwrap();
    let failed_reply = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "conversation_id": &first.id, "turn_id": &turn_id, "role": "assistant",
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recovery_for_message(&db, &user, &first.id, &failed_reply.id)
            .await
            .unwrap()
            .reason,
        "attachment_expired"
    );
    let result = begin_turn(&db, &user, &retry(&first.id, "inspect", &turn_id), keys).await;
    assert!(matches!(result, Err(AppError::AssistantAttachmentExpired)));

    let retryable = begin_turn(&db, &user, &request(Some(&first.id), "try again"), keys)
        .await
        .unwrap();
    let retryable_id = retryable.active_turn.as_ref().unwrap().turn_id.clone();
    settle(&db, keys, &user, &retryable, "", Some("idle_timeout")).await;
    let request = retry(&first.id, "try again", &retryable_id);
    let (a, b) = tokio::join!(
        begin_turn(&db, &user, &request, keys),
        begin_turn(&db, &user, &request, keys),
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
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
    let call = test_activity_started_with_digest(&db, &user, &id, "mail__send", None)
        .await
        .unwrap()
        .unwrap();
    test_activity_finished(&db, &user, &id, &call, true)
        .await
        .unwrap();
    test_activity_started_with_digest(&db, &user, &id, "calendar__create", None)
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
    let call = test_activity_started_with_digest(&db, &user, &id, "slack__post", None)
        .await
        .unwrap()
        .unwrap();
    test_activity_finished(&db, &user, &id, &call, true)
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
    let mut context = crate::services::assistant_instruction_context::Prepared::new(
        &[2; 32],
        &summary.conversation,
        None,
        &[],
    )
    .unwrap();
    context.set_checkpoint(summary.checkpoint.clone());
    let rendered = render_recap(&[], &[], &[], 0, 0, false);
    let recap = context.instructions_with_recap(&rendered);
    assert!(recap.len() <= context.instructions().len() + 16_384);
    assert!(recap.contains("End checkpoint."));
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
    let mut guest_row = summary.conversation.clone();
    guest_row.guest_turn = true;
    let mut guest_context = crate::services::assistant_instruction_context::Prepared::new(
        &[2; 32],
        &guest_row,
        None,
        &[],
    )
    .unwrap();
    guest_context.set_checkpoint(summary.checkpoint.clone());
    let guest = guest_context.instructions_with_recap(&render_recap(&[], &[], &[], 0, 0, true));
    assert!(!guest.contains("mail__send"));
    assert!(!guest.contains("Checkpoint (turn"));

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
        platform_delivery_recovery_queued: false,
        platform_reply: None,
        platform_origin: None,
        author: None,
        activity_total: 0,
        report_to: None,
        continuation_of: None,
        retry_of: None,
        retry_claimed_by: None,
        activity_count: None,
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
        turn_metadata: None,
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
        attach_routed_image(
            &db,
            keys,
            &owner,
            "nyxa-00000000000000000000000000000000",
            "t",
            "image/png",
            &png,
            true
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
        attach_routed_image(
            &db,
            keys,
            &owner,
            &row.id,
            &format!("cam__snapshot_{i}"),
            "image/png",
            &png,
            true,
        )
        .await
        .unwrap()
        .unwrap();
    }
    assert!(
        attach_routed_image(
            &db,
            keys,
            &owner,
            &row.id,
            "cam__overflow",
            "image/png",
            &png,
            true
        )
        .await
        .unwrap()
        .is_none(),
        "per-turn limit"
    );
    assert!(
        attach_routed_image(
            &db,
            keys,
            "someone-else",
            &row.id,
            "cam",
            "image/png",
            &png,
            true
        )
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
        attach_routed_image(&db, keys, &owner, &row.id, "late", "image/png", &png, true)
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
    assert!(!prepared.instructions().contains("Prefers metric units"));
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
    row.active_turn.as_mut().unwrap().origin = TurnOrigin::Event;
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

#[test]
fn secret_prefix_suffix_withholds_only_live_prefixes() {
    assert_eq!(
        secret_prefix_suffix("普通の文章", &["secret", "marker"]),
        ""
    );
    assert_eq!(secret_prefix_suffix("hello sec", &["secret"]), "sec");
    assert_eq!(secret_prefix_suffix("hello secret", &["secret"]), "");
    assert_eq!(secret_prefix_suffix("你好 mar", &["marker"]), "mar");
    assert_eq!(secret_prefix_suffix("x", &["x"]), "");
    assert_eq!(secret_prefix_suffix("你好 m", &["marker"]), "m");
    assert_eq!(secret_prefix_suffix("hello", &["", "secret"]), "");
}

#[test]
fn relay_hold_split_secrets_and_continuation_boundaries() {
    let secrets = ["nyxid_ag_test_secret", "context-marker-secret"];
    for secret in secrets {
        for cuts in [vec![3], vec![3, 9]] {
            let mut raw = String::new();
            let mut emitted = 0;
            let mut stream = String::new();
            let mut start = 0;
            for end in cuts.into_iter().chain(std::iter::once(secret.len())) {
                raw.push_str(&secret[start..end]);
                let redacted = raw.replace(secret, "[redacted]");
                stream.push_str(&relay_step(&redacted, &mut emitted, &secrets, false));
                assert!(!stream.contains(secret));
                start = end;
            }
            assert_eq!(stream, "[redacted]");
        }
        for reason in ["tool_budget_exhausted", "turn_timeout"] {
            let mut emitted = 0;
            let prefix = &secret[..9];
            let mut stream = relay_step(prefix, &mut emitted, &secrets, false);
            stream.push_str(&relay_step(prefix, &mut emitted, &secrets, true));
            let mut next_emitted = 0;
            stream.push_str(&relay_step(
                &secret[9..],
                &mut next_emitted,
                &secrets,
                false,
            ));
            assert_eq!(stream, format!("{prefix}\n\n{}", &secret[9..]), "{reason}");
            assert!(!stream.contains(secret));
        }
    }
    let mut emitted = 0;
    assert_eq!(
        relay_step("你好，继续。", &mut emitted, &secrets, false),
        "你好，继续。"
    );
    emitted = 0;
    assert_eq!(relay_step("nyxid_ag_", &mut emitted, &secrets, false), "");
    assert_eq!(
        relay_step("nyxid_ag_near-miss", &mut emitted, &secrets, false),
        "nyxid_ag_near-miss"
    );
    let long = format!("{}n", "界".repeat(100_000));
    assert_eq!(secret_prefix_suffix(&long, &secrets), "n");
}

#[test]
fn production_recap_preserves_owner_and_guest_provenance_in_recent_tail() {
    let owner = recap_owner_message(1, "Owner requirement".into());
    let mut guest = recap_owner_message(2, "Guest request".into());
    guest.author = Some(crate::models::assistant_message::MessageAuthor::Guest);
    let text = render_recap_with_owner_total(&[owner, guest], &[], &[], 2, 0, false, 1);
    assert!(text.contains("\nuser: Owner requirement"));
    assert!(text.contains("\nuser (guest): Guest request"));
    assert!(!text.contains("\nuser: Guest request"));
}

fn recap_owner_message(seq: i64, text: String) -> AssistantMessage {
    bson::from_document(doc! {
        "_id": format!("recap-owner-{seq}"), "conversation_id": "c", "user_id": "u",
        "seq": seq, "turn_id": "t", "role": "user", "text": text,
        "status": "completed", "author": "owner", "origin": "user",
        "created_at": bson::DateTime::now(),
    })
    .unwrap()
}

#[test]
fn production_recap_reports_tail_truncation_without_sniffing_message_text() {
    let long = recap_owner_message(1, format!("First rule {} Final rule", "x".repeat(6000)));
    let text = render_recap_with_owner_total(&[long], &[], &[], 1, 0, false, 1);
    assert!(text.contains("First rule"));
    assert!(text.contains(" … "));
    assert!(text.contains(
        "0 earlier owner messages / 1 truncated; read them with nyx__conversation_history"
    ));

    let short = recap_owner_message(
        1,
        "A literal [truncated] marker is part of the instruction".into(),
    );
    let text = render_recap_with_owner_total(&[short], &[], &[], 1, 0, false, 1);
    assert!(!text.contains("nyx__conversation_history"));
}

#[test]
fn production_recap_stops_tail_when_a_newer_prefix_cannot_fit() {
    let older = recap_owner_message(1, "older".into());
    let mut failed = recap_owner_message(2, "Failed reply".into());
    failed.role = "assistant".into();
    failed.status = "failed".into();
    failed.author = None;
    failed.origin = None;

    let overhead = recap_line(&recap_owner_message(3, String::new()), false, 1000)
        .unwrap()
        .text
        .len();
    let mut newest = vec![recap_owner_message(3, "x".repeat(880 - overhead))];
    newest.extend((4..8).map(|seq| recap_owner_message(seq, "x".repeat(2000))));
    let used: usize = newest
        .iter()
        .map(|message| recap_line(message, false, 1000).unwrap().text.len())
        .sum();
    let remaining = 4900 - used;
    assert_eq!(remaining, 20);
    assert!(recap_line(&failed, false, remaining).is_none());
    assert!(recap_line(&older, false, remaining).is_some());

    let mut owners = vec![older.clone()];
    owners.extend(newest.clone());
    let mut history = vec![older, failed];
    history.extend(newest);
    let text = render_recap_with_owner_total(&history, &owners, &[], 7, 0, false, 6);
    let (requirements, recent) = text.split_once("Recent conversation:").unwrap();
    assert!(requirements.contains("user: older"));
    assert!(!recent.contains("user: older"));
    assert!(!recent.contains("Failed reply"));
    assert!(recent.contains("[2 earlier messages omitted]"));

    let text = render_recap_with_owner_total(&history, &[], &[], 7, 0, false, 6);
    assert!(text.contains(
        "1 earlier owner messages / 4 truncated; read them with nyx__conversation_history"
    ));
}

#[test]
fn production_recap_keeps_unrendered_tail_owners_in_requirements_and_omission_count() {
    let owners: Vec<_> = (0..20)
        .map(|seq| recap_owner_message(seq, format!("rule-{seq}:{}", "x".repeat(2000))))
        .collect();
    let text = render_recap_with_owner_total(&owners, &owners, &[], 20, 0, false, 20);
    let requirements = text.split("Recent conversation:").next().unwrap();
    assert!(requirements.contains("Owner requirements of record:"));
    assert!(requirements.contains("rule-0:"));
    assert!(requirements.contains("rule-3:"));
    assert!(text.contains(
        "11 earlier owner messages / 9 truncated; read them with nyx__conversation_history"
    ));
    assert!(text.contains("[15 earlier messages omitted]"));
    assert!(text.len() < 12 * 1024);

    let text = render_recap_with_owner_total(&owners, &[], &[], 20, 0, false, 20);
    assert!(text.contains(
        "15 earlier owner messages / 5 truncated; read them with nyx__conversation_history"
    ));
}

#[test]
fn recap_sections_keep_mid_history_owner_rules_tail_and_manifest_with_cjk() {
    let message = |i, text: String| AssistantMessage {
        turn_metadata: None,
        platform_delivery_recovery_queued: false,
        platform_reply: None,
        platform_origin: None,
        retry_claimed_by: None,
        activity_count: None,
        report_to: None,
        continuation_of: None,
        specialist_id: None,
        retry_of: None,
        upstream_error_code: None,
        error_detail: None,
        author: Some(crate::models::assistant_message::MessageAuthor::Owner),
        steering: None,
        voice: None,
        execution_pending: false,
        id: format!("m{i}"),
        conversation_id: "c".into(),
        user_id: "u".into(),
        seq: i,
        turn_id: "t".into(),
        role: "user".into(),
        text,
        status: "completed".into(),
        error_code: None,
        created_at: Utc::now(),
        activities: vec![],
        activity_total: 0,
        attachments: vec![],
        origin: Some(TurnOrigin::User),
        via: None,
    };
    let owners = vec![
        message(0, "Hello".into()),
        message(12, "Mid-history requirement".into()),
    ];
    let history: Vec<_> = (21..=40)
        .map(|i| message(i, format!("tail{i}:{}", "界".repeat(600))))
        .collect();
    let uploads = vec![
        json!({"id":"upload","name":"old source","type":"text/plain","size":12,"pages":1,"expired":true}),
    ];
    let recap = render_recap(&history, &owners, &uploads, 41, 62, false);
    assert!(recap.find("Hello").unwrap() < recap.find("Mid-history").unwrap());
    assert!(recap.contains("tail40"));
    assert!(recap.contains("[expired]"));
    assert!(recap.contains("[61 older uploads"));
    assert!(recap.contains("earlier messages omitted"));
    assert!(recap.len() < 12 * 1024);
    let guest = render_recap(&history, &owners, &uploads, 41, 62, true);
    assert!(!guest.contains("Owner requirements"));
    assert!(!guest.contains("old source"));
}

#[test]
fn recap_owner_overflow_points_to_history_tool() {
    let message = |i: i64| AssistantMessage {
        turn_metadata: None,
        platform_delivery_recovery_queued: false,
        platform_reply: None,
        platform_origin: None,
        retry_claimed_by: None,
        activity_count: None,
        report_to: None,
        continuation_of: None,
        specialist_id: None,
        retry_of: None,
        upstream_error_code: None,
        error_detail: None,
        author: Some(crate::models::assistant_message::MessageAuthor::Owner),
        steering: None,
        voice: None,
        execution_pending: false,
        id: format!("owner-{i}"),
        conversation_id: "c".into(),
        user_id: "u".into(),
        seq: i,
        turn_id: "t".into(),
        role: "user".into(),
        text: format!("rule {i} {}", "x".repeat(500)),
        status: "completed".into(),
        error_code: None,
        created_at: Utc::now(),
        activities: vec![],
        activity_total: 0,
        attachments: vec![],
        origin: Some(TurnOrigin::User),
        via: None,
    };
    let owners: Vec<_> = (0..45).map(message).collect();
    let text = render_recap_with_owner_total(&[], &owners, &[], 45, 0, false, 45);
    assert!(text.contains("earlier owner messages"));
    assert!(text.contains("nyx__conversation_history"));
}

#[tokio::test]
async fn reset_recap_queries_owner_rules_and_expired_manifest_outside_tail() {
    let db = connect_transaction_test_database("nyxa_recap_independent").await;
    let state = test_app_state(db.clone());
    let row = begin_turn(
        &db,
        "owner",
        &request(None, "Hello"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let mut template = messages(&db, "owner", &row.id, 10, None)
        .await
        .unwrap()
        .remove(0);
    template.author = Some(crate::models::assistant_message::MessageAuthor::Owner);
    template.turn_id = "earlier".into();
    for seq in 1..65 {
        let mut message = template.clone();
        message.id = Uuid::new_v4().to_string();
        message.seq = seq;
        message.text = match seq {
            1 => "Greeting of record".into(),
            30 => "Mid-history owner rule: preserve source detail".into(),
            _ => format!("Owner line {seq}"),
        };
        if seq == 2 {
            let upload_id = Uuid::new_v4().to_string();
            message
                .attachments
                .push(crate::models::assistant_conversation::TurnAttachment {
                    chat_delivery: None,
                    id: upload_id.clone(),
                    label: "Old source.txt".into(),
                    content_type: "text/plain".into(),
                    size: 123,
                    pages: Some(1),
                    origin: "user_upload".into(),
                    image_input: None,
                });
            db.collection::<bson::Document>(crate::models::assistant_upload_retention::TOMBSTONES)
                .insert_one(doc! {"_id":upload_id,"user_id":"owner","conversation_id":&row.id})
                .await
                .unwrap();
        }
        db.collection::<AssistantMessage>(MESSAGES)
            .insert_one(message)
            .await
            .unwrap();
    }
    let history = messages(&db, "owner", &row.id, 20, None).await.unwrap();
    assert!(!history.iter().any(|message| message.seq == 30));
    let text = recap(
        &db,
        &row,
        &history,
        &row.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap();
    assert!(text.contains("Mid-history owner rule"));
    assert!(
        text.find("Greeting of record").unwrap() < text.find("Mid-history owner rule").unwrap()
    );
    assert!(text.contains("Old source.txt") && text.contains("[expired]"));
    assert!(text.contains("earlier messages omitted"));
    let mut guest = row.clone();
    guest.guest_turn = true;
    let text = recap(
        &db,
        &guest,
        &history,
        &row.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap();
    assert!(!text.contains("Mid-history owner rule") && !text.contains("Old source.txt"));
    db.drop().await.unwrap();
}

#[test]
fn relay_step_never_moves_the_emitted_boundary_backwards() {
    let mut emitted = 3;
    assert_eq!(relay_step("secr", &mut emitted, &["secret"], false), "");
    assert_eq!(emitted, 3);
}

#[test]
fn recap_omits_unknown_or_zero_page_counts() {
    let uploads = [
        json!({"id":"unknown","name":"unknown.txt","size":3,"type":"text/plain","pages":null}),
        json!({"id":"zero","name":"zero.txt","size":3,"type":"text/plain","pages":0}),
        json!({"id":"known","name":"known.pdf","size":3,"type":"application/pdf","pages":2}),
    ];
    let text = render_recap(&[], &[], &uploads, 0, 3, false);
    assert!(!text.contains("null pages"));
    assert!(!text.contains("0 pages"));
    assert!(text.contains("2 pages"));
}

#[tokio::test]
async fn conversation_history_cursor_reconstructs_owner_messages_and_refuses_invalid_authority() {
    use crate::models::assistant_message::MessageAuthor;
    use crate::services::assistant_acknowledgement_service as acks;

    let db = connect_transaction_test_database("nyxa_history_cursor").await;
    let user = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let row = begin_turn(
        &db,
        &user,
        &request(None, "greeting"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let credential =
        credentials::load_for_conversation(&db, &state.encryption_keys, &user, &row.id)
            .await
            .unwrap()
            .unwrap();
    let chat = acks::for_key(&db, &user, Some(&credential.api_key_id))
        .await
        .unwrap()
        .unwrap();
    let turn_id = row.active_turn.as_ref().unwrap().turn_id.clone();
    let mut template = messages(&db, &user, &row.id, 1, None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    // Seed completed historical turns while the credential remains bound to
    // this tab's live turn; the recap excludes only that live turn's messages.
    template.turn_id = Uuid::new_v4().to_string();
    db.collection::<bson::Document>(MESSAGES)
        .update_one(
            doc! {"_id": &template.id},
            doc! {"$set": {"turn_id": &template.turn_id}},
        )
        .await
        .unwrap();
    let expected = vec![
        "greeting".to_owned(),
        "message two contains the original requirements".to_owned(),
        "later owner note".to_owned(),
        "split ✨ text with quotes \\\" and slashes \\\\".repeat(800),
    ];
    for (index, text) in expected.iter().enumerate().skip(1) {
        let mut message = template.clone();
        message.id = Uuid::new_v4().to_string();
        message.seq = index as i64 + 1;
        message.turn_id = template.turn_id.clone();
        message.author = Some(MessageAuthor::Owner);
        message.origin = Some(TurnOrigin::User);
        message.text = text.clone();
        db.collection(MESSAGES).insert_one(message).await.unwrap();
    }

    let mut before = None;
    let mut offset = 0;
    let mut seen: Vec<(i64, String)> = Vec::new();
    let mut complete = false;
    let mut page_count = 0;
    for _ in 0..200 {
        let page = conversation_history_page(&db, &chat, before, offset, 20)
            .await
            .unwrap();
        page_count += 1;
        assert!(mcp_history_envelope_size(&page) < 9000);
        for message in page["messages"].as_array().unwrap() {
            let seq = message["seq"].as_i64().unwrap();
            let fragment = message["text"].as_str().unwrap();
            if seen.last().is_some_and(|(previous, _)| *previous == seq) {
                let (_, text) = seen.last_mut().unwrap();
                assert_eq!(message["offset"], text.chars().count());
                text.push_str(fragment);
            } else {
                assert!(seen.last().is_none_or(|(previous, _)| *previous > seq));
                assert_eq!(message["offset"], 0);
                seen.push((seq, fragment.to_owned()));
            }
        }
        if page["complete"].as_bool().unwrap() {
            complete = true;
            break;
        }
        let next = (
            page["before_seq"].as_i64(),
            page["offset"].as_u64().unwrap_or(0) as usize,
        );
        assert_ne!(next, (before, offset), "cursor must advance");
        (before, offset) = next;
    }
    assert!(complete && page_count > 1);
    assert_eq!(seen.len(), expected.len());
    assert_eq!(
        seen.into_iter()
            .rev()
            .map(|(_, text)| text)
            .collect::<Vec<_>>(),
        expected
    );

    let mut guest = chat.clone();
    guest.guest = true;
    assert!(matches!(
        conversation_history_page(&db, &guest, None, 0, 20).await,
        Err(AppError::AssistantTurnRequired)
    ));
    let mut stale = chat.clone();
    stale.turn_stopped = true;
    assert!(matches!(
        conversation_history_page(&db, &stale, None, 0, 20).await,
        Err(AppError::AssistantTurnRequired)
    ));
    let mut wrong_key = chat.clone();
    wrong_key.api_key_id = Uuid::new_v4().to_string();
    assert!(matches!(
        conversation_history_page(&db, &wrong_key, None, 0, 20).await,
        Err(AppError::AssistantTurnRequired)
    ));
    let mut other_chat = chat.clone();
    other_chat.conversation_id = format!("nyxa-{}", Uuid::new_v4().simple());
    assert!(matches!(
        conversation_history_page(&db, &other_chat, None, 0, 20).await,
        Err(AppError::NotFound(_))
    ));
    let other_thread = begin_turn(
        &db,
        &user,
        &request(None, "Private rules in another conversation"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    other_chat.conversation_id = other_thread.id;
    assert!(matches!(
        conversation_history_page(&db, &other_chat, None, 0, 20).await,
        Err(AppError::AssistantTurnRequired)
    ));

    // A greeting-first thread with many later owner turns must still expose
    // the second message, and a single envelope-sized message must be
    // reconstructible through the explicit offset cursor.
    for seq in 10..55 {
        let mut message = template.clone();
        message.id = Uuid::new_v4().to_string();
        message.seq = seq;
        message.turn_id = template.turn_id.clone();
        message.author = Some(MessageAuthor::Owner);
        message.origin = Some(TurnOrigin::User);
        message.text = format!("later-{seq}");
        db.collection(MESSAGES).insert_one(message).await.unwrap();
    }
    let tail = messages(&db, &user, &row.id, 20, None).await.unwrap();
    let recap_text = recap(&db, &row, &tail, &turn_id).await.unwrap();
    assert!(recap_text.contains("earlier owner messages"));
    assert!(recap_text.contains("nyx__conversation_history"));
    assert!(!recap_text.contains("message two contains the original requirements"));
    let mut before = None;
    let mut offset = 0;
    let mut found_requirement = false;
    for _ in 0..200 {
        let page = conversation_history_page(&db, &chat, before, offset, 20)
            .await
            .unwrap();
        assert!(mcp_history_envelope_size(&page) < 9000);
        found_requirement |= page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["text"] == "message two contains the original requirements");
        if page["complete"].as_bool().unwrap() {
            break;
        }
        before = page["before_seq"].as_i64();
        offset = page["offset"].as_u64().unwrap_or(0) as usize;
    }
    assert!(found_requirement);

    let long_text = format!(
        "{}Key rule beyond the recap truncation point.",
        "\"\\\n\t\u{0001}界".repeat(4_000)
    );
    let mut long = template;
    long.id = Uuid::new_v4().to_string();
    long.seq = 100;

    long.author = Some(MessageAuthor::Owner);
    long.origin = Some(TurnOrigin::User);
    long.text = long_text.clone();
    db.collection(MESSAGES).insert_one(long).await.unwrap();
    let mut before = Some(101);
    let mut offset = 0;
    let mut reconstructed = String::new();
    for _ in 0..200 {
        let page = conversation_history_page(&db, &chat, before, offset, 20)
            .await
            .unwrap();
        assert!(mcp_history_envelope_size(&page) < 9000);
        for message in page["messages"].as_array().unwrap() {
            if message["seq"] == 100 {
                reconstructed.push_str(message["text"].as_str().unwrap());
            }
        }
        if page["complete"].as_bool().unwrap() {
            break;
        }
        before = page["before_seq"].as_i64();
        offset = page["offset"].as_u64().unwrap_or(0) as usize;
    }
    assert_eq!(reconstructed, long_text);

    let mut stale_id = chat.clone();
    stale_id.turn_id = Some(Uuid::new_v4().to_string());
    assert!(matches!(
        conversation_history_page(&db, &stale_id, None, 0, 20).await,
        Err(AppError::AssistantTurnRequired)
    ));
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"active_turn.stop_requested": true}},
        )
        .await
        .unwrap();
    assert!(matches!(
        conversation_history_page(&db, &chat, None, 0, 20).await,
        Err(AppError::AssistantTurnRequired)
    ));
    db.collection::<bson::Document>(CONVERSATIONS).update_one(
        doc! {"_id": &row.id}, doc! {"$set": {"active_turn.stop_requested": false, "active_turn.lease_expires_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}},
    ).await.unwrap();
    assert!(matches!(
        conversation_history_page(&db, &chat, None, 0, 20).await,
        Err(AppError::AssistantTurnRequired)
    ));
    db.drop().await.unwrap();
}

#[test]
fn answer_routing_matches_image_and_settlement_destinations_in_768_rows() {
    use crate::handlers::mcp_transport::{ImageNoteDestination as Note, image_destination};
    use crate::models::channel_thread_follow::ThreadTurnBinding;
    fn origin(partition: &str, bound: bool) -> ChannelOrigin {
        ChannelOrigin {
            nyxbot_channel_id: "channel".into(),
            platform: "telegram-new".into(),
            partition: partition.into(),
            thread: bound.then(|| {
                Box::new(ThreadTurnBinding {
                    child_id: partition.into(),
                    source_message_id: "source".into(),
                    sender_id: "sender".into(),
                    guest: false,
                    revision: 1,
                    generation: 1,
                    channel_generation: 1,
                    agent_id: "agent".into(),
                    queued: true,
                })
            }),
        }
    }
    // Independent oracle for the destination that settlement must use.
    fn expected_destination(
        row: &AssistantConversation,
        turn: &ActiveTurn,
    ) -> (PrimaryDestination, Note) {
        if let Some(group_id) = &row.group_id {
            return (
                PrimaryDestination::Group {
                    group_id: group_id.clone(),
                },
                Note::Group {
                    group_id: group_id.clone(),
                },
            );
        }
        if let Some(run_id) = &turn.trigger_run_id {
            return (
                PrimaryDestination::Trigger {
                    run_id: run_id.clone(),
                },
                Note::Trigger {
                    run_id: run_id.clone(),
                },
            );
        }
        if turn.origin == TurnOrigin::Channel
            && let Some(origin) = &turn.asked_from
        {
            // start_chat_turn and prepare persist this exact admitted origin.
            return (
                PrimaryDestination::ChannelEvent {
                    origin: origin.clone(),
                },
                Note::Chat {
                    platform: origin.platform.clone(),
                    threads: 1,
                },
            );
        }
        if turn.origin == TurnOrigin::Event {
            let mut targets: Vec<ChannelOrigin> = Vec::new();
            for target in turn.events.iter().flat_map(|event| &event.reply_to) {
                if let Some(binding) = &target.thread
                    && !targets.iter().any(|prior| {
                        prior.thread.as_ref().unwrap().source_message_id
                            == binding.source_message_id
                    })
                {
                    targets.push(target.clone());
                }
            }
            if targets.is_empty()
                && let Some(target) = row.channel.as_ref().or(row.reply_channel.as_ref())
            {
                targets.push(target.clone());
            }
            if let Some(first) = targets.first() {
                let note = Note::Chat {
                    platform: first.platform.clone(),
                    threads: targets.len(),
                };
                return (PrimaryDestination::ChatUpdate { origins: targets }, note);
            }
        }
        (PrimaryDestination::App, Note::App)
    }
    let mut count = 0;
    for turn_origin in [
        TurnOrigin::User,
        TurnOrigin::Orchestrator,
        TurnOrigin::Event,
        TurnOrigin::Channel,
        TurnOrigin::Group,
        TurnOrigin::Trigger,
    ] {
        for flags in 0..128 {
            let has = |bit| flags & (1 << bit) != 0;
            let mut row = stale_test_row(Utc::now());
            row.channel = has(0).then(|| origin("channel", false));
            row.reply_channel = has(1).then(|| origin("reply-channel", false));
            row.group_id = has(4).then(|| "group".into());
            let turn = row.active_turn.as_mut().unwrap();
            turn.origin = turn_origin;
            turn.asked_from = has(2).then(|| origin("asked-from", false));
            turn.also_deliver = if has(3) {
                vec![origin("also-1", false), origin("also-2", false)]
            } else {
                vec![]
            };
            turn.trigger_run_id = has(5).then(|| "run".into());
            if has(6) {
                let mut event =
                    crate::services::assistant_team_service::event("message", "wake".into(), None);
                event.reply_to = vec![
                    origin("bound", true),
                    origin("duplicate", true),
                    origin("unbound", false),
                ];
                turn.events = vec![event];
            }
            let turn = row.active_turn.as_ref().unwrap();
            let (expected, note) = expected_destination(&row, turn);
            let routing = answer_routing(&row, turn);
            assert_eq!(
                routing.primary, expected,
                "origin={turn_origin:?}, flags={flags}"
            );
            assert_eq!(routing.also, turn.also_deliver);
            assert_eq!(
                image_destination(&routing),
                note,
                "origin={turn_origin:?}, flags={flags}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 768);
}

#[test]
fn channel_note_follows_the_turn_destination_and_keeps_image_ordering_explicit() {
    let mut row = stale_test_row(Utc::now());
    row.channel = Some(ChannelOrigin {
        nyxbot_channel_id: "channel".into(),
        platform: "lark".into(),
        partition: "chat".into(),
        thread: None,
    });
    for origin in [TurnOrigin::User, TurnOrigin::Orchestrator] {
        row.active_turn.as_mut().unwrap().origin = origin;
        let note = channel_note(&row);
        assert!(note.contains("this reply is shown in the app only"));
        assert!(note.contains("Lark channel bot"));
        assert!(!note.contains("Replies are delivered"));
    }
    for origin in [TurnOrigin::Event, TurnOrigin::Channel] {
        let turn = row.active_turn.as_mut().unwrap();
        turn.origin = origin;
        turn.asked_from = row.channel.clone();
        let note = channel_note(&row);
        assert!(note.contains("Replies are delivered as plain text messages"));
        assert!(note.contains("Images your tools return are sent to this chat by NyxID as image messages before your reply"));
    }
}

#[tokio::test]
async fn tool_image_chat_resolves_x_public_dm_and_trigger_destinations() {
    use crate::models::nyxbot_channel::{
        COLLECTION_NAME as CHANNELS, THREADS_COLLECTION_NAME as THREADS,
    };
    use crate::models::trigger_run::COLLECTION_NAME as RUNS;
    let db = connect_transaction_test_database("image_note_destinations").await;
    let date = bson::DateTime::now();
    db.collection::<bson::Document>(CHANNELS).insert_one(doc! {
        "_id":"channel", "user_id":"owner", "channel_bot_id":"bot", "platform":"x", "bot_label":"bot",
        "transport":"direct", "status":"active", "route_api_key_id":"key", "created_at":date, "updated_at":date,
    }).await.unwrap();
    let origin = ChannelOrigin {
        nyxbot_channel_id: "channel".into(),
        platform: "x".into(),
        partition: "chat".into(),
        thread: None,
    };
    let primary = PrimaryDestination::ChannelEvent { origin };
    assert_eq!(
        image_chat(&db, "owner", &primary).await.unwrap(),
        Some(("x".into(), false))
    );
    db.collection::<bson::Document>(THREADS).insert_one(doc! {
        "_id":"chat", "channel_id":"channel", "user_id":"owner", "partition":"chat", "platform_chat_id":"123-456",
        "created_at":date, "updated_at":date,
    }).await.unwrap();
    assert_eq!(
        image_chat(&db, "owner", &primary).await.unwrap(),
        Some(("x".into(), false))
    );
    db.collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"_id":"chat"},
            doc! {"$set":{"platform_chat_id":"post:123"}},
        )
        .await
        .unwrap();
    assert_eq!(
        image_chat(&db, "owner", &primary).await.unwrap(),
        Some(("x".into(), true))
    );
    db.collection::<bson::Document>(RUNS).insert_one(doc! {
        "_id":"run", "trigger_id":"trigger", "user_id":"owner", "scheduled_at":date, "deadline":date,
        "outcome":"started", "fence":"fence", "lease_until":date, "expires_at":date,
        "deliver_to":{"type":"chat", "chat_id":"chat"},
    }).await.unwrap();
    let primary = PrimaryDestination::Trigger {
        run_id: "run".into(),
    };
    assert_eq!(
        image_chat(&db, "owner", &primary).await.unwrap(),
        Some(("x".into(), true))
    );
    assert_eq!(image_chat(&db, "other", &primary).await.unwrap(), None);
    for target in ["thread", "notification"] {
        db.collection::<bson::Document>(RUNS)
            .update_one(
                doc! {"_id":"run"},
                doc! {"$set":{"deliver_to":{"type":target}}},
            )
            .await
            .unwrap();
        assert_eq!(image_chat(&db, "owner", &primary).await.unwrap(), None);
    }
    db.drop().await.unwrap();
}

#[test]
fn conversation_history_definition_exposes_split_cursor() {
    let definition = conversation_history_definition();
    assert_eq!(
        definition.input_schema["properties"]["offset"]["type"],
        "integer"
    );
    assert!(definition.description.contains("offset"));
}

async fn test_activity_started_with_digest(
    db: &Database,
    user: &str,
    conversation: &str,
    label: &str,
    digest: Option<&str>,
) -> AppResult<Option<String>> {
    let turn = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":conversation, "user_id":user})
        .await?
        .and_then(|row| row.active_turn.map(|turn| turn.turn_id))
        .unwrap_or_default();
    activity_started_with_digest(db, user, conversation, &turn, label, digest).await
}
async fn test_activity_finished(
    db: &Database,
    user: &str,
    conversation: &str,
    activity: &str,
    error: bool,
) -> AppResult<()> {
    let turn = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":conversation, "user_id":user})
        .await?
        .and_then(|row| row.active_turn.map(|turn| turn.turn_id))
        .unwrap_or_default();
    activity_finished_for_turn(db, user, conversation, &turn, activity, error, None).await
}

#[test]
fn channel_tool_image_filter_honors_explicit_policy_and_narrow_legacy_shim() {
    use crate::models::assistant_conversation::TurnAttachment;
    let mut attachment: TurnAttachment = serde_json::from_value(serde_json::json!({
        "id":"image","origin":"tool","content_type":"image/png","size":10,"label":"Machine screenshot"
    })).unwrap();
    assert!(!chat_tool_image(&attachment));
    attachment.chat_delivery = Some(true);
    assert!(
        chat_tool_image(&attachment),
        "explicit metadata supersedes the legacy label shim"
    );
    attachment.label = "Machine screenshot copy".into();
    attachment.chat_delivery = None;
    assert!(
        chat_tool_image(&attachment),
        "only the exact legacy label is excluded"
    );
    attachment.chat_delivery = Some(false);
    assert!(!chat_tool_image(&attachment));
    attachment.chat_delivery = Some(true);
    attachment.origin = "user_upload".into();
    assert!(!chat_tool_image(&attachment));
    attachment.origin = "tool".into();
    attachment.content_type = "application/pdf".into();
    assert!(!chat_tool_image(&attachment));
}

#[tokio::test]
async fn platform_stop_capture_is_exact_transactional_and_keeps_independent_work() {
    use crate::models::machine_job::{COLLECTION_NAME as JOBS, MachineJob};
    use crate::services::{
        platform_manager_contract as contract, platform_personal_turn_service as personal,
    };
    let db = connect_transaction_test_database("nyxa_exact_platform_stop").await;
    let mut row = stale_test_row(Utc::now());
    let proof = platform_test_origin(&row);
    row.platform_thread = Some(proof.thread.clone());
    row.active_turn.as_mut().unwrap().platform_origin = Some(proof.clone());
    row.active_turn.as_mut().unwrap().stop_requested = false;
    row.active_turn.as_mut().unwrap().machine_scopes.push(
        crate::models::assistant_conversation::MachineTurnScope {
            node_id: "node".into(),
            runtime_id: "issue-time-runtime".into(),
        },
    );
    let origin = personal::contract_origin(&proof).unwrap();
    assert_eq!(personal::model_origin(&origin), proof);
    let turn = contract::TurnRef {
        conversation_id: row.id.clone(),
        turn_id: row.active_turn.as_ref().unwrap().turn_id.clone(),
    };
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .insert_one(&row)
        .await
        .unwrap();
    let job = |id: &str, issuing_turn: Option<String>| MachineJob {
        id: id.into(),
        user_id: row.user_id.clone(),
        conversation_id: row.id.clone(),
        issuing_turn_id: issuing_turn,
        node_id: "node".into(),
        runtime_id: "runtime".into(),
        api_key_id: row.credential_api_key_id.clone(),
        agent_id: proof.thread.nyxbot_agent_id.clone(),
        state: "running".into(),
        services: Vec::new(),
        report_to: None,
        report_provenance: 0,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        finished_at: None,
    };
    db.collection::<MachineJob>(JOBS)
        .insert_many([
            job("exact-job", Some(turn.turn_id.clone())),
            job("old-job", Some("older-turn".into())),
            job("legacy-job", None),
        ])
        .await
        .unwrap();
    // Independent watches deliberately remain unchanged, including idle turns.
    db.collection::<bson::Document>(crate::models::async_service_operation::COLLECTION_NAME)
        .insert_one(
            doc! {"_id":"watch","user_id":&row.user_id,"conversation_id":&row.id,"state":"waiting"},
        )
        .await
        .unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let mut wrong = origin.clone();
    wrong.authority.binding_generation += 1;
    assert!(matches!(
        personal::capture_exact_stop_in_session(&db, &mut session, &wrong, &turn)
            .await
            .unwrap(),
        contract::ExactStopOutcome::NoMatchingActiveTurn
    ));
    let captured = personal::capture_exact_stop_in_session(&db, &mut session, &origin, &turn)
        .await
        .unwrap();
    let contract::ExactStopOutcome::Captured(captured) = captured else {
        panic!("matching turn must be captured")
    };
    assert_eq!(captured.machine_scopes.len(), 1);
    assert_eq!(captured.machine_scopes[0].runtime_id, "issue-time-runtime");
    assert_eq!(captured.machine_jobs.len(), 1);
    assert_eq!(captured.machine_jobs[0].job_id, "exact-job");
    session.abort_transaction().await.unwrap();
    assert!(
        !get(&db, &row.user_id, &row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .stop_requested
    );
    session.start_transaction().await.unwrap();
    personal::capture_exact_stop_in_session(&db, &mut session, &origin, &turn)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    assert!(
        get(&db, &row.user_id, &row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .stop_requested
    );
    assert_eq!(
        db.collection::<MachineJob>(JOBS)
            .count_documents(doc! {"state":"running"})
            .await
            .unwrap(),
        3
    );
    assert!(
        db.collection::<bson::Document>(crate::models::async_service_operation::COLLECTION_NAME)
            .find_one(doc! {"_id":"watch","state":"waiting"})
            .await
            .unwrap()
            .is_some()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn platform_thread_requires_live_turn_with_ordinary_rollout_disabled() {
    use crate::services::assistant_acknowledgement_service::{
        AssistantTurnFence, enforce_turn_gate,
    };
    let db = connect_transaction_test_database("nyxa_platform_gate_off").await;
    let owner = Uuid::new_v4().to_string();
    let mut fence = AssistantTurnFence {
        conversation_id: Uuid::new_v4().to_string(),
        turn_id: None,
        turn_live: false,
        turn_stopped: true,
        platform_thread: false,
        platform_origin: None,
    };
    // No rollout row: ordinary compatibility remains permissive.
    enforce_turn_gate(&db, &fence, &owner, Some("key"), None, None, None, "test")
        .await
        .unwrap();
    fence.platform_thread = true;
    assert!(matches!(
        enforce_turn_gate(&db, &fence, &owner, Some("key"), None, None, None, "test").await,
        Err(AppError::AssistantTurnRequired)
    ));
    fence.turn_id = Some("turn".into());
    fence.turn_live = true;
    fence.turn_stopped = false;
    // A fresh owner web turn carries no revoked transport authority.
    enforce_turn_gate(&db, &fence, &owner, Some("key"), None, None, None, "test")
        .await
        .unwrap();
    fence.turn_stopped = true;
    assert!(matches!(
        enforce_turn_gate(&db, &fence, &owner, Some("key"), None, None, None, "test").await,
        Err(AppError::AssistantTurnRequired)
    ));
    db.drop().await.unwrap();
}

#[test]
fn async_delivery_keeps_original_platform_proof_and_fresh_execution_origin_separate() {
    use crate::models::async_service_operation::{AsyncServiceOperation, DeliveryBinding};
    let mut row = stale_test_row(Utc::now());
    let original = platform_test_origin(&row);
    let mut fresh = original.clone();
    fresh.source = crate::models::assistant_conversation::PlatformOriginSource::ThreadEvent {
        admission_id: "fresh-event".into(),
    };
    row.platform_thread = Some(fresh.thread.clone());
    row.active_turn.as_mut().unwrap().platform_origin = Some(fresh.clone());
    let delivery = DeliveryBinding {
        platform_reply: Some(
            crate::models::assistant_conversation::PlatformReplyRouting::Captured {
                origin: Box::new(original.clone()),
                turn_id: "original-turn".into(),
                expires_at: bson::DateTime::from_chrono(Utc::now() + chrono::Duration::minutes(30))
                    .to_chrono(),
            },
        ),
        platform_origin: Some(original.clone()),
        origin: TurnOrigin::User,
        channel: None,
        reply_channel: None,
        group_id: None,
        group_request_id: None,
        voice_request_id: None,
        trigger_run_id: None,
        report_to: Some("nyxa-original-report".into()),
        report_provenance: crate::models::assistant_conversation::REPORT_PROVENANCE_V1,
    };
    let watch = AsyncServiceOperation {
        id: "watch".into(),
        user_id: row.user_id.clone(),
        conversation_id: row.id.clone(),
        agent_id: "agent".into(),
        turn_id: "original-turn".into(),
        api_key_id: "key".into(),
        service_id: "service".into(),
        submit_endpoint_id: "submit".into(),
        contract: crate::models::async_service_operation::AsyncOperationContract {
            status_operation: "status".into(),
            result_operation: "result".into(),
            cancel_operation: None,
            id_field: "id".into(),
            id_parameter: "id".into(),
            status_field: "status".into(),
            success_states: vec!["done".into()],
            failure_states: vec!["failed".into()],
            error_field: None,
        },
        delivery,
        operation_id: Some("operation".into()),
        state: "queued".into(),
        reason: None,
        attempts: 0,
        lease_id: None,
        lease_until: None,
        created_at: Utc::now(),
        deadline: Utc::now(),
        poll_at: Utc::now(),
        expires_at: Utc::now(),
        result_encrypted: None,
    };
    crate::services::async_service_operation::apply_delivery(&mut row, &watch);
    assert_eq!(
        row.active_turn.as_ref().unwrap().platform_origin,
        Some(fresh.clone())
    );
    assert_eq!(
        row.active_turn.as_ref().unwrap().platform_reply,
        watch.delivery.platform_reply
    );
    assert!(channel_note(&row).contains("private Telegram"));
    let mut web_watch = watch.clone();
    web_watch.delivery.platform_reply = None;
    crate::services::async_service_operation::apply_delivery(&mut row, &web_watch);
    assert_eq!(
        row.active_turn.as_ref().unwrap().platform_origin,
        Some(fresh)
    );
    assert_eq!(
        row.active_turn.as_ref().unwrap().platform_reply,
        Some(crate::models::assistant_conversation::PlatformReplyRouting::WebOnly)
    );
    assert!(!channel_note(&row).contains("private Telegram"));
    assert_eq!(watch.delivery.platform_origin, Some(original));
    assert_eq!(
        crate::models::assistant_conversation::carried_report_to(
            watch.delivery.report_to.as_deref(),
            watch.delivery.report_provenance,
        )
        .as_deref(),
        Some("nyxa-original-report"),
    );
    let mut legacy = bson::to_document(&watch.delivery).unwrap();
    legacy.remove("platform_origin");
    assert!(
        bson::from_document::<DeliveryBinding>(legacy)
            .unwrap()
            .platform_origin
            .is_none()
    );
    let mut legacy_report = bson::to_document(&watch.delivery).unwrap();
    legacy_report.remove("report_provenance");
    let legacy_report = bson::from_document::<DeliveryBinding>(legacy_report).unwrap();
    assert_eq!(
        legacy_report.platform_origin,
        watch.delivery.platform_origin
    );
    assert!(
        crate::models::assistant_conversation::carried_report_to(
            legacy_report.report_to.as_deref(),
            legacy_report.report_provenance,
        )
        .is_none()
    );
}

#[tokio::test]
async fn personal_settlement_receipt_mismatch_rolls_back_reply_and_pointer_clear() {
    let db = connect_transaction_test_database("nyxa_personal_settle_abort").await;
    ensure_indexes(&db).await.unwrap();
    let state = test_app_state(db.clone());
    let mut row = begin_turn(
        &db,
        "owner",
        &request(None, "hello"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let proof = platform_test_origin(&row);
    row.platform_thread = Some(proof.thread.clone());
    row.active_turn.as_mut().unwrap().platform_origin = Some(proof.clone());
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    seed_platform_execution(&db, &row, &proof).await;
    let receipts =
        db.collection::<bson::Document>(crate::models::platform_manager_ingress::COLLECTION_NAME);
    receipts
        .update_one(doc! {}, doc! {"$set":{"claim_id":"different-claim"}})
        .await
        .unwrap();
    let reply_id = Uuid::new_v4().to_string();
    assert!(
        finish_turn(
            &db,
            &row,
            &row.credential_api_key_id,
            &reply_id,
            &TurnResult {
                text: "uncommitted".into(),
                session_id: None,
                response_id: None,
                error: None,
            }
        )
        .await
        .is_err()
    );
    assert!(
        get(&db, "owner", &row.id)
            .await
            .unwrap()
            .active_turn
            .is_some()
    );
    assert!(
        db.collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"_id":reply_id})
            .await
            .unwrap()
            .is_none()
    );
    let binding = db
        .collection::<bson::Document>(crate::models::platform_personal_binding::COLLECTION_NAME)
        .find_one(doc! {"_id":&proof.thread.binding_id})
        .await
        .unwrap()
        .unwrap();
    assert!(binding.get_document("active_turn").is_ok());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn deleting_personal_thread_interrupts_only_its_exact_claim_and_clears_reference() {
    let db = connect_transaction_test_database("nyxa_personal_delete").await;
    ensure_indexes(&db).await.unwrap();
    let state = test_app_state(db.clone());
    let mut row = begin_turn(
        &db,
        "owner",
        &request(None, "hello"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let proof = platform_test_origin(&row);
    row.platform_thread = Some(proof.thread.clone());
    row.active_turn.as_mut().unwrap().platform_origin = Some(proof.clone());
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    seed_platform_execution(&db, &row, &proof).await;
    delete(&db, "owner", &row.id).await.unwrap();
    let binding = db
        .collection::<bson::Document>(crate::models::platform_personal_binding::COLLECTION_NAME)
        .find_one(doc! {"_id":&proof.thread.binding_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.get("conversation_id"), Some(&bson::Bson::Null));
    assert_eq!(binding.get("active_turn"), Some(&bson::Bson::Null));
    let receipt = db
        .collection::<bson::Document>(crate::models::platform_manager_ingress::COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.get_str("phase").unwrap(), "interrupted");
    assert_eq!(
        receipt.get_str("interrupted_reason").unwrap(),
        "conversation_deleted"
    );
    assert_eq!(receipt.get("reply_id"), Some(&bson::Bson::Null));
    assert!(matches!(
        get(&db, "owner", &row.id).await,
        Err(AppError::NotFound(_))
    ));
    db.drop().await.unwrap();
}

/// Plan v7 §2.2: the reducer never creates NyxBot involvement from
/// ambiguity, legacy data or owner-started work; carriers prove a target only
/// with the first validated provenance version.
#[test]
fn report_target_reducer_never_creates_involvement_from_ambiguity() {
    use crate::models::assistant_conversation::{REPORT_PROVENANCE_V1, carried_report_to};
    use crate::services::assistant_team_service::{bound_event, event};
    let bound = |target: Option<&str>| bound_event("permission_decided", "x".into(), None, target);
    let legacy = || event("message", "x".into(), None);
    let nb = "nyxa-nyxbot";
    // An assignment keeps its exact assigning thread only while every drained
    // event agrees with it.
    assert_eq!(
        reduce_report_target(TurnOrigin::Orchestrator, Some(nb), &[]).as_deref(),
        Some(nb)
    );
    assert_eq!(
        reduce_report_target(
            TurnOrigin::Orchestrator,
            Some(nb),
            &[bound(Some(nb)), bound(Some(nb))]
        )
        .as_deref(),
        Some(nb)
    );
    assert_eq!(
        reduce_report_target(TurnOrigin::Orchestrator, Some(nb), &[bound(None)]),
        None
    );
    assert_eq!(
        reduce_report_target(TurnOrigin::Orchestrator, Some(nb), &[bound(Some("other"))]),
        None
    );
    assert_eq!(
        reduce_report_target(TurnOrigin::Orchestrator, Some(nb), &[legacy()]),
        None
    );
    assert_eq!(
        reduce_report_target(TurnOrigin::Orchestrator, None, &[bound(Some(nb))]),
        None
    );
    // An event turn reports only when every drained event is bound to one target.
    assert_eq!(
        reduce_report_target(TurnOrigin::Event, None, &[bound(Some(nb)), bound(Some(nb))])
            .as_deref(),
        Some(nb)
    );
    assert_eq!(
        reduce_report_target(
            TurnOrigin::Event,
            Some(nb),
            &[bound(Some(nb)), bound(Some("other"))]
        ),
        None
    );
    assert_eq!(
        reduce_report_target(TurnOrigin::Event, Some(nb), &[bound(Some(nb)), bound(None)]),
        None
    );
    assert_eq!(
        reduce_report_target(TurnOrigin::Event, Some(nb), &[legacy()]),
        None
    );
    assert_eq!(
        reduce_report_target(TurnOrigin::Event, Some(nb), &[legacy(), bound(Some(nb))]),
        None
    );
    assert_eq!(reduce_report_target(TurnOrigin::Event, Some(nb), &[]), None);
    // Owner, channel, group and trigger turns never report, whatever was drained.
    for origin in [
        TurnOrigin::User,
        TurnOrigin::Channel,
        TurnOrigin::Group,
        TurnOrigin::Trigger,
    ] {
        assert_eq!(
            reduce_report_target(origin, Some(nb), &[bound(Some(nb))]),
            None,
            "{origin:?}"
        );
    }
    // Delayed carriers: only the validated version proves a target.
    assert_eq!(
        carried_report_to(Some(nb), REPORT_PROVENANCE_V1).as_deref(),
        Some(nb)
    );
    assert_eq!(carried_report_to(Some(nb), 0), None);
    assert_eq!(carried_report_to(Some(nb), 2), None);
    assert_eq!(carried_report_to(None, REPORT_PROVENANCE_V1), None);
    // Constructors: `event` proves nothing; `bound_to` proves what it is given.
    assert!(!legacy().bound && legacy().report_to.is_none());
    let proven = legacy().bound_to(Some(nb));
    assert!(proven.bound && proven.report_to.as_deref() == Some(nb));
    let owner_work = legacy().bound_to(None);
    assert!(owner_work.bound && owner_work.report_to.is_none());
}

async fn newest_reply(db: &Database, owner: &str, id: &str) -> AssistantMessage {
    messages(db, owner, id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find(|message| message.role == "assistant")
        .expect("a settled reply")
}

/// Plan v7 §2.7 entry-path matrix and lifecycle: the turn snapshot is decided
/// once at admission from proven sources, agrees with the chat key and the
/// settled reply, and the legacy conversation marker is never read.
#[tokio::test]
async fn turn_snapshot_is_decided_at_admission_and_copied_to_the_settled_reply() {
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_team_service as team,
    };
    let db = connect_transaction_test_database("nyxa_report_snapshot").await;
    ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let keys = &state.encryption_keys;
    // NyxBot's thread, and a specialist with a home thread.
    let nyxbot_thread = begin_turn(&db, &owner, &request(None, "Coordinate my work"), keys)
        .await
        .unwrap();
    settle(&db, keys, &owner, &nyxbot_thread, "ok", None).await;
    let (_, home) = team::create_specialist(
        &db,
        keys,
        &owner,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: "worker".into(),
            description: "Helps with notes".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let nb = nyxbot_thread.id.as_str();
    let assigned = |text: &str| {
        let mut start = TurnStart::from(&request(Some(&home.id), text));
        start.origin = TurnOrigin::Orchestrator;
        start.report_to = Some(nb.to_owned());
        start
    };
    let event_turn = || {
        let mut start = TurnStart::from(&request(Some(&home.id), ""));
        start.origin = TurnOrigin::Event;
        start
    };
    let bound = |target: Option<&str>| {
        team::bound_event("permission_decided", "decided".into(), None, target)
    };
    let legacy = || team::event("message", "legacy".into(), None);
    let turn_target =
        |row: &AssistantConversation| row.active_turn.as_ref().unwrap().report_to.clone();

    // 1. Assignment: the snapshot is the exact assigning thread; the legacy
    // marker is written for old readers; the chat key sees the snapshot; the
    // settled reply copies it.
    let row = begin_turn(&db, &owner, assigned("Summarize the notes"), keys)
        .await
        .unwrap();
    assert_eq!(turn_target(&row).as_deref(), Some(nb));
    assert!(!row.active_turn.as_ref().unwrap().with_nyxbot);
    assert_eq!(row.report_to.as_deref(), Some(nb));
    let chat = acks::for_key(&db, &owner, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chat.report_to.as_deref(), Some(nb));
    assert!(chat.involved());
    settle(&db, keys, &owner, &row, "done", None).await;
    assert_eq!(
        newest_reply(&db, &owner, &home.id)
            .await
            .report_to
            .as_deref(),
        Some(nb)
    );

    // 2. A direct owner message is owner work: no snapshot, the legacy marker
    // is cleared, the chat key is not involved, the reply carries no target.
    let row = begin_turn(
        &db,
        &owner,
        &request(Some(&home.id), "Also add a summary line"),
        keys,
    )
    .await
    .unwrap();
    assert_eq!(turn_target(&row), None);
    assert_eq!(row.report_to, None);
    let chat = acks::for_key(&db, &owner, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(!chat.involved() && chat.report_to.is_none());
    settle(&db, keys, &owner, &row, "done", None).await;
    assert_eq!(newest_reply(&db, &owner, &home.id).await.report_to, None);

    // 3. A stale legacy marker is never read: a legacy (unbound) event drains
    // into an event turn with no target.
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id": &home.id}, doc! {"$set": {"report_to": nb}})
        .await
        .unwrap();
    push_events(&db, &owner, &home.id, vec![legacy()])
        .await
        .unwrap();
    let row = begin_turn(&db, &owner, event_turn(), keys).await.unwrap();
    assert_eq!(turn_target(&row), None);
    settle(&db, keys, &owner, &row, "done", None).await;
    assert_eq!(newest_reply(&db, &owner, &home.id).await.report_to, None);

    // 4. Event batches: all bound to one target report there; mixed targets, a
    // bound None, or a legacy event beside a bound one resolve to None.
    for (events, expected) in [
        (vec![bound(Some(nb)), bound(Some(nb))], Some(nb.to_owned())),
        (vec![bound(Some(nb)), bound(Some("nyxa-other"))], None),
        (vec![bound(Some(nb)), bound(None)], None),
        (vec![legacy(), bound(Some(nb))], None),
    ] {
        push_events(&db, &owner, &home.id, events).await.unwrap();
        let row = begin_turn(&db, &owner, event_turn(), keys).await.unwrap();
        assert_eq!(turn_target(&row), expected);
        settle(&db, keys, &owner, &row, "done", None).await;
        assert_eq!(
            newest_reply(&db, &owner, &home.id).await.report_to,
            expected
        );
    }

    // 5. An assignment drained with a queued owner message (bound None), an
    // event bound elsewhere or a legacy event is not NyxBot's work (R4-1);
    // with an event bound to the same thread it is.
    for (events, expected) in [
        (
            vec![team::bound_event("message", "owner".into(), None, None)],
            None,
        ),
        (vec![bound(Some("nyxa-other"))], None),
        (vec![legacy()], None),
        (vec![bound(Some(nb))], Some(nb.to_owned())),
    ] {
        push_events(&db, &owner, &home.id, events).await.unwrap();
        let row = begin_turn(&db, &owner, assigned("Continue"), keys)
            .await
            .unwrap();
        assert_eq!(turn_target(&row), expected);
        settle(&db, keys, &owner, &row, "done", None).await;
    }

    // 6. A browser card continuation is not a new owner message: it keeps the
    // legacy marker for old readers but still snapshots None.
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id": &home.id}, doc! {"$set": {"report_to": nb}})
        .await
        .unwrap();
    let card_id = Uuid::new_v4().to_string();
    db.collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &card_id, "conversation_id": &home.id, "user_id": &owner,
            "api_key_id": &row.credential_api_key_id, "kind": "action",
            "summary": "Send the mail", "status": "allowed", "decided_by": "user",
            "created_at": bson::DateTime::now(),
            "expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1)),
        })
        .await
        .unwrap();
    let mut continuation = TurnStart::from(&request(Some(&home.id), "Retry it"));
    continuation.acknowledgement_id = Some(card_id);
    let row = begin_turn(&db, &owner, continuation, keys).await.unwrap();
    assert_eq!(turn_target(&row), None);
    assert_eq!(row.report_to.as_deref(), Some(nb));
    settle(&db, keys, &owner, &row, "done", None).await;

    // 7. A retry has no report snapshot, but preserves the compatibility
    // marker because it continues an earlier owner request (only owner
    // turns can be retried).
    let row = begin_turn(&db, &owner, &request(Some(&home.id), "Fragile work"), keys)
        .await
        .unwrap();
    settle(&db, keys, &owner, &row, "", Some("upstream")).await;
    let failed_turn = row.active_turn.as_ref().unwrap().turn_id.clone();
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(doc! {"_id": &home.id}, doc! {"$set": {"report_to": nb}})
        .await
        .unwrap();
    let retried = begin_turn(
        &db,
        &owner,
        &retry(&home.id, "Fragile work", &failed_turn),
        keys,
    )
    .await
    .unwrap();
    assert_eq!(turn_target(&retried), None);
    assert_eq!(retried.report_to.as_deref(), Some(nb));
    settle(&db, keys, &owner, &retried, "done", None).await;

    // 8. A lost assigned turn is reclaimed with its snapshot on the failed
    // reply; the owner message that reclaimed it is new owner work.
    let row = begin_turn(&db, &owner, assigned("Long work"), keys)
        .await
        .unwrap();
    expire_turn(&db, &row).await;
    let reclaimed = begin_turn(&db, &owner, &request(Some(&home.id), "Hello again"), keys)
        .await
        .unwrap();
    let lost = messages(&db, &owner, &home.id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .find(|message| message.error_code.as_deref() == Some("turn_lost"))
        .expect("the lost reply");
    assert_eq!(lost.report_to.as_deref(), Some(nb));
    assert_eq!(turn_target(&reclaimed), None);
    assert_eq!(reclaimed.report_to, None);
    settle(&db, keys, &owner, &reclaimed, "done", None).await;
    // The persisted conversation agrees with the turn snapshot every time.
    let persisted = get(&db, &owner, &home.id).await.unwrap();
    assert!(persisted.active_turn.is_none());
}

/// Plan v7 §4 (R4-6/R5-4): queued allowed-skill notifications are admitted
/// into a NyxBot thread only while eligible. An all-ineligible batch commits a
/// "drained; no turn started" outcome in the admission transaction (queue
/// entries removed, outbox markers terminal, no streak, no message); a mixed
/// batch continues with the eligible items; a transaction that aborts after
/// the cleanup rolls every marker and queue entry back.
#[tokio::test]
async fn ineligible_skill_events_are_drained_transactionally_without_a_turn() {
    use crate::services::assistant_team_service as team;
    let db = connect_transaction_test_database("nyxa_drained_skill_events").await;
    ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let keys = &state.encryption_keys;
    let nyxbot = begin_turn(&db, &owner, &request(None, "Coordinate my work"), keys)
        .await
        .unwrap();
    settle(&db, keys, &owner, &nyxbot, "ok", None).await;
    let (_, worker_home) = team::create_specialist(
        &db,
        keys,
        &owner,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: "worker".into(),
            description: "Helps".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let acks =
        db.collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME);
    // An owner-allowed skill card with its durable outbox marker and the
    // queued notification for NyxBot's thread.
    let card = |routing: bson::Document| {
        let acks = acks.clone();
        let owner = owner.clone();
        let home = worker_home.id.clone();
        async move {
            let id = Uuid::new_v4().to_string();
            acks.insert_one(doc! {
                "_id": &id, "conversation_id": &home, "user_id": &owner,
                "api_key_id": "key", "kind": "skills", "summary": "Attach the skill",
                "status": "allowed", "decided_by": "user", "routing": routing,
                "skill_notification_pending": true, "skill_notification_fence": 0_i64,
                "created_at": bson::DateTime::now(),
                "expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1)),
            })
            .await
            .unwrap();
            let mut event = team::event(
                "skill_request_allowed",
                "The user allowed a skill".into(),
                None,
            );
            event.id = id.clone();
            event
        }
    };
    let assigned = |target: &str| doc! {"kind": "assigned", "report_to": target};
    let not_involved = || doc! {"kind": "not_involved"};
    let marker = |id: &str| {
        let acks = acks.clone();
        let id = id.to_owned();
        async move {
            let row = acks.find_one(doc! {"_id": &id}).await.unwrap().unwrap();
            (
                row.get_bool("skill_notification_pending").unwrap_or(false),
                row.get_document("skill_notification_terminal")
                    .ok()
                    .and_then(|t| t.get_str("reason").ok().map(str::to_owned)),
            )
        }
    };
    let event_turn = || {
        let mut start = TurnStart::from(&request(Some(&nyxbot.id), ""));
        start.origin = TurnOrigin::Event;
        start
    };
    let thread = |db: &Database| {
        let db = db.clone();
        let owner = owner.clone();
        let id = nyxbot.id.clone();
        async move { get(&db, &owner, &id).await.unwrap() }
    };
    let before = thread(&db).await;

    // All ineligible: drained without a turn.
    let dropped_a = card(not_involved()).await;
    let dropped_b = card(not_involved()).await;
    push_events(
        &db,
        &owner,
        &nyxbot.id,
        vec![dropped_a.clone(), dropped_b.clone()],
    )
    .await
    .unwrap();
    assert!(matches!(
        begin_turn(&db, &owner, event_turn(), keys).await,
        Err(AppError::Conflict(message)) if message == "No eligible pending events"
    ));
    let after = thread(&db).await;
    assert!(after.pending_events.is_empty());
    assert!(after.active_turn.is_none());
    assert_eq!(after.event_streak, before.event_streak);
    assert_eq!(after.message_count, before.message_count);
    assert_eq!(
        messages(&db, &owner, &nyxbot.id, 100, None)
            .await
            .unwrap()
            .len(),
        before.message_count as usize
    );
    for id in [&dropped_a.id, &dropped_b.id] {
        assert_eq!(marker(id).await, (false, Some("not_involved".into())));
    }
    // No eligible items left means no wake-up either.
    assert!(matches!(
        begin_turn(&db, &owner, event_turn(), keys).await,
        Err(AppError::Conflict(message)) if message == "No pending events"
    ));

    // Mixed: the eligible item starts the turn, the other is terminalized
    // and gone from the queue.
    let kept = card(assigned(&nyxbot.id)).await;
    let dropped = card(not_involved()).await;
    let plain = team::bound_event(
        "subagent_settled",
        "Specialist worker replied.".into(),
        None,
        None,
    );
    push_events(
        &db,
        &owner,
        &nyxbot.id,
        vec![dropped.clone(), kept.clone(), plain.clone()],
    )
    .await
    .unwrap();
    let row = begin_turn(&db, &owner, event_turn(), keys).await.unwrap();
    let turn = row.active_turn.as_ref().unwrap();
    assert_eq!(
        turn.events
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        vec![kept.id.as_str(), plain.id.as_str()]
    );
    assert!(!turn_input(&row, &event_turn()).contains(&dropped.id));
    assert!(row.pending_events.is_empty());
    assert_eq!(row.event_streak, before.event_streak + 1);
    assert_eq!(marker(&kept.id).await, (false, None));
    assert_eq!(
        marker(&dropped.id).await,
        (false, Some("not_involved".into()))
    );
    settle(&db, keys, &owner, &row, "attached", None).await;

    // A transaction that fails after the cleanup rolls it back: the markers
    // stay pending and the queue keeps every entry, so the next admission
    // drains them again (transient failures never count as ineligibility).
    let dropped = card(not_involved()).await;
    let kept = card(assigned(&nyxbot.id)).await;
    push_events(&db, &owner, &nyxbot.id, vec![dropped.clone(), kept.clone()])
        .await
        .unwrap();
    let mut aborted = event_turn();
    aborted.attachment_ids = vec!["not-an-upload-id".into()];
    assert!(matches!(
        begin_turn(&db, &owner, aborted, keys).await,
        Err(AppError::BadRequest(_))
    ));
    let rolled_back = thread(&db).await;
    assert_eq!(rolled_back.pending_events.len(), 2);
    assert!(rolled_back.active_turn.is_none());
    assert_eq!(marker(&dropped.id).await, (true, None));
    assert_eq!(marker(&kept.id).await, (true, None));
    let row = begin_turn(&db, &owner, event_turn(), keys).await.unwrap();
    assert_eq!(row.active_turn.as_ref().unwrap().events.len(), 1);
    assert_eq!(
        marker(&dropped.id).await,
        (false, Some("not_involved".into()))
    );
    assert_eq!(marker(&kept.id).await, (false, None));
    settle(&db, keys, &owner, &row, "attached", None).await;

    // Regression: the report target is reduced over every DRAINED event, not
    // only the eligible ones. An assignment that drains an ineligible (or
    // any unbound) skill notification is no longer proven NyxBot's work, even
    // though the filtered item never reaches the model context.
    let assigned_start = |text: &str| {
        let mut start = TurnStart::from(&request(Some(&worker_home.id), text));
        start.origin = TurnOrigin::Orchestrator;
        start.report_to = Some(nyxbot.id.clone());
        start
    };
    for routing in [not_involved(), assigned(&nyxbot.id)] {
        let eligible = routing.get_str("kind").unwrap() == "assigned";
        let skill = card(routing).await;
        push_events(&db, &owner, &worker_home.id, vec![skill.clone()])
            .await
            .unwrap();
        let row = begin_turn(&db, &owner, assigned_start("Attach and continue"), keys)
            .await
            .unwrap();
        let turn = row.active_turn.as_ref().unwrap();
        assert_eq!(turn.report_to, None);
        assert_eq!(
            turn.events.iter().any(|event| event.id == skill.id),
            eligible
        );
        assert!(row.pending_events.is_empty());
        settle(&db, keys, &owner, &row, "done", None).await;
    }
    // A clean assignment on the same thread still proves its target.
    let row = begin_turn(&db, &owner, assigned_start("Clean assignment"), keys)
        .await
        .unwrap();
    assert_eq!(
        row.active_turn.as_ref().unwrap().report_to.as_deref(),
        Some(nyxbot.id.as_str())
    );
    settle(&db, keys, &owner, &row, "done", None).await;
    // An event batch that drains a removed ineligible item beside an event
    // bound to the assigning thread resolves to None too; only the bound
    // event reaches the turn.
    let dropped = card(not_involved()).await;
    let bound = team::bound_event(
        "permission_decided",
        "allowed".into(),
        None,
        Some(&nyxbot.id),
    );
    push_events(
        &db,
        &owner,
        &nyxbot.id,
        vec![dropped.clone(), bound.clone()],
    )
    .await
    .unwrap();
    let row = begin_turn(&db, &owner, event_turn(), keys).await.unwrap();
    let turn = row.active_turn.as_ref().unwrap();
    assert_eq!(turn.report_to, None);
    assert_eq!(
        turn.events
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        vec![bound.id.as_str()]
    );
    assert_eq!(
        marker(&dropped.id).await,
        (false, Some("not_involved".into()))
    );
    settle(&db, keys, &owner, &row, "done", None).await;
    // Without the removed item, the same bound event proves the target.
    push_events(&db, &owner, &nyxbot.id, vec![bound.clone()])
        .await
        .unwrap();
    let row = begin_turn(&db, &owner, event_turn(), keys).await.unwrap();
    assert_eq!(
        row.active_turn.as_ref().unwrap().report_to.as_deref(),
        Some(nyxbot.id.as_str())
    );
    settle(&db, keys, &owner, &row, "done", None).await;
}

#[tokio::test]
async fn model_facts_are_attempt_and_turn_fenced_and_preserve_upstream_diagnostics() {
    use crate::services::assistant_model_observer::{ModelRequest, attribute};
    let db = connect_transaction_test_database("model_facts").await;
    let now = Utc::now();
    let mut row = stale_test_row(now);
    row.active_turn.as_mut().unwrap().started_at = now;
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .insert_one(&row)
        .await
        .unwrap();
    let generic = TurnError::new("turn_failed");
    for (error, code) in [
        (AppError::InsufficientCredits, "insufficient_credits"),
        (AppError::WalletSuspended, "wallet_suspended"),
        (
            AppError::PlanEntitlementRequired("service".into()),
            "plan_entitlement_required",
        ),
    ] {
        let attempt = ModelRequest::capture(
            &db,
            &row.id,
            "key",
            &row.active_turn.as_ref().unwrap().turn_id,
        )
        .await
        .unwrap()
        .unwrap();
        attempt.refusal(&error).await.unwrap();
        attempt.fault("transport_error", true).await.unwrap();
        let saved = db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        let turn = saved.active_turn.as_ref().unwrap();
        let classified = attribute(&generic, turn, Utc::now());
        assert_eq!(classified.code, code);
        assert_eq!(classified.upstream_code.as_deref(), Some("turn_failed"));
        assert_eq!(
            attribute(&TurnError::new("capacity_exceeded"), turn, Utc::now()).code,
            "capacity_exceeded"
        );
        assert_eq!(
            attribute(
                &generic,
                turn,
                turn.model_refusal.as_ref().unwrap().at
                    + chrono::Duration::from_std(
                        engine_policy::MODEL_ATTRIBUTION_WINDOW + Duration::from_secs(1)
                    )
                    .unwrap()
            )
            .code,
            "assistant_unavailable"
        );
        attempt.complete().await.unwrap();
        let success = ModelRequest::capture(
            &db,
            &row.id,
            "key",
            &row.active_turn.as_ref().unwrap().turn_id,
        )
        .await
        .unwrap()
        .unwrap();
        success.accepted().await.unwrap();
        // Older writes arrive after a newer accepted request, even on another replica.
        attempt.refusal(&error).await.unwrap();
        attempt.fault("transport_error", true).await.unwrap();
        let saved = db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(saved.active_turn.as_ref().unwrap().model_refusal.is_none());
        assert_eq!(
            attribute(&generic, saved.active_turn.as_ref().unwrap(), Utc::now()).code,
            "assistant_unavailable"
        );
        success.fault("consumer_cancelled", false).await.unwrap();
        let saved = db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            attribute(&generic, saved.active_turn.as_ref().unwrap(), Utc::now()).code,
            "assistant_unavailable"
        );
        success.fault("provider_error", true).await.unwrap();
        success.fault("consumer_cancelled", false).await.unwrap();
        let saved = db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            attribute(&generic, saved.active_turn.as_ref().unwrap(), Utc::now()).code,
            "model_interrupted"
        );
        assert_eq!(
            saved
                .active_turn
                .as_ref()
                .unwrap()
                .model_stream_cancellation
                .as_ref()
                .unwrap()
                .kind,
            "consumer_cancelled"
        );

        success.complete().await.unwrap();
        row.active_turn.as_mut().unwrap().turn_id = Uuid::new_v4().to_string();
        db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id": &row.id}, doc! {"$set": {"active_turn.turn_id": &row.active_turn.as_ref().unwrap().turn_id}, "$unset": {"active_turn.model_refusal": "", "active_turn.model_stream_fault": ""}}).await.unwrap();
        success.refusal(&error).await.unwrap();
        success.fault("transport_error", true).await.unwrap();
        let saved = db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(saved.active_turn.as_ref().unwrap().model_refusal.is_none());
        assert!(
            saved
                .active_turn
                .as_ref()
                .unwrap()
                .model_stream_fault
                .is_none()
        );
    }
    assert!(
        ModelRequest::capture(
            &db,
            &row.id,
            "wrong-key",
            &row.active_turn.as_ref().unwrap().turn_id
        )
        .await
        .unwrap()
        .is_none()
    );
    db.drop().await.unwrap();
}

#[test]
fn model_stream_protocols_require_terminals_but_never_usage() {
    use crate::services::assistant_model_observer::{Protocol, StreamObserver};
    for (protocol, terminal) in [
        (Protocol::Responses, r#"{"type":"response.completed"}"#),
        (Protocol::ChatCompletions, "[DONE]"),
        (Protocol::Anthropic, r#"{"type":"message_stop"}"#),
    ] {
        let mut stream = StreamObserver::new(protocol);
        assert_eq!(stream.eof(), Some("missing_terminal"));
        stream.event(Some("message"), terminal);
        assert_eq!(stream.eof(), None);
    }
    let mut unknown = StreamObserver::new(Protocol::Unknown);
    unknown.event(None, r#"{"error":{"code":"bad"}}"#);
    assert!(!unknown.provider_failed());
    assert_eq!(unknown.eof(), Some("unknown"));
    let mut failed = StreamObserver::new(Protocol::Responses);
    failed.event(None, r#"{"type":"response.failed"}"#);
    failed.invalidate();
    assert_eq!(failed.eof(), Some("unknown"));
}

#[test]
fn capacity_retry_after_and_full_jitter_are_bounded() {
    for value in [
        "",
        "0",
        "-1",
        "1.2",
        " 1",
        "Wed, 21 Oct 2015 07:28:00 GMT",
        "9999999999999999999999",
    ] {
        assert_eq!(retry_after(value), None);
    }
    assert_eq!(
        retry_after(&(engine_policy::RETRY_AFTER_MAX.as_secs() + 1).to_string()),
        None
    );
    assert_eq!(
        retry_after(&(engine_policy::RETRY_AFTER_MIN.as_secs() - 1).to_string()),
        None
    );
    for value in [
        engine_policy::RETRY_AFTER_MIN.as_secs(),
        engine_policy::CAPACITY_BACKOFF_CAP.as_secs(),
        engine_policy::RETRY_AFTER_MAX.as_secs(),
    ] {
        assert_eq!(
            retry_after(&value.to_string()),
            Some(Duration::from_secs(value))
        );
    }
    for retry in 1..100 {
        assert_eq!(capacity_delay(retry, None, 0), Duration::ZERO);
        assert!(capacity_delay(retry, None, u64::MAX) <= engine_policy::CAPACITY_BACKOFF_CAP);
        assert_eq!(
            capacity_delay(retry, Some(engine_policy::RETRY_AFTER_MAX), 0),
            engine_policy::RETRY_AFTER_MAX
        );
    }
}

#[test]
fn capacity_deadline_covers_request_age_and_expired_grace() {
    let now = Utc::now();
    assert_eq!(
        capacity_budget(now, None, now),
        engine_policy::INTERACTIVE_CAPACITY_BUDGET
    );
    assert_eq!(
        capacity_budget(now - chrono::Duration::seconds(32), None, now),
        engine_policy::INTERACTIVE_CAPACITY_BUDGET - Duration::from_secs(32)
    );
    assert_eq!(
        capacity_budget(now, Some(now - chrono::Duration::seconds(1)), now),
        Duration::ZERO
    );
    assert_eq!(
        capacity_budget(now, Some(now + chrono::Duration::seconds(900)), now),
        engine_policy::AUTOMATION_CAPACITY_CAP
    );
    assert_eq!(
        capacity_budget(
            now - chrono::Duration::from_std(
                engine_policy::AUTOMATION_CAPACITY_CAP - Duration::from_secs(1)
            )
            .unwrap(),
            Some(now + chrono::Duration::seconds(900)),
            now
        ),
        Duration::from_secs(1)
    );
}

#[tokio::test]
async fn overlapping_model_requests_keep_facts_diagnostic_only() {
    use crate::services::assistant_model_observer::{ModelRequest, attribute};
    let db = connect_transaction_test_database("model_overlap").await;
    let mut row = stale_test_row(Utc::now());
    row.active_turn.as_mut().unwrap().started_at = Utc::now();
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .insert_one(&row)
        .await
        .unwrap();
    let turn = &row.active_turn.as_ref().unwrap().turn_id;
    let first = ModelRequest::capture(&db, &row.id, "key", turn)
        .await
        .unwrap()
        .unwrap();
    let second = ModelRequest::capture(&db, &row.id, "key", turn)
        .await
        .unwrap()
        .unwrap();
    second
        .refusal(&AppError::InsufficientCredits)
        .await
        .unwrap();
    second.fault("provider_error", true).await.unwrap();
    first.complete().await.unwrap();
    // Even after overlapping requests finish, causality for this turn is uncertain.
    let saved = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    let turn = saved.active_turn.as_ref().unwrap();
    assert_eq!(turn.model_requests_active, 0);
    assert!(turn.model_attribution_uncertain);
    assert!(turn.model_refusal.is_some());
    assert!(turn.model_stream_fault.is_some());
    assert_eq!(
        attribute(&TurnError::new("turn_failed"), turn, Utc::now()).code,
        "assistant_unavailable"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn capacity_wait_metadata_survives_reload_notice_claims_and_audits_are_deduplicated() {
    use crate::services::assistant_turn_observability as observed;
    let db = connect_transaction_test_database("capacity_observability").await;
    let state = test_app_state(db.clone());
    for (wait, fail) in [(false, false), (true, false), (true, true)] {
        let row = begin_turn(
            &db,
            "owner",
            &request(None, "private prompt"),
            &state.encryption_keys,
        )
        .await
        .unwrap();
        assert!(!observed::claim_notice(&db, &row, Utc::now()).await.unwrap());
        assert!(
            row.active_turn
                .as_ref()
                .unwrap()
                .capacity_wait_note_origin
                .is_none()
        );
        let first_attempt = observed::attempt(&db, &row).await.unwrap();
        assert_eq!(first_attempt, 1);
        let run_id = Uuid::new_v4().to_string();
        db.collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME)
            .insert_one(doc! {"_id": &run_id, "user_id": "owner",
            "turn_id": &row.active_turn.as_ref().unwrap().turn_id,
            "attempts": 3_i64, "capacity_wait_ms": 1_000_i64})
            .await
            .unwrap();
        let began = Utc::now() - chrono::Duration::from_std(engine_policy::ACK_DELAY * 2).unwrap();
        let deadline =
            began + chrono::Duration::from_std(engine_policy::INTERACTIVE_CAPACITY_BUDGET).unwrap();
        observed::request_started(&db, &row, began, deadline)
            .await
            .unwrap();
        if wait {
            waiting_for_capacity(&db, &row, began, deadline)
                .await
                .unwrap();
            waiting_for_capacity(
                &db,
                &row,
                Utc::now(),
                deadline + chrono::Duration::seconds(30),
            )
            .await
            .unwrap();
            assert!(
                observed::acknowledge(
                    &db,
                    &row,
                    began + chrono::Duration::from_std(engine_policy::ACK_DELAY).unwrap()
                        - chrono::Duration::milliseconds(1)
                )
                .await
                .unwrap()
                .is_none()
            );
            observed::acknowledge(&db, &row, Utc::now())
                .await
                .unwrap()
                .unwrap();
            let reloaded = get(&db, "owner", &row.id).await.unwrap();
            let turn = reloaded.active_turn.as_ref().unwrap();
            assert_eq!(
                turn.capacity_wait_note_origin.as_deref(),
                Some("nyxid_system")
            );
            let input = messages(&db, "owner", &row.id, 20, None).await.unwrap();
            assert_eq!(input.len(), 1);
            assert!(!bounded_recap(&input).contains(observed::CAPACITY_ACKNOWLEDGEMENT));
            assert!(checkpoint(&db, "owner", &row.id).await.unwrap().is_none());
            assert_eq!(
                turn.capacity_wait_started_at.unwrap().timestamp_millis(),
                began.timestamp_millis()
            );
            assert_eq!(
                turn.capacity_wait_deadline.unwrap().timestamp_millis(),
                deadline.timestamp_millis()
            );
            assert!(
                !observed::claim_notice(
                    &db,
                    &row,
                    began + chrono::Duration::from_std(engine_policy::ACK_DELAY).unwrap()
                        - chrono::Duration::milliseconds(1)
                )
                .await
                .unwrap()
            );
            let (a, b) = tokio::join!(
                observed::claim_notice(&db, &row, Utc::now()),
                observed::claim_notice(&db, &row, Utc::now())
            );
            assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
            assert_eq!(observed::attempt(&db, &row).await.unwrap(), 2);
            if !fail {
                observed::accepted(&db, &row).await.unwrap();
                observed::accepted(&db, &row).await.unwrap();
                assert!(!observed::claim_notice(&db, &row, Utc::now()).await.unwrap());
                assert!(
                    !get(&db, "owner", &row.id)
                        .await
                        .unwrap()
                        .active_turn
                        .unwrap()
                        .waiting_for_capacity
                );
            }
        }
        db.collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"active_turn.trigger_run_id": &run_id}},
            )
            .await
            .unwrap();
        let message_id = Uuid::new_v4().to_string();
        let result = TurnResult {
            text: if fail { "" } else { "private reply" }.into(),
            session_id: None,
            response_id: None,
            error: fail.then(|| TurnError::internal("capacity_exceeded", "upstream_response")),
        };
        for _ in 0..2 {
            let error = finish_turn(&db, &row, "key", &message_id, &result)
                .await
                .unwrap();
            if fail {
                let error = error.unwrap();
                assert_eq!(error.attempts(), Some(2));
                assert!(
                    error.capacity_wait_ms().unwrap()
                        >= (engine_policy::ACK_DELAY * 2).as_millis() as u64
                );
                assert!(error.message.contains("waited"));
            }
        }
        let saved = db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"_id": &message_id})
            .await
            .unwrap()
            .unwrap();
        let metadata = saved.turn_metadata.unwrap();
        assert_eq!(metadata.attempts, Some(if wait { 2 } else { 1 }));
        assert!(metadata.capacity_wait_ms.is_some());
        if !wait {
            assert_eq!(metadata.capacity_wait_ms, Some(0));
        }
        let run = db
            .collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME)
            .find_one(doc! {"_id": &run_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            run.get_i64("attempts").unwrap(),
            3 + i64::from(metadata.attempts.unwrap())
        );
        assert_eq!(
            run.get_i64("capacity_wait_ms").unwrap(),
            1_000 + metadata.capacity_wait_ms.unwrap() as i64
        );
        if let Some(detail) = saved.error_detail {
            assert_eq!(detail.attempts, Some(2));
            assert_eq!(detail.capacity_wait_ms, metadata.capacity_wait_ms);
        }
        for (event, expected) in [
            ("assistant_turn_failed", u64::from(fail)),
            ("assistant_turn_capacity_wait", u64::from(wait && !fail)),
        ] {
            let filter = doc! {"event_type": event, "event_data.conversation_id": &row.id};
            assert_eq!(
                db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                    .count_documents(filter.clone())
                    .await
                    .unwrap(),
                expected
            );
            if expected != 0 {
                let audit = db
                    .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                    .find_one(filter)
                    .await
                    .unwrap()
                    .unwrap();
                let json = serde_json::to_string(&audit).unwrap();
                assert!(!json.contains("private prompt") && !json.contains("private reply"));
                assert_eq!(
                    audit
                        .get_document("event_data")
                        .unwrap()
                        .get_str("origin")
                        .unwrap(),
                    "chat"
                );
            }
        }
    }
    db.drop().await.unwrap();
}

#[test]
fn capacity_exhaustion_copy_uses_actual_wait_for_interactive_and_short_trigger_budgets() {
    use crate::services::assistant_turn_observability::capacity_message;
    assert_eq!(
        capacity_message(Some(120_000)),
        "NyxBot is at capacity right now (waited 2 minutes). Try again shortly."
    );
    assert!(capacity_message(Some(2_000)).contains("waited 2 seconds"));
    assert!(capacity_message(Some(119_999)).contains("waited 2 minutes"));
    assert!(capacity_message(Some(120_001)).contains("waited 2 minutes 1 second"));
    assert!(capacity_message(Some(600_000)).contains("waited 10 minutes"));
    assert!(!capacity_message(None).contains("waited"));
}

#[tokio::test]
async fn admission_acknowledgement_covers_slow_connect_but_never_accepted_stopped_or_automation_turns()
 {
    use crate::services::assistant_turn_observability as observed;
    let db = connect_transaction_test_database("admission_acknowledgement").await;
    let now = Utc::now();
    let began = now
        - chrono::Duration::from_std(engine_policy::ACK_DELAY).unwrap()
        - chrono::Duration::seconds(1);
    let deadline =
        began + chrono::Duration::from_std(engine_policy::INTERACTIVE_CAPACITY_BUDGET).unwrap();
    for mode in [
        "owner",
        "guest",
        "accepted",
        "stopped",
        "trigger",
        "trigger_delivery",
    ] {
        let mut row = stale_test_row(now);
        row.guest_turn = mode == "guest";
        let turn = row.active_turn.as_mut().unwrap();
        turn.started_at = began;
        turn.stop_requested = mode == "stopped";
        if mode == "trigger" {
            turn.origin = TurnOrigin::Trigger;
        }
        if mode == "trigger_delivery" {
            turn.trigger_run_id = Some(Uuid::new_v4().to_string());
        }
        db.collection::<AssistantConversation>(CONVERSATIONS)
            .insert_one(&row)
            .await
            .unwrap();
        observed::request_started(&db, &row, began, deadline)
            .await
            .unwrap();
        if mode == "accepted" {
            observed::accepted(&db, &row).await.unwrap();
        }
        let note = observed::acknowledge(&db, &row, now).await.unwrap();
        assert_eq!(note.is_some(), matches!(mode, "owner" | "guest"), "{mode}");
        if let Some(turn) = note {
            assert!(!turn.waiting_for_capacity);
            assert_eq!(
                turn.capacity_wait_note_origin.as_deref(),
                Some("nyxid_system")
            );
            assert_eq!(observed::metadata(&turn, now).capacity_wait_ms, Some(0));
            assert!(
                messages(&db, &row.user_id, &row.id, 20, None)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                checkpoint(&db, &row.user_id, &row.id)
                    .await
                    .unwrap()
                    .is_none()
            );
            let (a, b) = tokio::join!(
                observed::claim_notice(&db, &row, now),
                observed::claim_notice(&db, &row, now)
            );
            assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
            observed::accepted(&db, &row).await.unwrap();
            assert!(
                observed::acknowledge(&db, &row, now)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                get(&db, &row.user_id, &row.id)
                    .await
                    .unwrap()
                    .active_turn
                    .unwrap()
                    .capacity_wait_note_origin
                    .is_none()
            );
        }
        assert!(!observed::claim_notice(&db, &row, now).await.unwrap());
    }
    assert_eq!(
        db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type": "assistant_turn_capacity_wait"})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[test]
fn capacity_wait_durations_are_grammatical_and_empty_waits_are_omitted() {
    use crate::services::assistant_turn_observability::{capacity_message, wait_duration};
    for (ms, expected) in [
        (0, "0 seconds"),
        (1, "1 second"),
        (1_000, "1 second"),
        (45_000, "45 seconds"),
        (60_000, "1 minute"),
        (65_000, "1 minute 5 seconds"),
        (120_000, "2 minutes"),
        (120_001, "2 minutes 1 second"),
    ] {
        assert_eq!(wait_duration(ms), expected);
        assert_eq!(capacity_message(Some(ms)).contains("waited"), ms > 0);
    }
    assert!(!capacity_message(None).contains("waited"));
}

#[tokio::test]
async fn capacity_waits_accumulate_without_execution_time_and_acceptance_is_idempotent() {
    use crate::services::assistant_turn_observability as observed;
    let db = connect_transaction_test_database("capacity_request_waits").await;
    let state = test_app_state(db.clone());
    let row = begin_turn(
        &db,
        "owner",
        &request(None, "hello"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    // An additive wait flag without a timestamp cannot prove any elapsed wait.
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"active_turn.waiting_for_capacity": true}},
        )
        .await
        .unwrap();
    observed::accepted(&db, &row).await.unwrap(); // Common, non-transactional path.
    let now = Utc::now();
    let deadline =
        now + chrono::Duration::from_std(engine_policy::INTERACTIVE_CAPACITY_BUDGET).unwrap();
    for seconds in [2, 3] {
        waiting_for_capacity(
            &db,
            &row,
            Utc::now() - chrono::Duration::seconds(seconds),
            deadline,
        )
        .await
        .unwrap();
        observed::accepted(&db, &row).await.unwrap();
        observed::accepted(&db, &row).await.unwrap();
    }
    let saved = get(&db, "owner", &row.id).await.unwrap();
    let turn = saved.active_turn.as_ref().unwrap();
    let data = observed::metadata(turn, Utc::now() + chrono::Duration::hours(1));
    assert!((5_000..6_000).contains(&data.capacity_wait_ms.unwrap()));
    assert!(!turn.waiting_for_capacity);
    assert!(
        observed::acknowledge(&db, &row, Utc::now())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc! {"event_type":"assistant_turn_capacity_wait", "event_data.conversation_id": &row.id})
        .await.unwrap(), 2);
    db.drop().await.unwrap();
}
