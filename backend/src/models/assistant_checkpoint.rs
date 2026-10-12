use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_checkpoints";

/// The durable working-context checkpoint of one NyxAgent conversation,
/// written in the same transaction that settles each turn. `_id` is the
/// conversation id. Metadata and bounded excerpts only: no credentials, tool
/// arguments or tool results.
#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointAction {
    /// The tool identifier shown on the activity; never arguments.
    pub label: String,
    pub turn_id: String,
}
