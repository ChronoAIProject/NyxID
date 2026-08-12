use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "durable_operation_executions";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableExecutionStatus {
    Reserved,
    Dispatched,
    Completed,
    Failed,
    Rejected,
    OutcomeUncertain,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DurableOperationExecution {
    #[serde(rename = "_id")]
    pub id: String,
    pub grant_id: String,
    pub operation_id: String,
    pub api_key_id: String,
    pub user_id: String,
    pub user_service_id: String,
    pub endpoint_id: String,
    pub contract_digest: String,
    pub request_digest: String,
    pub status: DurableExecutionStatus,
    #[serde(default)]
    pub downstream_attempts: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_detail: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub dispatched_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub terminal_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
