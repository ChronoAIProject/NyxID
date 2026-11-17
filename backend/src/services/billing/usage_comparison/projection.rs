//! Bounded rebuilds use event time; subsequent folds use acknowledgement order.
use super::*;
use crate::models::billing_usage_delivery::{BillingUsageDelivery, COLLECTION_NAME as DELIVERIES};

const RECEIPTS_PER_VISIT: i64 = 1_000;
#[cfg(test)]
tokio::task_local! { pub(super) static TEST_PAGE_SIZE: i64; }
fn page_size() -> i64 {
    #[cfg(test)]
    if let Ok(size) = TEST_PAGE_SIZE.try_with(|size| *size) {
        return size;
    }
    RECEIPTS_PER_VISIT
}
pub(super) const MAX_METRICS: usize = 256;

pub(super) async fn fold(
    db: &Database,
    target: &BillingUsageComparison,
    usage: &CurrentUsage,
    coverage: &UsageCoverage,
) -> AppResult<(UsageProjection, bool)> {
    let layout: Vec<_> = usage.quantities_micros.keys().cloned().collect();
    let mut projection = target
        .projection
        .clone()
        .filter(|p| {
            p.start_second == usage.start_second
                && p.end_second == usage.end_second_exclusive
                && p.layout == layout
                && p.generation == coverage.generation
                && p.uncertainty_revision == target.uncertainty_revision
                && p.sequence <= target.acknowledgement_sequence
                && p.rebuild_upper
                    .is_none_or(|upper| upper <= target.acknowledgement_sequence)
        })
        .unwrap_or_else(|| empty(target, usage, coverage));
    let mut remaining = page_size();
    if let Some(upper) = projection.rebuild_upper {
        let mut filter = target_filter(target);
        filter.extend(doc! {"state":"acknowledged", "acknowledgement_sequence":{"$gt":0_i64,"$lte":upper},
            "event_second":{"$gte":projection.start_second,"$lt":projection.end_second},
            "$or":[{"event_second":{"$gt":projection.cursor_second}},
                {"event_second":projection.cursor_second,"acknowledgement_sequence":{"$gt":projection.cursor_sequence}}]});
        let rows: Vec<BillingUsageDelivery> = db
            .collection(DELIVERIES)
            .find(filter)
            .sort(doc! {"event_second":1,"acknowledgement_sequence":1})
            .limit(remaining)
            .await?
            .try_collect()
            .await?;
        let read = rows.len() as i64;
        for receipt in rows {
            add(&mut projection, &receipt, coverage)?;
            projection.cursor_second = receipt.event_second;
            projection.cursor_sequence = receipt
                .acknowledgement_sequence
                .ok_or_else(|| invalid("Receipt has no acknowledgement sequence"))?;
        }
        if read == remaining {
            return Ok((projection, false));
        }
        remaining -= read;
        projection.sequence = upper;
        projection.rebuild_upper = None;
    }
    if projection.sequence < target.acknowledgement_sequence && remaining > 0 {
        let mut filter = target_filter(target);
        filter.insert(
            "acknowledgement_sequence",
            doc! {"$gt":projection.sequence,"$lte":target.acknowledgement_sequence},
        );
        let rows: Vec<BillingUsageDelivery> = db
            .collection(DELIVERIES)
            .find(filter)
            .sort(doc! {"acknowledgement_sequence":1})
            .limit(remaining)
            .await?
            .try_collect()
            .await?;
        if rows.is_empty() {
            return Err(invalid("Acknowledged receipt history is missing"));
        }
        for receipt in rows {
            let sequence = receipt
                .acknowledgement_sequence
                .ok_or_else(|| invalid("Receipt sequence missing"))?;
            if sequence != projection.sequence + 1
                || receipt.state
                    != crate::models::billing_usage_delivery::DeliveryState::Acknowledged
            {
                return Err(invalid("Acknowledged receipt history has a gap"));
            }
            if receipt.event_second >= projection.start_second
                && receipt.event_second < projection.end_second
            {
                add(&mut projection, &receipt, coverage)?;
            }
            projection.sequence = sequence;
        }
    }
    let complete = projection.sequence == target.acknowledgement_sequence;
    Ok((projection, complete))
}

fn add(
    projection: &mut UsageProjection,
    receipt: &BillingUsageDelivery,
    coverage: &UsageCoverage,
) -> AppResult<()> {
    if receipt.duplicate_acknowledgement
        && receipt.usage_created_at.is_none_or(|created| {
            coverage
                .continuous_since
                .is_none_or(|since| created <= since)
        })
    {
        projection.unverified_origin = true;
    }
    let quantity = super::super::lago_client::current_usage::units(&receipt.properties.quantity)
        .map_err(|_| invalid("Receipt quantity cannot be projected"))?;
    if receipt.code.trim().is_empty() || receipt.code.len() > 256 {
        return Err(invalid("Receipt metric cannot be projected"));
    }
    let total = projection.totals.entry(receipt.code.clone()).or_default();
    *total = total
        .checked_add(quantity)
        .ok_or_else(|| invalid("Usage projection exceeds exact quantity range"))?;
    if projection.totals.len() > MAX_METRICS {
        return Err(invalid("Usage projection has too many metrics"));
    }
    Ok(())
}

// This read follows the provider response: a prepared snapshot always precedes
// sending. A resolution racing this read changes the pinned sequence/revision.
pub(super) async fn has_unresolved(
    db: &Database,
    target: &BillingUsageComparison,
    usage: &CurrentUsage,
) -> AppResult<bool> {
    let mut filter = target_filter(target);
    filter.extend(doc! {"state":"prepared", "event_second":{"$gte":usage.start_second,"$lt":usage.end_second_exclusive}});
    Ok(db
        .collection::<Document>(DELIVERIES)
        .find_one(filter)
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

pub(super) fn empty(
    target: &BillingUsageComparison,
    usage: &CurrentUsage,
    coverage: &UsageCoverage,
) -> UsageProjection {
    UsageProjection {
        start_second: usage.start_second,
        end_second: usage.end_second_exclusive,
        layout: usage.quantities_micros.keys().cloned().collect(),
        generation: coverage.generation.clone(),
        uncertainty_revision: target.uncertainty_revision,
        sequence: 0,
        rebuild_upper: Some(target.acknowledgement_sequence),
        cursor_second: usage.start_second,
        cursor_sequence: 0,
        totals: BTreeMap::new(),
        unverified_origin: false,
    }
}
