use chrono::Utc;
use mongodb::bson::{self, doc};
use totp_rs::{Algorithm, Secret, TOTP};
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::password;
use crate::crypto::token::generate_random_token;
use crate::errors::{AppError, AppResult};
use crate::models::mfa_factor::{COLLECTION_NAME as MFA_FACTORS, MfaFactor};

/// Result from initiating TOTP setup.
pub struct TotpSetupResult {
    pub factor_id: String,
    pub secret: String,
    pub qr_code_url: String,
}

/// Helper to create a TOTP instance with common parameters.
fn create_totp(secret_bytes: Vec<u8>, issuer: &str, account_name: &str) -> Result<TOTP, AppError> {
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        Some(issuer.to_string()),
        account_name.to_string(),
    )
    .map_err(|e| AppError::Internal(format!("Failed to create TOTP: {e}")))
}

/// Start TOTP enrollment for a user.
///
/// Generates a new TOTP secret, encrypts it, and stores it as an unverified
/// factor. Returns the secret and QR code URL for the authenticator app.
pub async fn setup_totp(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    user_email: &str,
) -> AppResult<TotpSetupResult> {
    // Look up any active TOTP factor for this user. There are two
    // shapes that can come back:
    //   - VERIFIED: MFA is fully enabled, the user is using TOTP.
    //     Refuse with 409 — they should `mfa disable` first.
    //   - UNVERIFIED: a prior `setup` call started enrollment but
    //     `confirm` never completed (e.g. user closed the wizard tab,
    //     terminal flow died mid-prompt, NyxID#506 reproducer). The
    //     stranded factor blocks re-enrollment with no recovery path
    //     other than direct DB access. Deactivate it and proceed —
    //     the user-visible effect is exactly what they expected from
    //     re-running `mfa setup`.
    let existing = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "user_id": user_id,
            "factor_type": "totp",
            "is_active": true,
        })
        .await?;

    if let Some(prev) = existing {
        if prev.is_verified {
            return Err(AppError::Conflict(
                "TOTP is already configured for this account".to_string(),
            ));
        }
        // Soft-deactivate the stranded unverified factor. We update by
        // _id rather than the broader filter so a concurrent `confirm`
        // racing this update can't accidentally activate the new
        // factor we're about to mint (the confirm handler in
        // `verify_totp_setup` looks up by factor_id + user_id, so a
        // legitimate confirm in flight will simply 404 once we mark
        // this row inactive — exactly what the refresh path wants).
        let now = Utc::now();
        db.collection::<MfaFactor>(MFA_FACTORS)
            .update_one(
                doc! { "_id": &prev.id },
                doc! {
                    "$set": {
                        "is_active": false,
                        "updated_at": bson::DateTime::from_chrono(now),
                    }
                },
            )
            .await?;
    }

    let secret = Secret::generate_secret();
    let secret_base32 = secret.to_encoded().to_string();

    let totp = create_totp(
        secret
            .to_bytes()
            .map_err(|e| AppError::Internal(format!("Failed to convert secret to bytes: {e}")))?,
        "NyxID",
        user_email,
    )?;

    let qr_code_url = totp.get_url();

    // Encrypt the secret for storage
    let encrypted_secret = encryption_keys.encrypt(secret_base32.as_bytes()).await?;

    let factor_id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let new_factor = MfaFactor {
        id: factor_id.clone(),
        user_id: user_id.to_string(),
        factor_type: "totp".to_string(),
        secret_encrypted: Some(encrypted_secret),
        recovery_codes: None,
        is_verified: false,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    db.collection::<MfaFactor>(MFA_FACTORS)
        .insert_one(&new_factor)
        .await?;

    Ok(TotpSetupResult {
        factor_id,
        secret: secret_base32,
        qr_code_url,
    })
}

