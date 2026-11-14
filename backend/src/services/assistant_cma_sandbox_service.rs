//! Durable admission for one NyxID-created Sandbox. A timeout never proves absence.
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

use super::assistant_cma_gateway::{
    CmaEventSnapshot, CmaGateway, CmaGatewayError, CmaSandbox, validate_idempotency_key,
    validate_remote_id,
};
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::assistant_cma_sandbox::{
        AssistantCmaSandbox, COLLECTION_NAME, SandboxAdmissionState, SandboxCapacitySource,
        SandboxObservationState,
    },
};

pub const MAX_RECOVERY_BYTES: usize = 64 * 1024;
pub const MAX_LOCAL_PAGE_SIZE: i64 = 100;
/// Written only by the local creation admission path, never by discovery.
pub const NYXID_CREATED_SOURCE: &str = "nyxbot";

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

pub(super) fn created_filter(owner_id: &str) -> Document {
    doc! {"user_id": owner_id, "source": NYXID_CREATED_SOURCE, "user_service_id":{"$type":"string"}, "execution_base_url":{"$type":"string"}}
}

/// Previous versions let discovery occupy the account slot. Release only
/// metadata-only external references; never alter a durable creation intent
/// or infer remote retirement. Keep the rows for receipt reconciliation.
async fn release_external_slots(db: &Database, owner_id: &str) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(
            doc! {
                "user_id": owner_id,
                "user_service_id": {"$type": "string"},
                "execution_base_url": {"$type": "string"},
                "source": {"$in": ["discovered", "conflict", "claimed"]},
                "occupies_account_slot": true,
                "idempotency_key": null, "request_hash": null,
                "request_ciphertext": null, "operation_id": null,
                "request_context_version": null,
            },
            doc! {"$set": {"occupies_account_slot": false, "updated_at": bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

pub(super) async fn active_created(
    db: &Database,
    owner_id: &str,
) -> AppResult<Vec<AssistantCmaSandbox>> {
    let mut filter = created_filter(owner_id);
    filter.insert("admission_state", doc! {"$ne": "retired"});
    let rows: Vec<AssistantCmaSandbox> = db
        .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
        .find(filter)
        .limit(MAX_LOCAL_PAGE_SIZE + 1)
        .await?
        .try_collect()
        .await?;
    if rows.len() > MAX_LOCAL_PAGE_SIZE as usize {
        return Err(CmaGatewayError::InventoryIncomplete.into());
    }
    Ok(rows)
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
                    .unique(true)
                    .partial_filter_expression(doc! {"occupies_account_slot": true})
                    .build(),
            )
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id": 1, "idempotency_key": 1})
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .partial_filter_expression(doc! {"idempotency_key": {"$type": "string"}})
                    .build(),
            )
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id": 1, "user_service_id": 1, "execution_base_url": 1, "cma_service_id": 1, "cma_sandbox_id": 1})
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .partial_filter_expression(doc! {"cma_sandbox_id": {"$type": "string"}, "user_service_id":{"$type":"string"}, "execution_base_url":{"$type":"string"}})
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
) -> AppResult<Option<AssistantCmaSandbox>> {
    Ok(db
        .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
        .find_one(doc! {"user_id": owner_id, "idempotency_key": key, "user_service_id":{"$type":"string"}, "execution_base_url":{"$type":"string"}})
        .await?)
}

fn same_creation(
    row: AssistantCmaSandbox,
    connection: &super::assistant_cma_gateway::CmaConnection,
    hash: &str,
) -> AppResult<AssistantCmaSandbox> {
    if row.user_service_id != connection.user_service_id
        || row.cma_service_id != connection.cma_service_id
        || row.execution_base_url != connection.execution_base_url
        || row.request_hash.as_deref() != Some(hash)
    {
        return Err(AppError::Conflict(
            "cma_creation_idempotency_conflict".into(),
        ));
    }
    Ok(row)
}

/// Reserve before dispatch. This method never sends a remote create request.
/// The same owner/key/hash returns its original identity. Only an explicit
/// confirmed-not-created outcome can reacquire local capacity. A retired
/// resource can never allocate a replacement. External resources use no slot.
pub async fn begin_creation(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    idempotency_key: &str,
    request: &[u8],
) -> AppResult<AssistantCmaSandbox> {
    validate_idempotency_key(idempotency_key)?;
    let normalized = prepare_creation_request(request)?;
    let parsed: Value = serde_json::from_slice(request).map_err(|_| invalid_request())?;
    let connection = gateway.connection(owner_id).await?;
    let db = super::transaction_service::durable_database(db);
    ensure_indexes(&db).await?;
    release_external_slots(&db, owner_id).await?;
    let retry = match creation_by_key(&db, owner_id, idempotency_key).await? {
        Some(row) => {
            let row = same_creation(row, &connection, &normalized.hash)?;
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
        .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
        .find_one(doc! {"user_id": owner_id, "occupies_account_slot": true, "user_service_id":{"$type":"string"}, "execution_base_url":{"$type":"string"}})
        .await?
    {
        if row.idempotency_key.as_deref() == Some(idempotency_key) {
            return same_creation(row, &connection, &normalized.hash);
        }
        return Err(occupied());
    }
    let previous = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"user_id":owner_id})
        .sort(doc! {"generation": -1})
        .projection(doc! {"generation":1})
        .await?;
    let generation = previous
        .map_or(Some(1), |row| {
            row.get_i64("generation")
                .ok()
                .and_then(|n| n.checked_add(1))
        })
        .ok_or_else(stale)?;
    let id = retry
        .as_ref()
        .map(|row| row.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let field = ["request_ciphertext"];
    let context = RecordContext::new(COLLECTION_NAME, &id, owner_id, &field)?;
    let ciphertext = super::record_field_service::encrypt_context(
        encryption_keys,
        &context,
        normalized.bytes.as_slice(),
    )
    .await?;
    let now = Utc::now();
    let row = AssistantCmaSandbox {
        id,
        user_id: owner_id.to_owned(),
        user_service_id: connection.user_service_id.clone(),
        cma_service_id: connection.cma_service_id.clone(),
        execution_base_url: connection.execution_base_url.clone(),
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
        source: NYXID_CREATED_SOURCE.into(),
        last_observed: None,
        machine_included: parsed.get("machine_included").and_then(Value::as_bool),
        resource_state: None,
        cpu_millis: None,
        memory_mib: None,
        capacity_source: None,
        last_activity_at: None,
        retire_requested_at: None,
        retirement_idempotency_key: None,
        retired_at: None,
        idempotency_key: Some(idempotency_key.to_owned()),
        request_hash: Some(normalized.hash.clone()),
        operation_id: None,
        submission_state: "pending".into(),
        request_ciphertext: Some(ciphertext),
        request_context_version: encryption_keys.contextual_record_writes().then_some(3),
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
                        .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
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
                    &connection,
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
                same_creation(row, &connection, &normalized.hash)
            } else {
                Err(occupied())
            }
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn recover_creation_request(
    db: &Database,
    keys: &EncryptionKeys,
    owner_id: &str,
    id: &str,
    generation: i64,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let row = get_owned(db, owner_id, id).await?;
    require_generation(&row, generation)?;
    let version = match row.request_context_version {
        None => None,
        Some(3) => Some(3),
        // The persisted marker is i32: never truncate unknown versions to u8.
        Some(_) => {
            return Err(AppError::Conflict(
                "cma_creation_recovery_unavailable".into(),
            ));
        }
    };
    let field = ["request_ciphertext"];
    let context = RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &field)?;
    let bytes = super::record_field_service::decrypt_context(
        keys,
        &context,
        version,
        row.request_ciphertext
            .as_deref()
            .ok_or_else(|| AppError::Conflict("cma_creation_recovery_unavailable".into()))?,
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

pub async fn get_owned(db: &Database, owner_id: &str, id: &str) -> AppResult<AssistantCmaSandbox> {
    let mut filter = created_filter(owner_id);
    filter.insert("_id", id);
    db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
        .find_one(filter)
        .await?
        .ok_or_else(|| AppError::NotFound("Sandbox not found".into()))
}

pub fn require_generation(row: &AssistantCmaSandbox, expected: i64) -> AppResult<()> {
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

async fn verify_deployment(gateway: &dyn CmaGateway, row: &AssistantCmaSandbox) -> AppResult<()> {
    let connection = gateway.connection(&row.user_id).await?;
    if connection.user_service_id != row.user_service_id
        || connection.cma_service_id != row.cma_service_id
        || connection.execution_base_url != row.execution_base_url
    {
        return Err(AppError::Conflict("cma_deployment_changed".into()));
    }
    Ok(())
}

fn has_confirmed_capacity(row: &AssistantCmaSandbox) -> bool {
    row.capacity_source.is_some()
        && row.cpu_millis.is_some_and(|value| value > 0)
        && row.memory_mib.is_some_and(|value| value > 0)
}

fn apply_observation(
    row: &mut AssistantCmaSandbox,
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
    // Capacity is a saved resource specification for this creation identity.
    // Lifecycle observations must not erase it or turn refreshes into resizing.
    if !has_confirmed_capacity(row)
        && observation.capacity_source.is_some()
        && observation.cpu_millis.is_some_and(|value| value > 0)
        && observation.memory_mib.is_some_and(|value| value > 0)
    {
        row.cpu_millis = observation.cpu_millis;
        row.memory_mib = observation.memory_mib;
        row.capacity_source = observation.capacity_source;
    }
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

fn binding_filter(row: &AssistantCmaSandbox) -> Document {
    doc! {"_id": &row.id, "user_id": &row.user_id, "user_service_id":&row.user_service_id, "execution_base_url":&row.execution_base_url,"cma_service_id": &row.cma_service_id,
    "source": &row.source, "creation_ref": &row.creation_ref, "generation": row.generation,
    "cma_sandbox_id": &row.cma_sandbox_id}
}

pub(super) fn observation_filter(row: &AssistantCmaSandbox) -> Document {
    let mut filter = binding_filter(row);
    filter.extend(doc! {"occupies_account_slot": row.occupies_account_slot, "admission_state": row.admission_state.as_str(), "retire_requested_at": row.retire_requested_at.map(bson::DateTime::from_chrono), "last_observed": row.last_observed.map(bson::DateTime::from_chrono)});
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
    before: &AssistantCmaSandbox,
    after: &AssistantCmaSandbox,
) -> AppResult<AssistantCmaSandbox> {
    let collection = db.collection::<AssistantCmaSandbox>(COLLECTION_NAME);
    let mut expected = before.clone();
    let mut changed = after.clone();
    // Network I/O is not repeated. Reconcile only local observation/registration
    // contention; a changed immutable binding still fails closed.
    for _ in 0..3 {
        if let Some(saved) = collection
            .find_one_and_update(
                observation_filter(&expected),
                doc! {"$set": observation_set(&changed)?},
            )
            .return_document(ReturnDocument::After)
            .await?
        {
            return Ok(saved);
        }
        let current = collection
            .find_one(binding_filter(before))
            .await?
            .ok_or_else(stale)?;
        // Confirmed retirement is monotonic. Otherwise reuse a concurrent exact
        // observation instead of letting a slower read overwrite newer facts.
        if current.admission_state == SandboxAdmissionState::Retired
            || (after.admission_state != SandboxAdmissionState::Retired
                && current.last_observed != before.last_observed)
        {
            return Ok(current);
        }
        changed = after.clone();
        if after.admission_state != SandboxAdmissionState::Retired {
            if current.retire_requested_at.is_some()
                || current.admission_state == SandboxAdmissionState::Retiring
            {
                changed.admission_state = SandboxAdmissionState::Retiring;
            }
            changed.submission_state = current.submission_state.clone();
            changed.request_ciphertext = current.request_ciphertext.clone();
            changed.request_context_version = current.request_context_version;
        }
        expected = current;
    }
    // Contention cannot prove that the resource generation changed, or release
    // its occupied slot. Readers already project unavailable observations safely.
    Err(CmaGatewayError::Unavailable.into())
}

pub(super) fn observation_set(after: &AssistantCmaSandbox) -> AppResult<Document> {
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
        "capacity_source",
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
) -> AppResult<AssistantCmaSandbox> {
    let row = get_owned(db, owner_id, id).await?;
    require_generation(&row, generation)?;
    verify_deployment(gateway, &row).await?;
    let remote_id = row
        .cma_sandbox_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("cma_creation_result_unknown".into()))?;
    let observation = if has_confirmed_capacity(&row) {
        gateway.get_sandbox_authority(owner_id, remote_id).await?
    } else {
        gateway.get_sandbox(owner_id, remote_id).await?
    };
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
) -> AppResult<AssistantCmaSandbox> {
    let row = get_owned(db, owner_id, id).await?;
    require_generation(&row, generation)?;
    verify_deployment(gateway, &row).await?;
    let observation =
        if row.cma_sandbox_id.as_deref() == Some(remote_id) && has_confirmed_capacity(&row) {
            gateway.get_sandbox_authority(owner_id, remote_id).await?
        } else {
            gateway.get_sandbox(owner_id, remote_id).await?
        };
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
            let collection = db.collection::<AssistantCmaSandbox>(COLLECTION_NAME);
            let mut identity = binding_filter(before);
            identity.insert("cma_sandbox_id", doc! {"$in": [bson::to_bson(&before.cma_sandbox_id).map_err(|_| invalid_request())?, bson::Bson::String(observation.id.clone())]});
            let current = collection.find_one(identity).session(&mut *tx).await?.ok_or_else(stale)?;
            if current.cma_sandbox_id.as_deref() == Some(observation.id.as_str())
                && (current.admission_state == SandboxAdmissionState::Retired
                    || (observation.state != SandboxObservationState::Retired
                        && current.last_observed != before.last_observed))
            {
                return Ok(current);
            }
            let duplicate = collection.find_one(doc! {
                "_id":{"$ne":&current.id},"user_id":&current.user_id,
                "user_service_id":&current.user_service_id,"execution_base_url":&current.execution_base_url,"cma_service_id":&current.cma_service_id,"cma_sandbox_id":&observation.id,
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
                db.collection::<Document>(crate::models::assistant_cma_agent::COLLECTION_NAME)
                    .update_many(doc! {"user_id":&current.user_id,"user_service_id":&current.user_service_id,"execution_base_url":&current.execution_base_url,"cma_service_id":&current.cma_service_id,"cma_sandbox_id":&observation.id,"sandbox_generation":duplicate.generation}, doc! {"$set":{"sandbox_generation":current.generation}})
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
) -> AppResult<AssistantCmaSandbox> {
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
        .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
        .find_one_and_update(observation_filter(&row), doc! {"$set": set})
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(stale)
}

pub(super) fn apply_no_resource_confirmation(
    row: &mut AssistantCmaSandbox,
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
) -> AppResult<AssistantCmaSandbox> {
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
    pub source: Option<SandboxCapacitySource>,
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

impl From<&AssistantCmaSandbox> for SandboxDto {
    fn from(row: &AssistantCmaSandbox) -> Self {
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
                source: row.capacity_source,
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
    /// Live status of this instance's creation references was checked. This
    /// says nothing about the owner's other resources in CMA.
    pub inventory_complete: bool,
}

pub async fn list(
    db: &Database,
    owner_id: &str,
    cursor: Option<&str>,
) -> AppResult<SandboxListDto> {
    let mut filter = created_filter(owner_id);
    if let Some(cursor) = cursor {
        Uuid::parse_str(cursor).map_err(|_| invalid_request())?;
        filter.insert("_id", doc! {"$gt": cursor});
    }
    let mut rows: Vec<AssistantCmaSandbox> = db
        .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
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
            "source": NYXID_CREATED_SOURCE,
            "occupies_account_slot": true,
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
            "cma_observation_unavailable" | "cma_inventory_incomplete"))
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

/// Refresh only resources tied to NyxID creation identities. Exact reads do
/// not discover/import resources and omissions never release a pending slot.
pub async fn refresh_owner(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
) -> AppResult<HashSet<String>> {
    let db = super::transaction_service::durable_database(db);
    ensure_indexes(&db).await?;
    release_external_slots(&db, owner_id).await?;
    super::assistant_cma_admission::recover_owner(&db, gateway, owner_id).await?;
    let mut observed = HashSet::new();
    for row in active_created(&db, owner_id).await? {
        if row.cma_sandbox_id.is_some() {
            let scoped = gateway.for_connection(&super::assistant_cma_gateway::CmaConnection {
                user_service_id: row.user_service_id.clone(),
                cma_service_id: row.cma_service_id.clone(),
                execution_base_url: row.execution_base_url.clone(),
            });
            let gateway = scoped.as_deref().unwrap_or(gateway);
            observe(&db, gateway, owner_id, &row.id, row.generation).await?;
            observed.insert(row.id);
        }
    }
    Ok(observed)
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
    result.sandboxes = result
        .sandboxes
        .into_iter()
        .map(|sandbox| {
            if refresh
                .as_ref()
                .is_some_and(|ids| ids.contains(&sandbox.id))
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
    super::assistant_cma_admission::recover_owner(db, gateway, owner_id).await?;
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
        .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id": id, "user_id": owner_id, "generation": generation},
            doc! {"$set": {"label": label, "updated_at": bson::DateTime::now()}},
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(stale)?;
    Ok(SandboxDto::from(&changed))
}

/// Resolve an existing parent only through a durable NyxID creation record.
/// A matching remote owner or ID cannot establish creation provenance.
pub async fn resolve_created(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner_id: &str,
    remote_id: &str,
) -> AppResult<AssistantCmaSandbox> {
    validate_remote_id(remote_id)?;
    let connection = gateway.connection(owner_id).await?;
    let mut filter = created_filter(owner_id);
    filter.insert("user_service_id", connection.user_service_id);
    filter.insert("execution_base_url", connection.execution_base_url);
    filter.insert("cma_service_id", connection.cma_service_id);
    filter.insert("cma_sandbox_id", remote_id);
    let row = db
        .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
        .find_one(filter)
        .await?
        .ok_or_else(|| AppError::NotFound("Sandbox was not created through NyxID".into()))?;
    observe(db, gateway, owner_id, &row.id, row.generation).await
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

/// Shared by management and the final dispatch guard. A keyless, already
/// pending intent cannot safely be replaced with a different request.
pub(super) fn retirement_intent_set(row: &AssistantCmaSandbox, key: &str) -> AppResult<Document> {
    validate_idempotency_key(key)?;
    match row.retirement_idempotency_key.as_deref() {
        Some(original) if original != key => {
            return Err(AppError::Conflict(
                "cma_retirement_idempotency_conflict".into(),
            ));
        }
        None if row.retire_requested_at.is_some() => {
            return Err(AppError::Conflict(
                "cma_retirement_recovery_unavailable".into(),
            ));
        }
        _ => {}
    }
    Ok(
        doc! {"admission_state":"retiring", "retirement_idempotency_key":key,
        "retire_requested_at":row.retire_requested_at.map(bson::DateTime::from_chrono).unwrap_or_else(bson::DateTime::now),
        "updated_at":bson::DateTime::now()},
    )
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
    if !row.occupies_account_slot {
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
        let mut filter = binding_filter(&row);
        filter.extend(doc! {"occupies_account_slot": true, "retire_requested_at": null});
        let collection = db.collection::<AssistantCmaSandbox>(COLLECTION_NAME);
        row = match collection
            .find_one_and_update(
                filter,
                doc! {"$set": retirement_intent_set(&row, idempotency_key)?},
            )
            .return_document(ReturnDocument::After)
            .await?
        {
            Some(changed) => changed,
            None => collection
                .find_one(binding_filter(&row))
                .await?
                .filter(|current| {
                    current.retire_requested_at.is_some()
                        || current.admission_state == SandboxAdmissionState::Retired
                })
                .ok_or_else(stale)?,
        };
        if row.admission_state == SandboxAdmissionState::Retired {
            return Ok(SandboxCommandDto {
                sandbox: SandboxDto::from(&row),
                pending: false,
            });
        }
    }
    // A reopened dialog supplies only a candidate. Retry the persisted intent,
    // including retirements first admitted by the ordinary CMA dispatch guard.
    let key = if command == SandboxCommand::Retire {
        row.retirement_idempotency_key
            .as_deref()
            .ok_or_else(|| AppError::Conflict("cma_retirement_recovery_unavailable".into()))?
    } else {
        idempotency_key
    };
    let result = match command {
        SandboxCommand::Pause => gateway.pause_sandbox(owner_id, &remote_id, key).await,
        SandboxCommand::Revive => gateway.revive_sandbox(owner_id, &remote_id, key).await,
        SandboxCommand::Retire => gateway.retire_sandbox(owner_id, &remote_id, key).await,
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
    use super::super::assistant_cma_gateway::*;
    use super::*;
    use serde_json::json;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct FakeGateway {
        current: Mutex<CmaSandbox>,
        timeout: AtomicBool,
        calls: Mutex<Vec<String>>,
        retirement_keys: Mutex<Vec<(String, String)>>,
    }

    impl FakeGateway {
        fn new() -> Self {
            Self {
                current: Mutex::new(remote_sandbox(SandboxObservationState::Ready)),
                timeout: AtomicBool::new(false),
                calls: Mutex::new(Vec::new()),
                retirement_keys: Mutex::new(Vec::new()),
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
        async fn connection(
            &self,
            owner: &str,
        ) -> CmaResult<super::super::assistant_cma_gateway::CmaConnection> {
            Ok(super::super::assistant_cma_gateway::CmaConnection {
                user_service_id: "user-service".into(),
                cma_service_id: self.service_id(owner).await?,
                execution_base_url: "https://cma.example/".into(),
            })
        }

        async fn get_sandbox(&self, _: &str, id: &str) -> CmaResult<CmaSandbox> {
            self.calls.lock().unwrap().push(format!("get:{id}"));
            if self.timeout.load(Ordering::Relaxed) {
                return Err(CmaGatewayError::Timeout);
            }
            Ok(self.current.lock().unwrap().clone())
        }
        async fn write_agent_file(
            &self,
            _: &str,
            agent: &str,
            path: &str,
            bytes: &[u8],
        ) -> CmaResult<CmaFileWrite> {
            self.calls.lock().unwrap().push(format!("write:{agent}"));
            Ok(CmaFileWrite {
                path: path.into(),
                size_bytes: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
            })
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
            key: &str,
        ) -> CmaResult<CmaLifecycleResult> {
            self.retirement_keys
                .lock()
                .unwrap()
                .push((id.into(), key.into()));
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

    #[test]
    fn same_key_returns_original_identity_and_changed_payload_conflicts() {
        let row = fixture_row();
        let mut connection = super::super::assistant_cma_gateway::CmaConnection {
            user_service_id: row.user_service_id.clone(),
            cma_service_id: row.cma_service_id.clone(),
            execution_base_url: row.execution_base_url.clone(),
        };
        let replay = same_creation(row.clone(), &connection, "request-hash").unwrap();
        assert_eq!(replay.id, row.id);
        assert!(same_creation(row.clone(), &connection, "different-hash").is_err());
        connection.user_service_id = "another-connection".into();
        assert!(same_creation(row, &connection, "request-hash").is_err());
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
        db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
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
        let gateway = FakeGateway::new();
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
        // A reference left by the previous discovery behavior must fold into
        // the durable creation identity without becoming an extra allocation.
        let mut legacy = external_row("conflict", false, "remote-sandbox");
        legacy.generation = row.generation + 1;
        db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
            .insert_one(&legacy)
            .await
            .unwrap();
        let discovered = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert_eq!(discovered.sandboxes.len(), 1);
        assert!(!discovered.conflict);
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

    fn external_row(source: &str, occupied: bool, remote_id: &str) -> AssistantCmaSandbox {
        let mut row = fixture_row();
        row.source = source.into();
        row.cma_sandbox_id = Some(remote_id.into());
        row.occupies_account_slot = occupied;
        row.admission_state = SandboxAdmissionState::Associated;
        row.observation_state = SandboxObservationState::Ready;
        row.idempotency_key = None;
        row.request_hash = None;
        row.request_ciphertext = None;
        row.request_context_version = None;
        row.submission_state = "not_applicable".into();
        row
    }

    #[tokio::test]
    async fn external_environments_do_not_consume_the_nyxid_creation_slot() {
        let Some(db) = test_db().await else {
            return;
        };
        let gateway = FakeGateway::new();
        for state in [
            SandboxObservationState::Paused,
            SandboxObservationState::Ready,
        ] {
            gateway.current.lock().unwrap().state = state;
            let listing = list_live(&db, &gateway, "owner", None).await.unwrap();
            assert!(listing.sandboxes.is_empty());
            assert!(listing.inventory_complete);
            assert_eq!(listing.occupied, 0);
            assert!(!listing.conflict);
        }
        assert!(matches!(
            resolve_created(&db, &gateway, "owner", "remote-sandbox").await,
            Err(AppError::NotFound(_))
        ));
        let keys = crate::test_utils::test_encryption_keys();
        let row = begin_creation(&db, &keys, &gateway, "owner", "key", br#"{"name":"task"}"#)
            .await
            .unwrap();
        assert_eq!(row.source, "nyxbot");
        assert!(row.occupies_account_slot);
        assert!(gateway.calls.lock().unwrap().is_empty());
        assert!(
            begin_creation(&db, &keys, &gateway, "owner", "second", b"{}")
                .await
                .is_err()
        );
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_external_references_are_hidden_and_release_only_local_occupancy() {
        let Some(db) = test_db().await else {
            return;
        };
        ensure_indexes(&db).await.unwrap();
        let gateway = FakeGateway::new();
        let mut refs = Vec::new();
        for (source, occupied, remote_id) in [
            ("discovered", true, "remote-sandbox"),
            ("conflict", false, "external-two"),
            ("claimed", false, "external-three"),
        ] {
            let row = external_row(source, occupied, remote_id);
            db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
                .insert_one(&row)
                .await
                .unwrap();
            refs.push(row);
        }
        // Cached context and live management must use the same scope.
        assert!(list(&db, "owner", None).await.unwrap().sandboxes.is_empty());
        let listing = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert!(listing.sandboxes.is_empty());
        assert_eq!(listing.occupied, 0);
        assert!(!listing.conflict);
        for row in &refs {
            assert!(matches!(
                get_owned(&db, "owner", &row.id).await,
                Err(AppError::NotFound(_))
            ));
            assert!(
                update_label(&db, "owner", &row.id, row.generation, "renamed")
                    .await
                    .is_err()
            );
            assert!(
                retire(&db, &gateway, "owner", &row.id, row.generation, "retire")
                    .await
                    .is_err()
            );
        }
        let external_agent_id = Uuid::new_v4().to_string();
        db.collection::<Document>(crate::models::assistant_cma_agent::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &external_agent_id, "user_id": "owner",
                "user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id": "cma-service", "cma_agent_id": "external-agent",
                "cma_sandbox_id": "remote-sandbox",
                "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
            })
            .await
            .unwrap();
        assert!(
            super::super::assistant_cma_agent_service::require_bound(
                &db,
                &gateway,
                "owner",
                &external_agent_id,
            )
            .await
            .is_err(),
            "a legacy Agent without a generation must not bypass created-parent provenance"
        );
        assert!(list(&db, "owner", None).await.unwrap().sandboxes.is_empty());
        let parents = active_created(&db, "owner").await.unwrap();
        let parent_ids: Vec<_> = parents
            .iter()
            .filter_map(|parent| parent.cma_sandbox_id.as_ref())
            .collect();
        assert!(
            db.collection::<Document>(crate::models::assistant_cma_agent::COLLECTION_NAME)
                .find_one(
                    doc! {"user_id":"owner","deleted_at":null,"cma_sandbox_id":{"$in":parent_ids}}
                )
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            gateway.calls.lock().unwrap().is_empty(),
            "external references must not reach the remote management API"
        );
        let keys = crate::test_utils::test_encryption_keys();
        let created = begin_creation(&db, &keys, &gateway, "owner", "new", b"{}")
            .await
            .unwrap();
        assert!(created.occupies_account_slot);
        for row in &refs {
            let stored = db
                .collection::<AssistantCmaSandbox>(COLLECTION_NAME)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            assert!(!stored.occupies_account_slot);
            assert_eq!(stored.source, row.source);
            assert_eq!(stored.observation_state, row.observation_state);
            assert_eq!(stored.retired_at, None);
        }
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            4
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn created_resource_refresh_ignores_external_inventory_and_preserves_unknown_capacity() {
        let Some(db) = test_db().await else {
            return;
        };
        ensure_indexes(&db).await.unwrap();
        let gateway = FakeGateway::new();
        let row = fixture_row();
        db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
            .insert_one(&row)
            .await
            .unwrap();
        // A known resource is refreshed through its exact detail.
        let listing = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert_eq!(listing.sandboxes.len(), 1);
        assert_eq!(listing.sandboxes[0].status, SandboxObservationState::Ready);
        assert_eq!(listing.occupied, 1);
        assert!(listing.inventory_complete && !listing.conflict);
        gateway.timeout.store(true, Ordering::Relaxed);
        let unavailable = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert!(!unavailable.inventory_complete);
        assert_eq!(unavailable.occupied, 1);
        assert_eq!(
            unavailable.sandboxes[0].status,
            SandboxObservationState::Unknown
        );
        assert!(detail_live(&db, &gateway, "other", &row.id).await.is_err());
        let keys = crate::test_utils::test_encryption_keys();
        assert!(
            begin_creation(&db, &keys, &gateway, "owner", "another", b"{}")
                .await
                .is_err()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_cleanup_never_releases_a_nyxid_request_with_unknown_outcome() {
        let Some(db) = test_db().await else {
            return;
        };
        let gateway = FakeGateway::new();
        let keys = crate::test_utils::test_encryption_keys();
        let row = begin_creation(&db, &keys, &gateway, "owner", "original", b"{}")
            .await
            .unwrap();
        mark_submission(&db, "owner", &row.id, row.generation, None, true)
            .await
            .unwrap();
        let external = external_row("conflict", false, "external");
        db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
            .insert_one(&external)
            .await
            .unwrap();
        let listing = list_live(&db, &gateway, "owner", None).await.unwrap();
        assert_eq!(listing.sandboxes.len(), 1);
        assert_eq!(listing.occupied, 1);
        assert!(!listing.conflict);
        assert!(
            begin_creation(&db, &keys, &gateway, "owner", "different", b"{}")
                .await
                .is_err()
        );
        let retry = begin_creation(&db, &keys, &gateway, "owner", "original", b"{}")
            .await
            .unwrap();
        assert_eq!(retry.id, row.id);
        assert!(retry.occupies_account_slot && retry.request_ciphertext.is_some());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn native_attachment_does_not_write_after_direct_cma_retirement() {
        let Some(db) = test_db().await else {
            return;
        };
        let gateway = FakeGateway::new();
        let parent = fixture_row();
        assert!(parent.retire_requested_at.is_none());
        db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
            .insert_one(&parent)
            .await
            .unwrap();
        let agent: crate::models::assistant_cma_agent::AssistantCmaAgent =
            bson::from_document(doc! {
                "_id":Uuid::new_v4().to_string(), "user_id":"owner", "user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id":"cma-service",
                "cma_agent_id":"managed-agent", "cma_sandbox_id":"remote-sandbox",
                "sandbox_generation":parent.generation,
                "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
            })
            .unwrap();
        let mut stale_agent = agent.clone();
        stale_agent.sandbox_generation = Some(parent.generation + 1);
        assert!(
            super::super::assistant_cma_tools::write_attachment_to_created_parent(
                &db,
                &gateway,
                "owner",
                &stale_agent,
                "uploads/stale.txt",
                b"stale",
            )
            .await
            .is_err()
        );
        let saved = super::super::assistant_cma_tools::write_attachment_to_created_parent(
            &db,
            &gateway,
            "owner",
            &agent,
            "uploads/test.txt",
            b"test",
        )
        .await
        .unwrap();
        assert_eq!(saved.size_bytes, 4);
        gateway.current.lock().unwrap().state = SandboxObservationState::Retired;
        assert!(
            super::super::assistant_cma_tools::write_attachment_to_created_parent(
                &db,
                &gateway,
                "owner",
                &agent,
                "uploads/blocked.txt",
                b"blocked",
            )
            .await
            .is_err()
        );
        let retired = get_owned(&db, "owner", &parent.id).await.unwrap();
        assert!(retired.retire_requested_at.is_none());
        assert_eq!(retired.admission_state, SandboxAdmissionState::Retired);
        assert!(!retired.occupies_account_slot);
        assert_eq!(
            gateway
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.starts_with("write:"))
                .count(),
            1
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
    fn capacity_source_survives_storage_and_unknown_projection() {
        let mut row = fixture_row();
        row.cpu_millis = Some(2000);
        row.memory_mib = Some(8192);
        row.last_observed = Some(Utc::now());
        let mut document = bson::to_document(&row).unwrap();
        document.insert("capacity_source", "runtime_allocation");
        let stored: AssistantCmaSandbox = bson::from_document(document).unwrap();
        let projection = unknown_projection(SandboxDto::from(&stored));
        let value = serde_json::to_value(projection).unwrap();
        assert_eq!(value["capacity"]["source"], "runtime_allocation");
        assert_eq!(value["capacity"]["cpu_millis"], 2000);
        assert_eq!(value["capacity"]["memory_mib"], 8192);
        assert_eq!(value["cpu_millis"], 2000);
        assert_eq!(value["memory_mib"], 8192);
        assert_eq!(value["status"], "unknown");
        assert_eq!(
            value["last_observed"],
            serde_json::to_value(stored.last_observed).unwrap()
        );
        assert_eq!(
            observation_set(&stored)
                .unwrap()
                .get_str("capacity_source")
                .unwrap(),
            "runtime_allocation"
        );
    }

    #[test]
    fn capacity_observation_preserves_confirmed_values_through_lifecycle_changes() {
        let mut row = fixture_row();
        let mut observed = remote_sandbox(SandboxObservationState::Ready);
        observed.cpu_millis = Some(2000);
        observed.memory_mib = Some(8192);
        observed.capacity_source = Some(SandboxCapacitySource::RuntimeAllocation);
        let generation = row.generation;
        apply_observation(&mut row, &observed, generation, Utc::now()).unwrap();
        let update = observation_set(&row).unwrap();
        assert_eq!(update.get_i64("cpu_millis").unwrap(), 2000);
        assert_eq!(update.get_i64("memory_mib").unwrap(), 8192);
        assert_eq!(
            update.get_str("capacity_source").unwrap(),
            "runtime_allocation"
        );
        for state in [
            SandboxObservationState::Paused,
            SandboxObservationState::Provisioning,
            SandboxObservationState::Unknown,
            SandboxObservationState::Ready,
            SandboxObservationState::Retired,
        ] {
            apply_observation(&mut row, &remote_sandbox(state), generation, Utc::now()).unwrap();
            let stored: AssistantCmaSandbox =
                bson::from_document(bson::to_document(&row).unwrap()).unwrap();
            assert_eq!(
                (stored.cpu_millis, stored.memory_mib, stored.capacity_source),
                (
                    Some(2000),
                    Some(8192),
                    Some(SandboxCapacitySource::RuntimeAllocation)
                )
            );
            assert_eq!(stored.observation_state, state);
            assert_eq!(
                stored.occupies_account_slot,
                state != SandboxObservationState::Retired
            );
        }
        let mut legacy = bson::to_document(&row).unwrap();
        legacy.remove("capacity_source");
        let old: AssistantCmaSandbox = bson::from_document(legacy).unwrap();
        assert_eq!(old.capacity_source, None);
    }

    #[test]
    fn capacity_observation_stores_only_the_first_complete_verified_pair() {
        let mut row = fixture_row();
        let generation = row.generation;
        let mut observed = remote_sandbox(SandboxObservationState::Ready);
        observed.cpu_millis = Some(2000);
        observed.capacity_source = Some(SandboxCapacitySource::RuntimeAllocation);
        apply_observation(&mut row, &observed, generation, Utc::now()).unwrap();
        assert_eq!(
            (row.cpu_millis, row.memory_mib, row.capacity_source),
            (None, None, None)
        );
        observed.memory_mib = Some(8192);
        observed.capacity_source = None;
        apply_observation(&mut row, &observed, generation, Utc::now()).unwrap();
        assert_eq!(
            (row.cpu_millis, row.memory_mib, row.capacity_source),
            (None, None, None)
        );
        observed.capacity_source = Some(SandboxCapacitySource::RuntimeAllocation);
        apply_observation(&mut row, &observed, generation, Utc::now()).unwrap();
        observed.cpu_millis = Some(4000);
        observed.memory_mib = Some(16384);
        observed.capacity_source = Some(SandboxCapacitySource::ProfileRevision);
        apply_observation(&mut row, &observed, generation, Utc::now()).unwrap();
        assert_eq!(
            (row.cpu_millis, row.memory_mib, row.capacity_source),
            (
                Some(2000),
                Some(8192),
                Some(SandboxCapacitySource::RuntimeAllocation)
            )
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

    #[tokio::test]
    async fn concurrent_observation_keeps_newer_facts_without_a_generation_error() {
        let Some(db) = test_db().await else {
            return;
        };
        let before = fixture_row();
        let mut after = before.clone();
        apply_observation(
            &mut after,
            &remote_sandbox(SandboxObservationState::Ready),
            1,
            Utc::now(),
        )
        .unwrap();
        let collection = db.collection::<AssistantCmaSandbox>(COLLECTION_NAME);
        collection.insert_one(&before).await.unwrap();
        collection.update_one(doc! {"_id": &before.id}, doc! {"$set": {
            "last_observed": bson::DateTime::now(), "observation_state": "paused",
            "resource_state": "paused", "admission_state": "associated", "label": "Owner rename",
            "agent_reference_version": 3_i64
        }}).await.unwrap();
        let result = persist_observation(&db, &before, &after).await;
        db.drop().await.unwrap();
        let current = result.expect("a concurrent observation is not a different generation");
        assert_eq!(current.observation_state, SandboxObservationState::Paused);
        assert_eq!(current.agent_reference_version, 3);
        assert_eq!(current.label, "Owner rename");
        assert!(current.occupies_account_slot);
    }

    #[tokio::test]
    async fn concurrent_retirement_intent_survives_a_live_observation() {
        let Some(db) = test_db().await else {
            return;
        };
        let before = fixture_row();
        let mut after = before.clone();
        apply_observation(
            &mut after,
            &remote_sandbox(SandboxObservationState::Ready),
            1,
            Utc::now(),
        )
        .unwrap();
        let collection = db.collection::<AssistantCmaSandbox>(COLLECTION_NAME);
        collection.insert_one(&before).await.unwrap();
        collection
            .update_one(
                doc! {"_id": &before.id},
                doc! {"$set": {
                    "retire_requested_at": bson::DateTime::now(), "admission_state": "retiring",
                    "agent_reference_version": 1_i64
                }},
            )
            .await
            .unwrap();
        let result = persist_observation(&db, &before, &after).await;
        db.drop().await.unwrap();
        let current = result.expect("retirement intent must merge with observation");
        assert_eq!(current.admission_state, SandboxAdmissionState::Retiring);
        assert!(current.retire_requested_at.is_some());
        assert!(current.occupies_account_slot);
        assert_eq!(current.agent_reference_version, 1);
    }

    #[tokio::test]
    async fn confirmed_retirement_wins_observation_races_and_cannot_be_resurrected() {
        let Some(db) = test_db().await else {
            return;
        };
        let before = fixture_row();
        let mut retired = before.clone();
        apply_observation(
            &mut retired,
            &remote_sandbox(SandboxObservationState::Retired),
            1,
            Utc::now(),
        )
        .unwrap();
        let collection = db.collection::<AssistantCmaSandbox>(COLLECTION_NAME);
        collection.insert_one(&before).await.unwrap();
        collection
            .update_one(
                doc! {"_id": &before.id},
                doc! {"$set": {
                    "last_observed": bson::DateTime::now(), "admission_state": "associated",
                    "agent_reference_version": 2_i64
                }},
            )
            .await
            .unwrap();
        let result = persist_observation(&db, &before, &retired).await;
        if result.is_err() {
            db.drop().await.unwrap();
        }
        let confirmed = result.expect("exact retirement must survive concurrent metadata");
        assert!(!confirmed.occupies_account_slot);
        assert!(confirmed.request_ciphertext.is_none());
        let mut late_ready = before.clone();
        apply_observation(
            &mut late_ready,
            &remote_sandbox(SandboxObservationState::Ready),
            1,
            Utc::now(),
        )
        .unwrap();
        let result = persist_observation(&db, &before, &late_ready).await;
        db.drop().await.unwrap();
        let current = result.expect("a late observation must keep confirmed retirement");
        assert_eq!(current.admission_state, SandboxAdmissionState::Retired);
        assert!(!current.occupies_account_slot);
        assert!(current.request_ciphertext.is_none());
    }

    #[tokio::test]
    async fn observation_merge_still_rejects_changed_generation_or_parent_identity() {
        let Some(db) = test_db().await else {
            return;
        };
        let before = fixture_row();
        let mut after = before.clone();
        apply_observation(
            &mut after,
            &remote_sandbox(SandboxObservationState::Retired),
            1,
            Utc::now(),
        )
        .unwrap();
        let collection = db.collection::<AssistantCmaSandbox>(COLLECTION_NAME);
        for change in [
            doc! {"generation": 2_i64},
            doc! {"cma_sandbox_id": "another-sandbox"},
            doc! {"user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id": "another-service"},
        ] {
            collection.insert_one(&before).await.unwrap();
            collection
                .update_one(doc! {"_id": &before.id}, doc! {"$set": change})
                .await
                .unwrap();
            assert!(persist_observation(&db, &before, &after).await.is_err());
            let current = collection
                .find_one(doc! {"_id": &before.id})
                .await
                .unwrap()
                .unwrap();
            assert!(current.occupies_account_slot);
            collection
                .delete_one(doc! {"_id": &before.id})
                .await
                .unwrap();
        }
        db.drop().await.unwrap();
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
            capacity_source: None,
            last_activity_at: None,
        }
    }

    #[tokio::test]
    async fn retirement_retry_reuses_persisted_intent_until_exact_retired_observation() {
        let db = test_db()
            .await
            .expect("writable test replica set is required");
        ensure_indexes(&db).await.unwrap();
        let gateway = FakeGateway::new();
        let row = fixture_row();
        db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
            .insert_one(&row)
            .await
            .unwrap();
        gateway.timeout.store(true, Ordering::Relaxed);
        assert!(
            retire(&db, &gateway, "owner", &row.id, 1, "first-retirement")
                .await
                .is_err()
        );
        let persisted = get_owned(&db, "owner", &row.id).await.unwrap();
        assert!(persisted.occupies_account_slot);
        assert!(persisted.retire_requested_at.is_some());
        gateway.timeout.store(false, Ordering::Relaxed);

        // A newly opened dialog supplies a new candidate, but it is the same
        // durable retirement intent for this exact environment and generation.
        let pending = retire(&db, &gateway, "owner", &row.id, 1, "reopened-dialog")
            .await
            .unwrap();
        assert!(pending.pending && pending.sandbox.occupies_account_slot);
        assert_eq!(
            pending.sandbox.admission_state,
            SandboxAdmissionState::Retiring
        );
        assert_eq!(
            pending.sandbox.observation_state,
            SandboxObservationState::Ready
        );
        assert_eq!(
            *gateway.retirement_keys.lock().unwrap(),
            vec![
                ("remote-sandbox".into(), "first-retirement".into()),
                ("remote-sandbox".into(), "first-retirement".into()),
            ]
        );
        let document = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            document.get_str("retirement_idempotency_key").unwrap(),
            "first-retirement"
        );
        assert_eq!(
            get_owned(&db, "owner", &row.id)
                .await
                .unwrap()
                .retire_requested_at,
            persisted.retire_requested_at
        );
        let public = serde_json::to_string(&pending).unwrap();
        assert!(!public.contains("first-retirement") && !public.contains("idempotency"));
        assert!(!format!("{persisted:?}").contains("first-retirement"));

        gateway.current.lock().unwrap().state = SandboxObservationState::Retired;
        let retired = retire(&db, &gateway, "owner", &row.id, 1, "reloaded-page")
            .await
            .unwrap();
        assert!(!retired.pending && !retired.sandbox.occupies_account_slot);
        assert_eq!(
            gateway.retirement_keys.lock().unwrap().last().unwrap().1,
            "first-retirement"
        );
        let calls = gateway.calls.lock().unwrap().len();
        retire(&db, &gateway, "owner", &row.id, 1, "already-retired")
            .await
            .unwrap();
        assert_eq!(gateway.calls.lock().unwrap().len(), calls);
        db.drop().await.unwrap();
    }

    #[test]
    fn keyless_pending_retirement_cannot_invent_a_replacement_intent() {
        let mut row = fixture_row();
        row.retire_requested_at = Some(Utc::now());
        assert!(retirement_intent_set(&row, "new-dialog-candidate").is_err());
        row.retirement_idempotency_key = Some("original-retirement".into());
        assert!(retirement_intent_set(&row, "new-dialog-candidate").is_err());
        let set = retirement_intent_set(&row, "original-retirement").unwrap();
        assert_eq!(
            set.get_str("retirement_idempotency_key").unwrap(),
            "original-retirement"
        );
        assert_eq!(
            *set.get_datetime("retire_requested_at").unwrap(),
            bson::DateTime::from_chrono(row.retire_requested_at.unwrap())
        );
    }

    #[tokio::test]
    async fn retirement_intent_cannot_cross_owner_environment_generation_or_deployment() {
        let db = test_db()
            .await
            .expect("writable test replica set is required");
        let gateway = FakeGateway::new();
        gateway.timeout.store(true, Ordering::Relaxed);
        let row = fixture_row();
        db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
            .insert_one(&row)
            .await
            .unwrap();
        assert!(
            retire(&db, &gateway, "owner", &row.id, 1, "first-retirement")
                .await
                .is_err()
        );
        let calls = gateway.calls.lock().unwrap().len();
        for (owner, id, generation) in [
            ("other", row.id.as_str(), 1),
            ("owner", "other", 1),
            ("owner", row.id.as_str(), 2),
        ] {
            assert!(
                retire(&db, &gateway, owner, id, generation, "wrong-intent")
                    .await
                    .is_err()
            );
        }
        db.collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id":"other-deployment"}},
            )
            .await
            .unwrap();
        assert!(
            retire(&db, &gateway, "owner", &row.id, 1, "wrong-deployment")
                .await
                .is_err()
        );
        assert_eq!(gateway.calls.lock().unwrap().len(), calls);
        let mut other = fixture_row();
        other.id = Uuid::new_v4().to_string();
        other.cma_sandbox_id = Some("other-sandbox".into());
        other.generation = 2;
        db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
            .insert_one(&other)
            .await
            .unwrap();
        assert!(
            retire(&db, &gateway, "owner", &other.id, 2, "other-retirement")
                .await
                .is_err()
        );
        assert_eq!(
            *gateway.retirement_keys.lock().unwrap(),
            vec![
                ("remote-sandbox".into(), "first-retirement".into()),
                ("other-sandbox".into(), "other-retirement".into()),
            ]
        );
        db.drop().await.unwrap();
    }

    fn fixture_row() -> AssistantCmaSandbox {
        let now = Utc::now();
        AssistantCmaSandbox {
            id: Uuid::new_v4().to_string(),
            user_id: "owner".into(),
            user_service_id: "user-service".into(),
            execution_base_url: "https://cma.example/".into(),
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
            capacity_source: None,
            last_activity_at: None,
            retire_requested_at: None,
            retirement_idempotency_key: None,
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
