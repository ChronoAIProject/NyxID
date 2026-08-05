use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "durable_operation_grants";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DurableValueConstraint {
    Exact { value: Value },
    OneOf { values: Vec<Value> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DurableParameterConstraint {
    #[serde(default)]
    pub required: bool,
    #[serde(flatten)]
    pub rule: DurableValueConstraint,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DurableBodyConstraint {
    /// JSON Pointer -> bounded value rule. The empty pointer addresses the
    /// complete JSON body and is the simplest exact-body constraint.
    #[serde(default)]
    pub fields: BTreeMap<String, DurableParameterConstraint>,
    /// Phase 1 requires this to remain false.
    #[serde(default)]
    pub allow_additional_fields: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DurableOperationConstraints {
    #[serde(default)]
    pub path: BTreeMap<String, DurableParameterConstraint>,
    #[serde(default)]
    pub query: BTreeMap<String, DurableParameterConstraint>,
    #[serde(default)]
    pub headers: BTreeMap<String, DurableParameterConstraint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<DurableBodyConstraint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DurableUsageWindow {
    pub duration_seconds: i64,
    pub max_operations: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DurableReplayPolicy {
    #[default]
    NonReplayable,
    DownstreamIdempotencyKey,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DurableClientAuditBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_site: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DurableOperationSelection {
    pub user_service_id: String,
    pub endpoint_id: String,
    pub constraints: DurableOperationConstraints,
    pub valid_from: String,
    pub expires_at: String,
    pub total_limit: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<DurableUsageWindow>,
    #[serde(default)]
    pub replay_policy: DurableReplayPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_audit_binding: Option<DurableClientAuditBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DurableOperationPlan {
    pub user_service_id: String,
    pub endpoint_id: String,
    pub method: String,
    pub normalized_path_template: String,
    pub contract_digest: String,
    pub constraints: DurableOperationConstraints,
    pub valid_from: String,
    pub expires_at: String,
    pub total_limit: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<DurableUsageWindow>,
    pub replay_policy: DurableReplayPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_audit_binding: Option<DurableClientAuditBinding>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DurableOperationGrant {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub api_key_id: String,
    pub user_service_id: String,
    pub endpoint_id: String,
    pub method: String,
    pub normalized_path_template: String,
    pub contract_digest: String,
    pub constraints: DurableOperationConstraints,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub valid_from: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub total_limit: i64,
    #[serde(default)]
    pub total_used: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<DurableUsageWindow>,
    #[serde(default, with = "bson_datetime::optional")]
    pub window_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub window_used: i64,
    pub replay_policy: DurableReplayPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_audit_binding: Option<DurableClientAuditBinding>,
    #[serde(default, with = "bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
    pub state_version: i64,
    pub created_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reauthorized_from: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
