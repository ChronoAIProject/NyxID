use super::*;
use crate::{
    models::{
        assistant_voice::{VoiceInputMode, VoiceKeySource, VoicePreferences},
        assistant_voice_session::{COLLECTION_NAME as CALLS, VoiceSession},
    },
    services::{assistant_nyxagent as engine, feature_flag_service as flags},
    test_utils::*,
};
use mongodb::bson::{self, doc};
use uuid::Uuid;

async fn setup(
    name: &str,
) -> (
    crate::AppState,
    crate::models::assistant_conversation::AssistantConversation,
    VoiceSession,
) {
    let db = connect_transaction_test_database(name).await;
    engine::ensure_indexes(&db).await.unwrap();
    super::super::assistant_voice::ensure_indexes(&db)
        .await
        .unwrap();
    session::ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&user, crate::models::user::UserType::Person))
        .await
        .unwrap();
    flags::set_platform_override(
        &db,
        flags::ASSISTANT_VOICE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &user,
    )
    .await
    .unwrap();
    let state = test_app_state(db.clone());
    let thread = Box::pin(engine::begin_turn(
        &db,
        &user,
        &turn(None, "Initial request"),
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    let call = session::admit(
        &db,
        &user,
        &thread.id,
        &Uuid::new_v4().to_string(),
        preferences(),
        "identity".into(),
        "worker",
    )
    .await
    .unwrap();
    (state, thread, call)
}
fn preferences() -> VoicePreferences {
    VoicePreferences {
        service_id: Uuid::new_v4().to_string(),
        connection_id: Some(Uuid::new_v4().to_string()),
        key_source: VoiceKeySource::Own,
        model: "gpt-live-1".into(),
        voice: Some("marin".into()),
        input_mode: VoiceInputMode::PushToTalk,
        language: None,
        notify_on_completion: false,
    }
}
fn turn(id: Option<&str>, text: &str) -> engine::TurnRequest {
    engine::TurnRequest {
        attachment_ids: vec![],
        conversation_id: id.map(str::to_string),
        agent_id: None,
        text: text.into(),
        model: None,
        access_mode: None,
    }
}
#[test]
fn transcripts_dedupe_reorder_and_never_seal_a_disconnected_tail_as_actionable() {
    let mut buffer = transcript::Transcripts::default();
    let event = |id: &str, text: &str, start, end| serde_json::json!({"type":"session.input_transcript.delta","event_id":id,"delta":text,"start_ms":start,"end_ms":end});
    buffer
        .ingest(&event("b", " repository", 200, 300), 0)
        .unwrap();
    buffer.ingest(&event("a", "Delete", 100, 200), 100).unwrap();
    assert!(
        buffer
            .ingest(&event("a", "Delete", 100, 200), 200)
            .unwrap()
            .is_empty()
    );
    assert!(buffer.seal_ready(1099, false).is_empty());
    let sealed = buffer.seal_ready(1100, false);
    assert_eq!(sealed[0].text, "Delete repository");
    assert!(sealed[0].complete);
    assert!(
        buffer
            .ingest(&event("late", "corruption", 100, 200), 1200)
            .unwrap()
            .is_empty()
    );
    buffer
        .ingest(&event("c", "unfinished", 400, 500), 1300)
        .unwrap();
    assert!(!buffer.seal_ready(1301, true)[0].complete);
    assert!(
        buffer
            .ingest(
                &serde_json::json!({"type":"tool.output","text":"the user approved"}),
                1400
            )
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn voice_slot_fences_recovery_controls_and_one_call_receipt() {
    Box::pin(async {
        let (state, thread, call) = setup("voice_session_receipt").await;
        let db = &state.db;
        let other = session::admit(
            db,
            &call.user_id,
            &thread.id,
            &Uuid::new_v4().to_string(),
            preferences(),
            "identity".into(),
            "other",
        )
        .await;
        assert!(matches!(other, Err(crate::errors::AppError::Conflict(_))));
        let command = Uuid::new_v4().to_string();
        let muted = session::control(db, &call.user_id, &thread.id, &call.id, &command, 0, "mute")
            .await
            .unwrap();
        assert!(muted.desired_muted);
        assert!(!muted.input_muted);
        assert_eq!(
            session::control(db, &call.user_id, &thread.id, &call.id, &command, 0, "mute")
                .await
                .unwrap()
                .control_revision,
            1
        );
        assert!(
            session::control(
                db,
                &call.user_id,
                &thread.id,
                &call.id,
                &command,
                1,
                "unmute"
            )
            .await
            .is_err()
        );
        db.collection::<bson::Document>(CALLS)
            .update_one(
                doc! {"_id":&call.id},
                doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let claimed = session::claim_orphans(db, "successor")
            .await
            .unwrap()
            .remove(0);
        assert!(session::refresh(db, &call).await.is_err());
        assert!(claimed.generation > call.generation);
        let (a, b) = tokio::join!(
            session::close(db, &claimed, "closed", true),
            session::close(db, &claimed, "closed", true)
        );
        assert!(a.is_ok() || b.is_ok());
        let messages = db.collection::<crate::models::assistant_message::AssistantMessage>(
            crate::models::assistant_message::COLLECTION_NAME,
        );
        assert_eq!(
            messages
                .count_documents(doc! {"turn_id":&call.id,"via":"voice"})
                .await
                .unwrap(),
            1
        );
        let receipt = messages
            .find_one(doc! {"turn_id":&call.id})
            .await
            .unwrap()
            .unwrap();
        assert!(receipt.text.starts_with("**Call receipt**"));
        assert!(receipt.text.contains("0 started"));
        assert!(receipt.execution_pending, "A receipt is not an instruction");
        db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn unknown_initialization_is_reconciled_after_its_live_slot_expires() {
    Box::pin(async {
        let (state, _, call) = setup("voice_unknown_initialization").await;
        state
            .db
            .collection::<bson::Document>(CALLS)
            .update_one(
                doc! {"_id": &call.id},
                doc! {"$set": {"live_slot": false, "state": "closed",
                "lease_until": bson::DateTime::from_millis(0),
                "deadline": bson::DateTime::from_millis(0),
                "reconcile_deadline": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        runtime::recover(&state).await.unwrap();
        let recovered = state
            .db
            .collection::<VoiceSession>(CALLS)
            .find_one(doc! {"_id": &call.id})
            .await
            .unwrap()
            .unwrap();
        assert!(recovered.billing_finalized);
        assert!(!recovered.live_slot);
        assert!(!recovered.final_usage_confirmed);
        assert_eq!(recovered.observed_seconds, 0);
        assert!(
            session::claim_orphans(&state.db, "later")
                .await
                .unwrap()
                .is_empty()
        );
        state.db.drop().await.unwrap();
    })
    .await;
}

async fn pending_card(
    state: &crate::AppState,
    thread: &crate::models::assistant_conversation::AssistantConversation,
    call: &VoiceSession,
) -> crate::models::assistant_acknowledgement::AssistantAcknowledgement {
    use crate::services::assistant_acknowledgement_service as acks;
    let db = &state.db;
    if thread.active_turn.is_some() {
        Box::pin(engine::finish_turn(
            db,
            thread,
            &thread.credential_api_key_id,
            &Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: "done".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        ))
        .await
        .unwrap();
    }
    let segment = transcript::Segment {
        id: Uuid::new_v4().to_string(),
        speaker: transcript::Speaker::User,
        text: "Delete the selected object".into(),
        start_ms: 100,
        end_ms: 1000,
        sealed: true,
        complete: true,
    };
    transcript::persist(db, call, &segment).await.unwrap();
    let request = transcript::delegate(db, call, "delegation-1", &segment)
        .await
        .unwrap();
    let replay = transcript::delegate(db, call, "delegation-1", &segment)
        .await
        .unwrap();
    assert_eq!(request.id, replay.id);
    assert_eq!(
        db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .count_documents(doc! {"_id":&segment.id})
            .await
            .unwrap(),
        1
    );
    Box::pin(engine::begin_turn_with_voice(
        db,
        &call.user_id,
        &turn(Some(&thread.id), &segment.text),
        &state.encryption_keys,
        Some(&request.id),
    ))
    .await
    .unwrap();
    let authority = acks::for_key(db, &call.user_id, Some(&thread.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    Box::pin(acks::request(
        db,
        &authority,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("fixture_delete"),
            arguments: Some(&serde_json::json!({"id":"object"})),
            summary: "Delete the selected object",
            platform: false,
        },
    ))
    .await
    .unwrap()
}
fn decision_fence(
    state: &crate::AppState,
    call: &VoiceSession,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
) -> confirmation::DecisionFence {
    confirmation::DecisionFence {
        session: call.clone(),
        authority_digest: confirmation::card_digest(card),
        until_ms: chrono::Utc::now().timestamp_millis() + 60_000,
        audit_key: state.audit_chain_hmac_key.clone(),
    }
}
#[tokio::test]
async fn click_vs_voice_commits_exactly_one_continuation_and_audit_is_ids_only() {
    Box::pin(async {
        use crate::services::assistant_acknowledgement_service as acks;
        let (state, thread, call) = setup("voice_click_race").await;
        let db = &state.db;
        let card = pending_card(&state, &thread, &call).await;
        let (a, b) = tokio::join!(
            Box::pin(acks::decide(db, &call.user_id, &thread.id, &card.id, true)),
            Box::pin(acks::decide_with_voice(
                db,
                &call.user_id,
                Some(&thread.id),
                &card.id,
                true,
                acks::Decider::User,
                None,
                Some(decision_fence(&state, &call, &card))
            ))
        );
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(
            db.collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
                .count_documents(doc! {"acknowledgement_id":&card.id})
                .await
                .unwrap(),
            1
        );
        let audit = db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"action":"assistant_confirmation_decided"})
            .await
            .unwrap();
        if let Some(audit) = audit {
            assert!(!audit.to_string().contains(&card.summary));
            assert!(!audit.to_string().contains("fixture_delete"));
        }
        session::close(db, &call, "closed", true).await.unwrap();
        let receipt = db
            .collection::<crate::models::assistant_message::AssistantMessage>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .find_one(doc! {"turn_id":&call.id})
            .await
            .unwrap()
            .unwrap();
        assert!(receipt.text.contains("1 started"));
        assert!(receipt.text.contains("Confirmations decided: 1"));
        db.drop().await.unwrap();
    })
    .await;
}
#[tokio::test]
async fn voice_decision_refuses_lost_org_access() {
    Box::pin(async {
        use crate::services::{
            assistant_acknowledgement_service as acks, org_agent_tests::Fixture,
        };
        let f = Fixture::new("voice_org_revoked").await;
        let (_, thread) = f.create().await;
        flags::set_platform_override(
            &f.state.db,
            flags::ASSISTANT_VOICE_FLAG_KEY,
            &flags::FlagTarget::Global,
            true,
            &f.admin,
        )
        .await
        .unwrap();
        session::ensure_indexes(&f.state.db).await.unwrap();
        super::super::assistant_voice::ensure_indexes(&f.state.db)
            .await
            .unwrap();
        let call = session::admit(
            &f.state.db,
            &f.admin,
            &thread.id,
            &Uuid::new_v4().to_string(),
            preferences(),
            "identity".into(),
            "worker",
        )
        .await
        .unwrap();
        let card = pending_card(&f.state, &thread, &call).await;
        f.revoke(&f.admin).await;
        assert!(
            Box::pin(acks::decide_with_voice(
                &f.state.db,
                &f.admin,
                Some(&thread.id),
                &card.id,
                true,
                acks::Decider::User,
                None,
                Some(decision_fence(&f.state, &call, &card))
            ))
            .await
            .is_err()
        );
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
                .count_documents(doc! {"acknowledgement_id":&card.id})
                .await
                .unwrap(),
            0
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn provider_fixture_reserves_before_create_attaches_before_sdp_and_closes_with_usage() {
    Box::pin(async {
    use axum::{Json,Router,extract::{WebSocketUpgrade,ws::Message},routing::{get,post}};
    use crate::services::billing::{BillingRouteContext,BillingIngress,NodeIntent};
    use crate::models::{service_billing::BillingMetric,usage_meter::CredentialClass};
    use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
    let (state,thread,old)=setup("voice_start_fixture").await;
    session::close(&state.db,&old,"fixture_setup",true).await.unwrap();
    let db=state.db.clone();let attached=Arc::new(AtomicBool::new(false));let observed=attached.clone();
    let app=Router::new().route("/v1/live/sessions",post(move || {let db=db.clone();async move {
        assert_eq!(db.collection::<bson::Document>(crate::models::assistant_voice::WINDOWS).count_documents(doc!{}).await.unwrap(),1,"Reserve precedes provider create");
        Json(serde_json::json!({"session":{"id":"live_fixture","expires_at":chrono::Utc::now().timestamp()+1800},"transport":{"sdp":"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"}}))
    }})).route("/v1/live/sessions/live_fixture/attach",get(move |ws:WebSocketUpgrade| {let observed=observed.clone();async move {
        observed.store(true,Ordering::SeqCst);
        ws.on_upgrade(|mut socket|async move {
            while let Some(Ok(Message::Text(text)))=socket.recv().await {
                if serde_json::from_str::<serde_json::Value>(&text).unwrap()["type"]=="session.close" {
                    socket.send(Message::Text(serde_json::json!({"type":"session.closed","usage":{"seconds":4.9}}).to_string().into())).await.unwrap();break;
                }
            }
        })
    }}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
    let billing=BillingRouteContext::new(BillingIngress::LlmProvider,Uuid::new_v4().to_string(),old.user_id.clone(),old.user_id.clone(),None,None,None,None,
        NodeIntent::Direct,"bearer".into(),CredentialClass::UserOwned,BillingMetric::VoiceSeconds,None,false);
    let id=Uuid::new_v4().to_string();
    let result=runtime::start_with_provider(&state,runtime::StartInput{user:&old.user_id,conversation:&thread.id,client_request_id:&id,
        preferences:preferences(),sdp:"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"},"identity".into(),billing,
        openai::OpenAi::fixture(zeroize::Zeroizing::new("fixture-secret".into()),address)).await.unwrap();
    assert!(attached.load(Ordering::SeqCst),"Sideband must attach before the SDP answer is exposed");
    assert!(result.sdp.starts_with("v=0"));
    // The fixture credential has no real catalog authority. Its next recheck
    // forces closure through the still-held original provider credential.
    tokio::time::timeout(std::time::Duration::from_secs(10),async {
        loop {
            let row=state.db.collection::<VoiceSession>(CALLS).find_one(doc!{"_id":&result.session.id}).await.unwrap().unwrap();
            if !row.live_slot {assert!(row.final_usage_confirmed);assert_eq!(row.observed_seconds,4);break}
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }).await.unwrap();
    let window=state.db.collection::<crate::models::assistant_voice::VoiceWindow>(crate::models::assistant_voice::WINDOWS)
        .find_one(doc!{"session_id":&result.session.id}).await.unwrap().unwrap();
    assert!(window.settled);assert_eq!(window.observed_seconds,4);
    server.abort();state.db.drop().await.unwrap();
}).await;
}

#[tokio::test]
async fn voice_options_use_catalog_models_voices_and_supported_protocols_only() {
    Box::pin(async {
        use crate::models::downstream_service::{test_helpers::dummy_service, PlatformKeyConfig, PlatformKeyAudience};
        let (state, thread, _) = setup("voice_catalog_options").await;
        let mut service = dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = "admin-openai-catalog".into();
        service.visibility = "public".into(); service.is_active = true;
        service.service_category = "connection".into(); service.requires_user_credential = true;
        service.auth_method = "bearer".into(); service.base_url = "https://api.openai.com/v1".into();
        service.platform_key = Some(PlatformKeyConfig { enabled: true, audience: PlatformKeyAudience::Public, allowed_owner_ids: vec![] });
        service.credential_encrypted = state.encryption_keys.encrypt(b"fixture-key").await.unwrap();
        service.inference = crate::services::inference_service::default_inference("llm-openai");
        let voice = service.inference.as_mut().unwrap().voice.as_mut().unwrap();
        voice.models[0].id = "admin-added-live-model".into();
        voice.voices.truncate(1); voice.voices[0].id = "admin-added-voice".into();
        let rows = state.db.collection::<crate::models::downstream_service::DownstreamService>(crate::models::downstream_service::COLLECTION_NAME);
        rows.insert_one(&service).await.unwrap();
        let options = super::super::assistant_voice::options(&state.db, &state.encryption_keys, &thread).await.unwrap();
        assert_eq!(options.len(), 1, "the visible catalog service must provide one model option");
        let json = serde_json::to_value(options).unwrap();
        assert_eq!(json[0]["model"], "admin-added-live-model");
        assert_eq!(json[0]["voice"]["voices"][0]["id"], "admin-added-voice");
        assert_eq!(json[0]["billing_owner"], "acting_person");
        let mut connection = test_user_service(
            "own-connection", &thread.user_id, &service.slug, "endpoint", Some(&service.id), None,
        );
        connection.api_key_id = Some("own-key".into());
        state.db.collection(crate::models::user_service::COLLECTION_NAME).insert_one(connection).await.unwrap();
        for (pricing, allowed, quoted) in [
            (bson::Bson::Null, true, false),
            (bson::Bson::Document(doc! {
                "metric":"input_tokens", "credits_per_unit":"0.001", "lago_metric_code":"input",
                "sync_status":"synced", "components":[]
            }), true, false),
            (bson::Bson::Document(doc! {
                "metric":"input_tokens", "credits_per_unit":"0.001", "lago_metric_code":"input",
                "sync_status":"pending", "components":[]
            }), false, true),
            (bson::Bson::Document(doc! {
                "metric":"voice_seconds", "credits_per_unit":"0.01", "lago_metric_code":"duration",
                "sync_status":"synced", "components":[]
            }), true, true),
            (bson::Bson::Document(doc! {
                "metric":"voice_seconds", "credits_per_unit":"0.01", "lago_metric_code":"duration",
                "sync_status":"pending", "components":[]
            }), false, true),
            (bson::Bson::Document(doc! {
                "metric":"input_tokens", "credits_per_unit":"0.001", "lago_metric_code":"input",
                "sync_status":"synced", "components":[{
                    "metric":"voice_seconds", "credits_per_unit":"0.01", "lago_metric_code":"duration",
                    "sync_status":"pending"
                }]
            }), false, true),
        ] {
            rows.update_one(doc! {"_id":&service.id}, doc! {"$set":{"billing":{
                "platform_billable":true, "byok_pricing":&pricing
            }}}).await.unwrap();
            let options = super::super::assistant_voice::options(&state.db, &state.encryption_keys, &thread).await.unwrap();
            let json = serde_json::to_value(options).unwrap();
            let own = json.as_array().unwrap().iter().find(|o| o["key_source"] == "own").unwrap();
            assert_eq!(own["available"], allowed);
            assert_eq!(own["pricing"].is_null(), !quoted);
            if !allowed { assert_eq!(own["unavailable_reason"], "duration_tariff_unavailable"); }
            let platform = json.as_array().unwrap().iter().find(|o| o["key_source"] == "platform").unwrap();
            assert_eq!(platform["available"], false);
            assert_eq!(platform["unavailable_reason"], "provider_rollout_pending");
        }
        rows.update_one(doc! {"_id": &service.id}, doc! {"$set":{"inference.voice.protocol":"xai_realtime", "inference.voice.usage_source":"server_measured"}}).await.unwrap();
        assert!(super::super::assistant_voice::options(&state.db, &state.encryption_keys, &thread).await.unwrap().is_empty());
        rows.update_one(doc! {"_id": &service.id}, doc! {"$set":{"inference.voice":bson::Bson::Null,"inference.realtime":true}}).await.unwrap();
        assert!(super::super::assistant_voice::options(&state.db, &state.encryption_keys, &thread).await.unwrap().is_empty());
        state.db.drop().await.unwrap();
    }).await;
}

#[test]
fn duration_billing_requires_synced_authored_prices_and_meters_unpriced_byok() {
    use crate::models::service_billing::{
        BillingMetric, LanePriceComponent, LanePricing, PricingSyncStatus, ServiceBilling,
    };
    use credentials::duration_billing;
    let lane = LanePricing {
        metric: BillingMetric::InputTokens,
        credits_per_unit: "0.001".into(),
        lago_metric_code: "input".into(),
        sync_status: PricingSyncStatus::Synced,
        sync_error: None,
        components: vec![LanePriceComponent {
            metric: BillingMetric::VoiceSeconds,
            credits_per_unit: "0.01".into(),
            lago_metric_code: "duration".into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
        }],
    };
    assert!(
        duration_billing(&VoiceKeySource::Own, None)
            .unwrap()
            .is_none()
    );
    assert!(duration_billing(&VoiceKeySource::Platform, None).is_err());
    let legacy = crate::models::service_billing::ServiceBilling {
        platform_billable: true,
        ..Default::default()
    };
    assert!(
        duration_billing(&VoiceKeySource::Own, Some(&legacy))
            .unwrap()
            .is_none()
    );
    assert!(duration_billing(&VoiceKeySource::Platform, Some(&legacy)).is_err());
    for source in [VoiceKeySource::Own, VoiceKeySource::Platform] {
        for (primary, component, allowed) in [
            (PricingSyncStatus::Synced, PricingSyncStatus::Synced, true),
            (PricingSyncStatus::Pending, PricingSyncStatus::Synced, false),
            (PricingSyncStatus::Synced, PricingSyncStatus::Pending, false),
            (PricingSyncStatus::Failed, PricingSyncStatus::Synced, false),
            (PricingSyncStatus::Synced, PricingSyncStatus::Failed, false),
        ] {
            let mut configured = lane.clone();
            configured.sync_status = primary;
            configured.components[0].sync_status = component;
            let mut billing = ServiceBilling::default();
            match source {
                VoiceKeySource::Own => billing.byok_pricing = Some(configured),
                VoiceKeySource::Platform => billing.platform_key_pricing = Some(configured),
            }
            assert_eq!(duration_billing(&source, Some(&billing)).is_ok(), allowed);
        }
    }
    let mut billing = ServiceBilling {
        byok_pricing: Some(lane.clone()),
        ..Default::default()
    };
    assert!(
        duration_billing(&VoiceKeySource::Own, Some(&billing))
            .unwrap()
            .is_some()
    );
    assert!(duration_billing(&VoiceKeySource::Platform, Some(&billing)).is_err());
    billing.byok_pricing.as_mut().unwrap().components.clear();
    assert!(
        duration_billing(&VoiceKeySource::Own, Some(&billing))
            .unwrap()
            .is_none()
    );
    let mut platform_text = billing.clone();
    platform_text.platform_key_pricing = platform_text.byok_pricing.take();
    assert!(duration_billing(&VoiceKeySource::Platform, Some(&platform_text)).is_err());
    for status in [PricingSyncStatus::Pending, PricingSyncStatus::Failed] {
        billing.byok_pricing.as_mut().unwrap().sync_status = status;
        assert!(duration_billing(&VoiceKeySource::Own, Some(&billing)).is_err());
    }
    billing.byok_pricing.as_mut().unwrap().sync_status = PricingSyncStatus::Synced;
    billing.byok_pricing.as_mut().unwrap().metric = BillingMetric::VoiceSeconds;
    assert!(duration_billing(&VoiceKeySource::Own, Some(&billing)).is_ok());
    billing.byok_pricing = None;
    billing.platform_key_pricing = Some(lane);
    assert!(
        duration_billing(&VoiceKeySource::Own, Some(&billing))
            .unwrap()
            .is_none()
    );
    assert!(duration_billing(&VoiceKeySource::Platform, Some(&billing)).is_ok());
}
