//! Allocation-local ceilings. Counters share the allocation's total; no money is
//! earmarked, and other allocations and the purchased wallet remain independent.
use crate::models::credits::Credits;
use std::collections::BTreeSet;

use crate::errors::{AppError, AppResult};
use crate::models::billing_target::BillingServiceScope;
use crate::models::credit_grant::{CreditGrant, CreditGrantSettlementLock};
use crate::models::credit_service_limit::{CreditServiceLimit, CreditServiceLimitInput};

pub async fn resolve(
    db: &mongodb::Database,
    scope: &BillingServiceScope,
    amount: Credits,
    inputs: &[CreditServiceLimitInput],
) -> AppResult<Vec<CreditServiceLimit>> {
    if inputs.len() > 20 {
        return Err(AppError::ValidationError(
            "At most 20 service limits are allowed".into(),
        ));
    }
    let mut used = BTreeSet::new();
    let mut rules = Vec::new();
    for input in inputs {
        if input.service_refs.is_empty()
            || input.service_refs.len() > super::grants::MAX_SCOPED_SERVICES
        {
            return Err(AppError::ValidationError(
                "Each service limit needs 1–100 services".into(),
            ));
        }
        let limit = input.amount;
        if limit <= Credits::ZERO || limit > amount {
            return Err(AppError::ValidationError(
                "Service limits must be positive and no greater than the shared total".into(),
            ));
        }
        let resolved = super::grants::resolve_service_scope(db, false, &input.service_refs).await?;
        for id in &resolved.service_ids {
            if !used.insert(id.clone()) {
                return Err(AppError::ValidationError(
                    "A service can belong to only one limit (including ID and slug aliases)".into(),
                ));
            }
            if !scope.all_services && !scope.service_ids.contains(id) {
                return Err(AppError::ValidationError(
                    "Service limits must select eligible services".into(),
                ));
            }
        }
        let label = input
            .label
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        if label.as_ref().is_some_and(|v| v.chars().count() > 120) {
            return Err(AppError::ValidationError(
                "Service limit labels must not exceed 120 characters".into(),
            ));
        }
        // Stable key derived from the canonical group, independent of input aliases/order.
        let mut ids = resolved.service_ids.clone();
        ids.sort();
        let key =
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, ids.join(":").as_bytes()).to_string();
        rules.push(CreditServiceLimit {
            key,
            service_ids: ids,
            service_slugs: resolved.service_slugs,
            label,
            limit,
            consumed: Credits::ZERO,
            reserved: Credits::ZERO,
        });
    }
    rules.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(rules)
}

pub fn matching_rule<'a>(
    grant: &'a CreditGrant,
    service_id: Option<&str>,
    service_slug: Option<&str>,
) -> Option<&'a CreditServiceLimit> {
    grant.service_limits.iter().find(|rule| match service_id {
        Some(id) => rule.service_ids.iter().any(|v| v == id),
        None => service_slug.is_some_and(|slug| rule.service_slugs.iter().any(|v| v == slug)),
    })
}

pub fn rule_available(rule: &CreditServiceLimit) -> AppResult<Credits> {
    Ok(rule
        .limit
        .checked_sub(rule.consumed)?
        .checked_sub(rule.reserved)?
        .max(Credits::ZERO))
}

pub fn unapplied_extra(lock: Option<&CreditGrantSettlementLock>) -> AppResult<Credits> {
    lock.filter(|l| !l.applied).map_or(Ok(Credits::ZERO), |l| {
        Ok(l.consume.checked_sub(l.reserved)?.max(Credits::ZERO))
    })
}
pub fn held(grant: &CreditGrant) -> AppResult<Credits> {
    Ok(grant
        .reserved
        .checked_add(unapplied_extra(grant.active_settlement.as_ref())?)?
        .min(grant.remaining)
        .max(Credits::ZERO))
}
pub fn available_for_service(
    grant: &CreditGrant,
    service_id: Option<&str>,
    service_slug: Option<&str>,
) -> AppResult<Credits> {
    let available = super::grants::available_grant(grant)?;
    if let Some(rule) = matching_rule(grant, service_id, service_slug) {
        let extra = if grant
            .active_settlement
            .as_ref()
            .and_then(|l| l.service_limit_key.as_deref())
            == Some(&rule.key)
        {
            unapplied_extra(grant.active_settlement.as_ref())?
        } else {
            Credits::ZERO
        };
        Ok(available.min(rule_available(rule)?.checked_sub(extra)?.max(Credits::ZERO)))
    } else {
        Ok(available)
    }
}
pub fn realizable(grant: &CreditGrant) -> AppResult<Credits> {
    let available = super::grants::available_grant(grant)?;
    if grant.scope.all_services
        || grant.service_limits.is_empty()
        || grant.scope.service_ids.iter().any(|id| {
            !grant
                .service_limits
                .iter()
                .any(|r| r.service_ids.contains(id))
        })
    {
        return Ok(available);
    }
    let mut capacity = Credits::ZERO;
    for rule in &grant.service_limits {
        capacity = capacity.checked_add(rule_available(rule)?)?;
    }
    Ok(available.min(
        capacity
            .checked_sub(unapplied_extra(grant.active_settlement.as_ref())?)?
            .max(Credits::ZERO),
    ))
}

/// Scope and cap are evaluated from canonical IDs. A slug is a legacy fallback,
/// never an alternative identity when a canonical service is supplied.
pub fn rule_key(
    grant: &CreditGrant,
    service_id: Option<&str>,
    slug: Option<&str>,
) -> Option<String> {
    matching_rule(grant, service_id, slug).map(|r| r.key.clone())
}

/// One compare-and-set reserves both the allocation and matching ceiling.
/// The caller commits this write together with the durable meter/lifecycle row.
pub fn reservation_change(
    grant: &CreditGrant,
    service_id: Option<&str>,
    slug: Option<&str>,
    wanted: Credits,
    now: chrono::DateTime<chrono::Utc>,
) -> AppResult<(
    mongodb::bson::Document,
    mongodb::bson::Document,
    Option<String>,
)> {
    use mongodb::bson::{self, Bson, doc};
    let mut filter = super::funding::spendable_grant_filter(&grant.id, wanted, now);
    filter.insert("active_settlement", Bson::Null);
    filter.insert("revoke_requested_at", Bson::Null);
    let mut increments = doc! { "reserved": wanted };
    let key = rule_key(grant, service_id, slug);
    if let Some(key) = &key {
        let (index, rule) = grant
            .service_limits
            .iter()
            .enumerate()
            .find(|(_, r)| &r.key == key)
            .ok_or_else(|| AppError::Internal("Missing service ceiling".into()))?;
        rule.reserved.checked_add(wanted)?;
        filter.insert(format!("service_limits.{index}.key"), key);
        let cap = doc! { "$gte": [{ "$let": { "vars": { "rule": { "$arrayElemAt": ["$service_limits", index as i64] } }, "in": { "$subtract": ["$$rule.limit", { "$add": ["$$rule.consumed", "$$rule.reserved"] }] } } }, wanted] };
        let clauses = filter
            .get_array_mut("$and")
            .map_err(|_| AppError::Internal("Missing reservation predicate".into()))?;
        clauses.push(doc! { "$expr": cap }.into());
        increments.insert(format!("service_limits.{index}.reserved"), wanted);
    }
    grant.reserved.checked_add(wanted)?;
    Ok((
        filter,
        doc! { "$inc": increments, "$set": { "updated_at": bson::DateTime::from_chrono(now) } },
        key,
    ))
}
