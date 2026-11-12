//! Final-dispatch CMA admission. Names and user connection aliases never select
//! this contract; the dispatch registry selects the resolved catalog target.
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, doc},
    options::{IndexOptions, ReturnDocument},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;

use super::{
    assistant_cma_gateway::{
        CatalogCmaGateway, CmaGateway, validate_idempotency_key, validate_remote_id,
    },
    assistant_cma_sandbox_service as sandboxes,
    proxy_service::ProxyTarget,
};
use crate::{
    config::AppConfig,
    crypto::{
        aes::{EncryptionKeys, RecordContext},
        jwt::JwtKeys,
    },
    errors::{AppError, AppResult},
    models::{
        assistant_cma_creation::{AssistantCmaCreation, COLLECTION_NAME},
        assistant_cma_sandbox::{
            AssistantCmaSandbox, SandboxAdmissionState, SandboxObservationState,
        },
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Operation {
    Read,
    CreateAgent {
        sandbox: Option<String>,
        workspace: Option<String>,
    },
    CreateSandbox,
    Sandbox {
        id: String,
        retire: bool,
        admission: bool,
    },
    Agent {
        id: String,
        admission: bool,
    },
    WorkspaceAgent {
        id: String,
    },
}
fn invalid() -> AppError {
    AppError::BadRequest("Invalid CMA execution request".into())
}
fn unsupported() -> AppError {
    AppError::Forbidden(
        "cma_operation_not_supported: use the published personal Agent and Sandbox operations"
            .into(),
    )
}
fn drift() -> AppError {
    AppError::Conflict("cma_creation_authority_changed".into())
}
fn id(value: Option<&Value>) -> AppResult<Option<String>> {
    match value {
        None => Ok(None),
        Some(Value::String(value)) => {
            validate_remote_id(value)?;
            Ok(Some(value.clone()))
        }
        _ => Err(invalid()),
    }
}
fn classify(method: &str, path: &str, body: &[u8]) -> AppResult<Operation> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains(['?', '#', '%', '\\']) {
        return Err(invalid());
    }
    if matches!(method, "GET" | "HEAD" | "OPTIONS") {
        return Ok(Operation::Read);
    }
    let segments: Vec<_> = path.trim_start_matches('/').split('/').collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err(invalid());
    }
    for segment in segments.iter().skip(3) {
        validate_remote_id(segment)?;
    }
    match (method, segments.as_slice()) {
        ("POST", ["api", "v1", "agents"]) => {
            let parsed: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
            let sandbox = id(parsed.get("sandbox_id"))?;
            let workspace = id(parsed.get("workspace_id"))?;
            if sandbox.is_some() && workspace.is_some() {
                return Err(invalid());
            }
            if (sandbox.is_some() || workspace.is_some())
                && parsed.get("machine_included").is_some()
            {
                return Err(AppError::BadRequest(
                    "Omit machine_included when selecting an existing parent".into(),
                ));
            }
            Ok(Operation::CreateAgent { sandbox, workspace })
        }
        ("POST", ["api", "v1", "sandboxes"]) => Ok(Operation::CreateSandbox),
        ("POST", ["api", "v1", "workspaces", workspace, "agents"]) => {
            Ok(Operation::WorkspaceAgent {
                id: (*workspace).into(),
            })
        }
        ("DELETE", ["api", "v1", "sandboxes", sandbox]) => Ok(Operation::Sandbox {
            id: (*sandbox).into(),
            retire: true,
            admission: false,
        }),
        ("POST", ["api", "v1", "sandboxes", sandbox, "pause" | "revive"]) => {
            Ok(Operation::Sandbox {
                id: (*sandbox).into(),
                retire: false,
                admission: false,
            })
        }
        ("POST", ["api", "v1", "sandboxes", sandbox, "workspaces"])
        | ("POST", ["api", "v1", "sandboxes", sandbox, "workspaces", "inline"]) => {
            Ok(Operation::Sandbox {
                id: (*sandbox).into(),
                retire: false,
                admission: true,
            })
        }
        ("POST", ["api", "v1", "agents", agent, "stop"])
        | ("DELETE", ["api", "v1", "agents", agent]) => Ok(Operation::Agent {
            id: (*agent).into(),
            admission: false,
        }),
        ("POST", ["api", "v1", "agents", agent, "files" | "input"]) => Ok(Operation::Agent {
            id: (*agent).into(),
            admission: true,
        }),
        ("POST", ["api", "v2", "agents", agent, "responses"]) => {
            let value: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
            let conversation = value
                .get("conversation")
                .and_then(|v| v.as_str().or_else(|| v.get("id")?.as_str()));
            if value.get("conversation").is_some() && conversation != Some(*agent) {
                return Err(invalid());
            }
            if value.get("stream").and_then(Value::as_bool) != Some(true) {
                return Err(AppError::BadRequest("CMA tasks require stream=true".into()));
            }
            Ok(Operation::Agent {
                id: (*agent).into(),
                admission: true,
            })
        }
        (
            "POST",
            [
                "api",
                "v2",
                "agents",
                agent,
                "responses",
                _,
                action @ ("controls" | "interrupt"),
            ],
        ) => {
            validate_control(body, action == &"interrupt")?;
            Ok(Operation::Agent {
                id: (*agent).into(),
                admission: true,
            })
        }
        ("POST", ["api", "v1" | "v2", "responses"]) => {
            let value: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
            let agent = value
                .get("conversation")
                .and_then(|v| v.as_str().or_else(|| v.get("id")?.as_str()))
                .ok_or_else(unsupported)?;
            validate_remote_id(agent)?;
            if value.get("stream").and_then(Value::as_bool) != Some(true) {
                return Err(AppError::BadRequest("CMA tasks require stream=true".into()));
            }
            Ok(Operation::Agent {
                id: agent.into(),
                admission: true,
            })
        }
        _ => Err(unsupported()), // Launch/visitor/unknown allocation paths fail closed.
    }
}

// The hosted Aevatar subset cannot represent tagged oneOf schemas. Keep the
// remote strict union at the producer-owned execution boundary instead.
fn validate_control(body: &[u8], interrupt: bool) -> AppResult<()> {
    let value: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != 3
        || !["turn_sequence", "expected_agent_revision"]
            .iter()
            .all(|key| {
                value[*key]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= i64::MAX as u64)
            })
    {
        return Err(invalid());
    }
    let text_valid = |v: &Value| v.as_str().is_some_and(|text| !text.trim().is_empty());
    if interrupt {
        return if text_valid(&value["text"]) {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    let command = value["command"].as_object().ok_or_else(invalid)?;
    let valid = match value["command"]["type"].as_str() {
        Some("cancel") => command.len() == 1,
        Some("steer") => command.len() == 2 && text_valid(&value["command"]["text"]),
        Some("resolve_approval") => {
            command.len() == 3
                && value["command"]["approval_id"]
                    .as_str()
                    .is_some_and(|id| validate_remote_id(id).is_ok())
                && matches!(
                    value["command"]["decision"].as_str(),
                    Some("allow" | "deny" | "abort")
                )
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(invalid()) }
}

pub fn indexes() -> Vec<IndexModel> {
    vec![
        IndexModel::builder()
            .keys(doc! {"user_id":1,"idempotency_key":1})
            .options(IndexOptions::builder().unique(true).build())
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id":1,"user_service_id":1,"execution_base_url":1,"cma_service_id":1,"operation_id":1})
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .partial_filter_expression(doc! {"operation_id":{"$type":"string"},"user_service_id":{"$type":"string"},"execution_base_url":{"$type":"string"}})
                    .build(),
            )
            .build(),
        IndexModel::builder()
            .keys(doc! {"user_id":1,"user_service_id":1,"cma_service_id":1,"recovery_checked_at":1,"created_at":1})
            .build(),
    ]
}

const MAX_RECOVERY_OBSERVATIONS: i64 = 8;
const RECOVERY_BUDGET: Duration = Duration::from_secs(3);
const RECOVERY_COOLDOWN_SECONDS: i64 = 5;
// CMA's generic command cache expires at 24 hours. Lifecycle creation has a
// separate permanent ledger; generic allocations need a conservative bound.
const GENERIC_CREATION_REPLAY_HOURS: i64 = 23;

// Admission-only selector from the human recovery gateway; never forwarded.
// It narrows ordinary admission to an existing submission and grants no authority.
pub(super) const RECOVERY_SUBMISSION_HEADER: &str = "x-nyxid-cma-recovery-submission";
pub(super) fn recovery_submission(creation: &AssistantCmaCreation) -> String {
    format!(
        "{}:{}:{}",
        creation.id, creation.sandbox_generation, creation.submission_generation
    )
}

// No Debug: the binding includes an idempotency key and no caller may log it.
pub struct CmaDispatchPermit<'a> {
    db: Database,
    gateway: CatalogCmaGateway<'a>,
    owner: String,
    target: crate::models::downstream_service::DownstreamService,
    user_service_id: String,
    execution_base_url: String,
    operation: Operation,
    method: String,
    path: String,
    dispatch_path: zeroize::Zeroizing<String>,
    query: Option<String>,
    body_hash: String,
    // Only native/watch requests need the original business inputs and held
    // target again after asynchronous generic authentication has completed.
    authority_target: Option<&'a ProxyTarget>,
    authority_body: Option<zeroize::Zeroizing<Vec<u8>>>,
    key: Option<String>,
    sandbox: Option<AssistantCmaSandbox>,
    creation: Option<AssistantCmaCreation>,
}

impl<'a> CmaDispatchPermit<'a> {
    pub(super) fn with_gateway_authority(
        mut self,
        gateway: &CatalogCmaGateway<'_>,
        target: &'a ProxyTarget,
        body: &[u8],
    ) -> Self {
        self.gateway = self.gateway.with_authority_from(gateway);
        if self.gateway.has_dispatch_authority() {
            self.authority_target = Some(target);
            self.authority_body = Some(zeroize::Zeroizing::new(body.to_vec()));
        }
        self
    }

