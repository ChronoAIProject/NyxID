use super::lago_client::{
    LagoApi, LagoWalletTransaction, purchased_credit_expiry_transaction_name,
};
use crate::errors::AppResult;
use crate::models::billing_topup_session::{
    BillingTopUpSession, COLLECTION_NAME as BILLING_TOPUP_SESSIONS,
};
use crate::models::billing_wallet::{
    BillingWallet, COLLECTION_NAME as BILLING_WALLETS, PurchasedCreditExpiryItem,
    PurchasedCreditExpiryOperation,
};
use crate::models::credits::Credits;
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use std::collections::HashMap;
use uuid::Uuid;
pub const PURCHASED_CREDIT_LIFETIME_DAYS: i64 = 365;
const EXPIRY_WALLET_BATCH: i64 = 100;
const EXPIRY_PURCHASE_BATCH: usize = 100;
const EXPIRY_OPERATION_LEASE_SECS: i64 = 600;
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpiringPurchase {
    transaction_id: String,
    settled_at: DateTime<Utc>,
    remaining: Credits,
}

/// Expire the unused FIFO remainder of paid wallet transactions after one
/// year. Lago's wallet-level `expiration_at` expires the whole wallet at one
/// instant and cannot model rolling purchases. Lago v1.50 traceable wallets
/// expose per-inbound `remaining_credit_amount`; older non-traceable wallets
/// fall back to the local wallet balance and the same FIFO ordering.
pub async fn expire_purchased_credits(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    super::exact_migration::require_ready(db).await?;
    let wallets: Vec<BillingWallet> = db
        .collection::<BillingWallet>(BILLING_WALLETS)
        .find(doc! { "lago_wallet_id": { "$ne": null } })
        .sort(doc! { "topup_expiry_checked_at": 1, "updated_at": 1 })
        .limit(EXPIRY_WALLET_BATCH)
        .await?
        .try_collect()
        .await?;
    let mut expired_transactions = 0;
    for wallet in wallets {
        // Rotate the bounded batch even when Lago is unavailable or this
        // wallet has no purchases, preventing the oldest 100 wallets from
        // starving every other billing owner.
        db.collection::<BillingWallet>(BILLING_WALLETS)
            .update_one(
                doc! { "_id": &wallet.id },
                doc! { "$set": { "topup_expiry_checked_at": bson::DateTime::from_chrono(now), } },
            )
            .await?;
        if let Some(operation) = wallet.active_topup_expiry.clone() {
            match acquire_expiry_operation(db, &wallet, operation, now).await? {
                Some(operation) => {
                    match complete_expiry_operation(db, lago, &wallet, operation, now).await {
                        Ok(completed) => expired_transactions += completed,
                        Err(error) => {
                            tracing::warn!(
                                owner_id = %wallet.owner_id,
                                %error,
                                "failed to recover purchased-credit expiry operation"
                            );
                        }
                    }
                }
                None => {
                    tracing::debug!(
                        owner_id = %wallet.owner_id,
                        "purchased-credit expiry operation is leased by another reconciler"
                    );
                }
            }
            continue;
        }
        let Some(wallet_id) = wallet.lago_wallet_id.as_deref() else {
            continue;
        };
        let transactions = match lago.wallet_transactions(wallet_id).await {
            Ok(transactions) => transactions,
            Err(error) => {
                tracing::warn!(owner_id = %wallet.owner_id,
 %error,
 "failed to list Lago wallet transactions for expiry");
                continue;
            }
        };
        if !transactions.iter().any(is_settled_purchase) {
            continue;
        }
        let protected = wallet
            .pending_lago_debits
            .checked_add(wallet.reserved_credits)?
            .max(Credits::ZERO);
        let wallet_balance = if has_traceable_purchase_balances(&transactions) {
            Credits::ZERO
        } else {
            match super::webhook::provider_effective_balance(lago, &wallet).await {
                Ok(balance) => balance,
                Err(error) => {
                    tracing::warn!(owner_id = %wallet.owner_id,
 %error,
 "failed to read exact Lago wallet balance for expiry");
                    continue;
                }
            }
        };
        let purchases = purchased_remaining(transactions, wallet_balance, protected)?;
        // Populate the paid/expiry timestamps well before credits become due,
        // so history surfaces remain truthful without waiting for the expiry
        // mutation itself.
        let sessions = backfill_session_expiry(db, &wallet.owner_id, &purchases, now).await?;
        let expired = prepare_expired_purchases(db, &wallet.owner_id, purchases, now).await?;
        let amount = expired
            .iter()
            .map(|purchase| purchase.remaining)
            .try_fold(Credits::ZERO, Credits::checked_add)?;
        if amount <= Credits::ZERO {
            continue;
        }
        let operation_id = Uuid::new_v4().to_string();
        let processing_token = Uuid::new_v4().to_string();
        let operation = PurchasedCreditExpiryOperation {
            operation_id,
            processing_token,
            lease_until: now + Duration::seconds(EXPIRY_OPERATION_LEASE_SECS),
            amount,
            items: expired
                .iter()
                .map(|purchase| PurchasedCreditExpiryItem {
                    lago_purchase_transaction_id: purchase.transaction_id.clone(),
                    reference_id: sessions.get(&purchase.transaction_id).map_or_else(
                        || purchase.transaction_id.clone(),
                        |session| session.id.clone(),
                    ),
                    amount: purchase.remaining,
                    settled_at: purchase.settled_at,
                })
                .collect(),
            lago_void_transaction_id: None,
            wallet_balance_applied: false,
            history_applied: false,
            created_at: now,
            updated_at: now,
        };
        let operation_bson = bson::to_bson(&operation).map_err(|error| {
            crate::errors::AppError::Internal(format!(
                "failed to encode purchased-credit expiry operation: {error}"
            ))
        })?;
        let pending_credits = amount;
        let claim = db
            .collection::<BillingWallet>(BILLING_WALLETS)
            .update_one(
                doc! {
                    "_id": &wallet.id,
                    "$or": [
                        { "active_topup_expiry": { "$exists": false } },
                        { "active_topup_expiry": null },
                    ],
                    "pending_topup_expiry_credits": { "$in": [0_i64, null] },
                    // Recompute if a concurrent request changed protected funds.
                    "$expr": { "$and": [
                        { "$eq": [{ "$ifNull": ["$reserved_credits", 0_i64] }, wallet.reserved_credits] },
                        { "$eq": [{ "$ifNull": ["$pending_lago_debits", 0_i64] }, wallet.pending_lago_debits] },
                    ] },
                },
                doc! {
                    "$set": {
                        "active_topup_expiry": operation_bson,
                        "pending_topup_expiry_credits": pending_credits,
                        "updated_at": bson::DateTime::from_chrono(now),
                    },
                },
            )
            .await?;
        if claim.modified_count == 0 {
            continue;
        }
        match complete_expiry_operation(db, lago, &wallet, operation, now).await {
            Ok(completed) => expired_transactions += completed,
            Err(error) => {
                tracing::warn!(
                    owner_id = %wallet.owner_id,
                    %amount,
                    %error,
                    "failed to complete purchased-credit expiry operation"
                );
            }
        }
    }
    Ok(expired_transactions)
}

