use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::crm_record::CrmActor;

pub const COLLECTION_NAME: &str = "crm_saved_views";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrmSavedView {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub title: String,
    pub revision: i64,
    pub envelope: serde_json::Value,
    pub created_by: CrmActor,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
