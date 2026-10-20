//! One committed product send, with an independently resolved binding and a
//! receipt/terminal transaction. No auto-discovery, detached send or retry loop
//! may turn a loaded envelope or recovered claim into new dispatch authority.
use async_trait::async_trait;
use mongodb::{ClientSession, Database, bson::doc};
use uuid::Uuid;

use super::{
    Channel, DeliveryBinding, DeliveryOutcome, QueuedDelivery, SendOutcome, check_binding, load,
};
use crate::{
    errors::{AppError, AppResult},
    models::audit_outbox::{AuditDetail, AuditStage},
    services::audit_runtime::{AuditRuntime, transaction},
};

/// Contains protected product data in memory, never in the audit projection.
/// The resolver derives binding from its immutable version/destination store,
/// not by copying the supplied queue envelope.
pub struct ResolvedDelivery<P> {
    pub binding: DeliveryBinding,
    pub channel: Channel,
    pub prepared: P,
}

/// Implemented by the owning producer/sink. Resolution enforces live owner and
/// revision fences without effects. Sending does one provider operation inside
/// the consumed permit; no detach, fanout or ambiguous-error retry is allowed.
#[async_trait]
pub trait DeliveryConsumer: Sync {
    type Prepared: Send;
    type Receipt: Send + Sync;

    async fn resolve(
        &self,
        db: &Database,
        queued: &QueuedDelivery,
    ) -> AppResult<ResolvedDelivery<Self::Prepared>>;

    async fn send(
        &self,
        queued: QueuedDelivery,
        prepared: Self::Prepared,
    ) -> SendOutcome<Self::Receipt>;

    /// DB-only, bounded-runner callback. Persist/fence the protected receipt and
    /// product outcome by stable operation ID, accepting only identical replay.
    /// Never derive a new destination, issue a network call or mint an identity.
    async fn record_outcome_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        queued: &QueuedDelivery,
        outcome: &SendOutcome<Self::Receipt>,
    ) -> AppResult<()>;

    /// Closed post-dispatch evidence only. The core revalidates permitted kinds,
    /// stage, numeric bounds and exact replay. No arbitrary provider strings.
    fn terminal_detail(
        &self,
        outcome: &SendOutcome<Self::Receipt>,
    ) -> AppResult<Option<AuditDetail>>;
}

/// Invoke after confirmed enqueue commit and materialization, using the serving
/// runtime. A configured stale witness defers before sending. A recovered or
/// terminal claim never resends; reconciliation belongs to the product owner.
pub async fn dispatch_one<C: DeliveryConsumer>(
    db: &Database,
    runtime: &AuditRuntime,
    operation: Uuid,
    consumer: &C,
) -> AppResult<SendOutcome<C::Receipt>> {
    let queued = load(db, operation).await?;
    let resolved = consumer.resolve(db, &queued).await?;
    // Refuse a bad resolver before obtaining a durable dispatch claim.
    check_binding(&queued, &resolved.binding, resolved.channel)?;
    let permit = queued.clone().authorize(db, runtime).await?;
    let outcome = permit
        .run_bound(&resolved.binding, resolved.channel, |queued| async move {
            consumer.send(queued, resolved.prepared).await
        })
        .await?;
    commit_outcome(db, runtime, &queued, consumer, &outcome).await?;
    Ok(outcome)
}

/// Commit a known provider result, or reviewed reconciliation evidence, without
/// another send. If the DB result is unresolved, retain the durable claim for
/// product reconciliation; never call dispatch_one to retry a lost acknowledgment.
/// Unknown-commit retries stay on the original Mongo transaction.
pub async fn commit_outcome<C: DeliveryConsumer>(
    db: &Database,
    runtime: &AuditRuntime,
    queued: &QueuedDelivery,
    consumer: &C,
    outcome: &SendOutcome<C::Receipt>,
) -> AppResult<()> {
    // Freeze evidence once, outside all DB callback retries. It cannot depend
    // on mutable configuration or replace an existing intent policy detail.
    let detail = consumer.terminal_detail(outcome)?;
    let operation = Uuid::parse_str(queued.operation_id())
        .map_err(|_| crate::errors::AppError::AuditIntegrityConflict)?;
    transaction::run(
        db,
        &(db, runtime, queued, consumer, outcome, detail, operation),
        |session, (db, runtime, queued, consumer, outcome, detail, operation)| {
            Box::pin(async move {
                let current = db
                    .collection::<QueuedDelivery>(super::COLLECTION_NAME)
                    .find_one(doc! {"_id": operation.to_string()})
                    .session(&mut *session)
                    .await?
                    .ok_or(AppError::AuditIntegrityConflict)?;
                current.validate()?;
                if current != **queued {
                    return Err(AppError::AuditIntegrityConflict);
                }
                consumer
                    .record_outcome_in_session(db, session, queued, outcome)
                    .await?;
                let stage = match outcome.delivery_outcome() {
                    DeliveryOutcome::Succeeded => AuditStage::Succeeded,
                    DeliveryOutcome::DefinitiveFailure => AuditStage::Failed,
                    DeliveryOutcome::Cancelled => AuditStage::Cancelled,
                    DeliveryOutcome::Uncertain => AuditStage::Uncertain,
                };
                runtime
                    .finish_effect_with_detail_in_session(
                        db,
                        session,
                        queued.operation_id(),
                        stage,
                        detail.clone(),
                    )
                    .await
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests;
