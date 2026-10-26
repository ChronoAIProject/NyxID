//! Real transaction regressions. Provider barriers prove ordering; deadlines
//! bound failed tests and do not assert production latency or benchmark results.
use super::*;
use crate::{
    crypto::{
        aes::EncryptionKeys,
        key_provider::{KeyProvider, WrappedKey},
        local_key_provider::LocalKeyProvider,
    },
    models::{
        assistant_agent_credential::AssistantAgentCredential,
        platform_personal_binding::{PersonalRevocationReason, PlatformPersonalBinding},
    },
    services::{
        assistant_acknowledgement_service as acks,
        assistant_agent_credential_service as credentials,
        platform_personal_binding_service as bindings,
    },
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::{Semaphore, mpsc};
use zeroize::Zeroizing;

#[derive(Debug)]
struct DelayedKeyProvider {
    inner: LocalKeyProvider,
    delay: Duration,
    paused: AtomicBool,
    arrived: mpsc::UnboundedSender<()>,
    release: Semaphore,
    wraps: AtomicUsize,
    unwraps: AtomicUsize,
}

impl DelayedKeyProvider {
    async fn wait(&self) -> AppResult<()> {
        if self.paused.load(Ordering::SeqCst) {
            let _ = self.arrived.send(());
            tokio::time::timeout(Duration::from_secs(20), self.release.acquire())
                .await
                .map_err(|_| AppError::Internal("test provider barrier expired".into()))?
                .map_err(|_| AppError::Internal("test provider barrier closed".into()))?
                .forget();
        }
        tokio::time::sleep(self.delay).await;
        Ok(())
    }

    fn resume(&self, count: usize) {
        self.paused.store(false, Ordering::SeqCst);
        self.release.add_permits(count);
    }
}

#[async_trait::async_trait]
impl KeyProvider for DelayedKeyProvider {
    async fn wrap_dek(&self, bytes: &[u8]) -> AppResult<WrappedKey> {
        self.wraps.fetch_add(1, Ordering::SeqCst);
        self.wait().await?;
        self.inner.wrap_dek(bytes).await
    }

    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
        self.unwraps.fetch_add(1, Ordering::SeqCst);
        self.wait().await?;
        self.inner.unwrap_dek(wrapped).await
    }

    fn current_key_id(&self) -> u8 {
        self.inner.current_key_id()
    }
    fn has_key_id(&self, id: u8) -> bool {
        self.inner.has_key_id(id)
    }
    fn has_previous_key(&self) -> bool {
        self.inner.has_previous_key()
    }
}

fn delay_keys(
    state: &mut AppState,
    delay: Duration,
) -> (Arc<DelayedKeyProvider>, mpsc::UnboundedReceiver<()>) {
    let (arrived, receiver) = mpsc::unbounded_channel();
    let provider = Arc::new(DelayedKeyProvider {
        inner: LocalKeyProvider::from_config(&state.config),
        delay,
        paused: AtomicBool::new(true),
        arrived,
        release: Semaphore::new(0),
        wraps: AtomicUsize::new(0),
        unwraps: AtomicUsize::new(0),
    });
    state.encryption_keys = Arc::new(EncryptionKeys::with_provider(provider.clone()));
    (provider, receiver)
}

async fn arrived(receiver: &mut mpsc::UnboundedReceiver<()>, count: usize) {
    tokio::time::timeout(Duration::from_secs(15), async {
        for _ in 0..count {
            receiver.recv().await.expect("provider called");
        }
    })
    .await
    .expect("all owners reach KMS without holding shared manager writes");
}

