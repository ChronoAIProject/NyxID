//! Snapshot-consistent rolling reconciliation of journaled asset balances.
use crate::{
    errors::AppResult,
    models::{
        billing_ledger::{BillingLedgerEntry, COLLECTION_NAME as LEDGER},
        billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
        chain_verify_status::{COLLECTION_NAME as STATUS, ChainVerifyOutcome, ChainVerifyStatus},
        credit_grant::{COLLECTION_NAME as GRANTS, CreditGrant},
        credits::Credits,
    },
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    bson::{self, Document, doc},
    options::{FindOneAndUpdateOptions, ReadConcern, ReturnDocument},
};
use std::collections::BTreeMap;
pub const CHECK_ID: &str = "billing_accounts";
pub(super) const BATCH: i64 = 1_000;
const STATE: &str = "billing_account_verify_state";
const LEASE: &str = "billing_account_verify_lease";
/// One replica advances a pass in bounded batches, then the shared due time
/// keeps all replicas idle for the configured verification interval.
pub fn spawn(db: mongodb::Database, billing_enabled: bool, cadence_secs: u64) {
    if !billing_enabled {
        return;
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !super::exact_migration::ready(&db).await.unwrap_or(false) {
                continue;
            }
            if let Err(error) = run_scheduled_pass(&db, cadence_secs.max(5)).await {
                tracing::error!(%error, "billing account verification failed");
            }
        }
    });
}

