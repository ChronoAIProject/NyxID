//! Allocation-local ceilings. Counters share the allocation's total; no money is
//! earmarked, and other allocations and the purchased wallet remain independent.
use std::collections::BTreeSet;

use crate::errors::{AppError, AppResult};
use crate::models::billing_target::BillingServiceScope;
use crate::models::credit_grant::{CreditGrant, CreditGrantSettlementLock};
use crate::models::credit_service_limit::{CreditServiceLimit, CreditServiceLimitInput};

pub async fn resolve(
    db: &mongodb::Database,
    scope: &BillingServiceScope,
    amount_micros: i64,
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
        let limit = input
            .amount_credits
            .checked_mul(super::grants::CREDIT_MICROS)
            .filter(|v| *v > 0 && *v <= amount_micros)
            .ok_or_else(|| {
                AppError::ValidationError(
                    "Service limits must be positive and no greater than the shared total".into(),
                )
            })?;
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
            limit_micros: limit,
            consumed_micros: 0,
            reserved_micros: 0,
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

pub fn rule_available(rule: &CreditServiceLimit) -> i64 {
    rule.limit_micros
        .saturating_sub(rule.consumed_micros)
        .saturating_sub(rule.reserved_micros)
        .max(0)
}

/// An unapplied extra settlement is already committed, although its counters
/// have not moved yet. Reserved work is already included in reserved_micros.
pub fn unapplied_extra_micros(lock: Option<&CreditGrantSettlementLock>) -> i64 {
    lock.filter(|l| !l.applied).map_or(0, |l| {
        l.consume_micros.saturating_sub(l.reserved_micros).max(0)
    })
}
pub fn held_micros(grant: &CreditGrant) -> i64 {
    grant
        .reserved_micros
        .saturating_add(unapplied_extra_micros(grant.active_settlement.as_ref()))
        .min(grant.remaining_micros)
        .max(0)
}
pub fn rules_with_commitments(grant: &CreditGrant) -> Vec<CreditServiceLimit> {
    let mut rules = grant.service_limits.clone();
    apply_commitment_to_rules(&mut rules, grant.active_settlement.as_ref());
    rules
}
pub fn apply_commitment_to_rules(
    rules: &mut [CreditServiceLimit],
    lock: Option<&CreditGrantSettlementLock>,
) {
    let extra = unapplied_extra_micros(lock);
    if extra > 0 {
        if let Some(rule) = rules
            .iter_mut()
            .find(|r| Some(r.key.as_str()) == lock.and_then(|l| l.service_limit_key.as_deref()))
        {
            rule.reserved_micros = rule.reserved_micros.saturating_add(extra);
        }
    }
}

pub fn available_for_service(
    grant: &CreditGrant,
    service_id: Option<&str>,
    service_slug: Option<&str>,
) -> i64 {
    let available = super::grants::available_grant_micros(grant);
    matching_rule(grant, service_id, service_slug).map_or(available, |rule| {
        available.min(
            rule_available(rule)
                .saturating_sub(
                    grant
                        .active_settlement
                        .as_ref()
                        .filter(|l| l.service_limit_key.as_deref() == Some(rule.key.as_str()))
                        .map_or(0, |l| unapplied_extra_micros(Some(l))),
                )
                .max(0),
        )
    })
}

/// Realizable unheld value, distinct from the accounting remainder reclaimed at
/// termination. An uncapped eligible service can use the entire shared total.
pub fn realizable_micros(grant: &CreditGrant) -> i64 {
    let available = super::grants::available_grant_micros(grant);
    realizable_for_policy(
        &grant.scope,
        &rules_with_commitments(grant),
        i128::from(available),
    ) as i64
}

pub fn realizable_for_policy(
    scope: &BillingServiceScope,
    rules: &[CreditServiceLimit],
    available: i128,
) -> i128 {
    if scope.all_services
        || rules.is_empty()
        || scope
            .service_ids
            .iter()
            .any(|id| !rules.iter().any(|rule| rule.service_ids.contains(id)))
    {
        return available;
    }
    let capacity: i128 = rules.iter().map(|r| i128::from(rule_available(r))).sum();
    available.min(capacity)
}
