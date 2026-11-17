//! A successful pass checks retained generation evidence without depending on heartbeat cadence.
use super::*;
use crate::models::{security_capability_activation as activation, security_replica};

pub(crate) const READINESS: &str = "billing:usage-receipts-v1";
// Heartbeat observations survive seven days. Keep a full day for TTL/clock uncertainty.
pub(super) const EVIDENCE_HORIZON_MILLIS: i64 = 6 * 86_400_000;
const LIVE_MILLIS: i64 = crate::services::security_readiness_service::MAX_AGE_SECS * 1_000;

pub(super) async fn refresh(db: &Database) -> AppResult<UsageCoverage> {
    let rows: Vec<Document> = db
        .collection::<Document>(CONTROL_COLLECTION)
        .aggregate([
            doc! {"$match":{"_id":COVERAGE_ID}},
            doc! {"$facet":{"previous":[{"$limit":1}]}},
            doc! {"$set":{"check_now":clock()}},
        ])
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    let row = rows
        .first()
        .ok_or_else(|| invalid("Coverage clock is unavailable"))?;
    let now = *row
        .get_datetime("check_now")
        .map_err(|_| invalid("Coverage clock is invalid"))?;
    let previous = row
        .get_array("previous")
        .map_err(|_| invalid("Coverage evidence is invalid"))?
        .first()
        .and_then(Bson::as_document)
        .map(super::super::recovery_batch::decode::<UsageCoverage>)
        .transpose()?;
    if previous
        .as_ref()
        .is_some_and(|p| p.observed_at > now.to_chrono())
    {
        return Err(invalid(
            "Coverage check clock precedes its last successful check",
        ));
    }
    let activations: Vec<activation::SecurityCapabilityActivation> = db
        .collection(activation::COLLECTION_NAME)
        .find(doc! {"record_kind":activation::RECORD_KIND,"capability":READINESS})
        .limit(2)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    if activations.len() > 1 {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    let activated = !activations.is_empty();
    let live_start = bson::DateTime::from_millis(now.timestamp_millis() - LIVE_MILLIS);
    let since = previous.as_ref().and_then(|p| p.continuous_since);
    let lower = since.map(bson::DateTime::from_chrono).unwrap_or(live_start);
    let unsupported = db
        .collection::<Document>(security_replica::COLLECTION_NAME)
        .find_one(doc! {
            "observed_at":{"$gte":lower},
            "$or":[{"protocol":{"$ne":1}},{"capabilities":{"$ne":READINESS}}],
        })
        .sort(doc! {"observed_at":-1})
        .projection(doc! {"observed_at":1})
        .max_time(std::time::Duration::from_secs(5))
        .await?;
    let unsupported_at = unsupported
        .as_ref()
        .map(|r| {
            r.get_datetime("observed_at")
                .copied()
                .map_err(|_| invalid("Generation evidence is unreadable"))
        })
        .transpose()?;
    let horizon_lost = previous.as_ref().is_some_and(|p| {
        now.timestamp_millis() - p.observed_at.timestamp_millis() >= EVIDENCE_HORIZON_MILLIS
    });
    let reset = unsupported_at.is_some() || horizon_lost;
    let live_unsupported = unsupported_at.is_some_and(|at| at >= live_start);
    let continuous_since = if reset || since.is_none() {
        (activated && !live_unsupported).then_some(now.to_chrono())
    } else {
        since
    };
    let generation = match &previous {
        Some(previous) if !reset && (since.is_some() || continuous_since.is_none()) => {
            previous.generation.clone()
        }
        _ => uuid::Uuid::new_v4().to_string(),
    };
    let mut filter = doc! {"_id":COVERAGE_ID};
    if let Some(previous) = &previous {
        filter.extend(doc! {"generation":&previous.generation,"observed_at":bson::DateTime::from_chrono(previous.observed_at)});
    }
    // No write occurs before every evidence read succeeds. A failed look cannot reset coverage.
    let saved = db.collection::<Document>(CONTROL_COLLECTION).find_one_and_update(filter,
        doc! {"$set":{"kind":"coverage","generation":generation,"ready":activated && continuous_since.is_some() && !live_unsupported,
            "continuous_since":continuous_since.map(bson::DateTime::from_chrono),"observed_at":now}},
    ).upsert(previous.is_none()).return_document(ReturnDocument::After).await?
        .ok_or_else(|| invalid("Coverage changed during its evidence check"))?;
    super::super::recovery_batch::decode(&saved)
}

pub(super) fn eligible(
    target: &BillingUsageComparison,
    coverage: &UsageCoverage,
    period: &CurrentUsage,
    now: chrono::DateTime<Utc>,
) -> Result<(), &'static str> {
    if !coverage.ready
        || coverage
            .continuous_since
            .is_none_or(|since| since.timestamp() >= period.start_second)
    {
        return Err("warming_full_period");
    }
    if target.coverage_unknown_reason.is_some() && target.uncertain_through.is_none() {
        return Err("legacy_capture_uncertainty");
    }
    if target
        .uncertain_through
        .is_some_and(|at| at.timestamp() >= period.start_second)
    {
        return Err("capture_uncertainty_in_period");
    }
    if now.timestamp() - period.start_second
        >= super::super::usage_delivery::RESOLVED_RETENTION_DAYS * 86_400
    {
        return Err("receipt_history_outside_retention");
    }
    Ok(())
}
