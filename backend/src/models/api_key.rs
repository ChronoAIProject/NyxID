use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "api_keys";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub name: String,
    /// First 8 characters of the key, used for identification in the UI
    pub key_prefix: String,
    /// SHA-256 hash of the full API key
    pub key_hash: String,
    pub scopes: String,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "api_keys");
    }

    fn make_api_key() -> ApiKey {
        ApiKey {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            name: "My Key".to_string(),
            key_prefix: "abcdef01".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: "read write".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let key = make_api_key();
        let doc = bson::to_document(&key).expect("serialize");
        let restored: ApiKey = bson::from_document(doc).expect("deserialize");
        assert_eq!(key.id, restored.id);
        assert_eq!(key.name, restored.name);
        assert_eq!(key.scopes, restored.scopes);
    }

    #[test]
    fn bson_roundtrip_with_optional_dates() {
        let mut key = make_api_key();
        key.last_used_at = Some(Utc::now());
        key.expires_at = Some(Utc::now());
        let doc = bson::to_document(&key).expect("serialize");
        let restored: ApiKey = bson::from_document(doc).expect("deserialize");
        assert!(restored.last_used_at.is_some());
        assert!(restored.expires_at.is_some());
    }
}
