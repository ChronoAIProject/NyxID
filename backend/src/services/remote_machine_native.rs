//! Agent Sandbox / Kubernetes native lifecycle contract. These are control-plane
//! observations, never signatures, physical shutdown times or CSI erasure proofs.
use crate::{
    errors::{AppError, AppResult},
    models::remote_machine::{
        COLLECTION_NAME as MACHINES, ManagedMachine, NativeRuntimeObservation, ResourceState,
    },
};
use chrono::{DateTime, Duration, Utc};
use mongodb::{
    Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};

pub const MAX_OBSERVATION_AGE_SECONDS: i64 = 60;
fn pending() -> AppError {
    AppError::Conflict("native_runtime_observation_pending".into())
}

pub fn binding_matches(machine: &ManagedMachine, observation: &NativeRuntimeObservation) -> bool {
    machine.sandbox_uid.as_deref() == Some(observation.sandbox_uid.as_str())
        && machine.pod_uid == observation.pod_uid
        && machine.runtime_generation == observation.runtime_generation
        && machine.credential_epoch == observation.credential_epoch
        && machine.writer_fence == observation.writer_fence
        && observation.sandbox_generation > 0
        && !observation.sandbox_resource_version.is_empty()
        && !observation.evidence_ref.is_empty()
}
pub fn fresh(observation: &NativeRuntimeObservation, now: DateTime<Utc>) -> bool {
    observation.observed_at <= now + Duration::seconds(1)
        && observation.observed_at >= now - Duration::seconds(MAX_OBSERVATION_AGE_SECONDS)
}
pub fn current_running(
    machine: &ManagedMachine,
    observation: &NativeRuntimeObservation,
    now: DateTime<Utc>,
) -> bool {
    machine.native_uncertainty.is_none()
        && binding_matches(machine, observation)
        && machine.active_operation_id.as_deref() == Some(observation.operation_id.as_str())
        && fresh(observation, now)
        && observation.running
        && !observation.suspended
        && !observation.stop_confirmed
        && observation.pod_uid.is_some()
}
/// An owner resume changes operation ID, not the already completed predecessor.
/// A matching durable stop can survive that change; a new Pod cannot replace it.
pub fn predecessor_stopped(machine: &ManagedMachine) -> bool {
    machine.native_uncertainty.is_none()
        && machine.native_observation.as_ref().is_some_and(|o| {
            binding_matches(machine, o) && o.stop_confirmed && o.suspended && !o.running
        })
}

/// Latch known uncertainty before returning an observation failure. No later
/// NotFound/healthy replacement clears this row; operators investigate it.
pub async fn quarantine(
    db: &Database,
    machine: &ManagedMachine,
    reason: &'static str,
) -> AppResult<()> {
    // A late read may discover uncertainty after owner pause/delete commits.
    // Preserve the live lifecycle state while latching the same runtime fence.
    db.collection::<Document>(MACHINES).update_one(
        doc!{"_id":&machine.node_id,"runtime_generation":machine.runtime_generation,"credential_epoch":machine.credential_epoch,"writer_fence":&machine.writer_fence},
        vec![doc!{"$set":{
            "native_uncertainty":{"$ifNull":["$native_uncertainty",reason]},
            "writer_admitted":false,
            "resource_state":{"$cond":[{"$in":["$resource_state",["deleting","stopping","paused","deleted"]]},"$resource_state","reconciling"]},
            "stop_reason":{"$ifNull":["$stop_reason",reason]},
            "updated_at":bson::DateTime::now(),
            "revision":{"$add":["$revision",1_i64]}
        }}]
    ).await?;
    Ok(())
}

/// Called only after the Manager's exact API observation checks. Replaying an
/// older worker cannot overwrite a later accepted operation or generation.
pub async fn record(
    db: &Database,
    machine: &ManagedMachine,
    observation: &NativeRuntimeObservation,
    bindings: Document,
) -> AppResult<ManagedMachine> {
    if machine.native_uncertainty.is_some()
        || !binding_matches(machine, observation)
        || machine.active_operation_id.as_deref() != Some(observation.operation_id.as_str())
        || !fresh(observation, Utc::now())
    {
        return Err(pending());
    }
    let mut set = bindings;
    set.insert(
        "native_observation",
        bson::to_bson(observation).map_err(|_| pending())?,
    );
    set.insert(
        "updated_at",
        bson::DateTime::from_chrono(observation.observed_at),
    );
    if !observation.running {
        set.insert("writer_admitted", false);
    }
    db.collection::<ManagedMachine>(MACHINES).find_one_and_update(
        doc!{"_id":&machine.node_id,"revision":machine.revision,"runtime_generation":machine.runtime_generation,"credential_epoch":machine.credential_epoch,"active_operation_id":&machine.active_operation_id,"native_uncertainty":bson::Bson::Null},
        doc!{"$set":set,"$inc":{"revision":1_i64}}
    ).return_document(ReturnDocument::After).await?.ok_or_else(pending)
}

