//! Fenced reservation transitions shared by administration and background observation.
use super::*;
use crate::services::{
    assistant_cma_gateway::CmaSandbox,
    audit_service::{AuditActor, log_actor_event_in_transaction},
};

pub(crate) async fn creation_for_sandbox(
    db: &Database,
    parent: &AssistantCmaSandbox,
) -> AppResult<Option<AssistantCmaCreation>> {
    let Some(key) = &parent.idempotency_key else {
        return Ok(None);
    };
    Ok(db
        .collection::<AssistantCmaCreation>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": &parent.user_id, "cma_service_id": &parent.cma_service_id,
            "sandbox_ref_id": &parent.id, "sandbox_generation": parent.generation,
            "idempotency_key": key, "creates_sandbox": true,
        })
        .await?)
}

#[derive(Clone)]
pub(crate) enum Change {
    Associate(CmaSandbox),
    Release,
    ConfirmRetirement(CmaSandbox),
}

pub(crate) async fn commit(
    db: &Database,
    parent: &AssistantCmaSandbox,
    creation: Option<&AssistantCmaCreation>,
    change: Change,
    actor: &AuditActor,
    audit_data: Value,
) -> AppResult<(AssistantCmaSandbox, Option<AssistantCmaCreation>)> {
    commit_with_actor(db, parent, creation, change, Some(actor), audit_data).await
}

pub(crate) async fn commit_recovery(
    db: &Database,
    parent: &AssistantCmaSandbox,
    creation: Option<&AssistantCmaCreation>,
    change: Change,
    audit_data: Value,
) -> AppResult<(AssistantCmaSandbox, Option<AssistantCmaCreation>)> {
    commit_with_actor(db, parent, creation, change, None, audit_data).await
}

/// Provider reads stay outside the transaction. Both the observed parent and
/// creation revision are compared again here, before any metadata can change.
async fn commit_with_actor(
    db: &Database,
    parent: &AssistantCmaSandbox,
    creation: Option<&AssistantCmaCreation>,
    change: Change,
    actor: Option<&AuditActor>,
    audit_data: Value,
) -> AppResult<(AssistantCmaSandbox, Option<AssistantCmaCreation>)> {
    let context = (
        db.clone(),
        parent.clone(),
        creation.cloned(),
        change,
        actor.cloned(),
        audit_data,
    );
    super::super::transaction_service::run_db_only(db, &context, |tx, (db, before, expected, change, actor, audit_data)| {
        Box::pin(async move {
            super::super::erasure_admission::fence_in_session(db, &before.user_id, tx).await?;
            let parents = db.collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME);
            let current = parents.find_one(sandboxes::observation_filter(before)).session(&mut *tx).await?.ok_or_else(drift)?;
            let creations = db.collection::<AssistantCmaCreation>(COLLECTION_NAME);
            let mut creation = match expected {
                Some(expected) => {
                    let row = creations.find_one(revision_filter(expected)).session(&mut *tx).await?.ok_or_else(drift)?;
                    require_parent(&row, &current)?;
                    Some(row)
                },
                None => None,
            };
            let mut next = current.clone();
            match change {
                Change::Associate(remote) => {
                    let row = creation.as_mut().ok_or_else(drift)?;
                    if !current.occupies_account_slot || current.cma_sandbox_id.is_some()
                        || row.submission_state == "confirmed_not_created"
                        || remote.state == SandboxObservationState::Retired
                    { return Err(drift()); }
                    next = sandboxes::fold_created_observation(db, tx, &current, remote).await?;
                    // Associating a lifecycle creation's parent does not prove
                    // the Agent creation finished; keep its recovery material.
                    let change = if row.path == "/api/v1/sandboxes" {
                        CreationChange::Confirm { sandbox_id: remote.id.clone(), agent_id: None }
                    } else { CreationChange::Receipt(CreationReceipt::Sandbox(remote.id.clone())) };
                    apply_creation_change(row, &mut next, &change)?;
                },
                Change::ConfirmRetirement(remote) => {
                    if !current.occupies_account_slot
                        || current.cma_sandbox_id.as_deref() != Some(&remote.id)
                        || remote.state != SandboxObservationState::Retired
                    { return Err(drift()); }
                    if let Some(row) = creation.as_mut() {
                        apply_creation_change(row, &mut next, &CreationChange::Confirm {
                            sandbox_id: remote.id.clone(), agent_id: None,
                        })?;
                    }
                    sandboxes::apply_observation(&mut next, remote, current.generation, Utc::now())?;
                },
                Change::Release => {
                    let row = creation.as_mut().ok_or_else(drift)?;
                    if row.submission_state != "confirmed_not_created" || current.cma_sandbox_id.is_some() {
                        return Err(AppError::Conflict("cma_creation_absence_proof_required: CMA has not provided authoritative proof that this creation was never made".into()));
                    }
                    apply_creation_change(row, &mut next, &CreationChange::NoResource)?;
                },
            }
            let mut set = sandboxes::observation_set(&next)?;
            set.insert("retire_requested_at", next.retire_requested_at.map(bson::DateTime::from_chrono));
            set.insert("label", &next.label);
            let saved = parents.find_one_and_update(sandboxes::observation_filter(&current), doc! {"$set":set,"$inc":{"agent_reference_version":1_i64}})
                .return_document(ReturnDocument::After).session(&mut *tx).await?.ok_or_else(drift)?;
            if let (Some(expected), Some(next)) = (expected.as_ref(), creation.as_ref())
                && creations.replace_one(revision_filter(expected), next).session(&mut *tx).await?.matched_count != 1
            {
                return Err(drift());
            }
            if let Some(actor) = actor {
                log_actor_event_in_transaction(db, tx, actor, "admin_cma_sandbox_reservation_resolved", audit_data.clone()).await?;
            } else {
                let key = super::super::audit_service::audit_chain_hmac_key()
                    .ok_or_else(|| AppError::Internal("Audit authority unavailable".into()))?;
                super::super::audit_service::log_system_event_in_session(db, tx, key,
                    "cma_sandbox_reservation_recovered", audit_data.clone()).await?;
            }
            Ok((saved, creation))
        })
    }).await
}

/// Background receipt reconciliation uses the existing exact identity helpers;
/// it has no path to replay or to create a remote resource.
pub(crate) async fn recover_receipt(
    db: &Database,
    gateway: &dyn CmaGateway,
    creation: &AssistantCmaCreation,
) -> AppResult<&'static str> {
    verify_creation(gateway, creation).await?;
    let creation = change_creation(db, creation, CreationChange::Checked).await?;
    if let Some(operation) = &creation.operation_id {
        let value = gateway.get_lifecycle(&creation.user_id, operation).await?;
        if let Some(agent) = lifecycle_agent(operation, &value)? {
            finish_agent(db, gateway, &creation, agent).await?;
        } else {
            return Ok(
                super::super::assistant_cma_reservation_service::operation_without_resource(&value),
            );
        }
    } else if let Some(agent) = &creation.cma_agent_id {
        finish_agent(db, gateway, &creation, agent).await?;
    } else if let Some(sandbox) = &creation.cma_sandbox_id {
        finish_sandbox(db, gateway, &creation, sandbox).await?;
    } else {
        return Ok("creation_identity_not_observable_without_replay");
    }
    Ok("creation_confirmed")
}
