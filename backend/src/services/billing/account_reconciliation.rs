//! Snapshot-consistent rolling reconciliation of journaled asset balances.
use crate::{
    errors::AppResult,
    models::{
        billing_ledger::{BillingLedgerEntry, COLLECTION_NAME as LEDGER},
        billing_wallet::COLLECTION_NAME as WALLETS,
        chain_verify_status::{COLLECTION_NAME as STATUS, ChainVerifyOutcome, ChainVerifyStatus},
        credit_grant::COLLECTION_NAME as GRANTS,
        credits::Credits,
    },
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    bson::{self, Document, doc},
    options::{FindOneAndUpdateOptions, ReturnDocument},
};
use std::collections::BTreeMap;
pub const CHECK_ID: &str = "billing_accounts";
pub(super) const BATCH: i64 = 1_000;
const STATE: &str = "billing_account_verify_state";
const LEASE: &str = "billing_account_verify_lease";
const PERIODS: &str = crate::models::usage_allowance_period::COLLECTION_NAME;
/// One replica advances a pass in bounded batches, then the shared due time
/// keeps all replicas idle for the configured verification interval.
/// Disabled billing or a zero verification interval spawns no runner.
pub fn spawn(db: mongodb::Database, billing_enabled: bool, cadence_secs: u64) {
    if !billing_enabled || cadence_secs == 0 {
        return;
    }
    crate::services::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let Some(_work) = crate::services::shutdown::process()
                .tick(&mut interval)
                .await
            else {
                break;
            };
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
    let release =
        crate::services::shutdown::process().register_release(crate::services::shutdown::Release {
            safe_after_stop: true,
            db: db.clone(),
            collection: LEASE,
            filter: doc! {"_id": CHECK_ID, "token": &token},
            update: doc! {"$unset": {"token": "", "lease_until": ""}},
        });
    let result = async {
        loop {
            if run_batch(db, Some(&token)).await?
                || crate::services::shutdown::process().is_draining()
            {
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
                                if crate::services::shutdown::process().is_draining() { 0 } else { i64::try_from(cadence_secs).unwrap_or(i64::MAX).min(86_400) },
                            ),
                    ),
                },
            },
        )
        .await?;
    release.completed();
    result
}

pub async fn run_once(db: &mongodb::Database) -> AppResult<()> {
    run_batch(db, None).await.map(|_| ())
}

async fn run_batch(db: &mongodb::Database, lease_token: Option<&str>) -> AppResult<bool> {
    let context = (db.clone(), lease_token.map(str::to_owned));
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |session, (database, lease_token)| {
            Box::pin(async move {
                if let Some(token) = lease_token
                    && !renew_pass(database, session, token).await?
                {
                    return Ok(None);
                }
                verify_snapshot(database, session, lease_token.is_some())
                    .await
                    .map(Some)
            })
        },
    )
    .await?;
    let Some((status, wrapped, repairs)) = result else {
        return Ok(true);
    };
    if status.outcome == ChainVerifyOutcome::Broken {
        tracing::error!(detail = ?status.break_detail, "billing account reconciliation failed");
    }
    if status.outcome == ChainVerifyOutcome::Ok
        && let Some(token) = lease_token
    {
        for candidate in repairs {
            if crate::services::shutdown::process().is_draining() {
                break;
            }
            let context = (db.clone(), token.to_owned(), candidate);
            let owned = crate::services::transaction_service::run_db_only(
                db,
                &context,
                |session, (database, token, candidate)| {
                    Box::pin(async move {
                        if !renew_pass(database, session, token).await? {
                            return Ok(false);
                        }
                        super::hold_conservation::repair(database, session, candidate).await?;
                        Ok(true)
                    })
                },
            )
            .await?;
            if !owned {
                return Ok(true);
            }
        }
    }
    Ok(wrapped || status.outcome == ChainVerifyOutcome::Broken)
}

