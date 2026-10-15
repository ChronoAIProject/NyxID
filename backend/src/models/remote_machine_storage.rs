//! Platform storage samples are observations, not execution or money authority.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageComponent {
    pub kind: String,
    pub used_bytes: u64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub sampled_at: DateTime<Utc>,
}

/// Embedded on ManagedMachine, separate from the Node usage observation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformStorageObservation {
    pub binding: nyxid_machine::managed::Binding,
    pub runtime_id: String,
    pub pod_name: String,
    pub node_name: String,
    pub container_instances: Vec<String>,
    pub source: String,
    pub used_bytes: Option<u64>,
    pub complete: bool,
    pub components: Vec<StorageComponent>,
    pub missing_reasons: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub received_at: DateTime<Utc>,
}
