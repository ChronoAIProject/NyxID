//! Finite account-balance reservations using the same documents as service usage.
//!
//! This module owns money holds, not Machine eligibility or lifecycle. The caller
//! must validate the accepted quote, owner, provider and immutable price, then
//! commit the returned allocations together with its recoverable usage row and
//! lifecycle intent in the same transaction. An error requires aborting the
//! transaction; a caller must never commit a partial reservation.

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, Document, doc};

use crate::errors::{AppError, AppResult};
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS};
use crate::models::credit_grant::{COLLECTION_NAME as GRANTS, CreditGrant};
use crate::models::credits::Credits;
use crate::models::service_billing::BillingMetric;
use crate::models::usage_allowance::{COLLECTION_NAME as ALLOWANCES, UsageAllowance};
use crate::models::usage_allowance_period::{COLLECTION_NAME as PERIODS, UsageAllowancePeriod};
use crate::models::usage_meter::{
    AllowanceReservationAllocation, BillingLayer, GrantReservationAllocation,
};

use super::super::reservation::{BillingReservation, LayerReservation};

/// Only empty allowance periods are prepared outside the caller's transaction.
/// Every eligibility and balance decision is re-read before a hold is written.
/// Fields are private so callers cannot change the owner or SKU after preparation.
#[derive(Clone, Debug)]
pub struct MachineFundingCandidates {
    owner_id: String,
    service_id: Option<String>,
    service_slug: String,
    periods: Vec<UsageAllowancePeriod>,
}

pub async fn prepare_machine_window_funding(
    db: &mongodb::Database,
    owner_id: &str,
    runtime_service_id: Option<&str>,
    runtime_service_slug: &str,
    now: DateTime<Utc>,
) -> AppResult<MachineFundingCandidates> {
    if owner_id.is_empty() || runtime_service_slug.is_empty() {
        return Err(AppError::ValidationError(
            "Machine funding identity is required".into(),
        ));
    }
    let mut periods = Vec::new();
    for allowance in super::super::allowances::applicable_allowances(
        db,
        owner_id,
        runtime_service_id,
        Some(runtime_service_slug),
        BillingMetric::MachineRunningSeconds,
    )
    .await?
    {
        periods.push(
            super::super::allowances::ensure_current_period(db, &allowance, owner_id, now).await?,
        );
    }
    for period in &periods {
        if super::super::allowances::available_period_quantity(period) > 0
            && let Some(lock) = &period.active_settlement
        {
            super::complete_allowance_lock(db, &period.id, &period.allowance_id, lock).await?;
        }
    }
    for grant in super::super::grants::list_active_for_user(db, owner_id, now).await? {
        if super::super::grants::service_scope_applies(
            &grant.scope,
            runtime_service_id,
            Some(runtime_service_slug),
        ) && super::super::service_limits::available_for_service(
            &grant,
            runtime_service_id,
            Some(runtime_service_slug),
        )? > Credits::ZERO
            && let Some(lock) = &grant.active_settlement
        {
            super::complete_grant_lock(db, &grant.id, lock).await?;
        }
    }
    periods.sort_by(|left, right| super::expiry_order(left.period_end, right.period_end));
    Ok(MachineFundingCandidates {
        owner_id: owner_id.into(),
        service_id: runtime_service_id.map(str::to_owned),
        service_slug: runtime_service_slug.into(),
        periods,
    })
}

