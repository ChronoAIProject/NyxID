use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use regex::Regex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use uuid::Uuid;

use crate::errors::{AppError, AppResult, CrmValidationIssue};
use crate::models::{
    crm_record::CrmActor,
    crm_saved_view::{COLLECTION_NAME as VIEWS, CrmSavedView},
};

use super::records::{
    ReceiptContext, insert_receipt, lookup_receipt, receipt_in_session, recover_duplicate,
};
use super::{objects, query};

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
    pub created_by: CrmActor,
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

pub async fn list(db: &Database, owner_id: &str) -> AppResult<Vec<ViewResponse>> {
    let rows: Vec<CrmSavedView> = db
        .collection::<CrmSavedView>(VIEWS)
        .find(doc! {"owner_id": owner_id})
        .sort(doc! {"updated_at": -1, "_id": -1})
        .await?
        .try_collect()
        .await?;
    let mut responses = Vec::with_capacity(rows.len());
    for row in rows {
        responses.push(response(db, row).await);
    }
    Ok(responses)
}

pub async fn get(db: &Database, owner_id: &str, id: &str) -> AppResult<ViewResponse> {
    let row = db
        .collection::<CrmSavedView>(VIEWS)
        .find_one(doc! {"_id": id, "owner_id": owner_id})
        .await?
        .ok_or_else(|| AppError::NotFound("CRM view not found".into()))?;
    Ok(response(db, row).await)
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
    validate_for_save(db, owner_id, &input.envelope).await?;
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
                if let Some(value) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return serde_json::from_value(value).map_err(|_| {
                        AppError::Internal("Stored CRM view response is invalid".into())
                    });
                }
                let now = Utc::now();
                let row = CrmSavedView {
                    id: Uuid::new_v4().to_string(),
                    owner_id: owner.clone(),
                    title: input.title.clone(),
                    revision: 1,
                    envelope: input.envelope.clone(),
                    created_by: actor.clone(),
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
    validate_for_save(db, owner_id, &input.envelope).await?;
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
                if let Some(value) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return serde_json::from_value(value).map_err(|_| {
                        AppError::Internal("Stored CRM view response is invalid".into())
                    });
                }
                let current = db
                    .collection::<CrmSavedView>(VIEWS)
                    .find_one(doc! {"_id": id, "owner_id": owner})
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(|| AppError::NotFound("CRM view not found".into()))?;
                if current.revision != input.expected_revision {
                    return Err(view_revision_conflict(current));
                }
                let now = Utc::now();
                let updated = db
                    .collection::<CrmSavedView>(VIEWS)
                    .update_one(
                        doc! {"_id": id, "owner_id": owner, "revision": input.expected_revision},
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
                if let Some(value) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return serde_json::from_value(value).map_err(|_| {
                        AppError::Internal("Stored CRM view response is invalid".into())
                    });
                }
                let deleted = db
                    .collection::<CrmSavedView>(VIEWS)
                    .delete_one(doc! {"_id": id, "owner_id": owner})
                    .session(&mut *tx)
                    .await?;
                if deleted.deleted_count != 1 {
                    return Err(AppError::NotFound("CRM view not found".into()));
                }
                let now = Utc::now();
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

pub async fn validate(db: &Database, owner_id: &str, envelope: &Value) -> Vec<CrmValidationIssue> {
    let parsed: Envelope = match serde_json::from_value(envelope.clone()) {
        Ok(parsed) => parsed,
        Err(error) => {
            return vec![issue("envelope", format!("invalid envelope: {error}"))];
        }
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
            ViewQuery::Table { query } => query::validate_table_query(db, owner_id, query).await,
            ViewQuery::Aggregate { query } => {
                query::validate_aggregate_query(db, owner_id, query).await
            }
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
    validate_spec(db, owner_id, &parsed, &query_objects, &mut errors).await;
    errors
}

async fn validate_spec(
    db: &Database,
    owner_id: &str,
    envelope: &Envelope,
    query_objects: &BTreeMap<&str, crate::models::crm_object::CrmObject>,
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
    for (id, element) in &envelope.spec.elements {
        if !valid_id(id) {
            errors.push(issue(
                format!("spec.elements.{id}"),
                "element ids must match ^[A-Za-z][A-Za-z0-9_-]{0,63}$",
            ));
        }
        for (index, child) in element.children.iter().enumerate() {
            if !envelope.spec.elements.contains_key(child) {
                errors.push(issue(
                    format!("spec.elements.{id}.children.{index}"),
                    "must reference an existing element",
                ));
            }
        }
        validate_element(db, owner_id, id, element, envelope, query_objects, errors).await;
    }
    if envelope.spec.elements.contains_key(&envelope.spec.root) {
        let mut reachable = HashSet::new();
        let mut stack = HashSet::new();
        walk(
            &envelope.spec.root,
            &envelope.spec.elements,
            1,
            &mut reachable,
            &mut stack,
            errors,
        );
        for id in envelope.spec.elements.keys() {
            if !reachable.contains(id.as_str()) {
                errors.push(issue(
                    format!("spec.elements.{id}"),
                    "must be reachable from spec.root",
                ));
            }
        }
    }
}

async fn validate_element(
    db: &Database,
    owner_id: &str,
    id: &str,
    element: &ViewElement,
    envelope: &Envelope,
    query_objects: &BTreeMap<&str, crate::models::crm_object::CrmObject>,
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
                    Some(ViewQuery::Aggregate { query }) if query.group_by.is_some() => {}
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
                    Some(ViewQuery::Table { .. }) => {
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
                    Some(ViewQuery::Table { .. }) => {
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
                match objects::get(db, owner_id, &props.object).await {
                    Ok(object) => {
                        if let Some(fields) = &props.fields {
                            errors.extend(query::validate_live_fields(
                                &object,
                                fields,
                                &format!("{base}.fields"),
                            ));
                        }
                    }
                    Err(_) => errors.push(issue(format!("{base}.object"), "object is unavailable")),
                }
            }
        }
        _ => errors.push(issue(
            format!("spec.elements.{id}.type"),
            "unknown component type",
        )),
    }
}

fn walk<'a>(
    id: &'a str,
    elements: &'a BTreeMap<String, ViewElement>,
    depth: usize,
    reachable: &mut HashSet<&'a str>,
    stack: &mut HashSet<&'a str>,
    errors: &mut Vec<CrmValidationIssue>,
) {
    if depth > MAX_DEPTH {
        errors.push(issue(
            format!("spec.elements.{id}"),
            "element depth must not exceed 8",
        ));
        return;
    }
    if !stack.insert(id) {
        errors.push(issue(
            format!("spec.elements.{id}"),
            "children must not contain a cycle",
        ));
        return;
    }
    reachable.insert(id);
    if let Some(element) = elements.get(id) {
        for child in &element.children {
            if elements.contains_key(child) {
                walk(child, elements, depth + 1, reachable, stack, errors);
            }
        }
    }
    stack.remove(id);
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
        created_by: row.created_by,
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
        created_by: row.created_by,
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
