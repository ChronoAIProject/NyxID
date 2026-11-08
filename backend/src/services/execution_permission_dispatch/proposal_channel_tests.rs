use super::*;
use crate::models::{
    assistant_conversation::ChannelOrigin,
    execution_permission_channel::{COLLECTION_NAME as DELIVERIES, Delivery},
    nyxbot_channel::{ChannelTurnDelivery, NyxbotEvent},
};
use crate::services::execution_permission_channel as channel;

async fn channel_turn(name: &str, native: bool) -> (Fixture, AssistantConversation, String) {
    channel_turn_transport(name, if native { "native" } else { "gateway" }).await
}
async fn channel_turn_transport(
    name: &str,
    transport: &str,
) -> (Fixture, AssistantConversation, String) {
    let native = transport == "native";
    let sealed = transport == "sealed";
    let transport = if native || sealed {
        "gateway"
    } else {
        transport
    };
    let (mut f, previous) = assistant_for_cohort(name, true).await;
    channel::deliveries::ensure_indexes(&f.state.db)
        .await
        .unwrap();
    finish(&f, &previous, "Prior web turn").await;
    let id = Uuid::new_v4().to_string();
    let bot = Uuid::new_v4().to_string();
    let route = Uuid::new_v4().to_string();
    let thread = Uuid::new_v4().to_string();
    let event = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    let db = &f.state.db;
    db.collection::<Document>("channel_bots").insert_one(doc!{"_id":&bot,"user_id":&f.owner,"platform":if native {"discord"} else {"telegram"},"platform_bot_id":"100","label":"Owner bot","platform_bot_username":"owner_bot","webhook_registered":true,"webhook_secret_hash":"hash","bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1]},"is_active":true,"status":"active","created_at":now,"updated_at":now}).await.unwrap();
    db.collection::<Document>("channel_conversations").insert_one(doc!{"_id":&route,"user_id":&f.owner,"channel_bot_id":&bot,"platform":if native {"discord"} else {"telegram"},"platform_conversation_id":"200","platform_conversation_type":"private","agent_api_key_id":&f.key,"is_active":true,"created_at":now,"updated_at":now}).await.unwrap();
    let mut c = doc! {"_id":&id,"user_id":&f.owner,"channel_bot_id":&bot,"platform":"telegram","bot_label":"Owner bot","status":"active","transport":transport,"route_api_key_id":&f.key,"route_id":&route,"owner_sender_ids":["7"],"agent_id":&previous.agent_id,"created_at":now,"updated_at":now};
    c.insert("agent_api_key_id", &f.key);
    c.insert(
        "agent_key_ciphertext",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: f
                .state
                .encryption_keys
                .encrypt(f.raw.as_bytes())
                .await
                .unwrap(),
        },
    );
    c.insert("binding_id", "bound");
    if native {
        c.insert("platform", "discord");
        c.insert("gateway_source", doc! {"type":"discord","bot_id":"100"});
        c.insert("gateway_native_phase", "ready");
        c.insert("gateway_channel_id", "native-channel");
        c.insert("gateway_bot_id", "100");
    }
    db.collection::<Document>("nyxbot_channels")
        .insert_one(c)
        .await
        .unwrap();
    db.collection::<Document>("nyxbot_threads").insert_one(doc!{"_id":&thread,"user_id":&f.owner,"channel_id":&id,"partition":"original-chat","conversation_id":&previous.id,"kind":"private","owner_chat":true,"created_at":now,"updated_at":now}).await.unwrap();
    let origin = ChannelOrigin {
        org_execution: None,
        native_event_id: native.then(|| event.clone()),
        thread: None,
        nyxbot_channel_id: id.clone(),
        partition: "original-chat".into(),
        platform: if native { "discord" } else { "telegram" }.into(),
    };
    let delivery = ChannelTurnDelivery {
        execution_source_bound: false,
        execution_owner_verified: true,
        sender_kind: Some(crate::models::channel_thread::ThreadSenderKind::Human),
        version: 1,
        state: if native { "replay" } else { "streamed" }.into(),
        origin: origin.clone(),
        sender_id: "7".into(),
        guest: false,
        addressed: true,
        transport: transport.into(),
        source_route_id: Some(route.clone()),
        source_route_api_key_id: Some(f.key.clone()),
        stream_deadline: Utc::now() + chrono::Duration::seconds(570),
        checked_at: Utc::now(),
        claim_id: None,
        target_ciphertext: None,
        target_expires_at: None,
    };
    db.collection::<Document>("nyxbot_events").insert_one(doc!{"_id":&event,"channel_id":&id,"user_id":&f.owner,"partition":"original-chat","event_id":"original-provider-event","delivery":bson::to_bson(&delivery).unwrap(),"status":"running","created_at":now,"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(24))}).await.unwrap();
    if sealed {
        let target = f
            .state
            .encryption_keys
            .encrypt(b"original-target")
            .await
            .unwrap();
        db.collection::<Document>("nyxbot_events").update_one(doc!{"_id":&event},doc!{"$set":{
            "created_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(1)),
            "delivery.target_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:target},
            "delivery.target_expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(22))
        }}).await.unwrap();
    }
    let context = if native {
        json!({"schema_version":1,"channel_id":"native-channel","event_ref":"original-native-ref","activity":{"schema_version":1,"event_id":"discord:100:500","source":{"type":"discord","bot_id":"100"},"actor":{"kind":"human","id":"7"},"conversation":{"kind":"private","id":"200","thread_id":null},"kind":{"type":"message","text":"Update sheet","mentions_bot":false,"attachments":[]}}})
    } else {
        json!({"event_ref":"original-relay-ref"})
    };
    let cipher = f
        .state
        .encryption_keys
        .encrypt(&serde_json::to_vec(&context).unwrap())
        .await
        .unwrap();
    db.collection::<Document>("nyxbot_events").update_one(doc!{"_id":&event},doc!{"$set":{"event_context_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:cipher}}}).await.unwrap();
    if native {
        db.collection::<Document>("nyxbot_gateway_responses").insert_one(doc!{"_id":Uuid::new_v4().to_string(),"response_id":"original-response","user_id":&f.owner,"channel_id":&id,"binding_id":"bound","partition":"original-chat","event_key":&event,"request_hash":"hash","conversation_id":&previous.id,"terminal":false,"revision":1_i64,"payload_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1]},"created_at":now,"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(24))}).await.unwrap();
    }
    if transport == "direct" {
        let token = f
            .state
            .encryption_keys
            .encrypt(b"test-token")
            .await
            .unwrap();
        db.collection::<Document>("channel_bots").update_one(doc!{"_id":&bot},doc!{"$set":{"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:token}}}).await.unwrap();
        db.collection::<Document>("channel_messages").insert_one(doc!{"_id":"original-provider-event","conversation_id":&route,"channel_bot_id":&bot,"user_id":&f.owner,"direction":"inbound","platform":"telegram","platform_conversation_id":"200","platform_message_id":"500","sender_platform_id":"7","agent_api_key_id":&f.key,"content_type":"text","thread_id":"77","created_at":now}).await.unwrap();
    }
    let request = engine::TurnRequest {
        conversation_id: Some(previous.id.clone()),
        text: "Update this sheet".into(),
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&request).into();
    start.origin = TurnOrigin::Channel;
    start.execution_enrollment = true;
    start.channel_event_id = Some(event.clone());
    start.reply_channel = Some(origin);
    let row = engine::begin_turn(db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    f.key = row.credential_api_key_id.clone();
    assert!(
        row.active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .channel_source
            .is_some()
    );
    (f, row, event)
}

async fn channel_proposal(f: &Fixture) -> ApprovalRequest {
    f.state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"execution_proposal.version":4})
        .await
        .unwrap()
        .unwrap()
}
async fn approve(f: &Fixture, request: &ApprovalRequest, choice: &str) {
    let response = crate::handlers::approvals::decide_execution_request(
        axum::extract::State(f.state.clone()),
        test_utils::test_auth_user(&f.owner),
        axum::extract::Path(request.id.clone()),
        axum::Json(decision(request, choice)),
    )
    .await
    .unwrap();
    assert_eq!(response.0.id, request.id);
    assert_eq!(response.0.status, "approved");
}
async fn delivery(f: &Fixture) -> Delivery {
    f.state
        .db
        .collection::<Delivery>(DELIVERIES)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn migration_channel_review_resume_and_result_occurrence_keep_original_source() {
    let (f, row, event) = channel_turn("migration_channel_review", false).await;
    let original = f
        .state
        .db
        .collection::<NyxbotEvent>("nyxbot_events")
        .find_one(doc! {"_id":&event})
        .await
        .unwrap()
        .unwrap();
    let original_delivery = bson::to_bson(&original.delivery).unwrap();
    let (status, body) = write_call(&f).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["error_code"],
        7000,
        "{body}"
    );
    let request = channel_proposal(&f).await;
    let p = proposals::proposal(&request).unwrap();
    assert!(matches!(
        p.contract.context.audience,
        Audience::Channel { revision: 1, .. }
    ));
    assert_eq!(
        p.choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["operation", "task", "window", "persistent"]
    );
    assert!(p.review.audience.contains("everyone"));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
    assert!(
        crate::services::assistant_platform_reply_decision::decide_owner_reply(
            &f.state.db,
            &f.owner,
            &row.id,
            "yes"
        )
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(channel_proposal(&f).await.status, "pending");
    approve(&f, &request, "operation").await;
    finish(&f, &row, "Review in NyxID").await;
    assert_eq!(
        count(&f, DELIVERIES).await,
        0,
        "initial response retains original delivery"
    );
    let resumed = event_turn(&f, &row).await;
    let text = resume(&f, &resumed).await;
    assert!(text.contains("HTTP 200"), "{text}");
    assert_eq!(resume(&f, &resumed).await, text);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    finish(&f, &resumed, &text).await;
    let d = delivery(&f).await;
    assert_eq!(d.task.channel_source.as_ref().unwrap().event_id, event);
    let (a, b) = tokio::join!(
        channel::deliveries::lease(&f.state.db, &d.id),
        channel::deliveries::lease(&f.state.db, &d.id)
    );
    let claimed = a.unwrap().or(b.unwrap()).unwrap();
    assert!(
        channel::deliveries::claim(&f.state.db, &claimed)
            .await
            .unwrap()
    );
    assert!(
        !channel::deliveries::claim(&f.state.db, &claimed)
            .await
            .unwrap()
    );
    channel::deliveries::finish(&f.state.db, &claimed, "unknown")
        .await
        .unwrap();
    assert!(
        channel::deliveries::lease(&f.state.db, &d.id)
            .await
            .unwrap()
            .is_none()
    );
    // A replayed settlement must reuse the same UUID occurrence and preserve
    // its irreversible terminal state, even though enqueue generates a new ID.
    let message = f
        .state
        .db
        .collection::<crate::models::assistant_message::AssistantMessage>("assistant_messages")
        .find_one(doc! {"_id":&d.message_id})
        .await
        .unwrap()
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    channel::deliveries::enqueue(
        &f.state.db,
        &mut session,
        &resumed,
        resumed.active_turn.as_ref().unwrap(),
        &message,
    )
    .await
    .unwrap();
    assert_eq!(count(&f, DELIVERIES).await, 1);
    let replay = delivery(&f).await;
    assert_eq!(replay.id, d.id);
    assert_eq!(replay.state, "unknown");
    let final_event = f
        .state
        .db
        .collection::<NyxbotEvent>("nyxbot_events")
        .find_one(doc! {"_id":&event})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bson::to_bson(&final_event.delivery).unwrap(),
        original_delivery
    );
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_source_drift_refuses_decision_and_leaves_effect_unsent() {
    for (collection, filter, update) in [
        (
            "nyxbot_channels",
            doc! {},
            doc! {"$set":{"owner_sender_ids":[]}},
        ),
        (
            "nyxbot_channels",
            doc! {},
            doc! {"$inc":{"execution_policy_revision":1_i64}},
        ),
        (
            "nyxbot_threads",
            doc! {},
            doc! {"$inc":{"execution_policy_revision":1_i64}},
        ),
        (
            "channel_conversations",
            doc! {},
            doc! {"$set":{"is_active":false}},
        ),
        ("channel_bots", doc! {}, doc! {"$set":{"is_active":false}}),
        (
            "nyxbot_events",
            doc! {},
            doc! {"$set":{"delivery.execution_owner_verified":false}},
        ),
        ("api_keys", doc! {}, doc! {"$set":{"is_active":false}}),
        (
            "assistant_agents",
            doc! {},
            doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
        ),
        (
            "users",
            doc! {"user_type":"person"},
            doc! {"$set":{"is_active":false}},
        ),
        (
            "nyxbot_threads",
            doc! {},
            doc! {"$set":{"conversation_id":"different"}},
        ),
        (
            "nyxbot_events",
            doc! {},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}},
        ),
    ] {
        let (f, _row, _) = channel_turn("migration_channel_drift", false).await;
        write_call(&f).await;
        let request = channel_proposal(&f).await;
        f.state
            .db
            .collection::<Document>(collection)
            .update_many(filter, update)
            .await
            .unwrap();
        assert!(
            proposals::decide(
                &f.state,
                &f.owner,
                &request.id,
                &decision(&request, "operation")
            )
            .await
            .is_err(),
            "{collection}"
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
        assert_eq!(count(&f, "approval_grants").await, 0);
        f.cleanup().await;
    }
}

#[tokio::test]
async fn migration_channel_result_expiry_and_cancelled_epoch_do_not_reopen_task() {
    let (f, row, _) = channel_turn("migration_channel_receipt_cancel", false).await;
    write_call(&f).await;
    let request = channel_proposal(&f).await;
    approve(&f, &request, "operation").await;
    finish(&f, &row, "Review").await;
    let resumed = event_turn(&f, &row).await;
    let text = resume(&f, &resumed).await;
    finish(&f, &resumed, &text).await;
    let d = delivery(&f).await;
    let first = channel::deliveries::lease(&f.state.db, &d.id)
        .await
        .unwrap()
        .unwrap();
    f.state.db.collection::<Document>(DELIVERIES).update_one(doc!{"_id":&d.id},doc!{"$set":{"lease_expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    let next = channel::deliveries::lease(&f.state.db, &d.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(first.claim_id, next.claim_id);
    assert!(
        !channel::deliveries::claim(&f.state.db, &first)
            .await
            .unwrap()
    );
    f.state
        .db
        .collection::<Document>(tasks::collection(&d.task.task))
        .update_one(
            doc! {"_id":&d.task.task.root_id},
            doc! {"$inc":{"execution_task.epoch":1_i64}},
        )
        .await
        .unwrap();
    assert!(
        channel::deliveries::claim(&f.state.db, &next)
            .await
            .is_err()
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[test]
fn migration_channel_source_is_additive_and_redacted() {
    #[derive(serde::Deserialize)]
    struct OldEnvelope {
        version: u32,
        v1: Option<serde_json::Value>,
        v2: Option<serde_json::Value>,
        v3: Option<serde_json::Value>,
    }
    let old: OldEnvelope =
        serde_json::from_value(json!({"version":4,"v4":{"channel_source":"secret-source"}}))
            .unwrap();
    assert_eq!(old.version, 4);
    assert!(old.v1.is_none() && old.v2.is_none() && old.v3.is_none());
    let source = crate::models::execution_permission_channel::ChannelSource {
        version: 4,
        event_id: "secret-source".into(),
        actor_user_id: "owner".into(),
        agent_id: "agent".into(),
        conversation_id: "conversation".into(),
        key_id: "secret-key".into(),
        original_turn_id: "turn".into(),
        channel_id: "channel".into(),
        thread_id: "thread".into(),
        settings_thread_id: "settings".into(),
        channel_revision: 0,
        thread_revision: 0,
        audience_id: "audience".into(),
        audience_label: "Original chat".into(),
        policy_digest: "digest".into(),
        expires_at: Utc::now(),
    };
    let debug = format!("{source:?}");
    assert!(!debug.contains("secret-source") && !debug.contains("secret-key"));
}

async fn another_channel_turn(
    f: &Fixture,
    row: &AssistantConversation,
    event_id: &str,
) -> AssistantConversation {
    let mut event = f
        .state
        .db
        .collection::<NyxbotEvent>("nyxbot_events")
        .find_one(doc! {"_id":event_id})
        .await
        .unwrap()
        .unwrap();
    event.id = Uuid::new_v4().to_string();
    event.event_id = Uuid::new_v4().to_string();
    event.turn_id = None;
    event.conversation_id = None;
    event.status = "running".into();
    event.created_at = Utc::now();
    let id = event.id.clone();
    let origin = event.delivery.as_ref().unwrap().origin.clone();
    f.state
        .db
        .collection::<NyxbotEvent>("nyxbot_events")
        .insert_one(event)
        .await
        .unwrap();
    let request = engine::TurnRequest {
        conversation_id: Some(row.id.clone()),
        text: "Another update".into(),
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&request).into();
    start.origin = TurnOrigin::Channel;
    start.execution_enrollment = true;
    start.channel_event_id = Some(id);
    start.reply_channel = Some(origin);
    engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap()
}

#[tokio::test]
async fn migration_channel_persistent_permission_is_audience_bound_and_zero_prompt_when_covered() {
    let (f, row, event) = channel_turn("migration_channel_preapproved", false).await;
    write_call(&f).await;
    let request = channel_proposal(&f).await;
    approve(&f, &request, "persistent").await;
    finish(&f, &row, "Review").await;
    let resumed = event_turn(&f, &row).await;
    let text = resume(&f, &resumed).await;
    finish(&f, &resumed, &text).await;
    let second = another_channel_turn(&f, &row, &event).await;
    assert_eq!(write_call(&f).await.0, StatusCode::OK);
    assert_eq!(count(&f, "approval_requests").await, 1);
    assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
    finish(&f, &second, "Done").await;
    let third = another_channel_turn(&f, &row, &event).await;
    // Changed exact input is outside the displayed contract.
    let response = f
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/proxy/s/execution-fixture/status")
                .header("x-api-key", &f.raw)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"Different"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
    assert_eq!(error["error_code"], 7000, "{error}");
    assert_eq!(count(&f, "approval_requests").await, 2);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    tasks::stop(&f.state.db, &third).await.unwrap();
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_readiness_loss_after_review_never_downgrades() {
    let (mut f, row, _) = channel_turn("migration_channel_readiness", false).await;
    write_call(&f).await;
    let request = channel_proposal(&f).await;
    if let Some(task) = f.heartbeat.take() {
        task.abort();
        let _ = task.await;
    }
    f.state.db.collection::<Document>("security_replicas").update_many(doc!{},doc!{"$pull":{"capabilities":channel::READINESS},"$set":{"observed_at":bson::DateTime::now()}}).await.unwrap();
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation")
        )
        .await
        .is_err()
    );
    assert_ne!(write_call(&f).await.0, StatusCode::OK);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
    assert!(
        row.active_turn
            .unwrap()
            .execution_task
            .unwrap()
            .channel_source
            .is_some()
    );
    f.cleanup().await;
}

async fn result_ready(f: &Fixture, row: &AssistantConversation) -> Delivery {
    assert_eq!(write_call(f).await.0, StatusCode::FORBIDDEN);
    let request = channel_proposal(f).await;
    approve(f, &request, "operation").await;
    finish(f, row, "Review").await;
    let resumed = event_turn(f, row).await;
    let text = resume(f, &resumed).await;
    assert!(text.contains("HTTP 200"), "{text}");
    finish(f, &resumed, &text).await;
    delivery(f).await
}
async fn gateway(f: &Fixture, native: bool, outcome: &str) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "cmaeg".into();
    service.base_url = server.uri();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(service)
        .await
        .unwrap();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(if native {
            "/events/original-native-ref/replies"
        } else {
            "/events/original-relay-ref/replies"
        }))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"delivery":{"outcome":outcome}})),
        )
        .mount(&server)
        .await;
    server
}
#[tokio::test]
async fn migration_channel_result_transport_claims_once_and_retains_original_event() {
    for native in [false, true] {
        for outcome in ["confirmed", "rejected", "unknown"] {
            let (f, row, event) = channel_turn("migration_channel_result_send", native).await;
            let server = gateway(&f, native, outcome).await;
            let d = result_ready(&f, &row).await;
            let before = f
                .state
                .db
                .collection::<Document>("nyxbot_events")
                .find_one(doc! {"_id":&event})
                .await
                .unwrap()
                .unwrap();
            // Newer chat traffic cannot redirect a retained occurrence.
            f.state
                .db
                .collection::<Document>("nyxbot_threads")
                .update_many(doc! {}, doc! {"$set":{"last_message_id":"newer-source"}})
                .await
                .unwrap();
            let (a, b) = tokio::join!(
                crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id),
                crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
            );
            a.unwrap();
            b.unwrap();
            crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
                .await
                .unwrap();
            let sends = server.received_requests().await.unwrap();
            assert_eq!(
                sends.len(),
                1,
                "native={native}, {}",
                delivery(&f).await.state
            );
            assert_eq!(delivery(&f).await.state, outcome);
            assert_eq!(
                sends[0]
                    .headers
                    .get("idempotency-key")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                if native {
                    format!("{}:0", d.id)
                } else {
                    d.id.clone()
                }
            );
            assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
            let after = f
                .state
                .db
                .collection::<Document>("nyxbot_events")
                .find_one(doc! {"_id":&event})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(before.get("delivery"), after.get("delivery"));
            f.cleanup().await;
        }
    }
}
#[tokio::test]
async fn migration_channel_result_source_drift_withholds_stored_answer() {
    for mutation in [
        doc! {"$set":{"owner_sender_ids":[]}},
        doc! {"$inc":{"execution_policy_revision":1_i64}},
    ] {
        let (f, row, _) = channel_turn("migration_channel_result_drift", false).await;
        let server = gateway(&f, false, "confirmed").await;
        let d = result_ready(&f, &row).await;
        f.state
            .db
            .collection::<Document>("nyxbot_channels")
            .update_many(doc! {}, mutation)
            .await
            .unwrap();
        crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
            .await
            .unwrap();
        assert_eq!(delivery(&f).await.state, "refused");
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        f.cleanup().await;
    }
}
#[tokio::test]
async fn migration_channel_expired_source_and_crashed_dispatch_never_send() {
    for dispatched in [false, true] {
        let (f, row, _) = channel_turn("migration_channel_result_expiry", true).await;
        let server = gateway(&f, true, "confirmed").await;
        let d = result_ready(&f, &row).await;
        if dispatched {
            let lease = channel::deliveries::lease(&f.state.db, &d.id)
                .await
                .unwrap()
                .unwrap();
            assert!(
                channel::deliveries::claim(&f.state.db, &lease)
                    .await
                    .unwrap()
            );
            f.state.db.collection::<Document>(DELIVERIES).update_one(doc!{"_id":&d.id},doc!{"$set":{"lease_expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
        } else {
            f.state.db.collection::<Document>(DELIVERIES).update_one(doc!{"_id":&d.id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
        }
        assert!(
            channel::deliveries::due(&f.state.db)
                .await
                .unwrap()
                .is_empty()
        );
        crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
            .await
            .unwrap();
        assert_eq!(
            delivery(&f).await.state,
            if dispatched { "unknown" } else { "expired" }
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[tokio::test]
async fn migration_channel_direct_result_uses_original_reply_and_topic_once() {
    let (f, row, event) = channel_turn_transport("migration_channel_direct_reply", "direct").await;
    let bot = f
        .state
        .db
        .collection::<Document>("nyxbot_channels")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap()
        .get_str("channel_bot_id")
        .unwrap()
        .to_owned();
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/bottest-token/sendMessage"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"result":{"message_id":501,"chat":{"id":200}}})),
        )
        .mount(&server)
        .await;
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            bot.clone(),
            std::sync::Arc::new(
                crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
                    &server.uri(),
                ),
            ),
        );
    let d = result_ready(&f, &row).await;
    crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
        .await
        .unwrap();
    crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
        .await
        .unwrap();
    let sends = server.received_requests().await.unwrap();
    assert_eq!(sends.len(), 1, "{}", delivery(&f).await.state);
    let body: serde_json::Value = serde_json::from_slice(&sends[0].body).unwrap();
    assert_eq!(body["chat_id"], "200");
    assert_eq!(body["message_thread_id"], 77);
    assert_eq!(body["reply_to_message_id"], 500);
    assert_eq!(delivery(&f).await.state, "confirmed");
    assert_eq!(
        f.state
            .db
            .collection::<Document>("nyxbot_events")
            .find_one(doc! {"_id":event})
            .await
            .unwrap()
            .unwrap()
            .get_document("delivery")
            .unwrap()
            .get_str("state")
            .unwrap(),
        "streamed"
    );
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&bot);
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_policy_restoration_does_not_restore_old_review() {
    let (f, _row, _) = channel_turn("migration_channel_restore", false).await;
    write_call(&f).await;
    let request = channel_proposal(&f).await;
    let source = proposals::proposal(&request)
        .unwrap()
        .task
        .channel_source
        .as_ref()
        .unwrap()
        .clone();
    for allow in [true, false] {
        crate::services::channel_thread_follow_service::update_settings_as(
            &f.state.db,
            &f.owner,
            &f.owner,
            &source.channel_id,
            &source.settings_thread_id,
            doc! {"$set":{"allow_posts":allow}},
            false,
            false,
        )
        .await
        .unwrap();
    }
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &request.id,
            &decision(&request, "operation")
        )
        .await
        .is_err()
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_readback_is_separately_authorized_and_counted() {
    let (f, row, _) = channel_turn("migration_channel_readback", false).await;
    let server = gateway(&f, false, "confirmed").await;
    write_call(&f).await;
    let request = channel_proposal(&f).await;
    approve(&f, &request, "operation").await;
    finish(&f, &row, "Review").await;
    let resumed = event_turn(&f, &row).await;
    let write = resume(&f, &resumed).await;
    assert!(write.contains("HTTP 200"));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    let response = f
        .router()
        .oneshot(
            Request::builder()
                .uri("/proxy/s/execution-fixture/status")
                .header("x-api-key", &f.raw)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
    assert_eq!(error["error_code"], 7000, "{error}");
    assert_eq!(
        f.server.received_requests().await.unwrap().len(),
        1,
        "write grant cannot authorize readback"
    );
    let read = f
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"execution_proposal.version":4,"status":"pending"})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(request.id, read.id);
    approve(&f, &read, "operation").await;
    finish(&f, &resumed, "Write completed; readback needs review").await;
    let next = event_turn(&f, &row).await;
    let result = resume(&f, &next).await;
    assert!(result.contains("HTTP 200"));
    finish(&f, &next, &result).await;
    let sent = f.server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].method.as_str(), "POST");
    assert_eq!(sent[1].method.as_str(), "GET");
    assert_eq!(count(&f, "approval_requests").await, 2);
    assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
    let receipts: Vec<Delivery> = f
        .state
        .db
        .collection::<Delivery>(DELIVERIES)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(receipts.len(), 2);
    for receipt in receipts {
        crate::handlers::nyxbot::execution_delivery::process(&f.state, &receipt.id)
            .await
            .unwrap();
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_delegation_retains_audience_and_refuses_guest_descendant() {
    let (f, row, _) = channel_turn("migration_channel_delegation", false).await;
    let (agent, _) = crate::services::assistant_team_service::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        crate::services::assistant_team_service::CreateRequest {
            name: "channel-helper".into(),
            description: "Channel specialist".into(),
            display_name: None,
            persona: None,
            targets: crate::services::assistant_team_service::GrantTargets {
                service_ids: vec![f.connection.clone()],
                ..Default::default()
            },
            account_read: false,
            specialty: None,
            created_by: "user",
            machines: None,
            logins: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    let chat = acks::for_key(&f.state.db, &f.owner, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let request = engine::TurnRequest {
        conversation_id: None,
        text: "Help with the same approved task".into(),
        agent_id: Some(agent.id),
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&request).into();
    start.origin = TurnOrigin::Orchestrator;
    start.execution_parent = tasks::parent(&chat);
    start.report_to = Some(row.id.clone());
    let child = engine::begin_turn(
        &f.state.db,
        &f.owner,
        start.clone(),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let turn = child.active_turn.as_ref().unwrap();
    assert_eq!(
        turn.execution_task.as_ref().unwrap().channel_source,
        row.active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .channel_source
    );
    assert_eq!(turn.report_to.as_deref(), Some(row.id.as_str()));
    assert!(!channel::deliveries::owns_result(&child, turn));
    start.guest = true;
    assert!(
        engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
            .await
            .is_err()
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_silent_initial_answer_retains_only_its_bound_reply_source() {
    let (f, row, event) = channel_turn("migration_channel_silent", false).await;
    let server = gateway(&f, false, "confirmed").await;
    write_call(&f).await;
    let request = channel_proposal(&f).await;
    finish(&f, &row, "").await;
    f.state
        .db
        .collection::<Document>("nyxbot_events")
        .update_one(
            doc! {"_id":&event},
            doc! {"$set":{"delivery.state":"waiting"}},
        )
        .await
        .unwrap();
    assert!(
        crate::services::channel_turn_delivery::silence(&f.state.db, &event)
            .await
            .unwrap()
    );
    let before = f
        .state
        .db
        .collection::<Document>("nyxbot_events")
        .find_one(doc! {"_id":&event})
        .await
        .unwrap()
        .unwrap();
    assert!(before.get("event_context_ciphertext").is_some());
    // Exercise the decision transaction here; the handler's automatic wake is
    // covered above while its source turn is live. This case resumes explicitly.
    proposals::decide(
        &f.state,
        &f.owner,
        &request.id,
        &decision(&request, "operation"),
    )
    .await
    .unwrap();
    let next = event_turn(&f, &row).await;
    let result = resume(&f, &next).await;
    finish(&f, &next, &result).await;
    let d = delivery(&f).await;
    crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(delivery(&f).await.state, "confirmed");
    let after = f
        .state
        .db
        .collection::<Document>("nyxbot_events")
        .find_one(doc! {"_id":&event})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.get("delivery"), after.get("delivery"));
    // The existing nonparticipating silence path still removes its capability.
    f.state
        .db
        .collection::<Document>("nyxbot_events")
        .update_one(
            doc! {"_id":&event},
            doc! {"$set":{"delivery.state":"waiting","delivery.execution_source_bound":false}},
        )
        .await
        .unwrap();
    assert!(
        crate::services::channel_turn_delivery::silence(&f.state.db, &event)
            .await
            .unwrap()
    );
    let cleared = f
        .state
        .db
        .collection::<Document>("nyxbot_events")
        .find_one(doc! {"_id":&event})
        .await
        .unwrap()
        .unwrap();
    assert!(cleared.get("event_context_ciphertext").is_none());
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_gateway_sealed_target_outlives_event_window_without_retargeting() {
    let (f, row, _) = channel_turn_transport("migration_channel_sealed_target", "sealed").await;
    let server = gateway(&f, false, "confirmed").await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(
            "/reply-targets/original-target/messages",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
        )
        .mount(&server)
        .await;
    let d = result_ready(&f, &row).await;
    assert!(d.task.hard_deadline > Utc::now() + chrono::Duration::hours(20));
    f.state
        .db
        .collection::<Document>("nyxbot_events")
        .update_many(doc! {}, doc! {"$unset":{"event_context_ciphertext":""}})
        .await
        .unwrap();
    crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
        .await
        .unwrap();
    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].url.path(),
        "/reply-targets/original-target/messages"
    );
    assert_eq!(
        sent[0]
            .headers
            .get("idempotency-key")
            .unwrap()
            .to_str()
            .unwrap(),
        d.id
    );
    assert_eq!(delivery(&f).await.state, "confirmed");
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_nonretainable_effect_never_falls_back_to_private_or_legacy_review() {
    let (f, _row, _) = channel_turn("migration_channel_nonretainable", false).await;
    let response = f
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/proxy/s/execution-fixture/status")
                .header("x-api-key", &f.raw)
                .header("content-type", "application/octet-stream")
                .body(Body::from(vec![0xff, 0x00, 0x7f]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(
        String::from_utf8(body.to_vec())
            .unwrap()
            .contains("use the NyxID web thread")
    );
    assert_eq!(count(&f, "approval_requests").await, 0);
    assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_gateway_server_failure_is_unknown_and_never_sent_twice() {
    for native in [false, true] {
        let (f, row, _) = channel_turn("migration_channel_gateway_5xx", native).await;
        let server = gateway(&f, native, "confirmed").await;
        server.reset().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let d = result_ready(&f, &row).await;
        crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
            .await
            .unwrap();
        crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
            .await
            .unwrap();
        assert_eq!(delivery(&f).await.state, "unknown", "native={native}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        f.cleanup().await;
    }
}

async fn sheet_call(
    f: &Fixture,
    method: &str,
    range: &str,
    body: Option<&serde_json::Value>,
) -> (StatusCode, String) {
    let path = format!(
        "/proxy/s/execution-fixture/v4/spreadsheets/book/values/{range}?valueInputOption=RAW"
    );
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-api-key", &f.raw);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = f
        .router()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn migration_channel_sheets_range_inputs_review_readback_and_exact_original_reply() {
    for transport in ["direct", "gateway", "native"] {
        let (f, row, _) =
            channel_turn_transport("migration_channel_sheets_inputs", transport).await;
        let native = transport == "native";
        let mut direct_bot = None;
        let relay = if transport == "direct" {
            let bot = f
                .state
                .db
                .collection::<Document>("nyxbot_channels")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .get_str("channel_bot_id")
                .unwrap()
                .to_owned();
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/bottest-token/sendMessage"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                    json!({"ok":true,"result":{"message_id":501,"chat":{"id":200}}}),
                ))
                .mount(&server)
                .await;
            crate::handlers::channel_relay::TEST_REPLY_ADAPTERS.lock().unwrap().insert(bot.clone(),
                std::sync::Arc::new(crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(&server.uri())));
            direct_bot = Some(bot);
            server
        } else {
            gateway(&f, native, "confirmed").await
        };
        let values = json!({"range":"Sheet1!A1:B2","majorDimension":"ROWS","values":[["Type","Amount"],["Receipt",12]]});
        let (status, error) = sheet_call(&f, "PUT", "Sheet1!A1:B2", Some(&values)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
        let write = channel_proposal(&f).await;
        let proposal = proposals::proposal(&write).unwrap();
        assert!(
            proposal
                .choices
                .iter()
                .any(|choice| choice.id == "persistent")
        );
        let scope = &proposal.contract.operations;
        assert_eq!(scope.operations.len(), 1);
        assert_eq!(
            scope.operations[0].rule.path_template,
            "/v4/spreadsheets/book/values/Sheet1!A1:B2"
        );
        assert!(scope.operations[0].inputs.is_some());
        assert!(f.server.received_requests().await.unwrap().is_empty());
        approve(&f, &write, "persistent").await;
        finish(&f, &row, "Review in NyxID").await;
        let resumed = event_turn(&f, &row).await;
        let capture = super::super::migration_tests::Capture::default();
        let result = tracing::instrument::WithSubscriber::with_subscriber(
            resume(&f, &resumed),
            capture.subscriber(),
        )
        .await;
        assert!(
            result.contains("HTTP 200"),
            "{result}; metadata: {:?}",
            capture.0.lock().unwrap()
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        // The same valid permission cannot authorize an independent readback.
        assert_eq!(
            sheet_call(&f, "GET", "Sheet1!A1:B2", None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        let read = f
            .state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {"execution_proposal.version":4,"status":"pending"})
            .await
            .unwrap()
            .unwrap();
        approve(&f, &read, "operation").await;
        finish(&f, &resumed, "Write complete; readback needs review").await;
        let next = event_turn(&f, &row).await;
        let result = resume(&f, &next).await;
        assert!(result.contains("HTTP 200"), "{result}");
        let sent = f.server.received_requests().await.unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].method.as_str(), "PUT");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&sent[0].body).unwrap(),
            values
        );
        assert_eq!(sent[0].url.query(), Some("valueInputOption=RAW"));
        assert_eq!(sent[1].method.as_str(), "GET");
        // A changed range is outside the reviewed persistent contract.
        assert_eq!(
            sheet_call(&f, "PUT", "Sheet2!A1:B2", Some(&values)).await.0,
            StatusCode::FORBIDDEN
        );
        let changed = json!({"range":"Sheet1!A1:B2","majorDimension":"ROWS","values":[["Type","Amount"],["Receipt",999]]});
        assert_eq!(
            sheet_call(&f, "PUT", "Sheet1!A1:B2", Some(&changed))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
        assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
        finish(&f, &next, &result).await;
        let receipts: Vec<Delivery> = f
            .state
            .db
            .collection::<Delivery>(DELIVERIES)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(receipts.len(), 2);
        for d in receipts {
            crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
                .await
                .unwrap();
            crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
                .await
                .unwrap();
            let stored = f
                .state
                .db
                .collection::<Delivery>(DELIVERIES)
                .find_one(doc! {"_id":d.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.state, "confirmed");
        }
        let replies = relay.received_requests().await.unwrap();
        assert_eq!(replies.len(), 2);
        if let Some(bot) = direct_bot {
            for reply in replies {
                let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
                assert_eq!(body["chat_id"], "200");
                assert_eq!(body["message_thread_id"], 77);
                assert_eq!(body["reply_to_message_id"], 500);
            }
            crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
                .lock()
                .unwrap()
                .remove(&bot);
        }
        f.cleanup().await;
    }
}

#[tokio::test]
async fn migration_channel_multiple_decisions_coalesce_into_one_result_occurrence() {
    let (f, row, _) = channel_turn("migration_channel_coalesced_results", false).await;
    let server = gateway(&f, false, "confirmed").await;
    assert_eq!(write_call(&f).await.0, StatusCode::FORBIDDEN);
    let response = f
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/proxy/s/execution-fixture/status")
                .header("x-api-key", &f.raw)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"title":"Second separately reviewed write"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let requests: Vec<ApprovalRequest> = f
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find(doc! {"execution_proposal.version":4})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        approve(&f, request, "operation").await;
    }
    assert!(f.server.received_requests().await.unwrap().is_empty());
    finish(&f, &row, "Review in NyxID").await;
    let resumed = event_turn(&f, &row).await;
    let result = resume(&f, &resumed).await;
    assert!(result.contains("HTTP 200"), "{result}");
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    finish(&f, &resumed, &result).await;
    assert_eq!(count(&f, DELIVERIES).await, 1);
    let d = delivery(&f).await;
    let (a, b) = tokio::join!(
        crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id),
        crate::handlers::nyxbot::execution_delivery::process(&f.state, &d.id)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(delivery(&f).await.state, "confirmed");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_excluded_fresh_lanes_add_zero_commands() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let (f, original) = assistant_for_cohort("migration_channel_excluded_reads", true).await;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let events = captured.clone();
    let name = f.state.db.name().to_owned();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == name
        {
            events.lock().unwrap().push(event.command_name);
        }
    }));
    let db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());
    let mut session = db.client().start_session().await.unwrap();
    let request = engine::TurnRequest {
        conversation_id: Some(original.id.clone()),
        text: "Fresh channel message".into(),
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        model: None,
        access_mode: None,
    };
    let ids = engine::OwnerTurnIds {
        artifact_budget: Default::default(),
        conversation_id: original.id.clone(),
        turn_id: original.active_turn.as_ref().unwrap().turn_id.clone(),
        message_id: Uuid::new_v4().to_string(),
    };
    for lane in ["org", "shared", "group", "unmarked"] {
        let mut row = original.clone();
        row.active_turn.as_mut().unwrap().execution_task = None;
        row.agent_owner_id = (lane == "org").then(|| "organization".into());
        row.shared_agent_thread = lane == "shared";
        row.group_id = (lane == "group").then(|| "group".into());
        let mut start: engine::TurnStart = (&request).into();
        start.origin = TurnOrigin::Channel;
        start.execution_enrollment = lane != "unmarked";
        tasks::admit(&db, &mut session, &mut row, &start, &ids)
            .await
            .unwrap();
        assert!(
            row.active_turn.as_ref().unwrap().execution_task.is_none(),
            "{lane}"
        );
        assert!(captured.lock().unwrap().is_empty(), "{lane}");
    }
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_no_permission_floor_still_fences_original_source() {
    let (f, _row, event) = channel_turn("migration_channel_no_floor_fence", false).await;
    assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        assistant_settings_service::Update {
            access_mode: Some(AccessMode::Autopilot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<Document>("service_approval_configs")
        .update_many(doc! {}, doc! {"$set":{"approval_required":false}})
        .await
        .unwrap();
    let before = f
        .state
        .db
        .collection::<Document>("nyxbot_events")
        .find_one(doc! {"_id":&event})
        .await
        .unwrap()
        .unwrap();
    let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&f, "approval_requests").await, 0);
    assert_eq!(count(&f, "assistant_acknowledgements").await, 0);
    let after = f
        .state
        .db
        .collection::<Document>("nyxbot_events")
        .find_one(doc! {"_id":&event})
        .await
        .unwrap()
        .unwrap();
    assert!(
        after.get_i64("execution_source_fence").unwrap()
            > before.get_i64("execution_source_fence").unwrap()
    );
    f.state
        .db
        .collection::<Document>("nyxbot_channels")
        .update_many(doc! {}, doc! {"$inc":{"execution_policy_revision":1_i64}})
        .await
        .unwrap();
    assert_ne!(
        f.call("/proxy/s/execution-fixture/status").await.0,
        StatusCode::OK
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn migration_channel_conversation_deletion_removes_receipt_and_reply_material() {
    let (f, row, event_id) = channel_turn_transport("migration_channel_delete", "sealed").await;
    let d = result_ready(&f, &row).await;
    let original = f
        .state
        .db
        .collection::<NyxbotEvent>("nyxbot_events")
        .find_one(doc! {"_id": &event_id})
        .await
        .unwrap()
        .unwrap();
    assert!(original.event_context_ciphertext.is_some());
    assert!(
        original
            .delivery
            .as_ref()
            .unwrap()
            .target_ciphertext
            .is_some()
    );
    let original_state = original.delivery.unwrap().state;
    engine::delete(&f.state.db, &f.owner, &row.id)
        .await
        .unwrap();
    assert_eq!(count(&f, DELIVERIES).await, 0);
    assert!(
        channel::deliveries::lease(&f.state.db, &d.id)
            .await
            .unwrap()
            .is_none()
    );
    let event = f
        .state
        .db
        .collection::<NyxbotEvent>("nyxbot_events")
        .find_one(doc! {"_id": &event_id})
        .await
        .unwrap()
        .unwrap();
    assert!(event.event_context_ciphertext.is_none());
    let delivery = event.delivery.unwrap();
    assert!(!delivery.execution_source_bound);
    assert!(delivery.target_ciphertext.is_none());
    assert!(delivery.target_expires_at.is_none());
    assert_eq!(delivery.state, original_state);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}
