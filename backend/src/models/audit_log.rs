use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "audit_log";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditLog {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: Option<String>,
    /// Event type (e.g. "login", "register", "api_key_created")
    pub event_type: String,
    /// Additional event data as JSON
    pub event_data: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}