async fn renew_pass(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    token: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(LEASE)
        .update_one(
            doc! {"_id": CHECK_ID, "token": token, "lease_until":{"$gt":bson::DateTime::now()}},
            doc! {"$set":{"lease_until":bson::DateTime::from_chrono(
                Utc::now() + chrono::Duration::seconds(300),
            )}},
        )
        .session(session)
        .await?
        .matched_count
        == 1)
}

async fn verify_snapshot(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    scheduled: bool,
) -> AppResult<(
    ChainVerifyStatus,
    bool,
    Vec<super::hold_conservation::Repair>,
)> {
    // Manual Integrity checks must not advance scheduled hold observations or
    // consume their lexical pages. Only the leased runner can confirm/repair.
    let state_id = if scheduled {
        CHECK_ID
    } else {
        "billing_accounts_manual"
    };
    let collections: &[&str] = if scheduled {
        &[WALLETS, GRANTS, PERIODS]
    } else {
        &[WALLETS, GRANTS]
    };
    let state = db
        .collection::<Document>(STATE)
        .find_one(doc! { "_id": state_id })
        .session(&mut *session)
        .await?
        .unwrap_or_default();
    let pass = state
        .get_str("hold_pass")
        .map(str::to_owned)
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let previous_pass = state.get_str("hold_previous_pass").unwrap_or("");
    let mut next =
        doc! { "_id": state_id, "hold_pass": &pass, "hold_previous_pass": previous_pass };
    let mut expected = BTreeMap::new();
    let mut deferred = 0;
    let mut repairs = Vec::new();
    let mut wrapped = true;
    for &collection in collections {
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
            .batch_size(BATCH as u32)
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
        let hold_page = if scheduled {
            Some(super::hold_conservation::load_page(db, session, collection, &rows).await?)
        } else {
            None
        };
        for row in rows {
            if let Some(page) = &hold_page
                && page.needs_detail(collection, &row)
            {
                repairs.extend(
                    super::hold_conservation::verify(
                        db,
                        session,
                        collection,
                        &row,
                        page.previous(collection, &row),
                        &pass,
                        previous_pass,
                    )
                    .await?,
                );
            }
            if collection == PERIODS {
                continue;
            }
            // Locks and markers are durable work in progress. Check
            // these accounts on the next pass after recovery journals.
            if (collection == WALLETS && row.get_i32("exact_accounting_version").ok() != Some(2))
                || ["active_settlement", "active_topup_expiry"]
                    .iter()
                    .any(|field| {
                        row.get(field)
                            .is_some_and(|value| *value != bson::Bson::Null)
                    })
            {
                deferred += 1;
                continue;
            }
            // Only fields that define the asset/checkpoint identity belong
            // here; malformed hold policy is isolated by the hold verifier.
            match asset_balance(collection, &row) {
                Ok(Some((account, balance))) => {
                    expected.insert(account, balance);
                }
                Ok(None) | Err(_) => {
                    deferred += 1;
                }
            }
        }
    }
    let accounts: Vec<_> = expected.keys().cloned().collect();
    let mut cursor = db
        .collection::<Document>("billing_account_balances")
        .find(doc! { "_id": { "$in": &accounts } })
        .batch_size((accounts.len() + 1) as u32)
        .session(&mut *session)
        .await?;
    let totals: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    let checkpoints: BTreeMap<_, _> = totals
        .into_iter()
        .map(|row| (row.get_str("_id").unwrap_or("").to_string(), row))
        .collect();
    let latest = latest_postings(db, session, &accounts).await?;
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
        // An authenticated older checkpoint is still a replay. The batched
        // equality lookup retains one newest-posting index seek per account.
        let latest_seq = latest.get(account).copied().unwrap_or(0);
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
        next.insert("hold_previous_pass", &pass);
        next.insert("hold_pass", uuid::Uuid::new_v4().to_string());
        // A smaller collection waits at its end until every
        // collection finishes this cycle. Independent wraparound
        // could otherwise prevent a complete pass indefinitely.
        for &collection in collections {
            next.insert(format!("{collection}_done"), false);
        }
    }
    if mismatch.is_none() {
        if scheduled && wrapped {
            db.collection::<Document>(super::hold_conservation::OBSERVATIONS)
                .delete_many(doc! {"pass":{"$ne":&pass}})
                .session(&mut *session)
                .await?;
        }
        db.collection::<Document>(STATE)
            .replace_one(doc! { "_id": state_id }, next)
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
        revision: None,
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
            (deferred > 0).then(|| {
                format!("{deferred} accounts awaiting durable settlement or readable asset data")
            })
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
    Ok((status, wrapped, repairs))
}

