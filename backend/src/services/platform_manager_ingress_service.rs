//! Durable audience and turn claims. Every mutation joins its caller's transaction.
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};

use super::platform_manager_contract::*;
use crate::{
    errors::{AppError, AppResult},
    models::platform_manager_ingress::{
        COLLECTION_NAME, IngressAudience, IngressDecision, IngressExecution, IngressPhase,
        PlatformManagerIngress,
    },
};

pub fn producer(_state: crate::AppState) -> Arc<dyn ManagerIngressTurnStore> {
    Arc::new(IngressTurnStore)
}

pub struct IngressTurnStore;

fn changed() -> AppError {
    AppError::Conflict("Telegram update was already claimed or its authority changed".into())
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    let collection = db.collection::<Document>(COLLECTION_NAME);
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"manager_bot_id": 1, "telegram_bot_id": 1, "sender_id": 1, "update_id": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"phase": 1, "start_before": 1, "_id": 1})
                .build(),
        )
        .await?;
    // No TTL: unfinished execution and delivery recovery must retain their proof.
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"phase":1,"delivery_recovery_queued":1,"settled_at":1,"_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

fn update_identity(update: &VerifiedManagerUpdate) -> Document {
    doc! {
        "manager_bot_id": &update.manager_bot_id,
        "telegram_bot_id": update.telegram_bot_id,
        "sender_id": update.sender_id,
        "update_id": update.update_id,
    }
}

/// Check this before resolving the current audience. A retry never reclassifies.
pub async fn find_in_session(
    db: &Database,
    session: &mut ClientSession,
    update: &VerifiedManagerUpdate,
) -> AppResult<Option<PlatformManagerIngress>> {
    Ok(db
        .collection::<PlatformManagerIngress>(COLLECTION_NAME)
        .find_one(update_identity(update))
        .session(session)
        .await?)
}

/// The dispatcher has already fenced manager, sender and account authority in
/// this transaction. IDs are allocated before the transaction retry callback.
pub async fn record_admission_in_session(
    db: &Database,
    session: &mut ClientSession,
    update: &VerifiedManagerUpdate,
    manager: &ManagerAuthority,
    ingress: &IngressClaim,
    audience: &ManagerAudience,
) -> AppResult<IngressAdmission> {
    if let Some(existing) = find_in_session(db, session, update).await? {
        return Ok(IngressAdmission::Duplicate {
            receipt_id: existing.id,
        });
    }
    if manager.profile_id != update.profile_id
        || manager.manager_bot_id != update.manager_bot_id
        || manager.telegram_bot_id != update.telegram_bot_id
        || manager.manager_authority_epoch <= 0
        || ingress.receipt_id.is_empty()
        || ingress.claim_id.is_empty()
    {
        return Err(changed());
    }
    let stored_audience = match audience {
        ManagerAudience::Control
            if update.chat_kind == ManagerChatKind::Private
                && update.command_eligibility
                    == ManagerCommandEligibility::OriginalPrivateMessage
                && update.sender_id == update.chat_id =>
        {
            IngressAudience::Control
        }
        ManagerAudience::Personal(admitted)
            if admitted.ingress == *ingress
                && admitted.authority.manager == *manager
                && admitted.authority.sender_id == update.sender_id
                && admitted.authority.chat_id == update.chat_id
                && update.chat_kind == ManagerChatKind::Private =>
        {
            IngressAudience::Personal {
                user_id: admitted.authority.user_id.clone(),
                binding_id: admitted.authority.binding_id.clone(),
                binding_generation: admitted.authority.binding_generation,
                personal_control_revision: admitted.authority.personal_control_revision,
            }
        }
        ManagerAudience::Public(admitted)
            if admitted.ingress == *ingress
                && admitted.manager == *manager
                && admitted.sender_id == update.sender_id
                && admitted.chat_id == update.chat_id
                && update.chat_kind == ManagerChatKind::Private =>
        {
            IngressAudience::Public {
                public_policy_revision: admitted.public_policy_revision,
            }
        }
        ManagerAudience::Fixed(reason) => IngressAudience::Fixed {
            reason: reason_code(*reason).into(),
        },
        ManagerAudience::PrivateChatHandoff if update.chat_kind != ManagerChatKind::Private => {
            IngressAudience::PrivateChatHandoff
        }
        _ => return Err(changed()),
    };
    let now = Utc::now();
    let receipt = PlatformManagerIngress {
        id: ingress.receipt_id.clone(),
        claim_id: ingress.claim_id.clone(),
        profile_id: manager.profile_id.clone(),
        manager_bot_id: manager.manager_bot_id.clone(),
        telegram_bot_id: manager.telegram_bot_id,
        manager_authority_epoch: manager.manager_authority_epoch,
        observation_id: update.observation_id.clone(),
        sender_id: update.sender_id,
        chat_id: update.chat_id,
        update_id: update.update_id,
        message_id: update.message_id,
        original_private_command: update.chat_kind == ManagerChatKind::Private
            && update.command_eligibility == ManagerCommandEligibility::OriginalPrivateMessage,
        audience: stored_audience,
        phase: IngressPhase::Admitted,
        execution: None,
        decision: None,
        reply_id: None,
        delivery_recovery_queued: false,
        interrupted_reason: None,
        effect_revision: 0,
        admitted_at: now,
        start_before: now + Duration::minutes(10),
        settled_at: None,
    };
    db.collection::<PlatformManagerIngress>(COLLECTION_NAME)
        .insert_one(&receipt)
        .session(session)
        .await?;
    Ok(IngressAdmission::Claimed {
        ingress: ingress.clone(),
        audience: audience.clone(),
    })
}

