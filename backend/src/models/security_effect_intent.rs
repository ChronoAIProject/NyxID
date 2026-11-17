use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{audit_log::AuditLog, bson_datetime};

pub const COLLECTION_NAME: &str = "security_effect_intents";

/// Immutable occurrence data, separate from mutable delivery progress.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecurityEffect {
    Audit {
        event: Box<AuditLog>,
    },
    Cae {
        binding_id: String,
        client_id: String,
        revoke_source: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    Pending,
    Claimed,
    Sending,
    Delivered,
    Failed,
    Uncertain,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SecurityEffectIntent {
    #[serde(rename = "_id")]
    pub id: String,
    pub version: i32,
    #[serde(flatten)]
    pub effect: SecurityEffect,
    pub state: IntentState,
    pub attempts: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// Pending retry time or the current claim deadline, always set by MongoDB.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub next_attempt_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_token: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Fixed internal classification only, never a provider/database error body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}
