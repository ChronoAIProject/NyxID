use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "crm_links";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrmLink {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub field_key: String,
    pub source_object: String,
    pub source_record_id: String,
    pub target_object: String,
    pub target_record_id: String,
    pub single: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
