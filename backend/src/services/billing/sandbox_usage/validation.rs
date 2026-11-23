use super::{contract::Clock, scalars::MAX_QUANTITY, types::*};

pub(super) type Check = Result<(), &'static str>;
fn require(ok: bool, code: &'static str) -> Check {
    if ok { Ok(()) } else { Err(code) }
}

pub(super) fn event(header: &Header, payload: &Payload, clock: Clock) -> Check {
    require(header.schema_version == 1, "unsupported_schema_version")?;
    require(
        (header.sequence.get() == 1) == header.previous_event_digest.is_none(),
        "invalid_chain_origin",
    )?;
    let expected_definition = match header.basis {
        Basis::Allocated => "allocated-v1",
        Basis::Measured => "measured-v1",
    };
    require(
        header.metric_definition_version.as_str() == expected_definition,
        "unsupported_metric_definition",
    )?;
    let expected_algorithm = match header.basis {
        Basis::Allocated => "lifecycle-minute-split-v1",
        Basis::Measured => "binding-integral-half-even-v1",
    };
    require(
        header.algorithm_version.as_str() == expected_algorithm,
        "unsupported_algorithm_version",
    )?;
    require(
        header.source_cutoff_ms <= header.observed_at_ms
            && header.observed_at_ms <= header.produced_at_ms,
        "invalid_evidence_time_order",
    )?;
    clock.check(header.produced_at_ms.get())?;
    require(
        !header.source_refs.is_empty() && header.source_refs.len() <= 64,
        "source_reference_bound",
    )?;
    require(
        header.source_refs.windows(2).all(|w| w[0] < w[1]),
        "source_reference_order_or_duplicate",
    )?;
    require(
        (header.sequence.get() == 1) == matches!(payload, Payload::StreamOpen { .. }),
        "invalid_stream_open_sequence",
    )?;
    match payload {
        Payload::StreamOpen { opening } => {
            require(
                header.source_refs.contains(&opening.first_source),
                "opening_source_not_referenced",
            )?;
            require(
                opening.tracking_started_at_ms <= opening.capture_start_ms
                    && opening.capture_start_ms <= header.source_cutoff_ms,
                "invalid_capture_baseline",
            )?;
            require(
                opening.capture_start_ms.get() <= clock.now_ms,
                "future_capture_baseline",
            )?;
            require(
                !opening.complete_lifecycle
                    || (opening.capture_start_ms == opening.tracking_started_at_ms
                        && opening.historical_missing_ranges.is_empty()),
                "incomplete_lifecycle_baseline",
            )?;
            require(
                opening.historical_missing_ranges.len() <= 64,
                "missing_range_bound",
            )?;
            let mut previous_end = opening.tracking_started_at_ms;
            for range in &opening.historical_missing_ranges {
                require(
                    previous_end <= range.start_ms
                        && range.start_ms < range.end_ms
                        && range.end_ms <= opening.capture_start_ms,
                    "invalid_missing_range",
                )?;
                previous_end = range.end_ms;
            }
        }
        Payload::Usage { facts } => {
            require(facts.len() <= 64, "fact_bound")?;
            require(
                facts.windows(2).all(|w| w[0].fact_id < w[1].fact_id),
                "fact_order_or_duplicate",
            )?;
            for (i, item) in facts.iter().enumerate() {
                require(item.evidence_revision.get() == 1, "initial_fact_revision")?;
                fact(header, item, clock)?;
                for other in &facts[..i] {
                    if item.start_ms < other.end_ms && other.start_ms < item.end_ms {
                        let compute = has_compute(item) && has_compute(other);
                        let disk = has_home(item) && has_home(other);
                        require(
                            !(compute
                                && item.resource.runtime_id == other.resource.runtime_id
                                && item.resource.runtime_generation
                                    == other.resource.runtime_generation)
                                && !(disk
                                    && item.resource.home_id == other.resource.home_id
                                    && item.resource.home_generation
                                        == other.resource.home_generation),
                            "overlapping_resource_facts",
                        )?;
                    }
                }
            }
        }
        Payload::Correction { correction } => {
            require(
                correction.fact_id == correction.replacement.fact_id,
                "correction_fact_mismatch",
            )?;
            require(
                correction.expected_predecessor_event_id != header.event_id,
                "correction_self_reference",
            )?;
            require(
                correction.expected_evidence_revision.get().checked_add(1)
                    == Some(correction.replacement.evidence_revision.get()),
                "correction_revision",
            )?;
            fact(header, &correction.replacement, clock)?;
        }
        Payload::StreamSeal { seal } => {
            require(
                header.source_refs.contains(&seal.final_source),
                "seal_source_not_referenced",
            )?;
            require(
                seal.sealed_at_ms <= header.source_cutoff_ms
                    && seal.sealed_at_ms.get() <= clock.now_ms,
                "invalid_seal_time",
            )?;
        }
    }
    Ok(())
}

