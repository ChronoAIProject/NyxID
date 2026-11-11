use super::*;
use futures::TryStreamExt;

async fn audits(state: &AppState) -> Vec<bson::Document> {
    state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":"nyxbot_provider_refused"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn assert_audit(
    state: &AppState,
    channel: &str,
    endpoint: &str,
    status: i32,
    code: &str,
) -> bson::Document {
    let rows = audits(state).await;
    let matching: Vec<_> = rows
        .iter()
        .filter(|r| {
            let data = r.get_document("event_data").unwrap();
            data.get_str("endpoint") == Ok(endpoint) && data.get_str("code") == Ok(code)
        })
        .collect();
    assert_eq!(matching.len(), 1, "{rows:?}");
    let row = matching[0];
    assert_eq!(row.get_str("user_id").unwrap(), OWNER);
    assert!(row.contains_key("entry_hash"));
    let data = row.get_document("event_data").unwrap();
    assert_eq!(data.get_str("channel_agent_id").unwrap(), channel);
    assert_eq!(
        data.get_i64("status")
            .or_else(|_| data.get_i32("status").map(i64::from))
            .unwrap(),
        i64::from(status)
    );
    assert!(data.get_bool("partition_known").is_ok());
    data.clone()
}

#[tokio::test]
async fn provider_refused_each_endpoint_private_and_group_metadata_and_throttle() {
    crate::services::coordination_service::TEST_RATE_WINDOW_NOW
        .scope(
            std::cell::Cell::new(bson::DateTime::now()),
            endpoint_cases(),
        )
        .await;
}
async fn endpoint_cases() {
    for group in [false, true] {
        let (state, _, server) = setup("provider_refusal_endpoints").await;
        let (row, key) = channel(&state, "gateway").await;
        let binding = "bnd_secret-binding";
        let response = put_binding(
            State(state.clone()),
            Path(binding.into()),
            bearer(&key),
            Json(binding_body(&key, OWNER)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut bad = binding_body(&key, OWNER);
        bad["agent_key"] = json!("secret-wrong-token");
        let response = put_binding(
            State(state.clone()),
            Path(binding.into()),
            bearer(&key),
            Json(bad),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_audit(&state, &row.id, "binding_put", 403, "forbidden").await;
        let mut first = None;
        // Simulate two replicas: shared database, independent handler scopes.
        for _ in 0..2 {
            let response = put_conversation(
                State(state.clone()),
                Path(("bnd_wrong-secret".into(), PARTITION.into())),
                bearer(&key),
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            first = Some(body_text(response).await);
        }
        assert!(first.unwrap().contains("binding_not_found"));
        let data = assert_audit(
            &state,
            &row.id,
            "conversation_put",
            404,
            "binding_not_found",
        )
        .await;
        assert_eq!(data.get_bool("binding_match"), Ok(false));
        assert_eq!(data.get_bool("partition_known"), Ok(false));
        let body = if group {
            group_event(
                "private-message-content",
                "secret-sender",
                "Guest",
                "secret-event",
                PARTITION,
                true,
            )
        } else {
            event("private-message-content", "secret-sender", "secret-event")
        };
        let response = respond(&state, &key, &body, "secret-idempotency").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let data = assert_audit(&state, &row.id, "responses", 404, "conversation_not_found").await;
        assert_eq!(data.get_str("event_correlation_hash").unwrap().len(), 64);
        let response = get_event_context(
            State(state.clone()),
            Path((binding.into(), PARTITION.into(), "secret-event".into())),
            bearer(&key),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_audit(&state, &row.id, "events", 404, "event_not_found").await;
        let response = native_responses::snapshot(
            State(state.clone()),
            Path("resp_missing".into()),
            bearer(&key),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_audit(&state, &row.id, "response_get", 404, "response_not_found").await;
        let response = native_responses::events(
            State(state.clone()),
            Path("resp_missing".into()),
            axum::extract::Query(native_responses::Cursor {
                starting_after: None,
            }),
            bearer(&key),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_audit(&state, &row.id, "events", 404, "response_not_found").await;
        let response = native_responses::events(
            State(state.clone()),
            Path("resp_missing".into()),
            axum::extract::Query(native_responses::Cursor {
                starting_after: Some("secret-invalid-cursor".into()),
            }),
            bearer(&key),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_audit(&state, &row.id, "events", 400, "validation_error").await;
        let rows = audits(&state).await;
        assert_eq!(rows.len(), 7);
        let encoded = serde_json::to_string(&rows).unwrap();
        for secret in [
            &key,
            binding,
            "bnd_wrong-secret",
            "secret-sender",
            "secret-event",
            "secret-idempotency",
            "private-message-content",
            "secret-invalid-cursor",
            "secret-wrong-token",
        ] {
            assert!(!encoded.contains(secret), "audit leaked a request value");
        }
        // A new fixed minute can append again; time is request-local in tests.
        crate::services::coordination_service::TEST_RATE_WINDOW_NOW.with(|now| {
            now.set(bson::DateTime::from_millis(
                now.get().timestamp_millis() + 60_000,
            ));
        });
        let response = put_conversation(
            State(state.clone()),
            Path(("bnd_wrong-secret".into(), PARTITION.into())),
            bearer(&key),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(audits(&state).await.len(), 8);
        server.abort();
        state.db.drop().await.unwrap();
    }
}

// Force a real Mongo read error in one isolated collection while leaving keys
// and chained audit writes available. No global failpoint or server disruption.
async fn fail_reads(state: &AppState, collection: &str) {
    state
        .db
        .collection::<bson::Document>(collection)
        .drop()
        .await
        .unwrap();
    let source = format!("fault_{collection}");
    state
        .db
        .collection::<bson::Document>(&source)
        .insert_one(doc! {"_id":"fault"})
        .await
        .unwrap();
    state.db.run_command(doc! {"create":collection,"viewOn":source,"pipeline":[{"$replaceWith":{"$toInt":"$_id"}}]}).await.unwrap();
}

#[tokio::test]
async fn provider_refused_storage_errors_are_503_not_authority_or_missing() {
    for personal_routes in [true, false] {
        let (state, _, server) = setup("provider_refusal_storage").await;
        let (row, key) = channel(&state, "gateway").await;
        let response = put_binding(
            State(state.clone()),
            Path("bnd_live".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        if personal_routes {
            fail_reads(&state, crate::models::platform_bot_config::COLLECTION_NAME).await;
        } else {
            fail_reads(&state, THREADS).await;
        }
        let response = respond(&state, &key, &event("secret", "7", "evt"), "evt").await;
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{}",
            body_text(response).await
        );
        assert_audit(&state, &row.id, "responses", 503, "provider_unavailable").await;
        server.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn provider_refused_native_marker_and_revoked_key_stay_closed() {
    let (state, _, server) = setup("provider_refusal_authority").await;
    let (row, key) = channel(&state, "gateway").await;
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"platform_entry_version":1}},
        )
        .await
        .unwrap();
    let response = put_conversation(
        State(state.clone()),
        Path(("bnd_live".into(), PARTITION.into())),
        bearer(&key),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        body_text(response)
            .await
            .contains("shared_entry_requires_native_receipt")
    );
    let data = assert_audit(
        &state,
        &row.id,
        "conversation_put",
        403,
        "shared_entry_requires_native_receipt",
    )
    .await;
    assert_eq!(data.get_str("binding_match"), Ok("unknown"));
    key_service::delete_api_key(&state.db, OWNER, row.agent_api_key_id.as_deref().unwrap())
        .await
        .unwrap();
    for headers in [bearer(&key), HeaderMap::new(), bearer("unknown-secret")] {
        let response = put_conversation(
            State(state.clone()),
            Path(("bnd_live".into(), PARTITION.into())),
            headers,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(body_text(response).await.contains("not_authenticated"));
    }
    assert_eq!(
        audits(&state).await.len(),
        1,
        "unresolved callers cannot flood owner audit"
    );
    let raced = provider_refusal::scope(
        &state.db,
        provider_refusal::Endpoint::ConversationPut,
        async {
            // A valid first probe must not authorize auditing a later unresolved 401.
            provider_refusal::owner(OWNER);
            problem(StatusCode::UNAUTHORIZED, "not_authenticated")
        },
    )
    .await;
    assert_eq!(raced.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(audits(&state).await.len(), 1);

    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn provider_refused_known_partition_and_binding_are_distinct() {
    let (state, _, server) = setup("provider_refusal_partition").await;
    let (row, key) = channel(&state, "gateway").await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_live".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        put_conversation(
            State(state.clone()),
            Path(("bnd_live".into(), PARTITION.into())),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::OK
    );
    let mut body = event("never log this", "7", "event-correlation");
    body["event_context"]["conversation_id"] = json!("wrong");
    let response = respond(&state, &key, &body, "event-correlation").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let data = assert_audit(&state, &row.id, "responses", 400, "invalid_request").await;
    assert_eq!(data.get_bool("partition_known"), Ok(true));
    let response = put_conversation(
        State(state.clone()),
        Path(("bnd_live".into(), "invalid-partition".into())),
        bearer(&key),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let data = assert_audit(
        &state,
        &row.id,
        "conversation_put",
        404,
        "binding_not_found",
    )
    .await;
    assert_eq!(data.get_bool("binding_match"), Ok(true));
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn provider_refused_cold_telegram_new_guest_then_established_owner_one_turn_each() {
    use crate::services::assistant_team_service::{CreateRequest, create_specialist};
    let (state, calls, server) = setup("provider_refusal_cold").await;
    let (agent, _) = create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        CreateRequest {
            machines: None,
            logins: None,
            name: "support".into(),
            description: "Customer support".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let (row, key) = channel(&state, "gateway").await;
    link(&state, OWNER, &row.id, &agent).await.unwrap();
    // Production's canonical channel platform is telegram even for a telegram-new bot.
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"platform":"telegram-new"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"private_chats":"everyone","owner_sender_ids":["7"]}},
        )
        .await
        .unwrap();
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_current".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    let guest = event("hello", "42", "guest-event");
    // Fake gateway protocol: missing partition -> ensure once -> retry same event.
    let response = respond(&state, &key, &guest, "guest-event").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(body_text(response).await.contains("conversation_not_found"));
    assert!(calls.lock().await.is_empty());
    assert_eq!(
        put_conversation(
            State(state.clone()),
            Path(("bnd_current".into(), PARTITION.into())),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::OK
    );
    let response = respond(&state, &key, &guest, "guest-event").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("Here is your answer"));
    assert_eq!(calls.lock().await.len(), 1);
    let saved = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"user_id":OWNER,"guest_turn":true,"agent_id":&agent.id})
        .await
        .unwrap()
        .unwrap();
    settle(&state, &saved.id).await;
    assert!(
        body_text(respond(&state, &key, &guest, "guest-event").await)
            .await
            .contains("response.completed")
    );
    assert_eq!(
        calls.lock().await.len(),
        1,
        "gateway retry must not duplicate the turn"
    );
    let owner_partition = "conv_11111111111111111111111111111111";
    assert_eq!(
        put_conversation(
            State(state.clone()),
            Path(("bnd_current".into(), owner_partition.into())),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::OK
    );
    let mut owner = event("hello owner", "7", "owner-event");
    owner["conversation"] = json!(owner_partition);
    owner["event_context"]["conversation_id"] = json!(owner_partition);
    owner["event_context"]["activity"]["conversation"]["id"] = json!("owner-chat");
    let response = respond(&state, &key, &owner, "owner-event").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("Here is your answer"));
    assert_eq!(calls.lock().await.len(), 2);
    let owner_thread = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(
            doc! {"user_id":OWNER,"guest_turn":false,"reply_channel.nyxbot_channel_id":&row.id},
        )
        .await
        .unwrap()
        .unwrap();
    settle(&state, &owner_thread.id).await;
    assert!(
        body_text(respond(&state, &key, &owner, "owner-event").await)
            .await
            .contains("response.completed")
    );
    assert_eq!(calls.lock().await.len(), 2);
    assert_eq!(
        audits(&state).await.len(),
        1,
        "success creates no refusal audit"
    );
    // Provider unbind is not proof of terminal deletion. Retain the current
    // binding and keys when the gateway still has this record, so both the
    // established owner and a cold guest can continue without reconnecting.
    let unbound = delete_binding(
        State(state.clone()),
        Path("bnd_current".into()),
        bearer(&key),
    )
    .await;
    assert_eq!(unbound.status(), StatusCode::NO_CONTENT);
    let after_unbind = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(after_unbind.status, "active");
    assert_eq!(after_unbind.binding_id.as_deref(), Some("bnd_current"));
    assert!(key_service::validate_api_key(&state.db, &key).await.is_ok());
    assert_eq!(after_unbind.agent_api_key_id, row.agent_api_key_id);
    assert_eq!(after_unbind.route_api_key_id, row.route_api_key_id);
    owner["event_context"]["activity"]["event_id"] = json!("owner-after-drift");
    let response = respond(&state, &key, &owner, "owner-after-drift").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("Here is your answer"));
    settle(&state, &owner_thread.id).await;
    assert_eq!(calls.lock().await.len(), 3);
    let cold_partition = "conv_22222222222222222222222222222222";
    let mut cold = event("cold guest", "43", "guest-after-drift");
    cold["conversation"] = json!(cold_partition);
    cold["event_context"]["conversation_id"] = json!(cold_partition);
    assert_eq!(
        respond(&state, &key, &cold, "guest-after-drift")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let refused = put_conversation(
        State(state.clone()),
        Path(("bnd_current".into(), cold_partition.into())),
        bearer(&key),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::OK);
    let answer = respond(&state, &key, &cold, "guest-after-drift-retry").await;
    assert_eq!(answer.status(), StatusCode::OK);
    assert!(body_text(answer).await.contains("Here is your answer"));
    assert_eq!(
        calls.lock().await.len(),
        4,
        "one cold guest turn admitted after retained unbind"
    );
    assert!(!audits(&state).await.iter().any(|audit| {
        audit.get_document("event_data").unwrap().get_str("code") == Ok("binding_not_found")
    }));

    server.abort();
    state.db.drop().await.unwrap();
}
