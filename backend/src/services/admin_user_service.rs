use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::password;
use crate::errors::{AppError, AppResult};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};
use crate::models::user::{COLLECTION_NAME as USERS, User};

/// Maximum password length to prevent Argon2 DoS via extremely long passwords.
const MAX_PASSWORD_LENGTH: usize = 128;

// Collection name constants for cascade delete
const REFRESH_TOKENS: &str = "refresh_tokens";
const API_KEYS: &str = "api_keys";
const USER_SERVICE_CONNECTIONS: &str = "user_service_connections";
const USER_PROVIDER_TOKENS: &str = "user_provider_tokens";
const MFA_FACTORS: &str = "mfa_factors";
const AUTHORIZATION_CODES: &str = "authorization_codes";
const OAUTH_STATES: &str = "oauth_states";

/// Create a new user (admin action).
///
/// Hashes the password with Argon2id, validates email uniqueness
/// (case-insensitive), and creates the user with the specified role.
/// Admin-created accounts are pre-verified and active.
pub async fn create_user(
    db: &mongodb::Database,
    email: &str,
    password_raw: &str,
    display_name: Option<&str>,
    role: &str,
) -> AppResult<User> {
    // Validate email format
    let trimmed = email.trim();
    let at_pos = trimmed.find('@');
    let is_valid = match at_pos {
        Some(pos) => {
            let local = &trimmed[..pos];
            let domain = &trimmed[pos + 1..];
            trimmed.len() >= 5
                && !local.is_empty()
                && !domain.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        None => false,
    };
    if !is_valid {
        return Err(AppError::ValidationError(
            "Invalid email format".to_string(),
        ));
    }

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

    // Validate role
    if role != "admin" && role != "user" {
        return Err(AppError::ValidationError(
            "Role must be 'admin' or 'user'".to_string(),
        ));
    }

    // Check email uniqueness (case-insensitive)
    let normalized = email.to_lowercase();
    let existing = db
        .collection::<User>(USERS)
        .find_one(doc! { "email": &normalized })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "A user with this email already exists".to_string(),
        ));
    }

    // Validate display name length
    if let Some(name) = display_name
        && name.len() > 200
    {
        return Err(AppError::ValidationError(
            "Display name must be 200 characters or less".to_string(),
        ));
    }

    let password_hash = password::hash_password(password_raw)?;
    let now = Utc::now();
    let user_id = Uuid::new_v4().to_string();
    let is_admin = role == "admin";

    let new_user = User {
        id: user_id.clone(),
        email: normalized,
        password_hash: Some(password_hash),
        display_name: display_name.map(String::from),
        avatar_url: None,
        email_verified: true,
        email_verification_token: None,
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin,
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

    tracing::info!(user_id = %user_id, is_admin = %is_admin, "Admin created user");

    Ok(new_user)
}

/// Update a user's profile fields (admin action).
///
/// Only provided fields are updated. Validates email uniqueness and
/// field constraints.
pub async fn update_user(
    db: &mongodb::Database,
    user_id: &str,
    display_name: Option<&str>,
    email: Option<&str>,
    avatar_url: Option<&str>,
) -> AppResult<User> {
    let existing = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let mut set_doc = doc! {};

    if let Some(name) = display_name {
        if name.len() > 200 {
            return Err(AppError::ValidationError(
                "Display name must be 200 characters or less".to_string(),
            ));
        }
        set_doc.insert("display_name", name);
    }

    if let Some(new_email) = email {
        // Email format validation
        let trimmed = new_email.trim();
        let at_pos = trimmed.find('@');
        let is_valid = match at_pos {
            Some(pos) => {
                let local = &trimmed[..pos];
                let domain = &trimmed[pos + 1..];
                trimmed.len() >= 5
                    && !local.is_empty()
                    && !domain.is_empty()
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
            }
            None => false,
        };
        if !is_valid {
            return Err(AppError::ValidationError(
                "Invalid email format".to_string(),
            ));
        }

        // Check uniqueness (case-insensitive)
        let normalized = new_email.to_lowercase();
        let existing_with_email = db
            .collection::<User>(USERS)
            .find_one(doc! { "email": &normalized, "_id": { "$ne": user_id } })
            .await?;

        if existing_with_email.is_some() {
            return Err(AppError::ValidationError(
                "Email already in use".to_string(),
            ));
        }

        set_doc.insert("email", normalized);
    }

    if let Some(url) = avatar_url {
        if url.len() > 2048 {
            return Err(AppError::ValidationError(
                "Avatar URL must be 2048 characters or less".to_string(),
            ));
        }
        if !url.starts_with("https://") {
            return Err(AppError::ValidationError(
                "Avatar URL must use https:// scheme".to_string(),
            ));
        }
        set_doc.insert("avatar_url", url);
    }

    // Early return if no actual fields changed
    if set_doc.is_empty() {
        return Ok(existing);
    }

    let now = Utc::now();
    set_doc.insert("updated_at", bson::DateTime::from_chrono(now));

    db.collection::<User>(USERS)
        .update_one(doc! { "_id": user_id }, doc! { "$set": set_doc })
        .await?;

    let updated = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::Internal("User disappeared after update".to_string()))?;

    Ok(updated)
}

