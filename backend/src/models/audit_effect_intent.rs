//! Effect bookkeeping contains only reviewed identity/action metadata and hash
//! evidence. Never put provider arguments, URLs, credentials or responses here.
use super::audit_outbox::{AuditStage, SafeAuditEvent};
use chrono::{DateTime, Utc};
use nyxid::models::audit_witness::DispatchRequest;
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "audit_effect_intents";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectPhase {
    Pending,
    ClaimRequested,
    Claimed,
    Terminal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEffectIntent {
    #[serde(rename = "_id")]
    pub id: String,
    pub event: SafeAuditEvent,
    pub event_id: String,
    pub terminal_event_id: String,
    pub terminal_order: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub reserved_at: DateTime<Utc>,
    pub phase: EffectPhase,
    pub attempt: Option<DispatchRequest>,
    pub terminal_stage: Option<AuditStage>,
}
