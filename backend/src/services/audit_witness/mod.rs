//! Independent witness business rules. No application payloads or HMAC secrets
//! cross this boundary. HMAC verification remains the application verifier's job.
mod client;
mod codec;
pub mod storage;

pub use client::{HttpWitness, ObservationChallenge, VerifiedContact, WitnessClient};
pub use codec::{receipt_digest, verify_observation, verify_receipt};

use std::{collections::HashMap, sync::Mutex, time::SystemTime};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::models::audit_witness::*;

pub const PROTOCOL: u32 = 1;
pub const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const MAX_LINKS: usize = 128;
pub const MAX_FRAME_BYTES: usize = 128 * 1024;
pub const MAX_HTTP_BYTES: usize = 256 * 1024;

/// Deliberately categorical: neither transport errors, paths nor submitted bytes
/// are included in HTTP responses, Debug or Display. Server AppError mapping is
/// supplied by the application runtime integration, not the independent sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WitnessError {
    #[error("invalid witness request")]
    Invalid,
    #[error("witness authentication required")]
    Unauthorized,
    #[error("witness history conflicts with request")]
    Conflict,
    #[error("witness unavailable")]
    Unavailable,
    #[error("witness storage failed; restart required")]
    Storage,
    #[error("witness history authentication failed")]
    Corrupt,
    #[error("unsafe witness storage permissions")]
    Permissions,
}

pub type WitnessResult<T> = Result<T, WitnessError>;

pub fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_link(link: &AuditLink) -> bool {
    link.key_version > 0
        && link.seq <= i64::MAX as u64
        && valid_hash(&link.prev_hash)
        && valid_hash(&link.entry_hash)
        && (link.seq != 0 || (link.prev_hash == ZERO_HASH && link.entry_hash == ZERO_HASH))
}

fn valid_id(id: Uuid) -> bool {
    id.get_version_num() == 4 && id.get_variant() == uuid::Variant::RFC4122
}

pub fn now_ms() -> WitnessResult<u64> {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|v| v.as_millis().try_into().ok())
        .ok_or(WitnessError::Unavailable)
}

fn sign_receipt(body: ReceiptBody, key: &WitnessSigningKey) -> SignedReceipt {
    let signature = hex::encode(key.sign(&codec::receipt_bytes(&body)).to_bytes());
    SignedReceipt { body, signature }
}

// Persistent signing material lives only in Zeroizing. The short-lived
// dalek key used for an operation additionally zeroizes itself on drop.
pub(super) struct WitnessSigningKey {
    seed: Zeroizing<[u8; 32]>,
}
impl WitnessSigningKey {
    fn new(seed: Zeroizing<[u8; 32]>) -> Self {
        Self { seed }
    }
    fn verifying_key(&self) -> VerifyingKey {
        SigningKey::from_bytes(&self.seed).verifying_key()
    }
    fn sign(&self, message: &[u8]) -> Signature {
        SigningKey::from_bytes(&self.seed).sign(message)
    }
}

pub(crate) struct History {
    enrollment: Enrollment,
    records: Vec<SignedReceipt>,
    requests: HashMap<Uuid, usize>,
    dispatches: HashMap<Uuid, usize>,
    links: HashMap<u64, AuditLink>,
}

impl History {
    fn new(enrollment: Enrollment) -> WitnessResult<Self> {
        if enrollment.protocol != PROTOCOL
            || !valid_id(enrollment.chain_id)
            || !valid_link(&enrollment.initial_head)
            || !valid_hash(&enrollment.append_token_sha256)
            || !valid_hash(&enrollment.verification_key)
        {
            return Err(WitnessError::Corrupt);
        }
        Ok(Self {
            enrollment,
            records: Vec::new(),
            requests: HashMap::new(),
            dispatches: HashMap::new(),
            links: HashMap::new(),
        })
    }

    fn latest(&self) -> WitnessResult<&SignedReceipt> {
        self.records.last().ok_or(WitnessError::Corrupt)
    }