    pub(super) fn matches_binding(
        &self,
        target: &ProxyTarget,
        owner: &str,
        connection: Option<&str>,
    ) -> bool {
        self.owner == owner
            && connection == Some(self.user_service_id.as_str())
            && self.target.id == target.service.id
            && url::Url::parse(&target.base_url)
                .ok()
                .as_ref()
                .map(url::Url::as_str)
                == Some(self.execution_base_url.as_str())
    }
}
fn wire_hash(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}
fn completed() -> AppError {
    AppError::Conflict("cma_creation_already_completed: query the original resource".into())
}
fn duplicate(error: &AppError) -> bool {
    matches!(error, AppError::DatabaseError(error) if
        matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(w)) if w.code == 11000)
        || matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(w) if w.code == 11000))
}
fn same_request(
    previous: &AssistantCmaCreation,
    candidate: &AssistantCmaCreation,
) -> AppResult<()> {
    if previous.user_id != candidate.user_id
        || previous.user_service_id != candidate.user_service_id
        || previous.cma_service_id != candidate.cma_service_id
        || previous.execution_base_url != candidate.execution_base_url
        || previous.method != candidate.method
        || previous.path != candidate.path
        || previous.idempotency_key != candidate.idempotency_key
        || previous.request_hash != candidate.request_hash
        || previous.sandbox_ref_id != candidate.sandbox_ref_id
        || previous.creates_sandbox != candidate.creates_sandbox
        || previous.expected_workspace_id != candidate.expected_workspace_id
        || previous.submission_generation != candidate.submission_generation
    {
        return Err(drift());
    }
    if previous.submission_state == "confirmed" {
        return Err(completed());
    }
    if previous.sandbox_generation != candidate.sandbox_generation
        && previous.submission_state != "confirmed_not_created"
    {
        return Err(drift());
    }
    Ok(())
}
fn creation_filter(row: &AssistantCmaCreation) -> bson::Document {
    let mut filter = doc! {"_id":&row.id,"user_id":&row.user_id,"user_service_id":&row.user_service_id,
    "cma_service_id":&row.cma_service_id,"execution_base_url":&row.execution_base_url,
    "method":&row.method,"path":&row.path,
    "idempotency_key":&row.idempotency_key,"request_hash":&row.request_hash,
        "sandbox_ref_id":&row.sandbox_ref_id,"sandbox_generation":row.sandbox_generation};
    filter.insert(
        "submission_generation",
        if row.submission_generation == 0 {
            bson::Bson::Document(doc! {"$in":[0,bson::Bson::Null]})
        } else {
            row.submission_generation.into()
        },
    );
    filter
}
fn revision_filter(row: &AssistantCmaCreation) -> bson::Document {
    let mut filter = creation_filter(row);
    filter.insert(
        "revision",
        if row.revision == 0 {
            bson::Bson::Document(doc! {"$in":[0,bson::Bson::Null]})
        } else {
            row.revision.into()
        },
    );
    filter
}
fn require_parent(creation: &AssistantCmaCreation, parent: &AssistantCmaSandbox) -> AppResult<()> {
    if parent.id != creation.sandbox_ref_id
        || parent.user_id != creation.user_id
        || parent.user_service_id != creation.user_service_id
        || parent.execution_base_url != creation.execution_base_url
        || parent.cma_service_id != creation.cma_service_id
        || parent.generation != creation.sandbox_generation
    {
        return Err(drift());
    }
    if creation.cma_sandbox_id.as_deref().is_some_and(|remote| {
        parent
            .cma_sandbox_id
            .as_deref()
            .is_some_and(|actual| actual != remote)
    }) {
        return Err(drift());
    }
    Ok(())
}
async fn fence_parent(
    db: &Database,
    creation: &AssistantCmaCreation,
    tx: &mut mongodb::ClientSession,
) -> AppResult<AssistantCmaSandbox> {
    // This real write conflicts with retirement, generation replacement, and
    // receipt/discovery folding; a snapshot read alone is not a write fence.
    let parent = db
        .collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":&creation.sandbox_ref_id,"user_id":&creation.user_id,
            "user_service_id":&creation.user_service_id,"execution_base_url":&creation.execution_base_url,"cma_service_id":&creation.cma_service_id,"generation":creation.sandbox_generation},
            doc! {"$inc":{"agent_reference_version":1_i64}},
        )
        .return_document(ReturnDocument::After)
        .session(tx)
        .await?
        .ok_or_else(drift)?;
    require_parent(creation, &parent)?;
    Ok(parent)
}

