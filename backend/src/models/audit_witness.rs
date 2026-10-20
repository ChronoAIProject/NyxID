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
    #[serde(with = "uuid_text")]
    pub chain_id: Uuid,
    #[serde(with = "uuid_text")]
    pub request_id: Uuid,
    pub previous_receipt: String,
    pub links: Vec<AuditLink>,
}

/// One protected dispatch claim per logical operation, without lease expiry.
/// Retrieving a receipt does not confer permission to dispatch on a new process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchRequest {
    #[serde(with = "uuid_text")]
    pub chain_id: Uuid,
    #[serde(with = "uuid_text")]
    pub operation_id: Uuid,
    #[serde(with = "uuid_text")]
    pub attempt_id: Uuid,
    #[serde(with = "uuid_text")]
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
    #[serde(with = "uuid_text")]
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
    #[serde(with = "uuid_text")]
    pub chain_id: Uuid,
    pub challenge: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationBody {
    pub protocol: u32,
    #[serde(with = "uuid_text")]
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
    #[serde(with = "uuid_text")]
    pub chain_id: Uuid,
    pub from_receipt: u64,
    pub limit: usize,
}

/// Provisioned once by an independent administrator. No enrollment HTTP route.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub protocol: u32,
    #[serde(with = "uuid_text")]
    pub chain_id: Uuid,
    pub initial_head: AuditLink,
    pub verification_key: String,
    pub append_token_sha256: String,
}

// UUID's default serde switches representations on is_human_readable(). The
// Mongo raw-document reader and serde's in-memory BSON serializer disagree on
// that flag. Explicit text preserves the existing HTTP JSON and canonical raw
// UUID signature bytes while making nested durable receipts round-trip.
mod uuid_text {
    use serde::{Deserialize, Deserializer, Serializer};
    use uuid::Uuid;
    pub fn serialize<S: Serializer>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&id.hyphenated().to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
        let text = String::deserialize(deserializer)?;
        Uuid::parse_str(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_witness_uuid_json_is_unchanged_and_raw_bson_round_trips() {
        let request = DispatchRequest {
            chain_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            process_id: Uuid::new_v4(),
            intent_seq: 1,
            intent_hash: "a".repeat(64),
            previous_receipt: "b".repeat(64),
        };
        let receipt = SignedReceipt {
            body: ReceiptBody {
                protocol: 1,
                chain_id: request.chain_id,
                receipt_seq: 2,
                previous_receipt: request.previous_receipt.clone(),
                witness_time_ms: 1,
                head: AuditLink {
                    key_version: 1,
                    seq: 1,
                    prev_hash: "0".repeat(64),
                    entry_hash: "a".repeat(64),
                },
                event: WitnessEvent::Dispatch {
                    request: request.clone(),
                },
            },
            signature: "0".repeat(128),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap()["chain_id"],
            request.chain_id.hyphenated().to_string()
        );
        let encoded = bson::to_vec(&receipt).unwrap();
        assert_eq!(
            bson::from_slice::<SignedReceipt>(&encoded).unwrap(),
            receipt
        );
        assert_eq!(
            serde_json::from_str::<SignedReceipt>(&serde_json::to_string(&receipt).unwrap())
                .unwrap(),
            receipt
        );
    }
}
