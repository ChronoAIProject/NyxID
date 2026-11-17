//! Startup cutoff and read-only proof for untracked, pre-existing CMA Sandboxes.
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};

use super::{
    assistant_cma_gateway::{CatalogCmaGateway, CmaSandboxProvenance},
    assistant_cma_sandbox_service as sandboxes,
    transaction_service::durable_database,
};
use crate::errors::{AppError, AppResult};

pub(super) const COLLECTION: &str = "assistant_migrations";
pub(super) const MARKER: &str = "cma-legacy-sandboxes-v1";

/// Called before serving; requests never create or repair this marker.
pub(crate) async fn initialize(db: &Database) -> AppResult<()> {
    let db = durable_database(db);
    if db
        .collection::<bson::Document>(COLLECTION)
        .find_one(doc! {"_id": MARKER})
        .await?
        .is_some()
    {
        return Ok(());
    }
    let earliest = db
        .collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .find_one(doc! {"source": sandboxes::NYXID_CREATED_SOURCE})
        .sort(doc! {"created_at": 1})
        .projection(doc! {"created_at": 1})
        .await?
        .map(|row| {
            row.get_datetime("created_at")
                .copied()
                .map_err(|_| AppError::Internal("Invalid CMA tracking cutoff evidence".into()))
        })
        .transpose()?;
    insert_cutoff(&db, earliest).await?;
    Ok(())
}

// Each racing candidate supplies both fields in one atomic write. Missing
// differs from null: an existing malformed marker is not silently repaired.
pub(super) async fn insert_cutoff(
    db: &Database,
    earliest: Option<bson::DateTime>,
) -> Result<mongodb::results::UpdateResult, mongodb::error::Error> {
    let (cutoff, rule) = match earliest {
        Some(at) => (bson::Bson::DateTime(at), "earliest_creation"),
        None => (bson::Bson::String("$$NOW".into()), "database_time"),
    };
    durable_database(db).collection::<bson::Document>(COLLECTION)
        .update_one(doc! {"_id": MARKER}, vec![doc! {"$set": {
            "cutoff": {"$cond": [{"$eq": [{"$type": "$cutoff"}, "missing"]}, cutoff, "$cutoff"]},
            "rule": {"$cond": [{"$eq": [{"$type": "$rule"}, "missing"]}, rule, "$rule"]},
            "written_at": {"$cond": [{"$eq": [{"$type": "$written_at"}, "missing"]}, "$$NOW", "$written_at"]},
        }}]).upsert(true).await
}

async fn cutoff(db: &Database) -> Option<DateTime<Utc>> {
    let row = durable_database(db)
        .collection::<bson::Document>(COLLECTION)
        .find_one(doc! {"_id": MARKER})
        .projection(doc! {"cutoff": 1, "rule": 1, "observed_at": "$$NOW"})
        .await
        .ok()??;
    if !matches!(
        row.get_str("rule").ok()?,
        "earliest_creation" | "database_time"
    ) {
        return None;
    }
    let cutoff = *row.get_datetime("cutoff").ok()?;
    // Database time also excludes corrupt future cutoffs, without another read.
    (cutoff <= *row.get_datetime("observed_at").ok()?).then(|| cutoff.to_chrono())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyRuntimeEnrollmentPolicy {
    Refuse,
    AllowProspectiveBilling,
}

// CMA 68f26f17fd18a701958a27ea5b2b933227fa2fbf (paths under api/crates/):
// cma/src/store/{agent_admission.rs:356,workspace_admission.rs:323} revive parents;
// cma/src/sandbox/worker/lifecycle/restore.rs:96 requires funded coverage;
// layer2-core/src/sandbox/agent_context.rs:99 and cma/src/commands/preparation.rs:15
// prepare Messages; layer2-core/src/sandbox/agent_io/interruption/service.rs:38
// queues a replacement Message. An active-state read cannot rule out revival.
const OPERATIONS_THAT_CAN_CAUSE_RUNTIME_ENROLLMENT: &[&str] = &[
    "/api/v1/agents",
    "/api/v1/workspaces/{id}/agents",
    "/api/v1/sandboxes/{id}/workspaces",
    "/api/v1/sandboxes/{id}/workspaces/inline",
    "/api/v1/sandboxes/{id}/revive",
    "/api/v1/agents/{id}/input",
    "/api/v1/responses",
    "/api/v2/responses",
    "/api/v2/agents/{id}/responses",
    "/api/v2/agents/{id}/responses/{response_id}/interrupt",
];

fn legacy_runtime_enrollment_policy() -> LegacyRuntimeEnrollmentPolicy {
    // Owner decision, 2026-11-17: the same personal Sandbox owner may enter
    // ordinary CMA quote/window funding prospectively at the existing rate card.
    // No historical charge or new NyxID billing path is introduced here.
    LegacyRuntimeEnrollmentPolicy::AllowProspectiveBilling
}

fn can_enroll(method: &str, path: &str) -> bool {
    method == "POST"
        && OPERATIONS_THAT_CAN_CAUSE_RUNTIME_ENROLLMENT
            .iter()
            .any(|pattern| {
                let mut actual = path.split('/');
                pattern.split('/').all(|part| {
                    actual.next().is_some_and(|value| {
                        part == value || (part.starts_with('{') && !value.is_empty())
                    })
                }) && actual.next().is_none()
            })
}

pub(super) async fn verify(
    db: &Database,
    gateway: &CatalogCmaGateway,
    owner: &str,
    remote_id: &str,
    observed: Option<CmaSandboxProvenance>,
    method: &str,
    path: &str,
) -> AppResult<()> {
    let cutoff = cutoff(db).await.ok_or_else(sandboxes::missing_creation)?;
    let observed = match observed {
        Some(observed) => observed,
        None => gateway
            .get_sandbox_provenance(owner, remote_id)
            .await
            .map_err(|_| sandboxes::missing_creation())?,
    };
    if observed.id != remote_id || !observed.created_before(owner, cutoff) {
        return Err(sandboxes::missing_creation());
    }
    if can_enroll(method, path)
        && legacy_runtime_enrollment_policy() == LegacyRuntimeEnrollmentPolicy::Refuse
    {
        return Err(sandboxes::missing_creation());
    }
    Ok(())
}

#[cfg(test)]
#[path = "assistant_cma_legacy_service_tests.rs"]
mod tests;