    fn check_event(&self, event: &WitnessEvent) -> WitnessResult<AuditLink> {
        match event {
            WitnessEvent::Enrollment { head }
                if self.records.is_empty() && head == &self.enrollment.initial_head =>
            {
                Ok(head.clone())
            }
            WitnessEvent::Advance { request } => {
                let head = &self.latest()?.body.head;
                self.check_predecessor(request.chain_id, &request.previous_receipt)?;
                if !valid_id(request.request_id)
                    || request.links.is_empty()
                    || request.links.len() > MAX_LINKS
                    || self.requests.contains_key(&request.request_id)
                {
                    return Err(WitnessError::Conflict);
                }
                let mut previous = head;
                for link in &request.links {
                    if !valid_link(link)
                        || link.seq != previous.seq.checked_add(1).ok_or(WitnessError::Invalid)?
                        || link.prev_hash != previous.entry_hash
                        // A future authenticated key-transition event will open
                        // a new version. Arbitrary version switching is forbidden.
                        || link.key_version != previous.key_version
                    {
                        return Err(WitnessError::Conflict);
                    }
                    previous = link;
                }
                Ok(previous.clone())
            }
            WitnessEvent::Dispatch { request } => {
                self.check_predecessor(request.chain_id, &request.previous_receipt)?;
                if !valid_id(request.operation_id)
                    || !valid_id(request.attempt_id)
                    || !valid_id(request.process_id)
                    || self.dispatches.contains_key(&request.operation_id)
                    || request.intent_seq == 0
                    || !valid_hash(&request.intent_hash)
                    || self.links.get(&request.intent_seq).map(|l| &l.entry_hash)
                        != Some(&request.intent_hash)
                {
                    return Err(WitnessError::Conflict);
                }
                Ok(self.latest()?.body.head.clone())
            }
            _ => Err(WitnessError::Conflict),
        }
    }

    fn check_predecessor(&self, chain: Uuid, previous: &str) -> WitnessResult<()> {
        if chain != self.enrollment.chain_id || previous != receipt_digest(self.latest()?) {
            return Err(WitnessError::Conflict);
        }
        Ok(())
    }

    fn accept(&mut self, receipt: SignedReceipt, key: &VerifyingKey) -> WitnessResult<()> {
        verify_receipt(&receipt, self.enrollment.chain_id, key)?;
        let expected_previous = self
            .records
            .last()
            .map(receipt_digest)
            .unwrap_or_else(|| ZERO_HASH.into());
        if receipt.body.receipt_seq != self.records.len() as u64
            || receipt.body.previous_receipt != expected_previous
            || receipt.body.witness_time_ms
                < self
                    .records
                    .last()
                    .map(|r| r.body.witness_time_ms)
                    .unwrap_or(0)
            || self.check_event(&receipt.body.event)? != receipt.body.head
        {
            return Err(WitnessError::Corrupt);
        }
        let index = self.records.len();
        match &receipt.body.event {
            WitnessEvent::Enrollment { head } => {
                self.links.insert(head.seq, head.clone());
            }
            WitnessEvent::Advance { request } => {
                self.requests.insert(request.request_id, index);
                for link in &request.links {
                    self.links.insert(link.seq, link.clone());
                }
            }
            WitnessEvent::Dispatch { request } => {
                self.dispatches.insert(request.operation_id, index);
            }
        }
        self.records.push(receipt);
        Ok(())
    }
}

pub struct ReferenceWitness {
    inner: Mutex<Inner>,
    token_hash: Zeroizing<[u8; 32]>,
}

struct Inner {
    history: History,
    key: WitnessSigningKey,
    journal: storage::Journal,
    failed: bool,
}

impl ReferenceWitness {
    pub fn open(admin_dir: &std::path::Path, journal_dir: &std::path::Path) -> WitnessResult<Self> {
        let (enrollment, key, journal) = storage::open(admin_dir, journal_dir)?;
        Self::recover(enrollment, key, journal)
    }

    fn recover(
        enrollment: Enrollment,
        key: WitnessSigningKey,
        journal: storage::Journal,
    ) -> WitnessResult<Self> {
        if hex::encode(key.verifying_key().to_bytes()) != enrollment.verification_key {
            return Err(WitnessError::Corrupt);
        }
        let token_hash = Zeroizing::new(
            hex::decode(&enrollment.append_token_sha256)
                .map_err(|_| WitnessError::Corrupt)?
                .try_into()
                .map_err(|_| WitnessError::Corrupt)?,
        );
        let mut history = History::new(enrollment)?;
        for record in journal.read_all()? {
            history.accept(record, &key.verifying_key())?;
        }
        history.latest()?;
        Ok(Self {
            inner: Mutex::new(Inner {
                history,
                key,
                journal,
                failed: false,
            }),
            token_hash,
        })
    }

