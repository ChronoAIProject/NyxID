use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "oauth_states";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthState {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub provider_config_id: String,
    pub code_verifier: Option<String>,
    /// Encrypted device_auth_id (OpenAI) or device_code (RFC 8628) for polling
    #[serde(default)]
    pub device_code_encrypted: Option<String>,
    /// Encrypted user_code needed for OpenAI-style device code polling
    #[serde(default)]
    pub user_code_encrypted: Option<String>,
    /// Polling interval in seconds for device code flow
    #[serde(default)]
    pub poll_interval: Option<i32>,
    /// When an admin initiates a flow on behalf of a service account,
    /// this holds the SA ID. Tokens are stored under this ID instead of user_id.
    #[serde(default)]
    pub target_user_id: Option<String>,
    /// If the flow was initiated with user-provided OAuth app credentials,
    /// this records the credential owner's user ID so later steps keep using
    /// the same OAuth client even if provider config changes.
    #[serde(default)]
    pub credential_user_id: Option<String>,
    /// Custom frontend redirect path after OAuth callback completes.
    /// e.g., "/admin/service-accounts/{sa_id}" for admin flows.
    #[serde(default)]
    pub redirect_path: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "oauth_states");
    }

    #[test]
    fn bson_roundtrip() {
        let state = OAuthState {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            provider_config_id: uuid::Uuid::new_v4().to_string(),
            code_verifier: Some("verifier123".to_string()),
            device_code_encrypted: None,
            user_code_encrypted: None,
            poll_interval: None,
            target_user_id: None,
            credential_user_id: None,
            redirect_path: None,
            expires_at: Utc::now(),
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&state).expect("serialize");
        let restored: OAuthState = bson::from_document(doc).expect("deserialize");
        assert_eq!(state.id, restored.id);
        assert_eq!(state.code_verifier, restored.code_verifier);
        assert!(restored.target_user_id.is_none());
        assert!(restored.credential_user_id.is_none());
        assert!(restored.redirect_path.is_none());
    }

    #[test]
    fn bson_roundtrip_device_code_flow() {
        let state = OAuthState {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            provider_config_id: uuid::Uuid::new_v4().to_string(),
            code_verifier: None,
            device_code_encrypted: Some("encrypted_device_code".to_string()),
            user_code_encrypted: Some("encrypted_user_code".to_string()),
            poll_interval: Some(5),
            target_user_id: None,
            credential_user_id: Some(uuid::Uuid::new_v4().to_string()),
            redirect_path: None,
            expires_at: Utc::now(),
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&state).expect("serialize");
        let restored: OAuthState = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.poll_interval, Some(5));
        assert!(restored.device_code_encrypted.is_some());
        assert!(restored.credential_user_id.is_some());
    }

    #[test]
    fn bson_roundtrip_on_behalf_of() {
        let sa_id = uuid::Uuid::new_v4().to_string();
        let redirect = "/admin/service-accounts/some-sa-id".to_string();
        let state = OAuthState {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            provider_config_id: uuid::Uuid::new_v4().to_string(),
            code_verifier: Some("verifier456".to_string()),
            device_code_encrypted: None,
            user_code_encrypted: None,
            poll_interval: None,
            target_user_id: Some(sa_id.clone()),
            credential_user_id: Some(uuid::Uuid::new_v4().to_string()),
            redirect_path: Some(redirect.clone()),
            expires_at: Utc::now(),
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&state).expect("serialize");
        let restored: OAuthState = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.target_user_id, Some(sa_id));
        assert_eq!(restored.redirect_path, Some(redirect));
        assert!(restored.credential_user_id.is_some());
    }

    #[test]
    fn bson_backward_compat_missing_new_fields() {
        // Simulate a document from before the new fields were added
        let doc = bson::doc! {
            "_id": "state-123",
            "user_id": "user-456",
            "provider_config_id": "prov-789",
            "expires_at": bson::DateTime::from_chrono(Utc::now()),
            "created_at": bson::DateTime::from_chrono(Utc::now()),
        };
        let restored: OAuthState = bson::from_document(doc).expect("deserialize");
        assert!(restored.target_user_id.is_none());
        assert!(restored.credential_user_id.is_none());
        assert!(restored.redirect_path.is_none());
        assert!(restored.code_verifier.is_none());
    }
}
