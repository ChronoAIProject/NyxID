//! Owner-scoped CMA Agent references and public execution-view projections.

use chrono::{DateTime, Utc};
use mongodb::{
    Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use serde::Serialize;
use uuid::Uuid;

use super::assistant_cma_gateway::{
    CmaAgent, CmaDesktopDescriptor, CmaGateway, CmaGatewayError, validate_remote_id,
};
use super::assistant_cma_sandbox_service::NYXID_CREATED_SOURCE;
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_cma_agent::{AgentObservationState, AssistantCmaAgent, COLLECTION_NAME},
        assistant_cma_sandbox::{AssistantCmaSandbox, SandboxAdmissionState},
    },
};

pub fn indexes() -> Vec<IndexModel> {
    vec![
        IndexModel::builder()
            .keys(doc! {"user_id":1,"user_service_id":1,"execution_base_url":1,"cma_service_id":1,"cma_agent_id":1})
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .partial_filter_expression(doc! {"cma_agent_id":{"$type":"string"},"user_service_id":{"$type":"string"},"execution_base_url":{"$type":"string"}})
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

pub async fn require_enabled(db: &Database, owner: &str) -> AppResult<()> {
    use crate::models::user::{COLLECTION_NAME as USERS, User};
    let person = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":owner,"is_active":true})
        .await?
        .is_some_and(|user| user.user_type.is_person());
    if !person {
        return Err(AppError::Forbidden("A personal account is required".into()));
    }
    let enabled = super::feature_flag_service::personal_flag_enabled(
        db,
        owner,
        super::feature_flag_service::ASSISTANT_CMA_SANDBOXES_FLAG_KEY,
    )
    .await?;
    require_flag(enabled)
}

pub fn require_flag(enabled: bool) -> AppResult<()> {
    if enabled {
        Ok(())
    } else {
        Err(AppError::NotFound(
            "CMA Sandbox access is not enabled".into(),
        ))
    }
}

#[derive(Clone, Default)]
pub struct AgentSource {
    pub conversation_id: Option<String>,
    pub role: Option<String>,
    pub agent_id: Option<String>,
    pub turn_id: Option<String>,
    pub watch_id: Option<String>,
    pub response_id: Option<String>,
    pub creation_operation_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentDto {
    pub id: String,
    pub cma_agent_id: String,
    pub label: String,
    pub status: String,
    pub sandbox_id: Option<String>,
    pub cma_sandbox_id: Option<String>,
    pub cma_workspace_id: Option<String>,
    pub cma_worktree_id: Option<String>,
    pub last_observed: Option<DateTime<Utc>>,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub profile: Option<serde_json::Value>,
}

fn dto(
    row: &AssistantCmaAgent,
    parent: Option<&AssistantCmaSandbox>,
    reason: Option<&str>,
) -> AgentDto {
    AgentDto {
        id: row.id.clone(),
        cma_agent_id: row.cma_agent_id.clone(),
        label: row.label.clone(),
        status: if reason.is_some() {
            "unknown".into()
        } else {
            row.resource_state
                .clone()
                .unwrap_or_else(|| row.observation_state.as_str().into())
        },
        sandbox_id: parent.map(|parent| parent.id.clone()),
        cma_sandbox_id: row.cma_sandbox_id.clone(),
        cma_workspace_id: row.cma_workspace_id.clone(),
        cma_worktree_id: row.cma_worktree_id.clone(),
        last_observed: row.last_observed,
        available: reason.is_none(),
        unavailable_reason: reason.map(str::to_owned),
        profile: safe_profile(row.profile_snapshot.as_ref()),
    }
}

fn safe_profile(value: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let value = value?.as_object()?;
    let mut profile = serde_json::Map::new();
    for key in ["model", "permission_mode", "thinking_effort"] {
        if let Some(value) = value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| value.len() <= 256 && !value.contains('\0'))
        {
            profile.insert(key.into(), serde_json::Value::String(value.into()));
        }
    }
    Some(serde_json::Value::Object(profile))
}

fn observation_state(status: &str) -> AgentObservationState {
    match status {
        "starting" | "provisioning" => AgentObservationState::Provisioning,
        "ready" => AgentObservationState::Ready,
        "working" | "waiting_approval" | "running" => AgentObservationState::Running,
        "paused" => AgentObservationState::Paused,
        "stopped" => AgentObservationState::Stopped,
        "deleted" => AgentObservationState::Deleted,
        "failed" => AgentObservationState::Failed,
        _ => AgentObservationState::Unknown,
    }
}

fn unavailable(error: &CmaGatewayError) -> Option<&'static str> {
    match error {
        CmaGatewayError::NotFound => Some("agent_unavailable"),
        CmaGatewayError::Timeout
        | CmaGatewayError::Unavailable
        | CmaGatewayError::InvalidResponse
        | CmaGatewayError::UpstreamAuthorizationUnavailable => Some("cma_observation_unavailable"),
        _ => None,
    }
}

fn unavailable_parent(error: &AppError) -> Option<&'static str> {
    match error {
        AppError::NotFound(_) => Some("sandbox_unavailable"),
        AppError::Conflict(message) if message == "cma_observation_unavailable" => {
            Some("cma_observation_unavailable")
        }
        _ => None,
    }
}

