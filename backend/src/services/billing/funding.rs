use std::cmp::Ordering;

use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, Document, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::billing_ledger::BillingLedgerEventType;
use crate::models::credit_grant::{
    COLLECTION_NAME as CREDIT_GRANTS, CreditGrant, CreditGrantSettlementLock,
};
use crate::models::usage_allowance_period::{
    AllowanceSettlementLock, COLLECTION_NAME as USAGE_ALLOWANCE_PERIODS, UsageAllowancePeriod,
};
use crate::models::usage_meter::{
    AllowanceConsumptionAllocation, AllowanceReservationAllocation, BillingLayer,
    COLLECTION_NAME as USAGE_METER, DeferredQuantity, GrantConsumptionAllocation,
    GrantReservationAllocation, UsageFunding, UsageMeterRow,
};

use super::grants::CREDIT_MICROS;
use super::reservation::{LayerReservation, whole_credits_for_micros};
use super::route_context::BillingRouteContext;

const FUNDING_CLAIM_LEASE_SECS: i64 = 60;
const RESOURCE_LOCK_RETRIES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FundingSettlement {
    pub wallet_charge_credits: i64,
    pub lago_billable_quantity_micros: i64,
}

pub async fn reserve_estimated_funding(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
    layers: &mut [LayerReservation],
) -> AppResult<()> {
    for index in 0..layers.len() {
        let result = reserve_layer(db, ctx, &mut layers[index]).await;
        if let Err(error) = result {
            release_layer_reservations(db, &layers[..=index]).await?;
            return Err(error);
        }
    }
    Ok(())
}

async fn reserve_layer(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
    layer: &mut LayerReservation,
) -> AppResult<()> {
    let mut uncovered_quantity = layer.estimated_quantity.max(0);
    if layer.layer == BillingLayer::Platform && uncovered_quantity > 0 {
        layer.allowance_reservations = reserve_allowances(
            db,
            &ctx.billing_owner_id,
            ctx.catalog_service_id
                .as_deref()
                .or(ctx.user_service_id.as_deref()),
            ctx.service_slug.as_deref(),
            layer.metric,
            uncovered_quantity,
        )
        .await?;
        let reserved_units: i64 = layer
            .allowance_reservations
            .iter()
            .map(|allocation| allocation.quantity)
            .sum();
        uncovered_quantity = uncovered_quantity.saturating_sub(reserved_units);
    }

    let base_fee_micros = layer.base_fee_micros.max(0);
    layer.base_fee_grant_reservations = reserve_grants(
        db,
        &ctx.billing_owner_id,
        ctx.catalog_service_id
            .as_deref()
            .or(ctx.user_service_id.as_deref()),
        ctx.service_slug.as_deref(),
        base_fee_micros,
    )
    .await?;
    let base_fee_grant_micros: i64 = layer
        .base_fee_grant_reservations
        .iter()
        .map(|allocation| allocation.amount_micros.max(0))
        .sum();
    let quantity_micros = saturating_cost_micros(layer.credits_per_unit_micros, uncovered_quantity);
    layer.grant_reservations = reserve_grants(
        db,
        &ctx.billing_owner_id,
        ctx.catalog_service_id
            .as_deref()
            .or(ctx.user_service_id.as_deref()),
        ctx.service_slug.as_deref(),
        quantity_micros,
    )
    .await?;
    let grant_micros: i64 = layer
        .grant_reservations
        .iter()
        .map(|allocation| allocation.amount_micros)
        .sum();
    let wallet_micros = base_fee_micros
        .saturating_sub(base_fee_grant_micros)
        .saturating_add(quantity_micros.saturating_sub(grant_micros));
    layer.reserved_credits = whole_credits_for_micros(wallet_micros);
    Ok(())
}

async fn reserve_allowances(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: Option<&str>,
    service_slug: Option<&str>,
    metric: crate::models::service_billing::BillingMetric,
    quantity: i64,
) -> AppResult<Vec<AllowanceReservationAllocation>> {
    let definitions = super::allowances::applicable_allowances(
        db,
        owner_user_id,
        service_id,
        service_slug,
        metric,
    )
    .await?;
    let now = Utc::now();
    let mut candidates = Vec::new();
    for definition in definitions {
        let period =
            super::allowances::ensure_current_period(db, &definition, owner_user_id, now).await?;
        candidates.push((definition, period));
    }
    candidates.sort_by(|(_, left), (_, right)| expiry_order(left.period_end, right.period_end));

    let mut needed = quantity.max(0);
    let mut allocations = Vec::new();
    for (allowance, period) in candidates {
        if needed == 0 {
            break;
        }
        let wanted = needed.min(super::allowances::available_period_quantity(&period));
        if wanted <= 0 {
            continue;
        }
        let updated = db
            .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .update_one(
                doc! {
                    "_id": &period.id,
                    "$expr": {
                        "$gte": [
                            {
                                "$subtract": [
                                    "$total_quantity",
                                    { "$add": ["$consumed_quantity", "$reserved_quantity"] },
                                ]
                            },
                            wanted,
                        ]
                    },
                },
                doc! {
                    "$inc": { "reserved_quantity": wanted },
                    "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
                },
            )
            .await?;
        if updated.modified_count == 0 {
            continue;
        }
        allocations.push(AllowanceReservationAllocation {
            allowance_id: allowance.id,
            period_id: period.id,
            quantity: wanted,
        });
        needed -= wanted;
    }
    Ok(allocations)
}

async fn reserve_grants(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: Option<&str>,
    service_slug: Option<&str>,
    amount_micros: i64,
) -> AppResult<Vec<GrantReservationAllocation>> {
    let now = Utc::now();
    let mut grants = super::grants::list_active_for_user(db, owner_user_id, now).await?;
    grants.retain(|grant| {
        super::grants::service_scope_applies(&grant.scope, service_id, service_slug)
    });
    grants.sort_by(|left, right| {
        expiry_order(left.expires_at, right.expires_at)
            .then_with(|| left.created_at.cmp(&right.created_at))
    });

    let mut needed = amount_micros.max(0);
    let mut allocations = Vec::new();
    for grant in grants {
        if needed == 0 {
            break;
        }
        let wanted = needed.min(super::grants::available_grant_micros(&grant));
        if wanted <= 0 {
            continue;
        }
        let updated = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .update_one(
                spendable_grant_filter(&grant.id, wanted, now),
                doc! {
                    "$inc": { "reserved_micros": wanted },
                    "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
                },
            )
            .await?;
        if updated.modified_count == 0 {
            continue;
        }
        allocations.push(GrantReservationAllocation {
            grant_id: grant.id,
            amount_micros: wanted,
        });
        needed -= wanted;
    }
    Ok(allocations)
}

pub async fn release_layer_reservations(
    db: &mongodb::Database,
    layers: &[LayerReservation],
) -> AppResult<()> {
    for layer in layers {
        for reservation in &layer.allowance_reservations {
            release_allowance_reservation(db, reservation).await?;
        }
        for reservation in &layer.grant_reservations {
            release_grant_reservation(db, reservation).await?;
        }
        for reservation in &layer.base_fee_grant_reservations {
            release_grant_reservation(db, reservation).await?;
        }
    }
    Ok(())
}

