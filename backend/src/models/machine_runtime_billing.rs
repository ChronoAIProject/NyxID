//! Durable Machine contracts and evidence; amounts use the shared exact wallet.
use super::{
    credits::Credits,
    service_billing::{BillingMetric, PricingSyncStatus},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const RATE_COLLECTION: &str = "machine_runtime_rates";
pub const QUOTE_COLLECTION: &str = "machine_runtime_quotes";
pub const RUN_COLLECTION: &str = "machine_runtime_runs";
pub const INTERVAL_COLLECTION: &str = "machine_runtime_intervals";
pub const HEAD_COLLECTION: &str = "machine_runtime_heads";

/// Historical rates could only describe this original fixed profile. This
/// serde default must never follow the deployment's selected/default profile.
pub const HISTORICAL_PROFILE_VERSION: &str = "remote-machine";
fn historical_profile_version() -> String {
    HISTORICAL_PROFILE_VERSION.into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRuntimeRate {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default = "historical_profile_version")]
    pub profile_version: String,
    pub metric: BillingMetric,
    pub lago_metric_code: String,
    pub plan_code: String,
    #[serde(with = "crate::models::credits::whole")]
    pub credits_per_second: Credits,
    pub sync_status: PricingSyncStatus,
    pub retired: bool,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub verified_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineRuntimeTerms {
    pub profile_version: String,
    pub image_digest: String,
    pub policy_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineRuntimeQuote {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub terms: MachineRuntimeTerms,
    pub rate_version: String,
    #[serde(with = "crate::models::credits::whole")]
    pub credits_per_second: Credits,
    pub funding_mode: String,
    pub no_instance_cap: bool,
    pub cumulative_whole_seconds: bool,
    pub paused_storage_free: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub accept_before: DateTime<Utc>,
    pub accepted_operation_id: Option<String>,
    pub accepted_run_id: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub accepted_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MachineRuntimeRunState {
    Accepted,
    Running,
    StopPending,
    Stopped,
    Closed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineRuntimeRun {
    #[serde(rename = "_id")]
    pub id: String,
    pub node_id: String,
    pub owner_id: String,
    pub quote_id: String,
    pub operation_id: String,
    pub rate: MachineRuntimeRate,
    pub terms: MachineRuntimeTerms,
    pub state: MachineRuntimeRunState,
    pub runtime_generation: i64,
    pub revision: i64,
    pub generation_admitted_ns: i64,
    /// Nanosecond epoch values retain submillisecond evidence through BSON dates.
    pub ready_ns: Option<i64>,
    pub proven_until_ns: Option<i64>,
    pub proven_evidence_id: Option<String>,
    pub replacement_requested: bool,
    pub stop_ns: Option<i64>,
    /// Physical stop was observed by this instant, but its exact terminal time
    /// is unavailable. Such a bound never advances billable running evidence.
    #[serde(default)]
    pub stop_is_upper_bound: bool,
    pub ready_evidence_id: Option<String>,
    pub stop_evidence_id: Option<String>,
    pub covered_until_ns: i64,
    pub last_window_from_ns: i64,
    pub accounted_until_ns: i64,
    pub cumulative_covered_ns: i64,
    pub cumulative_seconds: i64,
    pub generation_covered_start_ns: i64,
    pub platform_residual_ns: i64,
    /// Uncertain tails are a platform responsibility estimate, not measured
    /// runtime or a debt collectable from later customer funds.
    #[serde(default)]
    pub platform_residual_upper_bound_ns: i64,
    pub generation_residual_recorded: bool,
    pub generation_floor_ns: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineRuntimeInterval {
    #[serde(rename = "_id")]
    pub id: String,
    pub run_id: String,
    pub node_id: String,
    pub runtime_generation: i64,
    pub rate_version: String,
    pub interval_id: String,
    pub usage_row_id: String,
    pub covered_from_ns: i64,
    pub covered_until_ns: i64,
    pub reserved_seconds: i64,
    #[serde(with = "crate::models::credits::whole")]
    pub reserved_total: Credits,
    pub evidence_id: Option<String>,
    pub evidenced_ns: Option<i64>,
    pub quantity: Option<i64>,
    #[serde(default)]
    pub settlement_complete: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineRuntimeHead {
    /// The stable UUID-v4 Node ID; serializes all contracts for this Machine.
    #[serde(rename = "_id")]
    pub node_id: String,
    pub owner_id: String,
    pub active_run_id: String,
    pub revision: i64,
    pub last_stop_ns: Option<i64>,
}