fn reason_code(reason: UnavailableReason) -> &'static str {
    match reason {
        UnavailableReason::ModeDisabled => "mode_disabled",
        UnavailableReason::ManagerUnavailable => "manager_unavailable",
        UnavailableReason::EmergencyStopped => "emergency_stopped",
        UnavailableReason::PersonalDisabled => "personal_disabled",
        UnavailableReason::PublicDisabled => "public_disabled",
        UnavailableReason::OwnerUnavailable => "owner_unavailable",
        UnavailableReason::TransportUnavailable => "transport_unavailable",
        UnavailableReason::OriginRevoked => "origin_revoked",
        UnavailableReason::EngineUnavailable => "engine_unavailable",
        UnavailableReason::InsufficientFunds => "insufficient_funds",
        UnavailableReason::CapacityLimited => "capacity_limited",
        UnavailableReason::PublicNotReady => "public_not_ready",
        UnavailableReason::PublicQuotaLimited => "public_quota_limited",
        UnavailableReason::RuntimeUninitialized => "runtime_uninitialized",
        UnavailableReason::SponsorUnavailable => "sponsor_unavailable",
        UnavailableReason::PublicContractUnsupported => "public_contract_unsupported",
        UnavailableReason::PublicAuthorityRejected => "public_authority_rejected",
        UnavailableReason::ProfileUnavailable => "profile_unavailable",
        UnavailableReason::ProofExpired => "proof_expired",
        UnavailableReason::PublicTurnFailed => "public_turn_failed",
    }
}

fn claim_filter(ingress: &IngressClaim) -> Document {
    doc! {"_id": &ingress.receipt_id, "claim_id": &ingress.claim_id}
}

fn personal_filter(admitted: &AdmittedPersonalUpdate) -> Document {
    let authority = &admitted.authority;
    let mut filter = claim_filter(&admitted.ingress);
    filter.extend(doc! {
        "profile_id": &authority.manager.profile_id,
        "manager_bot_id": &authority.manager.manager_bot_id,
        "telegram_bot_id": authority.manager.telegram_bot_id,
        "manager_authority_epoch": authority.manager.manager_authority_epoch,
        "sender_id": authority.sender_id, "chat_id": authority.chat_id,
        "audience.kind": "personal", "audience.user_id": &authority.user_id,
        "audience.binding_id": &authority.binding_id,
        "audience.binding_generation": authority.binding_generation,
        "audience.personal_control_revision": authority.personal_control_revision,
    });
    filter
}

fn unclaimed(filter: &mut Document) {
    filter.extend(doc! {
        "phase": "admitted", "execution": bson::Bson::Null,
        "decision": bson::Bson::Null, "reply_id": bson::Bson::Null,
        "start_before": {"$gt": bson::DateTime::from_chrono(Utc::now())},
    });
}

fn encode<T: serde::Serialize>(value: &T) -> AppResult<bson::Bson> {
    bson::to_bson(value).map_err(|_| AppError::Internal("Failed to encode ingress metadata".into()))
}

