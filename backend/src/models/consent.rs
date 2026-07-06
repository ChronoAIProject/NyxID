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
    #[serde(default = "default_allow_all_services")]
    pub allow_all_services: bool,
    #[serde(default)]
    pub allowed_service_ids: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub granted_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}

pub fn default_allow_all_services() -> bool {
    true
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
            allow_all_services: true,
            allowed_service_ids: Vec::new(),
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
    fn bson_legacy_row_defaults_to_allow_all_services() {
        let now = Utc::now();
        let doc = bson::doc! {
            "_id": "consent-legacy",
            "user_id": "user-1",
            "client_id": "client-1",
            "scopes": "openid profile",
            "granted_at": bson::DateTime::from_chrono(now),
        };

        let restored: Consent = bson::from_document(doc).expect("deserialize legacy consent");
        assert!(restored.allow_all_services);
        assert!(restored.allowed_service_ids.is_empty());
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
        assert!(keys.contains(&"allow_all_services"));
        assert!(keys.contains(&"allowed_service_ids"));
        assert!(keys.contains(&"granted_at"));
    }
}
