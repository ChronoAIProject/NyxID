// Frozen from main 35ff0715 for mixed-version compatibility tests.
//! Automatic hash-chain verification sweep.
//!
//! Continuously re-verifies the audit-log and billing-ledger chains in
//! bounded rolling chunks: each run walks up to `VERIFY_CHUNK` entries
//! from a persisted cursor, wraps back to seq 1 after passing the head,
//! and upserts a per-chain `ChainVerifyStatus` document that the admin
//! integrity page reads. Any break is escalated with an error-level log
//! every run until it clears, so log-based alerting fires.
//!
//! A broken chain is re-checked from its break seq on subsequent runs
//! (a restore of the original row clears it); an intact chain keeps
//! rolling so earlier regions are periodically re-covered.

use chrono::Utc;
use mongodb::bson::doc;
use mongodb::options::ReplaceOptions;

use super::ledger;
use crate::errors::AppResult;
use crate::models::chain_verify_status::{
    CHAIN_AUDIT_LOG, CHAIN_BILLING_LEDGER, COLLECTION_NAME as CHAIN_VERIFY_STATUS,
    ChainVerifyOutcome, ChainVerifyStatus,
};
use crate::services::audit_chain_service;

const VERIFY_CHUNK: i64 = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainVerifyRunReport {
    pub audit: ChainVerifyStatus,
    pub billing_ledger: ChainVerifyStatus,
}

pub async fn run_once(
    db: &mongodb::Database,
    audit_key: &[u8],
    ledger_key: &[u8],
) -> AppResult<ChainVerifyRunReport> {
    let audit = verify_audit_chunk(db, audit_key).await?;
    let billing_ledger = verify_ledger_chunk(db, ledger_key, audit_key).await?;
    Ok(ChainVerifyRunReport {
        audit,
        billing_ledger,
    })
}

async fn verify_audit_chunk(db: &mongodb::Database, key: &[u8]) -> AppResult<ChainVerifyStatus> {
    let from_seq = resume_seq(db, CHAIN_AUDIT_LOG).await?;
    let report =
        audit_chain_service::verify_chain(db, key, Some(from_seq), None, Some(VERIFY_CHUNK))
            .await?;

    let (outcome, break_seq, break_kind, break_detail) = match &report.break_info {
        Some(break_info) => (
            ChainVerifyOutcome::Broken,
            Some(break_info.break_seq),
            kind_string(&break_info.break_kind),
            Some(format!(
                "expected {} actual {}",
                break_info.expected, break_info.actual
            )),
        ),
        None => (ChainVerifyOutcome::Ok, None, None, None),
    };
    let wrapped = matches!(report.status, audit_chain_service::AuditChainStatus::Ok)
        && report.next_from_seq.is_none();
    let cursor_seq = match (&outcome, report.next_from_seq) {
        (ChainVerifyOutcome::Broken, _) => break_seq.unwrap_or(1),
        (_, Some(next)) => next,
        (_, None) => 1,
    };

    let status = ChainVerifyStatus {
        id: CHAIN_AUDIT_LOG.to_string(),
        outcome,
        cursor_seq,
        head_seq: report.head_seq,
        checked_entries: report.checked_count as i64,
        last_full_pass_at: full_pass_timestamp(db, CHAIN_AUDIT_LOG, wrapped, &outcome).await?,
        break_seq,
        break_kind,
        break_detail,
        anchor_seq: None,
        anchor_valid: None,
        pre_chain_count: Some(report.pre_chain_count as i64),
        last_run_at: Utc::now(),
        updated_at: Utc::now(),
    };
    persist_status(db, &status).await?;
    escalate_if_broken(&status);
    Ok(status)
}