fn asset_credit(
    row: &Document,
    field: &str,
    legacy: Option<&str>,
    required: bool,
) -> AppResult<Credits> {
    if let Some(value) = row.get(field) {
        return Credits::from_bson(value.clone(), crate::models::credits::SCALE)
            .map_err(Into::into);
    }
    if let Some(value) = legacy.and_then(|name| row.get(name)) {
        return Credits::from_bson(value.clone(), 1_000_000).map_err(Into::into);
    }
    if required {
        Err(crate::models::credits::CreditsError.into())
    } else {
        Ok(Credits::ZERO)
    }
}

fn ledger_marker(row: &Document, field: &str) -> AppResult<bool> {
    match row.get(field) {
        None | Some(bson::Bson::Null) => Ok(false),
        Some(bson::Bson::DateTime(_)) => Ok(true),
        _ => Err(crate::errors::AppError::Internal(
            "Unreadable billing asset marker".into(),
        )),
    }
}

fn asset_balance(collection: &str, row: &Document) -> AppResult<Option<(String, Credits)>> {
    let identity = |field| {
        row.get_str(field).map_err(|_| {
            crate::errors::AppError::Internal("Unreadable billing asset identity".into())
        })
    };
    if collection == WALLETS {
        return Ok(Some((
            format!("wallet:{}", identity("owner_id")?),
            asset_credit(row, "balance_credits", None, false)?.checked_sub(asset_credit(
                row,
                "pending_lago_debits",
                None,
                false,
            )?)?,
        )));
    }
    let status = identity("status")?;
    if !matches!(status, "active" | "consumed" | "expired" | "revoked") {
        return Err(crate::errors::AppError::Internal(
            "Unreadable billing asset status".into(),
        ));
    }
    // Both markers must decode, even when this status needs only issuance.
    let issued = ledger_marker(row, "issued_ledgered_at")?;
    let terminal = ledger_marker(row, "terminal_ledgered_at")?;
    if !issued || (matches!(status, "expired" | "revoked") && !terminal) {
        return Ok(None);
    }
    Ok(Some((
        format!("grant:{}", identity("_id")?),
        asset_credit(row, "remaining", Some("remaining_micros"), true)?,
    )))
}

fn latest_postings_pipeline(accounts: &[String]) -> Vec<Document> {
    vec![
        doc! {"$documents":accounts.iter().map(|id|doc! {"_id":id}).collect::<Vec<_>>()},
        // Each equality lookup retains the existing compound-index seek for
        // the newest posting; batch the seeks into one round trip.
        doc! {"$lookup":{
            "from":LEDGER,"localField":"_id","foreignField":"postings.account",
            "pipeline":[{"$sort":{"seq":-1}},{"$limit":1},{"$project":{"_id":0,"seq":{"$ifNull":["$seq",-1_i64]}}}],
            "as":"latest",
        }},
        doc! {"$project":{"_id":1,"seq":{"$ifNull":[{"$first":"$latest.seq"},0_i64]}}},
    ]
}

