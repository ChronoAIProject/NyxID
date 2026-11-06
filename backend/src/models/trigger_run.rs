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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator: Option<CoordinatorState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<super::assistant_message::WorkEvidence>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub trigger_generation: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub dispatch_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub terminal_result_recorded: bool,
    #[serde(default)]
    pub attempts: Option<u32>,
    #[serde(default)]
    pub capacity_wait_ms: Option<u64>,
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
    /// Sanitized model callback failure from the latest settled run turn.
    #[serde(default)]
    pub failure_reason: Option<Box<super::assistant_message::TurnFailureReason>>,
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
    pub image_refs: Vec<RunImageRef>,
    #[serde(default)]
    pub delegated_image_refs: Vec<super::assistant_delegated_image::DeliveryReference>,
    #[serde(default)]
    pub image_refs_truncated: u32,
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
pub struct RunImageRef {
    pub conversation_id: String,
    pub turn_id: String,
    pub attachment: crate::models::assistant_conversation::TurnAttachment,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MissedOccurrences {
    pub count: u64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub from: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub through: DateTime<Utc>,
}

/// Enrollment is per run. Legacy Started/Waiting rows never acquire this field.
/// Only the thread selected by the delivery owns this state: home, dedicated
/// NyxBot/specialist webhook, or per-run; run-now uses the same delivery policy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoordinatorState {
    pub protocol: u32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline: DateTime<Utc>,
    pub continuations: u32,
    pub dispatches: u32,
    pub closed: bool,
    pub version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_kind: Option<String>,
}

/// Lifecycle/result provenance, never execution authority or a trigger claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationWork {
    pub run_id: String,
    pub work_id: String,
    pub coordinator_thread_id: String,
    /// `coordinator`, `direct` or `group`; an open namespace for old readers.
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_policy: Option<super::trigger_schedule::ConfirmationPolicy>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_state_uses_bson_dates_and_omits_absent_wait() {
        let state = CoordinatorState {
            protocol: 2,
            deadline: Utc::now(),
            continuations: 0,
            dispatches: 0,
            closed: false,
            version: 0,
            wait_kind: None,
        };
        let encoded = bson::to_document(&state).unwrap();
        assert!(encoded.get_datetime("deadline").is_ok());
        assert!(!encoded.contains_key("wait_kind"));
        let restored: CoordinatorState = bson::from_document(encoded).unwrap();
        assert_eq!(restored.protocol, 2);
    }
}
