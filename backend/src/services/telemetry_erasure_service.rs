//! Background drain loop for the telemetry erasure queue.
//!
//! Account deletion enqueues `TelemetryErasureJob` rows BEFORE deleting
//! the user record (see `handlers/users.rs`). This service polls the
//! queue every 30 seconds, claims up to 16 pending jobs atomically,
//! and asks the vendor to delete the person. Failures revert the job
//! to `pending` so it will be picked up on the next 30s tick. After
//! `MAX_ATTEMPTS` failures the job is dead-lettered (`failed`) for
//! operator attention. There is no per-job backoff schedule; the
//! uniform 30s cadence is acceptable for GDPR (days-long SLA) and
//! keeps the service one file of code.
//!
//! The loop is a no-op when telemetry is hard-off (no client); we
//! return silently from `spawn_worker` in that case so default-off
//! deploys produce no new startup output.
//!
//! Mirrors the `interval.tick()` pattern used for approval expiry at
//! `main.rs:417-446`.

use std::sync::Arc;
use std::time::Duration;

use bson::doc;
use chrono::Utc;
use mongodb::Database;
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use tokio::time::{MissedTickBehavior, interval};

use crate::db::DbHandle;
use crate::errors::{AppError, AppResult};
use crate::models::telemetry_erasure_job::{
    COLLECTION_NAME, TelemetryErasureJob, TelemetryErasureStatus,
};
use crate::telemetry::TelemetryClient;

/// How often the drain loop wakes up. Errs on the short side: erasure
/// obligations under GDPR are stated in days, so 30s is a rounding
/// error. Under load this still bounds wasted queue scans to O(1).
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Max pending jobs claimed per tick. Bounded to prevent a large backlog
/// (e.g. after the vendor is down for a while) from swamping a single
/// iteration.
const BATCH_SIZE: usize = 16;

/// After this many failed attempts, the job is moved to `failed` for
/// operator review rather than retried indefinitely. Each attempt is
/// separated by at least one `POLL_INTERVAL` (30s), so a dead-letter
/// represents ≥ `MAX_ATTEMPTS × POLL_INTERVAL` of vendor unavailability.
const MAX_ATTEMPTS: u32 = 5;

/// Spawn the drain loop. Called once at server startup.
///
/// Silent no-op when `telemetry` is `None` — all pending jobs wait in
/// the collection until a process comes up with a DSN configured, at
/// which point they drain naturally. We return silently (no log line)
/// so the default-off deploy is byte-identical to a pre-telemetry
/// build in its startup output.
pub fn spawn_worker(db: DbHandle, telemetry: Option<Arc<TelemetryClient>>) {
    let Some(telemetry) = telemetry else {
        return;
    };
    tokio::spawn(async move {
        let mut ticker = interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(e) = drain_once(&db, &telemetry).await {
                tracing::warn!(error = %e, "telemetry erasure drain failed");
            }
        }
    });
}

/// Enqueue one erasure job for `user_id`. Called from account-deletion
/// handlers BEFORE the user row is removed.
pub async fn enqueue(db: &Database, user_id: &str) -> AppResult<String> {
    let job = TelemetryErasureJob::new(user_id);
    let job_id = job.id.clone();
    let coll = db.collection::<TelemetryErasureJob>(COLLECTION_NAME);
    coll.insert_one(&job).await.map_err(AppError::from)?;
    Ok(job_id)
}