async fn claim_pass(db: &mongodb::Database) -> AppResult<Option<String>> {
    let token = uuid::Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    let claimed = db
        .collection::<Document>(LEASE)
        .find_one_and_update(
            doc! {
                "_id": CHECK_ID,
                "$and": [
                    { "$or": [ { "lease_until": null }, { "lease_until": { "$lt": now } } ] },
                    { "$or": [ { "next_pass_at": null }, { "next_pass_at": { "$lte": now } } ] },
                ],
            },
            doc! {
                "$set": {
                    "token": &token,
                    "lease_until": bson::DateTime::from_chrono(
                        Utc::now() + chrono::Duration::seconds(300),
                    ),
                },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await;
    match claimed {
        Ok(Some(_)) => Ok(Some(token)),
        Ok(None) => Ok(None),
        Err(error) if super::ledger::is_duplicate_key_error(&error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn run_scheduled_pass(db: &mongodb::Database, cadence_secs: u64) -> AppResult<()> {
    let Some(token) = claim_pass(db).await? else {
        return Ok(());
    };
    let result = async {
        loop {
            if run_batch(db, Some(&token)).await? {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    }
    .await;
    // A deferred or broken pass also idles. It must not repeatedly rescan an
    // unchanged locked account merely because it cannot certify a full pass.
    db.collection::<Document>(LEASE)
        .update_one(
            doc! { "_id": CHECK_ID, "token": &token },
            doc! {
                "$unset": { "token": "", "lease_until": "" },
                "$set": {
                    "next_pass_at": bson::DateTime::from_chrono(
                        Utc::now()
                            + chrono::Duration::seconds(
                                i64::try_from(cadence_secs).unwrap_or(i64::MAX).min(86_400),
                            ),
                    ),
                },
            },
        )
        .await?;
    result
}

pub async fn run_once(db: &mongodb::Database) -> AppResult<()> {
    run_batch(db, None).await.map(|_| ())
}

async fn run_batch(db: &mongodb::Database, lease_token: Option<&str>) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    let database = db.clone();
    let lease_token = lease_token.map(str::to_owned);
    let result = session
        .start_transaction()
        .read_concern(ReadConcern::snapshot())
        .and_run2(async move |session| {
            let operation = async {
                if let Some(token) = &lease_token {
                    let renewed = database
                        .collection::<Document>(LEASE)
                        .update_one(
                            doc! {
                                "_id": CHECK_ID,
                                "token": token,
                                "lease_until": { "$gt": bson::DateTime::now() },
                            },
                            doc! {
                                "$set": {
                                    "lease_until": bson::DateTime::from_chrono(
                                        Utc::now() + chrono::Duration::seconds(300),
                                    ),
                                },
                            },
                        )
                        .session(&mut *session)
                        .await?;
                    if renewed.matched_count == 0 {
                        return Ok(None);
                    }
                }
                verify_snapshot(&database, session).await.map(Some)
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    let Some((status, wrapped)) = result else {
        return Ok(true);
    };
    if status.outcome == ChainVerifyOutcome::Broken {
        tracing::error!(detail = ?status.break_detail, "billing account reconciliation failed");
    }
    Ok(wrapped || status.outcome == ChainVerifyOutcome::Broken)
}

async fn verify_snapshot(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
) -> AppResult<(ChainVerifyStatus, bool)> {
    let state = db
        .collection::<Document>(STATE)
        .find_one(doc! { "_id": CHECK_ID })
        .session(&mut *session)
        .await?
        .unwrap_or_default();
    let mut next = doc! { "_id": CHECK_ID };
    let mut expected = BTreeMap::new();
    let mut deferred = 0;
    let mut wrapped = true;
    for collection in [WALLETS, GRANTS] {
        let done_key = format!("{collection}_done");
        if state.get_bool(&done_key).unwrap_or(false) {
            next.insert(done_key, true);
            next.insert(collection, "");
            continue;
        }
        let after = state.get_str(collection).unwrap_or("");
        let mut cursor = db
            .collection::<Document>(collection)
            .find(doc! { "_id": { "$gt": after } })
            .sort(doc! { "_id": 1 })
            .limit(BATCH)
            .session(&mut *session)
            .await?;
        let rows: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
        let last = if rows.len() == BATCH as usize {
            rows.last()
                .and_then(|row| row.get_str("_id").ok())
                .unwrap_or("")
        } else {
            ""
        };
        wrapped &= last.is_empty();
        next.insert(done_key, last.is_empty());
        next.insert(collection, last);
        for row in rows {
            // Locks and markers are durable work in progress. Check
            // these accounts on the next pass after recovery journals.
            if (collection == WALLETS && row.get_i32("exact_accounting_version").ok() != Some(2))
                || row.get_document("active_settlement").is_ok()
                || row.get_document("active_topup_expiry").is_ok()
            {
                deferred += 1;
                continue;
            }
            if collection == WALLETS {
                let wallet: BillingWallet = bson::from_document(row)
                    .map_err(|e| crate::errors::AppError::Internal(e.to_string()))?;
                expected.insert(
                    format!("wallet:{}", wallet.owner_id),
                    wallet
                        .balance_credits
                        .checked_sub(wallet.pending_lago_debits)?,
                );
            } else {
                let grant: CreditGrant = bson::from_document(row)
                    .map_err(|e| crate::errors::AppError::Internal(e.to_string()))?;
                if grant.issued_ledgered_at.is_none()
                    || (matches!(
                        grant.status,
                        crate::models::credit_grant::CreditGrantStatus::Expired
                            | crate::models::credit_grant::CreditGrantStatus::Revoked
                    ) && grant.terminal_ledgered_at.is_none())
                {
                    deferred += 1;
                    continue;
                }
                expected.insert(format!("grant:{}", grant.id), grant.remaining);
            }
        }
    }
    let accounts: Vec<_> = expected.keys().cloned().collect();
    let mut cursor = db
        .collection::<Document>("billing_account_balances")
        .find(doc! { "_id": { "$in": &accounts } })
        .session(&mut *session)
        .await?;
    let totals: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    let checkpoints: BTreeMap<_, _> = totals
        .into_iter()
        .map(|row| (row.get_str("_id").unwrap_or("").to_string(), row))
        .collect();
    let key = super::ledger::billing_ledger_hmac_key().ok_or_else(|| {
        crate::errors::AppError::Internal("billing ledger key unavailable".into())
    })?;
    let mut mismatch = None;
    for (account, stored) in &expected {
        let checkpoint = checkpoints
            .get(account)
            .map(|row| super::ledger::authenticated_checkpoint(row, key))
            .transpose();
        let (posted, through) = match checkpoint {
            Ok(value) => value.unwrap_or((Credits::ZERO, 0)),
            Err(error) => {
                mismatch = Some(format!("{account}: {error}"));
                break;
            }
        };
        // An authenticated older checkpoint is still a replay. The compound
        // account/seq index resolves its newest posting with one bounded seek.
        let latest = db
            .collection::<Document>(LEDGER)
            .find_one(doc! { "postings.account": account })
            .sort(doc! { "seq": -1 })
            .projection(doc! { "seq": 1 })
            .session(&mut *session)
            .await?;
        let latest_seq = latest
            .as_ref()
            .and_then(|row| row.get_i64("seq").ok())
            .unwrap_or(0);
        if *stored != posted || through != latest_seq {
            mismatch = Some(format!(
                "{account}: stored={stored}, journal={posted}, checkpoint={through}, latest={latest_seq}"
            ));
            break;
        }
    }
    let head = db
        .collection::<BillingLedgerEntry>(LEDGER)
        .find_one(doc! {})
        .sort(doc! { "seq": -1 })
        .session(&mut *session)
        .await?;
    let outcome = if mismatch.is_some() {
        ChainVerifyOutcome::Broken
    } else {
        ChainVerifyOutcome::Ok
    };
    let cycle_deferred = state.get_bool("deferred").unwrap_or(false) || deferred > 0;
    next.insert("deferred", !wrapped && cycle_deferred);
    if wrapped {
        // A smaller collection waits at its end until every
        // collection finishes this cycle. Independent wraparound
        // could otherwise prevent a complete pass indefinitely.
        for collection in [WALLETS, GRANTS] {
            next.insert(format!("{collection}_done"), false);
        }
    }
    if mismatch.is_none() {
        db.collection::<Document>(STATE)
            .replace_one(doc! { "_id": CHECK_ID }, next)
            .upsert(true)
            .session(&mut *session)
            .await?;
    }
    let now = Utc::now();
    let previous = db
        .collection::<ChainVerifyStatus>(STATUS)
        .find_one(doc! { "_id": CHECK_ID })
        .session(&mut *session)
        .await?;
    let status = ChainVerifyStatus {
        id: CHECK_ID.into(),
        outcome,
        cursor_seq: 1,
        head_seq: head.map(|h| h.seq),
        checked_entries: expected.len() as i64,
        last_full_pass_at: if wrapped && mismatch.is_none() && !cycle_deferred {
            Some(now)
        } else {
            previous.and_then(|p| p.last_full_pass_at)
        },
        break_seq: None,
        break_kind: mismatch.as_ref().map(|_| "account_balance_mismatch".into()),
        break_detail: mismatch.or_else(|| {
            (deferred > 0).then(|| format!("{deferred} accounts awaiting durable settlement"))
        }),
        anchor_seq: None,
        anchor_valid: None,
        pre_chain_count: None,
        last_run_at: now,
        updated_at: now,
    };
    db.collection::<ChainVerifyStatus>(STATUS)
        .replace_one(doc! { "_id": CHECK_ID }, &status)
        .upsert(true)
        .session(&mut *session)
        .await?;
    Ok((status, wrapped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scheduled_pass_is_leased_fenced_and_idles_after_deferred_accounts() {
        let db = crate::test_utils::connect_test_database("account_verify_lease")
            .await
            .unwrap();
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new([42; 32]));
        // A legacy wallet is deliberately deferred without requiring a model.
        db.collection::<Document>(WALLETS)
            .insert_one(doc! { "_id": "pending" })
            .await
            .unwrap();
        let (a, b) = tokio::join!(claim_pass(&db), claim_pass(&db));
        let claims: Vec<_> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
        assert_eq!(claims.len(), 1, "only one replica owns a pass");
        let stale = &claims[0];
        db.collection::<Document>(LEASE)
            .update_one(
                doc! { "_id": CHECK_ID },
                doc! { "$set": { "lease_until": bson::DateTime::from_millis(0) } },
            )
            .await
            .unwrap();
        let current = claim_pass(&db).await.unwrap().unwrap();
        assert!(run_batch(&db, Some(stale)).await.unwrap());
        assert!(
            db.collection::<Document>(STATUS)
                .find_one(doc! { "_id": CHECK_ID })
                .await
                .unwrap()
                .is_none(),
            "expired lease cannot advance cursors"
        );
        assert!(
            run_batch(&db, Some(&current)).await.unwrap(),
            "a deferred pass still ends"
        );
        db.collection::<Document>(LEASE)
            .delete_many(doc! {})
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_scheduled_pass(&db, 3600),
        )
        .await
        .unwrap()
        .unwrap();
        let status = db
            .collection::<ChainVerifyStatus>(STATUS)
            .find_one(doc! { "_id": CHECK_ID })
            .await
            .unwrap()
            .unwrap();
        assert!(status.last_full_pass_at.is_none());
        assert!(
            claim_pass(&db).await.unwrap().is_none(),
            "every replica observes the same idle interval"
        );
        let before = status.last_run_at;
        run_scheduled_pass(&db, 3600).await.unwrap();
        assert_eq!(
            db.collection::<ChainVerifyStatus>(STATUS)
                .find_one(doc! { "_id": CHECK_ID })
                .await
                .unwrap()
                .unwrap()
                .last_run_at,
            before
        );
        db.drop().await.unwrap();
    }
}