pub async fn get_owned(db: &Database, owner: &str, id: &str) -> AppResult<AssistantCmaAgent> {
    db.collection::<AssistantCmaAgent>(COLLECTION_NAME)
        .find_one(doc! {"_id":id,"user_id":owner,"user_service_id":{"$type":"string"},"execution_base_url":{"$type":"string"}})
        .await?
        .ok_or_else(|| AppError::NotFound("CMA Agent not found".into()))
}

async fn parent_ref(
    db: &Database,
    row: &AssistantCmaAgent,
) -> AppResult<Option<AssistantCmaSandbox>> {
    let Some(remote) = &row.cma_sandbox_id else {
        return Err(AppError::NotFound("CMA Agent parent not found".into()));
    };
    let parent = db.collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .find_one(doc! {"user_id":&row.user_id,"user_service_id":&row.user_service_id,"execution_base_url":&row.execution_base_url,"cma_service_id":&row.cma_service_id,"cma_sandbox_id":remote,"source":NYXID_CREATED_SOURCE}).await?
        .ok_or_else(|| AppError::NotFound("CMA Agent parent not found".into()))?;
    if row.sandbox_generation != Some(parent.generation) {
        return Err(AppError::Conflict("cma_sandbox_generation_changed".into()));
    }
    Ok(Some(parent))
}

fn verify_observation(row: &AssistantCmaAgent, remote: &CmaAgent) -> AppResult<()> {
    if remote.id != row.cma_agent_id
        || remote
            .owner_id
            .as_deref()
            .is_some_and(|owner| owner != row.user_id)
        || remote.sandbox_id != row.cma_sandbox_id
        || remote.workspace_id != row.cma_workspace_id
        || remote.worktree_id != row.cma_worktree_id
    {
        return Err(AppError::Forbidden("CMA Agent identity changed".into()));
    }
    if remote
        .revision
        .zip(row.last_observed_revision)
        .is_some_and(|(current, previous)| current < previous)
    {
        return Err(AppError::Conflict("cma_agent_observation_stale".into()));
    }
    Ok(())
}

pub async fn require_bound(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    id: &str,
) -> AppResult<(AssistantCmaAgent, Option<AssistantCmaSandbox>)> {
    let row = get_owned(db, owner, id).await?;
    let (connection, parent) = tokio::try_join!(
        async { gateway.connection(owner).await.map_err(AppError::from) },
        parent_ref(db, &row),
    )?;
    if connection.user_service_id != row.user_service_id
        || connection.cma_service_id != row.cma_service_id
        || connection.execution_base_url != row.execution_base_url
    {
        return Err(AppError::Conflict("cma_deployment_changed".into()));
    }
    Ok((row, parent))
}

async fn verify_parent(gateway: &dyn CmaGateway, owner: &str, remote: &CmaAgent) -> AppResult<()> {
    let parent_id = remote
        .sandbox_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("cma_agent_parent_unavailable".into()))?;
    verify_parent_id(gateway, owner, parent_id).await
}

async fn verify_parent_id(gateway: &dyn CmaGateway, owner: &str, parent_id: &str) -> AppResult<()> {
    let parent = gateway.get_sandbox_authority(owner, parent_id).await?;
    if parent.id != parent_id || parent.owner_id.as_deref() != Some(owner) {
        return Err(AppError::Forbidden("CMA Agent owner mismatch".into()));
    }
    Ok(())
}

async fn read_verified_agent(
    gateway: &dyn CmaGateway,
    owner: &str,
    row: &AssistantCmaAgent,
) -> AppResult<CmaAgent> {
    // The persisted parent is immutable and already bound to this owner and
    // generation. Its live read need not wait for another Agent GET.
    let parent_id = row
        .cma_sandbox_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("cma_agent_parent_unavailable".into()))?;
    let (remote, ()) = tokio::try_join!(
        async {
            gateway
                .get_agent(owner, &row.cma_agent_id)
                .await
                .map_err(AppError::from)
        },
        verify_parent_id(gateway, owner, parent_id),
    )?;
    verify_observation(row, &remote)?;
    Ok(remote)
}

fn observation_fields(remote: &CmaAgent, now: DateTime<Utc>) -> Document {
    let mut fields = doc! {"label":remote.name.clone().unwrap_or_else(||"CMA Agent".into()),
    "observation_state":observation_state(&remote.status).as_str(),"resource_state":&remote.status,
    "profile_snapshot":bson::to_bson(&safe_profile(remote.profile.as_ref())).unwrap_or(bson::Bson::Null),
    "phase":&remote.phase,"last_observed":bson::DateTime::from_chrono(now),
    "last_observed_revision":remote.revision,"updated_at":bson::DateTime::from_chrono(now)};
    if let Some(callback) = &remote.callback_url {
        fields.insert(
            "callback_url",
            super::cma_operation::validated_callback_url(callback.as_deref()),
        );
    }
    fields
}

