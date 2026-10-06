use chrono::{DateTime, Utc};
use nyxid_permissions::Policy;
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "permission_key_policies";

#[derive(Clone, Serialize, Deserialize)]
pub struct PermissionPolicy {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub user_service_id: String,
    pub catalog_service_id: String,
    pub execution_authority_digest: String,
    pub policy: Policy,
    pub paused: bool,
    pub revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
