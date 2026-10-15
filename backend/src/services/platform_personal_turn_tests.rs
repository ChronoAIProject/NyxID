use super::*;
use crate::models::{
    platform_manager_ingress::{COLLECTION_NAME as RECEIPTS, IngressPhase, PlatformManagerIngress},
    platform_personal_binding::{COLLECTION_NAME as BINDINGS, PlatformPersonalBinding},
};
use contract::PersonalTurnProducer;

struct RefusedEgress;
#[async_trait::async_trait]
impl contract::NyxAgentEgress for RefusedEgress {
    async fn request(
        &self,
        authority: &contract::NyxAgentExecutionContext,
        _: &contract::RuntimeCredential,
        _: contract::NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        assert!(matches!(
            authority,
            contract::NyxAgentExecutionContext::Owner {
                platform_origin: Some(_),
                ..
            }
        ));
        Err(AppError::Internal("test upstream unavailable".into()))
    }
}

struct CountingEgress(std::sync::atomic::AtomicUsize);
#[async_trait::async_trait]
impl contract::NyxAgentEgress for CountingEgress {
    async fn request(
        &self,
        authority: &contract::NyxAgentExecutionContext,
        _: &contract::RuntimeCredential,
        request: contract::NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        assert!(matches!(
            authority,
            contract::NyxAgentExecutionContext::Owner {
                platform_origin: Some(_),
                ..
            }
        ));
        assert_eq!(request.relative_path, "v1/responses");
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(AppError::Internal("test upstream unavailable".into()))
    }
}

#[test]
fn legacy_turn_without_execution_marker_remains_readable() {
    let turn: model::ActiveTurn = bson::from_document(doc! {
        "turn_id":uuid::Uuid::new_v4().to_string(),
        "started_at":bson::DateTime::now(),
    })
    .unwrap();
    assert!(!turn.execution_started);
    assert!(turn.platform_origin.is_none());
}

