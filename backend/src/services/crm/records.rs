use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Bson, Document, doc},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::errors::{AppError, AppResult, CrmValidationIssue};
use crate::models::{
    crm_activity::{
        COLLECTION_NAME as ACTIVITIES, CrmActivity, CrmActivityKind, CrmActivitySummary,
    },
    crm_link::{COLLECTION_NAME as LINKS, CrmLink},
    crm_mutation_receipt::{COLLECTION_NAME as RECEIPTS, CrmMutationReceipt},
    crm_object::{CrmFieldKind, CrmObject},
    crm_record::{COLLECTION_NAME as RECORDS, CrmActor, CrmRecord},
};

use super::{fields, objects, validation_result};

const RECEIPT_TTL_DAYS: i64 = 7;
const GENERIC_LINK_ERROR: &str =
    "one or more related records are unavailable for this owner and object type";

#[derive(Clone, Debug)]
pub struct ReceiptContext {
    pub id: String,
    pub owner_id: String,
    pub request_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRecordInput {
    pub object: String,
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
    #[serde(default)]
    pub protect: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PatchRecordInput {
    pub expected_revision: i64,
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
    #[serde(default)]
    pub protect: Vec<String>,
    #[serde(default)]
    pub unprotect: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionInput {
    pub expected_revision: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MoveRecordInput {
    pub expected_revision: i64,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecordResponse {
    pub id: String,
    pub owner_id: String,
    pub object_key: String,
    pub revision: i64,
    pub values: BTreeMap<String, Value>,
    pub links: BTreeMap<String, Vec<RecordLinkResponse>>,
    pub overrides: Vec<String>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordLinkResponse {
    pub id: String,
    pub object_key: String,
    pub label: String,
    pub archived: bool,
}

#[derive(Clone, Debug)]
struct PreparedValues {
    scalar_changes: BTreeMap<String, Option<Bson>>,
    ref_changes: BTreeMap<String, RefChange>,
}

#[derive(Clone, Debug)]
struct RefChange {
    target_object: String,
    single: bool,
    ids: Vec<String>,
}

pub fn receipt_context<T: Serialize>(
    owner_id: &str,
    actor_id: &str,
    idempotency_key: Option<&str>,
    method: &str,
    route_template: &str,
    path_ids: &[(&str, &str)],
    body: &T,
) -> AppResult<Option<ReceiptContext>> {
    let Some(key) = idempotency_key else {
        return Ok(None);
    };
    if key.is_empty() || key.len() > 128 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(super::validation_error(
            "Idempotency-Key",
            "must contain 1 to 128 visible ASCII characters",
        ));
    }
    let canonical = serde_json::json!({
        "method": method,
        "route": route_template,
        "path": path_ids.iter().copied().collect::<BTreeMap<_, _>>(),
        "owner_id": owner_id,
        "actor_id": actor_id,
        "body": serde_json::to_value(body).map_err(|error| AppError::Internal(error.to_string()))?,
    });
    let request_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical).expect("canonical mutation input is serializable"),
    ));
    let id = hex::encode(Sha256::digest(
        [
            owner_id.as_bytes(),
            b"\0",
            actor_id.as_bytes(),
            b"\0",
            key.as_bytes(),
        ]
        .concat(),
    ));
    Ok(Some(ReceiptContext {
        id,
        owner_id: owner_id.to_owned(),
        request_digest,
    }))
}

pub async fn get(db: &Database, owner_id: &str, record_id: &str) -> AppResult<RecordResponse> {
    let mut session = db.client().start_session().await?;
    let record = record_in_session(db, owner_id, record_id, &mut session).await?;
    hydrate_record(db, &record, &mut session).await
}

pub async fn raw_by_id(db: &Database, record_id: &str) -> AppResult<CrmRecord> {
    db.collection::<CrmRecord>(RECORDS)
        .find_one(doc! {"_id": record_id})
        .await?
        .ok_or_else(|| AppError::NotFound("CRM record not found".into()))
}

pub async fn create(
    db: &Database,
    owner_id: &str,
    actor: CrmActor,
    input: CreateRecordInput,
    receipt: Option<ReceiptContext>,
) -> AppResult<RecordResponse> {
    objects::ensure_seeded(db, owner_id).await?;
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
                if let Some(response) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return response_from_value(response);
                }
                let object = object_in_session(db, owner, &input.object, tx).await?;
                let prepared = prepare_values(db, tx, owner, &object, &input.values, true).await?;
                let overrides =
                    validate_override_edit(&object, &input.protect, &[], &BTreeSet::new())?;
                let now = Utc::now();
                let mut record = CrmRecord {
                    id: Uuid::new_v4().to_string(),
                    owner_id: owner.clone(),
                    object_id: object.id.clone(),
                    object_key: object.key.clone(),
                    revision: 1,
                    values: Document::new(),
                    refs: Document::new(),
                    overrides: overrides.into_iter().collect(),
                    created_by: actor.clone(),
                    updated_by: actor.clone(),
                    archived_at: None,
                    created_at: now,
                    updated_at: now,
                };
                apply_prepared(&mut record, &prepared);
                db.collection::<CrmRecord>(RECORDS)
                    .insert_one(&record)
                    .session(&mut *tx)
                    .await?;
                replace_links(db, tx, &record, &prepared.ref_changes, now).await?;
                insert_activity(
                    db,
                    tx,
                    &record,
                    CrmActivityKind::Created,
                    actor.clone(),
                    CrmActivitySummary::default(),
                    None,
                    now,
                )
                .await?;
                let response = hydrate_record(db, &record, tx).await?;
                insert_receipt(db, tx, receipt.as_ref(), &response, now).await?;
                Ok(response)
            })
        },
    )
    .await;
    recover_duplicate(db, receipt.as_ref(), result).await
}