/// Insert/reacquire one immutable creation identity under the account erasure
/// fence. An exact replay returns the winner of the owner/key race.
async fn store_creation(
    db: &Database,
    candidate: &AssistantCmaCreation,
) -> AppResult<AssistantCmaCreation> {
    let context = (db.clone(), candidate.clone());
    let result = super::transaction_service::run_db_only(db, &context, |tx, (db, candidate)| {
        Box::pin(async move {
            super::erasure_admission::fence_in_session(db, &candidate.user_id, tx).await?;
            let parent = fence_parent(db, candidate, tx).await?;
            require_available_parent(&parent)?;
            let collection = db.collection::<AssistantCmaCreation>(COLLECTION_NAME);
            if let Some(previous) = collection.find_one(doc! {"user_id":&candidate.user_id,"idempotency_key":&candidate.idempotency_key,"user_service_id":{"$type":"string"},"execution_base_url":{"$type":"string"}}).session(&mut *tx).await? {
                same_request(&previous, candidate)?;
                if previous.submission_state == "confirmed_not_created" {
                    let mut next = previous.clone();
                    next.sandbox_generation = candidate.sandbox_generation;
                    next.submission_generation = previous.submission_generation.checked_add(1).ok_or_else(drift)?;
                    next.submission_state = "pending".into();
                    next.request_ciphertext = candidate.request_ciphertext.clone();
                    next.request_context_version = candidate.request_context_version;
                    next.recovery_checked_at = None;
                    next.first_dispatched_at = None;
                    next.updated_at = Utc::now();
                    next.revision = next.revision.checked_add(1).ok_or_else(drift)?;
                    return collection.find_one_and_replace(revision_filter(&previous), next)
                        .return_document(ReturnDocument::After).session(tx).await?.ok_or_else(drift);
                }
                return Ok(previous);
            }
            collection.insert_one(candidate).session(tx).await?;
            Ok(candidate.clone())
        })
    }).await;
    match result {
        Err(error) if duplicate(&error) => {
            let previous = db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .find_one(doc! {"user_id":&candidate.user_id,"idempotency_key":&candidate.idempotency_key,"user_service_id":{"$type":"string"},"execution_base_url":{"$type":"string"}}).await?.ok_or_else(drift)?;
            same_request(&previous, candidate)?;
            if previous.sandbox_generation != candidate.sandbox_generation {
                return Err(drift());
            }
            Ok(previous)
        }
        result => result,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CreationReceipt {
    Lifecycle(String),
    Sandbox(String),
    Agent(String),
}
#[derive(Clone)]
enum CreationChange {
    Dispatch,
    Receipt(CreationReceipt),
    NoResource,
    Confirm {
        sandbox_id: String,
        agent_id: Option<String>,
    },
    Checked,
}
fn receipt_for(operation: &Operation, value: &Value) -> AppResult<CreationReceipt> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    validate_remote_id(id)?;
    Ok(match operation {
        Operation::CreateAgent { .. } => CreationReceipt::Lifecycle(id.into()),
        Operation::CreateSandbox => CreationReceipt::Sandbox(id.into()),
        Operation::WorkspaceAgent { .. } => CreationReceipt::Agent(id.into()),
        _ => return Err(invalid()),
    })
}
fn bind_id(current: &mut Option<String>, observed: &str) -> AppResult<()> {
    validate_remote_id(observed)?;
    if current
        .as_deref()
        .is_some_and(|current| current != observed)
    {
        return Err(drift());
    }
    *current = Some(observed.into());
    Ok(())
}
fn require_replay_window(row: &AssistantCmaCreation, now: chrono::DateTime<Utc>) -> AppResult<()> {
    if row.path == "/api/v1/agents" {
        return Ok(());
    }
    if row.cma_agent_id.is_some() || (row.creates_sandbox && row.cma_sandbox_id.is_some()) {
        return Err(AppError::Conflict(
            "cma_creation_observation_pending: query the original resource".into(),
        ));
    }
    let valid = match row.first_dispatched_at {
        Some(first) => {
            now >= first
                && first
                    .checked_add_signed(chrono::Duration::hours(GENERIC_CREATION_REPLAY_HOURS))
                    .is_some_and(|deadline| now < deadline)
        }
        None => row.submission_state == "pending",
    };
    if !valid {
        return Err(AppError::Conflict("cma_creation_replay_expired: retain the original request and confirm its result without resubmitting".into()));
    }
    Ok(())
}
fn apply_creation_change(
    row: &mut AssistantCmaCreation,
    parent: &mut AssistantCmaSandbox,
    change: &CreationChange,
) -> AppResult<()> {
    require_parent(row, parent)?;
    match change {
        CreationChange::Dispatch => {
            if row.submission_state == "confirmed" {
                return Err(completed());
            }
            if row.submission_state == "confirmed_not_created" {
                return Err(drift());
            }
            require_replay_window(row, Utc::now())?;
            require_available_parent(parent)?;
            if row.creates_sandbox
                && (!parent.occupies_account_slot || parent.cma_sandbox_id.is_some())
            {
                return Err(drift());
            }
            // An explicit replay may observe the same operation again. Keep an
            // already durable receipt; never turn it back into an unknown one.
            if row.operation_id.is_none() && row.cma_agent_id.is_none() {
                row.submission_state = "unknown".into();
            }
            row.first_dispatched_at.get_or_insert_with(Utc::now);
            if row.creates_sandbox {
                parent.admission_state = if row.operation_id.is_some() {
                    SandboxAdmissionState::Creating
                } else {
                    SandboxAdmissionState::Unknown
                };
                parent.submission_state = if row.operation_id.is_some() {
                    "submitted"
                } else {
                    "unknown"
                }
                .into();
                parent.updated_at = Utc::now();
            }
        }
        CreationChange::Receipt(receipt) => {
            if row.submission_state == "confirmed_not_created" {
                return Err(drift());
            }
            match receipt {
                CreationReceipt::Lifecycle(id) => bind_id(&mut row.operation_id, id)?,
                CreationReceipt::Sandbox(id) => {
                    if parent
                        .cma_sandbox_id
                        .as_deref()
                        .is_some_and(|actual| actual != id)
                    {
                        return Err(drift());
                    }
                    bind_id(&mut row.cma_sandbox_id, id)?;
                }
                CreationReceipt::Agent(id) => bind_id(&mut row.cma_agent_id, id)?,
            }
            if row.submission_state != "confirmed" {
                row.submission_state = "submitted".into();
            }
            if row.creates_sandbox && parent.cma_sandbox_id.is_none() {
                parent.operation_id = row.operation_id.clone();
                parent.submission_state = "submitted".into();
                // Receipt observation cannot reopen a retiring parent.
                if parent.retire_requested_at.is_none() {
                    parent.admission_state = SandboxAdmissionState::Creating;
                }
                parent.updated_at = Utc::now();
            }
        }
        CreationChange::NoResource => {
            if row.submission_state == "confirmed"
                || row.operation_id.is_some()
                || row.cma_agent_id.is_some()
                || (row.creates_sandbox && row.cma_sandbox_id.is_some())
            {
                return Err(drift());
            }
            if row.creates_sandbox {
                sandboxes::apply_no_resource_confirmation(
                    parent,
                    row.sandbox_generation,
                    &row.idempotency_key,
                    &row.request_hash,
                    Utc::now(),
                )?;
            }
            row.submission_state = "confirmed_not_created".into();
            row.request_ciphertext = None;
            row.request_context_version = None;
        }
        CreationChange::Confirm {
            sandbox_id,
            agent_id,
        } => {
            if row.submission_state == "confirmed_not_created"
                || parent.cma_sandbox_id.as_deref() != Some(sandbox_id)
            {
                return Err(drift());
            }
            bind_id(&mut row.cma_sandbox_id, sandbox_id)?;
            if let Some(agent_id) = agent_id {
                bind_id(&mut row.cma_agent_id, agent_id)?;
            }
            row.submission_state = "confirmed".into();
            row.request_ciphertext = None;
            row.request_context_version = None;
        }
        CreationChange::Checked => row.recovery_checked_at = Some(Utc::now()),
    }
    row.updated_at = Utc::now();
    row.revision = row.revision.checked_add(1).ok_or_else(drift)?;
    Ok(())
}
async fn change_creation(
    db: &Database,
    expected: &AssistantCmaCreation,
    change: CreationChange,
) -> AppResult<AssistantCmaCreation> {
    let context = (db.clone(), expected.clone(), change);
    super::transaction_service::run_db_only(db, &context, |tx, (db, expected, change)| {
        Box::pin(async move {
            super::erasure_admission::fence_in_session(db, &expected.user_id, tx).await?;
            let collection = db.collection::<AssistantCmaCreation>(COLLECTION_NAME);
            let current = collection
                .find_one(creation_filter(expected))
                .session(&mut *tx)
                .await?
                .ok_or_else(drift)?;
            let parent = fence_parent(db, &current, tx).await?;
            let mut next = current.clone();
            let mut next_parent = parent.clone();
            apply_creation_change(&mut next, &mut next_parent, change)?;
            if matches!(
                change,
                CreationChange::Dispatch | CreationChange::Receipt(_) | CreationChange::NoResource
            ) && current.creates_sandbox
            {
                let mut set = sandboxes::observation_set(&next_parent)?;
                set.insert("operation_id", next_parent.operation_id.clone());
                let result = db
                    .collection::<bson::Document>(
                        crate::models::assistant_cma_sandbox::COLLECTION_NAME,
                    )
                    .update_one(sandboxes::observation_filter(&parent), doc! {"$set":set})
                    .session(&mut *tx)
                    .await?;
                if result.matched_count != 1 {
                    return Err(drift());
                }
            }
            collection
                .find_one_and_replace(revision_filter(&current), next)
                .return_document(ReturnDocument::After)
                .session(tx)
                .await?
                .ok_or_else(drift)
        })
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn admit<'a>(
    db: &Database,
    keys: &'a EncryptionKeys,
    jwt: &JwtKeys,
    config: &AppConfig,
    owner: &str,
    user_service_id: &str,
    personal: bool,
    target: &ProxyTarget,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    headers: &[(String, String)],
    unsupported_transport: bool,
) -> AppResult<CmaDispatchPermit<'a>> {
    admit_with_gateway_authority(
        db,
        keys,
        jwt,
        config,
        owner,
        user_service_id,
        personal,
        target,
        method,
        path,
        query,
        body,
        headers,
        unsupported_transport,
        None,
    )
    .await
}

/// Native/watch authority must already cover the supplemental reads performed
/// during admission, as well as the final provider dispatch.
#[allow(clippy::too_many_arguments)]
pub(super) async fn admit_with_gateway_authority<'a>(
    db: &Database,
    keys: &'a EncryptionKeys,
    jwt: &JwtKeys,
    config: &AppConfig,
    owner: &str,
    user_service_id: &str,
    personal: bool,
    target: &ProxyTarget,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    headers: &[(String, String)],
    unsupported_transport: bool,
    authority: Option<&CatalogCmaGateway<'_>>,
) -> AppResult<CmaDispatchPermit<'a>> {
    if !personal || unsupported_transport {
        return Err(unsupported());
    }
    let operation = classify(method, path, body)?;
    // Reuse the generic path-auth construction and retain an exact final path.
    // The original path remains separately bound before any rewriting.
    let mut path_credentials = Vec::new();
    super::proxy_service::extend_with_path_credential(&mut path_credentials, target);
    let dispatch_path = zeroize::Zeroizing::new(super::proxy_service::build_forward_path(
        path,
        &path_credentials,
    )?);
    let execution_base_url = url::Url::parse(&target.base_url)
        .map_err(|_| invalid())?
        .to_string();
    let gateway = CatalogCmaGateway::from_user_service_parts(
        db,
        keys,
        jwt,
        config,
        user_service_id,
        Some(&target.service.id),
        Some(&execution_base_url),
    );
    let gateway = if let Some(source) = authority {
        gateway.with_authority_from(source)
    } else {
        gateway
    };
    if !matches!(operation, Operation::Read) {
        gateway
            .verify_target(owner, user_service_id, &execution_base_url)
            .await?;
    }
    if !matches!(operation, Operation::Read) && query.is_some_and(|q| !q.is_empty()) {
        return Err(invalid());
    }
    let keys_in_headers = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("idempotency-key"))
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    if keys_in_headers.len() > 1 {
        return Err(invalid());
    }
    let key = keys_in_headers.first().map(|value| (*value).clone());
    if !matches!(operation, Operation::Read) && !path.ends_with("/files") {
        validate_idempotency_key(key.as_deref().ok_or_else(|| {
            AppError::BadRequest("CMA changes require a stable Idempotency-Key".into())
        })?)?;
    }
    let db = super::transaction_service::durable_database(db);
    let mut permit = CmaDispatchPermit {
        db: db.clone(),
        gateway,
        owner: owner.into(),
        target: crate::models::downstream_service::DownstreamService {
            base_url: target.base_url.clone(),
            ..target.service.clone()
        },
        user_service_id: user_service_id.into(),
        execution_base_url: execution_base_url.clone(),
        operation: operation.clone(),
        method: method.into(),
        path: path.into(),
        dispatch_path,
        query: query.map(str::to_owned),
        body_hash: wire_hash(body),
        authority_target: None,
        authority_body: None,
        key,
        sandbox: None,
        creation: None,
    };
    if matches!(operation, Operation::Read) {
        return Ok(permit);
    }
    let creates_sandbox = matches!(
        &operation,
        Operation::CreateSandbox
            | Operation::CreateAgent {
                sandbox: None,
                workspace: None
            }
    );
    let creates_agent = matches!(
        &operation,
        Operation::CreateAgent { .. } | Operation::WorkspaceAgent { .. }
    );
    let pending_creation = if creates_sandbox || creates_agent {
        db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .find_one(doc! {"user_id":owner,"idempotency_key":&permit.key,"user_service_id":{"$type":"string"},"execution_base_url":{"$type":"string"}})
            .await?
    } else {
        None
    };
    if let Some(previous) = &pending_creation {
        if previous.user_service_id != user_service_id
            || previous.cma_service_id != target.service.id
            || previous.path != path
            || previous.method != method
            || previous.request_hash != sandboxes::prepare_creation_request(body)?.hash
            || previous.execution_base_url != execution_base_url
        {
            return Err(drift());
        }
        if previous.submission_state == "confirmed" {
            return Err(completed());
        }
    }
    let recovery = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(RECOVERY_SUBMISSION_HEADER))
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    if recovery.len() > 1 {
        return Err(invalid());
    }
    if let Some(identity) = recovery.first() {
        let previous = pending_creation.as_ref().ok_or_else(drift)?;
        if !creates_sandbox || **identity != recovery_submission(previous) {
            return Err(drift());
        }
        require_recovery_current(&db, previous).await?;
        let parent = sandboxes::get_owned(&db, owner, &previous.sandbox_ref_id).await?;
        require_parent(previous, &parent)?;
        permit.sandbox = Some(parent);
        permit.creation = Some(previous.clone());
        return Ok(permit);
    }
    let sandbox = if creates_sandbox {
        sandboxes::begin_creation(
            &db,
            keys,
            &permit.gateway,
            owner,
            permit.key.as_deref().ok_or_else(invalid)?,
            body,
        )
        .await?
    } else {
        let sandbox_id = match &operation {
            Operation::CreateAgent {
                sandbox: Some(id), ..
            }
            | Operation::Sandbox { id, .. } => id.clone(),
            Operation::CreateAgent {
                workspace: Some(id),
                ..
            }
            | Operation::WorkspaceAgent { id } => {
                permit.gateway.get_workspace_sandbox(owner, id).await?
            }
            Operation::Agent { id, .. } => {
                let remote = permit.gateway.get_agent(owner, id).await?;
                if remote.id != *id || remote.owner_id.as_deref().is_some_and(|id| id != owner) {
                    return Err(unsupported());
                }
                remote.sandbox_id.ok_or_else(invalid)?
            }
            _ => return Err(invalid()),
        };
        sandboxes::resolve_created(&db, &permit.gateway, owner, &sandbox_id).await?
    };
    let admits_work = creates_agent
        || matches!(
            &operation,
            Operation::Sandbox {
                admission: true,
                ..
            } | Operation::Agent {
                admission: true,
                ..
            }
        );
    if admits_work {
        require_available_parent(&sandbox)?;
    }
    if creates_sandbox && (sandbox.cma_sandbox_id.is_some() || !sandbox.occupies_account_slot) {
        return Err(drift());
    }
    if creates_sandbox || creates_agent {
        let normalized = sandboxes::prepare_creation_request(body)?;
        let id = pending_creation
            .as_ref()
            .map(|row| row.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let context = RecordContext::new(COLLECTION_NAME, &id, owner, &["request_ciphertext"])?;
        let request_ciphertext =
            super::record_field_service::encrypt_context(keys, &context, body).await?;
        let expected_workspace_id = match &operation {
            Operation::CreateAgent { workspace, .. } => workspace.clone(),
            Operation::WorkspaceAgent { id } => Some(id.clone()),
            _ => None,
        };
        let candidate = AssistantCmaCreation {
            id,
            user_id: owner.into(),
            user_service_id: user_service_id.into(),
            cma_service_id: target.service.id.clone(),
            execution_base_url: execution_base_url.clone(),
            method: method.into(),
            path: path.into(),
            idempotency_key: permit.key.clone().ok_or_else(invalid)?,
            request_hash: normalized.hash,
            sandbox_ref_id: sandbox.id.clone(),
            sandbox_generation: sandbox.generation,
            creates_sandbox,
            expected_workspace_id,
            submission_state: "pending".into(),
            operation_id: None,
            cma_agent_id: None,
            cma_sandbox_id: sandbox.cma_sandbox_id.clone(),
            revision: 0,
            submission_generation: pending_creation
                .as_ref()
                .map_or(0, |row| row.submission_generation),
            recovery_checked_at: None,
            first_dispatched_at: None,
            request_ciphertext: Some(request_ciphertext),
            request_context_version: keys.contextual_record_writes().then_some(3),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .create_indexes(indexes())
            .await?;
        permit.creation = Some(store_creation(&db, &candidate).await?);
    }
    permit.sandbox = Some(sandbox);
    Ok(permit)
}

pub(super) fn require_available_parent(row: &AssistantCmaSandbox) -> AppResult<()> {
    if row.source != sandboxes::NYXID_CREATED_SOURCE || !row.occupies_account_slot {
        return Err(AppError::Conflict("cma_nyxid_sandbox_required".into()));
    }
    if row.retire_requested_at.is_some()
        || matches!(
            row.admission_state,
            SandboxAdmissionState::Retiring | SandboxAdmissionState::Retired
        )
        || matches!(
            row.observation_state,
            SandboxObservationState::Retiring | SandboxObservationState::Retired
        )
    {
        return Err(AppError::Conflict(
            "cma_sandbox_retiring: new Agents and tasks are closed".into(),
        ));
    }
    Ok(())
}

fn creation_refusal_confirms_no_resource(path: &str, status: u16, value: &Value) -> bool {
    let code = value
        .get("code")
        .or_else(|| value.get("error"))
        .and_then(Value::as_str);
    if status == 409 && code == Some("sandbox_credential_setup_required") {
        return true;
    }
    // These synchronous refusals precede durable admission in CMA. Match known
    // contract violations exactly: generic validation and authorization
    // failures can also reject recovery after an upstream policy change.
    // NoResource additionally fences against a concurrently committed receipt.
    path == "/api/v1/agents"
        && status == 422
        && value.get("status").and_then(Value::as_u64) == Some(422)
        && ["id", "operation_id", "agent_id", "sandbox_id"]
            .iter()
            .all(|field| value.get(field).is_none_or(Value::is_null))
        && match (code, value.get("type").and_then(Value::as_str)) {
            (
                Some("invalid_request"),
                Some("https://spec.chrono-ai.fun/cma/1.0/problems/agent-manager-invalid-request"),
            ) => {
                value.get("detail").and_then(Value::as_str).is_some_and(|detail| {
                    detail == "invalid Agent lifecycle request: desktop creation requires initial_message"
                        // CMA checks replay before validating inline permissions,
                        // then validates them before persisting a new operation.
                        || detail.strip_prefix("invalid Agent lifecycle request: unsupported permission_mode ")
                            .is_some_and(|mode| !mode.trim().is_empty())
                })
            }
            (
                Some("desktop_unsupported"),
                Some(
                    "https://spec.chrono-ai.fun/cma/1.0/problems/agent-manager-desktop-unsupported",
                ),
            ) => true,
            _ => false,
        }
}

#[async_trait::async_trait]
impl super::service_dispatch_guard::ServiceDispatchGuard for CmaDispatchPermit<'_> {
    fn validate_original_request(
        &self,
        request: super::service_dispatch_guard::OriginalRequest<'_>,
    ) -> AppResult<()> {
        let path = super::proxy_authorization::CanonicalPath::from_mcp_built(request.path)?;
        if request.method != self.method
            || path.as_policy_path() != self.path
            || request.query != self.query.as_deref()
            || wire_hash(request.body) != self.body_hash
        {
            return Err(drift());
        }
        let mut keys = request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("idempotency-key"))
            .map(|(_, value)| value.as_str());
        let matches = match (self.key.as_deref(), keys.next()) {
            (None, None) => true,
            (Some(expected), Some(actual)) => actual == expected,
            _ => false,
        };
        if !matches || keys.next().is_some() {
            return Err(drift());
        }
        Ok(())
    }

    /// Called after all header/body rewriting, directly before network dispatch.
    async fn before_send(&self, request: &reqwest::Request) -> AppResult<()> {
        if let Some(target) = self.authority_target {
            self.gateway
                .revalidate_dispatch(
                    &self.owner,
                    target,
                    &self.method,
                    &self.path,
                    self.query.as_deref(),
                    self.authority_body
                        .as_deref()
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                )
                .await?;
        }
        self.gateway
            .verify_target(&self.owner, &self.user_service_id, &self.execution_base_url)
            .await?;
        let expected = format!(
            "{}/{}",
            self.target.base_url.trim_end_matches('/'),
            self.dispatch_path.trim_start_matches('/')
        );
        let expected = url::Url::parse(&expected).map_err(|_| drift())?;
        let mut actual = request.url().clone();
        // The original query/body were validated before the generic proxy
        // applied authentication. Keep every other URL component exact.
        actual.set_query(None);
        if request.method().as_str() != self.method || actual != expected {
            return Err(drift());
        }
        if let Some(key) = &self.key {
            let values = request
                .headers()
                .get_all("idempotency-key")
                .iter()
                .collect::<Vec<_>>();
            if values.len() != 1 || values[0].to_str().ok() != Some(key.as_str()) {
                return Err(drift());
            }
        }
        if let Some(creation) = &self.creation {
            change_creation(&self.db, creation, CreationChange::Dispatch).await?;
        } else if let Some(sandbox) = &self.sandbox {
            let context = (
                self.db.clone(),
                self.owner.clone(),
                sandbox.clone(),
                self.operation.clone(),
                self.key.clone(),
            );
            super::transaction_service::run_db_only(&self.db, &context, |tx, (db, owner, before, operation, key)| {
                Box::pin(async move {
                    super::erasure_admission::fence_in_session(db, owner, tx).await?;
                    let collection = db.collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME);
                    let fresh = collection.find_one_and_update(doc! {"_id":&before.id,"user_id":owner,"generation":before.generation,"user_service_id":&before.user_service_id,"execution_base_url":&before.execution_base_url,"cma_service_id":&before.cma_service_id,"cma_sandbox_id":&before.cma_sandbox_id}, doc! {"$inc":{"agent_reference_version":1_i64}})
                        .return_document(ReturnDocument::After).session(&mut *tx).await?.ok_or_else(drift)?;
                    if matches!(operation, Operation::Agent { admission:true,.. } | Operation::Sandbox { admission:true,.. }) { require_available_parent(&fresh)?; }
                    if matches!(operation, Operation::Sandbox { retire:true,.. }) && fresh.admission_state != SandboxAdmissionState::Retired {
                        let set = sandboxes::retirement_intent_set(&fresh, key.as_deref().ok_or_else(invalid)?)?;
                        let result = collection.update_one(sandboxes::observation_filter(&fresh), doc! {"$set":set}).session(tx).await?;
                        if result.matched_count != 1 { return Err(drift()); }
                    }
                    Ok(())
                })
            }).await?;
        }
        Ok(())
    }
    async fn observe_response(&self, response: reqwest::Response) -> AppResult<reqwest::Response> {
        let observes_creation =
            self.creation.is_some() || self.path.starts_with("/api/v1/agent-lifecycle/");
        if !observes_creation {
            return Ok(response);
        }
        let status = response.status();
        let headers = response.headers().clone();
        let version = response.version();
        let mut response = response;
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(chunk) = response.chunk().await.map_err(|_| drift())? {
                if bytes.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
                    return Err(invalid());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(())
        })
        .await
        .map_err(|_| AppError::Conflict("cma_creation_result_unknown".into()))??;
        let parsed = serde_json::from_slice::<Value>(&bytes);
        // Keep dispatched creations uncertain when a successful receipt cannot be decoded.
        if self.creation.is_some() && status.is_success() && parsed.is_err() {
            return Err(AppError::Conflict("cma_creation_result_unknown".into()));
        }
        if let Ok(value) = parsed {
            if let Some(creation) = &self.creation {
                if status.is_success() {
                    let receipt = receipt_for(&self.operation, &value)?;
                    let creation = change_creation(
                        &self.db,
                        creation,
                        CreationChange::Receipt(receipt.clone()),
                    )
                    .await?;
                    let observed = match receipt {
                        CreationReceipt::Lifecycle(operation) => {
                            record_creation_observation(
                                &self.db,
                                &self.gateway,
                                &self.owner,
                                &self.target.id,
                                &operation,
                                &value,
                            )
                            .await
                        }
                        CreationReceipt::Sandbox(id) => {
                            finish_sandbox(&self.db, &self.gateway, &creation, &id).await
                        }
                        CreationReceipt::Agent(id) => {
                            finish_agent(&self.db, &self.gateway, &creation, &id).await
                        }
                    };
                    if let Err(error) = observed
                        && !observation_unavailable(&error)
                    {
                        return Err(error);
                    }
                } else if creation_refusal_confirms_no_resource(&self.path, status.as_u16(), &value)
                {
                    // This exact pre-provisioning refusal is authoritative. A
                    // late refusal cannot erase an already recorded receipt.
                    change_creation(&self.db, creation, CreationChange::NoResource).await?;
                }
            } else if status.is_success()
                && let Some(operation) = self.path.strip_prefix("/api/v1/agent-lifecycle/")
            {
                record_creation_observation(
                    &self.db,
                    &self.gateway,
                    &self.owner,
                    &self.target.id,
                    operation,
                    &value,
                )
                .await?;
            }
        }
        let mut rebuilt = http::Response::builder()
            .status(status)
            .version(version)
            .body(bytes)
            .map_err(|_| invalid())?;
        *rebuilt.headers_mut() = headers;
        Ok(reqwest::Response::from(rebuilt))
    }
}

