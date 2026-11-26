use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    RoutedReplyInProgress,
    RoutedReplyUnavailable,
    ReplyIdempotencyConflict,
    ReplyOperationLimit,
    RoutedReplyRevoked,
    EventRefExpired,
    RoutedReplyInvalid,
    ReplyTargetExpired,
    ReplyTargetInvalid,
    ReplyTargetRevoked,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    NoResponse,
    ServerError,
    UnreadableResponse,
    Interrupted,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Observation {
    pub status: Option<u16>,
    pub code: Option<Code>,
    #[serde(default)]
    pub failure: Option<Failure>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub retry_after: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Confirmed,
    Pending,
    Rejected,
    Unknown,
}
pub struct Reply {
    pub outcome: Outcome,
    pub observation: Observation,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReceiptWait {
    pub version: i32,
    pub pending: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub started_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub admitted_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline: DateTime<Utc>,
    pub attempts: u32,
    pub observation: Observation,
}
