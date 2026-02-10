use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "mfa_factors";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MfaFactor {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// Factor type: "totp", "webauthn", "recovery_codes"
    pub factor_type: String,
    /// Encrypted TOTP secret or WebAuthn credential
    pub secret_encrypted: Option<Vec<u8>>,
    /// For recovery codes: JSON array of hashed codes
    #[allow(dead_code)]
    pub recovery_codes: Option<serde_json::Value>,
    pub is_verified: bool,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
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
            recovery_codes: None,
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
            recovery_codes: Some(serde_json::json!(["code1hash", "code2hash"])),
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
