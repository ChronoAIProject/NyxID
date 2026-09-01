use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_bytes;

pub const COLLECTION_NAME: &str = "oracle_login_snapshots";

#[derive(Clone, Serialize, Deserialize)]
pub struct OracleLoginSnapshot {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    pub format_version: u32,
    #[serde(with = "bson_bytes::required")]
    pub encrypted_envelope: Vec<u8>,
    pub envelope_size: u64,
    pub created_by_user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for OracleLoginSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OracleLoginSnapshot")
            .field("id", &self.id)
            .field("pool_id", &self.pool_id)
            .field("format_version", &self.format_version)
            .field("envelope_size", &self.envelope_size)
            .field("created_by_user_id", &self.created_by_user_id)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_encrypted_envelope() {
        let now = Utc::now();
        let snapshot = OracleLoginSnapshot {
            id: "snapshot-1".to_string(),
            pool_id: "pool-1".to_string(),
            format_version: 1,
            encrypted_envelope: vec![9, 8, 7, 6],
            envelope_size: 4,
            created_by_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now,
        };
        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("[9, 8, 7, 6]"));
        assert!(debug.contains("envelope_size: 4"));
    }
}
