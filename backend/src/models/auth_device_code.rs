use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::bson_datetime;
use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "auth_device_codes";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthDeviceCodeStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    Delivered,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthDeviceCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub device_code_hmac: String,
    pub user_code_hmac: String,
    pub status: AuthDeviceCodeStatus,
    pub poll_interval_secs: u32,
    pub slow_down_increments: u32,
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
    pub client_ip_hmac: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_polled_at: Option<DateTime<Utc>>,
    pub approved_user_id: Option<String>,
    pub approved_session_id: Option<String>,
    pub approver_ip_hmac: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub delivery_access_token_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub delivery_refresh_token_encrypted: Option<Vec<u8>>,
    pub delivery_access_token_expires_in: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub approved_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub delivered_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub denied_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for AuthDeviceCodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => f.write_str("Pending"),
            Self::Approved => f.write_str("Approved"),
            Self::Denied => f.write_str("Denied"),
            Self::Expired => f.write_str("Expired"),
            Self::Delivered => f.write_str("Delivered"),
        }
    }
}

impl fmt::Debug for AuthDeviceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthDeviceCode")
            .field("id", &self.id)
            .field(
                "device_code_hmac",
                &RedactedLen(self.device_code_hmac.len()),
            )
            .field("user_code_hmac", &RedactedLen(self.user_code_hmac.len()))
            .field("status", &self.status)
            .field("poll_interval_secs", &self.poll_interval_secs)
            .field("slow_down_increments", &self.slow_down_increments)
            .field("client_label", &self.client_label)
            .field("client_user_agent", &self.client_user_agent)
            .field(
                "client_ip_hmac",
                &self
                    .client_ip_hmac
                    .as_ref()
                    .map(|hash| RedactedLen(hash.len())),
            )
            .field("last_polled_at", &self.last_polled_at)
            .field("approved_user_id", &self.approved_user_id)
            .field("approved_session_id", &self.approved_session_id)
            .field(
                "approver_ip_hmac",
                &self
                    .approver_ip_hmac
                    .as_ref()
                    .map(|hash| RedactedLen(hash.len())),
            )
            .field(
                "delivery_access_token_encrypted",
                &self
                    .delivery_access_token_encrypted
                    .as_ref()
                    .map(|bytes| RedactedLen(bytes.len())),
            )
            .field(
                "delivery_refresh_token_encrypted",
                &self
                    .delivery_refresh_token_encrypted
                    .as_ref()
                    .map(|bytes| RedactedLen(bytes.len())),
            )
            .field(
                "delivery_access_token_expires_in",
                &self.delivery_access_token_expires_in,
            )
            .field("created_at", &self.created_at)
            .field("approved_at", &self.approved_at)
            .field("delivered_at", &self.delivered_at)
            .field("denied_at", &self.denied_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_auth_device_code() -> AuthDeviceCode {
        let now = Utc::now();
        AuthDeviceCode {
            id: uuid::Uuid::new_v4().to_string(),
            device_code_hmac: "abc123ff".repeat(8),
            user_code_hmac: "def456aa".repeat(8),
            status: AuthDeviceCodeStatus::Pending,
            poll_interval_secs: 5,
            slow_down_increments: 0,
            client_label: Some("wsl-calvin".to_string()),
            client_user_agent: Some("nyxid-cli/0.8.0".to_string()),
            client_ip_hmac: Some("11112222".repeat(8)),
            last_polled_at: Some(now + chrono::Duration::seconds(5)),
            approved_user_id: Some(uuid::Uuid::new_v4().to_string()),
            approved_session_id: Some(uuid::Uuid::new_v4().to_string()),
            approver_ip_hmac: Some("33334444".repeat(8)),
            delivery_access_token_encrypted: Some(vec![0xab, 0xcd, 0xef]),
            delivery_refresh_token_encrypted: Some(vec![0x12, 0x34, 0x56]),
            delivery_access_token_expires_in: Some(900),
            created_at: now,
            approved_at: Some(now + chrono::Duration::seconds(10)),
            delivered_at: Some(now + chrono::Duration::seconds(20)),
            denied_at: None,
            expires_at: now + chrono::Duration::minutes(10),
        }
    }

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "auth_device_codes");
    }

    #[test]
    fn status_serializes_as_lowercase() {
        let value = serde_json::to_value(AuthDeviceCodeStatus::Pending).expect("serialize status");
        assert_eq!(value, serde_json::json!("pending"));
    }

    #[test]
    fn bson_roundtrip_preserves_struct_identity() {
        let row = make_auth_device_code();
        let doc = bson::to_document(&row).expect("serialize");
        let restored: AuthDeviceCode = bson::from_document(doc).expect("deserialize");

        assert_eq!(row.id, restored.id);
        assert_eq!(row.device_code_hmac, restored.device_code_hmac);
        assert_eq!(row.user_code_hmac, restored.user_code_hmac);
        assert_eq!(row.status, restored.status);
        assert_eq!(row.poll_interval_secs, restored.poll_interval_secs);
        assert_eq!(row.slow_down_increments, restored.slow_down_increments);
        assert_eq!(row.client_label, restored.client_label);
        assert_eq!(row.client_user_agent, restored.client_user_agent);
        assert_eq!(row.client_ip_hmac, restored.client_ip_hmac);
        assert_eq!(row.approved_user_id, restored.approved_user_id);
        assert_eq!(row.approved_session_id, restored.approved_session_id);
        assert_eq!(row.approver_ip_hmac, restored.approver_ip_hmac);
        assert_eq!(
            row.delivery_access_token_encrypted,
            restored.delivery_access_token_encrypted
        );
        assert_eq!(
            row.delivery_refresh_token_encrypted,
            restored.delivery_refresh_token_encrypted
        );
        assert_eq!(
            row.delivery_access_token_expires_in,
            restored.delivery_access_token_expires_in
        );
        assert_eq!(
            row.created_at.timestamp_millis(),
            restored.created_at.timestamp_millis()
        );
        assert_eq!(
            row.last_polled_at.unwrap().timestamp_millis(),
            restored.last_polled_at.unwrap().timestamp_millis()
        );
        assert_eq!(
            row.approved_at.unwrap().timestamp_millis(),
            restored.approved_at.unwrap().timestamp_millis()
        );
        assert_eq!(
            row.delivered_at.unwrap().timestamp_millis(),
            restored.delivered_at.unwrap().timestamp_millis()
        );
        assert_eq!(row.denied_at, restored.denied_at);
        assert_eq!(
            row.expires_at.timestamp_millis(),
            restored.expires_at.timestamp_millis()
        );
    }

    #[test]
    fn debug_redacts_hashes_and_ciphertext_but_prints_safe_fields() {
        let row = make_auth_device_code();
        let debug = format!("{row:?}");

        for secret in [
            row.device_code_hmac.as_str(),
            row.user_code_hmac.as_str(),
            row.client_ip_hmac.as_deref().unwrap(),
            row.approver_ip_hmac.as_deref().unwrap(),
            "abcdef",
            "123456",
        ] {
            assert!(!debug.contains(secret), "{secret} leaked in {debug}");
        }

        assert!(debug.contains("Pending"));
        assert!(debug.contains("created_at"));
        assert!(debug.contains("expires_at"));
        assert!(debug.contains("wsl-calvin"));
        assert!(debug.contains("nyxid-cli/0.8.0"));
    }
}
