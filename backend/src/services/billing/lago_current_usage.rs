//! Plain per-metric current usage. Provider breakdowns are checked, never used as totals.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::models::credits::Credits;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::services::billing) enum Finding {
    MissingCustomerUsage,
    MissingChargesUsage,
    ChargeNotObject,
    TooManyCharges,
    BreakdownMismatch,
    ResponseTooLarge,
    InvalidPeriod,
    InvalidUnits,
    DuplicateMetric,
    MissingMetric,
    SubscriptionUnavailable,
    RequestRejected,
}

#[derive(Clone, Debug)]
pub(in crate::services::billing) struct CurrentUsage {
    pub start_second: i64,
    pub end_second_exclusive: i64,
    pub quantities_micros: BTreeMap<String, i64>,
    pub non_sum_metrics: BTreeMap<String, String>,
}

#[cfg(test)]
impl CurrentUsage {
    pub fn metric(&self, code: &str) -> Result<i64, Finding> {
        self.quantities_micros
            .get(code)
            .copied()
            .ok_or(Finding::MissingMetric)
    }
}

pub(in crate::services::billing) fn parse(raw: &Value) -> Result<CurrentUsage, Finding> {
    if raw.get("usage_comparison_finding").and_then(Value::as_str) == Some("response_too_large") {
        return Err(Finding::ResponseTooLarge);
    }
    match raw.get("usage_comparison_finding").and_then(Value::as_str) {
        Some("subscription_unavailable") => return Err(Finding::SubscriptionUnavailable),
        Some("provider_request_rejected") => return Err(Finding::RequestRejected),
        _ => {}
    }
    let usage = raw
        .get("customer_usage")
        .and_then(Value::as_object)
        .ok_or(Finding::MissingCustomerUsage)?;
    let second = |key: &str| -> Result<i64, Finding> {
        let value = usage
            .get(key)
            .and_then(Value::as_str)
            .ok_or(Finding::InvalidPeriod)?;
        let at: DateTime<Utc> = DateTime::parse_from_rfc3339(value)
            .map_err(|_| Finding::InvalidPeriod)?
            .with_timezone(&Utc);
        if at.timestamp_subsec_nanos() != 0 {
            return Err(Finding::InvalidPeriod);
        }
        Ok(at.timestamp())
    };
    let start_second = second("from_datetime")?;
    let last_second = second("to_datetime")?;
    if last_second < start_second {
        return Err(Finding::InvalidPeriod);
    }
    let end_second_exclusive = last_second.checked_add(1).ok_or(Finding::InvalidPeriod)?;
    let charges = usage
        .get("charges_usage")
        .and_then(Value::as_array)
        .ok_or(Finding::MissingChargesUsage)?;
    if charges.len() > 256 {
        return Err(Finding::TooManyCharges);
    }
    let mut quantities_micros = BTreeMap::new();
    let mut non_sum_metrics = BTreeMap::new();
    for charge in charges {
        let row = charge.as_object().ok_or(Finding::ChargeNotObject)?;
        let code = charge
            .pointer("/billable_metric/code")
            .and_then(Value::as_str)
            .filter(|code| !code.trim().is_empty() && code.len() <= 256)
            .ok_or(Finding::MissingMetric)?;
        if quantities_micros.contains_key(code) || non_sum_metrics.contains_key(code) {
            return Err(Finding::DuplicateMetric);
        }
        let aggregation = charge.pointer("/billable_metric/aggregation_type");
        if aggregation.and_then(Value::as_str) != Some("sum_agg") {
            non_sum_metrics.insert(code.into(), aggregation_name(aggregation));
            continue;
        }
        let units = row.get("units").ok_or(Finding::InvalidUnits)?;
        let micros = self::units(units)?;
        check_breakdown(row.get("filters"), micros)?;
        check_breakdown(row.get("grouped_usage"), micros)?;
        quantities_micros.insert(code.into(), micros);
    }
    Ok(CurrentUsage {
        start_second,
        end_second_exclusive,
        quantities_micros,
        non_sum_metrics,
    })
}