/// Bootstrap admission precedes application Ready, which still requires the
/// exact Node WSS authentication/capability handshake. This avoids a probe loop.
pub async fn activate(db: &Database, machine: &ManagedMachine) -> AppResult<ManagedMachine> {
    let observation = machine.native_observation.as_ref().ok_or_else(pending)?;
    if !current_running(machine, observation, Utc::now())
        || !super::remote_machine_runtime::execution_coverage_is_live(machine, Utc::now())
        || !matches!(
            machine.resource_state,
            ResourceState::Provisioning
                | ResourceState::Resuming
                | ResourceState::Reconciling
                | ResourceState::Ready
        )
    {
        return Err(pending());
    }
    db.collection::<ManagedMachine>(MACHINES).find_one_and_update(
        doc!{"_id":&machine.node_id,"revision":machine.revision,"native_observation":bson::to_bson(observation).map_err(|_|pending())?,"native_uncertainty":bson::Bson::Null},
        doc!{"$set":{"writer_admitted":true,"authorization_state":"active","updated_at":bson::DateTime::now()},"$inc":{"revision":1_i64}}
    ).return_document(ReturnDocument::After).await?.ok_or_else(pending)
}

/// Resource cleanup is separately persisted from Billing closure. The caller
/// supplies a reference only after checking bound Sandbox/Pod/PVC/PV absence and
/// Delete reclaim semantics. It does not assert physical erasure.
pub async fn finalize_stop(
    db: &Database,
    machine: &ManagedMachine,
    cleanup_ref: Option<&str>,
) -> AppResult<ManagedMachine> {
    if !predecessor_stopped(machine) {
        return Err(pending());
    }
    let observation = machine.native_observation.as_ref().ok_or_else(pending)?;
    if machine.active_operation_id.as_deref() != Some(observation.operation_id.as_str()) {
        return Err(pending());
    }
    let deleting = machine.resource_state == ResourceState::Deleting;
    if deleting && cleanup_ref.is_none_or(str::is_empty) {
        return Err(pending());
    }
    if !deleting
        && !matches!(
            machine.resource_state,
            ResourceState::Stopping | ResourceState::Reconciling | ResourceState::Paused
        )
    {
        return Err(pending());
    }
    if deleting {
        let r=db.collection::<Document>(MACHINES).update_one(doc!{"_id":&machine.node_id,"revision":machine.revision,"resource_state":"deleting","pvc_uid":&machine.pvc_uid,"native_pv_uid":&machine.native_pv_uid,"native_pv_reclaim_policy":&machine.native_pv_reclaim_policy},
            doc!{"$set":{"cleanup_state":"complete","cleanup_evidence_digest":cleanup_ref,"updated_at":bson::DateTime::now()}}).await?;
        if r.matched_count != 1 {
            return Err(pending());
        }
    }
    let db = db.clone();
    let machine = machine.clone();
    let observation = observation.clone();
    let cleanup_ref = cleanup_ref.map(str::to_owned);
    db.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
        let result:AppResult<ManagedMachine>=async {
            if deleting && !super::remote_machine_service::deletion_authorized_in_session(&db,session,&machine.node_id).await? {return Err(pending());}
            super::remote_machine_runtime::require_accounting_final_in_session(&db,session,&machine).await?;
            let now=bson::DateTime::now();
            let updated=db.collection::<ManagedMachine>(MACHINES).find_one_and_update(
                doc!{"_id":&machine.node_id,"revision":machine.revision,"runtime_generation":machine.runtime_generation,"credential_epoch":machine.credential_epoch,"writer_fence":&machine.writer_fence,"native_observation":bson::to_bson(&observation).map_err(|_|pending())?,"native_uncertainty":bson::Bson::Null,"pvc_uid":&machine.pvc_uid,"native_pv_uid":&machine.native_pv_uid,"native_pv_reclaim_policy":&machine.native_pv_reclaim_policy},
                doc!{"$set":{"resource_state":if deleting{"deleted"}else{"paused"},"writer_admitted":false,"billing_state":if deleting{"closed"}else{"suspended"},"authorization_state":"revoked","cleanup_state":if deleting{"complete"}else{"not_required"},"updated_at":now},"$inc":{"revision":1_i64}}
            ).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(pending)?;
            if deleting {
                let released=db.collection::<Document>(super::remote_machine_service::FREE_SLOTS).update_one(
                    doc!{"_id":&machine.owner_id,"machine_id":&machine.node_id,"state":{"$in":["reserved","active","releasing"]}},
                    doc!{"$set":{"state":"released","release_evidence_digest":&cleanup_ref,"released_at":now,"updated_at":now},"$inc":{"revision":1_i64}}
                ).session(&mut *session).await?;
                if machine.free_slot && released.matched_count!=1{return Err(pending());}
            }
            Ok(updated)
        }.await;super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)
}
