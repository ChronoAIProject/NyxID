//! Personal platform authority joined to the ordinary assistant engine.
//! No HTTP authentication, transport send, or public credential convergence.
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
    options::ReturnDocument,
};

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent_credential::COLLECTION_NAME as CREDENTIALS,
        assistant_conversation::{
            self as model, AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
        },
        machine_job::{COLLECTION_NAME as JOBS, MachineJob},
    },
    services::{assistant_nyxagent as engine, platform_manager_contract as contract},
};

pub(crate) fn model_origin(origin: &contract::PlatformTurnOrigin) -> model::PlatformTurnOrigin {
    let authority = &origin.authority;
    model::PlatformTurnOrigin {
        thread: model::PlatformThread {
            platform: super::platform_entry_scope::platform(&authority.manager.entry).into(),
            profile_id: authority.manager.profile_id.clone(),
            manager_bot_id: authority.manager.manager_bot_id.clone(),
            entry: authority.manager.entry.clone(),
            binding_id: authority.binding_id.clone(),
            binding_generation: authority.binding_generation,
            user_id: authority.user_id.clone(),
            nyxbot_agent_id: origin.nyxbot_agent_id.clone(),
        },
        manager_authority_epoch: authority.manager.manager_authority_epoch,
        personal_control_revision: authority.personal_control_revision,
        sender: authority.sender.clone(),
        source: match &origin.source {
            contract::PlatformOriginSource::Ingress(claim) => {
                model::PlatformOriginSource::Ingress {
                    receipt_id: claim.receipt_id.clone(),
                    claim_id: claim.claim_id.clone(),
                }
            }
            contract::PlatformOriginSource::ThreadEvent { admission_id } => {
                model::PlatformOriginSource::ThreadEvent {
                    admission_id: admission_id.clone(),
                }
            }
        },
    }
}

