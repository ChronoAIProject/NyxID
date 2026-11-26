//! Observational comparison of captured deliveries with provider subscription periods.
pub(crate) mod coverage;
mod diagnostics;
mod observation;
mod projection;
mod publication;
mod schedule;
#[cfg(test)]
mod tests;

use super::{
    lago_client::{
        LagoApi,
        current_usage::{CurrentUsage, Finding},
    },
    recovery_batch::Failures,
};
use crate::services::coordination_service::{LeaseToken, active_lease_filter};
use crate::{
    errors::{AppError, AppResult},
    models::billing_usage_comparison::{
        BillingUsageComparison, COLLECTION_NAME, CONTROL_COLLECTION, COVERAGE_ID, SCHEDULE_ID,
        UsageCoverage, UsageObservation, UsagePeriod, UsageProjection,
    },
};
use chrono::Utc;
pub(crate) use diagnostics::diagnostics;
use futures::{FutureExt, StreamExt, TryStreamExt};
use mongodb::{
    Database,
    bson::{self, Bson, Document, doc},
    options::ReturnDocument,
};
use publication::Evidence;
use std::collections::BTreeMap;

#[cfg(test)]
tokio::task_local! { pub(super) static TEST_NOW: std::cell::Cell<bson::DateTime>; }
fn clock() -> Bson {
    #[cfg(test)]
    if let Ok(now) = TEST_NOW.try_with(std::cell::Cell::get) {
        return Bson::DateTime(now);
    }
    Bson::String("$$NOW".into())
}
fn invalid(message: &str) -> AppError {
    AppError::Internal(message.into())
}
fn target_filter(target: &BillingUsageComparison) -> Document {
    doc! {"owner_id":&target.owner_id,"customer_id":&target.customer_id,"subscription_id":&target.subscription_id}
}
fn period(usage: &CurrentUsage) -> UsagePeriod {
    UsagePeriod {
        start_second: usage.start_second,
        end_second: usage.end_second_exclusive,
        layout: usage.quantities_micros.keys().cloned().collect(),
    }
}
fn known_usage(period: &UsagePeriod) -> CurrentUsage {
    CurrentUsage {
        start_second: period.start_second,
        end_second_exclusive: period.end_second,
        quantities_micros: period.layout.iter().map(|s| (s.clone(), 0)).collect(),
        non_sum_metrics: BTreeMap::new(),
    }
}
struct Candidate {
    target: BillingUsageComparison,
    local: Option<UsageProjection>,
}
struct Completed {
    target: BillingUsageComparison,
    evidence: Evidence,
    error: Option<AppError>,
}

pub(super) async fn run(
    db: &Database,
    lago: &dyn LagoApi,
    token: &LeaseToken,
    stats: &mut super::reconcile::ReconcileStats,
) -> AppResult<()> {
    let coverage = coverage::refresh(db).await?;
    let mut failures = Failures::default();
    for error in schedule::discover(db).await? {
        failures.note(error);
    }
    if !coverage.ready || !super::reconcile::continue_reconcile() {
        return failures.finish(());
    }
    let mut targets = Vec::new();
    for row in schedule::select(db, coverage.observed_at).await? {
        match super::recovery_batch::decode::<BillingUsageComparison>(&row) {
            Ok(target) => targets.push(target),
            Err(error) => failures.note(error),
        }
    }
    let wallets = schedule::live_wallets(db, &targets).await?;
    let mut candidates = Vec::new();
    let mut completed = Vec::new();
    for mut target in targets {
        if !super::reconcile::continue_reconcile() {
            break;
        }
        if !wallets.contains(&(
            target.wallet_id.clone(),
            target.owner_id.clone(),
            target.customer_id.clone(),
            target.subscription_id.clone(),
        )) {
            completed.push(Completed {
                target,
                evidence: Evidence::Retired,
                error: None,
            });
            continue;
        }
        if let Err(error) = adopt_legacy_uncertainty(db, &mut target).await {
            completed.push(Completed {
                target,
                evidence: Evidence::Failed("capture_evidence_unreadable"),
                error: Some(error),
            });
            continue;
        }
        match preflight(db, &target, &coverage).await {
            Ok(Ok(local)) => candidates.push(Candidate { target, local }),
            Ok(Err(evidence)) => completed.push(Completed {
                target,
                evidence,
                error: None,
            }),
            Err(error) => completed.push(Completed {
                target,
                evidence: Evidence::Failed("receipt_projection_failed"),
                error: Some(error),
            }),
        }
    }
    let reserved = schedule::reserve(db, candidates.len()).await?;
    let mut work = Vec::new();
    for candidate in candidates.into_iter().take(reserved) {
        let coverage = &coverage;
        work.push(async move { read(db, lago, token, coverage, candidate).await }.boxed());
    }
    completed.extend(
        futures::stream::iter(work)
            .buffer_unordered(4)
            .collect::<Vec<_>>()
            .await,
    );
    let can_publish =
        !completed.is_empty() && publication::can_publish(db, token, &coverage).await?;
    for work in completed {
        let saved = if can_publish {
            publication::save(db, token, &work.target, &coverage, work.evidence).await
        } else {
            Ok(None)
        };
        if let Some(error) = work.error {
            failures.note(error);
            continue;
        }
        let saved = match saved {
            Ok(saved) => saved,
            Err(error) => {
                failures.note(error);
                continue;
            }
        };
        if let Some(observation) = saved.filter(|o| o.status == "drift") {
            for (metric, difference) in &observation.differences {
                tracing::warn!(owner_id=%work.target.owner_id,customer_id=%work.target.customer_id,
                    subscription_id=%work.target.subscription_id,metric_code=%metric,
                    local_quantity=%observation.local[metric],remote_quantity=%observation.remote[metric],difference=%difference,
                    "Billing Lago usage drift detected");
            }
            stats.drift_alerts += observation.differences.len() as u64;
        }
    }
    failures.finish(())
}

