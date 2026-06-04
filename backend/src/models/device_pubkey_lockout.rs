use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::bson_datetime;
use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "device_pubkey_lockouts";

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct DevicePubkeyLockout {
    /// Natural `_id`: SHA-256 hex of the device public key rather than a UUID.
    /// Lockouts are one row per public key, and MongoDB's unique primary key
    /// makes the upsert-once transition atomic, matching the
    /// `oauth_broker_bindings` binding-hash `_id` pattern.
    #[serde(rename = "_id")]
    pub id: String,
    pub failed_poll_count: u32,
    #[serde(default, with = "bson_datetime::optional")]
    pub locked_until: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_failure_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_lockout_audited_at: Option<DateTime<Utc>>,
}

impl fmt::Debug for DevicePubkeyLockout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevicePubkeyLockout")
            .field("id", &RedactedLen(self.id.len()))
            .field("failed_poll_count", &self.failed_poll_count)
            .field("locked_until", &self.locked_until)
            .field("last_failure_at", &self.last_failure_at)
            .field("last_lockout_audited_at", &self.last_lockout_audited_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "device_pubkey_lockouts");
    }

    #[test]
    fn bson_roundtrip_preserves_optional_dates() {
        let now = Utc::now();
        let row = DevicePubkeyLockout {
            id: "ab".repeat(32),
            failed_poll_count: 3,
            locked_until: Some(now + chrono::Duration::hours(1)),
            last_failure_at: now,
            last_lockout_audited_at: Some(now),
        };

        let doc = bson::to_document(&row).expect("serialize");
        let restored: DevicePubkeyLockout = bson::from_document(doc).expect("deserialize");

        assert_eq!(row.id, restored.id);
        assert_eq!(row.failed_poll_count, restored.failed_poll_count);
        assert_eq!(
            row.locked_until.expect("locked").timestamp_millis(),
            restored.locked_until.expect("locked").timestamp_millis()
        );
        assert_eq!(
            row.last_failure_at.timestamp_millis(),
            restored.last_failure_at.timestamp_millis()
        );
        assert_eq!(
            row.last_lockout_audited_at
                .expect("audited")
                .timestamp_millis(),
            restored
                .last_lockout_audited_at
                .expect("audited")
                .timestamp_millis()
        );
    }
}
