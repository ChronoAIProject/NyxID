use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_change_events";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryActorKind {
    Person,
    ApiKey,
    ServiceAccount,
    App,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryActor {
    pub kind: HistoryActorKind,
    pub id: String,
    pub name: String,
    pub person_id: Option<String>,
    pub api_key_id: Option<String>,
    pub app_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryContext {
    pub actor: HistoryActor,
    pub change_group_id: String,
    pub operation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceChangeSummary {
    pub actor: HistoryActor,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub at: DateTime<Utc>,
    pub action: String,
    pub change_group_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SafeFieldChange {
    pub field: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceChangeEvent {
    #[serde(rename = "_id")]
    pub id: String,
    pub schema_version: u32,
    pub service_sequence: i64,
    pub service_id: String,
    #[serde(default)]
    pub service_slug: String,
    pub owner_id: String,
    pub change_group_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub action: String,
    pub actor: HistoryActor,
    pub operation: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub committed_at: DateTime<Utc>,
    pub changes: Vec<SafeFieldChange>,
    pub additional_changes: bool,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub audited_at: Option<DateTime<Utc>>,
    pub audit_log_id: Option<String>,
    #[serde(default)]
    pub audit_attempts: u32,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub next_audit_attempt_at: Option<DateTime<Utc>>,
}

/// Per-instance ordering survives physical service cleanup. It is not an audit chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceHistoryHead {
    #[serde(rename = "_id")]
    pub id: String,
    pub sequence: i64,
}
pub const HEADS_COLLECTION_NAME: &str = "service_history_heads";
