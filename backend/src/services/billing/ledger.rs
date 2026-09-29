//! Tamper-evident billing ledger.
//!
//! Mirrors the audit-log hash chain (`services/audit_chain_service.rs`):
//! every money-moving billing event appends an immutable entry whose
//! HMAC-SHA256 `entry_hash` commits to the previous entry, so mutating,
//! deleting, or inserting history breaks verification at that seq. The
//! operational billing rows (`usage_meter`, `billing_topup_sessions`)
//! stay mutable by design; this ledger is the append-only journal of
//! their money-moving transitions.

use crate::models::billing_ledger::{BillingPosting, PostingSide};
use crate::models::credits::Credits;
use std::sync::OnceLock;

use chrono::{DateTime, TimeZone, Utc};
use dashmap::DashMap;
use futures::TryStreamExt;
use hmac::{Hmac, Mac};
use mongodb::bson::{Document, doc};
use rand::Rng;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::errors::{AppError, AppResult};
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
use crate::models::billing_ledger::{
    BillingLedgerEntry, BillingLedgerEventType, COLLECTION_NAME as BILLING_LEDGER,
};
use crate::models::usage_meter::UsageMeterRow;
use crate::services::{audit_chain_service, audit_service};

type HmacSha256 = Hmac<Sha256>;

pub const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
const RECORD_SEPARATOR: u8 = 0x1e;
const APPEND_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
const APPEND_BATCH_MAX: usize = 256;
const APPEND_BATCH_WAIT: std::time::Duration = std::time::Duration::from_millis(10);
pub const DEFAULT_VERIFY_LIMIT: i64 = 10_000;
pub const MAX_VERIFY_LIMIT: i64 = 10_000;

static BILLING_LEDGER_HMAC_KEY: OnceLock<Zeroizing<[u8; 32]>> = OnceLock::new();
static APPEND_BATCHERS: OnceLock<DashMap<String, std::sync::Arc<AppendBatcher>>> = OnceLock::new();

#[cfg(test)]
pub(crate) const TEST_BILLING_LEDGER_HMAC_KEY: [u8; 32] = [3_u8; 32];

pub fn init_billing_ledger_hmac_key(key: Zeroizing<[u8; 32]>) {
    if BILLING_LEDGER_HMAC_KEY.set(key).is_err() {
        tracing::warn!("billing-ledger HMAC key was already initialized");
    }
}

pub(super) fn billing_ledger_hmac_key() -> Option<&'static [u8]> {
    BILLING_LEDGER_HMAC_KEY.get().map(|key| key.as_ref())
}

pub fn derive_billing_ledger_hmac_key(
    env_override: Option<&str>,
    encryption_key: Option<&str>,
    jwt_private_key_pem: Option<&[u8]>,
) -> Zeroizing<[u8; 32]> {
    if let Some(raw) = env_override {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            match hex::decode(trimmed) {
                Ok(bytes) if bytes.len() == 32 => {
                    let mut out = [0u8; 32];
                    out.copy_from_slice(&bytes);
                    tracing::info!("billing-ledger HMAC key loaded from BILLING_LEDGER_HMAC_KEY");
                    return Zeroizing::new(out);
                }
                _ => {
                    panic!("BILLING_LEDGER_HMAC_KEY must be 64 hex characters (32 bytes)");
                }
            }
        }
    }

    let jwt_private_key_pem = jwt_private_key_pem.unwrap_or_default();
    if let Some(master) = decode_hmac_seed(encryption_key) {
        tracing::info!("billing-ledger HMAC key derived from ENCRYPTION_KEY");
        return crate::crypto::hmac_keys::derive_hmac_key(
            "billing-ledger",
            Some(&master),
            jwt_private_key_pem,
        );
    }

    if !jwt_private_key_pem.is_empty() {
        tracing::info!("billing-ledger HMAC key derived from JWT private key");
        return crate::crypto::hmac_keys::derive_hmac_key(
            "billing-ledger",
            None,
            jwt_private_key_pem,
        );
    }

    panic!(
        "billing-ledger HMAC key has no source: BILLING_LEDGER_HMAC_KEY, ENCRYPTION_KEY, \
         and the JWT private key are all missing."
    );
}

fn decode_hmac_seed(encryption_key: Option<&str>) -> Option<Zeroizing<Vec<u8>>> {
    let hex_key = encryption_key?;
    let Ok(master) = hex::decode(hex_key.trim()) else {
        return None;
    };
    if master.len() == 32 {
        Some(Zeroizing::new(master))
    } else {
        None
    }
}

/// The wallet's durable settlement lock is retained until this append is confirmed.
pub async fn record_usage_settled(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    credits: Credits,
) -> AppResult<()> {
    if credits == Credits::ZERO {
        return Ok(());
    }
    let entry = exact_entry(
        &row.billing_owner_id,
        &row.id,
        "usage_settled",
        format!("usage-settled:{}", row.id),
        transfer(
            format!("usage:{}", row.id),
            format!("wallet:{}", row.billing_owner_id),
            credits,
        ),
        Some(row),
    );
    if append_and_confirm_dedupe(db, entry).await {
        Ok(())
    } else {
        Err(AppError::Internal("usage ledger append is pending".into()))
    }
}

pub fn transfer(debit: String, credit: String, amount: Credits) -> Vec<BillingPosting> {
    if amount == Credits::ZERO {
        return Vec::new();
    }
    vec![
        BillingPosting {
            account: debit,
            side: PostingSide::Debit,
            amount,
        },
        BillingPosting {
            account: credit,
            side: PostingSide::Credit,
            amount,
        },
    ]
}

pub fn exact_entry(
    owner: &str,
    reference: &str,
    movement: &str,
    dedupe: String,
    postings: Vec<BillingPosting>,
    row: Option<&UsageMeterRow>,
) -> BillingLedgerEntry {
    BillingLedgerEntry {
        id: Uuid::new_v4().to_string(),
        seq: 0,
        prev_hash: String::new(),
        entry_hash: String::new(),
        event_type: BillingLedgerEventType::AccountingV2,
        movement: Some(movement.into()),
        postings,
        owner_id: owner.into(),
        reference_id: reference.into(),
        transaction_id: row.map(|r| r.transaction_id.clone()),
        layer: row.map(|r| r.layer),
        metric: row.map(|r| r.metric),
        service_slug: row.and_then(|r| r.service_slug.clone()),
        model: row.and_then(|r| r.model.clone()),
        quantity: row.and_then(|r| r.quantity),
        amount_credits: None,
        amount_micros: None,
        balance_credits: None,
        dedupe_key: Some(dedupe),
        wallet_id: row.and_then(|r| r.wallet_id.clone()),
        created_at: Utc::now(),
    }
}

