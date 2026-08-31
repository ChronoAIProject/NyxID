use chrono::{DateTime, Utc};
use mongodb::bson::Bson;
use serde::{Deserialize, Serialize};

pub const LEASE_COLLECTION_NAME: &str = "coordination_leases";
pub const REPLAY_COLLECTION_NAME: &str = "coordination_replay_records";
pub const RATE_WINDOW_COLLECTION_NAME: &str = "coordination_rate_windows";
pub const TOKEN_BUCKET_COLLECTION_NAME: &str = "coordination_token_buckets";
pub const SLOT_COLLECTION_NAME: &str = "coordination_slots";
pub const EVENT_DEDUP_COLLECTION_NAME: &str = "event_dedup_records";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinationHolder {
    pub instance_id: String,
    pub generation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationLeaseKind {
    Ephemeral,
    Checkpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationLease {
    #[serde(rename = "_id")]
    pub id: String,
    pub holder: CoordinationHolder,
    pub lease_id: String,
    pub record_kind: CoordinationLeaseKind,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub acquired_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    /// Durable cursor owned by the named lease. Records with a checkpoint are
    /// retained after expiry so a replacement holder can resume work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Bson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub namespace: String,
    pub key_hash: String,
    pub claim_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateWindowRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub namespace: String,
    pub key_hash: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub window_start: DateTime<Utc>,
    pub count: i64,
    pub last_admission_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBucketRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub namespace: String,
    pub key_hash: String,
    pub tokens_millis: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_refill_at: DateTime<Utc>,
    pub last_admission_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationSlot {
    #[serde(rename = "_id")]
    pub id: String,
    pub namespace: String,
    pub scope_hash: String,
    pub slot: i64,
    pub holder: CoordinationHolder,
    pub lease_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub acquired_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventDedupState {
    Claimed,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDedupRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub namespace: String,
    pub scope_hash: String,
    pub event_hash: String,
    pub state: EventDedupState,
    pub claim_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
