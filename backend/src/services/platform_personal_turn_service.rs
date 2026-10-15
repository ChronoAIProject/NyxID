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
            platform: "telegram".into(),
            profile_id: authority.manager.profile_id.clone(),
            manager_bot_id: authority.manager.manager_bot_id.clone(),
            telegram_bot_id: authority.manager.telegram_bot_id,
            binding_id: authority.binding_id.clone(),
            binding_generation: authority.binding_generation,
            user_id: authority.user_id.clone(),
            nyxbot_agent_id: origin.nyxbot_agent_id.clone(),
        },
        manager_authority_epoch: authority.manager.manager_authority_epoch,
        personal_control_revision: authority.personal_control_revision,
        sender_id: authority.sender_id,
        chat_id: authority.chat_id,
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
    if origin.thread.platform != "telegram" {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(contract::PlatformTurnOrigin {
        authority: contract::PersonalBindingAuthority {
            manager: contract::ManagerAuthority {
                profile_id: origin.thread.profile_id.clone(),
                manager_bot_id: origin.thread.manager_bot_id.clone(),
                telegram_bot_id: origin.thread.telegram_bot_id,
                manager_authority_epoch: origin.manager_authority_epoch,
            },
            sender_id: origin.sender_id,
            chat_id: origin.chat_id,
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
    if !matches!(
        identity
            .fence_personal_origin(db, session, authority)
            .await?,
        contract::FenceOutcome::Authorized
    ) {
        return Err(AppError::AssistantTurnRequired);
    }
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
    let origin = contract::PlatformTurnOrigin {
        authority: authority.clone(),
        nyxbot_agent_id: agent.id.clone(),
        source: contract::PlatformOriginSource::Ingress(admitted.ingress.clone()),
    };
    let turn = contract::TurnRef {
        conversation_id: ids.conversation_id.clone(),
        turn_id: ids.turn_id.clone(),
    };
    let claim = engine::begin_owner_turn_in_session(
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
    )
    .await?;
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
