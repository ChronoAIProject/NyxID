use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const COLLECTION_NAME: &str = "chain_verify_status";

pub const CHAIN_AUDIT_LOG: &str = "audit_log";
pub const CHAIN_BILLING_LEDGER: &str = "billing_ledger";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChainVerifyOutcome {
    Ok,
    Broken,
}

/// Latest automatic verification state for one hash chain, upserted by the
/// chain-verify sweep after every run. One document per chain (`_id` is
/// `audit_log` or `billing_ledger`), so any instance in a multi-instance
/// deployment reads the same authoritative result.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ChainVerifyStatus {
    #[serde(rename = "_id")]
    pub id: String,
    pub outcome: ChainVerifyOutcome,
    /// Next seq the rolling walk resumes from. The sweep verifies the
    /// chain in bounded chunks and wraps back to 1 after reaching the
    /// head, so old regions are continuously re-verified.
    pub cursor_seq: i64,
    pub head_seq: Option<i64>,
    /// Entries checked by the most recent run.
    pub checked_entries: i64,
    /// When the rolling walk last wrapped past the head with no break
    /// found anywhere in that pass.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_full_pass_at: Option<DateTime<Utc>>,
    pub break_seq: Option<i64>,
    pub break_kind: Option<String>,
    pub break_detail: Option<String>,
    /// Billing ledger only: ledger seq recorded by the newest head anchor.
    pub anchor_seq: Option<i64>,
    /// Billing ledger only: false when the newest anchor failed validation.
    pub anchor_valid: Option<bool>,
    /// Audit log only: legacy rows predating the chain.
    pub pre_chain_count: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_run_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
