use std::collections::BTreeSet;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, Document, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::{BillingRateCache, COLLECTION_NAME as BILLING_RATE_CACHE};
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as BILLING_WALLET, PlanKind};
use crate::models::usage_meter::{
    BillingLayer, COLLECTION_NAME as USAGE_METER, UsageMeterRow, UsageStatus,
};

use super::lago_client::{Entitlement, LagoApi};
use super::meter::platform_metric_code;
use super::route_context::BillingRouteContext;

const CREDIT_MICROS: i64 = 1_000_000;
const RECOVERY_BATCH_SIZE: i64 = 100;
const SETTLEMENT_LOCK_RETRIES: usize = 4;
const SETTLEMENT_STRANDED_AFTER_SECS: i64 = 30;
const SETTLEMENT_RETRY_BASE_SECS: i64 = 30;
const SETTLEMENT_RETRY_MAX_SECS: i64 = 30 * 60;
const SETTLEMENT_CLAIM_LEASE_SECS: i64 = 60;
pub(crate) const MAX_SETTLEMENT_ATTEMPTS: i32 = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettlementRecoveryStats {
    pub recovered: u64,
    pub retried: u64,
    pub dead_lettered: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettlementFailureDisposition {
    RetryScheduled(i32),
    DeadLettered(i32),
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerReservation {
    pub layer: BillingLayer,
    pub reserved_credits: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BillingReservation {
    pub owner_id: String,
    pub wallet_id: String,
    pub total_reserved_credits: i64,
    pub layers: Vec<LayerReservation>,
}

impl BillingReservation {
    pub fn reserved_for(&self, layer: BillingLayer) -> i64 {
        self.layers
            .iter()
            .find(|reservation| reservation.layer == layer)
            .map(|reservation| reservation.reserved_credits)
            .unwrap_or(0)
    }
}

pub async fn gate_and_reserve(
    db: &mongodb::Database,
    lago: Option<&dyn LagoApi>,
    ctx: &BillingRouteContext,
    billing_fail_closed: bool,
) -> AppResult<Option<BillingReservation>> {
    if !ctx.has_billable_layers() {
        return Ok(None);
    }
    if billing_fail_closed {
        return Err(AppError::BillingProviderUnavailable(
            "billing fail-closed override is enabled".to_string(),
        ));
    }

    let wallet = db
        .collection::<BillingWallet>(BILLING_WALLET)
        .find_one(doc! { "owner_id": &ctx.billing_owner_id })
        .await?
        .ok_or_else(|| {
            tracing::warn!(
                owner_id = %ctx.billing_owner_id,
                "Billing wallet is missing; continuing without reservation"
            );
        })
        .ok();

    let Some(wallet) = wallet else {
        return Ok(None);
    };

    if wallet.is_suspended() {
        return Err(AppError::WalletSuspended);
    }
    let Some(subscription_id) = wallet.lago_subscription_id.as_deref() else {
        tracing::warn!(
            owner_id = %ctx.billing_owner_id,
            "Billing subscription is missing; continuing without reservation"
        );
        return Ok(None);
    };
    let Some(lago) = lago else {
        tracing::warn!("Lago client is not configured; continuing without billing reservation");
        return Ok(None);
    };
    let entitlements = lago.entitlements(subscription_id).await.map_err(|error| {
        tracing::warn!(
            owner_id = %ctx.billing_owner_id,
            subscription_id,
            error = %error,
            "Billing entitlement lookup failed closed"
        );
        AppError::PlanEntitlementRequired("billing entitlement could not be verified".to_string())
    })?;
    if !is_entitled(ctx, &entitlements) {
        return Err(AppError::PlanEntitlementRequired(
            "owner plan does not include this service".to_string(),
        ));
    }

    let layers = match estimate_layer_reservations(db, ctx).await {
        Ok(layers) => layers,
        Err(AppError::BillingNotConfigured(message)) => {
            tracing::warn!(
                owner_id = %ctx.billing_owner_id,
                error = %message,
                "Billing reservation is not fully configured; continuing without reservation"
            );
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let total_reserved_credits = layers
        .iter()
        .map(|reservation| reservation.reserved_credits)
        .sum::<i64>();

    if total_reserved_credits == 0 {
        return Ok(Some(BillingReservation {
            owner_id: wallet.owner_id,
            wallet_id: wallet.id,
            total_reserved_credits,
            layers,
        }));
    }

    if wallet.available_with_overdraft_credits() <= 0 && wallet.plan_kind != PlanKind::Prepaid {
        suspend_wallet(db, &wallet.owner_id).await?;
        return Err(AppError::WalletSuspended);
    }
    if wallet.available_credits() <= 0 && wallet.plan_kind == PlanKind::Prepaid {
        return Err(AppError::InsufficientCredits);
    }

    if try_reserve_prepaid(db, &wallet.owner_id, total_reserved_credits)
        .await?
        .is_some()
    {
        return Ok(Some(BillingReservation {
            owner_id: wallet.owner_id,
            wallet_id: wallet.id,
            total_reserved_credits,
            layers,
        }));
    }

    if wallet.plan_kind == PlanKind::Prepaid {
        return Err(AppError::InsufficientCredits);
    }

    if wallet.has_payment_instrument
        && try_reserve_overdraft(db, &wallet.owner_id, total_reserved_credits)
            .await?
            .is_some()
    {
        return Ok(Some(BillingReservation {
            owner_id: wallet.owner_id,
            wallet_id: wallet.id,
            total_reserved_credits,
            layers,
        }));
    }

    if wallet.has_payment_instrument {
        suspend_wallet(db, &wallet.owner_id).await?;
        return Err(AppError::WalletSuspended);
    }

    Err(AppError::InsufficientCredits)
}

pub async fn try_reserve_prepaid(
    db: &mongodb::Database,
    owner_id: &str,
    credits: i64,
) -> AppResult<Option<BillingWallet>> {
    if credits <= 0 {
        return db
            .collection::<BillingWallet>(BILLING_WALLET)
            .find_one(doc! { "owner_id": owner_id, "suspended": false })
            .await
            .map_err(Into::into);
    }

    let now = Utc::now();
    db.collection::<BillingWallet>(BILLING_WALLET)
        .find_one_and_update(
            doc! {
                "owner_id": owner_id,
                "suspended": false,
                "$expr": {
                    "$gte": [
                        {
                            "$subtract": [
                                {
                                    "$subtract": [
                                        "$balance_credits",
                                        "$reserved_credits"
                                    ]
                                },
                                "$pending_lago_debits"
                            ]
                        },
                        credits
                    ]
                },
            },
            doc! {
                "$inc": { "reserved_credits": credits },
                "$set": { "updated_at": bson::DateTime::from_chrono(now) },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await
        .map_err(Into::into)
}

pub async fn try_reserve_overdraft(
    db: &mongodb::Database,
    owner_id: &str,
    credits: i64,
) -> AppResult<Option<BillingWallet>> {
    if credits <= 0 {
        return try_reserve_prepaid(db, owner_id, credits).await;
    }

    let now = Utc::now();
    db.collection::<BillingWallet>(BILLING_WALLET)
        .find_one_and_update(
            doc! {
                "owner_id": owner_id,
                "suspended": false,
                "has_payment_instrument": true,
                "$expr": {
                    "$gte": [
                        {
                            "$subtract": [
                                {
                                    "$add": [
                                        "$balance_credits",
                                        "$overdraft_cap_credits"
                                    ]
                                },
                                {
                                    "$add": [
                                        "$reserved_credits",
                                        "$pending_lago_debits"
                                    ]
                                }
                            ]
                        },
                        credits
                    ]
                },
            },
            doc! {
                "$inc": { "reserved_credits": credits },
                "$set": { "updated_at": bson::DateTime::from_chrono(now) },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await
        .map_err(Into::into)
}

pub async fn release_wallet_hold(
    db: &mongodb::Database,
    owner_id: &str,
    credits: i64,
) -> AppResult<()> {
    if credits <= 0 {
        return Ok(());
    }

    db.collection::<BillingWallet>(BILLING_WALLET)
        .update_one(
            doc! {
                "owner_id": owner_id,
                "$expr": { "$gte": [ "$reserved_credits", credits ] },
            },
            doc! {
                "$inc": { "reserved_credits": -credits },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .await?;
    Ok(())
}

pub async fn actual_credits_for_row(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    quantity: i64,
    model: Option<&str>,
) -> AppResult<i64> {
    if row.wallet_id.is_none() {
        return Ok(0);
    }

    match estimate_credits(db, &row.lago_metric_code, model, quantity.max(0)).await {
        Ok(credits) => Ok(credits),
        Err(AppError::BillingNotConfigured(_)) => Ok(row.reserved_credits.max(0)),
        Err(error) => Err(error),
    }
}

pub async fn apply_settlement_for_row(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    actual_credits: i64,
) -> AppResult<bool> {
    let usage_rows = db.collection::<UsageMeterRow>(USAGE_METER);
    if row.released {
        if let Some(wallet_id) = row.wallet_id.as_deref() {
            clear_wallet_settlement_lock_any_state(db, wallet_id, &row.billing_owner_id, &row.id)
                .await?;
        }
        return Ok(false);
    }

    if usage_rows
        .count_documents(doc! { "_id": &row.id, "released": false })
        .await?
        == 0
    {
        if let Some(wallet_id) = row.wallet_id.as_deref() {
            clear_wallet_settlement_lock_any_state(db, wallet_id, &row.billing_owner_id, &row.id)
                .await?;
        }
        return Ok(false);
    }

    if let Some(wallet_id) = row.wallet_id.as_deref() {
        let lock = WalletSettlementLock {
            row_id: row.id.clone(),
            reserved_credits: row.reserved_credits.max(0),
            actual_credits: actual_credits.max(0),
            applied: false,
        };
        if lock.reserved_credits > 0 || lock.actual_credits > 0 {
            ensure_wallet_settlement_lock(db, wallet_id, &row.billing_owner_id, &lock).await?;
            if usage_rows
                .count_documents(doc! { "_id": &row.id, "released": false })
                .await?
                == 0
            {
                clear_wallet_settlement_lock_any_state(
                    db,
                    wallet_id,
                    &row.billing_owner_id,
                    &row.id,
                )
                .await?;
                return Ok(false);
            }
            apply_wallet_settlement_lock(db, wallet_id, &row.billing_owner_id, &lock).await?;
        }
    }

    let update = usage_rows
        .update_one(
            doc! {
                "_id": &row.id,
                "status": { "$in": ["finalized", "failed"] },
                "released": false,
            },
            doc! {
                "$set": {
                    "status": "finalized",
                    "released": true,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                },
                "$unset": {
                    "settlement_next_retry_at": "",
                    "last_error": "",
                },
            },
        )
        .await?;
    if update.matched_count == 0 {
        let released = usage_rows
            .count_documents(doc! { "_id": &row.id, "released": true })
            .await?
            > 0;
        if !released {
            return Err(AppError::Internal(format!(
                "billing settlement could not complete usage row {}",
                row.id
            )));
        }
    }
    if let Some(wallet_id) = row.wallet_id.as_deref() {
        clear_wallet_settlement_lock(db, wallet_id, &row.billing_owner_id, &row.id).await?;
    }
    Ok(update.modified_count > 0)
}

// Bounded crash bridge for one in-flight wallet settlement. Settled history
// remains on the usage row's `released` transition, not in the wallet document.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WalletSettlementLock {
    row_id: String,
    reserved_credits: i64,
    actual_credits: i64,
    applied: bool,
}

impl WalletSettlementLock {
    fn document(&self, applied: bool) -> Document {
        doc! {
            "row_id": &self.row_id,
            "reserved_credits": self.reserved_credits,
            "actual_credits": self.actual_credits,
            "applied": applied,
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        }
    }
}

async fn ensure_wallet_settlement_lock(
    db: &mongodb::Database,
    wallet_id: &str,
    owner_id: &str,
    lock: &WalletSettlementLock,
) -> AppResult<()> {
    let wallets = db.collection::<Document>(BILLING_WALLET);
    for _ in 0..SETTLEMENT_LOCK_RETRIES {
        let update = wallets
            .update_one(
                doc! {
                    "_id": wallet_id,
                    "owner_id": owner_id,
                    "$or": [
                        { "active_settlement": { "$exists": false } },
                        { "active_settlement": null },
                    ],
                },
                doc! {
                    "$set": {
                        "active_settlement": lock.document(false),
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    },
                },
            )
            .await?;
        if update.matched_count == 1 {
            return Ok(());
        }

        let Some(active) = load_wallet_settlement_lock(db, wallet_id, owner_id).await? else {
            continue;
        };
        if active.row_id == lock.row_id {
            return Ok(());
        }
        complete_wallet_settlement_lock(db, wallet_id, owner_id, &active).await?;
    }

    Err(AppError::Internal(format!(
        "billing wallet settlement lock busy for wallet {wallet_id}"
    )))
}

async fn complete_wallet_settlement_lock(
    db: &mongodb::Database,
    wallet_id: &str,
    owner_id: &str,
    lock: &WalletSettlementLock,
) -> AppResult<()> {
    apply_wallet_settlement_lock(db, wallet_id, owner_id, lock).await?;
    let update = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! {
                "_id": &lock.row_id,
                "status": { "$in": ["finalized", "failed"] },
                "released": false,
            },
            doc! {
                "$set": {
                    "status": "finalized",
                    "released": true,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                },
                "$unset": {
                    "settlement_next_retry_at": "",
                    "last_error": "",
                },
            },
        )
        .await?;

    if update.matched_count == 0 {
        let released = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .count_documents(doc! { "_id": &lock.row_id, "released": true })
            .await?
            > 0;
        if !released {
            return Err(AppError::Internal(format!(
                "billing wallet settlement lock references unfinished usage row {}",
                lock.row_id
            )));
        }
    }

    clear_wallet_settlement_lock(db, wallet_id, owner_id, &lock.row_id).await
}

async fn apply_wallet_settlement_lock(
    db: &mongodb::Database,
    wallet_id: &str,
    owner_id: &str,
    lock: &WalletSettlementLock,
) -> AppResult<()> {
    if lock.applied {
        return Ok(());
    }

    let wallets = db.collection::<Document>(BILLING_WALLET);
    let update = wallets
        .update_one(
            doc! {
                "_id": wallet_id,
                "owner_id": owner_id,
                "active_settlement.row_id": &lock.row_id,
                "active_settlement.applied": false,
            },
            doc! {
                "$inc": {
                    "reserved_credits": -lock.reserved_credits,
                    "pending_lago_debits": lock.actual_credits,
                },
                "$set": {
                    "active_settlement.applied": true,
                    "active_settlement.updated_at": bson::DateTime::from_chrono(Utc::now()),
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                },
            },
        )
        .await?;
    if update.matched_count == 1 {
        return Ok(());
    }

    let already_applied = load_wallet_settlement_lock(db, wallet_id, owner_id)
        .await?
        .is_some_and(|active| active.row_id == lock.row_id && active.applied);
    if already_applied {
        return Ok(());
    }

    let already_released = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .count_documents(doc! { "_id": &lock.row_id, "released": true })
        .await?
        > 0;
    if already_released {
        return Ok(());
    }

    Err(AppError::Internal(format!(
        "billing wallet settlement lock missing for usage row {}",
        lock.row_id
    )))
}

async fn clear_wallet_settlement_lock(
    db: &mongodb::Database,
    wallet_id: &str,
    owner_id: &str,
    row_id: &str,
) -> AppResult<()> {
    db.collection::<Document>(BILLING_WALLET)
        .update_one(
            doc! {
                "_id": wallet_id,
                "owner_id": owner_id,
                "active_settlement.row_id": row_id,
                "active_settlement.applied": true,
            },
            doc! {
                "$unset": { "active_settlement": "" },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .await?;
    Ok(())
}

async fn clear_wallet_settlement_lock_any_state(
    db: &mongodb::Database,
    wallet_id: &str,
    owner_id: &str,
    row_id: &str,
) -> AppResult<()> {
    db.collection::<Document>(BILLING_WALLET)
        .update_one(
            doc! {
                "_id": wallet_id,
                "owner_id": owner_id,
                "active_settlement.row_id": row_id,
            },
            doc! {
                "$unset": { "active_settlement": "" },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .await?;
    Ok(())
}

async fn load_wallet_settlement_lock(
    db: &mongodb::Database,
    wallet_id: &str,
    owner_id: &str,
) -> AppResult<Option<WalletSettlementLock>> {
    let wallet = db
        .collection::<Document>(BILLING_WALLET)
        .find_one(doc! { "_id": wallet_id, "owner_id": owner_id })
        .await?
        .ok_or_else(|| {
            AppError::Internal(format!(
                "billing wallet {wallet_id} missing for owner {owner_id}"
            ))
        })?;
    parse_wallet_settlement_lock(&wallet)
}

fn parse_wallet_settlement_lock(wallet: &Document) -> AppResult<Option<WalletSettlementLock>> {
    let Some(value) = wallet.get("active_settlement") else {
        return Ok(None);
    };
    if matches!(value, Bson::Null) {
        return Ok(None);
    }
    let lock = value.as_document().ok_or_else(|| {
        AppError::Internal("billing wallet settlement lock is malformed".to_string())
    })?;
    let row_id = lock
        .get_str("row_id")
        .map_err(|_| {
            AppError::Internal("billing wallet settlement lock has no row_id".to_string())
        })?
        .to_string();
    Ok(Some(WalletSettlementLock {
        row_id,
        reserved_credits: document_i64(lock, "reserved_credits").unwrap_or(0).max(0),
        actual_credits: document_i64(lock, "actual_credits").unwrap_or(0).max(0),
        applied: lock.get_bool("applied").unwrap_or(false),
    }))
}

fn document_i64(document: &Document, key: &str) -> Option<i64> {
    match document.get(key) {
        Some(Bson::Int32(value)) => Some(i64::from(*value)),
        Some(Bson::Int64(value)) => Some(*value),
        Some(Bson::Double(value)) if value.is_finite() => Some(*value as i64),
        _ => None,
    }
}

/// Settle a finalized row through the bounded wallet lock.
///
/// The money move is guarded by `active_settlement`, then the usage row's
/// `released:false -> released:true` transition is completed on the same path
/// used by recovery. This avoids both double-debit replay and lost debit if the
/// process stops after the wallet update but before the row marker is written.
pub async fn claim_released_and_settle(
    db: &mongodb::Database,
    row: &UsageMeterRow,
) -> AppResult<bool> {
    let quantity = row.quantity.unwrap_or(0);
    let actual_credits = actual_credits_for_row(db, row, quantity, row.model.as_deref()).await?;
    apply_settlement_for_row(db, row, actual_credits).await
}

pub(crate) async fn record_settlement_failure(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    now: chrono::DateTime<Utc>,
) -> AppResult<SettlementFailureDisposition> {
    let next_attempt = row.settlement_attempts.saturating_add(1);
    if next_attempt >= MAX_SETTLEMENT_ATTEMPTS {
        if finish_applied_lock_before_dead_letter(db, row).await? {
            return Ok(SettlementFailureDisposition::Unchanged);
        }

        let updated = update_settlement_failure(db, row, now, "dead_letter", None).await?;
        if updated {
            tracing::warn!(
                row_id = %row.id,
                transaction_id = %row.transaction_id,
                settlement_attempts = next_attempt,
                quantity = row.quantity.unwrap_or(0),
                "Billing settlement moved to dead letter"
            );
            return Ok(SettlementFailureDisposition::DeadLettered(next_attempt));
        }
        return Ok(SettlementFailureDisposition::Unchanged);
    }

    let next_retry_at = now + settlement_retry_backoff(next_attempt);
    let updated = update_settlement_failure(db, row, now, "failed", Some(next_retry_at)).await?;
    if updated {
        tracing::warn!(
            row_id = %row.id,
            transaction_id = %row.transaction_id,
            settlement_attempts = next_attempt,
            quantity = row.quantity.unwrap_or(0),
            "Billing settlement failed; retry scheduled"
        );
        Ok(SettlementFailureDisposition::RetryScheduled(next_attempt))
    } else {
        Ok(SettlementFailureDisposition::Unchanged)
    }
}

async fn update_settlement_failure(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    now: chrono::DateTime<Utc>,
    status: &str,
    next_retry_at: Option<chrono::DateTime<Utc>>,
) -> AppResult<bool> {
    let last_error = if status == "dead_letter" {
        "billing settlement retries exhausted"
    } else {
        "billing settlement failed"
    };
    let mut set = doc! {
        "status": status,
        "last_error": last_error,
        "updated_at": bson::DateTime::from_chrono(now),
    };
    if let Some(next_retry_at) = next_retry_at {
        set.insert(
            "settlement_next_retry_at",
            bson::DateTime::from_chrono(next_retry_at),
        );
    }
    let mut update = doc! {
        "$set": set,
        "$inc": { "settlement_attempts": 1 },
    };
    if next_retry_at.is_none() {
        update.insert("$unset", doc! { "settlement_next_retry_at": "" });
    }

    let result = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! {
                "_id": &row.id,
                "status": { "$in": ["finalized", "failed"] },
                "released": false,
                "$and": [settlement_attempt_filter(row.settlement_attempts)],
            },
            update,
        )
        .await?;
    Ok(result.modified_count == 1)
}

fn settlement_retry_backoff(attempt: i32) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1))
        .unwrap_or(0)
        .min(10);
    let seconds = SETTLEMENT_RETRY_BASE_SECS
        .saturating_mul(1_i64 << exponent)
        .min(SETTLEMENT_RETRY_MAX_SECS);
    Duration::seconds(seconds)
}

fn settlement_attempt_filter(expected: i32) -> Document {
    if expected == 0 {
        doc! {
            "$or": [
                { "settlement_attempts": 0 },
                { "settlement_attempts": { "$exists": false } },
            ]
        }
    } else {
        doc! { "settlement_attempts": expected }
    }
}

async fn finish_applied_lock_before_dead_letter(
    db: &mongodb::Database,
    row: &UsageMeterRow,
) -> AppResult<bool> {
    let Some(wallet_id) = row.wallet_id.as_deref() else {
        return Ok(false);
    };
    let Some(wallet) = db
        .collection::<Document>(BILLING_WALLET)
        .find_one(doc! { "_id": wallet_id, "owner_id": &row.billing_owner_id })
        .await?
    else {
        return Ok(false);
    };
    let Some(lock) = parse_wallet_settlement_lock(&wallet)? else {
        return Ok(false);
    };
    if lock.row_id != row.id {
        return Ok(false);
    }
    if lock.applied {
        complete_wallet_settlement_lock(db, wallet_id, &row.billing_owner_id, &lock).await?;
        return Ok(true);
    }
    clear_wallet_settlement_lock_any_state(db, wallet_id, &row.billing_owner_id, &row.id).await?;
    Ok(false)
}

pub async fn release_unforwarded_rows(
    db: &mongodb::Database,
    billing_request_id: &str,
    terminal_status: UsageStatus,
    reason: Option<&str>,
) -> AppResult<u64> {
    let mut cursor = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(doc! {
            "billing_request_id": billing_request_id,
            "forwarded": false,
            "status": "reserved",
            "released": false,
        })
        .await?;
    let mut released_count = 0;
    while let Some(row) = cursor.try_next().await? {
        if release_one_unforwarded_row(db, &row, terminal_status, reason).await? {
            released_count += 1;
        }
    }
    Ok(released_count)
}

pub async fn abandon_stale_unforwarded(
    db: &mongodb::Database,
    cutoff: chrono::DateTime<Utc>,
) -> AppResult<u64> {
    let mut cursor = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(doc! {
            "status": "reserved",
            "forwarded": false,
            "released": false,
            "updated_at": { "$lt": bson::DateTime::from_chrono(cutoff) },
        })
        .await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }

    let mut released_count = 0;
    for row in rows {
        if release_one_unforwarded_row(db, &row, UsageStatus::Abandoned, None).await? {
            released_count += 1;
        }
    }
    Ok(released_count)
}

pub async fn recover_retryable_settlements(
    db: &mongodb::Database,
) -> AppResult<SettlementRecoveryStats> {
    recover_retryable_settlements_at(db, Utc::now()).await
}

pub(crate) async fn recover_retryable_settlements_at(
    db: &mongodb::Database,
    now: chrono::DateTime<Utc>,
) -> AppResult<SettlementRecoveryStats> {
    super::meter::recover_pending_resale_intents(db).await?;

    let due = settlement_due_filter(now);
    let mut query = due.clone();
    query.insert("released", false);
    query.insert("quantity", doc! { "$ne": Bson::Null });
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(query)
        .sort(doc! {
            "settlement_next_retry_at": 1,
            "finalized_at": 1,
        })
        .limit(RECOVERY_BATCH_SIZE)
        .await?
        .try_collect()
        .await?;

    let mut stats = SettlementRecoveryStats::default();
    for row in rows {
        if row.settlement_attempts >= MAX_SETTLEMENT_ATTEMPTS {
            if mark_exhausted_settlement_dead_letter(db, &row, now).await? {
                stats.dead_lettered += 1;
            }
            continue;
        }

        let Some(claimed) = claim_due_settlement(db, &row, now, due.clone()).await? else {
            continue;
        };
        match claim_released_and_settle(db, &claimed).await {
            Ok(true) => stats.recovered += 1,
            Ok(false) => {}
            Err(_error) => match record_settlement_failure(db, &claimed, now).await? {
                SettlementFailureDisposition::RetryScheduled(_) => stats.retried += 1,
                SettlementFailureDisposition::DeadLettered(_) => stats.dead_lettered += 1,
                SettlementFailureDisposition::Unchanged => {}
            },
        }
    }
    Ok(stats)
}

fn settlement_due_filter(now: chrono::DateTime<Utc>) -> Document {
    let stranded_before = now - Duration::seconds(SETTLEMENT_STRANDED_AFTER_SECS);
    doc! {
        "$or": [
            {
                "status": "failed",
                "$or": [
                    { "settlement_next_retry_at": { "$lte": bson::DateTime::from_chrono(now) } },
                    { "settlement_next_retry_at": null },
                    { "settlement_next_retry_at": { "$exists": false } },
                ],
            },
            {
                "status": "finalized",
                "$or": [
                    { "finalized_at": { "$lte": bson::DateTime::from_chrono(stranded_before) } },
                    {
                        "finalized_at": null,
                        "updated_at": { "$lte": bson::DateTime::from_chrono(stranded_before) },
                    },
                ],
            },
        ]
    }
}

async fn claim_due_settlement(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    now: chrono::DateTime<Utc>,
    due: Document,
) -> AppResult<Option<UsageMeterRow>> {
    let lease_until = now + Duration::seconds(SETTLEMENT_CLAIM_LEASE_SECS);
    db.collection::<UsageMeterRow>(USAGE_METER)
        .find_one_and_update(
            doc! {
                "_id": &row.id,
                "released": false,
                "quantity": { "$ne": null },
                "$and": [due, settlement_attempt_filter(row.settlement_attempts)],
            },
            doc! {
                "$set": {
                    "status": "failed",
                    "settlement_next_retry_at": bson::DateTime::from_chrono(lease_until),
                    "updated_at": bson::DateTime::from_chrono(now),
                }
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await
        .map_err(Into::into)
}

async fn mark_exhausted_settlement_dead_letter(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    now: chrono::DateTime<Utc>,
) -> AppResult<bool> {
    if finish_applied_lock_before_dead_letter(db, row).await? {
        return Ok(false);
    }
    let result = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! {
                "_id": &row.id,
                "status": "failed",
                "released": false,
                "$and": [settlement_attempt_filter(row.settlement_attempts)],
            },
            doc! {
                "$set": {
                    "status": "dead_letter",
                    "last_error": "billing settlement retries exhausted",
                    "updated_at": bson::DateTime::from_chrono(now),
                },
                "$unset": { "settlement_next_retry_at": "" },
            },
        )
        .await?;
    if result.modified_count == 1 {
        tracing::warn!(
            row_id = %row.id,
            transaction_id = %row.transaction_id,
            settlement_attempts = row.settlement_attempts,
            quantity = row.quantity.unwrap_or(0),
            "Billing settlement moved to dead letter"
        );
    }
    Ok(result.modified_count == 1)
}

async fn estimate_layer_reservations(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
) -> AppResult<Vec<LayerReservation>> {
    let mut reservations = Vec::new();

    if ctx.platform_billable {
        let metric_code = platform_metric_code(ctx.platform_metric);
        reservations.push(LayerReservation {
            layer: BillingLayer::Platform,
            reserved_credits: estimate_credits(db, metric_code, None, 1).await?,
        });
    }

    if let Some(resale) = &ctx.resale {
        reservations.push(LayerReservation {
            layer: BillingLayer::Resale,
            reserved_credits: estimate_credits(db, &resale.lago_metric_code, None, 1).await?,
        });
    }

    Ok(reservations)
}

async fn estimate_credits(
    db: &mongodb::Database,
    lago_metric_code: &str,
    model: Option<&str>,
    quantity: i64,
) -> AppResult<i64> {
    if quantity <= 0 {
        return Ok(0);
    }

    let rate = find_rate(db, lago_metric_code, model)
        .await?
        .ok_or_else(|| {
            AppError::BillingNotConfigured(format!(
                "billing rate cache is missing for metric {lago_metric_code}"
            ))
        })?;
    Ok(credits_from_micros(rate.credits_per_unit_micros, quantity))
}

async fn find_rate(
    db: &mongodb::Database,
    lago_metric_code: &str,
    model: Option<&str>,
) -> AppResult<Option<BillingRateCache>> {
    let collection = db.collection::<BillingRateCache>(BILLING_RATE_CACHE);
    if let Some(model) = model
        && let Some(rate) = collection
            .find_one(doc! { "_id": BillingRateCache::cache_id(lago_metric_code, Some(model)) })
            .await?
    {
        return Ok(Some(rate));
    }
    collection
        .find_one(doc! { "_id": BillingRateCache::cache_id(lago_metric_code, None) })
        .await
        .map_err(Into::into)
}

fn credits_from_micros(credits_per_unit_micros: i64, quantity: i64) -> i64 {
    if credits_per_unit_micros <= 0 || quantity <= 0 {
        return 0;
    }

    let micros = i128::from(credits_per_unit_micros) * i128::from(quantity);
    let credits = (micros + i128::from(CREDIT_MICROS - 1)) / i128::from(CREDIT_MICROS);
    credits.min(i128::from(i64::MAX)) as i64
}

async fn release_one_unforwarded_row(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    terminal_status: UsageStatus,
    reason: Option<&str>,
) -> AppResult<bool> {
    let now = Utc::now();
    let mut set_doc = doc! {
        "status": bson::to_bson(&terminal_status)
            .unwrap_or_else(|_| bson::Bson::String("failed".to_string())),
        "released": true,
        "updated_at": bson::DateTime::from_chrono(now),
        "finalized_at": bson::DateTime::from_chrono(now),
    };
    if let Some(reason) = reason {
        set_doc.insert("last_error", reason);
    }

    let claimed = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one_and_update(
            doc! {
                "_id": &row.id,
                "forwarded": false,
                "status": "reserved",
                "released": false,
            },
            doc! { "$set": set_doc },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::Before)
                .build(),
        )
        .await?;

    let Some(claimed) = claimed else {
        return Ok(false);
    };

    release_wallet_hold(db, &claimed.billing_owner_id, claimed.reserved_credits).await?;
    Ok(true)
}

async fn suspend_wallet(db: &mongodb::Database, owner_id: &str) -> AppResult<()> {
    db.collection::<BillingWallet>(BILLING_WALLET)
        .update_one(
            doc! { "owner_id": owner_id },
            doc! {
                "$set": {
                    "suspended": true,
                    "collection_state": "suspended",
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;
    Ok(())
}

fn is_entitled(ctx: &BillingRouteContext, entitlements: &[Entitlement]) -> bool {
    let candidates = entitlement_candidates(ctx);
    entitlements.iter().any(|entitlement| {
        entitlement.code == "*"
            || entitlement.code == "all_services"
            || candidates.contains(&entitlement.code)
    })
}

fn entitlement_candidates(ctx: &BillingRouteContext) -> BTreeSet<String> {
    [
        ctx.service_slug.clone(),
        ctx.catalog_service_id.clone(),
        ctx.user_service_id.clone(),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.trim().is_empty())
    .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::Utc;
    use mongodb::bson::doc;
    use serde_json::json;
    use uuid::Uuid;

    use crate::models::billing_rate_cache::BillingRateCache;
    use crate::models::billing_wallet::{BillingWallet, CollectionState, PlanKind};
    use crate::models::service_billing::{BillingMetric, ServiceBilling};
    use crate::models::usage_meter::CredentialClass;
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput,
    };
    use crate::services::billing::route_context::{BillingRouteContext, NodeIntent};
    use crate::test_utils::connect_test_database;

    use super::{LagoApi, gate_and_reserve, try_reserve_prepaid};

    #[derive(Clone)]
    struct EntitledLago {
        entitlements: Vec<Entitlement>,
    }

    #[async_trait]
    impl LagoApi for EntitledLago {
        async fn ensure_customer(
            &self,
            owner: &OwnerProvisionInput,
        ) -> crate::errors::AppResult<String> {
            Ok(owner.external_customer_id.clone())
        }

        async fn ensure_subscription(
            &self,
            customer_id: &str,
            _plan_code: &str,
        ) -> crate::errors::AppResult<String> {
            Ok(customer_id.to_string())
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
        ) -> crate::errors::AppResult<LagoUsage> {
            Ok(LagoUsage {
                customer_id: customer_id.to_string(),
                subscription_id: subscription_id.to_string(),
                raw: json!({}),
            })
        }

        async fn wallet_balance(&self, _customer_id: &str) -> crate::errors::AppResult<i64> {
            Ok(0)
        }

        async fn entitlements(
            &self,
            _subscription_id: &str,
        ) -> crate::errors::AppResult<Vec<Entitlement>> {
            Ok(self.entitlements.clone())
        }
    }

    fn wallet(owner_id: &str, balance_credits: i64) -> BillingWallet {
        let now = Utc::now();
        BillingWallet {
            id: Uuid::new_v4().to_string(),
            owner_id: owner_id.to_string(),
            lago_customer_id: owner_id.to_string(),
            lago_wallet_id: Some(format!("{owner_id}:wallet")),
            lago_subscription_id: Some(format!("{owner_id}:plan")),
            plan_kind: PlanKind::Prepaid,
            balance_credits,
            reserved_credits: 0,
            pending_lago_debits: 0,
            has_payment_instrument: false,
            overdraft_cap_credits: 0,
            suspended: false,
            collection_state: CollectionState::Good,
            balance_synced_at: now,
            created_at: now,
            updated_at: now,
        }
    }

    async fn insert_platform_rate(db: &mongodb::Database, credits: i64) {
        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id("platform_requests", None),
                lago_metric_code: "platform_requests".to_string(),
                model: None,
                credits_per_unit_micros: credits * 1_000_000,
                synced_at: Utc::now(),
            })
            .await
            .expect("insert platform rate");
    }

    fn route_context(owner_id: &str) -> BillingRouteContext {
        BillingRouteContext::new(
            Uuid::new_v4().to_string(),
            owner_id.to_string(),
            "actor-1".to_string(),
            None,
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("service-one".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::UserOwned,
            BillingMetric::Requests,
            None::<&ServiceBilling>,
            false,
        )
        .with_platform_metering(true)
    }

    #[tokio::test]
    async fn concurrent_prepaid_reserves_never_overcommit_wallet() {
        let Some(db) = connect_test_database("billing_reserve_concurrency").await else {
            return;
        };
        let owner_id = "owner-concurrent";
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(wallet(owner_id, 10))
            .await
            .expect("insert wallet");

        let db = Arc::new(db);
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let db = db.clone();
            tasks.push(tokio::spawn(async move {
                try_reserve_prepaid(&db, owner_id, 1)
                    .await
                    .expect("reserve query")
                    .is_some()
            }));
        }

        let mut successes = 0;
        for task in tasks {
            if task.await.expect("reserve task") {
                successes += 1;
            }
        }
        let saved = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");

        assert_eq!(successes, 10);
        assert_eq!(saved.reserved_credits, 10);
        assert_eq!(saved.available_credits(), 0);
    }

    #[tokio::test]
    async fn gate_fails_closed_when_entitlement_is_missing() {
        let Some(db) = connect_test_database("billing_gate_entitlement").await else {
            return;
        };
        let owner_id = "owner-no-entitlement";
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(wallet(owner_id, 10))
            .await
            .expect("insert wallet");
        insert_platform_rate(&db, 1).await;
        let lago = EntitledLago {
            entitlements: Vec::new(),
        };

        let err = gate_and_reserve(&db, Some(&lago), &route_context(owner_id), false)
            .await
            .expect_err("missing entitlement must deny");

        assert!(matches!(
            err,
            crate::errors::AppError::PlanEntitlementRequired(_)
        ));
    }

    #[tokio::test]
    async fn missing_wallet_degrades_to_meter_only() {
        let Some(db) = connect_test_database("billing_gate_missing_wallet").await else {
            return;
        };
        let owner_id = "owner-missing-wallet";
        let reservation = gate_and_reserve(&db, None, &route_context(owner_id), false)
            .await
            .expect("missing wallet should not deny proxy traffic");

        assert!(reservation.is_none());
    }

    #[tokio::test]
    async fn gate_reserves_against_cached_rate_when_entitled() {
        let Some(db) = connect_test_database("billing_gate_reserves").await else {
            return;
        };
        let owner_id = "owner-entitled";
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(wallet(owner_id, 10))
            .await
            .expect("insert wallet");
        insert_platform_rate(&db, 3).await;
        let lago = EntitledLago {
            entitlements: vec![Entitlement {
                code: "service-one".to_string(),
                raw: json!({}),
            }],
        };

        let reservation = gate_and_reserve(&db, Some(&lago), &route_context(owner_id), false)
            .await
            .expect("gate")
            .expect("reservation");
        let saved = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");

        assert_eq!(reservation.total_reserved_credits, 3);
        assert_eq!(saved.reserved_credits, 3);
        assert_eq!(saved.available_credits(), 7);
    }
}
