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
            member_use: Default::default(),
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
            secret_context_versions: Default::default(),
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
    let alias = respond(&state, &agent_key, &body, "custom-second-key").await;
    assert!(alias.status().is_success() || alias.status() == StatusCode::CONFLICT);
    let _ = body_text(alias).await;
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    for dispatch_key in ["event-delivery-idempotency", "custom-second-key"] {
        let mut headers = bearer(&agent_key);
        headers.insert("idempotency-key", dispatch_key.parse().unwrap());
        let response = dispatches::get(
            State(state.clone()),
            headers,
            axum::extract::Query(dispatches::Query {
                dispatched_at: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let result: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert!(matches!(
            result["state"].as_str(),
            Some("in_progress" | "finished")
        ));
        assert_eq!(
            result["response_id"],
            dispatch_identity::custom_response(&event.id)
        );
    }

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

#[path = "nyxbot_mixed_event_bindings.rs"]
mod mixed;

#[tokio::test]
async fn org_event_binding_setup_and_retirement_preserve_the_key_rewrap_winner() {
    org_binding_outcomes(&["ready", "reject", "unbind", "revoke"]).await;
}

#[tokio::test]
async fn eligibility_revision_checks_custom_event_binding() {
    org_binding_outcomes(&["repromote"]).await;
}

async fn org_binding_outcomes(outcomes: &[&str]) {
    use crate::{
        models::{
            assistant_agent::{AgentKind, COLLECTION_NAME as AGENTS},
            org_membership::{COLLECTION_NAME as MEMBERS, OrgRole},
        },
        services::nyxbot_event_binding_service::{self as bindings, storage},
    };
    for &outcome in outcomes {
        let (mut state, _, upstream) = setup("event_binding_prep_race").await;
        let _upstream = tokio_util::task::AbortOnDropHandle::new(upstream);
        state.config.contextual_record_writes = false;
        state.encryption_keys = Arc::new(crate::crypto::aes::EncryptionKeys::from_config(
            &state.config,
        ));
        let keys = crate::test_utils::test_encryption_keys();
        let org = Uuid::new_v4().to_string();
        state
            .db
            .collection(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        state
            .db
            .collection(MEMBERS)
            .insert_one(crate::test_utils::test_membership(
                &org,
                OWNER,
                OrgRole::Member,
                None,
            ))
            .await
            .unwrap();
        let mut agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
            .await
            .unwrap();
        agent.id = Uuid::new_v4().to_string();
        agent.kind = AgentKind::Specialist;
        agent.name = "org-specialist".into();
        agent.user_id = org.clone();
        state
            .db
            .collection::<crate::models::assistant_agent::AssistantAgent>(AGENTS)
            .insert_one(&agent)
            .await
            .unwrap();
        let dest = Box::pin(mixed::destination_fixture(&state, &agent.id)).await;
        let mut remote = mixed::gateway(&state, true, outcome == "reject").await;
        let rows = state.db.collection::<bson::Document>(EVENT_BINDINGS);
        let setup = Box::pin(bindings::create(&state, OWNER, &agent.id, &dest, PARTITION));
        let race = async {
            let _hashes =
                tokio::time::timeout(std::time::Duration::from_secs(8), remote.entered.recv())
                    .await
                    .unwrap()
                    .unwrap();
            let source = rows
                .find_one(doc! {"status":"preparing"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(source.get_str("user_id"), Ok(OWNER));
            assert_eq!(source.get_str("agent_owner_id"), Ok(org.as_str()));
            let id = source.get_str("_id").unwrap();
            if outcome == "unbind" {
                let row = bindings::get(&state, OWNER, id).await.unwrap();
                Box::pin(bindings::unbind_provider(&state, &row))
                    .await
                    .unwrap();
                assert!(!storage::migrate(&state.db, &keys, &source).await.unwrap());
            } else {
                assert!(storage::migrate(&state.db, &keys, &source).await.unwrap());
                let rewrapped = rows.find_one(doc! {"_id":id}).await.unwrap().unwrap();
                storage::verify(&state.db, &keys, &rewrapped).await.unwrap();
                rows.update_one(doc! {"_id":id}, doc! {"$set":{"unrelated_sibling":"kept"}})
                    .await
                    .unwrap();
            }
            remote.resume.notify_one();
            source
        };
        let (result, source) = tokio::join!(setup, Box::pin(race));
        let id = source.get_str("_id").unwrap();
        let row = bindings::get(&state, OWNER, id).await.unwrap();
        let raw = rows.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        if outcome != "unbind" {
            assert_eq!(raw.get_str("unrelated_sibling"), Ok("kept"));
        }
        if matches!(outcome, "ready" | "revoke" | "repromote") {
            assert!(result.is_ok());
            Box::pin(bindings::authorize_provider(&state, &row))
                .await
                .unwrap();
            storage::verify(&state.db, &keys, &raw).await.unwrap();
            let debug = format!("{row:?}");
            assert!(!debug.contains("ciphertext") && !debug.contains(&row.producer_key_id));
            if outcome == "repromote" {
                let membership =
                    crate::services::org_service::get_active_membership(&state.db, &org, OWNER)
                        .await
                        .unwrap()
                        .unwrap();
                for role in [OrgRole::Viewer, OrgRole::Member] {
                    crate::services::org_service::update_membership(
                        &state.db,
                        &membership.id,
                        Some(role),
                        None,
                        None,
                    )
                    .await
                    .unwrap();
                }
                assert!(bindings::authorize_provider(&state, &row).await.is_err());
                remote.resume.notify_one();
                let (fresh, _, _) = bindings::create(&state, OWNER, &agent.id, &dest, PARTITION)
                    .await
                    .unwrap();
                assert_eq!(fresh.member_use.eligibility_revision, 1);
                bindings::authorize_provider(&state, &fresh).await.unwrap();
                continue;
            }
            if outcome == "ready" {
                let agents = state.db.collection::<bson::Document>(AGENTS);
                agents
                    .update_one(
                        doc! {"_id": &agent.id},
                        doc! {"$set": {"org_user_ids": [], "org_use_revision": 1_i64}},
                    )
                    .await
                    .unwrap();
                assert!(bindings::authorize_provider(&state, &row).await.is_err());
                agents
                    .update_one(
                        doc! {"_id": &agent.id},
                        doc! {"$set": {"org_user_ids": [OWNER]}},
                    )
                    .await
                    .unwrap();
                assert!(bindings::authorize_provider(&state, &row).await.is_err());
                continue;
            }
            Box::pin(bindings::revoke(&state, OWNER, id)).await.unwrap();
        } else {
            assert!(result.is_err());
        }
        let retired = rows.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        assert_eq!(
            retired.get_str("status"),
            Ok(if outcome == "reject" {
                "failed"
            } else {
                "revoked"
            })
        );
        for field in ["provider_key_ciphertext", "producer_secret_ciphertext"] {
            assert!(retired.get(field).is_none_or(|v| v == &bson::Bson::Null));
            assert!(
                retired
                    .get_document("secret_context_versions")
                    .ok()
                    .is_none_or(|m| !m.contains_key(field))
            );
        }
        assert!(!storage::migrate(&state.db, &keys, &source).await.unwrap());
        storage::verify(&state.db, &keys, &retired).await.unwrap();
        assert!(retired == rows.find_one(doc! {"_id":id}).await.unwrap().unwrap());
    }
}