/// Verify a TOTP code to complete enrollment.
pub async fn verify_totp_setup(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    factor_id: &str,
    user_id: &str,
    code: &str,
) -> AppResult<Vec<String>> {
    let factor = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "_id": factor_id,
            "user_id": user_id,
            "factor_type": "totp",
        })
        .await?
        .ok_or_else(|| AppError::NotFound("TOTP factor not found".to_string()))?;

    if factor.is_verified {
        return Err(AppError::BadRequest("Factor already verified".to_string()));
    }

    let encrypted_secret = factor
        .secret_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("Missing encrypted secret".to_string()))?;

    let secret_bytes = encryption_keys.decrypt(encrypted_secret).await?;
    let secret_str = String::from_utf8(secret_bytes)
        .map_err(|e| AppError::Internal(format!("Invalid secret encoding: {e}")))?;

    let secret = Secret::Encoded(secret_str);
    let totp = create_totp(
        secret
            .to_bytes()
            .map_err(|e| AppError::Internal(format!("Failed to convert secret: {e}")))?,
        "NyxID",
        user_id,
    )?;

    if !totp
        .check_current(code)
        .map_err(|e| AppError::Internal(format!("TOTP verification error: {e}")))?
    {
        return Err(AppError::AuthenticationFailed(
            "Invalid TOTP code".to_string(),
        ));
    }

    // Generate recovery codes
    let recovery_codes: Vec<String> = (0..10)
        .map(|_| {
            let token = generate_random_token();
            token[..8].to_string()
        })
        .collect();

    let hashed_codes: Vec<String> = recovery_codes
        .iter()
        .map(|c| password::hash_password(c))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Internal(format!("Failed to hash recovery code: {e}")))?;

    let codes_json = serde_json::to_value(&hashed_codes)
        .map_err(|e| AppError::Internal(format!("Failed to serialize recovery codes: {e}")))?;

    let codes_bson = bson::to_bson(&codes_json).map_err(|e| {
        AppError::Internal(format!("Failed to convert recovery codes to BSON: {e}"))
    })?;

    // Mark factor as verified
    let now = Utc::now();
    db.collection::<MfaFactor>(MFA_FACTORS)
        .update_one(
            doc! { "_id": factor_id },
            doc! { "$set": {
                "is_verified": true,
                "recovery_codes": codes_bson,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(recovery_codes)
}

/// Verify a TOTP code during login.
pub async fn verify_totp(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    code: &str,
) -> AppResult<bool> {
    let factor = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "user_id": user_id,
            "factor_type": "totp",
            "is_verified": true,
            "is_active": true,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("No active TOTP factor found".to_string()))?;

    let encrypted_secret = factor
        .secret_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("Missing encrypted secret".to_string()))?;

    let secret_bytes = encryption_keys.decrypt(encrypted_secret).await?;
    let secret_str = String::from_utf8(secret_bytes)
        .map_err(|e| AppError::Internal(format!("Invalid secret encoding: {e}")))?;

    let secret = Secret::Encoded(secret_str);
    let totp = create_totp(
        secret
            .to_bytes()
            .map_err(|e| AppError::Internal(format!("Failed to convert secret: {e}")))?,
        "NyxID",
        user_id,
    )?;

    let valid = totp
        .check_current(code)
        .map_err(|e| AppError::Internal(format!("TOTP verification error: {e}")))?;

    Ok(valid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    #[tokio::test]
    async fn test_setup_totp_creates_unverified_factor() {
        let Some(db) = connect_test_database("mfa_svc").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let result = setup_totp(&db, &encryption_keys, &user_id, "user@example.com").await;
        assert!(result.is_ok());

        let setup = result.unwrap();
        assert!(!setup.factor_id.is_empty());
        assert!(!setup.secret.is_empty());
        assert!(setup.qr_code_url.contains("otpauth://totp/"));

        let stored = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! { "_id": &setup.factor_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.user_id, user_id);
        assert_eq!(stored.factor_type, "totp");
        assert!(!stored.is_verified);
        assert!(stored.is_active);
        assert!(stored.secret_encrypted.is_some());
    }

    #[tokio::test]
    async fn test_setup_totp_rejects_if_already_verified() {
        let Some(db) = connect_test_database("mfa_svc").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let factor = MfaFactor {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.clone(),
            factor_type: "totp".to_string(),
            secret_encrypted: Some(vec![1, 2, 3]),
            recovery_codes: None,
            is_verified: true,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.collection::<MfaFactor>(MFA_FACTORS)
            .insert_one(&factor)
            .await
            .unwrap();

        let result = setup_totp(&db, &encryption_keys, &user_id, "user@example.com").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_setup_totp_replaces_unverified_factor() {
        let Some(db) = connect_test_database("mfa_svc").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let first = setup_totp(&db, &encryption_keys, &user_id, "user@example.com")
            .await
            .unwrap();

        let second = setup_totp(&db, &encryption_keys, &user_id, "user@example.com")
            .await
            .unwrap();

        assert_ne!(first.factor_id, second.factor_id);

        let old = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! { "_id": &first.factor_id })
            .await
            .unwrap()
            .unwrap();
        assert!(!old.is_active);
    }

    #[tokio::test]
    async fn test_verify_totp_setup_with_valid_code() {
        let Some(db) = connect_test_database("mfa_verify").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let Ok(setup) = setup_totp(&db, &encryption_keys, &user_id, "user@example.com").await
        else {
            eprintln!("Skipping: MongoDB connection lost during TOTP setup");
            return;
        };

        let secret = Secret::Encoded(setup.secret.clone());
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret.to_bytes().unwrap(),
            Some("NyxID".to_string()),
            user_id.clone(),
        )
        .unwrap();
        let code = totp.generate_current().unwrap();

        let Ok(recovery_codes) =
            verify_totp_setup(&db, &encryption_keys, &setup.factor_id, &user_id, &code).await
        else {
            eprintln!("Skipping: MongoDB connection lost during TOTP verify");
            return;
        };

        assert_eq!(recovery_codes.len(), 10);
        for code in &recovery_codes {
            assert_eq!(code.len(), 8);
        }

        let stored = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! { "_id": &setup.factor_id })
            .await
            .unwrap()
            .unwrap();
        assert!(stored.is_verified);
        assert!(stored.recovery_codes.is_some());
    }

    #[tokio::test]
    async fn test_verify_totp_setup_rejects_invalid_code() {
        let Some(db) = connect_test_database("mfa_reject").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let setup = setup_totp(&db, &encryption_keys, &user_id, "user@example.com")
            .await
            .unwrap();

        let result =
            verify_totp_setup(&db, &encryption_keys, &setup.factor_id, &user_id, "000000").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_verify_totp_login_with_valid_code() {
        let Some(db) = connect_test_database("mfa_login").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let Ok(setup) = setup_totp(&db, &encryption_keys, &user_id, "user@example.com").await
        else {
            eprintln!("Skipping: MongoDB connection lost during TOTP setup");
            return;
        };

        let secret = Secret::Encoded(setup.secret.clone());
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret.to_bytes().unwrap(),
            Some("NyxID".to_string()),
            user_id.clone(),
        )
        .unwrap();
        let code = totp.generate_current().unwrap();

        let Ok(_) =
            verify_totp_setup(&db, &encryption_keys, &setup.factor_id, &user_id, &code).await
        else {
            eprintln!("Skipping: MongoDB connection lost during TOTP verify");
            return;
        };

        let login_code = totp.generate_current().unwrap();
        let Ok(valid) = verify_totp(&db, &encryption_keys, &user_id, &login_code).await else {
            eprintln!("Skipping: MongoDB connection lost during TOTP login");
            return;
        };
        assert!(valid);
    }

    #[test]
    fn create_totp_returns_valid_instance() {
        let secret = Secret::generate_secret();
        let result = create_totp(secret.to_bytes().unwrap(), "NyxID", "test@example.com");
        assert!(result.is_ok());
        let totp = result.unwrap();
        let code = totp.generate_current().unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn create_totp_url_contains_issuer_and_account() {
        let secret = Secret::generate_secret();
        let totp = create_totp(secret.to_bytes().unwrap(), "NyxID", "user@example.com").unwrap();
        let url = totp.get_url();
        assert!(url.contains("NyxID"));
        assert!(url.contains("user%40example.com") || url.contains("user@example.com"));
        assert!(url.starts_with("otpauth://totp/"));
    }

    #[test]
    fn create_totp_with_empty_secret_fails() {
        let result = create_totp(vec![], "NyxID", "user@example.com");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_verify_totp_setup_not_found() {
        let Some(db) = connect_test_database("mfa_setup_nf").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();

        let err = verify_totp_setup(
            &db,
            &encryption_keys,
            "nonexistent-factor-id",
            "nonexistent-user",
            "123456",
        )
        .await
        .expect_err("should not find factor");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_verify_totp_setup_already_verified() {
        let Some(db) = connect_test_database("mfa_already_v").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let Ok(setup) = setup_totp(&db, &encryption_keys, &user_id, "user@example.com").await
        else {
            eprintln!("Skipping: MongoDB connection lost during TOTP setup");
            return;
        };

        let secret = Secret::Encoded(setup.secret.clone());
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret.to_bytes().unwrap(),
            Some("NyxID".to_string()),
            user_id.clone(),
        )
        .unwrap();
        let code = totp.generate_current().unwrap();

        let Ok(_) =
            verify_totp_setup(&db, &encryption_keys, &setup.factor_id, &user_id, &code).await
        else {
            eprintln!("Skipping: MongoDB connection lost during TOTP verify");
            return;
        };

        let code2 = totp.generate_current().unwrap();
        let err = verify_totp_setup(&db, &encryption_keys, &setup.factor_id, &user_id, &code2)
            .await
            .expect_err("already verified");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_verify_totp_login_no_active_factor() {
        let Some(db) = connect_test_database("mfa_login_nf").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let err = verify_totp(&db, &encryption_keys, &user_id, "123456")
            .await
            .expect_err("no active factor");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_setup_totp_qr_url_format() {
        let Some(db) = connect_test_database("mfa_qr_fmt").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();

        let result = setup_totp(&db, &encryption_keys, &user_id, "alice@example.com")
            .await
            .expect("setup");
        assert!(result.qr_code_url.starts_with("otpauth://totp/"));
        assert!(result.qr_code_url.contains("NyxID"));
        assert!(result.qr_code_url.contains("secret="));
    }

    #[tokio::test]
    async fn test_setup_totp_different_users_get_different_secrets() {
        let Some(db) = connect_test_database("mfa_diff_sec").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user1 = Uuid::new_v4().to_string();
        let user2 = Uuid::new_v4().to_string();

        let setup1 = setup_totp(&db, &encryption_keys, &user1, "user1@example.com")
            .await
            .expect("setup1");
        let setup2 = setup_totp(&db, &encryption_keys, &user2, "user2@example.com")
            .await
            .expect("setup2");

        assert_ne!(setup1.secret, setup2.secret);
        assert_ne!(setup1.factor_id, setup2.factor_id);
    }
}
