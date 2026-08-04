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

/// Constant-time comparison of two byte slices.
/// Prevents timing attacks by always comparing all bytes regardless of mismatches.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
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

    #[test]
    fn test_constant_time_eq_equal() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn test_constant_time_eq_different() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"short", b"longer string"));
    }

    #[test]
    fn test_constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_constant_time_eq_single_byte_match() {
        assert!(constant_time_eq(b"x", b"x"));
    }

    #[test]
    fn test_constant_time_eq_single_byte_mismatch() {
        assert!(!constant_time_eq(b"x", b"y"));
    }

    #[test]
    fn test_constant_time_eq_one_empty_one_not() {
        assert!(!constant_time_eq(b"", b"a"));
        assert!(!constant_time_eq(b"a", b""));
    }

    #[test]
    fn test_constant_time_eq_differ_in_last_byte() {
        assert!(!constant_time_eq(b"abcde", b"abcdf"));
    }

    #[test]
    fn test_constant_time_eq_differ_in_first_byte() {
        assert!(!constant_time_eq(b"Xbcde", b"abcde"));
    }

    #[test]
    fn test_hash_token_known_value() {
        // SHA-256 of the empty string is a well-known constant
        let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(hash_token(""), expected);
    }

    #[test]
    fn test_hash_token_known_value_hello() {
        // SHA-256 of "hello" is well-known
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert_eq!(hash_token("hello"), expected);
    }

    #[test]
    fn test_hash_token_output_is_lowercase_hex() {
        let hash = hash_token("some-input");
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }

    #[test]
    fn test_hash_token_length_always_64() {
        for input in ["", "x", "a longer string for testing", "nyx_abc123"] {
            assert_eq!(
                hash_token(input).len(),
                64,
                "hash of {input:?} should be 64 hex chars"
            );
        }
    }

    #[test]
    fn test_random_token_no_prefix() {
        let token = generate_random_token();
        // Should be pure hex, no prefix
        assert!(!token.starts_with("nyx_"));
        assert!(!token.starts_with("0x"));
    }

    #[test]
    fn test_api_key_full_key_length() {
        let (_, full_key, _) = generate_api_key();
        // "nyx_" prefix (4 chars) + 32 bytes hex-encoded (64 chars) = 68 chars
        assert_eq!(full_key.len(), 4 + API_KEY_LENGTH * 2);
    }

    #[test]
    fn test_api_key_prefix_matches_key_body() {
        let (prefix, full_key, _) = generate_api_key();
        // The prefix should be the first API_KEY_PREFIX_LENGTH chars of the hex body (after "nyx_")
        let key_body = full_key.strip_prefix("nyx_").unwrap();
        assert_eq!(&key_body[..API_KEY_PREFIX_LENGTH], &prefix);
    }

    #[test]
    fn test_api_key_hash_is_sha256_of_full_key() {
        let (_, full_key, hash) = generate_api_key();
        // Manually compute SHA-256 and compare
        let mut hasher = Sha256::new();
        hasher.update(full_key.as_bytes());
        let expected = hex::encode(hasher.finalize());
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_multiple_random_tokens_all_different() {
        let tokens: Vec<String> = (0..10).map(|_| generate_random_token()).collect();
        for i in 0..tokens.len() {
            for j in (i + 1)..tokens.len() {
                assert_ne!(tokens[i], tokens[j], "tokens at index {i} and {j} collided");
            }
        }
    }

    #[test]
    fn test_hash_token_unicode_input() {
        let hash = hash_token("hello-world-\u{1F600}");
        assert_eq!(hash.len(), 64);
        // Deterministic
        assert_eq!(hash, hash_token("hello-world-\u{1F600}"));
    }

    #[test]
    fn test_constant_time_eq_binary_values() {
        let a: Vec<u8> = (0..=255).collect();
        let b: Vec<u8> = (0..=255).collect();
        assert!(constant_time_eq(&a, &b));

        let mut c = a.clone();
        c[128] ^= 1; // flip one bit
        assert!(!constant_time_eq(&a, &c));
    }
}
