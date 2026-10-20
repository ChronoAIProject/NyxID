//! Local chain allocation is always transactional, including without a witness.
//! Only explicit startup adoption may initialize the head; append never infers
//! a replacement head from surviving rows.
use chrono::{DateTime, Utc};
use mongodb::{ClientSession, Database, bson::doc, options::IndexOptions};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG},
        audit_outbox::{
            AuditOutbox, COLLECTION_NAME as OUTBOX, PENDING_COLLECTION_NAME as PENDING,
            PendingAuditEvent,
        },
        audit_state::{AuditState, COLLECTION_NAME as STATE},
    },
    services::{audit_chain_service as chain, audit_runtime::AuditConfig},
};

pub const PROTOCOL: u32 = 1;

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    // Required before any appender, including pre-AppState startup writers.
    db.collection::<AuditLog>(AUDIT_LOG)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"seq": 1})
                .options(
                    IndexOptions::builder()
                        .name("audit_log_seq_unique".to_owned())
                        .unique(true)
                        .partial_filter_expression(doc! {"seq": {"$exists": true}})
                        .build(),
                )
                .build(),
        )
        .await?;
    for (collection, keys, name) in [
        (STATE, doc! {"singleton": 1}, "audit_single_head"),
        (
            OUTBOX,
            doc! {"chain_id": 1, "order": 1},
            "audit_outbox_order",
        ),
        (OUTBOX, doc! {"chain_id": 1, "seq": 1}, "audit_outbox_seq"),
    ] {
        db.collection::<mongodb::bson::Document>(collection)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .unique(true)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    db.collection::<PendingAuditEvent>(PENDING)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"event.operation_id": 1, "event.stage": 1})
                .options(
                    IndexOptions::builder()
                        .name("audit_operation_stage".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<PendingAuditEvent>(PENDING)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"materialized_seq": 1, "order": 1})
                .build(),
        )
        .await?;
    db.collection::<AuditOutbox>(OUTBOX)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"witnessed_at": 1, "order": 1})
                .build(),
        )
        .await?;
    db.collection::<bson::Document>(crate::models::audit_effect_intent::COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"terminal_stage": 1, "terminal_order": 1})
                .build(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
pub async fn load(db: &Database) -> AppResult<AuditState> {
    db.collection::<AuditState>(STATE)
        .find_one(doc! {"singleton": true})
        .await?
        .ok_or(AppError::AuditNotInitialized)
}

/// No mutation is performed before the entire old chain has been validated.
/// Old writers must be drained by the operator: an old binary cannot be made
/// to honor a new Mongo head/CAS protocol by publishing a heartbeat.
pub async fn initialize(db: &Database, key: &[u8], config: &AuditConfig) -> AppResult<AuditState> {
    if let Some(witness) = &config.witness {
        witness
            .pin
            .validate()
            .map_err(|_| AppError::AuditIntegrityConflict)?;
        if config
            .chain_id
            .is_some_and(|id| id != witness.pin.enrollment.body.chain_id)
        {
            return Err(AppError::AuditIntegrityConflict);
        }
    }
    ensure_indexes(db).await?;
    for attempt in 0..8 {
        match initialize_once(db, key, config).await {
            Err(AppError::DatabaseError(error))
                if attempt < 7
                    && (error.contains_label("TransientTransactionError")
                        || duplicate_key(&error)) =>
            {
                tokio::task::yield_now().await;
            }
            result => return result,
        }
    }
    unreachable!("last initialization attempt returns")
}

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(
        mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
}

async fn initialize_once(db: &Database, key: &[u8], config: &AuditConfig) -> AppResult<AuditState> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(majority())
        .await?;
    if let Some(mut state) = db
        .collection::<AuditState>(STATE)
        .find_one(doc! {"singleton": true})
        .session(&mut session)
        .await?
    {
        validate_identity(&state, config)?;
        validate_tail(db, &mut session, key, &state).await?;
        if config.witness.is_some() && !state.witness_enrolled {
            // The protected pin proves enrollment, not live contact. Persist
            // policy activation before serving, even if the first observation
            // is unreachable. Removing settings must not downgrade admission.
            db.collection::<AuditState>(STATE)
                .update_one(
                    doc! {"_id": &state.id},
                    doc! {"$set": {"witness_enrolled": true}},
                )
                .session(&mut session)
                .await?;
            commit(&mut session).await?;
            state.witness_enrolled = true;
        } else {
            session.abort_transaction().await?;
        }
        return Ok(state);
    }
    let tail = db
        .collection::<AuditLog>(AUDIT_LOG)
        .find_one(doc! {"seq": {"$exists": true}})
        .sort(doc! {"seq": -1})
        .await?;
    if tail.is_some() && !config.cutover_drained {
        return Err(AppError::AuditCutoverRequired);
    }
    if tail.is_some() {
        super::security_readiness_service::require_audit_cutover(db, config.cutover_drained)
            .await?;
    }
    let mut from = None;
    loop {
        let report = chain::verify_chain(db, key, from, None, None).await?;
        if report.break_info.is_some() {
            return Err(AppError::AuditIntegrityConflict);
        }
        match report.next_from_seq {
            Some(next) => from = Some(next),
            None => break,
        }
    }
    let (seq, hash) = match tail {
        Some(entry) => (
            entry.seq.ok_or(AppError::AuditIntegrityConflict)?,
            entry.entry_hash.ok_or(AppError::AuditIntegrityConflict)?,
        ),
        None => (0, chain::GENESIS_PREV_HASH.to_owned()),
    };
    let state = AuditState {
        id: config
            .chain_id
            .or_else(|| {
                config
                    .witness
                    .as_ref()
                    .map(|w| w.pin.enrollment.body.chain_id)
            })
            .unwrap_or_else(Uuid::new_v4)
            .to_string(),
        singleton: true,
        protocol: PROTOCOL,
        key_version: 1,
        head_seq: seq,
        head_hash: hash,
        next_order: 0,
        pending_count: 0,
        witnessed_seq: 0,
        witnessed_hash: chain::GENESIS_PREV_HASH.to_owned(),
        witness_enrolled: config.witness.is_some(),
        oldest_pending_at: None,
        adopted_at: Utc::now(),
        adopted_v1_seq: seq,
    };
    // Detect a writer racing the inspection. Drain remains mandatory because
    // old writers do not update the state and cannot be fenced by this CAS.
    let current = db
        .collection::<AuditLog>(AUDIT_LOG)
        .find_one(doc! {"seq": {"$exists": true}})
        .sort(doc! {"seq": -1})
        .session(&mut session)
        .await?;
    if current.as_ref().and_then(|e| e.seq).unwrap_or(0) != seq
        || current
            .as_ref()
            .and_then(|e| e.entry_hash.as_deref())
            .unwrap_or(chain::GENESIS_PREV_HASH)
            != state.head_hash
    {
        return Err(AppError::AuditIntegrityConflict);
    }
    db.collection::<AuditState>(STATE)
        .insert_one(&state)
        .session(&mut session)
        .await?;
    commit(&mut session).await?;
    Ok(state)
}

