use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

use crate::errors::AppError;

/// Create an Argon2id hasher with OWASP-recommended parameters.
///
/// Parameters: m_cost=65536 KiB (64 MiB), t_cost=3 iterations, p_cost=4 parallelism
fn create_argon2() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 4, None).expect("Invalid Argon2 parameters"),
    )
}

/// Hash a plaintext password using Argon2id with a random salt.
///
/// Returns the PHC-formatted hash string that includes the algorithm
/// parameters, salt, and hash -- suitable for direct storage.
pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = create_argon2();

    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hashing failed: {e}")))?;

    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored Argon2 hash.
///
/// Returns true if the password matches, false otherwise.
/// Returns an error only if the hash string is malformed.
pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("Invalid password hash format: {e}")))?;

    // Note: verify_password uses constant-time comparison internally
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify() {
        let password = "correct-horse-battery-staple";
        let hash = hash_password(password).unwrap();

        assert!(verify_password(password, &hash).unwrap());
        assert!(!verify_password("wrong-password", &hash).unwrap());
    }

    #[test]
    fn test_different_salts() {
        let password = "test-password";
        let hash1 = hash_password(password).unwrap();
        let hash2 = hash_password(password).unwrap();

        // Same password should produce different hashes (different salts)
        assert_ne!(hash1, hash2);

        // Both should verify correctly
        assert!(verify_password(password, &hash1).unwrap());
        assert!(verify_password(password, &hash2).unwrap());
    }

    #[test]
    fn test_hash_format_is_phc() {
        let hash = hash_password("test-pass").unwrap();
        // PHC-format Argon2 hashes start with $argon2id$
        assert!(
            hash.starts_with("$argon2id$"),
            "Expected PHC format starting with $argon2id$, got: {hash}"
        );
        // PHC format has sections separated by $
        let sections: Vec<&str> = hash.split('$').collect();
        // ["", "argon2id", "v=19", "m=65536,t=3,p=4", "<salt>", "<hash>"]
        assert!(
            sections.len() >= 6,
            "PHC format should have at least 6 $-separated sections, got {}: {hash}",
            sections.len()
        );
    }

    #[test]
    fn test_hash_contains_expected_parameters() {
        let hash = hash_password("test-pass").unwrap();
        // OWASP parameters: m_cost=65536, t_cost=3, p_cost=4
        assert!(hash.contains("m=65536"), "Expected m=65536 in hash: {hash}");
        assert!(hash.contains("t=3"), "Expected t=3 in hash: {hash}");
        assert!(hash.contains("p=4"), "Expected p=4 in hash: {hash}");
        // Version 0x13 = 19
        assert!(hash.contains("v=19"), "Expected v=19 in hash: {hash}");
    }

    #[test]
    fn test_empty_password_roundtrip() {
        let hash = hash_password("").unwrap();
        assert!(verify_password("", &hash).unwrap());
        assert!(!verify_password("non-empty", &hash).unwrap());
    }

    #[test]
    fn test_long_password_roundtrip() {
        // Argon2 handles passwords up to 2^32 - 1 bytes; test with a long one
        let long_password = "a".repeat(1024);
        let hash = hash_password(&long_password).unwrap();
        assert!(verify_password(&long_password, &hash).unwrap());
        assert!(!verify_password(&"a".repeat(1023), &hash).unwrap());
    }

    #[test]
    fn test_unicode_password_roundtrip() {
        let password = "\u{1F512}\u{1F511} secr\u{00E9}t-p\u{00E4}ssw\u{00F6}rd";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash).unwrap());
        assert!(!verify_password("wrong", &hash).unwrap());
    }

    #[test]
    fn test_verify_with_malformed_hash_returns_error() {
        let result = verify_password("test", "not-a-valid-hash");
        assert!(result.is_err(), "Expected error for malformed hash");
    }

    #[test]
    fn test_verify_with_empty_hash_returns_error() {
        let result = verify_password("test", "");
        assert!(result.is_err(), "Expected error for empty hash");
    }

    #[test]
    fn test_similar_passwords_do_not_match() {
        let hash = hash_password("password123").unwrap();
        // Off-by-one and case differences
        assert!(!verify_password("password12", &hash).unwrap());
        assert!(!verify_password("password1234", &hash).unwrap());
        assert!(!verify_password("Password123", &hash).unwrap());
        assert!(!verify_password("password123 ", &hash).unwrap());
        assert!(!verify_password(" password123", &hash).unwrap());
    }

    #[test]
    fn test_special_characters_in_password() {
        let password = r#"p@$$w0rd!#%^&*(){}[]|\"':;<>?,./~`"#;
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash).unwrap());
    }

    #[test]
    fn test_whitespace_only_password() {
        let password = "   \t\n  ";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash).unwrap());
        assert!(!verify_password("", &hash).unwrap());
        assert!(!verify_password("   \t\n ", &hash).unwrap()); // one less space
    }
}