pub fn validate_postings(entry: &BillingLedgerEntry) -> AppResult<()> {
    if entry.event_type != BillingLedgerEventType::AccountingV2 {
        return Ok(());
    }
    let movement = entry
        .movement
        .as_deref()
        .ok_or_else(|| AppError::Internal("v2 movement is missing".into()))?;
    if entry.postings.is_empty()
        && matches!(
            movement,
            "topup_created" | "opening_balance" | "wallet_credited"
        )
    {
        return Ok(());
    }
    let mut debit = Credits::ZERO;
    let mut credit = Credits::ZERO;
    for posting in &entry.postings {
        if posting.amount <= Credits::ZERO || posting.account.is_empty() {
            return Err(AppError::Internal("invalid v2 posting".into()));
        }
        match posting.side {
            PostingSide::Debit => debit = debit.checked_add(posting.amount)?,
            PostingSide::Credit => credit = credit.checked_add(posting.amount)?,
        }
    }
    if debit == Credits::ZERO || debit != credit {
        return Err(AppError::Internal("unbalanced v2 transaction".into()));
    }
    Ok(())
}

/// Record a top-up checkout that was actually created with the provider
/// (not the local pending stub, which may still fail before any money can
/// move).
pub async fn record_topup_created(
    db: &mongodb::Database,
    owner_id: &str,
    session_id: &str,
    amount_credits: i64,
    lago_wallet_id: &str,
) {
    let mut entry = exact_entry(
        owner_id,
        session_id,
        "topup_created",
        format!("topup-created:{session_id}"),
        Vec::new(),
        None,
    );
    entry.amount_credits = Some(amount_credits);
    entry.wallet_id = Some(lago_wallet_id.into());
    append_best_effort(db, entry).await;
}

/// Record paid credits landing on a wallet (Lago `invoice.paid_credit_added`).
/// `balance_credits` is the post-credit wallet balance snapshot. Lago
/// retries webhook deliveries, so a `dedupe_key` (the Lago invoice id)
/// makes the record at-most-once; without one, duplicates are possible.
pub async fn record_wallet_credited(
    db: &mongodb::Database,
    owner_id: &str,
    customer_id: &str,
    balance_credits: Credits,
    dedupe_key: Option<String>,
) {
    // The balance movement was committed by refresh_wallet_balance. This
    // provider-invoice memo preserves the receipt without double posting it.
    let mut entry = exact_entry(
        owner_id,
        customer_id,
        "wallet_credited",
        dedupe_key.unwrap_or_else(|| format!("wallet-credit:{}", Uuid::new_v4())),
        Vec::new(),
        None,
    );
    entry.balance_credits = Some(balance_credits.display_whole());
    append_best_effort(db, entry).await;
}

#[allow(clippy::too_many_arguments)]
pub async fn record_grant_event(
    db: &mongodb::Database,
    event_type: BillingLedgerEventType,
    owner_id: &str,
    grant_id: &str,
    amount: Credits,
    usage_row: Option<&UsageMeterRow>,
    dedupe_key: String,
) -> bool {
    if !matches!(
        event_type,
        BillingLedgerEventType::GrantIssued
            | BillingLedgerEventType::GrantConsumed
            | BillingLedgerEventType::GrantExpired
            | BillingLedgerEventType::GrantRevoked
    ) {
        tracing::warn!(
            event_type = event_type.as_str(),
            "invalid event passed to grant ledger writer"
        );
        return false;
    }
    match ledger_dedupe_exists(db, &dedupe_key).await {
        Ok(true) => return true,
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, dedupe_key, "billing-ledger dedupe check failed");
            return false;
        }
    }
    let grant = format!("grant:{grant_id}");
    let postings = match event_type {
        BillingLedgerEventType::GrantIssued => {
            transfer(grant, "platform:promotions".into(), amount)
        }
        BillingLedgerEventType::GrantConsumed => {
            let Some(row) = usage_row else {
                return false;
            };
            transfer(format!("usage:{}", row.id), grant, amount)
        }
        BillingLedgerEventType::GrantExpired => {
            transfer("writeoff:grant_expired".into(), grant, amount)
        }
        BillingLedgerEventType::GrantRevoked => {
            transfer("writeoff:grant_revoked".into(), grant, amount)
        }
        _ => return false,
    };
    if amount == Credits::ZERO {
        return true;
    }
    let entry = exact_entry(
        owner_id,
        grant_id,
        event_type.as_str(),
        dedupe_key,
        postings,
        usage_row,
    );
    append_and_confirm_dedupe(db, entry).await
}

pub(super) async fn ledger_dedupe_exists(
    db: &mongodb::Database,
    dedupe_key: &str,
) -> Result<bool, mongodb::error::Error> {
    if db
        .collection::<mongodb::bson::Document>(super::exact_migration::ABSORBED)
        .count_documents(doc! { "_id": dedupe_key })
        .read_concern(mongodb::options::ReadConcern::majority())
        .await?
        > 0
    {
        return Ok(true);
    }
    db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
        .count_documents(doc! { "dedupe_key": dedupe_key })
        .read_concern(mongodb::options::ReadConcern::majority())
        .await
        .map(|count| count > 0)
}

pub(super) async fn append_and_confirm_dedupe(
    db: &mongodb::Database,
    entry: BillingLedgerEntry,
) -> bool {
    let Some(key) = billing_ledger_hmac_key() else {
        tracing::warn!(
            event_type = entry.event_type.as_str(),
            reference_id = %entry.reference_id,
            "billing-ledger HMAC key is not initialized; ledger entry skipped"
        );
        return false;
    };
    let dedupe_key = entry.dedupe_key.clone();
    match append_chained_entry(db, entry, key).await {
        Ok(_) => true,
        Err(error) => {
            // A concurrent writer may have won the unique dedupe-key race.
            // Confirm that case before reporting a failed durable append.
            let confirmed = match dedupe_key.as_deref() {
                Some(dedupe_key) => ledger_dedupe_exists(db, dedupe_key).await.unwrap_or(false),
                None => false,
            };
            if !confirmed {
                tracing::warn!(%error, "failed to append billing ledger entry");
            }
            confirmed
        }
    }
}

async fn append_best_effort(db: &mongodb::Database, entry: BillingLedgerEntry) {
    let Some(key) = billing_ledger_hmac_key() else {
        tracing::warn!(
            event_type = entry.event_type.as_str(),
            reference_id = %entry.reference_id,
            "billing-ledger HMAC key is not initialized; ledger entry skipped"
        );
        return;
    };
    if let Err(error) = append_chained_entry(db, entry, key).await {
        tracing::warn!(%error, "failed to append billing ledger entry");
    }
}

