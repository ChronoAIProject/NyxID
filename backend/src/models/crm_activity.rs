use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::crm_record::CrmActor;

pub const COLLECTION_NAME: &str = "crm_activities";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrmActivity {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub record_id: String,
    pub object_key: String,
    pub kind: CrmActivityKind,
    pub actor: CrmActor,
    pub summary: CrmActivitySummary,
    pub body: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrmActivityKind {
    Created,
    Updated,
    StageChanged,
    Note,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrmActivitySummary {
    pub changed_fields: Option<Vec<String>>,
    pub from: Option<String>,
    pub to: Option<String>,
}