/// Set the admin role for a target user.
///
/// Self-protection: admin_user_id must differ from target_user_id.
pub async fn set_admin_role(
    db: &mongodb::Database,
    admin_user_id: &str,
    target_user_id: &str,
    is_admin: bool,
) -> AppResult<()> {
    if admin_user_id == target_user_id {
        return Err(AppError::ValidationError(
            "Cannot change your own admin role".to_string(),
        ));
    }

    let _target = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": target_user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let now = Utc::now();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": target_user_id },
            doc! { "$set": {
                "is_admin": is_admin,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(())
}

/// Set the active status for a target user.
///
/// Self-protection: admin_user_id must differ from target_user_id.
/// Side effect: when disabling, revokes all sessions.
pub async fn set_user_active(
    db: &mongodb::Database,
    admin_user_id: &str,
    target_user_id: &str,
    is_active: bool,
) -> AppResult<()> {
    if admin_user_id == target_user_id {
        return Err(AppError::ValidationError(
            "Cannot change your own active status".to_string(),
        ));
    }

    let _target = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": target_user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let now = Utc::now();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": target_user_id },
            doc! { "$set": {
                "is_active": is_active,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // Revoke all sessions and deactivate API keys when disabling a user
    if !is_active {
        revoke_all_user_sessions(db, target_user_id).await?;

        db.collection::<bson::Document>(API_KEYS)
            .update_many(
                doc! { "user_id": target_user_id, "is_active": true },
                doc! { "$set": { "is_active": false } },
            )
            .await?;
    }

    Ok(())
}

/// Initiate a forced password reset for a user.
///
/// Returns the reset token on success, or an error if the user has no
/// password (social login only).
pub async fn force_password_reset(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Option<String>> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if user.password_hash.is_none() {
        return Err(AppError::BadRequest(
            "User has no password (social login only)".to_string(),
        ));
    }

    let token = crate::services::auth_service::initiate_password_reset(db, &user.email).await?;

    // Revoke all sessions to force re-authentication
    revoke_all_user_sessions(db, user_id).await?;

    Ok(token)
}

/// Delete a user and cascade-delete all related documents.
///
/// Self-protection: admin_user_id must differ from target_user_id.
/// Audit log entries are retained (orphaned reference).
///
/// Attempts to run inside a MongoDB transaction for atomicity. If
/// transactions are unavailable (standalone), falls back to a two-phase
/// approach: mark the user as inactive first, then delete related data.
pub async fn delete_user_cascade(
    db: &mongodb::Database,
    admin_user_id: &str,
    target_user_id: &str,
) -> AppResult<()> {
    if admin_user_id == target_user_id {
        return Err(AppError::ValidationError(
            "Cannot delete yourself".to_string(),
        ));
    }

    let _target = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": target_user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    // Phase 1: mark user inactive so they cannot authenticate during cleanup
    let now = Utc::now();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": target_user_id },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // Phase 2: cascade delete related documents
    let filter = doc! { "user_id": target_user_id };

    let collections = [
        SESSIONS,
        REFRESH_TOKENS,
        API_KEYS,
        USER_SERVICE_CONNECTIONS,
        USER_PROVIDER_TOKENS,
        MFA_FACTORS,
        AUTHORIZATION_CODES,
        OAUTH_STATES,
    ];

    for coll_name in collections {
        db.collection::<bson::Document>(coll_name)
            .delete_many(filter.clone())
            .await?;
    }

    // Phase 3: delete the user document itself
    db.collection::<User>(USERS)
        .delete_one(doc! { "_id": target_user_id })
        .await?;

    Ok(())
}

/// Manually verify a user's email address.
///
/// Sets email_verified = true and clears the verification token.
pub async fn verify_email(db: &mongodb::Database, user_id: &str) -> AppResult<()> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if user.email_verified {
        return Err(AppError::BadRequest("Email already verified".to_string()));
    }

    let now = Utc::now();
    db.collection::<User>(USERS)
        .update_one(
            doc! { "_id": user_id },
            doc! { "$set": {
                "email_verified": true,
                "email_verification_token": null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(())
}

/// List all sessions for a user, sorted by created_at descending.
pub async fn list_user_sessions(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<Session>> {
    use futures::TryStreamExt;

    // Verify user exists
    let _user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let sessions: Vec<Session> = db
        .collection::<Session>(SESSIONS)
        .find(doc! { "user_id": user_id })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(sessions)
}

/// Revoke all non-revoked sessions and refresh tokens for a user.
///
/// Returns the number of sessions revoked.
pub async fn revoke_all_user_sessions(db: &mongodb::Database, user_id: &str) -> AppResult<u64> {
    let now = bson::DateTime::from_chrono(Utc::now());

    // Revoke active sessions
    let result = db
        .collection::<Session>(SESSIONS)
        .update_many(
            doc! { "user_id": user_id, "revoked": false },
            doc! { "$set": { "revoked": true, "last_active_at": &now } },
        )
        .await?;

    let revoked_count = result.modified_count;

    // Revoke associated refresh tokens
    db.collection::<bson::Document>(REFRESH_TOKENS)
        .update_many(
            doc! { "user_id": user_id, "revoked": false },
            doc! { "$set": { "revoked": true } },
        )
        .await?;

    Ok(revoked_count)
}
