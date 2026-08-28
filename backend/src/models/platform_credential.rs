use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_credentials";

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformCredential {
    #[serde(rename = "_id")]
    pub id: String,
    pub catalog_service_id: String,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub credential_encrypted: Vec<u8>,
    pub auth_method: String,
    pub auth_key_name: String,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for PlatformCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlatformCredential")
            .field("id", &self.id)
            .field("catalog_service_id", &self.catalog_service_id)
            .field("credential_encrypted", &"[REDACTED]")
            .field("auth_method", &self.auth_method)
            .field("auth_key_name", &self.auth_key_name)
            .field("created_by", &self.created_by)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every field is deterministic, including the timestamps.
    //
    // `debug_redacts_ciphertext` proves the ciphertext is absent by searching
    // the Debug output for the decimal bytes it would render as. With random
    // UUIDs and `Utc::now()` those digits could appear by coincidence -- a
    // fractional second of `.153` was enough -- so the test failed at random
    // and, worse, could have passed while redaction was broken.
    fn fixture() -> PlatformCredential {
        // 2023-11-14T22:13:20Z: contains neither "153" nor "66".
        let fixed = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp");
        PlatformCredential {
            id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            catalog_service_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
            credential_encrypted: vec![0x02, 0x99, 0x42],
            auth_method: "bearer".to_string(),
            auth_key_name: "Authorization".to_string(),
            created_by: "cccccccc-cccc-4ccc-8ccc-cccccccccccc".to_string(),
            created_at: fixed,
            updated_at: fixed,
        }
    }

    #[test]
    fn bson_roundtrip_uses_binary_ciphertext_and_preserves_dates() {
        let credential = fixture();
        let document = bson::to_document(&credential).expect("serialize platform credential");
        assert!(matches!(
            document.get("credential_encrypted"),
            Some(bson::Bson::Binary(_))
        ));

        let restored: PlatformCredential =
            bson::from_document(document).expect("deserialize platform credential");
        assert_eq!(restored.id, credential.id);
        assert_eq!(
            restored.credential_encrypted,
            credential.credential_encrypted
        );
        assert_eq!(
            restored.updated_at.timestamp_millis(),
            credential.updated_at.timestamp_millis()
        );
    }

    #[test]
    fn debug_redacts_ciphertext() {
        let credential = fixture();
        let debug = format!("{credential:?}");

        assert!(debug.contains("[REDACTED]"));
        // 0x99 and 0x42 render as 153 and 66 in a byte-slice Debug.
        assert!(!debug.contains("153"), "ciphertext byte leaked: {debug}");
        assert!(!debug.contains("66"), "ciphertext byte leaked: {debug}");
    }
}