fn observation_unavailable(error: &AppError) -> bool {
    matches!(error, AppError::NotFound(_))
        || matches!(error, AppError::Conflict(message) if matches!(message.as_str(),
        "cma_observation_unavailable" | "cma_creation_authority_changed" | "cma_sandbox_generation_changed"))
}
async fn verify_creation(
    gateway: &dyn CmaGateway,
    creation: &AssistantCmaCreation,
) -> AppResult<()> {
    gateway
        .verify_creation_deployment(
            &creation.user_id,
            &creation.user_service_id,
            &creation.execution_base_url,
        )
        .await?;
    Ok(())
}
async fn finish_sandbox(
    db: &Database,
    gateway: &dyn CmaGateway,
    creation: &AssistantCmaCreation,
    remote_id: &str,
) -> AppResult<()> {
    if !creation.creates_sandbox || creation.path != "/api/v1/sandboxes" {
        return Err(drift());
    }
    verify_creation(gateway, creation).await?;
    sandboxes::associate_created(
        db,
        gateway,
        &creation.user_id,
        &creation.sandbox_ref_id,
        creation.sandbox_generation,
        remote_id,
    )
    .await?;
    change_creation(
        db,
        creation,
        CreationChange::Confirm {
            sandbox_id: remote_id.into(),
            agent_id: None,
        },
    )
    .await?;
    Ok(())
}
async fn finish_agent(
    db: &Database,
    gateway: &dyn CmaGateway,
    creation: &AssistantCmaCreation,
    agent_id: &str,
) -> AppResult<()> {
    verify_creation(gateway, creation).await?;
    let agent = gateway.get_agent(&creation.user_id, agent_id).await?;
    if agent.id != agent_id
        || agent
            .owner_id
            .as_deref()
            .is_some_and(|owner| owner != creation.user_id)
    {
        return Err(AppError::Forbidden("CMA resource access denied".into()));
    }
    let remote_parent = agent.sandbox_id.as_deref().ok_or_else(invalid)?;
    let parent = sandboxes::get_owned(db, &creation.user_id, &creation.sandbox_ref_id).await?;
    require_parent(creation, &parent)?;
    if creation
        .expected_workspace_id
        .as_deref()
        .is_some_and(|workspace| agent.workspace_id.as_deref() != Some(workspace))
    {
        return Err(drift());
    }
    if creation.creates_sandbox {
        sandboxes::associate_created(
            db,
            gateway,
            &creation.user_id,
            &creation.sandbox_ref_id,
            creation.sandbox_generation,
            remote_parent,
        )
        .await?;
    } else if parent.cma_sandbox_id.as_deref() != Some(remote_parent) {
        return Err(drift());
    }
    let source = super::assistant_cma_agent_service::AgentSource {
        creation_operation_id: creation.operation_id.clone(),
        ..Default::default()
    };
    let registered = super::assistant_cma_agent_service::register_live(
        db,
        gateway,
        &creation.user_id,
        agent_id,
        Some(&source),
    )
    .await?;
    if registered.sandbox_generation != Some(creation.sandbox_generation)
        || registered.cma_sandbox_id.as_deref() != Some(remote_parent)
    {
        return Err(drift());
    }
    change_creation(
        db,
        creation,
        CreationChange::Confirm {
            sandbox_id: remote_parent.into(),
            agent_id: Some(agent_id.into()),
        },
    )
    .await?;
    Ok(())
}
fn lifecycle_agent<'a>(operation: &str, value: &'a Value) -> AppResult<Option<&'a str>> {
    validate_remote_id(operation)?;
    if value.get("id").and_then(Value::as_str) != Some(operation) {
        return Err(drift());
    }
    match value.get("agent_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => {
            validate_remote_id(id)?;
            Ok(Some(id))
        }
        _ => Err(invalid()),
    }
}
pub async fn record_creation_observation(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    service: &str,
    operation: &str,
    value: &Value,
) -> AppResult<()> {
    let agent_id = lifecycle_agent(operation, value)?;
    let connection = gateway.connection(owner).await?;
    let db = super::transaction_service::durable_database(db);
    let Some(creation) = db
        .collection::<AssistantCmaCreation>(COLLECTION_NAME)
        .find_one(doc! {"user_id":owner,"user_service_id":&connection.user_service_id,"execution_base_url":&connection.execution_base_url,"cma_service_id":service,"operation_id":operation})
        .await?
    else {
        return Ok(());
    };
    if creation.path != "/api/v1/agents" || creation.submission_state == "confirmed_not_created" {
        return Err(drift());
    }
    verify_creation(gateway, &creation).await?;
    if let Some(agent_id) = agent_id {
        finish_agent(&db, gateway, &creation, agent_id).await?;
    }
    // A failed/unknown lifecycle without an Agent is not absence proof. Keep
    // both the slot and encrypted request, independent of chat/watch retention.
    Ok(())
}