pub async fn patch(
    db: &Database,
    owner_id: &str,
    record_id: &str,
    actor: CrmActor,
    input: PatchRecordInput,
    receipt: Option<ReceiptContext>,
) -> AppResult<RecordResponse> {
    let context = (
        db.clone(),
        owner_id.to_owned(),
        record_id.to_owned(),
        actor,
        input,
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, record_id, actor, input, receipt)| {
            Box::pin(async move {
                if let Some(response) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return response_from_value(response);
                }
                let mut record = record_in_session(db, owner, record_id, tx).await?;
                let object = object_in_session(db, owner, &record.object_key, tx).await?;
                require_revision(db, tx, &record, input.expected_revision).await?;
                let prepared =
                    prepare_values(db, tx, owner, &object, &input.values, false).await?;
                let old_overrides: BTreeSet<String> = record.overrides.iter().cloned().collect();
                let overrides = validate_override_edit(
                    &object,
                    &input.protect,
                    &input.unprotect,
                    &old_overrides,
                )?;
                let mut changed_fields = changed_fields(&record, &prepared);
                if overrides != old_overrides {
                    changed_fields.insert("overrides".into());
                }
                if changed_fields.is_empty() {
                    let response = hydrate_record(db, &record, tx).await?;
                    insert_receipt(db, tx, receipt.as_ref(), &response, Utc::now()).await?;
                    return Ok(response);
                }
                apply_prepared(&mut record, &prepared);
                record.overrides = overrides.into_iter().collect();
                record.revision += 1;
                record.updated_by = actor.clone();
                record.updated_at = Utc::now();
                let update = doc! {
                    "$set": {
                        "values": &record.values,
                        "refs": &record.refs,
                        "overrides": bson::to_bson(&record.overrides).map_err(|error| AppError::Internal(error.to_string()))?,
                        "updated_by": bson::to_bson(&record.updated_by).map_err(|error| AppError::Internal(error.to_string()))?,
                        "updated_at": bson::DateTime::from_chrono(record.updated_at),
                    },
                    "$inc": {"revision": 1_i64},
                };
                let updated = db
                    .collection::<CrmRecord>(RECORDS)
                    .update_one(
                        doc! {"_id": record_id, "owner_id": owner, "revision": input.expected_revision},
                        update,
                    )
                    .session(&mut *tx)
                    .await?;
                if updated.modified_count != 1 {
                    let current = record_in_session(db, owner, record_id, tx).await?;
                    return Err(revision_conflict(db, tx, &current).await?);
                }
                replace_links(db, tx, &record, &prepared.ref_changes, record.updated_at).await?;
                insert_activity(
                    db,
                    tx,
                    &record,
                    CrmActivityKind::Updated,
                    actor.clone(),
                    CrmActivitySummary {
                        changed_fields: Some(changed_fields.into_iter().collect()),
                        ..Default::default()
                    },
                    None,
                    record.updated_at,
                )
                .await?;
                let response = hydrate_record(db, &record, tx).await?;
                insert_receipt(db, tx, receipt.as_ref(), &response, record.updated_at).await?;
                Ok(response)
            })
        },
    )
    .await;
    recover_duplicate(db, receipt.as_ref(), result).await
}

