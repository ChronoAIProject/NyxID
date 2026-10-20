use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "lark_base_write_photos";

#[derive(Clone, Serialize, Deserialize)]
pub struct LarkBaseWritePhoto {
    #[serde(rename = "_id")]
    pub id: String,
    pub photo_key: String,
    pub user_id: String,
    pub write_id: String,
    pub attempt: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for LarkBaseWritePhoto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LarkBaseWritePhoto([REDACTED])")
    }
}
