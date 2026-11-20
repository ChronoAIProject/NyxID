use super::{
    contract::{self, Event, Metric},
    contract_tests::{decode, fixture},
    policy,
    pricing::*,
};
use crate::models::{credits::Credits, sandbox_usage_rates::*};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

pub(super) fn time() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_795_132_800_000).unwrap()
}
pub(super) fn scope() -> UsageScope {
    UsageScope {
        producer_namespace: "cma-example".into(),
        owner_id: "owner-example".into(),
        sandbox_id: "sbx_synthetic".into(),
        source_epoch: "00000000-0000-4000-8000-000000000002".into(),
    }
}
pub(super) fn rates() -> Vec<UsageRate> {
    METRICS
        .into_iter()
        .map(|m| UsageRate {
            metric_code: code(m).into(),
            definition_version: definition(m).into(),
            charge_unit: unit(m),
            credits_per_unit: Credits::from_whole(1),
            provider_proof: None,
        })
        .collect()
}
pub(super) fn tariff() -> UsageTariff {
    UsageTariff {
        id: uuid::Uuid::new_v4().to_string(),
        schema_version: 1,
        producer_namespace: scope().producer_namespace,
        version: 1,
        effective_from: time(),
        rates: rates(),
        created_by: "admin".into(),
        created_at: time() - Duration::minutes(10),
    }
}
pub(super) fn pin(tariff: &UsageTariff) -> PricePin {
    PricePin {
        id: uuid::Uuid::new_v4().to_string(),
        schema_version: 1,
        scope_key: policy::scope_key(&scope()).unwrap(),
        scope: scope(),
        minute: time(),
        charge_mode: ChargeMode::Shadow,
        policy_id: Some(uuid::Uuid::new_v4().to_string()),
        policy_version: Some(1),
        dimensions: DimensionPolicy {
            cpu: Selection::Measured,
            memory: Selection::Measured,
            disk: Selection::Allocated,
        },
        legacy_coverage: Some(LegacyCoverage {
            covered_until: time(),
            manifest_digest: "a".repeat(64),
        }),
        tariff_id: Some(tariff.id.clone()),
        tariff_version: Some(tariff.version),
        created_at: time(),
    }
}
fn plan(event: &Event, p: &PricePin, t: Option<&UsageTariff>) -> Vec<PricePlan> {
    shadow_plan(event, "00000000-0000-4000-8000-000000000064", p, t).unwrap()
}

#[test]
fn publication_uses_full_301_second_notice_then_ceil_minute() {
    let now = time() + Duration::milliseconds(1);
    assert!(policy::effective_minute(now + Duration::seconds(300), now).is_err());
    assert_eq!(
        policy::effective_minute(now + Duration::seconds(301), now).unwrap(),
        time() + Duration::minutes(6)
    );
    assert_eq!(
        policy::effective_minute(time() + Duration::minutes(6), now).unwrap(),
        time() + Duration::minutes(6)
    );
    assert_eq!(
        policy::effective_minute(
            time() + Duration::minutes(6) + Duration::nanoseconds(1),
            now
        )
        .unwrap(),
        time() + Duration::minutes(7)
    );
    assert!(policy::effective_minute(now - Duration::days(1), now).is_err());
    assert!(policy::effective_minute(DateTime::<Utc>::MAX_UTC, DateTime::<Utc>::MAX_UTC).is_err());
}

#[test]
fn exact_divisors_tiny_rates_and_split_conservation() {
    let divisors = [
        3_600_000,
        3_686_400,
        3_600,
        3_600_000_000_000,
        3_865_470_566_400_000,
        3_865_470_566_400_000,
    ];
    for ((metric, mut rate), divisor) in METRICS.into_iter().zip(rates()).zip(divisors) {
        assert_eq!(metric.units_per_hour(), divisor);
        rate.credits_per_unit = Credits::from_pico(1).unwrap();
        assert_eq!(
            exact_fraction(divisor, &rate, metric).unwrap(),
            (divisor, divisor)
        );
        let a = exact_fraction(1, &rate, metric).unwrap();
        let b = exact_fraction(divisor - 1, &rate, metric).unwrap();
        assert_eq!(a.0 + b.0, divisor);
        assert_eq!(a.1, b.1);
    }
}

