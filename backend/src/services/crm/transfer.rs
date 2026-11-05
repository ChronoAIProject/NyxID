//! Bounded, explicit matching shared by human imports and recurring specialists.
use super::records::{self, CreateRecordInput, PatchRecordInput, ReceiptContext, RecordResponse};
use crate::{
    errors::{AppError, AppResult, CrmValidationIssue},
    models::{
        crm_object::{CrmFieldKind, CrmObject},
        crm_record::{COLLECTION_NAME as RECORDS, CrmActor, CrmRecord},
    },
};

use mongodb::{ClientSession, Database, bson::doc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

fn same_identity(left: &str, right: &str) -> bool {
    regex::RegexBuilder::new(&format!("^{}$", regex::escape(left.trim())))
        .case_insensitive(true)
        .unicode(true)
        .build()
        .is_ok_and(|pattern| pattern.is_match(right.trim()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertInput {
    pub object: String,
    pub match_field: String,
    pub values: BTreeMap<String, Value>,
    pub expected_revision: Option<i64>,
}

/// Exact text identity, ignoring surrounding whitespace and case. Ambiguity and
/// archived matches require human review; names are never guessed or fuzzy matched.
async fn matching(
    db: &Database,
    owner: &str,
    object: &CrmObject,
    key: &str,
    values: &BTreeMap<String, Value>,
    tx: &mut ClientSession,
) -> AppResult<Option<CrmRecord>> {
    let field = object
        .fields
        .iter()
        .find(|f| f.key == key && !f.archived && matches!(f.kind, CrmFieldKind::Text { .. }))
        .ok_or_else(|| {
            super::validation_error("match_field", "must be an active text identity field")
        })?;
    let value = values
        .get(&field.key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 500)
        .ok_or_else(|| {
            super::validation_error(
                "match_field",
                "requires a nonblank identity of at most 500 bytes",
            )
        })?;
    let path = format!("values.{}", field.key);
    let mut cursor = db.collection::<CrmRecord>(RECORDS).find(doc! {"owner_id": owner, "object_key": &object.key, path: {"$regex": format!(r"^\s*{}\s*$", regex::escape(value)), "$options": "i"}}).limit(2).max_time(std::time::Duration::from_secs(5)).session(&mut *tx).await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.next(&mut *tx).await.transpose()? {
        rows.push(row);
    }
    if rows.len() > 1 {
        return Err(super::validation_error(
            "match_field",
            "ambiguous identity; review duplicates before writing",
        ));
    }
    if rows.first().is_some_and(|row| row.archived_at.is_some()) {
        return Err(super::validation_error(
            "match_field",
            "matched record is archived; restore it before updating",
        ));
    }
    Ok(rows.pop())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpsertResponse {
    #[serde(flatten)]
    pub record: RecordResponse,
    pub created: bool,
}

pub async fn upsert(
    db: &Database,
    owner: &str,
    actor: CrmActor,
    input: UpsertInput,
    receipt: Option<ReceiptContext>,
) -> AppResult<UpsertResponse> {
    let context = (db.clone(), owner.to_owned(), actor, input, receipt.clone());
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, actor, input, receipt)| {
            Box::pin(async move {
                super::fence_owner(db, owner, tx).await?;
                if let Some(value) = records::receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return serde_json::from_value(value)
                        .map_err(|_| AppError::Internal("Invalid stored CRM result".into()));
                }
                let object = records::object_in_session(db, owner, &input.object, tx).await?;
                let (record, created) = if let Some(row) =
                    matching(db, owner, &object, &input.match_field, &input.values, tx).await?
                {
                    // Agents still carry server-bound source ordering. Re-reading a revision
                    // can never bypass protected fields or overwrite a newer observation.
                    records::patch_in_session(
                        db,
                        owner,
                        &row.id,
                        actor,
                        &PatchRecordInput {
                            expected_revision: input.expected_revision.ok_or_else(|| {
                                super::validation_error(
                                    "expected_revision",
                                    "matched records require the revision you inspected",
                                )
                            })?,
                            values: input.values.clone(),
                            protect: vec![],
                            unprotect: vec![],
                        },
                        &None,
                        tx,
                    )
                    .await
                    .map(|record| (record, false))?
                } else {
                    if input.expected_revision.is_some() {
                        return Err(super::validation_error(
                            "expected_revision",
                            "matched record no longer exists",
                        ));
                    }
                    records::create_in_session(
                        db,
                        owner,
                        actor,
                        &CreateRecordInput {
                            object: input.object.clone(),
                            values: input.values.clone(),
                            protect: vec![],
                        },
                        &None,
                        tx,
                    )
                    .await
                    .map(|record| (record, true))?
                };
                let response = UpsertResponse { record, created };
                records::insert_receipt(db, tx, receipt.as_ref(), &response, chrono::Utc::now())
                    .await?;
                Ok(response)
            })
        },
    )
    .await;
    records::recover_duplicate(db, receipt.as_ref(), result).await
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchPolicy {
    CreateOnly,
    UpdateOrCreate,
    SkipExisting,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportInput {
    pub object: String,
    /// Source column -> immutable CRM field key. Unmapped columns are ignored.
    pub mapping: BTreeMap<String, String>,
    pub rows: Vec<BTreeMap<String, Value>>,
    pub match_field: String,
    pub policy: MatchPolicy,
    #[serde(default)]
    pub snapshot: Option<ImportSnapshot>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportRow {
    pub index: usize,
    pub matched_id: Option<String>,
    pub revision: Option<i64>,
    pub action: String,
    pub errors: Vec<CrmValidationIssue>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub rows: Vec<ImportRow>,
    pub committed: bool,
    pub snapshot: ImportSnapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImportExpectation {
    pub index: usize,
    pub matched_id: Option<String>,
    pub revision: Option<i64>,
    pub action: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImportSnapshot {
    pub schema_revision: i64,
    pub payload_digest: String,
    pub rows: Vec<ImportExpectation>,
}

fn convert(
    object: &CrmObject,
    input: &ImportInput,
    row: &BTreeMap<String, Value>,
) -> AppResult<BTreeMap<String, Value>> {
    let mut values = BTreeMap::new();
    for (column, key) in &input.mapping {
        let field = object
            .fields
            .iter()
            .find(|f| f.key == *key && !f.archived)
            .ok_or_else(|| {
                super::validation_error(format!("mapping.{column}"), "unknown or archived field")
            })?;
        // A missing source column is an omitted PATCH field. Explicit null/blank
        // remains an intentional clear; create validation handles required fields.
        let Some(raw) = row.get(column).cloned() else {
            continue;
        };
        let value = if let Value::String(text) = &raw {
            let text = text.trim();
            if text.is_empty() {
                Value::Null
            } else {
                match &field.kind {
                    CrmFieldKind::Number { .. } => serde_json::from_str::<Value>(text)
                        .ok()
                        .filter(Value::is_number)
                        .ok_or_else(|| {
                            super::validation_error(
                                format!("values.{key}"),
                                "enter a finite number (percent values are fractions)",
                            )
                        })?,
                    CrmFieldKind::Currency { .. } => {
                        let negative = text.starts_with('-');
                        let text = text.strip_prefix('-').unwrap_or(text);
                        let parts: Vec<_> = text.split('.').collect();
                        if parts.len() > 2
                            || parts[0].is_empty()
                            || !parts[0].bytes().all(|c| c.is_ascii_digit())
                            || parts.get(1).is_some_and(|s| {
                                s.len() > 2 || !s.bytes().all(|c| c.is_ascii_digit())
                            })
                        {
                            return Err(super::validation_error(
                                format!("values.{key}"),
                                "currency requires a decimal amount with at most two places",
                            ));
                        }
                        let major: i128 = parts[0]
                            .parse()
                            .map_err(|_| super::validation_error(key, "amount is too large"))?;
                        let fraction = format!("{:<02}", parts.get(1).unwrap_or(&""))
                            .replace(' ', "0")
                            .parse::<i128>()
                            .unwrap_or(0);
                        let amount = major
                            .checked_mul(100)
                            .and_then(|v| v.checked_add(fraction))
                            .ok_or_else(|| super::validation_error(key, "amount is too large"))?;
                        let signed = if negative { -amount } else { amount };
                        if signed.abs() > 9_007_199_254_740_991 {
                            return Err(super::validation_error(
                                key,
                                "amount exceeds safe integer range",
                            ));
                        }
                        Value::from(signed as i64)
                    }
                    CrmFieldKind::Boolean {} => match text.to_ascii_lowercase().as_str() {
                        "true" | "yes" => Value::Bool(true),
                        "false" | "no" => Value::Bool(false),
                        _ => return Err(super::validation_error(key, "use true/false or yes/no")),
                    },
                    _ => raw,
                }
            }
        } else {
            raw
        };
        if values.insert(key.clone(), value).is_some() {
            return Err(super::validation_error(
                "mapping",
                "multiple columns map to the same field",
            ));
        }
    }
    Ok(values)
}

pub async fn import(
    db: &Database,
    owner: &str,
    actor: CrmActor,
    input: ImportInput,
    commit: bool,
    receipt: Option<ReceiptContext>,
) -> AppResult<ImportResult> {
    if input.rows.is_empty()
        || input.rows.len() > 100
        || input.mapping.len() > 100
        || serde_json::to_vec(&input).map_or(true, |s| s.len() > 512_000)
    {
        return Err(super::validation_error(
            "rows",
            "import requires 1–100 rows, at most 100 mapped columns and 512000 bytes",
        ));
    }
    let context = (db.clone(), owner.to_owned(), actor, input, receipt.clone());
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, actor, input, receipt)| {
            Box::pin(async move {
                super::fence_owner(db, owner, tx).await?;
                if commit {
                    if let Some(value) =
                        records::receipt_in_session(db, receipt.as_ref(), tx).await?
                    {
                        return serde_json::from_value(value)
                            .map_err(|_| AppError::Internal("Invalid CRM import receipt".into()));
                    }
                }
                let object = records::object_in_session(db, owner, &input.object, tx).await?;
                let mut rows = Vec::new();
                let mut prepared = Vec::new();
                let mut identities = Vec::<String>::new();
                let mut matched_ids = HashSet::new();
                for (index, row) in input.rows.iter().enumerate() {
                    let resolved: AppResult<_> = async {
                        let values = convert(&object, input, row)?;
                        let identity = values
                            .get(&input.match_field)
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim();
                        if identities
                            .iter()
                            .any(|existing| same_identity(existing, identity))
                        {
                            return Err(super::validation_error(
                                "match_field",
                                "duplicate identity within import",
                            ));
                        }
                        identities.push(identity.to_owned());
                        let matched =
                            matching(db, owner, &object, &input.match_field, &values, tx).await?;
                        if matched
                            .as_ref()
                            .is_some_and(|record| !matched_ids.insert(record.id.clone()))
                        {
                            return Err(super::validation_error(
                                "match_field",
                                "multiple import rows match the same record",
                            ));
                        }
                        if matched.is_some() && matches!(input.policy, MatchPolicy::CreateOnly) {
                            return Err(super::validation_error(
                                "match_field",
                                "identity already exists",
                            ));
                        }
                        let skip =
                            matched.is_some() && matches!(input.policy, MatchPolicy::SkipExisting);
                        if !skip {
                            records::prepare_values(
                                db,
                                tx,
                                owner,
                                &object,
                                &values,
                                matched.is_none(),
                            )
                            .await?;
                        }
                        Ok((values, matched, skip))
                    }
                    .await;
                    match resolved {
                        Ok((values, matched, skip)) => {
                            rows.push(ImportRow {
                                index,
                                matched_id: matched.as_ref().map(|r| r.id.clone()),
                                revision: matched.as_ref().map(|r| r.revision),
                                action: if skip {
                                    "skip"
                                } else if matched.is_some() {
                                    "update"
                                } else {
                                    "create"
                                }
                                .into(),
                                errors: vec![],
                            });
                            prepared.push((values, matched, skip));
                        }
                        Err(AppError::CrmValidationFailed(errors)) => rows.push(ImportRow {
                            index,
                            matched_id: None,
                            revision: None,
                            action: "error".into(),
                            errors,
                        }),
                        Err(error) => return Err(error),
                    }
                }
                use sha2::{Digest, Sha256};
                let mut payload = input.clone(); payload.snapshot = None;
                let snapshot = ImportSnapshot { schema_revision: object.schema_revision, payload_digest: hex::encode(Sha256::digest(crate::services::mcp_service::canonical_json(serde_json::to_value(payload).map_err(|_| AppError::Internal("Invalid import".into()))?).to_string())), rows: rows.iter().map(|row| ImportExpectation { index: row.index, matched_id: row.matched_id.clone(), revision: row.revision, action: row.action.clone() }).collect() };
                if commit && input.snapshot.as_ref() != Some(&snapshot) { return Err(super::validation_error("snapshot", "preview changed: schema, input or matched record identity/revision/action differs; preview again")); }
                if commit && rows.iter().any(|r| !r.errors.is_empty()) {
                    return Err(AppError::CrmValidationFailed(
                        rows.iter()
                            .flat_map(|r| {
                                r.errors.iter().map(move |e| CrmValidationIssue {
                                    path: format!("rows.{}.{}", r.index, e.path),
                                    message: e.message.clone(),
                                })
                            })
                            .collect(),
                    ));
                }
                if commit {
                    for (values, matched, skip) in prepared {
                        if skip {
                            continue;
                        }
                        if let Some(row) = matched {
                            records::patch_in_session(
                                db,
                                owner,
                                &row.id,
                                actor,
                                &PatchRecordInput {
                                    expected_revision: row.revision,
                                    values,
                                    protect: vec![],
                                    unprotect: vec![],
                                },
                                &None,
                                tx,
                            )
                            .await?;
                        } else {
                            records::create_in_session(
                                db,
                                owner,
                                actor,
                                &CreateRecordInput {
                                    object: input.object.clone(),
                                    values,
                                    protect: vec![],
                                },
                                &None,
                                tx,
                            )
                            .await?;
                        }
                    }
                }
                let response = ImportResult {
                    rows,
                    committed: commit,
                    snapshot,
                };
                if commit {
                    records::insert_receipt(
                        db,
                        tx,
                        receipt.as_ref(),
                        &response,
                        chrono::Utc::now(),
                    )
                    .await?;
                }
                Ok(response)
            })
        },
    )
    .await;
    records::recover_duplicate(db, receipt.as_ref(), result).await
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