/// Reserve a single finite Machine window in a caller-owned transaction.
///
/// The frozen rate is supplied by the accepted runtime contract; this function
/// never reads the mutable price cache. It always uses real account availability
/// (including pending Lago debits and purchased-credit expiry), even for wallets
/// with a payment instrument and overdraft allowance. Service reservations use
/// these exact same wallet, allowance-period and grant fields, so MongoDB write
/// conflicts serialize concurrent consumers without a Machine-only balance.
#[allow(clippy::too_many_arguments)]
pub async fn reserve_machine_window_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    owner_id: &str,
    wallet_id: &str,
    candidates: &MachineFundingCandidates,
    mut layer: LayerReservation,
    now: DateTime<Utc>,
) -> AppResult<BillingReservation> {
    validate_layer(owner_id, candidates, &layer)?;
    super::super::exact_migration::require_ready(db).await?;
    let wallets = db.collection::<BillingWallet>(WALLETS);
    let wallet_document = db
        .collection::<Document>(WALLETS)
        .find_one(doc! { "_id": wallet_id, "owner_id": owner_id })
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::BillingNotConfigured("Machine wallet is required".into()))?;
    // An unapplied service settlement may debit more than its earlier estimate.
    // Until its shared crash bridge is journaled and cleared, the apparent free
    // balance is not authoritative for a new strictly funded runtime window.
    if wallet_document
        .get("active_settlement")
        .is_some_and(|value| !matches!(value, Bson::Null))
    {
        return Err(AppError::BillingProviderUnavailable(
            "Machine wallet settlement requires reconciliation".into(),
        ));
    }
    let wallet: BillingWallet = bson::from_document(wallet_document)
        .map_err(|error| AppError::Internal(format!("Machine wallet is invalid: {error}")))?;
    if wallet.is_suspended() {
        return Err(AppError::WalletSuspended);
    }
    if wallet
        .lago_subscription_id
        .as_deref()
        .is_none_or(str::is_empty)
        || wallet.lago_wallet_id.as_deref().is_none_or(str::is_empty)
        || wallet.lago_customer_id.is_empty()
    {
        return Err(AppError::BillingNotConfigured(
            "Machine billing account is incomplete".into(),
        ));
    }

    // Reuse the shared live membership policy, rather than expanding a second
    // interpretation of org/group targeting. The caller separately fences owner
    // eligibility with its lifecycle mutation.
    let target_clauses = super::super::targets::allowance_clauses(db, owner_id).await?;
    let mut service_clauses = vec![doc! { "service_slug": &candidates.service_slug }];
    if let Some(service_id) = &candidates.service_id {
        service_clauses.push(doc! { "service_id": service_id });
    }
    let mut remaining = layer.estimated_quantity;
    for candidate in &candidates.periods {
        if remaining == 0 {
            break;
        }
        let Some(definition) = db
            .collection::<UsageAllowance>(ALLOWANCES)
            .find_one(doc! {
                "_id": &candidate.allowance_id,
                "is_active": true,
                "metric": BillingMetric::MachineRunningSeconds.as_str(),
                "$and": [
                    { "$or": service_clauses.clone() },
                    { "$or": target_clauses.clone() },
                ],
            })
            .session(&mut *session)
            .await?
        else {
            continue;
        };
        let window = super::super::allowances::allowance_window(
            definition.recurrence,
            definition.created_at,
            now,
        );
        if super::super::allowances::period_id(&definition.id, owner_id, window.start)
            != candidate.id
        {
            continue;
        }
        let periods = db.collection::<UsageAllowancePeriod>(PERIODS);
        let Some(period) = periods
            .find_one(doc! {
                "_id": &candidate.id, "allowance_id": &definition.id, "owner_user_id": owner_id,
            })
            .session(&mut *session)
            .await?
        else {
            continue;
        };
        if period.period_start > now || period.period_end.is_some_and(|end| end <= now) {
            continue;
        }
        // A definition can shrink after preparation; preserve existing holds but
        // never reserve new units against its superseded larger quantity.
        let committed = period
            .consumed_quantity
            .checked_add(period.reserved_quantity)
            .ok_or_else(|| AppError::Internal("Allowance quantity overflow".into()))?;
        let total = definition.quantity.max(committed);
        let available = total
            .checked_sub(committed)
            .ok_or_else(|| AppError::Internal("Allowance quantity overflow".into()))?
            .max(0);
        let wanted = remaining.min(available);
        if wanted == 0 {
            continue;
        }
        if period.active_settlement.is_some() {
            return Err(AppError::Conflict(
                "Machine allowance settlement is completing; retry".into(),
            ));
        }
        let changed = periods.update_one(doc! { "_id": &period.id }, doc! {
            "$inc": { "reserved_quantity": wanted },
            "$set": { "total_quantity": total, "updated_at": bson::DateTime::from_chrono(now) },
        }).session(&mut *session).await?;
        if changed.modified_count != 1 {
            return Err(AppError::Conflict(
                "Machine allowance changed during admission".into(),
            ));
        }
        layer
            .allowance_reservations
            .push(AllowanceReservationAllocation {
                allowance_id: definition.id,
                period_id: period.id,
                quantity: wanted,
            });
        remaining -= wanted;
    }

    let rate = super::super::amounts::rate_pico(
        layer.credits_per_unit_pico,
        layer.credits_per_unit_micros,
    );
    let mut remaining_cost = super::super::amounts::cost(rate, remaining)?;
    let grants_collection = db.collection::<CreditGrant>(GRANTS);
    let mut cursor = grants_collection
        .find(doc! {
            "recipient_user_id": owner_id,
            "status": "active",
            "issued_ledgered_at": { "$type": "date" },
            "remaining": { "$gt": Credits::ZERO },
            "$or": [
                { "expires_at": Bson::Null },
                { "expires_at": { "$gt": bson::DateTime::from_chrono(now) } },
            ],
        })
        .session(&mut *session)
        .await?;
    let mut grants: Vec<CreditGrant> = cursor.stream(&mut *session).try_collect().await?;
    grants.retain(|grant| {
        super::super::grants::service_scope_applies(
            &grant.scope,
            candidates.service_id.as_deref(),
            Some(&candidates.service_slug),
        )
    });
    grants.sort_by(|left, right| {
        super::expiry_order(left.expires_at, right.expires_at)
            .then_with(|| left.created_at.cmp(&right.created_at))
    });
    for grant in grants {
        if remaining_cost == Credits::ZERO {
            break;
        }
        if grant.revoke_requested_at.is_some() {
            continue;
        }
        if super::super::service_limits::available_for_service(
            &grant,
            candidates.service_id.as_deref(),
            Some(&candidates.service_slug),
        )? <= Credits::ZERO
        {
            continue;
        }
        if grant.active_settlement.is_some() {
            return Err(AppError::Conflict(
                "Issued credit settlement is completing; retry".into(),
            ));
        }
        let wanted = remaining_cost.min(super::super::service_limits::available_for_service(
            &grant,
            candidates.service_id.as_deref(),
            Some(&candidates.service_slug),
        )?);
        if wanted <= Credits::ZERO {
            continue;
        }
        let (mut filter, update, key) = super::super::service_limits::reservation_change(
            &grant,
            candidates.service_id.as_deref(),
            Some(&candidates.service_slug),
            wanted,
            now,
        )?;
        filter.insert("recipient_user_id", owner_id);
        let changed = grants_collection
            .update_one(filter, update)
            .session(&mut *session)
            .await?;
        if changed.modified_count != 1 {
            return Err(AppError::Conflict(
                "Machine grant changed during admission".into(),
            ));
        }
        layer.grant_reservations.push(GrantReservationAllocation {
            grant_id: grant.id,
            amount: wanted,
            service_limit_key: key,
        });
        remaining_cost = remaining_cost.checked_sub(wanted)?;
    }

    if remaining_cost > Credits::ZERO && wallet.available_credits()? < remaining_cost {
        return Err(AppError::InsufficientCredits);
    }
    if remaining_cost > Credits::ZERO {
        // Checked before MongoDB arithmetic, so accepted values stay within the
        // same Decimal128 range as every other exact Billing path.
        wallet.reserved_credits.checked_add(remaining_cost)?;
        let changed = wallets
            .update_one(
                doc! { "_id": &wallet.id, "owner_id": owner_id },
                doc! {
                    "$inc": { "reserved_credits": remaining_cost },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) },
                },
            )
            .session(&mut *session)
            .await?;
        if changed.modified_count != 1 {
            return Err(AppError::Conflict(
                "Machine wallet changed during admission".into(),
            ));
        }
    }
    layer.reserved_credits = remaining_cost;
    Ok(BillingReservation {
        owner_id: owner_id.into(),
        wallet_id: wallet.id,
        total_reserved_credits: remaining_cost,
        layers: vec![layer],
    })
}

