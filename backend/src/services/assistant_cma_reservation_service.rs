//! Metadata-only administration of held CMA account-slot reservations.
use super::{
    assistant_cma_admission::reservation_admin as admission,
    assistant_cma_gateway::{
        CatalogCmaGateway, CmaGateway, CmaGatewayError, CmaSandbox, validate_remote_id,
    },
    assistant_cma_sandbox_service as sandboxes,
    audit_service::AuditActor,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_cma_creation::AssistantCmaCreation,
        assistant_cma_sandbox::{
            AssistantCmaSandbox, COLLECTION_NAME, SandboxAdmissionState, SandboxObservationState,
        },
    },
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const MAX_PAGE_SIZE: i64 = 100;

#[derive(Clone, Debug, Serialize)]
pub struct ReservationRow {
    pub id: String,
    pub owner_id: String,
    pub cma_service_id: String,
    pub source: String,
    pub admission_state: SandboxAdmissionState,
    pub submission_state: String,
    pub observation_state: SandboxObservationState,
    pub generation: i64,
    pub revision: i64,
    pub occupies_account_slot: bool,
    pub remote_sandbox_id: Option<String>,
    pub associated: bool,
    pub lifecycle_operation_id: Option<String>,
    pub creation_revision: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_observed: Option<DateTime<Utc>>,
    pub creation_updated_at: Option<DateTime<Utc>>,
    pub recovery_checked_at: Option<DateTime<Utc>>,
    pub holding: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InspectionOutcome {
    Exists {
        remote_sandbox_id: String,
        state: SandboxObservationState,
    },
    ProvenAbsent {
        evidence: &'static str,
    },
    Unknown {
        reason: &'static str,
    },
}
#[derive(Clone, Debug, Serialize)]
pub struct Inspection {
    pub reservation: ReservationRow,
    pub outcome: InspectionOutcome,
    pub next_step: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolveAction {
    Associate,
    Release,
    ConfirmRetirement,
}

impl ResolveAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Associate => "associate",
            Self::Release => "release",
            Self::ConfirmRetirement => "confirm_retirement",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    pub generation: i64,
    pub revision: i64,
    pub creation_revision: Option<i64>,
    pub action: ResolveAction,
    pub remote_sandbox_id: Option<String>,
}

fn drift() -> AppError {
    AppError::Conflict("CMA reservation changed; inspect it again before resolving".into())
}
fn valid_cursor(value: &str) -> AppResult<()> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(AppError::ValidationError(
            "Invalid reservation cursor or owner".into(),
        ));
    }
    Ok(())
}
fn holding(row: &AssistantCmaSandbox, creation: Option<&AssistantCmaCreation>) -> &'static str {
    if !row.occupies_account_slot {
        "released"
    } else if row.retire_requested_at.is_some()
        || row.admission_state == SandboxAdmissionState::Retiring
    {
        "retiring"
    } else if row.cma_sandbox_id.is_some() || creation.is_some_and(|c| c.cma_sandbox_id.is_some()) {
        "live_sandbox"
    } else if creation.map_or(row.submission_state.as_str(), |c| {
        c.submission_state.as_str()
    }) == "unknown"
    {
        "creation_unknown"
    } else {
        "creation_in_progress"
    }
}
fn project(row: &AssistantCmaSandbox, creation: Option<&AssistantCmaCreation>) -> ReservationRow {
    ReservationRow {
        id: row.id.clone(),
        owner_id: row.user_id.clone(),
        cma_service_id: row.cma_service_id.clone(),
        source: row.source.clone(),
        admission_state: row.admission_state,
        submission_state: creation
            .map(|c| c.submission_state.clone())
            .unwrap_or_else(|| row.submission_state.clone()),
        observation_state: row.observation_state,
        generation: row.generation,
        revision: row.agent_reference_version,
        occupies_account_slot: row.occupies_account_slot,
        remote_sandbox_id: row
            .cma_sandbox_id
            .clone()
            .or_else(|| creation.and_then(|c| c.cma_sandbox_id.clone())),
        associated: row.cma_sandbox_id.is_some(),
        lifecycle_operation_id: creation
            .and_then(|c| c.operation_id.clone())
            .or_else(|| row.operation_id.clone()),
        creation_revision: creation.map(|c| c.revision),
        created_at: row.created_at,
        updated_at: row.updated_at,
        last_observed: row.last_observed,
        creation_updated_at: creation.map(|c| c.updated_at),
        recovery_checked_at: creation.and_then(|c| c.recovery_checked_at),
        holding: holding(row, creation),
    }
}

