//! Versioned private assistant-model admission; stored with assistant execution records.
use super::agent_operation_scope::OperationScopes;
use chrono::Utc;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Admission {
    #[serde(rename = "_id")]
    pub handle: String,
    pub kind: String,
    pub version: i32,
    pub user_id: String,
    pub conversation_id: String,
    pub api_key_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub model: String,
    pub revision: i64,
    pub authorization_revision: i64,
    pub offering: String,
    pub scopes: OperationScopes,
    pub key_scopes: OperationScopes,
    #[serde(default)]
    pub bridge_ready: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: chrono::DateTime<Utc>,
}
