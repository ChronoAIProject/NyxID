//! Secret-free witness protocol DTOs, not Mongo models or application payloads.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditLink {
    pub key_version: u32,
    pub seq: u64,
    pub prev_hash: String,
    pub entry_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdvanceRequest {
    pub chain_id: Uuid,
    pub request_id: Uuid,
    pub previous_receipt: String,
    pub links: Vec<AuditLink>,
}

/// One protected dispatch claim per logical operation, without lease expiry.
/// Retrieving a receipt does not confer permission to dispatch on a new process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchRequest {
    pub chain_id: Uuid,
    pub operation_id: Uuid,
    pub attempt_id: Uuid,
    pub process_id: Uuid,
    pub intent_seq: u64,
    pub intent_hash: String,
    pub previous_receipt: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WitnessEvent {
    Enrollment { head: AuditLink },
    Advance { request: AdvanceRequest },
    Dispatch { request: DispatchRequest },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptBody {
    pub protocol: u32,
    pub chain_id: Uuid,
    pub receipt_seq: u64,
    pub previous_receipt: String,
    pub witness_time_ms: u64,
    pub head: AuditLink,
    pub event: WitnessEvent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedReceipt {
    pub body: ReceiptBody,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveRequest {
    pub chain_id: Uuid,
    pub challenge: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationBody {
    pub protocol: u32,
    pub chain_id: Uuid,
    pub challenge: String,
    pub witness_time_ms: u64,
    pub latest: SignedReceipt,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedObservation {
    pub body: ObservationBody,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRequest {
    pub chain_id: Uuid,
    pub from_receipt: u64,
    pub limit: usize,
}

/// Provisioned once by an independent administrator. No enrollment HTTP route.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub protocol: u32,
    pub chain_id: Uuid,
    pub initial_head: AuditLink,
    pub verification_key: String,
    pub append_token_sha256: String,
}
