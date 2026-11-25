use bson::Binary;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const PRODUCERS: &str = "sandbox_usage_producers_v1";
pub const EVENTS: &str = "sandbox_usage_events_v1";
pub const STREAMS: &str = "sandbox_usage_streams_v1";
pub const EFFECTIVE_FACTS: &str = "sandbox_usage_effective_facts_v1";

#[derive(Deserialize, Serialize)]
pub struct Producer {
    #[serde(rename = "_id")]
    pub service_account_id: String,
    pub namespace: String,
    pub owner_issuer: String,
    pub enabled: bool,
    #[serde(default)]
    pub accepting: bool,
    pub revision: i64,
}

#[derive(Deserialize, Serialize)]
pub struct Event {
    #[serde(rename = "_id")]
    pub id: String,
    pub namespace: String,
    pub source_event_id: String,
    pub stream_id: String,
    pub owner_subject: String,
    pub sequence: i64,
    pub canonical: Binary,
    pub digest: String,
    pub receipt_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub accepted_at: DateTime<Utc>,
}

#[derive(Deserialize, Serialize)]
pub struct Stream {
    #[serde(rename = "_id")]
    pub id: String,
    pub namespace: String,
    pub stream_id: String,
    pub owner_subject: String,
    pub sandbox_id: String,
    pub source_epoch: String,
    pub basis: String,
    pub capture_start_ms: i64,
    pub last_fact_end_ms: i64,
    pub sequence: i64,
    pub head_id: String,
    pub head_digest: String,
}

#[derive(Deserialize, Serialize)]
pub struct EffectiveFact {
    #[serde(rename = "_id")]
    pub id: String,
    pub namespace: String,
    pub stream_id: String,
    pub owner_subject: String,
    pub fact_id: String,
    pub source_event_id: String,
    pub bucket_start_ms: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub runtime_id: Option<String>,
    pub runtime_generation: Option<i64>,
    pub home_id: Option<String>,
    pub home_generation: Option<String>,
}