async fn drain_once(db: &Database, telemetry: &TelemetryClient) -> AppResult<()> {
    let coll = db.collection::<TelemetryErasureJob>(COLLECTION_NAME);

    for _ in 0..BATCH_SIZE {
        // Atomically claim the oldest pending job. Using
        // `find_one_and_update` so two workers (e.g. during a rolling
        // deploy) can never both grab the same row.
        let claim: Option<TelemetryErasureJob> = coll
            .find_one_and_update(
                doc! { "status": "pending" },
                doc! {
                    "$set": {
                        "status": "in_flight",
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    },
                    "$inc": { "attempts": 1 },
                },
            )
            .with_options(
                FindOneAndUpdateOptions::builder()
                    .sort(doc! { "created_at": 1 })
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
            .map_err(AppError::from)?;

        let Some(job) = claim else {
            break; // no more pending jobs this tick
        };

        match telemetry.delete_person(&job.user_id).await {
            Ok(()) => {
                coll.update_one(
                    doc! { "_id": &job.id },
                    doc! {
                        "$set": {
                            "status": "completed",
                            "updated_at": bson::DateTime::from_chrono(Utc::now()),
                        },
                    },
                )
                .await
                .map_err(AppError::from)?;
            }
            Err(e) => {
                let err_text = format!("{e}");
                let next_status = if job.attempts >= MAX_ATTEMPTS {
                    TelemetryErasureStatus::Failed
                } else {
                    TelemetryErasureStatus::Pending
                };
                let status_str = match next_status {
                    TelemetryErasureStatus::Failed => "failed",
                    _ => "pending",
                };
                coll.update_one(
                    doc! { "_id": &job.id },
                    doc! {
                        "$set": {
                            "status": status_str,
                            "last_error": err_text.chars().take(512).collect::<String>(),
                            "updated_at": bson::DateTime::from_chrono(Utc::now()),
                        },
                    },
                )
                .await
                .map_err(AppError::from)?;
                if matches!(next_status, TelemetryErasureStatus::Failed) {
                    tracing::error!(
                        user_id = %job.user_id,
                        attempts = job.attempts,
                        "telemetry erasure dead-lettered"
                    );
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::telemetry_erasure_job::{
        COLLECTION_NAME, TelemetryErasureJob, TelemetryErasureStatus,
    };
    use crate::test_utils::connect_test_database;
    use futures::TryStreamExt;

    #[tokio::test]
    async fn enqueue_inserts_pending_job() {
        let Some(db) = connect_test_database("tele_erasure_enqueue").await else {
            return;
        };

        let user_id = "user-to-erase-1";
        let job_id = enqueue(&db, user_id).await.unwrap();

        // Verify the job was persisted
        let coll = db.collection::<TelemetryErasureJob>(COLLECTION_NAME);
        let job = coll
            .find_one(doc! { "_id": &job_id })
            .await
            .unwrap()
            .expect("job should exist in DB");

        assert_eq!(job.id, job_id);
        assert_eq!(job.user_id, user_id);
        assert_eq!(job.status, TelemetryErasureStatus::Pending);
        assert_eq!(job.attempts, 0);
        assert!(job.last_error.is_none());
    }

    #[tokio::test]
    async fn enqueue_multiple_jobs_for_same_user() {
        let Some(db) = connect_test_database("tele_erasure_multi").await else {
            return;
        };

        let user_id = "user-to-erase-2";
        let id1 = enqueue(&db, user_id).await.unwrap();
        let id2 = enqueue(&db, user_id).await.unwrap();

        // Both should exist with distinct IDs
        assert_ne!(id1, id2);

        let coll = db.collection::<TelemetryErasureJob>(COLLECTION_NAME);
        let jobs: Vec<TelemetryErasureJob> = coll
            .find(doc! { "user_id": user_id })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(jobs.len(), 2);
    }

    #[tokio::test]
    async fn enqueue_returns_unique_job_id() {
        let Some(db) = connect_test_database("tele_erasure_unique_id").await else {
            return;
        };

        let id1 = enqueue(&db, "user-a").await.unwrap();
        let id2 = enqueue(&db, "user-b").await.unwrap();
        assert_ne!(id1, id2);
        // Job IDs should be valid UUIDs (36 chars with hyphens)
        assert_eq!(id1.len(), 36);
        assert_eq!(id2.len(), 36);
    }
}
