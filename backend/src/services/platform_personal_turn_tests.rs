use super::*;
use crate::models::{
    platform_manager_ingress::{COLLECTION_NAME as RECEIPTS, IngressPhase, PlatformManagerIngress},
    platform_personal_binding::{COLLECTION_NAME as BINDINGS, PlatformPersonalBinding},
};
use contract::PersonalTurnProducer;

struct RefusedEgress;
struct RecordingSettled(tokio::sync::mpsc::Sender<contract::ReplyRef>);
#[async_trait::async_trait]
impl runtime::OwnerTurnSettledHook for RecordingSettled {
    async fn settled(&self, _: AssistantConversation, reply: contract::ReplyRef) {
        self.0.send(reply).await.unwrap();
    }
}
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
        settled_hook: crate::handlers::assistant_team::owner_turn_settled_hook(state.clone()),
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
    let mut service = personal(state);
    let (hook_sender, mut hook_receiver) = tokio::sync::mpsc::channel(4);
    service.settled_hook = Arc::new(RecordingSettled(hook_sender));
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
    let hooked = tokio::time::timeout(Duration::from_secs(15), hook_receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hooked.reply_id, reply.reply_id);
    assert_eq!(hooked.turn, reply.turn);
    assert!(hook_receiver.try_recv().is_err());
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