/// Register only a resource observed through the owner-authenticated boundary.
/// This is shared by tool receipts, discovery and watch completion; no browser
/// endpoint accepts caller-selected catalog or remote owner identifiers.
pub async fn register_live(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    remote_id: &str,
    source: Option<&AgentSource>,
) -> AppResult<AssistantCmaAgent> {
    validate_remote_id(remote_id)?;
    let connection = gateway.connection(owner).await?;
    let service_id = connection.cma_service_id.clone();
    let remote = gateway.get_agent(owner, remote_id).await?;
    if remote.id != remote_id
        || remote
            .owner_id
            .as_deref()
            .is_some_and(|value| value != owner)
    {
        return Err(AppError::Forbidden("CMA Agent owner mismatch".into()));
    }
    verify_parent(gateway, owner, &remote).await?;
    let parent_id = remote
        .sandbox_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("cma_agent_parent_unavailable".into()))?;
    let db = super::transaction_service::durable_database(db);
    ensure_indexes(&db).await?;
    db.collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .find_one(doc! {"user_id":owner,"user_service_id":&connection.user_service_id,"execution_base_url":&connection.execution_base_url,"cma_service_id":&service_id,"cma_sandbox_id":parent_id,"source":NYXID_CREATED_SOURCE})
        .await?
        .ok_or_else(|| AppError::NotFound("Sandbox was not created through NyxID".into()))?;
    let now = Utc::now();
    let mut set = observation_fields(&remote, now);
    if remote.status == "deleted" {
        set.insert("deleted_at", bson::DateTime::from_chrono(now));
    }
    // Bounded metadata only. Never store prompts or arbitrary tool arguments.
    if let Some(source) = source {
        for (key, value) in [
            ("source_conversation_id", &source.conversation_id),
            ("source_role", &source.role),
            ("source_agent_id", &source.agent_id),
            ("source_turn_id", &source.turn_id),
            ("source_watch_id", &source.watch_id),
            ("source_response_id", &source.response_id),
            ("creation_operation_id", &source.creation_operation_id),
        ] {
            if let Some(value) = value {
                validate_remote_id(value)?;
                set.insert(key, value);
            }
        }
    }
    let context = (
        db.clone(),
        owner.to_owned(),
        connection,
        remote,
        set,
        Uuid::new_v4().to_string(),
        now,
    );
    super::transaction_service::run_db_only(&db,&context,|tx,(db,owner,connection,remote,set,local_id,now)| {
        Box::pin(async move {
            super::erasure_admission::fence_in_session(db,owner,tx).await?;
            let service=&connection.cma_service_id;
            let parent_id=remote.sandbox_id.as_deref().ok_or_else(||AppError::Conflict("cma_agent_parent_unavailable".into()))?;
            // A real parent write serializes this insert with receipt folding,
            // so a late registration cannot recreate a discarded generation.
            let parent=db.collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
                .find_one_and_update(doc! {"user_id":owner,"user_service_id":&connection.user_service_id,"execution_base_url":&connection.execution_base_url,"cma_service_id":service,"cma_sandbox_id":parent_id,"source":NYXID_CREATED_SOURCE},doc! {"$inc":{"agent_reference_version":1}})
                .return_document(ReturnDocument::After).session(&mut *tx).await?
                .ok_or_else(||AppError::Conflict("cma_agent_parent_unavailable".into()))?;
            let identity=doc! {"user_id":owner,"user_service_id":&connection.user_service_id,"execution_base_url":&connection.execution_base_url,"cma_service_id":service,"cma_agent_id":&remote.id};
            if let Some(existing)=db.collection::<AssistantCmaAgent>(COLLECTION_NAME).find_one(identity.clone()).session(&mut *tx).await? {
                verify_observation(&existing,remote)?;
                if existing.sandbox_generation!=Some(parent.generation) {return Err(AppError::Conflict("cma_sandbox_generation_changed".into()));}
            }
            let mut filter=identity;filter.insert("cma_sandbox_id",parent_id);filter.insert("sandbox_generation",parent.generation);
            let insert=doc! {"_id":local_id,"user_id":owner,"user_service_id":&connection.user_service_id,"execution_base_url":&connection.execution_base_url,"cma_service_id":service,
                "cma_agent_id":&remote.id,"cma_sandbox_id":parent_id,"sandbox_generation":parent.generation,
                "cma_workspace_id":&remote.workspace_id,"cma_worktree_id":&remote.worktree_id,
                "created_at":bson::DateTime::from_chrono(*now)};
            db.collection::<AssistantCmaAgent>(COLLECTION_NAME).find_one_and_update(filter,doc! {"$set":set,"$setOnInsert":insert})
                .upsert(true).return_document(ReturnDocument::After).session(tx).await?
                .ok_or_else(||AppError::Conflict("cma_agent_registration_changed".into()))
        })
    }).await
}

