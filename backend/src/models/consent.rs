use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "consents";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consent {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub client_id: String,
    pub scopes: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub granted_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "consents");
    }

    fn make_consent() -> Consent {
        Consent {
            id: "550e8400-e29b-41d4-a716-446655440002".to_string(),
            user_id: "user-1".to_string(),
            client_id: "client-1".to_string(),
            scopes: "openid profile email".to_string(),
            granted_at: Utc::now(),
            expires_at: None,
        }
    }

    #[test]
    fn bson_roundtrip() {
        let consent = make_consent();
        let doc = bson::to_document(&consent).expect("serialize consent to bson");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: Consent = bson::from_document(doc).expect("deserialize consent from bson");
        assert_eq!(consent.id, restored.id);
        assert_eq!(consent.user_id, restored.user_id);
        assert_eq!(consent.scopes, restored.scopes);
    }

    #[test]
    fn bson_roundtrip_with_expires() {
        let mut consent = make_consent();
        consent.expires_at = Some(Utc::now());
        let doc = bson::to_document(&consent).expect("serialize");
        let restored: Consent = bson::from_document(doc).expect("deserialize");
        assert!(restored.expires_at.is_some());
    }

    #[test]
    fn bson_all_fields_serialized() {
        let consent = make_consent();
        let doc = bson::to_document(&consent).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"user_id"));
        assert!(keys.contains(&"client_id"));
        assert!(keys.contains(&"scopes"));
        assert!(keys.contains(&"granted_at"));
    }
}