async fn update_exact(
    db: &Database,
    session: &mut ClientSession,
    filter: Document,
    fields: Document,
) -> AppResult<()> {
    let result = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            filter,
            doc! {"$set": fields, "$inc": {"effect_revision": 1_i64}},
        )
        .session(session)
        .await?;
    if result.matched_count != 1 {
        return Err(changed());
    }
    Ok(())
}

fn decision_metadata(decision: &PlatformCardDecision) -> IngressDecision {
    IngressDecision {
        acknowledgement_id: decision.acknowledgement_id.clone(),
        conversation_id: decision.requested_turn.conversation_id.clone(),
        requested_turn_id: decision.requested_turn.turn_id.clone(),
        allowed: decision.allowed,
        voice_request_id: decision.voice_request_id.clone(),
        continuation_receipt_id: decision.continuation_receipt_id.clone(),
        trigger_run_id: decision.trigger_run_id.clone(),
    }
}

#[async_trait]
impl ManagerIngressTurnStore for IngressTurnStore {
    async fn claim_personal_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        admitted: &AdmittedPersonalUpdate,
        turn: &TurnRef,
    ) -> AppResult<()> {
        let mut filter = personal_filter(admitted);
        unclaimed(&mut filter);
        let execution = IngressExecution {
            conversation_id: turn.conversation_id.clone(),
            turn_id: turn.turn_id.clone(),
            public_session_id: None,
            payer_user_id: admitted.authority.user_id.clone(),
            public_policy_digest: None,
        };
        update_exact(
            db,
            session,
            filter,
            doc! {"phase": "executing", "execution": encode(&execution)?},
        )
        .await
    }

    async fn settle_personal_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        ingress: &IngressClaim,
        reply: &ReplyRef,
    ) -> AppResult<()> {
        settle_execution(db, session, ingress, reply, "personal").await
    }

    async fn claim_personal_decision(
        &self,
        db: &Database,
        session: &mut ClientSession,
        admitted: &AdmittedPersonalUpdate,
        command: &OriginalManagerCommand,
        decision: &PlatformCardDecision,
    ) -> AppResult<()> {
        let update = command.update();
        if update.profile_id != admitted.authority.manager.profile_id
            || update.manager_bot_id != admitted.authority.manager.manager_bot_id
            || update.telegram_bot_id != admitted.authority.manager.telegram_bot_id
            || update.sender_id != admitted.authority.sender_id
            || update.chat_id != admitted.authority.chat_id
        {
            return Err(changed());
        }
        let mut filter = personal_filter(admitted);
        unclaimed(&mut filter);
        filter.extend(doc! {
            "original_private_command": true,
            "observation_id": &update.observation_id,
            "update_id": update.update_id,
            "message_id": update.message_id,
        });
        update_exact(
            db,
            session,
            filter,
            doc! {"phase": "deciding", "decision": encode(&decision_metadata(decision))?},
        )
        .await
    }

    async fn settle_personal_decision(
        &self,
        db: &Database,
        session: &mut ClientSession,
        ingress: &IngressClaim,
        decision: &PlatformCardDecision,
    ) -> AppResult<()> {
        let mut filter = claim_filter(ingress);
        filter.extend(doc! {
            "audience.kind": "personal", "original_private_command": true,
            "phase": {"$in": ["deciding", "settled"]}, "execution": bson::Bson::Null,
            "decision": encode(&decision_metadata(decision))?,
        });
        update_exact(
            db,
            session,
            filter,
            doc! {"phase": "settled", "settled_at": bson::DateTime::from_chrono(Utc::now())},
        )
        .await
    }

    async fn claim_public_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        context: &PublicExecutionContext,
    ) -> AppResult<()> {
        let ingress = &context.ingress;
        let mut filter = claim_filter(&ingress.ingress);
        filter.extend(doc! {
            "profile_id": &ingress.manager.profile_id,
            "manager_bot_id": &ingress.manager.manager_bot_id,
            "telegram_bot_id": ingress.manager.telegram_bot_id,
            "manager_authority_epoch": ingress.manager.manager_authority_epoch,
            "sender_id": ingress.sender_id, "chat_id": ingress.chat_id,
            "audience.kind": "public", "audience.public_policy_revision": ingress.public_policy_revision,
        });
        unclaimed(&mut filter);
        if context.expires_at <= Utc::now() {
            return Err(changed());
        }
        let execution = IngressExecution {
            conversation_id: context.turn.conversation_id.clone(),
            turn_id: context.turn.turn_id.clone(),
            public_session_id: Some(context.public_session_id.clone()),
            payer_user_id: context.sponsor_owner_user_id.clone(),
            public_policy_digest: Some(context.policy_digest.clone()),
        };
        update_exact(
            db,
            session,
            filter,
            doc! {"phase": "executing", "execution": encode(&execution)?},
        )
        .await
    }

    async fn settle_public_execution(
        &self,
        db: &Database,
        session: &mut ClientSession,
        ingress: &IngressClaim,
        reply: &ReplyRef,
    ) -> AppResult<()> {
        settle_execution(db, session, ingress, reply, "public").await
    }
}