pub async fn detail(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    id: &str,
) -> AppResult<AgentDto> {
    let row = get_owned(db, owner, id).await?;
    let parent = parent_ref(db, &row).await?;
    let remote = match gateway.connection(owner).await {
        Ok(connection)
            if connection.user_service_id == row.user_service_id
                && connection.cma_service_id == row.cma_service_id
                && connection.execution_base_url == row.execution_base_url =>
        {
            gateway.get_agent(owner, &row.cma_agent_id).await
        }
        Ok(_) => return Err(AppError::Conflict("cma_deployment_changed".into())),
        Err(error) => Err(error),
    };
    let remote = match remote {
        Ok(remote) => remote,
        Err(error) => {
            return match unavailable(&error) {
                Some(reason) => Ok(dto(&row, parent.as_ref(), Some(reason))),
                None => Err(error.into()),
            };
        }
    };
    verify_observation(&row, &remote)?;
    if let Err(error) = verify_parent(gateway, owner, &remote).await {
        return match unavailable_parent(&error) {
            Some(reason) => Ok(dto(&row, parent.as_ref(), Some(reason))),
            None => Err(error),
        };
    }
    let updated = db.collection::<AssistantCmaAgent>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id":&row.id,"user_id":owner,"user_service_id":&row.user_service_id,"execution_base_url":&row.execution_base_url,"cma_agent_id":&row.cma_agent_id,"cma_service_id":&row.cma_service_id,"sandbox_generation":row.sandbox_generation},
        doc! {"$set":observation_fields(&remote,Utc::now())}).return_document(ReturnDocument::After).await?
        .ok_or_else(||AppError::NotFound("CMA Agent not found".into()))?;
    Ok(dto(&updated, parent.as_ref(), None))
}

pub fn desktop_reason(
    remote: &CmaAgent,
    parent: Option<&AssistantCmaSandbox>,
) -> Option<&'static str> {
    if remote.desktop != Some(true) {
        return Some("desktop_not_enabled");
    }
    match remote.status.as_str() {
        "stopped" => return Some("agent_stopped"),
        "deleted" => return Some("agent_deleted"),
        "failed" => return Some("agent_failed"),
        "paused" => return Some("agent_paused"),
        "starting" | "ready" | "working" | "waiting_approval" => {}
        _ => return Some("agent_state_unknown"),
    }
    if parent.is_some_and(|parent| parent.admission_state == SandboxAdmissionState::Retired) {
        return Some("sandbox_retired");
    }
    if parent.is_some_and(|parent| parent.retire_requested_at.is_some()) {
        return Some("sandbox_retiring");
    }
    match remote.sandbox_status.as_deref() {
        Some("active" | "ready" | "running") => None,
        Some("paused" | "pausing") => Some("sandbox_paused"),
        Some("retired" | "retiring") => Some("sandbox_retired"),
        _ => Some("sandbox_unavailable"),
    }
}

pub async fn desktop(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    id: &str,
) -> AppResult<CmaDesktopDescriptor> {
    let (row, parent) = require_bound(db, gateway, owner, id).await?;
    let remote = match gateway.get_agent(owner, &row.cma_agent_id).await {
        Ok(remote) => remote,
        Err(error) => {
            return match unavailable(&error) {
                Some(reason) => Ok(unavailable_desktop(reason)),
                None => Err(error.into()),
            };
        }
    };
    verify_observation(&row, &remote)?;
    if let Err(error) = verify_parent(gateway, owner, &remote).await {
        return match unavailable_parent(&error) {
            Some(reason) => Ok(unavailable_desktop(reason)),
            None => Err(error),
        };
    }
    if let Some(reason) = desktop_reason(&remote, parent.as_ref()) {
        return Ok(unavailable_desktop(reason));
    }
    let mut descriptor = match gateway.desktop_descriptor(owner, &row.cma_agent_id).await {
        Ok(descriptor) => descriptor,
        Err(error) => {
            return match unavailable(&error) {
                Some(reason) => Ok(unavailable_desktop(reason)),
                None => Err(error.into()),
            };
        }
    };
    if descriptor.available {
        descriptor.websocket_path = Some(format!(
            "/api/v1/assistant/nyxagent/cma-agents/{}/desktop/ws",
            row.id
        ));
    }
    Ok(descriptor)
}

fn unavailable_desktop(reason: &str) -> CmaDesktopDescriptor {
    CmaDesktopDescriptor {
        available: false,
        reason: Some(reason.into()),
        shared: true,
        width: None,
        height: None,
        websocket_path: None,
    }
}

fn verify_reference(current: &AssistantCmaAgent, original: &AssistantCmaAgent) -> AppResult<()> {
    if current.id != original.id
        || current.user_id != original.user_id
        || current.cma_agent_id != original.cma_agent_id
        || current.user_service_id != original.user_service_id
        || current.execution_base_url != original.execution_base_url
        || current.cma_service_id != original.cma_service_id
        || current.cma_sandbox_id != original.cma_sandbox_id
        || current.cma_workspace_id != original.cma_workspace_id
        || current.cma_worktree_id != original.cma_worktree_id
        || current.sandbox_generation != original.sandbox_generation
    {
        return Err(AppError::Forbidden("CMA Agent authority changed".into()));
    }
    Ok(())
}

