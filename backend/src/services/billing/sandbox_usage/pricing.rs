//! Candidate fractions only: rounding, funding and delivery belong to settlement.
use super::{
    contract::{Basis, Event, Fact, Metric, Payload, Quality},
    policy::{invalid, scope_key},
};
use crate::{
    errors::AppResult,
    models::sandbox_usage_rates::{
        ChargeMode, ChargeUnit, PricePin, Selection, UsageRate, UsageTariff,
    },
};
use chrono::{DateTime, Datelike, Utc};

pub const METRICS: [Metric; 6] = [
    Metric::AllocatedCpu,
    Metric::AllocatedMemory,
    Metric::AllocatedDisk,
    Metric::MeasuredCpu,
    Metric::MeasuredMemory,
    Metric::MeasuredDisk,
];

pub fn code(metric: Metric) -> &'static str {
    match metric {
        Metric::AllocatedCpu => "sandbox_cpu_millis_seconds",
        Metric::AllocatedMemory => "sandbox_memory_mib_seconds",
        Metric::AllocatedDisk => "sandbox_disk_gib_seconds",
        Metric::MeasuredCpu => "sandbox_cpu_core_nanoseconds_v1",
        Metric::MeasuredMemory => "sandbox_memory_working_set_byte_milliseconds_v1",
        Metric::MeasuredDisk => "sandbox_disk_used_byte_milliseconds_v1",
    }
}
pub fn definition(metric: Metric) -> &'static str {
    match metric {
        Metric::AllocatedCpu | Metric::AllocatedMemory | Metric::AllocatedDisk => "allocated-v1",
        _ => "measured-v1",
    }
}
pub fn unit(metric: Metric) -> ChargeUnit {
    match metric {
        Metric::AllocatedCpu | Metric::MeasuredCpu => ChargeUnit::CoreHour,
        _ => ChargeUnit::GibHour,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NonBillable {
    Shadow,
    Free,
    Excluded,
    OtherBasis,
    LegacyCoverage,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    IncompleteEvidence,
    MissingTariff,
    LegacyCoverageUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Disposition {
    NonBillable(NonBillable),
    Pending(Pending),
    /// Must never be treated as a settled or externally authorized charge.
    Candidate {
        numerator_pico: u128,
        divisor: u128,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PricePlan {
    pub charge_mode: ChargeMode,
    pub metric: Metric,
    pub pin_id: String,
    pub tariff_id: Option<String>,
    pub billing_period: String,
    pub disposition: Disposition,
}

pub fn exact_fraction(quantity: u128, rate: &UsageRate, metric: Metric) -> AppResult<(u128, u128)> {
    if rate.metric_code != code(metric)
        || rate.definition_version != definition(metric)
        || rate.charge_unit != unit(metric)
        || rate.credits_per_unit.pico() < 0
    {
        return Err(invalid("Usage tariff metric definition mismatch"));
    }
    let numerator = quantity
        .checked_mul(rate.credits_per_unit.pico() as u128)
        .ok_or_else(|| invalid("Usage price multiplication overflow"))?;
    let divisor = metric.units_per_hour();
    if numerator / divisor > crate::models::credits::MAX_PICO as u128
        || (numerator / divisor == crate::models::credits::MAX_PICO as u128
            && numerator % divisor != 0)
    {
        return Err(invalid("Usage price exceeds exact credit storage"));
    }
    Ok((numerator, divisor))
}

pub fn shadow_plan(
    event: &Event,
    fact_id: &str,
    pin: &PricePin,
    tariff: Option<&UsageTariff>,
) -> AppResult<Vec<PricePlan>> {
    let header = event.header();
    let fact = match event.payload() {
        Payload::Usage { facts } => facts.iter().find(|f| f.fact_id.as_str() == fact_id),
        Payload::Correction { correction } if correction.fact_id.as_str() == fact_id => {
            Some(&correction.replacement)
        }
        _ => None,
    }
    .ok_or_else(|| invalid("Usage fact not found"))?;
    if pin.schema_version != 1
        || pin.scope_key != scope_key(&pin.scope)?
        || pin.scope.producer_namespace != header.producer_namespace.as_str()
        || pin.scope.sandbox_id != header.sandbox_id.as_str()
        || pin.scope.source_epoch != header.source_epoch.as_str()
        || pin.minute.timestamp_millis() != fact.bucket_start_ms.get() as i64
    {
        return Err(invalid("Usage price pin does not match evidence"));
    }
    if let Some(tariff) = tariff
        && (tariff.schema_version != 1
            || Some(&tariff.id) != pin.tariff_id.as_ref()
            || Some(tariff.version) != pin.tariff_version
            || tariff.producer_namespace != pin.scope.producer_namespace
            || tariff.effective_from > pin.minute)
    {
        return Err(invalid("Usage tariff does not match price pin"));
    }
    fact.quantities
        .iter()
        .map(|(metric, quantity)| {
            let selection = match metric {
                Metric::AllocatedCpu | Metric::MeasuredCpu => pin.dimensions.cpu,
                Metric::AllocatedMemory | Metric::MeasuredMemory => pin.dimensions.memory,
                Metric::AllocatedDisk | Metric::MeasuredDisk => pin.dimensions.disk,
            };
            let disposition = match selection {
                Selection::Shadow => Disposition::NonBillable(NonBillable::Shadow),
                Selection::Free => Disposition::NonBillable(NonBillable::Free),
                Selection::Excluded => Disposition::NonBillable(NonBillable::Excluded),
                Selection::Allocated if header.basis != Basis::Allocated => {
                    Disposition::NonBillable(NonBillable::OtherBasis)
                }
                Selection::Measured if header.basis != Basis::Measured => {
                    Disposition::NonBillable(NonBillable::OtherBasis)
                }
                _ => candidate(fact, *metric, quantity.map(|q| q.get()), pin, tariff)?,
            };
            Ok(PricePlan {
                charge_mode: pin.charge_mode,
                metric: *metric,
                pin_id: pin.id.clone(),
                tariff_id: pin.tariff_id.clone(),
                billing_period: period(pin.minute),
                disposition,
            })
        })
        .collect()
}

fn period(at: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", at.year(), at.month())
}
fn candidate(
    fact: &Fact,
    metric: Metric,
    quantity: Option<u128>,
    pin: &PricePin,
    tariff: Option<&UsageTariff>,
) -> AppResult<Disposition> {
    let Some(legacy) = pin.legacy_coverage.as_ref() else {
        return Ok(Disposition::Pending(Pending::LegacyCoverageUnknown));
    };
    if pin.minute < legacy.covered_until {
        return Ok(Disposition::NonBillable(NonBillable::LegacyCoverage));
    }
    let coverage = match metric {
        Metric::AllocatedCpu | Metric::MeasuredCpu => &fact.coverage.cpu,
        Metric::AllocatedMemory | Metric::MeasuredMemory => &fact.coverage.memory,
        Metric::AllocatedDisk | Metric::MeasuredDisk => &fact.coverage.disk,
    };
    if coverage.quality != Quality::Observed
        || coverage.covered_binding_ms != coverage.expected_binding_ms
        || coverage.missing_reason.is_some()
    {
        return Ok(Disposition::Pending(Pending::IncompleteEvidence));
    }
    let Some(quantity) = quantity else {
        return Ok(Disposition::Pending(Pending::IncompleteEvidence));
    };
    let Some(rate) = tariff.and_then(|t| t.rates.iter().find(|r| r.metric_code == code(metric)))
    else {
        return Ok(Disposition::Pending(Pending::MissingTariff));
    };
    let (numerator_pico, divisor) = exact_fraction(quantity, rate, metric)?;
    Ok(Disposition::Candidate {
        numerator_pico,
        divisor,
    })
}