async fn record_next(
    state: &AppState,
    previous: &contract::AdmittedPersonalUpdate,
    sequence: i64,
) -> contract::AdmittedPersonalUpdate {
    let admitted = contract::AdmittedPersonalUpdate {
        ingress: contract::IngressClaim {
            receipt_id: uuid::Uuid::new_v4().to_string(),
            claim_id: uuid::Uuid::new_v4().to_string(),
        },
        authority: previous.authority.clone(),
    };
    let profile = state
        .db
        .collection::<crate::models::platform_bot_config::PlatformBotConfig>(
            crate::models::platform_bot_config::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&admitted.authority.manager.profile_id})
        .await
        .unwrap()
        .unwrap();
    let update = contract::VerifiedManagerUpdate {
        profile_id: profile.id,
        manager_bot_id: admitted.authority.manager.manager_bot_id.clone(),
        entry: admitted.authority.manager.entry.clone(),
        observation_id: profile.observation_id,
        sender: admitted.authority.sender.clone(),
        chat_kind: contract::ManagerChatKind::Private,
        event: crate::models::platform_entry_projection::StoredEventIdentity::TelegramV1 {
            update_id: sequence,
            message_id: sequence,
        },
        command_eligibility: contract::ManagerCommandEligibility::OriginalPrivateMessage,
    };
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    super::super::platform_manager_ingress_service::record_admission_in_session(
        &state.db,
        &mut session,
        &update,
        &admitted.authority.manager,
        &admitted.ingress,
        &contract::ManagerAudience::Personal(admitted.clone()),
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    admitted
}

