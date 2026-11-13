use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Separate records in the existing cutover collection let readers fetch both
// facts in one command. The shape is unchanged from the first activation reader;
// earlier cutover-only binaries query just the fixed platform-policy ID.
pub const COLLECTION_NAME: &str = "security_rollouts";
pub const RECORD_KIND: &str = "capability_activation_v1";

/// Durable evidence of clean or settled observed support, not execution authority.
/// No TTL: roster churn cannot erase the deployment's activation history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecurityCapabilityActivation {
    #[serde(rename = "_id")]
    pub id: String,
    pub record_kind: String,
    pub capability: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub active_since: DateTime<Utc>,
}
