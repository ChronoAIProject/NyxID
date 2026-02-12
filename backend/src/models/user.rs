use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "users";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "_id")]
    pub id: String,
    pub email: String,
    pub password_hash: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified: bool,
    pub email_verification_token: Option<String>,
    pub password_reset_token: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub password_reset_expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub is_admin: bool,
    #[serde(default)]
    pub role_ids: Vec<String>,
    #[serde(default)]
    pub group_ids: Vec<String>,
    pub mfa_enabled: bool,
    #[serde(default)]
    pub social_provider: Option<String>,
    #[serde(default)]
    pub social_provider_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_login_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "users");
    }

    fn make_user() -> User {
        User {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            email: "test@example.com".to_string(),
            password_hash: Some("$argon2id$hash".to_string()),
            display_name: Some("Test User".to_string()),
            avatar_url: None,
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            role_ids: vec![],
            group_ids: vec![],
            mfa_enabled: false,
            social_provider: None,
            social_provider_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login_at: None,
        }
    }

    #[test]
    fn bson_roundtrip() {
        let user = make_user();
        let doc = bson::to_document(&user).expect("serialize user to bson");
        // _id field should be present (serde rename)
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: User = bson::from_document(doc).expect("deserialize user from bson");
        assert_eq!(user.id, restored.id);
        assert_eq!(user.email, restored.email);
        assert_eq!(user.is_active, restored.is_active);
    }

    #[test]
    fn bson_roundtrip_with_optional_datetimes() {
        let mut user = make_user();
        user.password_reset_expires_at = Some(Utc::now());
        user.last_login_at = Some(Utc::now());
        let doc = bson::to_document(&user).expect("serialize");
        let restored: User = bson::from_document(doc).expect("deserialize");
        assert!(restored.password_reset_expires_at.is_some());
        assert!(restored.last_login_at.is_some());
    }

    #[test]
    fn bson_all_fields_serialized() {
        let user = make_user();
        let doc = bson::to_document(&user).expect("serialize");
        // Ensure critical fields are present in the document (not skipped)
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"email"));
        assert!(keys.contains(&"is_active"));
        assert!(keys.contains(&"is_admin"));
        assert!(keys.contains(&"mfa_enabled"));
        assert!(keys.contains(&"created_at"));
        assert!(keys.contains(&"updated_at"));
    }
}
