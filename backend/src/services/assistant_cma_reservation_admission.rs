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
    ReleaseEmptyInventory(Vec<String>),
    CoveredAbsent(Option<String>),
    CoveredProgress(String),
    CoveredCreated(CmaSandbox, Option<String>),
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
                Change::ReleaseEmptyInventory(live_ids) => {
                    let row = creation.as_mut().ok_or_else(drift)?;
                    super::super::assistant_cma_reservation_service::require_inventory_release_candidate(&current, row)?;
                    if crate::services::audit_service::audit_chain_hmac_key().is_none() {
                        return Err(AppError::Internal("Audit chain unavailable".into()));
                    }
                    require_linked_inventory(db, tx, &current, live_ids).await?;
                    apply_creation_change(row, &mut next, &CreationChange::NoResource)?;
                    row.released_by_inventory = true;
                }
                Change::CoveredProgress(operation) => {
                    let row = creation.as_mut().ok_or_else(drift)?;
                    if row.covered_key.is_none() || !row.creates_sandbox || !current.occupies_account_slot
                        || current.cma_sandbox_id.is_some() || row.cma_sandbox_id.is_some() || row.cma_agent_id.is_some()
                        || current.idempotency_key.as_deref()!=Some(row.idempotency_key.as_str())
                        || current.request_hash.as_deref()!=Some(row.request_hash.as_str())
                        || current.operation_id.as_deref().is_some_and(|id| id!=operation)
                        || row.operation_id.as_deref().is_some_and(|id| id!=operation)
                        || matches!(row.submission_state.as_str(),"confirmed"|"confirmed_not_created")
                    { return Err(drift()); }
                    apply_creation_change(row,&mut next,&CreationChange::Receipt(CreationReceipt::Lifecycle(operation.clone())))?;
                },
                Change::CoveredAbsent(operation) => {
                    let row = creation.as_mut().ok_or_else(drift)?;
                    if row.covered_key.is_none() || !current.occupies_account_slot
                        || current.cma_sandbox_id.is_some() || row.cma_sandbox_id.is_some()
                        || row.cma_agent_id.is_some() || row.submission_state == "confirmed"
                        || row.operation_id.as_deref().is_some_and(|id| Some(id) != operation.as_deref())
                        || current.operation_id.as_deref().is_some_and(|id| Some(id) != operation.as_deref())
                    { return Err(drift()); }
                    if let Some(operation) = operation { bind_id(&mut row.operation_id, operation)?; }
                    sandboxes::apply_no_resource_confirmation(&mut next,row.sandbox_generation,&row.idempotency_key,&row.request_hash,Utc::now())?;
                    row.submission_state = "confirmed_not_created".into();
                    row.request_ciphertext = None;
                    row.request_context_version = None;
                    row.updated_at = Utc::now();
                    row.revision = row.revision.checked_add(1).ok_or_else(drift)?;
                    next.operation_id = row.operation_id.clone();
                },
                Change::CoveredCreated(remote, operation) => {
                    let row = creation.as_mut().ok_or_else(drift)?;
                    if row.covered_key.is_none() || !row.creates_sandbox || !current.occupies_account_slot
                        || current.idempotency_key.as_deref() != Some(row.idempotency_key.as_str())
                        || current.request_hash.as_deref() != Some(row.request_hash.as_str())
                        || row.submission_state == "confirmed_not_created"
                        || remote.owner_id.as_deref() != Some(current.user_id.as_str())
                        || current.cma_sandbox_id.as_deref().is_some_and(|id| id != remote.id)
                        || row.cma_sandbox_id.as_deref().is_some_and(|id| id != remote.id)
                        || current.operation_id.as_deref().is_some_and(|id| Some(id) != operation.as_deref())
                        || row.operation_id.as_deref().is_some_and(|id| Some(id) != operation.as_deref())
                    { return Err(drift()); }
                    if let Some(operation) = operation { bind_id(&mut row.operation_id, operation)?; }
                    next = sandboxes::fold_created_observation(db, tx, &current, remote).await?;
                    next.operation_id = row.operation_id.clone();
                    if row.path == "/api/v1/sandboxes" || remote.state == SandboxObservationState::Retired {
                        apply_creation_change(row,&mut next,&CreationChange::Confirm{sandbox_id:remote.id.clone(),agent_id:None})?;
                    } else {
                        apply_creation_change(row,&mut next,&CreationChange::Receipt(CreationReceipt::Sandbox(remote.id.clone())))?;
                    }
                },
                Change::Release => {
                    let row = creation.as_mut().ok_or_else(drift)?;
                    if row.submission_state != "confirmed_not_created" || row.released_by_inventory || current.cma_sandbox_id.is_some() {
                        return Err(AppError::Conflict("cma_creation_absence_proof_required: CMA has not provided authoritative proof that this creation was never made".into()));
                    }
                    apply_creation_change(row, &mut next, &CreationChange::NoResource)?;
                },
            }
            let mut set = sandboxes::observation_set(&next)?;
            set.insert("retire_requested_at", next.retire_requested_at.map(bson::DateTime::from_chrono));
            set.insert("label", &next.label);
            set.insert("operation_id", next.operation_id.clone());
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