pub async fn get(db: &Database, id: &str) -> AppResult<AssistantCmaSandbox> {
    valid_cursor(id)?;
    db.collection::<AssistantCmaSandbox>(COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(|| AppError::NotFound("CMA reservation not found".into()))
}

pub async fn list(
    db: &Database,
    owner_id: Option<&str>,
    states: Option<&[String]>,
    after: Option<&str>,
    limit: i64,
) -> AppResult<Vec<ReservationRow>> {
    let mut filter = doc! {"occupies_account_slot":true};
    if let Some(owner) = owner_id {
        valid_cursor(owner)?;
        filter.insert("user_id", owner);
    }
    if let Some(after) = after {
        valid_cursor(after)?;
        filter.insert("_id", doc! {"$gt":after});
    }
    if let Some(states) = states {
        if states.is_empty()
            || states.len() > 6
            || states.iter().any(|state| {
                !matches!(
                    state.as_str(),
                    "reserved" | "creating" | "associated" | "retiring" | "unknown" | "retired"
                )
            })
        {
            return Err(AppError::ValidationError(
                "Invalid reservation states".into(),
            ));
        }
        filter.insert("admission_state", doc! {"$in":states});
    }
    let rows: Vec<AssistantCmaSandbox> = db
        .collection(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(limit.clamp(1, MAX_PAGE_SIZE))
        .await?
        .try_collect()
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let creation = admission::creation_for_sandbox(db, &row).await?;
        result.push(project(&row, creation.as_ref()));
    }
    Ok(result)
}
fn unknown(reason: &'static str) -> InspectionOutcome {
    InspectionOutcome::Unknown { reason }
}
fn unknown_reason(error: CmaGatewayError) -> InspectionOutcome {
    unknown(match error {
        CmaGatewayError::NotFound => "resource_not_found_without_creation_proof",
        CmaGatewayError::Timeout => "gateway_timeout",
        CmaGatewayError::Unavailable => "gateway_unavailable",
        CmaGatewayError::Forbidden => "owner_mismatch",
        CmaGatewayError::ConfigurationRequired => "cma_configuration_changed",
        CmaGatewayError::InvalidResponse => "invalid_gateway_observation",
        CmaGatewayError::UpstreamAuthorizationUnavailable => "gateway_authorization_unavailable",
        CmaGatewayError::CredentialSetupRequired { .. } => "credential_setup_required",
        _ => "observation_unknown",
    })
}
pub(super) async fn verify(
    gateway: &dyn CmaGateway,
    parent: &AssistantCmaSandbox,
    creation: Option<&AssistantCmaCreation>,
) -> Result<(), CmaGatewayError> {
    if let Some(creation) = creation {
        gateway
            .verify_creation_deployment(
                &parent.user_id,
                &parent.cma_service_id,
                &creation.deployment_fingerprint,
            )
            .await
    } else if gateway.service_id(&parent.user_id).await? == parent.cma_service_id {
        Ok(())
    } else {
        Err(CmaGatewayError::ConfigurationRequired)
    }
}
pub(super) async fn exact_sandbox(
    gateway: &dyn CmaGateway,
    owner: &str,
    id: &str,
) -> Result<CmaSandbox, CmaGatewayError> {
    validate_remote_id(id)?;
    let remote = gateway.get_sandbox_authority(owner, id).await?;
    if remote.id != id
        || remote
            .owner_id
            .as_deref()
            .is_some_and(|value| value != owner)
    {
        return Err(CmaGatewayError::Forbidden);
    }
    Ok(remote)
}

fn has_non_creation_proof(creation: &AssistantCmaCreation) -> bool {
    !creation.released_by_inventory
        && creation.submission_state == "confirmed_not_created"
        && (creation.covered_key.is_some() || creation.operation_id.is_none())
        && creation.cma_agent_id.is_none()
        && creation.cma_sandbox_id.is_none()
}

/// Resolve only saved identities. This function has no create/replay/mutation call.
async fn observe(
    gateway: &dyn CmaGateway,
    parent: &AssistantCmaSandbox,
    creation: Option<&AssistantCmaCreation>,
) -> (InspectionOutcome, Option<CmaSandbox>) {
    if let Err(error) = verify(gateway, parent, creation).await {
        return (unknown_reason(error), None);
    }
    if creation.is_some_and(|c| c.released_by_inventory) {
        return (unknown("released_after_inventory_check"), None);
    }
    if parent.cma_sandbox_id.is_none() && creation.is_some_and(has_non_creation_proof) {
        return (
            InspectionOutcome::ProvenAbsent {
                evidence: "confirmed_not_created",
            },
            None,
        );
    }
    if let Some(creation) = creation.filter(|row| row.covered_key.is_some()) {
        use super::assistant_cma_admission::creation_outcome::{self, Evidence};
        match creation_outcome::lookup(gateway, creation).await {
            Ok(Evidence::Absent { .. }) if parent.cma_sandbox_id.is_none() => {
                return (
                    InspectionOutcome::ProvenAbsent {
                        evidence: "covered_creation_outcome",
                    },
                    None,
                );
            }
            Ok(Evidence::Created { sandbox, .. }) => {
                return match exact_sandbox(gateway, &parent.user_id, &sandbox).await {
                    Ok(remote)
                        if remote.owner_id.as_deref() == Some(parent.user_id.as_str())
                            && parent
                                .cma_sandbox_id
                                .as_deref()
                                .is_none_or(|id| id == sandbox) =>
                    {
                        (
                            InspectionOutcome::Exists {
                                remote_sandbox_id: remote.id.clone(),
                                state: remote.state,
                            },
                            Some(remote),
                        )
                    }
                    Ok(_) => (unknown("owner_mismatch"), None),
                    Err(error) => (unknown_reason(error), None),
                };
            }
            Ok(Evidence::Progress { operation: None }) if creation.operation_id.is_none() => {
                return (unknown("covered_creation_in_progress"), None);
            }
            Ok(Evidence::Unknown) => return (unknown("covered_creation_outcome_unknown"), None),
            Err(error) => return (unknown_reason(error), None),
            _ => {}
        }
    }
    let mut operation_reason = None;
    let result = async {
        if let Some(id) = parent
            .cma_sandbox_id
            .as_deref()
            .or_else(|| creation.and_then(|c| c.cma_sandbox_id.as_deref()))
        {
            return exact_sandbox(gateway, &parent.user_id, id).await.map(Some);
        }
        let mut agent = creation.and_then(|c| c.cma_agent_id.clone());
        if let Some(operation) = creation
            .and_then(|c| c.operation_id.as_deref())
            .or(parent.operation_id.as_deref())
        {
            validate_remote_id(operation)?;
            let value = gateway.get_lifecycle(&parent.user_id, operation).await?;
            if value.get("id").and_then(serde_json::Value::as_str) != Some(operation) {
                return Err(CmaGatewayError::InvalidResponse);
            }
            operation_reason = Some(operation_without_resource(&value));
            if let Some(observed) = value.get("agent_id").filter(|v| !v.is_null()) {
                let observed = observed.as_str().ok_or(CmaGatewayError::InvalidResponse)?;
                validate_remote_id(observed)?;
                if agent.as_deref().is_some_and(|id| id != observed) {
                    return Err(CmaGatewayError::InvalidResponse);
                }
                agent = Some(observed.into());
            }
        }
        if let Some(id) = agent {
            validate_remote_id(&id)?;
            let agent = gateway.get_agent(&parent.user_id, &id).await?;
            if agent.id != id
                || agent
                    .owner_id
                    .as_deref()
                    .is_some_and(|owner| owner != parent.user_id)
                || creation
                    .and_then(|c| c.expected_workspace_id.as_deref())
                    .is_some_and(|workspace| agent.workspace_id.as_deref() != Some(workspace))
            {
                return Err(CmaGatewayError::Forbidden);
            }
            if let Some(id) = agent.sandbox_id {
                return exact_sandbox(gateway, &parent.user_id, &id).await.map(Some);
            }
        }
        Ok(None)
    }
    .await;
    match result {
        Ok(Some(remote)) => (
            InspectionOutcome::Exists {
                remote_sandbox_id: remote.id.clone(),
                state: remote.state,
            },
            Some(remote),
        ),
        Err(error) => (unknown_reason(error), None),
        Ok(None) => (
            unknown(match creation {
                Some(c) if c.operation_id.is_some() => {
                    operation_reason.unwrap_or("operation_without_resource")
                }
                Some(c) if c.submission_state == "pending" && c.first_dispatched_at.is_none() => {
                    "creation_not_dispatched"
                }
                Some(c) if c.path != "/api/v1/agents" && c.first_dispatched_at.is_none() => {
                    "creation_dispatch_time_unknown"
                }
                Some(c)
                    if c.path != "/api/v1/agents"
                        && c.first_dispatched_at.is_some_and(|at| {
                            Utc::now().signed_duration_since(at) >= chrono::Duration::hours(23)
                        }) =>
                {
                    "idempotency_window_passed"
                }
                Some(c)
                    if c.path == "/api/v1/sandboxes"
                        && c.submission_state == "unknown"
                        && c.first_dispatched_at.is_some() =>
                {
                    "owner_retry_will_recover"
                }
                Some(_) => "creation_identity_not_observable_without_replay",
                None => "creation_record_missing",
            }),
            None,
        ),
    }
}
/// Status alone never proves that a lifecycle created no resource. CMA may
/// retain an internal sandbox identity which its public operation omits.
pub(super) fn operation_without_resource(value: &serde_json::Value) -> &'static str {
    match value.get("status").and_then(serde_json::Value::as_str) {
        Some("queued" | "running" | "ready") => "operation_in_progress_without_resource",
        Some("waiting_for_connection") => "operation_waiting_for_connection",
        Some("failed") => "operation_failed_without_resource",
        _ => "operation_without_resource",
    }
}

