use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
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
}
