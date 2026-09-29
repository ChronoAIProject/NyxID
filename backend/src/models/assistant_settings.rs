use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_settings";

pub const DEFAULT_MAX_LIVE_SUBAGENTS: i32 = 8;
pub const DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS: i32 = 3;
/// Hard ceilings that bound what an owner can configure.
pub const MAX_LIVE_SUBAGENTS_LIMIT: i32 = 32;
pub const MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT: i32 = 8;
/// Agent-to-agent hand-offs in a group per owner message, and per hour across
/// all of an owner's groups. Owner settings: every hand-off is a paid turn.
pub const DEFAULT_MAX_GROUP_HANDOFFS: i32 = 6;
pub const DEFAULT_MAX_GROUP_HANDOFFS_PER_HOUR: i32 = 60;
pub const MAX_GROUP_HANDOFFS_LIMIT: i32 = 24;
pub const MAX_GROUP_HANDOFFS_PER_HOUR_LIMIT: i32 = 600;

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
    /// 0 turns agent hand-offs in groups off.
    #[serde(default = "default_group_handoffs")]
    pub max_group_handoffs: i32,
    #[serde(default = "default_group_handoffs_per_hour")]
    pub max_group_handoffs_per_hour: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_live() -> i32 {
    DEFAULT_MAX_LIVE_SUBAGENTS
}
fn default_concurrent() -> i32 {
    DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS
}
fn default_group_handoffs() -> i32 {
    DEFAULT_MAX_GROUP_HANDOFFS
}
fn default_group_handoffs_per_hour() -> i32 {
    DEFAULT_MAX_GROUP_HANDOFFS_PER_HOUR
}

impl AssistantSettings {
    pub fn defaults(user_id: &str) -> Self {
        Self {
            user_id: user_id.into(),
            skip_destructive_confirmation: false,
            max_live_subagents: DEFAULT_MAX_LIVE_SUBAGENTS,
            max_concurrent_subagent_turns: DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS,
            max_group_handoffs: DEFAULT_MAX_GROUP_HANDOFFS,
            max_group_handoffs_per_hour: DEFAULT_MAX_GROUP_HANDOFFS_PER_HOUR,
            updated_at: Utc::now(),
        }
    }
}
