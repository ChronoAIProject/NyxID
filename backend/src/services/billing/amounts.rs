//! Exact fixed-point unit rates and checked credit multiplication.
pub const PRICE_FRACTIONAL_DIGITS: usize = 12;
pub const PICO_PER_CREDIT: i128 = 1_000_000_000_000;
pub const PICO_PER_MICRO: i128 = 1_000_000;
pub const MAX_PRICE_PICO: i64 = 1_000_000_000_000_000_000;

pub fn rate_pico(precise: Option<i64>, legacy_micros: i64) -> i128 {
    precise
        .map(|value| i128::from(value.max(0)))
        .unwrap_or_else(|| i128::from(legacy_micros.max(0)) * PICO_PER_MICRO)
}

pub fn decimal_to_pico(raw: &str) -> Option<i64> {
    let value = raw.trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() > PRICE_FRACTIONAL_DIGITS
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let fraction: i64 = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i64>()
            .ok()?
            .checked_mul(10_i64.pow((PRICE_FRACTIONAL_DIGITS - fraction.len()) as u32))?
    };
    whole
        .checked_mul(PICO_PER_CREDIT as i64)?
        .checked_add(fraction)
}

pub fn format_pico(pico: i64) -> String {
    let whole = i128::from(pico) / PICO_PER_CREDIT;
    let fraction = i128::from(pico) % PICO_PER_CREDIT;
    if fraction == 0 {
        return whole.to_string();
    }
    format!("{whole}.{fraction:012}")
        .trim_end_matches('0')
        .to_string()
}

/// Checked exact accounting; legacy display helpers must not size movements.
pub fn cost(
    rate: i128,
    quantity: i64,
) -> Result<crate::models::credits::Credits, crate::models::credits::CreditsError> {
    crate::models::credits::Credits::from_pico(rate)?.checked_mul(quantity.max(0))
}

/// Exact keys denote credits; legacy keys denote microcredits regardless of
/// BSON numeric type. Normalize before summing so mixed buckets retain every pico.
pub fn credit_expr(value: impl Into<mongodb::bson::Bson>) -> mongodb::bson::Bson {
    let value = value.into();
    if let mongodb::bson::Bson::String(path) = &value {
        let (prefix, name) = path
            .rsplit_once('.')
            .unwrap_or(("", path.trim_start_matches('$')));
        let legacy = match name {
            "legacy_grant_cost" => Some("legacy_grant".to_string()),
            "amount" | "total_charge" | "allowance_funded" | "grant_funded" | "wallet_funded"
            | "gross_cost" | "wallet_cost" | "grant_cost" | "allowance_cost"
            | "expired_credits" => Some(format!("{name}_micros")),
            _ => None,
        };
        if let Some(legacy) = legacy {
            let old_path = if prefix.is_empty() {
                format!("${legacy}")
            } else {
                format!("{prefix}.{legacy}")
            };
            // The common Decimal128 path needs no type conversion. Evaluate
            // presence only in the lazy fallback: an explicit new null still
            // takes precedence over an older value under the legacy key.
            return mongodb::bson::doc! { "$ifNull": [path, { "$cond": [
                { "$eq": [{ "$type": path }, "missing"] },
                legacy_credit_expr(mongodb::bson::doc! { "$ifNull": [old_path, 0_i64] }.into()),
                null,
            ] }] }
            .into();
        }
    }
    legacy_credit_expr(mongodb::bson::doc! { "$ifNull": [value, 0_i64] }.into())
}

/// Every numeric value under a legacy money key is in microcredits, including
/// released Decimal128 query mirrors. Callers choose their missing/error policy.
pub fn legacy_credit_expr(value: mongodb::bson::Bson) -> mongodb::bson::Bson {
    mongodb::bson::doc! { "$divide": [{ "$toDecimal": value }, 1_000_000_i64] }.into()
}