pub async fn set_archived(
    db: &Database,
    owner_id: &str,
    record_id: &str,
    actor: CrmActor,
    input: RevisionInput,
    archived: bool,
    receipt: Option<ReceiptContext>,
) -> AppResult<RecordResponse> {
    let context = (
        db.clone(),
        owner_id.to_owned(),
        record_id.to_owned(),
        actor,
        input,
        archived,
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, record_id, actor, input, archived, receipt)| {
            Box::pin(async move {
                if let Some(response) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return response_from_value(response);
                }
                let mut record = record_in_session(db, owner, record_id, tx).await?;
                require_revision(db, tx, &record, input.expected_revision).await?;
                if record.archived_at.is_some() == *archived {
                    let response = hydrate_record(db, &record, tx).await?;
                    insert_receipt(db, tx, receipt.as_ref(), &response, Utc::now()).await?;
                    return Ok(response);
                }
                let now = Utc::now();
                record.archived_at = archived.then_some(now);
                record.updated_at = now;
                record.updated_by = actor.clone();
                record.revision += 1;
                let updated = db
                    .collection::<CrmRecord>(RECORDS)
                    .update_one(
                        doc! {"_id": record_id, "owner_id": owner, "revision": input.expected_revision},
                        doc! {"$set": {
                            "archived_at": record.archived_at.map(bson::DateTime::from_chrono),
                            "updated_at": bson::DateTime::from_chrono(now),
                            "updated_by": bson::to_bson(actor).map_err(|error| AppError::Internal(error.to_string()))?,
                        }, "$inc": {"revision": 1_i64}},
                    )
                    .session(&mut *tx)
                    .await?;
                if updated.modified_count != 1 {
                    let current = record_in_session(db, owner, record_id, tx).await?;
                    return Err(revision_conflict(db, tx, &current).await?);
                }
                insert_activity(
                    db,
                    tx,
                    &record,
                    CrmActivityKind::Updated,
                    actor.clone(),
                    CrmActivitySummary {
                        changed_fields: Some(vec![if *archived {
                            "archived"
                        } else {
                            "restored"
                        }
                        .into()]),
                        ..Default::default()
                    },
                    None,
                    now,
                )
                .await?;
                let response = hydrate_record(db, &record, tx).await?;
                insert_receipt(db, tx, receipt.as_ref(), &response, now).await?;
                Ok(response)
            })
        },
    )
    .await;
    recover_duplicate(db, receipt.as_ref(), result).await
}

pub async fn move_stage(
    db: &Database,
    owner_id: &str,
    record_id: &str,
    actor: CrmActor,
    input: MoveRecordInput,
    receipt: Option<ReceiptContext>,
) -> AppResult<RecordResponse> {
    let context = (
        db.clone(),
        owner_id.to_owned(),
        record_id.to_owned(),
        actor,
        input,
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, record_id, actor, input, receipt)| {
            Box::pin(async move {
                if let Some(response) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return response_from_value(response);
                }
                let mut record = record_in_session(db, owner, record_id, tx).await?;
                let object = object_in_session(db, owner, &record.object_key, tx).await?;
                require_revision(db, tx, &record, input.expected_revision).await?;
                let pipeline = object.pipeline.as_ref().ok_or_else(|| {
                    super::validation_error("to", "this object does not have a pipeline")
                })?;
                let field = object
                    .fields
                    .iter()
                    .find(|field| field.key == pipeline.stage_field)
                    .ok_or_else(|| {
                        AppError::Internal("CRM pipeline stage field is missing".into())
                    })?;
                let to_value = Value::String(input.to.clone());
                let stored = match fields::validate_write_value(field, &to_value, "to") {
                    Ok(fields::ValidatedValue::Stored(value)) => value,
                    Ok(_) => {
                        return Err(super::validation_error("to", "must be a stage option"));
                    }
                    Err(error) => return Err(AppError::CrmValidationFailed(vec![error])),
                };
                let previous = record
                    .values
                    .get_str(&pipeline.stage_field)
                    .ok()
                    .map(str::to_owned);
                if previous.as_deref() == Some(input.to.as_str()) {
                    let response = hydrate_record(db, &record, tx).await?;
                    insert_receipt(db, tx, receipt.as_ref(), &response, Utc::now()).await?;
                    return Ok(response);
                }
                let now = Utc::now();
                record.values.insert(&pipeline.stage_field, stored);
                record.revision += 1;
                record.updated_by = actor.clone();
                record.updated_at = now;
                let updated = db
                    .collection::<CrmRecord>(RECORDS)
                    .update_one(
                        doc! {"_id": record_id, "owner_id": owner, "revision": input.expected_revision},
                        doc! {"$set": {
                            format!("values.{}", pipeline.stage_field): &input.to,
                            "updated_at": bson::DateTime::from_chrono(now),
                            "updated_by": bson::to_bson(actor).map_err(|error| AppError::Internal(error.to_string()))?,
                        }, "$inc": {"revision": 1_i64}},
                    )
                    .session(&mut *tx)
                    .await?;
                if updated.modified_count != 1 {
                    let current = record_in_session(db, owner, record_id, tx).await?;
                    return Err(revision_conflict(db, tx, &current).await?);
                }
                insert_activity(
                    db,
                    tx,
                    &record,
                    CrmActivityKind::StageChanged,
                    actor.clone(),
                    CrmActivitySummary {
                        changed_fields: Some(vec![pipeline.stage_field.clone()]),
                        from: previous,
                        to: Some(input.to.clone()),
                    },
                    None,
                    now,
                )
                .await?;
                let response = hydrate_record(db, &record, tx).await?;
                insert_receipt(db, tx, receipt.as_ref(), &response, now).await?;
                Ok(response)
            })
        },
    )
    .await;
    recover_duplicate(db, receipt.as_ref(), result).await
}