fn aggregation_name(value: Option<&Value>) -> String {
    let Some(name) = value.and_then(Value::as_str) else {
        return "other".into();
    };
    if !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        name.into()
    } else {
        "other".into()
    }
}

fn check_breakdown(value: Option<&Value>, total: i64) -> Result<(), Finding> {
    let entries = match value {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Array(entries)) if entries.is_empty() => return Ok(()),
        Some(Value::Array(entries)) => entries,
        Some(_) => return Err(Finding::BreakdownMismatch),
    };
    let sum = entries.iter().try_fold(0_i64, |sum, entry| {
        let units = entry
            .as_object()
            .and_then(|entry| entry.get("units"))
            .ok_or(Finding::BreakdownMismatch)
            .and_then(|units| self::units(units).map_err(|_| Finding::BreakdownMismatch))?;
        sum.checked_add(units).ok_or(Finding::BreakdownMismatch)
    })?;
    (sum == total)
        .then_some(())
        .ok_or(Finding::BreakdownMismatch)
}

pub(in crate::services::billing) fn units(value: &Value) -> Result<i64, Finding> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) if number.is_i64() || number.is_u64() => number.to_string(),
        _ => return Err(Finding::InvalidUnits),
    };
    let quantity: Credits = text.parse().map_err(|_| Finding::InvalidUnits)?;
    if quantity < Credits::ZERO || quantity.pico() % 1_000_000 != 0 {
        return Err(Finding::InvalidUnits);
    }
    quantity.legacy_micros().map_err(|_| Finding::InvalidUnits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({"customer_usage":{
            "from_datetime":"2026-10-01T00:00:00Z", "to_datetime":"2026-10-31T23:59:59Z",
            "charges_usage":[{"billable_metric":{"code":"platform_requests","name":"Requests", "aggregation_type":"sum_agg"},
                "charge":{"lago_id":"fixture-charge","code":"requests","charge_model":"standard","invoice_display_name":"Usage"},
                "units":"1000000001.250000", "events_count":3, "amount_cents":1000, "amount_currency":"USD",
                "filters":[], "grouped_usage":[]}]
        }})
    }

    #[test]
    fn current_usage_decimal_quantity_supports_fractional_and_large_units() {
        let parsed = parse(&fixture()).unwrap();
        assert_eq!(
            parsed.metric("platform_requests"),
            Ok(1_000_000_001_250_000)
        );
        assert_eq!(parsed.metric("missing"), Err(Finding::MissingMetric));
        assert_eq!(
            parsed.end_second_exclusive - parsed.start_second,
            31 * 86400
        );
    }

    #[test]
    fn usage_comparison_parser_accepts_real_provider_shape() {
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"][0]["filters"] = json!([]);
        raw["customer_usage"]["charges_usage"][0]["grouped_usage"] = json!([]);
        raw["customer_usage"]["charges_usage"][0]["presentation_breakdowns"] =
            json!([{"units":"999"}]);
        raw["customer_usage"]["charges_usage"][0]["pricing_unit_details"] =
            json!({"unit":"request"});
        raw["customer_usage"]["charges_usage"][0]["total_aggregated_units"] = json!("999");
        raw["customer_usage"]["charges_usage"][0]["future_metadata"] = json!({"new_array":[1, 2]});
        assert_eq!(
            parse(&raw).unwrap().metric("platform_requests"),
            Ok(1_000_000_001_250_000)
        );

        assert_eq!(
            parse(&fixture()).unwrap().metric("platform_requests"),
            Ok(1_000_000_001_250_000)
        );
    }

    #[test]
    fn usage_comparison_parser_checks_breakdowns_without_double_counting() {
        let mut filters = fixture();
        filters["customer_usage"]["charges_usage"][0]["units"] = json!("3");
        filters["customer_usage"]["charges_usage"][0]["filters"] = json!([
            {"units":"1", "values":{"region":"west"}}, {"units":2, "values":{}}
        ]);
        assert_eq!(
            parse(&filters).unwrap().metric("platform_requests"),
            Ok(3_000_000)
        );

        let mut grouped = filters.clone();
        grouped["customer_usage"]["charges_usage"][0]["grouped_usage"] = json!([
            {"units":"1", "grouped_by":{"region":"west"}, "filters":[{"units":"1"}]},
            {"units":2, "grouped_by":{"region":"east"}, "filters":[{"units":2}]}
        ]);
        assert_eq!(
            parse(&grouped).unwrap().metric("platform_requests"),
            Ok(3_000_000)
        );

        for key in ["filters", "grouped_usage"] {
            for entries in [
                json!([{"units":"9"}]),
                json!([{"units":"invalid"}]),
                json!([{"units":"0.0000001"}]),
                json!([{"units":-1}]),
                json!([{}]),
                json!([null]),
                json!({"units":"3"}),
                json!("invalid"),
                json!([{"units":"9223372036854.775807"}, {"units":"0.000001"}]),
            ] {
                let mut mismatch = grouped.clone();
                mismatch["customer_usage"]["charges_usage"][0][key] = entries;
                assert_eq!(parse(&mismatch).unwrap_err(), Finding::BreakdownMismatch);
            }
        }
    }

    #[test]
    fn usage_comparison_parser_reports_specific_findings() {
        let mut raw = fixture();
        raw["customer_usage"] = Value::Null;
        assert_eq!(parse(&raw).unwrap_err(), Finding::MissingCustomerUsage);
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"] = Value::Null;
        assert_eq!(parse(&raw).unwrap_err(), Finding::MissingChargesUsage);
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"][0] = json!("charge");
        assert_eq!(parse(&raw).unwrap_err(), Finding::ChargeNotObject);
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"] = Value::Array(
            (0..257)
                .map(|_| fixture()["customer_usage"]["charges_usage"][0].clone())
                .collect(),
        );
        assert_eq!(parse(&raw).unwrap_err(), Finding::TooManyCharges);
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"][0]["billable_metric"]["aggregation_type"] =
            json!("count_agg");
        assert_eq!(
            parse(&raw).unwrap().non_sum_metrics["platform_requests"],
            "count_agg"
        );
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"][0]["billable_metric"]["aggregation_type"] =
            json!("COUNT_AGG");
        assert_eq!(
            parse(&raw).unwrap().non_sum_metrics["platform_requests"],
            "other"
        );
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"][0]["billable_metric"]["code"] = json!(" ");
        assert_eq!(parse(&raw).unwrap_err(), Finding::MissingMetric);
        for units in [
            json!("-1"),
            json!("0.0000001"),
            json!("bad"),
            json!(1.25),
            Value::Null,
        ] {
            let mut raw = fixture();
            raw["customer_usage"]["charges_usage"][0]["units"] = units;
            assert_eq!(parse(&raw).unwrap_err(), Finding::InvalidUnits);
        }
        let mut raw = fixture();
        let duplicate = raw["customer_usage"]["charges_usage"][0].clone();
        raw["customer_usage"]["charges_usage"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert_eq!(parse(&raw).unwrap_err(), Finding::DuplicateMetric);
        let mut raw = fixture();
        raw["customer_usage"]["from_datetime"] = json!("2026-11-01T00:00:00Z");
        assert_eq!(parse(&raw).unwrap_err(), Finding::InvalidPeriod);
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"][0]["billable_metric"] = Value::Null;
        assert_eq!(parse(&raw).unwrap_err(), Finding::MissingMetric);
    }

    #[test]
    fn usage_comparison_parser_accepts_contract_units() {
        for (units, micros) in [
            (json!("12.0"), 12_000_000),
            (json!("0.0"), 0),
            (json!("0.000001"), 1),
            (json!(12), 12_000_000),
        ] {
            let mut raw = fixture();
            raw["customer_usage"]["charges_usage"][0]["units"] = units;
            assert_eq!(parse(&raw).unwrap().metric("platform_requests"), Ok(micros));
        }
    }

    #[test]
    fn usage_comparison_parser_bounds_aggregation_names() {
        for (value, name) in [
            (json!("count_agg"), "count_agg".to_owned()),
            (json!("a".repeat(32)), "a".repeat(32)),
            (json!("a".repeat(33)), "other".into()),
            (json!("COUNT_AGG"), "other".into()),
            (json!("sum_agg2"), "other".into()),
            (json!("count\nagg"), "other".into()),
            (json!("计数"), "other".into()),
            (json!(""), "other".into()),
            (Value::Null, "other".into()),
            (json!({"kind":"count_agg"}), "other".into()),
        ] {
            let mut raw = fixture();
            raw["customer_usage"]["charges_usage"][0]["billable_metric"]["aggregation_type"] =
                value;
            let parsed = parse(&raw).unwrap();
            assert!(parsed.quantities_micros.is_empty());
            assert_eq!(parsed.non_sum_metrics["platform_requests"], name);
        }
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"][0]["billable_metric"]
            .as_object_mut()
            .unwrap()
            .remove("aggregation_type");
        assert_eq!(
            parse(&raw).unwrap().non_sum_metrics["platform_requests"],
            "other"
        );
    }
    #[test]
    fn usage_comparison_parser_accepts_v1_50_0_serializer_fixture() {
        // lago-api v1.50.0: customers/usage_serializer.rb and charge_usage_serializer.rb.
        let raw: Value =
            serde_json::from_str(include_str!("fixtures/lago_current_usage_v1_50_0.json")).unwrap();
        let parsed = parse(&raw).unwrap();
        assert_eq!(
            parsed.quantities_micros,
            BTreeMap::from([
                ("filtered_requests".into(), 3_000_000),
                ("grouped_requests".into(), 10_000_000),
            ])
        );
        assert!(parsed.non_sum_metrics.is_empty());
        assert_eq!(
            parsed.end_second_exclusive - parsed.start_second,
            30 * 86400
        );
    }

    #[test]
    fn usage_comparison_parser_keeps_structural_checks_for_non_sum_metrics() {
        let mut raw = fixture();
        raw["customer_usage"]["charges_usage"][0]["billable_metric"]["aggregation_type"] =
            json!("count_agg");
        raw["customer_usage"]["charges_usage"][0]["filters"] = json!([{"units":"invalid"}]);
        let parsed = parse(&raw).unwrap();
        assert!(parsed.quantities_micros.is_empty());
        assert_eq!(parsed.non_sum_metrics["platform_requests"], "count_agg");
        let mut invalid_units = raw.clone();
        invalid_units["customer_usage"]["charges_usage"][0]["units"] = json!("invalid");
        assert_eq!(
            parse(&invalid_units).unwrap().non_sum_metrics["platform_requests"],
            "count_agg"
        );
        for aggregation in ["sum_agg", "count_agg"] {
            let mut duplicate = raw.clone();
            let mut charge = fixture()["customer_usage"]["charges_usage"][0].clone();
            charge["billable_metric"]["aggregation_type"] = json!(aggregation);
            duplicate["customer_usage"]["charges_usage"]
                .as_array_mut()
                .unwrap()
                .push(charge);
            assert_eq!(parse(&duplicate).unwrap_err(), Finding::DuplicateMetric);
        }
    }

    #[test]
    fn usage_comparison_followup_parser_skips_non_sum_units() {
        for aggregation in ["count_agg", "weighted_sum_agg"] {
            for units in [json!("invalid"), json!("0.0000001"), Value::Null, json!([])] {
                let mut raw = fixture();
                let row = &mut raw["customer_usage"]["charges_usage"][0];
                row["billable_metric"]["aggregation_type"] = json!(aggregation);
                row["units"] = units;
                row["filters"] = json!([{"units":"invalid"}]);
                row["grouped_usage"] = json!("invalid");
                let parsed = parse(&raw).unwrap();
                assert!(parsed.quantities_micros.is_empty());
                assert_eq!(parsed.non_sum_metrics["platform_requests"], aggregation);
                raw["customer_usage"]["charges_usage"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("units");
                assert_eq!(
                    parse(&raw).unwrap().non_sum_metrics["platform_requests"],
                    aggregation
                );
                let duplicate = raw["customer_usage"]["charges_usage"][0].clone();
                raw["customer_usage"]["charges_usage"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
                assert_eq!(parse(&raw).unwrap_err(), Finding::DuplicateMetric);
            }
        }
    }
}