pub async fn release_usage_reservations(
    db: &mongodb::Database,
    row: &UsageMeterRow,
) -> AppResult<()> {
    let Some(funding) = row.funding.as_ref() else {
        return Ok(());
    };
    for reservation in &funding.allowance_reservations {
        let operation_id = format!("{}:allowance-release:{}", row.id, reservation.period_id);
        if has_allowance_operation(row, &operation_id) {
            finish_allowance_operation_if_locked(db, &reservation.period_id, &operation_id).await?;
        } else {
            settle_allowance_resource(
                db,
                row,
                &reservation.allowance_id,
                &reservation.period_id,
                &operation_id,
                reservation.quantity.max(0),
                0,
                true,
            )
            .await?;
        }
    }
    for reservation in &funding.grant_reservations {
        let operation_id = format!("{}:grant-release:{}", row.id, reservation.grant_id);
        if has_grant_operation(row, &operation_id) {
            finish_grant_operation_if_locked(db, &reservation.grant_id, &operation_id).await?;
        } else {
            settle_grant_resource(
                db,
                row,
                &reservation.grant_id,
                &operation_id,
                reservation.amount_micros.max(0),
                0,
                true,
            )
            .await?;
        }
    }
    for reservation in &funding.base_fee_grant_reservations {
        let operation_id = format!("{}:base-fee-grant-release:{}", row.id, reservation.grant_id);
        if has_base_fee_grant_operation(row, &operation_id) {
            finish_grant_operation_if_locked(db, &reservation.grant_id, &operation_id).await?;
        } else {
            settle_grant_resource(
                db,
                row,
                &reservation.grant_id,
                &operation_id,
                reservation.amount_micros.max(0),
                0,
                true,
            )
            .await?;
        }
    }
    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! { "_id": &row.id, "funding.settled": { "$ne": true } },
            doc! {
                "$set": {
                    "funding.settled": true,
                    "funding.wallet_charge_credits": 0_i64,
                    "funding.lago_billable_quantity_micros": 0_i64,
                    "funding.settled_at": bson::DateTime::from_chrono(Utc::now()),
                },
                "$unset": {
                    "funding.settlement_claim_id": "",
                    "funding.settlement_claimed_at": "",
                },
            },
        )
        .await?;
    Ok(())
}

pub async fn recover_terminal_releases(db: &mongodb::Database) -> AppResult<u64> {
    let mut recovered = recover_resource_settlement_locks(db).await?;
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(doc! {
            "released": true,
            "status": { "$in": ["failed", "abandoned"] },
            "funding": { "$ne": Bson::Null },
            "funding.settled": { "$ne": true },
        })
        .sort(doc! { "updated_at": 1 })
        .limit(500)
        .await?
        .try_collect()
        .await?;
    for row in rows {
        release_usage_reservations(db, &row).await?;
        recovered += 1;
    }
    Ok(recovered)
}

/// Complete resource operations left between their durable lock and lock
/// clear. Any worker may help these operations: every mutation and usage-row
/// allocation is guarded by the operation id, and grant ledger appends are
/// deduplicated. This also recovers locks on rows that exhausted the normal
/// settlement retry budget before a later request needs the same benefit.
async fn recover_resource_settlement_locks(db: &mongodb::Database) -> AppResult<u64> {
    let allowance_periods: Vec<UsageAllowancePeriod> = db
        .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .find(doc! { "active_settlement": { "$type": "object" } })
        .sort(doc! { "active_settlement.updated_at": 1 })
        .limit(500)
        .await?
        .try_collect()
        .await?;
    let grants: Vec<CreditGrant> = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "active_settlement": { "$type": "object" } })
        .sort(doc! { "active_settlement.updated_at": 1 })
        .limit(500)
        .await?
        .try_collect()
        .await?;

    let mut recovered = 0;
    for period in allowance_periods {
        if let Some(lock) = period.active_settlement {
            complete_allowance_lock(db, &period.id, &period.allowance_id, &lock).await?;
            recovered += 1;
        }
    }
    for grant in grants {
        if let Some(lock) = grant.active_settlement {
            complete_grant_lock(db, &grant.id, &lock).await?;
            recovered += 1;
        }
    }
    Ok(recovered)
}

