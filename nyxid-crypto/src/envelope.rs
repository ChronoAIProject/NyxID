use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::MAX_CIPHERTEXT_SIZE;
use crate::codec::{decode_b64u_array, decode_b64u_capped, encode_b64u};
use crate::error::Result;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiphertextEnvelope {
    pub version: String,
    #[serde(with = "b64_32")]
    pub admin_pubkey: [u8; 32],
    #[serde(with = "b64_24")]
    pub nonce: [u8; 24],
    #[serde(with = "b64_ciphertext")]
    pub ciphertext: Vec<u8>,
}

impl CiphertextEnvelope {
    pub fn new(
        version: impl Into<String>,
        admin_pubkey: [u8; 32],
        nonce: [u8; 24],
        ciphertext: Vec<u8>,
    ) -> Self {
        Self {
            version: version.into(),
            admin_pubkey,
            nonce,
            ciphertext,
        }
    }
}

impl fmt::Debug for CiphertextEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CiphertextEnvelope")
            .field("version", &self.version)
            .field("admin_pubkey", &"[REDACTED]")
            .field("nonce", &"[REDACTED]")
            .field(
                "ciphertext",
                &format!("[REDACTED; {} bytes]", self.ciphertext.len()),
            )
            .finish()
    }
}

mod b64_32 {
    use super::*;

    pub fn serialize<S>(value: &[u8; 32], serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_b64u(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        decode_b64u_array("admin_pubkey", &encoded).map_err(serde::de::Error::custom)
    }
}

mod b64_24 {
    use super::*;

    pub fn serialize<S>(value: &[u8; 24], serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_b64u(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<[u8; 24], D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        decode_b64u_array("nonce", &encoded).map_err(serde::de::Error::custom)
    }
}

mod b64_ciphertext {
    use super::*;

    pub fn serialize<S>(value: &[u8], serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_b64u(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        decode_b64u_capped("ciphertext", &encoded, MAX_CIPHERTEXT_SIZE)
            .map_err(serde::de::Error::custom)
    }
}

pub fn envelope_from_encoded_parts(
    version: impl Into<String>,
    admin_pubkey: &str,
    nonce: &str,
    ciphertext: &str,
) -> Result<CiphertextEnvelope> {
    Ok(CiphertextEnvelope::new(
        version,
        decode_b64u_array("admin_pubkey", admin_pubkey)?,
        decode_b64u_array("nonce", nonce)?,
        decode_b64u_capped("ciphertext", ciphertext, MAX_CIPHERTEXT_SIZE)?,
    ))
}
