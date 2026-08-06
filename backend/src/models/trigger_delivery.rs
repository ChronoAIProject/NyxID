use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;
use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "trigger_deliveries";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerDeliveryRecordStatus {
    Pending,
    Delivered,
    Failed,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TriggerDeliveryRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub trigger_id: String,
    pub user_id: String,
    pub event_id: String,
    pub status: TriggerDeliveryRecordStatus,
    pub attempts: u32,
    #[serde(default)]
    pub last_status_code: Option<u16>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub envelope_encrypted: Option<Vec<u8>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub delivered_at: Option<DateTime<Utc>>,
}

impl fmt::Debug for TriggerDeliveryRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TriggerDeliveryRecord")
            .field("id", &self.id)
            .field("trigger_id", &self.trigger_id)
            .field("user_id", &self.user_id)
            .field("event_id", &self.event_id)
            .field("status", &self.status)
            .field("attempts", &self.attempts)
            .field("last_status_code", &self.last_status_code)
            .field(
                "envelope_encrypted",
                &self
                    .envelope_encrypted
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("expires_at", &self.expires_at)
            .field("delivered_at", &self.delivered_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_roundtrip_redacts_encrypted_envelope() {
        let now = Utc::now();
        let record = TriggerDeliveryRecord {
            id: "delivery-id".to_string(),
            trigger_id: "trigger-id".to_string(),
            user_id: "user-id".to_string(),
            event_id: "event-id".to_string(),
            status: TriggerDeliveryRecordStatus::Pending,
            attempts: 0,
            last_status_code: None,
            envelope_encrypted: Some(vec![1, 2, 3]),
            created_at: now,
            updated_at: now,
            expires_at: now + chrono::Duration::hours(72),
            delivered_at: None,
        };
        let restored: TriggerDeliveryRecord =
            bson::from_document(bson::to_document(&record).expect("serialize trigger delivery"))
                .expect("deserialize trigger delivery");
        assert_eq!(restored.event_id, record.event_id);
        assert!(!format!("{record:?}").contains("[1, 2, 3]"));
    }
}