async fn latest_postings(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    accounts: &[String],
) -> AppResult<BTreeMap<String, i64>> {
    if accounts.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut cursor = db
        .aggregate(latest_postings_pipeline(accounts))
        .batch_size((accounts.len() + 1) as u32)
        .session(&mut *session)
        .await?;
    let rows: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    rows.into_iter()
        .map(|row| {
            let id = row.get_str("_id").map_err(|_| {
                crate::errors::AppError::Internal("Unreadable posting account".into())
            })?;
            let seq = row
                .get_i64("seq")
                .or_else(|_| row.get_i32("seq").map(i64::from))
                .map_err(|_| {
                    crate::errors::AppError::Internal("Unreadable posting sequence".into())
                })?;
            Ok((id.to_owned(), seq))
        })
        .collect()
}

#[cfg(test)]
pub(crate) async fn hold_pass_for_test(db: &mongodb::Database) -> AppResult<()> {
    db.collection::<Document>(super::hold_conservation::OBSERVATIONS)
        .update_many(
            doc! {},
            vec![doc! {"$set":{"stable_since":{"$subtract":["$$NOW",301_000_i64]}}}],
        )
        .await?;
    hold_pass_immediately_for_test(db).await
}

#[cfg(test)]
pub(crate) async fn hold_pass_immediately_for_test(db: &mongodb::Database) -> AppResult<()> {
    db.collection::<Document>(LEASE)
        .update_one(doc! {"_id":CHECK_ID}, doc! {"$unset":{"next_pass_at":""}})
        .await?;
    run_scheduled_pass(db, 3600).await
}

#[cfg(test)]
mod reader_parity_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hold_round3_latest_postings_are_batched_index_seeks() {
        let db = crate::test_utils::connect_transaction_test_database("hold_latest_plans").await;
        db.collection::<Document>(LEDGER)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"postings.account":1,"seq":1})
                    .build(),
            )
            .await
            .unwrap();
        db.collection::<Document>(LEDGER)
            .insert_many((1..=3000_i64).map(|seq| {
                doc! {
                    "_id":seq.to_string(),"seq":seq,"postings":[{"account":"wallet:w"}]
                }
            }))
            .await
            .unwrap();
        let accounts = vec!["wallet:w".to_owned(), "wallet:absent".to_owned()];
        let explain = db
            .run_command(doc! {"explain":{
                "aggregate":1,"pipeline":latest_postings_pipeline(&accounts),"cursor":{}
            },"verbosity":"executionStats"})
            .await
            .unwrap();
        let lookup = explain
            .get_array("stages")
            .unwrap()
            .iter()
            .filter_map(bson::Bson::as_document)
            .find(|stage| stage.contains_key("$lookup"))
            .unwrap();
        assert_eq!(lookup.get_i64("collectionScans").unwrap(), 0);
        assert!(
            lookup
                .get_array("indexesUsed")
                .unwrap()
                .contains(&bson::Bson::String("postings.account_1_seq_1".into()))
        );
        assert!(lookup.get_i64("totalKeysExamined").unwrap() <= 2);
        println!("HOLD_R3_LATEST_EXPLAIN {lookup:?}");
        let expected_accounts = accounts.clone();
        let result = crate::services::transaction_service::run_db_only(
            &db,
            &(db.clone(), accounts),
            |session, (db, accounts)| {
                Box::pin(async move { latest_postings(db, session, accounts).await })
            },
        )
        .await
        .unwrap();
        assert_eq!(result[&expected_accounts[0]], 3000);
        assert_eq!(result[&expected_accounts[1]], 0);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn disabled_configuration_spawns_no_runner() {
        let client = mongodb::Client::with_options(Default::default()).unwrap();
        let db = client.database("account_verify_disabled");
        // No runtime is entered on this thread: accidentally spawning a task
        // panics immediately, without relying on sleeps or observing no writes.
        std::thread::spawn(move || {
            assert!(tokio::runtime::Handle::try_current().is_err());
            spawn(db.clone(), true, 0);
            spawn(db, false, 3600);
        })
        .join()
        .expect("disabled account verification must not spawn a runner");
    }

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
