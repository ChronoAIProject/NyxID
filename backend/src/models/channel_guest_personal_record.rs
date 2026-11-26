use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_guest_personal_records";

#[derive(Deserialize, Serialize)]
pub struct ChannelGuestPersonalRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub bot_id: String,
    pub bot_ownership_version: i64,
    #[serde(default)]
    pub meta_grant_id: Option<String>,
    pub owner_id: String,
    pub title: Option<String>,
    pub body: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
