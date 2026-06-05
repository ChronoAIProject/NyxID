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
    fn fingerprint_helper_matches_sha256_truncated_lower_hex_golden_vectors() {
        for (pubkey, expected) in [
            ([0_u8; 32], "66687aadf862bd776c8fc18b8e9f8e20"),
            ([7_u8; 32], "4bb06f8e4e3a7715d201d573d0aa4237"),
        ] {
            let fingerprint = rci_pubkey_fingerprint(&pubkey);

            assert_eq!(fingerprint, expected);
            assert_eq!(fingerprint.len(), 32);
            assert!(
                fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            );
            assert_eq!(
                rci_pubkey_fingerprint_b64u(&encode_b64u(&pubkey)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn fingerprint_b64u_adapter_rejects_non_pubkey_encodings() {
        let short = encode_b64u(&[7_u8; 31]);
        assert!(rci_pubkey_fingerprint_b64u(&short).is_err());
        assert!(rci_pubkey_fingerprint_b64u("sha256:66687aadf862bd776c8fc18b8e9f8e20").is_err());
        assert!(rci_pubkey_fingerprint_b64u("66687AADF862BD776C8FC18B8E9F8E20").is_err());
        assert!(
            rci_pubkey_fingerprint_b64u(
                "66687aadf862bd776c8fc18b8e9f8e20123456789abcdef0123456789abcdef0",
            )
            .is_err()
        );
        assert!(
            rci_pubkey_fingerprint_b64u("//////////////////////////////////////////8=").is_err()
        );
    }
}
