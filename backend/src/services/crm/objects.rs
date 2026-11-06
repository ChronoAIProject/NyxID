use std::collections::{HashMap, HashSet};

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{AppError, AppResult, CrmValidationIssue};
use crate::models::crm_object::{
    COLLECTION_NAME, CrmCardinality, CrmField, CrmFieldKind, CrmObject, CrmPipeline,
    CrmSelectOption,
};

use super::{fields, validation_result};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObjectApi {
    pub id: String,
    pub owner_id: String,
    pub key: String,
    pub label: String,
    pub plural_label: String,
    pub schema_revision: i64,
    pub fields: Vec<CrmField>,
    pub pipeline: Option<CrmPipeline>,
    pub default_columns: Vec<String>,
    pub seeded: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<CrmObject> for ObjectApi {
    fn from(object: CrmObject) -> Self {
        Self {
            id: object.id,
            owner_id: object.owner_id,
            key: object.key,
            label: object.label,
            plural_label: object.plural_label,
            schema_revision: object.schema_revision,
            fields: object.fields,
            pipeline: object.pipeline,
            default_columns: object.default_columns,
            seeded: object.seeded,
            created_at: object.created_at.to_rfc3339(),
            updated_at: object.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PatchField {
    pub expected_schema_revision: i64,
    pub label: Option<String>,
    pub required: Option<bool>,
    pub archived: Option<bool>,
    pub options: Option<Vec<CrmSelectOption>>,
}

pub async fn ensure_seeded(db: &Database, owner_id: &str) -> AppResult<()> {
    let context = (db.clone(), owner_id.to_owned());
    let mut attempts = 0;
    loop {
        let result =
            crate::services::transaction_service::run_db_only(db, &context, |tx, (db, owner)| {
                Box::pin(async move {
                    super::fence_owner(db, owner, tx).await?;
                    for object in seeded_objects(owner, Utc::now()) {
                        let document = bson::to_document(&object)
                            .map_err(|error| AppError::Internal(error.to_string()))?;
                        db.collection::<CrmObject>(COLLECTION_NAME)
                            .update_one(
                                doc! {"owner_id": owner, "key": &object.key},
                                doc! {"$setOnInsert": document},
                            )
                            .upsert(true)
                            .session(&mut *tx)
                            .await?;
                    }
                    Ok(())
                })
            })
            .await;
        match result {
            Err(AppError::DatabaseError(error))
                if attempts < 7
                    && crate::services::assistant_team_service::is_duplicate(&error) =>
            {
                attempts += 1;
                tokio::task::yield_now().await;
            }
            result => return result,
        }
    }
}

pub async fn list(db: &Database, owner_id: &str) -> AppResult<Vec<CrmObject>> {
    ensure_seeded(db, owner_id).await?;
    Ok(db
        .collection::<CrmObject>(COLLECTION_NAME)
        .find(doc! {"owner_id": owner_id})
        .sort(doc! {"key": 1})
        .await?
        .try_collect()
        .await?)
}

pub async fn get(db: &Database, owner_id: &str, key: &str) -> AppResult<CrmObject> {
    db.collection::<CrmObject>(COLLECTION_NAME)
        .find_one(doc! {"owner_id": owner_id, "key": key})
        .await?
        .ok_or_else(|| AppError::NotFound("CRM object not found".into()))
}

pub const MAX_OWNER_OBJECTS: u64 = 50;
pub const RESERVED_OBJECT_KEYS: &[&str] = &["company", "contact", "opportunity", "task"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateObjectInput {
    pub key: String,
    pub label: String,
    pub plural_label: String,
    pub fields: Vec<CrmField>,
    #[serde(default)]
    pub pipeline: Option<CrmPipeline>,
}

fn validate_create(input: &CreateObjectInput) -> AppResult<()> {
    if !fields::valid_key(&input.key) || RESERVED_OBJECT_KEYS.contains(&input.key.as_str()) {
        return Err(super::validation_error(
            "key",
            "must be an unreserved immutable object key",
        ));
    }
    for (path, label) in [
        ("label", &input.label),
        ("plural_label", &input.plural_label),
    ] {
        if label.trim().is_empty()
            || label.chars().count() > 200
            || label.chars().any(char::is_control)
        {
            return Err(super::validation_error(
                path,
                "must contain 1 to 200 printable characters",
            ));
        }
    }
    if input.fields.is_empty() || input.fields.len() > 100 {
        return Err(super::validation_error(
            "fields",
            "must contain 1 to 100 fields",
        ));
    }
    let mut keys = HashSet::new();
    for (index, field) in input.fields.iter().enumerate() {
        validation_result(fields::validate_definition(
            field,
            &format!("fields.{index}"),
        ))?;
        if !keys.insert(&field.key) || field.archived {
            return Err(super::validation_error(
                format!("fields.{index}"),
                "initial fields must have unique keys and be active",
            ));
        }
    }
    if let Some(pipeline) = &input.pipeline {
        let stage = input
            .fields
            .iter()
            .find(|f| f.key == pipeline.stage_field)
            .filter(|f| f.required)
            .ok_or_else(|| {
                super::validation_error("pipeline.stage_field", "must name a required select field")
            })?;
        let CrmFieldKind::Select { options } = &stage.kind else {
            return Err(super::validation_error(
                "pipeline.stage_field",
                "must name a required select field",
            ));
        };
        let mut closed = HashSet::new();
        if options.len() > 50
            || pipeline.closed_stages.iter().any(|key| {
                !closed.insert(key) || !options.iter().any(|o| &o.value == key && !o.archived)
            })
        {
            return Err(super::validation_error(
                "pipeline.closed_stages",
                "must name unique live stages from at most 50 options",
            ));
        }
    }
    Ok(())
}

pub async fn create(
    db: &Database,
    owner_id: &str,
    input: CreateObjectInput,
    receipt: Option<super::records::ReceiptContext>,
) -> AppResult<CrmObject> {
    if let Some(response) = super::records::lookup_receipt(db, receipt.as_ref()).await? {
        return Ok(response);
    }
    validate_create(&input)?;
    ensure_seeded(db, owner_id).await?;
    let context = (db.clone(), owner_id.to_owned(), input, receipt.clone());
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, input, receipt)| {
            Box::pin(async move {
                super::fence_owner(db, owner, tx).await?;
                if let Some(value) =
                    super::records::receipt_in_session(db, receipt.as_ref(), tx).await?
                {
                    return serde_json::from_value(value).map_err(|_| {
                        AppError::Internal("Stored CRM object response is invalid".into())
                    });
                }
                let collection = db.collection::<CrmObject>(COLLECTION_NAME);
                if collection
                    .count_documents(doc! {"owner_id":owner})
                    .session(&mut *tx)
                    .await?
                    >= MAX_OWNER_OBJECTS
                {
                    return Err(super::validation_error(
                        "key",
                        "an owner may contain at most 50 objects, including seeded objects",
                    ));
                }
                if collection
                    .find_one(doc! {"owner_id":owner,"key":&input.key})
                    .session(&mut *tx)
                    .await?
                    .is_some()
                {
                    return Err(super::validation_error(
                        "key",
                        "already exists on this owner",
                    ));
                }
                for (index, field) in input.fields.iter().enumerate() {
                    if let CrmFieldKind::Record { target, .. } = &field.kind
                        && target != &input.key
                        && collection
                            .find_one(doc! {"owner_id":owner,"key":target})
                            .session(&mut *tx)
                            .await?
                            .is_none()
                    {
                        return Err(super::validation_error(
                            format!("fields.{index}.target"),
                            "must name an object owned by the same CRM owner",
                        ));
                    }
                }
                let now = bson::DateTime::now().to_chrono();
                let object = CrmObject {
                    id: Uuid::new_v4().to_string(),
                    owner_id: owner.clone(),
                    key: input.key.clone(),
                    label: input.label.trim().into(),
                    plural_label: input.plural_label.trim().into(),
                    schema_revision: 1,
                    fields: input.fields.clone(),
                    pipeline: input.pipeline.clone(),
                    default_columns: input.fields.iter().take(6).map(|f| f.key.clone()).collect(),
                    seeded: false,
                    created_at: now,
                    updated_at: now,
                };
                collection.insert_one(&object).session(&mut *tx).await?;
                super::records::insert_receipt(db, tx, receipt.as_ref(), &object, now).await?;
                Ok(object)
            })
        },
    )
    .await;
    super::records::recover_duplicate(db, receipt.as_ref(), result).await
}

pub async fn add_field(
    db: &Database,
    owner_id: &str,
    object_key: &str,
    expected_schema_revision: i64,
    mut field: CrmField,
    receipt: Option<super::records::ReceiptContext>,
) -> AppResult<CrmObject> {
    if let Some(response) = super::records::lookup_receipt(db, receipt.as_ref()).await? {
        return Ok(response);
    }
    if field.archived {
        field.required = false;
    }
    validation_result(fields::validate_definition(&field, "field"))?;
    if let CrmFieldKind::Record { target, .. } = &field.kind {
        get(db, owner_id, target).await.map_err(|_| {
            super::validation_error(
                "field.target",
                "must name an object owned by the same CRM owner",
            )
        })?;
    }
    let context = (
        db.clone(),
        owner_id.to_owned(),
        object_key.to_owned(),
        expected_schema_revision,
        field,
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, object_key, expected, field, receipt)| Box::pin(async move {
            super::fence_owner(db, owner, tx).await?;
            if let Some(value) = super::records::receipt_in_session(db, receipt.as_ref(), tx).await? {
                return serde_json::from_value(value).map_err(|_| AppError::Internal("Stored CRM object response is invalid".into()));
            }
            let current = db.collection::<CrmObject>(COLLECTION_NAME)
                .find_one(doc! {"owner_id": owner, "key": object_key}).session(&mut *tx).await?
                .ok_or_else(|| AppError::NotFound("CRM object not found".into()))?;
            require_schema_revision(&current, *expected)?;
            if current.fields.len() >= 100 {
                return Err(super::validation_error("fields", "an object may contain at most 100 fields, including archived fields"));
            }
            if current.fields.iter().any(|candidate| candidate.key == field.key) {
                return Err(super::validation_error("field.key", "already exists on this object"));
            }
            let now = bson::DateTime::now().to_chrono();
            let updated = db.collection::<CrmObject>(COLLECTION_NAME).update_one(
                doc! {"_id": &current.id, "owner_id": owner, "schema_revision": expected},
                doc! {"$push": {"fields": bson::to_bson(field).map_err(|error| AppError::Internal(error.to_string()))?}, "$inc": {"schema_revision": 1_i64}, "$set": {"updated_at": bson::DateTime::from_chrono(now)}},
            ).session(&mut *tx).await?;
            if updated.modified_count != 1 {
                let current = db.collection::<CrmObject>(COLLECTION_NAME).find_one(doc! {"_id": &current.id, "owner_id": owner}).session(&mut *tx).await?.ok_or_else(|| AppError::NotFound("CRM object not found".into()))?;
                return Err(schema_conflict(current));
            }
            let object = CrmObject { fields: current.fields.iter().cloned().chain(std::iter::once(field.clone())).collect(), schema_revision: current.schema_revision + 1, updated_at: now, ..current };
            super::records::insert_receipt(db, tx, receipt.as_ref(), &object, now).await?;
            Ok(object)
        }),
    ).await;
    super::records::recover_duplicate(db, receipt.as_ref(), result).await
}

pub async fn patch_field(
    db: &Database,
    owner_id: &str,
    object_key: &str,
    field_key: &str,
    patch: PatchField,
    receipt: Option<super::records::ReceiptContext>,
) -> AppResult<CrmObject> {
    if let Some(response) = super::records::lookup_receipt(db, receipt.as_ref()).await? {
        return Ok(response);
    }
    let context = (
        db.clone(),
        owner_id.to_owned(),
        object_key.to_owned(),
        field_key.to_owned(),
        patch,
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(db, &context, |tx, (db, owner, object_key, field_key, patch, receipt)| Box::pin(async move {
    super::fence_owner(db, owner, tx).await?;
    if let Some(value) = super::records::receipt_in_session(db, receipt.as_ref(), tx).await? {
        return serde_json::from_value(value).map_err(|_| AppError::Internal("Stored CRM object response is invalid".into()));
    }
    let current = db.collection::<CrmObject>(COLLECTION_NAME)
        .find_one(doc! {"owner_id": owner, "key": object_key}).session(&mut *tx).await?
        .ok_or_else(|| AppError::NotFound("CRM object not found".into()))?;
    require_schema_revision(&current, patch.expected_schema_revision)?;
    let Some(index) = current
        .fields
        .iter()
        .position(|field| field.key == field_key.as_str())
    else {
        return Err(AppError::NotFound("CRM field not found".into()));
    };
    let mut object_fields = current.fields.clone();
    let field = &mut object_fields[index];
    if let Some(label) = &patch.label {
        field.label = label.clone();
    }
    if let Some(required) = patch.required {
        if !required && current.pipeline.as_ref().is_some_and(|pipeline| pipeline.stage_field == *field_key) {
            return Err(super::validation_error("required", "the pipeline stage field must remain required"));
        }
        field.required = required;
    }
    if patch.archived == Some(true) {
        if current
            .pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.stage_field == field_key.as_str())
        {
            return Err(super::validation_error(
                "archived",
                "the pipeline stage field cannot be archived",
            ));
        }
        field.archived = true;
        field.required = false;
    } else if patch.archived == Some(false) {
        field.archived = false;
    }
    if let Some(options) = &patch.options {
        if current.pipeline.as_ref().is_some_and(|pipeline| pipeline.stage_field == field_key.as_str()) && options.len() > 50 {
            return Err(super::validation_error("options", "pipeline stages support at most 50 options, including archived options"));
        }
        let CrmFieldKind::Select { options: existing } = &field.kind else {
            return Err(super::validation_error(
                "options",
                "is only valid for select fields",
            ));
        };
        validate_option_edit(existing, options)?;
        if field.required && !field.archived && !options.iter().any(|option| !option.archived) {
            return Err(super::validation_error(
                "options",
                "a required select must keep at least one live option",
            ));
        }
        field.kind = CrmFieldKind::Select {
            options: options.clone(),
        };
    }
    validation_result(fields::validate_definition(field, "field"))?;
    let updated_field = field.clone();
    let default_columns = if updated_field.archived {
        current
            .default_columns
            .iter()
            .filter(|key| key.as_str() != field_key.as_str())
            .cloned()
            .collect()
    } else {
        current.default_columns.clone()
    };
    if object_fields == current.fields && default_columns == current.default_columns {
        super::records::insert_receipt(db, tx, receipt.as_ref(), &current, Utc::now()).await?;
        return Ok(current);
    }
    let now = bson::DateTime::now().to_chrono();
    let updated = db
        .collection::<CrmObject>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &current.id, "owner_id": owner, "schema_revision": patch.expected_schema_revision},
            doc! {
                "$set": {
                    format!("fields.{index}"): bson::to_bson(&updated_field).map_err(|error| AppError::Internal(error.to_string()))?,
                    "default_columns": bson::to_bson(&default_columns).map_err(|error| AppError::Internal(error.to_string()))?,
                    "updated_at": bson::DateTime::from_chrono(now),
                },
                "$inc": {"schema_revision": 1_i64},
            },
        )
        .session(&mut *tx)
        .await?;
    if updated.modified_count != 1 {
        let current = db.collection::<CrmObject>(COLLECTION_NAME).find_one(doc! {"_id": &current.id, "owner_id": owner}).session(&mut *tx).await?.ok_or_else(|| AppError::NotFound("CRM object not found".into()))?;
        return Err(schema_conflict(current));
    }
    let object = CrmObject { fields: object_fields, default_columns, schema_revision: current.schema_revision + 1, updated_at: now, ..current };
    super::records::insert_receipt(db, tx, receipt.as_ref(), &object, now).await?;
    Ok(object)
    })).await;
    super::records::recover_duplicate(db, receipt.as_ref(), result).await
}

