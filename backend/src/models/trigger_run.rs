//! Durable occurrence identity and metadata. Never stores instructions or payloads.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "trigger_runs";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Pending,
    Started,
    Waiting,
    Completed,
    Failed,
    Skipped,
}

impl RunOutcome {
    /// Stable BSON filter spelling, shared with the serde representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Started => "started",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TriggerRun {
    #[serde(rename = "_id")]
    pub id: String,
    pub trigger_id: String,
    pub user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub scheduled_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline: DateTime<Utc>,
    pub outcome: RunOutcome,
    /// Snapshotted on admission for webhook turns, including continuations.
    #[serde(default)]
    pub confirmation_policy: Option<crate::models::trigger_schedule::ConfirmationPolicy>,
    /// Includes upstream assistant error codes as well as scheduler reasons;
    /// this is deliberately an open diagnostic namespace.
    pub reason: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub finished_at: Option<DateTime<Utc>>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub missed: Option<MissedOccurrences>,
    #[serde(default)]
    pub waiting_for: Vec<String>,
    #[serde(default)]
    pub result_claimed: bool,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub deliver_to: crate::models::trigger_schedule::DeliverTo,
    #[serde(default)]
    pub label: String,
    pub fence: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub lease_until: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MissedOccurrences {
    pub count: u64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub from: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub through: DateTime<Utc>,
}
