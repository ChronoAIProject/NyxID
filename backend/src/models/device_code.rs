use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::bson_datetime;
use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "device_codes";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceCodeStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    Delivered,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct UserCodeGen {
    pub code: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub device_code_hash: String,
    pub device_pubkey: Vec<u8>,
    pub hw_id: String,
    pub suggested_label: Option<String>,
    pub user_code_history: Vec<UserCodeGen>,
    pub status: DeviceCodeStatus,
    pub approved_by_user_id: Option<String>,
    pub approved_org_id: Option<String>,
    pub issued_api_key_id: Option<String>,
    pub issued_node_id: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub delivery_api_key_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub delivery_refresh_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_hash: Option<String>,
    pub failed_poll_count: u32,
    #[serde(default, with = "bson_datetime::optional")]
    pub locked_until: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub lock_alert_sent_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_polled_at: Option<DateTime<Utc>>,
    pub last_poll_timestamp: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_rotated_at: DateTime<Utc>,
}

impl fmt::Debug for UserCodeGen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserCodeGen")
            .field("code", &RedactedLen(self.code.len()))
            .field("generated_at", &self.generated_at)
            .finish()
    }
}

impl fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCode")
            .field("id", &self.id)
            .field(
                "device_code_hash",
                &RedactedLen(self.device_code_hash.len()),
            )
            .field("device_pubkey", &RedactedLen(self.device_pubkey.len()))
            .field("hw_id", &self.hw_id)
            .field("suggested_label", &self.suggested_label)
            .field("user_code_history", &self.user_code_history)
            .field("status", &self.status)
            .field("approved_by_user_id", &self.approved_by_user_id)
            .field("approved_org_id", &self.approved_org_id)
            .field(
                "issued_api_key_id",
                &self
                    .issued_api_key_id
                    .as_ref()
                    .map(|id| RedactedLen(id.len())),
            )
            .field("issued_node_id", &self.issued_node_id)
            .field(
                "delivery_api_key_encrypted",
                &self
                    .delivery_api_key_encrypted
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
                "refresh_token_hash",
                &self
                    .refresh_token_hash
                    .as_ref()
                    .map(|hash| RedactedLen(hash.len())),
            )
            .field("failed_poll_count", &self.failed_poll_count)
            .field("locked_until", &self.locked_until)
            .field("lock_alert_sent_at", &self.lock_alert_sent_at)
            .field("expires_at", &self.expires_at)
            .field("created_at", &self.created_at)
            .field("last_polled_at", &self.last_polled_at)
            .field("last_poll_timestamp", &self.last_poll_timestamp)
            .field("last_rotated_at", &self.last_rotated_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_device_code() -> DeviceCode {
        let now = Utc::now();
        DeviceCode {
            id: uuid::Uuid::new_v4().to_string(),
            device_code_hash: "deadbeef".repeat(8),
            device_pubkey: vec![7; 32],
            hw_id: "esp32-p4-lab".to_string(),
            suggested_label: Some("Lab Camera".to_string()),
            user_code_history: vec![UserCodeGen {
                code: "ABCD-EFGH-JKLM".to_string(),
                generated_at: now,
            }],
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
            expires_at: now + chrono::Duration::minutes(15),
            created_at: now,
            last_polled_at: None,
            last_poll_timestamp: None,
            last_rotated_at: now,
        }
    }

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "device_codes");
    }

    #[test]
    fn status_serializes_as_lowercase() {
        let value = serde_json::to_value(DeviceCodeStatus::Delivered).expect("serialize status");
        assert_eq!(value, serde_json::json!("delivered"));
    }

    #[test]
    fn bson_roundtrip_preserves_required_fields() {
        let row = make_device_code();
        let doc = bson::to_document(&row).expect("serialize");
        let restored: DeviceCode = bson::from_document(doc).expect("deserialize");

        assert_eq!(row.id, restored.id);
        assert_eq!(row.device_code_hash, restored.device_code_hash);
        assert_eq!(restored.device_pubkey.len(), 32);
        assert_eq!(row.hw_id, restored.hw_id);
        assert_eq!(row.status, restored.status);
        assert_eq!(
            row.user_code_history[0].code,
            restored.user_code_history[0].code
        );
        assert_eq!(
            row.user_code_history[0].generated_at.timestamp_millis(),
            restored.user_code_history[0]
                .generated_at
                .timestamp_millis()
        );
        assert_eq!(row.failed_poll_count, restored.failed_poll_count);
    }

    #[test]
    fn bson_serializes_optional_none_fields_as_null() {
        let mut row = make_device_code();
        row.suggested_label = None;
        let doc = bson::to_document(&row).expect("serialize");

        for field in [
            "suggested_label",
            "approved_by_user_id",
            "approved_org_id",
            "issued_api_key_id",
            "issued_node_id",
            "delivery_api_key_encrypted",
            "delivery_refresh_token_encrypted",
            "refresh_token_hash",
            "locked_until",
            "lock_alert_sent_at",
            "last_polled_at",
            "last_poll_timestamp",
        ] {
            assert_eq!(doc.get(field), Some(&bson::Bson::Null), "{field}");
        }
    }

    #[test]
    fn bson_roundtrip_preserves_optional_dates() {
        let mut row = make_device_code();
        let lock_until = Utc::now() + chrono::Duration::hours(1);
        let alert_sent_at = Utc::now();
        let polled_at = Utc::now();
        row.locked_until = Some(lock_until);
        row.lock_alert_sent_at = Some(alert_sent_at);
        row.last_polled_at = Some(polled_at);

        let doc = bson::to_document(&row).expect("serialize");
        let restored: DeviceCode = bson::from_document(doc).expect("deserialize");

        assert_eq!(
            restored
                .locked_until
                .expect("locked_until")
                .timestamp_millis(),
            lock_until.timestamp_millis()
        );
        assert_eq!(
            restored
                .lock_alert_sent_at
                .expect("lock_alert_sent_at")
                .timestamp_millis(),
            alert_sent_at.timestamp_millis()
        );
        assert_eq!(
            restored
                .last_polled_at
                .expect("last_polled_at")
                .timestamp_millis(),
            polled_at.timestamp_millis()
        );
    }
}