fn require_schema_revision(object: &CrmObject, expected: i64) -> AppResult<()> {
    if object.schema_revision == expected {
        Ok(())
    } else {
        Err(schema_conflict(object.clone()))
    }
}

fn schema_conflict(object: CrmObject) -> AppError {
    AppError::CrmRevisionConflict(Box::new(
        serde_json::to_value(ObjectApi::from(object)).expect("CRM object response is serializable"),
    ))
}

fn validate_option_edit(
    existing: &[CrmSelectOption],
    replacement: &[CrmSelectOption],
) -> AppResult<()> {
    let old: HashMap<&str, &CrmSelectOption> = existing
        .iter()
        .map(|option| (option.value.as_str(), option))
        .collect();
    let replacement_values: HashSet<&str> = replacement
        .iter()
        .map(|option| option.value.as_str())
        .collect();
    let mut errors = Vec::new();
    for option in existing {
        if !replacement_values.contains(option.value.as_str()) {
            errors.push(CrmValidationIssue {
                path: "options".into(),
                message: format!(
                    "option '{}' must be retained and archived instead of removed",
                    option.value
                ),
            });
        }
    }
    for (index, option) in replacement.iter().enumerate() {
        if let Some(previous) = old.get(option.value.as_str())
            && previous.value != option.value
        {
            errors.push(CrmValidationIssue {
                path: format!("options.{index}.value"),
                message: "option values are immutable".into(),
            });
        }
    }
    validation_result(errors)
}

