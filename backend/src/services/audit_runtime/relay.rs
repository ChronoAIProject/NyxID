//! Conditional delivery is persisted before the network call. A verified live
//! observation, authenticated history and a local transaction acknowledge it.
use std::{collections::HashMap, sync::Arc, time::Duration};

use mongodb::{ClientSession, Database, bson::doc};
use nyxid::{
    models::audit_witness::{
        AdvanceRequest, AuditLink, DispatchRequest, SignedReceipt, WitnessEvent,
    },
    services::audit_witness::{MAX_LINKS, WitnessError, ZERO_HASH, receipt_digest, verify_receipt},
};
use tokio::sync::Mutex;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{AuditRuntime, ConfiguredWitness};
use crate::{
    errors::{AppError, AppResult},
    models::{
        audit_log::{AuditLog, COLLECTION_NAME as LOG},
        audit_outbox::{
            AuditOutbox, COLLECTION_NAME as OUTBOX, PENDING_COLLECTION_NAME as PENDING,
            PendingAuditEvent,
        },
        audit_state::{AuditState, COLLECTION_NAME as STATE},
        audit_witness_delivery::{COLLECTION_NAME as DELIVERY, WitnessDelivery, WitnessDivergence},
    },
    services::{audit_chain_service, audit_state_service},
};

pub struct WitnessRelay {
    db: Database,
    runtime: Arc<AuditRuntime>,
    witness: ConfiguredWitness,
    key: Arc<Zeroizing<[u8; 32]>>,
    cursor: Mutex<Cursor>,
}

struct Cursor {
    latest: SignedReceipt,
    digests: HashMap<u64, String>,
}

enum Failure {
    Local(AppError),
    Remote(WitnessError),
    Diverged(WitnessDivergence),
}
impl From<AppError> for Failure {
    fn from(e: AppError) -> Self {
        Self::Local(e)
    }
}
impl From<mongodb::error::Error> for Failure {
    fn from(e: mongodb::error::Error) -> Self {
        Self::Local(e.into())
    }
}
impl From<WitnessError> for Failure {
    fn from(e: WitnessError) -> Self {
        Self::Remote(e)
    }
}
type Result<T> = std::result::Result<T, Failure>;

fn conflict(seq: u64, local: &str, witnessed: &str) -> Failure {
    Failure::Diverged(WitnessDivergence {
        seq: i64::try_from(seq).unwrap_or(i64::MAX),
        local_hash: local.to_owned(),
        witnessed_hash: witnessed.to_owned(),
    })
}

impl WitnessRelay {
    pub fn new(
        db: Database,
        runtime: Arc<AuditRuntime>,
        witness: ConfiguredWitness,
        key: Arc<Zeroizing<[u8; 32]>>,
    ) -> Self {
        let initial = witness.pin.enrollment.clone();
        Self {
            db,
            runtime,
            witness,
            key,
            cursor: Mutex::new(Cursor {
                digests: HashMap::from([(0, receipt_digest(&initial))]),
                latest: initial,
            }),
        }
    }

