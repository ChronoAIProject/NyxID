use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::password;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};

/// Maximum password length to prevent Argon2 DoS via extremely long passwords.
const MAX_PASSWORD_LENGTH: usize = 128;

/// Result of a successful registration.
pub struct RegisterResult {
    pub user_id: String,
    /// Used in debug builds to log the verification token.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub email_verification_token: String,
}

/// Register a new user with email and password.
///
/// Validates that the email is not already taken, hashes the password,
/// and generates an email verification token.
///
/// To prevent email enumeration, returns a generic success response
/// even if the email is already registered. The caller should always
/// show the same message to the user regardless of the outcome.
pub async fn register_user(
    db: &mongodb::Database,
    email: &str,
    password_raw: &str,
    display_name: Option<&str>,
) -> AppResult<RegisterResult> {
    // Validate password length
    if password_raw.len() < 8 {
        return Err(AppError::ValidationError(
            "Password must be at least 8 characters".to_string(),
        ));
    }

    if password_raw.len() > MAX_PASSWORD_LENGTH {
        return Err(AppError::ValidationError(format!(
            "Password must be at most {} characters",
            MAX_PASSWORD_LENGTH
        )));
    }

    // Check for existing user - return fake success to prevent email enumeration
    let existing = db
        .collection::<User>(USERS)
        .find_one(doc! { "email": email.to_lowercase() })
        .await?;

    if existing.is_some() {
        // Return a fake success result to prevent email enumeration.
        // In production, send an email to the existing user informing them
        // that someone attempted to register with their address.
        tracing::warn!(email = %email, "Registration attempt for existing email");
        return Ok(RegisterResult {
            user_id: Uuid::new_v4().to_string(), // Fake ID, not stored
            email_verification_token: generate_random_token(), // Fake token
        });
    }

    let password_hash = password::hash_password(password_raw)?;
    let verification_token = generate_random_token();
    // Store the hash of the verification token, not the raw token
    let verification_token_hash = hash_token(&verification_token);
    let now = Utc::now();
    let user_id = Uuid::new_v4().to_string();

    let new_user = User {
        id: user_id.clone(),
        email: email.to_lowercase(),
        password_hash: Some(password_hash),
        display_name: display_name.map(String::from),
        avatar_url: None,
        email_verified: false,
        email_verification_token: Some(verification_token_hash),
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin: false,
        role_ids: vec![],
        group_ids: vec![],
        mfa_enabled: false,
        social_provider: None,
        social_provider_id: None,
        created_at: now,
        updated_at: now,
        last_login_at: None,
    };

    db.collection::<User>(USERS).insert_one(&new_user).await?;

    tracing::info!(user_id = %user_id, "User registered");

    Ok(RegisterResult {
        user_id,
        email_verification_token: verification_token,
    })
}

/// Authenticate a user with email and password.
///
/// Returns the user model on success, or an authentication error.
pub async fn authenticate_user(
    db: &mongodb::Database,
    email: &str,
    password_raw: &str,
) -> AppResult<User> {
    // Enforce maximum password length to prevent DoS
    if password_raw.len() > MAX_PASSWORD_LENGTH {
        return Err(AppError::AuthenticationFailed(
            "Invalid email or password".to_string(),
        ));
    }

    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "email": email.to_lowercase() })
        .await?
        .ok_or_else(|| AppError::AuthenticationFailed("Invalid email or password".to_string()))?;

    if !user.is_active {
        return Err(AppError::Forbidden("Account is deactivated".to_string()));
    }

    let password_hash = user.password_hash.as_deref().ok_or_else(|| {
        AppError::AuthenticationFailed(
            "This account uses social login. Please sign in with your social provider.".to_string(),
        )
    })?;

    let valid = password::verify_password(password_raw, password_hash)?;

    if !valid {
        return Err(AppError::AuthenticationFailed(
            "Invalid email or password".to_string(),
        ));
    }

    // Update last_login_at
    let now = Utc::now();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": &user.id },
            doc! { "$set": {
                "last_login_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(user)
}

/// Verify a user's email with the verification token.
///
/// Hashes the incoming token and compares against stored hash.
pub async fn verify_email(db: &mongodb::Database, token: &str) -> AppResult<String> {
    let token_hash = hash_token(token);

    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "email_verification_token": &token_hash })
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid or expired verification token".to_string()))?;

    if user.email_verified {
        return Err(AppError::BadRequest("Email already verified".to_string()));
    }

    let now = Utc::now();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": &user.id },
            doc! { "$set": {
                "email_verified": true,
                "email_verification_token": null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    tracing::info!(user_id = %user.id, "Email verified");

    Ok(user.id)
}

/// Initiate a password reset by generating a reset token.
///
/// Stores the hash of the token, not the raw token.
pub async fn initiate_password_reset(
    db: &mongodb::Database,
    email: &str,
) -> AppResult<Option<String>> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "email": email.to_lowercase() })
        .await?;

    // Always return Ok to prevent email enumeration
    let Some(user) = user else {
        return Ok(None);
    };

    let reset_token = generate_random_token();
    let reset_token_hash = hash_token(&reset_token);
    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let now = Utc::now();

    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": &user.id },
            doc! { "$set": {
                "password_reset_token": &reset_token_hash,
                "password_reset_expires_at": bson::DateTime::from_chrono(expires_at),
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(Some(reset_token))
}

/// Complete a password reset with the token and new password.
///
/// Hashes the incoming token for comparison against stored hash.
pub async fn reset_password(
    db: &mongodb::Database,
    token: &str,
    new_password: &str,
) -> AppResult<()> {
    if new_password.len() < 8 {
        return Err(AppError::ValidationError(
            "Password must be at least 8 characters".to_string(),
        ));
    }

    if new_password.len() > MAX_PASSWORD_LENGTH {
        return Err(AppError::ValidationError(format!(
            "Password must be at most {} characters",
            MAX_PASSWORD_LENGTH
        )));
    }

    let token_hash = hash_token(token);

    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "password_reset_token": &token_hash })
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid or expired reset token".to_string()))?;

    // Check token expiration
    if let Some(expires_at) = user.password_reset_expires_at {
        if expires_at < Utc::now() {
            return Err(AppError::BadRequest("Reset token has expired".to_string()));
        }
    } else {
        return Err(AppError::BadRequest("Invalid reset token".to_string()));
    }

    let new_hash = password::hash_password(new_password)?;
    let now = Utc::now();

    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": &user.id },
            doc! { "$set": {
                "password_hash": &new_hash,
                "password_reset_token": null,
                "password_reset_expires_at": null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    tracing::info!(user_id = %user.id, "Password reset completed");

    Ok(())
}

/// Promote an existing user to admin by email address.
///
/// Sets `is_admin = true` and `email_verified = true` on the user.
/// Returns the user ID on success.
pub async fn promote_user_to_admin(db: &mongodb::Database, email: &str) -> AppResult<String> {
    let normalized = email.to_lowercase();

    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "email": &normalized })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No user found with email: {}", normalized)))?;

    if user.is_admin {
        return Err(AppError::Conflict(format!(
            "User {} is already an admin",
            normalized
        )));
    }

    let now = Utc::now();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": &user.id },
            doc! { "$set": {
                "is_admin": true,
                "email_verified": true,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    tracing::info!(user_id = %user.id, email = %normalized, "User promoted to admin");

    Ok(user.id)
}
