use std::collections::HashSet;
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
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult, CrmValidationIssue};
use crate::models::{
    crm_object::{CrmField, CrmFieldKind, CrmObject},
    crm_record::{COLLECTION_NAME as RECORDS, CrmRecord},
};

use super::records::{RecordResponse, hydrate_records};
use super::{fields, objects, objects::ObjectApi, validation_result};

const MAX_FILTERS: usize = 20;
const MAX_PAGE_SIZE: u32 = 200;
const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_GROUPS: usize = 50;
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SAFE_INTEGER: i128 = 9_007_199_254_740_991;

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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
    #[serde(default)]
    pub date_bucket: Option<DateBucket>,
    pub measure: AggregateMeasure,
    pub order: AggregateOrder,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DateBucket {
    pub unit: CalendarUnit,
    pub timezone: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarUnit {
    Day,
    Week,
    Month,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
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
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AggregateUnit {
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

pub async fn table(db: &Database, owner_id: &str, query: &CrmTableQuery) -> AppResult<TableResult> {
    let validated = validate_table(db, owner_id, query).await?;
    let cursor = decode_table_cursor(query)?;
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
    let filter = if let Some(cursor) = &cursor {
        doc! {"$and": [validated.filter.clone(), keyset_filter(query, cursor)?]}
    } else {
        validated.filter.clone()
    };
    let mut records: Vec<CrmRecord> = db
        .collection::<CrmRecord>(RECORDS)
        .find(filter)
        .sort(sort)
        .limit(i64::from(query.page_size) + 1)
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_collect()
        .await?;
    let has_more = records.len() > query.page_size as usize;
    records.truncate(query.page_size as usize);
    let next_cursor = has_more
        .then(|| records.last())
        .flatten()
        .map(|record| encode_table_cursor(query, record))
        .transpose()?;
    let selected: Option<HashSet<&str>> = query
        .select
        .as_ref()
        .map(|fields| fields.iter().map(String::as_str).collect());
    let mut session = db.client().start_session().await?;
    let mut rows = hydrate_records(db, &records, &mut session).await?;
    for row in &mut rows {
        if let Some(selected) = &selected {
            row.values.retain(|key, _| selected.contains(key.as_str()));
            row.links.retain(|key, _| selected.contains(key.as_str()));
        }
    }
    Ok(TableResult {
        object: validated.object.into(),
        rows,
        next_cursor,
        total_count,
    })
}

pub async fn aggregate(
    db: &Database,
    owner_id: &str,
    query: &CrmAggregateQuery,
) -> AppResult<AggregateResult> {
    let validated = validate_aggregate(db, owner_id, query).await?;
    let (_, unit) = measure_accumulator(&validated.object, &query.measure)?;
    let pipeline =
        build_aggregate_pipeline(db, owner_id, &validated.object, &validated.filter, query).await?;
    let result = db
        .collection::<CrmRecord>(RECORDS)
        .aggregate(pipeline)
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_next()
        .await?
        .ok_or_else(|| AppError::Internal("CRM aggregate returned no facet result".into()))?;
    let raw_points = result
        .get_array("points")
        .map_err(|_| AppError::Internal("CRM aggregate points are invalid".into()))?;
    let mut points = raw_points
        .iter()
        .map(|point| {
            point
                .as_document()
                .ok_or_else(|| AppError::Internal("CRM aggregate point is invalid".into()))
                .and_then(|point| aggregate_point(point, &query.measure, &unit))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let remainder = result
        .get_array("remainder")
        .ok()
        .and_then(|rows| rows.first())
        .and_then(Bson::as_document);
    let truncated = remainder.is_some_and(|row| bson_u64(row.get("group_count")).unwrap_or(0) > 0);
    if let Some(remainder) = remainder.filter(|_| truncated) {
        points.push(AggregatePoint {
            key: json!("__other__"),
            label: "Other".into(),
            value: aggregate_output(remainder, &query.measure, &unit)?,
            archived: false,
        });
    }
    let total_row = result
        .get_array("total")
        .ok()
        .and_then(|rows| rows.first())
        .and_then(Bson::as_document);
    let total = match total_row {
        Some(row) => aggregate_output(row, &query.measure, &unit)?,
        None if matches!(query.measure, AggregateMeasure::Count) => json!(0),
        None => Value::Null,
    };
    Ok(AggregateResult {
        points,
        unit,
        total,
        truncated,
    })
}

async fn build_aggregate_pipeline(
    db: &Database,
    owner_id: &str,
    object: &CrmObject,
    filter: &Document,
    query: &CrmAggregateQuery,
) -> AppResult<Vec<Document>> {
    let group_id = match &query.group_by {
        Some(key) => {
            let field = field(object, key).expect("validated group field");
            if let Some(bucket) = &query.date_bucket {
                let format = match bucket.unit {
                    CalendarUnit::Day => "%Y-%m-%d",
                    CalendarUnit::Week => "%G-W%V",
                    CalendarUnit::Month => "%Y-%m",
                };
                let value = format!("$values.{key}");
                doc! {"$dateToString": {"date": {"$convert": {"input": value, "to": "date", "onNull": Bson::Null, "onError": Bson::Null}}, "format": format, "timezone": &bucket.timezone, "onNull": Bson::Null}}.into()
            } else if matches!(field.kind, CrmFieldKind::Record { .. }) {
                doc! {"$arrayElemAt": [format!("$refs.{key}"), 0]}.into()
            } else {
                Bson::String(format!("$values.{key}"))
            }
        }
        None => Bson::Null,
    };
    let mut pipeline = vec![
        doc! {"$match": filter.clone()},
        doc! {"$group": aggregate_group(object, &query.measure, group_id)?},
    ];
    append_group_labels(
        db,
        owner_id,
        object,
        query.group_by.as_deref(),
        &mut pipeline,
    )
    .await?;
    if matches!(query.order, AggregateOrder::ValueDesc) {
        pipeline.push(doc! {"$set": {
            "sort_value": aggregate_value_expression(&query.measure),
        }});
    }
    pipeline.push(doc! {"$sort": aggregate_sort(query)});
    pipeline.push(doc! {"$facet": {
        "points": [{"$limit": MAX_GROUPS as i64}],
        "remainder": [
            {"$skip": MAX_GROUPS as i64},
            {"$group": aggregate_summary_group(&query.measure, true)},
        ],
        "total": [
            {"$group": aggregate_summary_group(&query.measure, false)},
        ],
    }});
    Ok(pipeline)
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

pub(crate) struct ValidatedQuery {
    pub(crate) object: CrmObject,
    filter: Document,
}

async fn validate_table(
    db: &Database,
    owner_id: &str,
    query: &CrmTableQuery,
) -> AppResult<ValidatedQuery> {
    let object = objects::get(db, owner_id, &query.object).await?;
    validate_table_metadata(owner_id, object, query)
}

pub(crate) fn validate_table_metadata(
    owner_id: &str,
    object: CrmObject,
    query: &CrmTableQuery,
) -> AppResult<ValidatedQuery> {
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
    match decode_table_cursor(query) {
        Ok(Some(cursor)) if !cursor_matches_query(&object, query, &cursor) => {
            errors.push(issue("cursor", "is invalid"));
        }
        Err(_) => errors.push(issue("cursor", "is invalid")),
        _ => {}
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
    validate_aggregate_metadata(owner_id, object, query)
}

pub(crate) fn validate_aggregate_metadata(
    owner_id: &str,
    object: CrmObject,
    query: &CrmAggregateQuery,
) -> AppResult<ValidatedQuery> {
    let mut errors = validate_filters(&object, &query.filters);
    if let Some(group_by) = &query.group_by {
        match field(&object, group_by) {
            None => errors.push(issue("group_by", "unknown field")),
            Some(field) if field.archived => {
                errors.push(issue("group_by", "archived fields cannot be queried"))
            }
            Some(field)
                if !fields::group_eligible(field)
                    && !matches!(
                        field.kind,
                        CrmFieldKind::Date {} | CrmFieldKind::Datetime {}
                    ) =>
            {
                errors.push(issue(
                    "group_by",
                    "must be a select, boolean, user, or one-record field",
                ))
            }
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
    let calendar_field = query
        .group_by
        .as_ref()
        .and_then(|key| field(&object, key))
        .filter(|f| matches!(f.kind, CrmFieldKind::Date {} | CrmFieldKind::Datetime {}));
    match (&query.date_bucket, calendar_field) {
        (Some(bucket), Some(field)) => {
            if bucket.timezone.parse::<chrono_tz::Tz>().is_err() {
                errors.push(issue("date_bucket.timezone", "must be an IANA timezone"));
            }
            if matches!(field.kind, CrmFieldKind::Date {}) && bucket.timezone != "UTC" {
                errors.push(issue(
                    "date_bucket.timezone",
                    "calendar dates require UTC; their date is never converted",
                ));
            }
        }
        (None, Some(_)) => errors.push(issue(
            "date_bucket",
            "date groups require an explicit calendar bucket and timezone",
        )),
        (Some(_), None) => errors.push(issue(
            "date_bucket",
            "requires a date or datetime group_by field",
        )),
        _ => {}
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

fn aggregate_group(
    object: &CrmObject,
    measure: &AggregateMeasure,
    group_id: Bson,
) -> AppResult<Document> {
    let mut group = doc! {
        "_id": group_id,
        "count": {"$sum": 1_i64},
    };
    let Some(field_key) = measure_field(measure) else {
        return Ok(group);
    };
    let metadata = field(object, field_key).expect("measure field was validated");
    let path = format!("$values.{field_key}");
    let present = doc! {"$and": [
        {"$ne": [{"$type": &path}, "missing"]},
        {"$ne": [&path, Bson::Null]},
    ]};
    let numeric = if matches!(metadata.kind, CrmFieldKind::Currency { .. }) {
        Bson::Document(doc! {"$toDecimal": &path})
    } else {
        Bson::String(path)
    };
    let zero = if matches!(metadata.kind, CrmFieldKind::Currency { .. }) {
        Bson::Document(doc! {"$toDecimal": 0_i64})
    } else {
        Bson::Double(0.0)
    };
    let present_numeric = Bson::Document(doc! {"$cond": [present.clone(), numeric.clone(), zero]});
    match measure {
        AggregateMeasure::Sum { .. } => {
            group.insert("sum", doc! {"$sum": present_numeric});
            group.insert(
                "measure_count",
                doc! {"$sum": {"$cond": [present, 1_i64, 0_i64]}},
            );
        }
        AggregateMeasure::Avg { .. } => {
            group.insert("sum", doc! {"$sum": present_numeric});
            group.insert(
                "measure_count",
                doc! {"$sum": {"$cond": [present, 1_i64, 0_i64]}},
            );
        }
        AggregateMeasure::Min { .. } => {
            group.insert(
                "value",
                doc! {"$min": {"$cond": [present, numeric, Bson::Null]}},
            );
        }
        AggregateMeasure::Max { .. } => {
            group.insert(
                "value",
                doc! {"$max": {"$cond": [present, numeric, Bson::Null]}},
            );
        }
        AggregateMeasure::Count => {}
    }
    Ok(group)
}

async fn append_group_labels(
    db: &Database,
    owner_id: &str,
    object: &CrmObject,
    group_by: Option<&str>,
    pipeline: &mut Vec<Document>,
) -> AppResult<()> {
    let Some(group_by) = group_by else {
        pipeline.push(doc! {"$set": {"label": "Total", "archived": false}});
        return Ok(());
    };
    let field = field(object, group_by).expect("group field was validated");
    match &field.kind {
        CrmFieldKind::Select { options } => {
            let label_branches: Vec<Bson> = options
                .iter()
                .map(|option| {
                    Bson::Document(doc! {
                        "case": {"$eq": ["$_id", &option.value]},
                        "then": {"$literal": &option.label},
                    })
                })
                .collect();
            let archived_branches: Vec<Bson> = options
                .iter()
                .map(|option| {
                    Bson::Document(doc! {
                        "case": {"$eq": ["$_id", &option.value]},
                        "then": option.archived,
                    })
                })
                .collect();
            let values: Vec<Bson> = options
                .iter()
                .map(|option| Bson::String(option.value.clone()))
                .collect();
            let value_label = if label_branches.is_empty() {
                Bson::Document(doc! {"$toString": "$_id"})
            } else {
                Bson::Document(doc! {"$switch": {
                    "branches": label_branches,
                    "default": {"$toString": "$_id"},
                }})
            };
            let value_archived = if archived_branches.is_empty() {
                Bson::Boolean(true)
            } else {
                Bson::Document(doc! {"$switch": {
                    "branches": archived_branches,
                    "default": true,
                }})
            };
            pipeline.push(doc! {"$set": {
                "label": {"$cond": [
                    {"$eq": ["$_id", Bson::Null]},
                    "None",
                    value_label,
                ]},
                "archived": {"$cond": [
                    {"$eq": ["$_id", Bson::Null]},
                    false,
                    value_archived,
                ]},
                "option_index": {"$cond": [
                    {"$eq": ["$_id", Bson::Null]},
                    i64::MAX,
                    {"$indexOfArray": [values, "$_id"]},
                ]},
            }});
        }
        CrmFieldKind::Date {} | CrmFieldKind::Datetime {} => pipeline
            .push(doc! {"$set": {"label": {"$ifNull": ["$_id", "No date"]}, "archived": false}}),
        CrmFieldKind::Boolean {} => pipeline.push(doc! {"$set": {
            "label": {"$switch": {
                "branches": [
                    {"case": {"$eq": ["$_id", Bson::Null]}, "then": "None"},
                    {"case": {"$eq": ["$_id", true]}, "then": "True"},
                ],
                "default": "False",
            }},
            "archived": false,
        }}),
        CrmFieldKind::User {} => {
            pipeline.push(doc! {"$lookup": {
                "from": crate::models::user::COLLECTION_NAME,
                "let": {"group_id": "$_id"},
                "pipeline": [
                    {"$match": {"is_active":true,"user_type":"person","$expr": {"$eq": ["$_id", "$$group_id"]}}},
                    {"$limit": 1_i64},
                    {"$lookup": {
                        "from":crate::models::org_membership::COLLECTION_NAME,
                        "let":{"person_id":"$_id"},
                        "pipeline":[
                            {"$match":{"org_user_id":owner_id,"revoked_at":Bson::Null,"$expr":{"$eq":["$member_user_id","$$person_id"]}}},
                            {"$project":{"_id":1_i64}}, {"$limit":1_i64}
                        ],"as":"membership"
                    }},
                    {"$match":{"$expr":{"$or":[{"$eq":["$_id",owner_id]},{"$gt":[{"$size":"$membership"},0_i64]}]}}},
                    {"$project": {"_id": 0_i64, "display_name": {"$cond":[{"$gt":[{"$strLenCP":{"$trim":{"input":{"$ifNull":["$display_name",""]}}}},0_i64]},"$display_name","Member"]}}},
                ],
                "as": "label_row",
            }});
            pipeline.push(doc! {"$set": {
                "label": {"$cond": [
                    {"$eq": ["$_id", Bson::Null]},
                    "None",
                    {"$ifNull": [{"$arrayElemAt": ["$label_row.display_name", 0_i64]}, "Unavailable member"]},
                ]},
                "archived": false,
            }});
            pipeline.push(doc! {"$unset": "label_row"});
        }
        CrmFieldKind::Record { target, .. } => {
            let target_object = objects::get(db, owner_id, target).await?;
            let label_expression = target_object
                .fields
                .iter()
                .find(|field| field.required && matches!(field.kind, CrmFieldKind::Text { .. }))
                .map_or_else(
                    || Bson::Document(doc! {"$literal": "(unnamed)"}),
                    |field| Bson::String(format!("$values.{}", field.key)),
                );
            pipeline.push(doc! {"$lookup": {
                "from": RECORDS,
                "let": {"group_id": "$_id"},
                "pipeline": [
                    {"$match": {"$expr": {"$and": [
                        {"$eq": ["$_id", "$$group_id"]},
                        {"$eq": ["$owner_id", owner_id]},
                        {"$eq": ["$object_key", target]},
                    ]}}},
                    {"$project": {
                        "_id": 0_i64,
                        "archived_at": 1_i64,
                        "label": label_expression,
                    }},
                    {"$limit": 1_i64},
                ],
                "as": "label_row",
            }});
            let live_label = Bson::Document(doc! {"$ifNull": [
                {"$arrayElemAt": ["$label_row.label", 0_i64]},
                "(unnamed)",
            ]});
            pipeline.push(doc! {"$set": {
                "label": {"$cond": [
                    {"$eq": ["$_id", Bson::Null]},
                    "None",
                    {"$cond": [
                        {"$eq": [{"$size": "$label_row"}, 0_i64]},
                        "Deleted record",
                        {"$cond": [
                            {"$ne": [{"$arrayElemAt": ["$label_row.archived_at", 0_i64]}, Bson::Null]},
                            "(archived)",
                            live_label,
                        ]},
                    ]},
                ]},
                "archived": {"$cond": [
                    {"$eq": ["$_id", Bson::Null]},
                    false,
                    {"$or": [
                        {"$eq": [{"$size": "$label_row"}, 0_i64]},
                        {"$ne": [{"$arrayElemAt": ["$label_row.archived_at", 0_i64]}, Bson::Null]},
                    ]},
                ]},
            }});
            pipeline.push(doc! {"$unset": "label_row"});
        }
        _ => {
            return Err(AppError::Internal(
                "Validated CRM aggregate group was not groupable".into(),
            ));
        }
    }
    Ok(())
}

fn aggregate_value_expression(measure: &AggregateMeasure) -> Bson {
    match measure {
        AggregateMeasure::Count => Bson::String("$count".into()),
        AggregateMeasure::Sum { .. } => Bson::String("$sum".into()),
        AggregateMeasure::Avg { .. } => Bson::Document(doc! {"$cond": [
            {"$gt": ["$measure_count", 0_i64]},
            {"$divide": ["$sum", "$measure_count"]},
            Bson::Null,
        ]}),
        AggregateMeasure::Min { .. } | AggregateMeasure::Max { .. } => {
            Bson::String("$value".into())
        }
    }
}

fn aggregate_sort(query: &CrmAggregateQuery) -> Document {
    match query.order {
        AggregateOrder::OptionOrder => doc! {"option_index": 1_i64, "_id": 1_i64},
        AggregateOrder::ValueDesc => {
            doc! {"sort_value": -1_i64, "label": 1_i64, "_id": 1_i64}
        }
        AggregateOrder::LabelAsc => doc! {"label": 1_i64, "_id": 1_i64},
    }
}

fn aggregate_summary_group(measure: &AggregateMeasure, group_count: bool) -> Document {
    let mut summary = doc! {
        "_id": Bson::Null,
        "count": {"$sum": "$count"},
    };
    if group_count {
        summary.insert("group_count", doc! {"$sum": 1_i64});
    }
    match measure {
        AggregateMeasure::Count => {}
        AggregateMeasure::Sum { .. } => {
            summary.insert("sum", doc! {"$sum": "$sum"});
            summary.insert("measure_count", doc! {"$sum": "$measure_count"});
        }
        AggregateMeasure::Avg { .. } => {
            summary.insert("sum", doc! {"$sum": "$sum"});
            summary.insert("measure_count", doc! {"$sum": "$measure_count"});
        }
        AggregateMeasure::Min { .. } => {
            summary.insert("value", doc! {"$min": "$value"});
        }
        AggregateMeasure::Max { .. } => {
            summary.insert("value", doc! {"$max": "$value"});
        }
    }
    summary
}

fn aggregate_point(
    row: &Document,
    measure: &AggregateMeasure,
    unit: &AggregateUnit,
) -> AppResult<AggregatePoint> {
    Ok(AggregatePoint {
        key: row.get("_id").map_or(Value::Null, bson_json),
        label: row.get_str("label").unwrap_or("None").to_owned(),
        value: aggregate_output(row, measure, unit)?,
        archived: row.get_bool("archived").unwrap_or(false),
    })
}

fn aggregate_output(
    row: &Document,
    measure: &AggregateMeasure,
    unit: &AggregateUnit,
) -> AppResult<Value> {
    if matches!(measure, AggregateMeasure::Count) {
        return safe_integer_output(i128::from(bson_u64(row.get("count")).unwrap_or(0)));
    }
    if matches!(measure, AggregateMeasure::Sum { .. })
        && bson_u64(row.get("measure_count")).unwrap_or(0) == 0
    {
        return Ok(Value::Null);
    }
    if unit.format == "currency" {
        return match measure {
            AggregateMeasure::Sum { .. } => exact_integer(row.get("sum"))
                .map(safe_integer_output)
                .ok_or_else(|| {
                    super::validation_error(
                        "measure",
                        "currency aggregate exceeds exact numeric range",
                    )
                })?,
            AggregateMeasure::Avg { .. } => {
                let count = bson_u64(row.get("measure_count")).unwrap_or(0);
                if count == 0 {
                    Ok(Value::Null)
                } else {
                    let sum = exact_integer(row.get("sum")).ok_or_else(|| {
                        AppError::Internal("CRM currency aggregate sum is invalid".into())
                    })?;
                    safe_integer_output(round_ratio_ties_even_i128(sum, count))
                }
            }
            AggregateMeasure::Min { .. } | AggregateMeasure::Max { .. } => row
                .get("value")
                .filter(|value| !matches!(value, Bson::Null))
                .and_then(|value| exact_integer(Some(value)))
                .map(safe_integer_output)
                .unwrap_or_else(|| Ok(Value::Null)),
            AggregateMeasure::Count => unreachable!(),
        };
    }
    let value = match measure {
        AggregateMeasure::Sum { .. } => row.get("sum"),
        AggregateMeasure::Avg { .. } => {
            let count = bson_u64(row.get("measure_count")).unwrap_or(0);
            if count == 0 {
                return Ok(Value::Null);
            }
            return finite_output(bson_f64(row.get("sum")).map(|sum| sum / count as f64));
        }
        AggregateMeasure::Min { .. } | AggregateMeasure::Max { .. } => row.get("value"),
        AggregateMeasure::Count => unreachable!(),
    };
    finite_output(value.and_then(|value| bson_f64(Some(value))))
}

fn finite_output(value: Option<f64>) -> AppResult<Value> {
    match value {
        Some(value) if !value.is_finite() => Err(super::validation_error(
            "measure",
            "aggregate exceeds finite numeric range",
        )),
        Some(value) => Ok(json!(value)),
        None => Ok(Value::Null),
    }
}

fn exact_integer(value: Option<&Bson>) -> Option<i128> {
    match value {
        Some(Bson::Int32(value)) => Some(i128::from(*value)),
        Some(Bson::Int64(value)) => Some(i128::from(*value)),
        Some(Bson::Decimal128(value)) => parse_decimal_integer(&value.to_string()),
        _ => None,
    }
}

fn parse_decimal_integer(value: &str) -> Option<i128> {
    let (mantissa, exponent) = if let Some((mantissa, exponent)) = value.split_once(['e', 'E']) {
        (mantissa, exponent.parse().ok()?)
    } else {
        (value, 0_i32)
    };
    let negative = mantissa.starts_with('-');
    let unsigned = mantissa.trim_start_matches(['-', '+']);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let mut digits = format!("{whole}{fraction}");
    let scale = i32::try_from(fraction.len()).ok()? - exponent;
    if scale > 0 {
        let scale = usize::try_from(scale).ok()?;
        if scale > digits.len() || !digits[digits.len() - scale..].bytes().all(|b| b == b'0') {
            return None;
        }
        digits.truncate(digits.len() - scale);
    } else if scale < 0 {
        digits.extend(std::iter::repeat_n('0', usize::try_from(-scale).ok()?));
    }
    if digits.is_empty() {
        digits.push('0');
    }
    let parsed = digits.parse::<i128>().ok()?;
    Some(if negative { -parsed } else { parsed })
}

fn safe_integer_output(value: i128) -> AppResult<Value> {
    if value.abs() > MAX_SAFE_INTEGER {
        Err(super::validation_error(
            "aggregate",
            "result exceeds the maximum safe integer 9007199254740991",
        ))
    } else {
        Ok(Value::from(value as i64))
    }
}

fn round_ratio_ties_even_i128(sum: i128, count: u64) -> i128 {
    let count = i128::from(count);
    let quotient = sum / count;
    let remainder = sum % count;
    let twice_remainder = remainder.abs() * 2;
    if twice_remainder < count || (twice_remainder == count && quotient % 2 == 0) {
        quotient
    } else if sum.is_negative() {
        quotient - 1
    } else {
        quotient + 1
    }
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
                    format: if matches!(
                        metadata.kind,
                        CrmFieldKind::Number {
                            format: crate::models::crm_object::CrmNumberFormat::Percent
                        }
                    ) {
                        "percent"
                    } else {
                        "number"
                    }
                    .into(),
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

fn issue(path: impl Into<String>, message: impl Into<String>) -> CrmValidationIssue {
    CrmValidationIssue {
        path: path.into(),
        message: message.into(),
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct TableCursor {
    digest: String,
    direction: SortDirection,
    is_null: bool,
    value: Option<Bson>,
    id: String,
}

fn query_digest(query: &CrmTableQuery) -> AppResult<String> {
    let mut canonical = query.clone();
    canonical.cursor = None;
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| AppError::Internal(format!("CRM query digest failed: {error}")))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn cursor_direction(query: &CrmTableQuery) -> SortDirection {
    query
        .sort
        .as_ref()
        .map_or(SortDirection::Asc, |sort| sort.direction)
}

fn encode_table_cursor(query: &CrmTableQuery, record: &CrmRecord) -> AppResult<String> {
    let value = query
        .sort
        .as_ref()
        .and_then(|sort| record.values.get(&sort.field))
        .filter(|value| !matches!(value, Bson::Null))
        .cloned();
    let cursor = TableCursor {
        digest: query_digest(query)?,
        direction: cursor_direction(query),
        is_null: value.is_none(),
        value,
        id: record.id.clone(),
    };
    bson::to_vec(&cursor)
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
        .map_err(|_| super::validation_error("cursor", "is invalid"))
}

fn decode_table_cursor(query: &CrmTableQuery) -> AppResult<Option<TableCursor>> {
    let Some(cursor) = query.cursor.as_deref() else {
        return Ok(None);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| super::validation_error("cursor", "is invalid"))?;
    let cursor: TableCursor =
        bson::from_slice(&bytes).map_err(|_| super::validation_error("cursor", "is invalid"))?;
    if cursor.digest != query_digest(query)?
        || cursor.direction != cursor_direction(query)
        || cursor.is_null != cursor.value.is_none()
        || cursor.id.is_empty()
    {
        return Err(super::validation_error("cursor", "is invalid"));
    }
    Ok(Some(cursor))
}

fn cursor_matches_query(object: &CrmObject, query: &CrmTableQuery, cursor: &TableCursor) -> bool {
    if uuid::Uuid::parse_str(&cursor.id).is_err() {
        return false;
    }
    let Some(sort) = &query.sort else {
        return cursor.is_null;
    };
    let Some(value) = cursor.value.as_ref() else {
        return cursor.is_null;
    };
    let Some(field) = field(object, &sort.field) else {
        return false;
    };
    match (&field.kind, value) {
        (
            CrmFieldKind::Text { .. } | CrmFieldKind::Date {} | CrmFieldKind::Select { .. },
            Bson::String(_),
        ) => true,
        (CrmFieldKind::Number { .. }, Bson::Double(value)) => value.is_finite(),
        (CrmFieldKind::Currency { .. }, Bson::Int64(_)) => true,
        (CrmFieldKind::Boolean {}, Bson::Boolean(_)) => true,
        (CrmFieldKind::Datetime {}, Bson::DateTime(_)) => true,
        _ => false,
    }
}

fn keyset_filter(query: &CrmTableQuery, cursor: &TableCursor) -> AppResult<Document> {
    let Some(sort) = &query.sort else {
        return Ok(doc! {"_id": {"$gt": &cursor.id}});
    };
    let path = format!("values.{}", sort.field);
    if cursor.is_null {
        return Ok(match sort.direction {
            SortDirection::Asc => doc! {"$or": [
                {&path: Bson::Null, "_id": {"$gt": &cursor.id}},
                {&path: {"$exists": true, "$ne": Bson::Null}},
            ]},
            SortDirection::Desc => {
                doc! {&path: Bson::Null, "_id": {"$gt": &cursor.id}}
            }
        });
    }
    let value = cursor
        .value
        .as_ref()
        .ok_or_else(|| super::validation_error("cursor", "is invalid"))?;
    Ok(match sort.direction {
        SortDirection::Asc => doc! {"$or": [
            {&path: {"$gt": value}},
            {&path: value, "_id": {"$gt": &cursor.id}},
        ]},
        SortDirection::Desc => doc! {"$or": [
            {&path: {"$lt": value}},
            {&path: value, "_id": {"$gt": &cursor.id}},
            {&path: Bson::Null},
        ]},
    })
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
    fn table_cursor_binds_query_sort_value_null_state_and_id() {
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
        let mut query = CrmTableQuery {
            object: "company".into(),
            select: None,
            filters: vec![],
            sort: Some(CrmSort {
                field: "name".into(),
                direction: SortDirection::Asc,
            }),
            page_size: 10,
            cursor: None,
            include_archived: false,
        };
        let record = CrmRecord {
            field_sources: mongodb::bson::Document::new(),
            id: "00000000-0000-4000-8000-000000000001".into(),
            owner_id: "owner".into(),
            object_id: "object".into(),
            object_key: "company".into(),
            revision: 1,
            values: doc! {"name": "Acme"},
            refs: Document::new(),
            overrides: vec![],
            created_by: crate::models::crm_record::CrmActor::human("owner"),
            updated_by: crate::models::crm_record::CrmActor::human("owner"),
            archived_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        query.cursor = Some(encode_table_cursor(&query, &record).unwrap());
        let cursor = decode_table_cursor(&query).unwrap().unwrap();
        assert_eq!(cursor.value, Some(Bson::String("Acme".into())));
        assert!(!cursor.is_null);
        assert_eq!(cursor.id, "00000000-0000-4000-8000-000000000001");
        assert!(cursor_matches_query(&object, &query, &cursor));

        let invalid_value = TableCursor {
            value: Some(Bson::Boolean(true)),
            ..cursor
        };
        assert!(!cursor_matches_query(&object, &query, &invalid_value));

        query.page_size = 11;
        assert!(decode_table_cursor(&query).is_err());
        query.cursor = Some("bad".into());
        assert!(decode_table_cursor(&query).is_err());
    }

    #[test]
    fn currency_average_uses_half_even_rounding_for_both_signs() {
        assert_eq!(round_ratio_ties_even_i128(5, 2), 2);
        assert_eq!(round_ratio_ties_even_i128(7, 2), 4);
        assert_eq!(round_ratio_ties_even_i128(-5, 2), -2);
        assert_eq!(round_ratio_ties_even_i128(-7, 2), -4);
    }

    #[test]
    fn truncated_currency_extrema_stay_integer_minor_units() {
        let unit = AggregateUnit {
            format: "currency".into(),
            currency: Some("USD".into()),
        };
        assert_eq!(
            aggregate_output(
                &doc! {"value": 7_i64},
                &AggregateMeasure::Min {
                    field: "amount".into(),
                },
                &unit,
            )
            .unwrap(),
            json!(7)
        );
        assert_eq!(
            aggregate_output(
                &doc! {"value": 9_007_199_254_740_991_i64},
                &AggregateMeasure::Max {
                    field: "amount".into(),
                },
                &unit,
            )
            .unwrap(),
            json!(9_007_199_254_740_991_i64)
        );
        assert_eq!(
            aggregate_output(
                &doc! {"value": Bson::Null},
                &AggregateMeasure::Min {
                    field: "amount".into(),
                },
                &unit,
            )
            .unwrap(),
            Value::Null
        );
        assert!(matches!(
            aggregate_output(
                &doc! {"sum": Bson::Decimal128("9007199254740992".parse().unwrap()), "measure_count": 1_i64},
                &AggregateMeasure::Sum {
                    field: "amount".into(),
                },
                &unit,
            ),
            Err(AppError::CrmValidationFailed(_))
        ));
    }

    #[tokio::test]
    async fn aggregate_groups_missing_empty_select_as_none() {
        let db = crate::test_utils::connect_transaction_test_database("crm_aggregate_empty_select")
            .await;
        super::super::ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        objects::ensure_seeded(&db, &owner).await.unwrap();
        let object = objects::get(&db, &owner, "company").await.unwrap();
        let now = chrono::Utc::now();
        db.collection::<CrmRecord>(RECORDS)
            .insert_one(CrmRecord {
                field_sources: mongodb::bson::Document::new(),
                id: uuid::Uuid::new_v4().to_string(),
                owner_id: owner.clone(),
                object_id: object.id,
                object_key: "company".into(),
                revision: 1,
                values: doc! {"name": "No industry"},
                refs: Document::new(),
                overrides: vec![],
                created_by: crate::models::crm_record::CrmActor::human(&owner),
                updated_by: crate::models::crm_record::CrmActor::human(&owner),
                archived_at: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let result = aggregate(
            &db,
            &owner,
            &CrmAggregateQuery {
                date_bucket: None,
                object: "company".into(),
                filters: vec![],
                group_by: Some("industry".into()),
                measure: AggregateMeasure::Count,
                order: AggregateOrder::LabelAsc,
            },
        )
        .await
        .unwrap();

        assert_eq!(result.total, json!(1));
        assert!(!result.truncated);
        assert_eq!(result.points.len(), 1);
        assert_eq!(result.points[0].key, Value::Null);
        assert_eq!(result.points[0].label, "None");
        assert_eq!(result.points[0].value, json!(1));
        assert!(!result.points[0].archived);
    }

    #[tokio::test]
    async fn aggregate_treats_dollar_prefixed_select_labels_as_literals() {
        let db = crate::test_utils::connect_transaction_test_database(
            "crm_aggregate_literal_select_labels",
        )
        .await;
        super::super::ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        objects::ensure_seeded(&db, &owner).await.unwrap();
        let opportunity = objects::get(&db, &owner, "opportunity").await.unwrap();
        let mut options = match &opportunity
            .fields
            .iter()
            .find(|field| field.key == "stage")
            .unwrap()
            .kind
        {
            CrmFieldKind::Select { options } => options.clone(),
            _ => panic!("seeded opportunity stage must be a select"),
        };
        options[0].label = "$$crm_missing_variable".into();
        options[1].label = "$Revenue".into();
        let opportunity = objects::patch_field(
            &db,
            &owner,
            "opportunity",
            "stage",
            objects::PatchField {
                expected_schema_revision: opportunity.schema_revision,
                label: None,
                required: None,
                archived: None,
                options: Some(options),
            },
            None,
        )
        .await
        .unwrap();
        let now = chrono::Utc::now();
        let actor = crate::models::crm_record::CrmActor::human(&owner);
        let records = [("Dollar variable", "new"), ("Dollar field", "qualified")]
            .into_iter()
            .map(|(name, stage)| CrmRecord {
                field_sources: mongodb::bson::Document::new(),
                id: uuid::Uuid::new_v4().to_string(),
                owner_id: owner.clone(),
                object_id: opportunity.id.clone(),
                object_key: "opportunity".into(),
                revision: 1,
                values: doc! {"name": name, "stage": stage},
                refs: Document::new(),
                overrides: vec![],
                created_by: actor.clone(),
                updated_by: actor.clone(),
                archived_at: None,
                created_at: now,
                updated_at: now,
            });
        db.collection::<CrmRecord>(RECORDS)
            .insert_many(records)
            .await
            .unwrap();

        let result = aggregate(
            &db,
            &owner,
            &CrmAggregateQuery {
                date_bucket: None,
                object: "opportunity".into(),
                filters: vec![],
                group_by: Some("stage".into()),
                measure: AggregateMeasure::Count,
                order: AggregateOrder::LabelAsc,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            result
                .points
                .iter()
                .map(|point| point.label.as_str())
                .collect::<Vec<_>>(),
            vec!["$$crm_missing_variable", "$Revenue"]
        );
        assert_eq!(
            result
                .points
                .iter()
                .map(|point| point.value.clone())
                .collect::<Vec<_>>(),
            vec![json!(1), json!(1)]
        );
    }

    #[tokio::test]
    async fn aggregate_label_lookups_project_and_discard_joined_rows() {
        let db = crate::test_utils::connect_transaction_test_database(
            "crm_aggregate_label_lookup_projection",
        )
        .await;
        super::super::ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        objects::ensure_seeded(&db, &owner).await.unwrap();

        let opportunity = objects::get(&db, &owner, "opportunity").await.unwrap();
        let record_query = CrmAggregateQuery {
            date_bucket: None,
            object: "opportunity".into(),
            filters: vec![],
            group_by: Some("company".into()),
            measure: AggregateMeasure::Count,
            order: AggregateOrder::LabelAsc,
        };
        let record_filter = build_filter(&owner, &opportunity, &[], false).unwrap();
        let record_pipeline =
            build_aggregate_pipeline(&db, &owner, &opportunity, &record_filter, &record_query)
                .await
                .unwrap();
        let record_lookup = record_pipeline
            .iter()
            .find_map(|stage| stage.get_document("$lookup").ok())
            .filter(|lookup| lookup.get_str("from") == Ok(RECORDS))
            .unwrap();
        let record_project = record_lookup
            .get_array("pipeline")
            .unwrap()
            .iter()
            .filter_map(Bson::as_document)
            .find_map(|stage| stage.get_document("$project").ok())
            .unwrap();
        assert_eq!(record_project.len(), 3);
        assert_eq!(record_project.get_i64("_id"), Ok(0));
        assert_eq!(record_project.get_i64("archived_at"), Ok(1));
        assert_eq!(record_project.get_str("label"), Ok("$values.name"));
        assert!(!record_project.contains_key("values"));
        let record_unset = record_pipeline
            .iter()
            .position(|stage| stage.get_str("$unset") == Ok("label_row"))
            .unwrap();
        let record_sort = record_pipeline
            .iter()
            .position(|stage| stage.contains_key("$sort"))
            .unwrap();
        let record_facet = record_pipeline
            .iter()
            .position(|stage| stage.contains_key("$facet"))
            .unwrap();
        assert!(record_unset < record_sort && record_unset < record_facet);

        let company = objects::get(&db, &owner, "company").await.unwrap();
        let user_query = CrmAggregateQuery {
            date_bucket: None,
            object: "company".into(),
            filters: vec![],
            group_by: Some("owner".into()),
            measure: AggregateMeasure::Count,
            order: AggregateOrder::LabelAsc,
        };
        let user_filter = build_filter(&owner, &company, &[], false).unwrap();
        let user_pipeline =
            build_aggregate_pipeline(&db, &owner, &company, &user_filter, &user_query)
                .await
                .unwrap();
        let user_lookup = user_pipeline
            .iter()
            .find_map(|stage| stage.get_document("$lookup").ok())
            .filter(|lookup| lookup.get_str("from") == Ok(crate::models::user::COLLECTION_NAME))
            .unwrap();
        let user_project = user_lookup
            .get_array("pipeline")
            .unwrap()
            .iter()
            .filter_map(Bson::as_document)
            .find_map(|stage| stage.get_document("$project").ok())
            .unwrap();
        assert_eq!(user_project.len(), 2);
        assert_eq!(user_project.get_i64("_id"), Ok(0));
        // Only an authorized, active person's display label may leave the join;
        // the expression also supplies the same blank-name fallback as people::lookup.
        let display_name = user_project.get_document("display_name").unwrap();
        let choices = display_name.get_array("$cond").unwrap();
        assert_eq!(choices[1].as_str(), Some("$display_name"));
        assert_eq!(choices[2].as_str(), Some("Member"));
        let user_unset = user_pipeline
            .iter()
            .position(|stage| stage.get_str("$unset") == Ok("label_row"))
            .unwrap();
        let user_sort = user_pipeline
            .iter()
            .position(|stage| stage.contains_key("$sort"))
            .unwrap();
        let user_facet = user_pipeline
            .iter()
            .position(|stage| stage.contains_key("$facet"))
            .unwrap();
        assert!(user_unset < user_sort && user_unset < user_facet);
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
        let options: Vec<crate::models::crm_object::CrmSelectOption> = (0..52)
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
                field_sources: mongodb::bson::Document::new(),
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
        rows[50].values.insert("score", -5.0);
        rows[51].values.remove("score");
        rows.push(CrmRecord {
            field_sources: mongodb::bson::Document::new(),
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
            field_sources: mongodb::bson::Document::new(),
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
                date_bucket: None,
                object: "company".into(),
                filters: vec![],
                group_by: Some("bucket".into()),
                measure: AggregateMeasure::Count,
                order: AggregateOrder::OptionOrder,
            },
        )
        .await
        .unwrap();
        assert_eq!(grouped.total, json!(52));
        assert!(grouped.truncated);
        assert_eq!(grouped.points.len(), 51);
        assert_eq!(grouped.points[0].key, json!("option_00"));
        assert_eq!(grouped.points[0].value, json!(1));
        assert!(grouped.points[0].archived);
        assert_eq!(grouped.points[50].key, json!("__other__"));
        assert_eq!(grouped.points[50].value, json!(2));
        assert_eq!(
            grouped
                .points
                .iter()
                .map(|point| point.value.as_i64().unwrap())
                .sum::<i64>(),
            52
        );

        for (measure, expected_total) in [
            (
                AggregateMeasure::Min {
                    field: "score".into(),
                },
                -5.0,
            ),
            (
                AggregateMeasure::Max {
                    field: "score".into(),
                },
                49.5,
            ),
        ] {
            let extrema = aggregate(
                &db,
                &owner,
                &CrmAggregateQuery {
                    date_bucket: None,
                    object: "company".into(),
                    filters: vec![],
                    group_by: Some("bucket".into()),
                    measure,
                    order: AggregateOrder::OptionOrder,
                },
            )
            .await
            .unwrap();
            assert_eq!(extrema.points[50].value, json!(-5.0));
            assert_eq!(extrema.total, json!(expected_total));
        }

        let average = aggregate(
            &db,
            &owner,
            &CrmAggregateQuery {
                date_bucket: None,
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
        let average = average.total.as_f64().unwrap();
        assert!((average - (1245.0 / 51.0)).abs() < f64::EPSILON);
    }
}
