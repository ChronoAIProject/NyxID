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
    COLLECTION_NAME as USAGE_METER, GrantConsumptionAllocation, GrantReservationAllocation,
    UsageFunding, UsageMeterRow,
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

pub(crate) async fn reserve_estimated_funding_in_session(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
    layers: &mut [LayerReservation],
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    for layer in layers {
        reserve_layer(db, ctx, layer, session).await?;
    }
    Ok(())
}

async fn reserve_layer(
    db: &mongodb::Database,
    ctx: &BillingRouteContext,
    layer: &mut LayerReservation,
    session: &mut mongodb::ClientSession,
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
            ctx.platform_metric,
            uncovered_quantity,
            session,
        )
        .await?;
        let reserved_units: i64 = layer
            .allowance_reservations
            .iter()
            .map(|allocation| allocation.quantity)
            .sum();
        uncovered_quantity = uncovered_quantity.saturating_sub(reserved_units);
    }

    let estimated_micros =
        saturating_cost_micros(layer.credits_per_unit_micros, uncovered_quantity);
    layer.grant_reservations = reserve_grants(
        db,
        &ctx.billing_owner_id,
        ctx.catalog_service_id
            .as_deref()
            .or(ctx.user_service_id.as_deref()),
        ctx.service_slug.as_deref(),
        estimated_micros,
        session,
    )
    .await?;
    let grant_micros: i64 = layer
        .grant_reservations
        .iter()
        .map(|allocation| allocation.amount_micros)
        .sum();
    layer.reserved_credits =
        whole_credits_for_micros(estimated_micros.saturating_sub(grant_micros));
    Ok(())
}