    pub fn authenticate(&self, token: &[u8]) -> WitnessResult<()> {
        if token.len() != 64
            || !bool::from(
                Sha256::digest(token)
                    .as_slice()
                    .ct_eq(self.token_hash.as_ref()),
            )
        {
            return Err(WitnessError::Unauthorized);
        }
        Ok(())
    }

    fn with_inner<T>(&self, f: impl FnOnce(&mut Inner) -> WitnessResult<T>) -> WitnessResult<T> {
        let mut inner = self.inner.lock().map_err(|_| WitnessError::Unavailable)?;
        if inner.failed {
            return Err(WitnessError::Unavailable);
        }
        f(&mut inner)
    }

    pub fn advance(&self, request: AdvanceRequest) -> WitnessResult<SignedReceipt> {
        self.with_inner(|inner| {
            if let Some(&index) = inner.history.requests.get(&request.request_id) {
                let record = &inner.history.records[index];
                return if record.body.event == (WitnessEvent::Advance { request }) {
                    Ok(record.clone())
                } else {
                    Err(WitnessError::Conflict)
                };
            }
            inner.commit(WitnessEvent::Advance { request })
        })
    }

    pub fn claim_dispatch(&self, request: DispatchRequest) -> WitnessResult<SignedReceipt> {
        self.with_inner(|inner| {
            if let Some(&index) = inner.history.dispatches.get(&request.operation_id) {
                let record = &inner.history.records[index];
                return if record.body.event == (WitnessEvent::Dispatch { request }) {
                    Ok(record.clone())
                } else {
                    Err(WitnessError::Conflict)
                };
            }
            inner.commit(WitnessEvent::Dispatch { request })
        })
    }

    pub fn observe(&self, request: ObserveRequest) -> WitnessResult<SignedObservation> {
        self.with_inner(|inner| {
            if request.chain_id != inner.history.enrollment.chain_id
                || !valid_hash(&request.challenge)
            {
                return Err(WitnessError::Invalid);
            }
            let body = ObservationBody {
                protocol: PROTOCOL,
                chain_id: request.chain_id,
                challenge: request.challenge,
                witness_time_ms: now_ms()?,
                latest: inner.history.latest()?.clone(),
            };
            let signature =
                hex::encode(inner.key.sign(&codec::observation_bytes(&body)).to_bytes());
            Ok(SignedObservation { body, signature })
        })
    }

    pub fn history(&self, request: HistoryRequest) -> WitnessResult<Vec<SignedReceipt>> {
        self.with_inner(|inner| {
            if request.chain_id != inner.history.enrollment.chain_id
                || request.limit == 0
                || request.limit > MAX_LINKS
            {
                return Err(WitnessError::Invalid);
            }
            let from: usize = request
                .from_receipt
                .try_into()
                .map_err(|_| WitnessError::Invalid)?;
            // Limit bytes as well as record count; each receipt can carry 128 links.
            let mut size = 2;
            let mut result = Vec::new();
            for record in inner.history.records.iter().skip(from).take(request.limit) {
                size += serde_json::to_vec(record)
                    .map_err(|_| WitnessError::Corrupt)?
                    .len()
                    + 1;
                if size > MAX_HTTP_BYTES {
                    break;
                }
                result.push(record.clone());
            }
            Ok(result)
        })
    }
}

impl Inner {
    fn commit(&mut self, event: WitnessEvent) -> WitnessResult<SignedReceipt> {
        let head = self.history.check_event(&event)?;
        let previous = self.history.latest()?;
        let body = ReceiptBody {
            protocol: PROTOCOL,
            chain_id: self.history.enrollment.chain_id,
            receipt_seq: self.history.records.len() as u64,
            previous_receipt: receipt_digest(previous),
            witness_time_ms: now_ms()?.max(previous.body.witness_time_ms),
            head,
            event,
        };
        let record = sign_receipt(body, &self.key);
        if self.journal.append(&record).is_err() {
            // Publication may have happened: never continue from a stale memory head.
            self.failed = true;
            return Err(WitnessError::Storage);
        }
        self.history
            .accept(record.clone(), &self.key.verifying_key())?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests;
