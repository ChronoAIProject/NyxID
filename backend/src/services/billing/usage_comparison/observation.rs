use super::*;
use sha2::{Digest, Sha256};

pub(super) fn waiting(
    target: &BillingUsageComparison,
    coverage: &UsageCoverage,
    now: chrono::DateTime<Utc>,
    pass: &str,
    status: &str,
    reason: &str,
) -> UsageObservation {
    UsageObservation {
        status: status.into(),
        reason: reason.into(),
        generation: coverage.generation.clone(),
        sequence: target.acknowledgement_sequence,
        uncertainty_revision: target.uncertainty_revision,
        start_second: 0,
        end_second: 0,
        local: BTreeMap::new(),
        remote: BTreeMap::new(),
        differences: BTreeMap::new(),
        fingerprint: String::new(),
        first_pass_id: pass.into(),
        pass_id: pass.into(),
        stale_drift: target
            .comparison
            .as_ref()
            .is_some_and(|p| p.status == "drift" || p.stale_drift),
        first_observed_at: now,
        observed_at: now,
    }
}

pub(super) fn complete(
    target: &BillingUsageComparison,
    coverage: &UsageCoverage,
    projection: &UsageProjection,
    usage: &CurrentUsage,
    now: chrono::DateTime<Utc>,
    pass: &str,
) -> Result<UsageObservation, String> {
    if let Some(name) = projection
        .totals
        .keys()
        .find_map(|code| usage.non_sum_metrics.get(code))
    {
        return Err(format!("non_sum_aggregation:{name}"));
    }
    if projection
        .totals
        .keys()
        .any(|code| !usage.quantities_micros.contains_key(code))
    {
        return Err("delivered_metric_missing".into());
    }
    let mut observation = waiting(target, coverage, now, pass, "equal", "");
    observation.start_second = usage.start_second;
    observation.end_second = usage.end_second_exclusive;
    observation.stale_drift = false;
    for (code, remote) in &usage.quantities_micros {
        let local = projection.totals.get(code).copied().unwrap_or_default();
        observation
            .local
            .insert(code.clone(), decimal(local, false));
        observation
            .remote
            .insert(code.clone(), decimal(*remote, false));
        if local != *remote {
            observation
                .differences
                .insert(code.clone(), decimal(local - remote, true));
        }
    }
    if observation.differences.is_empty() {
        return Ok(observation);
    }
    observation.status = "pending".into();
    observation.reason = "awaiting_persistent_difference".into();
    let bytes = serde_json::to_vec(&(
        &observation.local,
        &observation.remote,
        observation.start_second,
        observation.end_second,
        &observation.generation,
        observation.sequence,
        observation.uncertainty_revision,
    ))
    .expect("primitive usage observation serializes");
    observation.fingerprint = hex::encode(Sha256::digest(bytes));
    if let Some(previous) = &target.comparison
        && matches!(previous.status.as_str(), "pending" | "drift")
        && previous.fingerprint == observation.fingerprint
        && previous.first_observed_at <= now
    {
        observation.first_observed_at = previous.first_observed_at;
        observation.first_pass_id = previous.first_pass_id.clone();
        if previous.first_pass_id != pass
            && now - previous.first_observed_at >= chrono::Duration::minutes(10)
        {
            observation.status = "drift".into();
            observation.reason = "persistent_difference".into();
        }
    }
    Ok(observation)
}

pub(super) fn decimal(value: i64, signed: bool) -> String {
    let absolute = i128::from(value).abs();
    let sign = if value < 0 {
        "-"
    } else if signed && value > 0 {
        "+"
    } else {
        ""
    };
    let fraction = format!("{:06}", absolute % 1_000_000);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        format!("{sign}{}", absolute / 1_000_000)
    } else {
        format!("{sign}{}.{fraction}", absolute / 1_000_000)
    }
}