fn validate_layer(
    owner_id: &str,
    candidates: &MachineFundingCandidates,
    layer: &LayerReservation,
) -> AppResult<()> {
    if owner_id != candidates.owner_id
        || layer.layer != BillingLayer::Platform
        || layer.metric != BillingMetric::MachineRunningSeconds
        || layer.estimated_quantity <= 0
        || layer.estimated_quantity > super::super::machine_runtime::MAX_WINDOW_SECONDS
        || layer.lago_metric_code.is_empty()
        || super::super::amounts::rate_pico(
            layer.credits_per_unit_pico,
            layer.credits_per_unit_micros,
        ) <= 0
        || !layer.allowance_reservations.is_empty()
        || !layer.grant_reservations.is_empty()
    {
        return Err(AppError::ValidationError(
            "Invalid finite Machine funding reservation".into(),
        ));
    }
    Ok(())
}

pub(super) fn is_machine_runtime(row: &crate::models::usage_meter::UsageMeterRow) -> bool {
    row.metric == BillingMetric::MachineRunningSeconds
}

/// Unknown-stop rows retain quantity=None. Only the runtime evidence transition
/// is permitted to finalize them; generic metering-only or free-row semantics
/// cannot release their liability. Capacity checks precede any funding mutation.
pub(super) fn validate_machine_settlement(
    row: &crate::models::usage_meter::UsageMeterRow,
) -> AppResult<()> {
    if !is_machine_runtime(row) {
        return Ok(());
    }
    let invalid = || {
        AppError::BillingProviderUnavailable(
            "Machine settlement requires finalized, frozen and fully reserved usage".into(),
        )
    };
    let funding = row.funding.as_ref().ok_or_else(invalid)?;
    let rate = funding
        .settlement_rate
        .filter(|rate| *rate > Credits::ZERO)
        .ok_or_else(invalid)?;
    let quantity = row
        .quantity
        .filter(|quantity| *quantity >= 0)
        .ok_or_else(invalid)?;
    if row.wallet_id.is_none()
        || !matches!(
            row.status,
            crate::models::usage_meter::UsageStatus::Finalized
                | crate::models::usage_meter::UsageStatus::Failed
        )
        || row.layer != BillingLayer::Platform
        || funding.wallet_only
        || row.reserved_credits < Credits::ZERO
    {
        return Err(invalid());
    }
    let mut allowance_quantity = 0_i64;
    for allocation in &funding.allowance_reservations {
        if allocation.quantity < 0 {
            return Err(invalid());
        }
        allowance_quantity = allowance_quantity
            .checked_add(allocation.quantity)
            .ok_or_else(invalid)?;
    }
    let mut capacity = rate
        .checked_mul(allowance_quantity)?
        .checked_add(row.reserved_credits)?;
    for allocation in &funding.grant_reservations {
        if allocation.amount < Credits::ZERO {
            return Err(invalid());
        }
        capacity = capacity.checked_add(allocation.amount)?;
    }
    if rate.checked_mul(quantity)? > capacity {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
#[path = "machine_tests.rs"]
mod machine_tests;
