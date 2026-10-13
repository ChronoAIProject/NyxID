use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const COLLECTION_NAME: &str = "assistant_checkpoints";

/// The durable working-context checkpoint of one NyxAgent conversation,
/// written in the same transaction that settles each turn. `_id` is the
/// conversation id. Metadata and bounded excerpts only: no credentials, tool
/// arguments or tool results.
#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantCheckpoint {
    #[serde(rename = "_id")]
    pub conversation_id: String,
    pub user_id: String,
    /// The turn that wrote this checkpoint.
    pub turn_id: String,
    /// `completed` or `failed`: the outcome of `turn_id`.
    pub status: String,
    #[serde(default)]
    pub error_code: Option<String>,
    /// The request the current task is working on: the owner message that
    /// opened it, kept across a run of failed turns until one succeeds.
    #[serde(default)]
    pub goal: Option<String>,
    /// Actions completed since the last successful turn, oldest first.
    #[serde(default)]
    pub completed_actions: Vec<CheckpointAction>,
    /// Actions that failed or were interrupted in the same window.
    #[serde(default)]
    pub failed_actions: Vec<CheckpointAction>,
    /// A bounded excerpt of the latest reply text.
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub last_successful_turn_id: Option<String>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub last_successful_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for AssistantCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssistantCheckpoint")
            .field("conversation_id", &self.conversation_id)
            .field("user_id", &self.user_id)
            .field("turn_id", &self.turn_id)
            .field("status", &self.status)
            .field("error_code", &self.error_code)
            .field("goal_len", &self.goal.as_ref().map(String::len))
            .field("completed_actions", &self.completed_actions.len())
            .field("failed_actions", &self.failed_actions.len())
            .field("output_len", &self.output.as_ref().map(String::len))
            .field("last_successful_turn_id", &self.last_successful_turn_id)
            .field("last_successful_at", &self.last_successful_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointAction {
    /// The tool identifier shown on the activity; never arguments.
    pub label: String,
    pub turn_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_goal_output_and_action_labels() {
        let debug = format!(
            "{:?}",
            AssistantCheckpoint {
                conversation_id: "conversation".into(),
                user_id: "user".into(),
                turn_id: "turn".into(),
                status: "failed".into(),
                error_code: None,
                goal: Some("sample goal text".into()),
                completed_actions: vec![CheckpointAction {
                    label: "sample action text".into(),
                    turn_id: "turn".into(),
                }],
                failed_actions: Vec::new(),
                output: Some("sample assistant output".into()),
                last_successful_turn_id: None,
                last_successful_at: None,
                updated_at: Utc::now(),
            }
        );
        assert!(!debug.contains("sample goal text"));
        assert!(!debug.contains("sample assistant output"));
        assert!(!debug.contains("sample action text"));
    }
}