async fn revalidate_reference(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    original: &AssistantCmaAgent,
) -> AppResult<(AssistantCmaAgent, Option<AssistantCmaSandbox>)> {
    let ((), (current, parent)) = tokio::try_join!(
        require_enabled(db, owner),
        require_bound(db, gateway, owner, &original.id),
    )?;
    verify_reference(&current, original)?;
    Ok((current, parent))
}

/// Revalidate immutable row/parent identity and live remote ownership.
pub async fn revalidate(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    original: &AssistantCmaAgent,
) -> AppResult<()> {
    let (current, _) = revalidate_reference(db, gateway, owner, original).await?;
    read_verified_agent(gateway, owner, &current).await?;
    Ok(())
}

async fn stream_observation(
    gateway: &dyn CmaGateway,
    owner: &str,
    row: &AssistantCmaAgent,
    parent: Option<&AssistantCmaSandbox>,
    response: Option<&str>,
    desktop: bool,
) -> AppResult<()> {
    let (remote, (), desktop_available) = tokio::try_join!(
        read_verified_agent(gateway, owner, row),
        async {
            if let Some(response_id) = response {
                let response = gateway
                    .get_response(owner, &row.cma_agent_id, response_id)
                    .await?;
                if response.id != response_id || response.agent_id != row.cma_agent_id {
                    return Err(AppError::Forbidden("CMA Response identity changed".into()));
                }
            }
            Ok(())
        },
        async {
            if desktop {
                Ok::<_, AppError>(
                    gateway
                        .desktop_descriptor(owner, &row.cma_agent_id)
                        .await?
                        .available,
                )
            } else {
                Ok(true)
            }
        },
    )?;
    if desktop && (!desktop_available || desktop_reason(&remote, parent).is_some()) {
        return Err(AppError::Forbidden("CMA desktop is unavailable".into()));
    }
    Ok(())
}

