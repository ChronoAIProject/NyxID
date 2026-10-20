//! Durable intent/outcome capacity plus process-local one-shot authorization.
//! Provider idempotency/reconciliation belongs to the effect's business owner.
use super::{AuditRuntime, WitnessRelay, policy::AuditTier};
use crate::{
    errors::{AppError, AppResult, AuditAdmissionReason},
    models::{
        audit_effect_intent::{AuditEffectIntent, COLLECTION_NAME as INTENTS, EffectPhase},
        audit_outbox::{
            AuditDetail, AuditStage, PENDING_COLLECTION_NAME as PENDING, PendingAuditEvent,
        },
        audit_state::{AuditState, COLLECTION_NAME as STATE},
        audit_witness_delivery::{COLLECTION_NAME as DELIVERY, WitnessDelivery},
    },
    services::{
        audit_outbox_service::{self, AuditEvent, AuditEventSource},
        audit_state_service,
    },
};
use mongodb::{ClientSession, Database, bson::doc};
use nyxid::models::audit_witness::{DispatchRequest, WitnessEvent};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, Weak},
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub(super) struct EffectContext {
    process: Uuid,
    key: Arc<Zeroizing<[u8; 32]>>,
    relay: Option<Weak<WitnessRelay>>,
    issued: Mutex<HashSet<String>>,
}

/// Allocate this once outside the business transaction's retry loop. A later
/// process can recover by the stable logical operation UUID, without reusing it
/// for a different effect. The owner must not generate a new UUID on retry.
pub struct EffectReservation<E = AuditEvent> {
    pub event: E,
    pub event_id: Uuid,
    pub terminal_event_id: Uuid,
}

/// Non-cloneable, non-serializable, privately constructed, consumed by run().
/// Dropping it does not release the durable claim or permit another send.
pub struct DispatchPermit {
    operation: String,
    process: Uuid,
    attempt: Uuid,
    relay: Option<Arc<WitnessRelay>>,
    db: Database,
}
impl DispatchPermit {
    pub async fn run<F, Fut, T>(self, effect: F) -> AppResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let intent = self
            .db
            .collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": &self.operation})
            .await?
            .ok_or(AppError::AuditIntegrityConflict)?;
        if intent.phase != EffectPhase::Claimed
            || intent.terminal_stage.is_some()
            || intent.attempt.as_ref().is_none_or(|claim| {
                claim.process_id != self.process || claim.attempt_id != self.attempt
            })
        {
            return Err(deferred());
        }
        if let Some(relay) = &self.relay {
            relay.require_dispatch_now().await?;
        }
        // Keep the identity owned by this future until the provider finishes.
        let identity = (self.operation, self.process, self.attempt);
        let result = effect().await;
        drop(identity);
        Ok(result)
    }
}

fn deferred() -> AppError {
    AppError::AuditAdmissionClosed {
        reason: AuditAdmissionReason::WitnessUnavailable,
    }
}

fn apply_terminal_detail(
    event: &mut crate::models::audit_outbox::SafeAuditEvent,
    detail: Option<&AuditDetail>,
) -> AppResult<()> {
    if let Some(detail) = detail {
        if !matches!(detail, AuditDetail::WebhookDeliveryFailure { .. })
            || !matches!(event.stage, AuditStage::Failed | AuditStage::Uncertain)
            || event.detail.is_some()
        {
            return Err(AppError::AuditIntegrityConflict);
        }
        event.detail = Some(detail.clone());
    }
    Ok(())
}

impl AuditRuntime {
    /// Uses the same generation UUID advertised by this serving process. Tests
    /// opt in explicitly; ordinary test/local constructors create no readiness.
    pub fn bind_effects(
        &self,
        process: Uuid,
        key: Arc<Zeroizing<[u8; 32]>>,
        relay: Option<&Arc<WitnessRelay>>,
    ) -> AppResult<()> {
        if process.get_version_num() != 4 {
            return Err(AppError::AuditIntegrityConflict);
        }
        self.effects
            .set(EffectContext {
                process,
                key,
                relay: relay.map(Arc::downgrade),
                issued: Mutex::new(HashSet::new()),
            })
            .map_err(|_| AppError::AuditIntegrityConflict)
    }