#[test]
fn overflow_and_mismatched_units_are_rejected_without_rounding() {
    let mut rate = rates()[0].clone();
    assert!(exact_fraction(10_u128.pow(38) - 1, &rate, Metric::AllocatedCpu).is_err());
    rate.credits_per_unit = Credits::from_pico(crate::models::credits::MAX_PICO).unwrap();
    assert!(
        exact_fraction(
            Metric::AllocatedCpu.units_per_hour() * 2,
            &rate,
            Metric::AllocatedCpu
        )
        .is_err()
    );
    rate = rates()[0].clone();
    rate.charge_unit = ChargeUnit::GibHour;
    assert!(exact_fraction(1, &rate, Metric::AllocatedCpu).is_err());
}

#[test]
fn one_basis_per_dimension_and_no_fourth_uptime_charge() {
    let tariff = tariff();
    let pin = pin(&tariff);
    let allocated = plan(&decode(&fixture("allocated")).unwrap(), &pin, Some(&tariff));
    let measured = plan(&decode(&fixture("measured")).unwrap(), &pin, Some(&tariff));
    assert_eq!(allocated.len() + measured.len(), 6);
    assert!(
        allocated
            .iter()
            .chain(&measured)
            .all(|p| p.charge_mode == ChargeMode::Shadow)
    );
    assert_eq!(
        allocated
            .iter()
            .filter(|p| matches!(p.disposition, Disposition::Candidate { .. }))
            .count(),
        1
    );
    assert_eq!(
        measured
            .iter()
            .filter(|p| matches!(p.disposition, Disposition::Candidate { .. }))
            .count(),
        2
    );
    assert_eq!(
        measured[1].disposition,
        Disposition::Candidate {
            numerator_pico: 9_007_199_254_740_993 * 1_000_000_000_000,
            divisor: 3_865_470_566_400_000
        }
    );
}

#[test]
fn incomplete_measured_evidence_never_falls_back_or_claims_zero() {
    let tariff = tariff();
    let pin = pin(&tariff);
    for quality in ["partial", "missing", "estimated"] {
        let mut value = fixture("measured");
        let fact = &mut value["facts"][0];
        fact["coverage"]["cpu"]["quality"] = json!(quality);
        fact["coverage"]["cpu"]["missing_reason"] = json!("late_samples");
        fact["coverage"]["cpu"]["covered_binding_ms"] =
            json!(if quality == "missing" { "0" } else { "60000" });
        if quality == "missing" {
            fact["quantities"][code(Metric::MeasuredCpu)] = json!(null);
        }
        let plans = plan(&decode(&value).unwrap(), &pin, Some(&tariff));
        assert_eq!(
            plans[0].disposition,
            Disposition::Pending(Pending::IncompleteEvidence)
        );
    }
}

#[test]
fn missing_price_and_unknown_legacy_effects_hold_candidate() {
    let tariff = tariff();
    let mut pin = pin(&tariff);
    let event = decode(&fixture("measured")).unwrap();
    assert_eq!(
        plan(&event, &pin, None)[0].disposition,
        Disposition::Pending(Pending::MissingTariff)
    );
    pin.legacy_coverage = None;
    assert_eq!(
        plan(&event, &pin, Some(&tariff))[0].disposition,
        Disposition::Pending(Pending::LegacyCoverageUnknown)
    );
    pin.legacy_coverage = Some(LegacyCoverage {
        covered_until: time() + Duration::minutes(1),
        manifest_digest: "a".repeat(64),
    });
    assert_eq!(
        plan(&event, &pin, Some(&tariff))[0].disposition,
        Disposition::NonBillable(NonBillable::LegacyCoverage)
    );
}

#[test]
fn immutable_nonbillable_classification_survives_corrections_and_new_rates() {
    let tariff = tariff();
    let mut pin = pin(&tariff);
    for (choice, reason) in [
        (Selection::Shadow, NonBillable::Shadow),
        (Selection::Free, NonBillable::Free),
        (Selection::Excluded, NonBillable::Excluded),
    ] {
        pin.dimensions = DimensionPolicy {
            cpu: choice,
            memory: choice,
            disk: choice,
        };
        for fixture_name in ["allocated", "correction", "measured", "measured-correction"] {
            assert!(
                plan(&decode(&fixture(fixture_name)).unwrap(), &pin, None)
                    .iter()
                    .all(|p| p.disposition == Disposition::NonBillable(reason.clone()))
            );
        }
    }
    let mut next = tariff.clone();
    next.id = uuid::Uuid::new_v4().to_string();
    next.version = 2;
    assert!(
        shadow_plan(
            &decode(&fixture("allocated")).unwrap(),
            "00000000-0000-4000-8000-000000000064",
            &pin,
            Some(&next)
        )
        .is_err()
    );
}