/// Read each live stream authority once. No observation data leaves this
/// boundary until local provenance/generation and every remote fence pass.
pub async fn revalidate_stream(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    original: &AssistantCmaAgent,
    response: Option<&str>,
    desktop: bool,
) -> AppResult<()> {
    let (current, parent) = revalidate_reference(db, gateway, owner, original).await?;
    stream_observation(gateway, owner, &current, parent.as_ref(), response, desktop).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn remote() -> CmaAgent {
        CmaAgent {
            id: "agent".into(),
            owner_id: Some("owner".into()),
            sandbox_id: Some("sandbox".into()),
            workspace_id: Some("workspace".into()),
            worktree_id: Some("worktree".into()),
            name: None,
            status: "ready".into(),
            phase: None,
            revision: None,
            profile: None,
            current_response_id: None,
            desktop: Some(true),
            sandbox_status: Some("active".into()),
            callback_url: None,
        }
    }
    #[test]
    fn desktop_requires_exact_live_agent_capability_and_available_parent() {
        let mut agent = remote();
        assert_eq!(desktop_reason(&agent, None), None);
        for (status, reason) in [
            ("stopped", "agent_stopped"),
            ("deleted", "agent_deleted"),
            ("unknown", "agent_state_unknown"),
        ] {
            agent.status = status.into();
            assert_eq!(desktop_reason(&agent, None), Some(reason));
        }
        agent.status = "ready".into();
        agent.desktop = Some(false);
        assert_eq!(desktop_reason(&agent, None), Some("desktop_not_enabled"));
        agent.desktop = Some(true);
        agent.sandbox_status = Some("paused".into());
        assert_eq!(desktop_reason(&agent, None), Some("sandbox_paused"));
    }
    #[test]
    fn flag_off_and_unknown_states_fail_closed() {
        assert!(require_flag(false).is_err());
        assert!(require_flag(true).is_ok());
        assert_eq!(
            observation_state("new_remote_state"),
            AgentObservationState::Unknown
        );
        assert!(unavailable(&CmaGatewayError::Forbidden).is_none());
    }

    fn apply_callback_observation(
        callback: Option<serde_json::Value>,
    ) -> (AssistantCmaAgent, Document) {
        let now = Utc::now();
        let mut stored = doc! {
            "_id":"internal-agent","user_id":"owner","user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id":"catalog-service",
            "cma_agent_id":"agent","cma_sandbox_id":"sandbox",
            "cma_workspace_id":"workspace","cma_worktree_id":"worktree","sandbox_generation":3,
            "created_at":bson::DateTime::from_chrono(now),"updated_at":bson::DateTime::from_chrono(now),
            "last_observed_revision":6,"source_conversation_id":"private-chat",
            "callback_url":"https://old.example/agents/agent"
        };
        let previous: AssistantCmaAgent = bson::from_document(stored.clone()).unwrap();
        let mut payload = serde_json::to_value(remote()).unwrap();
        payload.as_object_mut().unwrap().remove("callback_url");
        if let Some(callback) = callback {
            payload["callback_url"] = callback;
        }
        payload["status"] = serde_json::json!("working");
        payload["revision"] = serde_json::json!(7);
        payload["profile"] = serde_json::json!({"model":"model_1","api_key":"not stored"});
        let observed: CmaAgent = serde_json::from_value(payload).unwrap();
        verify_observation(&previous, &observed).unwrap();
        let fields = observation_fields(&observed, now);
        // Apply the same $set fields used by registration and live detail to an
        // existing BSON row, including an earlier callback and immutable IDs.
        stored.extend(fields.clone());
        let updated = bson::from_document(stored).unwrap();
        (updated, fields)
    }

    #[test]
    fn cma_agent_callback_observation_replaces_the_stored_url_without_changing_identity() {
        let callback = "https://CMA.example:8443/agents/agent";
        let (updated, fields) = apply_callback_observation(Some(serde_json::json!(callback)));
        assert_eq!(updated.callback_url.as_deref(), Some(callback));
        assert_eq!(fields.get_str("callback_url").unwrap(), callback);
        assert_eq!(updated.observation_state, AgentObservationState::Running);
        assert_eq!(updated.resource_state.as_deref(), Some("working"));
        assert_eq!(updated.last_observed_revision, Some(7));
        assert_eq!(updated.cma_agent_id, "agent");
        assert_eq!(updated.user_id, "owner");
        assert_eq!(updated.cma_service_id, "catalog-service");
        assert_eq!(updated.cma_sandbox_id.as_deref(), Some("sandbox"));
        assert_eq!(updated.cma_workspace_id.as_deref(), Some("workspace"));
        assert_eq!(updated.cma_worktree_id.as_deref(), Some("worktree"));
        assert_eq!(updated.sandbox_generation, Some(3));
        assert_eq!(
            updated.source_conversation_id.as_deref(),
            Some("private-chat")
        );
        assert_eq!(
            updated.profile_snapshot,
            Some(serde_json::json!({"model":"model_1"}))
        );
        let public = serde_json::to_value(dto(&updated, None, None)).unwrap();
        assert!(public.get("callback_url").is_none());
        assert_eq!(public["status"], "working");
    }

    #[test]
    fn cma_agent_callback_observation_omission_preserves_the_stored_url() {
        let (updated, fields) = apply_callback_observation(None);
        assert!(!fields.contains_key("callback_url"));
        assert_eq!(
            updated.callback_url.as_deref(),
            Some("https://old.example/agents/agent")
        );
        assert_eq!(updated.observation_state, AgentObservationState::Running);
    }

    #[test]
    fn cma_agent_callback_observation_null_and_invalid_values_clear_the_stored_url() {
        for callback in [
            serde_json::json!(null),
            serde_json::json!({"url":"https://cma.example/agents/agent"}),
            serde_json::json!("javascript:alert(1)"),
            serde_json::json!("https://cma.example/agents/agent?token=secret"),
            serde_json::json!(format!("https://cma.example/{}", "a".repeat(2048))),
        ] {
            let (updated, fields) = apply_callback_observation(Some(callback.clone()));
            assert_eq!(
                fields.get("callback_url"),
                Some(&bson::Bson::Null),
                "withdrawn or invalid callback must clear persisted metadata: {callback}"
            );
            assert!(updated.callback_url.is_none());
            assert_eq!(updated.observation_state, AgentObservationState::Running);
            assert_eq!(updated.resource_state.as_deref(), Some("working"));
        }
    }

    #[test]
    fn public_agent_projection_omits_private_source_and_untrusted_profile_fields() {
        let mut row:AssistantCmaAgent=bson::from_document(doc! {
            "_id":"internal-agent","user_id":"owner","user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id":"catalog-service","cma_agent_id":"agent",
            "cma_sandbox_id":"sandbox","cma_workspace_id":"workspace","cma_worktree_id":"worktree",
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            "source_conversation_id":"private-chat","source_watch_id":"watch","callback_url":"https://untrusted.example",
        }).unwrap();
        row.profile_snapshot =
            Some(serde_json::json!({"model":"m","permission_mode":"guarded","api_key":"secret"}));
        let value =
            serde_json::to_value(dto(&row, None, Some("cma_observation_unavailable"))).unwrap();
        for key in [
            "user_id",
            "cma_service_id",
            "source_conversation_id",
            "source_watch_id",
            "callback_url",
            "sandbox_generation",
        ] {
            assert!(value.get(key).is_none());
        }
        assert_eq!(value["available"], false);
        assert_eq!(value["status"], "unknown");
        assert_eq!(value["profile"]["model"], "m");
        assert!(value["profile"].get("api_key").is_none());
        let mut observed = remote();
        assert!(verify_observation(&row, &observed).is_ok());
        observed.sandbox_id = Some("different-sandbox".into());
        assert!(verify_observation(&row, &observed).is_err());
        observed = remote();
        observed.owner_id = Some("other-owner".into());
        assert!(verify_observation(&row, &observed).is_err());
    }
}