async fn admit(
    state: AppState,
    admitted: contract::AdmittedPersonalUpdate,
) -> AppResult<engine::OwnerTurnClaim> {
    let agent =
        super::super::assistant_team_service::ensure_nyxbot(&state.db, &admitted.authority.user_id)
            .await?;
    let start = engine::TurnStart::from(&engine::parse_turn(br#"{"text":"work"}"#)?);
    let ids = engine::OwnerTurnIds {
        conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
        turn_id: uuid::Uuid::new_v4().to_string(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut session = state.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (state, admitted, start, ids, agent),
            |session, (state, admitted, start, ids, agent)| {
                Box::pin(async move {
                    let result = claim_personal_turn_in_session(
                        &state.db,
                        session,
                        &state.encryption_keys,
                        &bindings::IdentityProducer,
                        &super::super::platform_manager_ingress_service::IngressTurnStore,
                        admitted,
                        start,
                        ids,
                        agent,
                    )
                    .await;
                    transactions::transaction_result(result)
                })
            },
        )
        .await
        .map_err(transactions::map_transaction_error)
}

async fn settle(state: &AppState, claim: &engine::OwnerTurnClaim) {
    engine::finish_turn(
        &state.db,
        &claim.conversation,
        &claim.credential.api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "saved".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
}

async fn gate(state: &AppState, claim: &engine::OwnerTurnClaim) -> AppResult<()> {
    let chat = acks::for_key(
        &state.db,
        &claim.conversation.user_id,
        Some(&claim.credential.api_key_id),
    )
    .await?
    .ok_or(AppError::AssistantTurnRequired)?;
    acks::enforce_turn_gate(
        &state.db,
        &chat.turn_fence(),
        &chat.user_id,
        Some(&chat.api_key_id),
        None,
        None,
        None,
        "test/personal-effect",
    )
    .await
}

async fn machine_node(state: &AppState, owner: &str) -> crate::models::node::Node {
    let (_, token, _) = super::super::node_service::create_registration_token(
        &state.db,
        owner,
        "kms-regression",
        100,
        300,
    )
    .await
    .unwrap();
    let (mut node, _, _) =
        super::super::node_service::register_node(&state.db, &state.encryption_keys, &token, None)
            .await
            .unwrap();
    node.status = crate::models::node::NodeStatus::Online;
    node.machine = Some(nyxid_machine::MachineProfile {
        runtime_id: uuid::Uuid::new_v4().to_string(),
        shell: true,
        ..Default::default()
    });
    state
        .db
        .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .replace_one(doc! {"_id":&node.id}, &node)
        .await
        .unwrap();
    node
}

fn lifecycle(state: &AppState) -> bindings::LifecycleProducer {
    let identity = Arc::new(bindings::IdentityProducer);
    let egress = super::super::assistant_nyxagent_egress::producer(
        state.clone(),
        identity.clone(),
        crate::handlers::agent_skills::preload_context(state.clone()),
    );
    let runtime = super::super::platform_public_credential_service::producer(state.clone());
    let execution =
        super::super::assistant_nyxagent_runtime::producer(state.clone(), egress.clone(), runtime);
    let ingress = Arc::new(super::super::platform_manager_ingress_service::IngressTurnStore);
    bindings::LifecycleProducer {
        personal: producer(
            state.clone(),
            identity,
            ingress.clone(),
            egress,
            crate::handlers::assistant_team::owner_turn_settled_hook(state.clone()),
        ),
        public: super::super::platform_public_agent_service::producer(
            state.clone(),
            execution,
            ingress,
        ),
    }
}

async fn revoke(state: &AppState, binding_id: &str) {
    let binding = state
        .db
        .collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":binding_id})
        .await
        .unwrap()
        .unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    bindings::revoke_binding_in_session(
        &state.db,
        &mut session,
        &binding,
        PersonalRevocationReason::Unlinked,
        &lifecycle(state),
        None,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), session.commit_transaction())
        .await
        .unwrap()
        .unwrap();
}

async fn emergency_stop(state: &AppState, profile_id: &str) {
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    super::super::platform_bot_runtime_control_service::emergency_stop_in_session(
        &state.db,
        &mut session,
        profile_id,
        false,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), session.commit_transaction())
        .await
        .unwrap()
        .unwrap();
}

async fn eight_owners(delay: Duration) {
    let (mut state, first) = super::tests::fixture("personal_kms_eight_owners").await;
    let mut admitted = vec![first.clone()];
    for index in 1..8 {
        let owner = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let mut binding = state
            .db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&first.authority.binding_id})
            .await
            .unwrap()
            .unwrap();
        binding.id = uuid::Uuid::new_v4().to_string();
        binding.user_id = owner;
        if let crate::models::platform_entry_projection::StoredSenderIdentity::TelegramV1 {
            telegram_sender_id,
            telegram_chat_id,
        } = &mut binding.sender
        {
            *telegram_sender_id += index;
            *telegram_chat_id = *telegram_sender_id;
        } else {
            panic!("legacy fixture");
        }
        state
            .db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .insert_one(&binding)
            .await
            .unwrap();
        let next = contract::AdmittedPersonalUpdate {
            ingress: first.ingress.clone(),
            authority: bindings::authority(&binding, 1),
        };
        admitted.push(record_next(&state, &next, index + 1).await);
    }
    let mut nodes = Vec::new();
    let mut next = Vec::new();
    for (index, admitted) in admitted.into_iter().enumerate() {
        let warm = admit(state.clone(), admitted.clone()).await.unwrap();
        settle(&state, &warm).await;
        nodes.push(machine_node(&state, &admitted.authority.user_id).await);
        next.push(record_next(&state, &admitted, 20 + index as i64).await);
    }
    let (provider, mut calls) = delay_keys(&mut state, delay);
    let (effects_done, mut effects) = mpsc::unbounded_channel();
    let mut workers = Vec::new();
    for (admitted, node) in next.into_iter().zip(nodes) {
        let state = state.clone();
        let done = effects_done.clone();
        workers.push(tokio::spawn(async move {
            let claim = admit(state.clone(), admitted).await?;
            for _ in 0..10 {
                gate(&state, &claim).await?;
            }
            let chat = acks::for_key(
                &state.db,
                &claim.conversation.user_id,
                Some(&claim.credential.api_key_id),
            )
            .await?
            .unwrap();
            for _ in 0..2 {
                super::super::machine_service::issue_job(&state.db, &chat, &node, 30, Vec::new())
                    .await?;
            }
            let _ = done.send(());
            Ok::<_, AppError>((claim, node))
        }));
    }
    arrived(&mut calls, 8).await;
    assert_eq!(provider.unwraps.load(Ordering::SeqCst), 8);
    // Let one owner admit and perform real effect/job fences while seven KMS
    // calls stay paused. Early shared writes would prevent this phase.
    provider.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(15), effects.recv())
        .await
        .unwrap()
        .unwrap();
    provider.resume(7);
    let mut completed = Vec::new();
    for worker in workers {
        completed.push(worker.await.unwrap().unwrap());
    }
    let victim = completed[3]
        .0
        .conversation
        .active_turn
        .as_ref()
        .unwrap()
        .platform_origin
        .as_ref()
        .unwrap();
    revoke(&state, &victim.thread.binding_id).await;
    for (index, (claim, _)) in completed.iter().enumerate() {
        assert_eq!(gate(&state, claim).await.is_ok(), index != 3);
    }
    emergency_stop(&state, &first.authority.manager.profile_id).await;
    // Every effect admitted after the committed control changes is refused;
    // rejected issues add no job records and existing records remain durable.
    for (claim, node) in &completed {
        assert!(gate(&state, claim).await.is_err());
        let chat = acks::for_key(
            &state.db,
            &claim.conversation.user_id,
            Some(&claim.credential.api_key_id),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            super::super::machine_service::issue_job(&state.db, &chat, node, 30, Vec::new())
                .await
                .is_err()
        );
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(JOBS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        16
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        8
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn eight_owner_admission_and_effects_commit_with_local_and_200ms_key_provider() {
    tokio::time::timeout(Duration::from_secs(90), async {
        eight_owners(Duration::ZERO).await;
        eight_owners(Duration::from_millis(200)).await;
    })
    .await
    .expect("bounded concurrent admission/effect regression");
}

#[tokio::test]
async fn revocation_during_slow_unwrap_aborts_admission_without_changing_history_or_key() {
    let (mut state, admitted) = super::tests::fixture("personal_kms_revoke_admission").await;
    let warm = admit(state.clone(), admitted.clone()).await.unwrap();
    settle(&state, &warm).await;
    let before = engine::get(&state.db, &warm.conversation.user_id, &warm.conversation.id)
        .await
        .unwrap();
    let next = record_next(&state, &admitted, 2).await;
    let (provider, mut calls) = delay_keys(&mut state, Duration::from_millis(200));
    let pending = tokio::spawn(admit(state.clone(), next.clone()));
    arrived(&mut calls, 1).await;
    // Revocation must commit while the admission still waits in unwrap.
    revoke(&state, &admitted.authority.binding_id).await;
    provider.resume(1);
    assert!(
        tokio::time::timeout(Duration::from_secs(15), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let after = engine::get(&state.db, &before.user_id, &before.id)
        .await
        .unwrap();
    assert!(after.active_turn.is_none());
    assert_eq!(after.message_count, before.message_count);
    assert_eq!(after.credential_api_key_id, before.credential_api_key_id);
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(CREDENTIALS)
            .count_documents(doc! {"conversation_id":&before.id})
            .await
            .unwrap(),
        1
    );
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        !bindings::validate_personal_origin_in_session(&state.db, &mut session, &next.authority)
            .await
            .unwrap()
    );
    session.abort_transaction().await.unwrap();
    let receipt = state
        .db
        .collection::<crate::models::platform_manager_ingress::PlatformManagerIngress>(
            crate::models::platform_manager_ingress::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&next.ingress.receipt_id})
        .await
        .unwrap()
        .unwrap();
    assert!(receipt.phase == crate::models::platform_manager_ingress::IngressPhase::Admitted);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn emergency_stop_during_slow_wrap_rolls_back_predecessor_delete_and_successor_adoption() {
    let (mut state, admitted) = super::tests::fixture("personal_kms_stop_replacement").await;
    let claim = admit(state.clone(), admitted.clone()).await.unwrap();
    let before = state
        .db
        .collection::<AssistantAgentCredential>(CREDENTIALS)
        .find_one(doc! {"conversation_id":&claim.conversation.id})
        .await
        .unwrap()
        .unwrap();
    let (provider, mut calls) = delay_keys(&mut state, Duration::from_millis(200));
    let pending_state = state.clone();
    let row = claim.conversation.clone();
    let old = claim.credential.api_key_id.clone();
    let pending = tokio::spawn(async move {
        credentials::replace_for_turn(
            &pending_state.db,
            &pending_state.encryption_keys,
            &row,
            &old,
        )
        .await
    });
    arrived(&mut calls, 1).await;
    assert_eq!(provider.wraps.load(Ordering::SeqCst), 1);
    emergency_stop(&state, &admitted.authority.manager.profile_id).await;
    provider.resume(1);
    assert!(
        tokio::time::timeout(Duration::from_secs(15), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let after = state
        .db
        .collection::<AssistantAgentCredential>(CREDENTIALS)
        .find_one(doc! {"conversation_id":&claim.conversation.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.api_key_id, before.api_key_id);
    assert!(after.key_ciphertext == before.key_ciphertext);
    assert_eq!(
        engine::get(
            &state.db,
            &claim.conversation.user_id,
            &claim.conversation.id
        )
        .await
        .unwrap()
        .credential_api_key_id,
        before.api_key_id
    );
    assert_eq!(state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).count_documents(doc! {"user_id":&claim.conversation.user_id,"platform":credentials::ASSISTANT_PLATFORM}).await.unwrap(),1);
    assert!(gate(&state, &claim).await.is_err());
    state.db.drop().await.unwrap();
}
