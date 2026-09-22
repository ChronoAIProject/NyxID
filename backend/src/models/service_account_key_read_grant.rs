use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_account_key_read_grants";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyReadTarget {
    pub user_service_id: String,
    pub owner_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceAccountKeyReadGrant {
    #[serde(rename = "_id")]
    pub service_account_id: String,
    pub owner_id: String,
    pub targets: Vec<KeyReadTarget>,
    pub issued_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub issued_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}
