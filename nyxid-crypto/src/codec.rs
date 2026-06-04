use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::error::{RciCryptoError, Result};

pub fn encode_b64u(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode_b64u(field: &'static str, encoded: &str) -> Result<Vec<u8>> {
    if encoded.contains('=') {
        return Err(RciCryptoError::Base64Padding { field });
    }
    URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|source| RciCryptoError::Base64Decode { field, source })
}

pub fn decode_b64u_array<const N: usize>(field: &'static str, encoded: &str) -> Result<[u8; N]> {
    let decoded = decode_b64u(field, encoded)?;
    decoded
        .try_into()
        .map_err(|bytes: Vec<u8>| RciCryptoError::InvalidLength {
            field,
            expected: N,
            actual: bytes.len(),
        })
}

pub fn decode_b64u_capped(field: &'static str, encoded: &str, max: usize) -> Result<Vec<u8>> {
    let decoded = decode_b64u(field, encoded)?;
    if decoded.len() > max {
        return Err(RciCryptoError::CiphertextTooLarge {
            actual: decoded.len(),
            max,
        });
    }
    Ok(decoded)
}
