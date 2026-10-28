//! Uses the existing real personal ingress and saved reply delivery fixtures.
use super::*;
use crate::models::assistant_conversation::{
    PlatformOriginSource as SavedSource, PlatformReplyRouting, TurnOrigin,
};
use crate::services::{
    assistant_acknowledgement_service as acks, assistant_model_denial_notice as notice,
    assistant_nyxagent_runtime as runtime, platform_personal_turn_service as personal,
};

struct NoModel(AtomicUsize);
#[async_trait]
impl NyxAgentEgress for NoModel {
    async fn request(
        &self,
        _: &NyxAgentExecutionContext,
        _: &RuntimeCredential,
        _: NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(AppError::Internal("notice must not call model".into()))
    }
}
async fn initial(j: &Journey, id: i64) -> engine::OwnerTurnClaim {
    let update = j.update(id, "Please manage my account", false);
    let verified =
        dispatch::verified_message(&j.profile.id, "100", 100, "test-observation", &update).unwrap();
    let IngressAdmission::Claimed {
        audience: ManagerAudience::Personal(admitted),
        ..
    } = j
        .producers
        .transport
        .admit_manager_update(&j.state.db, &verified)
        .await
        .unwrap()
    else {
        panic!("real personal ingress required");
    };
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&j.state.db, &j.owner)
        .await
        .unwrap();
    let ids = engine::OwnerTurnIds {
        conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let start = engine::TurnStart::from(
        &engine::parse_turn(br#"{"text":"Please manage my account"}"#).unwrap(),
    );
    let mut session = j.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let claim = Box::pin(personal::claim_personal_turn_in_session(
        &j.state.db,
        &mut session,
        &j.state.encryption_keys,
        &crate::services::platform_personal_binding_service::IdentityProducer,
        &crate::services::platform_manager_ingress_service::IngressTurnStore,
        &admitted,
        &start,
        &ids,
        &agent,
    ))
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    claim
}
async fn deny(
    j: &Journey,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> String {
    crate::services::assistant_settings_service::update(
        &j.state.db,
        &j.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.inference = Some(crate::models::downstream_service::ServiceInference {
        wire_protocol: crate::models::downstream_service::InferenceWireProtocol::OpenaiCompletions,
        model_list: true,
        realtime: false,
        voice: None,
    });
    let catalog_id = catalog.id.clone();
    j.state
        .db
        .collection("downstream_services")
        .insert_one(catalog)
        .await
        .unwrap();
    let connection = crate::services::assistant_authority_tests::connected(
        &j.state.db,
        &j.owner,
        "notice-model",
        "https://example.invalid",
    )
    .await;
    j.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&connection},
            doc! {"$set":{"catalog_service_id":catalog_id}},
        )
        .await
        .unwrap();
    let target = crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
        &j.state.db,
        &j.state.encryption_keys,
        &j.owner,
        &connection,
        None,
        None,
        crate::services::proxy_service::ProxyExecutionContext::new(
            None,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .target;
    let chat = acks::for_key(&j.state.db, &j.owner, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let mut auth = crate::test_utils::test_auth_user(&j.owner);
    auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    auth.scope = "proxy".into();
    auth.api_key_id = Some(row.credential_api_key_id.clone());
    auth.assistant_chat = Some(Arc::new(chat));
    let body = json!({"model":"model-v1","store":false,"input":[{"role":"user","content":"initial"}],"client_metadata":{"thread_id":"fixed"}});
    let response = Box::pin(
        crate::services::assistant_service_execution::check_http_projected(
            &j.state,
            &auth,
            Some(&connection),
            Some(&j.owner),
            &target,
            "POST",
            "responses",
            None,
            &serde_json::to_vec(&body).unwrap(),
            None,
            None,
            &Default::default(),
            &Default::default(),
        ),
    )
    .await
    .unwrap();
    let card = response.refusal.unwrap()["acknowledgement_id"]
        .as_str()
        .unwrap()
        .to_owned();
    acks::decide(&j.state.db, &j.owner, &row.id, &card, false)
        .await
        .unwrap();
    card
}
async fn finish(
    j: &Journey,
    row: &crate::models::assistant_conversation::AssistantConversation,
    result: &engine::TurnResult,
) -> ReplyRef {
    let reply = ReplyRef {
        turn: TurnRef {
            conversation_id: row.id.clone(),
            turn_id: row.active_turn.as_ref().unwrap().turn_id.clone(),
        },
        reply_id: uuid::Uuid::new_v4().to_string(),
    };
    engine::finish_turn(
        &j.state.db,
        row,
        &row.credential_api_key_id,
        &reply.reply_id,
        result,
    )
    .await
    .unwrap();
    reply
}
async fn run_notice(
    j: &Journey,
    row: &crate::models::assistant_conversation::AssistantConversation,
    egress: &NoModel,
) -> engine::TurnResult {
    let mut credential =
        crate::services::assistant_agent_credential_service::load_for_conversation(
            &j.state.db,
            &j.state.encryption_keys,
            &j.owner,
            &row.id,
        )
        .await
        .unwrap()
        .unwrap();
    let (sender, _) = tokio::sync::broadcast::channel(32);
    let mut events = runtime::Events {
        checkpoint: None,
        sender,
        cursor: 0,
        reset_notified: false,
    };
    Box::pin(runtime::execute_owner_turn(
        &j.state,
        egress,
        &runtime::owner_context(row).unwrap(),
        row,
        "saved notice",
        &mut credential,
        None,
        &mut events,
        "notice-reply",
        &mut String::new(),
        &mut None,
    ))
    .await
    .unwrap()
}
fn event_start(id: &str) -> engine::TurnStart {
    let mut start = engine::TurnStart::from(
        &engine::parse_turn(
            &serde_json::to_vec(&json!({"text":"event","conversation_id":id})).unwrap(),
        )
        .unwrap(),
    );
    start.origin = TurnOrigin::Event;
    start.text.clear();
    start
}

#[tokio::test]
async fn model_deny_saved_captured_platform_uses_distinct_reply_and_original_source_fence() {
    let j = Journey::new().await;
    j.link().await;
    let initial = initial(&j, 1900).await;
    let original = initial
        .conversation
        .active_turn
        .as_ref()
        .unwrap()
        .platform_reply
        .clone()
        .unwrap();
    let card = deny(&j, &initial.conversation).await;
    let old = finish(
        &j,
        &initial.conversation,
        &engine::TurnResult {
            text: "Waiting for decision".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    assert!(notify_local(&j, &old).await.unwrap().is_some());
    let before = j.sends().await.len();
    let row = engine::begin_turn(
        &j.state.db,
        &j.owner,
        event_start(&initial.conversation.id),
        &j.state.encryption_keys,
    )
    .await
    .unwrap();
    let turn = row.active_turn.as_ref().unwrap();
    assert!(
        matches!(&turn.platform_origin.as_ref().unwrap().source,SavedSource::ThreadEvent{admission_id}if admission_id==&turn.turn_id)
    );
    assert_eq!(turn.platform_reply.as_ref(), Some(&original));
    assert_eq!(turn.events[0].id, card);
    let egress = NoModel(AtomicUsize::new(0));
    let result = run_notice(&j, &row, &egress).await;
    assert!(result.error.is_none());
    assert_eq!(egress.0.load(Ordering::SeqCst), 0);
    let reply = finish(&j, &row, &result).await;
    assert_ne!(reply.reply_id, old.reply_id);
    assert_ne!(reply.turn.turn_id, old.turn.turn_id);
    let saved = j
        .state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"_id":&reply.reply_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.platform_reply, Some(original.clone()));
    // The real captured-destination transport limits ordinary sends to one
    // per second. Keep its policy when sending the distinct retained reply.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let outcome = notify_local(&j, &reply).await.unwrap().unwrap();
    assert!(matches!(outcome, DeliveryOutcome::Sent { .. }));
    assert!(matches!(
        notify_local(&j, &reply).await.unwrap(),
        Some(DeliveryOutcome::Duplicate)
    ));
    assert_eq!(j.sends().await.len(), before + 1);
    let delivery = j
        .state
        .db
        .collection::<PlatformManagerDelivery>(
            crate::models::platform_manager_delivery::COLLECTION_NAME,
        )
        .find_one(doc! {"reference.reply_id":&reply.reply_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        delivery.origin_turn_id.as_deref(),
        Some(old.turn.turn_id.as_str())
    );
    let receipt = j
        .state
        .db
        .collection::<PlatformManagerIngress>(INGRESS)
        .find_one(doc! {"execution.turn_id":&old.turn.turn_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.reply_id.as_ref(), Some(&old.reply_id));
    assert!(receipt.phase == crate::models::platform_manager_ingress::IngressPhase::Settled);
    assert_eq!(
        j.state
            .db
            .collection::<Document>("assistant_model_denial_notices")
            .count_documents(doc! {"_id":card,"state":"settled","reply_id":&reply.reply_id})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        j.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn model_deny_unproven_or_expired_captured_destination_never_uses_current_thread() {
    for drift in ["missing", "expired"] {
        let j = Journey::new().await;
        j.link().await;
        let initial = initial(&j, 1901).await;
        let source=j.state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc!{"turn_id":&initial.conversation.active_turn.as_ref().unwrap().turn_id,"role":"user"}).await.unwrap().unwrap();
        let mut route = source.platform_reply.clone();
        if drift == "missing" {
            route = None;
        } else if let Some(PlatformReplyRouting::Captured { expires_at, .. }) = &mut route {
            *expires_at = Utc::now() - Duration::seconds(1);
        }
        j.state
            .db
            .collection::<Document>(MESSAGES)
            .update_one(
                doc! {"_id":source.id},
                doc! {"$set":{"platform_reply":bson::to_bson(&route).unwrap()}},
            )
            .await
            .unwrap();
        let before = j.sends().await.len();
        let card = deny(&j, &initial.conversation).await;
        notice::recover(&j.state.db).await.unwrap();
        assert_eq!(
            j.state
                .db
                .collection::<Document>("assistant_model_denial_notices")
                .find_one(doc! {"_id":card})
                .await
                .unwrap()
                .unwrap()
                .get_str("state")
                .unwrap(),
            "unsupported"
        );
        assert!(
            engine::get(&j.state.db, &j.owner, &initial.conversation.id)
                .await
                .unwrap()
                .pending_events
                .is_empty()
        );
        assert_eq!(j.sends().await.len(), before);
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 0);
        j.state.db.drop().await.unwrap();
    }
}

/// Actual platform claim/admission and Mongo transaction boundary. Aborting a
/// platform claim cannot publish recorder coverage for a turn that never began.
#[tokio::test]
async fn platform_recorder_admission_abort_and_same_input_retry_commit_exact_new_turn_marker() {
    let j=Journey::new().await;j.link().await;
    crate::services::security_readiness_service::ready_test_replica(&j.state.db).await;
    let update=j.update(1980,"Please manage my account",false);
    let verified=dispatch::verified_message(&j.profile.id,"100",100,"test-observation",&update).unwrap();
    let IngressAdmission::Claimed {audience:ManagerAudience::Personal(admitted),..}=j.producers.transport
        .admit_manager_update(&j.state.db,&verified).await.unwrap() else {panic!("real personal ingress required");};
    let agent=crate::services::assistant_team_service::ensure_nyxbot(&j.state.db,&j.owner).await.unwrap();
    let ids=engine::OwnerTurnIds {conversation_id:format!("nyxa-{}",uuid::Uuid::new_v4().simple()),turn_id:uuid::Uuid::new_v4().to_string(),message_id:uuid::Uuid::new_v4().to_string()};
    let start=engine::TurnStart::from(&engine::parse_turn(br#"{"text":"Please manage my account"}"#).unwrap());
    let markers=j.state.db.collection::<Document>(crate::services::assistant_service_execution::COLLECTION);
    let scope=||doc! {"kind":"model_control_recorder_admission","user_id":&j.owner,"conversation_id":&ids.conversation_id,"turn_id":&ids.turn_id};
    for commit in [false,true] {
        let mut session=j.state.db.client().start_session().await.unwrap();session.start_transaction().await.unwrap();
        let claim=Box::pin(personal::claim_personal_turn_in_session(&j.state.db,&mut session,&j.state.encryption_keys,
            &crate::services::platform_personal_binding_service::IdentityProducer,
            &crate::services::platform_manager_ingress_service::IngressTurnStore,&admitted,&start,&ids,&agent)).await.unwrap();
        assert_eq!(claim.conversation.active_turn.as_ref().unwrap().turn_id,ids.turn_id);
        let marker=markers.find_one(scope()).session(&mut session).await.unwrap().unwrap();
        assert_eq!(marker.get_str("api_key_id").unwrap(),claim.conversation.credential_api_key_id);
        assert_eq!(marker.get_str("revision").unwrap(),crate::services::assistant_model_control::READINESS_CAPABILITY);
        assert!(!marker.contains_key("expires_at"));
        if commit {session.commit_transaction().await.unwrap();} else {
            session.abort_transaction().await.unwrap();
            assert!(markers.find_one(scope()).await.unwrap().is_none());
            assert!(j.state.db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
                .find_one(doc! {"_id":&ids.conversation_id}).await.unwrap().is_none());
        }
    }
    assert_eq!(markers.count_documents(scope()).await.unwrap(),1);
    assert_eq!(j.sends().await.len(),0);
    j.state.db.drop().await.unwrap();
}
