pub use super::auth_device_code::AuthDeviceCodeStatus as AgentKeyLoginStatus;
use super::{bson_datetime, login_client_context::LoginClientContext};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "agent_key_login_requests";

#[derive(Clone, Deserialize, Serialize)]
pub struct AgentKeyLoginRequest {
    #[serde(rename = "_id")]
    pub id: String,
    pub device_code_hmac: String,
    pub user_code_hmac: String,
    pub status: AgentKeyLoginStatus,
    pub poll_interval_secs: u32,
    #[serde(default)]
    pub slow_down_increments: u32,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_polled_at: Option<DateTime<Utc>>,
    #[serde(flatten)]
    pub context: LoginClientContext,
    pub client_ip_hmac: Option<String>,
    pub approved_user_id: Option<String>,
    pub approver_ip_hmac: Option<String>,
    pub api_key_id: Option<String>,
    pub credential_id: Option<String>,
    #[serde(default)]
    pub key_was_created: bool,
    #[serde(default)]
    pub key_created_by_approval: bool,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub delivery_credential_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub approved_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub delivered_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub denied_at: Option<DateTime<Utc>>,
    pub denied_by_user_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for AgentKeyLoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentKeyLoginRequest")
            .field("id", &self.id)
            .field("status", &self.status)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn bson_dates_ciphertext_legacy_defaults_and_redaction() {
        let now = bson::DateTime::now();
        let mut row: AgentKeyLoginRequest = bson::from_document(doc! {
            "_id": "request", "device_code_hmac": "private-device-hmac",
            "user_code_hmac": "private-user-hmac", "status": "pending",
            "poll_interval_secs": 5, "created_at": now, "expires_at": now,
        })
        .unwrap();
        assert_eq!(row.slow_down_increments, 0);
        assert!(!row.key_was_created);
        assert!(row.context.client_label.is_none() && row.context.requested_profile.is_none());
        assert!(row.approved_at.is_none() && row.last_polled_at.is_none());
        row.approved_at = Some(now.to_chrono());
        row.delivered_at = Some(now.to_chrono());
        row.denied_at = Some(now.to_chrono());
        row.last_polled_at = Some(now.to_chrono());
        row.delivery_credential_encrypted = Some(b"encrypted-login-material".to_vec());
        row.context.requested_profile = Some("fixture-profile".into());
        let raw = bson::to_vec(&row).unwrap();
        let raw_restored: AgentKeyLoginRequest = bson::from_slice(&raw).unwrap();
        assert_eq!(
            raw_restored.context.requested_profile.as_deref(),
            Some("fixture-profile")
        );
        let document = bson::to_document(&row).unwrap();
        for field in [
            "created_at",
            "expires_at",
            "approved_at",
            "delivered_at",
            "denied_at",
            "last_polled_at",
        ] {
            assert_eq!(document.get_datetime(field).unwrap(), &now);
        }
        assert_eq!(
            document
                .get_binary_generic("delivery_credential_encrypted")
                .unwrap(),
            b"encrypted-login-material"
        );
        let restored: AgentKeyLoginRequest = bson::from_document(document).unwrap();
        assert_eq!(
            restored.delivery_credential_encrypted,
            row.delivery_credential_encrypted
        );
        let debug = format!("{restored:?}");
        for secret in [
            "private-device-hmac",
            "private-user-hmac",
            "encrypted-login-material",
        ] {
            assert!(!debug.contains(secret));
        }
    }
}
