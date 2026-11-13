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

use crate::errors::{AppError, AppResult};
use crate::models::chain_verify_status::{
    CHAIN_AUDIT_LOG, CHAIN_BILLING_LEDGER, COLLECTION_NAME as CHAIN_VERIFY_STATUS,
    ChainVerifyOutcome, ChainVerifyStatus,
};
use crate::services::audit_chain_service;
use crate::services::billing::ledger;

const VERIFY_CHUNK: i64 = 10_000;
const MAX_BROKEN_STATUS_RETRIES: usize = 3;

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
    let previous = load_status(db, CHAIN_AUDIT_LOG).await?;
    let from_seq = resume_seq(previous.as_ref());
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
        revision: Some(uuid::Uuid::new_v4().to_string()),
        id: CHAIN_AUDIT_LOG.to_string(),
        outcome,
        cursor_seq,
        head_seq: report.head_seq,
        checked_entries: report.checked_count as i64,
        last_full_pass_at: full_pass_timestamp(previous.as_ref(), wrapped, &outcome),
        break_seq,
        break_kind,
        break_detail,
        anchor_seq: None,
        anchor_valid: None,
        pre_chain_count: Some(report.pre_chain_count as i64),
        last_run_at: Utc::now(),
        updated_at: Utc::now(),
    };
    persist_status(db, previous.as_ref(), status).await
}

async fn verify_ledger_chunk(
    db: &mongodb::Database,
    ledger_key: &[u8],
    audit_key: &[u8],
) -> AppResult<ChainVerifyStatus> {
    let previous = load_status(db, CHAIN_BILLING_LEDGER).await?;
    let from_seq = resume_seq(previous.as_ref());
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
        revision: Some(uuid::Uuid::new_v4().to_string()),
        id: CHAIN_BILLING_LEDGER.to_string(),
        outcome,
        cursor_seq,
        head_seq: report.head_seq,
        checked_entries: report.checked_count as i64,
        last_full_pass_at: full_pass_timestamp(previous.as_ref(), wrapped, &outcome),
        break_seq,
        break_kind,
        break_detail,
        anchor_seq: anchor.anchor_seq,
        anchor_valid: anchor.anchor_valid,
        pre_chain_count: None,
        last_run_at: Utc::now(),
        updated_at: Utc::now(),
    };
    persist_status(db, previous.as_ref(), status).await
}

/// Where this run's walk starts: the break seq while broken (so a break
/// is re-checked until it clears), otherwise the persisted rolling cursor.
fn resume_seq(previous: Option<&ChainVerifyStatus>) -> i64 {
    previous
        .map(|previous| match previous.outcome {
            ChainVerifyOutcome::Broken => previous.break_seq.unwrap_or(1),
            ChainVerifyOutcome::Ok => previous.cursor_seq,
        })
        .unwrap_or(1)
        .max(1)
}

