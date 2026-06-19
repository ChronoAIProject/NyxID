use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "oracle_tasks";

pub fn default_task_kind() -> String {
    "prompt".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OracleTaskStatus {
    /// Waiting in the pool's FIFO queue.
    Queued,
    /// Claimed by a worker; lease active.
    Dispatched,
    Completed,
    Failed,
    Cancelled,
}

impl OracleTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Dispatched => "dispatched",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether the task has reached a final state (no further dispatch).
    /// Used by consumers polling for completion.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// One oracle task: a prompt relayed to a browser worker, answered
/// asynchronously. Consumers poll `GET /api/v1/oracle/tasks/{id}` until a
/// terminal status. Prompt/response bodies live only in this collection
/// (and are TTL-expired via `expires_at`); audit/tracing record metadata
/// only.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OracleTask {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    pub submitter_user_id: String,
    /// "prompt" for normal oracle turns; "scrape" for transcript-import
    /// control tasks; "extract" for general web page extraction.
    #[serde(default = "default_task_kind")]
    pub kind: String,
    /// Target URL for general web extraction tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
    /// API key attribution when submitted by an agent (mirrors AuditLog).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_name: Option<String>,
    pub prompt: String,
    /// Opaque model hint forwarded to the worker and echoed on results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_label: Option<String>,
    /// Optional per-task ChatGPT Project URL override for prompt tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Optional PDF attachment (base64), uploaded by the worker on the
    /// first turn of a fresh conversation. Size-capped at submit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_name: Option<String>,
    /// Multi-turn session this task belongs to (None = single-shot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// True when this task continues an existing conversation.
    #[serde(default)]
    pub is_followup: bool,
    /// When set, only the worker with this label may claim the task.
    /// Copied from the owning session for follow-ups so multi-turn lands
    /// back on the account that created the conversation in a
    /// multi-account pool. `None` = fresh task, claimable by any worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_worker_label: Option<String>,
    /// Optional submitter-scoped idempotency key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ref: Option<String>,
    pub status: OracleTaskStatus,
    /// Worker-reported progress phase (e.g. "sent", "waiting_response").
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
    /// Lease deadline while dispatched; refreshed by worker heartbeats.
    /// Expired leases are requeued to the front on the next claim.
    #[serde(default, with = "bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_chars: Option<u64>,
    /// Browser-side conversation URL reported by the worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_script_version: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    /// TTL anchor (MongoDB `expireAfterSeconds: 0` index): set when the
    /// task reaches a terminal status, `completed_at` + retention.
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "oracle_tasks");
    }

    fn make_task() -> OracleTask {
        OracleTask {
            id: uuid::Uuid::new_v4().to_string(),
            pool_id: uuid::Uuid::new_v4().to_string(),
            submitter_user_id: uuid::Uuid::new_v4().to_string(),
            kind: "prompt".to_string(),
            target_url: None,
            api_key_id: None,
            api_key_name: None,
            prompt: "What is the BEDC closure of item 8?".to_string(),
            model_label: Some("chatgpt-5.5-pro".to_string()),
            project_url: None,
            tag: Some("bedc-deep".to_string()),
            pdf_base64: None,
            pdf_name: None,
            conversation_id: None,
            is_followup: false,
            required_worker_label: None,
            client_ref: None,
            status: OracleTaskStatus::Queued,
            phase: None,
            phase_detail: None,
            phase_at: None,
            assigned_worker_id: None,
            dispatched_at: None,
            lease_expires_at: None,
            response: None,
            response_chars: None,
            chatgpt_url: None,
            failure_reason: None,
            worker_script_version: None,
            completed_at: None,
            expires_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip_queued() {
        let task = make_task();
        let doc = bson::to_document(&task).expect("serialize");
        let restored: OracleTask = bson::from_document(doc).expect("deserialize");
        assert_eq!(task.id, restored.id);
        assert_eq!(restored.status, OracleTaskStatus::Queued);
        assert!(restored.lease_expires_at.is_none());
        assert!(!restored.is_followup);
        assert_eq!(restored.kind, "prompt");
    }

    #[test]
    fn missing_kind_defaults_to_prompt() {
        let task = make_task();
        let mut doc = bson::to_document(&task).expect("serialize");
        doc.remove("kind");
        let restored: OracleTask = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.kind, "prompt");
    }

    #[test]
    fn bson_roundtrip_dispatched_with_lease() {
        let mut task = make_task();
        task.status = OracleTaskStatus::Dispatched;
        task.assigned_worker_id = Some("tab_1".to_string());
        task.dispatched_at = Some(Utc::now());
        task.lease_expires_at = Some(Utc::now() + chrono::Duration::hours(4));
        task.phase = Some("waiting_response".to_string());
        task.phase_at = Some(Utc::now());
        let doc = bson::to_document(&task).expect("serialize");
        let restored: OracleTask = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.status, OracleTaskStatus::Dispatched);
        assert!(restored.lease_expires_at.is_some());
        assert_eq!(restored.assigned_worker_id.as_deref(), Some("tab_1"));
    }

    #[test]
    fn bson_roundtrip_completed_with_ttl_anchor() {
        let mut task = make_task();
        task.status = OracleTaskStatus::Completed;
        task.response = Some("BREAKTHROUGH: ...".to_string());
        task.response_chars = Some(17);
        task.completed_at = Some(Utc::now());
        task.expires_at = Some(Utc::now() + chrono::Duration::days(30));
        let doc = bson::to_document(&task).expect("serialize");
        let restored: OracleTask = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.status, OracleTaskStatus::Completed);
        assert!(restored.expires_at.is_some());
        assert_eq!(restored.response_chars, Some(17));
    }

    #[test]
    fn status_terminal_classification() {
        assert!(!OracleTaskStatus::Queued.is_terminal());
        assert!(!OracleTaskStatus::Dispatched.is_terminal());
        assert!(OracleTaskStatus::Completed.is_terminal());
        assert!(OracleTaskStatus::Failed.is_terminal());
        assert!(OracleTaskStatus::Cancelled.is_terminal());
    }

    #[test]
    fn status_serde_uses_lowercase() {
        assert_eq!(
            serde_json::to_string(&OracleTaskStatus::Dispatched).unwrap(),
            "\"dispatched\""
        );
        let back: OracleTaskStatus = serde_json::from_str("\"cancelled\"").unwrap();
        assert_eq!(back, OracleTaskStatus::Cancelled);
    }
}
