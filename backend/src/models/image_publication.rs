//! Immutable publication identities and outcomes. No captions, URLs or credentials.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "image_publications";

#[derive(Clone, Serialize, Deserialize)]
pub struct ImagePublication {
    #[serde(rename = "_id")]
    pub id: String,
    pub key: String,
    pub user_id: String,
    pub conversation_id: String,
    pub connection_id: String,
    pub authority: String,
    pub contract: String,
    pub company: String,
    pub caption_sha256: String,
    pub wire_caption_sha256: String,
    pub attempt: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub lease_until: DateTime<Utc>,
    pub attachment_id: String,
    pub content_sha256: String,
    pub metadata_identity: String,
    pub state: String,
    pub image: Option<String>,
    #[serde(default)]
    pub uploaded: bool,
    pub post: Option<String>,
    pub stage: String,
    pub last_code: Option<String>,
    pub http_status: Option<u16>,
    pub verification: String,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub posted_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub checked_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub verified_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
impl std::fmt::Debug for ImagePublication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ImagePublication([REDACTED])")
    }
}
