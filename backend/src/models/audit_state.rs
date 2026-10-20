use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "audit_chain_state";

/// One chain per database. The UUID is also the externally enrolled identity;
/// the unique singleton index prevents two independently initialized heads.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditState {
    #[serde(rename = "_id")]
    pub id: String,
    pub singleton: bool,
    pub protocol: u32,
    pub key_version: u32,
    pub head_seq: i64,
    pub head_hash: String,
    pub next_order: i64,
    pub pending_count: i64,
    pub witnessed_seq: i64,
    pub witnessed_hash: String,
    pub witness_enrolled: bool,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub oldest_pending_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub adopted_at: DateTime<Utc>,
    /// Nonzero only for a drained adoption of an existing v1 chain.
    pub adopted_v1_seq: i64,
}