async fn adopt_legacy_uncertainty(
    db: &Database,
    target: &mut BillingUsageComparison,
) -> AppResult<()> {
    if target.coverage_unknown_reason.is_some() && target.uncertain_through.is_none() {
        let saved=db.collection::<Document>(COLLECTION_NAME).find_one_and_update(
            doc! {"_id":&target.id,"uncertain_through":Bson::Null},
            vec![doc! {"$set":{"uncertain_through":clock(),"coverage_verified":false,
                "uncertainty_revision":{"$add":[{"$ifNull":["$uncertainty_revision",0_i64]},1_i64]}}}],
        ).return_document(ReturnDocument::After).await?;
        let saved = match saved {
            Some(saved) => saved,
            None => db
                .collection::<Document>(COLLECTION_NAME)
                .find_one(doc! {"_id":&target.id})
                .await?
                .ok_or_else(|| invalid("Usage target disappeared"))?,
        };
        *target = super::recovery_batch::decode(&saved)?;
    }
    if target.acknowledgement_sequence < 0 || target.uncertainty_revision < 0 {
        return Err(invalid("Usage capture cursor is negative"));
    }
    Ok(())
}

async fn preflight(
    db: &Database,
    target: &BillingUsageComparison,
    coverage: &UsageCoverage,
) -> AppResult<Result<Option<UsageProjection>, Evidence>> {
    let now = coverage.observed_at;
    let Some(known) = target
        .provider_period
        .as_ref()
        .filter(|p| p.start_second <= now.timestamp() && now.timestamp() < p.end_second)
    else {
        return Ok(Ok(None));
    };
    let usage = known_usage(known);
    if let Err(reason) = coverage::eligible(target, coverage, &usage, now) {
        return Ok(Err(Evidence::Waiting(reason)));
    }
    if target
        .last_acknowledged_at
        .is_some_and(|at| now - at < chrono::Duration::minutes(10))
    {
        return Ok(Err(Evidence::Waiting("acknowledgements_not_quiet")));
    }
    if projection::has_unresolved(db, target, &usage).await? {
        return Ok(Err(Evidence::Waiting("unresolved_deliveries_in_period")));
    }
    let (folded, complete) = projection::fold(db, target, &usage, coverage).await?;
    if folded.unverified_origin || !complete {
        return Ok(Err(Evidence::Parsed {
            usage: Box::new(usage),
            projection: Some(folded),
            complete,
            blocked: None,
        }));
    }
    Ok(Ok(Some(folded)))
}

