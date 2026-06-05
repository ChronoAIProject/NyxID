use sha2::{Digest, Sha256};

use crate::{Result, decode_b64u_array};

/// Canonical out-of-band fingerprint for an RCI node ephemeral public key.
///
/// Format: lowercase hex of `sha256(pubkey)[0..16]`, with no prefix.
pub fn rci_pubkey_fingerprint(pubkey: &[u8; 32]) -> String {
    let digest = Sha256::digest(pubkey);
    let mut out = String::with_capacity(32);
    for byte in &digest[..16] {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

pub fn rci_pubkey_fingerprint_b64u(encoded: &str) -> Result<String> {
    let pubkey = decode_b64u_array::<32>("node_pubkey", encoded)?;
    Ok(rci_pubkey_fingerprint(&pubkey))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode_b64u;

    #[test]
    fn fingerprint_helper_outputs_32_lower_hex_and_rejects_bad_length() {
        let pubkey = [7_u8; 32];
        let fingerprint = rci_pubkey_fingerprint(&pubkey);

        assert_eq!(fingerprint.len(), 32);
        assert!(
            fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(
            rci_pubkey_fingerprint_b64u(&encode_b64u(&pubkey)).unwrap(),
            fingerprint
        );

        let short = encode_b64u(&[7_u8; 31]);
        assert!(rci_pubkey_fingerprint_b64u(&short).is_err());
    }
}
