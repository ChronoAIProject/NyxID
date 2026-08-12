use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "triggers";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerStatus {
    Active,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerTokenLocation {
    Bearer,
    Query,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TriggerVerification {
    Token { location: TriggerTokenLocation },
    HmacSha256 { header_name: String },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerDelivery {
    Webhook { url: String },
    Agent { conversation_id: String },
    Notification,
}

impl fmt::Debug for TriggerDelivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Webhook { url } => f
                .debug_struct("Webhook")
                .field("url", &RedactedLen(url.len()))
                .finish(),
            Self::Agent { conversation_id } => f
                .debug_struct("Agent")
                .field("conversation_id", conversation_id)
                .finish(),
            Self::Notification => f.write_str("Notification"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Trigger {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub label: String,
    #[serde(default)]
    pub user_service_id: Option<String>,
    pub status: TriggerStatus,
    pub secret_hash: String,
    pub verification: TriggerVerification,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub verification_secret_encrypted: Option<Vec<u8>>,
    pub delivery: TriggerDelivery,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub delivery_secret_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub delivery_key_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Trigger")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("label", &self.label)
            .field("user_service_id", &self.user_service_id)
            .field("status", &self.status)
            .field("secret_hash", &RedactedLen(self.secret_hash.len()))
            .field("verification", &self.verification)
            .field(
                "verification_secret_encrypted",
                &self
                    .verification_secret_encrypted
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field("delivery", &self.delivery)
            .field(
                "delivery_secret_encrypted",
                &self
                    .delivery_secret_encrypted
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field("delivery_key_id", &self.delivery_key_id)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_roundtrip_and_debug_redaction() {
        let now = Utc::now();
        let trigger = Trigger {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            label: "Build completed".to_string(),
            user_service_id: None,
            status: TriggerStatus::Active,
            secret_hash: "ab".repeat(32),
            verification: TriggerVerification::HmacSha256 {
                header_name: "X-Hub-Signature-256".to_string(),
            },
            verification_secret_encrypted: Some(vec![1, 2, 3]),
            delivery: TriggerDelivery::Notification,
            delivery_secret_encrypted: None,
            delivery_key_id: None,
            created_at: now,
            updated_at: now,
        };
        let restored: Trigger =
            bson::from_document(bson::to_document(&trigger).expect("serialize trigger"))
                .expect("deserialize trigger");
        assert_eq!(restored.verification, trigger.verification);
        let debug = format!("{trigger:?}");
        assert!(!debug.contains(&trigger.secret_hash));
        assert!(!debug.contains("[1, 2, 3]"));
    }

    #[test]
    fn webhook_delivery_debug_redacts_target_url() {
        let delivery = TriggerDelivery::Webhook {
            url: "https://receiver.example.test/hooks/private-path".to_string(),
        };
        let debug = format!("{delivery:?}");
        assert!(!debug.contains("receiver.example.test"));
        assert!(debug.contains("redacted"));
    }
}
