use chrono::{DateTime, Utc};
use serde::Serialize;
use std::fmt;

use crate::errors::{AppError, AppResult};
use crate::models::device_code::DeviceCode;
use crate::redaction::RedactedLen;

mod approve;
mod initiate;
mod lockout;
mod onboard;
mod poll;
mod rotation;

pub use approve::approve;
pub use initiate::initiate;
pub use lockout::{claim_lockout_notification, is_locked};
use lockout::{is_pubkey_locked, record_pubkey_signature_failure, reset_pubkey_lockout};
pub use onboard::onboard;
pub use poll::poll;

pub const DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD: u32 = 3;
pub const DEVICE_CODE_LOCKOUT_SECS: i64 = 60 * 60;
pub const DEVICE_CODE_EXPIRES_IN_SECS: i64 = 15 * 60;
pub const DEVICE_CODE_POLL_INTERVAL_SECS: u32 = 5;
pub const DEVICE_CODE_ROTATE_SECS: i64 = 30;
pub const DEVICE_CODE_TIMESTAMP_SKEW_SECS: i64 = 10;
pub const DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS: i64 = 24 * 60 * 60;
pub(super) const DEVICE_CODE_API_KEY_SCOPES: &str = "proxy";
pub(super) const DEVICE_CODE_USER_CODE_WRITE_RETRIES: usize = 5;

pub(super) fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

#[derive(Clone)]
pub struct DeviceCodeInitiateInput {
    pub device_pubkey: [u8; 32],
    pub hw_id: String,
    pub suggested_label: Option<String>,
    pub frontend_url: String,
}

#[derive(Clone, Serialize, PartialEq)]
pub struct DeviceCodeInitiate {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub poll_interval: u32,
}

#[derive(Clone)]
pub struct DeviceCodePollInput {
    pub device_code: String,
    pub timestamp: i64,
    pub signature: [u8; 64],
}

#[derive(Clone, Serialize, PartialEq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum DeviceCodePoll {
    Pending {
        current_user_code: String,
        interval: u32,
    },
    Approved {
        api_key: String,
        node_id: String,
        refresh_token: String,
        expires_in: i64,
    },
}

#[derive(Clone, PartialEq)]
pub struct DeviceCodeApproveInput {
    pub user_code: String,
    pub org_id: Option<String>,
    pub label: Option<String>,
    pub default_services: Option<Vec<String>>,
}

#[derive(Clone, Serialize, PartialEq)]
pub struct DeviceCodeApprove {
    pub device_label: String,
    pub hw_id: String,
    pub api_key_id: String,
    pub node_id: String,
    pub owner_user_id: String,
    pub org_id: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct DeviceOnboardInput {
    pub org_id: Option<String>,
    pub label: String,
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub default_services: Option<Vec<String>>,
    pub base_url: String,
}

#[derive(Clone, Serialize, PartialEq)]
pub struct DeviceOnboard {
    pub qr_payload: String,
    pub node_id: String,
    pub api_key_id: String,
    pub label: String,
}

#[derive(Clone, PartialEq)]
pub struct DeviceCodeLockoutNotification {
    pub recipients: Vec<String>,
    pub device_label: String,
    pub hw_id: String,
    pub node_id: Option<String>,
    pub device_pubkey_fingerprint: String,
    pub failed_poll_count: u32,
    pub locked_until: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SignatureFailureLockout {
    pub failed_poll_count: u32,
    pub locked_until: Option<DateTime<Utc>>,
}

impl fmt::Debug for DeviceCodeInitiateInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCodeInitiateInput")
            .field("device_pubkey", &RedactedLen(self.device_pubkey.len()))
            .field("hw_id", &self.hw_id)
            .field("suggested_label", &self.suggested_label)
            .field("frontend_url", &self.frontend_url)
            .finish()
    }
}

impl fmt::Debug for DeviceCodeInitiate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCodeInitiate")
            .field("device_code", &RedactedLen(self.device_code.len()))
            .field("user_code", &RedactedLen(self.user_code.len()))
            .field("verification_uri", &self.verification_uri)
            .field(
                "verification_uri_complete",
                &RedactedLen(self.verification_uri_complete.len()),
            )
            .field("expires_in", &self.expires_in)
            .field("poll_interval", &self.poll_interval)
            .finish()
    }
}

impl fmt::Debug for DeviceCodePollInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCodePollInput")
            .field("device_code", &RedactedLen(self.device_code.len()))
            .field("timestamp", &self.timestamp)
            .field("signature", &RedactedLen(self.signature.len()))
            .finish()
    }
}

impl fmt::Debug for DeviceCodePoll {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending {
                current_user_code,
                interval,
            } => f
                .debug_struct("DeviceCodePoll::Pending")
                .field("current_user_code", &RedactedLen(current_user_code.len()))
                .field("interval", interval)
                .finish(),
            Self::Approved {
                api_key,
                node_id,
                refresh_token,
                expires_in,
            } => f
                .debug_struct("DeviceCodePoll::Approved")
                .field("api_key", &RedactedLen(api_key.len()))
                .field("node_id", node_id)
                .field("refresh_token", &RedactedLen(refresh_token.len()))
                .field("expires_in", expires_in)
                .finish(),
        }
    }
}

