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
    pub api_key_id: String,
    pub node_id: String,
    pub refresh_token_hash: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl fmt::Debug for DeviceOnboardCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceOnboardCredential")
            .field("id", &RedactedLen(self.id.len()))
            .field("owner_user_id", &RedactedLen(self.owner_user_id.len()))
            .field("api_key_id", &RedactedLen(self.api_key_id.len()))
            .field("node_id", &RedactedLen(self.node_id.len()))
            .field(
                "refresh_token_hash",
                &RedactedLen(self.refresh_token_hash.len()),
            )
            .field("created_at", &self.created_at)
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
            api_key_id: uuid::Uuid::new_v4().to_string(),
            node_id: uuid::Uuid::new_v4().to_string(),
            refresh_token_hash: "deadbeef".repeat(8),
            created_at: Utc::now(),
        };

        let doc = bson::to_document(&credential).expect("serialize");
        let restored: DeviceOnboardCredential = bson::from_document(doc).expect("deserialize");

        assert_eq!(restored.id, credential.id);
        assert_eq!(restored.owner_user_id, credential.owner_user_id);
        assert_eq!(restored.api_key_id, credential.api_key_id);
        assert_eq!(restored.node_id, credential.node_id);
        assert_eq!(restored.refresh_token_hash, credential.refresh_token_hash);
        assert_eq!(
            restored.created_at.timestamp_millis(),
            credential.created_at.timestamp_millis()
        );
    }
}
