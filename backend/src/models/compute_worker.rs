use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "compute_workers";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComputeWorker {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    pub worker_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub vram_total_mb: Option<u64>,
    #[serde(default)]
    pub vram_free_mb: Option<u64>,
    #[serde(default)]
    pub max_concurrency: u32,
    #[serde(default)]
    pub current_inflight: u32,
    #[serde(default)]
    pub avg_tokens_per_sec: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_version: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub first_seen_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_seen_at: DateTime<Utc>,
}

pub fn worker_doc_id(pool_id: &str, worker_label: &str) -> String {
    format!("{pool_id}:{worker_label}")
}
