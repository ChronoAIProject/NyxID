use super::*;
use crate::services::{
    assistant_model_denial_notice as notice, assistant_nyxagent_runtime as runtime,
    platform_manager_contract as contract,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct NoModel(AtomicUsize);
#[async_trait::async_trait]
impl contract::NyxAgentEgress for NoModel {
    async fn request(
        &self,
        _: &contract::NyxAgentExecutionContext,
        _: &contract::RuntimeCredential,
        _: contract::NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(AppError::Internal(
            "model egress is forbidden in this fixture".into(),
        ))
    }
}
async fn settle(
    c: &Controlled,
    row: &crate::models::assistant_conversation::AssistantConversation,
    result: &engine::TurnResult,
    id: &str,
) {
    engine::finish_turn(&c.f.state.db, row, &row.credential_api_key_id, id, result)
        .await
        .unwrap();
}
fn ordinary(id: &str) -> engine::TurnStart {
    (&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: Some(id.into()),
        text: "separate owner request".into(),
        model: None,
        access_mode: None,
    })
        .into()
}
async fn begin_notice(
    c: &Controlled,
) -> crate::models::assistant_conversation::AssistantConversation {
    let mut start = ordinary(&c.f.row.id);
    start.origin = crate::models::assistant_conversation::TurnOrigin::Event;
    start.text.clear();
    engine::begin_turn(&c.f.state.db, &c.f.owner, start, &c.f.state.encryption_keys)
        .await
        .unwrap()
}
async fn execute(
    c: &Controlled,
    row: &crate::models::assistant_conversation::AssistantConversation,
    egress: &NoModel,
) -> Result<engine::TurnResult, engine::TurnError> {
    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        &c.f.state.db,
        &c.f.state.encryption_keys,
        &c.f.owner,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap();
    execute_with_credential(c, row, egress, credential).await
}
async fn execute_with_credential(
    c: &Controlled,
    row: &crate::models::assistant_conversation::AssistantConversation,
    egress: &NoModel,
    mut credential: crate::services::assistant_agent_credential_service::AssistantCredential,
) -> Result<engine::TurnResult, engine::TurnError> {
    let (sender, _) = tokio::sync::broadcast::channel(32);
    let mut events = runtime::Events {
        checkpoint: None,
        sender,
        cursor: 0,
        reset_notified: false,
    };
    Box::pin(runtime::execute_owner_turn(
        &c.f.state,
        egress,
        &runtime::owner_context(row).unwrap(),
        row,
        "server model-Deny notice",
        &mut credential,
        None,
        &mut events,
        "notice-reply",
        &mut String::new(),
        &mut None,
    ))
    .await
}
async fn deny(c: &Controlled) -> String {
    let card = c.observe().await;
    deny_card(c, card).await
}
async fn deny_card(c: &Controlled, card: String) -> String {
    let (a, b) = tokio::join!(
        acks::decide(&c.f.state.db, &c.f.owner, &c.f.row.id, &card, false),
        acks::decide(&c.f.state.db, &c.f.owner, &c.f.row.id, &card, false)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(
        acks::decide(&c.f.state.db, &c.f.owner, &c.f.row.id, &card, false)
            .await
            .is_err()
    );
    settle(
        c,
        &c.f.row,
        &engine::TurnResult {
            text: "Waiting for decision".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
        &uuid::Uuid::new_v4().to_string(),
    )
    .await;
    card
}

#[tokio::test]
async fn unknown_and_oversized_fixed_metadata_deny_settles_without_model_or_second_card() {
    for metadata in [
        json!({"unknown-provider-option":"opaque-value"}),
        json!({"x-codex-turn-metadata":"x".repeat(1025)}),
    ] {
        let c = Controlled::new("p2_opaque_metadata_deny").await;
        let mut candidate = request(vec![user("initial")]);
        candidate.body["client_metadata"] = metadata;
        let card = c.observe_request(candidate).await;
        let card = deny_card(&c, card).await;
        let row = begin_notice(&c).await;
        assert_eq!(row.active_turn.as_ref().unwrap().events[0].id, card);
        let egress = NoModel(AtomicUsize::new(0));
        let result = execute(&c, &row, &egress).await.unwrap();
        assert!(result.error.is_none());
        assert!(result.text.contains("You denied this model request"));
        assert_eq!(egress.0.load(Ordering::SeqCst), 0);
        let reply = uuid::Uuid::new_v4().to_string();
        settle(&c, &row, &result, &reply).await;
        settle(&c, &row, &result, &reply).await;
        notice::recover(&c.f.state.db).await.unwrap();
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_model_denial_notices")
                .count_documents(doc! {"_id":&card,"state":"settled","reply_id":&reply})
                .await
                .unwrap(),
            1
        );
        assert!(
            engine::get(&c.f.state.db, &c.f.owner, &row.id)
                .await
                .unwrap()
                .pending_events
                .is_empty()
        );
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn model_deny_notice_excludes_mixed_events_and_uses_one_durable_no_model_reply() {
    let c = Controlled::new("p2_model_deny_notice_queue").await;
    let ordinary_event = crate::services::assistant_team_service::bound_event(
        "message",
        "ordinary work".into(),
        None,
        None,
    );
    engine::push_events(
        &c.f.state.db,
        &c.f.owner,
        &c.f.row.id,
        vec![ordinary_event.clone()],
    )
    .await
    .unwrap();
    let card = deny(&c).await;
    notice::recover(&c.f.state.db).await.unwrap();
    // A real user turn drains its ordinary event while leaving the notice out
    // of model input. The queued notice then receives its own real event turn.
    let user_turn = engine::begin_turn(
        &c.f.state.db,
        &c.f.owner,
        ordinary(&c.f.row.id),
        &c.f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(user_turn.active_turn.as_ref().unwrap().events.len(), 1);
    assert_eq!(
        user_turn.active_turn.as_ref().unwrap().events[0].id,
        ordinary_event.id
    );
    assert_eq!(user_turn.pending_events.len(), 1);
    assert_eq!(user_turn.pending_events[0].id, card);
    settle(
        &c,
        &user_turn,
        &engine::TurnResult {
            text: "ordinary work settled".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
        &uuid::Uuid::new_v4().to_string(),
    )
    .await;
    // A later chat preference is deliberately different. It may not reroute a
    // saved app notice to the owner's current chat/home.
    c.f.state.db.collection::<Document>("assistant_conversations").update_one(doc!{"_id":&c.f.row.id},
        doc!{"$set":{"reply_channel":{"nyxbot_channel_id":"later-channel","partition":"later-chat","platform":"lark"}}}).await.unwrap();
    let row = begin_notice(&c).await;
    assert!(row.pending_events.is_empty());
    assert_eq!(row.active_turn.as_ref().unwrap().events.len(), 1);
    assert_eq!(row.active_turn.as_ref().unwrap().events[0].id, card);
    assert_eq!(
        engine::answer_routing(&row, row.active_turn.as_ref().unwrap()).primary,
        engine::PrimaryDestination::App
    );
    let egress = NoModel(AtomicUsize::new(0));
    let result = execute(&c, &row, &egress).await.unwrap();
    assert!(result.error.is_none());
    assert!(result.text.contains("You denied this model request"));
    assert_eq!(egress.0.load(Ordering::SeqCst), 0);
    let live = engine::get(&c.f.state.db, &c.f.owner, &row.id)
        .await
        .unwrap();
    assert!(
        live.active_turn
            .as_ref()
            .unwrap()
            .model_attempt_id
            .is_none()
    );
    assert_eq!(live.active_turn.as_ref().unwrap().model_requests_active, 0);
    let reply = uuid::Uuid::new_v4().to_string();
    settle(&c, &row, &result, &reply).await;
    settle(&c, &row, &result, &reply).await;
    notice::recover(&c.f.state.db).await.unwrap();
    notice::recover(&c.f.state.db).await.unwrap();
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_messages")
            .count_documents(doc! {"_id":&reply})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_model_denial_notices")
            .count_documents(doc! {"_id":&card,"state":"settled","reply_id":&reply})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(
        engine::get(&c.f.state.db, &c.f.owner, &row.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn model_deny_drained_worker_crash_requeues_same_notice_without_later_model_work() {
    let c = Controlled::new("p2_model_deny_notice_crash").await;
    let card = deny(&c).await;
    let crashed = begin_notice(&c).await;
    assert!(crashed.pending_events.is_empty());
    c.f.state.db.collection::<Document>("assistant_conversations").update_one(doc!{"_id":&crashed.id},
        doc!{"$set":{"active_turn.heartbeat_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(engine::ACTIVE_TURN_HEARTBEAT_STALE_SECS+1)),
        "active_turn.lease_expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    // Production runs the ordinary orphan settlement BEFORE notice recovery.
    let swept = engine::sweep_orphaned_turn_rows(&c.f.state.db)
        .await
        .unwrap();
    assert_eq!(swept.len(), 1);
    let failed =
        c.f.state
            .db
            .collection::<Document>("assistant_messages")
            .find_one(doc! {
            "turn_id":&crashed.active_turn.as_ref().unwrap().turn_id,"role":"assistant"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(failed.get_str("error_code").unwrap(), "turn_lost");
    assert_eq!(failed.get_str("text").unwrap(), "");
    notice::recover(&c.f.state.db).await.unwrap();
    notice::recover(&c.f.state.db).await.unwrap();
    let row = begin_notice(&c).await;
    assert_ne!(
        row.active_turn.as_ref().unwrap().turn_id,
        crashed.active_turn.as_ref().unwrap().turn_id
    );
    assert_eq!(row.active_turn.as_ref().unwrap().events.len(), 1);
    assert_eq!(row.active_turn.as_ref().unwrap().events[0].id, card);
    let egress = NoModel(AtomicUsize::new(0));
    let result = execute(&c, &row, &egress).await.unwrap();
    let reply = uuid::Uuid::new_v4().to_string();
    settle(&c, &row, &result, &reply).await;
    assert_eq!(egress.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_messages")
            .count_documents(doc! {"conversation_id":&row.id,
            "role":"assistant","text":&result.text,"status":"completed"})
            .await
            .unwrap(),
        1
    );
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn model_deny_stop_revocation_and_changed_source_fail_without_model_or_new_card() {
    for drift in ["stop", "revoke", "expiry", "source", "source_metadata"] {
        let c = Controlled::new("p2_model_deny_notice_fence").await;
        let card = deny(&c).await;
        let row = begin_notice(&c).await;
        let credential =
            crate::services::assistant_agent_credential_service::load_for_conversation(
                &c.f.state.db,
                &c.f.state.encryption_keys,
                &c.f.owner,
                &row.id,
            )
            .await
            .unwrap()
            .unwrap();
        match drift {
            "stop" => engine::request_stop(&c.f.state.db, &c.f.owner, &row.id)
                .await
                .unwrap(),
            "revoke" => {
                c.f.state
                    .db
                    .collection::<Document>("api_keys")
                    .update_one(
                        doc! {"_id":&row.credential_api_key_id},
                        doc! {"$set":{"is_active":false}},
                    )
                    .await
                    .unwrap();
            }
            "expiry" => {
                c.f.state.db.collection::<Document>("assistant_model_denial_notices").update_one(
                    doc!{"_id":&card},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
            }
            _ => {
                let saved =
                    c.f.state
                        .db
                        .collection::<Document>("assistant_model_denial_notices")
                        .find_one(doc! {"_id":&card})
                        .await
                        .unwrap()
                        .unwrap();
                c.f.state
                    .db
                    .collection::<Document>("assistant_messages")
                    .update_one(
                        doc! {"_id":saved.get_str("source_input_id").unwrap()},
                        doc! {"$set":if drift=="source_metadata" {
                            doc!{"platform_delivery_recovery_queued":true}
                        }else {doc!{"text":"changed saved source"}}},
                    )
                    .await
                    .unwrap();
            }
        }
        // Recovery cannot invalidate an admitted turn's shared failure
        // settlement, including when expiry/key retirement happens mid-turn.
        notice::recover(&c.f.state.db).await.unwrap();
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_model_denial_notices")
                .find_one(doc! {"_id":&card})
                .await
                .unwrap()
                .unwrap()
                .get_str("state")
                .unwrap(),
            "claimed"
        );
        let egress = NoModel(AtomicUsize::new(0));
        let error = execute_with_credential(&c, &row, &egress, credential)
            .await
            .unwrap_err();
        settle(
            &c,
            &row,
            &engine::TurnResult {
                text: String::new(),
                session_id: None,
                response_id: None,
                error: Some(error),
            },
            &uuid::Uuid::new_v4().to_string(),
        )
        .await;
        notice::recover(&c.f.state.db).await.unwrap();
        let final_notice =
            c.f.state
                .db
                .collection::<Document>("assistant_model_denial_notices")
                .find_one(doc! {"_id":&card})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(final_notice.get_str("state").unwrap(), "unsupported");
        assert!(
            engine::get(&c.f.state.db, &c.f.owner, &row.id)
                .await
                .unwrap()
                .active_turn
                .is_none()
        );
        assert_eq!(egress.0.load(Ordering::SeqCst), 0);
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn saved_service_deny_requires_complete_binding_and_keeps_proven_effect_explanation() {
    for drift in ["missing", "operation", "digest", "model", "effect"] {
        let c = Controlled::new("p2_legacy_deny_binding").await;
        let mut auth = c.f.auth.clone();
        auth.assistant_chat = Some(std::sync::Arc::new(c.f.chat.clone()));
        let model = drift != "effect";
        let bytes = serde_json::to_vec(&request(vec![user("initial")]).body).unwrap();
        let decision = Box::pin(
            crate::services::assistant_service_execution::check_http_projected(
                &c.f.state,
                &auth,
                Some(&c.connection),
                Some(&c.f.owner),
                &c.target,
                if model { "POST" } else { "DELETE" },
                if model { "responses" } else { "objects/saved" },
                None,
                &bytes,
                None,
                None,
                &HeaderMap::new(),
                &Default::default(),
            ),
        )
        .await
        .unwrap();
        let card = decision.refusal.unwrap()["acknowledgement_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let bindings =
            c.f.state
                .db
                .collection::<Document>(crate::services::assistant_service_execution::COLLECTION);
        match drift {
            "missing" => {
                bindings
                    .delete_one(doc! {"_id":&card,"kind":"card_binding"})
                    .await
                    .unwrap();
            }
            "operation" => {
                bindings
                    .update_one(
                        doc! {"_id":&card},
                        doc! {"$set":{"operation.kind":"unproven"}},
                    )
                    .await
                    .unwrap();
            }
            "digest" => {
                bindings
                    .update_one(
                        doc! {"_id":&card},
                        doc! {"$set":{"binding":"changed-binding"}},
                    )
                    .await
                    .unwrap();
            }
            _ => {}
        }
        acks::decide(&c.f.state.db, &c.f.owner, &c.f.row.id, &card, false)
            .await
            .unwrap();
        let saved =
            c.f.state
                .db
                .collection::<Document>("assistant_model_denial_notices")
                .find_one(doc! {"_id":&card})
                .await
                .unwrap();
        if drift == "effect" {
            assert!(
                saved.is_none(),
                "positively bound effect keeps existing explanation"
            );
        } else {
            assert_eq!(
                saved.unwrap().get_str("state").unwrap(),
                if drift == "model" {
                    "pending"
                } else {
                    "unsupported"
                }
            );
            assert!(
                notice::exists(&c.f.state.db, &card).await.unwrap(),
                "unsupported evidence blocks model-backed denial explanation"
            );
        }
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn saved_ssh_effect_denials_keep_existing_explanation_only_for_known_bound_server_shapes() {
    for kind in [
        "exec",
        "SSH certificate",
        "SSH terminal session",
        "SSH tunnel session",
    ] {
        let c = Controlled::new("p2_ssh_effect_deny").await;
        let connection = crate::services::user_service_service::find_user_service_by_id(
            &c.f.state.db,
            &c.connection,
        )
        .await
        .unwrap()
        .unwrap();
        let config: crate::models::downstream_service::SshServiceConfig =
            serde_json::from_value(json!({"host":"ssh.example.invalid","port":22})).unwrap();
        let refusal = if kind == "exec" {
            crate::services::assistant_native_execution::ssh_gate(
                &c.f.state,
                Some(&c.f.chat),
                &connection,
                &config,
                &json!({"command":"true"}),
            )
            .await
            .unwrap()
            .refusal
            .unwrap()
        } else {
            match crate::services::assistant_ssh_session::admit(
                &c.f.state,
                Some(&c.f.chat),
                Some(&connection),
                &config,
                &c.target.service.id,
                kind,
                json!({"principal":"test"}),
                None,
                60,
            )
            .await
            .unwrap()
            {
                Err(refusal) => refusal,
                Ok(_) => panic!("SSH effect requires a real action card"),
            }
        };
        let card = refusal["acknowledgement_id"].as_str().unwrap();
        acks::decide(&c.f.state.db, &c.f.owner, &c.f.row.id, card, false)
            .await
            .unwrap();
        assert!(
            !notice::exists(&c.f.state.db, card).await.unwrap(),
            "known SSH effect retains its explanation path"
        );
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        c.f.state.db.drop().await.unwrap();
    }
}