async fn verify_ledger_chunk(
    db: &mongodb::Database,
    ledger_key: &[u8],
    audit_key: &[u8],
) -> AppResult<ChainVerifyStatus> {
    let from_seq = resume_seq(db, CHAIN_BILLING_LEDGER).await?;
    let report =
        ledger::verify_chain(db, ledger_key, Some(from_seq), None, Some(VERIFY_CHUNK)).await?;
    let anchor = ledger::check_head_anchor(db, audit_key).await?;

    let break_info = report.break_info.as_ref().or(anchor.break_info.as_ref());
    let (outcome, break_seq, break_kind, break_detail) = match break_info {
        Some(break_info) => (
            ChainVerifyOutcome::Broken,
            Some(break_info.break_seq),
            kind_string(&break_info.break_kind),
            Some(format!(
                "expected {} actual {}",
                break_info.expected, break_info.actual
            )),
        ),
        None => (ChainVerifyOutcome::Ok, None, None, None),
    };
    let wrapped = matches!(outcome, ChainVerifyOutcome::Ok) && report.next_from_seq.is_none();
    let cursor_seq = match (&outcome, report.next_from_seq) {
        (ChainVerifyOutcome::Broken, _) => break_seq.unwrap_or(1),
        (_, Some(next)) => next,
        (_, None) => 1,
    };

    let status = ChainVerifyStatus {
        id: CHAIN_BILLING_LEDGER.to_string(),
        outcome,
        cursor_seq,
        head_seq: report.head_seq,
        checked_entries: report.checked_count as i64,
        last_full_pass_at: full_pass_timestamp(db, CHAIN_BILLING_LEDGER, wrapped, &outcome).await?,
        break_seq,
        break_kind,
        break_detail,
        anchor_seq: anchor.anchor_seq,
        anchor_valid: anchor.anchor_valid,
        pre_chain_count: None,
        last_run_at: Utc::now(),
        updated_at: Utc::now(),
    };
    persist_status(db, &status).await?;
    escalate_if_broken(&status);
    Ok(status)
}

/// Where this run's walk starts: the break seq while broken (so a break
/// is re-checked until it clears), otherwise the persisted rolling cursor.
async fn resume_seq(db: &mongodb::Database, chain_id: &str) -> AppResult<i64> {
    let previous = load_status(db, chain_id).await?;
    Ok(previous
        .map(|previous| match previous.outcome {
            ChainVerifyOutcome::Broken => previous.break_seq.unwrap_or(1),
            ChainVerifyOutcome::Ok => previous.cursor_seq,
        })
        .unwrap_or(1)
        .max(1))
}

/// Preserve the previous full-pass timestamp unless this run completed a
/// clean wrap past the head.
async fn full_pass_timestamp(
    db: &mongodb::Database,
    chain_id: &str,
    wrapped: bool,
    outcome: &ChainVerifyOutcome,
) -> AppResult<Option<chrono::DateTime<Utc>>> {
    if wrapped && matches!(outcome, ChainVerifyOutcome::Ok) {
        return Ok(Some(Utc::now()));
    }
    Ok(load_status(db, chain_id)
        .await?
        .and_then(|previous| previous.last_full_pass_at))
}

pub async fn load_status(
    db: &mongodb::Database,
    chain_id: &str,
) -> AppResult<Option<ChainVerifyStatus>> {
    db.collection::<ChainVerifyStatus>(CHAIN_VERIFY_STATUS)
        .find_one(doc! { "_id": chain_id })
        .await
        .map_err(Into::into)
}

async fn persist_status(db: &mongodb::Database, status: &ChainVerifyStatus) -> AppResult<()> {
    db.collection::<ChainVerifyStatus>(CHAIN_VERIFY_STATUS)
        .replace_one(doc! { "_id": &status.id }, status)
        .with_options(ReplaceOptions::builder().upsert(true).build())
        .await?;
    Ok(())
}

fn kind_string<T: serde::Serialize>(kind: &T) -> Option<String> {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
}

fn escalate_if_broken(status: &ChainVerifyStatus) {
    if status.outcome == ChainVerifyOutcome::Broken {
        tracing::error!(
            chain = %status.id,
            break_seq = status.break_seq,
            break_kind = status.break_kind.as_deref().unwrap_or("unknown"),
            "hash chain integrity check FAILED; possible tampering"
        );
    }
}

pub fn spawn_chain_verify_worker(
    db: mongodb::Database,
    audit_key: std::sync::Arc<zeroize::Zeroizing<[u8; 32]>>,
    ledger_key: std::sync::Arc<zeroize::Zeroizing<[u8; 32]>>,
    interval_secs: u64,
) {
    if interval_secs == 0 {
        return;
    }

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(error) = run_once(&db, audit_key.as_slice(), ledger_key.as_slice()).await {
                tracing::warn!(error = %error, "chain verification sweep failed");
            }
        }
    });
}