async fn release_allowance_reservation(
    db: &mongodb::Database,
    reservation: &AllowanceReservationAllocation,
) -> AppResult<()> {
    if reservation.quantity <= 0 {
        return Ok(());
    }
    db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .update_one(
            doc! {
                "_id": &reservation.period_id,
                "reserved_quantity": { "$gte": reservation.quantity },
            },
            doc! {
                "$inc": { "reserved_quantity": -reservation.quantity },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .await?;
    Ok(())
}

async fn release_grant_reservation(
    db: &mongodb::Database,
    reservation: &GrantReservationAllocation,
) -> AppResult<()> {
    if reservation.amount_micros <= 0 {
        return Ok(());
    }
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .update_one(
            doc! {
                "_id": &reservation.grant_id,
                "reserved_micros": { "$gte": reservation.amount_micros },
            },
            doc! {
                "$inc": { "reserved_micros": -reservation.amount_micros },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .await?;
    Ok(())
}

fn spendable_grant_filter(grant_id: &str, amount_micros: i64, now: DateTime<Utc>) -> Document {
    doc! {
        "_id": grant_id,
        "status": "active",
        "$and": [
            { "$or": [
                { "expires_at": Bson::Null },
                { "expires_at": { "$exists": false } },
                { "expires_at": { "$gt": bson::DateTime::from_chrono(now) } },
            ] },
            { "$expr": {
                "$gte": [
                    { "$subtract": ["$remaining_micros", "$reserved_micros"] },
                    amount_micros,
                ]
            } },
        ],
    }
}

fn expiry_order(left: Option<DateTime<Utc>>, right: Option<DateTime<Utc>>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn saturating_cost_micros(rate_micros: i64, quantity: i64) -> i64 {
    (i128::from(rate_micros.max(0)) * i128::from(quantity.max(0))).min(i128::from(i64::MAX)) as i64
}

pub async fn apply_deferred_base_fee(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    descriptor: &DeferredQuantity,
) -> AppResult<bool> {
    let mut claimed = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one(doc! { "_id": &row.id })
        .await?
        .ok_or_else(|| AppError::Internal("deferred usage row disappeared".to_string()))?;
    if claimed.base_fee_applied {
        return Ok(false);
    }

    let base_fee_micros = claimed.base_fee_micros.unwrap_or(0).max(0);
    let reservations = claimed
        .funding
        .as_ref()
        .map(|funding| funding.base_fee_grant_reservations.clone())
        .unwrap_or_default();
    for reservation in reservations {
        let operation_id = format!("{}:base-fee-grant:{}", claimed.id, reservation.grant_id);
        if has_base_fee_grant_operation(&claimed, &operation_id) {
            finish_grant_operation_if_locked(db, &reservation.grant_id, &operation_id).await?;
            continue;
        }
        if settle_grant_resource(
            db,
            &claimed,
            &reservation.grant_id,
            &operation_id,
            reservation.amount_micros.max(0),
            reservation.amount_micros.max(0),
            true,
        )
        .await?
        {
            refresh_claimed_funding(db, &mut claimed).await?;
        }
    }
    let grant_micros = claimed
        .funding
        .as_ref()
        .map(|funding| {
            funding
                .base_fee_grant_consumptions
                .iter()
                .map(|allocation| allocation.amount_micros.max(0))
                .sum::<i64>()
        })
        .unwrap_or(0)
        .min(base_fee_micros);
    let wallet_credits = whole_credits_for_micros(base_fee_micros.saturating_sub(grant_micros));
    super::reservation::apply_deferred_base_fee(db, &claimed, descriptor, wallet_credits).await
}

/// Convert a finalized usage row's funding reservations into actual
/// consumption. Each allowance/grant document has a single bounded settlement
/// lock. The resource mutation is applied first, then the operation id is
/// persisted on the usage row before the lock is cleared. A retry can therefore
/// finish either side of a crash without consuming the same benefit twice.
pub async fn settle_usage_funding(
    db: &mongodb::Database,
    row: &UsageMeterRow,
) -> AppResult<FundingSettlement> {
    let Some(funding) = row.funding.as_ref() else {
        let quantity = row.quantity.unwrap_or(0).max(0);
        return Ok(FundingSettlement {
            wallet_charge_credits: super::reservation::actual_credits_for_row(
                db,
                row,
                quantity,
                row.model.as_deref(),
            )
            .await?,
            lago_billable_quantity_micros: quantity_to_micros(quantity),
        });
    };
    if funding.settled {
        return Ok(settlement_from_funding(funding));
    }

    let claim_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let stale_before = now - Duration::seconds(FUNDING_CLAIM_LEASE_SECS);
    let claimed = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one_and_update(
            doc! {
                "_id": &row.id,
                "released": false,
                "funding.settled": { "$ne": true },
                "$or": [
                    { "funding.settlement_claim_id": Bson::Null },
                    { "funding.settlement_claim_id": { "$exists": false } },
                    { "funding.settlement_claimed_at": { "$lte": bson::DateTime::from_chrono(stale_before) } },
                ],
            },
            doc! { "$set": {
                "funding.settlement_claim_id": &claim_id,
                "funding.settlement_claimed_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let Some(mut claimed) = claimed else {
        let current = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": &row.id })
            .await?
            .ok_or_else(|| AppError::Internal("usage funding row disappeared".to_string()))?;
        if let Some(funding) = current.funding.as_ref()
            && funding.settled
        {
            return Ok(settlement_from_funding(funding));
        }
        return Err(AppError::Internal(format!(
            "usage funding settlement is busy for row {}",
            row.id
        )));
    };

    let quantity = claimed.quantity.unwrap_or(0).max(0);
    let rate_micros = settlement_rate_micros(db, &claimed).await?;
    let mut allowance_covered = claimed
        .funding
        .as_ref()
        .map(|value| {
            value
                .allowance_consumptions
                .iter()
                .map(|allocation| allocation.quantity.max(0))
                .sum::<i64>()
        })
        .unwrap_or(0)
        .min(quantity);

    let allowance_reservations = claimed
        .funding
        .as_ref()
        .map(|value| value.allowance_reservations.clone())
        .unwrap_or_default();
    for reservation in allowance_reservations {
        let operation_id = format!("{}:allowance:{}", claimed.id, reservation.period_id);
        if has_allowance_operation(&claimed, &operation_id) {
            finish_allowance_operation_if_locked(db, &reservation.period_id, &operation_id).await?;
            continue;
        }
        let consume = reservation
            .quantity
            .max(0)
            .min(quantity.saturating_sub(allowance_covered));
        if settle_allowance_resource(
            db,
            &claimed,
            &reservation.allowance_id,
            &reservation.period_id,
            &operation_id,
            reservation.quantity.max(0),
            consume,
            true,
        )
        .await?
        {
            allowance_covered = allowance_covered.saturating_add(consume).min(quantity);
            refresh_claimed_funding(db, &mut claimed).await?;
        }
    }

    if claimed.layer == BillingLayer::Platform && allowance_covered < quantity {
        let definitions = super::allowances::applicable_allowances(
            db,
            &claimed.billing_owner_id,
            claimed.service_id.as_deref(),
            claimed.service_slug.as_deref(),
            claimed.metric,
        )
        .await?;
        let now = Utc::now();
        let mut periods = Vec::new();
        for definition in definitions {
            let period = super::allowances::ensure_current_period(
                db,
                &definition,
                &claimed.billing_owner_id,
                now,
            )
            .await?;
            periods.push((definition.id, period));
        }
        periods.sort_by(|(_, left), (_, right)| expiry_order(left.period_end, right.period_end));
        for (allowance_id, period) in periods {
            if allowance_covered >= quantity {
                break;
            }
            let operation_id = format!("{}:allowance-extra:{}", claimed.id, period.id);
            if has_allowance_operation(&claimed, &operation_id) {
                finish_allowance_operation_if_locked(db, &period.id, &operation_id).await?;
                continue;
            }
            let consume = super::allowances::available_period_quantity(&period)
                .min(quantity.saturating_sub(allowance_covered));
            if consume <= 0 {
                continue;
            }
            if settle_allowance_resource(
                db,
                &claimed,
                &allowance_id,
                &period.id,
                &operation_id,
                0,
                consume,
                false,
            )
            .await?
            {
                allowance_covered = allowance_covered.saturating_add(consume).min(quantity);
                refresh_claimed_funding(db, &mut claimed).await?;
            }
        }
    }

    let chargeable_quantity = quantity.saturating_sub(allowance_covered);
    let total_charge_micros = saturating_cost_micros(rate_micros, chargeable_quantity)
        .saturating_add(claimed.base_fee_micros.unwrap_or(0).max(0));
    let mut grant_covered_micros = claimed
        .funding
        .as_ref()
        .map(|value| {
            value
                .grant_consumptions
                .iter()
                .map(|allocation| allocation.amount_micros.max(0))
                .chain(
                    value
                        .base_fee_grant_consumptions
                        .iter()
                        .map(|allocation| allocation.amount_micros.max(0)),
                )
                .sum::<i64>()
        })
        .unwrap_or(0)
        .min(total_charge_micros);
    let base_fee_grant_reservations = claimed
        .funding
        .as_ref()
        .map(|value| value.base_fee_grant_reservations.clone())
        .unwrap_or_default();
    for reservation in base_fee_grant_reservations {
        let operation_id = format!("{}:base-fee-grant:{}", claimed.id, reservation.grant_id);
        if has_base_fee_grant_operation(&claimed, &operation_id) {
            finish_grant_operation_if_locked(db, &reservation.grant_id, &operation_id).await?;
            continue;
        }
        let consume = reservation
            .amount_micros
            .max(0)
            .min(total_charge_micros.saturating_sub(grant_covered_micros));
        if settle_grant_resource(
            db,
            &claimed,
            &reservation.grant_id,
            &operation_id,
            reservation.amount_micros.max(0),
            consume,
            true,
        )
        .await?
        {
            grant_covered_micros = grant_covered_micros
                .saturating_add(consume)
                .min(total_charge_micros);
            refresh_claimed_funding(db, &mut claimed).await?;
        }
    }

    let grant_reservations = claimed
        .funding
        .as_ref()
        .map(|value| value.grant_reservations.clone())
        .unwrap_or_default();
    for reservation in grant_reservations {
        let operation_id = format!("{}:grant:{}", claimed.id, reservation.grant_id);
        if has_grant_operation(&claimed, &operation_id) {
            finish_grant_operation_if_locked(db, &reservation.grant_id, &operation_id).await?;
            continue;
        }
        let consume = reservation
            .amount_micros
            .max(0)
            .min(total_charge_micros.saturating_sub(grant_covered_micros));
        if settle_grant_resource(
            db,
            &claimed,
            &reservation.grant_id,
            &operation_id,
            reservation.amount_micros.max(0),
            consume,
            true,
        )
        .await?
        {
            grant_covered_micros = grant_covered_micros
                .saturating_add(consume)
                .min(total_charge_micros);
            refresh_claimed_funding(db, &mut claimed).await?;
        }
    }

    if grant_covered_micros < total_charge_micros {
        let now = Utc::now();
        let mut grants =
            super::grants::list_active_for_user(db, &claimed.billing_owner_id, now).await?;
        grants.retain(|grant| {
            super::grants::service_scope_applies(
                &grant.scope,
                claimed.service_id.as_deref(),
                claimed.service_slug.as_deref(),
            )
        });
        grants.sort_by(|left, right| {
            expiry_order(left.expires_at, right.expires_at)
                .then_with(|| left.created_at.cmp(&right.created_at))
        });
        for grant in grants {
            if grant_covered_micros >= total_charge_micros {
                break;
            }
            let operation_id = format!("{}:grant-extra:{}", claimed.id, grant.id);
            if has_grant_operation(&claimed, &operation_id) {
                finish_grant_operation_if_locked(db, &grant.id, &operation_id).await?;
                continue;
            }
            let consume = super::grants::available_grant_micros(&grant)
                .min(total_charge_micros.saturating_sub(grant_covered_micros));
            if consume <= 0 {
                continue;
            }
            if settle_grant_resource(db, &claimed, &grant.id, &operation_id, 0, consume, false)
                .await?
            {
                grant_covered_micros = grant_covered_micros
                    .saturating_add(consume)
                    .min(total_charge_micros);
                refresh_claimed_funding(db, &mut claimed).await?;
            }
        }
    }

    let wallet_micros = total_charge_micros.saturating_sub(grant_covered_micros);
    let total_wallet_charge_credits = whole_credits_for_micros(wallet_micros);
    let wallet_charge_credits =
        total_wallet_charge_credits.saturating_sub(claimed.base_fee_applied_credits.max(0));
    // Grant- and allowance-funded usage is deliberately absent from Lago's
    // charging stream. Only the wallet-funded fraction is emitted, so Lago's
    // invoice, NyxID's pending debit, wallet refresh, and drift comparison all
    // describe the same chargeable usage.
    let lago_billable_quantity_micros = billable_quantity_micros(wallet_micros, rate_micros);
    let settled_at = Utc::now();
    let result = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! {
                "_id": &claimed.id,
                "funding.settlement_claim_id": &claim_id,
                "funding.settled": { "$ne": true },
            },
            doc! {
                "$set": {
                    "funding.settled": true,
                    "funding.wallet_charge_credits": wallet_charge_credits,
                    "funding.lago_billable_quantity_micros": lago_billable_quantity_micros,
                    "funding.settled_at": bson::DateTime::from_chrono(settled_at),
                    "updated_at": bson::DateTime::from_chrono(settled_at),
                },
                "$unset": {
                    "funding.settlement_claim_id": "",
                    "funding.settlement_claimed_at": "",
                },
            },
        )
        .await?;
    if result.modified_count != 1 {
        return Err(AppError::Internal(format!(
            "usage funding settlement lost its claim for row {}",
            claimed.id
        )));
    }
    Ok(FundingSettlement {
        wallet_charge_credits,
        lago_billable_quantity_micros,
    })
}

fn settlement_from_funding(funding: &UsageFunding) -> FundingSettlement {
    FundingSettlement {
        wallet_charge_credits: funding.wallet_charge_credits.unwrap_or(0).max(0),
        lago_billable_quantity_micros: funding.lago_billable_quantity_micros.unwrap_or(0).max(0),
    }
}

async fn settlement_rate_micros(db: &mongodb::Database, row: &UsageMeterRow) -> AppResult<i64> {
    if let Some(rate) =
        super::reservation::find_rate(db, &row.lago_metric_code, row.model.as_deref()).await?
    {
        return Ok(rate.credits_per_unit_micros.max(0));
    }
    if row.model.is_some()
        && let Some(rate) = super::reservation::find_rate(db, &row.lago_metric_code, None).await?
    {
        return Ok(rate.credits_per_unit_micros.max(0));
    }
    Ok(row
        .funding
        .as_ref()
        .map(|value| value.credits_per_unit_micros)
        .unwrap_or(0)
        .max(0))
}

async fn refresh_claimed_funding(db: &mongodb::Database, row: &mut UsageMeterRow) -> AppResult<()> {
    *row = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one(doc! { "_id": &row.id })
        .await?
        .ok_or_else(|| AppError::Internal("usage funding row disappeared".to_string()))?;
    Ok(())
}

fn has_allowance_operation(row: &UsageMeterRow, operation_id: &str) -> bool {
    row.funding.as_ref().is_some_and(|funding| {
        funding
            .allowance_consumptions
            .iter()
            .any(|allocation| allocation.operation_id == operation_id)
    })
}

fn has_grant_operation(row: &UsageMeterRow, operation_id: &str) -> bool {
    row.funding.as_ref().is_some_and(|funding| {
        funding
            .grant_consumptions
            .iter()
            .any(|allocation| allocation.operation_id == operation_id)
    })
}

fn has_base_fee_grant_operation(row: &UsageMeterRow, operation_id: &str) -> bool {
    row.funding.as_ref().is_some_and(|funding| {
        funding
            .base_fee_grant_consumptions
            .iter()
            .any(|allocation| allocation.operation_id == operation_id)
    })
}

async fn finish_allowance_operation_if_locked(
    db: &mongodb::Database,
    period_id: &str,
    operation_id: &str,
) -> AppResult<()> {
    let Some(period) = db
        .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .find_one(doc! {
            "_id": period_id,
            "active_settlement.operation_id": operation_id,
        })
        .await?
    else {
        return Ok(());
    };
    if let Some(lock) = period.active_settlement {
        complete_allowance_lock(db, period_id, &period.allowance_id, &lock).await?;
    }
    Ok(())
}

async fn finish_grant_operation_if_locked(
    db: &mongodb::Database,
    grant_id: &str,
    operation_id: &str,
) -> AppResult<()> {
    let Some(grant) = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! {
            "_id": grant_id,
            "active_settlement.operation_id": operation_id,
        })
        .await?
    else {
        return Ok(());
    };
    if let Some(lock) = grant.active_settlement {
        complete_grant_lock(db, grant_id, &lock).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn settle_allowance_resource(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    allowance_id: &str,
    period_id: &str,
    operation_id: &str,
    reserved_quantity: i64,
    consume_quantity: i64,
    honor_reservation: bool,
) -> AppResult<bool> {
    let lock = AllowanceSettlementLock {
        operation_id: operation_id.to_string(),
        usage_row_id: row.id.clone(),
        reserved_quantity,
        consume_quantity,
        applied: false,
        updated_at: Utc::now(),
    };
    for _ in 0..RESOURCE_LOCK_RETRIES {
        let mut filter = doc! {
            "_id": period_id,
            "$or": [
                { "active_settlement": Bson::Null },
                { "active_settlement": { "$exists": false } },
            ],
            "$expr": {
                "$and": [
                    { "$gte": ["$reserved_quantity", reserved_quantity] },
                    { "$gte": [
                        { "$add": [
                            { "$subtract": [
                                { "$subtract": ["$total_quantity", "$consumed_quantity"] },
                                "$reserved_quantity",
                            ] },
                            reserved_quantity,
                        ] },
                        consume_quantity,
                    ] },
                ]
            },
        };
        if !honor_reservation {
            filter.insert(
                "$and",
                vec![doc! { "$or": [
                    { "period_end": Bson::Null },
                    { "period_end": { "$exists": false } },
                    { "period_end": { "$gt": bson::DateTime::from_chrono(Utc::now()) } },
                ] }],
            );
        }
        let lock_bson = bson::to_bson(&lock).map_err(|error| {
            AppError::Internal(format!("failed to encode allowance lock: {error}"))
        })?;
        let result = db
            .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .update_one(
                filter,
                doc! { "$set": {
                    "active_settlement": lock_bson,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                } },
            )
            .await?;
        if result.modified_count == 1 {
            complete_allowance_lock(db, period_id, allowance_id, &lock).await?;
            return Ok(true);
        }
        let Some(period) = db
            .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .find_one(doc! { "_id": period_id })
            .await?
        else {
            return Ok(false);
        };
        let Some(active) = period.active_settlement else {
            return Ok(false);
        };
        complete_allowance_lock(db, period_id, &period.allowance_id, &active).await?;
        if active.operation_id == operation_id {
            return Ok(true);
        }
    }
    Err(AppError::Internal(format!(
        "usage allowance settlement lock busy for period {period_id}"
    )))
}

async fn complete_allowance_lock(
    db: &mongodb::Database,
    period_id: &str,
    allowance_id: &str,
    lock: &AllowanceSettlementLock,
) -> AppResult<()> {
    if !lock.applied {
        db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .update_one(
                doc! {
                    "_id": period_id,
                    "active_settlement.operation_id": &lock.operation_id,
                    "active_settlement.applied": false,
                },
                doc! {
                    "$inc": {
                        "reserved_quantity": -lock.reserved_quantity,
                        "consumed_quantity": lock.consume_quantity,
                    },
                    "$set": {
                        "active_settlement.applied": true,
                        "active_settlement.updated_at": bson::DateTime::from_chrono(Utc::now()),
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    },
                },
            )
            .await?;
    }
    let allocation = AllowanceConsumptionAllocation {
        operation_id: lock.operation_id.clone(),
        allowance_id: allowance_id.to_string(),
        period_id: period_id.to_string(),
        quantity: lock.consume_quantity,
    };
    let allocation_bson = bson::to_bson(&allocation).map_err(|error| {
        AppError::Internal(format!("failed to encode allowance consumption: {error}"))
    })?;
    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! {
                "_id": &lock.usage_row_id,
                "funding.allowance_consumptions.operation_id": { "$ne": &lock.operation_id },
            },
            doc! { "$push": { "funding.allowance_consumptions": allocation_bson } },
        )
        .await?;
    db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .update_one(
            doc! {
                "_id": period_id,
                "active_settlement.operation_id": &lock.operation_id,
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

#[allow(clippy::too_many_arguments)]
async fn settle_grant_resource(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    grant_id: &str,
    operation_id: &str,
    reserved_micros: i64,
    consume_micros: i64,
    honor_reservation: bool,
) -> AppResult<bool> {
    let lock = CreditGrantSettlementLock {
        operation_id: operation_id.to_string(),
        usage_row_id: row.id.clone(),
        reserved_micros,
        consume_micros,
        applied: false,
        updated_at: Utc::now(),
    };
    for _ in 0..RESOURCE_LOCK_RETRIES {
        let mut filter = doc! {
            "_id": grant_id,
            "status": "active",
            "$or": [
                { "active_settlement": Bson::Null },
                { "active_settlement": { "$exists": false } },
            ],
            "$expr": {
                "$and": [
                    { "$gte": ["$reserved_micros", reserved_micros] },
                    { "$gte": ["$remaining_micros", consume_micros] },
                    { "$gte": [
                        { "$add": [
                            { "$subtract": ["$remaining_micros", "$reserved_micros"] },
                            reserved_micros,
                        ] },
                        consume_micros,
                    ] },
                ]
            },
        };
        if !honor_reservation {
            filter.insert(
                "$and",
                vec![doc! { "$or": [
                    { "expires_at": Bson::Null },
                    { "expires_at": { "$exists": false } },
                    { "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) } },
                ] }],
            );
        }
        let lock_bson = bson::to_bson(&lock)
            .map_err(|error| AppError::Internal(format!("failed to encode grant lock: {error}")))?;
        let result = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .update_one(
                filter,
                doc! { "$set": {
                    "active_settlement": lock_bson,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                } },
            )
            .await?;
        if result.modified_count == 1 {
            complete_grant_lock(db, grant_id, &lock).await?;
            return Ok(true);
        }
        let Some(grant) = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": grant_id })
            .await?
        else {
            return Ok(false);
        };
        let Some(active) = grant.active_settlement else {
            return Ok(false);
        };
        complete_grant_lock(db, grant_id, &active).await?;
        if active.operation_id == operation_id {
            return Ok(true);
        }
    }
    Err(AppError::Internal(format!(
        "credit grant settlement lock busy for grant {grant_id}"
    )))
}

async fn complete_grant_lock(
    db: &mongodb::Database,
    grant_id: &str,
    lock: &CreditGrantSettlementLock,
) -> AppResult<()> {
    if !lock.applied {
        let now = Utc::now();
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .update_one(
                doc! {
                    "_id": grant_id,
                    "active_settlement.operation_id": &lock.operation_id,
                    "active_settlement.applied": false,
                },
                vec![doc! { "$set": {
                    "reserved_micros": { "$subtract": ["$reserved_micros", lock.reserved_micros] },
                    "remaining_micros": { "$subtract": ["$remaining_micros", lock.consume_micros] },
                    "status": { "$cond": [
                        { "$lte": [
                            { "$subtract": ["$remaining_micros", lock.consume_micros] },
                            0_i64,
                        ] },
                        "consumed",
                        "$status",
                    ] },
                    "consumed_at": { "$cond": [
                        { "$lte": [
                            { "$subtract": ["$remaining_micros", lock.consume_micros] },
                            0_i64,
                        ] },
                        bson::DateTime::from_chrono(now),
                        "$consumed_at",
                    ] },
                    "active_settlement.applied": true,
                    "active_settlement.updated_at": bson::DateTime::from_chrono(now),
                    "updated_at": bson::DateTime::from_chrono(now),
                } }],
            )
            .await?;
    }
    let allocation = GrantConsumptionAllocation {
        operation_id: lock.operation_id.clone(),
        grant_id: grant_id.to_string(),
        amount_micros: lock.consume_micros,
    };
    let allocation_bson = bson::to_bson(&allocation).map_err(|error| {
        AppError::Internal(format!("failed to encode grant consumption: {error}"))
    })?;
    let allocation_field = if lock.operation_id.contains(":base-fee-grant:")
        || lock.operation_id.contains(":base-fee-grant-release:")
    {
        "funding.base_fee_grant_consumptions"
    } else {
        "funding.grant_consumptions"
    };
    let mut allocation_filter = doc! { "_id": &lock.usage_row_id };
    allocation_filter.insert(
        format!("{allocation_field}.operation_id"),
        doc! { "$ne": &lock.operation_id },
    );
    let mut push = Document::new();
    push.insert(allocation_field, allocation_bson);
    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(allocation_filter, doc! { "$push": push })
        .await?;
    if lock.consume_micros > 0 {
        if let (Some(grant), Some(usage_row)) = (
            db.collection::<CreditGrant>(CREDIT_GRANTS)
                .find_one(doc! { "_id": grant_id })
                .await?,
            db.collection::<UsageMeterRow>(USAGE_METER)
                .find_one(doc! { "_id": &lock.usage_row_id })
                .await?,
        ) {
            let ledgered = super::ledger::record_grant_event(
                db,
                BillingLedgerEventType::GrantConsumed,
                &grant.recipient_user_id,
                grant_id,
                lock.consume_micros,
                Some(&usage_row),
                format!("grant-consumed:{}:{}", grant_id, lock.operation_id),
            )
            .await;
            if !ledgered {
                return Err(AppError::Internal(format!(
                    "grant consumption ledger append is pending for operation {}",
                    lock.operation_id
                )));
            }
        } else {
            return Err(AppError::Internal(format!(
                "grant consumption ledger context is missing for operation {}",
                lock.operation_id
            )));
        }
    }
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .update_one(
            doc! {
                "_id": grant_id,
                "active_settlement.operation_id": &lock.operation_id,
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

fn quantity_to_micros(quantity: i64) -> i64 {
    (i128::from(quantity.max(0)) * i128::from(CREDIT_MICROS)).min(i128::from(i64::MAX)) as i64
}

fn billable_quantity_micros(wallet_micros: i64, rate_micros: i64) -> i64 {
    if wallet_micros <= 0 || rate_micros <= 0 {
        return 0;
    }
    let numerator = i128::from(wallet_micros) * i128::from(CREDIT_MICROS);
    let units = (numerator + i128::from(rate_micros - 1)) / i128::from(rate_micros);
    units.min(i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use crate::models::billing_ledger::{BillingLedgerEntry, COLLECTION_NAME as BILLING_LEDGER};
    use crate::models::billing_rate_cache::BillingRateCache;
    use crate::models::billing_target::{BillingServiceScope, BillingTargetKind};
    use crate::models::credit_grant::{
        COLLECTION_NAME as CREDIT_GRANTS, CreditGrantSettlementLock, CreditGrantStatus,
    };
    use crate::models::service_billing::BillingMetric;
    use crate::models::usage_allowance::{
        AllowanceRecurrence, COLLECTION_NAME as USAGE_ALLOWANCES, UsageAllowance,
    };
    use crate::models::usage_meter::{CredentialClass, UsageStatus};
    use crate::services::billing::{BillingIngress, NodeIntent};
    use crate::test_utils::connect_test_database;

    use super::*;

    #[test]
    fn expiring_resources_sort_before_non_expiring_resources() {
        let early = Utc.with_ymd_and_hms(2026, 8, 22, 0, 0, 0).unwrap();
        let late = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        assert_eq!(expiry_order(Some(early), Some(late)), Ordering::Less);
        assert_eq!(expiry_order(Some(early), None), Ordering::Less);
        assert_eq!(expiry_order(None, Some(late)), Ordering::Greater);
    }

    #[test]
    fn cost_multiplication_saturates() {
        assert_eq!(saturating_cost_micros(125_000, 8), CREDIT_MICROS);
        assert_eq!(saturating_cost_micros(i64::MAX, 2), i64::MAX);
    }

    #[tokio::test]
    async fn allowances_cover_quantity_while_grants_then_wallet_cover_base_and_remainder() {
        let Some(db) = connect_test_database("billing_base_funding_precedence").await else {
            return;
        };
        let now = Utc::now();
        let owner_id = "base-funding-owner";
        let service_id = "base-funding-service";
        db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
            .insert_one(UsageAllowance {
                id: "base-funding-allowance".to_string(),
                service_id: service_id.to_string(),
                service_slug: "api-twilio".to_string(),
                metric: BillingMetric::Seconds,
                quantity: 300,
                recurrence: AllowanceRecurrence::Daily,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: Vec::new(),
                is_active: true,
                created_by: "admin".to_string(),
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert allowance");
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .insert_one(CreditGrant {
                id: "base-funding-grant".to_string(),
                batch_id: "base-funding-batch".to_string(),
                schedule_origin: None,
                recipient_user_id: owner_id.to_string(),
                target_kind: BillingTargetKind::SelectedUsers,
                amount_credits: 2,
                amount_micros: 2_000_000,
                remaining_micros: 2_000_000,
                reserved_micros: 0,
                scope: BillingServiceScope {
                    all_services: true,
                    service_ids: Vec::new(),
                    service_slugs: Vec::new(),
                },
                expires_at: Some(now + Duration::days(1)),
                reason: None,
                granted_by: "admin".to_string(),
                status: CreditGrantStatus::Active,
                issued_ledgered_at: Some(now),
                terminal_ledgered_at: None,
                terminal_amount_micros: 0,
                active_settlement: None,
                created_at: now,
                updated_at: now,
                consumed_at: None,
                expired_at: None,
                revoked_at: None,
            })
            .await
            .expect("insert grant");
        let operation_billing = crate::models::platform_operation::OperationBilling {
            metric: BillingMetric::Seconds,
            price_per_unit: "0.01".to_string(),
            secondary: None,
            base_fee_per_call: Some("1.5".to_string()),
            lago_metric_code: "platform_op_api_twilio_constrained_call_and_say".to_string(),
            sync_status: crate::models::service_billing::PricingSyncStatus::Synced,
            sync_error: None,
        };
        let estimated_usage =
            crate::models::service_billing::PlatformUsage::single_request(0).with_seconds(600);
        let ctx = BillingRouteContext::new(
            BillingIngress::PlatformOperation,
            "base-funding-request".to_string(),
            owner_id.to_string(),
            owner_id.to_string(),
            None,
            None,
            Some(service_id.to_string()),
            Some("api-twilio".to_string()),
            NodeIntent::Direct,
            "basic".to_string(),
            CredentialClass::NyxidManagedMaster,
            BillingMetric::Seconds,
            None,
            false,
        )
        .with_platform_operation_billing(
            &operation_billing,
            10_000,
            None,
            1_500_000,
            &estimated_usage,
        );
        let mut layer = LayerReservation {
            layer: BillingLayer::Platform,
            flush_seq: None,
            metric: BillingMetric::Seconds,
            estimated_quantity: 600,
            credits_per_unit_micros: 10_000,
            base_fee_micros: 1_500_000,
            reserved_credits: 8,
            allowance_reservations: Vec::new(),
            grant_reservations: Vec::new(),
            base_fee_grant_reservations: Vec::new(),
        };

        reserve_layer(&db, &ctx, &mut layer)
            .await
            .expect("reserve funding layers");

        assert_eq!(
            layer
                .allowance_reservations
                .iter()
                .map(|allocation| allocation.quantity)
                .sum::<i64>(),
            300
        );
        assert_eq!(
            layer
                .base_fee_grant_reservations
                .iter()
                .map(|allocation| allocation.amount_micros)
                .sum::<i64>(),
            1_500_000
        );
        assert_eq!(
            layer
                .grant_reservations
                .iter()
                .map(|allocation| allocation.amount_micros)
                .sum::<i64>(),
            500_000
        );
        assert_eq!(layer.reserved_credits, 3);
    }

    #[tokio::test]
    async fn split_token_allowances_fund_only_the_matching_component() {
        let Some(db) = connect_test_database("billing_split_token_allowances").await else {
            return;
        };
        let now = Utc::now();
        let owner_id = "split-token-owner";
        let service_id = "split-token-service";
        for (id, metric, quantity) in [
            ("input-allowance", BillingMetric::InputTokens, 10),
            ("output-allowance", BillingMetric::OutputTokens, 20),
            ("combined-allowance", BillingMetric::Tokens, 100),
        ] {
            db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
                .insert_one(UsageAllowance {
                    id: id.to_string(),
                    service_id: service_id.to_string(),
                    service_slug: "llm-test".to_string(),
                    metric,
                    quantity,
                    recurrence: AllowanceRecurrence::Daily,
                    target_kind: BillingTargetKind::AllUsers,
                    target_user_ids: Vec::new(),
                    is_active: true,
                    created_by: "admin".to_string(),
                    created_at: now,
                    updated_at: now,
                })
                .await
                .expect("insert split token allowance");
        }
        let billing = crate::models::platform_operation::OperationBilling {
            metric: BillingMetric::InputTokens,
            price_per_unit: "1".to_string(),
            secondary: Some(
                crate::models::platform_operation::OperationBillingComponent {
                    metric: BillingMetric::OutputTokens,
                    price_per_unit: "1".to_string(),
                    lago_metric_code: "split-output".to_string(),
                },
            ),
            base_fee_per_call: None,
            lago_metric_code: "split-input".to_string(),
            sync_status: crate::models::service_billing::PricingSyncStatus::Synced,
            sync_error: None,
        };
        let usage = crate::models::service_billing::PlatformUsage::llm_completion(10, 18)
            .with_token_breakdown(Some(crate::models::service_billing::TokenBreakdown {
                prompt_tokens: 7,
                completion_tokens: 11,
                ..Default::default()
            }));
        let ctx = BillingRouteContext::new(
            BillingIngress::PlatformOperation,
            "split-token-request".to_string(),
            owner_id.to_string(),
            owner_id.to_string(),
            None,
            None,
            Some(service_id.to_string()),
            Some("llm-test".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::NyxidManagedMaster,
            BillingMetric::Requests,
            None,
            false,
        )
        .with_platform_operation_billing(&billing, 1_000_000, Some(1_000_000), 0, &usage);
        let mut layers = vec![
            LayerReservation {
                layer: BillingLayer::Platform,
                flush_seq: None,
                metric: BillingMetric::InputTokens,
                estimated_quantity: 7,
                credits_per_unit_micros: 1_000_000,
                base_fee_micros: 0,
                reserved_credits: 7,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
                base_fee_grant_reservations: Vec::new(),
            },
            LayerReservation {
                layer: BillingLayer::Platform,
                flush_seq: Some(1),
                metric: BillingMetric::OutputTokens,
                estimated_quantity: 11,
                credits_per_unit_micros: 1_000_000,
                base_fee_micros: 0,
                reserved_credits: 11,
                allowance_reservations: Vec::new(),
                grant_reservations: Vec::new(),
                base_fee_grant_reservations: Vec::new(),
            },
        ];

        reserve_estimated_funding(&db, &ctx, &mut layers)
            .await
            .expect("reserve split token funding");
        assert_eq!(layers[0].reserved_credits, 0);
        assert_eq!(layers[0].allowance_reservations.len(), 1);
        assert_eq!(
            layers[0].allowance_reservations[0].allowance_id,
            "input-allowance"
        );
        assert_eq!(layers[0].allowance_reservations[0].quantity, 7);
        assert_eq!(layers[1].reserved_credits, 0);
        assert_eq!(layers[1].allowance_reservations.len(), 1);
        assert_eq!(
            layers[1].allowance_reservations[0].allowance_id,
            "output-allowance"
        );
        assert_eq!(layers[1].allowance_reservations[0].quantity, 11);
        assert_eq!(
            db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
                .count_documents(doc! { "allowance_id": "combined-allowance" })
                .await
                .expect("count combined allowance periods"),
            0
        );
    }

    #[tokio::test]
    async fn deferred_base_applies_once_then_final_settlement_releases_same_hold() {
        let Some(db) = connect_test_database("billing_deferred_base_once").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let now = Utc::now();
        let row_id = "deferred-base-row";
        db.collection::<mongodb::bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": "wallet-deferred-base",
                "owner_id": "owner-deferred-base",
                "reserved_credits": 8_i64,
                "pending_lago_debits": 0_i64,
                "updated_at": bson::DateTime::from_chrono(now),
            })
            .await
            .expect("insert wallet");
        let descriptor = DeferredQuantity::TwilioCall {
            account_sid: "AC11111111111111111111111111111111".to_string(),
            call_sid: "CA22222222222222222222222222222222".to_string(),
        };
        let row = UsageMeterRow {
            id: row_id.to_string(),
            transaction_id: "deferred-base-tx".to_string(),
            billing_request_id: "deferred-base-request".to_string(),
            layer: BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: "owner-deferred-base".to_string(),
            wallet_id: Some("wallet-deferred-base".to_string()),
            actor_user_id: "owner-deferred-base".to_string(),
            api_key_id: None,
            service_id: Some("catalog-twilio".to_string()),
            service_slug: Some("api-twilio".to_string()),
            metric: BillingMetric::Seconds,
            lago_metric_code: "platform_op_api_twilio_constrained_call_and_say".to_string(),
            credential_class: CredentialClass::NyxidManagedMaster,
            model: None,
            token_breakdown: None,
            reserved_credits: 8,
            funding: Some(UsageFunding {
                credits_per_unit_micros: 10_000,
                ..Default::default()
            }),
            quantity: None,
            base_fee_micros: Some(1_500_000),
            base_fee_applied: false,
            base_fee_applied_credits: 0,
            deferred_quantity: Some(descriptor.clone()),
            deferred_attempts: 0,
            deferred_next_retry_at: Some(now),
            pending_resale_quantity: None,
            pending_platform_secondary_quantity: None,
            status: UsageStatus::Forwarded,
            forwarded: true,
            released: false,
            lago_acked: false,
            attempt: 0,
            settlement_attempts: 0,
            settlement_next_retry_at: None,
            created_at: now,
            updated_at: now,
            finalized_at: None,
            expires_at: None,
            last_error: None,
        };
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .expect("insert usage row");

        assert!(
            apply_deferred_base_fee(&db, &row, &descriptor)
                .await
                .expect("apply base fee")
        );
        let after_base = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find row after base")
            .expect("row exists");
        assert!(
            !apply_deferred_base_fee(&db, &after_base, &descriptor)
                .await
                .expect("replay base fee")
        );
        assert!(after_base.base_fee_applied);
        assert_eq!(after_base.base_fee_applied_credits, 2);
        assert_eq!(after_base.reserved_credits, 6);

        let finalized = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one_and_update(
                doc! { "_id": row_id, "status": "forwarded" },
                doc! {
                    "$set": {
                        "status": "finalized",
                        "quantity": 37_i64,
                        "finalized_at": bson::DateTime::from_chrono(now),
                    },
                    "$unset": { "deferred_quantity": "", "deferred_next_retry_at": "" },
                },
            )
            .return_document(mongodb::options::ReturnDocument::After)
            .await
            .expect("finalize row")
            .expect("finalized row");
        super::super::reservation::claim_released_and_settle(&db, &finalized)
            .await
            .expect("settle final quantity");

        let wallet = db
            .collection::<mongodb::bson::Document>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "_id": "wallet-deferred-base" })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        assert_eq!(wallet.get_i64("reserved_credits").expect("reserved"), 0);
        assert_eq!(wallet.get_i64("pending_lago_debits").expect("pending"), 2);
        let saved = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find settled row")
            .expect("settled row exists");
        assert!(saved.released);
        let funding = saved.funding.expect("funding");
        assert!(funding.settled);
        assert_eq!(funding.wallet_charge_credits, Some(0));
        assert_eq!(funding.lago_billable_quantity_micros, Some(187_000_000));

        let ledger = db.collection::<BillingLedgerEntry>(BILLING_LEDGER);
        let entry = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(entry) = ledger
                    .find_one(doc! { "reference_id": row_id, "event_type": "usage_settled" })
                    .await
                    .expect("find usage ledger")
                {
                    break entry;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("usage ledger append timed out");
        assert_eq!(entry.amount_credits, Some(2));
        assert_eq!(entry.base_fee_micros, Some(1_500_000));
        assert_eq!(
            ledger
                .count_documents(doc! { "reference_id": row_id, "event_type": "usage_settled" })
                .await
                .expect("count usage ledger"),
            1
        );
    }

    #[tokio::test]
    async fn actual_usage_consumes_allowance_then_grant_then_wallet() {
        let Some(db) = connect_test_database("billing_funding_precedence").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let now = Utc::now();
        let owner_id = "funding-owner";
        let service_id = "funding-service";
        let service_slug = "funding-service";
        let allowance_id = "allowance-1";
        let window =
            super::super::allowances::allowance_window(AllowanceRecurrence::Daily, now, now);
        let period_id = super::super::allowances::period_id(allowance_id, owner_id, window.start);
        let grant_id = "grant-1";
        let row_id = "funding-row-1";

        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id("platform_funding", None),
                lago_metric_code: "platform_funding".to_string(),
                model: None,
                credits_per_unit_micros: 500_000,
                synced_at: now,
            })
            .await
            .expect("insert rate");
        db.collection::<UsageAllowance>(USAGE_ALLOWANCES)
            .insert_one(UsageAllowance {
                id: allowance_id.to_string(),
                service_id: service_id.to_string(),
                service_slug: service_slug.to_string(),
                metric: BillingMetric::Requests,
                quantity: 3,
                recurrence: AllowanceRecurrence::Daily,
                target_kind: BillingTargetKind::AllUsers,
                target_user_ids: Vec::new(),
                is_active: true,
                created_by: "admin-1".to_string(),
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert allowance");
        db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .insert_one(UsageAllowancePeriod {
                id: period_id.clone(),
                allowance_id: allowance_id.to_string(),
                owner_user_id: owner_id.to_string(),
                total_quantity: 3,
                consumed_quantity: 0,
                reserved_quantity: 1,
                period_start: window.start,
                period_end: window.end,
                active_settlement: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert allowance period");
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .insert_one(CreditGrant {
                id: grant_id.to_string(),
                batch_id: "batch-1".to_string(),
                schedule_origin: None,
                recipient_user_id: owner_id.to_string(),
                target_kind: BillingTargetKind::SelectedUsers,
                amount_credits: 2,
                amount_micros: 2_000_000,
                remaining_micros: 2_000_000,
                reserved_micros: 500_000,
                scope: BillingServiceScope {
                    all_services: true,
                    service_ids: Vec::new(),
                    service_slugs: Vec::new(),
                },
                expires_at: Some(now + Duration::days(1)),
                reason: None,
                granted_by: "admin-1".to_string(),
                status: CreditGrantStatus::Active,
                issued_ledgered_at: Some(now),
                terminal_ledgered_at: None,
                terminal_amount_micros: 0,
                active_settlement: None,
                created_at: now,
                updated_at: now,
                consumed_at: None,
                expired_at: None,
                revoked_at: None,
            })
            .await
            .expect("insert grant");

        let row = UsageMeterRow {
            id: row_id.to_string(),
            transaction_id: "funding-tx-1".to_string(),
            billing_request_id: "funding-request-1".to_string(),
            layer: BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: owner_id.to_string(),
            wallet_id: Some("wallet-1".to_string()),
            actor_user_id: owner_id.to_string(),
            api_key_id: None,
            service_id: Some(service_id.to_string()),
            service_slug: Some(service_slug.to_string()),
            metric: BillingMetric::Requests,
            lago_metric_code: "platform_funding".to_string(),
            credential_class: CredentialClass::UserOwned,
            model: None,
            token_breakdown: None,
            reserved_credits: 0,
            funding: Some(UsageFunding {
                credits_per_unit_micros: 500_000,
                allowance_reservations: vec![AllowanceReservationAllocation {
                    allowance_id: allowance_id.to_string(),
                    period_id: period_id.clone(),
                    quantity: 1,
                }],
                grant_reservations: vec![GrantReservationAllocation {
                    grant_id: grant_id.to_string(),
                    amount_micros: 500_000,
                }],
                ..Default::default()
            }),
            quantity: Some(10),
            base_fee_micros: None,
            base_fee_applied: false,
            base_fee_applied_credits: 0,
            deferred_quantity: None,
            deferred_attempts: 0,
            deferred_next_retry_at: None,
            pending_resale_quantity: None,
            pending_platform_secondary_quantity: None,
            status: UsageStatus::Finalized,
            forwarded: true,
            released: false,
            lago_acked: false,
            attempt: 0,
            settlement_attempts: 0,
            settlement_next_retry_at: None,
            created_at: now,
            updated_at: now,
            finalized_at: Some(now),
            expires_at: None,
            last_error: None,
        };
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .expect("insert usage row");

        let settlement = settle_usage_funding(&db, &row)
            .await
            .expect("settle funding");
        assert_eq!(settlement.wallet_charge_credits, 2);
        assert_eq!(settlement.lago_billable_quantity_micros, 3_000_000);

        let period = db
            .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .find_one(doc! { "_id": &period_id })
            .await
            .expect("find period")
            .expect("period exists");
        assert_eq!(period.consumed_quantity, 3);
        assert_eq!(period.reserved_quantity, 0);
        let grant = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": grant_id })
            .await
            .expect("find grant")
            .expect("grant exists");
        assert_eq!(grant.remaining_micros, 0);
        assert_eq!(grant.reserved_micros, 0);
        assert_eq!(grant.status, CreditGrantStatus::Consumed);
        let saved = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": row_id })
            .await
            .expect("find usage row")
            .expect("usage row exists");
        let funding = saved.funding.expect("funding persisted");
        assert!(funding.settled);
        assert_eq!(funding.allowance_consumptions.len(), 2);
        assert_eq!(funding.grant_consumptions.len(), 2);

        // Recreate the crash point after the grant and usage allocation were
        // durable but before the ledger append/lock clear. Recovery must
        // restore the deduplicated journal entry and release the grant lock.
        let allocation = funding
            .grant_consumptions
            .last()
            .expect("grant allocation")
            .clone();
        let dedupe_key = format!(
            "grant-consumed:{}:{}",
            allocation.grant_id, allocation.operation_id
        );
        db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
            .delete_one(doc! { "dedupe_key": &dedupe_key })
            .await
            .expect("delete tail ledger entry to simulate crash");
        let lock = CreditGrantSettlementLock {
            operation_id: allocation.operation_id.clone(),
            usage_row_id: row_id.to_string(),
            reserved_micros: 0,
            consume_micros: allocation.amount_micros,
            applied: true,
            updated_at: now,
        };
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .update_one(
                doc! { "_id": grant_id },
                doc! { "$set": {
                    "active_settlement": bson::to_bson(&lock).expect("encode lock"),
                } },
            )
            .await
            .expect("restore grant settlement lock");

        let recovered = recover_terminal_releases(&db)
            .await
            .expect("recover grant consumption ledger");
        assert_eq!(recovered, 1);

        let recovered_grant = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": grant_id })
            .await
            .expect("find recovered grant")
            .expect("recovered grant exists");
        assert!(recovered_grant.active_settlement.is_none());
        assert_eq!(
            db.collection::<BillingLedgerEntry>(BILLING_LEDGER)
                .count_documents(doc! { "dedupe_key": dedupe_key })
                .await
                .expect("count recovered ledger entry"),
            1
        );
    }
}
