use super::scalars::nullable;
pub use super::scalars::{Counter, Digest, Id, Millis, Quantity, SourcePart, Text};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub schema_version: u8,
    pub producer_namespace: Text,
    pub stream_id: Id,
    pub event_id: Id,
    pub sequence: Counter,
    #[serde(deserialize_with = "nullable")]
    pub previous_event_digest: Option<Digest>,
    pub owner_ref: OwnerRef,
    pub sandbox_id: Text,
    pub source_epoch: Id,
    pub basis: Basis,
    pub metric_definition_version: Text,
    pub algorithm_version: Text,
    pub source_refs: Vec<SourceRef>,
    pub source_digest: Digest,
    pub observed_at_ms: Millis,
    pub source_cutoff_ms: Millis,
    pub produced_at_ms: Millis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerRef {
    pub issuer: Text,
    pub subject: Text,
    /// Unknown owner kinds retain evidence; resolution belongs to the inbox.
    pub kind: Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    Allocated,
    Measured,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub kind: Text,
    pub id: Text,
    pub revision: Counter,
    pub part: SourcePart,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Payload {
    StreamOpen { opening: Opening },
    Usage { facts: Vec<Fact> },
    Correction { correction: Box<Correction> },
    StreamSeal { seal: Seal },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Opening {
    pub first_source: SourceRef,
    pub capture_start_ms: Millis,
    pub tracking_started_at_ms: Millis,
    pub complete_lifecycle: bool,
    pub historical_missing_ranges: Vec<TimeRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeRange {
    pub start_ms: Millis,
    pub end_ms: Millis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seal {
    pub final_source: SourceRef,
    pub sealed_at_ms: Millis,
    pub reason: SealReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealReason {
    HomeReleased,
    OwnerChanged,
    IncarnationEnded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub fact_id: Id,
    pub evidence_revision: Counter,
    pub bucket_start_ms: Millis,
    pub start_ms: Millis,
    pub end_ms: Millis,
    pub resource: ResourceIdentity,
    #[serde(deserialize_with = "nullable")]
    pub allocation: Option<Allocation>,
    pub quantities: BTreeMap<Metric, Option<Quantity>>,
    pub audit_time: AuditTime,
    pub coverage: CoverageSet,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceIdentity {
    #[serde(deserialize_with = "nullable")]
    pub runtime_id: Option<Text>,
    #[serde(deserialize_with = "nullable")]
    pub runtime_generation: Option<Counter>,
    #[serde(deserialize_with = "nullable")]
    pub home_id: Option<Text>,
    #[serde(deserialize_with = "nullable")]
    pub home_generation: Option<Text>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allocation {
    pub cpu_millis: Quantity,
    pub memory_mib: Quantity,
    pub disk_gib: Quantity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditTime {
    pub compute_uptime_ms: Millis,
    pub retained_home_ms: Millis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageSet {
    pub cpu: Coverage,
    pub memory: Coverage,
    pub disk: Coverage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub covered_binding_ms: Quantity,
    pub expected_binding_ms: Quantity,
    pub quality: Quality,
    #[serde(deserialize_with = "nullable")]
    pub missing_reason: Option<Text>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Observed,
    Partial,
    Missing,
    Estimated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum Metric {
    #[serde(rename = "sandbox_cpu_millis_seconds")]
    AllocatedCpu,
    #[serde(rename = "sandbox_memory_mib_seconds")]
    AllocatedMemory,
    #[serde(rename = "sandbox_disk_gib_seconds")]
    AllocatedDisk,
    #[serde(rename = "sandbox_cpu_core_nanoseconds_v1")]
    MeasuredCpu,
    #[serde(rename = "sandbox_memory_working_set_byte_milliseconds_v1")]
    MeasuredMemory,
    #[serde(rename = "sandbox_disk_used_byte_milliseconds_v1")]
    MeasuredDisk,
}

impl Metric {
    /// Exact resource-time units per core-hour / GiB-hour, never a price.
    pub fn units_per_hour(self) -> u128 {
        match self {
            Self::AllocatedCpu => 3_600_000,
            Self::AllocatedMemory => 3_686_400,
            Self::AllocatedDisk => 3_600,
            Self::MeasuredCpu => 3_600_000_000_000,
            Self::MeasuredMemory | Self::MeasuredDisk => (1_u128 << 30) * 3_600_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Correction {
    pub fact_id: Id,
    pub expected_predecessor_event_id: Id,
    pub expected_predecessor_digest: Digest,
    pub expected_evidence_revision: Counter,
    pub replacement: Fact,
    pub reason: CorrectionReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionReason {
    LateSample,
    BindingRepair,
    AlgorithmCorrection,
    LifecycleCorrection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub receipt_id: Id,
    pub event_id: Id,
    pub stream_id: Id,
    pub sequence: Counter,
    pub payload_digest: Digest,
    pub accepted_at_ms: Millis,
    pub custody: Custody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Custody {
    Persisted,
}
