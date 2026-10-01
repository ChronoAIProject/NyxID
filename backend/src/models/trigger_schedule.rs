//! Additive trigger scheduling and assistant delivery configuration.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSource {
    #[default]
    Webhook,
    Schedule,
}

/// Persisted instants are UTC BSON dates; API DTOs accept RFC3339 offsets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleKind {
    Cron {
        expression: String,
        timezone: String,
    },
    Every {
        amount: u32,
        unit: IntervalUnit,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        anchor: DateTime<Utc>,
    },
    At {
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        at: DateTime<Utc>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalUnit {
    Minutes,
    Hours,
    Days,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleSpec {
    #[serde(flatten)]
    pub timing: ScheduleKind,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub start: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub end: Option<DateTime<Utc>>,
    #[serde(default)]
    pub max_runs: Option<u32>,
    #[serde(default)]
    pub grace_seconds: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapPolicy {
    #[default]
    Skip,
    Queue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadPolicy {
    Home,
    Dedicated,
    New,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliverTo {
    #[default]
    Thread,
    Chat {
        chat_id: String,
    },
    Notification,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScheduleState {
    #[serde(default)]
    pub last_run: Option<super::trigger_run::TriggerRun>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub next_due_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub occurrences: u32,
    #[serde(default)]
    pub pause_reason: Option<String>,
    #[serde(default)]
    pub dedicated_thread_id: Option<String>,
}

/// Webhook event data cannot authorize changing operations by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationPolicy {
    #[default]
    Changes,
    Destructive,
}