fn validate_identity(state: &AuditState, config: &AuditConfig) -> AppResult<()> {
    if state.pending_count < 0
        || state.next_order < state.pending_count
        || (state.pending_count == 0) != state.oldest_pending_at.is_none()
        || state.head_seq < 0
        || state.witnessed_seq < 0
        || state.witnessed_seq > state.head_seq
        || state.protocol != PROTOCOL
        || state.key_version != 1
        || config.chain_id.is_some_and(|id| id.to_string() != state.id)
        || config
            .witness
            .as_ref()
            .is_some_and(|w| w.pin.enrollment.body.chain_id.to_string() != state.id)
    {
        return Err(AppError::AuditIntegrityConflict);
    }
    Ok(())
}

async fn validate_tail(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
    state: &AuditState,
) -> AppResult<()> {
    let tail = db
        .collection::<AuditLog>(AUDIT_LOG)
        .find_one(doc! {"seq": {"$exists": true}})
        .sort(doc! {"seq": -1})
        .session(session)
        .await?;
    match tail {
        Some(tail)
            if tail.seq == Some(state.head_seq)
                && tail.entry_hash.as_deref() == Some(&state.head_hash)
                && chain::compute_entry_hash(&tail, key)? == state.head_hash =>
        {
            Ok(())
        }
        None if state.head_seq == 0 && state.head_hash == chain::GENESIS_PREV_HASH => Ok(()),
        _ => Err(AppError::AuditIntegrityConflict),
    }
}

/// Caller owns the transaction and its retry/commit outcome. Returning an error
/// never falls back to an independent insert or a new transaction.
pub async fn append_in_session(
    db: &Database,
    session: &mut ClientSession,
    entry: AuditLog,
    key: &[u8],
) -> AppResult<AuditLog> {
    append(db, session, entry, key, None).await
}

/// Reservation and replay use the same local boundary checks as direct append.
/// This does not establish external continuity or configured-witness admission.
pub(super) async fn writer_snapshot(
    db: &Database,
    session: &mut ClientSession,
    key: &[u8],
) -> AppResult<(AuditState, DateTime<Utc>)> {
    let (state, now) = super::audit_runtime::load_in_session(db, session).await?;
    let state = state.ok_or(AppError::AuditNotInitialized)?;
    validate_identity(&state, &AuditConfig::default())?;
    validate_tail(db, session, key, &state).await?;
    Ok((state, now))
}

