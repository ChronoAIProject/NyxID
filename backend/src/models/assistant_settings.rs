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

pub const DEFAULT_SCHEDULE_MINIMUM_MINUTES: i32 = 5;
pub const SCHEDULE_MINIMUM_MINUTES_LIMIT: i32 = 1440;
pub const DEFAULT_TRIGGER_RUNS_PER_HOUR: i32 = 30;
pub const TRIGGER_RUNS_PER_HOUR_LIMIT: i32 = 300;
pub const DEFAULT_TRIGGER_RUNS_PER_DAY: i32 = 300;
pub const TRIGGER_RUNS_PER_DAY_LIMIT: i32 = 3000;

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
    #[serde(default = "default_auto_continuations")]
    pub max_auto_continuations: i32,
    #[serde(default = "default_concurrent")]
    pub max_concurrent_subagent_turns: i32,
    /// 0 turns agent hand-offs in groups off.
    #[serde(default = "default_group_handoffs")]
    pub max_group_handoffs: i32,
    #[serde(default = "default_group_handoffs_per_hour")]
    pub max_group_handoffs_per_hour: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default = "default_schedule_minimum")]
    pub schedule_minimum_minutes: i32,
    #[serde(default = "default_trigger_hourly")]
    pub trigger_runs_per_hour: i32,
    #[serde(default = "default_trigger_daily")]
    pub trigger_runs_per_day: i32,
}

fn default_schedule_minimum() -> i32 {
    DEFAULT_SCHEDULE_MINIMUM_MINUTES
}
fn default_trigger_hourly() -> i32 {
    DEFAULT_TRIGGER_RUNS_PER_HOUR
}
fn default_trigger_daily() -> i32 {
    DEFAULT_TRIGGER_RUNS_PER_DAY
}

fn default_auto_continuations() -> i32 {
    8
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
            max_auto_continuations: default_auto_continuations(),
            max_concurrent_subagent_turns: DEFAULT_MAX_CONCURRENT_SUBAGENT_TURNS,
            max_group_handoffs: DEFAULT_MAX_GROUP_HANDOFFS,
            max_group_handoffs_per_hour: DEFAULT_MAX_GROUP_HANDOFFS_PER_HOUR,
            updated_at: Utc::now(),
            timezone: None,
            schedule_minimum_minutes: DEFAULT_SCHEDULE_MINIMUM_MINUTES,
            trigger_runs_per_hour: DEFAULT_TRIGGER_RUNS_PER_HOUR,
            trigger_runs_per_day: DEFAULT_TRIGGER_RUNS_PER_DAY,
        }
    }
}
