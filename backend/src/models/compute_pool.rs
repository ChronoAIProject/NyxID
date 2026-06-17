use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "compute_pools";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ComputePoolVisibility {
    Private,
    Org,
    Platform,
}

impl ComputePoolVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Org => "org",
            Self::Platform => "platform",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ComputeSchedulingPolicy {
    Fifo,
    LeastBusy,
    ModelFit,
}

impl ComputeSchedulingPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fifo => "fifo",
            Self::LeastBusy => "least_busy",
            Self::ModelFit => "model_fit",
        }
    }
}

/// Shared GPU / local-compute capacity pool. A pool is owned by a person
/// or org (`user_id` is polymorphic, matching Node/UserService) and is
/// consumed by users or agent API keys. Workers authenticate with a
/// rotatable token and pull tasks; NyxID never SSHes into arbitrary hosts
/// to run unbounded commands.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComputePool {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub visibility: ComputePoolVisibility,
    pub scheduling_policy: ComputeSchedulingPolicy,
    pub worker_token_hash: String,
    pub max_workers: u32,
    pub max_queue_length: u32,
    pub per_user_max_inflight: u32,
    pub task_timeout_secs: u64,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

pub const DEFAULT_MAX_WORKERS: u32 = 8;
pub const DEFAULT_MAX_QUEUE_LENGTH: u32 = 200;
pub const DEFAULT_PER_USER_MAX_INFLIGHT: u32 = 4;
pub const DEFAULT_TASK_TIMEOUT_SECS: u64 = 7_200;
