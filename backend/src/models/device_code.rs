#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserCodeGen {
    pub code: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub device_code_hash: String,
    pub device_pubkey: Vec<u8>,
    pub hw_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_label: Option<String>,
    pub user_code_history: Vec<UserCodeGen>,
    pub status: DeviceCodeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_api_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_poll_timestamp: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_rotated_at: DateTime<Utc>,
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
            delivery_api_key: None,
            delivery_refresh_token: None,
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
