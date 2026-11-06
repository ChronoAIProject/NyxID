use chrono::{DateTime, Utc};
use mongodb::bson::Document;
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "crm_records";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrmRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub object_id: String,
    pub object_key: String,
    pub revision: i64,
    pub values: Document,
    pub refs: Document,
    pub overrides: Vec<String>,
    #[serde(default)]
    pub field_sources: Document,
    pub created_by: CrmActor,
    pub updated_by: CrmActor,
    #[serde(with = "super::bson_datetime::optional")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrmActor {
    pub kind: CrmActorKind,
    pub user_id: String,
    pub agent_id: Option<String>,
    pub api_key_id: Option<String>,
    #[serde(default)]
    pub source: Option<CrmSource>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrmActorKind {
    Human,
    Agent,
}

impl CrmActor {
    pub fn human(user_id: impl Into<String>) -> Self {
        Self {
            kind: CrmActorKind::Human,
            user_id: user_id.into(),
            agent_id: None,
            api_key_id: None,
            source: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrmSource {
    pub conversation_id: String,
    pub turn_id: String,
    pub run_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub ordered_at: DateTime<Utc>,
}
