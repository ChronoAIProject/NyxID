use bson::{Document, doc};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{Database, options::Hint};
use serde::Serialize;
use std::time::Duration;

use super::{COLLECTION_NAME, DIAGNOSTIC_INDEX, INTENT_VERSION};
use crate::errors::{AppError, AppResult};

const COUNT_LIMIT: i64 = 1_000;
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
pub struct SecurityEffectCount {
    pub kind: &'static str,
    pub state: &'static str,
    pub count: i64,
    /// Counts stop at COUNT_LIMIT; true means the displayed count is a lower bound.
    pub at_least: bool,
}

#[derive(Debug, Serialize)]
pub struct SecurityEffectDiagnostics {
    pub counts: Vec<SecurityEffectCount>,
    pub oldest_pending_at: Option<DateTime<Utc>>,
    pub oldest_pending_age_seconds: Option<i64>,
}

/// Only the Integrity endpoints call this. Each indexed range examines at
/// most 1,001 metadata keys, including the oldest row in that range. No effect
/// contents are fetched and no progress marker is written by a diagnostic.
pub async fn diagnostics(db: &Database) -> AppResult<SecurityEffectDiagnostics> {
    let mut result = SecurityEffectDiagnostics {
        counts: Vec::new(),
        oldest_pending_at: None,
        oldest_pending_age_seconds: None,
    };
    for kind in ["audit", "cae"] {
        for state in [
            "pending",
            "claimed",
            "sending",
            "delivered",
            "failed",
            "uncertain",
        ] {
            let row = db.collection::<Document>(COLLECTION_NAME)
                .aggregate([
                    doc! {"$match": {"version": INTENT_VERSION, "kind": kind, "state": state}},
                    doc! {"$sort": {"created_at": 1}},
                    doc! {"$limit": COUNT_LIMIT + 1},
                    doc! {"$project": {"_id": 0, "created_at": 1}},
                    doc! {"$group": {"_id": null, "count": {"$sum": 1_i64}, "oldest": {"$first": "$created_at"}}},
                    doc! {"$set": {"age_seconds": {"$toLong": {"$divide": [{"$subtract": ["$$NOW", "$oldest"]}, 1000]}}}},
                ])
                .hint(Hint::Name(DIAGNOSTIC_INDEX.to_owned()))
                .max_time(QUERY_TIMEOUT)
                .await?
                .try_next()
                .await?;
            let Some(row) = row else { continue };
            let invalid =
                || AppError::Internal("Invalid security effect diagnostic metadata".into());
            let count = row.get_i64("count").map_err(|_| invalid())?;
            result.counts.push(SecurityEffectCount {
                kind,
                state,
                count: count.min(COUNT_LIMIT),
                at_least: count > COUNT_LIMIT,
            });
            if matches!(state, "pending" | "claimed" | "sending") {
                let oldest = row
                    .get_datetime("oldest")
                    .map_err(|_| invalid())?
                    .to_chrono();
                if result
                    .oldest_pending_at
                    .is_none_or(|previous| oldest < previous)
                {
                    result.oldest_pending_at = Some(oldest);
                    result.oldest_pending_age_seconds =
                        Some(row.get_i64("age_seconds").map_err(|_| invalid())?.max(0));
                }
            }
        }
    }
    Ok(result)
}
