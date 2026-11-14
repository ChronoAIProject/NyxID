//! Bounded, explicit account export. Model structs are never serialized here:
//! every field is selected into a dedicated JSON value so secrets cannot leak
//! when a model gains a new field.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use bson::{Bson, Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use serde::Serialize;

use crate::{
    errors::{AppError, AppResult},
    services::personal_data_registry,
};

pub const FORMAT: &str = "nyxid.account-export";
pub const VERSION: u32 = 1;
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const SECTION_ROW_CAP: i64 = 1_000;
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
pub const FILE_PAYLOADS_METADATA_ONLY: bool = true;
const SECTION_ITEM_CAP: usize = SECTION_ROW_CAP as usize;

// These are the only fields used by the registry-driven fallback exporter.
// They are deliberately boring, owner-visible metadata fields; adding a
// model field never makes it appear in an export accidentally.
const COMMON_SAFE_FIELDS: &[&str] = &[
    "_id",
    "user_id",
    "owner_id",
    "owner_user_id",
    "conversation_id",
    "service_id",
    "service_slug",
    "slug",
    "name",
    "display_name",
    "kind",
    "type",
    "status",
    "state",
    "role",
    "action",
    "direction",
    "content_type",
    "filename",
    "size",
    "created_at",
    "updated_at",
    "completed_at",
    "expires_at",
    "deleted_at",
    "email_verified",
    "last_login_at",
];

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

fn owner_filter(entry: &personal_data_registry::Entry, account_id: &str) -> Option<Document> {
    let personal_fields = match entry.owner {
        personal_data_registry::OwnerSelector::Fields(fields) if !fields.is_empty() => fields,
        personal_data_registry::OwnerSelector::Fields(_)
        | personal_data_registry::OwnerSelector::NotPersonal => return None,
    };
    if personal_fields.len() == 1 && personal_fields[0] == "_id" {
        return Some(doc! {"_id": account_id});
    }
    let clauses = personal_fields
        .iter()
        .map(|field| {
            let mut clause = Document::new();
            clause.insert(*field, account_id);
            clause
        })
        .collect::<Vec<_>>();
    Some(if clauses.len() == 1 {
        clauses.into_iter().next().unwrap()
    } else {
        doc! {"$or": clauses}
    })
}

fn append_section(
    sections: &mut BTreeMap<String, ExportSection>,
    name: &str,
    mut incoming: ExportSection,
) {
    let section = sections.entry(name.to_owned()).or_default();
    section.total_count = section.total_count.saturating_add(incoming.total_count);
    section.truncated |= incoming.truncated;
    let remaining = SECTION_ITEM_CAP.saturating_sub(section.items.len());
    if incoming.items.len() > remaining {
        incoming.items.truncate(remaining);
        section.truncated = true;
    }
    section.items.extend(incoming.items);
}

fn file_metadata_fields(collection: &str) -> Option<&'static [&'static str]> {
    match collection {
        "assistant_attachments" | "async_service_operation_artifacts" => Some(&[
            "_id",
            "filename",
            "content_type",
            "size",
            "created_at",
            "expires_at",
        ]),
        // Delivery rows intentionally contain only lifecycle metadata. The
        // attachment itself is exported from the two collections above.
        "assistant_artifact_deliveries" => Some(&["_id", "expires_at"]),
        _ => None,
    }
}

fn fields_for_collection(collection: &str) -> &'static [&'static str] {
    match collection {
        // Shared catalog attribution keeps only the opaque account id and
        // operational identifiers. Human names and contact fields are never
        // selected from this retained collection.
        "downstream_services" => &[
            "_id",
            "owner_user_id",
            "service_id",
            "slug",
            "status",
            "created_at",
            "updated_at",
        ],
        _ => COMMON_SAFE_FIELDS,
    }
}

async fn rows(
    db: &mongodb::Database,
    collection: &str,
    filter: Document,
    fields: &[&str],
    sort: Document,
) -> AppResult<ExportSection> {
    let total_count = db
        .collection::<Document>(collection)
        .count_documents(filter.clone())
        .max_time(QUERY_TIMEOUT)
        .await?;
    let mut projection = doc! {"_id": 1};
    for field in fields {
        projection.insert(*field, 1);
    }
    let mut cursor = db
        .collection::<Document>(collection)
        .find(filter)
        .projection(projection)
        .sort(sort)
        .limit(SECTION_ROW_CAP + 1)
        .max_time(QUERY_TIMEOUT)
        .await?;
    let mut items = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        if items.len() == SECTION_ROW_CAP as usize {
            break;
        }
        items.push(safe_item(&row, fields));
    }
    Ok(ExportSection {
        truncated: total_count > SECTION_ROW_CAP as u64,
        total_count,
        items,
    })
}