async fn prepare_values(
    db: &Database,
    tx: &mut ClientSession,
    owner_id: &str,
    object: &CrmObject,
    values: &BTreeMap<String, Value>,
    creating: bool,
) -> AppResult<PreparedValues> {
    let fields_by_key: HashMap<&str, _> = object
        .fields
        .iter()
        .map(|field| (field.key.as_str(), field))
        .collect();
    let mut errors = Vec::new();
    let mut validated = BTreeMap::new();
    for (key, value) in values {
        let path = format!("values.{key}");
        let Some(field) = fields_by_key.get(key.as_str()) else {
            errors.push(CrmValidationIssue {
                path,
                message: "unknown field".into(),
            });
            continue;
        };
        match fields::validate_write_value(field, value, &path) {
            Ok(value) => {
                validated.insert(key.clone(), value);
            }
            Err(error) => errors.push(error),
        }
    }
    if creating {
        for field in object
            .fields
            .iter()
            .filter(|field| field.required && !field.archived)
        {
            if !values.contains_key(&field.key) {
                errors.push(CrmValidationIssue {
                    path: format!("values.{}", field.key),
                    message: "is required".into(),
                });
            }
        }
    }
    validation_result(errors)?;

    let mut scalar_changes = BTreeMap::new();
    let mut ref_changes = BTreeMap::new();
    let mut reference_errors = Vec::new();
    for (key, value) in validated {
        match value {
            fields::ValidatedValue::Clear => {
                if let CrmFieldKind::Record {
                    target,
                    cardinality,
                } = &fields_by_key[key.as_str()].kind
                {
                    ref_changes.insert(
                        key,
                        RefChange {
                            target_object: target.clone(),
                            single: matches!(
                                cardinality,
                                crate::models::crm_object::CrmCardinality::One
                            ),
                            ids: vec![],
                        },
                    );
                } else {
                    scalar_changes.insert(key, None);
                }
            }
            fields::ValidatedValue::Stored(value) => {
                scalar_changes.insert(key, Some(value));
            }
            fields::ValidatedValue::User(user_id) => {
                if user_has_owner_access(db, tx, owner_id, &user_id).await? {
                    scalar_changes.insert(key, Some(Bson::String(user_id)));
                } else {
                    reference_errors.push(CrmValidationIssue {
                        path: format!("values.{key}"),
                        message: "person is unavailable for this owner".into(),
                    });
                }
            }
            fields::ValidatedValue::Record {
                target_object,
                single,
                ids,
            } => {
                if !targets_are_valid(db, tx, owner_id, &target_object, &ids).await? {
                    reference_errors.push(CrmValidationIssue {
                        path: format!("values.{key}"),
                        message: GENERIC_LINK_ERROR.into(),
                    });
                } else {
                    ref_changes.insert(
                        key,
                        RefChange {
                            target_object,
                            single,
                            ids,
                        },
                    );
                }
            }
        }
    }
    validation_result(reference_errors)?;
    Ok(PreparedValues {
        scalar_changes,
        ref_changes,
    })
}

fn validate_override_edit(
    object: &CrmObject,
    protect: &[String],
    unprotect: &[String],
    current: &BTreeSet<String>,
) -> AppResult<BTreeSet<String>> {
    let known: HashSet<&str> = object
        .fields
        .iter()
        .map(|field| field.key.as_str())
        .collect();
    let protect_set: BTreeSet<String> = protect.iter().cloned().collect();
    let unprotect_set: BTreeSet<String> = unprotect.iter().cloned().collect();
    let mut errors = Vec::new();
    for (name, values) in [("protect", protect), ("unprotect", unprotect)] {
        for (index, key) in values.iter().enumerate() {
            if !known.contains(key.as_str()) {
                errors.push(CrmValidationIssue {
                    path: format!("{name}.{index}"),
                    message: "unknown field".into(),
                });
            }
        }
    }
    if let Some(key) = protect_set.intersection(&unprotect_set).next() {
        errors.push(CrmValidationIssue {
            path: "protect".into(),
            message: format!("field '{key}' cannot be protected and unprotected together"),
        });
    }
    validation_result(errors)?;
    let mut result = current.clone();
    result.extend(protect_set);
    for key in unprotect_set {
        result.remove(&key);
    }
    Ok(result)
}

fn changed_fields(record: &CrmRecord, prepared: &PreparedValues) -> BTreeSet<String> {
    let mut changed = BTreeSet::new();
    for (key, value) in &prepared.scalar_changes {
        if value.as_ref() != record.values.get(key) {
            changed.insert(key.clone());
        }
    }
    for (key, change) in &prepared.ref_changes {
        let existing = record
            .refs
            .get_array(key)
            .ok()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Bson::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if existing != change.ids {
            changed.insert(key.clone());
        }
    }
    changed
}

fn apply_prepared(record: &mut CrmRecord, prepared: &PreparedValues) {
    for (key, value) in &prepared.scalar_changes {
        match value {
            Some(value) => {
                record.values.insert(key, value.clone());
            }
            None => {
                record.values.remove(key);
            }
        }
    }
    for (key, change) in &prepared.ref_changes {
        if change.ids.is_empty() {
            record.refs.remove(key);
        } else {
            record.refs.insert(
                key,
                Bson::Array(change.ids.iter().cloned().map(Bson::String).collect()),
            );
        }
    }
}

