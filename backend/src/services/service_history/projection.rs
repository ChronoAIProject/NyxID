use bson::{Bson, Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::definitions::{self, Action};
use crate::models::service_change_event::{
    COLLECTION_NAME, HistoryContext, SafeFieldChange, ServiceChangeEvent, ServiceChangeSummary,
};

pub type EventIds = Arc<Mutex<HashMap<(String, String, String, u64), String>>>;

fn normalized(entity: &str, document: &Document) -> Document {
    let normalized = match entity {
        "user_services" => {
            bson::from_document::<crate::models::user_service::UserService>(document.clone())
                .ok()
                .and_then(|service| {
                    let binding =
                        crate::services::platform_key_service::binding(&service).to_string();
                    let mut normalized = bson::to_document(&service).ok()?;
                    normalized.insert("credential_binding", binding);
                    Some(normalized)
                })
        }
        "user_api_keys" => {
            bson::from_document::<crate::models::user_api_key::UserApiKey>(document.clone())
                .ok()
                .and_then(|key| bson::to_document(&key).ok())
        }
        "user_endpoints" => {
            bson::from_document::<crate::models::user_endpoint::UserEndpoint>(document.clone())
                .ok()
                .and_then(|endpoint| bson::to_document(&endpoint).ok())
        }
        _ => None,
    };
    let mut result = document.clone();
    if let Some(normalized) = normalized {
        for (key, value) in normalized {
            let merged = merge_defaults(result.get(&key), value);
            result.insert(key, merged);
        }
    }
    result
}

// Deserialization supplies known defaults, but must never erase future nested
// fields while deciding whether a real persisted configuration change occurred.
fn merge_defaults(raw: Option<&Bson>, normalized: Bson) -> Bson {
    match (raw, normalized) {
        (Some(Bson::Document(raw)), Bson::Document(normalized)) => {
            let mut merged = raw.clone();
            for (key, value) in normalized {
                merged.insert(&key, merge_defaults(raw.get(&key), value));
            }
            Bson::Document(merged)
        }
        (Some(Bson::Array(raw)), Bson::Array(normalized)) => Bson::Array(
            normalized
                .into_iter()
                .enumerate()
                .map(|(index, value)| merge_defaults(raw.get(index), value))
                .collect(),
        ),
        (_, normalized) => normalized,
    }
}

fn operational(entity: &str, key: &str) -> bool {
    matches!(
        key,
        "_id"
            | "user_id"
            | "created_at"
            | "updated_at"
            | "state_version"
            | "created_by"
            | "last_change"
            | "last_used_at"
            | "credential_epoch"
            | "service_history_ref_epoch"
            | "source"
            | "source_id"
    ) || (entity == "user_api_keys"
        && matches!(
            key,
            "expires_at" | "error_message" | "status" | "oauth_attempt_nonce"
        ))
}

fn safe_value(field: &str, value: &Bson) -> Option<serde_json::Value> {
    match field {
        "is_active"
        | "admin_only"
        | "identity_include_user_id"
        | "identity_include_email"
        | "identity_include_name"
        | "forward_access_token"
        | "inject_delegation_token" => value.as_bool().map(Into::into),
        "node_priority" => value
            .as_i32()
            .map(serde_json::Value::from)
            .or_else(|| value.as_i64().map(Into::into)),
        "auth_method" => enum_value(
            value,
            crate::services::user_service_service::VALID_AUTH_METHODS,
        ),
        "ssh_auth_mode" => value
            .as_str()
            .and_then(|v| v.parse::<crate::models::ssh_auth_mode::SshAuthMode>().ok())
            .map(|v| v.as_str().into()),
        "identity_propagation_mode" => enum_value(
            value,
            crate::services::user_service_service::VALID_IDENTITY_MODES,
        ),
        "credential_binding" => enum_value(value, &["user", "platform"]),
        "node_id" => value
            .as_str()
            .filter(|v| uuid::Uuid::parse_str(v).is_ok())
            .map(Into::into),
        "default_request_headers" => value.as_array().map(|rows| {
            let names: Vec<_> = rows
                .iter()
                .take(16)
                .filter_map(|row| row.as_document()?.get_str("name").ok())
                .filter(|name| {
                    name.len() <= 256
                        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
                .map(|name| name.to_ascii_lowercase())
                .collect();
            serde_json::json!({ "count": rows.len(), "names": names })
        }),
        "ws_frame_injections" => value.as_array().map(|rows| {
            let directions: Vec<_> = rows
                .iter()
                .take(32)
                .filter_map(|row| row.as_document())
                .filter_map(|row| {
                    let direction = row.get_str("direction").unwrap_or("downstream");
                    matches!(direction, "downstream" | "upstream").then_some(direction)
                })
                .collect();
            serde_json::json!({ "count": rows.len(), "directions": directions })
        }),
        _ => None,
    }
}
fn enum_value(value: &Bson, allowed: &[&str]) -> Option<serde_json::Value> {
    value
        .as_str()
        .filter(|v| allowed.contains(v))
        .map(Into::into)
}

pub fn diff(
    entity: &str,
    before: &Document,
    after: &Document,
    routine_refresh: bool,
) -> (Vec<SafeFieldChange>, bool, bool) {
    let before = normalized(entity, before);
    let after = normalized(entity, after);
    let mut keys: BTreeSet<&str> = before
        .keys()
        .chain(after.keys())
        .map(String::as_str)
        .collect();
    if entity == "user_services" {
        keys.insert("credential_binding");
    }
    let mut changes = Vec::new();
    let mut additional = false;
    let mut changed = false;
    for key in keys {
        if operational(entity, key)
            || (routine_refresh
                && entity == "user_api_keys"
                && matches!(
                    key,
                    "credential_encrypted"
                        | "access_token_encrypted"
                        | "refresh_token_encrypted"
                        | "token_scopes"
                        | "credential_type"
                ))
        {
            continue;
        }
        let (old, new) = (
            before.get(key).cloned().unwrap_or(Bson::Null),
            after.get(key).cloned().unwrap_or(Bson::Null),
        );
        if old == new {
            continue;
        }
        changed = true;
        let field = match key {
            "credential_encrypted"
            | "access_token_encrypted"
            | "refresh_token_encrypted"
            | "last_authorized_at" => "credential",
            "user_oauth_client_id_encrypted" | "user_oauth_client_secret_encrypted" => {
                "oauth_application"
            }
            other => other,
        };
        if definitions::field(field).is_some() {
            if !changes.iter().any(|c: &SafeFieldChange| c.field == field) {
                changes.push(SafeFieldChange {
                    field: field.into(),
                    before: safe_value(field, &old),
                    after: safe_value(field, &new),
                });
            }
        } else {
            additional = true;
        }
    }
    if changes.len() > 48 {
        changes.truncate(48);
        additional = true;
    }
    (changes, additional, changed)
}

fn has_material(document: Option<&Document>) -> bool {
    document.is_some_and(|d| {
        [
            "credential_encrypted",
            "access_token_encrypted",
            "refresh_token_encrypted",
        ]
        .iter()
        .any(|field| d.get(*field).is_some_and(|v| !matches!(v, Bson::Null)))
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn record_changes(
    db: &Database,
    session: &mut ClientSession,
    entity: &str,
    before: &[Document],
    after: &[Document],
    context: &HistoryContext,
    ids: &EventIds,
    routine_refresh: bool,
) -> mongodb::error::Result<()> {
    let entity_ids: BTreeSet<_> = before
        .iter()
        .chain(after)
        .filter_map(|d| d.get_str("_id").ok())
        .collect();
    for entity_id in entity_ids {
        let old = before.iter().find(|d| d.get_str("_id") == Ok(entity_id));
        let new = after.iter().find(|d| d.get_str("_id") == Ok(entity_id));
        let empty = Document::new();
        let (mut changes, additional_changes, changed) = diff(
            entity,
            old.unwrap_or(&empty),
            new.unwrap_or(&empty),
            routine_refresh,
        );
        if old.is_some() && new.is_some() && !changed {
            continue;
        }
        let action = if entity == "user_services" && new.is_none() {
            Action::Deleted
        } else if entity == "user_services" && old.is_none() {
            Action::Created
        } else if entity == "user_services"
            && new
                .and_then(|d| d.get_datetime("deleted_at").ok())
                .is_some()
            && old
                .and_then(|d| d.get_datetime("deleted_at").ok())
                .is_none()
        {
            Action::Deleted
        } else if entity == "user_api_keys"
            && (new.is_none() || (has_material(old) && !has_material(new)))
        {
            Action::CredentialRemoved
        } else if entity == "user_api_keys" && old.is_none() {
            Action::CredentialReplaced
        } else if entity == "user_api_keys"
            && new.and_then(|d| d.get_str("credential_type").ok()) == Some("oauth2")
            && new
                .and_then(|d| d.get_datetime("last_authorized_at").ok())
                .is_some()
            && old.and_then(|d| d.get("last_authorized_at"))
                != new.and_then(|d| d.get("last_authorized_at"))
        {
            Action::CredentialReauthorized
        } else if changes.iter().any(|c| c.field == "credential_binding") {
            Action::CredentialBindingChanged
        } else if changes
            .iter()
            .any(|c| c.field == "credential" || c.field == "api_key_id")
        {
            Action::CredentialReplaced
        } else if entity == "user_services" && changes.iter().any(|c| c.field == "is_active") {
            if new
                .and_then(|d| d.get_bool("is_active").ok())
                .unwrap_or(false)
            {
                Action::Enabled
            } else {
                Action::Disabled
            }
        } else if changes
            .iter()
            .any(|c| c.field == "node_id" || c.field == "node_priority")
        {
            Action::RoutingChanged
        } else if changes.iter().any(|c| c.field.starts_with("ssh_")) {
            Action::SshChanged
        } else {
            Action::Updated
        };
        if let Some(change) = changes.iter_mut().find(|c| c.field == "token_scopes") {
            let provider_id = new
                .or(old)
                .and_then(|d| d.get_str("provider_config_id").ok());
            if let Some(provider_id) = provider_id
                && let Some(provider) = db
                    .collection::<crate::models::provider_config::ProviderConfig>(
                        crate::models::provider_config::COLLECTION_NAME,
                    )
                    .find_one(doc! { "_id": provider_id })
                    .session(&mut *session)
                    .await?
            {
                let reviewed = crate::services::scope_catalog::for_provider(&provider.slug)
                    .unwrap_or_default();
                let scopes = |document: Option<&Document>| {
                    document
                        .and_then(|d| d.get_str("token_scopes").ok())
                        .map(|raw| {
                            let values: BTreeSet<_> = raw
                                .split_whitespace()
                                .filter(|value| reviewed.iter().any(|d| d.scope == *value))
                                .take(100)
                                .collect();
                            serde_json::json!(values)
                        })
                };
                change.before = scopes(old);
                change.after = scopes(new);
            }
        }
        if entity == "user_services" {
            record_service(
                db,
                session,
                entity,
                entity_id,
                new.or(old).expect("entity exists"),
                old.is_none(),
                context,
                ids,
                action,
                &changes,
                additional_changes,
            )
            .await?;
        } else {
            let reference = if entity == "user_endpoints" {
                "endpoint_id"
            } else {
                "api_key_id"
            };
            let mut cursor = db
                .collection::<Document>("user_services")
                .find(doc! { reference: entity_id })
                .batch_size(128)
                .session(&mut *session)
                .await?;
            while cursor.advance(&mut *session).await? {
                let service = cursor.deserialize_current()?;
                record_service(
                    db,
                    session,
                    entity,
                    entity_id,
                    &service,
                    old.is_none(),
                    context,
                    ids,
                    action,
                    &changes,
                    additional_changes,
                )
                .await?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn record_service(
    db: &Database,
    session: &mut ClientSession,
    entity: &str,
    entity_id: &str,
    service: &Document,
    inserted: bool,
    context: &HistoryContext,
    ids: &EventIds,
    action: Action,
    changes: &[SafeFieldChange],
    additional_changes: bool,
) -> mongodb::error::Result<()> {
    let service_id = service
        .get_str("_id")
        .map_err(|_| mongodb::error::Error::custom("Service history missing identity"))?;
    let owner_id = service
        .get_str("user_id")
        .map_err(|_| mongodb::error::Error::custom("Service history missing owner"))?;
    let ordinal = db.collection::<ServiceChangeEvent>(COLLECTION_NAME).count_documents(doc! {
                "service_id": service_id, "entity_id": entity_id, "change_group_id": &context.change_group_id,
            }).session(&mut *session).await?;
    let id = ids
        .lock()
        .expect("history event IDs poisoned")
        .entry((
            context.change_group_id.clone(),
            entity_id.into(),
            service_id.into(),
            ordinal,
        ))
        .or_insert_with(|| Uuid::new_v4().to_string())
        .clone();
    let head = db.collection::<crate::models::service_change_event::ServiceHistoryHead>(crate::models::service_change_event::HEADS_COLLECTION_NAME)
                .find_one_and_update(doc! { "_id": service_id }, doc! { "$inc": { "sequence": 1_i64 } })
                .upsert(true).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await.map_err(|error| {
                    if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000) || matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 11000) {
                        mongodb::error::Error::custom(super::transaction::ConcurrentHeadCreation)
                    } else { error }
                })?
                .ok_or_else(|| mongodb::error::Error::custom("Missing service history sequence"))?;
    let committed_at = bson::DateTime::from_chrono(Utc::now()).to_chrono();
    let event = ServiceChangeEvent {
        id,
        schema_version: 1,
        service_sequence: head.sequence,
        service_id: service_id.into(),
        service_slug: service
            .get_str("slug")
            .ok()
            .filter(|slug| {
                slug.len() <= 128 && slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            .unwrap_or("service")
            .into(),
        owner_id: owner_id.into(),
        change_group_id: context.change_group_id.clone(),
        entity_type: entity.into(),
        entity_id: entity_id.into(),
        action: action.definition().code.into(),
        actor: context.actor.clone(),
        operation: context.operation.clone(),
        committed_at,
        changes: if action == Action::Created || action == Action::Deleted {
            vec![]
        } else {
            changes.to_vec()
        },
        additional_changes: additional_changes
            && action != Action::Created
            && action != Action::Deleted,
        audited_at: None,
        audit_log_id: None,
        audit_attempts: 0,
        next_audit_attempt_at: None,
    };
    db.collection::<ServiceChangeEvent>(COLLECTION_NAME)
        .insert_one(&event)
        .session(&mut *session)
        .await?;
    let summary = ServiceChangeSummary {
        actor: event.actor,
        at: committed_at,
        action: event.action,
        change_group_id: event.change_group_id,
    };
    let summary = bson::to_bson(&summary)?;
    if !(entity == "user_services" && inserted)
        && service
            .get_document("created_by")
            .ok()
            .and_then(|d| d.get_str("change_group_id").ok())
            == Some(context.change_group_id.as_str())
    {
        return Ok(());
    }
    let field = if entity == "user_services" && inserted {
        "created_by"
    } else {
        "last_change"
    };
    db.collection::<Document>("user_services")
        .update_one(
            doc! { "_id": service_id, "user_id": owner_id },
            doc! { "$set": { field: summary } },
        )
        .session(&mut *session)
        .await?;
    Ok(())
}
