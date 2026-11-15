//! The owner's charging choice lives here, separate from death/ownership proof.
use crate::models::{credits::Credits, usage_meter::UsageMeterRow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UnknownConsumptionPolicy {
    Waive,
    ReservedEstimate,
}

// One-place policy choice. Final evidence always bypasses this function.
pub(super) fn unknown_consumption_policy() -> UnknownConsumptionPolicy {
    const CHARGE_RESERVED_ESTIMATE: bool = false;
    if CHARGE_RESERVED_ESTIMATE {
        UnknownConsumptionPolicy::ReservedEstimate
    } else {
        UnknownConsumptionPolicy::Waive
    }
}

impl UnknownConsumptionPolicy {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Waive => "waive_unknown_consumption",
            Self::ReservedEstimate => "charge_reserved_estimate",
        }
    }

    pub(super) fn quantity(self, row: &UsageMeterRow) -> Option<i64> {
        if row.quantity.is_some()
            || row.pending_platform_usage.is_some()
            || row.pending_resale_quantity.is_some()
        {
            return None;
        }
        match self {
            Self::Waive => Some(0),
            Self::ReservedEstimate => reserved_quantity(row),
        }
    }
}

// Old rows did not persist their estimated quantity. Recover it only when the
// saved unit rate and exact reservation uniquely determine the same integer.
// Never round a credit amount into a quantity or consult a possibly newer rate.
fn reserved_quantity(row: &UsageMeterRow) -> Option<i64> {
    let funding = row.funding.as_ref()?;
    if let Some(quantity) = funding.reserved_quantity {
        return (quantity >= 0).then_some(quantity);
    }
    let allowance = funding
        .allowance_reservations
        .iter()
        .try_fold(0_i64, |sum, a| {
            (a.quantity >= 0)
                .then(|| sum.checked_add(a.quantity))
                .flatten()
        })?;
    let grant = funding
        .grant_reservations
        .iter()
        .try_fold(Credits::ZERO, |sum, g| {
            (g.amount >= Credits::ZERO)
                .then(|| sum.checked_add(g.amount).ok())
                .flatten()
        })?;
    if row.reserved_credits < Credits::ZERO {
        return None;
    }
    let rate = Credits::from_pico(super::super::amounts::rate_pico(
        funding.credits_per_unit_pico,
        funding.credits_per_unit_micros,
    ))
    .ok()?;
    if rate <= Credits::ZERO {
        return None;
    }
    let tax = funding
        .applied_tax
        .as_ref()
        .map(|t| t.rate_percent)
        .unwrap_or(Credits::ZERO);
    if tax < Credits::ZERO {
        return None;
    }
    let target = grant.checked_add(row.reserved_credits).ok()?;
    let held = |quantity: i64| -> Option<Credits> {
        let cost = rate.checked_mul(quantity).ok()?;
        let granted = cost.min(grant);
        let wallet =
            super::super::taxes::gross_ceiling(cost.checked_sub(granted).ok()?, tax).ok()?;
        granted.checked_add(wallet).ok()
    };
    // Overflow is above the finite target, never a small/negative amount.
    let (mut low, mut high) = (0_i64, i64::MAX - allowance);
    while low < high {
        let mid = low + (high - low) / 2;
        if held(mid).is_some_and(|amount| amount < target) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    (held(low) == Some(target))
        .then(|| allowance.checked_add(low))
        .flatten()
}