pub(crate) fn contract_origin(
    origin: &model::PlatformTurnOrigin,
) -> AppResult<contract::PlatformTurnOrigin> {
    if origin.thread.platform != super::platform_entry_scope::platform(&origin.thread.entry)
        || super::platform_entry_scope::validate(
            &origin.thread.entry,
            &origin.thread.manager_bot_id,
            Some(&origin.sender),
        )
        .is_err()
    {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(contract::PlatformTurnOrigin {
        authority: contract::PersonalBindingAuthority {
            manager: contract::ManagerAuthority {
                profile_id: origin.thread.profile_id.clone(),
                manager_bot_id: origin.thread.manager_bot_id.clone(),
                entry: origin.thread.entry.clone(),
                manager_authority_epoch: origin.manager_authority_epoch,
            },
            sender: origin.sender.clone(),
            binding_id: origin.thread.binding_id.clone(),
            binding_generation: origin.thread.binding_generation,
            personal_control_revision: origin.personal_control_revision,
            user_id: origin.thread.user_id.clone(),
        },
        nyxbot_agent_id: origin.thread.nyxbot_agent_id.clone(),
        source: match &origin.source {
            model::PlatformOriginSource::Ingress {
                receipt_id,
                claim_id,
            } => contract::PlatformOriginSource::Ingress(contract::IngressClaim {
                receipt_id: receipt_id.clone(),
                claim_id: claim_id.clone(),
            }),
            model::PlatformOriginSource::ThreadEvent { admission_id } => {
                contract::PlatformOriginSource::ThreadEvent {
                    admission_id: admission_id.clone(),
                }
            }
        },
    })
}

pub(crate) async fn fresh_thread_event_origin_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    turn_id: &str,
) -> AppResult<Option<model::PlatformTurnOrigin>> {
    let Some(thread) = row.platform_thread.as_ref() else {
        return Ok(None);
    };
    if thread.user_id != row.user_id
        || row.agent_id.as_ref() != Some(&thread.nyxbot_agent_id)
        || row.is_subagent()
        || row.channel.is_some()
        || row.group_id.is_some()
    {
        return Err(AppError::AssistantTurnRequired);
    }
    let resolved = super::platform_personal_binding_service::resolve_thread_in_session(
        db,
        session,
        &row.user_id,
        &thread.binding_id,
        thread.binding_generation,
        &row.id,
        false,
    )
    .await?;
    let contract::ThreadAuthorityResolution::Personal(authority) = resolved else {
        return Ok(None);
    };
    let origin = model_origin(&contract::PlatformTurnOrigin {
        authority: *authority,
        nyxbot_agent_id: thread.nyxbot_agent_id.clone(),
        source: contract::PlatformOriginSource::ThreadEvent {
            admission_id: turn_id.to_owned(),
        },
    });
    if origin.thread != *thread {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(Some(origin))
}

/// A write on the exact conversation/key conflicts with Stop, rotation and
/// deletion. Live consent is then fenced in the very same transaction.
pub(crate) async fn fence_execution_in_session(
    db: &Database,
    session: &mut ClientSession,
    identity: &dyn contract::ManagerIdentityProducer,
    user_id: &str,
    key_id: &str,
    turn: &contract::TurnRef,
    expected: &contract::PlatformTurnOrigin,
) -> AppResult<contract::FenceOutcome> {
    use contract::{FenceOutcome, UnavailableReason};
    let refused = || FenceOutcome::Unavailable(UnavailableReason::OriginRevoked);
    if expected.authority.user_id != user_id {
        return Ok(refused());
    }
    let origin = model_origin(expected);
    let row = db.collection::<AssistantConversation>(CONVERSATIONS).find_one_and_update(
        doc! {"_id":&turn.conversation_id,"user_id":user_id,"credential_api_key_id":key_id,
            "agent_id":&expected.nyxbot_agent_id,"role":"orchestrator","guest_turn":{"$ne":true},
            "platform_thread":bson::to_bson(&origin.thread).map_err(|_| AppError::AssistantTurnRequired)?,
            "active_turn.turn_id":&turn.turn_id,"active_turn.stop_requested":false,
            "active_turn.platform_origin":bson::to_bson(&origin).map_err(|_| AppError::AssistantTurnRequired)?},
        doc! {"$inc":{"authority_fence":1_i64}},
    ).session(&mut *session).await?;
    let Some(row) = row else {
        return Ok(refused());
    };
    if engine::live_turn(&row, chrono::Utc::now()).is_none() {
        return Ok(refused());
    }
    let credential = db
        .collection::<bson::Document>(CREDENTIALS)
        .update_one(
            doc! {"user_id":user_id,"conversation_id":&turn.conversation_id,"api_key_id":key_id},
            doc! {"$inc":{"authority_fence":1_i64}},
        )
        .session(&mut *session)
        .await?;
    if credential.matched_count != 1 {
        return Ok(refused());
    }
    identity
        .fence_personal_origin(db, session, &expected.authority)
        .await
}

/// Inline continuation/rebinding cannot renew a revoked platform execution.
/// The current credential may be a successor already adopted by recovery.
pub(crate) async fn update_platform_turn(
    db: &Database,
    row: &AssistantConversation,
    update: bson::Document,
) -> AppResult<()> {
    mutate_platform_turn(db, row, update, None).await
}

pub(crate) async fn claim_platform_worker(
    db: &Database,
    row: &AssistantConversation,
    key_id: &str,
) -> AppResult<()> {
    mutate_platform_turn(
        db,
        row,
        doc! {"$set":{
            "active_turn.execution_started":true,
            "active_turn.worker_instance":bson::to_bson(super::assistant_engine_worker::instance())
                .map_err(|_|AppError::Internal("Worker identity encoding failed".into()))?,
        }},
        Some(key_id.to_owned()),
    )
    .await
}

async fn mutate_platform_turn(
    db: &Database,
    row: &AssistantConversation,
    update: bson::Document,
    worker_key_id: Option<String>,
) -> AppResult<()> {
    let origin = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.platform_origin.as_ref())
        .ok_or(AppError::AssistantTurnRequired)?;
    let expected = contract_origin(origin)?;
    let turn = contract::TurnRef {
        conversation_id: row.id.clone(),
        turn_id: row
            .active_turn
            .as_ref()
            .ok_or(AppError::AssistantTurnRequired)?
            .turn_id
            .clone(),
    };
    let mut session = db.client().start_session().await?;
    let context = (
        db.clone(),
        row.user_id.clone(),
        turn,
        expected,
        update,
        worker_key_id,
    );
    session
        .start_transaction()
        .and_run(
            context,
            |session, (db, user, turn, origin, update, worker_key_id)| {
                Box::pin(async move {
                    let result: AppResult<()> = async {
                        let row = db
                            .collection::<AssistantConversation>(CONVERSATIONS)
                            .find_one(doc! {
                                "_id": &turn.conversation_id, "user_id": &*user,
                                "active_turn.turn_id": &turn.turn_id,
                            })
                            .session(&mut *session)
                            .await?
                            .ok_or(AppError::AssistantTurnRequired)?;
                        if worker_key_id
                            .as_ref()
                            .is_some_and(|key| key != &row.credential_api_key_id)
                        {
                            return Err(AppError::AssistantTurnRequired);
                        }
                        if fence_execution_in_session(
                            db,
                            session,
                            &super::platform_personal_binding_service::IdentityProducer,
                            user,
                            &row.credential_api_key_id,
                            turn,
                            origin,
                        )
                        .await?
                            != contract::FenceOutcome::Authorized
                        {
                            return Err(AppError::AssistantTurnRequired);
                        }
                        let mut filter = doc! {
                            "_id": &turn.conversation_id, "user_id": &*user,
                            "active_turn.turn_id": &turn.turn_id,
                            "active_turn.stop_requested": false,
                            "active_turn.platform_origin": bson::to_bson(&model_origin(origin))
                                .map_err(|_| AppError::AssistantTurnRequired)?,
                        };
                        if let Some(key) = worker_key_id.as_ref() {
                            filter.insert("credential_api_key_id", key);
                            filter.insert("active_turn.execution_started", doc! {"$ne": true});
                        }
                        let changed = db
                            .collection::<AssistantConversation>(CONVERSATIONS)
                            .update_one(filter, update.clone())
                            .session(&mut *session)
                            .await?;
                        if changed.matched_count != 1 {
                            return Err(AppError::AssistantTurnRequired);
                        }
                        Ok(())
                    }
                    .await;
                    super::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

/// Called inside revocation's transaction. It deliberately does not require
/// still-live consent: only the exact previously admitted origin/turn may stop.
/// Committed independent watches and jobs from older turns remain untouched.
pub(crate) async fn capture_exact_stop_in_session(
    db: &Database,
    session: &mut ClientSession,
    origin: &contract::PlatformTurnOrigin,
    turn: &contract::TurnRef,
) -> AppResult<contract::ExactStopOutcome> {
    let expected = model_origin(origin);
    let now = chrono::Utc::now();
    let row = db.collection::<AssistantConversation>(CONVERSATIONS).find_one_and_update(
        doc! {"_id":&turn.conversation_id,"user_id":&origin.authority.user_id,
            "platform_thread":bson::to_bson(&expected.thread).map_err(|_|AppError::AssistantTurnRequired)?,
            "active_turn.turn_id":&turn.turn_id,
            "active_turn.platform_origin":bson::to_bson(&expected).map_err(|_|AppError::AssistantTurnRequired)?},
        doc! {"$set":{"active_turn.stop_requested":true},"$inc":{"authority_fence":1_i64}},
    ).return_document(ReturnDocument::After).session(&mut *session).await?;
    let Some(row) = row else {
        return Ok(contract::ExactStopOutcome::NoMatchingActiveTurn);
    };
    let machine_scopes = row
        .active_turn
        .as_ref()
        .into_iter()
        .flat_map(|turn| turn.machine_scopes.iter())
        .map(|scope| contract::ExactMachineScope {
            node_id: scope.node_id.clone(),
            runtime_id: scope.runtime_id.clone(),
        })
        .collect();
    let mut cursor = db
        .collection::<MachineJob>(JOBS)
        .find(doc! {
            "user_id":&origin.authority.user_id,"conversation_id":&turn.conversation_id,
            "issuing_turn_id":&turn.turn_id,"state":"running",
        })
        .session(&mut *session)
        .await?;
    let jobs: Vec<MachineJob> = cursor.stream(&mut *session).try_collect().await?;
    Ok(contract::ExactStopOutcome::Captured(
        contract::CapturedPlatformStop {
            origin: origin.clone(),
            turn: turn.clone(),
            stop_requested_at: now,
            machine_scopes,
            machine_jobs: jobs
                .into_iter()
                .map(|job| contract::ExactMachineJob {
                    node_id: job.node_id,
                    runtime_id: job.runtime_id,
                    job_id: job.id,
                })
                .collect(),
        },
    ))
}

/// Personal first use resolves the real NyxBot before this transaction. No
/// optional binding hint can stand in for an agent's execution authority.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn claim_personal_turn_in_session(
    db: &Database,
    session: &mut ClientSession,
    keys: &crate::crypto::aes::EncryptionKeys,
    identity: &dyn contract::ManagerIdentityProducer,
    ingress: &dyn contract::ManagerIngressTurnStore,
    admitted: &contract::AdmittedPersonalUpdate,
    start: &engine::TurnStart,
    candidate_ids: &engine::OwnerTurnIds,
    agent: &crate::models::assistant_agent::AssistantAgent,
) -> AppResult<engine::OwnerTurnClaim> {
    let authority = &admitted.authority;
    if agent.user_id != authority.user_id || !agent.is_nyxbot() || agent.destroyed_at.is_some() {
        return Err(AppError::AssistantTurnRequired);
    }
    // claim_binding_thread performs the read validation before key work;
    // claim_binding_turn takes the shared write fence after that work.
    let thread = identity
        .claim_binding_thread(
            db,
            session,
            authority,
            &agent.id,
            &candidate_ids.conversation_id,
        )
        .await?;
    if thread.nyxbot_agent_id != agent.id {
        return Err(AppError::AssistantTurnRequired);
    }
    let ids = engine::OwnerTurnIds {
        conversation_id: thread.conversation_id.clone(),
        turn_id: candidate_ids.turn_id.clone(),
        message_id: candidate_ids.message_id.clone(),
    };
    let mut start = start.clone();
    start.conversation_id = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":&ids.conversation_id,"user_id":&authority.user_id})
        .session(&mut *session)
        .await?
        .map(|row| row.id);
    start.agent_id = Some(agent.id.clone());
    if let Some(selected) =
        super::platform_manager_management::selected_context_in_session(db, session, authority)
            .await?
    {
        start.note = Some(match start.note.take() {
            Some(note) => format!("{selected}\n\n{note}"),
            None => selected,
        });
    }
    let origin = contract::PlatformTurnOrigin {
        authority: authority.clone(),
        nyxbot_agent_id: agent.id.clone(),
        source: contract::PlatformOriginSource::Ingress(admitted.ingress.clone()),
    };
    let turn = contract::TurnRef {
        conversation_id: ids.conversation_id.clone(),
        turn_id: ids.turn_id.clone(),
    };
    let claim = Box::pin(engine::begin_owner_turn_in_session(
        db,
        session,
        keys,
        &authority.user_id,
        &start,
        &ids,
        agent,
        None,
        Some(&model_origin(&origin)),
        None,
    ))
    .await?
    .into_started()?;
    identity
        .claim_binding_turn(db, session, &origin, &turn)
        .await?;
    ingress
        .claim_personal_execution(db, session, admitted, &turn)
        .await?;
    Ok(claim)
}

/// Reply insertion, ingress settlement and pointer clearing share the caller's
/// transaction. Revoked consent must still let this exact old turn settle.
pub(crate) async fn settle_saved_reply_in_session(
    db: &Database,
    session: &mut ClientSession,
    message: &crate::models::assistant_message::AssistantMessage,
) -> AppResult<()> {
    use contract::{ManagerIdentityProducer, ManagerIngressTurnStore};
    let Some(origin) = message.platform_origin.as_ref() else {
        return Ok(());
    };
    if origin.thread.user_id != message.user_id {
        return Err(AppError::AssistantTurnRequired);
    }
    let turn = contract::TurnRef {
        conversation_id: message.conversation_id.clone(),
        turn_id: message.turn_id.clone(),
    };
    super::platform_personal_binding_service::IdentityProducer
        .clear_binding_turn(
            db,
            session,
            &message.user_id,
            &origin.thread.binding_id,
            origin.thread.binding_generation,
            &turn,
        )
        .await?;
    if let model::PlatformOriginSource::Ingress {
        receipt_id,
        claim_id,
    } = &origin.source
    {
        super::platform_manager_ingress_service::IngressTurnStore
            .settle_personal_execution(
                db,
                session,
                &contract::IngressClaim {
                    receipt_id: receipt_id.clone(),
                    claim_id: claim_id.clone(),
                },
                &contract::ReplyRef {
                    turn,
                    reply_id: message.id.clone(),
                },
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn clear_thread_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<bool> {
    use contract::ManagerIdentityProducer;
    let Some(thread) = row.platform_thread.as_ref() else {
        return Ok(false);
    };
    if let Some(turn) = row.active_turn.as_ref()
        && let Some(origin) = turn.platform_origin.as_ref()
        && matches!(origin.source, model::PlatformOriginSource::Ingress { .. })
    {
        super::platform_manager_ingress_service::interrupt_personal_execution_in_session(
            db,
            session,
            &contract_origin(origin)?,
            &contract::TurnRef {
                conversation_id: row.id.clone(),
                turn_id: turn.turn_id.clone(),
            },
        )
        .await?;
    }
    super::platform_personal_binding_service::IdentityProducer
        .clear_binding_thread_reference(
            db,
            session,
            &row.user_id,
            &thread.binding_id,
            thread.binding_generation,
            &row.id,
        )
        .await
}

use super::{api_key_mutation_service as transactions, assistant_nyxagent_runtime as runtime};
use crate::AppState;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tracing::Instrument;

pub fn producer(
    state: AppState,
    identity: Arc<dyn contract::ManagerIdentityProducer>,
    ingress: Arc<dyn contract::ManagerIngressTurnStore>,
    egress: Arc<dyn contract::NyxAgentEgress>,
    settled_hook: Arc<dyn runtime::OwnerTurnSettledHook>,
) -> Arc<dyn contract::PersonalTurnProducer> {
    Arc::new(PersonalTurns {
        state,
        identity,
        ingress,
        egress,
        settled_hook,
    })
}

struct PersonalTurns {
    state: AppState,
    identity: Arc<dyn contract::ManagerIdentityProducer>,
    ingress: Arc<dyn contract::ManagerIngressTurnStore>,
    egress: Arc<dyn contract::NyxAgentEgress>,
    settled_hook: Arc<dyn runtime::OwnerTurnSettledHook>,
}

#[async_trait::async_trait]
impl contract::PersonalTurnProducer for PersonalTurns {
    async fn start_personal_turn(
        &self,
        db: &Database,
        admitted: &contract::AdmittedPersonalUpdate,
        input: contract::TurnInput,
        events: tokio::sync::mpsc::Sender<contract::ExecutionEvent>,
    ) -> AppResult<contract::PersonalTurnStart> {
        engine::require_enabled(db, &admitted.authority.user_id).await?;
        let request = engine::parse_turn(
            &serde_json::to_vec(&serde_json::json!({"text":input.text}))
                .map_err(|_| AppError::BadRequest("Invalid assistant input".into()))?,
        )?;
        let Some(permit) = self
            .state
            .direct_chat_limiter
            .try_acquire_pool("assistant_channel", &admitted.authority.user_id, 2)
            .await?
        else {
            return Ok(contract::PersonalTurnStart::Unavailable(
                contract::UnavailableReason::CapacityLimited,
            ));
        };
        let agent =
            super::assistant_team_service::ensure_nyxbot(db, &admitted.authority.user_id).await?;
        let mut start = engine::TurnStart::from(&request);
        start.model = Some(agent.model.clone());
        let ids = engine::OwnerTurnIds {
            conversation_id: format!("nyxa-{}", uuid::Uuid::new_v4().simple()),
            turn_id: uuid::Uuid::new_v4().to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
        };
        let mut session = db.client().start_session().await?;
        let context = (
            db.clone(),
            self.state.encryption_keys.clone(),
            self.identity.clone(),
            self.ingress.clone(),
            admitted.clone(),
            start.clone(),
            ids,
            agent,
        );
        let claim = session
            .start_transaction()
            .and_run(
                context,
                |session, (db, keys, identity, ingress, admitted, start, ids, agent)| {
                    Box::pin(async move {
                        let result = claim_personal_turn_in_session(
                            db,
                            session,
                            keys,
                            identity.as_ref(),
                            ingress.as_ref(),
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
            .map_err(transactions::map_transaction_error)?;
        super::assistant_agent_credential_service::audit_provision(
            db,
            &admitted.authority.user_id,
            &claim.conversation.id,
            &claim.credential,
            false,
        )
        .await;
        let origin = claim
            .conversation
            .active_turn
            .as_ref()
            .and_then(|turn| turn.platform_origin.as_ref())
            .ok_or(AppError::AssistantTurnRequired)?;
        let origin = contract_origin(origin)?;
        let turn = contract::TurnRef {
            conversation_id: claim.conversation.id.clone(),
            turn_id: claim
                .conversation
                .active_turn
                .as_ref()
                .ok_or(AppError::AssistantTurnRequired)?
                .turn_id
                .clone(),
        };
        let destination = contract::ReplyDestination {
            manager: origin.authority.manager.clone(),
            sender: origin.authority.sender.clone(),
            authority: contract::ReplyAuthority::Personal {
                origin,
                turn: turn.clone(),
            },
            expires_at: chrono::Utc::now()
                + chrono::Duration::seconds(engine::ACTIVE_TURN_TTL_SECS),
        };
        let state = self.state.clone();
        let egress = self.egress.clone();
        let running = turn.clone();
        claim.log_reclaimed();
        let settled_hook = self.settled_hook.clone();
        // The bounded receiver belongs to the caller before the worker starts.
        tokio::spawn(
            async move {
                let row = claim.conversation;
                let text = engine::turn_input(&row, &start);
                let (sender, _) = tokio::sync::broadcast::channel(256);
                let settled = runtime::run_owner_turn(
                    state.clone(),
                    egress,
                    row.clone(),
                    text,
                    claim.credential,
                    Some(
                        super::billing::route_inventory::BillingRoutePolicy::Metered(
                            super::billing::route_inventory::BillingIngress::Proxy,
                        ),
                    ),
                    permit,
                    runtime::Events {
                        checkpoint: None,
                        sender,
                        cursor: 0,
                        reset_notified: false,
                    },
                )
                .await;
                if let Some(settled) = settled {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(2),
                        events.send(contract::ExecutionEvent::ReplyReady(contract::ReplyRef {
                            turn: running.clone(),
                            reply_id: settled.message_id.clone(),
                        })),
                    )
                    .await;
                    if settled.error.is_none()
                        && row.message_count == 1
                        && super::assistant_title_service::eligible(&row)
                    {
                        super::assistant_title_service::spawn(
                            state.clone(),
                            row.user_id.clone(),
                            row.id.clone(),
                        );
                    }
                    let _ = tokio::time::timeout(
                        Duration::from_secs(2),
                        events.send(contract::ExecutionEvent::Settled {
                            turn: running.clone(),
                        }),
                    )
                    .await;
                    settled_hook
                        .settled(
                            row,
                            contract::ReplyRef {
                                turn: running,
                                reply_id: settled.message_id,
                            },
                        )
                        .await;
                }
            }
            .in_current_span(),
        );
        Ok(contract::PersonalTurnStart::Started { turn, destination })
    }

    async fn stop_exact_platform_turn(
        &self,
        db: &Database,
        session: &mut ClientSession,
        origin: &contract::PlatformTurnOrigin,
        turn: &contract::TurnRef,
    ) -> AppResult<contract::ExactStopOutcome> {
        capture_exact_stop_in_session(db, session, origin, turn).await
    }

    async fn apply_exact_platform_stop(
        &self,
        effects: &contract::CapturedPlatformStop,
    ) -> AppResult<contract::ExactStopProgress> {
        apply_exact_stop(&self.state, effects).await
    }

    async fn decide_platform_reply(
        &self,
        db: &Database,
        admitted: &contract::AdmittedPersonalUpdate,
        conversation_id: &str,
        command: contract::OriginalManagerCommand,
    ) -> AppResult<contract::PlatformDecisionOutcome> {
        let mut session = db.client().start_session().await?;
        let context = (
            db.clone(),
            self.identity.clone(),
            self.ingress.clone(),
            admitted.clone(),
            conversation_id.to_owned(),
            command,
        );
        session.start_transaction().and_run(context,
            |session,(db,identity,ingress,admitted,conversation_id,command)|Box::pin(async move {
                let result=super::assistant_platform_reply_decision::decide_personal_reply_in_session(
                    db,session,identity.as_ref(),ingress.as_ref(),admitted,conversation_id,command).await;
                transactions::transaction_result(result)
            }),
        ).await.map_err(transactions::map_transaction_error)
    }
}

async fn apply_exact_stop(
    state: &AppState,
    effects: &contract::CapturedPlatformStop,
) -> AppResult<contract::ExactStopProgress> {
    use contract::{ExactMachineCancelOutcome as Outcome, ExactStopSettlement as Settlement};
    if !engine::valid_id(&effects.turn.conversation_id)
        || uuid::Uuid::parse_str(&effects.turn.turn_id).is_err()
    {
        return Err(AppError::AssistantTurnRequired);
    }
    let expected = model_origin(&effects.origin);
    let db = &state.db;
    let collection = db.collection::<AssistantConversation>(CONVERSATIONS);
    let current = collection
        .find_one(
            doc! {"_id":&effects.turn.conversation_id,"user_id":&effects.origin.authority.user_id},
        )
        .await?;
    let settlement = match current {
        None => Settlement::ConversationDeleted,
        Some(row)
            if row.active_turn.as_ref().is_some_and(|turn| {
                turn.turn_id == effects.turn.turn_id
                    && turn.platform_origin.as_ref() == Some(&expected)
                    && turn.stop_requested
            }) =>
        {
            match tokio::time::timeout(
                Duration::from_secs(5),
                engine::force_stop(db, &row.user_id, &row.id, &effects.turn.turn_id),
            )
            .await
            {
                Ok(Ok(true)) => Settlement::Settled,
                Ok(Ok(false)) => Settlement::AlreadySettled,
                Ok(Err(AppError::NotFound(_))) => Settlement::AlreadySettled,
                Ok(Err(_)) | Err(_) => Settlement::Pending,
            }
        }
        Some(row)
            if row
                .active_turn
                .as_ref()
                .is_some_and(|turn| turn.turn_id == effects.turn.turn_id) =>
        {
            Settlement::Pending
        }
        Some(_) => Settlement::AlreadySettled,
    };
    // Each node/runtime is contacted once; job IDs supplement the exact turn
    // scope. Empty IDs still cancel browser/files/computer work of this turn.
    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for scope in &effects.machine_scopes {
        groups
            .entry((scope.node_id.clone(), scope.runtime_id.clone()))
            .or_default();
    }
    for job in &effects.machine_jobs {
        groups
            .entry((job.node_id.clone(), job.runtime_id.clone()))
            .or_default()
            .push(job.job_id.clone());
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    use futures::StreamExt;
    let outcomes:Vec<_>=futures::stream::iter(groups).map(|((node_id,runtime_id),ids)|async move {
        let work=async {
            let nodes=db.collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME);
            let Some(node)=nodes.find_one(doc! {"_id":&node_id}).await? else {return AppResult::Ok(Outcome::NodeUnavailable);};
            if node.machine.as_ref().is_none_or(|profile|profile.runtime_id!=runtime_id) {return Ok(Outcome::RuntimeChanged);}
            let result=super::machine_service::dispatch(state,&node,nyxid_machine::Operation::CancelExact,
                serde_json::json!({"conversation_id":effects.turn.conversation_id,"turn_id":effects.turn.turn_id,"job_ids":ids,"runtime_id":runtime_id})).await;
            let Some(latest)=nodes.find_one(doc! {"_id":&node_id}).await? else {return Ok(Outcome::NodeUnavailable);};
            if latest.machine.as_ref().is_none_or(|profile|profile.runtime_id!=runtime_id) {return Ok(Outcome::RuntimeChanged);}
            Ok(match result {
                Ok(value) if value.get("stopped").and_then(serde_json::Value::as_bool)==Some(true)=>Outcome::Cancelled,
                Ok(_)=>Outcome::Unsupported,
                Err(AppError::MachineAuthorityUnsupported)=>Outcome::Unsupported,
                Err(_)=>Outcome::NodeUnavailable,
            })
        };
        let outcome=match tokio::time::timeout_at(deadline,work).await {Ok(Ok(outcome))=>outcome,_=>Outcome::NodeUnavailable};
        ((node_id,runtime_id),outcome)
    }).buffer_unordered(8).collect().await;
    let outcomes: BTreeMap<_, _> = outcomes.into_iter().collect();
    let mut machine_jobs = Vec::new();
    for job in &effects.machine_jobs {
        let mut outcome = *outcomes
            .get(&(job.node_id.clone(), job.runtime_id.clone()))
            .unwrap_or(&Outcome::NodeUnavailable);
        if matches!(outcome, Outcome::Cancelled | Outcome::RuntimeChanged) {
            let result=db.collection::<MachineJob>(JOBS).update_one(doc! {
                "_id":&job.job_id,"user_id":&effects.origin.authority.user_id,"node_id":&job.node_id,"runtime_id":&job.runtime_id,
                "conversation_id":&effects.turn.conversation_id,"issuing_turn_id":&effects.turn.turn_id,"state":"running",
            },doc! {"$set":{"state":"finished","finished_at":bson::DateTime::now()}}).await?;
            if result.matched_count == 0 {
                outcome = Outcome::AlreadyFinished;
            }
        }
        machine_jobs.push(contract::ExactMachineJobProgress {
            job: job.clone(),
            outcome,
        });
    }
    let machine_scopes = effects
        .machine_scopes
        .iter()
        .map(|scope| contract::ExactMachineScopeProgress {
            scope: scope.clone(),
            outcome: *outcomes
                .get(&(scope.node_id.clone(), scope.runtime_id.clone()))
                .unwrap_or(&Outcome::NodeUnavailable),
        })
        .collect();
    Ok(contract::ExactStopProgress {
        settlement,
        machine_jobs,
        machine_scopes,
    })
}

#[cfg(test)]
#[path = "platform_personal_turn_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "platform_personal_turn_kms_tests.rs"]
mod kms_tests;