    pub fn spawn(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                // The monitor owns rate-limited categorical transition alarms.
                // Transport details, endpoint, credentials and bodies stay out.
                let _ = self.tick().await;
            }
        });
    }

    /// No network operation occurs in startup. An unreachable configured sink
    /// leaves contact unknown and the persisted pending-age clock unchanged.
    pub async fn tick(&self) -> AppResult<bool> {
        self.run_tick(true).await
    }

    pub(super) async fn refresh_for_claim(&self) -> AppResult<()> {
        self.run_tick(false).await.map(|_| ())
    }

    async fn run_tick(&self, deliver: bool) -> AppResult<bool> {
        let mut cursor = self.cursor.lock().await;
        match self.tick_inner(&mut cursor, deliver).await {
            Ok(value) => Ok(value),
            Err(failure) => self.failed(failure, &cursor).await,
        }
    }

    async fn failed<T>(&self, failure: Failure, cursor: &Cursor) -> AppResult<T> {
        match failure {
            Failure::Diverged(detail) => {
                self.quarantine(detail).await?;
                Err(AppError::AuditWitnessDiverged)
            }
            Failure::Remote(WitnessError::Corrupt)
            | Failure::Local(AppError::AuditIntegrityConflict | AppError::AuditWitnessDiverged) => {
                self.quarantine(WitnessDivergence {
                    seq: cursor.latest.body.head.seq as i64,
                    local_hash: cursor.latest.body.head.entry_hash.clone(),
                    witnessed_hash: String::new(),
                })
                .await?;
                Err(AppError::AuditWitnessDiverged)
            }
            Failure::Local(error) => Err(error),
            Failure::Remote(_) => Err(AppError::AuditAdmissionClosed {
                reason: crate::errors::AuditAdmissionReason::WitnessUnavailable,
            }),
        }
    }

    async fn quarantine(&self, detail: WitnessDivergence) -> AppResult<()> {
        self.runtime.mark_divergence(&detail);
        let mut session = self.db.client().start_session().await?;
        session
            .start_transaction()
            .write_concern(audit_state_service::majority())
            .await?;
        let progress = self.db.collection::<WitnessDelivery>(DELIVERY);
        if let Some(saved) = progress
            .find_one(doc! {"_id": self.chain()})
            .session(&mut session)
            .await?
        {
            if saved.divergence.is_none() {
                progress.update_one(doc! {"_id": self.chain(), "divergence": bson::Bson::Null},
                    doc! {"$set": {"divergence": bson::to_bson(&detail).map_err(|_| AppError::AuditIntegrityConflict)?}})
                    .session(&mut session).await?;
            }
        } else {
            progress
                .insert_one(WitnessDelivery {
                    id: self.chain(),
                    enrollment_digest: self.witness.pin.digest(),
                    receipt: self.witness.pin.enrollment.clone(),
                    pending: None,
                    divergence: Some(detail),
                })
                .session(&mut session)
                .await?;
        }
        audit_state_service::commit(&mut session).await?;
        Ok(())
    }

    fn chain(&self) -> String {
        self.witness.pin.enrollment.body.chain_id.to_string()
    }

    pub(super) async fn claim_dispatch(
        &self,
        request: DispatchRequest,
    ) -> AppResult<SignedReceipt> {
        let cursor = self.cursor.lock().await;
        let receipt = match self.witness.client.claim_dispatch(request.clone()).await {
            Ok(receipt) => receipt,
            Err(error) => return self.failed(Failure::Remote(error), &cursor).await,
        };
        if let Err(failure) = self.check_successor(&cursor.latest, &receipt).await {
            return self.failed(failure, &cursor).await;
        }
        if receipt.body.event != (WitnessEvent::Dispatch { request }) {
            return self
                .failed(
                    conflict(receipt.body.head.seq, "", &receipt_digest(&receipt)),
                    &cursor,
                )
                .await;
        }
        // A claim receipt is not a new challenge and never renews freshness.
        Ok(receipt)
    }

    pub(super) async fn require_dispatch_now(&self) -> AppResult<()> {
        if let Some(saved) = self
            .db
            .collection::<WitnessDelivery>(DELIVERY)
            .find_one(doc! {"_id": self.chain()})
            .await?
            && let Some(detail) = saved.divergence
        {
            self.runtime.mark_divergence(&detail);
            return Err(AppError::AuditWitnessDiverged);
        }
        let report = super::external_evidence(&self.db, &self.runtime).await?;
        if report.diagnostics.admission.effect == super::policy::Admission::Closed {
            return Err(if report.diagnostics.divergence.is_some() {
                AppError::AuditWitnessDiverged
            } else {
                AppError::AuditAdmissionClosed {
                    reason: crate::errors::AuditAdmissionReason::WitnessUnavailable,
                }
            });
        }
        Ok(())
    }

    async fn tick_inner(&self, cursor: &mut Cursor, deliver: bool) -> Result<bool> {
        let persisted = self
            .db
            .collection::<WitnessDelivery>(DELIVERY)
            .find_one(doc! {"_id": self.chain()})
            .await?;
        if let Some(saved) = &persisted {
            if let Some(detail) = &saved.divergence {
                return Err(Failure::Diverged(detail.clone()));
            }
            if saved.enrollment_digest != self.witness.pin.digest() {
                return Err(conflict(
                    0,
                    &saved.enrollment_digest,
                    &self.witness.pin.digest(),
                ));
            }
        }
        self.check_link(&self.witness.pin.enrollment.body.head)
            .await?;
        let contact = self.witness.client.observe().await?;
        let observed = contact.receipt();
        self.authenticate(observed)?;
        if observed.body.receipt_seq < cursor.latest.body.receipt_seq {
            return Err(conflict(
                observed.body.head.seq,
                &receipt_digest(&cursor.latest),
                &receipt_digest(observed),
            ));
        }
        // Bound each tick; large histories continue without publishing freshness
        // until the final receipt is bound to a new challenge observation.
        for _ in 0..256 {
            if cursor.latest.body.receipt_seq == observed.body.receipt_seq {
                break;
            }
            let from = cursor
                .latest
                .body
                .receipt_seq
                .checked_add(1)
                .ok_or(AppError::AuditIntegrityConflict)?;
            let records = self.witness.client.history(from, MAX_LINKS).await?;
            if records.is_empty() {
                return Err(conflict(from, &receipt_digest(&cursor.latest), ""));
            }
            for receipt in records {
                if receipt.body.receipt_seq > observed.body.receipt_seq {
                    break;
                }
                self.check_successor(&cursor.latest, &receipt).await?;
                cursor
                    .digests
                    .insert(receipt.body.receipt_seq, receipt_digest(&receipt));
                cursor.latest = receipt;
            }
        }
        if cursor.latest.body.receipt_seq != observed.body.receipt_seq {
            return Ok(false);
        }
        if receipt_digest(&cursor.latest) != receipt_digest(observed) {
            return Err(conflict(
                observed.body.head.seq,
                &receipt_digest(&cursor.latest),
                &receipt_digest(observed),
            ));
        }
        if let Some(saved) = &persisted {
            // A sibling may have committed after the observation began. Retry
            // with another observation instead of declaring a concurrent fork.
            if saved.receipt.body.receipt_seq > observed.body.receipt_seq {
                return Ok(false);
            }
            if cursor.digests.get(&saved.receipt.body.receipt_seq)
                != Some(&receipt_digest(&saved.receipt))
            {
                return Err(conflict(
                    saved.receipt.body.head.seq,
                    &receipt_digest(&saved.receipt),
                    &receipt_digest(observed),
                ));
            }
        }
        self.acknowledge(observed).await?;
        self.runtime.accept_contact(contact);
        // A claim must not stale its own predecessor by sending an unrelated
        // advance immediately before reserving the conditional dispatch.
        if !deliver {
            return Ok(false);
        }
        let request = self.prepare_batch().await?;
        if let Some(request) = request {
            // The request identity/predecessor/links were committed first. A
            // lost acknowledgment is recovered from signed history next tick.
            let receipt = self.witness.client.advance(request.clone()).await?;
            self.authenticate(&receipt)?;
            if receipt.body.event != (WitnessEvent::Advance { request }) {
                return Err(conflict(
                    receipt.body.head.seq,
                    "",
                    &receipt_digest(&receipt),
                ));
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn authenticate(&self, receipt: &SignedReceipt) -> Result<()> {
        let key = self
            .witness
            .pin
            .key()
            .map_err(|_| AppError::AuditIntegrityConflict)?;
        verify_receipt(receipt, self.witness.pin.enrollment.body.chain_id, &key)?;
        Ok(())
    }

    async fn check_successor(
        &self,
        previous: &SignedReceipt,
        receipt: &SignedReceipt,
    ) -> Result<()> {
        self.authenticate(receipt)?;
        let body = &receipt.body;
        if body.receipt_seq
            != previous
                .body
                .receipt_seq
                .checked_add(1)
                .ok_or(AppError::AuditIntegrityConflict)?
            || body.previous_receipt != receipt_digest(previous)
            || body.witness_time_ms < previous.body.witness_time_ms
        {
            return Err(conflict(
                body.head.seq,
                &receipt_digest(previous),
                &body.previous_receipt,
            ));
        }
        match &body.event {
            WitnessEvent::Advance { request } => {
                if request.chain_id != body.chain_id
                    || request.previous_receipt != body.previous_receipt
                    || request.request_id.get_version_num() != 4
                    || request.links.is_empty()
                    || request.links.len() > MAX_LINKS
                {
                    return Err(conflict(body.head.seq, "", &receipt_digest(receipt)));
                }
                let mut head = &previous.body.head;
                for link in &request.links {
                    if link.seq
                        != head
                            .seq
                            .checked_add(1)
                            .ok_or(AppError::AuditIntegrityConflict)?
                        || link.prev_hash != head.entry_hash
                        || link.key_version != head.key_version
                    {
                        return Err(conflict(link.seq, &head.entry_hash, &link.prev_hash));
                    }
                    self.check_link(link).await?;
                    head = link;
                }
                if &body.head != head {
                    return Err(conflict(
                        body.head.seq,
                        &head.entry_hash,
                        &body.head.entry_hash,
                    ));
                }
            }
            WitnessEvent::Dispatch { request } => {
                if body.head != previous.body.head
                    || request.chain_id != body.chain_id
                    || request.previous_receipt != body.previous_receipt
                    || request.intent_seq == 0
                    || [request.operation_id, request.attempt_id, request.process_id]
                        .iter()
                        .any(|id| id.get_version_num() != 4)
                    || request.intent_seq > body.head.seq
                {
                    return Err(conflict(body.head.seq, "", &receipt_digest(receipt)));
                }
                let entry = self.local_link(request.intent_seq).await?;
                if entry.entry_hash != request.intent_hash {
                    return Err(conflict(entry.seq, &entry.entry_hash, &request.intent_hash));
                }
            }
            WitnessEvent::Enrollment { .. } => {
                return Err(conflict(body.head.seq, "", &receipt_digest(receipt)));
            }
        }
        Ok(())
    }

    async fn local_link(&self, seq: u64) -> Result<AuditLink> {
        if seq == 0 {
            return Ok(AuditLink {
                key_version: 1,
                seq: 0,
                prev_hash: ZERO_HASH.into(),
                entry_hash: ZERO_HASH.into(),
            });
        }
        let seq_i64 = i64::try_from(seq).map_err(|_| AppError::AuditIntegrityConflict)?;
        let entry = self
            .db
            .collection::<AuditLog>(LOG)
            .find_one(doc! {"seq": seq_i64})
            .await?
            .ok_or_else(|| conflict(seq, "", "missing_local_entry"))?;
        let hash = audit_chain_service::compute_entry_hash(&entry, self.key.as_ref().as_ref())?;
        if entry.entry_hash.as_deref() != Some(&hash) {
            return Err(conflict(
                seq,
                &hash,
                entry.entry_hash.as_deref().unwrap_or(""),
            ));
        }
        Ok(AuditLink {
            key_version: 1,
            seq,
            prev_hash: entry.prev_hash.ok_or(AppError::AuditIntegrityConflict)?,
            entry_hash: hash,
        })
    }

    async fn check_link(&self, witnessed: &AuditLink) -> Result<()> {
        let local = self.local_link(witnessed.seq).await?;
        if local != *witnessed {
            return Err(conflict(
                witnessed.seq,
                &local.entry_hash,
                &witnessed.entry_hash,
            ));
        }
        Ok(())
    }

    async fn start(&self) -> Result<ClientSession> {
        let mut session = self.db.client().start_session().await?;
        session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(audit_state_service::majority())
            .await?;
        Ok(session)
    }

    async fn acknowledge(&self, receipt: &SignedReceipt) -> Result<()> {
        let mut session = self.start().await?;
        let (state, now) = audit_state_service::writer_snapshot(
            &self.db,
            &mut session,
            self.key.as_ref().as_ref(),
        )
        .await?;
        let seq =
            i64::try_from(receipt.body.head.seq).map_err(|_| AppError::AuditIntegrityConflict)?;
        if state.id != self.chain() || seq > state.head_seq {
            return Err(conflict(
                receipt.body.head.seq,
                &state.head_hash,
                &receipt.body.head.entry_hash,
            ));
        }
        // A concurrent sibling can already be further ahead. Never regress it.
        if state.witnessed_seq > seq {
            return Ok(());
        }
        if state.witness_enrolled
            && state.witnessed_seq == seq
            && state.witnessed_hash != receipt.body.head.entry_hash
        {
            return Err(conflict(
                receipt.body.head.seq,
                &state.witnessed_hash,
                &receipt.body.head.entry_hash,
            ));
        }
        let delivered = self.db.collection::<AuditOutbox>(OUTBOX).update_many(
            doc! {"chain_id": &state.id, "seq": {"$lte": seq}, "witnessed_at": bson::Bson::Null},
            doc! {"$set": {"witnessed_at": bson::DateTime::from_chrono(now)}}).session(&mut session).await?;
        let count = state
            .pending_count
            .checked_sub(
                i64::try_from(delivered.modified_count)
                    .map_err(|_| AppError::AuditIntegrityConflict)?,
            )
            .filter(|v| *v >= 0)
            .ok_or(AppError::AuditIntegrityConflict)?;
        let outbox = self
            .db
            .collection::<AuditOutbox>(OUTBOX)
            .find_one(doc! {"chain_id": &state.id, "witnessed_at": bson::Bson::Null})
            .sort(doc! {"order": 1})
            .session(&mut session)
            .await?;
        let pending = self
            .db
            .collection::<PendingAuditEvent>(PENDING)
            .find_one(doc! {"materialized_seq": bson::Bson::Null})
            .sort(doc! {"order": 1})
            .session(&mut session)
            .await?;
        let oldest = match (outbox, pending) {
            (Some(a), Some(b)) => Some(if a.order < b.order {
                a.reserved_at
            } else {
                b.reserved_at
            }),
            (Some(a), None) => Some(a.reserved_at),
            (None, Some(b)) => Some(b.reserved_at),
            (None, None) => None,
        };
        let terminal = self
            .db
            .collection::<crate::models::audit_effect_intent::AuditEffectIntent>(
                crate::models::audit_effect_intent::COLLECTION_NAME,
            )
            .find_one(doc! {"terminal_stage": bson::Bson::Null})
            .sort(doc! {"terminal_order": 1})
            .session(&mut session)
            .await?;
        // Unused terminal slots remain pending even after the intent is
        // witnessed. They cannot vanish from count/age admission bookkeeping.
        let oldest = match (oldest, terminal) {
            (Some(at), Some(terminal)) => Some(at.min(terminal.reserved_at)),
            (None, Some(terminal)) => Some(terminal.reserved_at),
            (at, None) => at,
        };
        if (count == 0) != oldest.is_none() {
            return Err(AppError::AuditIntegrityConflict.into());
        }
        self.db.collection::<AuditState>(STATE).update_one(doc! {"_id": &state.id, "next_order": state.next_order, "head_hash": &state.head_hash},
            doc! {"$set": {"witness_enrolled": true, "witnessed_seq": seq, "witnessed_hash": &receipt.body.head.entry_hash,
                "pending_count": count, "oldest_pending_at": oldest.map(bson::DateTime::from_chrono)}})
            .session(&mut session).await?;
        let progress = self.db.collection::<WitnessDelivery>(DELIVERY);
        let saved = progress
            .find_one(doc! {"_id": &state.id})
            .session(&mut session)
            .await?;
        if let Some(saved) = saved {
            if saved.receipt.body.receipt_seq > receipt.body.receipt_seq {
                return Ok(());
            }
            if saved.divergence.is_some() || saved.enrollment_digest != self.witness.pin.digest() {
                return Err(AppError::AuditWitnessDiverged.into());
            }
            let pending = saved.pending.filter(|r| {
                r.links
                    .last()
                    .is_some_and(|l| l.seq > receipt.body.head.seq)
            });
            progress.update_one(doc! {"_id": &state.id}, doc! {"$set": {
                "receipt": bson::to_bson(receipt).map_err(|_| AppError::AuditIntegrityConflict)?,
                "pending": bson::to_bson(&pending).map_err(|_| AppError::AuditIntegrityConflict)?}}).session(&mut session).await?;
        } else {
            progress
                .insert_one(WitnessDelivery {
                    id: state.id,
                    enrollment_digest: self.witness.pin.digest(),
                    receipt: receipt.clone(),
                    pending: None,
                    divergence: None,
                })
                .session(&mut session)
                .await?;
        }
        audit_state_service::commit(&mut session).await?;
        Ok(())
    }

    async fn prepare_batch(&self) -> Result<Option<AdvanceRequest>> {
        let mut session = self.start().await?;
        let (state, _) = audit_state_service::writer_snapshot(
            &self.db,
            &mut session,
            self.key.as_ref().as_ref(),
        )
        .await?;
        let progress = self.db.collection::<WitnessDelivery>(DELIVERY);
        let saved = progress
            .find_one(doc! {"_id": &state.id})
            .session(&mut session)
            .await?
            .ok_or(AppError::AuditIntegrityConflict)?;
        if saved.divergence.is_some() {
            return Err(AppError::AuditWitnessDiverged.into());
        }
        if let Some(request) = saved.pending {
            if request.chain_id != self.witness.pin.enrollment.body.chain_id
                || request.request_id.get_version_num() != 4
                || request.previous_receipt != receipt_digest(&saved.receipt)
                || request.links.is_empty()
                || request.links.len() > MAX_LINKS
            {
                return Err(AppError::AuditIntegrityConflict.into());
            }
            let mut previous = &saved.receipt.body.head;
            for link in &request.links {
                if link.seq
                    != previous
                        .seq
                        .checked_add(1)
                        .ok_or(AppError::AuditIntegrityConflict)?
                    || link.prev_hash != previous.entry_hash
                    || link.key_version != previous.key_version
                {
                    return Err(conflict(link.seq, &previous.entry_hash, &link.prev_hash));
                }
                self.check_link(link).await?;
                previous = link;
            }
            return Ok(Some(request));
        }
        let mut rows = self
            .db
            .collection::<AuditOutbox>(OUTBOX)
            .find(doc! {"chain_id": &state.id, "seq": {"$gt": state.witnessed_seq}})
            .sort(doc! {"seq": 1})
            .limit(MAX_LINKS as i64)
            .session(&mut session)
            .await?;
        let mut links = Vec::new();
        let mut previous = saved.receipt.body.head.clone();
        while let Some(row) = rows.next(&mut session).await.transpose()? {
            let link = AuditLink {
                key_version: row.key_version,
                seq: u64::try_from(row.seq).map_err(|_| AppError::AuditIntegrityConflict)?,
                prev_hash: row.prev_hash,
                entry_hash: row.entry_hash,
            };
            if link.seq != previous.seq + 1
                || link.prev_hash != previous.entry_hash
                || link.key_version != previous.key_version
                || row.witnessed_at.is_some()
            {
                return Err(conflict(link.seq, &previous.entry_hash, &link.prev_hash));
            }
            // Outbox evidence alone is not trusted to attest an event.
            self.check_link(&link).await?;
            previous = link.clone();
            links.push(link);
        }
        if links.is_empty() {
            return Ok(None);
        }
        let request = AdvanceRequest {
            chain_id: self.witness.pin.enrollment.body.chain_id,
            request_id: Uuid::new_v4(),
            previous_receipt: receipt_digest(&saved.receipt),
            links,
        };
        progress.update_one(doc! {"_id": &state.id}, doc! {"$set": {"pending": bson::to_bson(&request).map_err(|_| AppError::AuditIntegrityConflict)?}}).session(&mut session).await?;
        audit_state_service::commit(&mut session).await?;
        Ok(Some(request))
    }
}

#[cfg(test)]
#[path = "relay_tests.rs"]
mod tests;