async fn require_linked_inventory(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    parent: &AssistantCmaSandbox,
    live_ids: &[String],
) -> AppResult<()> {
    let links =
        db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME);
    for batch in live_ids.chunks(100) {
        let mut missing: std::collections::HashSet<&str> =
            batch.iter().map(String::as_str).collect();
        let mut cursor = links
            .find(doc! {
                "user_id": &parent.user_id, "cma_service_id": &parent.cma_service_id,
                "cma_sandbox_id": {"$in": batch}, "admission_state": {"$ne":"retired"},
            })
            .projection(doc! {"_id":0, "cma_sandbox_id":1})
            .session(&mut *tx)
            .await?;
        let mut stream = cursor.stream(&mut *tx);
        while let Some(link) = stream.try_next().await? {
            if let Ok(id) = link.get_str("cma_sandbox_id") {
                missing.remove(id);
            }
        }
        if !missing.is_empty() {
            return Err(AppError::Conflict("cma_inventory_unlinked_sandbox".into()));
        }
    }
    Ok(())
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

/// Observe the immutable covered receipt before settling through both row revisions.
pub(crate) async fn recover_covered(
    db: &Database,
    gateway: &dyn CmaGateway,
    parent: &AssistantCmaSandbox,
    creation: &AssistantCmaCreation,
) -> AppResult<&'static str> {
    use super::creation_outcome::Evidence;
    require_parent(creation, parent)?;
    if !parent.occupies_account_slot || !creation.creates_sandbox {
        return Ok("unchanged");
    }
    let (change, reason) = match super::creation_outcome::lookup(gateway, creation).await? {
        Evidence::Absent { operation } => (
            Change::CoveredAbsent(operation),
            "creation_confirmed_not_created",
        ),
        Evidence::Created { sandbox, operation } => {
            let remote = super::super::assistant_cma_reservation_service::exact_sandbox(
                gateway,
                &parent.user_id,
                &sandbox,
            )
            .await?;
            if parent.cma_sandbox_id.as_deref() == Some(&sandbox)
                && remote.state != SandboxObservationState::Retired
            {
                return recover_receipt(db, gateway, creation).await;
            }
            (
                Change::CoveredCreated(remote, operation),
                "creation_confirmed",
            )
        }
        Evidence::Progress {
            operation: Some(operation),
        } if creation.operation_id.is_none() => {
            (Change::CoveredProgress(operation), "creation_in_progress")
        }
        Evidence::Progress { .. } => return Ok("creation_in_progress"),
        Evidence::Unknown => return Ok("creation_outcome_unknown"),
    };
    let expected_sandbox = match &change {
        Change::CoveredCreated(remote, _) => Some(remote.id.clone()),
        _ => None,
    };
    let expected_operation = match &change {
        Change::CoveredCreated(_, operation) | Change::CoveredAbsent(operation) => {
            operation.clone()
        }
        Change::CoveredProgress(operation) => Some(operation.clone()),
        _ => None,
    };
    let result = commit_recovery(db,parent,Some(creation),change,serde_json::json!({
        "reservation_id":parent.id,"owner_id":parent.user_id,"cma_service_id":parent.cma_service_id,
        "generation":parent.generation,"action":"covered_creation_lookup","result":reason
    })).await;
    match result {
        Ok(_) => Ok(reason),
        Err(error) => {
            // The transaction may have committed before its acknowledgement was lost.
            let durable = super::super::transaction_service::durable_database(db);
            let observed = sandboxes::get_owned(&durable, &parent.user_id, &parent.id).await?;
            let saved = durable
                .collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .find_one(creation_filter(creation))
                .await?;
            if saved.as_ref().is_some_and(|row| {
                require_parent(row, &observed).is_ok()
                    && observed.idempotency_key.as_deref() == Some(&row.idempotency_key)
                    && observed.request_hash.as_deref() == Some(&row.request_hash)
                    && !row.released_by_inventory
                    && row.revision > creation.revision
                    && row.operation_id == expected_operation
                    && observed.operation_id == expected_operation
                    && ((reason == "creation_in_progress" && observed.occupies_account_slot)
                        || (reason == "creation_confirmed_not_created"
                            && row.submission_state == "confirmed_not_created"
                            && observed.submission_state == "confirmed_not_created"
                            && row.cma_agent_id.is_none()
                            && row.cma_sandbox_id.is_none()
                            && observed.cma_sandbox_id.is_none()
                            && !observed.occupies_account_slot)
                        || (reason == "creation_confirmed"
                            && row.cma_sandbox_id.is_some()
                            && row.cma_sandbox_id == expected_sandbox
                            && row.cma_sandbox_id == observed.cma_sandbox_id))
            }) {
                Ok(reason)
            } else {
                Err(error)
            }
        }
    }
}

