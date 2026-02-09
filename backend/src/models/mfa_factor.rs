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
    #[serde(skip_serializing)]
    pub secret_encrypted: Option<Vec<u8>>,
    /// For recovery codes: JSON array of hashed codes (read via MongoDB queries, not Rust field access)
    #[allow(dead_code)]
    #[serde(skip_serializing)]
    pub recovery_codes: Option<serde_json::Value>,
    pub is_verified: bool,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