async fn settle_execution(
    db: &Database,
    session: &mut ClientSession,
    ingress: &IngressClaim,
    reply: &ReplyRef,
    audience: &str,
) -> AppResult<()> {
    let mut filter = claim_filter(ingress);
    filter.extend(doc! {
        "audience.kind": audience,
        "execution.conversation_id": &reply.turn.conversation_id,
        "execution.turn_id": &reply.turn.turn_id,
        "decision": bson::Bson::Null,
        "$or": [
            {"phase": "executing", "reply_id": bson::Bson::Null},
            {"phase": "settled", "reply_id": &reply.reply_id},
        ],
    });
    // The caller persists the reply in this same transaction. Revocation or an
    // expired admission deadline must not prevent exact old-turn settlement.
    update_exact(
        db,
        session,
        filter,
        doc! {
            "phase": "settled", "reply_id": &reply.reply_id,
            "settled_at": bson::DateTime::from_chrono(Utc::now()),
        },
    )
    .await
}

/// Deletion invalidates delivery without inventing a reply or changing the
/// originally admitted audience. A retry may only observe this same terminal.
pub(crate) async fn interrupt_personal_execution_in_session(
    db: &Database,
    session: &mut ClientSession,
    origin: &PlatformTurnOrigin,
    turn: &TurnRef,
) -> AppResult<()> {
    let PlatformOriginSource::Ingress(ingress) = &origin.source else {
        return Err(changed());
    };
    let mut filter = personal_filter(&AdmittedPersonalUpdate {
        ingress: ingress.clone(),
        authority: origin.authority.clone(),
    });
    filter.extend(doc! {
        "execution.conversation_id": &turn.conversation_id,
        "execution.turn_id": &turn.turn_id,
        "execution.payer_user_id": &origin.authority.user_id,
        "decision": bson::Bson::Null, "reply_id":bson::Bson::Null,
        "$or": [
            {"phase":"executing", "interrupted_reason":bson::Bson::Null},
            {"phase":"interrupted", "interrupted_reason":"conversation_deleted"},
        ],
    });
    update_exact(
        db,
        session,
        filter,
        doc! {
            "phase":"interrupted", "interrupted_reason":"conversation_deleted",
            "settled_at":bson::DateTime::from_chrono(Utc::now()),
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    async fn fixture() -> (Database, VerifiedManagerUpdate, AdmittedPersonalUpdate) {
        let db = crate::test_utils::connect_transaction_test_database("manager_ingress").await;
        ensure_indexes(&db).await.unwrap();
        let update = VerifiedManagerUpdate {
            profile_id: Uuid::new_v4().to_string(),
            manager_bot_id: Uuid::new_v4().to_string(),
            telegram_bot_id: 100,
            observation_id: Uuid::new_v4().to_string(),
            sender_id: 200,
            chat_id: 200,
            chat_kind: ManagerChatKind::Private,
            update_id: 300,
            message_id: 400,
            command_eligibility: ManagerCommandEligibility::OriginalPrivateMessage,
        };
        let admitted = AdmittedPersonalUpdate {
            ingress: IngressClaim {
                receipt_id: Uuid::new_v4().to_string(),
                claim_id: Uuid::new_v4().to_string(),
            },
            authority: PersonalBindingAuthority {
                manager: ManagerAuthority {
                    profile_id: update.profile_id.clone(),
                    manager_bot_id: update.manager_bot_id.clone(),
                    telegram_bot_id: update.telegram_bot_id,
                    manager_authority_epoch: 1,
                },
                sender_id: update.sender_id,
                chat_id: update.chat_id,
                binding_id: Uuid::new_v4().to_string(),
                binding_generation: 1,
                personal_control_revision: 1,
                user_id: Uuid::new_v4().to_string(),
            },
        };
        (db, update, admitted)
    }

    async fn admit(
        db: &Database,
        update: &VerifiedManagerUpdate,
        admitted: &AdmittedPersonalUpdate,
    ) {
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        record_admission_in_session(
            db,
            &mut session,
            update,
            &admitted.authority.manager,
            &admitted.ingress,
            &ManagerAudience::Personal(admitted.clone()),
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
    }

    fn turn() -> TurnRef {
        TurnRef {
            conversation_id: format!("nyxa-{}", Uuid::new_v4().simple()),
            turn_id: Uuid::new_v4().to_string(),
        }
    }

    #[tokio::test]
    async fn deletion_interruption_is_exact_transactional_and_terminal() {
        let (db, update, admitted) = fixture().await;
        admit(&db, &update, &admitted).await;
        let turn = turn();
        let origin = PlatformTurnOrigin {
            authority: admitted.authority.clone(),
            nyxbot_agent_id: Uuid::new_v4().to_string(),
            source: PlatformOriginSource::Ingress(admitted.ingress.clone()),
        };
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        IngressTurnStore
            .claim_personal_execution(&db, &mut session, &admitted, &turn)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        interrupt_personal_execution_in_session(&db, &mut session, &origin, &turn)
            .await
            .unwrap();
        session.abort_transaction().await.unwrap();
        let receipt = db
            .collection::<PlatformManagerIngress>(COLLECTION_NAME)
            .find_one(doc! {"_id":&admitted.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert!(receipt.phase == IngressPhase::Executing);
        let mut wrong = origin.clone();
        wrong.authority.binding_generation += 1;
        session.start_transaction().await.unwrap();
        assert!(
            interrupt_personal_execution_in_session(&db, &mut session, &wrong, &turn)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        for _ in 0..2 {
            session.start_transaction().await.unwrap();
            interrupt_personal_execution_in_session(&db, &mut session, &origin, &turn)
                .await
                .unwrap();
            session.commit_transaction().await.unwrap();
        }
        session.start_transaction().await.unwrap();
        assert!(
            IngressTurnStore
                .settle_personal_execution(
                    &db,
                    &mut session,
                    &admitted.ingress,
                    &ReplyRef {
                        turn: turn.clone(),
                        reply_id: "invented".into()
                    }
                )
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        let receipt = db
            .collection::<PlatformManagerIngress>(COLLECTION_NAME)
            .find_one(doc! {"_id":&admitted.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert!(receipt.phase == IngressPhase::Interrupted);
        assert_eq!(
            receipt.interrupted_reason.as_deref(),
            Some("conversation_deleted")
        );
        assert!(receipt.reply_id.is_none());
        assert!(
            receipt.audience
                == IngressAudience::Personal {
                    user_id: admitted.authority.user_id.clone(),
                    binding_id: admitted.authority.binding_id.clone(),
                    binding_generation: admitted.authority.binding_generation,
                    personal_control_revision: admitted.authority.personal_control_revision
                }
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn duplicate_retains_audience_and_claim_is_transactional_and_exclusive() {
        let (db, update, admitted) = fixture().await;
        admit(&db, &update, &admitted).await;
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let public = PublicIngress {
            ingress: IngressClaim {
                receipt_id: Uuid::new_v4().to_string(),
                claim_id: Uuid::new_v4().to_string(),
            },
            manager: admitted.authority.manager.clone(),
            sender_id: update.sender_id,
            chat_id: update.chat_id,
            public_policy_revision: 8,
        };
        let duplicate = record_admission_in_session(
            &db,
            &mut session,
            &update,
            &public.manager,
            &public.ingress,
            &ManagerAudience::Public(public.clone()),
        )
        .await
        .unwrap();
        assert!(
            matches!(duplicate, IngressAdmission::Duplicate {receipt_id} if receipt_id == admitted.ingress.receipt_id)
        );
        session.commit_transaction().await.unwrap();

        let abandoned = turn();
        session.start_transaction().await.unwrap();
        IngressTurnStore
            .claim_personal_execution(&db, &mut session, &admitted, &abandoned)
            .await
            .unwrap();
        session.abort_transaction().await.unwrap();
        let chosen = turn();
        session.start_transaction().await.unwrap();
        IngressTurnStore
            .claim_personal_execution(&db, &mut session, &admitted, &chosen)
            .await
            .unwrap();
        // A separate transaction cannot acquire the receipt while the winning
        // claim is uncommitted, even on another backend replica.
        let mut rival = db.client().start_session().await.unwrap();
        rival.start_transaction().await.unwrap();
        assert!(
            IngressTurnStore
                .claim_personal_execution(&db, &mut rival, &admitted, &abandoned)
                .await
                .is_err()
        );
        let _ = rival.abort_transaction().await;
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            IngressTurnStore
                .claim_personal_execution(&db, &mut session, &admitted, &chosen)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();

        // Settlement survives expiry/revocation but remains pinned to the exact
        // claim/turn and cannot replace a previously saved reply.
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(doc! {"_id": &admitted.ingress.receipt_id}, doc! {"$set": {"start_before": bson::DateTime::from_chrono(Utc::now() - Duration::hours(1))}})
            .await.unwrap();
        let reply = ReplyRef {
            turn: chosen,
            reply_id: Uuid::new_v4().to_string(),
        };
        for _ in 0..2 {
            session.start_transaction().await.unwrap();
            IngressTurnStore
                .settle_personal_execution(&db, &mut session, &admitted.ingress, &reply)
                .await
                .unwrap();
            session.commit_transaction().await.unwrap();
        }
        session.start_transaction().await.unwrap();
        let different = ReplyRef {
            turn: reply.turn.clone(),
            reply_id: Uuid::new_v4().to_string(),
        };
        assert!(
            IngressTurnStore
                .settle_personal_execution(&db, &mut session, &admitted.ingress, &different)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        let stored = db
            .collection::<PlatformManagerIngress>(COLLECTION_NAME)
            .find_one(doc! {"_id": &admitted.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(stored.audience, IngressAudience::Personal { .. }));
        assert_eq!(stored.reply_id.as_deref(), Some(reply.reply_id.as_str()));
        assert!(stored.decision.is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn decisions_require_exact_original_command_and_exclude_turn_start() {
        let (db, update, admitted) = fixture().await;
        admit(&db, &update, &admitted).await;
        let decision = PlatformCardDecision {
            acknowledgement_id: Uuid::new_v4().to_string(),
            requested_turn: turn(),
            allowed: true,
            voice_request_id: None,
            continuation_receipt_id: None,
            trigger_run_id: None,
        };
        let mut session = db.client().start_session().await.unwrap();
        let mut wrong_update = update.clone();
        wrong_update.update_id += 1;
        let wrong_command = OriginalManagerCommand::from_verified(
            wrong_update,
            TurnInput {
                text: "yes 1234".into(),
            },
        )
        .unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            IngressTurnStore
                .claim_personal_decision(&db, &mut session, &admitted, &wrong_command, &decision)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        let command = OriginalManagerCommand::from_verified(
            update.clone(),
            TurnInput {
                text: "yes 1234".into(),
            },
        )
        .unwrap();
        // Even a valid opaque command cannot upgrade a receipt recorded as
        // forwarded/non-control input.
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &admitted.ingress.receipt_id},
                doc! {"$set": {"original_private_command": false}},
            )
            .await
            .unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            IngressTurnStore
                .claim_personal_decision(&db, &mut session, &admitted, &command, &decision)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &admitted.ingress.receipt_id},
                doc! {"$set": {"original_private_command": true}},
            )
            .await
            .unwrap();
        session.start_transaction().await.unwrap();
        IngressTurnStore
            .claim_personal_decision(&db, &mut session, &admitted, &command, &decision)
            .await
            .unwrap();
        IngressTurnStore
            .settle_personal_decision(&db, &mut session, &admitted.ingress, &decision)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            IngressTurnStore
                .claim_personal_execution(&db, &mut session, &admitted, &turn())
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        let raw = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": &admitted.ingress.receipt_id})
            .await
            .unwrap()
            .unwrap();
        assert!(!raw.to_string().contains("yes 1234"));
        assert_eq!(raw.get_str("phase").unwrap(), "settled");
        db.drop().await.unwrap();
    }
}