#[test]
fn utc_month_and_rate_edge_preserve_the_original_period_and_version() {
    let mut value = fixture("measured");
    let mut tariff = tariff();
    let mut pin = pin(&tariff);
    for stamp in ["2026-11-30T23:59:00Z", "2026-12-01T00:00:00Z"] {
        let at = DateTime::parse_from_rfc3339(stamp)
            .unwrap()
            .with_timezone(&Utc);
        let start = at.timestamp_millis();
        value["facts"][0]["bucket_start_ms"] = json!(start.to_string());
        value["facts"][0]["start_ms"] = json!(start.to_string());
        value["facts"][0]["end_ms"] = json!((start + 60_000).to_string());
        for key in ["observed_at_ms", "source_cutoff_ms", "produced_at_ms"] {
            value[key] = json!((start + 60_000).to_string());
        }
        let event = contract::decode_event(
            &serde_json::to_vec(&value).unwrap(),
            contract::Clock {
                now_ms: (start + 60_000) as u64,
                allowed_clock_skew_ms: 0,
            },
        )
        .unwrap();
        pin.minute = at;
        tariff.effective_from = at;
        assert_eq!(
            plan(&event, &pin, Some(&tariff))[0].billing_period,
            &stamp[..7]
        );
        tariff.effective_from = at + Duration::minutes(1);
        assert!(
            shadow_plan(
                &event,
                "00000000-0000-4000-8000-000000000064",
                &pin,
                Some(&tariff)
            )
            .is_err()
        );
    }
}

#[test]
fn mongo_records_keep_exact_credits_dates_and_reject_future_schema() {
    let tariff = tariff();
    let doc = bson::to_document(&tariff).unwrap();
    assert!(doc.get_datetime("effective_from").is_ok());
    let rate = doc.get_array("rates").unwrap()[0].as_document().unwrap();
    assert!(matches!(
        rate.get("credits_per_unit"),
        Some(bson::Bson::Decimal128(_))
    ));
    assert_eq!(bson::from_document::<UsageTariff>(doc).unwrap(), tariff);
    let mut pin = pin(&tariff);
    pin.schema_version = 2;
    assert!(
        shadow_plan(
            &decode(&fixture("allocated")).unwrap(),
            "00000000-0000-4000-8000-000000000064",
            &pin,
            Some(&tariff)
        )
        .is_err()
    );
    pin.schema_version = 1;
    pin.scope.sandbox_id = "other".into();
    assert!(
        shadow_plan(
            &decode(&fixture("allocated")).unwrap(),
            "00000000-0000-4000-8000-000000000064",
            &pin,
            Some(&tariff)
        )
        .is_err()
    );
}

#[test]
fn nullable_bindings_keep_observed_absence_at_zero_without_inventing_a_charge() {
    let tariff = tariff();
    let mut pin = pin(&tariff);
    pin.dimensions = DimensionPolicy {
        cpu: Selection::Allocated,
        memory: Selection::Allocated,
        disk: Selection::Allocated,
    };
    for (name, absent) in [
        ("allocated-compute-only", vec![Metric::AllocatedDisk]),
        (
            "allocated-retained-home",
            vec![Metric::AllocatedCpu, Metric::AllocatedMemory],
        ),
    ] {
        let event = decode(&fixture(name)).unwrap();
        let plans = plan(&event, &pin, Some(&tariff));
        for metric in absent {
            assert_eq!(
                plans
                    .iter()
                    .find(|p| p.metric == metric)
                    .unwrap()
                    .disposition,
                Disposition::Candidate {
                    numerator_pico: 0,
                    divisor: metric.units_per_hour()
                }
            );
        }
    }
}

#[test]
fn persisted_price_pins_cannot_claim_charge_activation() {
    let pin = pin(&tariff());
    let mut record = bson::to_document(&pin).unwrap();
    assert_eq!(record.get_str("charge_mode"), Ok("shadow"));
    assert_eq!(
        bson::from_document::<PricePin>(record.clone()).unwrap(),
        pin
    );
    record.insert("charge_mode", "activated");
    assert!(bson::from_document::<PricePin>(record.clone()).is_err());
    record.remove("charge_mode");
    assert!(bson::from_document::<PricePin>(record).is_err());
}
