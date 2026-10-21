use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub const COLLECTION_NAME: &str = "machine_lifecycle_events";
/// Safe, disposable owner timeline projection; durable operation evidence is
/// retained separately and never receives a TTL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineLifecycleEvent {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub machine_id: Option<String>,
    pub operation_id: String,
    pub kind: String,
    pub message: String,
    pub notification: bool,
    pub responsibility: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SafeLifecycleEvent {
    pub id: String,
    pub machine_id: Option<String>,
    pub operation_id: String,
    pub kind: String,
    pub message: String,
    pub notification: bool,
    pub responsibility: String,
    pub created_at: DateTime<Utc>,
}
