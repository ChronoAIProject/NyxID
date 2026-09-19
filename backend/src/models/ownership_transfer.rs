use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "ownership_transfers";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnershipTransfer {
    #[serde(rename = "_id")]
    pub id: String,
    pub actor_user_id: String,
    pub resource_kind: String,
    pub resource_id: String,
    pub previous_owner_user_id: String,
    pub new_owner_user_id: String,
    pub preview_version: String,
    pub retired_routes: u64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
