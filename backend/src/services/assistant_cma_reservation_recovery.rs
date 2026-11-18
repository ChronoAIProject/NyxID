//! Bounded, read-only CMA reconciliation independent of owner activity.
use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde_json::json;

use super::{
    assistant_cma_admission::reservation_admin as admission,
    assistant_cma_gateway::{CatalogCmaGateway, CmaGateway},
    assistant_cma_reservation_service as reservations,
    assistant_cma_sandbox_service::NYXID_CREATED_SOURCE,
    coordination_service::{LeaseStore, ReplayStore, cluster_lease_runtime},
};
use crate::{
    AppState,
    errors::AppResult,
    models::assistant_cma_sandbox::{
        AssistantCmaSandbox, COLLECTION_NAME, SandboxAdmissionState, SandboxObservationState,
    },
};

const LEASE: &str = "assistant:cma-reservation-recovery";
const PAGE: i64 = 16;
const ROW_BUDGET: Duration = Duration::from_secs(15);
const PAGE_BUDGET: Duration = Duration::from_secs(30);
const REST_MILLIS: i64 = 300_000;

pub fn spawn_worker(state: AppState) {
    state.shutdown.clone().spawn_worker(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while let Some(_work) = state.shutdown.tick(&mut interval).await {
            if sweep(&state).await.is_err() {
                tracing::warn!("CMA reservation recovery pass deferred");
            }
        }
    });
}

/// The lease and cursor bound cost across replicas and keep one bad owner from
/// starving later rows. A cancelled read/transaction is safe to re-observe.
pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    let Some(_claim) = state.shutdown.background() else {
        return Ok(());
    };
    let db = super::transaction_service::durable_database(&state.db);
    let runtime = cluster_lease_runtime();
    let Some((token, checkpoint)) =
        LeaseStore::acquire_with_checkpoint(&db, LEASE, &runtime.holder, runtime.ttl).await?
    else {
        return Ok(());
    };
    let result: Option<AppResult<()>> = runtime
        .run_while_renewed(&db, &token, async {
            let checkpoint = checkpoint
                .and_then(|v| v.as_document().cloned())
                .unwrap_or_default();
            if checkpoint
                .get_datetime("resume_at")
                .is_ok_and(|at| at.timestamp_millis() > bson::DateTime::now().timestamp_millis())
            {
                return Ok(());
            }
            let mut filter = doc! {"source": NYXID_CREATED_SOURCE, "occupies_account_slot": true};
            if let Ok(after) = checkpoint.get_str("after") {
                filter.insert("_id", doc! {"$gt":after});
            }
            let rows: Vec<AssistantCmaSandbox> = db
                .collection(COLLECTION_NAME)
                .find(filter)
                .sort(doc! {"_id":1})
                .limit(PAGE)
                .await?
                .try_collect()
                .await?;
            let deadline = tokio::time::Instant::now() + PAGE_BUDGET;
            let count = rows.len();
            let mut completed = 0;
            for row in rows {
                if state.shutdown.is_draining() || tokio::time::Instant::now() >= deadline {
                    return Ok(());
                }
                let gateway = CatalogCmaGateway::for_service(state, &row.cma_service_id);
                let result =
                    tokio::time::timeout(ROW_BUDGET, recover_one(&db, &gateway, &row)).await;
                let reason = match result {
                    Ok(Ok(reason)) => reason,
                    Ok(Err(_)) => "observation_or_fence_unavailable",
                    Err(_) => "observation_timeout",
                };
                if !matches!(reason, "unchanged" | "retirement_confirmed") {
                    diagnostic(&db, &row, reason).await;
                }
                // Losing the lease stops new reads; a repeat after a crash is safe.
                if !LeaseStore::store_checkpoint(&db, &token, doc! {"after":&row.id}.into()).await?
                {
                    return Ok(());
                }
                completed += 1;
            }
            if completed == count && count < PAGE as usize {
                let resume = bson::DateTime::from_millis(
                    bson::DateTime::now().timestamp_millis() + REST_MILLIS,
                );
                LeaseStore::store_checkpoint(&db, &token, doc! {"resume_at":resume}.into()).await?;
            }
            Ok(())
        })
        .await;
    let release = LeaseStore::release(&db, &token).await;
    if let Some(result) = result {
        result?;
    }
    release?;
    Ok(())
}

pub(super) async fn recover_one(
    db: &mongodb::Database,
    gateway: &dyn CmaGateway,
    row: &AssistantCmaSandbox,
) -> AppResult<&'static str> {
    if row.source != NYXID_CREATED_SOURCE || !row.occupies_account_slot {
        return Ok("unchanged");
    }
    if super::erasure_admission::is_erasure_accepted(db, &row.user_id).await? {
        return Ok("owner_erasure_in_progress");
    }
    let creation = admission::creation_for_sandbox(db, row).await?;
    if let Some(id) = &row.cma_sandbox_id
        && (row.retire_requested_at.is_some()
            || row.admission_state == SandboxAdmissionState::Retiring)
    {
        reservations::verify(gateway, row, creation.as_ref()).await?;
        let remote = reservations::exact_sandbox(gateway, &row.user_id, id).await?;
        if remote.state != SandboxObservationState::Retired {
            return Ok("retirement_not_confirmed");
        }
        let data = json!({"reservation_id":row.id,"owner_id":row.user_id,"cma_service_id":row.cma_service_id,
            "generation":row.generation,"action":"confirm_retirement","result":"resolved"});
        admission::commit_recovery(
            db,
            row,
            creation.as_ref(),
            admission::Change::ConfirmRetirement(remote),
            data,
        )
        .await?;
        return Ok("retirement_confirmed");
    }
    if let Some(creation) = creation
        && !matches!(
            creation.submission_state.as_str(),
            "confirmed" | "confirmed_not_created"
        )
        && (creation.operation_id.is_some()
            || creation.cma_agent_id.is_some()
            || creation.cma_sandbox_id.is_some())
    {
        return admission::recover_receipt(db, gateway, &creation).await;
    }
    Ok("unchanged")
}

async fn diagnostic(db: &mongodb::Database, row: &AssistantCmaSandbox, reason: &'static str) {
    let identity = format!("{}:{}:{reason}", row.id, row.generation);
    if matches!(
        ReplayStore::claim(db, LEASE, &identity, Duration::from_secs(3600)).await,
        Ok(true)
    ) {
        let event = if reason == "creation_confirmed" {
            "cma_sandbox_reservation_recovered"
        } else {
            "cma_sandbox_reservation_recovery_deferred"
        };
        if super::audit_service::log_system_event(
            db.clone(),
            event,
            Some(json!({
                "reservation_id":row.id,"owner_id":row.user_id,"cma_service_id":row.cma_service_id,
                "generation":row.generation,"reason":reason,
            })),
        )
        .await
        .is_err()
        {
            tracing::warn!("CMA reservation recovery diagnostic unavailable");
        }
    }
}
