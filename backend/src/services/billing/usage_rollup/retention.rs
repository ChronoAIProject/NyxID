//! Bounded sweeps over the existing primary-key index. Eligibility is checked
//! only within the page, so sparse matches cannot turn a pass into a full scan.
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Document, doc},
    options::{Acknowledgment, DatabaseOptions, Hint, ReadConcern, WriteConcern},
};

use super::{METERS, STATE};
use crate::errors::{AppError, AppResult};
use crate::models::usage_rollup_state::MeteringRetentionState;
use crate::services::billing::reconcile::{ACKED_RETENTION_DAYS, continue_reconcile};

const CURSOR_ID: &str = "metering-retention-v1";
const BATCH_SIZE: i64 = 100;
const PASS_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);
const SWEEP_INTERVAL: chrono::Duration = chrono::Duration::days(1);

fn page_filter(after: Option<&str>) -> Document {
    after.map_or_else(Document::new, |id| doc! {"_id": {"$gt": id}})
}

fn eligible(ids: &[String]) -> Document {
    doc! {
        "_id": {"$in": ids}, "wallet_id": null, "expires_at": null,
        "rollup_pending": false, "quantity": {"$ne": null},
        "$or": [{"status": "finalized"}, {"status": "dead_letter", "forwarded": true}],
    }
}

fn elapsed(started: std::time::Instant) -> std::time::Duration {
    #[cfg(test)]
    if let Ok(elapsed) = tests::CLOCK.try_with(|(elapsed, _)| elapsed.get()) {
        return elapsed;
    }
    started.elapsed()
}

fn touch_time(now: DateTime<Utc>, started: std::time::Instant) -> DateTime<Utc> {
    #[cfg(test)]
    if tests::CLOCK.try_with(|_| ()).is_ok() {
        return now;
    }
    now + chrono::Duration::milliseconds(started.elapsed().as_millis() as i64)
}

pub(crate) async fn backfill(db: &Database, now: DateTime<Utc>) -> AppResult<u64> {
    let started = std::time::Instant::now();
    if !continue_reconcile() {
        return Ok(0);
    }
    let durable = db.client().database_with_options(
        db.name(),
        DatabaseOptions::builder()
            .write_concern(
                WriteConcern::builder()
                    .w(Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .read_concern(ReadConcern::majority())
            .build(),
    );
    let db = &durable;
    let states = db.collection::<MeteringRetentionState>(STATE);
    let current = match states.find_one(doc! {"_id": CURSOR_ID}).await? {
        Some(state) => state,
        None => {
            states
                .update_one(
                    doc! {"_id": CURSOR_ID},
                    doc! {"$setOnInsert": {"revision": 0_i64, "after": null}},
                )
                .upsert(true)
                .await?;
            #[cfg(test)]
            tests::point(tests::Phase::Initialized).await;
            if !continue_reconcile() {
                return Ok(0);
            }
            states
                .find_one(doc! {"_id": CURSOR_ID})
                .await?
                .ok_or_else(|| AppError::Internal("Missing metering retention cursor".into()))?
        }
    };
    if current
        .next_due_at
        .is_some_and(|next_due_at| next_due_at > now)
    {
        return Ok(0);
    }

    let mut current = current;
    let mut changed = 0;
    loop {
        if !continue_reconcile() || elapsed(started) >= PASS_BUDGET {
            break;
        }
        let rows: Vec<Document> = db
            .collection::<Document>(METERS)
            .find(page_filter(current.after.as_deref()))
            .hint(Hint::Name("_id_".into()))
            .sort(doc! {"_id": 1})
            .projection(doc! {"_id": 1})
            .limit(BATCH_SIZE)
            .max_time(std::time::Duration::from_secs(2))
            .await?
            .try_collect()
            .await?;
        let ids = rows
            .iter()
            .map(|row| {
                row.get_str("_id")
                    .map(str::to_owned)
                    .map_err(|_| AppError::Internal("Invalid metering retention row id".into()))
            })
            .collect::<AppResult<Vec<_>>>()?;
        #[cfg(test)]
        tests::point(tests::Phase::Selected).await;
        if !continue_reconcile() {
            break;
        }
        if ids.is_empty() {
            let completed_at = touch_time(now, started);
            states
                .update_one(
                    doc! {"_id": CURSOR_ID, "revision": current.revision},
                    doc! {"$set": {
                        "after": null,
                        "completed_at": bson::DateTime::from_chrono(completed_at),
                        "next_due_at": bson::DateTime::from_chrono(completed_at + SWEEP_INTERVAL),
                    }, "$inc": {"revision": 1_i64}},
                )
                .await?;
            #[cfg(test)]
            tests::point(tests::Phase::Advanced).await;
            break;
        }
        let expires_at = bson::DateTime::from_chrono(
            touch_time(now, started) + chrono::Duration::days(ACKED_RETENTION_DAYS),
        );
        changed += db
            .collection::<Document>(METERS)
            .update_many(eligible(&ids), doc! {"$set": {"expires_at": expires_at}})
            .hint(Hint::Name("_id_".into()))
            .await?
            .modified_count;
        #[cfg(test)]
        tests::point(tests::Phase::Dated).await;
        if !continue_reconcile() {
            break;
        }
        // Advance only after the guarded page write is durable. A stale
        // helper cannot move the cursor over a newer page or completed sweep.
        let after = ids.last().cloned();
        let advanced = states
            .update_one(
                doc! {"_id": CURSOR_ID, "revision": current.revision},
                doc! {"$set": {"after": &after, "next_due_at": null}, "$inc": {"revision": 1_i64}},
            )
            .await?;
        #[cfg(test)]
        tests::point(tests::Phase::Advanced).await;
        if advanced.modified_count == 0 {
            break;
        }
        current.after = after;
        current.revision += 1;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests;
