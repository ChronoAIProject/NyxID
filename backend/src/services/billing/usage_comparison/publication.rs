//! Each target atomically publishes its projection, observation and schedule.
use super::*;
use crate::models::coordination::LEASE_COLLECTION_NAME;

pub(super) enum Evidence {
    Parsed {
        usage: Box<CurrentUsage>,
        projection: Option<UsageProjection>,
        complete: bool,
        blocked: Option<&'static str>,
    },
    Finding(String),
    Failed(&'static str),
    Waiting(&'static str),
    Retired,
}

pub(super) async fn can_publish(
    db: &Database,
    token: &LeaseToken,
    coverage: &UsageCoverage,
) -> AppResult<bool> {
    let lease = db
        .collection::<Document>(LEASE_COLLECTION_NAME)
        .find_one(active_lease_filter(
            &token.name,
            &token.holder,
            &token.lease_id,
        ))
        .projection(doc! {"_id":1})
        .await?;
    if lease.is_none() {
        return Ok(false);
    }
    Ok(db
        .collection::<Document>(CONTROL_COLLECTION)
        .find_one(doc! {"_id":COVERAGE_ID,"generation":&coverage.generation,"ready":true})
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

pub(super) async fn save(
    db: &Database,
    token: &LeaseToken,
    target: &BillingUsageComparison,
    coverage: &UsageCoverage,
    evidence: Evidence,
) -> AppResult<Option<UsageObservation>> {
    let now = coverage.observed_at;
    let evidence = &evidence;
    let pass = token.lease_id.as_str();
    let mut projection = None;
    let mut known_period = target.provider_period.clone();
    let observation = match evidence {
        Evidence::Finding(reason) => {
            observation::waiting(target, coverage, now, pass, "unsupported", reason)
        }
        Evidence::Failed(reason) => {
            observation::waiting(target, coverage, now, pass, "error", reason)
        }
        Evidence::Waiting(reason) => observation::waiting(
            target,
            coverage,
            now,
            pass,
            if coverage_reason(reason) {
                "coverage"
            } else {
                "waiting"
            },
            reason,
        ),
        Evidence::Retired => observation::waiting(
            target,
            coverage,
            now,
            pass,
            "retired",
            "wallet_binding_removed",
        ),
        Evidence::Parsed {
            usage,
            projection: folded,
            complete,
            blocked,
        } => {
            known_period = Some(period(usage));
            if now.timestamp() < usage.start_second || now.timestamp() >= usage.end_second_exclusive
            {
                observation::waiting(
                    target,
                    coverage,
                    now,
                    pass,
                    "waiting",
                    "period_changed_during_read",
                )
            } else if let Err(reason) = coverage::eligible(target, coverage, usage, now) {
                observation::waiting(target, coverage, now, pass, "coverage", reason)
            } else if target
                .last_acknowledged_at
                .is_some_and(|at| now - at < chrono::Duration::minutes(10))
            {
                observation::waiting(
                    target,
                    coverage,
                    now,
                    pass,
                    "waiting",
                    "acknowledgements_not_quiet",
                )
            } else if let Some(reason) = blocked {
                observation::waiting(target, coverage, now, pass, "waiting", reason)
            } else if let Some(folded) = folded {
                projection = Some(folded);
                if folded.unverified_origin {
                    observation::waiting(
                        target,
                        coverage,
                        now,
                        pass,
                        "coverage",
                        "duplicate_delivery_origin_unverified",
                    )
                } else if !complete {
                    observation::waiting(
                        target,
                        coverage,
                        now,
                        pass,
                        "coverage",
                        "projection_backlog",
                    )
                } else {
                    observation::complete(target, coverage, folded, usage, now, pass)
                        .unwrap_or_else(|reason| {
                            observation::waiting(
                                target,
                                coverage,
                                now,
                                pass,
                                "unsupported",
                                &reason,
                            )
                        })
                }
            } else {
                observation::waiting(
                    target,
                    coverage,
                    now,
                    pass,
                    "coverage",
                    "projection_pending",
                )
            }
        }
    };
    let provider_errors = match evidence {
        Evidence::Failed(_) => target.provider_errors.saturating_add(1).min(10),
        Evidence::Parsed { .. } | Evidence::Finding(_) => 0,
        _ => target.provider_errors,
    };
    let next = next_visit(
        target,
        &observation,
        known_period.as_ref(),
        provider_errors,
        now,
    );
    let mut fields = doc! {"comparison":bson::to_bson(&observation).map_err(|_|invalid("Cannot encode usage observation"))?,
    "coverage_verified":matches!(observation.status.as_str(),"equal"|"pending"|"drift"),
    "provider_period":bson::to_bson(&known_period).map_err(|_|invalid("Cannot encode provider period"))?,
    "next_visit_at":next.map(bson::DateTime::from_chrono),"provider_errors":i64::from(provider_errors)};
    if matches!(evidence, Evidence::Retired) {
        fields.insert("retired_at", bson::DateTime::from_chrono(now));
    }
    if let Some(projection) = projection {
        fields.insert(
            "projection",
            bson::to_bson(projection).map_err(|_| invalid("Cannot encode usage projection"))?,
        );
    }
    let time_guard = if matches!(observation.status.as_str(), "equal" | "pending" | "drift") {
        doc! {"$and":[{"$gte":[clock(),bson::DateTime::from_millis(observation.start_second.saturating_mul(1_000))]},
        {"$lt":[clock(),bson::DateTime::from_millis(observation.end_second.saturating_mul(1_000))]}]}
    } else {
        doc! {"$eq":[1,1]}
    };
    let changed=db.collection::<Document>(COLLECTION_NAME).update_one(doc! {
        "_id":&target.id,"acknowledgement_sequence":target.acknowledgement_sequence,
        "coverage_unknown_reason":&target.coverage_unknown_reason,"comparison.pass_id":{"$ne":pass},
        "$expr":{"$and":[time_guard,{"$eq":[{"$ifNull":["$uncertainty_revision",0_i64]},target.uncertainty_revision]}]},
    },doc! {"$set":fields}).await?;
    Ok((changed.matched_count == 1).then_some(observation))
}

fn coverage_reason(reason: &str) -> bool {
    matches!(
        reason,
        "warming_full_period"
            | "legacy_capture_uncertainty"
            | "capture_uncertainty_in_period"
            | "receipt_history_outside_retention"
            | "duplicate_delivery_origin_unverified"
    )
}
fn next_visit(
    target: &BillingUsageComparison,
    observation: &UsageObservation,
    known: Option<&UsagePeriod>,
    errors: u32,
    now: chrono::DateTime<Utc>,
) -> Option<chrono::DateTime<Utc>> {
    let after = |seconds| Some(now + chrono::Duration::seconds(seconds));
    if observation.status == "retired" {
        return None;
    }
    if coverage_reason(&observation.reason) {
        return known
            .and_then(|p| chrono::DateTime::from_timestamp(p.end_second, 0))
            .filter(|end| *end > now)
            .or_else(|| after(300));
    }
    if observation.reason == "acknowledgements_not_quiet" {
        return Some(
            target
                .last_acknowledged_at
                .map(|at| at + chrono::Duration::minutes(10))
                .unwrap_or(now + chrono::Duration::minutes(10))
                .max(now),
        );
    }
    match observation.status.as_str() {
        "equal" => after(6 * 3600),
        "pending" | "drift" => after(600),
        "unsupported" => after(24 * 3600),
        "error" => after((300_i64 * (1_i64 << errors.saturating_sub(1).min(7))).min(6 * 3600)),
        _ if observation.reason == "unresolved_deliveries_in_period"
            || observation.reason == "provider_try_later" =>
        {
            after(600)
        }
        _ => after(300),
    }
}