pub async fn append_chained_entry(
    db: &mongodb::Database,
    entry: BillingLedgerEntry,
    key: &[u8],
) -> Result<BillingLedgerEntry, mongodb::error::Error> {
    validate_postings(&entry).map_err(|error| mongodb::error::Error::custom(error.to_string()))?;
    // Keep the worker strongly reachable so concurrent callers share one
    // queue and actually receive group commit. A weak-only registry creates a
    // new worker for every append after the caller drops its temporary Arc.
    let batchers = APPEND_BATCHERS.get_or_init(Default::default);
    let key_id = hex::encode(Sha256::digest(key));
    let slot_key = format!("{}:{key_id}", db.name());
    for attempt in 0..2 {
        let batcher = {
            batchers
                .entry(slot_key.clone())
                .or_insert_with(|| {
                    let (tx, rx) = tokio::sync::mpsc::channel(512);
                    let batcher = std::sync::Arc::new(AppendBatcher { tx });
                    tokio::spawn(run_append_batches(
                        db.clone(),
                        Zeroizing::new(key.to_vec()),
                        rx,
                    ));
                    batcher
                })
                .clone()
        };
        let (reply, result) = tokio::sync::oneshot::channel();
        match batcher
            .tx
            .send(AppendRequest {
                entry: entry.clone(),
                reply,
            })
            .await
        {
            Ok(()) => {
                return result
                    .await
                    .map_err(|_| {
                        mongodb::error::Error::custom(
                            "billing ledger append worker dropped the result",
                        )
                    })?
                    .map_err(mongodb::error::Error::custom);
            }
            Err(_) => {
                batchers.remove_if(&slot_key, |_, current| {
                    std::sync::Arc::ptr_eq(current, &batcher)
                });
                if attempt == 1 {
                    return Err(mongodb::error::Error::custom(
                        "billing ledger append worker stopped",
                    ));
                }
            }
        }
    }
    unreachable!("append worker retry loop returns on every iteration")
}

struct AppendRequest {
    entry: BillingLedgerEntry,
    reply: tokio::sync::oneshot::Sender<Result<BillingLedgerEntry, String>>,
}

struct AppendBatcher {
    tx: tokio::sync::mpsc::Sender<AppendRequest>,
}

async fn run_append_batches(
    db: mongodb::Database,
    key: Zeroizing<Vec<u8>>,
    mut requests: tokio::sync::mpsc::Receiver<AppendRequest>,
) {
    while let Some(first) = requests.recv().await {
        let mut batch = Vec::with_capacity(APPEND_BATCH_MAX);
        batch.push(first);
        let deadline = tokio::time::Instant::now() + APPEND_BATCH_WAIT;
        while batch.len() < APPEND_BATCH_MAX {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, requests.recv()).await {
                Ok(Some(request)) => batch.push(request),
                _ => break,
            }
        }
        append_batch_with_retry(&db, &key, batch).await;
    }
}

async fn append_batch_with_retry(db: &mongodb::Database, key: &[u8], requests: Vec<AppendRequest>) {
    let deadline = tokio::time::Instant::now() + APPEND_DEADLINE;
    let mut attempt = 0;
    loop {
        let entries = requests
            .iter()
            .map(|request| request.entry.clone())
            .collect::<Vec<_>>();
        let result = append_entries_transaction(db, key, entries).await;
        match result {
            Ok(saved) => {
                for (request, entry) in requests.into_iter().zip(saved) {
                    let _ = request.reply.send(Ok(entry));
                }
                return;
            }
            Err(error)
                if is_duplicate_key_error(&error) && tokio::time::Instant::now() < deadline =>
            {
                attempt += 1;
                sleep_before_retry(attempt).await;
            }
            Err(error) => {
                if requests.len() > 1 {
                    // A malformed checkpoint or other account-local problem
                    // must not poison otherwise valid entries in the group.
                    append_requests_individually(db, key, requests).await;
                } else {
                    let request = requests
                        .into_iter()
                        .next()
                        .expect("single request is present");
                    let _ = request.reply.send(Err(error.to_string()));
                }
                return;
            }
        }
    }
}

async fn append_entries_transaction(
    db: &mongodb::Database,
    key: &[u8],
    entries: Vec<BillingLedgerEntry>,
) -> Result<Vec<BillingLedgerEntry>, mongodb::error::Error> {
    let mut session = db.client().start_session().await?;
    let database = std::sync::Arc::new(db.clone());
    let entries = std::sync::Arc::new(entries);
    let batch_key = std::sync::Arc::new(Zeroizing::new(key.to_vec()));
    session
        .start_transaction()
        .write_concern(
            mongodb::options::WriteConcern::builder()
                .w(mongodb::options::Acknowledgment::Majority)
                .journal(true)
                .build(),
        )
        .and_run2(async move |session| {
            let result =
                append_entries_with_key(&database, session, (*entries).clone(), &batch_key).await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })
        .await
}

async fn append_requests_individually(
    db: &mongodb::Database,
    key: &[u8],
    requests: Vec<AppendRequest>,
) {
    for request in requests {
        let result = append_one_with_retry(db, key, request.entry.clone()).await;
        let _ = request
            .reply
            .send(result.map_err(|error| error.to_string()));
    }
}

