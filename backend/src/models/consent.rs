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
    /// Explicitly grants unrestricted service access. Legacy rows lack this
    /// field and deserialize to false.
    #[serde(default)]
    pub allow_all_services: bool,
    /// None means a legacy pre-default-deny consent. Some(ids), including an
    /// empty list, is the explicit resource-owner-approved UserService
    /// allowlist for this client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_service_ids: Option<Vec<String>>,
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
            allow_all_services: false,
            allowed_service_ids: None,
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
        assert_eq!(consent.allow_all_services, restored.allow_all_services);
        assert_eq!(consent.allowed_service_ids, restored.allowed_service_ids);
    }

    #[test]
    fn bson_defaults_service_grant() {
        let now = Utc::now();
        let doc = bson::doc! {
            "_id": "consent-legacy",
            "user_id": "user-1",
            "client_id": "client-1",
            "scopes": "openid",
            "granted_at": bson::DateTime::from_chrono(now),
        };
        let restored: Consent = bson::from_document(doc).expect("deserialize");
        assert!(!restored.allow_all_services);
        assert!(restored.allowed_service_ids.is_none());
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
    fn bson_roundtrip_with_service_grant() {
        let mut consent = make_consent();
        consent.allowed_service_ids = Some(vec!["svc-1".to_string(), "svc-2".to_string()]);
        let doc = bson::to_document(&consent).expect("serialize");
        let restored: Consent = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.allowed_service_ids, consent.allowed_service_ids);
    }

    #[test]
    fn bson_roundtrip_with_explicit_all_services() {
        let mut consent = make_consent();
        consent.allow_all_services = true;
        consent.allowed_service_ids = Some(vec![]);
        let doc = bson::to_document(&consent).expect("serialize");
        let restored: Consent = bson::from_document(doc).expect("deserialize");
        assert!(restored.allow_all_services);
        assert_eq!(restored.allowed_service_ids, Some(vec![]));
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
