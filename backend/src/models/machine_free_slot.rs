use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "machine_free_slots";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FreeSlotState {
    Reserved,
    Active,
    Releasing,
    Released,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MachineFreeSlot {
    #[serde(rename = "_id")]
    pub owner_id: String,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub operation_id: Option<String>,
    pub state: FreeSlotState,
    pub revision: i64,
    #[serde(default)]
    pub release_evidence_digest: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub released_at: Option<DateTime<Utc>>,
}