/// Explicit owner recovery of the original Sandbox creation. The local identity
/// and generation select recovery material; the caller cannot replace it.
pub async fn recover_creation(
    db: &Database,
    keys: &EncryptionKeys,
    gateway: &CatalogCmaGateway<'_>,
    owner: &str,
    id: &str,
    generation: i64,
) -> AppResult<sandboxes::SandboxCommandDto> {
    super::assistant_cma_agent_service::require_enabled(db, owner).await?;
    let parent = sandboxes::get_owned(db, owner, id).await?;
    sandboxes::require_generation(&parent, generation)?;
    let creation = db
        .collection::<AssistantCmaCreation>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id":owner,"user_service_id":&parent.user_service_id,"execution_base_url":&parent.execution_base_url,"cma_service_id":&parent.cma_service_id,
            "sandbox_ref_id":id,"sandbox_generation":generation,"creates_sandbox":true,
            "idempotency_key":parent.idempotency_key.as_deref().ok_or_else(drift)?,
        })
        .await?
        .ok_or_else(|| AppError::Conflict("cma_creation_recovery_unavailable".into()))?;
    require_parent(&creation, &parent)?;
    verify_creation(gateway, &creation).await?;
    if creation.submission_state == "confirmed_not_created" {
        return Err(drift());
    }
    if creation.submission_state != "confirmed" {
        if let Some(operation) = &creation.operation_id {
            let value = gateway.get_lifecycle(owner, operation).await?;
            record_creation_observation(
                db,
                gateway,
                owner,
                &creation.cma_service_id,
                operation,
                &value,
            )
            .await?;
        } else if let Some(agent) = &creation.cma_agent_id {
            finish_agent(db, gateway, &creation, agent).await?;
        } else if let Some(sandbox) = &creation.cma_sandbox_id {
            finish_sandbox(db, gateway, &creation, sandbox).await?;
        } else {
            require_recovery_current(db, &creation).await?;
            let body = sandboxes::recover_creation_request(db, keys, owner, id, generation).await?;
            if sandboxes::prepare_creation_request(&body)?.hash != creation.request_hash {
                return Err(drift());
            }
            gateway.replay_creation(keys, &creation, &body).await?;
        }
    }
    let current = db
        .collection::<AssistantCmaCreation>(COLLECTION_NAME)
        .find_one(creation_filter(&creation))
        .await?
        .ok_or_else(drift)?;
    let parent = sandboxes::get_owned(db, owner, id).await?;
    require_parent(&current, &parent)?;
    Ok(sandboxes::SandboxCommandDto {
        sandbox: sandboxes::SandboxDto::from(&parent),
        pending: current.submission_state != "confirmed",
    })
}

