//! Coordination metadata only; never queried by price readers.
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "billing_rate_epoch";
pub const SINGLETON_ID: &str = "rate-cache";

#[derive(Debug, Deserialize, Serialize)]
pub struct RateEpoch {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default)]
    pub generation: i64,
}
