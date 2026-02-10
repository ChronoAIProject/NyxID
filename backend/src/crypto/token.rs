use rand::RngCore;
use sha2::{Digest, Sha256};

/// Length of a generated API key in bytes (before encoding).
const API_KEY_LENGTH: usize = 32;

/// Length of a random token in bytes (before hex encoding).
const RANDOM_TOKEN_LENGTH: usize = 32;

/// Prefix length for API keys (used for lookup without exposing the full key).
const API_KEY_PREFIX_LENGTH: usize = 8;

/// Generate an API key.
///
/// Returns a tuple of (prefix, full_key, sha256_hash):
/// - `prefix`: first 8 characters, stored in plaintext for key lookup
/// - `full_key`: the complete key shown once to the user (nyx_<hex>)
/// - `hash`: SHA-256 hash of the full key, stored for verification
pub fn generate_api_key() -> (String, String, String) {
    let mut bytes = [0u8; API_KEY_LENGTH];
    rand::thread_rng().fill_bytes(&mut bytes);

    let hex_encoded = hex::encode(bytes);
    let full_key = format!("nyx_{hex_encoded}");
    let prefix = hex_encoded[..API_KEY_PREFIX_LENGTH].to_string();
    let hash = hash_token(&full_key);

    (prefix, full_key, hash)
}

/// Generate a cryptographically random token as a hex string.
///
/// Suitable for email verification tokens, password reset tokens,
/// PKCE code verifiers, and other one-time-use secrets.
pub fn generate_random_token() -> String {
    let mut bytes = [0u8; RANDOM_TOKEN_LENGTH];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Compute SHA-256 hash of a token string, returning hex-encoded digest.
///
/// Used to store hashed versions of tokens and API keys so that the
/// raw secret is never persisted.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_format() {
        let (prefix, full_key, hash) = generate_api_key();

        assert_eq!(prefix.len(), API_KEY_PREFIX_LENGTH);
        assert!(full_key.starts_with("nyx_"));
        assert_eq!(hash.len(), 64); // SHA-256 hex digest
        assert!(full_key.contains(&prefix));
    }

    #[test]
    fn test_random_token_length() {
        let token = generate_random_token();
        assert_eq!(token.len(), RANDOM_TOKEN_LENGTH * 2); // hex doubles length
    }

    #[test]
    fn test_hash_deterministic() {
        let token = "test-token";
        let hash1 = hash_token(token);
        let hash2 = hash_token(token);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_unique_keys() {
        let (_, key1, _) = generate_api_key();
        let (_, key2, _) = generate_api_key();
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_hash_different_inputs_differ() {
        let hash1 = hash_token("input-a");
        let hash2 = hash_token("input-b");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_empty_string() {
        let hash = hash_token("");
        assert_eq!(hash.len(), 64); // SHA-256 is always 64 hex chars
    }

    #[test]
    fn test_api_key_hash_matches_full_key() {
        let (_, full_key, hash) = generate_api_key();
        let recomputed = hash_token(&full_key);
        assert_eq!(hash, recomputed);
    }

    #[test]
    fn test_random_tokens_unique() {
        let t1 = generate_random_token();
        let t2 = generate_random_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_random_token_is_hex() {
        let token = generate_random_token();
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_api_key_prefix_is_hex() {
        let (prefix, _, _) = generate_api_key();
        assert!(prefix.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
