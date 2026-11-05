use std::collections::HashSet;
use std::sync::OnceLock;

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use mongodb::bson::{self, Bson};
use regex::Regex;
use serde_json::{Value, json};

use crate::errors::CrmValidationIssue;
use crate::models::crm_object::{CrmCardinality, CrmField, CrmFieldKind, CrmSelectOption};

const CURRENCIES: &[&str] = &["USD", "EUR", "GBP", "SGD", "AUD", "CAD", "HKD", "CNY"];
const FILTER_OPS_TEXT: &[&str] = &["eq", "neq", "in", "contains", "is_set", "is_unset"];
const FILTER_OPS_ORDERED: &[&str] = &["eq", "neq", "gt", "gte", "lt", "lte", "is_set", "is_unset"];
const FILTER_OPS_BOOLEAN: &[&str] = &["eq", "is_set", "is_unset"];
const FILTER_OPS_ENUM: &[&str] = &["eq", "neq", "in", "nin", "is_set", "is_unset"];
const FILTER_OPS_RECORD: &[&str] = &["eq", "in", "is_set", "is_unset"];
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq)]
pub enum ValidatedValue {
    Clear,
    Stored(Bson),
    User(String),
    Record {
        target_object: String,
        single: bool,
        ids: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValidatedFilterValue {
    None,
    Scalar(Bson),
    List(Vec<Bson>),
    RecordIds(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasureUnit {
    Number,
    Currency,
}

fn field_key_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^[a-z][a-z0-9_]{0,47}$").expect("fixed CRM field regex"))
}

fn option_value_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^[a-z0-9_]{1,48}$").expect("fixed CRM option regex"))
}

pub fn valid_key(value: &str) -> bool {
    field_key_regex().is_match(value)
}

pub fn validate_definition(field: &CrmField, path: &str) -> Vec<CrmValidationIssue> {
    let mut errors = Vec::new();
    if !valid_key(&field.key) {
        issue(
            &mut errors,
            format!("{path}.key"),
            "must start with a letter and contain only lowercase letters, digits, and underscores (max 48 characters)",
        );
    }
    if field.label.trim().is_empty() || field.label.chars().count() > 200 {
        issue(
            &mut errors,
            format!("{path}.label"),
            "must contain 1 to 200 characters",
        );
    }
    match &field.kind {
        CrmFieldKind::Text { max_length } if !(1..=10_000).contains(max_length) => issue(
            &mut errors,
            format!("{path}.max_length"),
            "must be between 1 and 10000",
        ),
        CrmFieldKind::Currency { currency } if !CURRENCIES.contains(&currency.as_str()) => issue(
            &mut errors,
            format!("{path}.currency"),
            "must be one of USD, EUR, GBP, SGD, AUD, CAD, HKD, or CNY",
        ),
        CrmFieldKind::Select { options } => {
            validate_options(options, path, &mut errors);
            if field.required && !field.archived && !options.iter().any(|option| !option.archived) {
                issue(
                    &mut errors,
                    format!("{path}.options"),
                    "a required select must contain at least one live option",
                );
            }
        }
        CrmFieldKind::Record { target, .. } if !valid_key(target) => issue(
            &mut errors,
            format!("{path}.target"),
            "must be a valid object key",
        ),
        _ => {}
    }
    errors
}

fn validate_options(options: &[CrmSelectOption], path: &str, errors: &mut Vec<CrmValidationIssue>) {
    if options.len() > 100 {
        issue(
            errors,
            format!("{path}.options"),
            "must contain at most 100 options, including archived options",
        );
        return;
    }
    let mut seen = HashSet::new();
    for (index, option) in options.iter().enumerate() {
        let option_path = format!("{path}.options.{index}");
        if !option_value_regex().is_match(&option.value) {
            issue(
                errors,
                format!("{option_path}.value"),
                "must contain only lowercase letters, digits, and underscores (max 48 characters)",
            );
        } else if !seen.insert(option.value.as_str()) {
            issue(
                errors,
                format!("{option_path}.value"),
                "must be unique within the field",
            );
        }
        if option.label.trim().is_empty() || option.label.chars().count() > 200 {
            issue(
                errors,
                format!("{option_path}.label"),
                "must contain 1 to 200 characters",
            );
        }
        if option
            .color
            .as_ref()
            .is_some_and(|color| color.chars().count() > 64)
        {
            issue(
                errors,
                format!("{option_path}.color"),
                "must be at most 64 characters",
            );
        }
    }
}

pub fn validate_write_value(
    field: &CrmField,
    value: &Value,
    path: &str,
) -> Result<ValidatedValue, CrmValidationIssue> {
    validate_value(field, value, path, false)
}

fn validate_value(
    field: &CrmField,
    value: &Value,
    path: &str,
    allow_archived_option: bool,
) -> Result<ValidatedValue, CrmValidationIssue> {
    if value.is_null() {
        if field.required {
            return Err(error(path, "required fields cannot be cleared"));
        }
        return Ok(ValidatedValue::Clear);
    }
    if field.archived {
        return Err(error(path, "archived fields cannot receive new values"));
    }
    match &field.kind {
        CrmFieldKind::Text { max_length } => {
            let text = value
                .as_str()
                .ok_or_else(|| error(path, "must be a string"))?;
            if field.required && text.trim().is_empty() {
                return Err(error(path, "required text cannot be blank"));
            }
            if text.chars().count() > *max_length as usize {
                return Err(error(
                    path,
                    format!("must be at most {max_length} characters"),
                ));
            }
            Ok(ValidatedValue::Stored(Bson::String(text.to_owned())))
        }
        CrmFieldKind::Number { .. } => {
            let number = value
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| error(path, "must be a finite number"))?;
            Ok(ValidatedValue::Stored(Bson::Double(number)))
        }
        CrmFieldKind::Currency { currency } => Ok(ValidatedValue::Stored(Bson::Int64(
            currency_amount(value, currency, path)?,
        ))),
        CrmFieldKind::Boolean {} => value
            .as_bool()
            .map(|value| ValidatedValue::Stored(Bson::Boolean(value)))
            .ok_or_else(|| error(path, "must be a boolean")),
        CrmFieldKind::Date {} => {
            let date = value
                .as_str()
                .ok_or_else(|| error(path, "must be a YYYY-MM-DD string"))?;
            let bytes = date.as_bytes();
            let exact_width = bytes.len() == 10
                && bytes[..4].iter().all(u8::is_ascii_digit)
                && bytes[4] == b'-'
                && bytes[5..7].iter().all(u8::is_ascii_digit)
                && bytes[7] == b'-'
                && bytes[8..].iter().all(u8::is_ascii_digit);
            if !exact_width {
                return Err(error(
                    path,
                    "must be a valid calendar date in YYYY-MM-DD format",
                ));
            }
            let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|_| error(path, "must be a valid calendar date in YYYY-MM-DD format"))?;
            let canonical = parsed.format("%Y-%m-%d").to_string();
            if canonical != date {
                return Err(error(
                    path,
                    "must be a valid calendar date in YYYY-MM-DD format",
                ));
            }
            Ok(ValidatedValue::Stored(Bson::String(canonical)))
        }
        CrmFieldKind::Datetime {} => {
            let instant = value
                .as_str()
                .ok_or_else(|| error(path, "must be an RFC3339 timestamp"))?;
            let parsed = DateTime::parse_from_rfc3339(instant)
                .map_err(|_| error(path, "must be an RFC3339 timestamp"))?
                .with_timezone(&Utc);
            Ok(ValidatedValue::Stored(Bson::DateTime(
                bson::DateTime::from_chrono(parsed),
            )))
        }
        CrmFieldKind::Select { options } => {
            let selected = value
                .as_str()
                .ok_or_else(|| error(path, "must be an option value string"))?;
            let option = options
                .iter()
                .find(|option| option.value == selected)
                .ok_or_else(|| error(path, "must name an existing option"))?;
            if option.archived && !allow_archived_option {
                return Err(error(path, "archived options cannot receive new values"));
            }
            Ok(ValidatedValue::Stored(Bson::String(selected.to_owned())))
        }
        CrmFieldKind::Record {
            target,
            cardinality,
        } => {
            let ids = match cardinality {
                CrmCardinality::One => vec![
                    value
                        .as_str()
                        .ok_or_else(|| error(path, "must be a record id string"))?
                        .to_owned(),
                ],
                CrmCardinality::Many => value
                    .as_array()
                    .ok_or_else(|| error(path, "must be an array of record id strings"))?
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        value.as_str().map(str::to_owned).ok_or_else(|| {
                            error(format!("{path}.{index}"), "must be a record id string")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            if ids.iter().any(String::is_empty) {
                return Err(error(path, "record ids cannot be empty"));
            }
            if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
                return Err(error(path, "record ids must be unique"));
            }
            Ok(ValidatedValue::Record {
                target_object: target.clone(),
                single: matches!(cardinality, CrmCardinality::One),
                ids,
            })
        }
        CrmFieldKind::User {} => value
            .as_str()
            .filter(|value| !value.is_empty())
            .map(|value| ValidatedValue::User(value.to_owned()))
            .ok_or_else(|| error(path, "must be a person id string")),
    }
}

pub fn allowed_filter_operators(field: &CrmField) -> &'static [&'static str] {
    match field.kind {
        CrmFieldKind::Text { .. } => FILTER_OPS_TEXT,
        CrmFieldKind::Number { .. }
        | CrmFieldKind::Currency { .. }
        | CrmFieldKind::Date {}
        | CrmFieldKind::Datetime {} => FILTER_OPS_ORDERED,
        CrmFieldKind::Boolean {} => FILTER_OPS_BOOLEAN,
        CrmFieldKind::Select { .. } | CrmFieldKind::User {} => FILTER_OPS_ENUM,
        CrmFieldKind::Record { .. } => FILTER_OPS_RECORD,
    }
}

pub fn validate_filter_value(
    field: &CrmField,
    op: &str,
    value: Option<&Value>,
    path: &str,
) -> Result<ValidatedFilterValue, CrmValidationIssue> {
    if field.archived {
        return Err(error(path, "archived fields cannot be queried"));
    }
    if !allowed_filter_operators(field).contains(&op) {
        return Err(error(
            path,
            format!("operator '{op}' is not allowed for this field type"),
        ));
    }
    if matches!(op, "is_set" | "is_unset") {
        if value.is_some() {
            return Err(error(
                format!("{path}.value"),
                "must be omitted for this operator",
            ));
        }
        return Ok(ValidatedFilterValue::None);
    }
    let value =
        value.ok_or_else(|| error(format!("{path}.value"), "is required for this operator"))?;
    if op == "contains" {
        let text = value
            .as_str()
            .ok_or_else(|| error(format!("{path}.value"), "must be a string"))?;
        if text.chars().count() > 100 {
            return Err(error(
                format!("{path}.value"),
                "must be at most 100 characters",
            ));
        }
        return Ok(ValidatedFilterValue::Scalar(Bson::String(text.to_owned())));
    }
    if matches!(op, "in" | "nin") {
        let values = value
            .as_array()
            .ok_or_else(|| error(format!("{path}.value"), "must be an array"))?;
        if values.len() > 100 {
            return Err(error(
                format!("{path}.value"),
                "must contain at most 100 values",
            ));
        }
        if matches!(field.kind, CrmFieldKind::Record { .. }) {
            let ids = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    value
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            error(
                                format!("{path}.value.{index}"),
                                "must be a record id string",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(ValidatedFilterValue::RecordIds(ids));
        }
        let mut out = Vec::with_capacity(values.len());
        for (index, value) in values.iter().enumerate() {
            out.push(filter_scalar(
                field,
                value,
                &format!("{path}.value.{index}"),
            )?);
        }
        return Ok(ValidatedFilterValue::List(out));
    }
    if matches!(field.kind, CrmFieldKind::Record { .. }) {
        return value
            .as_str()
            .filter(|id| !id.is_empty())
            .map(|id| ValidatedFilterValue::RecordIds(vec![id.to_owned()]))
            .ok_or_else(|| error(format!("{path}.value"), "must be a record id string"));
    }
    Ok(ValidatedFilterValue::Scalar(filter_scalar(
        field,
        value,
        &format!("{path}.value"),
    )?))
}

fn filter_scalar(field: &CrmField, value: &Value, path: &str) -> Result<Bson, CrmValidationIssue> {
    match validate_value(field, value, path, true)? {
        ValidatedValue::Stored(value) => Ok(value),
        ValidatedValue::User(user) => Ok(Bson::String(user)),
        _ => Err(error(path, "must be a scalar value")),
    }
}

pub fn group_eligible(field: &CrmField) -> bool {
    matches!(
        field.kind,
        CrmFieldKind::Select { .. }
            | CrmFieldKind::Boolean {}
            | CrmFieldKind::User {}
            | CrmFieldKind::Record {
                cardinality: CrmCardinality::One,
                ..
            }
    )
}

pub fn measure_unit(field: &CrmField) -> Option<MeasureUnit> {
    match field.kind {
        CrmFieldKind::Number { .. } => Some(MeasureUnit::Number),
        CrmFieldKind::Currency { .. } => Some(MeasureUnit::Currency),
        _ => None,
    }
}

pub fn currency(field: &CrmField) -> Option<&str> {
    match &field.kind {
        CrmFieldKind::Currency { currency } => Some(currency),
        _ => None,
    }
}

pub fn render_api_value(field: &CrmField, value: &Bson) -> Value {
    match (&field.kind, value) {
        (
            CrmFieldKind::Text { .. }
            | CrmFieldKind::Select { .. }
            | CrmFieldKind::Date {}
            | CrmFieldKind::User {},
            Bson::String(value),
        ) => json!(value),
        (CrmFieldKind::Number { .. }, Bson::Double(value)) => json!(value),
        (CrmFieldKind::Number { .. }, Bson::Int32(value)) => json!(*value as f64),
        (CrmFieldKind::Number { .. }, Bson::Int64(value)) => json!(*value as f64),
        (CrmFieldKind::Currency { currency }, Bson::Int64(value)) => {
            json!({"amount_minor": value, "currency": currency})
        }
        (CrmFieldKind::Currency { currency }, Bson::Int32(value)) => {
            json!({"amount_minor": i64::from(*value), "currency": currency})
        }
        (CrmFieldKind::Boolean {}, Bson::Boolean(value)) => json!(value),
        (CrmFieldKind::Datetime {}, Bson::DateTime(value)) => json!(
            value
                .to_chrono()
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        ),
        _ => Value::Null,
    }
}

fn currency_amount(
    value: &Value,
    expected_currency: &str,
    path: &str,
) -> Result<i64, CrmValidationIssue> {
    if let Some(amount) = value.as_i64() {
        return validate_currency_bound(amount, path);
    }
    let object = value.as_object().ok_or_else(|| {
        error(
            path,
            "must be an integer minor-unit amount or a currency object",
        )
    })?;
    if object.len() != 2 || !object.contains_key("amount_minor") || !object.contains_key("currency")
    {
        return Err(error(
            path,
            "currency objects must contain only amount_minor and currency",
        ));
    }
    let currency = object
        .get("currency")
        .and_then(Value::as_str)
        .ok_or_else(|| error(format!("{path}.currency"), "must be a currency code string"))?;
    if currency != expected_currency {
        return Err(error(
            format!("{path}.currency"),
            format!("must be {expected_currency}"),
        ));
    }
    let amount = object
        .get("amount_minor")
        .and_then(Value::as_i64)
        .ok_or_else(|| error(format!("{path}.amount_minor"), "must be an integer"))?;
    validate_currency_bound(amount, &format!("{path}.amount_minor"))
}

fn validate_currency_bound(amount: i64, path: &str) -> Result<i64, CrmValidationIssue> {
    if amount.unsigned_abs() > MAX_SAFE_INTEGER as u64 {
        Err(error(
            path,
            "absolute value must be at most 9007199254740991",
        ))
    } else {
        Ok(amount)
    }
}

fn issue(
    errors: &mut Vec<CrmValidationIssue>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    errors.push(error(path, message));
}

fn error(path: impl Into<String>, message: impl Into<String>) -> CrmValidationIssue {
    CrmValidationIssue {
        path: path.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::crm_object::{CrmNumberFormat, CrmSelectOption};

    fn field(kind: CrmFieldKind) -> CrmField {
        CrmField {
            key: "field".into(),
            label: "Field".into(),
            required: false,
            archived: false,
            kind,
        }
    }

    #[test]
    fn every_stored_kind_round_trips_through_the_registry() {
        let cases = vec![
            (
                field(CrmFieldKind::Text { max_length: 10 }),
                json!("hello"),
                json!("hello"),
            ),
            (
                field(CrmFieldKind::Number {
                    format: CrmNumberFormat::Plain,
                }),
                json!(1.25),
                json!(1.25),
            ),
            (
                field(CrmFieldKind::Currency {
                    currency: "USD".into(),
                }),
                json!({"amount_minor": 125000, "currency": "USD"}),
                json!({"amount_minor": 125000, "currency": "USD"}),
            ),
            (field(CrmFieldKind::Boolean {}), json!(true), json!(true)),
            (
                field(CrmFieldKind::Date {}),
                json!("2026-11-04"),
                json!("2026-11-04"),
            ),
            (
                field(CrmFieldKind::Datetime {}),
                json!("2026-11-04T01:02:03Z"),
                json!("2026-11-04T01:02:03.000Z"),
            ),
            (
                field(CrmFieldKind::Select {
                    options: vec![CrmSelectOption {
                        value: "open".into(),
                        label: "Open".into(),
                        color: None,
                        archived: false,
                    }],
                }),
                json!("open"),
                json!("open"),
            ),
            (
                field(CrmFieldKind::User {}),
                json!("person"),
                json!("person"),
            ),
        ];
        for (field, input, expected) in cases {
            let stored = match validate_write_value(&field, &input, "values.field").unwrap() {
                ValidatedValue::Stored(value) => value,
                ValidatedValue::User(value) => Bson::String(value),
                other => panic!("unexpected value: {other:?}"),
            };
            assert_eq!(render_api_value(&field, &stored), expected);
        }
    }

    #[test]
    fn filter_operator_matrix_rejects_cross_kind_operators() {
        let cases = [
            (CrmFieldKind::Text { max_length: 10 }, "gt"),
            (
                CrmFieldKind::Number {
                    format: CrmNumberFormat::Plain,
                },
                "contains",
            ),
            (CrmFieldKind::Boolean {}, "neq"),
            (CrmFieldKind::Select { options: vec![] }, "gt"),
            (
                CrmFieldKind::Record {
                    target: "company".into(),
                    cardinality: CrmCardinality::One,
                },
                "neq",
            ),
        ];
        for (kind, op) in cases {
            assert!(validate_filter_value(&field(kind), op, Some(&json!(1)), "filters.0").is_err());
        }
    }

    #[test]
    fn archived_fields_options_and_required_nulls_are_rejected() {
        let mut archived = field(CrmFieldKind::Text { max_length: 10 });
        archived.archived = true;
        assert!(validate_write_value(&archived, &json!("x"), "values.field").is_err());

        let select = field(CrmFieldKind::Select {
            options: vec![CrmSelectOption {
                value: "old".into(),
                label: "Old".into(),
                color: None,
                archived: true,
            }],
        });
        assert!(validate_write_value(&select, &json!("old"), "values.field").is_err());

        let mut required = field(CrmFieldKind::Boolean {});
        required.required = true;
        assert!(validate_write_value(&required, &Value::Null, "values.field").is_err());
    }

    #[test]
    fn required_select_definition_needs_a_live_option() {
        let mut required = field(CrmFieldKind::Select {
            options: vec![CrmSelectOption {
                value: "old".into(),
                label: "Old".into(),
                color: None,
                archived: true,
            }],
        });
        required.required = true;
        assert_eq!(
            validate_definition(&required, "field"),
            vec![CrmValidationIssue {
                path: "field.options".into(),
                message: "a required select must contain at least one live option".into(),
            }]
        );
    }

    #[test]
    fn canonical_number_currency_and_date_rules_are_exact() {
        assert_eq!(
            validate_write_value(
                &field(CrmFieldKind::Number {
                    format: CrmNumberFormat::Plain,
                }),
                &json!(7),
                "values.score",
            )
            .unwrap(),
            ValidatedValue::Stored(Bson::Double(7.0))
        );
        assert_eq!(
            validate_write_value(
                &field(CrmFieldKind::Currency {
                    currency: "USD".into(),
                }),
                &json!(9_007_199_254_740_991_i64),
                "values.amount",
            )
            .unwrap(),
            ValidatedValue::Stored(Bson::Int64(9_007_199_254_740_991))
        );
        let wrong_currency = validate_write_value(
            &field(CrmFieldKind::Currency {
                currency: "USD".into(),
            }),
            &json!({"amount_minor": 1, "currency": "EUR"}),
            "values.amount",
        )
        .unwrap_err();
        assert_eq!(wrong_currency.path, "values.amount.currency");
        assert!(
            validate_write_value(
                &field(CrmFieldKind::Currency {
                    currency: "USD".into(),
                }),
                &json!(9_007_199_254_740_992_i64),
                "values.amount",
            )
            .is_err()
        );
        assert!(
            validate_write_value(
                &field(CrmFieldKind::Date {}),
                &json!("2026-02-29"),
                "values.date",
            )
            .is_err()
        );
        for noncanonical in ["2026-2-01", "2026-02-1", " 2026-02-01"] {
            assert!(
                validate_write_value(
                    &field(CrmFieldKind::Date {}),
                    &json!(noncanonical),
                    "values.date",
                )
                .is_err(),
                "accepted noncanonical date {noncanonical}"
            );
        }
        assert_eq!(
            validate_write_value(
                &field(CrmFieldKind::Date {}),
                &json!("2028-02-29"),
                "values.date",
            )
            .unwrap(),
            ValidatedValue::Stored(Bson::String("2028-02-29".into()))
        );
    }
}
