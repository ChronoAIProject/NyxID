use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "user_provider_credentials";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserProviderCredentials {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub provider_config_id: String,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub client_id_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub client_secret_encrypted: Option<Vec<u8>>,
    pub label: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "user_provider_credentials");
    }

    #[test]
    fn bson_roundtrip_with_credentials() {
        let cred = UserProviderCredentials {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            provider_config_id: uuid::Uuid::new_v4().to_string(),
            client_id_encrypted: Some(vec![1, 2, 3]),
            client_secret_encrypted: Some(vec![4, 5, 6]),
            label: Some("My Twitter App".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&cred).expect("serialize");
        let restored: UserProviderCredentials = bson::from_document(doc).expect("deserialize");
        assert_eq!(cred.id, restored.id);
        assert_eq!(cred.user_id, restored.user_id);
        assert!(restored.client_id_encrypted.is_some());
        assert!(restored.client_secret_encrypted.is_some());
    }

    #[test]
    fn bson_roundtrip_without_secrets() {
        let cred = UserProviderCredentials {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            provider_config_id: uuid::Uuid::new_v4().to_string(),
            client_id_encrypted: None,
            client_secret_encrypted: None,
            label: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&cred).expect("serialize");
        let restored: UserProviderCredentials = bson::from_document(doc).expect("deserialize");
        assert_eq!(cred.id, restored.id);
        assert!(restored.client_id_encrypted.is_none());
    }
}
