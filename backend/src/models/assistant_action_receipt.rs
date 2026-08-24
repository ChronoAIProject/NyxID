use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "assistant_action_receipts";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantActionReceiptStatus {
    Pending,
    Completed,
}

/// Durable, secret-free evidence for a NyxID-owned browser action.
///
/// `resource_id` is reserved before the effect is attempted. A retry can
/// therefore distinguish "not applied yet" from "applied but the response was
/// lost" without persisting or replaying one-time credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantActionReceipt {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub action: String,
    pub action_request_id: String,
    pub request_fingerprint: String,
    pub resource_id: String,
    /// Optional monotonic marker captured immediately before a mutation. It
    /// is secret-free evidence used to recover an interrupted one-time effect.
    #[serde(default)]
    pub resource_state_version: Option<i64>,
    /// Optional access-material revision used by node token rotation recovery.
    #[serde(default)]
    pub resource_access_revision: Option<i64>,
    /// Optional keyed fingerprint of secret material captured immediately
    /// before a secret rotation. It proves the secret itself changed without
    /// exposing the stored hash or raw credential.
    #[serde(default)]
    pub resource_secret_fingerprint: Option<String>,
    pub status: AssistantActionReceiptStatus,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_roundtrip_never_contains_secret_material() {
        let receipt = AssistantActionReceipt {
            id: "receipt-alpha".to_string(),
            user_id: "user-alpha".to_string(),
            action: "key.create".to_string(),
            action_request_id: "action-alpha".to_string(),
            request_fingerprint: "abc123".to_string(),
            resource_id: "key-alpha".to_string(),
            resource_state_version: None,
            resource_access_revision: None,
            resource_secret_fingerprint: None,
            status: AssistantActionReceiptStatus::Completed,
            created_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };

        let document = bson::to_document(&receipt).expect("serialize receipt");
        let serialized = document.to_string().to_ascii_lowercase();
        for forbidden in ["full_key", "key_hash", "secret", "credential"] {
            assert!(!serialized.contains(forbidden));
        }
        let restored: AssistantActionReceipt =
            bson::from_document(document).expect("deserialize receipt");
        assert_eq!(restored.resource_id, "key-alpha");
        assert_eq!(restored.status, AssistantActionReceiptStatus::Completed);
    }
}
