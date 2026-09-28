use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_settings";

pub const DEFAULT_MAX_LIVE_SUBAGENTS: i32 = 8;
pub const DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS: i32 = 3;
/// Hard ceilings that bound what an owner can configure.
pub const MAX_LIVE_SUBAGENTS_LIMIT: i32 = 32;
pub const MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT: i32 = 8;

/// Per-person NyxBot preferences. `_id` is the user ID; absent rows use defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantSettings {
    #[serde(rename = "_id")]
    pub user_id: String,
    /// When false (the default), destructive account actions show a single-use
    /// confirmation card before they run.
    #[serde(default)]
    pub skip_destructive_confirmation: bool,
    #[serde(default = "default_live")]
    pub max_live_subagents: i32,
    #[serde(default = "default_concurrent")]
    pub max_concurrent_subagent_turns: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_live() -> i32 {
    DEFAULT_MAX_LIVE_SUBAGENTS
}
fn default_concurrent() -> i32 {
    DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS
}

impl AssistantSettings {
    pub fn defaults(user_id: &str) -> Self {
        Self {
            user_id: user_id.into(),
            skip_destructive_confirmation: false,
            max_live_subagents: DEFAULT_MAX_LIVE_SUBAGENTS,
            max_concurrent_subagent_turns: DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS,
            updated_at: Utc::now(),
        }
    }
}
