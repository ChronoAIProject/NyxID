//! Organization-only scans. No global or population job is ever queued.
use super::*;
use crate::models::execution_enrollment_job::{
    COLLECTION_NAME as JOBS, ExecutionEnrollmentJob as Job,
};
use chrono::{Duration, Utc};
use mongodb::options::{IndexOptions, ReturnDocument, WriteConcern};
use uuid::Uuid;

const BATCH: i64 = 100;
const LEASE_SECONDS: i64 = 30;

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    lifecycle::ensure_indexes(db).await?;
    db.collection::<Job>(JOBS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"org_id":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("execution_enrollment_org".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Job>(JOBS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"state":1,"lease_until":1,"updated_at":1})
                .options(
                    IndexOptions::builder()
                        .name("execution_enrollment_claim".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Job>(JOBS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"updated_at":-1,"_id":1})
                .options(
                    IndexOptions::builder()
                        .name("execution_enrollment_progress".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub(crate) async fn enqueue(
    db: &Database,
    org: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Job>(JOBS).update_one(doc! {"org_id":org}, doc! {
        "$set":{"generation":Uuid::new_v4().to_string(),"state":"pending","cursor":null,
            "processed":0_i64,"lease_token":null,"lease_until":null,"last_error":null,"updated_at":bson::DateTime::now()},
        "$setOnInsert":{"_id":Uuid::new_v4().to_string(),"org_id":org}
    }).upsert(true).session(session).await?;
    Ok(())
}

pub(super) async fn claim(db: &Database) -> AppResult<Option<Job>> {
    let now = Utc::now();
    // A periodic org-only repair catches an enable racing a membership insert
    // behind a scan cursor. This is eventual rollout convergence, not an effect
    // fence. It never walks people outside an explicitly configured org job.
    db.collection::<Job>(JOBS).update_one(doc! {
        "state":"complete","lease_until":{"$lte":bson::DateTime::from_chrono(now)}
    }, doc! {"$set":{"state":"pending","cursor":null,"processed":0_i64,"lease_until":null,"updated_at":bson::DateTime::from_chrono(now)}}).await?;
    Ok(db.collection::<Job>(JOBS).find_one_and_update(doc! {
        "state":{"$in":["pending","running"]},"$or":[{"lease_until":null},{"lease_until":{"$lte":bson::DateTime::from_chrono(now)}}]
    }, doc! {"$set":{"state":"running","lease_token":Uuid::new_v4().to_string(),
        "lease_until":bson::DateTime::from_chrono(now + Duration::seconds(LEASE_SECONDS)),"updated_at":bson::DateTime::from_chrono(now)}})
        .sort(doc! {"updated_at":1,"_id":1}).return_document(ReturnDocument::After).await?)
}

fn lease(job: &Job) -> Document {
    doc! {"_id":&job.id,"generation":&job.generation,"state":"running","lease_token":&job.lease_token,
    "lease_until":{"$gt":bson::DateTime::now()}}
}

pub(super) async fn batch(db: &Database, job: &Job) -> AppResult<bool> {
    let db = db.clone();
    let job = job.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().write_concern(WriteConcern::majority())
        .and_run2(async move |session| {
            transactions::transaction_result(async {
                if db.collection::<Job>(JOBS).find_one(lease(&job)).session(&mut *session).await?.is_none() { return Ok(false); }
                let mut filter = doc! {"org_user_id":&job.org_id};
                // Include soft-revoked members so a disable also repairs old hints.
                if let Some(cursor) = &job.cursor { filter.insert("member_user_id", doc! {"$gt":cursor}); }
                let people: Vec<OrgMembership> = db.collection::<OrgMembership>(MEMBERSHIPS).find(filter)
                    .sort(doc! {"member_user_id":1}).limit(BATCH).max_time(READ_TIMEOUT)
                    .session(&mut *session).await?.stream(&mut *session).try_collect().await?;
                for person in &people { recompute_in_session(&db, &person.member_user_id, session).await?; }
                let complete = people.len() < BATCH as usize;
                if complete && db.collection::<Document>(USERS).find_one(doc! {"_id":&job.org_id,"user_type":"org"}).session(&mut *session).await?.is_none() {
                    let removed = db.collection::<Job>(JOBS).delete_one(lease(&job)).session(session).await?;
                    if removed.deleted_count != 1 { return Err(AppError::Conflict("Enrollment reconciliation lease changed".into())); }
                    return Ok(true);
                }
                let cursor = people.last().map(|p| p.member_user_id.clone()).or_else(|| job.cursor.clone());
                let updated = db.collection::<Job>(JOBS).update_one(lease(&job), doc! {
                    "$set":{"state":if complete {"complete"} else {"pending"},"cursor":cursor,
                        "lease_token":null,"lease_until":if complete { Some(bson::DateTime::from_chrono(Utc::now() + Duration::minutes(5))) } else { None },"last_error":null,"updated_at":bson::DateTime::now()},
                    "$inc":{"processed":people.len() as i64}
                }).session(session).await?;
                if updated.matched_count != 1 { return Err(AppError::Conflict("Enrollment reconciliation lease changed".into())); }
                Ok(true)
            }.await)
        }).await.map_err(transactions::map_transaction_error)
}

pub async fn run(db: Database) {
    loop {
        if lifecycle::repair_due(&db).await.is_err() {
            tracing::warn!(
                reason = "person_repair_failed",
                "Execution enrollment repair delayed"
            );
        }
        match claim(&db).await {
            Ok(Some(job)) => {
                if batch(&db, &job).await.is_err() {
                    // Never persist error text (which may contain database values).
                    let _ = db.collection::<Job>(JOBS).update_one(lease(&job), doc! {
                        "$set":{"state":"pending","lease_token":null,"last_error":"reconcile_failed",
                            "lease_until":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(5)),"updated_at":bson::DateTime::now()}
                    }).await;
                    tracing::warn!(
                        reason = "reconcile_failed",
                        "Execution enrollment reconciliation delayed"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_secs(5)).await,
            Err(_) => {
                tracing::warn!(
                    reason = "claim_failed",
                    "Execution enrollment reconciliation delayed"
                );
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

/// Metadata only; the HTTP layer maps models into its dedicated response DTO.
pub async fn progress(db: &Database) -> AppResult<Vec<Job>> {
    Ok(db
        .collection::<Job>(JOBS)
        .find(doc! {})
        .sort(doc! {"updated_at":-1,"_id":1})
        .limit(101)
        .await?
        .try_collect()
        .await?)
}

/// The org deletion cascade already removes its flag rows first. Delete member
/// rows in bounded transactions, retaining repair receipts before their actor
/// IDs disappear. No hint lookup can abort deletion; no population scan is needed.
pub(crate) async fn delete_org_memberships(db: &Database, org: &str) -> AppResult<()> {
    let mut deferred = false;
    loop {
        let db = db.clone();
        let org = org.to_owned();
        let mut session = db.client().start_session().await?;
        let count = session
            .start_transaction()
            .write_concern(WriteConcern::majority())
            .and_run2(async move |session| {
                transactions::transaction_result(
                    async {
                        let members: Vec<OrgMembership> = db
                            .collection::<OrgMembership>(MEMBERSHIPS)
                            .find(doc! {"org_user_id":&org})
                            .sort(doc! {"member_user_id":1})
                            .limit(BATCH)
                            .session(&mut *session)
                            .await?
                            .stream(&mut *session)
                            .try_collect()
                            .await?;
                        let ids: Vec<_> = members.iter().map(|m| &m.id).collect();
                        db.collection::<OrgMembership>(MEMBERSHIPS)
                            .delete_many(doc! {"_id":{"$in":ids}})
                            .session(&mut *session)
                            .await?;
                        if !members.is_empty() {
                            enqueue(&db, &org, session).await?;
                        }
                        // No hint reads in this multi-person transaction: even
                        // bounded per-person timeouts could exhaust its lifetime.
                        for member in &members {
                            lifecycle::defer_person(&db, &member.member_user_id, session).await?;
                        }
                        Ok(members.len())
                    }
                    .await,
                )
            })
            .await
            .map_err(transactions::map_transaction_error)?;
        deferred |= count > 0;
        if count < BATCH as usize {
            break;
        }
    }
    if !deferred {
        db.collection::<Job>(JOBS)
            .delete_one(doc! {"org_id":org})
            .await?;
    }
    Ok(())
}
