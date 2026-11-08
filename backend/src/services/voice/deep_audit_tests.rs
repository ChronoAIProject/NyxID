//! Adversarial regressions for device authority and recovery.
use super::*;
use crate::services::{
    assistant_acknowledgement_service as acks, assistant_nyxagent as engine, voice,
};

async fn device_task(
    name: &str,
) -> (
    Fixture,
    voice::client_key::ClientKey,
    crate::models::assistant_voice_session::VoiceSession,
    crate::models::assistant_voice::VoiceRequest,
) {
    let (f, provider, _, raw) = setup(name).await;
    let device = auth(&f, &raw).await;
    let binding = keys::authorize(&f.state.db, &device, &f.row.id)
        .await
        .unwrap()
        .unwrap();
    assert!(crate::mw::auth::require_first_party_human(&device).is_err());
    let call = session::admit_for_client_key(
        &f.state.db,
        &f.owner,
        &f.row.id,
        &Uuid::new_v4().to_string(),
        platform(&provider),
        "fixture".into(),
        "worker",
        Some(binding.clone()),
    )
    .await
    .unwrap();
    Box::pin(engine::finish_turn(
        &f.state.db,
        &f.row,
        &f.row.credential_api_key_id,
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
    let request = voice::transcript::delegate_task(
        &f.state.db,
        &call,
        "audit-call",
        "Delete the selected object",
        crate::models::assistant_voice::VoiceTaskSource {
            model: "fixture".into(),
            response_id: "response".into(),
            delegation_id: "delegation".into(),
        },
    )
    .await
    .unwrap();
    Box::pin(engine::begin_turn_with_voice(
        &f.state.db,
        &f.owner,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            conversation_id: Some(f.row.id.clone()),
            agent_id: None,
            text: "Delete the selected object".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
        Some(&request.id),
    ))
    .await
    .unwrap();
    (f, binding, call, request)
}

#[tokio::test]
async fn audit_device_voice_cannot_decide_an_owner_action_but_owner_can() {
    Box::pin(async {
        let (f, _, call, _) = device_task("audit_device_owner_decision").await;
        let mut chat = acks::for_key(&f.state.db, &f.owner, Some(&f.row.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        // Seed a legacy owner card. New Voice requests cannot create cards,
        // but device authority must still never decide an existing owner card.
        chat.execution_voice_task = false;
        let card = Box::pin(acks::request(
            &f.state.db,
            &chat,
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
        .unwrap();
        let result = Box::pin(acks::decide_with_voice(
            &f.state.db,
            &f.owner,
            None,
            &card.id,
            true,
            acks::Decider::User,
            None,
            Some(voice::confirmation::DecisionFence {
                session: call.clone(),
                authority_digest: voice::confirmation::card_digest(&card),
                until_ms: chrono::Utc::now().timestamp_millis() + 60_000,
                audit_key: f.state.audit_chain_hmac_key.clone(),
            }),
        ))
        .await;
        assert!(
            matches!(result, Err(crate::errors::AppError::Forbidden(_))),
            "A service Key is not owner confirmation authority"
        );
        let saved = f
            .state
            .db
            .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                crate::models::assistant_acknowledgement::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&card.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, "pending");
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
                .count_documents(doc! {"acknowledgement_id":&card.id})
                .await
                .unwrap(),
            0
        );
        Box::pin(acks::decide(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &card.id,
            true,
        ))
        .await
        .unwrap();
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
                .count_documents(doc! {"acknowledgement_id":&card.id})
                .await
                .unwrap(),
            1
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn audit_explicit_deny_blocks_voice_service_without_requesting_approval() {
    Box::pin(async {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let (f, provider, service, raw) = setup("audit_voice_deny_ingress").await;
        let device = auth(&f,&raw).await;
        let identity = voice::execution::resolve(&f.state.db,&device,&service).await.unwrap();
        crate::services::approval_service::set_service_approval_config(&f.state.db,&f.owner,
            identity.service.catalog_service_id.as_deref().unwrap(),"Voice",None,None,None,
            Some(&crate::models::service_approval_config::ApprovalEffect::Deny)).await.unwrap();
        assert!(matches!(keys::authorize(&f.state.db,&device,&f.row.id).await,Err(crate::errors::AppError::Forbidden(_))));
        let (_, router) = crate::routes::build_router_with_state(f.state.clone());
        let app = router.with_state(f.state.clone());
        {
            let uri = format!("/api/v1/proxy/nyxbot-voice/{service}/voice/sessions");
            let response = app.clone().oneshot(Request::builder().method("POST").uri(&uri)
                .header("authorization",format!("Bearer {raw}")).header("content-type","application/json")
                .body(Body::from(serde_json::json!({"service_id":service,"client_request_id":Uuid::new_v4(),
                    "preferences":platform(&provider),"sdp_offer":"invalid"}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(),403,"Deny must precede provider admission: {uri}");
        }
        let response = app.oneshot(Request::builder().uri(format!("/api/v1/proxy/nyxbot-voice/{service}/voice/agents"))
            .header("authorization",format!("Bearer {raw}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),403,"An explicitly denied service cannot be discovered through its ingress");
        assert_eq!(f.state.db.collection::<bson::Document>("approval_requests").count_documents(doc!{}).await.unwrap(),0);
        f.state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn audit_unconfirmed_recovery_preserves_usage_until_bounded_finalization() {
    Box::pin(async {
        for native in [true, false] {
        let (state, _, mut call) = voice::tests::setup(if native {"audit_native_unconfirmed_recovery"} else {"audit_live_unconfirmed_recovery"}).await;
        if native {
            use crate::services::billing::{BillingIngress, BillingRouteContext, NodeIntent};
            let context = BillingRouteContext::new(
                BillingIngress::LlmProvider, "fixture".into(), call.user_id.clone(),
                call.user_id.clone(), None, None, Some(call.preferences.service_id.clone()),
                Some("fixture-voice".into()), NodeIntent::Direct, "bearer".into(),
                crate::models::usage_meter::CredentialClass::UserOwned,
                crate::models::service_billing::BillingMetric::Tokens, None, false,
            ).with_platform_metering(false);
            call = voice::session::write(&state.db, &call, doc! {"responses_model":"fixture-model"}).await.unwrap();
            voice::responses::reserve(&state,&mut call,&context).await.unwrap();
            assert!(call.responses.reserved());
        }
        state
            .db
            .collection::<bson::Document>(crate::models::assistant_voice_session::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&call.id},
                doc! {"$set":{
                "state":"active", "protocol":"openai_live", "responses_model":if native {bson::Bson::String("fixture-model".into())} else {bson::Bson::Null},
                "provider_session_id":"live_unreachable", "observed_seconds":12_i64,
                "lease_until":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        // The configured provider connection is unavailable; no upstream request is made.
        voice::runtime::recover(&state).await.unwrap();
        let recovered = state
            .db
            .collection::<crate::models::assistant_voice_session::VoiceSession>(
                crate::models::assistant_voice_session::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&call.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.end_reason.as_deref(), Some("close_unconfirmed"));
        assert!(
            !recovered.final_usage_confirmed,
            "Failed attach/close cannot confirm final provider usage"
        );
        assert_eq!(recovered.observed_seconds, 12);
        assert!(recovered.end_requested);
        assert!(!recovered.billing_finalized);
        assert!(
            !recovered.responses.finalized(),
            "Do not freeze incomplete response usage while closure is uncertain"
        );
        // A second worker must preserve uncertainty, too. Once the bounded
        // reconciliation deadline passes, release holds without inventing usage.
        for expired in [false, true] {
            let mut fields=doc!{"lease_until":bson::DateTime::from_millis(0)};
            if expired {
                fields.insert("deadline",bson::DateTime::from_millis(0));
                fields.insert("reconcile_deadline",bson::DateTime::from_millis(0));
            }
            state.db.collection::<bson::Document>(crate::models::assistant_voice_session::COLLECTION_NAME)
                .update_one(doc!{"_id":&call.id},doc!{"$set":fields}).await.unwrap();
            voice::runtime::recover(&state).await.unwrap();
            let current = state.db.collection::<crate::models::assistant_voice_session::VoiceSession>(crate::models::assistant_voice_session::COLLECTION_NAME)
                .find_one(doc!{"_id":&call.id}).await.unwrap().unwrap();
            assert!(!current.final_usage_confirmed);
            assert_eq!(current.observed_seconds,12);
            assert_eq!(current.billing_finalized,expired);
            assert_eq!(current.live_slot,!expired);
            assert_eq!(current.responses.finalized(),native && expired);
        }
        assert!(session::claim_orphans(&state.db,"another-worker").await.unwrap().is_empty());
        state.db.drop().await.unwrap();
        }
    })
    .await;
}

#[tokio::test]
async fn audit_ended_grok_relay_recovery_finishes_without_waiting_for_remote_close() {
    Box::pin(async {
        let (state,_,call)=voice::tests::setup("audit_grok_ended_recovery").await;
        state.db.collection::<bson::Document>(crate::models::assistant_voice_session::COLLECTION_NAME)
            .update_one(doc!{"_id":&call.id},doc!{"$set":{"protocol":"xai_realtime","end_requested":true,"lease_until":bson::DateTime::from_millis(0)}})
            .await.unwrap();
        voice::runtime::recover(&state).await.unwrap();
        let recovered=state.db.collection::<crate::models::assistant_voice_session::VoiceSession>(crate::models::assistant_voice_session::COLLECTION_NAME)
            .find_one(doc!{"_id":&call.id}).await.unwrap().unwrap();
        assert!(!recovered.live_slot);
        assert!(recovered.billing_finalized);
        assert!(recovered.final_usage_confirmed);
        state.db.drop().await.unwrap();
    }).await;
}

#[tokio::test]
async fn audit_device_revocation_ends_audio_but_only_stop_cancels_accepted_bot_work() {
    Box::pin(async {
        use crate::models::assistant_voice::RequestState;
        let (f, binding, call, request) =
            device_task("audit_device_independent_task_lifetime").await;
        f.state
            .db
            .collection::<bson::Document>(KEYS)
            .update_one(
                doc! {"_id":&binding.key_id},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
        assert!(keys::revalidate(&f.state.db, &call).await.is_err());
        keys::end(&f.state.db, &call).await.unwrap();
        assert!(
            session::get(&f.state.db, &f.owner, &f.row.id, &call.id)
                .await
                .unwrap()
                .end_requested
        );
        let active = engine::get(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .active_turn
            .unwrap();
        assert_eq!(
            active.voice_request_id.as_deref(),
            Some(request.id.as_str())
        );
        assert!(!active.stop_requested);
        assert_eq!(
            crate::services::assistant_voice::get(&f.state.db, &f.owner, &f.row.id, &request.id)
                .await
                .unwrap()
                .state,
            RequestState::Claimed
        );
        assert!(
            acks::for_key(&f.state.db, &f.owner, Some(&f.row.credential_api_key_id))
                .await
                .unwrap()
                .is_some(),
            "The selected Bot retains its own execution authority"
        );
        crate::services::assistant_voice::cancel(&f.state.db, &f.owner, &f.row.id, &request.id)
            .await
            .unwrap();
        assert_eq!(
            crate::services::assistant_voice::get(&f.state.db, &f.owner, &f.row.id, &request.id)
                .await
                .unwrap()
                .state,
            RequestState::Cancelled
        );
        assert!(
            engine::get(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap()
                .active_turn
                .unwrap()
                .stop_requested
        );
        f.state.db.drop().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn audit_proxy_only_key_executes_voice_service_but_cannot_publish_or_manage() {
    Box::pin(async {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let (f, provider, service, raw) = setup("audit_voice_route_classification").await;
        let device = auth(&f, &raw).await;
        f.state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":device.api_key_id.as_deref().unwrap()},
                doc! {"$set":{"scopes":"proxy"}},
            )
            .await
            .unwrap();
        let (_, router) = crate::routes::build_router_with_state(f.state.clone());
        let app = router.with_state(f.state.clone());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/proxy/nyxbot-voice/{service}/voice/sessions"
                    ))
                    .header("authorization", format!("Bearer {raw}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"service_id":service,"client_request_id":Uuid::new_v4(),
                "preferences":platform(&provider),"sdp_offer":"invalid"})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            status, 400,
            "A proxy-only key must reach the Voice transport validator: {body}"
        );
        assert_eq!(body["details"]["stage"], "transport");
        for (method, uri) in [
            ("POST", "/api/v1/keys".to_owned()),
            (
                "PUT",
                format!(
                    "/api/v1/assistant/nyxagent/agents/{}/voice-service",
                    f.chat.agent_id
                ),
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("authorization", format!("Bearer {raw}"))
                        .header("service-execution-route", "true")
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 403);
        }
        f.state.db.drop().await.unwrap();
    })
    .await;
}
