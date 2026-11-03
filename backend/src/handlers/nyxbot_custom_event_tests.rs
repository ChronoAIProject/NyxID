use super::*;
use crate::models::nyxbot_event_binding::{COLLECTION_NAME as EVENT_BINDINGS, NyxbotEventBinding};
use crate::services::feature_flag_service::{self as flags, FlagTarget};

async fn custom_binding(
    state: &AppState,
    row: &NyxbotChannel,
    id: &str,
    provider_key_id: &str,
    gateway_channel_id: &str,
) {
    let now = Utc::now();
    state
        .db
        .collection::<NyxbotEventBinding>(EVENT_BINDINGS)
        .insert_one(NyxbotEventBinding {
            id: id.into(),
            user_id: OWNER.into(),
            agent_id: row.agent_id.clone().unwrap(),
            agent_owner_id: OWNER.into(),
            destination_channel_id: row.id.clone(),
            destination_partition: PARTITION.into(),
            partition_policy: "per_event".into(),
            provider_binding_id: Some(id.into()),
            provider_api_key_id: Some(provider_key_id.into()),
            provider_key_ciphertext: None,
            producer_secret_ciphertext: None,
            producer_key_id: format!("producer-{id}"),
            gateway_channel_id: Some(gateway_channel_id.into()),
            custom_event_url: Some(format!(
                "https://gateway.example/channels/{gateway_channel_id}/events"
            )),
            gateway_record_id: Some(format!("record-{id}")),
            gateway_version: Some(1),
            status: "ready".into(),
            last_error: None,
            revoked_at: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

fn custom_binding_body(agent_key: &str, agent_id: &str) -> Value {
    let mut body = binding_body(agent_key, OWNER);
    body["profile"]["permission"]["action_mode"] = json!("guarded");
    body["target"] = json!({"type": "existing_agent", "agent_id": agent_id});
    body
}

fn custom_event(event_id: &str, channel_id: &str, conversation_id: &str) -> Value {
    json!({
        "conversation": conversation_id,
        "stream": true,
        "input": [{"role":"user", "content":[{"type":"input_text", "text":"build completed"}]}],
        "event_context": {
            "schema_version": 1,
            "channel_id": channel_id,
            "conversation_id": conversation_id,
            "activity": {
                "schema_version": 1,
                "event_id": event_id,
                "source": {"type":"custom"},
                "actor": {"id":"business.example","kind":"system"},
                "conversation": {"id":format!("subject-{event_id}"),"kind":"custom","thread_id":null},
                "kind": {"type":"event","ce":{"id":event_id,"source":"business.example","type":"build.completed"}}
            },
            "event_ref": "cmaeg1.ev.custom-test"
        }
    })
}

#[tokio::test]
async fn custom_provider_key_isolated_for_two_channels_on_one_specialist() {
    let (state, _, server) = setup("nyxbot_custom_binding_isolation").await;
    let (agent, _) = crate::services::assistant_team_service::create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        crate::services::assistant_team_service::CreateRequest {
            machines: None,
            logins: None,
            name: "custom-events".into(),
            description: "Receives independent custom events".into(),
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
    let (mut row, key_a) = channel(&state, "direct").await;
    row.agent_id = Some(agent.id.clone());
    row.gateway_channel_id = Some("cmaeg1.custom-a".into());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"agent_id": &agent.id, "gateway_channel_id": "cmaeg1.custom-a"}},
        )
        .await
        .unwrap();
    let key_b = key(&state, "custom-b").await;
    custom_binding(
        &state,
        &row,
        "binding-a",
        &row.agent_api_key_id.clone().unwrap(),
        "cmaeg1.custom-a",
    )
    .await;
    custom_binding(&state, &row, "binding-b", &key_b.id, "cmaeg1.custom-b").await;
    flags::set_platform_override(
        &state.db,
        flags::NYXBOT_CUSTOM_EVENT_GATEWAY_FLAG_KEY,
        &FlagTarget::User(OWNER.into()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    let foreign_target = put_binding(
        State(state.clone()),
        Path("binding-a".into()),
        bearer(&key_a),
        Json(custom_binding_body(&key_a, &Uuid::new_v4().to_string())),
    )
    .await;
    assert_eq!(foreign_target.status(), StatusCode::FORBIDDEN);

    let bound = put_binding(
        State(state.clone()),
        Path("binding-a".into()),
        bearer(&key_a),
        Json(custom_binding_body(&key_a, &agent.id)),
    )
    .await;
    assert_eq!(bound.status(), StatusCode::OK);

    // A provider key from binding B cannot submit an event whose verified
    // channel identity belongs to binding A.
    let wrong = respond(
        &state,
        &key_b.full_key,
        &custom_event("event-a", "cmaeg1.custom-a", "conv-custom-a"),
        "event-a",
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);

    state
        .db
        .collection::<NyxbotEventBinding>(EVENT_BINDINGS)
        .update_one(
            doc! {"_id": "binding-a"},
            doc! {"$set": {"status": "revoked"}},
        )
        .await
        .unwrap();
    let revoked = put_binding(
        State(state.clone()),
        Path("binding-a".into()),
        bearer(&key_a),
        Json(custom_binding_body(&key_a, &agent.id)),
    )
    .await;
    assert_eq!(revoked.status(), StatusCode::FORBIDDEN);
    server.abort();
}

#[tokio::test]
async fn custom_event_retries_share_one_turn_and_preserve_the_pinned_chat_anchor() {
    let (state, _, server) =
        setup_with_reply("nyxbot_custom_event_delivery", "custom answer").await;
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let (mut row, agent_key) = channel(&state, "direct").await;
    row.agent_id = Some(agent.id.clone());
    row.gateway_channel_id = Some("cmaeg1.custom-delivery".into());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"agent_id": &agent.id, "gateway_channel_id": "cmaeg1.custom-delivery"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(THREADS)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(),
            "channel_id": &row.id,
            "user_id": OWNER,
            "partition": PARTITION,
            "kind": "private",
            "owner_chat": true,
            "agent_id": &agent.id,
            "last_message_id": "source-message-1",
            "platform_chat_id": "destination-chat",
            "created_at": bson::DateTime::now(),
            "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let key_id = row.agent_api_key_id.clone().unwrap();
    custom_binding(
        &state,
        &row,
        "binding-delivery",
        &key_id,
        "cmaeg1.custom-delivery",
    )
    .await;
    flags::set_platform_override(
        &state.db,
        flags::NYXBOT_CUSTOM_EVENT_GATEWAY_FLAG_KEY,
        &FlagTarget::User(OWNER.into()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    let bound = put_binding(
        State(state.clone()),
        Path("binding-delivery".into()),
        bearer(&agent_key),
        Json(custom_binding_body(&agent_key, &agent.id)),
    )
    .await;
    assert_eq!(bound.status(), StatusCode::OK);

    let body = custom_event(
        "event-delivery-1",
        "cmaeg1.custom-delivery",
        "cmaeg1.conv.event-delivery-1",
    );
    let first = respond(&state, &agent_key, &body, "event-delivery-idempotency").await;
    let first_status = first.status();
    let first_bytes = axum::body::to_bytes(first.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let first_text = String::from_utf8_lossy(&first_bytes);
    assert_eq!(
        first_status,
        StatusCode::OK,
        "custom response: {first_text}"
    );
    assert!(first_text.contains("response.completed"));
    assert!(!first_text.contains(&agent_key));

    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"custom_binding_id": "binding-delivery", "event_id": "event-delivery-1"})
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event.status.as_str(), "completed" | "queued"));
    let conversation = event.conversation_id.clone().unwrap();
    let messages_before = state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .count_documents(doc! {"conversation_id": &conversation, "role": "assistant"})
        .await
        .unwrap();

    let retry = respond(&state, &agent_key, &body, "event-delivery-idempotency").await;
    assert!(matches!(
        retry.status(),
        StatusCode::OK | StatusCode::CONFLICT
    ));
    let _ = axum::body::to_bytes(retry.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let messages_after = state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .count_documents(doc! {"conversation_id": &conversation, "role": "assistant"})
        .await
        .unwrap();
    assert_eq!(messages_after, messages_before);
    assert_eq!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(
                doc! {"custom_binding_id": "binding-delivery", "event_id": "event-delivery-1"}
            )
            .await
            .unwrap(),
        1
    );
    server.abort();
}