async fn reserve_allowances(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: Option<&str>,
    service_slug: Option<&str>,
    metric: crate::models::service_billing::BillingMetric,
    quantity: i64,
    session: &mut mongodb::ClientSession,
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
        let period = super::allowances::ensure_current_period_in_session(
            db,
            &definition,
            owner_user_id,
            now,
            session,
        )
        .await?;
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
        if period.active_settlement.is_some() {
            return Err(AppError::Conflict(format!(
                "billing allowance settlement pending:{}",
                period.id
            )));
        }
        let updated = db
            .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .update_one(
                doc! {
                    "_id": &period.id,
                    "active_settlement": Bson::Null,
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
            .session(&mut *session)
            .await?;
        if updated.modified_count == 0 {
            return Err(AppError::Conflict(
                "billing funding changed during reservation; retry".into(),
            ));
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
    session: &mut mongodb::ClientSession,
) -> AppResult<Vec<GrantReservationAllocation>> {
    let now = Utc::now();
    let mut cursor = db.collection::<CreditGrant>(CREDIT_GRANTS).find(doc! {
        "recipient_user_id": owner_user_id, "status": "active", "revoke_requested_at": Bson::Null,
        "issued_ledgered_at": { "$type": "date" }, "remaining_micros": { "$gt": 0_i64 },
        "$or": [{ "expires_at": Bson::Null }, { "expires_at": { "$gt": bson::DateTime::from_chrono(now) } }],
    }).session(&mut *session).await?;
    let mut grants: Vec<CreditGrant> = cursor.stream(&mut *session).try_collect().await?;
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
        let rule = super::service_limits::matching_rule(&grant, service_id, service_slug);
        let wanted = needed.min(super::service_limits::available_for_service(
            &grant,
            service_id,
            service_slug,
        ));
        if wanted <= 0 {
            continue;
        }
        if grant.active_settlement.is_some() {
            return Err(AppError::Conflict(format!(
                "billing grant settlement pending:{}",
                grant.id
            )));
        }
        let mut filter = spendable_grant_filter(&grant.id, wanted, now);
        let mut inc = doc! { "reserved_micros": wanted };
        if let Some(rule) = rule {
            let index = grant
                .service_limits
                .iter()
                .position(|r| r.key == rule.key)
                .expect("matched rule");
            add_rule_filter(&mut filter, index, 0, wanted);
            inc.insert(format!("service_limits.{index}.reserved_micros"), wanted);
        }
        let updated = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .update_one(
                filter,
                doc! {
                    "$inc": inc,
                    "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
                },
            )
            .session(&mut *session)
            .await?;
        if updated.modified_count == 0 {
            return Err(AppError::Conflict(
                "billing funding changed during reservation; retry".into(),
            ));
        }
        allocations.push(GrantReservationAllocation {
            service_limit_key: rule.map(|r| r.key.clone()),
            grant_id: grant.id,
            amount_micros: wanted,
        });
        needed -= wanted;
    }
    Ok(allocations)
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
                reservation.service_limit_key.as_deref(),
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
pub(super) async fn help_grant_settlement_lock(
    db: &mongodb::Database,
    grant_id: &str,
) -> AppResult<()> {
    if let Some(grant) = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": grant_id })
        .await?
    {
        if let Some(lock) = grant.active_settlement {
            complete_grant_lock(db, grant_id, &lock).await?;
        }
    }
    Ok(())
}
pub(super) async fn help_allowance_settlement_lock(
    db: &mongodb::Database,
    period_id: &str,
) -> AppResult<()> {
    if let Some(period) = db
        .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .find_one(doc! { "_id": period_id })
        .await?
    {
        if let Some(lock) = period.active_settlement {
            complete_allowance_lock(db, period_id, &period.allowance_id, &lock).await?;
        }
    }
    Ok(())
}

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

// A durable resource lock belongs to its usage row even after the claim lease
// changes or the resource is exhausted. Finish these before recomputing funding.
async fn finish_usage_resource_locks(db: &mongodb::Database, usage_id: &str) -> AppResult<()> {
    let mut periods = db
        .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .find(doc! { "active_settlement.usage_row_id": usage_id })
        .await?;
    while let Some(period) = periods.try_next().await? {
        if let Some(lock) = period.active_settlement {
            complete_allowance_lock(db, &period.id, &period.allowance_id, &lock).await?;
        }
    }
    let mut grants = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find(doc! { "active_settlement.usage_row_id": usage_id })
        .await?;
    while let Some(grant) = grants.try_next().await? {
        if let Some(lock) = grant.active_settlement {
            complete_grant_lock(db, &grant.id, &lock).await?;
        }
    }
    Ok(())
}

/// Conditional WRITE is essential: a snapshot read alone allows takeover and
/// wallet settlement to commit concurrently on a different resource. The nonce
/// guarantees a write conflict with takeover/finalization in either order.
async fn fence_resource_operation(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    usage_id: &str,
    expected_claim: Option<&str>,
    release: bool,
) -> mongodb::error::Result<()> {
    let mut filter =
        doc! { "_id": usage_id, "funding.settled": { "$ne": true }, "released": release };
    if release {
        filter.insert("status", doc! { "$in": ["failed", "abandoned"] });
    } else {
        let claim = expected_claim
            .ok_or_else(|| mongodb::error::Error::custom("Usage settlement claim missing"))?;
        filter.insert("funding.settlement_claim_id", claim);
    }
    let result = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            filter,
            doc! { "$set": { "funding.resource_operation_fence": Uuid::new_v4().to_string() } },
        )
        .session(session)
        .await?;
    if result.modified_count != 1 {
        return Err(mongodb::error::Error::custom(
            "Usage settlement ownership changed",
        ));
    }
    Ok(())
}
fn allowance_consumed(row: &UsageMeterRow) -> i64 {
    row.funding.as_ref().map_or(0, |f| {
        f.allowance_consumptions
            .iter()
            .fold(0_i64, |sum, a| sum.saturating_add(a.quantity.max(0)))
    })
}
fn grant_consumed(row: &UsageMeterRow) -> i64 {
    row.funding.as_ref().map_or(0, |f| {
        f.grant_consumptions
            .iter()
            .fold(0_i64, |sum, a| sum.saturating_add(a.amount_micros.max(0)))
    })
}

fn add_rule_filter(filter: &mut Document, index: usize, reserved: i64, consume: i64) {
    // Dot indexes in aggregation field paths do not index arrays; explicitly
    // take the frozen element before comparing exact integer counters.
    let rule = doc! { "$arrayElemAt": ["$service_limits", index as i64] };
    let expression = doc! { "$let": { "vars": { "rule": rule }, "in": { "$and": [
        { "$gte": ["$$rule.reserved_micros", reserved] },
        { "$gte": [{ "$add": [{ "$subtract": ["$$rule.limit_micros", { "$add": ["$$rule.consumed_micros", "$$rule.reserved_micros"] }] }, reserved] }, consume] },
    ] } } };
    filter
        .entry("$and".to_string())
        .or_insert_with(|| Bson::Array(Vec::new()));
    filter
        .get_array_mut("$and")
        .expect("and conditions")
        .push(doc! { "$expr": expression }.into());
}

fn spendable_grant_filter(grant_id: &str, amount_micros: i64, now: DateTime<Utc>) -> Document {
    doc! {
        "_id": grant_id,
        "status": "active",
        "active_settlement": Bson::Null,
        "revoke_requested_at": Bson::Null,
        "issued_ledgered_at": { "$type": "date" },
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

    finish_usage_resource_locks(db, &claimed.id).await?;
    refresh_claimed_funding(db, &mut claimed, &claim_id).await?;
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
            refresh_claimed_funding(db, &mut claimed, &claim_id).await?;
            allowance_covered = allowance_consumed(&claimed).min(quantity);
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
                refresh_claimed_funding(db, &mut claimed, &claim_id).await?;
                allowance_covered = allowance_consumed(&claimed).min(quantity);
            }
        }
    }

    let chargeable_quantity = quantity.saturating_sub(allowance_covered);
    let total_charge_micros = saturating_cost_micros(rate_micros, chargeable_quantity);
    let mut grant_covered_micros = claimed
        .funding
        .as_ref()
        .map(|value| {
            value
                .grant_consumptions
                .iter()
                .map(|allocation| allocation.amount_micros.max(0))
                .sum::<i64>()
        })
        .unwrap_or(0)
        .min(total_charge_micros);
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
            reservation.service_limit_key.as_deref(),
            true,
        )
        .await?
        {
            refresh_claimed_funding(db, &mut claimed, &claim_id).await?;
            grant_covered_micros = grant_consumed(&claimed).min(total_charge_micros);
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
            let rule = super::service_limits::matching_rule(
                &grant,
                claimed.service_id.as_deref(),
                claimed.service_slug.as_deref(),
            );
            let consume = super::service_limits::available_for_service(
                &grant,
                claimed.service_id.as_deref(),
                claimed.service_slug.as_deref(),
            )
            .min(total_charge_micros.saturating_sub(grant_covered_micros));
            if consume <= 0 {
                continue;
            }
            if settle_grant_resource(
                db,
                &claimed,
                &grant.id,
                &operation_id,
                0,
                consume,
                rule.map(|r| r.key.as_str()),
                false,
            )
            .await?
            {
                refresh_claimed_funding(db, &mut claimed, &claim_id).await?;
                grant_covered_micros = grant_consumed(&claimed).min(total_charge_micros);
            }
        }
    }

    let wallet_micros = total_charge_micros.saturating_sub(grant_covered_micros);
    let wallet_charge_credits = whole_credits_for_micros(wallet_micros);
    // Grant- and allowance-funded usage is deliberately absent from Lago's
    // charging stream. Only the wallet-funded fraction is emitted, so Lago's
    // invoice, NyxID's pending debit, wallet refresh, and drift comparison all
    // describe the same chargeable usage.
    let lago_billable_quantity_micros =
        billable_quantity_micros(wallet_micros, rate_micros, chargeable_quantity);
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

