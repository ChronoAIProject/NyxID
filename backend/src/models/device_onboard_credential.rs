use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "device_onboard_credentials";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceOnboardCredential {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_user_id: String,
    pub bootstrap_token_hash: String,
    pub label: String,
    #[serde(default)]
    pub default_service_ids: Vec<String>,
    #[serde(default)]
    pub allow_auto_connected_services: bool,
    #[serde(default)]
    pub used: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redeemed_api_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redeemed_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redeemed_refresh_token_hash: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for DeviceOnboardCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceOnboardCredential")
            .field("id", &RedactedLen(self.id.len()))
            .field("owner_user_id", &RedactedLen(self.owner_user_id.len()))
            .field(
                "bootstrap_token_hash",
                &RedactedLen(self.bootstrap_token_hash.len()),
            )
            .field("label", &self.label)
            .field("default_service_ids", &self.default_service_ids)
            .field("used", &self.used)
            .field(
                "redeemed_api_key_id",
                &self
                    .redeemed_api_key_id
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field("redeemed_node_id", &self.redeemed_node_id)
            .field(
                "redeemed_refresh_token_hash",
                &self
                    .redeemed_refresh_token_hash
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "device_onboard_credentials");
    }

    #[test]
    fn bson_roundtrip() {
        let credential = DeviceOnboardCredential {
            id: uuid::Uuid::new_v4().to_string(),
            owner_user_id: uuid::Uuid::new_v4().to_string(),
            bootstrap_token_hash: "deadbeef".repeat(8),
            label: "Kitchen Camera".to_string(),
            default_service_ids: vec![uuid::Uuid::new_v4().to_string()],
            allow_auto_connected_services: false,
            used: false,
            redeemed_api_key_id: None,
            redeemed_node_id: None,
            redeemed_refresh_token_hash: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(15),
        };

        let doc = bson::to_document(&credential).expect("serialize");
        let restored: DeviceOnboardCredential = bson::from_document(doc).expect("deserialize");

        assert_eq!(restored.id, credential.id);
        assert_eq!(restored.owner_user_id, credential.owner_user_id);
        assert_eq!(
            restored.bootstrap_token_hash,
            credential.bootstrap_token_hash
        );
        assert_eq!(restored.label, credential.label);
        assert_eq!(restored.default_service_ids, credential.default_service_ids);
        assert_eq!(restored.used, credential.used);
        assert_eq!(restored.redeemed_api_key_id, credential.redeemed_api_key_id);
        assert_eq!(restored.redeemed_node_id, credential.redeemed_node_id);
        assert_eq!(
            restored.redeemed_refresh_token_hash,
            credential.redeemed_refresh_token_hash
        );
        assert_eq!(
            restored.created_at.timestamp_millis(),
            credential.created_at.timestamp_millis()
        );
        assert_eq!(
            restored.expires_at.timestamp_millis(),
            credential.expires_at.timestamp_millis()
        );
    }
}
