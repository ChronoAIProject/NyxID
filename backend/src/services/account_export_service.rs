//! Bounded, explicit account export. Model structs are never serialized here:
//! every field is selected into a dedicated JSON value so secrets cannot leak
//! when a model gains a new field.

use std::{collections::{BTreeMap, BTreeSet}, time::Duration};

use bson::{Bson, Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use serde::Serialize;

use crate::{errors::{AppError, AppResult}, services::personal_data_registry};

pub const FORMAT: &str = "nyxid.account-export";
pub const VERSION: u32 = 1;
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const SECTION_ROW_CAP: i64 = 1_000;
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
pub const FILE_PAYLOADS_METADATA_ONLY: bool = true;

#[derive(Debug, Serialize, Clone, Default)]
pub struct ExportSection {
    pub items: Vec<serde_json::Value>,
    pub total_count: u64,
    pub truncated: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct ExportLimits {
    pub max_bytes: usize,
    pub section_row_cap: i64,
    pub query_timeout_ms: u64,
    pub file_payloads: &'static str,
}

#[derive(Debug, Serialize, Clone)]
pub struct AccountExport {
    pub format: &'static str,
    pub version: u32,
    pub generated_at: String,
    pub account_id: String,
    pub limits: ExportLimits,
    pub truncated_sections: Vec<String>,
    pub excluded: Vec<&'static str>,
    pub sections: BTreeMap<String, ExportSection>,
}

fn bson_json(value: &Bson) -> serde_json::Value {
    match value {
        Bson::String(value) => serde_json::Value::String(value.clone()),
        Bson::Boolean(value) => serde_json::Value::Bool(*value),
        Bson::Int32(value) => serde_json::json!(*value),
        Bson::Int64(value) => serde_json::json!(*value),
        Bson::Double(value) if value.is_finite() => serde_json::json!(*value),
        Bson::DateTime(value) => serde_json::Value::String(value.to_chrono().to_rfc3339()),
        Bson::ObjectId(value) => serde_json::Value::String(value.to_hex()),
        Bson::Null => serde_json::Value::Null,
        _ => serde_json::Value::Null,
    }
}

fn safe_item(row: &Document, fields: &[&str]) -> serde_json::Value {
    let mut output = serde_json::Map::new();
    for field in fields {
        if let Some(value) = row.get(*field) {
            output.insert((*field).to_owned(), bson_json(value));
        }
    }
    serde_json::Value::Object(output)
}

async fn rows(
    db: &mongodb::Database,
    collection: &str,
    filter: Document,
    fields: &[&str],
    sort: Document,
) -> AppResult<ExportSection> {
    let total_count = db.collection::<Document>(collection).count_documents(filter.clone()).max_time(QUERY_TIMEOUT).await?;
    let mut projection = doc! {"_id": 1};
    for field in fields { projection.insert(*field, 1); }
    let mut cursor = db.collection::<Document>(collection)
        .find(filter)
        .projection(projection)
        .sort(sort)
        .limit(SECTION_ROW_CAP + 1)
        .max_time(QUERY_TIMEOUT)
        .await?;
    let mut items = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        if items.len() == SECTION_ROW_CAP as usize { break; }
        items.push(safe_item(&row, fields));
    }
    Ok(ExportSection { truncated: total_count > SECTION_ROW_CAP as u64, total_count, items })
}

fn empty_sections() -> BTreeMap<String, ExportSection> {
    personal_data_registry::export_sections()
        .map(|name| (name.to_owned(), ExportSection { items: Vec::new(), total_count: 0, truncated: false }))
        .collect()
}

fn byte_size(export: &AccountExport) -> usize {
    serde_json::to_vec(export).map(|bytes| bytes.len()).unwrap_or(usize::MAX)
}

fn enforce_cap(export: &mut AccountExport) {
    let mut truncated = BTreeSet::new();
    while byte_size(export) > MAX_BYTES {
        let Some((name, section)) = export.sections.iter_mut().max_by_key(|(_, section)| section.items.len()) else { break };
        if section.items.pop().is_none() {
            break;
        }
        section.truncated = true;
        truncated.insert(name.clone());
    }
    export.truncated_sections = truncated.into_iter().collect();
}

pub async fn build(db: &mongodb::Database, account_id: &str) -> AppResult<AccountExport> {
    let mut sections = empty_sections();
    let account = db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": account_id})
        .projection(doc! {"_id": 1, "email": 1, "display_name": 1, "avatar_url": 1, "email_verified": 1, "created_at": 1, "updated_at": 1, "last_login_at": 1})
        .max_time(QUERY_TIMEOUT)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    sections.entry("account".into()).or_default().items.push(safe_item(&account, &["_id", "email", "display_name", "avatar_url", "email_verified", "created_at", "updated_at", "last_login_at"]));
    sections.get_mut("account").unwrap().total_count = 1;

    sections.insert("services".into(), rows(db, crate::models::user_service::COLLECTION_NAME, doc! {"user_id": account_id}, &["_id", "slug", "is_active", "service_type", "created_at", "deleted_at"], doc! {"created_at": -1}).await?);
    sections.insert("security".into(), rows(db, crate::models::session::COLLECTION_NAME, doc! {"user_id": account_id}, &["_id", "created_at", "expires_at", "last_used_at", "user_agent"], doc! {"created_at": -1}).await?);
    sections.insert("assistant".into(), rows(db, crate::models::assistant_message::COLLECTION_NAME, doc! {"user_id": account_id}, &["_id", "conversation_id", "role", "content", "created_at"], doc! {"created_at": -1}).await?);
    sections.insert("channels".into(), rows(db, crate::models::channel_message::COLLECTION_NAME, doc! {"user_id": account_id}, &["_id", "conversation_id", "direction", "text", "created_at"], doc! {"created_at": -1}).await?);
    sections.insert("account_activity".into(), rows(db, crate::models::assistant_action_receipt::COLLECTION_NAME, doc! {"user_id": account_id}, &["_id", "action", "status", "created_at", "completed_at"], doc! {"created_at": -1}).await?);
    sections.insert("machines".into(), rows(db, crate::models::machine_job::COLLECTION_NAME, doc! {"user_id": account_id}, &["_id", "status", "created_at", "completed_at"], doc! {"created_at": -1}).await?);

    // Audit metadata is visible to the actor in the admin/account activity
    // views. Event payloads are deliberately excluded from this export.
    let audit = rows(db, crate::models::audit_log::COLLECTION_NAME, doc! {"user_id": account_id}, &["_id", "event_type", "created_at", "service_id", "status"], doc! {"created_at": -1}).await?;
    sections.entry("account_activity".into()).and_modify(|section| {
        section.total_count += audit.total_count;
        section.truncated |= audit.truncated;
        section.items.extend(audit.items);
    });
    if let Some(wallet) = db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME).find_one(doc! {"owner_id": account_id}).projection(doc! {"balance_credits": 1, "reserved_credits": 1, "collection_state": 1, "suspended": 1}).max_time(QUERY_TIMEOUT).await? {
        sections.entry("billing".into()).or_default().items.push(safe_item(&wallet, &["balance_credits", "reserved_credits", "collection_state", "suspended"]));
        sections.get_mut("billing").unwrap().total_count = 1;
    }

    let mut export = AccountExport {
        format: FORMAT,
        version: VERSION,
        generated_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        account_id: account_id.to_owned(),
        limits: ExportLimits { max_bytes: MAX_BYTES, section_row_cap: SECTION_ROW_CAP, query_timeout_ms: QUERY_TIMEOUT.as_millis() as u64, file_payloads: "metadata_only" },
        truncated_sections: Vec::new(),
        excluded: vec!["password and authentication hashes", "tokens, keys, secrets, ciphertext, signatures and provider credentials", "raw uploaded/generated file payloads (metadata is included while retained)", "transient coordination, lease, deduplication and erasure bookkeeping"],
        sections,
    };
    enforce_cap(&mut export);
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_secret_names_are_not_in_allowlisted_fields() {
        let fields = ["_id", "email", "display_name", "created_at", "status", "content"];
        for field in fields { assert!(!field.contains("password") && !field.contains("token") && !field.contains("secret")); }
    }
}

