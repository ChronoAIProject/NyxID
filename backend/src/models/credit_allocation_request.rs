use chrono::{DateTime, Utc};
use mongodb::bson::Document;
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "credit_allocation_requests";

/// Actor-scoped retry identity and immutable submission snapshot. Grant/policy
/// identities are derived from this id; completion is recoverable by replay.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreditAllocationRequest {
    #[serde(rename = "_id")]
    pub id: String,
    pub actor_id: String,
    pub payload: String,
    pub snapshot: Document,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
