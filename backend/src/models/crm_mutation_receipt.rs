use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "crm_mutation_receipts";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrmMutationReceipt {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub request_digest: String,
    pub response: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