pub(super) async fn append_reserved_in_session(
    db: &Database,
    session: &mut ClientSession,
    entry: AuditLog,
    key: &[u8],
    reservation: &PendingAuditEvent,
) -> AppResult<AuditLog> {
    append(db, session, entry, key, Some(reservation)).await
}

async fn append(
    db: &Database,
    session: &mut ClientSession,
    mut entry: AuditLog,
    key: &[u8],
    reservation: Option<&PendingAuditEvent>,
) -> AppResult<AuditLog> {
    let entries = db.collection::<AuditLog>(AUDIT_LOG);
    // Replay must validate the durable boundary too: an identical old UUID is
    // not permission to accept a rolled-back head or an old-writer tail.
    let (state, database_now) = writer_snapshot(db, session, key).await?;
    if let Some(existing) = entries
        .find_one(doc! {"_id": &entry.id})
        .session(&mut *session)
        .await?
    {
        if chain::immutable_entry_matches(&existing, &entry)
            && existing.entry_hash.as_ref().is_some_and(|hash| {
                chain::compute_entry_hash(&existing, key).is_ok_and(|actual| &actual == hash)
            })
        {
            let delivery = db
                .collection::<AuditOutbox>(OUTBOX)
                .find_one(doc! {"_id": &existing.id})
                .session(&mut *session)
                .await?
                .ok_or(AppError::AuditIntegrityConflict)?;
            if delivery.chain_id != state.id
                || delivery.key_version != state.key_version
                || Some(delivery.seq) != existing.seq
                || Some(&delivery.entry_hash) != existing.entry_hash.as_ref()
                || Some(&delivery.prev_hash) != existing.prev_hash.as_ref()
                || reservation.is_some_and(|r| r.id != delivery.id || r.order != delivery.order)
            {
                return Err(AppError::AuditIntegrityConflict);
            }
            return Ok(existing);
        }
        return Err(AppError::AuditIntegrityConflict);
    }
    let seq = state
        .head_seq
        .checked_add(1)
        .ok_or(AppError::AuditIntegrityConflict)?;
    let (order, next_order, pending, reserved_at) = match reservation {
        Some(reserved)
            if reserved.id == entry.id
                && reserved.order > 0
                && reserved.order <= state.next_order
                && state.pending_count > 0 =>
        {
            (
                reserved.order,
                state.next_order,
                state.pending_count,
                reserved.reserved_at,
            )
        }
        Some(_) => return Err(AppError::AuditIntegrityConflict),
        None => {
            let order = state
                .next_order
                .checked_add(1)
                .ok_or(AppError::AuditIntegrityConflict)?;
            (
                order,
                order,
                state
                    .pending_count
                    .checked_add(1)
                    .ok_or(AppError::AuditIntegrityConflict)?,
                database_now,
            )
        }
    };
    chain::prepare_chained_entry(&mut entry, seq, state.head_hash.clone(), key)?;
    let hash = entry
        .entry_hash
        .clone()
        .ok_or(AppError::AuditIntegrityConflict)?;
    let changed = db.collection::<AuditState>(STATE).update_one(
        doc! {"_id": &state.id, "protocol": PROTOCOL, "head_seq": state.head_seq,
            "head_hash": &state.head_hash, "next_order": state.next_order},
        doc! {"$set": {"head_seq": seq, "head_hash": &hash, "next_order": next_order,
            "pending_count": pending, "oldest_pending_at": bson::DateTime::from_chrono(state.oldest_pending_at.unwrap_or(reserved_at))}}
    ).session(&mut *session).await?;
    if changed.matched_count != 1 {
        return Err(AppError::AuditIntegrityConflict);
    }
    entries.insert_one(&entry).session(&mut *session).await?;
    db.collection::<AuditOutbox>(OUTBOX)
        .insert_one(AuditOutbox {
            id: entry.id.clone(),
            chain_id: state.id,
            order,
            key_version: state.key_version,
            seq,
            prev_hash: entry
                .prev_hash
                .clone()
                .ok_or(AppError::AuditIntegrityConflict)?,
            entry_hash: hash,
            reserved_at,
            witnessed_at: None,
        })
        .session(&mut *session)
        .await?;
    Ok(entry)
}

pub fn majority() -> mongodb::options::WriteConcern {
    mongodb::options::WriteConcern::builder()
        .w(mongodb::options::Acknowledgment::Majority)
        .journal(true)
        .build()
}

/// Retrying the same commit cannot allocate another event on an unknown result.
pub async fn commit(session: &mut ClientSession) -> mongodb::error::Result<()> {
    super::audit_runtime::transaction::commit(session).await
}

#[cfg(test)]
#[path = "audit_state_tests.rs"]
pub(super) mod tests;