    /// Business mutation, intent event and both reservations share this caller
    /// session. The terminal slot has no event body until an outcome exists.
    pub async fn begin_effect_in_session<E: AuditEventSource>(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tier: AuditTier,
        request: &EffectReservation<E>,
    ) -> AppResult<String> {
        let event = request.event.project();
        audit_outbox_service::validate(&event)?;
        if event.stage != AuditStage::Intent
            || request.event_id.get_version_num() != 4
            || request.terminal_event_id.get_version_num() != 4
            || request.event_id == request.terminal_event_id
        {
            return Err(AppError::AuditIntegrityConflict);
        }
        let key = self.effects.get().ok_or(AppError::AuditNotInitialized)?;
        let (initial_state, _) =
            audit_state_service::writer_snapshot(db, session, key.key.as_ref().as_ref()).await?;
        if let Some(existing) = db
            .collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": &event.operation_id})
            .session(&mut *session)
            .await?
        {
            if existing.event != event
                || (existing.terminal_stage.is_none() && existing.terminal_detail.is_some())
            {
                return Err(AppError::AuditIntegrityConflict);
            }
            let original = db
                .collection::<PendingAuditEvent>(PENDING)
                .find_one(doc! {"_id": &existing.event_id})
                .session(&mut *session)
                .await?
                .ok_or(AppError::AuditIntegrityConflict)?;
            if original.event != event
                || existing.terminal_order <= original.order
                || existing.terminal_order > initial_state.next_order
                || (existing.terminal_stage.is_none() && initial_state.pending_count == 0)
            {
                return Err(AppError::AuditIntegrityConflict);
            }
            let replay = audit_outbox_service::enqueue_in_session(
                db,
                session,
                &existing.event_id,
                &request.event,
            )
            .await?;
            if replay != existing.event_id {
                return Err(AppError::AuditIntegrityConflict);
            }
            if let Some(stage) = &existing.terminal_stage {
                let terminal = db
                    .collection::<PendingAuditEvent>(PENDING)
                    .find_one(doc! {"_id": &existing.terminal_event_id})
                    .session(&mut *session)
                    .await?
                    .ok_or(AppError::AuditIntegrityConflict)?;
                let mut expected = event.clone();
                expected.stage = stage.clone();
                apply_terminal_detail(&mut expected, existing.terminal_detail.as_ref())?;
                audit_outbox_service::validate(&expected)?;
                if terminal.event != expected
                    || terminal.order != existing.terminal_order
                    || terminal.reserved_at != existing.reserved_at
                {
                    return Err(AppError::AuditIntegrityConflict);
                }
            }
            return Ok(existing.id);
        }
        // Enqueue is permitted under the owning DB operation tier. Dispatch is
        // separately Effect-gated, so an outage can leave work durably deferred.
        self.require_new_in_session(db, session, tier, 2).await?;
        audit_outbox_service::enqueue_in_session(
            db,
            session,
            &request.event_id.to_string(),
            &request.event,
        )
        .await?;
        let (state, now) =
            audit_state_service::writer_snapshot(db, session, key.key.as_ref().as_ref()).await?;
        let order = state
            .next_order
            .checked_add(1)
            .ok_or(AppError::AuditIntegrityConflict)?;
        let count = state
            .pending_count
            .checked_add(1)
            .ok_or(AppError::AuditIntegrityConflict)?;
        let result = db.collection::<AuditState>(STATE).update_one(doc! {"_id": &state.id, "next_order": state.next_order, "pending_count": state.pending_count},
            doc! {"$set": {"next_order": order, "pending_count": count}}).session(&mut *session).await?;
        if result.matched_count != 1 {
            return Err(AppError::AuditIntegrityConflict);
        }
        db.collection::<AuditEffectIntent>(INTENTS)
            .insert_one(AuditEffectIntent {
                id: event.operation_id.clone(),
                event: event.clone(),
                event_id: request.event_id.to_string(),
                terminal_event_id: request.terminal_event_id.to_string(),
                terminal_order: order,
                reserved_at: now,
                phase: EffectPhase::Pending,
                attempt: None,
                terminal_stage: None,
                terminal_detail: None,
            })
            .session(session)
            .await?;
        Ok(event.operation_id)
    }

