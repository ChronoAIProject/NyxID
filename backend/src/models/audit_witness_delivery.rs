//! Secret-free durable relay cursor. Its trust comes from independently pinned
//! signed history, never from Mongo or a deserialized contact timestamp.
use nyxid::models::audit_witness::{AdvanceRequest, SignedReceipt};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "audit_witness_delivery";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitnessDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub enrollment_digest: String,
    pub receipt: SignedReceipt,
    pub pending: Option<AdvanceRequest>,
    pub divergence: Option<WitnessDivergence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitnessDivergence {
    pub seq: i64,
    pub local_hash: String,
    pub witnessed_hash: String,
}
