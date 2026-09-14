use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "app_requirement_results";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppRequirementResult {
    #[serde(rename = "_id")]
    pub id: String,
    pub oauth_client_id: String,
    pub user_id: String,
    pub manifest_id: String,
    pub manifest_version: u32,
    pub selections: Vec<RequirementSelection>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementSelection {
    pub requirement_id: String,
    pub user_service_id: Option<String>,
    #[serde(default)]
    pub explicit: bool,
}
