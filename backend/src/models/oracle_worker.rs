use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "oracle_workers";

/// Presence record for a browser worker tab, upserted on every poll /
/// heartbeat. Workers are ephemeral: a tab that stops polling simply goes
/// stale (pool status reports workers seen within a recency window).
/// Identity is `{pool_id}:{worker_label}` so labels are scoped per pool.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OracleWorker {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    /// Tab-chosen label (e.g. "tab_1"), unique within the pool.
    pub worker_label: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_seen_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_version: Option<String>,
    /// Last reported page URL tail (diagnostics; never logged elsewhere).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub first_seen_at: Option<DateTime<Utc>>,
}

/// Compose the worker document id from pool + label.
pub fn worker_doc_id(pool_id: &str, worker_label: &str) -> String {
    format!("{pool_id}:{worker_label}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "oracle_workers");
    }

    #[test]
    fn doc_id_composition() {
        assert_eq!(worker_doc_id("pool-1", "tab_2"), "pool-1:tab_2");
    }

    #[test]
    fn bson_roundtrip() {
        let worker = OracleWorker {
            id: worker_doc_id("pool-1", "tab_1"),
            pool_id: "pool-1".to_string(),
            worker_label: "tab_1".to_string(),
            last_seen_at: Utc::now(),
            current_task_id: Some("t1".to_string()),
            script_version: Some("nyxid-1.0".to_string()),
            page_url: Some("chatgpt.com/c/abc".to_string()),
            first_seen_at: Some(Utc::now()),
        };
        let doc = bson::to_document(&worker).expect("serialize");
        let restored: OracleWorker = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.id, "pool-1:tab_1");
        assert_eq!(restored.worker_label, "tab_1");
        assert!(restored.first_seen_at.is_some());
    }
}
