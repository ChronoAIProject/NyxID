//! Plain per-metric current usage. Unknown partitioning is a finding, never a total.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::models::credits::Credits;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::services::billing) enum Finding {
    UnsupportedShape,
    InvalidPeriod,
    InvalidUnits,
    DuplicateMetric,
    MissingMetric,
}

#[derive(Debug)]
pub(in crate::services::billing) struct CurrentUsage {
    pub start_second: i64,
    pub end_second_exclusive: i64,
    pub quantities_micros: BTreeMap<String, i64>,
}

impl CurrentUsage {
    pub fn metric(&self, code: &str) -> Result<i64, Finding> {
        self.quantities_micros
            .get(code)
            .copied()
            .ok_or(Finding::MissingMetric)
    }
}

pub(in crate::services::billing) fn parse(raw: &Value) -> Result<CurrentUsage, Finding> {
    let usage = raw
        .get("customer_usage")
        .and_then(Value::as_object)
        .ok_or(Finding::UnsupportedShape)?;
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
        .ok_or(Finding::UnsupportedShape)?;
    let mut quantities_micros = BTreeMap::new();
    for charge in charges {
        let row = charge.as_object().ok_or(Finding::UnsupportedShape)?;
        // Only the unpartitioned repository fixture is established offline.
        // Accept scalar metadata, but no unknown arrays or nested projections.
        for (key, value) in row {
            if !matches!(key.as_str(), "billable_metric" | "charge")
                && (value.is_array() || value.is_object())
            {
                return Err(Finding::UnsupportedShape);
            }
        }
        for object in [charge, &charge["charge"], &charge["billable_metric"]] {
            if let Some(fields) = object.as_object() {
                for key in [
                    "groups",
                    "grouped_usage",
                    "grouped_by",
                    "filters",
                    "filters_usage",
                    "group",
                    "filter",
                ] {
                    if fields.get(key).is_some_and(|v| !v.is_null()) {
                        return Err(Finding::UnsupportedShape);
                    }
                }
            }
        }
        if charge
            .pointer("/billable_metric/aggregation_type")
            .is_some_and(|v| v.as_str() != Some("sum_agg"))
        {
            return Err(Finding::UnsupportedShape);
        }
        let code = charge
            .pointer("/billable_metric/code")
            .and_then(Value::as_str)
            .filter(|code| !code.trim().is_empty())
            .ok_or(Finding::MissingMetric)?;
        let units = row.get("units").ok_or(Finding::InvalidUnits)?;
        let text = match units {
            Value::String(text) => text.clone(),
            Value::Number(number) if number.is_i64() || number.is_u64() => number.to_string(),
            _ => return Err(Finding::InvalidUnits),
        };
        let quantity: Credits = text.parse().map_err(|_| Finding::InvalidUnits)?;
        if quantity < Credits::ZERO || quantity.pico() % 1_000_000 != 0 {
            return Err(Finding::InvalidUnits);
        }
        let micros = quantity
            .legacy_micros()
            .map_err(|_| Finding::InvalidUnits)?;
        if quantities_micros.insert(code.into(), micros).is_some() {
            return Err(Finding::DuplicateMetric);
        }
    }
    Ok(CurrentUsage {
        start_second,
        end_second_exclusive,
        quantities_micros,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({"customer_usage":{
            "from_datetime":"2026-10-01T00:00:00Z", "to_datetime":"2026-10-31T23:59:59Z",
            "charges_usage":[{"billable_metric":{"code":"platform_requests","name":"Requests"},
                "charge":{"invoice_display_name":"Usage"}, "units":"1000000001.250000", "amount_cents":1000}]
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
    fn usage_capture_parser_reports_unsupported_or_invalid_shapes() {
        for key in [
            "groups",
            "filters",
            "grouped_usage",
            "grouped_by",
            "filters_usage",
        ] {
            let mut raw = fixture();
            raw["customer_usage"]["charges_usage"][0][key] = json!([]);
            assert_eq!(parse(&raw).unwrap_err(), Finding::UnsupportedShape);
        }
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
}
