//! Validate a producer completeness statement against the receiver's v1 facts.
use super::{
    completeness_contract::{
        Basis, ContributionKind, Coverage, Dimension, FactRef, Number, Resource, Statement,
    },
    contract::{self, Fact, Metric, Quality},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Evidence {
    pub reference: FactRef,
    pub source_digest: String,
    pub source_revision: u64,
    pub fact: Fact,
}

/// The caller has already established exact validated positions and stream identity.
/// This compares both the expected source timeline and the complete effective fact set.
pub(super) fn verify(
    statement: &Statement,
    allocated: &[Evidence],
    effective: &[Evidence],
) -> Result<(), &'static str> {
    verify_timeline(statement, allocated)?;
    let mut references = effective
        .iter()
        .map(|item| item.reference.clone())
        .collect::<Vec<_>>();
    references.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    if references != statement.facts {
        return Err("effective_fact_set_changed");
    }
    let by_id = effective
        .iter()
        .map(|item| (item.reference.fact_id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let mut assigned = BTreeMap::<(&str, Dimension), u64>::new();
    for contribution in &statement.contributions {
        let interval = &contribution.interval;
        let mut references = BTreeSet::new();
        for id in &contribution.facts {
            let item = by_id.get(id.as_str()).ok_or("unknown_fact")?;
            let fact = &item.fact;
            if fact.start_ms.get() > interval.start_ms.0
                || fact.end_ms.get() < interval.end_ms.0
                || resource(fact, contribution.dimension).as_ref() != Some(&contribution.resource)
                || coverage(fact, statement.basis, contribution.dimension) != contribution.coverage
            {
                return Err("contributor_fact_mismatch");
            }
            if !references.insert(id) {
                return Err("duplicate_fact_assignment");
            }
            let total = assigned
                .entry((id.as_str(), contribution.dimension))
                .or_default();
            *total = total
                .checked_add(interval.end_ms.0 - interval.start_ms.0)
                .ok_or("coverage_overflow")?;
        }
        if contribution.kind == ContributionKind::Residual && contribution.facts.is_empty() {
            return Err("residual_evidence_missing");
        }
    }
    for item in effective {
        for dimension in [Dimension::Cpu, Dimension::Memory, Dimension::Disk] {
            let fact = &item.fact;
            let metric = metric_coverage(fact, dimension);
            let actual = assigned
                .get(&(item.reference.fact_id.as_str(), dimension))
                .copied()
                .unwrap_or_default();
            // Expected binding time proves the observed/partial/missing quantity's
            // extent. A zero-duration inactive dimension has no priced contributor.
            if actual as u128 != metric.expected_binding_ms.get() {
                return Err("fact_interval_coverage_mismatch");
            }
        }
    }
    Ok(())
}

fn verify_timeline(statement: &Statement, allocated: &[Evidence]) -> Result<(), &'static str> {
    // Allocated v1 publication splits each authoritative capture into minute
    // fragments. Their exact references establish the statement's timeline.
    if allocated.len() != statement.timeline.len() {
        return Err("lifecycle_fact_set_changed");
    }
    let mut by_interval = BTreeMap::new();
    for item in allocated {
        let key = (item.fact.start_ms.get(), item.fact.end_ms.get());
        if by_interval.insert(key, item).is_some() {
            return Err("duplicate_lifecycle_interval");
        }
    }
    for slice in &statement.timeline {
        let item = by_interval
            .get(&(slice.interval.start_ms.0, slice.interval.end_ms.0))
            .ok_or("lifecycle_interval_missing")?;
        let fact = &item.fact;
        let duration = slice.interval.end_ms.0 - slice.interval.start_ms.0;
        let compute = match fact.audit_time.compute_uptime_ms.get() {
            0 => None,
            value if value == duration => resource(fact, Dimension::Cpu)
                .ok_or("compute_binding_missing")
                .map(Some)?,
            _ => return Err("lifecycle_compute_extent"),
        };
        let home = match fact.audit_time.retained_home_ms.get() {
            0 => None,
            value if value == duration => resource(fact, Dimension::Disk)
                .ok_or("home_binding_missing")
                .map(Some)?,
            _ => return Err("lifecycle_home_extent"),
        };
        if slice.compute != compute
            || slice.home != home
            || item.source_digest != slice.source_digest
            || slice.source_revision != Number(item.source_revision)
        {
            return Err("lifecycle_source_mismatch");
        }
    }
    Ok(())
}

fn resource(fact: &Fact, dimension: Dimension) -> Option<Resource> {
    let identity = &fact.resource;
    match dimension {
        Dimension::Cpu | Dimension::Memory => Some(Resource {
            id: identity.runtime_id.as_ref()?.as_str().into(),
            generation: identity.runtime_generation?.get().to_string(),
        }),
        Dimension::Disk => Some(Resource {
            id: identity.home_id.as_ref()?.as_str().into(),
            generation: identity.home_generation.as_ref()?.as_str().into(),
        }),
    }
}

fn metric_coverage(fact: &Fact, dimension: Dimension) -> &contract::Coverage {
    match dimension {
        Dimension::Cpu => &fact.coverage.cpu,
        Dimension::Memory => &fact.coverage.memory,
        Dimension::Disk => &fact.coverage.disk,
    }
}

fn coverage(fact: &Fact, basis: Basis, dimension: Dimension) -> Coverage {
    let metric = match (basis, dimension) {
        (Basis::Allocated, Dimension::Cpu) => Metric::AllocatedCpu,
        (Basis::Allocated, Dimension::Memory) => Metric::AllocatedMemory,
        (Basis::Allocated, Dimension::Disk) => Metric::AllocatedDisk,
        (Basis::Measured, Dimension::Cpu) => Metric::MeasuredCpu,
        (Basis::Measured, Dimension::Memory) => Metric::MeasuredMemory,
        (Basis::Measured, Dimension::Disk) => Metric::MeasuredDisk,
    };
    let value = fact.quantities.get(&metric).and_then(|quantity| *quantity);
    match metric_coverage(fact, dimension).quality {
        Quality::Observed if value.is_some_and(|quantity| quantity.get() == 0) => {
            Coverage::ObservedZero
        }
        Quality::Observed if value.is_some() => Coverage::Observed,
        Quality::Observed => Coverage::Unresolved,
        Quality::Missing => Coverage::Missing,
        Quality::Partial => Coverage::Partial,
        Quality::Estimated => Coverage::Estimated,
    }
}