async fn claimed_input(
    state: &AppState,
    admitted: &contract::AdmittedPersonalUpdate,
) -> engine::OwnerTurnClaim {
    let agent =
        super::super::assistant_team_service::ensure_nyxbot(&state.db, &admitted.authority.user_id)
            .await
            .unwrap();
    let ids = engine::OwnerTurnIds {
        conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let start = engine::TurnStart::from(&engine::parse_turn(br#"{"text":"work"}"#).unwrap());
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let claim = claim_personal_turn_in_session(
        &state.db,
        &mut session,
        &state.encryption_keys,
        &super::super::platform_personal_binding_service::IdentityProducer,
        &super::super::platform_manager_ingress_service::IngressTurnStore,
        admitted,
        &start,
        &ids,
        &agent,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    claim
}

async fn finish_claim(state: &AppState, claim: &engine::OwnerTurnClaim) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    engine::finish_turn(
        &state.db,
        &claim.conversation,
        &claim.credential.api_key_id,
        &id,
        &engine::TurnResult {
            text: "saved".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    id
}

fn event_start(id: &str) -> engine::TurnStart {
    let mut start = engine::TurnStart::from(
        &engine::parse_turn(
            &serde_json::to_vec(&serde_json::json!({"text":"event", "conversation_id":id}))
                .unwrap(),
        )
        .unwrap(),
    );
    start.origin = model::TurnOrigin::Event;
    start.text.clear();
    start
}

#[tokio::test]
async fn personal_event_captures_fresh_authority_and_unlinked_events_remain_web_only() {
    let (state, admitted) = fixture("personal_event_authority").await;
    let initial = claimed_input(&state, &admitted).await;
    finish_claim(&state, &initial).await;
    let owner = &initial.conversation.user_id;
    let id = &initial.conversation.id;
    let initial_origin = initial
        .conversation
        .active_turn
        .as_ref()
        .unwrap()
        .platform_origin
        .clone();
    for linked in [true, false] {
        if !linked {
            state
                .db
                .collection::<bson::Document>(BINDINGS)
                .update_one(
                    doc! {"_id":&admitted.authority.binding_id},
                    doc! {"$set":{"state":"revoked"}},
                )
                .await
                .unwrap();
        }
        engine::push_events(
            &state.db,
            owner,
            id,
            vec![super::super::assistant_team_service::event(
                "task_finished",
                "Independent work finished".into(),
                None,
            )],
        )
        .await
        .unwrap();
        let row = engine::begin_turn(&state.db, owner, event_start(id), &state.encryption_keys)
            .await
            .unwrap();
        let turn = row.active_turn.as_ref().unwrap();
        if linked {
            let origin = turn.platform_origin.as_ref().unwrap();
            assert!(
                matches!(&origin.source, model::PlatformOriginSource::ThreadEvent { admission_id } if admission_id == &turn.turn_id)
            );
            assert_ne!(turn.platform_origin, initial_origin);
            let binding = state
                .db
                .collection::<PlatformPersonalBinding>(BINDINGS)
                .find_one(doc! {"_id":&admitted.authority.binding_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(binding.active_turn.unwrap().turn_id, turn.turn_id);
            assert!(
                matches!(&turn.platform_reply, Some(model::PlatformReplyRouting::Captured { origin: reply_origin, turn_id, .. })
                if reply_origin.as_ref() == origin && turn_id == &turn.turn_id)
            );
        } else {
            assert!(turn.platform_origin.is_none());
            assert!(turn.platform_reply.is_none());
        }
        let credential = super::super::assistant_agent_credential_service::load_for_conversation(
            &state.db,
            &state.encryption_keys,
            owner,
            id,
        )
        .await
        .unwrap()
        .unwrap();
        let reply = finish_claim(
            &state,
            &engine::OwnerTurnClaim {
                conversation: row.clone(),
                credential,
            },
        )
        .await;
        let message = state
            .db
            .collection::<crate::models::assistant_message::AssistantMessage>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":reply})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.platform_origin, turn.platform_origin);
        assert_eq!(message.platform_reply, turn.platform_reply);
        let binding = state
            .db
            .collection::<PlatformPersonalBinding>(BINDINGS)
            .find_one(doc! {"_id":&admitted.authority.binding_id})
            .await
            .unwrap()
            .unwrap();
        assert!(binding.active_turn.is_none());
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn fresh_event_read_cannot_commit_after_concurrent_revocation() {
    let (state, admitted) = fixture("personal_event_unlink_race").await;
    let initial = claimed_input(&state, &admitted).await;
    finish_claim(&state, &initial).await;
    let owner = &initial.conversation.user_id;
    let id = &initial.conversation.id;
    engine::push_events(
        &state.db,
        owner,
        id,
        vec![super::super::assistant_team_service::event(
            "task_finished",
            "Independent work finished".into(),
            None,
        )],
    )
    .await
    .unwrap();
    let row = engine::get(&state.db, owner, id).await.unwrap();
    let ids = engine::OwnerTurnIds {
        conversation_id: id.clone(),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let agent = super::super::assistant_team_service::ensure_nyxbot(&state.db, owner)
        .await
        .unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        fresh_thread_event_origin_in_session(&state.db, &mut session, &row, &ids.turn_id)
            .await
            .unwrap()
            .is_some()
    );
    state
        .db
        .collection::<bson::Document>(BINDINGS)
        .update_one(
            doc! {"_id":&admitted.authority.binding_id},
            doc! {"$set":{"state":"revoked"}},
        )
        .await
        .unwrap();
    let result = engine::begin_owner_turn_in_session(
        &state.db,
        &mut session,
        &state.encryption_keys,
        owner,
        &event_start(id),
        &ids,
        &agent,
        None,
        None,
        None,
    )
    .await;
    assert!(result.is_err());
    let _ = session.abort_transaction().await;
    let saved = engine::get(&state.db, owner, id).await.unwrap();
    assert!(saved.active_turn.is_none());
    assert_eq!(saved.pending_events.len(), 1);
    state.db.drop().await.unwrap();
}

async fn admitted_command(
    state: &AppState,
    authority: &contract::PersonalBindingAuthority,
    text: String,
) -> (
    contract::AdmittedPersonalUpdate,
    contract::OriginalManagerCommand,
) {
    let profile = state
        .db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&authority.manager.profile_id})
        .await
        .unwrap()
        .unwrap();
    let update = contract::VerifiedManagerUpdate {
        profile_id: authority.manager.profile_id.clone(),
        manager_bot_id: authority.manager.manager_bot_id.clone(),
        telegram_bot_id: authority.manager.telegram_bot_id,
        observation_id: profile.observation_id,
        sender_id: authority.sender_id,
        chat_id: authority.chat_id,
        chat_kind: contract::ManagerChatKind::Private,
        update_id: 100,
        message_id: 100,
        command_eligibility: contract::ManagerCommandEligibility::OriginalPrivateMessage,
    };
    let contract::IngressAdmission::Claimed {
        audience: contract::ManagerAudience::Personal(admitted),
        ..
    } = super::super::platform_manager_dispatch_service::admit_manager_update(
        &state.db,
        &super::super::platform_personal_binding_service::IdentityProducer,
        &update,
    )
    .await
    .unwrap()
    else {
        panic!("original private command must be personally admitted");
    };
    let command =
        contract::OriginalManagerCommand::from_verified(update, contract::TurnInput { text })
            .unwrap();
    (admitted, command)
}

#[tokio::test]
async fn personal_card_decision_queues_one_durable_continuation_that_survives_unlink() {
    use super::super::assistant_acknowledgement_service as acks;
    for allowed in [true, false] {
        let (state, admitted) = fixture("personal_card_continuation").await;
        let initial = claimed_input(&state, &admitted).await;
        let row = &initial.conversation;
        let chat = acks::for_key(
            &state.db,
            &row.user_id,
            Some(&initial.credential.api_key_id),
        )
        .await
        .unwrap()
        .unwrap();
        let args = serde_json::json!({"target":"test"});
        let card = acks::request(
            &state.db,
            &chat,
            acks::Request {
                kind: "action",
                service: None,
                tool: Some("nyxid__test_action"),
                arguments: Some(&args),
                summary: "Perform the owner-requested test action",
                platform: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            card.requested_turn_id.as_ref(),
            row.active_turn.as_ref().map(|turn| &turn.turn_id)
        );
        finish_claim(&state, &initial).await;
        let phrase = format!(
            "{} {}",
            if allowed { "yes" } else { "no" },
            acks::confirm_code(&card.id)
        );
        let (decision_admission, command) =
            admitted_command(&state, &admitted.authority, phrase).await;
        let replay = contract::OriginalManagerCommand::from_verified(
            command.update().clone(),
            contract::TurnInput {
                text: command.text().to_owned(),
            },
        )
        .unwrap();
        let service = personal(state.clone());
        let result = service
            .decide_platform_reply(&state.db, &decision_admission, &row.id, command)
            .await
            .unwrap();
        assert!(
            matches!(result, contract::PlatformDecisionOutcome::Decided(ref decision)
            if decision.acknowledgement_id == card.id && decision.allowed == allowed)
        );
        let saved = engine::get(&state.db, &row.user_id, &row.id).await.unwrap();
        assert_eq!(saved.pending_events.len(), 1);
        let event = &saved.pending_events[0];
        assert_eq!(event.id, card.id);
        assert_eq!(event.kind, "owner_action_decided");
        assert!(event.bound);
        assert!(event.report_to.is_none());
        assert!(matches!(
            service
                .decide_platform_reply(&state.db, &decision_admission, &row.id, replay)
                .await
                .unwrap(),
            contract::PlatformDecisionOutcome::NoPendingDecision
        ));
        assert_eq!(
            engine::get(&state.db, &row.user_id, &row.id)
                .await
                .unwrap()
                .pending_events
                .len(),
            1
        );
        let receipt = state
            .db
            .collection::<PlatformManagerIngress>(RECEIPTS)
            .find_one(doc! {"_id":&decision_admission.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert!(receipt.phase == IngressPhase::Settled);
        assert!(receipt.execution.is_none());
        assert!(receipt.decision.is_some());
        state
            .db
            .collection::<bson::Document>(BINDINGS)
            .update_one(
                doc! {"_id":&admitted.authority.binding_id},
                doc! {"$set":{"state":"revoked"}},
            )
            .await
            .unwrap();
        let continued = engine::begin_turn(
            &state.db,
            &row.user_id,
            event_start(&row.id),
            &state.encryption_keys,
        )
        .await
        .unwrap();
        let turn = continued.active_turn.as_ref().unwrap();
        assert!(turn.platform_origin.is_none());
        assert!(turn.platform_reply.is_none());
        assert_eq!(turn.events.len(), 1);
        assert_eq!(turn.events[0].id, card.id);
        let saved_card = state
            .db
            .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                crate::models::assistant_acknowledgement::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&card.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved_card.status,
            if allowed { "allowed" } else { "denied" }
        );
        assert!(acks::decisions_note(&[saved_card]).contains(if allowed {
            "Retry an allowed action"
        } else {
            "Do not request denied items"
        }));
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn async_result_settlement_retains_original_destination_across_new_revision_and_unlink() {
    use crate::models::async_service_operation::{
        AsyncOperationContract, AsyncServiceOperation, DeliveryBinding,
    };
    let (state, admitted) = fixture("personal_async_destination").await;
    let initial = claimed_input(&state, &admitted).await;
    let row = &initial.conversation;
    let original = row.active_turn.as_ref().unwrap().platform_reply.clone();
    let proof = row.active_turn.as_ref().unwrap().platform_origin.clone();
    finish_claim(&state, &initial).await;
    state
        .db
        .collection::<bson::Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
        .update_one(
            doc! {"platform_config_id":&admitted.authority.manager.profile_id},
            doc! {"$set":{"personal_control_revision":2_i64}},
        )
        .await
        .map(|result| assert_eq!(result.matched_count, 1))
        .unwrap();
    for (linked, destination) in [
        (true, original.clone()),
        (true, None),
        (false, original.clone()),
    ] {
        if !linked {
            state
                .db
                .collection::<bson::Document>(BINDINGS)
                .update_one(
                    doc! {"_id":&admitted.authority.binding_id},
                    doc! {"$set":{"state":"revoked"}},
                )
                .await
                .unwrap();
        }
        let now = chrono::Utc::now();
        let watch = AsyncServiceOperation {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: row.user_id.clone(),
            conversation_id: row.id.clone(),
            agent_id: row.agent_id.clone().unwrap(),
            turn_id: row.active_turn.as_ref().unwrap().turn_id.clone(),
            api_key_id: row.credential_api_key_id.clone(),
            service_id: "test-service".into(),
            submit_endpoint_id: "submit".into(),
            contract: AsyncOperationContract {
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
            delivery: DeliveryBinding {
                platform_reply: destination.clone(),
                platform_origin: proof.clone(),
                origin: model::TurnOrigin::User,
                channel: None,
                reply_channel: None,
                group_id: None,
                group_request_id: None,
                voice_request_id: None,
                trigger_run_id: None,
                report_to: None,
                report_provenance: model::REPORT_PROVENANCE_V1,
            },
            operation_id: Some(uuid::Uuid::new_v4().to_string()),
            state: "queued".into(),
            reason: Some("done".into()),
            attempts: 0,
            lease_id: None,
            lease_until: None,
            created_at: now,
            deadline: now,
            poll_at: now,
            expires_at: now + chrono::Duration::hours(1),
            result_encrypted: None,
        };
        state
            .db
            .collection::<AsyncServiceOperation>(
                crate::models::async_service_operation::COLLECTION_NAME,
            )
            .insert_one(&watch)
            .await
            .unwrap();
        let mut event = super::super::assistant_team_service::event(
            super::super::async_service_operation::EVENT_KIND,
            "The previously authorized operation finished".into(),
            None,
        )
        .bound_to(None);
        event.id = watch.id.clone();
        engine::push_events(&state.db, &row.user_id, &row.id, vec![event])
            .await
            .unwrap();
        let continued = engine::begin_turn(
            &state.db,
            &row.user_id,
            event_start(&row.id),
            &state.encryption_keys,
        )
        .await
        .unwrap();
        let turn = continued.active_turn.as_ref().unwrap();
        if linked {
            assert_eq!(
                turn.platform_origin
                    .as_ref()
                    .unwrap()
                    .personal_control_revision,
                2
            );
        } else {
            assert!(turn.platform_origin.is_none());
        }
        let expected = destination.unwrap_or(model::PlatformReplyRouting::WebOnly);
        assert_eq!(turn.platform_reply.as_ref(), Some(&expected));
        let credential = super::super::assistant_agent_credential_service::load_for_conversation(
            &state.db,
            &state.encryption_keys,
            &row.user_id,
            &row.id,
        )
        .await
        .unwrap()
        .unwrap();
        let reply = finish_claim(
            &state,
            &engine::OwnerTurnClaim {
                conversation: continued.clone(),
                credential,
            },
        )
        .await;
        let saved = state
            .db
            .collection::<crate::models::assistant_message::AssistantMessage>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":reply})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.platform_reply, Some(expected));
        assert_eq!(saved.platform_origin, turn.platform_origin);
        let original_watch = state
            .db
            .collection::<AsyncServiceOperation>(
                crate::models::async_service_operation::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":watch.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            original_watch.delivery.platform_reply,
            watch.delivery.platform_reply
        );
        assert_eq!(original_watch.state, "delivered");
    }
    state.db.drop().await.unwrap();
}
