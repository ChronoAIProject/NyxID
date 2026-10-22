//! Vendor completion is relative to current durable inventory and dispatch
//! evidence. A late registry discovery invalidates the personal checkpoint too.
use super::*;
use crate::models::telemetry_erasure_job::{TelemetryErasureJob, TelemetryErasureStatus};
use futures::TryStreamExt;

pub(super) enum Pass {
    Complete(String),
    Pending,
    Action,
}
pub(super) async fn pass(db: &Database, user: &str) -> AppResult<Pass> {
    let db = super::super::transaction_service::durable_database(db);
    if let Some(evidence) =
        super::super::telemetry_erasure_service::completion_evidence(&db, user).await?
    {
        return Ok(Pass::Complete(evidence));
    }
    super::super::telemetry_erasure_service::wake_parent(&db, user).await?;
    let parent = db
        .collection::<TelemetryErasureJob>(crate::models::telemetry_erasure_job::COLLECTION_NAME)
        .find_one(doc! {"user_id":user,"destination_id":bson::Bson::Null})
        .await?
        .ok_or_else(|| AppError::Internal("Durable vendor erasure intent is unavailable".into()))?;
    Ok(match parent.status {
        TelemetryErasureStatus::Failed | TelemetryErasureStatus::OperatorAction => Pass::Action,
        _ => Pass::Pending,
    })
}

/// Status remains truthful even before the bounded background walk revisits a
/// completed job. Invalidation cannot overwrite an active worker's checkpoint.
pub(super) async fn refresh(
    db: &Database,
    mut job: PersonalErasureJob,
) -> AppResult<PersonalErasureJob> {
    let Some(index) = job
        .domains
        .iter()
        .position(|d| d.domain == "vendor" && d.status == DomainStatus::Complete)
    else {
        return Ok(job);
    };
    let evidence =
        super::super::telemetry_erasure_service::completion_evidence(db, &job.user_id).await?;
    if evidence.is_some() && evidence.as_deref() == job.domains[index].cursor.as_deref() {
        return Ok(job);
    }
    let original = bson::to_bson(&job.domains)
        .map_err(|_| AppError::Internal("Erasure evidence encoding failed".into()))?;
    job.domains[index].status = DomainStatus::Pending;
    job.domains[index].completed_at = None;
    job.domains[index].cursor = None;
    job.domains[index].reason = Some("vendor_inventory_or_dispatch_changed".into());
    job.status = ErasureStatus::Pending;
    job.completed_at = None;
    let updated = bson::to_bson(&job.domains)
        .map_err(|_| AppError::Internal("Erasure evidence encoding failed".into()))?;
    let durable = super::super::transaction_service::durable_database(db);
    durable.collection::<Document>(JOBS).update_one(doc! {"_id":&job.id,"domains":original,
        "$expr":{"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}},
        vec![doc! {"$set":{"domains":{"$literal":updated},"status":"pending","completed_at":bson::Bson::Null,
            "updated_at":"$$NOW","retry_at":"$$NOW","lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null}}]).await?;
    super::super::telemetry_erasure_service::wake_parent(&durable, &job.user_id).await?;
    // Even a lost CAS cannot attest stale completion to this reader.
    Ok(job)
}

/// Bounded keyset walk revisits retained completion evidence after source
/// removal, including when nobody requests the account's status again.
pub(super) async fn revisit(db: &Database) -> AppResult<()> {
    let db = super::super::transaction_service::durable_database(db);
    let progress = db.collection::<Document>("security_remediation_progress");
    let id = "personal-erasure-vendor-evidence";
    let cursor = progress
        .find_one(doc! {"_id":id})
        .await?
        .and_then(|row| row.get_str("cursor").ok().map(str::to_owned))
        .unwrap_or_default();
    let jobs: Vec<PersonalErasureJob> = db.collection(JOBS)
        .find(doc! {"_id":{"$gt":&cursor},"domains":{"$elemMatch":{"domain":"vendor","status":"complete"}}})
        .sort(doc! {"_id":1}).limit(16).await?.try_collect().await?;
    let next = jobs.last().map(|job| job.id.clone()).unwrap_or_default();
    for job in jobs {
        refresh(&db, job).await?;
    }
    progress
        .update_one(
            doc! {"_id":id},
            vec![doc! {"$set":{"cursor":{"$literal":next},"updated_at":"$$NOW"}}],
        )
        .upsert(true)
        .await?;
    Ok(())
}