async fn refresh_claimed_funding(
    db: &mongodb::Database,
    row: &mut UsageMeterRow,
    expected_claim: &str,
) -> AppResult<()> {
    let current = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one(doc! { "_id": &row.id })
        .await?
        .ok_or_else(|| AppError::Internal("usage funding row disappeared".to_string()))?;
    if current.released
        || !current
            .funding
            .as_ref()
            .is_some_and(|f| !f.settled && f.settlement_claim_id.as_deref() == Some(expected_claim))
    {
        return Err(AppError::Conflict(
            "Usage settlement ownership changed".into(),
        ));
    }
    *row = current;
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
    let release = operation_id.contains(":allowance-release:");
    let expected_claim = row
        .funding
        .as_ref()
        .and_then(|f| f.settlement_claim_id.clone());
    let proposed_lock = AllowanceSettlementLock {
        operation_id: operation_id.to_string(),
        usage_row_id: row.id.clone(),
        reserved_quantity,
        consume_quantity,
        applied: false,
        updated_at: Utc::now(),
    };
    for _ in 0..RESOURCE_LOCK_RETRIES {
        let Some(period) = db
            .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .find_one(doc! { "_id": period_id })
            .await?
        else {
            return Ok(false);
        };
        if let Some(active) = period.active_settlement.as_ref() {
            complete_allowance_lock(db, period_id, &period.allowance_id, active).await?;
            if active.operation_id == operation_id {
                return Ok(true);
            }
            continue;
        }
        let mut lock = proposed_lock.clone();
        if !honor_reservation {
            if period.period_end.is_some_and(|end| end <= Utc::now()) {
                return Ok(false);
            }
            lock.consume_quantity = lock
                .consume_quantity
                .min(super::allowances::available_period_quantity(&period));
            if lock.consume_quantity <= 0 {
                return Ok(false);
            }
        }
        let consume_quantity = lock.consume_quantity;
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
        let tx_db = db.clone();
        let usage_id = row.id.clone();
        let op_id = operation_id.to_owned();
        let expected_claim = expected_claim.clone();
        let lock_update = doc! { "$set": { "active_settlement": lock_bson,
        "updated_at": bson::DateTime::from_chrono(Utc::now()) } };
        let mut session = db.client().start_session().await?;
        let (already_recorded, acquired) = session
            .start_transaction()
            .and_run2(async move |session| {
                let current = tx_db
                    .collection::<UsageMeterRow>(USAGE_METER)
                    .find_one(doc! { "_id": &usage_id })
                    .session(&mut *session)
                    .await?;
                if current
                    .as_ref()
                    .is_some_and(|r| has_allowance_operation(r, &op_id))
                {
                    return Ok((true, false));
                }
                // No usage row means no durable owner for this operation.
                if current.is_none() {
                    return Err(mongodb::error::Error::custom(
                        "Usage funding owner disappeared",
                    ));
                }
                fence_resource_operation(
                    &tx_db,
                    session,
                    &usage_id,
                    expected_claim.as_deref(),
                    release,
                )
                .await?;
                let result = tx_db
                    .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
                    .update_one(filter.clone(), lock_update.clone())
                    .session(&mut *session)
                    .await?;
                Ok((false, result.modified_count == 1))
            })
            .await?;
        if already_recorded {
            finish_allowance_operation_if_locked(db, period_id, operation_id).await?;
            return Ok(true);
        }
        if acquired {
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
            continue;
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
    service_limit_key: Option<&str>,
    honor_reservation: bool,
) -> AppResult<bool> {
    let release = operation_id.contains(":grant-release:");
    let expected_claim = row
        .funding
        .as_ref()
        .and_then(|f| f.settlement_claim_id.clone());
    let proposed_lock = CreditGrantSettlementLock {
        operation_id: operation_id.to_string(),
        usage_row_id: row.id.clone(),
        reserved_micros,
        consume_micros,
        service_limit_key: service_limit_key.map(str::to_owned),
        applied: false,
        updated_at: Utc::now(),
    };
    for _ in 0..RESOURCE_LOCK_RETRIES {
        let Some(grant) = db
            .collection::<CreditGrant>(CREDIT_GRANTS)
            .find_one(doc! { "_id": grant_id })
            .await?
        else {
            return Ok(false);
        };
        if let Some(active) = grant.active_settlement.as_ref() {
            complete_grant_lock(db, grant_id, active).await?;
            if active.operation_id == operation_id {
                return Ok(true);
            }
            continue;
        }
        if grant.status != crate::models::credit_grant::CreditGrantStatus::Active {
            return Ok(false);
        }
        let mut lock = proposed_lock.clone();
        if !honor_reservation {
            if grant.revoke_requested_at.is_some()
                || grant.issued_ledgered_at.is_none()
                || grant.expires_at.is_some_and(|end| end <= Utc::now())
            {
                return Ok(false);
            }
            lock.consume_micros =
                lock.consume_micros
                    .min(super::service_limits::available_for_service(
                        &grant,
                        row.service_id.as_deref(),
                        row.service_slug.as_deref(),
                    ));
            if lock.consume_micros <= 0 {
                return Ok(false);
            }
        }
        let consume_micros = lock.consume_micros;
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
        if !honor_reservation {
            filter.insert("revoke_requested_at", Bson::Null);
            filter.insert("issued_ledgered_at", doc! { "$type": "date" });
        }
        if let Some(key) = service_limit_key {
            let index = grant
                .service_limits
                .iter()
                .position(|r| r.key == key)
                .ok_or_else(|| AppError::Internal("Reserved service limit is missing".into()))?;
            add_rule_filter(&mut filter, index, reserved_micros, consume_micros);
        } else if super::service_limits::matching_rule(
            &grant,
            row.service_id.as_deref(),
            row.service_slug.as_deref(),
        )
        .is_some()
        {
            return Err(AppError::Internal(
                "Service limit identity missing from settlement".into(),
            ));
        }
        let lock_bson = bson::to_bson(&lock)
            .map_err(|error| AppError::Internal(format!("failed to encode grant lock: {error}")))?;
        let tx_db = db.clone();
        let usage_id = row.id.clone();
        let op_id = operation_id.to_owned();
        let expected_claim = expected_claim.clone();
        let lock_update = doc! { "$set": { "active_settlement": lock_bson,
        "updated_at": bson::DateTime::from_chrono(Utc::now()) } };
        let mut session = db.client().start_session().await?;
        let (already_recorded, acquired) = session
            .start_transaction()
            .and_run2(async move |session| {
                let current = tx_db
                    .collection::<UsageMeterRow>(USAGE_METER)
                    .find_one(doc! { "_id": &usage_id })
                    .session(&mut *session)
                    .await?;
                if current
                    .as_ref()
                    .is_some_and(|r| has_grant_operation(r, &op_id))
                {
                    return Ok((true, false));
                }
                // No usage row means no durable owner for this operation.
                if current.is_none() {
                    return Err(mongodb::error::Error::custom(
                        "Usage funding owner disappeared",
                    ));
                }
                fence_resource_operation(
                    &tx_db,
                    session,
                    &usage_id,
                    expected_claim.as_deref(),
                    release,
                )
                .await?;
                let result = tx_db
                    .collection::<CreditGrant>(CREDIT_GRANTS)
                    .update_one(filter.clone(), lock_update.clone())
                    .session(&mut *session)
                    .await?;
                Ok((false, result.modified_count == 1))
            })
            .await?;
        if already_recorded {
            finish_grant_operation_if_locked(db, grant_id, operation_id).await?;
            return Ok(true);
        }
        if acquired {
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
            continue;
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
                    "service_limits": { "$map": { "input": { "$ifNull": ["$service_limits", []] }, "as": "rule", "in": {
                        "$cond": [{ "$eq": ["$$rule.key", lock.service_limit_key.as_deref()] },
                            { "$mergeObjects": ["$$rule", {
                                "reserved_micros": { "$subtract": ["$$rule.reserved_micros", lock.reserved_micros] },
                                "consumed_micros": { "$add": ["$$rule.consumed_micros", lock.consume_micros] },
                            }] }, "$$rule"]
                    } } },
                    "status": { "$cond": [
                        { "$and": [{ "$eq": [{ "$ifNull": ["$revoke_requested_at", Bson::Null] }, Bson::Null] }, { "$lte": [
                            { "$subtract": ["$remaining_micros", lock.consume_micros] },
                            0_i64,
                        ] }] },
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
        service_limit_key: lock.service_limit_key.clone(),
    };
    let allocation_bson = bson::to_bson(&allocation).map_err(|error| {
        AppError::Internal(format!("failed to encode grant consumption: {error}"))
    })?;
    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! {
                "_id": &lock.usage_row_id,
                "funding.grant_consumptions.operation_id": { "$ne": &lock.operation_id },
            },
            doc! { "$push": { "funding.grant_consumptions": allocation_bson } },
        )
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
    super::grants::finalize_reclaim(db, grant_id).await?;
    Ok(())
}

fn quantity_to_micros(quantity: i64) -> i64 {
    (i128::from(quantity.max(0)) * i128::from(CREDIT_MICROS)).min(i128::from(i64::MAX)) as i64
}

fn billable_quantity_micros(wallet_micros: i64, rate_micros: i64, chargeable_quantity: i64) -> i64 {
    if wallet_micros <= 0 || rate_micros <= 0 || chargeable_quantity <= 0 {
        return 0;
    }
    let numerator = i128::from(wallet_micros) * i128::from(CREDIT_MICROS);
    let units = (numerator + i128::from(rate_micros - 1)) / i128::from(rate_micros);
    units
        .min(i128::from(quantity_to_micros(chargeable_quantity)))
        .min(i128::from(i64::MAX)) as i64
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
                service_limits: Vec::new(),
                revoke_requested_at: None,
                revoke_requested_by: None,

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
                    service_limit_key: None,

                    grant_id: grant_id.to_string(),
                    amount_micros: 500_000,
                }],
                ..Default::default()
            }),
            quantity: Some(10),
            pending_resale_quantity: None,
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
            service_limit_key: None,

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

#[cfg(test)]
mod expanded_race_tests {
    use super::super::expanded_tests::{add_allowance, allocation, database, row, saved, setup};
    use super::super::reservation::tests::route_context;
    use super::*;
    use crate::models::usage_meter::UsageFunding;

    async fn claimed_row(db: &mongodb::Database) -> UsageMeterRow {
        let ctx = route_context("owner");
        super::super::meter::open(db, &ctx, None).await.unwrap();
        let current = row(db, &ctx.billing_request_id).await;
        db.collection::<UsageMeterRow>(USAGE_METER).update_one(doc! { "_id": &current.id }, doc! { "$set": {
            "quantity": 1_i64, "status": "finalized", "funding": bson::to_bson(&UsageFunding {
                credits_per_unit_micros: 1_000_000,
                settlement_claim_id: Some("old-worker".into()),
                settlement_claimed_at: Some(Utc::now() - Duration::seconds(120)),
                ..Default::default()
            }).unwrap()
        } }).await.unwrap();
        row(db, &ctx.billing_request_id).await
    }

    #[tokio::test]
    async fn taken_over_worker_cannot_acquire_new_grant_or_allowance_locks() {
        let db = database("billing_stale_consumption_claim_fence").await;
        setup(&db, "owner", 1).await;
        let stale = claimed_row(&db).await;
        let settled = settle_usage_funding(&db, &stale).await.unwrap();
        assert_eq!(settled.wallet_charge_credits, 1);
        // These sources appear only after the successor finalized wallet funding.
        let grant = allocation("owner", 100_000_000, Some(100_000_000));
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .insert_one(&grant)
            .await
            .unwrap();
        let allowance = add_allowance(&db, "owner").await;
        let period =
            super::super::allowances::ensure_current_period(&db, &allowance, "owner", Utc::now())
                .await
                .unwrap();
        assert!(
            settle_grant_resource(
                &db,
                &stale,
                &grant.id,
                "stale-grant-extra",
                0,
                1_000_000,
                Some("catalog-cap"),
                false
            )
            .await
            .is_err()
        );
        assert!(
            settle_allowance_resource(
                &db,
                &stale,
                &allowance.id,
                &period.id,
                "stale-allowance-extra",
                0,
                1,
                false
            )
            .await
            .is_err()
        );
        assert_eq!(saved(&db, &grant.id).await.remaining_micros, 100_000_000);
        assert_eq!(
            db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
                .find_one(doc! { "_id": &period.id })
                .await
                .unwrap()
                .unwrap()
                .consumed_quantity,
            0
        );
        let current = row(&db, &stale.billing_request_id).await;
        assert_eq!(current.funding.unwrap().wallet_charge_credits, Some(1));
        assert_eq!(
            db.collection::<crate::models::billing_ledger::BillingLedgerEntry>(
                crate::models::billing_ledger::COLLECTION_NAME
            )
            .count_documents(doc! { "event_type": "grant_consumed" })
            .await
            .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn takeover_recovers_applied_unrecorded_exhausted_resources_before_wallet() {
        for allowance_funded in [false, true] {
            let db = database(if allowance_funded {
                "billing_takeover_exhausted_allowance"
            } else {
                "billing_takeover_exhausted_grant"
            })
            .await;
            setup(&db, "owner", 1).await;
            let stale = claimed_row(&db).await;
            if allowance_funded {
                let allowance = add_allowance(&db, "owner").await;
                let period = super::super::allowances::ensure_current_period(
                    &db,
                    &allowance,
                    "owner",
                    Utc::now(),
                )
                .await
                .unwrap();
                let lock = AllowanceSettlementLock {
                    operation_id: format!("{}:allowance-extra:{}", stale.id, period.id),
                    usage_row_id: stale.id.clone(),
                    reserved_quantity: 0,
                    consume_quantity: 1,
                    applied: true,
                    updated_at: Utc::now(),
                };
                db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS).update_one(doc! { "_id": &period.id }, doc! { "$set": { "consumed_quantity": 1_i64, "active_settlement": bson::to_bson(&lock).unwrap() } }).await.unwrap();
            } else {
                let mut grant = allocation("owner", 1_000_000, Some(1_000_000));
                grant.remaining_micros = 0;
                grant.status = crate::models::credit_grant::CreditGrantStatus::Consumed;
                grant.service_limits[0].consumed_micros = 1_000_000;
                grant.active_settlement = Some(CreditGrantSettlementLock {
                    operation_id: format!("{}:grant-extra:{}", stale.id, grant.id),
                    usage_row_id: stale.id.clone(),
                    reserved_micros: 0,
                    consume_micros: 1_000_000,
                    service_limit_key: Some("catalog-cap".into()),
                    applied: true,
                    updated_at: Utc::now(),
                });
                db.collection::<CreditGrant>(CREDIT_GRANTS)
                    .insert_one(&grant)
                    .await
                    .unwrap();
            }
            assert_eq!(
                settle_usage_funding(&db, &stale)
                    .await
                    .unwrap()
                    .wallet_charge_credits,
                0
            );
            let current = row(&db, &stale.billing_request_id).await;
            let f = current.funding.as_ref().unwrap();
            assert_eq!(
                if allowance_funded {
                    f.allowance_consumptions.len()
                } else {
                    f.grant_consumptions.len()
                },
                1
            );
            assert_eq!(
                settle_usage_funding(&db, &current)
                    .await
                    .unwrap()
                    .wallet_charge_credits,
                0
            );
        }
    }

    #[tokio::test]
    async fn resource_fence_write_conflicts_with_concurrent_claim_takeover() {
        let db = database("billing_claim_fence_write_conflict").await;
        setup(&db, "owner", 1).await;
        let stale = claimed_row(&db).await;
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        // Establish the old snapshot before the successor replaces the claim.
        db.collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": &stale.id })
            .session(&mut session)
            .await
            .unwrap();
        settle_usage_funding(&db, &stale).await.unwrap();
        assert!(
            fence_resource_operation(&db, &mut session, &stale.id, Some("old-worker"), false)
                .await
                .is_err()
        );
        let _ = session.abort_transaction().await;
    }

    #[tokio::test]
    async fn capped_consumption_ledger_failure_retains_lock_until_reclaim_can_finish() {
        let db = database("billing_capped_reclaim_ledger_retry").await;
        setup(&db, "owner", 1).await;
        let usage = claimed_row(&db).await;
        let mut grant = allocation("owner", 100_000_000, Some(50_000_000));
        let lock = CreditGrantSettlementLock {
            operation_id: "capped-consume-before-ledger".into(),
            usage_row_id: usage.id.clone(),
            reserved_micros: 40_000_000,
            consume_micros: 30_000_000,
            service_limit_key: Some("catalog-cap".into()),
            applied: false,
            updated_at: Utc::now(),
        };
        grant.reserved_micros = 40_000_000;
        grant.service_limits[0].reserved_micros = 40_000_000;
        grant.active_settlement = Some(lock.clone());
        db.collection::<CreditGrant>(CREDIT_GRANTS)
            .insert_one(&grant)
            .await
            .unwrap();
        super::super::grants::request_revoke_grant(&db, &grant.id, Some("admin"))
            .await
            .unwrap();
        // A validator rejects the consumption journal only. Financial apply and
        // history recording happen, but the bridge must remain durable.
        let ledger = crate::models::billing_ledger::COLLECTION_NAME;
        db.create_collection(ledger).await.unwrap();
        db.run_command(doc! { "collMod": ledger, "validator": { "event_type": { "$ne": "grant_consumed" } }, "validationLevel": "strict" }).await.unwrap();
        assert!(complete_grant_lock(&db, &grant.id, &lock).await.is_err());
        let pending = saved(&db, &grant.id).await;
        assert_eq!(pending.remaining_micros, 70_000_000);
        assert_eq!(pending.service_limits[0].consumed_micros, 30_000_000);
        assert!(pending.active_settlement.unwrap().applied);
        assert!(pending.revoked_at.is_none());
        db.run_command(doc! { "collMod": ledger, "validator": {} })
            .await
            .unwrap();
        help_grant_settlement_lock(&db, &grant.id).await.unwrap();
        help_grant_settlement_lock(&db, &grant.id).await.unwrap();
        let terminal = saved(&db, &grant.id).await;
        assert_eq!(terminal.terminal_amount_micros, 70_000_000);
        assert_eq!(terminal.service_limits[0].consumed_micros, 30_000_000);
        assert_eq!(terminal.service_limits[0].reserved_micros, 0);
        assert!(terminal.active_settlement.is_none());
        assert_eq!(db.collection::<crate::models::billing_ledger::BillingLedgerEntry>(ledger).count_documents(doc! { "dedupe_key": format!("grant-consumed:{}:{}", grant.id, lock.operation_id) }).await.unwrap(), 1);
        assert_eq!(
            db.collection::<crate::models::billing_ledger::BillingLedgerEntry>(ledger)
                .count_documents(doc! { "dedupe_key": format!("grant-revoked:{}", grant.id) })
                .await
                .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn funding_refresh_never_adopts_a_successor_claim() {
        let db = database("billing_immutable_settlement_claim_refresh").await;
        setup(&db, "owner", 1).await;
        let mut stale = claimed_row(&db).await;
        db.collection::<UsageMeterRow>(USAGE_METER)
            .update_one(
                doc! { "_id": &stale.id },
                doc! { "$set": { "funding.settlement_claim_id": "successor" } },
            )
            .await
            .unwrap();
        assert!(
            refresh_claimed_funding(&db, &mut stale, "old-worker")
                .await
                .is_err()
        );
        assert_eq!(
            stale
                .funding
                .as_ref()
                .unwrap()
                .settlement_claim_id
                .as_deref(),
            Some("old-worker")
        );
        db.collection::<UsageMeterRow>(USAGE_METER)
            .update_one(
                doc! { "_id": &stale.id },
                doc! { "$set": { "funding.settlement_claim_id": "old-worker" } },
            )
            .await
            .unwrap();
        refresh_claimed_funding(&db, &mut stale, "old-worker")
            .await
            .unwrap();
        // The same fence applies after a later operation's history refresh.
        db.collection::<UsageMeterRow>(USAGE_METER).update_one(doc! { "_id": &stale.id }, doc! { "$set": { "funding.settlement_claim_id": "successor", "funding.settled": true } }).await.unwrap();
        assert!(
            refresh_claimed_funding(&db, &mut stale, "old-worker")
                .await
                .is_err()
        );
    }
}
