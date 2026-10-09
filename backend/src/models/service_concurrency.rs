use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_concurrency_leases";

/// Limits count acting people (or service accounts), never credential owners.
/// None is explicitly unlimited, including in overrides.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConcurrencyOverride {
    pub id: String,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceConcurrencyPolicy {
    /// Server-bound catalog identity, preserved when resolving a UserService.
    pub service_id: String,
    pub default_limit: Option<u32>,
    #[serde(default)]
    pub users: Vec<ConcurrencyOverride>,
    #[serde(default)]
    pub orgs: Vec<ConcurrencyOverride>,
}
