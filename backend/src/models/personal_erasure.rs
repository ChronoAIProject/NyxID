//! Durable accepted personal-erasure intent. Retained checkpoints contain no
//! bearer credentials or user content. Domain evidence survives source removal.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "personal_erasure_jobs";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErasureStatus {
    Accepted,
    Pending,
    OperatorAction,
    Complete,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DomainStatus {
    Pending,
    Complete,
    OperatorAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DomainCheckpoint {
    /// Explicit operator retry round. An old request cannot renew a later round.
    #[serde(default)]
    pub retry_generation: i64,
    #[serde(default)]
    pub last_retry_key_hash: Option<String>,
    #[serde(default)]
    pub last_retry_request_digest: Option<String>,
    /// Consecutive failed adapter passes; normal durable progress resets it.
    #[serde(default)]
    pub consecutive_failures: u32,
    pub domain: String,
    pub status: DomainStatus,
    /// Finite service-owned reason code, never a provider response/error body.
    pub reason: Option<String>,
    /// Adapter-owned bounded progress. Never an offset into a mutable result.
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalErasureJob {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub requested_by: String,
    pub owner_deletion_policy: Option<String>,
    pub status: ErasureStatus,
    pub inventory_version: u32,
    pub domains: Vec<DomainCheckpoint>,
    pub lease_token: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub retry_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub accepted_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
}