fn empty_sections() -> BTreeMap<String, ExportSection> {
    personal_data_registry::export_sections()
        .map(|name| {
            (
                name.to_owned(),
                ExportSection {
                    items: Vec::new(),
                    total_count: 0,
                    truncated: false,
                },
            )
        })
        .collect()
}

fn byte_size(export: &AccountExport) -> usize {
    serde_json::to_vec(export)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn enforce_cap(export: &mut AccountExport) {
    let mut truncated = BTreeSet::new();
    while byte_size(export) > MAX_BYTES {
        let Some((name, section)) = export
            .sections
            .iter_mut()
            .max_by_key(|(_, section)| section.items.len())
        else {
            break;
        };
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
    sections
        .entry("account".into())
        .or_default()
        .items
        .push(safe_item(
            &account,
            &[
                "_id",
                "email",
                "display_name",
                "avatar_url",
                "email_verified",
                "created_at",
                "updated_at",
                "last_login_at",
            ],
        ));
    sections.get_mut("account").unwrap().total_count = 1;

    // Dedicated allowlists cover content, lifecycle metadata, files and
    // billing. The registry-driven pass below covers every other personal
    // section without serializing an entire model.
    let mut handled = BTreeSet::from([
        crate::models::user_service::COLLECTION_NAME,
        crate::models::session::COLLECTION_NAME,
        crate::models::assistant_message::COLLECTION_NAME,
        crate::models::channel_message::COLLECTION_NAME,
        crate::models::assistant_action_receipt::COLLECTION_NAME,
        crate::models::machine_job::COLLECTION_NAME,
        crate::models::audit_log::COLLECTION_NAME,
        crate::models::billing_wallet::COLLECTION_NAME,
        "assistant_attachments",
        "assistant_artifact_deliveries",
        "async_service_operation_artifacts",
        crate::models::credit_grant::COLLECTION_NAME,
        crate::models::usage_meter::COLLECTION_NAME,
    ]);
    let explicit = [
        (
            "services",
            crate::models::user_service::COLLECTION_NAME,
            &[
                "_id",
                "slug",
                "is_active",
                "service_type",
                "created_at",
                "deleted_at",
            ][..],
        ),
        (
            "security",
            crate::models::session::COLLECTION_NAME,
            &["_id", "created_at", "expires_at", "last_used_at"][..],
        ),
        (
            "assistant",
            crate::models::assistant_message::COLLECTION_NAME,
            &["_id", "conversation_id", "role", "content", "created_at"][..],
        ),
        (
            "channels",
            crate::models::channel_message::COLLECTION_NAME,
            &["_id", "conversation_id", "direction", "text", "created_at"][..],
        ),
        (
            "account_activity",
            crate::models::assistant_action_receipt::COLLECTION_NAME,
            &["_id", "action", "status", "created_at", "completed_at"][..],
        ),
        (
            "machines",
            crate::models::machine_job::COLLECTION_NAME,
            &["_id", "status", "created_at", "completed_at"][..],
        ),
    ];
    for (section, collection, fields) in explicit {
        append_section(
            &mut sections,
            section,
            rows(
                db,
                collection,
                doc! {"user_id": account_id},
                fields,
                doc! {"created_at": -1},
            )
            .await?,
        );
    }

    // Audit metadata is visible to the actor in account activity. Event
    // payloads and chain material are deliberately excluded.
    append_section(
        &mut sections,
        "account_activity",
        rows(
            db,
            crate::models::audit_log::COLLECTION_NAME,
            doc! {"user_id": account_id},
            &["_id", "event_type", "created_at", "service_id", "status"],
            doc! {"created_at": -1},
        )
        .await?,
    );

    for (collection, fields) in [
        (
            "assistant_attachments",
            file_metadata_fields("assistant_attachments").unwrap(),
        ),
        (
            "assistant_artifact_deliveries",
            file_metadata_fields("assistant_artifact_deliveries").unwrap(),
        ),
        (
            "async_service_operation_artifacts",
            file_metadata_fields("async_service_operation_artifacts").unwrap(),
        ),
    ] {
        append_section(
            &mut sections,
            "assistant",
            rows(
                db,
                collection,
                doc! {"user_id": account_id},
                fields,
                doc! {"created_at": -1},
            )
            .await?,
        );
    }

    // Billing output is a bounded owner-visible summary. Amounts and statuses
    // are useful to the account holder; provider identifiers, ledger hashes,
    // settlement locks and raw billing documents are not exported.
    if let Some(wallet) = db
        .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
        .find_one(doc! {"owner_id": account_id})
        .projection(doc! {"balance_credits": 1, "reserved_credits": 1, "collection_state": 1, "suspended": 1})
        .max_time(QUERY_TIMEOUT)
        .await?
    {
        append_section(&mut sections, "billing", ExportSection {
            items: vec![safe_item(&wallet, &["balance_credits", "reserved_credits", "collection_state", "suspended"])],
            total_count: 1,
            truncated: false,
        });
    }
    append_section(
        &mut sections,
        "billing",
        rows(
            db,
            crate::models::credit_grant::COLLECTION_NAME,
            doc! {"recipient_user_id": account_id},
            &[
                "_id",
                "amount",
                "remaining",
                "reserved",
                "status",
                "scope",
                "expires_at",
                "reason",
                "created_at",
                "updated_at",
                "consumed_at",
                "expired_at",
                "revoked_at",
            ],
            doc! {"created_at": -1},
        )
        .await?,
    );
    append_section(
        &mut sections,
        "billing",
        rows(
            db,
            crate::models::usage_meter::COLLECTION_NAME,
            doc! {"actor_user_id": account_id},
            &[
                "_id",
                "service_slug",
                "metric",
                "model",
                "quantity",
                "reserved_credits",
                "status",
                "created_at",
                "finalized_at",
            ],
            doc! {"created_at": -1},
        )
        .await?,
    );

    for entry in personal_data_registry::entries() {
        let personal_data_registry::Export::Section(section) = entry.export else {
            continue;
        };
        if handled.contains(entry.collection) {
            continue;
        }
        let Some(filter) = owner_filter(entry, account_id) else {
            continue;
        };
        let result = rows(
            db,
            entry.collection,
            filter,
            fields_for_collection(entry.collection),
            doc! {"created_at": -1},
        )
        .await?;
        append_section(&mut sections, section, result);
        handled.insert(entry.collection);
    }

    let mut export = AccountExport {
        format: FORMAT,
        version: VERSION,
        generated_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        account_id: account_id.to_owned(),
        limits: ExportLimits {
            max_bytes: MAX_BYTES,
            section_row_cap: SECTION_ROW_CAP,
            query_timeout_ms: QUERY_TIMEOUT.as_millis() as u64,
            file_payloads: if FILE_PAYLOADS_METADATA_ONLY {
                "metadata_only"
            } else {
                "included"
            },
        },
        truncated_sections: Vec::new(),
        excluded: vec![
            "password and authentication hashes",
            "tokens, keys, secrets, ciphertext, signatures and provider credentials",
            "raw uploaded/generated file payloads (metadata is included while retained)",
            "transient coordination, lease, deduplication and erasure bookkeeping",
        ],
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
        let mut fields = COMMON_SAFE_FIELDS.to_vec();
        fields.extend_from_slice(&[
            "email",
            "avatar_url",
            "content",
            "event_type",
            "service_type",
            "last_used_at",
            "finalized_at",
        ]);
        for field in fields {
            assert!(
                !field.contains("password")
                    && !field.contains("token")
                    && !field.contains("secret")
            );
        }
    }

    #[test]
    fn file_allowlist_is_metadata_only() {
        for collection in [
            "assistant_attachments",
            "assistant_artifact_deliveries",
            "async_service_operation_artifacts",
        ] {
            let fields = file_metadata_fields(collection).expect("file collection is registered");
            assert!(fields.iter().all(|field| {
                matches!(
                    *field,
                    "_id" | "filename" | "content_type" | "size" | "created_at" | "expires_at"
                )
            }));
            assert!(
                !fields
                    .iter()
                    .any(|field| field.ends_with("encrypted") || field.contains("payload"))
            );
        }
    }
}