async fn read(
    db: &Database,
    lago: &dyn LagoApi,
    token: &LeaseToken,
    coverage: &UsageCoverage,
    candidate: Candidate,
) -> Completed {
    let target = candidate.target;
    let result = read_evidence(db, lago, token, coverage, &target, candidate.local).await;
    let (evidence, error) = match result {
        Ok(evidence) => (evidence, None),
        Err(error) => (
            Evidence::Failed("provider_or_projection_failed"),
            Some(error),
        ),
    };
    Completed {
        target,
        evidence,
        error,
    }
}

async fn read_evidence(
    db: &Database,
    lago: &dyn LagoApi,
    token: &LeaseToken,
    coverage: &UsageCoverage,
    target: &BillingUsageComparison,
    local: Option<UsageProjection>,
) -> AppResult<Evidence> {
    let live = db
        .collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
        .find_one(active_lease_filter(
            &token.name,
            &token.holder,
            &token.lease_id,
        ))
        .projection(doc! {"_id":1})
        .await?;
    if live.is_none() || !super::reconcile::continue_reconcile() {
        return Ok(Evidence::Waiting("lease_changed"));
    }
    // These target sequence/revision values were pinned before the sole provider read.
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        lago.current_usage_for_comparison(&target.customer_id, &target.subscription_id),
    )
    .await;
    let response = match response {
        Err(_) => {
            return Err(AppError::BillingProviderUnavailable(
                "Usage comparison timed out".into(),
            ));
        }
        Ok(Err(error))
            if super::recovery_batch::is_wait(&error)
                || matches!(
                    error,
                    AppError::RateLimited | AppError::RateLimitedWithRetry { .. }
                ) =>
        {
            return Ok(Evidence::Waiting("provider_try_later"));
        }
        Ok(Err(error)) => return Err(error),
        Ok(Ok(response)) => response,
    };
    if response.customer_id != target.customer_id
        || response.subscription_id != target.subscription_id
    {
        return Ok(Evidence::Finding("provider_target_mismatch".into()));
    }
    let usage = match super::lago_client::current_usage::parse(&response.raw) {
        Ok(usage) => usage,
        Err(finding) => return Ok(Evidence::Finding(finding_reason(finding))),
    };
    if coverage::eligible(target, coverage, &usage, coverage.observed_at).is_err()
        || target
            .last_acknowledged_at
            .is_some_and(|at| coverage.observed_at - at < chrono::Duration::minutes(10))
    {
        return Ok(Evidence::Parsed {
            usage: Box::new(usage),
            projection: None,
            complete: false,
            blocked: None,
        });
    }
    // A preparation racing the provider read may already have been accepted remotely.
    if projection::has_unresolved(db, target, &usage).await? {
        return Ok(Evidence::Parsed {
            usage: Box::new(usage),
            projection: None,
            complete: false,
            blocked: Some("unresolved_deliveries_in_period"),
        });
    }
    let (folded, complete) = if let Some(local) = local {
        if local.start_second == usage.start_second
            && local.end_second == usage.end_second_exclusive
            && local.layout == usage.quantities_micros.keys().cloned().collect::<Vec<_>>()
        {
            (local, true)
        } else {
            // Local folding already spent this visit's receipt budget. Rebuild the changed layout next time.
            (projection::empty(target, &usage, coverage), false)
        }
    } else {
        projection::fold(db, target, &usage, coverage).await?
    };
    Ok(Evidence::Parsed {
        usage: Box::new(usage),
        projection: Some(folded),
        complete,
        blocked: None,
    })
}

fn finding_reason(finding: Finding) -> String {
    match finding {
        Finding::MissingCustomerUsage => "missing_customer_usage".into(),
        Finding::MissingChargesUsage => "missing_charges_usage".into(),
        Finding::ChargeNotObject => "charge_not_object".into(),
        Finding::TooManyCharges => "too_many_charges".into(),
        Finding::BreakdownMismatch => "breakdown_mismatch".into(),
        Finding::ResponseTooLarge => "response_too_large".into(),
        Finding::InvalidPeriod => "invalid_provider_period".into(),
        Finding::InvalidUnits => "unsupported_provider_units".into(),
        Finding::DuplicateMetric => "duplicate_metric".into(),
        Finding::MissingMetric => "missing_metric_code".into(),
        Finding::SubscriptionUnavailable => "subscription_unavailable".into(),
        Finding::RequestRejected => "provider_request_rejected".into(),
    }
}
