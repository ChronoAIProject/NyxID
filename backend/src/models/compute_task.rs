use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "compute_tasks";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ComputeTaskStatus {
    Queued,
    Dispatched,
    Completed,
    Failed,
    Cancelled,
}

impl ComputeTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Dispatched => "dispatched",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComputeTask {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    pub submitter_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_name: Option<String>,
    /// "chat_completion" | "completion" | "embedding" | "batch" for v1.
    pub kind: String,
    pub model: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ref: Option<String>,
    pub status: ComputeTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_detail: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub phase_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_worker_id: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub dispatched_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
