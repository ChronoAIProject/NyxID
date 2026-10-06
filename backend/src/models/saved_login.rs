use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "saved_logins";

#[derive(Clone, Deserialize, Serialize)]
pub struct SavedLogin {
    #[serde(rename = "_id")]
    pub id: String,
    /// Person or organization, using the same owner ACL as nodes.
    pub user_id: String,
    pub label: String,
    pub allowed_origins: Vec<String>,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub username_encrypted: Vec<u8>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub password_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub totp_secret_encrypted: Option<Vec<u8>>,
    pub username_hint: String,
    #[serde(default)]
    pub confirm_each_sign_in: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for SavedLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SavedLogin { [REDACTED] }")
    }
}
