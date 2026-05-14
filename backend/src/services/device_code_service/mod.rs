#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::errors::{AppError, AppResult};
use crate::models::device_code::DeviceCode;

mod approve;
mod initiate;
mod lockout;
mod poll;
mod rotation;

pub use approve::approve;
pub use initiate::initiate;
pub use lockout::{apply_signature_failure_lockout, claim_lockout_notification, is_locked};
pub use poll::poll;

pub const DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD: u32 = 3;
pub const DEVICE_CODE_LOCKOUT_SECS: i64 = 60 * 60;
pub const DEVICE_CODE_EXPIRES_IN_SECS: i64 = 15 * 60;
pub const DEVICE_CODE_POLL_INTERVAL_SECS: u32 = 5;
pub const DEVICE_CODE_ROTATE_SECS: i64 = 30;
pub const DEVICE_CODE_TIMESTAMP_SKEW_SECS: i64 = 60;
pub const DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS: i64 = 24 * 60 * 60;
pub(super) const DEVICE_CODE_API_KEY_SCOPES: &str = "read write proxy";

#[derive(Clone, Debug)]
pub struct DeviceCodeInitiateInput {
    pub device_pubkey: [u8; 32],
    pub hw_id: String,
    pub suggested_label: Option<String>,
    pub frontend_url: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DeviceCodeInitiate {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub poll_interval: u32,
}

#[derive(Clone, Debug)]
pub struct DeviceCodePollInput {
    pub device_code: String,
    pub timestamp: i64,
    pub signature: [u8; 64],
}

#[derive(Clone, Debug, Serialize, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCodeApproveInput {
    pub user_code: String,
    pub org_id: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DeviceCodeApprove {
    pub device_label: String,
    pub hw_id: String,
    pub api_key_id: String,
    pub node_id: String,
    pub owner_user_id: String,
    pub org_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCodeLockoutNotification {
    pub recipients: Vec<String>,
    pub device_label: String,
    pub hw_id: String,
    pub node_id: Option<String>,
    pub failed_poll_count: u32,
    pub locked_until: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SignatureFailureLockout {
    pub failed_poll_count: u32,
    pub locked_until: Option<DateTime<Utc>>,
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
            delivery_api_key: None,
            delivery_refresh_token: None,
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
    use crate::crypto::device_code::decode_device_code;
    use crate::test_utils::connect_test_database;
    use ed25519_dalek::{Signer, SigningKey};
    use mongodb::Database;

    pub(crate) fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[77u8; 32])
    }

    pub(crate) fn sign_poll(device_code: &str, timestamp: i64, key: &SigningKey) -> [u8; 64] {
        let decoded = decode_device_code(device_code).expect("valid device code");
        let mut message = Vec::with_capacity(decoded.len() + std::mem::size_of::<i64>());
        message.extend_from_slice(&decoded);
        message.extend_from_slice(&timestamp.to_be_bytes());
        key.sign(&message).to_bytes()
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