async fn replace_links(
    db: &Database,
    tx: &mut ClientSession,
    record: &CrmRecord,
    changes: &BTreeMap<String, RefChange>,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let collection = db.collection::<CrmLink>(LINKS);
    for (field_key, change) in changes {
        collection
            .delete_many(doc! {"source_record_id": &record.id, "field_key": field_key})
            .session(&mut *tx)
            .await?;
        if !change.ids.is_empty() {
            let rows = change.ids.iter().map(|target| CrmLink {
                id: Uuid::new_v4().to_string(),
                owner_id: record.owner_id.clone(),
                field_key: field_key.clone(),
                source_object: record.object_key.clone(),
                source_record_id: record.id.clone(),
                target_object: change.target_object.clone(),
                target_record_id: target.clone(),
                single: change.single,
                created_at: now,
            });
            collection.insert_many(rows).session(&mut *tx).await?;
        }
    }
    Ok(())
}

async fn insert_activity(
    db: &Database,
    tx: &mut ClientSession,
    record: &CrmRecord,
    kind: CrmActivityKind,
    actor: CrmActor,
    summary: CrmActivitySummary,
    body: Option<String>,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    db.collection::<CrmActivity>(ACTIVITIES)
        .insert_one(CrmActivity {
            id: Uuid::new_v4().to_string(),
            owner_id: record.owner_id.clone(),
            record_id: record.id.clone(),
            object_key: record.object_key.clone(),
            kind,
            actor,
            summary,
            body,
            created_at: now,
        })
        .session(&mut *tx)
        .await?;
    Ok(())
}

async fn object_in_session(
    db: &Database,
    owner_id: &str,
    key: &str,
    tx: &mut ClientSession,
) -> AppResult<CrmObject> {
    db.collection::<CrmObject>(crate::models::crm_object::COLLECTION_NAME)
        .find_one(doc! {"owner_id": owner_id, "key": key})
        .session(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("CRM object not found".into()))
}

async fn record_in_session(
    db: &Database,
    owner_id: &str,
    record_id: &str,
    tx: &mut ClientSession,
) -> AppResult<CrmRecord> {
    db.collection::<CrmRecord>(RECORDS)
        .find_one(doc! {"_id": record_id, "owner_id": owner_id})
        .session(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("CRM record not found".into()))
}

async fn require_revision(
    db: &Database,
    tx: &mut ClientSession,
    record: &CrmRecord,
    expected: i64,
) -> AppResult<()> {
    if record.revision == expected {
        Ok(())
    } else {
        Err(revision_conflict(db, tx, record).await?)
    }
}

async fn revision_conflict(
    db: &Database,
    tx: &mut ClientSession,
    record: &CrmRecord,
) -> AppResult<AppError> {
    let response = hydrate_record(db, record, tx).await?;
    Ok(AppError::CrmRevisionConflict(Box::new(
        serde_json::to_value(response).expect("CRM record response is serializable"),
    )))
}