pub(super) async fn fixture(name: &str) -> (AppState, contract::AdmittedPersonalUpdate) {
    let (db, update, owner) =
        super::super::platform_personal_binding_service::tests::fixture(name).await;
    engine::ensure_indexes(&db).await.unwrap();
    super::super::platform_manager_ingress_service::ensure_indexes(&db)
        .await
        .unwrap();
    super::super::feature_flag_service::set_platform_override(
        &db,
        super::super::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
        &super::super::feature_flag_service::FlagTarget::Global,
        true,
        "admin",
    )
    .await
    .unwrap();
    let binding = super::super::platform_personal_binding_service::tests::binding(&update, &owner);
    let admitted = contract::AdmittedPersonalUpdate {
        ingress: contract::IngressClaim {
            receipt_id: uuid::Uuid::new_v4().to_string(),
            claim_id: uuid::Uuid::new_v4().to_string(),
        },
        authority: super::super::platform_personal_binding_service::authority(&binding, 1),
    };
    db.collection::<PlatformPersonalBinding>(BINDINGS)
        .insert_one(binding)
        .await
        .unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    super::super::platform_manager_ingress_service::record_admission_in_session(
        &db,
        &mut session,
        &update,
        &admitted.authority.manager,
        &admitted.ingress,
        &contract::ManagerAudience::Personal(admitted.clone()),
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    (crate::test_utils::test_app_state(db), admitted)
}

fn personal(state: AppState) -> PersonalTurns {
    PersonalTurns {
        state,
        identity: Arc::new(super::super::platform_personal_binding_service::IdentityProducer),
        ingress: Arc::new(super::super::platform_manager_ingress_service::IngressTurnStore),
        egress: Arc::new(RefusedEgress),
    }
}

#[tokio::test]
async fn first_use_resolves_real_nyxbot_and_emits_only_durable_reply_reference() {
    let (state, admitted) = fixture("personal_first_start").await;
    let db = state.db.clone();
    let service = personal(state);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    let start = service
        .start_personal_turn(
            &db,
            &admitted,
            contract::TurnInput {
                text: "hello".into(),
            },
            sender,
        )
        .await
        .unwrap();
    let contract::PersonalTurnStart::Started { turn, destination } = start else {
        panic!("start required");
    };
    assert!(
        matches!(&destination.authority,contract::ReplyAuthority::Personal {turn:delivery,..} if *delivery==turn)
    );
    let reply = tokio::time::timeout(Duration::from_secs(15), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let contract::ExecutionEvent::ReplyReady(reply) = reply else {
        panic!("durable reply required");
    };
    assert_eq!(reply.turn, turn);
    let message=db.collection::<crate::models::assistant_message::AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {"_id":&reply.reply_id,"conversation_id":&turn.conversation_id,"turn_id":&turn.turn_id}).await.unwrap().unwrap();
    let origin = message.platform_origin.unwrap();
    let binding = db
        .collection::<PlatformPersonalBinding>(BINDINGS)
        .find_one(doc! {"_id":&admitted.authority.binding_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        binding.nyxbot_agent_id.as_deref(),
        Some(origin.thread.nyxbot_agent_id.as_str())
    );
    assert!(binding.active_turn.is_none());
    let receipt = db
        .collection::<PlatformManagerIngress>(RECEIPTS)
        .find_one(doc! {"_id":&admitted.ingress.receipt_id})
        .await
        .unwrap()
        .unwrap();
    assert!(receipt.phase == IngressPhase::Settled);
    assert_eq!(receipt.reply_id.as_deref(), Some(reply.reply_id.as_str()));
    assert!(
        matches!(receiver.recv().await,Some(contract::ExecutionEvent::Settled {turn:done}) if done==turn)
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn revoked_admission_cannot_claim_thread_key_turn_or_receipt() {
    let (state, admitted) = fixture("personal_revoked_start").await;
    let db = state.db.clone();
    db.collection::<bson::Document>(BINDINGS)
        .update_one(
            doc! {"_id":&admitted.authority.binding_id},
            doc! {"$set":{"state":"revoked"}},
        )
        .await
        .unwrap();
    let service = personal(state);
    let (sender, _) = tokio::sync::mpsc::channel(1);
    assert!(
        service
            .start_personal_turn(
                &db,
                &admitted,
                contract::TurnInput {
                    text: "old admission".into()
                },
                sender
            )
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<bson::Document>(CONVERSATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<bson::Document>(CREDENTIALS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let binding = db
        .collection::<PlatformPersonalBinding>(BINDINGS)
        .find_one(doc! {"_id":&admitted.authority.binding_id})
        .await
        .unwrap()
        .unwrap();
    assert!(binding.conversation_id.is_none());
    let receipt = db
        .collection::<PlatformManagerIngress>(RECEIPTS)
        .find_one(doc! {"_id":&admitted.ingress.receipt_id})
        .await
        .unwrap()
        .unwrap();
    assert!(receipt.phase == IngressPhase::Admitted);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_stop_settles_revoked_turn_and_retains_independent_work() {
    let (state, admitted) = fixture("personal_exact_apply").await;
    let db = &state.db;
    let service = personal(state.clone());
    let agent =
        super::super::assistant_team_service::ensure_nyxbot(db, &admitted.authority.user_id)
            .await
            .unwrap();
    let request = engine::parse_turn(br#"{"text":"work"}"#).unwrap();
    let ids = engine::OwnerTurnIds {
        conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let claim = claim_personal_turn_in_session(
        db,
        &mut session,
        &state.encryption_keys,
        service.identity.as_ref(),
        service.ingress.as_ref(),
        &admitted,
        &engine::TurnStart::from(&request),
        &ids,
        &agent,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    let row = claim.conversation;
    let origin = contract_origin(
        row.active_turn
            .as_ref()
            .unwrap()
            .platform_origin
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let turn = contract::TurnRef {
        conversation_id: row.id.clone(),
        turn_id: ids.turn_id,
    };
    db.collection::<bson::Document>(crate::models::async_service_operation::COLLECTION_NAME).insert_one(doc! {"_id":"independent","conversation_id":&row.id,"user_id":&row.user_id,"status":"pending"}).await.unwrap();
    session.start_transaction().await.unwrap();
    let contract::ExactStopOutcome::Captured(effects) = service
        .stop_exact_platform_turn(db, &mut session, &origin, &turn)
        .await
        .unwrap()
    else {
        panic!("captured");
    };
    db.collection::<bson::Document>(BINDINGS)
        .update_one(
            doc! {"_id":&admitted.authority.binding_id},
            doc! {"$set":{"state":"revoked"}},
        )
        .session(&mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let progress = service.apply_exact_platform_stop(&effects).await.unwrap();
    assert_eq!(progress.settlement, contract::ExactStopSettlement::Settled);
    assert!(
        engine::get(db, &row.user_id, &row.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    let retained = db
        .collection::<bson::Document>(crate::models::async_service_operation::COLLECTION_NAME)
        .find_one(doc! {"_id":"independent"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.get_str("status").unwrap(), "pending");
    let again = service.apply_exact_platform_stop(&effects).await.unwrap();
    assert_eq!(
        again.settlement,
        contract::ExactStopSettlement::AlreadySettled
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn personal_credential_recovery_adopts_exact_successor_and_revocation_prevents_mutation() {
    let (state, admitted) = fixture("personal_credential_recovery").await;
    let db = &state.db;
    let service = personal(state.clone());
    let agent =
        super::super::assistant_team_service::ensure_nyxbot(db, &admitted.authority.user_id)
            .await
            .unwrap();
    let start = engine::TurnStart::from(&engine::parse_turn(br#"{"text":"work"}"#).unwrap());
    let ids = engine::OwnerTurnIds {
        conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let claim = claim_personal_turn_in_session(
        db,
        &mut session,
        &state.encryption_keys,
        service.identity.as_ref(),
        service.ingress.as_ref(),
        &admitted,
        &start,
        &ids,
        &agent,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    let row = claim.conversation;
    let old = claim.credential.api_key_id;
    use super::super::assistant_agent_credential_service as credentials;
    // Generic metadata/ordinary replacement cannot borrow a platform turn.
    assert!(
        credentials::replace(db, &state.encryption_keys, &row.user_id, &row.id, &old)
            .await
            .is_err()
    );
    let successor = credentials::replace_for_turn(db, &state.encryption_keys, &row, &old)
        .await
        .unwrap();
    assert_ne!(old, successor.api_key_id);
    let rebound = engine::get(db, &row.user_id, &row.id).await.unwrap();
    assert_eq!(rebound.credential_api_key_id, successor.api_key_id);
    assert!(rebound.nyxagent_session_id.is_none());
    assert!(
        credentials::load_for_conversation(db, &state.encryption_keys, &row.user_id, &row.id)
            .await
            .unwrap()
            .is_some()
    );
    let origin = contract_origin(
        row.active_turn
            .as_ref()
            .unwrap()
            .platform_origin
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let turn = contract::TurnRef {
        conversation_id: row.id.clone(),
        turn_id: ids.turn_id,
    };
    session.start_transaction().await.unwrap();
    assert_eq!(
        fence_execution_in_session(
            db,
            &mut session,
            service.identity.as_ref(),
            &row.user_id,
            &old,
            &turn,
            &origin
        )
        .await
        .unwrap(),
        contract::FenceOutcome::Unavailable(contract::UnavailableReason::OriginRevoked)
    );
    assert_eq!(
        fence_execution_in_session(
            db,
            &mut session,
            service.identity.as_ref(),
            &row.user_id,
            &successor.api_key_id,
            &turn,
            &origin
        )
        .await
        .unwrap(),
        contract::FenceOutcome::Authorized
    );
    session.abort_transaction().await.unwrap();
    db.collection::<bson::Document>(BINDINGS)
        .update_one(
            doc! {"_id":&admitted.authority.binding_id},
            doc! {"$set":{"state":"revoked"}},
        )
        .await
        .unwrap();
    assert!(
        credentials::replace_for_turn(db, &state.encryption_keys, &row, &successor.api_key_id)
            .await
            .is_err()
    );
    assert!(
        update_platform_turn(db, &row, doc! {"$set":{"active_turn.continuations":1_i64}})
            .await
            .is_err()
    );
    let retained = engine::get(db, &row.user_id, &row.id).await.unwrap();
    assert_eq!(retained.credential_api_key_id, successor.api_key_id);
    assert_eq!(retained.active_turn.unwrap().continuations, 0);
    assert!(
        db.collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! {"_id":&successor.api_key_id,"user_id":&row.user_id,"purpose":"general"})
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        db.collection::<bson::Document>(CREDENTIALS)
            .count_documents(doc! {"conversation_id":&row.id})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn execution_factory_resumes_exact_input_once_across_competing_workers() {
    let (state, admitted) = fixture("personal_execution_factory").await;
    let db = &state.db;
    let service = personal(state.clone());
    let agent =
        super::super::assistant_team_service::ensure_nyxbot(db, &admitted.authority.user_id)
            .await
            .unwrap();
    let start = engine::TurnStart::from(&engine::parse_turn(br#"{"text":"work"}"#).unwrap());
    let ids = engine::OwnerTurnIds {
        conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let claim = claim_personal_turn_in_session(
        db,
        &mut session,
        &state.encryption_keys,
        service.identity.as_ref(),
        service.ingress.as_ref(),
        &admitted,
        &start,
        &ids,
        &agent,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    let authority = runtime::owner_context(&claim.conversation).unwrap();
    let egress = Arc::new(CountingEgress(std::sync::atomic::AtomicUsize::new(0)));
    let execution = runtime::producer(
        state.clone(),
        egress.clone(),
        super::super::platform_public_credential_service::producer(state.clone()),
    );
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    assert!(
        execution
            .execute_nyxagent_turn(
                db,
                authority.clone(),
                contract::TurnInput {
                    text: "changed".into()
                },
                sender.clone(),
            )
            .await
            .is_err()
    );
    let mut wrong = authority.clone();
    if let contract::NyxAgentExecutionContext::Owner {
        platform_origin: Some(origin),
        ..
    } = &mut wrong
    {
        origin.authority.binding_generation += 1;
    }
    assert!(
        execution
            .execute_nyxagent_turn(
                db,
                wrong,
                contract::TurnInput {
                    text: "work".into()
                },
                sender.clone(),
            )
            .await
            .is_err()
    );
    assert!(
        !engine::get(db, &claim.conversation.user_id, &claim.conversation.id)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .execution_started
    );
    let (first, second) = tokio::join!(
        execution.execute_nyxagent_turn(
            db,
            authority.clone(),
            contract::TurnInput {
                text: "work".into()
            },
            sender.clone()
        ),
        execution.execute_nyxagent_turn(
            db,
            authority,
            contract::TurnInput {
                text: "work".into()
            },
            sender
        ),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(egress.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    let Some(contract::ExecutionEvent::ReplyReady(reply)) = receiver.recv().await else {
        panic!("durable reply required");
    };
    assert_eq!(reply.turn.turn_id, ids.turn_id);
    assert!(matches!(
        receiver.recv().await,
        Some(contract::ExecutionEvent::Settled { .. })
    ));
    assert!(receiver.recv().await.is_none());
    assert_eq!(
        db.collection::<bson::Document>(CONVERSATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<bson::Document>(CREDENTIALS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn execution_factory_selects_public_prepare_and_rejects_ordinary_context_before_owner_io() {
    let db = mongodb::Client::with_uri_str("mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=25")
        .await
        .unwrap()
        .database("no_owner_io");
    let state = crate::test_utils::test_app_state(db.clone());
    let egress = Arc::new(CountingEgress(std::sync::atomic::AtomicUsize::new(0)));
    let execution = runtime::producer(
        state.clone(),
        egress.clone(),
        super::super::platform_public_credential_service::producer(state),
    );
    let turn = contract::TurnRef {
        conversation_id: "missing".into(),
        turn_id: uuid::Uuid::new_v4().to_string(),
    };
    let (sender, _) = tokio::sync::mpsc::channel(1);
    let result = execution
        .execute_nyxagent_turn(
            &db,
            contract::NyxAgentExecutionContext::Owner {
                user_id: "missing".into(),
                turn: turn.clone(),
                platform_origin: None,
            },
            contract::TurnInput {
                text: "work".into(),
            },
            sender.clone(),
        )
        .await;
    assert!(matches!(result, Err(AppError::AssistantTurnRequired)));
    let public = contract::PublicExecutionContext {
        ingress: contract::PublicIngress {
            ingress: contract::IngressClaim {
                receipt_id: uuid::Uuid::new_v4().to_string(),
                claim_id: uuid::Uuid::new_v4().to_string(),
            },
            manager: contract::ManagerAuthority {
                profile_id: "manager".into(),
                manager_bot_id: "bot".into(),
                telegram_bot_id: 1,
                manager_authority_epoch: 1,
            },
            sender_id: 1,
            chat_id: 1,
            public_policy_revision: 1,
        },
        public_session_id: "public".into(),
        turn,
        sponsor_owner_user_id: "missing".into(),
        policy_digest: "unproven".into(),
        nyxagent_profile: "public".into(),
        model_operations: Vec::new(),
        limits: contract::PublicExecutionLimits {
            max_duration_secs: 1,
            max_model_calls: 1,
            max_tool_calls: 1,
            max_continuations: 0,
            max_input_tokens: 1,
            max_output_tokens: 1,
            max_input_bytes: 1,
            max_context_bytes: 1,
            max_output_bytes: 1,
        },
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(1),
    };
    let result = execution
        .execute_nyxagent_turn(
            &db,
            contract::NyxAgentExecutionContext::Public(public),
            contract::TurnInput {
                text: "work".into(),
            },
            sender,
        )
        .await;
    assert!(matches!(result, Err(AppError::Conflict(_))));
    assert_eq!(egress.0.load(std::sync::atomic::Ordering::SeqCst), 0);
}