/// Recheck the exact saved submission after ordinary admission as well. Its
/// dispatch guard then fences this generation transactionally before sending.
pub(super) async fn require_recovery_current(
    db: &Database,
    expected: &AssistantCmaCreation,
) -> AppResult<()> {
    let current = db
        .collection::<AssistantCmaCreation>(COLLECTION_NAME)
        .find_one(creation_filter(expected))
        .await?
        .ok_or_else(drift)?;
    if !current.creates_sandbox
        || !matches!(current.submission_state.as_str(), "unknown" | "submitted")
        || current.first_dispatched_at.is_none()
        || current.operation_id.is_some()
        || current.cma_agent_id.is_some()
        || current.cma_sandbox_id.is_some()
    {
        return Err(drift());
    }
    require_replay_window(&current, Utc::now())?;
    let parent = sandboxes::get_owned(db, &current.user_id, &current.sandbox_ref_id).await?;
    require_parent(&current, &parent)?;
    require_available_parent(&parent)?;
    if parent.cma_sandbox_id.is_some()
        || parent.idempotency_key.as_deref() != Some(current.idempotency_key.as_str())
        || parent.request_hash.as_deref() != Some(current.request_hash.as_str())
    {
        return Err(drift());
    }
    Ok(())
}

/// Bounded observation only: no POST, no key replacement, and no request-body
/// rewriting. The stored operation/resource identities survive chat deletion.
pub async fn recover_owner(db: &Database, gateway: &dyn CmaGateway, owner: &str) -> AppResult<()> {
    let db = super::transaction_service::durable_database(db);
    let cutoff = bson::DateTime::from_chrono(
        Utc::now() - chrono::Duration::seconds(RECOVERY_COOLDOWN_SECONDS),
    );
    let pending: Vec<AssistantCmaCreation> = db.collection::<AssistantCmaCreation>(COLLECTION_NAME).find(doc! {
        "user_id":owner,"user_service_id":{"$type":"string"},"execution_base_url":{"$type":"string"},"submission_state":{"$nin":["confirmed","confirmed_not_created"]},
        "$and":[{"$or":[{"operation_id":{"$type":"string"}},{"cma_agent_id":{"$type":"string"}},{"creates_sandbox":true,"cma_sandbox_id":{"$type":"string"}}]},
            {"$or":[{"recovery_checked_at":bson::Bson::Null},{"recovery_checked_at":{"$lte":cutoff}}]}],
    }).sort(doc! {"recovery_checked_at":1,"created_at":1,"_id":1}).limit(MAX_RECOVERY_OBSERVATIONS).await?.try_collect().await?;
    let deadline = tokio::time::Instant::now() + RECOVERY_BUDGET;
    for creation in pending {
        let scoped = gateway.for_connection(&super::assistant_cma_gateway::CmaConnection {
            user_service_id: creation.user_service_id.clone(),
            cma_service_id: creation.cma_service_id.clone(),
            execution_base_url: creation.execution_base_url.clone(),
        });
        let gateway = scoped.as_deref().unwrap_or(gateway);
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        let observe = async {
            verify_creation(gateway, &creation).await?;
            let creation = change_creation(&db, &creation, CreationChange::Checked).await?;
            if let Some(operation) = &creation.operation_id {
                let value = gateway.get_lifecycle(owner, operation).await?;
                record_creation_observation(
                    &db,
                    gateway,
                    owner,
                    &creation.cma_service_id,
                    operation,
                    &value,
                )
                .await
            } else if let Some(agent) = &creation.cma_agent_id {
                finish_agent(&db, gateway, &creation, agent).await
            } else if let Some(sandbox) = &creation.cma_sandbox_id {
                finish_sandbox(&db, gateway, &creation, sandbox).await
            } else {
                Ok(())
            }
        };
        match tokio::time::timeout_at(deadline, observe).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) if observation_unavailable(&error) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod fingerprint_regressions {
        use super::*;
        include!("assistant_cma_admission_fingerprint_tests.rs");

        mod writer_cutover {
            use super::*;
            include!("assistant_cma_writer_cutover_tests.rs");
        }
    }

    #[test]
    fn lifecycle_pre_admission_refusals_confirm_no_resource() {
        for (code, kind) in [
            ("invalid_request", "agent-manager-invalid-request"),
            ("desktop_unsupported", "agent-manager-desktop-unsupported"),
        ] {
            let problem = serde_json::json!({
                "type": format!("https://spec.chrono-ai.fun/cma/1.0/problems/{kind}"),
                "status": 422,
                "code": code,
                "detail": "invalid Agent lifecycle request: desktop creation requires initial_message",
            });
            assert!(creation_refusal_confirms_no_resource(
                "/api/v1/agents",
                422,
                &problem
            ));
            for path in [
                "/api/v1/sandboxes",
                "/api/v1/workspaces/wks_1/agents",
                "/api/v1/agent-lifecycle/op_1",
            ] {
                assert!(!creation_refusal_confirms_no_resource(path, 422, &problem));
            }
            for status in [200, 202, 400, 401, 403, 404, 409, 500, 503] {
                assert!(!creation_refusal_confirms_no_resource(
                    "/api/v1/agents",
                    status,
                    &problem
                ));
            }
            for (field, replacement) in [
                (
                    "type",
                    serde_json::json!("https://other.test/agent-manager-invalid-request"),
                ),
                ("status", serde_json::json!(409)),
                ("code", serde_json::json!("connection_required")),
            ] {
                let mut unrelated = problem.clone();
                unrelated[field] = replacement;
                assert!(!creation_refusal_confirms_no_resource(
                    "/api/v1/agents",
                    422,
                    &unrelated
                ));
            }
            for field in ["id", "operation_id", "agent_id", "sandbox_id"] {
                let mut conflicting = problem.clone();
                conflicting[field] = serde_json::json!("existing-resource");
                assert!(!creation_refusal_confirms_no_resource(
                    "/api/v1/agents",
                    422,
                    &conflicting
                ));
            }
        }
    }

    #[test]
    fn unsupported_inline_permission_refusal_confirms_no_resource() {
        for mode in ["ask", "default", "approval_required"] {
            let problem = serde_json::json!({
                "type":"https://spec.chrono-ai.fun/cma/1.0/problems/agent-manager-invalid-request",
                "status":422,
                "code":"invalid_request",
                "detail":format!("invalid Agent lifecycle request: unsupported permission_mode {mode}"),
            });
            assert!(creation_refusal_confirms_no_resource(
                "/api/v1/agents",
                422,
                &problem
            ));
            for path in ["/api/v1/sandboxes", "/api/v1/agent-lifecycle/op_1"] {
                assert!(!creation_refusal_confirms_no_resource(path, 422, &problem));
            }
            for status in [400, 403, 409, 500] {
                assert!(!creation_refusal_confirms_no_resource(
                    "/api/v1/agents",
                    status,
                    &problem
                ));
            }
            for field in ["id", "operation_id", "agent_id", "sandbox_id"] {
                let mut with_receipt = problem.clone();
                with_receipt[field] = serde_json::json!("existing-resource");
                assert!(!creation_refusal_confirms_no_resource(
                    "/api/v1/agents",
                    422,
                    &with_receipt
                ));
            }
            for (field, value) in [
                (
                    "type",
                    serde_json::json!("https://other.test/agent-manager-invalid-request"),
                ),
                ("code", serde_json::json!("permission_denied")),
                ("status", serde_json::json!(403)),
                (
                    "detail",
                    serde_json::json!(
                        "invalid Agent lifecycle request: unsupported permission_mode "
                    ),
                ),
                (
                    "detail",
                    serde_json::json!("permission_mode rejected after admission"),
                ),
            ] {
                let mut unrelated = problem.clone();
                unrelated[field] = value;
                assert!(!creation_refusal_confirms_no_resource(
                    "/api/v1/agents",
                    422,
                    &unrelated
                ));
            }
        }
    }

    #[test]
    fn unclassified_creation_failures_never_confirm_no_resource() {
        for problem in [
            serde_json::json!({"code":"invalid_request"}),
            serde_json::json!({
                "code":"invalid_request", "status":422,
                "type":"https://spec.chrono-ai.fun/cma/1.0/problems/agent-manager-invalid-request",
                "detail":"Validation changed after an earlier admission",
            }),
            serde_json::json!({
                "code":"permission_denied", "status":403,
                "type":"https://spec.chrono-ai.fun/cma/1.0/problems/agent-manager-forbidden",
            }),
            serde_json::json!({"code":"temporarily_unavailable","status":503}),
            serde_json::json!({"code":"not_authenticated","status":401}),
            serde_json::json!({"status":"failed","id":"operation_1"}),
            serde_json::json!({"items":[]}),
            serde_json::Value::Null,
        ] {
            for status in [200, 403, 404, 409, 422, 500, 503] {
                assert!(!creation_refusal_confirms_no_resource(
                    "/api/v1/agents",
                    status,
                    &problem
                ));
            }
        }
        assert!(creation_refusal_confirms_no_resource(
            "/api/v1/sandboxes",
            409,
            &serde_json::json!({"code":"sandbox_credential_setup_required"}),
        ));
    }

    fn creation() -> AssistantCmaCreation {
        AssistantCmaCreation {
            id: "creation".into(),
            user_id: "owner".into(),
            user_service_id: "user-service".into(),
            execution_base_url: "https://cma.example/".into(),
            cma_service_id: "service".into(),
            method: "POST".into(),
            path: "/api/v1/agents".into(),
            idempotency_key: "original-key".into(),
            request_hash: "original-hash".into(),
            sandbox_ref_id: "slot".into(),
            sandbox_generation: 1,
            creates_sandbox: true,
            expected_workspace_id: None,
            submission_state: "pending".into(),
            operation_id: None,
            cma_agent_id: None,
            cma_sandbox_id: None,
            revision: 0,
            submission_generation: 0,
            recovery_checked_at: None,
            first_dispatched_at: None,
            request_ciphertext: Some(vec![1, 2, 3]),
            request_context_version: Some(3),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
    fn parent() -> AssistantCmaSandbox {
        bson::from_document(doc! {"_id":"slot","user_id":"owner","user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id":"service",
            "label":"Sandbox","occupies_account_slot":true,"creation_ref":"original-creation",
            "generation":1_i64,"source":"nyxbot","idempotency_key":"original-key",
            "request_hash":"original-hash","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap()
    }
    #[test]
    fn a_saved_receipt_cannot_be_erased_by_late_dispatch_or_refusal() {
        let mut row = creation();
        let mut parent = parent();
        apply_creation_change(&mut row, &mut parent, &CreationChange::Dispatch).unwrap();
        apply_creation_change(
            &mut row,
            &mut parent,
            &CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
        )
        .unwrap();
        apply_creation_change(&mut row, &mut parent, &CreationChange::Dispatch).unwrap();
        assert_eq!(row.submission_state, "submitted");
        assert_eq!(row.operation_id.as_deref(), Some("op_1"));
        assert!(apply_creation_change(&mut row, &mut parent, &CreationChange::NoResource).is_err());
        assert!(parent.occupies_account_slot);
        assert!(row.request_ciphertext.is_some());
        assert!(
            apply_creation_change(
                &mut row,
                &mut parent,
                &CreationChange::Receipt(CreationReceipt::Lifecycle("op_other".into()))
            )
            .is_err()
        );
    }
    #[test]
    fn generic_allocation_replay_stops_before_cma_forgets_the_original_key() {
        let now = Utc::now();
        for path in ["/api/v1/sandboxes", "/api/v1/workspaces/workspace_1/agents"] {
            let mut row = creation();
            row.path = path.into();
            row.submission_state = "unknown".into();
            assert!(require_replay_window(&row, now).is_err());
            row.first_dispatched_at = Some(now - chrono::Duration::hours(22));
            assert!(require_replay_window(&row, now).is_ok());
            row.first_dispatched_at = Some(now - chrono::Duration::hours(23));
            assert!(require_replay_window(&row, now).is_err());
            row.first_dispatched_at = Some(now - chrono::Duration::hours(48));
            assert!(require_replay_window(&row, now).is_err());
            row.first_dispatched_at = None;
            row.submission_state = "pending".into();
            assert!(require_replay_window(&row, now).is_ok());
        }
        let mut permanent = creation();
        permanent.submission_state = "unknown".into();
        assert!(require_replay_window(&permanent, now).is_ok());
    }
    #[test]
    fn known_direct_receipts_use_observation_and_first_dispatch_time_never_slides() {
        let mut row = creation();
        row.path = "/api/v1/sandboxes".into();
        let mut parent = parent();
        apply_creation_change(&mut row, &mut parent, &CreationChange::Dispatch).unwrap();
        let first = row.first_dispatched_at;
        apply_creation_change(&mut row, &mut parent, &CreationChange::Dispatch).unwrap();
        assert_eq!(row.first_dispatched_at, first);
        apply_creation_change(
            &mut row,
            &mut parent,
            &CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into())),
        )
        .unwrap();
        assert!(apply_creation_change(&mut row, &mut parent, &CreationChange::Dispatch).is_err());
        assert_eq!(row.idempotency_key, "original-key");
        assert_eq!(row.request_hash, "original-hash");
        let mut agent = creation();
        agent.path = "/api/v1/workspaces/workspace_1/agents".into();
        agent.cma_agent_id = Some("agent_1".into());
        assert!(require_replay_window(&agent, Utc::now()).is_err());
    }
    #[test]
    fn exact_replay_is_bound_to_owner_deployment_parent_and_original_body() {
        let row = creation();
        let mut candidate = row.clone();
        candidate.sandbox_generation = 2;
        assert!(same_request(&row, &candidate).is_err());
        let mut no_resource = row.clone();
        no_resource.submission_state = "confirmed_not_created".into();
        assert!(same_request(&no_resource, &candidate).is_ok());
        for change in 0..8 {
            let mut candidate = row.clone();
            match change {
                0 => candidate.user_id = "other".into(),
                1 => candidate.cma_service_id = "alias-to-another-deployment".into(),
                2 => candidate.execution_base_url = "https://changed.example/".into(),
                3 => candidate.request_hash = "reformatted-body".into(),
                4 => candidate.sandbox_ref_id = "other-slot".into(),
                5 => candidate.path = "/api/v1/sandboxes".into(),
                6 => candidate.expected_workspace_id = Some("other-workspace".into()),
                _ => candidate.user_service_id = "other-connection".into(),
            }
            assert!(same_request(&row, &candidate).is_err());
        }
    }
    #[test]
    fn recovery_never_reopens_retirement_or_substitutes_a_generation() {
        let mut row = creation();
        let mut parent = parent();
        parent.retire_requested_at = Some(Utc::now());
        parent.admission_state = SandboxAdmissionState::Retiring;
        assert!(apply_creation_change(&mut row, &mut parent, &CreationChange::Dispatch).is_err());
        apply_creation_change(
            &mut row,
            &mut parent,
            &CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
        )
        .unwrap();
        assert_eq!(parent.admission_state, SandboxAdmissionState::Retiring);
        parent.generation = 2;
        assert!(
            apply_creation_change(
                &mut row,
                &mut parent,
                &CreationChange::Confirm {
                    sandbox_id: "sandbox_1".into(),
                    agent_id: Some("agent_1".into())
                }
            )
            .is_err()
        );
    }
    #[test]
    fn completion_is_monotonic_and_receipt_kinds_follow_the_actual_endpoint() {
        let value = serde_json::json!({"id":"result_1"});
        assert_eq!(
            receipt_for(&Operation::CreateSandbox, &value).unwrap(),
            CreationReceipt::Sandbox("result_1".into())
        );
        assert_eq!(
            receipt_for(
                &Operation::WorkspaceAgent {
                    id: "workspace_1".into()
                },
                &value
            )
            .unwrap(),
            CreationReceipt::Agent("result_1".into())
        );
        assert_eq!(
            receipt_for(
                &Operation::CreateAgent {
                    sandbox: None,
                    workspace: None
                },
                &value
            )
            .unwrap(),
            CreationReceipt::Lifecycle("result_1".into())
        );
        let mut row = creation();
        let mut parent = parent();
        row.operation_id = Some("op_1".into());
        parent.cma_sandbox_id = Some("sandbox_1".into());
        apply_creation_change(
            &mut row,
            &mut parent,
            &CreationChange::Confirm {
                sandbox_id: "sandbox_1".into(),
                agent_id: Some("agent_1".into()),
            },
        )
        .unwrap();
        apply_creation_change(
            &mut row,
            &mut parent,
            &CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
        )
        .unwrap();
        assert_eq!(row.submission_state, "confirmed");
        assert!(row.request_ciphertext.is_none());
        assert!(apply_creation_change(&mut row, &mut parent, &CreationChange::Dispatch).is_err());
    }
    #[test]
    fn only_an_exact_lifecycle_identity_can_supply_an_agent() {
        assert_eq!(
            lifecycle_agent("op_1", &serde_json::json!({"id":"op_1","status":"failed"})).unwrap(),
            None
        );
        assert_eq!(
            lifecycle_agent(
                "op_1",
                &serde_json::json!({"id":"op_1","agent_id":"agent_1"})
            )
            .unwrap(),
            Some("agent_1")
        );
        for value in [
            serde_json::json!({"id":"op_other","agent_id":"agent_1"}),
            serde_json::json!({"agent_id":"agent_1"}),
            serde_json::json!({"id":"op_1","agent_id":true}),
        ] {
            assert!(lifecycle_agent("op_1", &value).is_err());
        }
    }

    async fn test_db() -> Option<Database> {
        let uri = std::env::var("NYXID_TEST_DATABASE_URL")
            .unwrap_or_else(|_| "mongodb://127.0.0.1:27017/?directConnection=true".into());
        let mut options = mongodb::options::ClientOptions::parse(uri).await.ok()?;
        options.server_selection_timeout = Some(Duration::from_millis(500));
        let client = mongodb::Client::with_options(options).ok()?;
        let Ok(hello) = client.database("admin").run_command(doc! {"hello":1}).await else {
            eprintln!("skipping CMA creation transaction test: MongoDB unavailable");
            return None;
        };
        if !hello.contains_key("setName") && hello.get_str("msg") != Ok("isdbgrid") {
            eprintln!("skipping CMA creation transaction test: replica-set MongoDB required");
            return None;
        }
        Some(client.database(&format!(
            "nyxid_test_cma_creation_{}",
            uuid::Uuid::new_v4().simple()
        )))
    }
    async fn seeded(db: &Database) -> AssistantCmaCreation {
        db.collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
            .insert_one(parent())
            .await
            .unwrap();
        db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .create_indexes(indexes())
            .await
            .unwrap();
        store_creation(db, &creation()).await.unwrap()
    }
    #[tokio::test]
    async fn erased_owner_and_stale_generation_cannot_write_creation_results() {
        let Some(db) = test_db().await else {
            return;
        };
        let row = seeded(&db).await;
        db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
            .update_one(doc! {"_id":"slot"}, doc! {"$set":{"generation":2_i64}})
            .await
            .unwrap();
        assert!(
            change_creation(&db, &row, CreationChange::Dispatch)
                .await
                .is_err()
        );
        let unchanged = db
            .collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.submission_state, "pending");
        db.collection::<bson::Document>(super::super::erasure_admission::COLLECTION)
            .update_one(
                doc! {"_id":"owner"},
                doc! {"$set":{"erasure_accepted":true}},
            )
            .await
            .unwrap();
        db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .delete_many(doc! {})
            .await
            .unwrap();
        assert!(store_creation(&db, &row).await.is_err());
        assert_eq!(
            db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn no_resource_confirmation_and_slot_release_commit_together() {
        let Some(db) = test_db().await else {
            return;
        };
        let row = seeded(&db).await;
        change_creation(&db, &row, CreationChange::Dispatch)
            .await
            .unwrap();
        let confirmed = change_creation(&db, &row, CreationChange::NoResource)
            .await
            .unwrap();
        let parent = sandboxes::get_owned(&db, "owner", "slot").await.unwrap();
        assert_eq!(confirmed.submission_state, "confirmed_not_created");
        assert_eq!(parent.submission_state, "confirmed_not_created");
        assert!(!parent.occupies_account_slot);
        assert!(confirmed.request_ciphertext.is_none());
        assert!(
            change_creation(
                &db,
                &row,
                CreationChange::Receipt(CreationReceipt::Lifecycle("late_op".into()))
            )
            .await
            .is_err()
        );
        db.drop().await.unwrap();
    }
    struct RecoveryGateway {
        ready: bool,
        reads: std::sync::atomic::AtomicUsize,
    }
    impl RecoveryGateway {
        fn sandbox() -> super::super::assistant_cma_gateway::CmaSandbox {
            super::super::assistant_cma_gateway::CmaSandbox {
                id: "sandbox_1".into(),
                owner_id: Some("owner".into()),
                state: SandboxObservationState::Ready,
                resource_state: Some("active".into()),
                name: None,
                machine_included: Some(false),
                cpu_millis: None,
                memory_mib: None,
                capacity_source: None,
                last_activity_at: None,
            }
        }
    }
    use super::super::assistant_cma_gateway::{
        CmaAgent, CmaDesktopDescriptor, CmaEventSnapshot, CmaGatewayError, CmaLifecycleResult,
        CmaResponse, CmaResult, CmaSandbox, CmaTranscript,
    };
    #[async_trait::async_trait]
    impl CmaGateway for RecoveryGateway {
        async fn service_id(&self, _: &str) -> CmaResult<String> {
            Ok("service".into())
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

        async fn get_lifecycle(&self, owner: &str, id: &str) -> CmaResult<Value> {
            assert_eq!(owner, "owner");
            self.reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(if self.ready {
                serde_json::json!({"id":id,"status":"ready","agent_id":"agent_1"})
            } else {
                serde_json::json!({"id":id,"status":"failed"})
            })
        }
        async fn get_sandbox(&self, owner: &str, id: &str) -> CmaResult<CmaSandbox> {
            assert_eq!(owner, "owner");
            assert_eq!(id, "sandbox_1");
            Ok(Self::sandbox())
        }
        async fn get_agent(&self, owner: &str, id: &str) -> CmaResult<CmaAgent> {
            assert_eq!(owner, "owner");
            assert_eq!(id, "agent_1");
            Ok(CmaAgent {
                id: id.into(),
                owner_id: Some(owner.into()),
                sandbox_id: Some("sandbox_1".into()),
                workspace_id: Some("workspace_1".into()),
                worktree_id: Some("worktree_1".into()),
                name: Some("Recovered Agent".into()),
                status: "ready".into(),
                phase: None,
                revision: None,
                profile: None,
                current_response_id: None,
                desktop: Some(false),
                sandbox_status: Some("active".into()),
                callback_url: None,
            })
        }
        async fn pause_sandbox(&self, _: &str, _: &str, _: &str) -> CmaResult<CmaLifecycleResult> {
            panic!("recovery must never mutate CMA")
        }
        async fn revive_sandbox(&self, _: &str, _: &str, _: &str) -> CmaResult<CmaLifecycleResult> {
            panic!("recovery must never mutate CMA")
        }
        async fn retire_sandbox(&self, _: &str, _: &str, _: &str) -> CmaResult<CmaLifecycleResult> {
            panic!("recovery must never mutate CMA")
        }
        async fn list_sandbox_events(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> CmaResult<CmaEventSnapshot> {
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
    #[tokio::test]
    async fn sandbox_refresh_recovers_a_known_operation_without_a_chat_or_watch() {
        let Some(db) = test_db().await else {
            return;
        };
        let row = seeded(&db).await;
        change_creation(&db, &row, CreationChange::Dispatch)
            .await
            .unwrap();
        change_creation(
            &db,
            &row,
            CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
        )
        .await
        .unwrap();
        let gateway = RecoveryGateway {
            ready: true,
            reads: std::sync::atomic::AtomicUsize::new(0),
        };
        let list = sandboxes::list_live(&db, &gateway, "owner", None)
            .await
            .unwrap();
        assert_eq!(list.sandboxes.len(), 1);
        assert_eq!(list.sandboxes[0].id, "slot");
        let recovered = db
            .collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.submission_state, "confirmed");
        assert_eq!(recovered.cma_agent_id.as_deref(), Some("agent_1"));
        assert!(recovered.request_ciphertext.is_none());
        assert_eq!(
            db.collection::<bson::Document>(crate::models::assistant_cma_agent::COLLECTION_NAME)
                .count_documents(doc! {"user_id":"owner","cma_agent_id":"agent_1"})
                .await
                .unwrap(),
            1
        );
        assert_eq!(gateway.reads.load(std::sync::atomic::Ordering::Relaxed), 1);
        recover_owner(&db, &gateway, "owner").await.unwrap();
        assert_eq!(gateway.reads.load(std::sync::atomic::Ordering::Relaxed), 1);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn recovery_is_bounded_and_failure_never_proves_no_resource() {
        let Some(db) = test_db().await else {
            return;
        };
        let base = seeded(&db).await;
        for index in 0..10 {
            let mut row = base.clone();
            row.id = format!("creation_{index}");
            row.idempotency_key = format!("original-key-{index}");
            row.operation_id = Some(format!("op_{index}"));
            row.submission_state = "submitted".into();
            db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .insert_one(row)
                .await
                .unwrap();
        }
        let gateway = RecoveryGateway {
            ready: false,
            reads: std::sync::atomic::AtomicUsize::new(0),
        };
        recover_owner(&db, &gateway, "owner").await.unwrap();
        let count = gateway.reads.load(std::sync::atomic::Ordering::Relaxed);
        assert!(count > 0 && count <= MAX_RECOVERY_OBSERVATIONS as usize);
        assert!(
            sandboxes::get_owned(&db, "owner", "slot")
                .await
                .unwrap()
                .occupies_account_slot
        );
        let rows: Vec<AssistantCmaCreation> = db
            .collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(rows.iter().all(|row| row.request_ciphertext.is_some()
            && row.submission_state != "confirmed_not_created"));
        assert_eq!(
            rows.iter()
                .find(|row| row.id == base.id)
                .unwrap()
                .recovery_checked_at,
            None
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn reacquired_parent_local_creation_rejects_old_submission_receipts() {
        let Some(db) = test_db().await else {
            return;
        };
        let mut parent = parent();
        parent.cma_sandbox_id = Some("sandbox_1".into());
        parent.admission_state = SandboxAdmissionState::Associated;
        db.collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
            .insert_one(parent)
            .await
            .unwrap();
        db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .create_indexes(indexes())
            .await
            .unwrap();
        let mut candidate = creation();
        candidate.creates_sandbox = false;
        candidate.cma_sandbox_id = Some("sandbox_1".into());
        let first = store_creation(&db, &candidate).await.unwrap();
        change_creation(&db, &first, CreationChange::NoResource)
            .await
            .unwrap();
        let next = store_creation(&db, &candidate).await.unwrap();
        assert_eq!(next.sandbox_generation, first.sandbox_generation);
        assert_eq!(next.submission_generation, first.submission_generation + 1);
        assert!(
            change_creation(
                &db,
                &first,
                CreationChange::Receipt(CreationReceipt::Lifecycle("old_op".into()))
            )
            .await
            .is_err()
        );
        let received = change_creation(
            &db,
            &next,
            CreationChange::Receipt(CreationReceipt::Lifecycle("new_op".into())),
        )
        .await
        .unwrap();
        assert_eq!(received.operation_id.as_deref(), Some("new_op"));
        db.drop().await.unwrap();
    }
    #[test]
    fn every_allocation_surface_is_classified_or_refused() {
        assert!(matches!(
            classify("POST", "/api/v1/agents", br#"{"name":"a"}"#).unwrap(),
            Operation::CreateAgent {
                sandbox: None,
                workspace: None
            }
        ));
        assert!(matches!(
            classify("POST", "/api/v1/sandboxes", b"{}").unwrap(),
            Operation::CreateSandbox
        ));
        for path in [
            "/api/v1/triggers/public/a/launches",
            "/api/v1/launches/a/start",
            "/api/v1/unknown",
        ] {
            assert!(classify("POST", path, b"{}").is_err());
        }
        assert!(matches!(
            classify("POST", "/api/v1/workspaces/wks_1/agents", b"{}").unwrap(),
            Operation::WorkspaceAgent { .. }
        ));
    }
    #[test]
    fn parent_choice_and_response_identity_cannot_be_rewritten() {
        for body in [
            br#"{"sandbox_id":"s","workspace_id":"w"}"#.as_slice(),
            br#"{"sandbox_id":"s","machine_included":false}"#,
        ] {
            assert!(classify("POST", "/api/v1/agents", body).is_err());
        }
        assert!(
            classify(
                "POST",
                "/api/v2/agents/agt_1/responses",
                br#"{"input":"hello","stream":true,"conversation":"agt_other"}"#
            )
            .is_err()
        );
        assert!(
            classify(
                "POST",
                "/api/v2/agents/agt_1/responses",
                br#"{"input":"hello","stream":true,"conversation":"agt_1"}"#
            )
            .is_ok()
        );
    }
    #[test]
    fn control_commands_bind_required_data_to_the_exact_turn() {
        assert!(validate_control(br#"{"turn_sequence":1,"expected_agent_revision":2,"command":{"type":"resolve_approval","approval_id":"apr_a","decision":"allow"}}"#,false).is_ok());
        for body in [br#"{"turn_sequence":1,"expected_agent_revision":2,"command":{"type":"steer"}}"#.as_slice(),br#"{"turn_sequence":1,"expected_agent_revision":2,"command":{"type":"cancel","approval_id":"a"}}"#,br#"{"turn_sequence":0,"expected_agent_revision":2,"command":{"type":"cancel"}}"#] { assert!(validate_control(body,false).is_err()); }
    }
    #[test]
    fn raw_body_is_bound_including_whitespace() {
        assert_ne!(wire_hash(b"{\"x\":1}"), wire_hash(b"{ \"x\": 1 }"));
    }
}
