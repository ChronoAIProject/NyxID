use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Bson, Document, doc},
};
use regex::escape;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::errors::{AppError, AppResult, CrmValidationIssue};
use crate::models::{
    crm_object::{CrmField, CrmFieldKind, CrmObject},
    crm_record::{COLLECTION_NAME as RECORDS, CrmRecord},
};

use super::records::{RecordResponse, hydrate_record};
use super::{fields, objects, objects::ObjectApi, validation_result};

const MAX_FILTERS: usize = 20;
const MAX_PAGE_SIZE: u32 = 200;
const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_GROUPS: usize = 50;
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CrmTableQuery {
    pub object: String,
    pub select: Option<Vec<String>>,
    #[serde(default)]
    pub filters: Vec<CrmFilter>,
    pub sort: Option<CrmSort>,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    pub cursor: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}

fn default_page_size() -> u32 {
    DEFAULT_PAGE_SIZE
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CrmFilter {
    pub field: String,
    pub op: String,
    pub value: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CrmSort {
    pub field: String,
    pub direction: SortDirection,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CrmAggregateQuery {
    pub object: String,
    #[serde(default)]
    pub filters: Vec<CrmFilter>,
    pub group_by: Option<String>,
    pub measure: AggregateMeasure,
    pub order: AggregateOrder,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AggregateMeasure {
    Count,
    Sum { field: String },
    Avg { field: String },
    Min { field: String },
    Max { field: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AggregateOrder {
    OptionOrder,
    ValueDesc,
    LabelAsc,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TableResult {
    pub object: ObjectApi,
    pub rows: Vec<RecordResponse>,
    pub next_cursor: Option<String>,
    pub total_count: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AggregateResult {
    pub points: Vec<AggregatePoint>,
    pub unit: AggregateUnit,
    pub total: Value,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AggregatePoint {
    pub key: Value,
    pub label: String,
    pub value: Value,
    pub archived: bool,
    #[serde(skip, default)]
    count: u64,
    #[serde(skip, default)]
    measure_count: u64,
    #[serde(skip, default)]
    measure_sum: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AggregateUnit {
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

pub async fn table(db: &Database, owner_id: &str, query: &CrmTableQuery) -> AppResult<TableResult> {
    let validated = validate_table(db, owner_id, query).await?;
    let offset = decode_cursor(query.cursor.as_deref())?;
    let total_count = db
        .collection::<CrmRecord>(RECORDS)
        .count_documents(validated.filter.clone())
        .max_time(QUERY_TIMEOUT)
        .await?;
    let mut sort = Document::new();
    if let Some(requested) = &query.sort {
        sort.insert(
            format!("values.{}", requested.field),
            match requested.direction {
                SortDirection::Asc => 1,
                SortDirection::Desc => -1,
            },
        );
    }
    sort.insert("_id", 1);
    let mut records: Vec<CrmRecord> = db
        .collection::<CrmRecord>(RECORDS)
        .find(validated.filter)
        .sort(sort)
        .skip(offset)
        .limit(i64::from(query.page_size) + 1)
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_collect()
        .await?;
    let has_more = records.len() > query.page_size as usize;
    records.truncate(query.page_size as usize);
    let selected: Option<HashSet<&str>> = query
        .select
        .as_ref()
        .map(|fields| fields.iter().map(String::as_str).collect());
    let mut session = db.client().start_session().await?;
    let mut rows = Vec::with_capacity(records.len());
    for record in records {
        let mut row = hydrate_record(db, &record, &mut session).await?;
        if let Some(selected) = &selected {
            row.values.retain(|key, _| selected.contains(key.as_str()));
            row.links.retain(|key, _| selected.contains(key.as_str()));
        }
        rows.push(row);
    }
    Ok(TableResult {
        object: validated.object.into(),
        rows,
        next_cursor: has_more.then(|| encode_cursor(offset + u64::from(query.page_size))),
        total_count,
    })
}

pub async fn aggregate(
    db: &Database,
    owner_id: &str,
    query: &CrmAggregateQuery,
) -> AppResult<AggregateResult> {
    let validated = validate_aggregate(db, owner_id, query).await?;
    let (accumulator, unit) = measure_accumulator(&validated.object, &query.measure)?;
    let group_id = match &query.group_by {
        Some(key) => {
            let field = field(&validated.object, key).expect("validated group field");
            if matches!(field.kind, CrmFieldKind::Record { .. }) {
                doc! {"$arrayElemAt": [format!("$refs.{key}"), 0]}.into()
            } else {
                Bson::String(format!("$values.{key}"))
            }
        }
        None => Bson::Null,
    };
    let mut group = doc! {"_id": group_id, "value": accumulator, "count": {"$sum": 1_i64}};
    if matches!(query.measure, AggregateMeasure::Avg { .. }) {
        if let Some(measure_field) = measure_field(&query.measure) {
            group.insert("sum", doc! {"$sum": format!("$values.{measure_field}")});
            group.insert(
                "measure_count",
                doc! {"$sum": {"$cond": [
                    {"$ne": [{"$type": format!("$values.{measure_field}")}, "missing"]},
                    1_i64,
                    0_i64,
                ]}},
            );
        }
    }
    let pipeline = vec![
        doc! {"$match": validated.filter.clone()},
        doc! {"$group": group},
    ];
    let raw_points: Vec<Document> = db
        .collection::<CrmRecord>(RECORDS)
        .aggregate(pipeline)
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_collect()
        .await?;
    let total = aggregate_total(db, &validated.filter, &query.measure, &unit).await?;
    let mut points = Vec::with_capacity(raw_points.len());
    for raw in raw_points {
        let key = raw.get("_id").cloned().unwrap_or(Bson::Null);
        let count = bson_u64(raw.get("count")).unwrap_or(0);
        let raw_value = raw.get("value").cloned().unwrap_or(Bson::Null);
        let value = format_measure_value(&raw_value, &unit, &query.measure);
        let measure_count = bson_u64(raw.get("measure_count")).unwrap_or(0);
        let measure_sum = raw.get("sum").map(bson_json);
        let (label, archived) = group_label(
            db,
            owner_id,
            &validated.object,
            query.group_by.as_deref(),
            &key,
        )
        .await?;
        points.push(AggregatePoint {
            key: bson_json(&key),
            label,
            value,
            archived,
            count,
            measure_count,
            measure_sum,
        });
    }
    sort_points(&mut points, &validated.object, query)?;
    let truncated = points.len() > MAX_GROUPS;
    if truncated {
        let remainder = points.split_off(MAX_GROUPS);
        let other = combine_other(remainder, &query.measure, &unit);
        points.push(other);
    }
    Ok(AggregateResult {
        points,
        unit,
        total,
        truncated,
    })
}

pub async fn validate_table_query(
    db: &Database,
    owner_id: &str,
    query: &CrmTableQuery,
) -> AppResult<CrmObject> {
    Ok(validate_table(db, owner_id, query).await?.object)
}

pub async fn validate_aggregate_query(
    db: &Database,
    owner_id: &str,
    query: &CrmAggregateQuery,
) -> AppResult<CrmObject> {
    Ok(validate_aggregate(db, owner_id, query).await?.object)
}

pub fn validate_live_fields(
    object: &CrmObject,
    fields: &[String],
    path: &str,
) -> Vec<CrmValidationIssue> {
    let mut errors = Vec::new();
    validate_field_list(object, fields, path, &mut errors);
    errors
}

struct ValidatedQuery {
    object: CrmObject,
    filter: Document,
}

async fn validate_table(
    db: &Database,
    owner_id: &str,
    query: &CrmTableQuery,
) -> AppResult<ValidatedQuery> {
    let object = objects::get(db, owner_id, &query.object).await?;
    let mut errors = validate_filters(&object, &query.filters);
    if !(1..=MAX_PAGE_SIZE).contains(&query.page_size) {
        errors.push(issue("page_size", "must be between 1 and 200"));
    }
    if let Some(select) = &query.select {
        validate_field_list(&object, select, "select", &mut errors);
    }
    if let Some(sort) = &query.sort {
        match field(&object, &sort.field) {
            None => errors.push(issue("sort.field", "unknown field")),
            Some(field) if field.archived => {
                errors.push(issue("sort.field", "archived fields cannot be queried"))
            }
            Some(field)
                if !matches!(
                    field.kind,
                    CrmFieldKind::Text { .. }
                        | CrmFieldKind::Number { .. }
                        | CrmFieldKind::Currency { .. }
                        | CrmFieldKind::Boolean {}
                        | CrmFieldKind::Date {}
                        | CrmFieldKind::Datetime {}
                        | CrmFieldKind::Select { .. }
                ) =>
            {
                errors.push(issue("sort.field", "only scalar fields can be sorted"));
            }
            Some(_) => {}
        }
    }
    if let Some(cursor) = query.cursor.as_deref()
        && decode_cursor(Some(cursor)).is_err()
    {
        errors.push(issue("cursor", "is invalid"));
    }
    validation_result(errors)?;
    let filter = build_filter(owner_id, &object, &query.filters, query.include_archived)?;
    Ok(ValidatedQuery { object, filter })
}

async fn validate_aggregate(
    db: &Database,
    owner_id: &str,
    query: &CrmAggregateQuery,
) -> AppResult<ValidatedQuery> {
    let object = objects::get(db, owner_id, &query.object).await?;
    let mut errors = validate_filters(&object, &query.filters);
    if let Some(group_by) = &query.group_by {
        match field(&object, group_by) {
            None => errors.push(issue("group_by", "unknown field")),
            Some(field) if field.archived => {
                errors.push(issue("group_by", "archived fields cannot be queried"))
            }
            Some(field) if !fields::group_eligible(field) => errors.push(issue(
                "group_by",
                "must be a select, boolean, user, or one-record field",
            )),
            Some(field)
                if query.order == AggregateOrder::OptionOrder
                    && !matches!(field.kind, CrmFieldKind::Select { .. }) =>
            {
                errors.push(issue(
                    "order",
                    "option_order is valid only for select groups",
                ));
            }
            Some(_) => {}
        }
    } else if query.order == AggregateOrder::OptionOrder {
        errors.push(issue(
            "order",
            "option_order requires a select group_by field",
        ));
    }
    if let Some(measure_field) = measure_field(&query.measure) {
        match field(&object, measure_field) {
            None => errors.push(issue("measure.field", "unknown field")),
            Some(field) if field.archived => {
                errors.push(issue("measure.field", "archived fields cannot be queried"))
            }
            Some(field) if fields::measure_unit(field).is_none() => {
                errors.push(issue("measure.field", "must be a number or currency field"))
            }
            Some(_) => {}
        }
    }
    validation_result(errors)?;
    let filter = build_filter(owner_id, &object, &query.filters, false)?;
    Ok(ValidatedQuery { object, filter })
}

fn validate_filters(object: &CrmObject, filters: &[CrmFilter]) -> Vec<CrmValidationIssue> {
    let mut errors = Vec::new();
    if filters.len() > MAX_FILTERS {
        errors.push(issue("filters", "must contain at most 20 filters"));
    }
    for (index, filter) in filters.iter().enumerate() {
        let path = format!("filters.{index}");
        match field(object, &filter.field) {
            None => errors.push(issue(format!("{path}.field"), "unknown field")),
            Some(field) => {
                if let Err(error) =
                    fields::validate_filter_value(field, &filter.op, filter.value.as_ref(), &path)
                {
                    errors.push(error);
                }
            }
        }
    }
    errors
}

fn validate_field_list(
    object: &CrmObject,
    fields_to_validate: &[String],
    path: &str,
    errors: &mut Vec<CrmValidationIssue>,
) {
    let mut seen = HashSet::new();
    for (index, key) in fields_to_validate.iter().enumerate() {
        match field(object, key) {
            None => errors.push(issue(format!("{path}.{index}"), "unknown field")),
            Some(field) if field.archived => errors.push(issue(
                format!("{path}.{index}"),
                "archived fields cannot be queried",
            )),
            Some(_) if !seen.insert(key) => {
                errors.push(issue(format!("{path}.{index}"), "field is duplicated"))
            }
            Some(_) => {}
        }
    }
}

fn build_filter(
    owner_id: &str,
    object: &CrmObject,
    filters: &[CrmFilter],
    include_archived: bool,
) -> AppResult<Document> {
    let mut clauses = vec![doc! {"owner_id": owner_id, "object_key": &object.key}];
    if !include_archived {
        clauses.push(doc! {"archived_at": Bson::Null});
    }
    for (index, filter) in filters.iter().enumerate() {
        let field = field(object, &filter.field).expect("filters were validated");
        let value = fields::validate_filter_value(
            field,
            &filter.op,
            filter.value.as_ref(),
            &format!("filters.{index}"),
        )
        .map_err(|error| AppError::CrmValidationFailed(vec![error]))?;
        let prefix = if matches!(field.kind, CrmFieldKind::Record { .. }) {
            "refs"
        } else {
            "values"
        };
        let path = format!("{prefix}.{}", field.key);
        let clause = match (filter.op.as_str(), value) {
            ("eq", fields::ValidatedFilterValue::Scalar(value)) => doc! {path: value},
            ("eq", fields::ValidatedFilterValue::RecordIds(ids)) => doc! {path: ids[0].clone()},
            ("neq", fields::ValidatedFilterValue::Scalar(value)) => doc! {path: {"$ne": value}},
            ("in", fields::ValidatedFilterValue::List(values)) => doc! {path: {"$in": values}},
            ("in", fields::ValidatedFilterValue::RecordIds(ids)) => doc! {path: {"$in": ids}},
            ("nin", fields::ValidatedFilterValue::List(values)) => doc! {path: {"$nin": values}},
            ("contains", fields::ValidatedFilterValue::Scalar(Bson::String(value))) => {
                doc! {path: {"$regex": escape(&value), "$options": "i"}}
            }
            ("gt", fields::ValidatedFilterValue::Scalar(value)) => doc! {path: {"$gt": value}},
            ("gte", fields::ValidatedFilterValue::Scalar(value)) => doc! {path: {"$gte": value}},
            ("lt", fields::ValidatedFilterValue::Scalar(value)) => doc! {path: {"$lt": value}},
            ("lte", fields::ValidatedFilterValue::Scalar(value)) => doc! {path: {"$lte": value}},
            ("is_set", fields::ValidatedFilterValue::None) => {
                doc! {path: {"$exists": true, "$ne": Bson::Array(vec![])}}
            }
            ("is_unset", fields::ValidatedFilterValue::None) => doc! {path: {"$exists": false}},
            _ => {
                return Err(AppError::Internal(
                    "Validated CRM filter had an invalid shape".into(),
                ));
            }
        };
        clauses.push(clause);
    }
    Ok(doc! {"$and": clauses})
}

fn measure_accumulator(
    object: &CrmObject,
    measure: &AggregateMeasure,
) -> AppResult<(Bson, AggregateUnit)> {
    match measure {
        AggregateMeasure::Count => Ok((
            doc! {"$sum": 1_i64}.into(),
            AggregateUnit {
                format: "count".into(),
                currency: None,
            },
        )),
        AggregateMeasure::Sum { field: field_key }
        | AggregateMeasure::Avg { field: field_key }
        | AggregateMeasure::Min { field: field_key }
        | AggregateMeasure::Max { field: field_key } => {
            let metadata = field(object, field_key).expect("measure was validated");
            let operator = match measure {
                AggregateMeasure::Sum { .. } => "$sum",
                AggregateMeasure::Avg { .. } => "$avg",
                AggregateMeasure::Min { .. } => "$min",
                AggregateMeasure::Max { .. } => "$max",
                AggregateMeasure::Count => unreachable!(),
            };
            let unit = match fields::measure_unit(metadata).expect("measure was validated") {
                fields::MeasureUnit::Number => AggregateUnit {
                    format: "number".into(),
                    currency: None,
                },
                fields::MeasureUnit::Currency => AggregateUnit {
                    format: "currency".into(),
                    currency: fields::currency(metadata).map(str::to_owned),
                },
            };
            Ok((doc! {operator: format!("$values.{field_key}")}.into(), unit))
        }
    }
}

async fn aggregate_total(
    db: &Database,
    filter: &Document,
    measure: &AggregateMeasure,
    unit: &AggregateUnit,
) -> AppResult<Value> {
    let accumulator = match measure {
        AggregateMeasure::Count => doc! {"$sum": 1_i64},
        AggregateMeasure::Sum { field } => doc! {"$sum": format!("$values.{field}")},
        AggregateMeasure::Avg { field } => doc! {"$avg": format!("$values.{field}")},
        AggregateMeasure::Min { field } => doc! {"$min": format!("$values.{field}")},
        AggregateMeasure::Max { field } => doc! {"$max": format!("$values.{field}")},
    };
    let row = db
        .collection::<CrmRecord>(RECORDS)
        .aggregate(vec![
            doc! {"$match": filter},
            doc! {"$group": {"_id": Bson::Null, "value": accumulator}},
        ])
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_next()
        .await?;
    let value = row
        .as_ref()
        .and_then(|row| row.get("value"))
        .cloned()
        .unwrap_or_else(|| match measure {
            AggregateMeasure::Count | AggregateMeasure::Sum { .. } => Bson::Int64(0),
            _ => Bson::Null,
        });
    let currency_measure =
        unit.format == "currency" && matches!(measure, AggregateMeasure::Avg { .. });
    Ok(if currency_measure {
        round_numeric_json(&value)
    } else {
        bson_json(&value)
    })
}

fn format_measure_value(value: &Bson, unit: &AggregateUnit, measure: &AggregateMeasure) -> Value {
    if unit.format == "currency" && matches!(measure, AggregateMeasure::Avg { .. }) {
        round_numeric_json(value)
    } else {
        bson_json(value)
    }
}

fn round_numeric_json(value: &Bson) -> Value {
    bson_f64(Some(value))
        .map(|value| json!(value.round_ties_even() as i64))
        .unwrap_or(Value::Null)
}

async fn group_label(
    db: &Database,
    owner_id: &str,
    object: &CrmObject,
    group_by: Option<&str>,
    key: &Bson,
) -> AppResult<(String, bool)> {
    if key == &Bson::Null {
        return Ok(("None".into(), false));
    }
    let Some(group_by) = group_by else {
        return Ok(("Total".into(), false));
    };
    let field = field(object, group_by).expect("group field was validated");
    match &field.kind {
        CrmFieldKind::Select { options } => {
            let value = key.as_str().unwrap_or_default();
            Ok(options
                .iter()
                .find(|option| option.value == value)
                .map(|option| (option.label.clone(), option.archived))
                .unwrap_or_else(|| (value.to_owned(), true)))
        }
        CrmFieldKind::Boolean {} => Ok((
            if key.as_bool().unwrap_or(false) {
                "True"
            } else {
                "False"
            }
            .into(),
            false,
        )),
        CrmFieldKind::User {} => {
            let id = key.as_str().unwrap_or_default();
            let user = db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! {"_id": id, "user_type": "person"})
                .await?;
            Ok((
                user.and_then(|user| user.display_name)
                    .unwrap_or_else(|| "Deleted user".into()),
                false,
            ))
        }
        CrmFieldKind::Record { target, .. } => {
            let id = key.as_str().unwrap_or_default();
            let record = db
                .collection::<CrmRecord>(RECORDS)
                .find_one(doc! {"_id": id, "owner_id": owner_id, "object_key": target})
                .await?;
            let Some(record) = record else {
                return Ok(("Deleted record".into(), true));
            };
            if record.archived_at.is_some() {
                return Ok(("(archived)".into(), true));
            }
            let target_object = objects::get(db, owner_id, target).await?;
            let label = target_object
                .fields
                .iter()
                .find(|field| field.required && matches!(field.kind, CrmFieldKind::Text { .. }))
                .and_then(|field| record.values.get_str(&field.key).ok())
                .unwrap_or("(unnamed)")
                .to_owned();
            Ok((label, false))
        }
        _ => Ok((bson_json(key).to_string(), false)),
    }
}

fn sort_points(
    points: &mut [AggregatePoint],
    object: &CrmObject,
    query: &CrmAggregateQuery,
) -> AppResult<()> {
    match query.order {
        AggregateOrder::LabelAsc => points.sort_by(|left, right| left.label.cmp(&right.label)),
        AggregateOrder::ValueDesc => points.sort_by(|left, right| {
            json_f64(&right.value)
                .partial_cmp(&json_f64(&left.value))
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.label.cmp(&right.label))
        }),
        AggregateOrder::OptionOrder => {
            let field = field(
                object,
                query.group_by.as_deref().expect("validated option order"),
            )
            .expect("validated option field");
            let CrmFieldKind::Select { options } = &field.kind else {
                return Err(AppError::Internal(
                    "Validated option order is not a select".into(),
                ));
            };
            let positions: HashMap<&str, usize> = options
                .iter()
                .enumerate()
                .map(|(index, option)| (option.value.as_str(), index))
                .collect();
            points.sort_by_key(|point| {
                point
                    .key
                    .as_str()
                    .and_then(|key| positions.get(key).copied())
                    .unwrap_or(usize::MAX)
            });
        }
    }
    Ok(())
}

fn combine_other(
    points: Vec<AggregatePoint>,
    measure: &AggregateMeasure,
    unit: &AggregateUnit,
) -> AggregatePoint {
    let count: u64 = points.iter().map(|point| point.count).sum();
    let values: Vec<f64> = points.iter().map(|point| json_f64(&point.value)).collect();
    let value = match measure {
        AggregateMeasure::Count | AggregateMeasure::Sum { .. } => {
            if unit.format == "number" {
                let sum: f64 = values.iter().sum();
                json!(sum)
            } else {
                integer_sum(points.iter().map(|point| &point.value))
            }
        }
        AggregateMeasure::Avg { .. } => {
            let measure_count: u64 = points.iter().map(|point| point.measure_count).sum();
            if measure_count == 0 {
                Value::Null
            } else if unit.format == "currency" {
                let sum = points
                    .iter()
                    .filter_map(|point| point.measure_sum.as_ref())
                    .filter_map(Value::as_i64)
                    .map(i128::from)
                    .sum::<i128>();
                round_ratio_ties_even(sum, measure_count)
            } else {
                let sum: f64 = points
                    .iter()
                    .filter_map(|point| point.measure_sum.as_ref())
                    .map(json_f64)
                    .sum();
                json!(sum / measure_count as f64)
            }
        }
        AggregateMeasure::Min { .. } if unit.format == "currency" => points
            .iter()
            .filter_map(|point| point.value.as_i64())
            .min()
            .map_or(Value::Null, Value::from),
        AggregateMeasure::Max { .. } if unit.format == "currency" => points
            .iter()
            .filter_map(|point| point.value.as_i64())
            .max()
            .map_or(Value::Null, Value::from),
        AggregateMeasure::Min { .. } => values
            .into_iter()
            .reduce(f64::min)
            .map_or(Value::Null, Value::from),
        AggregateMeasure::Max { .. } => values
            .into_iter()
            .reduce(f64::max)
            .map_or(Value::Null, Value::from),
    };
    AggregatePoint {
        key: json!("__other__"),
        label: "Other".into(),
        value,
        archived: false,
        count,
        measure_count: 0,
        measure_sum: None,
    }
}

fn integer_sum<'a>(values: impl Iterator<Item = &'a Value>) -> Value {
    let sum = values
        .filter_map(Value::as_i64)
        .map(i128::from)
        .sum::<i128>();
    i64::try_from(sum).map_or(Value::Null, Value::from)
}

fn round_ratio_ties_even(sum: i128, count: u64) -> Value {
    let count = i128::from(count);
    let quotient = sum / count;
    let remainder = sum % count;
    let twice_remainder = remainder.abs() * 2;
    let rounded = if twice_remainder < count || (twice_remainder == count && quotient % 2 == 0) {
        quotient
    } else if sum.is_negative() {
        quotient - 1
    } else {
        quotient + 1
    };
    i64::try_from(rounded).map_or(Value::Null, Value::from)
}

fn field<'a>(object: &'a CrmObject, key: &str) -> Option<&'a CrmField> {
    object.fields.iter().find(|field| field.key == key)
}

fn measure_field(measure: &AggregateMeasure) -> Option<&str> {
    match measure {
        AggregateMeasure::Count => None,
        AggregateMeasure::Sum { field }
        | AggregateMeasure::Avg { field }
        | AggregateMeasure::Min { field }
        | AggregateMeasure::Max { field } => Some(field),
    }
}

fn bson_json(value: &Bson) -> Value {
    match value {
        Bson::Null => Value::Null,
        Bson::String(value) => json!(value),
        Bson::Boolean(value) => json!(value),
        Bson::Int32(value) => json!(value),
        Bson::Int64(value) => json!(value),
        Bson::Double(value) if value.is_finite() => json!(value),
        Bson::Decimal128(value) => value
            .to_string()
            .parse::<f64>()
            .map_or(Value::Null, |value| json!(value)),
        _ => Value::Null,
    }
}

fn bson_u64(value: Option<&Bson>) -> Option<u64> {
    match value {
        Some(Bson::Int32(value)) => (*value).try_into().ok(),
        Some(Bson::Int64(value)) => (*value).try_into().ok(),
        Some(Bson::Double(value)) if *value >= 0.0 => Some(*value as u64),
        _ => None,
    }
}

fn bson_f64(value: Option<&Bson>) -> Option<f64> {
    match value {
        Some(Bson::Int32(value)) => Some(f64::from(*value)),
        Some(Bson::Int64(value)) => Some(*value as f64),
        Some(Bson::Double(value)) => Some(*value),
        Some(Bson::Decimal128(value)) => value.to_string().parse().ok(),
        _ => None,
    }
}

fn json_f64(value: &Value) -> f64 {
    value.as_f64().unwrap_or(0.0)
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> CrmValidationIssue {
    CrmValidationIssue {
        path: path.into(),
        message: message.into(),
    }
}

fn encode_cursor(offset: u64) -> String {
    URL_SAFE_NO_PAD.encode(offset.to_be_bytes())
}

fn decode_cursor(cursor: Option<&str>) -> AppResult<u64> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| super::validation_error("cursor", "is invalid"))?;
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| super::validation_error("cursor", "is invalid"))?;
    Ok(u64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_filter_is_regex_escaped() {
        let object = CrmObject {
            id: "object".into(),
            owner_id: "owner".into(),
            key: "company".into(),
            label: "Company".into(),
            plural_label: "Companies".into(),
            schema_revision: 1,
            fields: vec![CrmField {
                key: "name".into(),
                label: "Name".into(),
                required: true,
                archived: false,
                kind: CrmFieldKind::Text { max_length: 100 },
            }],
            pipeline: None,
            default_columns: vec!["name".into()],
            seeded: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let filter = build_filter(
            "owner",
            &object,
            &[CrmFilter {
                field: "name".into(),
                op: "contains".into(),
                value: Some(json!("a.*b")),
            }],
            false,
        )
        .unwrap();
        assert_eq!(
            filter["$and"].as_array().unwrap()[2]["values.name"]["$regex"],
            Bson::String("a\\.\\*b".into())
        );
    }

    #[test]
    fn cursor_round_trip_is_opaque() {
        let cursor = encode_cursor(1234);
        assert_eq!(decode_cursor(Some(&cursor)).unwrap(), 1234);
        assert!(decode_cursor(Some("bad")).is_err());
    }

    #[test]
    fn currency_average_uses_half_even_rounding_for_both_signs() {
        assert_eq!(round_ratio_ties_even(5, 2), json!(2));
        assert_eq!(round_ratio_ties_even(7, 2), json!(4));
        assert_eq!(round_ratio_ties_even(-5, 2), json!(-2));
        assert_eq!(round_ratio_ties_even(-7, 2), json!(-4));
    }

    #[test]
    fn truncated_currency_extrema_stay_integer_minor_units() {
        let point = |value: i64| AggregatePoint {
            key: json!(value),
            label: value.to_string(),
            value: json!(value),
            archived: false,
            count: 1,
            measure_count: 0,
            measure_sum: None,
        };
        let unit = AggregateUnit {
            format: "currency".into(),
            currency: Some("USD".into()),
        };
        assert_eq!(
            combine_other(
                vec![point(9_007_199_254_740_991), point(7)],
                &AggregateMeasure::Min {
                    field: "amount".into(),
                },
                &unit,
            )
            .value,
            json!(7)
        );
        assert_eq!(
            combine_other(
                vec![point(9_007_199_254_740_991), point(7)],
                &AggregateMeasure::Max {
                    field: "amount".into(),
                },
                &unit,
            )
            .value,
            json!(9_007_199_254_740_991_i64)
        );
    }

    #[tokio::test]
    async fn aggregate_scopes_sources_and_preserves_truncated_groups() {
        let db =
            crate::test_utils::connect_transaction_test_database("crm_aggregate_contract").await;
        super::super::ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        let other_owner = uuid::Uuid::new_v4().to_string();
        objects::ensure_seeded(&db, &owner).await.unwrap();
        objects::ensure_seeded(&db, &other_owner).await.unwrap();
        let options: Vec<crate::models::crm_object::CrmSelectOption> = (0..51)
            .map(|index| crate::models::crm_object::CrmSelectOption {
                value: format!("option_{index:02}"),
                label: format!("Option {index:02}"),
                color: None,
                archived: false,
            })
            .collect();
        objects::add_field(
            &db,
            &owner,
            "company",
            1,
            CrmField {
                key: "bucket".into(),
                label: "Bucket".into(),
                required: false,
                archived: false,
                kind: CrmFieldKind::Select {
                    options: options.clone(),
                },
            },
            None,
        )
        .await
        .unwrap();
        let mut archived_options = options.clone();
        archived_options[0].archived = true;
        objects::patch_field(
            &db,
            &owner,
            "company",
            "bucket",
            objects::PatchField {
                expected_schema_revision: 2,
                label: None,
                required: None,
                archived: None,
                options: Some(archived_options),
            },
            None,
        )
        .await
        .unwrap();
        objects::add_field(
            &db,
            &owner,
            "company",
            3,
            CrmField {
                key: "score".into(),
                label: "Score".into(),
                required: false,
                archived: false,
                kind: CrmFieldKind::Number {
                    format: crate::models::crm_object::CrmNumberFormat::Plain,
                },
            },
            None,
        )
        .await
        .unwrap();
        let object = objects::get(&db, &owner, "company").await.unwrap();
        let other_object = objects::get(&db, &other_owner, "company").await.unwrap();
        let actor = crate::models::crm_record::CrmActor::human(&owner);
        let now = chrono::Utc::now();
        let mut rows: Vec<CrmRecord> = options
            .iter()
            .enumerate()
            .map(|(index, option)| CrmRecord {
                id: uuid::Uuid::new_v4().to_string(),
                owner_id: owner.clone(),
                object_id: object.id.clone(),
                object_key: "company".into(),
                revision: 1,
                values: doc! {
                    "name": format!("Company {index}"),
                    "bucket": &option.value,
                    "score": index as f64 + 0.5,
                },
                refs: Document::new(),
                overrides: vec![],
                created_by: actor.clone(),
                updated_by: actor.clone(),
                archived_at: None,
                created_at: now,
                updated_at: now,
            })
            .collect();
        rows.push(CrmRecord {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: owner.clone(),
            object_id: object.id.clone(),
            object_key: "company".into(),
            revision: 1,
            values: doc! {"name": "Archived source", "bucket": "option_00", "score": 1000.0},
            refs: Document::new(),
            overrides: vec![],
            created_by: actor.clone(),
            updated_by: actor.clone(),
            archived_at: Some(now),
            created_at: now,
            updated_at: now,
        });
        rows.push(CrmRecord {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: other_owner.clone(),
            object_id: other_object.id,
            object_key: "company".into(),
            revision: 1,
            values: doc! {"name": "Other owner", "bucket": "option_00", "score": 1000.0},
            refs: Document::new(),
            overrides: vec![],
            created_by: crate::models::crm_record::CrmActor::human(&other_owner),
            updated_by: crate::models::crm_record::CrmActor::human(&other_owner),
            archived_at: None,
            created_at: now,
            updated_at: now,
        });
        db.collection::<CrmRecord>(RECORDS)
            .insert_many(rows)
            .await
            .unwrap();

        let grouped = aggregate(
            &db,
            &owner,
            &CrmAggregateQuery {
                object: "company".into(),
                filters: vec![],
                group_by: Some("bucket".into()),
                measure: AggregateMeasure::Count,
                order: AggregateOrder::OptionOrder,
            },
        )
        .await
        .unwrap();
        assert_eq!(grouped.total, json!(51));
        assert!(grouped.truncated);
        assert_eq!(grouped.points.len(), 51);
        assert_eq!(grouped.points[0].key, json!("option_00"));
        assert_eq!(grouped.points[0].value, json!(1));
        assert!(grouped.points[0].archived);
        assert_eq!(grouped.points[50].key, json!("__other__"));
        assert_eq!(grouped.points[50].value, json!(1));
        assert_eq!(
            grouped
                .points
                .iter()
                .map(|point| point.value.as_i64().unwrap())
                .sum::<i64>(),
            51
        );

        let average = aggregate(
            &db,
            &owner,
            &CrmAggregateQuery {
                object: "company".into(),
                filters: vec![],
                group_by: None,
                measure: AggregateMeasure::Avg {
                    field: "score".into(),
                },
                order: AggregateOrder::ValueDesc,
            },
        )
        .await
        .unwrap();
        assert_eq!(average.unit.format, "number");
        assert_eq!(average.total, json!(25.5));
    }
}
