use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "oracle_workers";

/// Presence record for a browser worker tab, upserted on every poll /
/// heartbeat. Workers are ephemeral: a tab that stops polling simply goes
/// stale (pool status reports workers seen within a recency window).
/// Identity is `{pool_id}:{worker_label}` so labels are scoped per pool.
#[derive(Clone, Serialize, Deserialize)]
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
    #[serde(default, with = "bson_datetime::optional")]
    pub provisioned_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub desired_state: OracleWorkerDesiredState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logged_in: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome_alive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OracleWorkerDesiredState {
    #[default]
    Active,
    Draining,
}

impl std::fmt::Debug for OracleWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OracleWorker")
            .field("id", &self.id)
            .field("pool_id", &self.pool_id)
            .field("worker_label", &self.worker_label)
            .field("current_task_id", &self.current_task_id)
            .field("script_version", &self.script_version)
            .field("platform", &self.platform)
            .field("desired_state", &self.desired_state)
            .field("logged_in", &self.logged_in)
            .field("chrome_alive", &self.chrome_alive)
            .finish()
    }
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
            provisioned_at: None,
            instance_id: Some("process-1".to_string()),
            platform: Some("macos-arm64".to_string()),
            capabilities: vec!["commands_v1".to_string()],
            desired_state: OracleWorkerDesiredState::Active,
            logged_in: Some(true),
            chrome_alive: Some(true),
            last_error: None,
        };
        let doc = bson::to_document(&worker).expect("serialize");
        let restored: OracleWorker = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.id, "pool-1:tab_1");
        assert_eq!(restored.worker_label, "tab_1");
        assert!(restored.first_seen_at.is_some());
        assert_eq!(restored.desired_state, OracleWorkerDesiredState::Active);
    }

    #[test]
    fn legacy_presence_defaults_to_active_without_capabilities() {
        let now = bson::DateTime::from_chrono(Utc::now());
        let worker: OracleWorker = bson::from_document(bson::doc! {
            "_id": "pool-1:legacy",
            "pool_id": "pool-1",
            "worker_label": "legacy",
            "last_seen_at": now,
        })
        .expect("legacy worker");
        assert_eq!(worker.desired_state, OracleWorkerDesiredState::Active);
        assert!(worker.capabilities.is_empty());
        assert_eq!(worker.logged_in, None);
    }
}