fn seeded_objects(owner_id: &str, now: chrono::DateTime<Utc>) -> Vec<CrmObject> {
    let text = |key: &str, label: &str, required: bool, max_length| CrmField {
        key: key.into(),
        label: label.into(),
        required,
        archived: false,
        kind: CrmFieldKind::Text { max_length },
    };
    let user = |key: &str, label: &str| CrmField {
        key: key.into(),
        label: label.into(),
        required: false,
        archived: false,
        kind: CrmFieldKind::User {},
    };
    let record = |key: &str, label: &str, target: &str, cardinality: CrmCardinality| CrmField {
        key: key.into(),
        label: label.into(),
        required: false,
        archived: false,
        kind: CrmFieldKind::Record {
            target: target.into(),
            cardinality,
        },
    };
    let select = |key: &str, label: &str, required: bool, options: &[(&str, &str)]| CrmField {
        key: key.into(),
        label: label.into(),
        required,
        archived: false,
        kind: CrmFieldKind::Select {
            options: options
                .iter()
                .map(|(value, label)| CrmSelectOption {
                    value: (*value).into(),
                    label: (*label).into(),
                    color: None,
                    archived: false,
                })
                .collect(),
        },
    };
    let date = |key: &str, label: &str| CrmField {
        key: key.into(),
        label: label.into(),
        required: false,
        archived: false,
        kind: CrmFieldKind::Date {},
    };
    let object = |key: &str,
                  label: &str,
                  plural_label: &str,
                  fields: Vec<CrmField>,
                  pipeline: Option<CrmPipeline>,
                  default_columns: Vec<&str>| CrmObject {
        id: Uuid::new_v4().to_string(),
        owner_id: owner_id.into(),
        key: key.into(),
        label: label.into(),
        plural_label: plural_label.into(),
        schema_revision: 1,
        fields,
        pipeline,
        default_columns: default_columns.into_iter().map(str::to_owned).collect(),
        seeded: true,
        created_at: now,
        updated_at: now,
    };
    vec![
        object(
            "company",
            "Company",
            "Companies",
            vec![
                text("name", "Name", true, 200),
                text("domain", "Domain", false, 253),
                select("industry", "Industry", false, &[]),
                user("owner", "Owner"),
            ],
            None,
            vec!["name", "domain", "industry", "owner"],
        ),
        object(
            "contact",
            "Contact",
            "Contacts",
            vec![
                text("name", "Name", true, 200),
                text("email", "Email", false, 254),
                record("company", "Company", "company", CrmCardinality::One),
                text("title", "Title", false, 200),
            ],
            None,
            vec!["name", "email", "company", "title"],
        ),
        object(
            "opportunity",
            "Opportunity",
            "Opportunities",
            vec![
                text("name", "Name", true, 200),
                record("company", "Company", "company", CrmCardinality::One),
                CrmField {
                    key: "amount".into(),
                    label: "Amount".into(),
                    required: false,
                    archived: false,
                    kind: CrmFieldKind::Currency {
                        currency: "USD".into(),
                    },
                },
                select(
                    "stage",
                    "Stage",
                    true,
                    &[
                        ("new", "New"),
                        ("qualified", "Qualified"),
                        ("proposal", "Proposal"),
                        ("won", "Won"),
                        ("lost", "Lost"),
                    ],
                ),
                date("expected_close", "Expected close"),
                user("owner", "Owner"),
            ],
            Some(CrmPipeline {
                stage_field: "stage".into(),
                closed_stages: vec!["won".into(), "lost".into()],
            }),
            vec![
                "name",
                "company",
                "amount",
                "stage",
                "expected_close",
                "owner",
            ],
        ),
        object(
            "task",
            "Task",
            "Tasks",
            vec![
                text("title", "Title", true, 200),
                select(
                    "status",
                    "Status",
                    true,
                    &[("open", "Open"), ("done", "Done")],
                ),
                date("due", "Due"),
                user("assignee", "Assignee"),
                record("related", "Related", "opportunity", CrmCardinality::Many),
            ],
            Some(CrmPipeline {
                stage_field: "status".into(),
                closed_stages: vec!["done".into()],
            }),
            vec!["title", "status", "due", "assignee", "related"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_contract_has_stable_shapes() {
        let objects = seeded_objects("owner", Utc::now());
        assert_eq!(
            objects
                .iter()
                .map(|object| object.key.as_str())
                .collect::<Vec<_>>(),
            vec!["company", "contact", "opportunity", "task"]
        );
        let opportunity = objects
            .iter()
            .find(|object| object.key == "opportunity")
            .unwrap();
        assert_eq!(
            opportunity.pipeline.as_ref().unwrap().closed_stages,
            vec!["won", "lost"]
        );
        assert!(matches!(
            opportunity
                .fields
                .iter()
                .find(|field| field.key == "amount")
                .unwrap()
                .kind,
            CrmFieldKind::Currency { ref currency } if currency == "USD"
        ));
    }

    #[test]
    fn option_removal_requires_archiving() {
        let existing = vec![CrmSelectOption {
            value: "open".into(),
            label: "Open".into(),
            color: None,
            archived: false,
        }];
        let error = validate_option_edit(&existing, &[]).unwrap_err();
        assert!(matches!(error, AppError::CrmValidationFailed(_)));
    }

    #[tokio::test]
    async fn seeding_is_rejected_by_the_durable_owner_erasure_fence() {
        let db = crate::test_utils::connect_transaction_test_database("crm_seed_fence").await;
        let owner = Uuid::new_v4().to_string();
        db.collection::<mongodb::bson::Document>(crate::services::erasure_admission::COLLECTION)
            .insert_one(doc! {
                "_id": &owner,
                "erasure_accepted": true,
                "revision": 1_i64,
            })
            .await
            .unwrap();

        assert!(matches!(
            ensure_seeded(&db, &owner).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            db.collection::<CrmObject>(COLLECTION_NAME)
                .count_documents(doc! {"owner_id": &owner})
                .await
                .unwrap(),
            0
        );
    }
}