fn fact(header: &Header, fact: &Fact, clock: Clock) -> Check {
    let resource = &fact.resource;
    require(
        resource.runtime_id.is_some() == resource.runtime_generation.is_some()
            && resource.home_id.is_some() == resource.home_generation.is_some(),
        "partial_resource_binding",
    )?;
    require(
        !has_compute(fact) || resource.runtime_id.is_some(),
        "compute_binding_required",
    )?;
    require(
        !has_home(fact) || resource.home_id.is_some(),
        "home_binding_required",
    )?;
    let start = fact.start_ms.get();
    let end = fact.end_ms.get();
    let bucket = fact.bucket_start_ms.get();
    require(
        bucket.is_multiple_of(60_000) && start >= bucket && end > start && end <= bucket + 60_000,
        "invalid_minute_interval",
    )?;
    require(
        end <= header.source_cutoff_ms.get() && end <= clock.now_ms,
        "future_interval",
    )?;
    let audit = &fact.audit_time;
    require(
        audit.compute_uptime_ms.get() <= end - start && audit.retained_home_ms.get() <= end - start,
        "invalid_audit_time",
    )?;
    let metrics = match header.basis {
        Basis::Allocated => [
            Metric::AllocatedCpu,
            Metric::AllocatedMemory,
            Metric::AllocatedDisk,
        ],
        Basis::Measured => [
            Metric::MeasuredCpu,
            Metric::MeasuredMemory,
            Metric::MeasuredDisk,
        ],
    };
    require(
        fact.quantities.len() == 3 && metrics.iter().all(|m| fact.quantities.contains_key(m)),
        "metric_basis_mismatch",
    )?;
    let coverage = [
        &fact.coverage.cpu,
        &fact.coverage.memory,
        &fact.coverage.disk,
    ];
    for (metric, coverage) in metrics.iter().zip(coverage) {
        quality(coverage, fact.quantities[metric].map(|q| q.get()))?;
    }
    match header.basis {
        Basis::Measured => require(fact.allocation.is_none(), "measured_allocation_forbidden")?,
        Basis::Allocated => {
            let allocation = fact.allocation.as_ref().ok_or("allocation_required")?;
            require(
                start.is_multiple_of(1000) && end.is_multiple_of(1000),
                "allocated_subsecond_interval",
            )?;
            let expected = [
                audit.compute_uptime_ms.get(),
                audit.compute_uptime_ms.get(),
                audit.retained_home_ms.get(),
            ];
            let capacities = [
                allocation.cpu_millis.get(),
                allocation.memory_mib.get(),
                allocation.disk_gib.get(),
            ];
            for index in 0..3 {
                let covered = coverage[index].covered_binding_ms.get();
                require(
                    coverage[index].expected_binding_ms.get() == expected[index] as u128
                        && covered.is_multiple_of(1000)
                        && expected[index].is_multiple_of(1000),
                    "allocated_coverage",
                )?;
                let amount = capacities[index]
                    .checked_mul(covered / 1000)
                    .filter(|v| *v <= MAX_QUANTITY)
                    .ok_or("allocation_overflow")?;
                if let Some(quantity) = fact.quantities[&metrics[index]] {
                    require(quantity.get() == amount, "allocation_quantity_mismatch")?;
                }
            }
        }
    }
    Ok(())
}

fn quality(coverage: &Coverage, quantity: Option<u128>) -> Check {
    let covered = coverage.covered_binding_ms.get();
    let expected = coverage.expected_binding_ms.get();
    require(covered <= expected, "coverage_overflow")?;
    match coverage.quality {
        Quality::Observed => require(
            covered == expected
                && quantity.is_some()
                && coverage.missing_reason.is_none()
                && (expected != 0 || quantity == Some(0)),
            "invalid_observed_quantity",
        ),
        Quality::Partial => require(
            covered > 0
                && covered < expected
                && quantity.is_some()
                && coverage.missing_reason.is_some(),
            "invalid_partial_quantity",
        ),
        Quality::Missing => require(
            covered == 0 && expected > 0 && quantity.is_none() && coverage.missing_reason.is_some(),
            "missing_is_not_zero",
        ),
        Quality::Estimated => require(
            quantity.is_some()
                && coverage.missing_reason.is_some()
                && (expected != 0 || quantity == Some(0)),
            "invalid_estimated_quantity",
        ),
    }
}

fn has_compute(fact: &Fact) -> bool {
    fact.audit_time.compute_uptime_ms.get() > 0
        || fact.coverage.cpu.expected_binding_ms.get() > 0
        || fact.coverage.memory.expected_binding_ms.get() > 0
}

fn has_home(fact: &Fact) -> bool {
    fact.audit_time.retained_home_ms.get() > 0 || fact.coverage.disk.expected_binding_ms.get() > 0
}