pub(crate) async fn hydrate_record(
    db: &Database,
    record: &CrmRecord,
    tx: &mut ClientSession,
) -> AppResult<RecordResponse> {
    let object = object_in_session(db, &record.owner_id, &record.object_key, tx).await?;
    let mut values = BTreeMap::new();
    for field in &object.fields {
        if matches!(field.kind, CrmFieldKind::Record { .. }) {
            continue;
        }
        if let Some(value) = record.values.get(&field.key) {
            values.insert(field.key.clone(), fields::render_api_value(field, value));
        }
    }
    let mut links = BTreeMap::new();
    for field in &object.fields {
        let CrmFieldKind::Record { target, .. } = &field.kind else {
            continue;
        };
        let ids = record
            .refs
            .get_array(&field.key)
            .ok()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Bson::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let target_object = object_in_session(db, &record.owner_id, target, tx).await?;
        let label_field = target_object.fields.iter().find(|candidate| {
            candidate.required && matches!(candidate.kind, CrmFieldKind::Text { .. })
        });
        let rows: Vec<CrmRecord> = if ids.is_empty() {
            vec![]
        } else {
            db.collection::<CrmRecord>(RECORDS)
                .find(
                    doc! {"_id": {"$in": &ids}, "owner_id": &record.owner_id, "object_key": target},
                )
                .session(&mut *tx)
                .await?
                .stream(&mut *tx)
                .try_collect()
                .await?
        };
        let by_id: HashMap<_, _> = rows.into_iter().map(|row| (row.id.clone(), row)).collect();
        let projected = ids
            .into_iter()
            .map(|id| {
                let row = by_id.get(&id);
                let archived = row.is_some_and(|row| row.archived_at.is_some());
                let label = if archived {
                    "(archived)".into()
                } else {
                    row.and_then(|row| {
                        label_field.and_then(|field| row.values.get_str(&field.key).ok())
                    })
                    .unwrap_or("(deleted)")
                    .to_owned()
                };
                RecordLinkResponse {
                    id,
                    object_key: target.clone(),
                    label,
                    archived,
                }
            })
            .collect();
        links.insert(field.key.clone(), projected);
    }
    Ok(RecordResponse {
        id: record.id.clone(),
        owner_id: record.owner_id.clone(),
        object_key: record.object_key.clone(),
        revision: record.revision,
        values,
        links,
        overrides: record.overrides.clone(),
        archived: record.archived_at.is_some(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    })
}

async fn targets_are_valid(
    db: &Database,
    tx: &mut ClientSession,
    owner_id: &str,
    object_key: &str,
    ids: &[String],
) -> AppResult<bool> {
    if ids.is_empty() {
        return Ok(true);
    }
    let count = db
        .collection::<CrmRecord>(RECORDS)
        .count_documents(doc! {
            "_id": {"$in": ids},
            "owner_id": owner_id,
            "object_key": object_key,
            "archived_at": bson::Bson::Null,
        })
        .session(&mut *tx)
        .await?;
    Ok(count == ids.len() as u64)
}

async fn user_has_owner_access(
    db: &Database,
    tx: &mut ClientSession,
    owner_id: &str,
    person_id: &str,
) -> AppResult<bool> {
    let person = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": person_id, "user_type": "person", "is_active": true})
        .session(&mut *tx)
        .await?;
    if person.is_none() {
        return Ok(false);
    }
    if owner_id == person_id {
        return Ok(true);
    }
    Ok(db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .find_one(doc! {
            "org_user_id": owner_id,
            "member_user_id": person_id,
            "revoked_at": bson::Bson::Null,
        })
        .session(&mut *tx)
        .await?
        .is_some())
}

pub(crate) async fn receipt_in_session(
    db: &Database,
    receipt: Option<&ReceiptContext>,
    tx: &mut ClientSession,
) -> AppResult<Option<Value>> {
    let Some(receipt) = receipt else {
        return Ok(None);
    };
    let stored = db
        .collection::<CrmMutationReceipt>(RECEIPTS)
        .find_one(doc! {"_id": &receipt.id, "owner_id": &receipt.owner_id})
        .session(&mut *tx)
        .await?;
    match stored {
        Some(stored) if stored.request_digest == receipt.request_digest => {
            Ok(Some(stored.response))
        }
        Some(_) => Err(AppError::CrmIdempotencyMismatch),
        None => Ok(None),
    }
}

pub(crate) async fn lookup_receipt<T: for<'de> Deserialize<'de>>(
    db: &Database,
    receipt: Option<&ReceiptContext>,
) -> AppResult<Option<T>> {
    let Some(receipt) = receipt else {
        return Ok(None);
    };
    let stored = db
        .collection::<CrmMutationReceipt>(RECEIPTS)
        .find_one(doc! {"_id": &receipt.id, "owner_id": &receipt.owner_id})
        .await?;
    match stored {
        Some(stored) if stored.request_digest == receipt.request_digest => {
            serde_json::from_value(stored.response)
                .map(Some)
                .map_err(|_| AppError::Internal("Stored CRM mutation response is invalid".into()))
        }
        Some(_) => Err(AppError::CrmIdempotencyMismatch),
        None => Ok(None),
    }
}

pub(crate) async fn insert_receipt<T: Serialize>(
    db: &Database,
    tx: &mut ClientSession,
    receipt: Option<&ReceiptContext>,
    response: &T,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let Some(receipt) = receipt else {
        return Ok(());
    };
    db.collection::<CrmMutationReceipt>(RECEIPTS)
        .insert_one(CrmMutationReceipt {
            id: receipt.id.clone(),
            owner_id: receipt.owner_id.clone(),
            request_digest: receipt.request_digest.clone(),
            response: serde_json::to_value(response)
                .map_err(|error| AppError::Internal(error.to_string()))?,
            created_at: now,
            expires_at: now + Duration::days(RECEIPT_TTL_DAYS),
        })
        .session(&mut *tx)
        .await?;
    Ok(())
}

pub(crate) async fn recover_duplicate<T: for<'de> Deserialize<'de>>(
    db: &Database,
    receipt: Option<&ReceiptContext>,
    result: AppResult<T>,
) -> AppResult<T> {
    let Err(AppError::DatabaseError(error)) = &result else {
        return result;
    };
    if !crate::services::assistant_team_service::is_duplicate(error) {
        return result;
    }
    let Some(receipt) = receipt else {
        return result;
    };
    let stored = db
        .collection::<CrmMutationReceipt>(RECEIPTS)
        .find_one(doc! {"_id": &receipt.id, "owner_id": &receipt.owner_id})
        .await?;
    match stored {
        Some(stored) if stored.request_digest == receipt.request_digest => {
            serde_json::from_value(stored.response)
                .map_err(|_| AppError::Internal("Stored CRM mutation response is invalid".into()))
        }
        Some(_) => Err(AppError::CrmIdempotencyMismatch),
        None => Err(AppError::CrmIdempotencyMismatch),
    }
}

pub(crate) async fn store_response<T>(
    db: &Database,
    receipt: Option<ReceiptContext>,
    response: T,
) -> AppResult<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let Some(receipt) = receipt else {
        return Ok(response);
    };
    let response =
        serde_json::to_value(response).map_err(|error| AppError::Internal(error.to_string()))?;
    let context = (db.clone(), receipt.clone(), response);
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, receipt, response)| {
            Box::pin(async move {
                if let Some(stored) = receipt_in_session(db, Some(receipt), tx).await? {
                    return Ok(stored);
                }
                insert_receipt(db, tx, Some(receipt), response, Utc::now()).await?;
                Ok(response.clone())
            })
        },
    )
    .await;
    let response = recover_duplicate(db, Some(&receipt), result).await?;
    serde_json::from_value(response)
        .map_err(|_| AppError::Internal("Stored CRM response is invalid".into()))
}