    /// A terminal/uncertain outcome consumes its precommitted slot even after
    /// admission closes. The owner commits its state change in this session.
    pub async fn finish_effect_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        operation: &str,
        stage: AuditStage,
    ) -> AppResult<()> {
        self.finish_effect_with_detail_in_session(db, session, operation, stage, None)
            .await
    }

    /// Commit reviewed post-dispatch failure evidence with the product outcome.
    /// Only WebhookDeliveryFailure on Failed/Uncertain is accepted. It cannot
    /// replace an intent's existing decision detail or create dispatch authority.
    pub async fn finish_effect_with_detail_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        operation: &str,
        stage: AuditStage,
        detail: Option<AuditDetail>,
    ) -> AppResult<()> {
        if !matches!(
            stage,
            AuditStage::Succeeded
                | AuditStage::Failed
                | AuditStage::Cancelled
                | AuditStage::Uncertain
        ) {
            return Err(AppError::AuditIntegrityConflict);
        }
        let context = self.effects.get().ok_or(AppError::AuditNotInitialized)?;
        let (state, _) =
            audit_state_service::writer_snapshot(db, session, context.key.as_ref().as_ref())
                .await?;
        let intents = db.collection::<AuditEffectIntent>(INTENTS);
        let intent = intents
            .find_one(doc! {"_id": operation})
            .session(&mut *session)
            .await?
            .ok_or(AppError::AuditIntegrityConflict)?;
        let mut event = intent.event.clone();
        event.stage = stage.clone();
        apply_terminal_detail(&mut event, detail.as_ref())?;
        audit_outbox_service::validate(&event)?;
        if let Some(previous) = &intent.terminal_stage {
            let terminal = db
                .collection::<PendingAuditEvent>(PENDING)
                .find_one(doc! {"_id": &intent.terminal_event_id})
                .session(&mut *session)
                .await?
                .ok_or(AppError::AuditIntegrityConflict)?;
            if previous != &stage
                || intent.terminal_detail != detail
                || terminal.event != event
                || terminal.order != intent.terminal_order
                || terminal.reserved_at != intent.reserved_at
            {
                return Err(AppError::AuditIntegrityConflict);
            }
            return Ok(());
        }
        if intent.terminal_detail.is_some() {
            return Err(AppError::AuditIntegrityConflict);
        }
        if state.pending_count <= 0
            || intent.terminal_order <= 0
            || intent.terminal_order > state.next_order
        {
            return Err(AppError::AuditIntegrityConflict);
        }
        if (stage == AuditStage::Succeeded
            && (intent.phase != EffectPhase::Claimed
                || intent
                    .attempt
                    .as_ref()
                    .is_none_or(|attempt| attempt.process_id != context.process)))
            || (stage == AuditStage::Cancelled && intent.phase != EffectPhase::Pending)
        {
            return Err(AppError::AuditIntegrityConflict);
        }
        db.collection::<PendingAuditEvent>(PENDING)
            .insert_one(PendingAuditEvent {
                id: intent.terminal_event_id,
                event,
                order: intent.terminal_order,
                reserved_at: intent.reserved_at,
                materialized_seq: None,
            })
            .session(&mut *session)
            .await?;
        let mut outcome = doc! {"phase": "terminal", "terminal_stage": bson::to_bson(&stage).map_err(|_| AppError::AuditIntegrityConflict)?};
        if let Some(detail) = detail {
            outcome.insert(
                "terminal_detail",
                bson::to_bson(&detail).map_err(|_| AppError::AuditIntegrityConflict)?,
            );
        }
        let updated = intents
            .update_one(
                doc! {"_id": operation, "terminal_stage": bson::Bson::Null},
                doc! {"$set": outcome},
            )
            .session(session)
            .await?;
        if updated.matched_count != 1 {
            return Err(AppError::AuditIntegrityConflict);
        }
        Ok(())
    }

    async fn uncertain_effect(&self, db: &Database, operation: &str) -> AppResult<()> {
        super::transaction::run(
            db,
            &(self, db, operation),
            |session, (runtime, db, operation)| {
                Box::pin(runtime.finish_effect_in_session(
                    db,
                    session,
                    operation,
                    AuditStage::Uncertain,
                ))
            },
        )
        .await
    }

    /// No lease expiry or retrieved claim authorizes resend. Any ambiguous
    /// attempt stays durably reserved and becomes uncertain on recovery.
    pub async fn authorize_effect(
        &self,
        db: &Database,
        operation: &str,
    ) -> AppResult<DispatchPermit> {
        let context = self.effects.get().ok_or(AppError::AuditNotInitialized)?;
        let prior = db
            .collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": operation})
            .await?
            .ok_or(AppError::AuditIntegrityConflict)?;
        if prior.terminal_stage.is_some() {
            return Err(deferred());
        }
        if prior.phase != EffectPhase::Pending {
            // Recording a recovered outcome consumes reserved capacity and must
            // work while the witness is down; it never authorizes an effect.
            self.uncertain_effect(db, operation).await?;
            return Err(deferred());
        }
        let relay = context.relay.as_ref().and_then(Weak::upgrade);
        if self.configured && relay.is_none() {
            return Err(deferred());
        }
        if let Some(relay) = &relay {
            relay.refresh_for_claim().await?;
        }
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(audit_state_service::majority())
            .await?;
        let (state, now) =
            audit_state_service::writer_snapshot(db, &mut session, context.key.as_ref().as_ref())
                .await?;
        let intents = db.collection::<AuditEffectIntent>(INTENTS);
        let intent = intents
            .find_one(doc! {"_id": operation})
            .session(&mut session)
            .await?
            .ok_or(AppError::AuditIntegrityConflict)?;
        if intent.terminal_stage.is_some() {
            return Err(deferred());
        }
        if intent.phase != EffectPhase::Pending {
            // Conservatively terminalize recovered/duplicate attempts. A paused
            // original may still run; reconciliation must account for that.
            session.abort_transaction().await?;
            self.uncertain_effect(db, operation).await?;
            return Err(deferred());
        }
        let evidence = self.diagnostics(Some(&state), now);
        if evidence.admission.effect == super::policy::Admission::Closed {
            return Err(if evidence.divergence.is_some() {
                AppError::AuditWitnessDiverged
            } else {
                deferred()
            });
        }
        let pending = db
            .collection::<PendingAuditEvent>(PENDING)
            .find_one(doc! {"_id": &intent.event_id})
            .session(&mut session)
            .await?
            .ok_or(AppError::AuditIntegrityConflict)?;
        let seq = pending.materialized_seq.ok_or_else(deferred)?;
        let row = db
            .collection::<crate::models::audit_log::AuditLog>(
                crate::models::audit_log::COLLECTION_NAME,
            )
            .find_one(doc! {"seq": seq})
            .session(&mut session)
            .await?
            .ok_or(AppError::AuditIntegrityConflict)?;
        let hash = crate::services::audit_chain_service::compute_entry_hash(
            &row,
            context.key.as_ref().as_ref(),
        )?;
        if row.entry_hash.as_deref() != Some(&hash)
            || row.id != intent.event_id
            || pending.event != intent.event
            || row.event_data
                != Some(
                    serde_json::to_value(&intent.event)
                        .map_err(|_| AppError::AuditIntegrityConflict)?,
                )
        {
            return Err(AppError::AuditIntegrityConflict);
        }
        let configured = self.configured || state.witness_enrolled;
        if configured && (relay.is_none() || state.witnessed_seq < seq) {
            return Err(deferred());
        }
        let previous = if configured {
            let progress = db
                .collection::<WitnessDelivery>(DELIVERY)
                .find_one(doc! {"_id": &state.id})
                .session(&mut session)
                .await?
                .ok_or_else(deferred)?;
            if progress.divergence.is_some() {
                return Err(AppError::AuditWitnessDiverged);
            }
            nyxid::services::audit_witness::receipt_digest(&progress.receipt)
        } else {
            nyxid::services::audit_witness::ZERO_HASH.to_owned()
        };
        let request = DispatchRequest {
            chain_id: Uuid::parse_str(&state.id).map_err(|_| AppError::AuditIntegrityConflict)?,
            operation_id: Uuid::parse_str(operation)
                .map_err(|_| AppError::AuditIntegrityConflict)?,
            attempt_id: Uuid::new_v4(),
            process_id: context.process,
            intent_seq: u64::try_from(seq).map_err(|_| AppError::AuditIntegrityConflict)?,
            intent_hash: hash,
            previous_receipt: previous,
        };
        let updated = intents.update_one(doc! {"_id": operation, "phase": "pending", "terminal_stage": bson::Bson::Null},
            doc! {"$set": {"phase": "claim_requested", "attempt": bson::to_bson(&request).map_err(|_| AppError::AuditIntegrityConflict)?}}).session(&mut session).await?;
        if updated.matched_count != 1 {
            return Err(deferred());
        }
        audit_state_service::commit(&mut session).await?;
        if !context
            .issued
            .lock()
            .expect("effect permits poisoned")
            .insert(operation.to_owned())
        {
            return Err(deferred());
        }
        if let Some(relay) = &relay {
            // Failure may mean the protected claim was accepted. Keep the
            // ClaimRequested marker; a later attempt records uncertain.
            let receipt = relay.claim_dispatch(request.clone()).await?;
            if receipt.body.event
                != (WitnessEvent::Dispatch {
                    request: request.clone(),
                })
            {
                return Err(AppError::AuditWitnessDiverged);
            }
        }
        let claimed = intents.update_one(doc! {"_id": operation, "phase": "claim_requested", "terminal_stage": bson::Bson::Null,
            "attempt.attempt_id": bson::to_bson(&request.attempt_id).map_err(|_| AppError::AuditIntegrityConflict)?},
            doc! {"$set": {"phase": "claimed"}}).write_concern(audit_state_service::majority()).await?;
        if claimed.matched_count != 1 {
            return Err(deferred());
        }
        Ok(DispatchPermit {
            operation: operation.to_owned(),
            process: context.process,
            attempt: request.attempt_id,
            relay,
            db: db.clone(),
        })
    }
}