#[cfg(test)]
mod stream_authority_tests {
    use super::*;
    use crate::services::assistant_cma_gateway::{
        CmaEventSnapshot, CmaLifecycleResult, CmaResponse, CmaResult, CmaSandbox, CmaTranscript,
    };
    use std::{sync::Mutex, time::Duration};

    struct AuthorityGateway {
        agent: CmaAgent,
        parent: CmaSandbox,
        response: CmaResponse,
        desktop: CmaDesktopDescriptor,
        rendezvous: Option<tokio::sync::Barrier>,
        unavailable: Option<&'static str>,
        reads: Mutex<Vec<&'static str>>,
    }

    impl AuthorityGateway {
        fn new() -> Self {
            Self {
                agent: CmaAgent {
                    id: "agent".into(),
                    owner_id: Some("owner".into()),
                    sandbox_id: Some("sandbox".into()),
                    workspace_id: Some("workspace".into()),
                    worktree_id: Some("worktree".into()),
                    name: None,
                    status: "ready".into(),
                    phase: None,
                    revision: Some(7),
                    profile: None,
                    current_response_id: None,
                    desktop: Some(true),
                    sandbox_status: Some("active".into()),
                    callback_url: None,
                },
                parent: CmaSandbox {
                    id: "sandbox".into(),
                    owner_id: Some("owner".into()),
                    state: crate::models::assistant_cma_sandbox::SandboxObservationState::Ready,
                    resource_state: Some("active".into()),
                    name: None,
                    machine_included: Some(true),
                    cpu_millis: None,
                    memory_mib: None,
                    capacity_source: None,
                    last_activity_at: None,
                },
                response: CmaResponse {
                    id: "response".into(),
                    agent_id: "agent".into(),
                    status: "in_progress".into(),
                    output: None,
                },
                desktop: CmaDesktopDescriptor {
                    available: true,
                    reason: None,
                    shared: true,
                    width: None,
                    height: None,
                    websocket_path: Some("/api/v1/agents/agent/desktop/ws".into()),
                },
                rendezvous: None,
                unavailable: None,
                reads: Mutex::new(Vec::new()),
            }
        }

        async fn read(&self, resource: &'static str) -> CmaResult<()> {
            self.reads.lock().unwrap().push(resource);
            if let Some(rendezvous) = &self.rendezvous {
                rendezvous.wait().await;
            }
            if self.unavailable == Some(resource) {
                Err(CmaGatewayError::UpstreamAuthorizationUnavailable)
            } else {
                Ok(())
            }
        }
    }

    #[async_trait::async_trait]
    impl CmaGateway for AuthorityGateway {
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

        async fn get_agent(&self, _: &str, id: &str) -> CmaResult<CmaAgent> {
            assert_eq!(id, "agent");
            self.read("agent").await?;
            Ok(self.agent.clone())
        }
        async fn get_sandbox(&self, _: &str, _: &str) -> CmaResult<CmaSandbox> {
            self.read("capacity").await?;
            Ok(self.parent.clone())
        }
        async fn get_sandbox_authority(&self, _: &str, id: &str) -> CmaResult<CmaSandbox> {
            assert_eq!(id, "sandbox");
            self.read("parent").await?;
            Ok(self.parent.clone())
        }
        async fn get_response(&self, _: &str, agent: &str, id: &str) -> CmaResult<CmaResponse> {
            assert_eq!((agent, id), ("agent", "response"));
            self.read("response").await?;
            Ok(self.response.clone())
        }
        async fn desktop_descriptor(&self, _: &str, id: &str) -> CmaResult<CmaDesktopDescriptor> {
            assert_eq!(id, "agent");
            self.read("desktop").await?;
            Ok(self.desktop.clone())
        }
        async fn pause_sandbox(&self, _: &str, _: &str, _: &str) -> CmaResult<CmaLifecycleResult> {
            Err(CmaGatewayError::Unavailable)
        }
        async fn revive_sandbox(&self, _: &str, _: &str, _: &str) -> CmaResult<CmaLifecycleResult> {
            Err(CmaGatewayError::Unavailable)
        }
        async fn retire_sandbox(&self, _: &str, _: &str, _: &str) -> CmaResult<CmaLifecycleResult> {
            Err(CmaGatewayError::Unavailable)
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
    }

    fn reference() -> AssistantCmaAgent {
        bson::from_document(doc! {
            "_id":"local-agent", "user_id":"owner", "user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id":"service",
            "cma_agent_id":"agent", "cma_sandbox_id":"sandbox",
            "cma_workspace_id":"workspace", "cma_worktree_id":"worktree",
            "sandbox_generation":3, "last_observed_revision":7,
            "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
        })
        .unwrap()
    }

