use chrono::Utc;
use mongodb::bson::{self, doc};
use totp_rs::{Algorithm, Secret, TOTP};
use uuid::Uuid;

use crate::crypto::aes;
use crate::crypto::password;
use crate::crypto::token::generate_random_token;
use crate::errors::{AppError, AppResult};
use crate::models::mfa_factor::{MfaFactor, COLLECTION_NAME as MFA_FACTORS};

/// Result from initiating TOTP setup.
pub struct TotpSetupResult {
    pub factor_id: String,
    pub secret: String,
    pub qr_code_url: String,
}

/// Helper to create a TOTP instance with common parameters.
fn create_totp(
    secret_bytes: Vec<u8>,
    issuer: &str,
    account_name: &str,
) -> Result<TOTP, AppError> {
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
    encryption_key: &[u8],
    user_id: &str,
    user_email: &str,
) -> AppResult<TotpSetupResult> {
    // Check if user already has an active TOTP factor
    let existing = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "user_id": user_id,
            "factor_type": "totp",
            "is_active": true,
        })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "TOTP is already configured for this account".to_string(),
        ));
    }

    let secret = Secret::generate_secret();
    let secret_base32 = secret.to_encoded().to_string();

    let totp = create_totp(
        secret.to_bytes().map_err(|e| {
            AppError::Internal(format!("Failed to convert secret to bytes: {e}"))
        })?,
        "NyxID",
        user_email,
    )?;

    let qr_code_url = totp.get_url();

    // Encrypt the secret for storage
    let encrypted_secret = aes::encrypt(secret_base32.as_bytes(), encryption_key)?;

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
    encryption_key: &[u8],
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

    let secret_bytes = aes::decrypt(encrypted_secret, encryption_key)?;
    let secret_str = String::from_utf8(secret_bytes)
        .map_err(|e| AppError::Internal(format!("Invalid secret encoding: {e}")))?;

    let secret = Secret::Encoded(secret_str);
    let totp = create_totp(
        secret.to_bytes().map_err(|e| {
            AppError::Internal(format!("Failed to convert secret: {e}"))
        })?,
        "NyxID",
        user_id,
    )?;

    if !totp.check_current(code).map_err(|e| {
        AppError::Internal(format!("TOTP verification error: {e}"))
    })? {
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

    let codes_bson = bson::to_bson(&codes_json)
        .map_err(|e| AppError::Internal(format!("Failed to convert recovery codes to BSON: {e}")))?;

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
    encryption_key: &[u8],
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

    let secret_bytes = aes::decrypt(encrypted_secret, encryption_key)?;
    let secret_str = String::from_utf8(secret_bytes)
        .map_err(|e| AppError::Internal(format!("Invalid secret encoding: {e}")))?;

    let secret = Secret::Encoded(secret_str);
    let totp = create_totp(
        secret.to_bytes().map_err(|e| {
            AppError::Internal(format!("Failed to convert secret: {e}"))
        })?,
        "NyxID",
        user_id,
    )?;

    let valid = totp.check_current(code).map_err(|e| {
        AppError::Internal(format!("TOTP verification error: {e}"))
    })?;

    Ok(valid)
}
