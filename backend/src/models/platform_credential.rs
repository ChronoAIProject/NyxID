use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_credentials";

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformCredential {
    #[serde(rename = "_id")]
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, bson::Binary>,
    pub updated_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for PlatformCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformCredential")
            .field("provider", &self.provider)
            .field("fields", &"[REDACTED]")
            .field("secrets", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