/// A present exact field, including null, supersedes the legacy funding field.
pub fn funding_cost_present() -> mongodb::bson::Bson {
    mongodb::bson::doc! { "$ne": [{ "$ifNull": ["$funding.total_charge", { "$cond": [
        { "$eq": [{ "$type": "$funding.total_charge" }, "missing"] },
        { "$ifNull": ["$funding.total_charge_micros", null] },
        null,
    ] }] }, null] }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_precise_rates_and_checked_cost() {
        let pico = decimal_to_pico("0.000000250001").unwrap();
        assert_eq!(format_pico(pico), "0.000000250001");
        assert_eq!(
            cost(i128::from(pico), 4_000_000).unwrap().to_string(),
            "1.000004"
        );
        // The former one-credit ceiling was the sub-micro overcharge bug.
        assert_eq!(
            cost(i128::from(pico), 1).unwrap().to_string(),
            "0.000000250001"
        );
        assert_eq!(cost(rate_pico(None, 125_000), 8).unwrap().to_string(), "1");
        assert_eq!(decimal_to_pico("1000000"), Some(MAX_PRICE_PICO));
        assert!(decimal_to_pico("0.0000000000001").is_none());
        // Saturation silently hid accounting overflow; exact arithmetic must fail.
        assert!(cost(rate_pico(None, i64::MAX), i64::MAX).is_err());
    }

    #[tokio::test]
    async fn legacy_decimal_micros_use_micro_scale_and_preserve_exact_precedence() {
        use crate::models::credits::{Credits, SCALE};
        use futures::TryStreamExt;
        use mongodb::bson::{Bson, Decimal128, Document, doc};

        let db = crate::test_utils::connect_test_database("legacy_credit_scale")
            .await
            .unwrap();
        let collection = db.collection::<Document>("amounts");
        let pico = Credits::from_pico(1).unwrap();
        let mut documents = Vec::new();
        for (kind, legacy) in [
            (
                "decimal",
                Bson::Decimal128("1234567".parse::<Decimal128>().unwrap()),
            ),
            ("int64", Bson::Int64(1_234_567)),
            ("int32", Bson::Int32(1_234_567)),
            ("double", Bson::Double(1_234_567.0)),
        ] {
            for (presence, exact) in [
                ("legacy", None),
                ("exact", Some(Bson::from(pico))),
                ("null", Some(Bson::Null)),
            ] {
                let mut money = doc! {
                    "total_charge_micros": legacy.clone(),
                    "amount_micros": legacy.clone(),
                    "gross_cost_micros": legacy.clone(),
                };
                if let Some(exact) = exact {
                    for field in ["total_charge", "amount", "gross_cost"] {
                        money.insert(field, exact.clone());
                    }
                }
                documents.push(doc! {
                    "_id": format!("{kind}-{presence}"),
                    "funding": money.clone(),
                    "allocations": [money.clone()],
                    "query_costs": money,
                });
            }
        }
        collection.insert_many(documents).await.unwrap();
        let rows: Vec<Document> = collection
            .aggregate(vec![doc! { "$project": {
                "funding": credit_expr("$funding.total_charge"),
                "allocations": { "$map": {
                    "input": "$allocations", "as": "allocation",
                    "in": credit_expr("$$allocation.amount"),
                } },
                "cost": credit_expr("$query_costs.gross_cost"),
                // Fold increments pass a legacy $getField expression directly.
                "fold_legacy": credit_expr(doc! { "$getField": {
                    "field": "gross_cost_micros", "input": "$query_costs",
                } }),
            } }])
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 12);
        for row in rows {
            let id = row.get_str("_id").unwrap();
            for actual in [
                &row["funding"],
                &row.get_array("allocations").unwrap()[0],
                &row["cost"],
            ] {
                if id.ends_with("-null") {
                    assert_eq!(actual, &Bson::Null, "{id}");
                } else {
                    let expected = if id.ends_with("-exact") {
                        pico
                    } else {
                        Credits::from_micros(1_234_567)
                    };
                    assert_eq!(
                        Credits::from_bson(actual.clone(), SCALE).unwrap(),
                        expected,
                        "{id}"
                    );
                }
            }
            assert_eq!(
                Credits::from_bson(row["fold_legacy"].clone(), SCALE).unwrap(),
                Credits::from_micros(1_234_567),
                "{id}",
            );
        }
        db.drop().await.unwrap();
    }
}
