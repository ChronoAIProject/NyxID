//! Durable queue row for the telemetry erasure worker.
//!
//! Account deletion enqueues one job per user before the user row is
//! removed. The worker (see `services::telemetry_erasure_service`)
//! drains these, calls the vendor's delete-person API, and retries on
//! transient errors with exponential backoff. After too many failures
//! a job moves to `failed` (dead-letter) for operator attention.
//!
//! Only the `user_id` is stored. PostHog's own `identify()` merges anon
//! distinct_ids into the authenticated person, so deleting by `user_id`
//! cascades to every aliased anon trail across FE, Mobile, and CLI
//! (`docs/TELEMETRY.md` §4.2). No server-side alias tracking needed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "telemetry_erasure_jobs";

/// Lifecycle states of an erasure job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryErasureStatus {
    /// Enqueued; the worker will pick it up on the next tick.
    Pending,
    /// Currently being processed. Used to prevent two workers from
    /// claiming the same row during a briefly-overlapping handover.
    InFlight,
    /// Vendor accepted the delete; no further action.
    Completed,
    /// Retries exhausted. Flagged for operator attention; the user's
    /// record will remain in the vendor's store until a human resolves.
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TelemetryErasureJob {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub status: TelemetryErasureStatus,
    /// Number of delete attempts made so far. Incremented by the worker
    /// before each attempt.
    #[serde(default)]
    pub attempts: u32,
    /// Last error string, if the most recent attempt failed. Scrubbed
    /// of any sensitive substrings before persistence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl TelemetryErasureJob {
    /// Construct a fresh `pending` job for `user_id`.
    pub fn new(user_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.into(),
            status: TelemetryErasureStatus::Pending,
            attempts: 0,
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }
}
