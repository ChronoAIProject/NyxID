//! Short-lived source locators. File URLs and source bodies are never persisted.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_resources";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOutput {
    Attach,
    Inspect,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResourceConfirmation {
    pub acknowledgement_id: String,
    pub output: ResourceOutput,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceSource {
    pub adapter: String,
    pub service_id: String,
    pub endpoint_id: String,
    pub generation: i64,
    pub endpoint_digest: String,
    pub authority_digest: String,
    pub tool_name: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantResource {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub api_key_id: String,
    pub agent_id: String,
    pub guest: bool,
    pub identity: String,
    pub source: ResourceSource,
    pub arguments: serde_json::Value,
    pub selector: String,
    pub revision: String,
    pub attachment_id: String,
    pub completed: bool,
    #[serde(default)]
    pub pending_confirmation: Option<ResourceConfirmation>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