/// Explicit owner action. An uncertain cancellation never releases a reservation.
pub(crate) async fn cancel_covered(
    db: &Database,
    gateway: &dyn CmaGateway,
    owner: &str,
    id: &str,
    generation: i64,
) -> AppResult<sandboxes::SandboxCommandDto> {
    let db = &super::super::transaction_service::durable_database(db);
    let mut parent = sandboxes::get_owned(db, owner, id).await?;
    sandboxes::require_generation(&parent, generation)?;
    let mut creation = creation_for_sandbox(db, &parent).await?.ok_or_else(drift)?;
    require_parent(&creation, &parent)?;
    if creation.covered_key.is_none() {
        return Err(AppError::Conflict("This creation predates durable evidence; recover the original request or exact Sandbox retirement".into()));
    }
    if parent.occupies_account_slot
        && creation.operation_id.is_none()
        && creation.path == "/api/v1/agents"
    {
        recover_covered(db, gateway, &parent, &creation).await?;
        parent = sandboxes::get_owned(db, owner, id).await?;
        sandboxes::require_generation(&parent, generation)?;
        creation = creation_for_sandbox(db, &parent).await?.ok_or_else(drift)?;
        require_parent(&creation, &parent)?;
    }
    if !parent.occupies_account_slot && creation.submission_state == "confirmed_not_created" {
        return Ok(sandboxes::SandboxCommandDto {
            sandbox: sandboxes::SandboxDto::from(&parent),
            pending: false,
        });
    }
    if parent.cma_sandbox_id.is_some()
        || creation.cma_sandbox_id.is_some()
        || creation.cma_agent_id.is_some()
        || creation.path != "/api/v1/agents"
    {
        return Err(drift());
    }
    let operation = creation.operation_id.as_deref().ok_or_else(drift)?;
    verify_creation(gateway, &creation).await?;
    let result = gateway.cancel_creation(owner, operation).await;
    // Even an uncertain HTTP reply may have committed cancellation. The lookup
    // remains the authority, and both original revisions fence settlement.
    let outcome = recover_covered(db, gateway, &parent, &creation).await;
    let after = sandboxes::get_owned(db, owner, id).await?;
    sandboxes::require_generation(&after, generation)?;
    if after.occupies_account_slot {
        result?;
        outcome?;
    }
    Ok(sandboxes::SandboxCommandDto {
        pending: after.occupies_account_slot,
        sandbox: sandboxes::SandboxDto::from(&after),
    })
}