impl fmt::Debug for DeviceCodeApproveInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCodeApproveInput")
            .field("user_code", &RedactedLen(self.user_code.len()))
            .field("org_id", &self.org_id)
            .field("label", &self.label)
            .field("default_services", &self.default_services)
            .finish()
    }
}

impl fmt::Debug for DeviceCodeApprove {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCodeApprove")
            .field("device_label", &self.device_label)
            .field("hw_id", &self.hw_id)
            .field("api_key_id", &RedactedLen(self.api_key_id.len()))
            .field("node_id", &self.node_id)
            .field("owner_user_id", &self.owner_user_id)
            .field("org_id", &self.org_id)
            .finish()
    }
}

impl fmt::Debug for DeviceOnboardInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceOnboardInput")
            .field("org_id", &self.org_id)
            .field("label", &self.label)
            .field("wifi_ssid", &self.wifi_ssid)
            .field("wifi_password", &RedactedLen(self.wifi_password.len()))
            .field("default_services", &self.default_services)
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl fmt::Debug for DeviceOnboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceOnboard")
            .field("qr_payload", &RedactedLen(self.qr_payload.len()))
            .field("node_id", &self.node_id)
            .field("api_key_id", &RedactedLen(self.api_key_id.len()))
            .field("label", &self.label)
            .finish()
    }
}

impl fmt::Debug for DeviceCodeLockoutNotification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCodeLockoutNotification")
            .field("recipients", &self.recipients)
            .field("device_label", &self.device_label)
            .field("hw_id", &self.hw_id)
            .field("node_id", &self.node_id)
            .field(
                "device_pubkey_fingerprint",
                &RedactedLen(self.device_pubkey_fingerprint.len()),
            )
            .field("failed_poll_count", &self.failed_poll_count)
            .field("locked_until", &self.locked_until)
            .finish()
    }
}

pub(super) fn choose_device_label(
    row: &DeviceCode,
    requested_label: Option<&str>,
) -> AppResult<String> {
    let candidate = requested_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .or_else(|| {
            row.suggested_label
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty())
        })
        .map(str::to_string)
        .unwrap_or_else(|| format!("device-{}", row.hw_id));

    let label = truncate_to_max_bytes(candidate.trim(), 200);
    if label.is_empty() {
        return Err(AppError::ValidationError(
            "Device label must be between 1 and 200 characters".to_string(),
        ));
    }

    Ok(label)
}

fn truncate_to_max_bytes(value: &str, max_bytes: usize) -> String {
    let mut output = String::new();
    for c in value.chars() {
        if output.len() + c.len_utf8() > max_bytes {
            break;
        }
        output.push(c);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::device_code::DeviceCodeStatus;
    use chrono::Duration;
    use uuid::Uuid;

    #[test]
    fn choose_device_label_prefers_request_then_suggested_then_hw_id() {
        let row = DeviceCode {
            id: Uuid::new_v4().to_string(),
            device_code_hash: "deadbeef".repeat(8),
            device_pubkey: vec![1u8; 32],
            hw_id: "esp32-cam".to_string(),
            suggested_label: Some("Kitchen".to_string()),
            user_code_history: vec![],
            status: DeviceCodeStatus::Pending,
            approved_by_user_id: None,
            approved_org_id: None,
            issued_api_key_id: None,
            issued_node_id: None,
            delivery_api_key_encrypted: None,
            delivery_refresh_token_encrypted: None,
            refresh_token_hash: None,
            failed_poll_count: 0,
            locked_until: None,
            lock_alert_sent_at: None,
            expires_at: Utc::now() + Duration::minutes(15),
            created_at: Utc::now(),
            last_polled_at: None,
            last_poll_timestamp: None,
            last_rotated_at: Utc::now(),
        };

        assert_eq!(
            choose_device_label(&row, Some(" Hallway ")).unwrap(),
            "Hallway"
        );
        assert_eq!(choose_device_label(&row, None).unwrap(), "Kitchen");

        let mut row_without_suggestion = row;
        row_without_suggestion.suggested_label = None;
        assert_eq!(
            choose_device_label(&row_without_suggestion, None).unwrap(),
            "device-esp32-cam"
        );
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::crypto::device_code::poll_signature_message;
    use crate::test_utils::connect_test_database;
    use ed25519_dalek::{Signer, SigningKey};
    use mongodb::Database;

    pub(crate) fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[77u8; 32])
    }

    pub(crate) fn sign_poll(device_code: &str, timestamp: i64, key: &SigningKey) -> [u8; 64] {
        key.sign(&poll_signature_message(device_code, timestamp).expect("valid device code"))
            .to_bytes()
    }

    pub(crate) async fn setup_pending_row(
        prefix: &str,
    ) -> Option<(Database, DeviceCodeInitiate, SigningKey)> {
        let db = connect_test_database(prefix).await?;
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let key = signing_key();
        let response = initiate(
            &db,
            DeviceCodeInitiateInput {
                device_pubkey: key.verifying_key().to_bytes(),
                hw_id: "esp32-p4-cam-1".to_string(),
                suggested_label: Some("Kitchen cam".to_string()),
                frontend_url: "https://app.example.com".to_string(),
            },
        )
        .await
        .expect("initiate");
        Some((db, response, key))
    }
}