fn audit_data(
    parent: &AssistantCmaSandbox,
    creation: Option<&AssistantCmaCreation>,
    outcome: &InspectionOutcome,
) -> serde_json::Value {
    json!({"reservation_id":parent.id,"owner_id":parent.user_id,"cma_service_id":parent.cma_service_id,
        "generation":parent.generation,"revision":parent.agent_reference_version,"holding":holding(parent,creation),"outcome":outcome})
}
pub async fn inspect(
    db: &Database,
    gateway: &dyn CmaGateway,
    id: &str,
    actor: &AuditActor,
) -> AppResult<Inspection> {
    let db = super::transaction_service::durable_database(db);
    let parent = get(&db, id).await?;
    let creation = admission::creation_for_sandbox(&db, &parent).await?;
    // Inspection never participates in owner write/CAS/cooldown state.
    if super::erasure_admission::is_erasure_accepted(&db, &parent.user_id).await? {
        return Err(AppError::Conflict(
            "Account erasure is in progress for this owner; inspection is unavailable".into(),
        ));
    }
    let (outcome, _) = observe(gateway, &parent, creation.as_ref()).await;
    super::audit_service::log_actor_event(
        db.clone(),
        actor,
        "admin_cma_sandbox_reservation_inspected",
        Some(audit_data(&parent, creation.as_ref(), &outcome)),
    )
    .await?;
    let next_step = match &outcome {
        InspectionOutcome::Unknown {
            reason: "owner_retry_will_recover",
        } => Some(
            "The owner's next allocating create or retry of the original request automatically attempts recovery inside the 23-hour window. If CMA remains unavailable or uncertain, the slot stays held.",
        ),
        InspectionOutcome::Unknown {
            reason: "operation_in_progress_without_resource",
        } => Some(
            "CMA is still creating the resource. NyxID automatically checks the saved operation; the account slot stays held until CMA provides an exact resource identity.",
        ),
        InspectionOutcome::Unknown {
            reason: "operation_waiting_for_connection",
        } => Some(
            "The owner may repair the connection and retry the original request, or explicitly cancel a covered operation. An older creation remains held until authoritative evidence is available.",
        ),
        InspectionOutcome::Unknown {
            reason: "operation_failed_without_resource" | "operation_without_resource",
        } => Some(
            "CMA has not provided a resource identity or proof of non-creation. NyxID keeps checking, but cannot release this slot from operation status alone; CMA must provide authoritative creation identity or absence evidence.",
        ),
        InspectionOutcome::Unknown {
            reason:
                "covered_creation_outcome_unknown"
                | "covered_creation_in_progress"
                | "creation_identity_not_observable_without_replay",
        } => Some(
            "NyxID retains the reservation while CMA creation evidence is uncertain. Recover the original request or confirm retirement of the exact associated Sandbox.",
        ),
        _ => None,
    };
    Ok(Inspection {
        reservation: project(&parent, creation.as_ref()),
        outcome,
        next_step,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryReleaseRequest {
    pub generation: i64,
    pub revision: i64,
    pub creation_revision: i64,
    pub reason: String,
}

pub(super) fn require_inventory_release_candidate(
    parent: &AssistantCmaSandbox,
    creation: &AssistantCmaCreation,
) -> AppResult<()> {
    if parent.source != sandboxes::NYXID_CREATED_SOURCE
        || !matches!(
            holding(parent, Some(creation)),
            "creation_unknown" | "creation_in_progress"
        )
        || !matches!(
            parent.admission_state,
            SandboxAdmissionState::Unknown
                | SandboxAdmissionState::Creating
                | SandboxAdmissionState::Reserved
        )
        || creation.covered_key.is_some()
        || parent.operation_id.is_some()
        || creation.operation_id.is_some()
        || creation.cma_agent_id.is_some()
        || !matches!(
            creation.submission_state.as_str(),
            "unknown" | "submitted" | "pending"
        )
        || creation.method != "POST"
        || creation.path != "/api/v1/sandboxes"
    {
        return Err(AppError::Conflict(
            "cma_inventory_release_ineligible".into(),
        ));
    }
    if !creation.first_dispatched_at.is_some_and(|first| {
        Utc::now().signed_duration_since(first)
            >= chrono::Duration::hours(
                super::assistant_cma_admission::GENERIC_CREATION_REPLAY_HOURS,
            )
    }) {
        return Err(AppError::Conflict("cma_replay_window_not_expired".into()));
    }
    Ok(())
}

async fn owner_inventory(
    gateway: &CatalogCmaGateway,
    owner: &str,
) -> Result<(usize, Vec<String>), CmaGatewayError> {
    let mut cursors = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut live = Vec::new();
    let mut cursor = None;
    for _ in 0..100 {
        let page = gateway
            .sandbox_inventory_page(owner, cursor.as_deref())
            .await?;
        for (id, state) in page.items {
            if !seen.insert(id.clone()) {
                return Err(CmaGatewayError::InventoryIncomplete);
            }
            if state != SandboxObservationState::Retired {
                live.push(id);
            }
        }
        let Some(next) = page.next_cursor else {
            return Ok((seen.len(), live));
        };
        if !cursors.insert(next.clone()) {
            return Err(CmaGatewayError::InventoryIncomplete);
        }
        cursor = Some(next);
    }
    Err(CmaGatewayError::InventoryIncomplete)
}

pub async fn release_empty_inventory(
    db: &Database,
    gateway: &CatalogCmaGateway,
    parent: &AssistantCmaSandbox,
    request: &InventoryReleaseRequest,
    actor: &AuditActor,
) -> AppResult<ReservationRow> {
    let reason = request.reason.trim();
    if reason.is_empty() || request.reason.len() > 512 || reason.chars().any(char::is_control) {
        return Err(AppError::ValidationError(
            "An audit reason of 1 to 512 bytes is required".into(),
        ));
    }
    sandboxes::require_generation(parent, request.generation)?;
    if parent.agent_reference_version != request.revision {
        return Err(drift());
    }
    let db = super::transaction_service::durable_database(db);
    let creation = admission::creation_for_sandbox(&db, parent)
        .await?
        .ok_or_else(drift)?;
    if creation.revision != request.creation_revision {
        return Err(drift());
    }
    require_inventory_release_candidate(parent, &creation)?;
    verify(gateway, parent, Some(&creation)).await?;
    let (checked, live) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        owner_inventory(gateway, &parent.user_id),
    )
    .await
    .map_err(|_| CmaGatewayError::InventoryIncomplete)?
    .map_err(|_| CmaGatewayError::InventoryIncomplete)?;
    let data = json!({
        "reservation_id": parent.id, "owner_id": parent.user_id, "cma_service_id": parent.cma_service_id,
        "generation": request.generation, "revision": request.revision, "creation_revision": request.creation_revision,
        "action": "release_empty_inventory", "result": "resolved", "reason": reason,
        "inventory_checked": checked, "inventory_live": live.len(), "unknown_live": 0
    });
    let (saved, creation) = admission::commit(
        &db,
        parent,
        Some(&creation),
        admission::Change::ReleaseEmptyInventory(live),
        actor,
        data,
    )
    .await?;
    Ok(project(&saved, creation.as_ref()))
}

pub async fn resolve(
    db: &Database,
    gateway: &dyn CmaGateway,
    id: &str,
    request: &ResolveRequest,
    actor: &AuditActor,
) -> AppResult<ReservationRow> {
    let db = super::transaction_service::durable_database(db);
    let parent = get(&db, id).await?;
    sandboxes::require_generation(&parent, request.generation)?;
    if parent.agent_reference_version != request.revision {
        return Err(drift());
    }
    // Legacy live rows may predate creation journaling. Their known binding
    // still requires the same clear refusal, not a missing-journal error.
    if request.action == ResolveAction::Release && parent.cma_sandbox_id.is_some() {
        return Err(AppError::Conflict("cma_sandbox_live: this account already has its NyxID sandbox; create agents in its workspace or ask the owner to retire it".into()));
    }
    if parent.source != sandboxes::NYXID_CREATED_SOURCE {
        return Err(drift());
    }
    if request.action == ResolveAction::ConfirmRetirement {
        let creation = admission::creation_for_sandbox(&db, &parent).await?;
        if creation.as_ref().map(|c| c.revision) != request.creation_revision {
            return Err(drift());
        }
        let id = parent.cma_sandbox_id.as_deref().ok_or_else(drift)?;
        if request
            .remote_sandbox_id
            .as_deref()
            .is_some_and(|named| named != id)
        {
            return Err(drift());
        }
        verify(gateway, &parent, creation.as_ref()).await?;
        let remote = exact_sandbox(gateway, &parent.user_id, id).await?;
        if remote.state != SandboxObservationState::Retired {
            return Err(AppError::Conflict("cma_sandbox_retirement_proof_required: CMA has not confirmed retirement of this exact sandbox".into()));
        }
        let data = json!({"reservation_id":parent.id,"owner_id":parent.user_id,"cma_service_id":parent.cma_service_id,
            "generation":parent.generation,"revision":parent.agent_reference_version,"creation_revision":request.creation_revision,
            "action":"confirm_retirement","result":"resolved"});
        let (parent, creation) = admission::commit(
            &db,
            &parent,
            creation.as_ref(),
            admission::Change::ConfirmRetirement(remote),
            actor,
            data,
        )
        .await?;
        return Ok(project(&parent, creation.as_ref()));
    }
    let creation = admission::creation_for_sandbox(&db, &parent)
        .await?
        .ok_or_else(|| AppError::Conflict("cma_creation_record_missing: no exact creation evidence is available; the reservation cannot be resolved".into()))?;
    if Some(creation.revision) != request.creation_revision {
        return Err(drift());
    }
    verify(gateway, &parent, Some(&creation)).await?;
    let change = if creation.covered_key.is_some() {
        use super::assistant_cma_admission::creation_outcome::{self, Evidence};
        match (
            request.action,
            creation_outcome::lookup(gateway, &creation).await?,
        ) {
            (ResolveAction::Release, Evidence::Absent { operation }) => {
                admission::Change::CoveredAbsent(operation)
            }
            (ResolveAction::Associate, Evidence::Created { sandbox, operation })
                if request.remote_sandbox_id.as_deref() == Some(&sandbox)
                    && parent.cma_sandbox_id.is_none() =>
            {
                let remote = exact_sandbox(gateway, &parent.user_id, &sandbox).await?;
                admission::Change::CoveredCreated(remote, operation)
            }
            _ => return Err(drift()),
        }
    } else {
        match request.action {
            ResolveAction::ConfirmRetirement => unreachable!("handled above"),
            ResolveAction::Release => {
                if !has_non_creation_proof(&creation) {
                    return Err(AppError::Conflict("cma_creation_absence_proof_required: CMA has not provided authoritative proof that this creation was never made".into()));
                }
                admission::Change::Release
            }
            ResolveAction::Associate => {
                if parent.cma_sandbox_id.is_some()
                    || !parent.occupies_account_slot
                    || creation.submission_state == "confirmed_not_created"
                {
                    return Err(drift());
                }
                let id = request.remote_sandbox_id.as_deref().ok_or_else(|| {
                    AppError::ValidationError("remote_sandbox_id is required".into())
                })?;
                if creation
                    .cma_sandbox_id
                    .as_deref()
                    .is_some_and(|saved| saved != id)
                {
                    return Err(drift());
                }
                if creation.cma_sandbox_id.is_none()
                    && creation.operation_id.is_none()
                    && creation.cma_agent_id.is_none()
                {
                    return Err(AppError::Conflict("cma_creation_provenance_required: no saved receipt or CMA lineage ties a sandbox to this creation; ownership alone cannot establish NyxID creation provenance".into()));
                }
                if creation.operation_id.is_some() || creation.cma_agent_id.is_some() {
                    let (_, observed) = observe(gateway, &parent, Some(&creation)).await;
                    if observed.as_ref().is_none_or(|remote| remote.id != id) {
                        return Err(drift());
                    }
                }
                let remote = exact_sandbox(gateway, &parent.user_id, id).await?;
                if remote.state == SandboxObservationState::Retired {
                    return Err(AppError::Conflict(
                    "cma_sandbox_retired: a retired resource cannot resolve an uncertain creation"
                        .into(),
                ));
                }
                admission::Change::Associate(remote)
            }
        }
    };
    let data = json!({"reservation_id":parent.id,"owner_id":parent.user_id,"cma_service_id":parent.cma_service_id,
        "generation":parent.generation,"revision":parent.agent_reference_version,"creation_revision":creation.revision,
        "action":request.action.as_str(),"result":"resolved"});
    let (parent, creation) =
        admission::commit(&db, &parent, Some(&creation), change, actor, data).await?;
    Ok(project(&parent, creation.as_ref()))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> AssistantCmaSandbox {
        let now = Utc::now();
        AssistantCmaSandbox {
            id: "reservation".into(),
            user_id: "owner".into(),
            cma_service_id: "cma".into(),
            label: "private label".into(),
            cma_sandbox_id: None,
            occupies_account_slot: true,
            admission_state: SandboxAdmissionState::Reserved,
            observation_state: SandboxObservationState::Unknown,
            creation_ref: "creation".into(),
            generation: 1,
            agent_reference_version: 0,
            source: sandboxes::NYXID_CREATED_SOURCE.into(),
            last_observed: None,
            machine_included: None,
            resource_state: None,
            cpu_millis: None,
            memory_mib: None,
            capacity_source: None,
            last_activity_at: None,
            retire_requested_at: None,
            retirement_idempotency_key: Some("secret-retirement-key".into()),
            retired_at: None,
            idempotency_key: Some("secret-idempotency-key".into()),
            request_hash: Some("private-request-hash".into()),
            operation_id: Some("operation".into()),
            submission_state: "pending".into(),
            request_ciphertext: Some(vec![1, 2, 3]),
            request_context_version: Some(3),
            created_at: now,
            updated_at: now,
        }
    }

    fn creation(submission_state: &str) -> AssistantCmaCreation {
        AssistantCmaCreation {
            id: "creation".into(),
            user_id: "owner".into(),
            cma_service_id: "cma".into(),
            deployment_fingerprint: "fingerprint".into(),
            method: "POST".into(),
            path: "/api/v1/sandboxes".into(),
            idempotency_key: "secret-idempotency-key".into(),
            covered_key: None,
            request_hash: "private-request-hash".into(),
            sandbox_ref_id: "reservation".into(),
            sandbox_generation: 1,
            creates_sandbox: true,
            expected_workspace_id: None,
            released_by_inventory: false,
            submission_state: submission_state.into(),
            operation_id: Some("operation".into()),
            cma_agent_id: None,
            cma_sandbox_id: None,
            revision: 4,
            submission_generation: 0,
            recovery_checked_at: None,
            first_dispatched_at: None,
            request_ciphertext: Some(vec![4, 5, 6]),
            request_context_version: Some(3),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn holding_projection_covers_each_slot_state() {
        let mut row = sandbox();
        row.admission_state = SandboxAdmissionState::Unknown;
        row.submission_state = "unknown".into();
        assert_eq!(holding(&row, None), "creation_unknown");
        row.submission_state = "pending".into();

        row.admission_state = SandboxAdmissionState::Creating;
        assert_eq!(holding(&row, None), "creation_in_progress");

        row.admission_state = SandboxAdmissionState::Reserved;
        row.cma_sandbox_id = Some("sandbox".into());
        assert_eq!(holding(&row, None), "live_sandbox");

        row.retire_requested_at = Some(Utc::now());
        assert_eq!(holding(&row, None), "retiring");
    }

    #[test]
    fn creation_facts_drive_progress_and_projection_is_redacted() {
        let row = sandbox();
        let projected = project(&row, Some(&creation("unknown")));
        assert_eq!(row.submission_state, "pending");
        assert_eq!(projected.submission_state, "unknown");
        assert_eq!(projected.holding, "creation_unknown");
        assert_eq!(
            project(&row, Some(&creation("submitted"))).holding,
            "creation_in_progress"
        );
        assert_eq!(
            projected.lifecycle_operation_id.as_deref(),
            Some("operation")
        );
        let encoded = serde_json::to_string(&projected).unwrap();
        for (index, marker) in [
            "secret-retirement-key",
            "secret-idempotency-key",
            "private-request-hash",
            "request_ciphertext",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                !encoded.contains(marker),
                "projection leaked redacted field #{index}"
            );
        }
    }

    #[test]
    fn cursor_and_state_filters_are_bounded() {
        assert!(valid_cursor("reservation").is_ok());
        assert!(valid_cursor("").is_err());
        assert!(valid_cursor("line\nfeed").is_err());
        assert!(valid_cursor(&"x".repeat(129)).is_err());
    }
}
