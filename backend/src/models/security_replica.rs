use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "security_replicas";

/// Observed process support, never a substitute for draining old writers or
/// configuring the complete serving roster. One row per process generation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecurityReplica {
    #[serde(rename = "_id")]
    pub id: String,
    pub instance_name: String,
    pub generation_id: String,
    pub protocol: u32,
    pub capabilities: Vec<String>,
    pub expected_replicas: Vec<String>,
    pub drained: bool,
    pub platform_policy_required: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub observed_at: DateTime<Utc>,
}
