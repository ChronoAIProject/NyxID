//! Transactional safe-event queue. Event identity is allocated outside a caller's
//! retry loop; operation/stage is the logical replay identity across processes.
use mongodb::{ClientSession, Database, bson::doc};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        audit_log::AuditLog,
        audit_outbox::{
            AuditOutbox, AuditOwner, AuditPrincipal, AuditStage, COLLECTION_NAME as OUTBOX,
            PENDING_COLLECTION_NAME as PENDING, PendingAuditEvent, SafeAuditEvent,
        },
        audit_state::{AuditState, COLLECTION_NAME as STATE},
    },
};

/// Caller projection accepts only a reviewed static action and typed identities.
/// Stored Mongo models are deliberately not accepted as enqueue input.
#[derive(Clone)]
pub struct AuditEvent {
    pub operation_id: String,
    pub stage: AuditStage,
    pub principal: AuditPrincipal,
    pub owner: AuditOwner,
    pub action: &'static str,
    pub target_id: Option<String>,
}

impl AuditEvent {
    pub(super) fn safe_projection(&self) -> SafeAuditEvent {
        SafeAuditEvent {
            operation_id: self.operation_id.clone(),
            stage: self.stage.clone(),
            principal: self.principal.clone(),
            owner: self.owner.clone(),
            action: self.action.to_owned(),
            target_id: self.target_id.clone(),
        }
    }
}

fn uuid(value: &str) -> AppResult<()> {
    if Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 4) {
        Ok(())
    } else {
        Err(AppError::ValidationError(
            "Audit identity must be a UUID v4".into(),
        ))
    }
}

pub(super) fn validate(event: &SafeAuditEvent) -> AppResult<()> {
    uuid(&event.operation_id)?;
    match &event.principal {
        AuditPrincipal::Person { id }
        | AuditPrincipal::Node { id }
        | AuditPrincipal::ServiceAccount { id } => uuid(id)?,
        AuditPrincipal::ApiKey { id, owner_id } => {
            uuid(id)?;
            uuid(owner_id)?;
        }
        AuditPrincipal::OracleWorker { id } => {
            let (pool, label) = id.split_once(':').ok_or_else(|| {
                AppError::ValidationError(
                    "Audit Oracle identity must be pool UUID:worker label".into(),
                )
            })?;
            uuid(pool)?;
            // Same closed alphabet/limit as the authenticated Oracle protocol.
            if label.is_empty()
                || label.len() > 64
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                || Uuid::parse_str(pool).is_ok_and(|value| value.to_string() != pool)
            {
                return Err(AppError::ValidationError(
                    "Invalid audit Oracle worker identity".into(),
                ));
            }
        }
        AuditPrincipal::System { .. } => (),
    }
    match &event.owner {
        AuditOwner::Person { id } | AuditOwner::Org { id } => uuid(id)?,
        AuditOwner::Platform => (),
    }
    if let Some(target) = &event.target_id {
        uuid(target)?;
    }
    if event.action.is_empty()
        || event.action.len() > 96
        || !event
            .action
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(AppError::ValidationError(
            "Audit action must be a bounded identifier".into(),
        ));
    }
    Ok(())
}

/// The caller must commit this session with the business mutation. A count query
/// followed by an independent insert is never used as a capacity reservation.
pub async fn enqueue_in_session(
    db: &Database,
    session: &mut ClientSession,
    event_id: &str,
    event: &AuditEvent,
) -> AppResult<String> {
    enqueue(db, session, event_id, event, None).await
}

/// Preferred T/O entrypoint: admission and the actual capacity reservation are
/// in the caller's business transaction. Identical replays need no new slot.
pub async fn enqueue_admitted_in_session(
    db: &Database,
    session: &mut ClientSession,
    runtime: &super::audit_runtime::AuditRuntime,
    tier: super::audit_runtime::policy::AuditTier,
    event_id: &Uuid,
    event: &AuditEvent,
) -> AppResult<String> {
    enqueue(
        db,
        session,
        &event_id.to_string(),
        event,
        Some((runtime, tier)),
    )
    .await
}

