//! Durable one-Sandbox admission. A remote timeout never proves absence.
//! The occupied owner slot and permanent creation identity share one document.
//! Owner-erasure fencing and that unique slot commit in the same transaction.

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, Bson, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::nyxbot_cma_gateway::{
    CmaEventSnapshot, CmaGateway, CmaGatewayError, CmaSandbox, validate_idempotency_key,
    validate_remote_id,
};
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::nyxbot_cma_sandbox::{
        COLLECTION_NAME, NyxbotCmaSandbox, SandboxAdmissionState, SandboxObservationState,
    },
};

pub const MAX_RECOVERY_BYTES: usize = 64 * 1024;
pub const MAX_INVENTORY_PAGES: usize = 20;
pub const MAX_LOCAL_PAGE_SIZE: i64 = 100;
pub const OCCUPIED_SLOT_INDEX: &str = "cma_sandbox_one_occupied_owner";
pub const CREATION_KEY_INDEX: &str = "cma_sandbox_owner_creation_key";
pub const REMOTE_REFERENCE_INDEX: &str = "cma_sandbox_owner_service_remote";

/// Recovery bytes deliberately have no Debug or Serialize implementation.
pub struct NormalizedCreationRequest {
    pub hash: String,
    bytes: Zeroizing<Vec<u8>>,
}

fn invalid_request() -> AppError {
    AppError::ValidationError("Invalid CMA creation request".into())
}
fn stale() -> AppError {
    AppError::Conflict("cma_sandbox_generation_changed".into())
}
fn occupied() -> AppError {
    AppError::Conflict("cma_sandbox_account_slot_occupied".into())
}

/// Canonicalizes object order only. Prompt whitespace, array order, nulls and
/// optional fields remain significant; a retry never silently changes intent.
pub fn normalize_creation_request(request: &Value) -> AppResult<NormalizedCreationRequest> {
    if !request.is_object() {
        return Err(invalid_request());
    }
    fn canonical(value: &Value, depth: usize) -> AppResult<Value> {
        if depth > 32 {
            return Err(invalid_request());
        }
        match value {
            Value::Object(object) => {
                if object.len() > 256 {
                    return Err(invalid_request());
                }
                let mut sorted = BTreeMap::new();
                for (key, value) in object {
                    let normalized = key.to_ascii_lowercase().replace('_', "-");
                    if key.len() > 256
                        || matches!(
                            normalized.as_str(),
                            "authorization"
                                | "proxy-authorization"
                                | "cookie"
                                | "set-cookie"
                                | "access-token"
                                | "refresh-token"
                                | "bearer"
                                | "api-key"
                                | "x-api-key"
                        )
                        || normalized.starts_with("x-nyxid-")
                    {
                        return Err(invalid_request());
                    }
                    sorted.insert(key.clone(), canonical(value, depth + 1)?);
                }
                Ok(Value::Object(sorted.into_iter().collect()))
            }
            Value::Array(values) => {
                if values.len() > 1024 {
                    return Err(invalid_request());
                }
                Ok(Value::Array(
                    values
                        .iter()
                        .map(|value| canonical(value, depth + 1))
                        .collect::<AppResult<Vec<_>>>()?,
                ))
            }
            Value::String(value) if value.len() > MAX_RECOVERY_BYTES => Err(invalid_request()),
            _ => Ok(value.clone()),
        }
    }
    let bytes =
        Zeroizing::new(serde_json::to_vec(&canonical(request, 0)?).map_err(|_| invalid_request())?);
    if bytes.len() > MAX_RECOVERY_BYTES {
        return Err(invalid_request());
    }
    let mut digest = Sha256::new();
    digest.update(b"nyxid:cma-sandbox-creation:v1\0");
    digest.update(bytes.as_slice());
    Ok(NormalizedCreationRequest {
        hash: hex::encode(digest.finalize()),
        bytes,
    })
}

/// CMA binds idempotency to the exact HTTP body. Validation must never rewrite
/// the approved bytes, including object order, whitespace and JSON escapes.
pub fn prepare_creation_request(request: &[u8]) -> AppResult<NormalizedCreationRequest> {
    if request.len() > MAX_RECOVERY_BYTES {
        return Err(invalid_request());
    }
    let value: Value = serde_json::from_slice(request).map_err(|_| invalid_request())?;
    normalize_creation_request(&value)?;
    let mut digest = Sha256::new();
    digest.update(b"nyxid:cma-sandbox-creation:wire-v1\0");
    digest.update(request);
    Ok(NormalizedCreationRequest {
        hash: hex::encode(digest.finalize()),
        bytes: Zeroizing::new(request.to_vec()),
    })
}

pub fn indexes() -> Vec<IndexModel> {
    vec![
        IndexModel::builder()
            .keys(doc! {"user_id": 1})
            .options(
                IndexOptions::builder()
                    .name(OCCUPIED_SLOT_INDEX.to_owned())
                    .unique(true)
                    .partial_filter_expression(doc! {"occupies_account_slot": true})
                    .build(),
            )
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id": 1, "idempotency_key": 1})
            .options(
                IndexOptions::builder()
                    .name(CREATION_KEY_INDEX.to_owned())
                    .unique(true)
                    .partial_filter_expression(doc! {"idempotency_key": {"$type": "string"}})
                    .build(),
            )
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id": 1, "cma_service_id": 1, "cma_sandbox_id": 1})
            .options(
                IndexOptions::builder()
                    .name(REMOTE_REFERENCE_INDEX.to_owned())
                    .unique(true)
                    .partial_filter_expression(doc! {"cma_sandbox_id": {"$type": "string"}})
                    .build(),
            )
            .build(),
    ]
}

pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME)
        .create_indexes(indexes())
        .await?;
    Ok(())
}

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(error)) if error.code == 11000)
        || matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(error) if error.code == 11000)
}

fn duplicate_app_key(error: &AppError) -> bool {
    matches!(error,AppError::DatabaseError(error) if duplicate_key(error))
}

async fn creation_by_key(
    db: &Database,
    owner_id: &str,
    key: &str,
) -> AppResult<Option<NyxbotCmaSandbox>> {
    Ok(db
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one(doc! {"user_id": owner_id, "idempotency_key": key})
        .await?)
}

fn same_creation(
    row: NyxbotCmaSandbox,
    service_id: &str,
    hash: &str,
) -> AppResult<NyxbotCmaSandbox> {
    if row.cma_service_id != service_id || row.request_hash.as_deref() != Some(hash) {
        return Err(AppError::Conflict(
            "cma_creation_idempotency_conflict".into(),
        ));
    }
    Ok(row)
}

/// Reads every page, including excess legacy resources. Failure, repeated
/// cursors and bounded-inventory exhaustion all close admission.
pub async fn live_inventory(
    gateway: &dyn CmaGateway,
    owner_id: &str,
) -> AppResult<Vec<CmaSandbox>> {
    let mut cursor: Option<String> = None;
    let mut cursors = HashSet::new();
    let mut ids = HashSet::new();
    let mut inventory = Vec::new();
    for _ in 0..MAX_INVENTORY_PAGES {
        let page = gateway.list_sandboxes(owner_id, cursor.as_deref()).await?;
        if page.items.len() > super::nyxbot_cma_gateway::MAX_PAGE_SIZE {
            return Err(CmaGatewayError::InventoryIncomplete.into());
        }
        for sandbox in page.items {
            verify_remote_owner(&sandbox, owner_id)?;
            if !ids.insert(sandbox.id.clone()) {
                return Err(CmaGatewayError::InventoryIncomplete.into());
            }
            inventory.push(sandbox);
        }
        match page.next_cursor {
            None => return Ok(inventory),
            Some(next)
                if !next.is_empty()
                    && next.len() <= super::nyxbot_cma_gateway::MAX_CURSOR_BYTES
                    && cursors.insert(next.clone()) =>
            {
                cursor = Some(next)
            }
            _ => return Err(CmaGatewayError::InventoryIncomplete.into()),
        }
    }
    Err(CmaGatewayError::InventoryIncomplete.into())
}

