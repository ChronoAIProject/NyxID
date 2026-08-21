use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::AppResult;
use crate::models::billing_ledger::{BillingLedgerEntry, COLLECTION_NAME as BILLING_LEDGER};
use crate::models::billing_topup_session::{
    BillingTopUpSession, COLLECTION_NAME as BILLING_TOPUP_SESSIONS,
};
use crate::models::billing_wallet::{
    BillingWallet, COLLECTION_NAME as BILLING_WALLETS, PurchasedCreditExpiryItem,
    PurchasedCreditExpiryOperation,
};

use super::lago_client::{
    LagoApi, LagoWalletTransaction, purchased_credit_expiry_transaction_name,
};

pub const PURCHASED_CREDIT_LIFETIME_DAYS: i64 = 365;
const EXPIRY_WALLET_BATCH: i64 = 100;
const EXPIRY_PURCHASE_BATCH: usize = 100;
const EXPIRY_OPERATION_LEASE_SECS: i64 = 600;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpiringPurchase {
    transaction_id: String,
    settled_at: DateTime<Utc>,
    remaining_micros: i64,
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
                doc! { "$set": {
                    "topup_expiry_checked_at": bson::DateTime::from_chrono(now),
                } },
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
                tracing::warn!(owner_id = %wallet.owner_id, %error, "failed to list Lago wallet transactions for expiry");
                continue;
            }
        };
        if !transactions.iter().any(is_settled_purchase) {
            continue;
        }
        let protected_micros = wallet
            .pending_lago_debits
            .saturating_add(wallet.reserved_credits)
            .max(0)
            .saturating_mul(1_000_000);
        let wallet_balance_micros = if has_traceable_purchase_balances(&transactions) {
            0
        } else {
            match wallet_balance_after_accrued_usage_micros(lago, &wallet).await {
                Ok(balance) => balance,
                Err(error) => {
                    tracing::warn!(owner_id = %wallet.owner_id, %error, "failed to read exact Lago wallet balance for expiry");
                    continue;
                }
            }
        };
        let purchases = purchased_remaining(transactions, wallet_balance_micros, protected_micros);
        // Populate the paid/expiry timestamps well before credits become due,
        // so history surfaces remain truthful without waiting for the expiry
        // mutation itself.
        let sessions = backfill_session_expiry(db, &wallet.owner_id, &purchases, now).await?;
        let cutoff = now - Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS);
        let expired: Vec<ExpiringPurchase> = purchases
            .into_iter()
            .filter(|purchase| purchase.settled_at <= cutoff && purchase.remaining_micros > 0)
            .take(EXPIRY_PURCHASE_BATCH)
            .collect();
        let amount_micros = expired
            .iter()
            .map(|purchase| purchase.remaining_micros)
            .sum::<i64>()
            / 10
            * 10;
        if amount_micros <= 0 {
            continue;
        }
        let operation_id = Uuid::new_v4().to_string();
        let processing_token = Uuid::new_v4().to_string();
        let operation = PurchasedCreditExpiryOperation {
            operation_id,
            processing_token,
            lease_until: now + Duration::seconds(EXPIRY_OPERATION_LEASE_SECS),
            amount_micros,
            items: expired
                .iter()
                .map(|purchase| PurchasedCreditExpiryItem {
                    lago_purchase_transaction_id: purchase.transaction_id.clone(),
                    reference_id: sessions.get(&purchase.transaction_id).map_or_else(
                        || purchase.transaction_id.clone(),
                        |session| session.id.clone(),
                    ),
                    amount_micros: purchase.remaining_micros,
                    settled_at: purchase.settled_at,
                })
                .collect(),
            lago_void_transaction_id: None,
            wallet_balance_applied: false,
            created_at: now,
            updated_at: now,
        };
        let operation_bson = bson::to_bson(&operation).map_err(|error| {
            crate::errors::AppError::Internal(format!(
                "failed to encode purchased-credit expiry operation: {error}"
            ))
        })?;
        let pending_credits = micros_to_held_credits(amount_micros);
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
                    // Reservations and locally settled usage are protected in
                    // `purchased_remaining`. If either changed since the
                    // wallet snapshot, recompute on the next sweep instead of
                    // expiring credits that a concurrent request just claimed.
                    "$expr": { "$and": [
                        { "$eq": [
                            { "$ifNull": ["$reserved_credits", 0_i64] },
                            wallet.reserved_credits,
                        ] },
                        { "$eq": [
                            { "$ifNull": ["$pending_lago_debits", 0_i64] },
                            wallet.pending_lago_debits,
                        ] },
                    ] },
                },
                doc! { "$set": {
                    "active_topup_expiry": operation_bson,
                    "pending_topup_expiry_credits": pending_credits,
                    "updated_at": bson::DateTime::from_chrono(now),
                } },
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
                    amount_micros,
                    %error,
                    "failed to complete purchased-credit expiry operation"
                );
            }
        }
    }
    Ok(expired_transactions)
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
            doc! { "$set": {
                "active_topup_expiry.processing_token": &processing_token,
                "active_topup_expiry.lease_until": bson::DateTime::from_chrono(lease_until),
                "active_topup_expiry.updated_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
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
                    lago.void_wallet_credits(
                        wallet_id,
                        operation.amount_micros,
                        &operation.operation_id,
                    )
                    .await?
                }
            };
            let update = db
                .collection::<BillingWallet>(BILLING_WALLETS)
                .update_one(
                    operation_filter(wallet, &operation),
                    doc! { "$set": {
                        "active_topup_expiry.lago_void_transaction_id": &transaction_id,
                        "active_topup_expiry.updated_at": bson::DateTime::from_chrono(now),
                        "updated_at": bson::DateTime::from_chrono(now),
                    } },
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
        let balance_micros = wallet_balance_after_accrued_usage_micros(lago, wallet).await?;
        // Availability is whole-credit based, so flooring the exact provider
        // balance is the only conservative conversion after an expiry debit.
        let balance_credits = balance_micros.max(0) / 1_000_000;
        let update = db
            .collection::<BillingWallet>(BILLING_WALLETS)
            .update_one(
                operation_filter(wallet, &operation),
                doc! { "$set": {
                    "balance_credits": balance_credits,
                    "pending_topup_expiry_credits": 0_i64,
                    "active_topup_expiry.wallet_balance_applied": true,
                    "active_topup_expiry.updated_at": bson::DateTime::from_chrono(now),
                    "balance_synced_at": bson::DateTime::from_chrono(now),
                    "updated_at": bson::DateTime::from_chrono(now),
                } },
            )
            .await?;
        if update.matched_count == 0 {
            return Ok(0);
        }
        operation.wallet_balance_applied = true;
    }

    finalize_session_expiry(db, &wallet.owner_id, &operation, &void_transaction_id).await?;
    let mut all_ledgered = true;
    for item in &operation.items {
        super::ledger::record_topup_expired(
            db,
            &wallet.owner_id,
            &item.reference_id,
            wallet_id,
            item.amount_micros,
            &void_transaction_id,
        )
        .await;
        let dedupe_key = format!(
            "topup-expired:{}:{}",
            item.reference_id, void_transaction_id
        );
        let ledgered = db
            .collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .count_documents(doc! { "dedupe_key": dedupe_key })
            .await?
            > 0;
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
                    "pending_topup_expiry_credits": 0_i64,
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

fn micros_to_held_credits(amount_micros: i64) -> i64 {
    amount_micros.max(0).saturating_add(999_999) / 1_000_000
}

async fn wallet_balance_after_accrued_usage_micros(
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
) -> AppResult<i64> {
    let balance_micros = lago.wallet_balance_micros(&wallet.lago_customer_id).await?;
    let accrued_micros = match wallet.lago_subscription_id.as_deref() {
        Some(subscription_id) => {
            let usage = lago
                .current_usage(&wallet.lago_customer_id, subscription_id)
                .await?;
            super::lago_client::extract_current_usage_amount_cents(&usage.raw)
                .unwrap_or(0)
                .max(0)
                .saturating_mul(10_000)
        }
        None => 0,
    };
    Ok(balance_micros.saturating_sub(accrued_micros))
}

fn has_traceable_purchase_balances(transactions: &[LagoWalletTransaction]) -> bool {
    let purchased: Vec<&LagoWalletTransaction> = transactions
        .iter()
        .filter(|transaction| is_settled_purchase(transaction))
        .collect();
    !purchased.is_empty()
        && purchased
            .iter()
            .all(|transaction| transaction.remaining_credit_micros.is_some())
}

fn is_settled_purchase(transaction: &LagoWalletTransaction) -> bool {
    transaction.status == "settled"
        && transaction.transaction_status == "purchased"
        && transaction.transaction_type == "inbound"
        && transaction.credit_amount_micros > 0
}

fn purchased_remaining(
    transactions: Vec<LagoWalletTransaction>,
    wallet_balance_micros: i64,
    protected_micros: i64,
) -> Vec<ExpiringPurchase> {
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
    let total_micros = purchased
        .iter()
        .map(|transaction| transaction.credit_amount_micros)
        .sum::<i64>();
    let traceable = purchased
        .iter()
        .all(|transaction| transaction.remaining_credit_micros.is_some());
    let mut remaining = if traceable {
        purchased
            .iter()
            .map(|transaction| transaction.remaining_credit_micros.unwrap_or(0).max(0))
            .collect::<Vec<_>>()
    } else {
        let mut consumed = total_micros.saturating_sub(wallet_balance_micros.max(0));
        purchased
            .iter()
            .map(|transaction| {
                let spent = consumed.min(transaction.credit_amount_micros);
                consumed = consumed.saturating_sub(spent);
                transaction.credit_amount_micros.saturating_sub(spent)
            })
            .collect::<Vec<_>>()
    };
    // Usage already settled or requests already admitted before the sweep are
    // entitled to the credits they reserved. Lago may not have consumed those
    // events yet, so remove the protected amount from FIFO remainders before
    // deciding what can expire.
    let mut protected = protected_micros.max(0);
    for amount in &mut remaining {
        let held = protected.min(*amount);
        *amount -= held;
        protected -= held;
        if protected == 0 {
            break;
        }
    }
    purchased
        .into_iter()
        .zip(remaining)
        .map(|(transaction, remaining_micros)| ExpiringPurchase {
            transaction_id: transaction.id,
            settled_at: transaction.settled_at.unwrap_or(transaction.created_at),
            remaining_micros,
        })
        .collect()
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
        .find(doc! {
            "owner_id": owner_id,
            "lago_wallet_transaction_id": { "$in": &ids },
        })
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
    let collection = db.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS);
    for item in &operation.items {
        collection
            .update_many(
                doc! {
                    "owner_id": owner_id,
                    "lago_wallet_transaction_id": &item.lago_purchase_transaction_id,
                },
                doc! { "$set": {
                    "paid_at": bson::DateTime::from_chrono(item.settled_at),
                    "credits_expire_at": bson::DateTime::from_chrono(
                        item.settled_at + Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS)
                    ),
                    "expired_credits_micros": item.amount_micros,
                    "credits_expired_at": bson::DateTime::from_chrono(operation.created_at),
                    "expiry_void_transaction_id": void_transaction_id,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                } },
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use chrono::TimeZone;

    use crate::models::billing_ledger::{
        BillingLedgerEntry, BillingLedgerEventType, COLLECTION_NAME as BILLING_LEDGER,
    };
    use crate::models::billing_topup_session::BillingTopUpStatus;
    use crate::models::billing_wallet::{CollectionState, PlanKind};
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, LagoWallet, OwnerProvisionInput,
    };
    use crate::test_utils::connect_test_database;

    use super::*;

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
            credit_amount_micros: credits,
            remaining_credit_micros: remaining,
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
            9_000_000,
            2_000_000,
        );
        assert_eq!(result[0].transaction_id, "old");
        assert_eq!(result[0].remaining_micros, 2_000_000);
        assert_eq!(result[1].remaining_micros, 5_000_000);
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
            7_000_000,
            0,
        );
        assert_eq!(result[0].remaining_micros, 2_000_000);
        assert_eq!(result[1].remaining_micros, 5_000_000);
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
                balance_credits: 10,
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
                }),
            })
        }

        async fn wallet_balance(&self, _customer_id: &str) -> AppResult<i64> {
            Ok(7)
        }

        async fn wallet_balance_micros(&self, _customer_id: &str) -> AppResult<i64> {
            Ok(7_000_000)
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
            amount_micros: i64,
            _operation_id: &str,
        ) -> AppResult<String> {
            assert_eq!(wallet_id, "lago-wallet-1");
            assert_eq!(amount_micros, 3_000_000);
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
                balance_credits: 10,
                reserved_credits: 0,
                pending_lago_debits: 0,
                pending_topup_expiry_credits: 0,
                has_payment_instrument: false,
                overdraft_cap_credits: 0,
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
                expired_credits_micros: 0,
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
            .find_one(doc! { "event_type": "topup_expired" })
            .await
            .expect("find ledger")
            .expect("expiry ledger entry exists");

        assert_eq!(expired, 1);
        assert_eq!(lago.void_calls.load(Ordering::SeqCst), 1);
        assert_eq!(wallet.balance_credits, 6);
        assert_eq!(session.paid_at, Some(paid_at));
        assert_eq!(
            session.credits_expire_at,
            Some(paid_at + Duration::days(PURCHASED_CREDIT_LIFETIME_DAYS))
        );
        assert_eq!(session.expired_credits_micros, 3_000_000);
        assert_eq!(session.credits_expired_at, Some(now));
        assert_eq!(
            session.expiry_void_transaction_id.as_deref(),
            Some("void-transaction-1")
        );
        assert_eq!(ledger.event_type, BillingLedgerEventType::TopupExpired);
        assert_eq!(ledger.amount_micros, Some(3_000_000));
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
            amount_micros: 3_000_000,
            items: vec![PurchasedCreditExpiryItem {
                lago_purchase_transaction_id: "purchase-recover".to_string(),
                reference_id: "topup-recover".to_string(),
                amount_micros: 3_000_000,
                settled_at: paid_at,
            }],
            lago_void_transaction_id: None,
            wallet_balance_applied: false,
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
                balance_credits: 10,
                reserved_credits: 0,
                pending_lago_debits: 0,
                pending_topup_expiry_credits: 3,
                has_payment_instrument: false,
                overdraft_cap_credits: 0,
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
        assert_eq!(wallet.balance_credits, 7);
        assert_eq!(wallet.pending_topup_expiry_credits, 0);
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
                balance_credits: 3,
                reserved_credits: 0,
                pending_lago_debits: 0,
                pending_topup_expiry_credits: 0,
                has_payment_instrument: false,
                overdraft_cap_credits: 0,
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
        assert_eq!(wallet.reserved_credits, 1);
        assert_eq!(wallet.pending_topup_expiry_credits, 0);
        assert!(wallet.active_topup_expiry.is_none());
    }
}