/// Carry metadata from the SAME snapshot the final write compares against.
fn full_pass_timestamp(
    previous: Option<&ChainVerifyStatus>,
    wrapped: bool,
    outcome: &ChainVerifyOutcome,
) -> Option<chrono::DateTime<Utc>> {
    if wrapped && matches!(outcome, ChainVerifyOutcome::Ok) {
        return Some(Utc::now());
    }
    previous.and_then(|previous| previous.last_full_pass_at)
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

#[cfg(test)]
tokio::task_local! {
    static PERSIST_PAUSE: (std::sync::Arc<tokio::sync::Notify>, std::sync::Arc<tokio::sync::Notify>);
    static PUBLISH_ATTEMPT_PAUSE: (std::sync::Arc<tokio::sync::Notify>, std::sync::Arc<tokio::sync::Notify>);
}

/// Clean scans publish only against their starting snapshot. A detected break
/// is escalated before publication and survives conflicting clean results.
async fn persist_status(
    db: &mongodb::Database,
    previous: Option<&ChainVerifyStatus>,
    status: ChainVerifyStatus,
) -> AppResult<ChainVerifyStatus> {
    escalate_if_broken(&status);
    #[cfg(test)]
    if status.id == CHAIN_BILLING_LEDGER
        && let Ok((entered, resume)) = PERSIST_PAUSE.try_with(Clone::clone)
    {
        entered.notify_one();
        resume.notified().await;
    }
    let result = publish_status(db, previous, &status).await;
    if status.outcome == ChainVerifyOutcome::Broken
        && let Err(error) = &result
    {
        tracing::error!(
            chain = %status.id,
            break_seq = status.break_seq,
            error = %error,
            "hash chain break could not be persisted; integrity status may be stale"
        );
    }
    result
}

async fn publish_status(
    db: &mongodb::Database,
    previous: Option<&ChainVerifyStatus>,
    status: &ChainVerifyStatus,
) -> AppResult<ChainVerifyStatus> {
    let collection = db.collection::<ChainVerifyStatus>(CHAIN_VERIFY_STATUS);
    let mut previous = previous.cloned();
    for _ in 0..=MAX_BROKEN_STATUS_RETRIES {
        #[cfg(test)]
        if status.id == CHAIN_BILLING_LEDGER
            && let Ok((entered, resume)) = PUBLISH_ATTEMPT_PAUSE.try_with(Clone::clone)
        {
            entered.notify_one();
            resume.notified().await;
        }
        let published = if let Some(previous) = &previous {
            // UUID revisions prevent ABA between new writers; comparing legacy
            // fields also detects an old writer that omitted the revision.
            let filter = bson::to_document(previous).map_err(|error| {
                AppError::Internal(format!("encode chain status fence: {error}"))
            })?;
            collection.replace_one(filter, status).await?.matched_count == 1
        } else {
            match collection.insert_one(status).await {
                Ok(_) => true,
                Err(error)
                    if matches!(error.kind.as_ref(),
                    mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write))
                        if write.code == 11000) =>
                {
                    false
                }
                Err(error) => return Err(error.into()),
            }
        };
        if published {
            return Ok(status.clone());
        }
        previous = load_status(db, &status.id).await?;
        if status.outcome == ChainVerifyOutcome::Ok {
            // A losing clean scan never retries, even against another Ok.
            return previous.ok_or_else(|| {
                AppError::Internal("chain status disappeared after concurrent publication".into())
            });
        }
        if let Some(current) = &previous
            && current.outcome == ChainVerifyOutcome::Broken
            && current.break_seq.unwrap_or(1) <= status.break_seq.unwrap_or(1)
        {
            // Keep the earliest break; retain the stored record on a tie.
            // Apply this only after contention, so a fresh scan can advance
            // past a repaired break or clear it using its original snapshot.
            return Ok(current.clone());
        }
    }
    Err(AppError::Internal(format!(
        "hash chain break publication contention exceeded {} attempts",
        MAX_BROKEN_STATUS_RETRIES + 1
    )))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::billing_ledger::{
        BillingLedgerEntry, BillingLedgerEventType, COLLECTION_NAME as BILLING_LEDGER,
    };
    use crate::test_utils::connect_test_database;
    use uuid::Uuid;

    const AUDIT_KEY: &[u8; 32] = &[2u8; 32];
    const LEDGER_KEY: &[u8; 32] = &[3u8; 32];

    pub(super) fn ledger_entry(reference: &str) -> BillingLedgerEntry {
        BillingLedgerEntry {
            id: Uuid::new_v4().to_string(),
            seq: 0,
            prev_hash: String::new(),
            entry_hash: String::new(),
            event_type: BillingLedgerEventType::UsageSettled,
            movement: None,
            postings: Vec::new(),
            owner_id: "owner-sweep".to_string(),
            reference_id: reference.to_string(),
            transaction_id: None,
            layer: None,
            metric: None,
            service_slug: None,
            model: None,
            quantity: Some(1),
            amount_credits: Some(5),
            amount_micros: None,
            balance_credits: None,
            dedupe_key: None,
            wallet_id: Some("wallet-sweep".to_string()),
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn sweep_reports_ok_then_detects_tamper_and_recovers() {
        let Some(db) = connect_test_database("chain_verify_sweep").await else {
            return;
        };

        for reference in ["req-1", "req-2"] {
            ledger::append_chained_entry(&db, ledger_entry(reference), LEDGER_KEY)
                .await
                .expect("append ledger entry");
        }

        let report = run_once(&db, AUDIT_KEY, LEDGER_KEY).await.expect("run");
        assert_eq!(report.billing_ledger.outcome, ChainVerifyOutcome::Ok);
        assert_eq!(report.billing_ledger.head_seq, Some(2));
        assert!(report.billing_ledger.last_full_pass_at.is_some());
        assert_eq!(report.audit.outcome, ChainVerifyOutcome::Ok);

        // Tamper mid-chain: the next run flags it and pins the cursor.
        db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .update_one(
                doc! { "seq": 1 },
                doc! { "$set": { "amount_credits": 999 } },
            )
            .await
            .expect("tamper");
        let report = run_once(&db, AUDIT_KEY, LEDGER_KEY).await.expect("run");
        assert_eq!(report.billing_ledger.outcome, ChainVerifyOutcome::Broken);
        assert_eq!(report.billing_ledger.break_seq, Some(1));
        let persisted = load_status(&db, CHAIN_BILLING_LEDGER)
            .await
            .expect("load status")
            .expect("status persisted");
        assert_eq!(persisted.outcome, ChainVerifyOutcome::Broken);

        // Restoring the original value clears the break on the next run.
        db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .update_one(doc! { "seq": 1 }, doc! { "$set": { "amount_credits": 5 } })
            .await
            .expect("restore");
        let report = run_once(&db, AUDIT_KEY, LEDGER_KEY).await.expect("run");
        assert_eq!(report.billing_ledger.outcome, ChainVerifyOutcome::Ok);
    }
}

#[cfg(test)]
mod race_tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn chain_status_race_stale_ok_cannot_replace_broken() {
        let db = crate::test_utils::connect_transaction_test_database("chain_status_race").await;
        let entered = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let slow_db = db.clone();
        let slow = tokio::spawn(
            PERSIST_PAUSE.scope((entered.clone(), resume.clone()), async move {
                run_once(&slow_db, &[2; 32], &[3; 32]).await.unwrap()
            }),
        );
        entered.notified().await;
        // B observed a break after A finished its clean scan but before A published.
        let mut broken = run_once(&db, &[2; 32], &[3; 32])
            .await
            .unwrap()
            .billing_ledger;
        broken.outcome = ChainVerifyOutcome::Broken;
        broken.break_seq = Some(1);
        broken.break_kind = Some("hash_mismatch".into());
        db.collection::<ChainVerifyStatus>(CHAIN_VERIFY_STATUS)
            .replace_one(doc! {"_id": CHAIN_BILLING_LEDGER}, &broken)
            .await
            .unwrap();
        resume.notify_one();
        let result = slow.await.unwrap();
        assert_eq!(
            load_status(&db, CHAIN_BILLING_LEDGER)
                .await
                .unwrap()
                .unwrap()
                .outcome,
            ChainVerifyOutcome::Broken,
            "the newer break must remain durable"
        );
        assert_eq!(
            result.billing_ledger.outcome,
            ChainVerifyOutcome::Broken,
            "the manual caller must receive the winner, not its rejected result"
        );
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use crate::models::{
        billing_ledger::COLLECTION_NAME as LEDGER,
        user::{User, UserType},
    };
    use crate::services::role_service;
    use crate::test_utils::{
        connect_transaction_test_database, test_app_state, test_auth_user, test_user,
    };
    use std::sync::Arc;
    use tokio::sync::Notify;

    async fn setup(label: &str) -> (mongodb::Database, String) {
        let db = connect_transaction_test_database(label).await;
        ledger::append_chained_entry(&db, super::tests::ledger_entry("race-entry"), &[3; 32])
            .await
            .unwrap();
        role_service::seed_system_roles(&db).await.unwrap();
        let roles = role_service::get_platform_role_ids(&db).await.unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        user.role_ids.push(roles.admin);
        db.collection::<User>("users")
            .insert_one(user)
            .await
            .unwrap();
        run_once(&db, &[2; 32], &[3; 32]).await.unwrap();
        (db, id)
    }

    async fn invoke(db: &mongodb::Database, admin: &str, manual: bool) -> ChainVerifyOutcome {
        if manual {
            crate::handlers::admin::run_chain_verification(
                axum::extract::State(test_app_state(db.clone())),
                test_auth_user(admin),
            )
            .await
            .unwrap()
            .0
            .chains
            .into_iter()
            .find(|s| s.chain == CHAIN_BILLING_LEDGER)
            .unwrap()
            .outcome
        } else {
            run_once(db, &[2; 32], &[3; 32])
                .await
                .unwrap()
                .billing_ledger
                .outcome
        }
    }

    async fn manual_timer_race(slow_manual: bool) {
        let (db, admin) = setup("manual_timer_race").await;
        let entered = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let slow_db = db.clone();
        let slow_admin = admin.clone();
        let slow = tokio::spawn(
            PERSIST_PAUSE.scope((entered.clone(), resume.clone()), async move {
                invoke(&slow_db, &slow_admin, slow_manual).await
            }),
        );
        tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
            .await
            .unwrap();
        db.collection::<mongodb::bson::Document>(LEDGER)
            .update_one(doc! {"seq":1}, doc! {"$set":{"amount_credits":999}})
            .await
            .unwrap();
        assert_eq!(
            invoke(&db, &admin, !slow_manual).await,
            ChainVerifyOutcome::Broken
        );
        resume.notify_one();
        assert_eq!(slow.await.unwrap(), ChainVerifyOutcome::Broken);
        assert_eq!(
            load_status(&db, CHAIN_BILLING_LEDGER)
                .await
                .unwrap()
                .unwrap()
                .outcome,
            ChainVerifyOutcome::Broken
        );
        db.collection::<mongodb::bson::Document>(LEDGER)
            .update_one(doc! {"seq":1}, doc! {"$set":{"amount_credits":5}})
            .await
            .unwrap();
        assert_eq!(invoke(&db, &admin, true).await, ChainVerifyOutcome::Ok);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn paused_manual_returns_timer_winner_and_repaired_chain_can_clear() {
        manual_timer_race(true).await;
    }
    #[tokio::test]
    async fn paused_timer_cannot_replace_manual_broken_result() {
        manual_timer_race(false).await;
    }

    #[tokio::test]
    async fn legacy_replacement_invalidates_snapshot_and_new_scan_can_publish() {
        let (db, _) = setup("chain_mixed_writer").await;
        let previous = load_status(&db, CHAIN_BILLING_LEDGER)
            .await
            .unwrap()
            .unwrap();
        let mut stale = previous.clone();
        stale.revision = Some(uuid::Uuid::new_v4().to_string());
        let mut legacy = bson::to_document(&previous).unwrap();
        legacy.remove("revision");
        legacy.insert("outcome", "broken");
        legacy.insert("break_seq", 1_i64);
        legacy.insert("break_kind", "hash_mismatch");
        db.collection::<mongodb::bson::Document>(CHAIN_VERIFY_STATUS)
            .replace_one(doc! {"_id":CHAIN_BILLING_LEDGER}, &legacy)
            .await
            .unwrap();
        assert!(
            load_status(&db, CHAIN_BILLING_LEDGER)
                .await
                .unwrap()
                .unwrap()
                .revision
                .is_none()
        );
        let winner = persist_status(&db, Some(&previous), stale).await.unwrap();
        assert_eq!(winner.outcome, ChainVerifyOutcome::Broken);
        // The next scan starts from the old writer's status and can replace it.
        let repaired = verify_ledger_chunk(&db, &[3; 32], &[2; 32]).await.unwrap();
        assert_eq!(repaired.outcome, ChainVerifyOutcome::Ok);
        assert!(repaired.revision.is_some());
        // Compatibility limit: an old writer still ignores our fence entirely.
        db.collection::<mongodb::bson::Document>(CHAIN_VERIFY_STATUS)
            .replace_one(doc! {"_id":CHAIN_BILLING_LEDGER}, legacy)
            .await
            .unwrap();
        assert_eq!(
            load_status(&db, CHAIN_BILLING_LEDGER)
                .await
                .unwrap()
                .unwrap()
                .outcome,
            ChainVerifyOutcome::Broken
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn same_millisecond_publications_are_fenced_for_both_chains() {
        let (db, _) = setup("chain_revision_aba").await;
        for chain in [CHAIN_AUDIT_LOG, CHAIN_BILLING_LEDGER] {
            let previous = load_status(&db, chain).await.unwrap().unwrap();
            let mut newer = previous.clone();
            newer.revision = Some(uuid::Uuid::new_v4().to_string());
            // Deliberately equal timestamps, head and cursor. Only revision differs.
            let stored = persist_status(&db, Some(&previous), newer.clone())
                .await
                .unwrap();
            assert_eq!(stored.revision, newer.revision);
            let mut stale = previous.clone();
            stale.revision = Some(uuid::Uuid::new_v4().to_string());
            assert_eq!(
                persist_status(&db, Some(&previous), stale).await.unwrap(),
                newer
            );
        }
        db.drop().await.unwrap();
    }

    async fn losing_break_race(broken_manual: bool) {
        use tracing::instrument::WithSubscriber;

        let (db, admin) = setup("losing_break_race").await;
        let clean_entered = Arc::new(Notify::new());
        let clean_resume = Arc::new(Notify::new());
        let clean_db = db.clone();
        let clean_admin = admin.clone();
        let clean = tokio::spawn(
            PERSIST_PAUSE.scope((clean_entered.clone(), clean_resume.clone()), async move {
                invoke(&clean_db, &clean_admin, !broken_manual).await
            }),
        );
        tokio::time::timeout(std::time::Duration::from_secs(10), clean_entered.notified())
            .await
            .unwrap();
        db.collection::<mongodb::bson::Document>(LEDGER)
            .update_one(doc! {"seq":1}, doc! {"$set":{"amount_credits":999}})
            .await
            .unwrap();

        let capture = tempfile::NamedTempFile::new().unwrap();
        let writer = capture.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::ERROR)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        let broken_entered = Arc::new(Notify::new());
        let broken_resume = Arc::new(Notify::new());
        let broken_db = db.clone();
        let broken_admin = admin.clone();
        let broken = tokio::spawn(
            PERSIST_PAUSE
                .scope(
                    (broken_entered.clone(), broken_resume.clone()),
                    async move { invoke(&broken_db, &broken_admin, broken_manual).await },
                )
                .with_subscriber(subscriber),
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            broken_entered.notified(),
        )
        .await
        .unwrap();
        // Both scans read the same starting status. Publish the clean scan first.
        clean_resume.notify_one();
        assert_eq!(clean.await.unwrap(), ChainVerifyOutcome::Ok);
        broken_resume.notify_one();
        let observed = broken.await.unwrap();
        let stored = load_status(&db, CHAIN_BILLING_LEDGER)
            .await
            .unwrap()
            .unwrap();
        let logs = std::fs::read_to_string(capture.path()).unwrap();
        let escalated = logs.lines().any(|line| {
            line.contains("ERROR")
                && line.contains("hash chain integrity check FAILED; possible tampering")
                && line.contains("break_seq=1")
        });
        assert_eq!(
            (observed, stored.outcome, escalated),
            (ChainVerifyOutcome::Broken, ChainVerifyOutcome::Broken, true),
            "a break detected by the losing scan must be returned, stored and escalated"
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round2_losing_manual_break_is_stored_and_escalated() {
        losing_break_race(true).await;
    }

    #[tokio::test]
    async fn round2_losing_timer_break_is_stored_and_escalated() {
        losing_break_race(false).await;
    }
    async fn late_break_after_repair(broken_manual: bool) {
        let (db, admin) = setup("late_break_after_repair").await;
        db.collection::<mongodb::bson::Document>(LEDGER)
            .update_one(doc! {"seq":1}, doc! {"$set":{"amount_credits":999}})
            .await
            .unwrap();
        let entered = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let slow_db = db.clone();
        let slow_admin = admin.clone();
        let broken = tokio::spawn(
            PERSIST_PAUSE.scope((entered.clone(), resume.clone()), async move {
                invoke(&slow_db, &slow_admin, broken_manual).await
            }),
        );
        tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
            .await
            .unwrap();
        db.collection::<mongodb::bson::Document>(LEDGER)
            .update_one(doc! {"seq":1}, doc! {"$set":{"amount_credits":5}})
            .await
            .unwrap();
        assert_eq!(
            invoke(&db, &admin, !broken_manual).await,
            ChainVerifyOutcome::Ok
        );
        resume.notify_one();
        assert_eq!(broken.await.unwrap(), ChainVerifyOutcome::Broken);
        let alarm = load_status(&db, CHAIN_BILLING_LEDGER)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(alarm.outcome, ChainVerifyOutcome::Broken);
        assert_eq!(alarm.break_seq, Some(1));
        assert_eq!(resume_seq(Some(&alarm)), 1);
        // The late alarm deliberately wins; rechecking its repaired break clears it.
        assert_eq!(invoke(&db, &admin, true).await, ChainVerifyOutcome::Ok);
        assert_eq!(
            load_status(&db, CHAIN_BILLING_LEDGER)
                .await
                .unwrap()
                .unwrap()
                .outcome,
            ChainVerifyOutcome::Ok
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round2_late_manual_break_after_repair_clears_on_next_scan() {
        late_break_after_repair(true).await;
    }

    #[tokio::test]
    async fn round2_late_timer_break_after_repair_clears_on_next_scan() {
        late_break_after_repair(false).await;
    }

    #[tokio::test]
    async fn round2_concurrent_breaks_keep_lower_sequence_in_either_order() {
        use tracing::instrument::WithSubscriber;
        for lower_first in [false, true] {
            let (db, _) = setup("two_breaks").await;
            ledger::append_chained_entry(&db, super::tests::ledger_entry("second"), &[3; 32])
                .await
                .unwrap();
            run_once(&db, &[2; 32], &[3; 32]).await.unwrap();
            db.collection::<mongodb::bson::Document>(LEDGER)
                .update_one(doc! {"seq":2}, doc! {"$set":{"amount_credits":999}})
                .await
                .unwrap();
            let capture = tempfile::NamedTempFile::new().unwrap();
            let writer = capture.reopen().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::ERROR)
                .with_writer(move || writer.try_clone().unwrap())
                .finish();
            let high_entered = Arc::new(Notify::new());
            let high_resume = Arc::new(Notify::new());
            let high_db = db.clone();
            let high = tokio::spawn(
                PERSIST_PAUSE
                    .scope((high_entered.clone(), high_resume.clone()), async move {
                        verify_ledger_chunk(&high_db, &[3; 32], &[2; 32])
                            .await
                            .unwrap()
                    })
                    .with_subscriber(subscriber),
            );
            tokio::time::timeout(std::time::Duration::from_secs(10), high_entered.notified())
                .await
                .unwrap();
            db.collection::<mongodb::bson::Document>(LEDGER)
                .update_one(doc! {"seq":1}, doc! {"$set":{"amount_credits":999}})
                .await
                .unwrap();
            let low_entered = Arc::new(Notify::new());
            let low_resume = Arc::new(Notify::new());
            let low_db = db.clone();
            let low = tokio::spawn(PERSIST_PAUSE.scope(
                (low_entered.clone(), low_resume.clone()),
                async move {
                    verify_ledger_chunk(&low_db, &[3; 32], &[2; 32])
                        .await
                        .unwrap()
                },
            ));
            tokio::time::timeout(std::time::Duration::from_secs(10), low_entered.notified())
                .await
                .unwrap();
            if lower_first {
                low_resume.notify_one();
                assert_eq!(low.await.unwrap().break_seq, Some(1));
                high_resume.notify_one();
                assert_eq!(high.await.unwrap().break_seq, Some(1));
            } else {
                high_resume.notify_one();
                assert_eq!(high.await.unwrap().break_seq, Some(2));
                low_resume.notify_one();
                assert_eq!(low.await.unwrap().break_seq, Some(1));
            }
            let stored = load_status(&db, CHAIN_BILLING_LEDGER)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.outcome, ChainVerifyOutcome::Broken);
            assert_eq!(stored.break_seq, Some(1));
            let logs = std::fs::read_to_string(capture.path()).unwrap();
            assert!(
                logs.lines().any(|line| {
                    line.contains("ERROR")
                        && line.contains("hash chain integrity check FAILED; possible tampering")
                        && line.contains("break_seq=2")
                }),
                "the higher break must log its own observation even when it returns the lower winner"
            );
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn round2_equal_break_keeps_stored_record_for_both_chains() {
        let (db, _) = setup("equal_breaks").await;
        for chain in [CHAIN_AUDIT_LOG, CHAIN_BILLING_LEDGER] {
            let previous = load_status(&db, chain).await.unwrap().unwrap();
            let mut first = previous.clone();
            first.outcome = ChainVerifyOutcome::Broken;
            first.break_seq = Some(1);
            first.break_kind = Some("hash_mismatch".into());
            first.break_detail = Some("first observation".into());
            first.revision = Some(uuid::Uuid::new_v4().to_string());
            let mut second = first.clone();
            second.break_detail = Some("second observation".into());
            second.revision = Some(uuid::Uuid::new_v4().to_string());
            persist_status(&db, Some(&previous), first.clone())
                .await
                .unwrap();
            assert_eq!(
                persist_status(&db, Some(&previous), second).await.unwrap(),
                first
            );
            assert_eq!(load_status(&db, chain).await.unwrap().unwrap(), first);
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round2_manual_break_retry_exhaustion_returns_error_and_logs_failure() {
        use tracing::instrument::WithSubscriber;
        let (db, admin) = setup("break_retry_exhaustion").await;
        db.collection::<mongodb::bson::Document>(LEDGER)
            .update_one(doc! {"seq":1}, doc! {"$set":{"amount_credits":999}})
            .await
            .unwrap();
        let capture = tempfile::NamedTempFile::new().unwrap();
        let writer = capture.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::ERROR)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        let entered = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let slow_db = db.clone();
        let run = tokio::spawn(
            PUBLISH_ATTEMPT_PAUSE
                .scope((entered.clone(), resume.clone()), async move {
                    crate::handlers::admin::run_chain_verification(
                        axum::extract::State(test_app_state(slow_db)),
                        test_auth_user(&admin),
                    )
                    .await
                })
                .with_subscriber(subscriber),
        );
        for _ in 0..=MAX_BROKEN_STATUS_RETRIES {
            tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
                .await
                .unwrap();
            // Invalidate the real CAS snapshot before every write, including retries.
            db.collection::<mongodb::bson::Document>(CHAIN_VERIFY_STATUS)
                .update_one(
                    doc! {"_id":CHAIN_BILLING_LEDGER},
                    doc! {"$set":{"revision":uuid::Uuid::new_v4().to_string()}},
                )
                .await
                .unwrap();
            resume.notify_one();
        }
        let error = match tokio::time::timeout(std::time::Duration::from_secs(10), run)
            .await
            .unwrap()
            .unwrap()
        {
            Err(error) => error,
            Ok(_) => panic!("an unstored break must never return a clean admin response"),
        };
        assert!(
            matches!(&error, AppError::Internal(message) if message.contains("break publication contention"))
        );
        let logs = std::fs::read_to_string(capture.path()).unwrap();
        for message in [
            "hash chain integrity check FAILED; possible tampering",
            "hash chain break could not be persisted; integrity status may be stale",
        ] {
            assert!(
                logs.lines()
                    .any(|line| line.contains("ERROR") && line.contains(message))
            );
        }
        db.drop().await.unwrap();
    }
}
