use std::fmt;

use serde::{Deserialize, Serialize};

pub const REMOTE_CREDENTIAL_CRYPTO_CAPABILITY: &str = "remote_credential_crypto_v1";
pub const PENDING_CREDENTIAL_DECRYPT_FAILED_CODE: u16 = 8006;
pub const PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE: u16 = 8007;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCredentialCryptoMetadata {
    pub pending_id: String,
    pub node_id: String,
    pub service_slug: String,
    pub injection_method: String,
    pub field_name: String,
    pub target_url: Option<String>,
    pub expires_at: String,
    pub version: String,
}

impl PendingCredentialCryptoMetadata {
    pub fn context(&self) -> nyxid_crypto::RciContext {
        nyxid_crypto::RciContext::new(
            self.node_id.clone(),
            self.pending_id.clone(),
            self.service_slug.clone(),
            self.injection_method.clone(),
            self.field_name.clone(),
            self.target_url.clone(),
            self.version.clone(),
        )
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingCredentialCiphertext {
    pub pending_id: String,
    pub version: String,
    pub admin_pubkey: String,
    pub nonce: String,
    pub ciphertext: String,
}

impl fmt::Debug for PendingCredentialCiphertext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingCredentialCiphertext")
            .field("pending_id", &self.pending_id)
            .field("version", &self.version)
            .field("admin_pubkey", &"[REDACTED]")
            .field("nonce", &"[REDACTED]")
            .field("ciphertext", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum RemoteCredentialCryptoOutbound {
    Pubkey {
        pending_id: String,
        version: String,
        node_pubkey: String,
    },
    DecryptResult {
        pending_id: String,
        status: String,
        error_code: Option<u16>,
    },
}

impl RemoteCredentialCryptoOutbound {
    pub fn pending_id(&self) -> &str {
        match self {
            Self::Pubkey { pending_id, .. } | Self::DecryptResult { pending_id, .. } => pending_id,
        }
    }

    pub fn to_ws_json(&self) -> serde_json::Value {
        match self {
            Self::Pubkey {
                pending_id,
                version,
                node_pubkey,
            } => serde_json::json!({
                "type": "pending_credential_pubkey",
                "pending_id": pending_id,
                "version": version,
                "node_pubkey": node_pubkey,
            }),
            Self::DecryptResult {
                pending_id,
                status,
                error_code,
            } => {
                let mut value = serde_json::json!({
                    "type": "pending_credential_decrypt_result",
                    "pending_id": pending_id,
                    "status": status,
                });
                if let Some(code) = error_code {
                    value["error_code"] = serde_json::Value::Number((*code).into());
                }
                value
            }
        }
    }
}

impl fmt::Debug for RemoteCredentialCryptoOutbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pubkey {
                pending_id,
                version,
                node_pubkey: _,
            } => f
                .debug_struct("Pubkey")
                .field("pending_id", pending_id)
                .field("version", version)
                .field("node_pubkey", &"[REDACTED]")
                .finish(),
            Self::DecryptResult {
                pending_id,
                status,
                error_code,
            } => f
                .debug_struct("DecryptResult")
                .field("pending_id", pending_id)
                .field("status", status)
                .field("error_code", error_code)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_pubkey_ws_json_matches_protocol_contract() {
        let outbound = RemoteCredentialCryptoOutbound::Pubkey {
            pending_id: "pending-1".to_string(),
            version: nyxid_crypto::VERSION_V1.to_string(),
            node_pubkey: "node-pubkey".to_string(),
        };

        assert_eq!(
            outbound.to_ws_json(),
            serde_json::json!({
                "type": "pending_credential_pubkey",
                "pending_id": "pending-1",
                "version": "v1",
                "node_pubkey": "node-pubkey",
            })
        );
    }

    #[test]
    fn outbound_success_ws_json_omits_error_code() {
        let outbound = RemoteCredentialCryptoOutbound::DecryptResult {
            pending_id: "pending-1".to_string(),
            status: "ok".to_string(),
            error_code: None,
        };
        let json = outbound.to_ws_json();

        assert_eq!(
            json,
            serde_json::json!({
                "type": "pending_credential_decrypt_result",
                "pending_id": "pending-1",
                "status": "ok",
            })
        );
        assert!(json.get("error_code").is_none());
    }

    #[test]
    fn outbound_error_ws_json_includes_error_code() {
        let outbound = RemoteCredentialCryptoOutbound::DecryptResult {
            pending_id: "pending-1".to_string(),
            status: "error".to_string(),
            error_code: Some(PENDING_CREDENTIAL_DECRYPT_FAILED_CODE),
        };

        assert_eq!(
            outbound.to_ws_json(),
            serde_json::json!({
                "type": "pending_credential_decrypt_result",
                "pending_id": "pending-1",
                "status": "error",
                "error_code": 8006,
            })
        );
    }
}