async fn enqueue(
    db: &Database,
    session: &mut ClientSession,
    event_id: &str,
    event: &AuditEvent,
    admission: Option<(
        &super::audit_runtime::AuditRuntime,
        super::audit_runtime::policy::AuditTier,
    )>,
) -> AppResult<String> {
    let event = event.safe_projection();
    uuid(event_id)?;
    validate(&event)?;
    // A replay is not permission to bypass a missing or rolled-back allocator.
    let key = super::audit_service::audit_chain_hmac_key().ok_or(AppError::AuditNotInitialized)?;
    let (state, now) = super::audit_state_service::writer_snapshot(db, session, key).await?;
    let filter = doc! {"event.operation_id": &event.operation_id,
    "event.stage": bson::to_bson(&event.stage).map_err(|_| AppError::AuditIntegrityConflict)?};
    let pending = db.collection::<PendingAuditEvent>(PENDING);
    if let Some(existing) = pending.find_one(filter).session(&mut *session).await? {
        if existing.event != event
            || existing.order <= 0
            || existing.order > state.next_order
            || existing
                .materialized_seq
                .is_some_and(|seq| seq > state.head_seq)
            || (existing.materialized_seq.is_none() && state.pending_count <= 0)
        {
            return Err(AppError::AuditIntegrityConflict);
        }
        if let Some(seq) = existing.materialized_seq {
            let delivery = db
                .collection::<AuditOutbox>(OUTBOX)
                .find_one(doc! {"_id": &existing.id})
                .session(&mut *session)
                .await?
                .ok_or(AppError::AuditIntegrityConflict)?;
            if delivery.chain_id != state.id
                || delivery.order != existing.order
                || delivery.seq != seq
                || delivery.key_version != state.key_version
                || (seq == state.head_seq && delivery.entry_hash != state.head_hash)
                || (delivery.witnessed_at.is_none() && state.pending_count <= 0)
            {
                return Err(AppError::AuditIntegrityConflict);
            }
        }
        return Ok(existing.id);
    }
    if let Some((runtime, tier)) = admission {
        runtime.require_new_in_session(db, session, tier, 1).await?;
    }
    let order = state
        .next_order
        .checked_add(1)
        .ok_or(AppError::AuditIntegrityConflict)?;
    let count = state
        .pending_count
        .checked_add(1)
        .ok_or(AppError::AuditIntegrityConflict)?;
    let updated = db.collection::<AuditState>(STATE).update_one(
        doc! {"_id": &state.id, "next_order": state.next_order, "pending_count": state.pending_count},
        doc! {"$set": {"next_order": order, "pending_count": count,
            "oldest_pending_at": bson::DateTime::from_chrono(state.oldest_pending_at.unwrap_or(now))}}
    ).session(&mut *session).await?;
    if updated.matched_count != 1 {
        return Err(AppError::AuditIntegrityConflict);
    }
    pending
        .insert_one(PendingAuditEvent {
            id: event_id.to_owned(),
            event,
            order,
            reserved_at: now,
            materialized_seq: None,
        })
        .session(&mut *session)
        .await?;
    Ok(event_id.to_owned())
}

fn audit_entry(pending: &PendingAuditEvent) -> AppResult<AuditLog> {
    // Revalidate storage input as well as producer input before materialization.
    validate(&pending.event)?;
    Ok(AuditLog {
        id: pending.id.clone(),
        user_id: match &pending.event.principal {
            AuditPrincipal::Person { id } => Some(id.clone()),
            _ => None,
        },
        event_type: pending.event.action.clone(),
        event_data: Some(
            serde_json::to_value(&pending.event).map_err(|_| AppError::AuditIntegrityConflict)?,
        ),
        ip_address: None,
        user_agent: None,
        api_key_id: match &pending.event.principal {
            AuditPrincipal::ApiKey { id, .. } => Some(id.clone()),
            _ => None,
        },
        api_key_name: None,
        seq: None,
        prev_hash: None,
        entry_hash: None,
        created_at: pending.reserved_at,
    })
}

/// Every process can help; head/pending-row transaction conflicts fence duplicate
/// materialization. The queue survives a task/process crash without a lease.
pub fn spawn_materializer(db: Database, key: Arc<zeroize::Zeroizing<[u8; 32]>>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            for _ in 0..128 {
                match materialize_one(&db, key.as_ref().as_ref()).await {
                    Ok(true) => tokio::task::yield_now().await,
                    Ok(false) => break,
                    Err(_) => {
                        tracing::error!(
                            "Durable audit materialization failed; pending evidence retained"
                        );
                        break;
                    }
                }
            }
        }
    });
}

/// Bounded local materialization. Its CAS and pending-row marker are in the same
/// transaction as the audit row; none of these writes release witness capacity.
pub async fn materialize_one(db: &Database, key: &[u8]) -> AppResult<bool> {
    super::audit_runtime::transaction::run(db, &(db, key), |session, (db, key)| {
        Box::pin(materialize_in_session(db, session, key))
    })
    .await
}

#[cfg(test)]
#[path = "audit_outbox_tests.rs"]
mod tests;

async fn materialize_in_session(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
) -> AppResult<bool> {
    let pending = db.collection::<PendingAuditEvent>(PENDING);
    let Some(event) = pending
        .find_one(doc! {"materialized_seq": bson::Bson::Null})
        .sort(doc! {"order": 1})
        .session(&mut *session)
        .await?
    else {
        return Ok(false);
    };
    let inserted = super::audit_state_service::append_reserved_in_session(
        db,
        session,
        audit_entry(&event)?,
        key,
        &event,
    )
    .await?;
    let marked = pending.update_one(doc! {"_id": &event.id, "materialized_seq": bson::Bson::Null},
        doc! {"$set": {"materialized_seq": inserted.seq.ok_or(AppError::AuditIntegrityConflict)?}})
        .session(session).await?;
    if marked.matched_count != 1 {
        return Err(AppError::AuditIntegrityConflict);
    }
    Ok(true)
}
