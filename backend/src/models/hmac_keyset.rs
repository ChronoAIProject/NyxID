//! Immutable consistency record; commitments never contain key material.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub const COLLECTION_NAME: &str = "security_hmac_keysets";
#[derive(Serialize, Deserialize)]
pub(crate) struct HmacKeysetPin {
    #[serde(rename = "_id")]
    pub id: String,
    pub version: i32,
    pub commitments: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