fn response_from_value(value: Value) -> AppResult<RecordResponse> {
    serde_json::from_value(value)
        .map_err(|_| AppError::Internal("Stored CRM mutation response is invalid".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn receipt_identity_binds_owner_actor_and_canonical_request() {
        let body = serde_json::json!({"b": 2, "a": 1});
        let first = receipt_context(
            "owner",
            "actor",
            Some("retry"),
            "POST",
            "/records",
            &[],
            &body,
        )
        .unwrap()
        .unwrap();
        let reordered = receipt_context(
            "owner",
            "actor",
            Some("retry"),
            "POST",
            "/records",
            &[],
            &serde_json::json!({"a": 1, "b": 2}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(first.id, reordered.id);
        assert_eq!(first.request_digest, reordered.request_digest);
        assert_ne!(
            first.id,
            receipt_context(
                "owner",
                "other",
                Some("retry"),
                "POST",
                "/records",
                &[],
                &body,
            )
            .unwrap()
            .unwrap()
            .id
        );
        assert!(
            receipt_context(
                "owner",
                "actor",
                Some("not visible"),
                "POST",
                "/records",
                &[],
                &body,
            )
            .is_err()
        );
    }

    #[test]
    fn unknown_override_fields_are_rejected() {
        let object = CrmObject {
            id: "object".into(),
            owner_id: "owner".into(),
            key: "company".into(),
            label: "Company".into(),
            plural_label: "Companies".into(),
            schema_revision: 1,
            fields: vec![],
            pipeline: None,
            default_columns: vec![],
            seeded: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(
            validate_override_edit(&object, &["missing".into()], &[], &BTreeSet::new()).is_err()
        );
    }

    fn create_input(object: &str, values: &[(&str, Value)]) -> CreateRecordInput {
        CreateRecordInput {
            object: object.into(),
            values: values
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
            protect: vec![],
        }
    }

    #[tokio::test]
    async fn transactional_writes_replay_once_and_replace_single_links() {
        let db = crate::test_utils::connect_transaction_test_database("crm_record_contract").await;
        super::super::ensure_indexes(&db).await.unwrap();
        let owner = Uuid::new_v4().to_string();
        let other_owner = Uuid::new_v4().to_string();
        objects::ensure_seeded(&db, &owner).await.unwrap();
        objects::ensure_seeded(&db, &other_owner).await.unwrap();
        let actor = CrmActor::human(&owner);

        let first_input = create_input("company", &[("name", json!("First"))]);
        let first_receipt = receipt_context(
            &owner,
            &owner,
            Some("create-first"),
            "POST",
            "/records",
            &[],
            &first_input,
        )
        .unwrap();
        let first = create(
            &db,
            &owner,
            actor.clone(),
            first_input.clone(),
            first_receipt.clone(),
        )
        .await
        .unwrap();
        let replay = create(&db, &owner, actor.clone(), first_input, first_receipt)
            .await
            .unwrap();
        assert_eq!(replay, first);
        assert_eq!(
            db.collection::<CrmActivity>(ACTIVITIES)
                .count_documents(doc! {"record_id": &first.id})
                .await
                .unwrap(),
            1
        );

        let changed_input = create_input("company", &[("name", json!("Changed"))]);
        let changed_receipt = receipt_context(
            &owner,
            &owner,
            Some("create-first"),
            "POST",
            "/records",
            &[],
            &changed_input,
        )
        .unwrap();
        assert!(matches!(
            create(&db, &owner, actor.clone(), changed_input, changed_receipt).await,
            Err(AppError::CrmIdempotencyMismatch)
        ));

        let revision_error = patch(
            &db,
            &owner,
            &first.id,
            actor.clone(),
            PatchRecordInput {
                expected_revision: 99,
                values: BTreeMap::new(),
                protect: vec![],
                unprotect: vec![],
            },
            None,
        )
        .await
        .unwrap_err();
        let AppError::CrmRevisionConflict(current) = revision_error else {
            panic!("expected revision conflict");
        };
        assert_eq!(current["id"], first.id);
        assert_eq!(current["revision"], 1);

        let second = create(
            &db,
            &owner,
            actor.clone(),
            create_input("company", &[("name", json!("Second"))]),
            None,
        )
        .await
        .unwrap();
        let contact = create(
            &db,
            &owner,
            actor.clone(),
            create_input(
                "contact",
                &[("name", json!("Contact")), ("company", json!(first.id))],
            ),
            None,
        )
        .await
        .unwrap();
        let moved = patch(
            &db,
            &owner,
            &contact.id,
            actor.clone(),
            PatchRecordInput {
                expected_revision: 1,
                values: BTreeMap::from([("company".into(), json!(second.id))]),
                protect: vec![],
                unprotect: vec![],
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(moved.links["company"][0].id, second.id);
        let links: Vec<CrmLink> = db
            .collection::<CrmLink>(LINKS)
            .find(doc! {"source_record_id": &contact.id, "field_key": "company"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_record_id, second.id);
        let stored = raw_by_id(&db, &contact.id).await.unwrap();
        assert_eq!(
            stored.refs.get_array("company").unwrap(),
            &vec![Bson::String(second.id.clone())]
        );
        let filtered = super::super::query::table(
            &db,
            &owner,
            &super::super::query::CrmTableQuery {
                object: "contact".into(),
                select: None,
                filters: vec![super::super::query::CrmFilter {
                    field: "company".into(),
                    op: "eq".into(),
                    value: Some(json!(second.id)),
                }],
                sort: None,
                page_size: 50,
                cursor: None,
                include_archived: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(filtered.total_count, 1);
        assert_eq!(filtered.rows[0].id, contact.id);

        let hidden = create(
            &db,
            &other_owner,
            CrmActor::human(&other_owner),
            create_input("company", &[("name", json!("Hidden"))]),
            None,
        )
        .await
        .unwrap();
        let cross_owner = create(
            &db,
            &owner,
            actor,
            create_input(
                "contact",
                &[("name", json!("Bad")), ("company", json!(hidden.id))],
            ),
            None,
        )
        .await
        .unwrap_err();
        let AppError::CrmValidationFailed(errors) = cross_owner else {
            panic!("expected generic link validation error");
        };
        assert_eq!(errors[0].path, "values.company");
        assert_eq!(errors[0].message, GENERIC_LINK_ERROR);
    }

    #[tokio::test]
    async fn record_validation_and_field_revision_rules_are_enforced() {
        let db = crate::test_utils::connect_transaction_test_database("crm_field_contract").await;
        super::super::ensure_indexes(&db).await.unwrap();
        let owner = Uuid::new_v4().to_string();
        objects::ensure_seeded(&db, &owner).await.unwrap();
        let actor = CrmActor::human(&owner);

        let unknown = create(
            &db,
            &owner,
            actor.clone(),
            create_input(
                "company",
                &[("name", json!("Known")), ("missing", json!(true))],
            ),
            None,
        )
        .await
        .unwrap_err();
        let AppError::CrmValidationFailed(errors) = unknown else {
            panic!("expected unknown-field validation error");
        };
        assert_eq!(errors[0].path, "values.missing");
        assert_eq!(errors[0].message, "unknown field");

        let required = create(
            &db,
            &owner,
            actor.clone(),
            create_input("company", &[]),
            None,
        )
        .await
        .unwrap_err();
        let AppError::CrmValidationFailed(errors) = required else {
            panic!("expected required-field validation error");
        };
        assert_eq!(errors[0].path, "values.name");
        assert_eq!(errors[0].message, "is required");

        let conflict = objects::add_field(
            &db,
            &owner,
            "company",
            99,
            crate::models::crm_object::CrmField {
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
        .unwrap_err();
        let AppError::CrmRevisionConflict(current) = conflict else {
            panic!("expected schema revision conflict");
        };
        assert_eq!(current["key"], "company");
        assert_eq!(current["schema_revision"], 1);

        objects::patch_field(
            &db,
            &owner,
            "company",
            "domain",
            objects::PatchField {
                expected_schema_revision: 1,
                label: None,
                required: None,
                archived: Some(true),
                options: None,
            },
            None,
        )
        .await
        .unwrap();
        let archived = create(
            &db,
            &owner,
            actor,
            create_input(
                "company",
                &[
                    ("name", json!("Archived")),
                    ("domain", json!("example.com")),
                ],
            ),
            None,
        )
        .await
        .unwrap_err();
        let AppError::CrmValidationFailed(errors) = archived else {
            panic!("expected archived-field validation error");
        };
        assert_eq!(errors[0].path, "values.domain");

        let opportunity = objects::get(&db, &owner, "opportunity").await.unwrap();
        let pipeline_archive = objects::patch_field(
            &db,
            &owner,
            "opportunity",
            "stage",
            objects::PatchField {
                expected_schema_revision: opportunity.schema_revision,
                label: None,
                required: None,
                archived: Some(true),
                options: None,
            },
            None,
        )
        .await
        .unwrap_err();
        let AppError::CrmValidationFailed(errors) = pipeline_archive else {
            panic!("expected pipeline archive validation error");
        };
        assert_eq!(errors[0].path, "archived");

        let mut options = opportunity
            .fields
            .iter()
            .find_map(|field| match &field.kind {
                CrmFieldKind::Select { options } if field.key == "stage" => Some(options.clone()),
                _ => None,
            })
            .unwrap();
        for option in &mut options {
            option.archived = true;
        }
        let last_live = objects::patch_field(
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
        .unwrap_err();
        let AppError::CrmValidationFailed(errors) = last_live else {
            panic!("expected required-select validation error");
        };
        assert_eq!(errors[0].path, "options");
    }
}
