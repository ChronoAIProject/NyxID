use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::OnceLock;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use regex::Regex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use uuid::Uuid;

use crate::errors::{AppError, AppResult, CrmValidationIssue};
use crate::models::{
    crm_record::CrmActor,
    crm_saved_view::{COLLECTION_NAME as VIEWS, CrmSavedView},
};

use super::query;
use super::records::{
    ReceiptContext, insert_receipt, lookup_receipt, receipt_in_session, recover_duplicate,
};
use crate::models::crm_object::{COLLECTION_NAME as OBJECTS, CrmObject};

const MAX_ELEMENTS: usize = 60;
const MAX_DEPTH: usize = 8;
const MAX_QUERIES: usize = 12;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateViewInput {
    pub title: String,
    pub envelope: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateViewInput {
    pub expected_revision: i64,
    pub title: String,
    pub envelope: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewResponse {
    pub id: String,
    pub owner_id: String,
    pub title: String,
    pub revision: i64,
    pub envelope: Value,
    pub created_by: super::activities::ActivityActorResponse,
    pub valid: bool,
    pub errors: Vec<CrmValidationIssue>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteViewResponse {
    pub id: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    #[serde(rename = "formatVersion")]
    format_version: u32,
    renderer: String,
    #[serde(rename = "rendererSpecVersion")]
    renderer_spec_version: String,
    queries: BTreeMap<String, ViewQuery>,
    spec: ViewSpec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ViewQuery {
    Table { query: query::CrmTableQuery },
    Aggregate { query: query::CrmAggregateQuery },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewSpec {
    root: String,
    elements: BTreeMap<String, ViewElement>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewElement {
    #[serde(rename = "type")]
    element_type: String,
    props: Value,
    #[serde(default)]
    children: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StackProps {
    gap: Gap,
    direction: Option<Direction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Gap {
    Sm,
    Md,
    Lg,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Vertical,
    Horizontal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GridProps {
    columns: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextProps {
    content: String,
    variant: TextVariant,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TextVariant {
    Title,
    Heading,
    Body,
    Muted,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricProps {
    #[serde(rename = "queryId")]
    query_id: String,
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChartProps {
    #[serde(rename = "queryId")]
    query_id: String,
    title: String,
    height: Option<ChartHeight>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChartHeight {
    Small,
    Medium,
    Large,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectTableProps {
    #[serde(rename = "queryId")]
    query_id: String,
    columns: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PipelineBoardProps {
    #[serde(rename = "queryId")]
    query_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordFormProps {
    object: String,
    fields: Option<Vec<String>>,
    title: String,
    #[serde(rename = "submitLabel")]
    submit_label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ViewPage {
    pub views: Vec<ViewResponse>,
    pub next_cursor: Option<String>,
}

pub async fn list_page(db: &Database, owner_id: &str, cursor: Option<&str>) -> AppResult<ViewPage> {
    let mut filter = doc! {"owner_id": owner_id, "deleted_at": mongodb::bson::Bson::Null};
    if let Some(cursor) = cursor {
        Uuid::parse_str(cursor).map_err(|_| super::validation_error("cursor", "is invalid"))?;
        filter.insert("_id", doc! {"$gt": cursor});
    }
    let rows: Vec<CrmSavedView> = db
        .collection::<CrmSavedView>(VIEWS)
        .find(filter)
        .sort(doc! {"_id": 1})
        .limit(21)
        .max_time(std::time::Duration::from_secs(10))
        .await?
        .try_collect()
        .await?;
    let has_more = rows.len() > 20;
    let mut rows = rows;
    rows.truncate(20);
    let next_cursor = has_more
        .then(|| rows.last().map(|row| row.id.clone()))
        .flatten();
    let metadata = load_metadata(db, owner_id).await?;
    let mut responses = Vec::with_capacity(rows.len());
    for row in rows {
        let errors = validate_metadata(owner_id, &row.envelope, &metadata);
        let mut item = valid_response(row);
        item.valid = errors.is_empty();
        item.errors = errors;
        responses.push(item);
    }
    Ok(ViewPage {
        views: responses,
        next_cursor,
    })
}

pub async fn get(db: &Database, owner_id: &str, id: &str) -> AppResult<ViewResponse> {
    let row = db
        .collection::<CrmSavedView>(VIEWS)
        .find_one(doc! {"_id": id, "owner_id": owner_id, "deleted_at": mongodb::bson::Bson::Null})
        .await?
        .ok_or_else(|| AppError::NotFound("CRM view not found".into()))?;
    Ok(response(db, row).await)
}

/// Authorizes deletion retries against the retained row; ordinary reads still
/// exclude soft-deleted views. The delete service alone decides receipt replay.
pub async fn for_delete(db: &Database, owner_id: &str, id: &str) -> AppResult<CrmSavedView> {
    db.collection::<CrmSavedView>(VIEWS)
        .find_one(doc! {"_id": id, "owner_id": owner_id})
        .await?
        .ok_or_else(|| AppError::NotFound("CRM view not found".into()))
}

pub async fn raw_by_id(db: &Database, id: &str) -> AppResult<CrmSavedView> {
    db.collection::<CrmSavedView>(VIEWS)
        .find_one(doc! {"_id": id})
        .await?
        .ok_or_else(|| AppError::NotFound("CRM view not found".into()))
}

pub async fn create(
    db: &Database,
    owner_id: &str,
    actor: CrmActor,
    input: CreateViewInput,
    receipt: Option<ReceiptContext>,
) -> AppResult<ViewResponse> {
    if let Some(response) = lookup_receipt(db, receipt.as_ref()).await? {
        return Ok(response);
    }
    validate_title(&input.title)?;
    let context = (
        db.clone(),
        owner_id.to_owned(),
        actor,
        input,
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, actor, input, receipt)| {
            Box::pin(async move {
                super::fence_owner(db, owner, tx).await?;
                if let Some(value) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return serde_json::from_value(value).map_err(|_| {
                        AppError::Internal("Stored CRM view response is invalid".into())
                    });
                }
                let now = Utc::now();
                // Every CRM schema writer holds this same owner fence until commit.
                validate_for_save(db, owner, &input.envelope).await?;
                let row = CrmSavedView {
                    id: Uuid::new_v4().to_string(),
                    owner_id: owner.clone(),
                    title: input.title.clone(),
                    revision: 1,
                    envelope: input.envelope.clone(),
                    created_by: actor.clone(),
                    deleted_at: None,
                    created_at: now,
                    updated_at: now,
                };
                db.collection::<CrmSavedView>(VIEWS)
                    .insert_one(&row)
                    .session(&mut *tx)
                    .await?;
                let response = valid_response(row);
                insert_receipt(db, tx, receipt.as_ref(), &response, now).await?;
                Ok(response)
            })
        },
    )
    .await;
    recover_duplicate(db, receipt.as_ref(), result).await
}

pub async fn update(
    db: &Database,
    owner_id: &str,
    id: &str,
    input: UpdateViewInput,
    receipt: Option<ReceiptContext>,
) -> AppResult<ViewResponse> {
    if let Some(response) = lookup_receipt(db, receipt.as_ref()).await? {
        return Ok(response);
    }
    validate_title(&input.title)?;
    let context = (
        db.clone(),
        owner_id.to_owned(),
        id.to_owned(),
        input,
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, id, input, receipt)| {
            Box::pin(async move {
                super::fence_owner(db, owner, tx).await?;
                if let Some(value) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return serde_json::from_value(value).map_err(|_| {
                        AppError::Internal("Stored CRM view response is invalid".into())
                    });
                }
                let current = db
                    .collection::<CrmSavedView>(VIEWS)
                    .find_one(doc! {"_id": id, "owner_id": owner, "deleted_at": mongodb::bson::Bson::Null})
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(|| AppError::NotFound("CRM view not found".into()))?;
                if current.revision != input.expected_revision {
                    return Err(view_revision_conflict(current));
                }
                validate_for_save(db, owner, &input.envelope).await?;
                let now = Utc::now();
                let updated = db
                    .collection::<CrmSavedView>(VIEWS)
                    .update_one(
                            doc! {"_id": id, "owner_id": owner, "revision": input.expected_revision, "deleted_at": mongodb::bson::Bson::Null},
                        doc! {"$set": {"title": &input.title, "envelope": bson::to_bson(&input.envelope).map_err(|error| AppError::Internal(error.to_string()))?, "updated_at": bson::DateTime::from_chrono(now)}, "$inc": {"revision": 1_i64}},
                    )
                    .session(&mut *tx)
                    .await?;
                if updated.modified_count != 1 {
                    let current = db
                        .collection::<CrmSavedView>(VIEWS)
                        .find_one(doc! {"_id": id, "owner_id": owner})
                        .session(&mut *tx)
                        .await?
                        .ok_or_else(|| AppError::NotFound("CRM view not found".into()))?;
                    return Err(view_revision_conflict(current));
                }
                let row = CrmSavedView {
                    title: input.title.clone(),
                    envelope: input.envelope.clone(),
                    revision: current.revision + 1,
                    updated_at: now,
                    ..current
                };
                let response = valid_response(row);
                insert_receipt(db, tx, receipt.as_ref(), &response, now).await?;
                Ok(response)
            })
        },
    )
    .await;
    recover_duplicate(db, receipt.as_ref(), result).await
}

pub async fn delete(
    db: &Database,
    owner_id: &str,
    id: &str,
    receipt: Option<ReceiptContext>,
) -> AppResult<DeleteViewResponse> {
    if let Some(response) = lookup_receipt(db, receipt.as_ref()).await? {
        return Ok(response);
    }
    let context = (
        db.clone(),
        owner_id.to_owned(),
        id.to_owned(),
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, id, receipt)| {
            Box::pin(async move {
                super::fence_owner(db, owner, tx).await?;
                if let Some(value) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return serde_json::from_value(value).map_err(|_| {
                        AppError::Internal("Stored CRM view response is invalid".into())
                    });
                }
                let now = Utc::now();
                let deleted = db
                    .collection::<CrmSavedView>(VIEWS)
                    .update_one(
                        doc! {"_id": id, "owner_id": owner, "deleted_at": mongodb::bson::Bson::Null},
                        doc! {
                            "$set": {
                                "deleted_at": bson::DateTime::from_chrono(now),
                                "updated_at": bson::DateTime::from_chrono(now),
                            },
                            "$inc": {"revision": 1_i64},
                        },
                    )
                    .session(&mut *tx)
                    .await?;
                if deleted.modified_count != 1 {
                    return Err(AppError::NotFound("CRM view not found".into()));
                }
                let response = DeleteViewResponse {
                    id: id.clone(),
                    deleted: true,
                };
                insert_receipt(db, tx, receipt.as_ref(), &response, now).await?;
                Ok(response)
            })
        },
    )
    .await;
    recover_duplicate(db, receipt.as_ref(), result).await
}

async fn load_metadata(db: &Database, owner_id: &str) -> AppResult<BTreeMap<String, CrmObject>> {
    let rows: Vec<CrmObject> = db
        .collection::<CrmObject>(OBJECTS)
        .find(doc! {"owner_id": owner_id})
        .limit(100)
        .max_time(std::time::Duration::from_secs(10))
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .map(|object| (object.key.clone(), object))
        .collect())
}

fn parse_bounded(envelope: &Value) -> Result<Envelope, Vec<CrmValidationIssue>> {
    if envelope.to_string().len() > 65_536 {
        return Err(vec![issue("envelope", "must not exceed 65536 bytes")]);
    }
    let parsed: Envelope = serde_json::from_value(envelope.clone())
        .map_err(|error| vec![issue("envelope", format!("invalid envelope: {error}"))])?;
    if parsed.queries.len() > MAX_QUERIES
        || parsed.spec.elements.len() > MAX_ELEMENTS
        || parsed
            .spec
            .elements
            .values()
            .map(|e| e.children.len())
            .sum::<usize>()
            > MAX_ELEMENTS
    {
        return Err(vec![issue(
            "envelope",
            "at most 12 queries, 60 elements and 60 edges are allowed",
        )]);
    }
    if parsed
        .spec
        .elements
        .values()
        .any(|e| e.props.to_string().len() > 8192)
    {
        return Err(vec![issue(
            "spec.elements",
            "component props must not exceed 8192 bytes",
        )]);
    }
    Ok(parsed)
}

pub async fn validate(db: &Database, owner_id: &str, envelope: &Value) -> Vec<CrmValidationIssue> {
    if let Err(errors) = parse_bounded(envelope) {
        return errors;
    }
    match load_metadata(db, owner_id).await {
        Ok(metadata) => validate_metadata(owner_id, envelope, &metadata),
        Err(_) => vec![issue("envelope", "object metadata is unavailable")],
    }
}

fn validate_metadata(
    owner_id: &str,
    envelope: &Value,
    metadata: &BTreeMap<String, CrmObject>,
) -> Vec<CrmValidationIssue> {
    let parsed = match parse_bounded(envelope) {
        Ok(parsed) => parsed,
        Err(errors) => return errors,
    };
    let mut errors = Vec::new();
    if parsed.format_version != 1 {
        errors.push(issue("formatVersion", "must be 1"));
    }
    if parsed.renderer != "json-render" {
        errors.push(issue("renderer", "must be json-render"));
    }
    if parsed.renderer_spec_version != "nyxid-crm-v1" {
        errors.push(issue("rendererSpecVersion", "must be nyxid-crm-v1"));
    }
    if parsed.queries.len() > MAX_QUERIES {
        errors.push(issue("queries", "must contain at most 12 queries"));
    }
    let mut query_objects = BTreeMap::new();
    for (id, view_query) in &parsed.queries {
        if !valid_id(id) {
            errors.push(issue(
                format!("queries.{id}"),
                "query ids must match ^[A-Za-z][A-Za-z0-9_-]{0,63}$",
            ));
        }
        let result = match view_query {
            ViewQuery::Table { query } => metadata
                .get(&query.object)
                .cloned()
                .ok_or_else(|| AppError::NotFound("object".into()))
                .and_then(|object| {
                    query::validate_table_metadata(owner_id, object, query).map(|v| v.object)
                }),
            ViewQuery::Aggregate { query } => metadata
                .get(&query.object)
                .cloned()
                .ok_or_else(|| AppError::NotFound("object".into()))
                .and_then(|object| {
                    query::validate_aggregate_metadata(owner_id, object, query).map(|v| v.object)
                }),
        };
        match result {
            Ok(object) => {
                query_objects.insert(id.as_str(), object);
            }
            Err(AppError::CrmValidationFailed(query_errors)) => {
                for error in query_errors {
                    errors.push(issue(
                        format!("queries.{id}.query.{}", error.path),
                        error.message,
                    ));
                }
            }
            Err(AppError::NotFound(_)) => errors.push(issue(
                format!("queries.{id}.query.object"),
                "object is unavailable",
            )),
            Err(_) => errors.push(issue(
                format!("queries.{id}.query"),
                "query metadata could not be validated",
            )),
        }
    }
    validate_spec(&parsed, &query_objects, metadata, &mut errors);
    errors
}

fn validate_spec(
    envelope: &Envelope,
    query_objects: &BTreeMap<&str, crate::models::crm_object::CrmObject>,
    metadata: &BTreeMap<String, CrmObject>,
    errors: &mut Vec<CrmValidationIssue>,
) {
    if envelope.spec.elements.len() > MAX_ELEMENTS {
        errors.push(issue("spec.elements", "must contain at most 60 elements"));
    }
    if !valid_id(&envelope.spec.root) {
        errors.push(issue(
            "spec.root",
            "must match ^[A-Za-z][A-Za-z0-9_-]{0,63}$",
        ));
    }
    if !envelope.spec.elements.contains_key(&envelope.spec.root) {
        errors.push(issue("spec.root", "must reference an existing element"));
    }
    let mut edge_count = 0_usize;
    let mut parent_counts: HashMap<&str, usize> = HashMap::new();
    for (id, element) in &envelope.spec.elements {
        if !valid_id(id) {
            errors.push(issue(
                format!("spec.elements.{id}"),
                "element ids must match ^[A-Za-z][A-Za-z0-9_-]{0,63}$",
            ));
        }
        if !matches!(element.element_type.as_str(), "Stack" | "Grid")
            && !element.children.is_empty()
        {
            errors.push(issue(
                format!("spec.elements.{id}.children"),
                "only Stack and Grid elements may have children",
            ));
        }
        edge_count = edge_count.saturating_add(element.children.len());
        let mut children = HashSet::new();
        for (index, child) in element.children.iter().enumerate() {
            if !children.insert(child.as_str()) {
                errors.push(issue(
                    format!("spec.elements.{id}.children.{index}"),
                    "child ids must not be duplicated",
                ));
                continue;
            }
            if !envelope.spec.elements.contains_key(child) {
                errors.push(issue(
                    format!("spec.elements.{id}.children.{index}"),
                    "must reference an existing element",
                ));
            } else {
                *parent_counts.entry(child).or_default() += 1;
            }
        }
        validate_element(id, element, envelope, query_objects, metadata, errors);
    }
    if edge_count > MAX_ELEMENTS {
        errors.push(issue(
            "spec.elements",
            "must contain at most 60 child edges",
        ));
    }
    for id in envelope.spec.elements.keys() {
        let parents = parent_counts.get(id.as_str()).copied().unwrap_or(0);
        if id == &envelope.spec.root {
            if parents != 0 {
                errors.push(issue(
                    format!("spec.elements.{id}"),
                    "spec.root must not be a child",
                ));
            }
        } else if parents != 1 {
            errors.push(issue(
                format!("spec.elements.{id}"),
                "every non-root element must have exactly one parent",
            ));
        }
    }
    if envelope.spec.elements.contains_key(&envelope.spec.root) {
        validate_tree_reachability(envelope, errors);
    }
}

fn validate_element(
    id: &str,
    element: &ViewElement,
    envelope: &Envelope,
    query_objects: &BTreeMap<&str, crate::models::crm_object::CrmObject>,
    metadata: &BTreeMap<String, CrmObject>,
    errors: &mut Vec<CrmValidationIssue>,
) {
    let base = format!("spec.elements.{id}.props");
    match element.element_type.as_str() {
        "Stack" => {
            if let Some(props) = props::<StackProps>(&element.props, &base, errors) {
                let _ = (props.gap, props.direction);
            }
        }
        "Grid" => {
            if let Some(props) = props::<GridProps>(&element.props, &base, errors)
                && !(1..=4).contains(&props.columns)
            {
                errors.push(issue(format!("{base}.columns"), "must be between 1 and 4"));
            }
        }
        "Text" => {
            if let Some(props) = props::<TextProps>(&element.props, &base, errors) {
                if props.content.is_empty() || props.content.chars().count() > 2000 {
                    errors.push(issue(
                        format!("{base}.content"),
                        "must contain 1 to 2000 characters",
                    ));
                }
                let _ = props.variant;
            }
        }
        "Metric" => {
            if let Some(props) = props::<MetricProps>(&element.props, &base, errors) {
                validate_title_prop(&props.title, &format!("{base}.title"), errors);
                match envelope.queries.get(&props.query_id) {
                    Some(ViewQuery::Aggregate { query }) if query.group_by.is_none() => {}
                    Some(_) => errors.push(issue(
                        format!("{base}.queryId"),
                        "must reference an aggregate query without group_by",
                    )),
                    None => errors.push(issue(
                        format!("{base}.queryId"),
                        "must reference an existing query",
                    )),
                }
            }
        }
        "BarChart" | "PieChart" => {
            if let Some(props) = props::<ChartProps>(&element.props, &base, errors) {
                validate_title_prop(&props.title, &format!("{base}.title"), errors);
                match envelope.queries.get(&props.query_id) {
                    Some(ViewQuery::Aggregate { query }) if query.group_by.is_some() => {
                        if element.element_type == "PieChart"
                            && !matches!(
                                query.measure,
                                query::AggregateMeasure::Count
                                    | query::AggregateMeasure::Sum { .. }
                            )
                        {
                            errors.push(issue(
                                format!("{base}.queryId"),
                                "pie charts require count or sum measures",
                            ));
                        }
                    }
                    Some(_) => errors.push(issue(
                        format!("{base}.queryId"),
                        "must reference a grouped aggregate query",
                    )),
                    None => errors.push(issue(
                        format!("{base}.queryId"),
                        "must reference an existing query",
                    )),
                }
                let _ = props.height;
            }
        }
        "ObjectTable" => {
            if let Some(props) = props::<ObjectTableProps>(&element.props, &base, errors) {
                match envelope.queries.get(&props.query_id) {
                    Some(ViewQuery::Table { query }) => {
                        if let (Some(columns), Some(select)) = (&props.columns, &query.select) {
                            if columns.iter().any(|column| !select.contains(column)) {
                                errors.push(issue(
                                    format!("{base}.columns"),
                                    "displayed columns must be included in query.select",
                                ));
                            }
                        }
                        if let Some(columns) = &props.columns
                            && let Some(object) = query_objects.get(props.query_id.as_str())
                        {
                            errors.extend(query::validate_live_fields(
                                object,
                                columns,
                                &format!("{base}.columns"),
                            ));
                        }
                    }
                    Some(_) => errors.push(issue(
                        format!("{base}.queryId"),
                        "must reference a table query",
                    )),
                    None => errors.push(issue(
                        format!("{base}.queryId"),
                        "must reference an existing query",
                    )),
                }
            }
        }
        "PipelineBoard" => {
            if let Some(props) = props::<PipelineBoardProps>(&element.props, &base, errors) {
                match envelope.queries.get(&props.query_id) {
                    Some(ViewQuery::Table { query }) => {
                        if query.include_archived {
                            errors.push(issue(
                                format!("{base}.queryId"),
                                "pipeline boards require include_archived=false",
                            ));
                        }
                        if query_objects
                            .get(props.query_id.as_str())
                            .is_some_and(|object| object.pipeline.is_none())
                        {
                            errors.push(issue(
                                format!("{base}.queryId"),
                                "must query an object with a pipeline",
                            ));
                        }
                    }
                    Some(_) => errors.push(issue(
                        format!("{base}.queryId"),
                        "must reference a table query",
                    )),
                    None => errors.push(issue(
                        format!("{base}.queryId"),
                        "must reference an existing query",
                    )),
                }
            }
        }
        "RecordForm" => {
            if let Some(props) = props::<RecordFormProps>(&element.props, &base, errors) {
                validate_title_prop(&props.title, &format!("{base}.title"), errors);
                if props
                    .submit_label
                    .as_ref()
                    .is_some_and(|label| label.is_empty() || label.chars().count() > 100)
                {
                    errors.push(issue(
                        format!("{base}.submitLabel"),
                        "must contain 1 to 100 characters",
                    ));
                }
                match metadata.get(&props.object) {
                    Some(object) => {
                        if let Some(fields) = &props.fields {
                            errors.extend(query::validate_live_fields(
                                &object,
                                fields,
                                &format!("{base}.fields"),
                            ));
                        }
                    }
                    None => errors.push(issue(format!("{base}.object"), "object is unavailable")),
                }
            }
        }
        _ => errors.push(issue(
            format!("spec.elements.{id}.type"),
            "unknown component type",
        )),
    }
}

fn validate_tree_reachability(envelope: &Envelope, errors: &mut Vec<CrmValidationIssue>) {
    let mut reachable = HashSet::new();
    let mut pending = VecDeque::from([(envelope.spec.root.as_str(), 1_usize)]);
    while let Some((id, depth)) = pending.pop_front() {
        if !reachable.insert(id) {
            errors.push(issue(
                format!("spec.elements.{id}"),
                "children must not contain a cycle",
            ));
            continue;
        }
        if depth > MAX_DEPTH {
            errors.push(issue(
                format!("spec.elements.{id}"),
                "element depth must not exceed 8",
            ));
            continue;
        }
        if let Some(element) = envelope.spec.elements.get(id) {
            for child in &element.children {
                if envelope.spec.elements.contains_key(child) {
                    pending.push_back((child, depth + 1));
                }
            }
        }
    }
    for id in envelope.spec.elements.keys() {
        if !reachable.contains(id.as_str()) {
            errors.push(issue(
                format!("spec.elements.{id}"),
                "must be reachable from spec.root",
            ));
        }
    }
}

fn props<T: DeserializeOwned>(
    value: &Value,
    path: &str,
    errors: &mut Vec<CrmValidationIssue>,
) -> Option<T> {
    match serde_json::from_value(value.clone()) {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(issue(path, format!("invalid component props: {error}")));
            None
        }
    }
}

fn valid_id(value: &str) -> bool {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(r"^[A-Za-z][A-Za-z0-9_-]{0,63}$").expect("fixed CRM view id regex")
        })
        .is_match(value)
}

fn validate_title(title: &str) -> AppResult<()> {
    if title.trim().is_empty() || title.chars().count() > 200 {
        Err(super::validation_error(
            "title",
            "must contain 1 to 200 characters",
        ))
    } else {
        Ok(())
    }
}

fn validate_title_prop(title: &str, path: &str, errors: &mut Vec<CrmValidationIssue>) {
    if title.trim().is_empty() || title.chars().count() > 200 {
        errors.push(issue(path, "must contain 1 to 200 characters"));
    }
}

async fn validate_for_save(db: &Database, owner_id: &str, envelope: &Value) -> AppResult<()> {
    let errors = validate(db, owner_id, envelope).await;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::CrmViewInvalid(errors))
    }
}

async fn response(db: &Database, row: CrmSavedView) -> ViewResponse {
    let errors = validate(db, &row.owner_id, &row.envelope).await;
    let valid = errors.is_empty();
    ViewResponse {
        id: row.id,
        owner_id: row.owner_id,
        title: row.title,
        revision: row.revision,
        envelope: row.envelope,
        created_by: row.created_by.into(),
        valid,
        errors,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

fn valid_response(row: CrmSavedView) -> ViewResponse {
    ViewResponse {
        id: row.id,
        owner_id: row.owner_id,
        title: row.title,
        revision: row.revision,
        envelope: row.envelope,
        created_by: row.created_by.into(),
        valid: true,
        errors: vec![],
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

fn view_revision_conflict(row: CrmSavedView) -> AppError {
    AppError::CrmRevisionConflict(Box::new(
        serde_json::to_value(valid_response(row)).expect("CRM view response is serializable"),
    ))
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> CrmValidationIssue {
    CrmValidationIssue {
        path: path.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::crm::objects;
    use serde_json::json;

    #[test]
    fn ids_are_closed_and_bounded() {
        assert!(valid_id("openDeals_1"));
        assert!(!valid_id("1bad"));
        assert!(!valid_id("has space"));
        assert!(!valid_id(&format!("A{}", "x".repeat(64))));
    }

    #[test]
    fn component_props_reject_unknown_fields_with_a_path() {
        let mut errors = vec![];
        assert!(
            props::<GridProps>(
                &json!({"columns": 2, "unknown": true}),
                "spec.elements.grid.props",
                &mut errors,
            )
            .is_none()
        );
        assert_eq!(errors[0].path, "spec.elements.grid.props");
    }

    #[test]
    fn aggregate_measure_rejects_unknown_fields() {
        assert!(
            serde_json::from_value::<query::AggregateMeasure>(json!({
                "op": "sum",
                "field": "amount",
                "url": "https://example.test"
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn view_graph_must_be_a_bounded_tree_with_container_children_only() {
        let db = crate::test_utils::connect_transaction_test_database("crm_view_tree").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let envelope = json!({
            "formatVersion": 1,
            "renderer": "json-render",
            "rendererSpecVersion": "nyxid-crm-v1",
            "queries": {},
            "spec": {
                "root": "Root",
                "elements": {
                    "Root": {
                        "type": "Stack",
                        "props": {"gap": "md"},
                        "children": ["Leaf", "Leaf", "Shared"]
                    },
                    "Other": {
                        "type": "Grid",
                        "props": {"columns": 1},
                        "children": ["Shared"]
                    },
                    "Leaf": {
                        "type": "Text",
                        "props": {"content": "leaf", "variant": "body"},
                        "children": []
                    },
                    "Shared": {
                        "type": "Text",
                        "props": {"content": "shared", "variant": "body"},
                        "children": ["Leaf"]
                    },
                    "Orphan": {
                        "type": "Text",
                        "props": {"content": "orphan", "variant": "body"},
                        "children": []
                    }
                }
            }
        });
        let errors = validate(&db, &owner, &envelope).await;
        assert!(errors.iter().any(|error| {
            error.path == "spec.elements.Root.children.1"
                && error.message == "child ids must not be duplicated"
        }));
        assert!(errors.iter().any(|error| {
            error.path == "spec.elements.Shared"
                && error.message == "every non-root element must have exactly one parent"
        }));
        assert!(errors.iter().any(|error| {
            error.path == "spec.elements.Orphan"
                && error.message == "must be reachable from spec.root"
        }));
        assert!(errors.iter().any(|error| {
            error.path == "spec.elements.Shared.children"
                && error.message == "only Stack and Grid elements may have children"
        }));

        let mut too_many = envelope;
        too_many["spec"]["elements"]["Root"]["children"] =
            Value::Array((0..61).map(|_| json!("Leaf")).collect());
        let errors = validate(&db, &owner, &too_many).await;
        assert_eq!(
            errors,
            vec![issue(
                "envelope",
                "at most 12 queries, 60 elements and 60 edges are allowed"
            )]
        );
    }

    #[tokio::test]
    async fn invalid_view_reports_paths_and_is_not_saved() {
        let db = crate::test_utils::connect_transaction_test_database("crm_view_contract").await;
        super::super::ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        objects::ensure_seeded(&db, &owner).await.unwrap();
        let envelope = json!({
            "formatVersion": 1,
            "renderer": "json-render",
            "rendererSpecVersion": "nyxid-crm-v1",
            "queries": {
                "1bad": {
                    "kind": "table",
                    "query": {
                        "object": "company",
                        "filters": [],
                        "page_size": 50,
                        "include_archived": false
                    }
                }
            },
            "spec": {
                "root": "root",
                "elements": {
                    "root": {
                        "type": "ObjectTable",
                        "props": {"queryId": "1bad", "columns": ["missing"]},
                        "children": []
                    }
                }
            }
        });
        let errors = validate(&db, &owner, &envelope).await;
        assert!(errors.iter().any(|error| error.path == "queries.1bad"));
        assert!(errors.iter().any(|error| {
            error.path == "spec.elements.root.props.columns.0" && error.message == "unknown field"
        }));

        let record_form = json!({
            "formatVersion": 1,
            "renderer": "json-render",
            "rendererSpecVersion": "nyxid-crm-v1",
            "queries": {},
            "spec": {
                "root": "form",
                "elements": {
                    "form": {
                        "type": "RecordForm",
                        "props": {
                            "object": "company",
                            "fields": ["missing"],
                            "title": "New company"
                        },
                        "children": []
                    }
                }
            }
        });
        let form_errors = validate(&db, &owner, &record_form).await;
        assert!(form_errors.iter().any(|error| {
            error.path == "spec.elements.form.props.fields.0" && error.message == "unknown field"
        }));

        let result = create(
            &db,
            &owner,
            CrmActor::human(&owner),
            CreateViewInput {
                title: "Invalid".into(),
                envelope,
            },
            None,
        )
        .await;
        assert!(matches!(result, Err(AppError::CrmViewInvalid(_))));
        assert_eq!(
            db.collection::<CrmSavedView>(VIEWS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}
