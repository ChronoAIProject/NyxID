use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "mfa_factors";

#[derive(Clone, Serialize, Deserialize)]
pub struct MfaFactor {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// Factor type: "totp", "webauthn", "recovery_codes"
    pub factor_type: String,
    /// Encrypted TOTP secret or WebAuthn credential
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub secret_encrypted: Option<Vec<u8>>,
    /// Trusted reader policy chooses whether an absent legacy marker is allowed.
    #[serde(default)]
    pub secret_context_version: Option<u32>,
    /// For recovery codes: JSON array of hashed codes
    pub recovery_codes: Option<serde_json::Value>,
    /// Highest accepted TOTP step, consumed by enrollment and every login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_totp_counter: Option<i64>,
    pub is_verified: bool,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for MfaFactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MfaFactor").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "mfa_factors");
    }

    #[test]
    fn bson_roundtrip() {
        let factor = MfaFactor {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            factor_type: "totp".to_string(),
            secret_encrypted: Some(vec![1, 2, 3, 4]),
            secret_context_version: None,
            recovery_codes: None,
            last_totp_counter: None,
            is_verified: true,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&factor).expect("serialize");
        let restored: MfaFactor = bson::from_document(doc).expect("deserialize");
        assert_eq!(factor.id, restored.id);
        assert_eq!(factor.factor_type, restored.factor_type);
        assert_eq!(factor.secret_encrypted, restored.secret_encrypted);
    }

    #[test]
    fn bson_roundtrip_with_recovery_codes() {
        let factor = MfaFactor {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            factor_type: "recovery_codes".to_string(),
            secret_encrypted: None,
            secret_context_version: None,
            recovery_codes: Some(serde_json::json!(["code1hash", "code2hash"])),
            last_totp_counter: None,
            is_verified: true,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&factor).expect("serialize");
        let restored: MfaFactor = bson::from_document(doc).expect("deserialize");
        assert!(restored.recovery_codes.is_some());
    }
}
