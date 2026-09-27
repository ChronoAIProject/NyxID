use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "usage_workspaces";

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageWorkspace {
    #[serde(rename = "_id")]
    pub user_id: String,
    pub revision: i64,
    pub config: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