async fn append_one_with_retry(
    db: &mongodb::Database,
    key: &[u8],
    entry: BillingLedgerEntry,
) -> Result<BillingLedgerEntry, mongodb::error::Error> {
    let deadline = tokio::time::Instant::now() + APPEND_DEADLINE;
    let mut attempt = 0;
    loop {
        match append_entries_transaction(db, key, vec![entry.clone()]).await {
            Ok(mut saved) => return Ok(saved.remove(0)),
            Err(error)
                if is_duplicate_key_error(&error) && tokio::time::Instant::now() < deadline =>
            {
                attempt += 1;
                sleep_before_retry(attempt).await;
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn compute_entry_hash(entry: &BillingLedgerEntry, key: &[u8]) -> String {
    hmac_sha256_hex(key, &canonical_entry_bytes(entry))
}

fn canonical_entry_bytes(entry: &BillingLedgerEntry) -> Vec<u8> {
    if entry.event_type == BillingLedgerEventType::AccountingV2 {
        // Version marker + length-prefixed fields; posting order is committed.
        let mut output = b"nyxid:billing-ledger:v2\0".to_vec();
        let fields = [
            entry.seq.to_string(),
            entry.prev_hash.clone(),
            entry.id.clone(),
            entry.owner_id.clone(),
            entry.reference_id.clone(),
            entry.movement.clone().unwrap_or_default(),
            entry.transaction_id.clone().unwrap_or_default(),
            entry
                .layer
                .map(|l| l.as_transaction_suffix().into())
                .unwrap_or_default(),
            entry.metric.map(|m| m.as_str().into()).unwrap_or_default(),
            entry.service_slug.clone().unwrap_or_default(),
            entry.model.clone().unwrap_or_default(),
            entry.quantity.map(|n| n.to_string()).unwrap_or_default(),
            entry
                .amount_credits
                .map(|n| n.to_string())
                .unwrap_or_default(),
            entry
                .amount_micros
                .map(|n| n.to_string())
                .unwrap_or_default(),
            entry
                .balance_credits
                .map(|n| n.to_string())
                .unwrap_or_default(),
            entry.dedupe_key.clone().unwrap_or_default(),
            entry.wallet_id.clone().unwrap_or_default(),
            entry.created_at.timestamp_millis().to_string(),
            entry.postings.len().to_string(),
        ];
        fn field(output: &mut Vec<u8>, value: &str) {
            output.extend_from_slice(&(value.len() as u64).to_be_bytes());
            output.extend_from_slice(value.as_bytes());
        }
        for value in &fields {
            field(&mut output, value);
        }
        for posting in &entry.postings {
            field(&mut output, &posting.account);
            field(
                &mut output,
                match posting.side {
                    PostingSide::Debit => "debit",
                    PostingSide::Credit => "credit",
                },
            );
            field(&mut output, &posting.amount.to_string());
        }
        return output;
    }
    let mut fields = vec![
        entry.seq.to_string(),
        entry.prev_hash.clone(),
        entry.id.clone(),
        entry.event_type.as_str().to_string(),
        entry.owner_id.clone(),
        entry.reference_id.clone(),
        entry.transaction_id.clone().unwrap_or_default(),
        entry
            .layer
            .map(|layer| layer.as_transaction_suffix().to_string())
            .unwrap_or_default(),
        entry
            .metric
            .map(|metric| metric.as_str().to_string())
            .unwrap_or_default(),
        entry.service_slug.clone().unwrap_or_default(),
        entry.model.clone().unwrap_or_default(),
        entry
            .quantity
            .map(|quantity| quantity.to_string())
            .unwrap_or_default(),
        entry
            .amount_credits
            .map(|credits| credits.to_string())
            .unwrap_or_default(),
        entry
            .balance_credits
            .map(|credits| credits.to_string())
            .unwrap_or_default(),
        entry.dedupe_key.clone().unwrap_or_default(),
        entry.wallet_id.clone().unwrap_or_default(),
        entry.created_at.timestamp_millis().to_string(),
    ];
    // Existing event hashes predate exact microcredit movements. Append the
    // extension only for newly introduced event types so every historical
    // usage/top-up/wallet entry retains its original canonical bytes.
    if entry.event_type.uses_extended_encoding() {
        fields.push(
            entry
                .amount_micros
                .map(|micros| micros.to_string())
                .unwrap_or_default(),
        );
    }
    join_record_fields(&fields)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingLedgerBreakKind {
    Unbalanced,
    Gap,
    LinkMismatch,
    HashMismatch,
    /// The newest audit-chain head anchor points past the surviving ledger
    /// head: entries were deleted from the tail of the chain.
    TailTruncated,
    /// The newest anchor event fails audit-chain hash validation: it was
    /// forged or corrupted, so the truncation check cannot be trusted.
    AnchorInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BillingLedgerBreak {
    pub break_seq: i64,
    pub break_kind: BillingLedgerBreakKind,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingLedgerStatus {
    Ok,
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BillingLedgerVerifyReport {
    pub status: BillingLedgerStatus,
    pub checked_count: u64,
    pub head_seq: Option<i64>,
    pub head_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_info: Option<BillingLedgerBreak>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_from_seq: Option<i64>,
}

#[derive(Debug, Clone)]
struct ChainTail {
    seq: i64,
    entry_hash: String,
}

/// Audit event type carrying billing-ledger head anchors. Anchoring the
/// head `(seq, entry_hash)` into the tamper-evident audit chain (plus the
/// server log line below) is what makes tail truncation detectable: a
/// shortened-but-valid chain no longer matches the newest anchor.
pub const HEAD_ANCHOR_EVENT_TYPE: &str = "billing_ledger_head_anchored";

/// Result of cross-checking the ledger head against the newest anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeadAnchorCheck {
    /// Ledger seq recorded by the newest valid anchor, if any exists.
    pub anchor_seq: Option<i64>,
    /// False when an anchor event exists but fails audit-chain hash
    /// validation (a forged or corrupted anchor); absent when no anchor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_info: Option<BillingLedgerBreak>,
}

/// Anchor the current ledger head into the audit chain when it advanced
/// past the newest anchor. Called from the reconcile sweep; also emits the
/// head to the server log so shipped logs form an external anchor.
pub async fn anchor_head(db: &mongodb::Database) -> AppResult<Option<i64>> {
    let Some(head) = read_tail(db).await? else {
        return Ok(None);
    };
    if let Some(anchor) = load_latest_anchor(db).await?
        && anchor.ledger_seq >= head.seq
    {
        return Ok(None);
    }

    audit_service::log_system_event(
        db.clone(),
        HEAD_ANCHOR_EVENT_TYPE,
        Some(serde_json::json!({
            "seq": head.seq,
            "head_hash": head.entry_hash,
        })),
    )
    .await?;
    tracing::info!(
        head_seq = head.seq,
        head_hash = %head.entry_hash,
        "billing ledger head anchored"
    );
    Ok(Some(head.seq))
}

/// Cross-check the ledger head against the newest audit-chain anchor.
/// `audit_key` is the audit-chain HMAC key, used to validate that the
/// anchor event itself was not forged or corrupted.
pub async fn check_head_anchor(
    db: &mongodb::Database,
    audit_key: &[u8],
) -> AppResult<HeadAnchorCheck> {
    let Some(anchor) = load_latest_anchor(db).await? else {
        return Ok(HeadAnchorCheck {
            anchor_seq: None,
            anchor_valid: None,
            break_info: None,
        });
    };

    let expected_hash = audit_chain_service::compute_entry_hash(&anchor.entry, audit_key)?;
    if anchor.entry.entry_hash.as_deref() != Some(expected_hash.as_str()) {
        // A forged anchor would otherwise silently disable the truncation
        // check, so it must surface as a chain break, not a side note.
        return Ok(HeadAnchorCheck {
            anchor_seq: Some(anchor.ledger_seq),
            anchor_valid: Some(false),
            break_info: Some(BillingLedgerBreak {
                break_seq: anchor.ledger_seq,
                break_kind: BillingLedgerBreakKind::AnchorInvalid,
                expected: expected_hash,
                actual: anchor.entry.entry_hash.unwrap_or_default(),
            }),
        });
    }

    let head = read_tail(db).await?;
    let truncated = match &head {
        None => true,
        Some(head) => head.seq < anchor.ledger_seq,
    };
    if truncated {
        return Ok(HeadAnchorCheck {
            anchor_seq: Some(anchor.ledger_seq),
            anchor_valid: Some(true),
            break_info: Some(BillingLedgerBreak {
                break_seq: anchor.ledger_seq,
                break_kind: BillingLedgerBreakKind::TailTruncated,
                expected: anchor.ledger_seq.to_string(),
                actual: head
                    .map(|head| head.seq.to_string())
                    .unwrap_or_else(|| "missing".to_string()),
            }),
        });
    }

    let break_info = match load_entry_by_seq(db, anchor.ledger_seq).await? {
        None => Some(BillingLedgerBreak {
            break_seq: anchor.ledger_seq,
            break_kind: BillingLedgerBreakKind::Gap,
            expected: anchor.ledger_seq.to_string(),
            actual: "missing".to_string(),
        }),
        Some(entry) if entry.entry_hash != anchor.head_hash => Some(BillingLedgerBreak {
            break_seq: anchor.ledger_seq,
            break_kind: BillingLedgerBreakKind::HashMismatch,
            expected: anchor.head_hash.clone(),
            actual: entry.entry_hash,
        }),
        Some(_) => None,
    };

    Ok(HeadAnchorCheck {
        anchor_seq: Some(anchor.ledger_seq),
        anchor_valid: Some(true),
        break_info,
    })
}

struct LatestAnchor {
    entry: AuditLog,
    ledger_seq: i64,
    head_hash: String,
}

/// Newest chained anchor event by audit-chain seq. Unchained events (no
/// audit seq) are ignored: without a chain position they carry no
/// integrity and a database-only attacker could insert them freely.
async fn load_latest_anchor(
    db: &mongodb::Database,
) -> Result<Option<LatestAnchor>, mongodb::error::Error> {
    let entry = db
        .collection::<AuditLog>(AUDIT_LOG)
        .find(doc! {
            "event_type": HEAD_ANCHOR_EVENT_TYPE,
            "seq": { "$exists": true },
        })
        .sort(doc! { "seq": -1 })
        .limit(1)
        .await?
        .try_next()
        .await?;

    Ok(entry.and_then(|entry| {
        let data = entry.event_data.clone()?;
        let ledger_seq = data.get("seq")?.as_i64()?;
        let head_hash = data.get("head_hash")?.as_str()?.to_string();
        Some(LatestAnchor {
            entry,
            ledger_seq,
            head_hash,
        })
    }))
}

pub async fn verify_chain(
    db: &mongodb::Database,
    key: &[u8],
    from_seq: Option<i64>,
    to_seq: Option<i64>,
    limit: Option<i64>,
) -> AppResult<BillingLedgerVerifyReport> {
    let from_seq = from_seq.unwrap_or(1);
    if from_seq < 1 {
        return Err(AppError::ValidationError(
            "from_seq must be greater than or equal to 1".to_string(),
        ));
    }

    if let Some(to_seq) = to_seq
        && to_seq < from_seq
    {
        return Err(AppError::ValidationError(
            "to_seq must be greater than or equal to from_seq".to_string(),
        ));
    }

    let limit = limit.unwrap_or(DEFAULT_VERIFY_LIMIT);
    if !(1..=MAX_VERIFY_LIMIT).contains(&limit) {
        return Err(AppError::ValidationError(format!(
            "limit must be between 1 and {MAX_VERIFY_LIMIT}"
        )));
    }

    let collection = db.collection::<BillingLedgerEntry>(BILLING_LEDGER);
    let tail = read_tail(db).await?;
    let head_seq = tail.as_ref().map(|tail| tail.seq);
    let head_hash = tail.map(|tail| tail.entry_hash);

    let mut filter = doc! { "seq": { "$gte": from_seq } };
    if let Some(to_seq) = to_seq {
        filter.insert("seq", doc! { "$gte": from_seq, "$lte": to_seq });
    }

    let entries: Vec<BillingLedgerEntry> = collection
        .find(filter)
        .sort(doc! { "seq": 1 })
        .limit(limit)
        .await?
        .try_collect()
        .await?;

    let mut checked_count = 0_u64;
    let mut expected_seq = from_seq;
    let mut expected_prev_hash = if from_seq == 1 {
        GENESIS_PREV_HASH.to_string()
    } else {
        match load_entry_by_seq(db, from_seq - 1).await? {
            Some(previous) => previous.entry_hash,
            None => {
                return Ok(broken_report(
                    checked_count,
                    head_seq,
                    head_hash,
                    BillingLedgerBreak {
                        break_seq: from_seq,
                        break_kind: BillingLedgerBreakKind::Gap,
                        expected: (from_seq - 1).to_string(),
                        actual: "missing".to_string(),
                    },
                ));
            }
        }
    };

    for entry in &entries {
        if entry.seq != expected_seq {
            return Ok(broken_report(
                checked_count,
                head_seq,
                head_hash,
                BillingLedgerBreak {
                    break_seq: expected_seq,
                    break_kind: BillingLedgerBreakKind::Gap,
                    expected: expected_seq.to_string(),
                    actual: entry.seq.to_string(),
                },
            ));
        }

        if entry.prev_hash != expected_prev_hash {
            return Ok(broken_report(
                checked_count,
                head_seq,
                head_hash,
                BillingLedgerBreak {
                    break_seq: expected_seq,
                    break_kind: BillingLedgerBreakKind::LinkMismatch,
                    expected: expected_prev_hash,
                    actual: entry.prev_hash.clone(),
                },
            ));
        }

        if validate_postings(entry).is_err() {
            return Ok(broken_report(
                checked_count,
                head_seq,
                head_hash,
                BillingLedgerBreak {
                    break_seq: entry.seq,
                    break_kind: BillingLedgerBreakKind::Unbalanced,
                    expected: "balanced v2 postings".into(),
                    actual: "invalid postings".into(),
                },
            ));
        }
        let expected_entry_hash = compute_entry_hash(entry, key);
        if entry.entry_hash != expected_entry_hash {
            return Ok(broken_report(
                checked_count,
                head_seq,
                head_hash,
                BillingLedgerBreak {
                    break_seq: expected_seq,
                    break_kind: BillingLedgerBreakKind::HashMismatch,
                    expected: expected_entry_hash,
                    actual: entry.entry_hash.clone(),
                },
            ));
        }

        checked_count += 1;
        expected_seq += 1;
        expected_prev_hash = entry.entry_hash.clone();
    }

    let next_from_seq = if entries.len() as i64 == limit
        && to_seq.is_none_or(|to_seq| expected_seq <= to_seq)
        && head_seq.is_some_and(|head_seq| expected_seq <= head_seq)
    {
        Some(expected_seq)
    } else {
        None
    };

    Ok(BillingLedgerVerifyReport {
        status: BillingLedgerStatus::Ok,
        checked_count,
        head_seq,
        head_hash,
        break_info: None,
        next_from_seq,
    })
}

async fn read_tail(db: &mongodb::Database) -> Result<Option<ChainTail>, mongodb::error::Error> {
    let tail = db
        .collection::<BillingLedgerEntry>(BILLING_LEDGER)
        .find(doc! { "seq": { "$exists": true, "$type": "long" } })
        .read_concern(mongodb::options::ReadConcern::majority())
        .sort(doc! { "seq": -1 })
        .limit(1)
        .await?
        .try_next()
        .await?;

    Ok(tail.map(|entry| ChainTail {
        seq: entry.seq,
        entry_hash: entry.entry_hash,
    }))
}

async fn load_entry_by_seq(
    db: &mongodb::Database,
    seq: i64,
) -> Result<Option<BillingLedgerEntry>, mongodb::error::Error> {
    db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
        .find_one(doc! { "seq": seq })
        .await
}

fn broken_report(
    checked_count: u64,
    head_seq: Option<i64>,
    head_hash: Option<String>,
    break_info: BillingLedgerBreak,
) -> BillingLedgerVerifyReport {
    BillingLedgerVerifyReport {
        status: BillingLedgerStatus::Broken,
        checked_count,
        head_seq,
        head_hash,
        break_info: Some(break_info),
        next_from_seq: None,
    }
}

/// Length-prefixed join: `<len>:<bytes>` per field, separated by 0x1E.
/// The prefix makes the encoding injective even when a field value itself
/// contains the separator, so adjacent user-influenced fields (slug,
/// model) cannot be re-split into a different tuple with the same bytes.
fn join_record_fields(fields: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push(RECORD_SEPARATOR);
        }
        out.extend_from_slice(field.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(field.as_bytes());
    }
    out
}

fn hmac_sha256_hex(key: &[u8], input: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(input);
    hex::encode(mac.finalize().into_bytes())
}

fn truncate_to_bson_millis(ts: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ts.timestamp_millis())
        .single()
        .expect("timestamp_millis from DateTime<Utc> is valid")
}

pub(super) fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match error.kind.as_ref() {
        ErrorKind::Write(WriteFailure::WriteError(error)) => error.code == 11000,
        ErrorKind::Command(error) => error.code == 11000,
        ErrorKind::InsertMany(error) => error.write_errors.as_ref().is_some_and(|errors| {
            !errors.is_empty() && errors.iter().all(|error| error.code == 11000)
        }),
        ErrorKind::BulkWrite(error) => {
            !error.write_errors.is_empty()
                && error.write_errors.values().all(|error| error.code == 11000)
        }
        _ => false,
    }
}

async fn sleep_before_retry(attempt: usize) {
    let base_ms = 5_u64.saturating_mul(1_u64 << attempt.min(5));
    let jitter_ms = rand::thread_rng().gen_range(0..=10);
    tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter_ms)).await;
}

/// Atomic opening/adjustment append with the caller's balance mutation. The
/// transaction runner retries write conflicts against the shared chain tail.
pub(super) async fn append_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    entry: BillingLedgerEntry,
) -> AppResult<()> {
    let key = billing_ledger_hmac_key()
        .ok_or_else(|| AppError::Internal("billing ledger key is unavailable".into()))?;
    append_with_key(db, session, entry, key).await?;
    Ok(())
}

async fn append_with_key(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    entry: BillingLedgerEntry,
    key: &[u8],
) -> AppResult<BillingLedgerEntry> {
    Ok(append_entries_with_key(db, session, vec![entry], key)
        .await?
        .remove(0))
}

async fn append_entries_with_key(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    entries_to_append: Vec<BillingLedgerEntry>,
    key: &[u8],
) -> AppResult<Vec<BillingLedgerEntry>> {
    for entry in &entries_to_append {
        validate_postings(entry)?;
    }
    let entries = db.collection::<BillingLedgerEntry>(BILLING_LEDGER);
    let tail = entries
        .find_one(doc! {})
        .sort(doc! { "seq": -1 })
        .session(&mut *session)
        .await?;
    let dedupe_keys: Vec<_> = entries_to_append
        .iter()
        .filter_map(|entry| entry.dedupe_key.clone())
        .collect();
    let mut existing = std::collections::HashMap::new();
    if !dedupe_keys.is_empty() {
        let mut cursor = entries
            .find(doc! { "dedupe_key": { "$in": &dedupe_keys } })
            .session(&mut *session)
            .await?;
        let found: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
        for entry in found {
            if let Some(dedupe) = entry.dedupe_key.clone() {
                existing.insert(dedupe, entry);
            }
        }
    }
    let mut next_seq = tail.as_ref().map_or(Ok(1), |tail| {
        tail.seq
            .checked_add(1)
            .ok_or(crate::models::credits::CreditsError)
    })?;
    let mut previous_hash = tail.map_or_else(|| GENESIS_PREV_HASH.to_string(), |t| t.entry_hash);
    let mut saved = Vec::with_capacity(entries_to_append.len());
    let mut new_entries = Vec::new();
    let mut batch_deduped: std::collections::HashMap<String, BillingLedgerEntry> =
        std::collections::HashMap::new();
    for mut entry in entries_to_append {
        if let Some(dedupe) = entry.dedupe_key.as_ref()
            && let Some(existing) = existing.get(dedupe)
        {
            saved.push(existing.clone());
            continue;
        }
        if let Some(dedupe) = entry.dedupe_key.as_ref()
            && let Some(inserted) = batch_deduped.get(dedupe)
        {
            saved.push(inserted.clone());
            continue;
        }
        validate_postings(&entry)?;
        entry.seq = next_seq;
        entry.prev_hash = previous_hash;
        entry.created_at = truncate_to_bson_millis(entry.created_at);
        entry.entry_hash = compute_entry_hash(&entry, key);
        previous_hash = entry.entry_hash.clone();
        next_seq = next_seq
            .checked_add(1)
            .ok_or(crate::models::credits::CreditsError)?;
        saved.push(entry.clone());
        if let Some(dedupe) = entry.dedupe_key.clone() {
            batch_deduped.insert(dedupe, entry.clone());
        }
        new_entries.push(entry);
    }
    if !new_entries.is_empty() {
        entries
            .insert_many(new_entries.clone())
            .session(&mut *session)
            .await?;
    }
    // Each posting and its account checkpoint commit atomically. Account checks
    // need one indexed point read, independent of the account's history length.
    let checkpoints = db.collection::<mongodb::bson::Document>("billing_account_balances");
    let mut deltas = std::collections::BTreeMap::new();
    for entry in &new_entries {
        for posting in &entry.postings {
            if !is_reconciled_account(&posting.account) {
                continue;
            }
            let delta = match posting.side {
                PostingSide::Debit => posting.amount,
                PostingSide::Credit => -posting.amount,
            };
            let (previous, _) = deltas
                .get(&posting.account)
                .copied()
                .unwrap_or((Credits::ZERO, 0));
            deltas.insert(
                posting.account.clone(),
                (previous.checked_add(delta)?, entry.seq),
            );
        }
    }
    let accounts: Vec<_> = deltas.keys().cloned().collect();
    let mut previous_checkpoints = std::collections::HashMap::new();
    if !accounts.is_empty() {
        let mut cursor = checkpoints
            .find(doc! { "_id": { "$in": &accounts } })
            .session(&mut *session)
            .await?;
        let rows: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
        for row in rows {
            let account = row
                .get_str("_id")
                .map_err(|_| AppError::Internal("billing checkpoint has no account".into()))?
                .to_owned();
            previous_checkpoints.insert(account, authenticated_checkpoint(&row, key)?);
        }
    }
    let mut checkpoint_updates = Vec::with_capacity(deltas.len());
    for (account, (delta, through_seq)) in deltas {
        let balance = previous_checkpoints
            .get(&account)
            .map(|(balance, _)| *balance)
            .unwrap_or(Credits::ZERO);
        let balance = balance.checked_add(delta)?;
        checkpoint_updates.push(
            mongodb::options::UpdateOneModel::builder()
                .namespace(mongodb::Namespace::new(
                    db.name(),
                    "billing_account_balances",
                ))
                .filter(doc! { "_id": &account })
                .update(doc! { "$set": {
                    "balance": balance, "through_seq": through_seq,
                    "signature": checkpoint_signature(&account, through_seq, balance, key),
                }})
                .upsert(true)
                .build(),
        );
    }
    if !checkpoint_updates.is_empty() {
        db.client()
            .bulk_write(checkpoint_updates)
            .session(&mut *session)
            .await?;
    }
    Ok(saved)
}

fn is_reconciled_account(account: &str) -> bool {
    account.starts_with("wallet:") || account.starts_with("grant:")
}

fn checkpoint_signature(account: &str, seq: i64, balance: Credits, key: &[u8]) -> String {
    let mut bytes = b"nyxid:billing-account:v1\0".to_vec();
    for value in [account.to_string(), seq.to_string(), balance.to_string()] {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    hmac_sha256_hex(key, &bytes)
}

pub(super) fn authenticated_checkpoint(
    row: &mongodb::bson::Document,
    key: &[u8],
) -> AppResult<(Credits, i64)> {
    let invalid = || AppError::Internal("invalid billing account checkpoint".into());
    let account = row.get_str("_id").map_err(|_| invalid())?;
    let seq = row.get_i64("through_seq").map_err(|_| invalid())?;
    let balance = Credits::from_bson(
        row.get("balance").cloned().ok_or_else(invalid)?,
        crate::models::credits::SCALE,
    )?;
    if row.get_str("signature").ok()
        != Some(checkpoint_signature(account, seq, balance, key).as_str())
    {
        return Err(invalid());
    }
    Ok((balance, seq))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::connect_test_database;

    const TEST_KEY: &[u8; 32] = &[7u8; 32];

    fn entry(reference: &str) -> BillingLedgerEntry {
        BillingLedgerEntry {
            id: Uuid::new_v4().to_string(),
            seq: 0,
            prev_hash: String::new(),
            entry_hash: String::new(),
            event_type: BillingLedgerEventType::UsageSettled,
            movement: None,
            postings: Vec::new(),
            owner_id: "owner-1".to_string(),
            reference_id: reference.to_string(),
            transaction_id: Some(format!("{reference}-platform")),
            layer: Some(crate::models::usage_meter::BillingLayer::Platform),
            metric: Some(crate::models::service_billing::BillingMetric::Tokens),
            service_slug: Some("chrono-llm".to_string()),
            model: None,
            quantity: Some(100),
            amount_credits: Some(5),
            amount_micros: None,
            balance_credits: None,
            dedupe_key: None,
            wallet_id: Some("wallet-1".to_string()),
            created_at: Utc::now(),
        }
    }

    fn v2_entry(reference: &str, debit_account: &str) -> BillingLedgerEntry {
        exact_entry(
            "owner-1",
            reference,
            "test_transfer",
            format!("test:{reference}"),
            transfer(
                debit_account.to_owned(),
                "platform:test".to_owned(),
                Credits::from_whole(1),
            ),
            None,
        )
    }

    #[test]
    fn canonical_bytes_are_stable_and_field_sensitive() {
        let mut a = entry("req-1");
        a.created_at = truncate_to_bson_millis(a.created_at);
        a.seq = 1;
        a.prev_hash = GENESIS_PREV_HASH.to_string();
        let hash = compute_entry_hash(&a, TEST_KEY);
        assert_eq!(hash, compute_entry_hash(&a, TEST_KEY));

        let mut tampered = a.clone();
        tampered.amount_credits = Some(6);
        assert_ne!(hash, compute_entry_hash(&tampered, TEST_KEY));

        let mut tampered = a.clone();
        tampered.owner_id = "owner-2".to_string();
        assert_ne!(hash, compute_entry_hash(&tampered, TEST_KEY));
    }

    #[test]
    fn extended_microcredit_encoding_preserves_legacy_hashes() {
        let mut legacy = entry("legacy");
        legacy.created_at = truncate_to_bson_millis(legacy.created_at);
        legacy.seq = 1;
        legacy.prev_hash = GENESIS_PREV_HASH.to_string();
        let original = compute_entry_hash(&legacy, TEST_KEY);
        legacy.amount_micros = Some(123_456);
        assert_eq!(
            original,
            compute_entry_hash(&legacy, TEST_KEY),
            "fields introduced after the legacy event must not rewrite its canonical bytes"
        );

        let mut grant = legacy;
        grant.event_type = BillingLedgerEventType::GrantConsumed;
        let exact = compute_entry_hash(&grant, TEST_KEY);
        grant.amount_micros = Some(123_457);
        assert_ne!(exact, compute_entry_hash(&grant, TEST_KEY));
    }

    #[test]
    fn new_money_event_names_are_stable() {
        assert_eq!(BillingLedgerEventType::GrantIssued.as_str(), "grant_issued");
        assert_eq!(
            BillingLedgerEventType::GrantConsumed.as_str(),
            "grant_consumed"
        );
        assert_eq!(
            BillingLedgerEventType::GrantExpired.as_str(),
            "grant_expired"
        );
        assert_eq!(
            BillingLedgerEventType::GrantRevoked.as_str(),
            "grant_revoked"
        );
        assert_eq!(
            BillingLedgerEventType::TopupExpired.as_str(),
            "topup_expired"
        );
    }

    #[tokio::test]
    async fn append_links_entries_and_verify_reports_ok() {
        let Some(db) = connect_test_database("billing_ledger_append_verify").await else {
            return;
        };

        let first = append_chained_entry(&db, entry("req-1"), TEST_KEY)
            .await
            .expect("append first");
        assert_eq!(first.seq, 1);
        assert_eq!(first.prev_hash, GENESIS_PREV_HASH);

        let second = append_chained_entry(&db, entry("req-2"), TEST_KEY)
            .await
            .expect("append second");
        assert_eq!(second.seq, 2);
        assert_eq!(second.prev_hash, first.entry_hash);

        let report = verify_chain(&db, TEST_KEY, None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, BillingLedgerStatus::Ok);
        assert_eq!(report.checked_count, 2);
        assert_eq!(report.head_seq, Some(2));
    }

    #[tokio::test]
    async fn verify_detects_mutation_deletion_and_relink() {
        let Some(db) = connect_test_database("billing_ledger_tamper").await else {
            return;
        };

        for reference in ["req-1", "req-2", "req-3"] {
            append_chained_entry(&db, entry(reference), TEST_KEY)
                .await
                .expect("append");
        }
        let collection = db.collection::<BillingLedgerEntry>(BILLING_LEDGER);

        // Mutating a stored amount breaks the entry hash at that seq.
        collection
            .update_one(
                doc! { "seq": 2 },
                doc! { "$set": { "amount_credits": 999 } },
            )
            .await
            .expect("tamper update");
        let report = verify_chain(&db, TEST_KEY, None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, BillingLedgerStatus::Broken);
        let break_info = report.break_info.expect("break info");
        assert_eq!(break_info.break_seq, 2);
        assert_eq!(break_info.break_kind, BillingLedgerBreakKind::HashMismatch);

        // Deleting the tampered entry leaves a detectable gap instead.
        collection
            .delete_one(doc! { "seq": 2 })
            .await
            .expect("tamper delete");
        let report = verify_chain(&db, TEST_KEY, None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, BillingLedgerStatus::Broken);
        assert_eq!(
            report.break_info.expect("break info").break_kind,
            BillingLedgerBreakKind::Gap
        );

        // Re-sequencing the tail cannot repair the chain: the link no
        // longer matches the surviving predecessor.
        collection
            .update_one(doc! { "seq": 3 }, doc! { "$set": { "seq": 2 } })
            .await
            .expect("tamper reseq");
        let report = verify_chain(&db, TEST_KEY, None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, BillingLedgerStatus::Broken);
        assert_eq!(
            report.break_info.expect("break info").break_kind,
            BillingLedgerBreakKind::LinkMismatch
        );
    }

    const AUDIT_TEST_KEY: &[u8; 32] = &[2u8; 32];

    #[tokio::test]
    async fn anchor_head_detects_tail_truncation() {
        let Some(db) = connect_test_database("billing_ledger_anchor").await else {
            return;
        };
        crate::services::audit_service::init_audit_chain_hmac_key(Zeroizing::new(*AUDIT_TEST_KEY));

        for reference in ["req-1", "req-2", "req-3"] {
            append_chained_entry(&db, entry(reference), TEST_KEY)
                .await
                .expect("append");
        }

        assert_eq!(anchor_head(&db).await.expect("anchor"), Some(3));
        // Re-anchoring an unchanged head is a no-op.
        assert_eq!(anchor_head(&db).await.expect("re-anchor"), None);

        let check = check_head_anchor(&db, AUDIT_TEST_KEY)
            .await
            .expect("check anchored");
        assert_eq!(check.anchor_seq, Some(3));
        assert_eq!(check.anchor_valid, Some(true));
        assert!(check.break_info.is_none());

        // New entries appended after the anchor are fine (anchor lag).
        append_chained_entry(&db, entry("req-4"), TEST_KEY)
            .await
            .expect("append after anchor");
        let check = check_head_anchor(&db, AUDIT_TEST_KEY)
            .await
            .expect("check lag");
        assert!(check.break_info.is_none());

        // Truncating the tail past the anchor leaves a shorter chain that
        // still verifies link-by-link, but no longer matches the anchor.
        db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .delete_many(doc! { "seq": { "$gte": 3 } })
            .await
            .expect("truncate tail");
        let report = verify_chain(&db, TEST_KEY, None, None, None)
            .await
            .expect("verify truncated");
        assert_eq!(
            report.status,
            BillingLedgerStatus::Ok,
            "walk alone is blind"
        );
        let check = check_head_anchor(&db, AUDIT_TEST_KEY)
            .await
            .expect("check truncated");
        let break_info = check.break_info.expect("truncation break");
        assert_eq!(break_info.break_kind, BillingLedgerBreakKind::TailTruncated);
        assert_eq!(break_info.break_seq, 3);
    }

    #[tokio::test]
    async fn forged_anchor_is_flagged_invalid() {
        let Some(db) = connect_test_database("billing_ledger_forged_anchor").await else {
            return;
        };

        append_chained_entry(&db, entry("req-1"), TEST_KEY)
            .await
            .expect("append");

        // An attacker with database access chains an anchor under a key of
        // their choosing; validation against the real audit key rejects it.
        let forged = AuditLog {
            id: Uuid::new_v4().to_string(),
            user_id: None,
            event_type: HEAD_ANCHOR_EVENT_TYPE.to_string(),
            event_data: Some(serde_json::json!({ "seq": 1, "head_hash": "bogus" })),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
            seq: None,
            prev_hash: None,
            entry_hash: None,
            created_at: Utc::now(),
        };
        audit_chain_service::append_chained_entry(&db, forged, &[9u8; 32])
            .await
            .expect("forged append");

        let check = check_head_anchor(&db, AUDIT_TEST_KEY)
            .await
            .expect("check forged");
        assert_eq!(check.anchor_valid, Some(false));
        assert_eq!(
            check.break_info.expect("forged anchor break").break_kind,
            BillingLedgerBreakKind::AnchorInvalid
        );
    }

    #[tokio::test]
    async fn verify_rejects_wrong_key() {
        let Some(db) = connect_test_database("billing_ledger_wrong_key").await else {
            return;
        };

        append_chained_entry(&db, entry("req-1"), TEST_KEY)
            .await
            .expect("append");
        let report = verify_chain(&db, &[9u8; 32], None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, BillingLedgerStatus::Broken);
    }

    #[tokio::test]
    async fn corrupt_checkpoint_does_not_poison_group_commit() {
        let Some(db) = connect_test_database("billing_ledger_group_isolation").await else {
            return;
        };
        db.collection::<Document>("billing_account_balances")
            .insert_one(doc! {
                "_id": "wallet:bad",
                "balance": Credits::from_whole(1),
                "through_seq": 0_i64,
                "signature": "corrupt",
            })
            .await
            .expect("insert corrupt checkpoint");

        // Submit an explicit group so scheduling cannot separate the corrupt
        // checkpoint from healthy accounts into different transactions.
        let mut requests = Vec::new();
        let mut replies = Vec::new();
        for (reference, account) in [
            ("bad", "wallet:bad"),
            ("good", "wallet:good"),
            ("also-good", "grant:good"),
        ] {
            let (reply, result) = tokio::sync::oneshot::channel();
            requests.push(AppendRequest {
                entry: v2_entry(reference, account),
                reply,
            });
            replies.push(result);
        }
        append_batch_with_retry(&db, TEST_KEY, requests).await;
        let mut replies = replies.into_iter();
        let bad_result = replies.next().unwrap().await.unwrap();
        let good_result = replies.next().unwrap().await.unwrap();
        assert!(replies.next().unwrap().await.unwrap().is_ok());
        assert!(bad_result.is_err(), "the corrupt account must fail closed");
        assert!(
            good_result.is_ok(),
            "a valid group member must still commit"
        );
        assert_eq!(
            db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
                .count_documents(doc! {})
                .await
                .expect("count committed entries"),
            2
        );
        assert_eq!(
            db.collection::<Document>("billing_account_balances")
                .count_documents(doc! { "_id": "wallet:good" })
                .await
                .expect("count good checkpoint"),
            1
        );
        let verified = verify_chain(&db, TEST_KEY, None, None, None).await.unwrap();
        assert_eq!(verified.status, BillingLedgerStatus::Ok);
        assert_eq!(verified.checked_count, 2);
    }

    #[tokio::test]
    async fn closed_batch_worker_is_replaced_and_retried() {
        let Some(db) = connect_test_database("billing_ledger_closed_worker").await else {
            return;
        };
        let key_id = hex::encode(Sha256::digest(TEST_KEY));
        let slot_key = format!("{}:{key_id}", db.name());
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(rx);
        let closed = std::sync::Arc::new(AppendBatcher { tx });
        APPEND_BATCHERS
            .get_or_init(Default::default)
            .insert(slot_key.clone(), closed.clone());

        let mut invalid = v2_entry("invalid", "wallet:invalid");
        invalid.postings.pop();
        assert!(append_chained_entry(&db, invalid, TEST_KEY).await.is_err());
        let current = APPEND_BATCHERS.get().unwrap().get(&slot_key).unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&current, &closed),
            "validation must precede enqueue"
        );
        drop(current);

        let saved = append_chained_entry(&db, entry("after-worker-close"), TEST_KEY)
            .await
            .expect("fresh worker append");
        assert_eq!(saved.seq, 1);
    }
}