async fn prepare_expired_purchases(
    db: &mongodb::Database,
    owner_id: &str,
    purchases: Vec<ExpiringPurchase>,
    now: DateTime<Utc>,
) -> AppResult<Vec<ExpiringPurchase>> {
    let _ = (db, owner_id);
    // No local carry: provider remainder is authoritative.
    let cutoff = now - Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS);
    let mut expired = Vec::new();
    for mut purchase in purchases.into_iter().filter(|p| p.settled_at <= cutoff) {
        let residue = Credits::from_pico(purchase.remaining.pico() % 10_000_000)?;
        purchase.remaining = purchase.remaining.checked_sub(residue)?;
        if purchase.remaining > Credits::ZERO && expired.len() < EXPIRY_PURCHASE_BATCH {
            expired.push(purchase);
        }
    }
    Ok(expired)
}

async fn acquire_expiry_operation(
    db: &mongodb::Database,
    wallet: &BillingWallet,
    mut operation: PurchasedCreditExpiryOperation,
    now: DateTime<Utc>,
) -> AppResult<Option<PurchasedCreditExpiryOperation>> {
    if operation.lease_until > now {
        return Ok(None);
    }
    let processing_token = Uuid::new_v4().to_string();
    let lease_until = now + Duration::seconds(EXPIRY_OPERATION_LEASE_SECS);
    let update = db
        .collection::<BillingWallet>(BILLING_WALLETS)
        .update_one(
            doc! {
                "_id": &wallet.id,
                "active_topup_expiry.operation_id": &operation.operation_id,
                "active_topup_expiry.lease_until": { "$lte": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": {
                    "active_topup_expiry.processing_token": &processing_token,
                    "active_topup_expiry.lease_until": bson::DateTime::from_chrono(lease_until),
                    "active_topup_expiry.updated_at": bson::DateTime::from_chrono(now),
                    "updated_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .await?;
    if update.modified_count == 0 {
        return Ok(None);
    }
    operation.processing_token = processing_token;
    operation.lease_until = lease_until;
    operation.updated_at = now;
    Ok(Some(operation))
}

async fn complete_expiry_operation(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
    mut operation: PurchasedCreditExpiryOperation,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    let Some(wallet_id) = wallet.lago_wallet_id.as_deref() else {
        return Ok(0);
    };
    let void_transaction_id = match operation.lago_void_transaction_id.clone() {
        Some(transaction_id) => transaction_id,
        None => {
            // The provider POST has no idempotency-key field. Its unique name
            // is therefore the recovery key: after a timeout or process crash,
            // history is checked before any retry can create another debit.
            let operation_name = purchased_credit_expiry_transaction_name(&operation.operation_id);
            let transactions = lago.wallet_transactions(wallet_id).await?;
            let recovered = transactions
                .iter()
                .find(|transaction| transaction.name.as_deref() == Some(operation_name.as_str()))
                .map(|transaction| transaction.id.clone());
            let transaction_id = match recovered {
                Some(transaction_id) => transaction_id,
                None => {
                    lago.void_wallet_credits(wallet_id, operation.amount, &operation.operation_id)
                        .await?
                }
            };
            let update = db
                .collection::<BillingWallet>(BILLING_WALLETS)
                .update_one(
                    operation_filter(wallet, &operation),
                    doc! {
                        "$set": {
                            "active_topup_expiry.lago_void_transaction_id": &transaction_id,
                            "active_topup_expiry.updated_at": bson::DateTime::from_chrono(now),
                            "updated_at": bson::DateTime::from_chrono(now),
                        },
                    },
                )
                .await?;
            if update.matched_count == 0 {
                return Ok(0);
            }
            operation.lago_void_transaction_id = Some(transaction_id.clone());
            transaction_id
        }
    };
    if !operation.wallet_balance_applied {
        // credits_balance is the reliable OSS Lago field, but it does not
        // include the current period's un-invoiced usage. Subtract the same
        // current_usage amount as refresh_wallet_balances before publishing
        // the post-void local balance, or expiry could re-expose spent credit.
        let balance = super::webhook::provider_effective_balance(lago, wallet).await?;
        if super::webhook::apply_balance(db, &wallet.id, balance, Some(&operation))
            .await?
            .is_none()
        {
            return Ok(0);
        }
        operation.wallet_balance_applied = true;
    }
    finalize_session_expiry(db, &wallet.owner_id, &operation, &void_transaction_id).await?;
    let mut all_ledgered = true;
    for item in &operation.items {
        let dedupe_key = format!(
            "topup-expired:{}:{}",
            item.reference_id, void_transaction_id
        );
        let ledgered = super::ledger::ledger_dedupe_exists(db, &dedupe_key).await?;
        all_ledgered &= ledgered;
    }
    if !all_ledgered {
        tracing::warn!(
            owner_id = %wallet.owner_id,
            operation_id = %operation.operation_id,
            "purchased-credit expiry remains pending until every ledger entry is durable"
        );
        return Ok(0);
    }
    let completed = db
        .collection::<BillingWallet>(BILLING_WALLETS)
        .update_one(
            operation_filter(wallet, &operation),
            doc! {
                "$unset": { "active_topup_expiry": "" },
                "$set": {
                    "pending_topup_expiry_credits": Credits::ZERO,
                    "updated_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .await?;
    Ok(if completed.modified_count == 1 {
        operation.items.len() as u64
    } else {
        0
    })
}

fn operation_filter(
    wallet: &BillingWallet,
    operation: &PurchasedCreditExpiryOperation,
) -> mongodb::bson::Document {
    doc! {
        "_id": &wallet.id,
        "active_topup_expiry.operation_id": &operation.operation_id,
        "active_topup_expiry.processing_token": &operation.processing_token,
    }
}

fn has_traceable_purchase_balances(transactions: &[LagoWalletTransaction]) -> bool {
    let purchased: Vec<&LagoWalletTransaction> = transactions
        .iter()
        .filter(|transaction| is_settled_purchase(transaction))
        .collect();
    !purchased.is_empty()
        && purchased
            .iter()
            .all(|transaction| transaction.remaining_credit.is_some())
}

fn is_settled_purchase(transaction: &LagoWalletTransaction) -> bool {
    transaction.status == "settled"
        && transaction.transaction_status == "purchased"
        && transaction.transaction_type == "inbound"
        && transaction.credit_amount > Credits::ZERO
}

fn purchased_remaining(
    transactions: Vec<LagoWalletTransaction>,
    wallet_balance: Credits,
    protected_credits: Credits,
) -> AppResult<Vec<ExpiringPurchase>> {
    let mut purchased: Vec<LagoWalletTransaction> = transactions
        .into_iter()
        .filter(is_settled_purchase)
        .collect();
    purchased.sort_by(|left, right| {
        left.settled_at
            .unwrap_or(left.created_at)
            .cmp(&right.settled_at.unwrap_or(right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    let total = purchased
        .iter()
        .map(|transaction| transaction.credit_amount)
        .try_fold(Credits::ZERO, Credits::checked_add)?;
    let traceable = purchased
        .iter()
        .all(|transaction| transaction.remaining_credit.is_some());
    let mut remaining = if traceable {
        purchased
            .iter()
            .map(|transaction| {
                transaction
                    .remaining_credit
                    .unwrap_or(Credits::ZERO)
                    .max(Credits::ZERO)
            })
            .collect::<Vec<_>>()
    } else {
        let mut consumed = total
            .checked_sub(wallet_balance.max(Credits::ZERO))?
            .max(Credits::ZERO);
        purchased
            .iter()
            .map(|transaction| {
                let spent = consumed.min(transaction.credit_amount);
                consumed = consumed.checked_sub(spent)?;
                transaction.credit_amount.checked_sub(spent)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    // Usage already settled or requests already admitted before the sweep are
    // entitled to the credits they reserved. Lago may not have consumed those
    // events yet, so remove the protected amount from FIFO remainders before
    // deciding what can expire.
    let mut protected = protected_credits.max(Credits::ZERO);
    for amount in &mut remaining {
        let held = protected.min(*amount);
        *amount = amount.checked_sub(held)?;
        protected = protected.checked_sub(held)?;
        if protected == Credits::ZERO {
            break;
        }
    }
    Ok(purchased
        .into_iter()
        .zip(remaining)
        .map(|(transaction, remaining)| ExpiringPurchase {
            transaction_id: transaction.id,
            settled_at: transaction.settled_at.unwrap_or(transaction.created_at),
            remaining,
        })
        .collect())
}

async fn backfill_session_expiry(
    db: &mongodb::Database,
    owner_id: &str,
    purchases: &[ExpiringPurchase],
    now: DateTime<Utc>,
) -> AppResult<HashMap<String, BillingTopUpSession>> {
    let ids: Vec<&str> = purchases
        .iter()
        .map(|purchase| purchase.transaction_id.as_str())
        .collect();
    let mut sessions = HashMap::new();
    if ids.is_empty() {
        return Ok(sessions);
    }
    let collection = db.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS);
    let rows: Vec<BillingTopUpSession> = collection
        .find(doc! { "owner_id": owner_id, "lago_wallet_transaction_id": { "$in": &ids } , })
        .await?
        .try_collect()
        .await?;
    for session in rows {
        let Some(purchase) = purchases.iter().find(|purchase| {
            session.lago_wallet_transaction_id.as_deref() == Some(purchase.transaction_id.as_str())
        }) else {
            continue;
        };
        let expires_at = purchase.settled_at + Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS);
        let set = doc! {
            "paid_at": bson::DateTime::from_chrono(purchase.settled_at),
            "credits_expire_at": bson::DateTime::from_chrono(expires_at),
            "updated_at": bson::DateTime::from_chrono(now),
        };
        collection
            .update_one(doc! { "_id": &session.id }, doc! { "$set": set })
            .await?;
        sessions.insert(purchase.transaction_id.clone(), session);
    }
    Ok(sessions)
}

async fn finalize_session_expiry(
    db: &mongodb::Database,
    owner_id: &str,
    operation: &PurchasedCreditExpiryOperation,
    void_transaction_id: &str,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let database = db.clone();
    let owner = owner_id.to_string();
    let operation = operation.clone();
    let void_id = void_transaction_id.to_string();
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
                finalize_history_in_session(&database, &owner, &operation, &void_id, session).await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    Ok(())
}

async fn finalize_history_in_session(
    database: &mongodb::Database,
    owner: &str,
    operation: &PurchasedCreditExpiryOperation,
    void_id: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let wallets = database.collection::<BillingWallet>(BILLING_WALLETS);
    let claimed = wallets
        .update_one(
            doc! {
                "owner_id": &owner,
                "active_topup_expiry.operation_id": &operation.operation_id,
                "active_topup_expiry.processing_token": &operation.processing_token,
                "active_topup_expiry.history_applied": { "$ne": true },
            },
            doc! { "$set": { "active_topup_expiry.history_applied": true } },
        )
        .session(&mut *session)
        .await?;
    if claimed.modified_count == 0 {
        return Ok(());
    }
    let collection = database.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS);
    for item in &operation.items {
        let mut cursor = collection
            .find(doc! {
                "owner_id": &owner,
                "lago_wallet_transaction_id": &item.lago_purchase_transaction_id,
            })
            .session(&mut *session)
            .await?;
        let rows: Vec<BillingTopUpSession> = cursor.stream(&mut *session).try_collect().await?;
        for row in rows {
            // Old operations lack history_applied. Their receipt
            // already identifies a history write completed before
            // the cutover or a crash; do not add that amount twice.
            if row.expiry_void_transaction_id.as_deref() == Some(void_id) {
                continue;
            }
            let total = row.expired_credits.checked_add(item.amount)?;
            collection
                .update_one(
                    doc! { "_id": &row.id },
                    doc! {
                        "$set": {
                            "paid_at": bson::DateTime::from_chrono(item.settled_at),
                            "credits_expire_at": bson::DateTime::from_chrono(
                                item.settled_at + Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS),
                            ),
                            "expired_credits": total,
                            "credits_expired_at": bson::DateTime::from_chrono(
                                operation.created_at,
                            ),
                            "expiry_void_transaction_id": &void_id,
                            "updated_at": bson::DateTime::from_chrono(Utc::now()),
                        },
                    },
                )
                .session(&mut *session)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::billing_ledger::{
        BillingLedgerEntry, BillingLedgerEventType, COLLECTION_NAME as BILLING_LEDGER,
    };
    use crate::models::billing_topup_session::BillingTopUpStatus;
    use crate::models::billing_wallet::{CollectionState, PlanKind};
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, LagoWallet, OwnerProvisionInput,
    };
    use crate::test_utils::connect_test_database;
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    fn transaction(
        id: &str,
        credits: i64,
        remaining: Option<i64>,
        settled_at: DateTime<Utc>,
    ) -> LagoWalletTransaction {
        LagoWalletTransaction {
            id: id.to_string(),
            status: "settled".to_string(),
            transaction_status: "purchased".to_string(),
            transaction_type: "inbound".to_string(),
            credit_amount: crate::models::credits::Credits::from_micros(credits),
            remaining_credit: remaining.map(crate::models::credits::Credits::from_micros),
            name: None,
            settled_at: Some(settled_at),
            created_at: settled_at,
        }
    }
    #[test]
    fn traceable_fifo_remainders_protect_in_flight_usage() {
        let first = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let second = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
        let result = purchased_remaining(
            vec![
                transaction("new", 5_000_000, Some(5_000_000), second),
                transaction("old", 10_000_000, Some(4_000_000), first),
            ],
            Credits::from_whole(9),
            Credits::from_whole(2),
        )
        .unwrap();
        assert_eq!(result[0].transaction_id, "old");
        assert_eq!(result[0].remaining, Credits::from_micros(2_000_000));
        assert_eq!(result[1].remaining, Credits::from_micros(5_000_000));
    }
    #[test]
    fn legacy_wallet_balance_is_allocated_fifo() {
        let first = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let second = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
        let result = purchased_remaining(
            vec![
                transaction("old", 10_000_000, None, first),
                transaction("new", 5_000_000, None, second),
            ],
            Credits::from_whole(7),
            Credits::ZERO,
        )
        .unwrap();
        assert_eq!(result[0].remaining, Credits::from_micros(2_000_000));
        assert_eq!(result[1].remaining, Credits::from_micros(5_000_000));
    }
    #[test]
    fn traceability_requires_every_settled_purchase_to_report_a_remainder() {
        let settled_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        assert!(has_traceable_purchase_balances(&[transaction(
            "traceable",
            1_000_000,
            Some(500_000),
            settled_at,
        )]));
        assert!(!has_traceable_purchase_balances(&[transaction(
            "legacy", 1_000_000, None, settled_at,
        )]));
    }
    #[tokio::test]
    async fn tiny_expiry_residues_do_not_starve_actionable_purchases_and_clear_when_consumed() {
        let db = connect_test_database("expiry_carry_batch").await.unwrap();
        let now = Utc::now();
        let settled_at = now - Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS + 1);
        let mut purchases = (0..EXPIRY_PURCHASE_BATCH)
            .map(|i| ExpiringPurchase {
                transaction_id: format!("tiny-{i}"),
                settled_at,
                remaining: Credits::from_pico(1).unwrap(),
            })
            .collect::<Vec<_>>();
        purchases.push(ExpiringPurchase {
            transaction_id: "actionable".into(),
            settled_at,
            remaining: "1.000000123456".parse().unwrap(),
        });
        let expired = prepare_expired_purchases(&db, "owner", purchases.clone(), now)
            .await
            .unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].transaction_id, "actionable");
        assert_eq!(expired[0].remaining, Credits::from_whole(1));
        // The removed write-only carry never funded expiry. Assert the actual
        // provider residue instead: it stays available for later spending.
        assert_eq!(
            purchases
                .last()
                .unwrap()
                .remaining
                .checked_sub(expired[0].remaining)
                .unwrap(),
            "0.000000123456".parse().unwrap()
        );
        for purchase in &mut purchases {
            purchase.remaining = Credits::ZERO;
        }
        assert!(
            prepare_expired_purchases(&db, "owner", purchases, now)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            db.collection::<bson::Document>("billing_expiry_carry")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn partial_expiry_history_accumulates_once_and_fences_stale_workers() {
        let db = connect_test_database("expiry_history_exact").await.unwrap();
        let now = Utc::now();
        let mut operation = PurchasedCreditExpiryOperation {
            operation_id: "first".into(),
            processing_token: "worker-first".into(),
            lease_until: now + Duration::minutes(1),
            amount: "0.125".parse().unwrap(),
            items: vec![PurchasedCreditExpiryItem {
                lago_purchase_transaction_id: "purchase".into(),
                reference_id: "topup".into(),
                amount: "0.125".parse().unwrap(),
                settled_at: now - Duration::days(366),
            }],
            lago_void_transaction_id: Some("void-first".into()),
            wallet_balance_applied: true,
            history_applied: false,
            created_at: now,
            updated_at: now,
        };
        let wallets = db.collection::<bson::Document>(BILLING_WALLETS);
        wallets
            .insert_one(doc! {
                "_id": "wallet",
                "owner_id": "owner",
                "active_topup_expiry": bson::to_bson(&operation).unwrap(),
            })
            .await
            .unwrap();
        db.collection::<bson::Document>(BILLING_TOPUP_SESSIONS)
            .insert_one(doc! {
                "_id": "topup",
                "owner_id": "owner",
                "idempotency_key": "topup",
                "amount_credits": 5_i64,
                "lago_wallet_id": "lago-wallet",
                "lago_wallet_transaction_id": "purchase",
                "status": "checkout_created",
                "expired_credits_micros": 2_000_000_i64,
                "expiry_void_transaction_id": "previous-void",
                "created_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            })
            .await
            .unwrap();
        for _ in 0..2 {
            finalize_session_expiry(&db, "owner", &operation, "void-first")
                .await
                .unwrap();
        }
        let sessions = db.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS);
        assert_eq!(
            sessions
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .expired_credits
                .to_string(),
            "2.125"
        );
        let stale = operation.clone();
        operation.operation_id = "second".into();
        operation.processing_token = "worker-second".into();
        operation.lago_void_transaction_id = Some("void-second".into());
        operation.amount = "0.00001".parse().unwrap();
        operation.items[0].amount = operation.amount;
        wallets
            .update_one(
                doc! {},
                doc! { "$set": { "active_topup_expiry": bson::to_bson(&operation).unwrap() } },
            )
            .await
            .unwrap();
        for _ in 0..2 {
            finalize_session_expiry(&db, "owner", &operation, "void-second")
                .await
                .unwrap();
        }
        finalize_session_expiry(&db, "owner", &stale, "void-first")
            .await
            .unwrap();
        // A legacy crash may have written history without the new marker.
        wallets
            .update_one(
                doc! {},
                doc! { "$unset": { "active_topup_expiry.history_applied": "" } },
            )
            .await
            .unwrap();
        finalize_session_expiry(&db, "owner", &operation, "void-second")
            .await
            .unwrap();
        assert_eq!(
            sessions
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .expired_credits
                .to_string(),
            "2.12501"
        );
    }
    struct ExpiryLago {
        transaction: LagoWalletTransaction,
        current_usage_cents: i64,
        void_calls: AtomicUsize,
        reserve_before_return: Option<(mongodb::Database, String)>,
        reservation_applied: AtomicBool,
    }
    #[async_trait::async_trait]
    impl LagoApi for ExpiryLago {
        async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
            Ok(owner.external_customer_id.clone())
        }
        async fn ensure_subscription(
            &self,
            customer_id: &str,
            plan_code: &str,
        ) -> AppResult<String> {
            Ok(format!("{customer_id}:{plan_code}"))
        }
        async fn ensure_wallet(&self, customer_id: &str) -> AppResult<LagoWallet> {
            Ok(LagoWallet {
                id: format!("{customer_id}:wallet"),
                balance_credits: crate::models::credits::Credits::from_whole(10),
            })
        }
        async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
            Ok(LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
        }
        async fn record_events_batch(
            &self,
            events: &[LagoEvent],
        ) -> Result<Vec<LagoAck>, LagoError> {
            Ok(events
                .iter()
                .map(|event| LagoAck {
                    transaction_id: event.transaction_id.clone(),
                })
                .collect())
        }
        async fn current_usage(
            &self,
            customer_id: &str,
            subscription_id: &str,
        ) -> AppResult<LagoUsage> {
            Ok(LagoUsage {
                customer_id: customer_id.to_string(),
                subscription_id: subscription_id.to_string(),
                raw: serde_json::json!({
                                    "customer_usage": {
                                        "total_amount_cents": self.current_usage_cents,
                                    }
                                }
                ),
            })
        }
        async fn wallet_balance(&self, _customer_id: &str) -> AppResult<i64> {
            Ok(7)
        }
        async fn wallet_balance_credits(&self, _customer_id: &str) -> AppResult<Credits> {
            Ok(Credits::from_whole(7))
        }
        async fn entitlements(&self, _subscription_id: &str) -> AppResult<Vec<Entitlement>> {
            Ok(Vec::new())
        }
        async fn wallet_transactions(
            &self,
            _wallet_id: &str,
        ) -> AppResult<Vec<LagoWalletTransaction>> {
            if let Some((db, wallet_id)) = &self.reserve_before_return
                && !self.reservation_applied.swap(true, Ordering::SeqCst)
            {
                db.collection::<BillingWallet>(BILLING_WALLETS)
                    .update_one(
                        doc! { "_id": wallet_id },
                        doc! { "$inc": { "reserved_credits": 1_i64 } },
                    )
                    .await?;
            }
            Ok(vec![self.transaction.clone()])
        }
        async fn void_wallet_credits(
            &self,
            wallet_id: &str,
            amount: Credits,
            _operation_id: &str,
        ) -> AppResult<String> {
            assert_eq!(wallet_id, "lago-wallet-1");
            assert_eq!(amount, Credits::from_whole(3));
            self.void_calls.fetch_add(1, Ordering::SeqCst);
            Ok("void-transaction-1".to_string())
        }
    }
    #[tokio::test]
    async fn expiry_sweep_voids_updates_history_and_ledgers() {
        let Some(db) = connect_test_database("topup_expiry_full_sweep").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap();
        let paid_at = now - Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS + 1);
        db.collection::<BillingWallet>(BILLING_WALLETS)
            .insert_one(BillingWallet {
                id: "wallet-1".to_string(),
                owner_id: "owner-1".to_string(),
                lago_customer_id: "customer-1".to_string(),
                lago_wallet_id: Some("lago-wallet-1".to_string()),
                lago_subscription_id: Some("subscription-1".to_string()),
                plan_kind: PlanKind::Prepaid,
                balance_credits: crate::models::credits::Credits::from_whole(10),
                reserved_credits: crate::models::credits::Credits::from_whole(0),
                pending_lago_debits: crate::models::credits::Credits::from_whole(0),
                pending_topup_expiry_credits: crate::models::credits::Credits::from_whole(0),
                has_payment_instrument: false,
                overdraft_cap_credits: crate::models::credits::Credits::from_whole(0),
                suspended: false,
                collection_state: CollectionState::Good,
                topup_expiry_checked_at: None,
                active_topup_expiry: None,
                balance_synced_at: now,
                created_at: paid_at,
                updated_at: now,
            })
            .await
            .expect("insert wallet");
        db.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
            .insert_one(BillingTopUpSession {
                id: "topup-1".to_string(),
                owner_id: "owner-1".to_string(),
                idempotency_key: "topup-key-1".to_string(),
                amount_credits: 5,
                lago_wallet_id: "lago-wallet-1".to_string(),
                lago_wallet_transaction_id: Some("purchase-1".to_string()),
                lago_invoice_id: Some("invoice-1".to_string()),
                payment_url: None,
                payment_provider: Some("stripe".to_string()),
                status: BillingTopUpStatus::CheckoutCreated,
                paid_at: None,
                credits_expire_at: None,
                expired_credits: crate::models::credits::Credits::from_micros(0),
                credits_expired_at: None,
                expiry_void_transaction_id: None,
                created_at: paid_at,
                updated_at: paid_at,
            })
            .await
            .expect("insert top-up session");
        let lago = ExpiryLago {
            transaction: transaction("purchase-1", 5_000_000, Some(3_000_000), paid_at),
            current_usage_cents: 100,
            void_calls: AtomicUsize::new(0),
            reserve_before_return: None,
            reservation_applied: AtomicBool::new(false),
        };
        let expired = expire_purchased_credits(&db, &lago, now)
            .await
            .expect("expire purchased credits");
        let wallet = db
            .collection::<BillingWallet>(BILLING_WALLETS)
            .find_one(doc! { "_id": "wallet-1" })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        let session = db
            .collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
            .find_one(doc! { "_id": "topup-1" })
            .await
            .expect("find top-up")
            .expect("top-up exists");
        let ledger = db
            .collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .find_one(doc! { "movement": "topup_expired" })
            .await
            .expect("find ledger")
            .expect("expiry ledger entry exists");
        assert_eq!(expired, 1);
        assert_eq!(lago.void_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            wallet.balance_credits,
            crate::models::credits::Credits::from_whole(6)
        );
        assert_eq!(session.paid_at, Some(paid_at));
        assert_eq!(
            session.credits_expire_at,
            Some(paid_at + Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS))
        );
        assert_eq!(
            session.expired_credits,
            crate::models::credits::Credits::from_micros(3_000_000)
        );
        assert_eq!(session.credits_expired_at, Some(now));
        assert_eq!(
            session.expiry_void_transaction_id.as_deref(),
            Some("void-transaction-1")
        );
        assert_eq!(ledger.event_type, BillingLedgerEventType::AccountingV2);
        assert_eq!(ledger.movement.as_deref(), Some("topup_expired"));
        assert_eq!(ledger.postings[0].amount, Credits::from_whole(3));
        assert_eq!(ledger.transaction_id.as_deref(), Some("void-transaction-1"));
    }
    #[tokio::test]
    async fn expiry_recovery_discovers_provider_debit_without_voiding_twice() {
        let Some(db) = connect_test_database("topup_expiry_crash_recovery").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap();
        let paid_at = now - Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS + 1);
        let operation_id = "expiry-operation-recover";
        let operation = PurchasedCreditExpiryOperation {
            operation_id: operation_id.to_string(),
            processing_token: "dead-process".to_string(),
            lease_until: now - Duration::seconds(1),
            amount: crate::models::credits::Credits::from_micros(3_000_000),
            items: vec![PurchasedCreditExpiryItem {
                lago_purchase_transaction_id: "purchase-recover".to_string(),
                reference_id: "topup-recover".to_string(),
                amount: crate::models::credits::Credits::from_micros(3_000_000),
                settled_at: paid_at,
            }],
            lago_void_transaction_id: None,
            wallet_balance_applied: false,
            history_applied: false,
            created_at: now - Duration::minutes(5),
            updated_at: now - Duration::minutes(5),
        };
        db.collection::<BillingWallet>(BILLING_WALLETS)
            .insert_one(BillingWallet {
                id: "wallet-recover".to_string(),
                owner_id: "owner-recover".to_string(),
                lago_customer_id: "customer-recover".to_string(),
                lago_wallet_id: Some("lago-wallet-1".to_string()),
                lago_subscription_id: Some("subscription-recover".to_string()),
                plan_kind: PlanKind::Prepaid,
                balance_credits: crate::models::credits::Credits::from_whole(10),
                reserved_credits: crate::models::credits::Credits::from_whole(0),
                pending_lago_debits: crate::models::credits::Credits::from_whole(0),
                pending_topup_expiry_credits: crate::models::credits::Credits::from_whole(3),
                has_payment_instrument: false,
                overdraft_cap_credits: crate::models::credits::Credits::from_whole(0),
                suspended: false,
                collection_state: CollectionState::Good,
                topup_expiry_checked_at: None,
                active_topup_expiry: Some(operation),
                balance_synced_at: now - Duration::minutes(5),
                created_at: paid_at,
                updated_at: now - Duration::minutes(5),
            })
            .await
            .expect("insert recovering wallet");
        let mut recovered_transaction = transaction("void-recovered", 0, None, now);
        recovered_transaction.status = "settled".to_string();
        recovered_transaction.transaction_status = "voided".to_string();
        recovered_transaction.transaction_type = "outbound".to_string();
        recovered_transaction.name = Some(purchased_credit_expiry_transaction_name(operation_id));
        let lago = ExpiryLago {
            transaction: recovered_transaction,
            current_usage_cents: 0,
            void_calls: AtomicUsize::new(0),
            reserve_before_return: None,
            reservation_applied: AtomicBool::new(false),
        };
        let expired = expire_purchased_credits(&db, &lago, now)
            .await
            .expect("recover purchased-credit expiry");
        let wallet = db
            .collection::<BillingWallet>(BILLING_WALLETS)
            .find_one(doc! { "_id": "wallet-recover" })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        let ledger = db
            .collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .find_one(doc! { "reference_id": "topup-recover" })
            .await
            .expect("find recovery ledger")
            .expect("recovery ledger exists");
        assert_eq!(expired, 1);
        assert_eq!(lago.void_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            wallet.balance_credits,
            crate::models::credits::Credits::from_whole(7)
        );
        assert_eq!(
            wallet.pending_topup_expiry_credits,
            crate::models::credits::Credits::from_whole(0)
        );
        assert!(wallet.active_topup_expiry.is_none());
        assert_eq!(ledger.transaction_id.as_deref(), Some("void-recovered"));
    }
    #[tokio::test]
    async fn expiry_recomputes_when_a_request_reserves_during_provider_read() {
        let Some(db) = connect_test_database("topup_expiry_reservation_race").await else {
            return;
        };
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap();
        let paid_at = now - Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS + 1);
        db.collection::<BillingWallet>(BILLING_WALLETS)
            .insert_one(BillingWallet {
                id: "wallet-race".to_string(),
                owner_id: "owner-race".to_string(),
                lago_customer_id: "customer-race".to_string(),
                lago_wallet_id: Some("lago-wallet-race".to_string()),
                lago_subscription_id: Some("subscription-race".to_string()),
                plan_kind: PlanKind::Prepaid,
                balance_credits: crate::models::credits::Credits::from_whole(3),
                reserved_credits: crate::models::credits::Credits::from_whole(0),
                pending_lago_debits: crate::models::credits::Credits::from_whole(0),
                pending_topup_expiry_credits: crate::models::credits::Credits::from_whole(0),
                has_payment_instrument: false,
                overdraft_cap_credits: crate::models::credits::Credits::from_whole(0),
                suspended: false,
                collection_state: CollectionState::Good,
                topup_expiry_checked_at: None,
                active_topup_expiry: None,
                balance_synced_at: now,
                created_at: paid_at,
                updated_at: now,
            })
            .await
            .expect("insert wallet");
        let lago = ExpiryLago {
            transaction: transaction("purchase-race", 3_000_000, Some(3_000_000), paid_at),
            current_usage_cents: 0,
            void_calls: AtomicUsize::new(0),
            reserve_before_return: Some((db.clone(), "wallet-race".to_string())),
            reservation_applied: AtomicBool::new(false),
        };
        let expired = expire_purchased_credits(&db, &lago, now)
            .await
            .expect("run expiry sweep");
        let wallet = db
            .collection::<BillingWallet>(BILLING_WALLETS)
            .find_one(doc! { "_id": "wallet-race" })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        assert_eq!(expired, 0);
        assert_eq!(lago.void_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            wallet.reserved_credits,
            crate::models::credits::Credits::from_whole(1)
        );
        assert_eq!(
            wallet.pending_topup_expiry_credits,
            crate::models::credits::Credits::from_whole(0)
        );
        assert!(wallet.active_topup_expiry.is_none());
    }
}
