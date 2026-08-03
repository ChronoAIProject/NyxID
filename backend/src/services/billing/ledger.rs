//! Tamper-evident billing ledger.
//!
//! Mirrors the audit-log hash chain (`services/audit_chain_service.rs`):
//! every money-moving billing event appends an immutable entry whose
//! HMAC-SHA256 `entry_hash` commits to the previous entry, so mutating,
//! deleting, or inserting history breaks verification at that seq. The
//! operational billing rows (`usage_meter`, `billing_topup_sessions`)
//! stay mutable by design; this ledger is the append-only journal of
//! their money-moving transitions.

use std::sync::OnceLock;

use chrono::{DateTime, TimeZone, Utc};
use futures::TryStreamExt;
use hmac::{Hmac, Mac};
use mongodb::bson::doc;
use rand::Rng;
use serde::Serialize;
use sha2::Sha256;
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
const MAX_APPEND_ATTEMPTS: usize = 8;
pub const DEFAULT_VERIFY_LIMIT: i64 = 10_000;
pub const MAX_VERIFY_LIMIT: i64 = 10_000;

static BILLING_LEDGER_HMAC_KEY: OnceLock<Zeroizing<[u8; 32]>> = OnceLock::new();

pub fn init_billing_ledger_hmac_key(key: Zeroizing<[u8; 32]>) {
    if BILLING_LEDGER_HMAC_KEY.set(key).is_err() {
        tracing::warn!("billing-ledger HMAC key was already initialized");
    }
}

fn billing_ledger_hmac_key() -> Option<&'static [u8]> {
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

/// Record a first-apply charged usage settlement on the ledger, off the
/// settlement path (spawned, like audit logging). Best-effort: an append
/// failure must never fail or roll back the settlement itself.
pub fn record_usage_settled_async(db: mongodb::Database, row: UsageMeterRow, credits: i64) {
    tokio::spawn(async move {
        let entry = BillingLedgerEntry {
            id: Uuid::new_v4().to_string(),
            seq: 0,
            prev_hash: String::new(),
            entry_hash: String::new(),
            event_type: BillingLedgerEventType::UsageSettled,
            owner_id: row.billing_owner_id.clone(),
            reference_id: row.id.clone(),
            transaction_id: Some(row.transaction_id.clone()),
            layer: Some(row.layer),
            metric: Some(row.metric),
            service_slug: row.service_slug.clone(),
            model: row.model.clone(),
            quantity: row.quantity,
            amount_credits: Some(credits),
            balance_credits: None,
            dedupe_key: None,
            wallet_id: row.wallet_id.clone(),
            created_at: Utc::now(),
        };
        append_best_effort(&db, entry).await;
    });
}

/// Record a charge whose `released` transition committed through the
/// wallet-lock crash bridge, where only the row id is at hand.
pub fn record_usage_settled_by_row_id_async(db: mongodb::Database, row_id: String, credits: i64) {
    tokio::spawn(async move {
        match db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "_id": &row_id })
            .await
        {
            Ok(Some(row)) => {
                record_usage_settled_async(db, row, credits);
            }
            Ok(None) => {
                tracing::warn!(row_id = %row_id, "settled usage row missing for ledger entry");
            }
            Err(error) => {
                tracing::warn!(%error, row_id = %row_id, "failed to load settled usage row for ledger");
            }
        }
    });
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
    let entry = BillingLedgerEntry {
        id: Uuid::new_v4().to_string(),
        seq: 0,
        prev_hash: String::new(),
        entry_hash: String::new(),
        event_type: BillingLedgerEventType::TopupCreated,
        owner_id: owner_id.to_string(),
        reference_id: session_id.to_string(),
        transaction_id: None,
        layer: None,
        metric: None,
        service_slug: None,
        model: None,
        quantity: None,
        amount_credits: Some(amount_credits),
        balance_credits: None,
        dedupe_key: None,
        wallet_id: Some(lago_wallet_id.to_string()),
        created_at: Utc::now(),
    };
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
    balance_credits: i64,
    dedupe_key: Option<String>,
) {
    if let Some(key) = dedupe_key.as_deref() {
        match db
            .collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .count_documents(doc! { "event_type": "wallet_credited", "dedupe_key": key })
            .await
        {
            Ok(0) => {}
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(%error, "wallet-credited ledger dedupe check failed");
                return;
            }
        }
    }

    let entry = BillingLedgerEntry {
        id: Uuid::new_v4().to_string(),
        seq: 0,
        prev_hash: String::new(),
        entry_hash: String::new(),
        event_type: BillingLedgerEventType::WalletCredited,
        owner_id: owner_id.to_string(),
        reference_id: customer_id.to_string(),
        transaction_id: None,
        layer: None,
        metric: None,
        service_slug: None,
        model: None,
        quantity: None,
        amount_credits: None,
        balance_credits: Some(balance_credits),
        dedupe_key,
        wallet_id: None,
        created_at: Utc::now(),
    };
    append_best_effort(db, entry).await;
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
    mut entry: BillingLedgerEntry,
    key: &[u8],
) -> Result<BillingLedgerEntry, mongodb::error::Error> {
    let collection = db.collection::<BillingLedgerEntry>(BILLING_LEDGER);
    let mut last_error = None;

    for attempt in 1..=MAX_APPEND_ATTEMPTS {
        let tail = read_tail(db).await?;
        let (seq, prev_hash) = match tail {
            Some(tail) => (tail.seq + 1, tail.entry_hash),
            None => (1, GENESIS_PREV_HASH.to_string()),
        };

        entry.created_at = truncate_to_bson_millis(entry.created_at);
        entry.seq = seq;
        entry.prev_hash = prev_hash;
        entry.entry_hash = compute_entry_hash(&entry, key);

        match collection.insert_one(&entry).await {
            Ok(_) => return Ok(entry),
            Err(error) if is_duplicate_key_error(&error) && attempt < MAX_APPEND_ATTEMPTS => {
                last_error = Some(error);
                sleep_before_retry(attempt).await;
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        mongodb::error::Error::custom("billing ledger append exhausted retries")
    }))
}

pub fn compute_entry_hash(entry: &BillingLedgerEntry, key: &[u8]) -> String {
    hmac_sha256_hex(key, &canonical_entry_bytes(entry))
}

fn canonical_entry_bytes(entry: &BillingLedgerEntry) -> Vec<u8> {
    let fields = [
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
    join_record_fields(&fields)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingLedgerBreakKind {
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

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

async fn sleep_before_retry(attempt: usize) {
    let base_ms = 5_u64.saturating_mul(1_u64 << attempt.min(5));
    let jitter_ms = rand::thread_rng().gen_range(0..=10);
    tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter_ms)).await;
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
            owner_id: "owner-1".to_string(),
            reference_id: reference.to_string(),
            transaction_id: Some(format!("{reference}-platform")),
            layer: Some(crate::models::usage_meter::BillingLayer::Platform),
            metric: Some(crate::models::service_billing::BillingMetric::Tokens),
            service_slug: Some("chrono-llm".to_string()),
            model: None,
            quantity: Some(100),
            amount_credits: Some(5),
            balance_credits: None,
            dedupe_key: None,
            wallet_id: Some("wallet-1".to_string()),
            created_at: Utc::now(),
        }
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
}