/// Reserve before dispatch. This method never sends a remote create request.
/// The same owner/key/hash returns its original identity. Only an explicit
/// confirmed-not-created outcome can reacquire capacity, after another full
/// inventory read. A retired resource can never allocate a replacement.
pub async fn begin_creation(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    idempotency_key: &str,
    request: &[u8],
) -> AppResult<NyxbotCmaSandbox> {
    validate_idempotency_key(idempotency_key)?;
    let normalized = prepare_creation_request(request)?;
    let parsed: Value = serde_json::from_slice(request).map_err(|_| invalid_request())?;
    let service_id = gateway.service_id(owner_id).await?;
    let db = super::transaction_service::durable_database(db);
    ensure_indexes(&db).await?;
    let retry = match creation_by_key(&db, owner_id, idempotency_key).await? {
        Some(row) => {
            let row = same_creation(row, &service_id, &normalized.hash)?;
            if row.submission_state == "confirmed_not_created"
                && !row.occupies_account_slot
                && row.cma_sandbox_id.is_none()
            {
                Some(row)
            } else {
                return Ok(row);
            }
        }
        None => None,
    };
    if let Some(row) = db
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one(doc! {"user_id": owner_id, "occupies_account_slot": true})
        .await?
    {
        if row.idempotency_key.as_deref() == Some(idempotency_key) {
            return same_creation(row, &service_id, &normalized.hash);
        }
        return Err(occupied());
    }
    if db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"user_id":owner_id,"source":"conflict","admission_state":{"$ne":"retired"}})
        .await?
        .is_some()
    {
        return Err(occupied());
    }
    if live_inventory(gateway, owner_id)
        .await?
        .iter()
        .any(|sandbox| sandbox.state != SandboxObservationState::Retired)
    {
        return Err(AppError::Conflict(
            "cma_existing_sandbox_requires_claim".into(),
        ));
    }
    let previous = db
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one(doc! {"user_id": owner_id})
        .sort(doc! {"generation": -1})
        .await?;
    let generation = previous
        .map_or(Some(1), |row| row.generation.checked_add(1))
        .ok_or_else(stale)?;
    let id = retry
        .as_ref()
        .map(|row| row.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let field = ["request_ciphertext"];
    let context = RecordContext::new(COLLECTION_NAME, &id, owner_id, &field)?;
    let ciphertext = encryption_keys
        .encrypt_record(normalized.bytes.as_slice(), &context)
        .await?;
    let now = Utc::now();
    let row = NyxbotCmaSandbox {
        id,
        user_id: owner_id.to_owned(),
        cma_service_id: service_id.clone(),
        label: retry
            .as_ref()
            .map(|row| row.label.clone())
            .unwrap_or_else(|| "Sandbox".into()),
        cma_sandbox_id: None,
        occupies_account_slot: true,
        admission_state: SandboxAdmissionState::Reserved,
        observation_state: SandboxObservationState::Unknown,
        creation_ref: retry
            .as_ref()
            .map(|row| row.creation_ref.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        generation,
        agent_reference_version: 0,
        source: "nyxbot".into(),
        last_observed: None,
        machine_included: parsed.get("machine_included").and_then(Value::as_bool),
        resource_state: None,
        cpu_millis: None,
        memory_mib: None,
        last_activity_at: None,
        retire_requested_at: None,
        retired_at: None,
        idempotency_key: Some(idempotency_key.to_owned()),
        request_hash: Some(normalized.hash.clone()),
        operation_id: None,
        submission_state: "pending".into(),
        request_ciphertext: Some(ciphertext),
        request_context_version: Some(3),
        created_at: retry.as_ref().map_or(now, |row| row.created_at),
        updated_at: now,
    };
    if let Some(previous) = retry {
        let mut filter = observation_filter(&previous);
        filter.insert("submission_state", "confirmed_not_created");
        filter.insert("request_hash", normalized.hash.clone());
        let mut replacement = bson::to_document(&row).map_err(|_| invalid_request())?;
        replacement.remove("_id");
        let context = (db.clone(), owner_id.to_owned(), filter, replacement);
        match super::transaction_service::run_db_only(
            &db,
            &context,
            |tx, (db, owner, filter, replacement)| {
                Box::pin(async move {
                    super::erasure_admission::fence_in_session(db, owner, tx).await?;
                    Ok(db
                        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
                        .find_one_and_update(filter.clone(), doc! {"$set":replacement})
                        .return_document(ReturnDocument::After)
                        .session(tx)
                        .await?)
                })
            },
        )
        .await
        {
            Ok(Some(row)) => return Ok(row),
            Ok(None) => {
                return same_creation(
                    creation_by_key(&db, owner_id, idempotency_key)
                        .await?
                        .ok_or_else(stale)?,
                    &service_id,
                    &normalized.hash,
                );
            }
            Err(error) if duplicate_app_key(&error) => return Err(occupied()),
            Err(error) => return Err(error),
        }
    }
    match super::erasure_admission::insert_owned(&db, COLLECTION_NAME, owner_id, &row).await {
        Ok(_) => Ok(row),
        Err(error) if duplicate_app_key(&error) => {
            if let Some(row) = creation_by_key(&db, owner_id, idempotency_key).await? {
                same_creation(row, &service_id, &normalized.hash)
            } else {
                Err(occupied())
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
async fn recover_creation_request(
    db: &Database,
    keys: &EncryptionKeys,
    owner_id: &str,
    id: &str,
    generation: i64,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let row = get_owned(db, owner_id, id).await?;
    require_generation(&row, generation)?;
    if row.request_context_version != Some(3) {
        return Err(AppError::Conflict(
            "cma_creation_recovery_unavailable".into(),
        ));
    }
    let field = ["request_ciphertext"];
    let context = RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &field)?;
    let bytes = keys
        .decrypt_record(
            row.request_ciphertext
                .as_deref()
                .ok_or_else(|| AppError::Conflict("cma_creation_recovery_unavailable".into()))?,
            &context,
        )
        .await?;
    if bytes.len() > MAX_RECOVERY_BYTES {
        return Err(invalid_request());
    }
    if prepare_creation_request(&bytes)?.hash != row.request_hash.as_deref().unwrap_or("") {
        return Err(AppError::Conflict(
            "cma_creation_recovery_unavailable".into(),
        ));
    }
    Ok(bytes)
}

pub async fn get_owned(db: &Database, owner_id: &str, id: &str) -> AppResult<NyxbotCmaSandbox> {
    db.collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one(doc! {"_id": id, "user_id": owner_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Sandbox not found".into()))
}

pub fn require_generation(row: &NyxbotCmaSandbox, expected: i64) -> AppResult<()> {
    if expected <= 0 || row.generation != expected {
        return Err(stale());
    }
    Ok(())
}

fn verify_remote_owner(sandbox: &CmaSandbox, owner_id: &str) -> AppResult<()> {
    validate_remote_id(&sandbox.id)?;
    // The gateway's signed owner context is the authority when CMA omits an
    // owner field. An explicit mismatched owner is always a refusal.
    if sandbox
        .owner_id
        .as_deref()
        .is_some_and(|owner| owner != owner_id)
    {
        return Err(CmaGatewayError::Forbidden.into());
    }
    Ok(())
}

async fn verify_deployment(gateway: &dyn CmaGateway, row: &NyxbotCmaSandbox) -> AppResult<()> {
    if gateway.service_id(&row.user_id).await? != row.cma_service_id {
        return Err(AppError::Conflict("cma_deployment_changed".into()));
    }
    Ok(())
}

fn apply_observation(
    row: &mut NyxbotCmaSandbox,
    observation: &CmaSandbox,
    expected_generation: i64,
    at: DateTime<Utc>,
) -> AppResult<()> {
    require_generation(row, expected_generation)?;
    verify_remote_owner(observation, &row.user_id)?;
    if row
        .cma_sandbox_id
        .as_deref()
        .is_some_and(|id| id != observation.id)
    {
        return Err(stale());
    }
    if !row.occupies_account_slot
        && row.source != "conflict"
        && observation.state != SandboxObservationState::Retired
    {
        return Err(stale());
    }
    row.cma_sandbox_id = Some(observation.id.clone());
    row.observation_state = observation.state;
    row.resource_state = observation
        .resource_state
        .clone()
        .or_else(|| Some(observation.state.as_str().into()));
    row.last_observed = Some(at);
    row.machine_included = observation.machine_included.or(row.machine_included);
    row.cpu_millis = observation.cpu_millis;
    row.memory_mib = observation.memory_mib;
    row.last_activity_at = observation.last_activity_at;
    row.updated_at = at;
    if observation.state == SandboxObservationState::Retired {
        row.occupies_account_slot = false;
        row.admission_state = SandboxAdmissionState::Retired;
        row.retired_at.get_or_insert(at);
        row.submission_state = "confirmed_retired".into();
        row.request_ciphertext = None;
        row.request_context_version = None;
    } else if row.retire_requested_at.is_some()
        || observation.state == SandboxObservationState::Retiring
    {
        row.admission_state = SandboxAdmissionState::Retiring;
    } else {
        row.admission_state = if observation.state == SandboxObservationState::Unknown {
            SandboxAdmissionState::Unknown
        } else {
            SandboxAdmissionState::Associated
        };
    }
    Ok(())
}

pub(super) fn observation_filter(row: &NyxbotCmaSandbox) -> Document {
    let mut filter = doc! {"_id": &row.id, "user_id": &row.user_id, "generation": row.generation, "cma_sandbox_id": &row.cma_sandbox_id, "occupies_account_slot": row.occupies_account_slot, "admission_state": row.admission_state.as_str(), "retire_requested_at": row.retire_requested_at.map(bson::DateTime::from_chrono), "last_observed": row.last_observed.map(bson::DateTime::from_chrono)};
    filter.insert(
        "agent_reference_version",
        if row.agent_reference_version == 0 {
            bson::Bson::Document(doc! {"$in":[0,bson::Bson::Null]})
        } else {
            row.agent_reference_version.into()
        },
    );
    filter
}

async fn persist_observation(
    db: &Database,
    before: &NyxbotCmaSandbox,
    after: &NyxbotCmaSandbox,
) -> AppResult<NyxbotCmaSandbox> {
    db.collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one_and_update(
            observation_filter(before),
            doc! {"$set": observation_set(after)?},
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(stale)
}

pub(super) fn observation_set(after: &NyxbotCmaSandbox) -> AppResult<Document> {
    let document = bson::to_document(after)
        .map_err(|_| AppError::Internal("Invalid Sandbox observation".into()))?;
    let fields = [
        "cma_sandbox_id",
        "occupies_account_slot",
        "admission_state",
        "observation_state",
        "last_observed",
        "machine_included",
        "resource_state",
        "cpu_millis",
        "memory_mib",
        "last_activity_at",
        "retired_at",
        "submission_state",
        "request_ciphertext",
        "request_context_version",
        "updated_at",
    ];
    Ok(fields
        .into_iter()
        .map(|key| {
            (
                key.to_owned(),
                document.get(key).cloned().unwrap_or(Bson::Null),
            )
        })
        .collect())
}

/// Only a fresh exact remote read may release a known resource. A lifecycle
/// command's 2xx/202 acknowledgement is intentionally insufficient.
pub async fn observe(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    id: &str,
    generation: i64,
) -> AppResult<NyxbotCmaSandbox> {
    let row = get_owned(db, owner_id, id).await?;
    require_generation(&row, generation)?;
    verify_deployment(gateway, &row).await?;
    let remote_id = row
        .cma_sandbox_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("cma_creation_result_unknown".into()))?;
    let observation = gateway.get_sandbox(owner_id, remote_id).await?;
    if observation.id != remote_id {
        return Err(stale());
    }
    let mut changed = row.clone();
    apply_observation(&mut changed, &observation, generation, Utc::now())?;
    persist_observation(
        &super::transaction_service::durable_database(db),
        &row,
        &changed,
    )
    .await
}

/// Attach an exact creation result without discarding recovery identity. The
/// caller must obtain the resource through the owner-authenticated gateway.
pub async fn associate_created(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    id: &str,
    generation: i64,
    remote_id: &str,
) -> AppResult<NyxbotCmaSandbox> {
    let row = get_owned(db, owner_id, id).await?;
    require_generation(&row, generation)?;
    verify_deployment(gateway, &row).await?;
    let observation = gateway.get_sandbox(owner_id, remote_id).await?;
    if observation.id != remote_id {
        return Err(stale());
    }
    // Discovery may have registered this resource before its create receipt
    // arrived. Preserve the original permanent key/row identity and atomically
    // fold that metadata-only reference into it, including a pending retirement.
    let context = (db.clone(), row, observation);
    super::transaction_service::run_db_only(db, &context, |tx, (db, before, observation)| {
        Box::pin(async move {
            super::erasure_admission::fence_in_session(db,&before.user_id,tx).await?;
            let collection = db.collection::<NyxbotCmaSandbox>(COLLECTION_NAME);
            let current = collection.find_one(observation_filter(before)).session(&mut *tx).await?.ok_or_else(stale)?;
            let duplicate = collection.find_one(doc! {
                "_id":{"$ne":&current.id},"user_id":&current.user_id,
                "cma_service_id":&current.cma_service_id,"cma_sandbox_id":&observation.id,
            }).session(&mut *tx).await?;
            let mut changed = current.clone();
            if let Some(duplicate) = duplicate {
                if duplicate.occupies_account_slot
                    || duplicate.idempotency_key.is_some()
                    || duplicate.request_hash.is_some()
                    || duplicate.request_ciphertext.is_some()
                    || !matches!(duplicate.source.as_str(), "discovered" | "conflict" | "claimed")
                {
                    return Err(stale());
                }
                changed.retire_requested_at = changed.retire_requested_at.or(duplicate.retire_requested_at);
                if changed.label == "Sandbox" { changed.label = duplicate.label.clone(); }
                if collection.delete_one(observation_filter(&duplicate)).session(&mut *tx).await?.deleted_count != 1 {
                    return Err(stale());
                }
                // Agent refs use the remote parent identity, so only its local
                // generation changes when the provisional reference is folded.
                db.collection::<Document>(crate::models::nyxbot_cma_agent::COLLECTION_NAME)
                    .update_many(doc! {"user_id":&current.user_id,"cma_service_id":&current.cma_service_id,"cma_sandbox_id":&observation.id,"sandbox_generation":duplicate.generation}, doc! {"$set":{"sandbox_generation":current.generation}})
                    .session(&mut *tx).await?;
            }
            apply_observation(&mut changed, observation, current.generation, Utc::now())?;
            let mut set = observation_set(&changed)?;
            set.insert("retire_requested_at", changed.retire_requested_at.map(bson::DateTime::from_chrono));
            set.insert("label", &changed.label);
            collection.find_one_and_update(observation_filter(&current), doc! {"$set":set})
                .return_document(ReturnDocument::After).session(tx).await?.ok_or_else(stale)
        })
    }).await
}

/// Persist submission before network I/O and retain the same identity on a
/// timeout. Unknown outcomes are never retried with a newly generated key.
#[cfg(test)]
async fn mark_submission(
    db: &Database,
    owner_id: &str,
    id: &str,
    generation: i64,
    operation_id: Option<&str>,
    unknown: bool,
) -> AppResult<NyxbotCmaSandbox> {
    if let Some(operation_id) = operation_id {
        validate_remote_id(operation_id)?;
    }
    let row = get_owned(db, owner_id, id).await?;
    require_generation(&row, generation)?;
    if !row.occupies_account_slot || row.retire_requested_at.is_some() {
        return Err(stale());
    }
    let mut set = doc! {"admission_state": if unknown {"unknown"} else {"creating"}, "submission_state": if unknown {"unknown"} else {"submitted"}, "updated_at": bson::DateTime::now()};
    if let Some(operation_id) = operation_id {
        set.insert("operation_id", operation_id);
    }
    super::transaction_service::durable_database(db)
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one_and_update(observation_filter(&row), doc! {"$set": set})
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(stale)
}

pub(super) fn apply_no_resource_confirmation(
    row: &mut NyxbotCmaSandbox,
    generation: i64,
    key: &str,
    request_hash: &str,
    at: DateTime<Utc>,
) -> AppResult<()> {
    require_generation(row, generation)?;
    if row.cma_sandbox_id.is_some()
        || row.idempotency_key.as_deref() != Some(key)
        || row.request_hash.as_deref() != Some(request_hash)
    {
        return Err(stale());
    }
    row.occupies_account_slot = false;
    row.admission_state = SandboxAdmissionState::Retired;
    row.submission_state = "confirmed_not_created".into();
    row.request_ciphertext = None;
    row.request_context_version = None;
    row.updated_at = at;
    Ok(())
}

/// Call only for a terminal, authoritative result of the exact original
/// create request that guarantees no resource and no pending creation. A
/// timeout, 404 resource read or empty resource listing is not such evidence.
#[cfg(test)]
async fn confirm_no_resource(
    db: &Database,
    owner_id: &str,
    id: &str,
    generation: i64,
    key: &str,
    request_hash: &str,
) -> AppResult<NyxbotCmaSandbox> {
    let row = get_owned(db, owner_id, id).await?;
    let mut changed = row.clone();
    apply_no_resource_confirmation(&mut changed, generation, key, request_hash, Utc::now())?;
    persist_observation(
        &super::transaction_service::durable_database(db),
        &row,
        &changed,
    )
    .await
}

#[derive(Clone, Debug, Serialize)]
pub struct SandboxBillingDto {
    pub provider: &'static str,
    pub cost_status: &'static str,
    pub cost: Option<String>,
    pub currency: Option<String>,
    pub note: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct SandboxCapacityDto {
    pub cpu_millis: Option<u32>,
    pub memory_mib: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SandboxDto {
    pub id: String,
    pub label: String,
    pub cma_sandbox_id: Option<String>,
    pub generation: i64,
    pub occupies_account_slot: bool,
    pub admission_state: SandboxAdmissionState,
    pub status: SandboxObservationState,
    pub observation_state: SandboxObservationState,
    pub resource_state: String,
    pub machine_included: Option<bool>,
    pub cpu_millis: Option<u32>,
    pub memory_mib: Option<u32>,
    pub capacity: SandboxCapacityDto,
    pub last_observed: Option<DateTime<Utc>>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub retire_requested_at: Option<DateTime<Utc>>,
    pub retired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub billing: SandboxBillingDto,
}

impl From<&NyxbotCmaSandbox> for SandboxDto {
    fn from(row: &NyxbotCmaSandbox) -> Self {
        Self {
            id: row.id.clone(),
            label: row.label.clone(),
            cma_sandbox_id: row.cma_sandbox_id.clone(),
            generation: row.generation,
            occupies_account_slot: row.occupies_account_slot,
            admission_state: row.admission_state,
            status: row.observation_state,
            observation_state: row.observation_state,
            resource_state: row
                .resource_state
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            machine_included: row.machine_included,
            cpu_millis: row.cpu_millis,
            memory_mib: row.memory_mib,
            capacity: SandboxCapacityDto {
                cpu_millis: row.cpu_millis,
                memory_mib: row.memory_mib,
            },
            last_observed: row.last_observed,
            last_activity_at: row.last_activity_at,
            retire_requested_at: row.retire_requested_at,
            retired_at: row.retired_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            billing: SandboxBillingDto {
                provider: "cma",
                cost_status: "unknown",
                cost: None,
                currency: None,
                note: "Resource charges are managed by CMA. Pausing an environment or deleting an Agent does not confirm that all charges have stopped.",
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SandboxListDto {
    pub sandboxes: Vec<SandboxDto>,
    pub next_cursor: Option<String>,
    pub occupied: usize,
    pub limit: usize,
    pub conflict: bool,
    /// A cached local listing cannot establish a complete remote inventory.
    pub inventory_complete: bool,
}

pub async fn list(
    db: &Database,
    owner_id: &str,
    cursor: Option<&str>,
) -> AppResult<SandboxListDto> {
    let mut filter = doc! {"user_id": owner_id};
    if let Some(cursor) = cursor {
        Uuid::parse_str(cursor).map_err(|_| invalid_request())?;
        filter.insert("_id", doc! {"$gt": cursor});
    }
    let mut rows: Vec<NyxbotCmaSandbox> = db
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id": 1})
        .limit(MAX_LOCAL_PAGE_SIZE + 1)
        .await?
        .try_collect()
        .await?;
    let next_cursor = if rows.len() > MAX_LOCAL_PAGE_SIZE as usize {
        rows.truncate(MAX_LOCAL_PAGE_SIZE as usize);
        rows.last().map(|row| row.id.clone())
    } else {
        None
    };
    let occupied = db
        .collection::<Document>(COLLECTION_NAME)
        .count_documents(doc! {
            "user_id": owner_id,
            "$or": [
                {"occupies_account_slot": true},
                {"source":"conflict", "admission_state":{"$ne":"retired"}},
            ],
        })
        .await? as usize;
    Ok(SandboxListDto {
        sandboxes: rows.iter().map(SandboxDto::from).collect(),
        next_cursor,
        occupied,
        limit: 1,
        conflict: occupied > 1,
        inventory_complete: false,
    })
}

fn observation_unavailable(error: &AppError) -> bool {
    matches!(error, AppError::NotFound(_))
        || matches!(error, AppError::Conflict(message) if matches!(message.as_str(),
            "cma_observation_unavailable" | "cma_inventory_incomplete" | "cma_configuration_required"))
}

fn unknown_projection(mut sandbox: SandboxDto) -> SandboxDto {
    // Keep the last successful observation timestamp and all durable admission
    // facts; a failed read must not advertise the cached lifecycle as current.
    if sandbox.admission_state != SandboxAdmissionState::Retired {
        sandbox.status = SandboxObservationState::Unknown;
        sandbox.observation_state = SandboxObservationState::Unknown;
        sandbox.resource_state = "unknown".into();
    }
    sandbox
}

/// Reconcile a complete owner inventory without treating omitted resources as
/// retired. Verified excess resources get cleanup references; the unique
/// occupied-owner index still arbitrates the one admission slot.
pub async fn refresh_owner(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
) -> AppResult<(usize, HashSet<String>)> {
    let service_id = gateway.service_id(owner_id).await?;
    super::nyxbot_cma_admission::recover_owner(db, gateway, owner_id).await?;
    let inventory = live_inventory(gateway, owner_id).await?;
    let db = super::transaction_service::durable_database(db);
    ensure_indexes(&db).await?;
    let live_count = inventory
        .iter()
        .filter(|remote| remote.state != SandboxObservationState::Retired)
        .count();
    let mut observed = HashSet::new();
    // Read back explicit retired entries before promoting/claiming live ones.
    // A list omission never clears a slot or creation recovery ciphertext.
    for remote in inventory
        .iter()
        .filter(|remote| remote.state == SandboxObservationState::Retired)
    {
        if let Some(row) = db
            .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
            .find_one(
                doc! {"user_id":owner_id,"cma_service_id":&service_id,"cma_sandbox_id":&remote.id},
            )
            .await?
        {
            match observe(&db, gateway, owner_id, &row.id, row.generation).await {
                Ok(_) => {
                    observed.insert(row.id);
                }
                Err(error) if observation_unavailable(&error) => {}
                Err(error) => return Err(error),
            }
        }
    }
    for remote in inventory
        .iter()
        .filter(|remote| remote.state != SandboxObservationState::Retired)
    {
        let row = register_verified(&db, owner_id, &service_id, remote, None, "discovered").await?;
        observed.insert(row.id);
    }
    Ok((live_count, observed))
}

pub async fn list_live(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    cursor: Option<&str>,
) -> AppResult<SandboxListDto> {
    let refresh = match refresh_owner(db, gateway, owner_id).await {
        Ok(result) => Some(result),
        Err(error) if observation_unavailable(&error) => None,
        Err(error) => return Err(error),
    };
    let mut result = list(db, owner_id, cursor).await?;
    result.inventory_complete = refresh.is_some();
    if let Some((live_count, _)) = &refresh {
        result.conflict |= *live_count > 1;
    }
    result.sandboxes = result
        .sandboxes
        .into_iter()
        .map(|sandbox| {
            if refresh
                .as_ref()
                .is_some_and(|(_, ids)| ids.contains(&sandbox.id))
            {
                sandbox
            } else {
                unknown_projection(sandbox)
            }
        })
        .collect();
    Ok(result)
}

pub async fn detail_live(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    id: &str,
) -> AppResult<SandboxDto> {
    // Known lifecycle receipts remain recoverable without their original chat
    // or watch; refresh only observes them and never resubmits creation.
    super::nyxbot_cma_admission::recover_owner(db, gateway, owner_id).await?;
    let row = get_owned(db, owner_id, id).await?;
    if row.cma_sandbox_id.is_none() {
        return Ok(unknown_projection(SandboxDto::from(&row)));
    }
    match observe(db, gateway, owner_id, id, row.generation).await {
        Ok(observed) => Ok(SandboxDto::from(&observed)),
        Err(error) if observation_unavailable(&error) => {
            Ok(unknown_projection(SandboxDto::from(&row)))
        }
        Err(error) => Err(error),
    }
}

pub async fn update_label(
    db: &Database,
    owner_id: &str,
    id: &str,
    generation: i64,
    label: &str,
) -> AppResult<SandboxDto> {
    let label = label.trim();
    if label.is_empty() || label.len() > 120 || label.chars().any(char::is_control) {
        return Err(AppError::ValidationError(
            "Sandbox label must contain 1–120 bytes without control characters".into(),
        ));
    }
    let row = get_owned(db, owner_id, id).await?;
    require_generation(&row, generation)?;
    let changed = db
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id": id, "user_id": owner_id, "generation": generation},
            doc! {"$set": {"label": label, "updated_at": bson::DateTime::now()}},
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(stale)?;
    Ok(SandboxDto::from(&changed))
}

pub async fn unregistered(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
) -> AppResult<Vec<CmaSandbox>> {
    let service_id = gateway.service_id(owner_id).await?;
    let registered: Vec<NyxbotCmaSandbox> = db
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find(doc! {"user_id": owner_id, "cma_service_id": service_id})
        .limit((MAX_INVENTORY_PAGES * super::nyxbot_cma_gateway::MAX_PAGE_SIZE + 1) as i64)
        .await?
        .try_collect()
        .await?;
    let ids: HashSet<&str> = registered
        .iter()
        .filter_map(|row| row.cma_sandbox_id.as_deref())
        .collect();
    Ok(live_inventory(gateway, owner_id)
        .await?
        .into_iter()
        .filter(|remote| {
            remote.state != SandboxObservationState::Retired && !ids.contains(remote.id.as_str())
        })
        .collect())
}

pub async fn claim(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    remote_id: &str,
    label: Option<&str>,
) -> AppResult<SandboxDto> {
    validate_remote_id(remote_id)?;
    let service_id = gateway.service_id(owner_id).await?;
    let db = super::transaction_service::durable_database(db);
    ensure_indexes(&db).await?;
    let remote = gateway.get_sandbox(owner_id, remote_id).await?;
    verify_remote_owner(&remote, owner_id)?;
    if remote.id != remote_id || remote.state == SandboxObservationState::Retired {
        return Err(AppError::Conflict("cma_sandbox_not_claimable".into()));
    }
    let row = register_verified(&db, owner_id, &service_id, &remote, label, "claimed").await?;
    Ok(SandboxDto::from(&row))
}

async fn register_verified(
    db: &Database,
    owner_id: &str,
    service_id: &str,
    remote: &CmaSandbox,
    label: Option<&str>,
    source: &str,
) -> AppResult<NyxbotCmaSandbox> {
    verify_remote_owner(remote, owner_id)?;
    let remote_id = &remote.id;
    let label = label.unwrap_or("Sandbox").trim();
    if label.is_empty() || label.len() > 120 || label.chars().any(char::is_control) {
        return Err(invalid_request());
    }
    if let Some(row) = db
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one(
            doc! {"user_id": owner_id, "cma_service_id": service_id, "cma_sandbox_id": remote_id},
        )
        .await?
    {
        let mut changed = row.clone();
        apply_observation(&mut changed, remote, row.generation, Utc::now())?;
        let row = persist_observation(db, &row, &changed).await?;
        if !row.occupies_account_slot
            && row.source == "conflict"
            && row.admission_state != SandboxAdmissionState::Retired
        {
            // Reclaim the single slot if it is now vacant; duplicate-key is
            // ordinary contention, not grounds to hide this verified ref.
            match db
                .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
                .find_one_and_update(
                    observation_filter(&row),
                    doc! {"$set":{"occupies_account_slot":true,"source":source}},
                )
                .return_document(ReturnDocument::After)
                .await
            {
                Ok(Some(promoted)) => return Ok(promoted),
                Ok(None) => return Err(stale()),
                Err(error) if duplicate_key(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }
        return Ok(row);
    }
    let previous = db
        .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
        .find_one(doc! {"user_id": owner_id})
        .sort(doc! {"generation": -1})
        .await?;
    let generation = previous
        .map_or(Some(1), |row| row.generation.checked_add(1))
        .ok_or_else(stale)?;
    let now = Utc::now();
    let mut row = NyxbotCmaSandbox {
        id: Uuid::new_v4().to_string(),
        user_id: owner_id.to_owned(),
        cma_service_id: service_id.to_owned(),
        label: label.to_owned(),
        cma_sandbox_id: Some(remote_id.to_owned()),
        occupies_account_slot: true,
        admission_state: SandboxAdmissionState::Associated,
        observation_state: SandboxObservationState::Unknown,
        creation_ref: Uuid::new_v4().to_string(),
        generation,
        agent_reference_version: 0,
        source: source.to_owned(),
        last_observed: None,
        machine_included: None,
        resource_state: None,
        cpu_millis: None,
        memory_mib: None,
        last_activity_at: None,
        retire_requested_at: None,
        retired_at: None,
        idempotency_key: None,
        request_hash: None,
        operation_id: None,
        submission_state: "not_applicable".into(),
        request_ciphertext: None,
        request_context_version: None,
        created_at: now,
        updated_at: now,
    };
    apply_observation(&mut row, remote, generation, now)?;
    match super::erasure_admission::insert_owned(db, COLLECTION_NAME, owner_id, &row).await {
        Ok(_) => Ok(row),
        Err(error) if duplicate_app_key(&error) => {
            let exact =
                doc! {"user_id":owner_id,"cma_service_id":service_id,"cma_sandbox_id":remote_id};
            if let Some(existing) = db
                .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
                .find_one(exact.clone())
                .await?
            {
                return Ok(existing);
            }
            row.occupies_account_slot = false;
            row.source = "conflict".into();
            match super::erasure_admission::insert_owned(db, COLLECTION_NAME, owner_id, &row).await
            {
                Ok(_) => Ok(row),
                Err(error) if duplicate_app_key(&error) => db
                    .collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
                    .find_one(exact)
                    .await?
                    .ok_or_else(stale),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SandboxCommandDto {
    pub sandbox: SandboxDto,
    pub pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxCommand {
    Pause,
    Revive,
    Retire,
}

pub async fn command(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    id: &str,
    generation: i64,
    command: SandboxCommand,
    idempotency_key: &str,
) -> AppResult<SandboxCommandDto> {
    validate_idempotency_key(idempotency_key)?;
    let db = super::transaction_service::durable_database(db);
    let mut row = get_owned(&db, owner_id, id).await?;
    require_generation(&row, generation)?;
    verify_deployment(gateway, &row).await?;
    if row.admission_state == SandboxAdmissionState::Retired && command == SandboxCommand::Retire {
        return Ok(SandboxCommandDto {
            sandbox: SandboxDto::from(&row),
            pending: false,
        });
    }
    if !row.occupies_account_slot && row.source != "conflict" {
        return Err(AppError::Conflict("cma_sandbox_retired".into()));
    }
    if command != SandboxCommand::Retire && row.retire_requested_at.is_some() {
        return Err(AppError::Conflict("cma_sandbox_retiring".into()));
    }
    let remote_id = row
        .cma_sandbox_id
        .clone()
        .ok_or_else(|| AppError::Conflict("cma_creation_result_unknown".into()))?;
    if command == SandboxCommand::Retire && row.retire_requested_at.is_none() {
        row = db.collection::<NyxbotCmaSandbox>(COLLECTION_NAME).find_one_and_update(observation_filter(&row), doc! {"$set": {"admission_state": "retiring", "retire_requested_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()}}).return_document(ReturnDocument::After).await?.ok_or_else(stale)?;
    }
    // Retain the caller's stable key across retries of this reviewed intent.
    let key = idempotency_key;
    let result = match command {
        SandboxCommand::Pause => gateway.pause_sandbox(owner_id, &remote_id, &key).await,
        SandboxCommand::Revive => gateway.revive_sandbox(owner_id, &remote_id, &key).await,
        SandboxCommand::Retire => gateway.retire_sandbox(owner_id, &remote_id, &key).await,
    };
    let acknowledgement = result?;
    // A command response is not retirement evidence. Read back the exact
    // resource; an unavailable read truthfully returns a pending local row.
    match observe(&db, gateway, owner_id, id, generation).await {
        Ok(observed) => {
            let completed = match command {
                SandboxCommand::Pause => {
                    observed.observation_state == SandboxObservationState::Paused
                }
                SandboxCommand::Revive => {
                    observed.observation_state == SandboxObservationState::Ready
                }
                SandboxCommand::Retire => {
                    observed.observation_state == SandboxObservationState::Retired
                }
            };
            Ok(SandboxCommandDto {
                sandbox: SandboxDto::from(&observed),
                pending: !completed,
            })
        }
        Err(error) if observation_unavailable(&error) => Ok(SandboxCommandDto {
            sandbox: unknown_projection(SandboxDto::from(&row)),
            pending: acknowledgement.pending
                || row.admission_state != SandboxAdmissionState::Retired,
        }),
        Err(error) => Err(error),
    }
}

pub async fn retire(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    id: &str,
    generation: i64,
    idempotency_key: &str,
) -> AppResult<SandboxCommandDto> {
    command(
        db,
        gateway,
        owner_id,
        id,
        generation,
        SandboxCommand::Retire,
        idempotency_key,
    )
    .await
}

pub async fn events(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    id: &str,
    cursor: Option<&str>,
) -> AppResult<CmaEventSnapshot> {
    let row = get_owned(db, owner_id, id).await?;
    verify_deployment(gateway, &row).await?;
    let remote_id = row
        .cma_sandbox_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("cma_creation_result_unknown".into()))?;
    Ok(gateway
        .list_sandbox_events(owner_id, remote_id, cursor)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::super::nyxbot_cma_gateway::*;
    use super::*;
    use serde_json::json;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct FakeGateway {
        inventory: Vec<CmaPage<CmaSandbox>>,
        current: Mutex<CmaSandbox>,
        timeout: AtomicBool,
        calls: Mutex<Vec<String>>,
    }

    impl FakeGateway {
        fn new() -> Self {
            Self {
                inventory: vec![CmaPage {
                    items: vec![],
                    next_cursor: None,
                }],
                current: Mutex::new(remote_sandbox(SandboxObservationState::Ready)),
                timeout: AtomicBool::new(false),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn lifecycle(&self, remote_id: &str, command: &str) -> CmaResult<CmaLifecycleResult> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{command}:{remote_id}"));
            if self.timeout.load(Ordering::Relaxed) {
                return Err(CmaGatewayError::Timeout);
            }
            // Even a terminal command response is only an acknowledgement.
            Ok(CmaLifecycleResult {
                pending: true,
                sandbox: Some(remote_sandbox(SandboxObservationState::Retired)),
            })
        }
    }

    #[async_trait::async_trait]
    impl CmaGateway for FakeGateway {
        async fn service_id(&self, _: &str) -> CmaResult<String> {
            Ok("cma-service".into())
        }
        async fn list_sandboxes(
            &self,
            _: &str,
            cursor: Option<&str>,
        ) -> CmaResult<CmaPage<CmaSandbox>> {
            if self.timeout.load(Ordering::Relaxed) {
                return Err(CmaGatewayError::Timeout);
            }
            let page = cursor
                .unwrap_or("0")
                .parse::<usize>()
                .map_err(|_| CmaGatewayError::InvalidRequest)?;
            self.inventory
                .get(page)
                .cloned()
                .ok_or(CmaGatewayError::InvalidResponse)
        }
        async fn get_sandbox(&self, _: &str, id: &str) -> CmaResult<CmaSandbox> {
            self.calls.lock().unwrap().push(format!("get:{id}"));
            if self.timeout.load(Ordering::Relaxed) {
                return Err(CmaGatewayError::Timeout);
            }
            Ok(self.current.lock().unwrap().clone())
        }
        async fn pause_sandbox(&self, _: &str, id: &str, _: &str) -> CmaResult<CmaLifecycleResult> {
            self.lifecycle(id, "pause")
        }
        async fn revive_sandbox(
            &self,
            _: &str,
            id: &str,
            _: &str,
        ) -> CmaResult<CmaLifecycleResult> {
            self.lifecycle(id, "revive")
        }
        async fn retire_sandbox(
            &self,
            _: &str,
            id: &str,
            _: &str,
        ) -> CmaResult<CmaLifecycleResult> {
            self.lifecycle(id, "retire")
        }
        async fn list_sandbox_events(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> CmaResult<CmaEventSnapshot> {
            Err(CmaGatewayError::Unavailable)
        }
        async fn get_agent(&self, _: &str, _: &str) -> CmaResult<CmaAgent> {
            Err(CmaGatewayError::Unavailable)
        }
        async fn get_transcript(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> CmaResult<CmaTranscript> {
            Err(CmaGatewayError::Unavailable)
        }
        async fn get_event_snapshot(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> CmaResult<CmaEventSnapshot> {
            Err(CmaGatewayError::Unavailable)
        }
        async fn get_response(&self, _: &str, _: &str, _: &str) -> CmaResult<CmaResponse> {
            Err(CmaGatewayError::Unavailable)
        }
        async fn desktop_descriptor(&self, _: &str, _: &str) -> CmaResult<CmaDesktopDescriptor> {
            Err(CmaGatewayError::Unavailable)
        }
    }

    // These tests are deterministic without MongoDB; the store tests below
    // probe a disposable database and run only when a local test DB is usable.
    #[tokio::test]
    async fn inventory_drains_every_page_and_refuses_repeated_cursor() {
        let mut gateway = FakeGateway::new();
        gateway.inventory = vec![
            CmaPage {
                items: vec![],
                next_cursor: Some("1".into()),
            },
            CmaPage {
                items: vec![remote_sandbox(SandboxObservationState::Paused)],
                next_cursor: None,
            },
        ];
        assert_eq!(live_inventory(&gateway, "owner").await.unwrap().len(), 1);
        gateway.inventory[1].next_cursor = Some("1".into());
        assert!(live_inventory(&gateway, "owner").await.is_err());
    }

    #[test]
    fn same_key_returns_original_identity_and_changed_payload_conflicts() {
        let row = fixture_row();
        let replay = same_creation(row.clone(), "cma-service", "request-hash").unwrap();
        assert_eq!(replay.id, row.id);
        assert!(same_creation(row.clone(), "cma-service", "different-hash").is_err());
        assert!(same_creation(row, "other-deployment", "request-hash").is_err());
    }

    #[tokio::test]
    async fn recovery_ciphertext_authenticates_exact_owner_record_and_field() {
        let keys = crate::test_utils::test_encryption_keys();
        let fields = ["request_ciphertext"];
        let context = RecordContext::new(COLLECTION_NAME, "row", "owner", &fields).unwrap();
        let normalized = normalize_creation_request(&json!({"name":"private task"})).unwrap();
        let encrypted = keys
            .encrypt_record(&normalized.bytes, &context)
            .await
            .unwrap();
        assert_ne!(encrypted.as_slice(), normalized.bytes.as_slice());
        assert_eq!(
            keys.decrypt_record(&encrypted, &context)
                .await
                .unwrap()
                .as_slice(),
            normalized.bytes.as_slice()
        );
        for (row, owner) in [("other", "owner"), ("row", "other")] {
            let context = RecordContext::new(COLLECTION_NAME, row, owner, &fields).unwrap();
            assert!(keys.decrypt_record(&encrypted, &context).await.is_err());
        }
    }

    async fn test_db() -> Option<Database> {
        let uri = std::env::var("NYXID_TEST_DATABASE_URL")
            .unwrap_or_else(|_| "mongodb://127.0.0.1:27017/?directConnection=true".into());
        let mut options = mongodb::options::ClientOptions::parse(&uri).await.ok()?;
        options.server_selection_timeout = Some(std::time::Duration::from_millis(500));
        let client = mongodb::Client::with_options(options).ok()?;
        if client
            .database("admin")
            .run_command(doc! {"ping": 1})
            .await
            .is_err()
        {
            eprintln!("skipping CMA MongoDB test: test database unavailable");
            return None;
        }
        let hello = client
            .database("admin")
            .run_command(doc! {"hello":1})
            .await
            .ok()?;
        if !hello.contains_key("setName") && hello.get_str("msg") != Ok("isdbgrid") {
            eprintln!(
                "skipping CMA MongoDB test: replica-set MongoDB required for owner-erasure transactions"
            );
            return None;
        }
        Some(client.database(&format!("nyxid_test_cma_{}", Uuid::new_v4().simple())))
    }

    #[tokio::test]
    async fn concurrent_first_creations_occupy_one_slot_and_retries_keep_identity() {
        let Some(db) = test_db().await else {
            return;
        };
        let gateway = FakeGateway::new();
        let keys = crate::test_utils::test_encryption_keys();
        let body = br#"{ "name" : "task", "profile": {"model":"m"} }"#.to_vec();
        let (first, second) = tokio::join!(
            begin_creation(&db, &keys, &gateway, "owner", "key-a", &body),
            begin_creation(&db, &keys, &gateway, "owner", "key-b", &body)
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let row = first.or(second).unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {"user_id":"owner","occupies_account_slot":true})
                .await
                .unwrap(),
            1
        );
        let replay = begin_creation(
            &db,
            &keys,
            &gateway,
            "owner",
            row.idempotency_key.as_deref().unwrap(),
            &body,
        )
        .await
        .unwrap();
        assert_eq!(replay.id, row.id);
        assert!(
            begin_creation(
                &db,
                &keys,
                &gateway,
                "owner",
                row.idempotency_key.as_deref().unwrap(),
                br#"{"name":"changed"}"#
            )
            .await
            .is_err()
        );
        let recovered = recover_creation_request(&db, &keys, "owner", &row.id, row.generation)
            .await
            .unwrap();
        assert_eq!(recovered.as_slice(), body.as_slice());
        let unknown = mark_submission(&db, "owner", &row.id, row.generation, None, true)
            .await
            .unwrap();
        assert!(unknown.occupies_account_slot && unknown.request_ciphertext.is_some());
        let released = confirm_no_resource(
            &db,
            "owner",
            &row.id,
            row.generation,
            row.idempotency_key.as_deref().unwrap(),
            row.request_hash.as_deref().unwrap(),
        )
        .await
        .unwrap();
        assert!(!released.occupies_account_slot);
        let replay = begin_creation(
            &db,
            &keys,
            &gateway,
            "owner",
            row.idempotency_key.as_deref().unwrap(),
            &body,
        )
        .await
        .unwrap();
        assert_eq!(replay.id, row.id);
        assert!(replay.occupies_account_slot);
        assert!(replay.generation > row.generation);
        confirm_no_resource(
            &db,
            "owner",
            &replay.id,
            replay.generation,
            replay.idempotency_key.as_deref().unwrap(),
            replay.request_hash.as_deref().unwrap(),
        )
        .await
        .unwrap();
        let next = begin_creation(&db, &keys, &gateway, "owner", "next-key", &body)
            .await
            .unwrap();
        assert!(next.generation > row.generation);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn retirement_acknowledgement_and_timeout_keep_slot_until_exact_read_confirms() {
        let Some(db) = test_db().await else {
            return;
        };
        ensure_indexes(&db).await.unwrap();
        let gateway = FakeGateway::new();
        let row = fixture_row();
        db.collection::<NyxbotCmaSandbox>(COLLECTION_NAME)
            .insert_one(&row)
            .await
            .unwrap();
        let result = retire(&db, &gateway, "owner", &row.id, 1, "retire-key")
            .await
            .unwrap();
        assert!(result.pending && result.sandbox.occupies_account_slot);
        assert!(result.sandbox.retire_requested_at.is_some());
        gateway.timeout.store(true, Ordering::Relaxed);
        assert!(
            retire(&db, &gateway, "owner", &row.id, 1, "retire-key")
                .await
                .is_err()
        );
        assert!(
            get_owned(&db, "owner", &row.id)
                .await
                .unwrap()
                .occupies_account_slot
        );
        let calls = gateway.calls.lock().unwrap().len();
        assert!(
            command(
                &db,
                &gateway,
                "owner",
                &row.id,
                0,
                SandboxCommand::Pause,
                "pause-key"
            )
            .await
            .is_err()
        );
        assert!(
            command(
                &db,
                &gateway,
                "other-owner",
                &row.id,
                1,
                SandboxCommand::Pause,
                "pause-key"
            )
            .await
            .is_err()
        );
        assert_eq!(gateway.calls.lock().unwrap().len(), calls);
        gateway.timeout.store(false, Ordering::Relaxed);
        gateway.current.lock().unwrap().state = SandboxObservationState::Retired;
        let result = observe(&db, &gateway, "owner", &row.id, 1).await.unwrap();
        assert!(!result.occupies_account_slot);
        assert!(result.request_ciphertext.is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn creation_receipt_folds_an_early_discovery_into_the_original_identity() {
        let Some(db) = test_db().await else {
            return;
        };
        let hello = db.run_command(doc! {"hello":1}).await.unwrap();
        if !hello.contains_key("setName") && hello.get_str("msg") != Ok("isdbgrid") {
            eprintln!("skipping CMA atomic association test: replica-set MongoDB required");
            db.drop().await.unwrap();
            return;
        }
        let keys = crate::test_utils::test_encryption_keys();
        let mut gateway = FakeGateway::new();
        let row = begin_creation(
            &db,
            &keys,
            &gateway,
            "owner",
            "create-key",
            b"{ \"name\": \"task\" }",
        )
        .await
        .unwrap();
        gateway.inventory[0]
            .items
            .push(remote_sandbox(SandboxObservationState::Ready));
        let discovered = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert_eq!(discovered.sandboxes.len(), 2);
        assert!(discovered.conflict);
        let linked = associate_created(
            &db,
            &gateway,
            "owner",
            &row.id,
            row.generation,
            "remote-sandbox",
        )
        .await
        .unwrap();
        assert_eq!(linked.id, row.id);
        assert_eq!(linked.idempotency_key.as_deref(), Some("create-key"));
        assert!(linked.occupies_account_slot && linked.request_ciphertext.is_some());
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {"user_id":"owner"})
                .await
                .unwrap(),
            1
        );
        gateway.current.lock().unwrap().state = SandboxObservationState::Retired;
        let retired = observe(&db, &gateway, "owner", &row.id, row.generation)
            .await
            .unwrap();
        assert!(!retired.occupies_account_slot);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn existing_external_environment_blocks_creation_even_when_paused() {
        let Some(db) = test_db().await else {
            return;
        };
        let mut gateway = FakeGateway::new();
        gateway.inventory[0]
            .items
            .push(remote_sandbox(SandboxObservationState::Paused));
        let keys = crate::test_utils::test_encryption_keys();
        assert!(
            begin_creation(&db, &keys, &gateway, "owner", "key", br#"{"name":"task"}"#)
                .await
                .is_err()
        );
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let claimed = claim(&db, &gateway, "owner", "remote-sandbox", Some("My sandbox"))
            .await
            .unwrap();
        assert!(claimed.occupies_account_slot);
        assert!(
            claim(&db, &gateway, "other-owner", "remote-sandbox", None)
                .await
                .is_err()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn live_listing_registers_all_conflicts_and_never_releases_a_missing_resource() {
        let Some(db) = test_db().await else {
            return;
        };
        let mut gateway = FakeGateway::new();
        let first = remote_sandbox(SandboxObservationState::Ready);
        let mut second = remote_sandbox(SandboxObservationState::Paused);
        second.id = "excess-sandbox".into();
        gateway.inventory[0].items = vec![first, second.clone()];
        let listing = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert!(listing.conflict && listing.inventory_complete);
        assert_eq!(listing.occupied, 2);
        assert_eq!(listing.sandboxes.len(), 2);
        assert_eq!(
            listing
                .sandboxes
                .iter()
                .filter(|row| row.occupies_account_slot)
                .count(),
            1
        );
        let excess = listing
            .sandboxes
            .iter()
            .find(|row| row.cma_sandbox_id.as_deref() == Some("excess-sandbox"))
            .unwrap();
        assert!(!excess.occupies_account_slot);
        *gateway.current.lock().unwrap() = second;
        let result = retire(
            &db,
            &gateway,
            "owner",
            &excess.id,
            excess.generation,
            "retire-excess",
        )
        .await
        .unwrap();
        assert!(result.pending);
        gateway.inventory[0].items.clear();
        let missing = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert!(missing.conflict && missing.inventory_complete);
        assert_eq!(missing.occupied, 2);
        assert!(
            missing
                .sandboxes
                .iter()
                .all(|row| row.status == SandboxObservationState::Unknown)
        );
        gateway.timeout.store(true, Ordering::Relaxed);
        let unavailable = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert!(!unavailable.inventory_complete);
        assert!(unavailable.conflict);
        assert!(
            detail_live(&db, &gateway, "other", &excess.id)
                .await
                .is_err()
        );
        assert_eq!(
            detail_live(&db, &gateway, "owner", &excess.id)
                .await
                .unwrap()
                .status,
            SandboxObservationState::Unknown
        );
        let keys = crate::test_utils::test_encryption_keys();
        assert!(
            begin_creation(&db, &keys, &gateway, "owner", "another", b"{}")
                .await
                .is_err()
        );
        db.drop().await.unwrap();
    }

    #[test]
    fn unknown_projection_retains_durable_capacity_and_last_successful_observation() {
        let mut row = fixture_row();
        row.observation_state = SandboxObservationState::Ready;
        row.last_observed = Some(Utc::now());
        let projection = unknown_projection(SandboxDto::from(&row));
        assert_eq!(projection.status, SandboxObservationState::Unknown);
        assert!(projection.occupies_account_slot);
        assert_eq!(projection.last_observed, row.last_observed);
        row.admission_state = SandboxAdmissionState::Retired;
        row.observation_state = SandboxObservationState::Retired;
        row.occupies_account_slot = false;
        assert_eq!(
            unknown_projection(SandboxDto::from(&row)).status,
            SandboxObservationState::Retired
        );
    }

    #[test]
    fn canonical_request_hash_preserves_intent_and_ignores_object_key_order() {
        let left = normalize_creation_request(
            &json!({"profile":{"model":"x","prompt":"  keep  "},"name":"test"}),
        )
        .unwrap();
        let right = normalize_creation_request(
            &json!({"name":"test","profile":{"prompt":"  keep  ","model":"x"}}),
        )
        .unwrap();
        assert_eq!(left.hash, right.hash);
        assert_ne!(
            left.hash,
            normalize_creation_request(
                &json!({"name":"test","profile":{"prompt":"keep","model":"x"}})
            )
            .unwrap()
            .hash
        );
    }

    #[test]
    fn creation_recovery_keeps_the_exact_approved_http_body() {
        let raw = br#"{ "z" : "\u0061", "a": { "prompt" : "  keep  " } }"#;
        let request = prepare_creation_request(raw).unwrap();
        assert_eq!(request.bytes.as_slice(), raw);
        let equivalent = br#"{"a":{"prompt":"  keep  "},"z":"a"}"#;
        assert_ne!(
            request.hash,
            prepare_creation_request(equivalent).unwrap().hash
        );
        assert!(prepare_creation_request(br#"{"Authorization":"secret"}"#).is_err());
    }

    #[test]
    fn recovery_request_is_bounded_and_rejects_transport_credentials() {
        for value in [
            json!([]),
            json!({"Authorization":"Bearer secret"}),
            json!({"headers":{"X-NyxID-Identity-Token":"secret"}}),
            json!({"name":"x".repeat(MAX_RECOVERY_BYTES)}),
        ] {
            assert!(normalize_creation_request(&value).is_err());
        }
    }

    #[test]
    fn ambiguous_and_paused_observations_never_release_capacity() {
        for state in [
            SandboxObservationState::Unknown,
            SandboxObservationState::Paused,
            SandboxObservationState::Failed,
            SandboxObservationState::Retiring,
        ] {
            let mut row = fixture_row();
            let observation = remote_sandbox(state);
            apply_observation(&mut row, &observation, 1, Utc::now()).unwrap();
            assert!(row.occupies_account_slot);
            assert!(row.request_ciphertext.is_some());
        }
    }

    #[test]
    fn retirement_and_confirmed_no_resource_are_the_only_release_paths() {
        let mut row = fixture_row();
        apply_observation(
            &mut row,
            &remote_sandbox(SandboxObservationState::Retired),
            1,
            Utc::now(),
        )
        .unwrap();
        assert!(!row.occupies_account_slot);
        assert!(row.retired_at.is_some());
        assert!(row.request_ciphertext.is_none());
        let mut no_resource = fixture_row();
        no_resource.cma_sandbox_id = None;
        apply_no_resource_confirmation(
            &mut no_resource,
            1,
            "request-key",
            "request-hash",
            Utc::now(),
        )
        .unwrap();
        assert!(!no_resource.occupies_account_slot);
        assert!(no_resource.request_ciphertext.is_none());
    }

    #[test]
    fn stale_generation_and_substituted_remote_resource_are_rejected() {
        let mut row = fixture_row();
        assert!(
            apply_observation(
                &mut row,
                &remote_sandbox(SandboxObservationState::Retired),
                0,
                Utc::now()
            )
            .is_err()
        );
        let mut other = remote_sandbox(SandboxObservationState::Retired);
        other.id = "different-sandbox".into();
        assert!(apply_observation(&mut row, &other, 1, Utc::now()).is_err());
        assert!(row.occupies_account_slot);
    }

    #[test]
    fn safe_projection_never_exposes_recovery_material_or_agent_lists() {
        let value = serde_json::to_value(SandboxDto::from(&fixture_row())).unwrap();
        for field in [
            "request_ciphertext",
            "request_hash",
            "idempotency_key",
            "creation_ref",
            "agents",
        ] {
            assert!(value.get(field).is_none());
        }
        assert_eq!(value["billing"]["provider"], "cma");
        assert_eq!(value["billing"]["cost_status"], "unknown");
    }

    fn remote_sandbox(state: SandboxObservationState) -> CmaSandbox {
        CmaSandbox {
            id: "remote-sandbox".into(),
            owner_id: Some("owner".into()),
            state,
            resource_state: Some(state.as_str().into()),
            name: None,
            machine_included: None,
            cpu_millis: None,
            memory_mib: None,
            last_activity_at: None,
        }
    }

    fn fixture_row() -> NyxbotCmaSandbox {
        let now = Utc::now();
        NyxbotCmaSandbox {
            id: Uuid::new_v4().to_string(),
            user_id: "owner".into(),
            cma_service_id: "cma-service".into(),
            label: "Sandbox".into(),
            cma_sandbox_id: Some("remote-sandbox".into()),
            occupies_account_slot: true,
            admission_state: SandboxAdmissionState::Creating,
            observation_state: SandboxObservationState::Unknown,
            creation_ref: Uuid::new_v4().to_string(),
            generation: 1,
            agent_reference_version: 0,
            source: "nyxbot".into(),
            last_observed: None,
            machine_included: None,
            resource_state: None,
            cpu_millis: None,
            memory_mib: None,
            last_activity_at: None,
            retire_requested_at: None,
            retired_at: None,
            idempotency_key: Some("request-key".into()),
            request_hash: Some("request-hash".into()),
            operation_id: None,
            submission_state: "submitted".into(),
            request_ciphertext: Some(vec![1, 2, 3]),
            request_context_version: Some(3),
            created_at: now,
            updated_at: now,
        }
    }
}