    fn parent_reference() -> AssistantCmaSandbox {
        bson::from_document(doc! {
            "_id":"local-sandbox", "user_id":"owner", "user_service_id":"user-service","execution_base_url":"https://cma.example/","cma_service_id":"service",
            "label":"Sandbox", "cma_sandbox_id":"sandbox", "occupies_account_slot":true,
            "admission_state":"associated", "creation_ref":"creation", "generation":3,
            "source":NYXID_CREATED_SOURCE,
            "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn parent_verification_does_not_read_display_capacity() {
        let gateway = AuthorityGateway::new();
        verify_parent(&gateway, "owner", &gateway.agent)
            .await
            .unwrap();
        assert_eq!(*gateway.reads.lock().unwrap(), ["parent"]);
    }

    #[tokio::test(start_paused = true)]
    async fn stream_observation_reads_required_resources_once_in_parallel_without_capacity() {
        let mut gateway = AuthorityGateway::new();
        // No read can finish until all four have started. Serial reads exhaust
        // the budget even when every individual resource is available.
        gateway.rendezvous = Some(tokio::sync::Barrier::new(4));
        tokio::time::timeout(
            Duration::from_secs(5),
            stream_observation(
                &gateway,
                "owner",
                &reference(),
                Some(&parent_reference()),
                Some("response"),
                true,
            ),
        )
        .await
        .expect("independent authority reads must progress together")
        .unwrap();
        let mut reads = gateway.reads.lock().unwrap().clone();
        reads.sort_unstable();
        assert_eq!(reads, ["agent", "desktop", "parent", "response"]);
    }

    #[tokio::test]
    async fn agent_events_only_read_agent_and_parent_authority() {
        let gateway = AuthorityGateway::new();
        stream_observation(&gateway, "owner", &reference(), None, None, false)
            .await
            .unwrap();
        let mut reads = gateway.reads.lock().unwrap().clone();
        reads.sort_unstable();
        assert_eq!(reads, ["agent", "parent"]);
    }

    #[tokio::test]
    async fn stream_observation_fails_closed_when_any_authority_read_is_unavailable() {
        for resource in ["agent", "parent", "response", "desktop"] {
            let mut gateway = AuthorityGateway::new();
            gateway.unavailable = Some(resource);
            assert!(
                stream_observation(
                    &gateway,
                    "owner",
                    &reference(),
                    None,
                    Some("response"),
                    true
                )
                .await
                .is_err(),
                "{resource} authority loss must stop observation"
            );
        }
    }

    #[tokio::test]
    async fn stream_observation_preserves_identity_response_and_desktop_fences() {
        for changed in [
            "agent_owner",
            "agent_parent",
            "workspace",
            "worktree",
            "revision",
            "parent_id",
            "parent_owner",
            "response_agent",
            "response_id",
            "desktop_disabled",
            "desktop_unavailable",
            "agent_stopped",
            "sandbox_paused",
        ] {
            let mut gateway = AuthorityGateway::new();
            match changed {
                "agent_owner" => gateway.agent.owner_id = Some("other".into()),
                "agent_parent" => gateway.agent.sandbox_id = Some("other".into()),
                "workspace" => gateway.agent.workspace_id = Some("other".into()),
                "worktree" => gateway.agent.worktree_id = Some("other".into()),
                "revision" => gateway.agent.revision = Some(6),
                "parent_id" => gateway.parent.id = "other".into(),
                "parent_owner" => gateway.parent.owner_id = Some("other".into()),
                "response_agent" => gateway.response.agent_id = "other".into(),
                "response_id" => gateway.response.id = "other".into(),
                "desktop_disabled" => gateway.agent.desktop = Some(false),
                "desktop_unavailable" => gateway.desktop.available = false,
                "agent_stopped" => gateway.agent.status = "stopped".into(),
                "sandbox_paused" => gateway.agent.sandbox_status = Some("paused".into()),
                _ => unreachable!(),
            }
            assert!(
                stream_observation(
                    &gateway,
                    "owner",
                    &reference(),
                    None,
                    Some("response"),
                    true
                )
                .await
                .is_err(),
                "{changed} must stop observation"
            );
        }
        for retiring in [false, true] {
            let gateway = AuthorityGateway::new();
            let mut parent = parent_reference();
            if retiring {
                parent.retire_requested_at = Some(Utc::now());
            } else {
                parent.admission_state = SandboxAdmissionState::Retired;
            }
            assert!(
                stream_observation(&gateway, "owner", &reference(), Some(&parent), None, true)
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn stream_reference_cannot_change_immutable_parent_or_generation() {
        let original = reference();
        assert!(verify_reference(&original, &original).is_ok());
        for field in [
            "owner",
            "service",
            "agent",
            "sandbox",
            "workspace",
            "worktree",
            "generation",
        ] {
            let mut current = original.clone();
            match field {
                "owner" => current.user_id = "other".into(),
                "service" => current.cma_service_id = "other".into(),
                "agent" => current.cma_agent_id = "other".into(),
                "sandbox" => current.cma_sandbox_id = Some("other".into()),
                "workspace" => current.cma_workspace_id = Some("other".into()),
                "worktree" => current.cma_worktree_id = Some("other".into()),
                "generation" => current.sandbox_generation = Some(4),
                _ => unreachable!(),
            }
            assert!(verify_reference(&current, &original).is_err(), "{field}");
        }
    }
}
