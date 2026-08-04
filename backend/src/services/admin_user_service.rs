use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::password;
use crate::errors::{AppError, AppResult};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};
use crate::models::user::{COLLECTION_NAME as USERS, PlatformRole, User};
use crate::services::role_service;

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
const CONSENTS: &str = "consents";
const MCP_SESSIONS: &str = "mcp_sessions";
const APPROVAL_REQUESTS: &str = "approval_requests";
const APPROVAL_GRANTS: &str = "approval_grants";
const SERVICE_APPROVAL_CONFIGS: &str = "service_approval_configs";
const NOTIFICATION_CHANNELS: &str = "notification_channels";
const OAUTH_CLIENTS: &str = "oauth_clients";
const SERVICE_ACCOUNTS: &str = "service_accounts";
const SERVICE_ACCOUNT_TOKENS: &str = "service_account_tokens";

/// Look up the email for `user_id` without erroring on "not found".
///
/// Returns `Ok(None)` if the user doesn't exist (or any other lookup
/// failure that the caller wants to treat as a soft miss). Used by the
/// OAuth callback handler to compose a session-mismatch message -- the
/// callback must not block on a database read.
pub async fn get_user_email(db: &mongodb::Database, user_id: &str) -> AppResult<Option<String>> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?;
    Ok(user.map(|u| u.email))
}

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
    if role != "admin" && role != "user" && role != "operator" {
        return Err(AppError::ValidationError(
            "Role must be 'admin', 'operator', or 'user'".to_string(),
        ));
    }

    // Check email uniqueness (case-insensitive). Scoped to person accounts
    // because the new partial-unique index on `users.email` only constrains
    // `user_type = "person"`, and orgs are allowed to share contact emails
    // with persons.
    let normalized = email.to_lowercase();
    let existing = db
        .collection::<User>(USERS)
        .find_one(doc! {
            "email": &normalized,
            "user_type": "person",
        })
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
    let platform_role = match role {
        "admin" => PlatformRole::Admin,
        "operator" => PlatformRole::Operator,
        "user" => PlatformRole::User,
        _ => {
            return Err(AppError::ValidationError(
                "Role must be 'admin', 'operator', or 'user'".to_string(),
            ));
        }
    };
    let (is_admin, is_operator) = platform_role.legacy_flags();

    // Auto-assign default roles to new admin-created users
    let mut role_ids = role_service::get_default_role_ids(db).await?;
    let platform_role_ids = role_service::get_platform_role_ids(db).await?;
    role_service::add_platform_role_id(&mut role_ids, platform_role, &platform_role_ids);

    let new_user = User {
        id: user_id.clone(),
        email: normalized,
        password_hash: Some(password_hash),
        display_name: display_name.map(String::from),
        slug: None,
        avatar_url: None,
        email_verified: true,
        email_verification_token: None,
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin,
        is_operator,
        role_ids,
        group_ids: vec![],
        invite_code_id: None,
        mfa_enabled: false,
        social_provider: None,
        social_provider_id: None,
        user_type: crate::models::user::UserType::Person,
        primary_org_id: None,
        created_at: now,
        updated_at: now,
        last_login_at: None,
        profile_config: Default::default(),
    };

    db.collection::<User>(USERS).insert_one(&new_user).await?;

    tracing::info!(user_id = %user_id, is_admin = %is_admin, is_operator = %is_operator, "Admin created user");

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

        // Check uniqueness (case-insensitive). Scoped to person accounts so
        // that an org's contact email does not spuriously block a person
        // rename, matching the partial-unique `users.email` index.
        let normalized = new_email.to_lowercase();
        let existing_with_email = db
            .collection::<User>(USERS)
            .find_one(doc! {
                "email": &normalized,
                "user_type": "person",
                "_id": { "$ne": user_id },
            })
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

/// Set the platform role for a target user. Accepts `"admin"`, `"operator"`,
/// or `"user"`. Self-protection: admin_user_id must differ from target_user_id.
///
/// Platform RBAC role membership is authoritative. The legacy boolean fields
/// are still mirrored so older deployment code and stored documents remain
/// compatible during the migration window.
pub async fn set_platform_role(
    db: &mongodb::Database,
    admin_user_id: &str,
    target_user_id: &str,
    role: &str,
) -> AppResult<User> {
    if admin_user_id == target_user_id {
        return Err(AppError::ValidationError(
            "Cannot change your own platform role".to_string(),
        ));
    }

    let platform_role = match role {
        "admin" => PlatformRole::Admin,
        "operator" => PlatformRole::Operator,
        "user" => PlatformRole::User,
        _ => {
            return Err(AppError::ValidationError(
                "Role must be 'admin', 'operator', or 'user'".to_string(),
            ));
        }
    };

    let _target = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": target_user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let platform_role_ids = role_service::get_platform_role_ids(db).await?;
    let pipeline = role_service::set_platform_role_update(
        platform_role,
        &platform_role_ids,
        bson::DateTime::from_chrono(Utc::now()),
    );
    db.collection::<User>(USERS)
        .update_one(doc! { "_id": target_user_id }, pipeline)
        .await?;

    db.collection::<User>(USERS)
        .find_one(doc! { "_id": target_user_id })
        .await?
        .ok_or_else(|| AppError::Internal("User disappeared after role update".to_string()))
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

    delete_user_cascade_internal(db, target_user_id).await
}

/// Delete the currently authenticated user and cascade-delete all related documents.
///
/// This is intended for self-service account deletion flows (e.g. DELETE /users/me).
pub async fn delete_current_user_cascade(db: &mongodb::Database, user_id: &str) -> AppResult<()> {
    delete_user_cascade_internal(db, user_id).await
}

async fn delete_user_cascade_internal(
    db: &mongodb::Database,
    target_user_id: &str,
) -> AppResult<()> {
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

    // Phase 2: cascade delete user-owned documents keyed by user_id
    let user_filter = doc! { "user_id": target_user_id };

    let user_scoped_collections = [
        SESSIONS,
        REFRESH_TOKENS,
        API_KEYS,
        USER_SERVICE_CONNECTIONS,
        USER_PROVIDER_TOKENS,
        MFA_FACTORS,
        AUTHORIZATION_CODES,
        OAUTH_STATES,
        CONSENTS,
        MCP_SESSIONS,
        APPROVAL_REQUESTS,
        APPROVAL_GRANTS,
        SERVICE_APPROVAL_CONFIGS,
        NOTIFICATION_CHANNELS,
    ];

    for coll_name in user_scoped_collections {
        db.collection::<bson::Document>(coll_name)
            .delete_many(user_filter.clone())
            .await?;
    }

    // Delete OAuth clients created by the deleted user.
    db.collection::<bson::Document>(OAUTH_CLIENTS)
        .delete_many(doc! { "created_by": target_user_id })
        .await?;

    // Delete service accounts owned by the deleted user and their issued tokens.
    let service_account_owner_filter = doc! {
        "$or": [
            { "owner_user_id": target_user_id },
            { "owner_user_id": bson::Bson::Null, "created_by": target_user_id },
            { "owner_user_id": { "$exists": false }, "created_by": target_user_id },
        ]
    };

    let owned_service_account_ids: Vec<String> = db
        .collection::<bson::Document>(SERVICE_ACCOUNTS)
        .distinct("_id", service_account_owner_filter.clone())
        .await?
        .into_iter()
        .filter_map(|value| match value {
            bson::Bson::String(id) => Some(id),
            _ => None,
        })
        .collect();

    db.collection::<bson::Document>(SERVICE_ACCOUNTS)
        .delete_many(service_account_owner_filter)
        .await?;

    if !owned_service_account_ids.is_empty() {
        db.collection::<bson::Document>(SERVICE_ACCOUNT_TOKENS)
            .delete_many(doc! { "service_account_id": { "$in": owned_service_account_ids } })
            .await?;
    }

    // Phase 3: delete the user document itself
    let user_delete_result = db
        .collection::<User>(USERS)
        .delete_one(doc! { "_id": target_user_id })
        .await?;

    if user_delete_result.deleted_count != 1 {
        return Err(AppError::Internal(
            "Failed to delete user record".to_string(),
        ));
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::role::{
        COLLECTION_NAME as ROLES, PLATFORM_ADMIN_ROLE_SLUG, PLATFORM_OPERATOR_ROLE_SLUG, Role,
    };
    use crate::test_utils::*;

    async fn seed_platform_roles(db: &mongodb::Database) -> (String, String) {
        let now = Utc::now();
        let admin_role = Role {
            id: Uuid::new_v4().to_string(),
            name: "Admin".to_string(),
            slug: PLATFORM_ADMIN_ROLE_SLUG.to_string(),
            description: None,
            permissions: vec!["*".to_string()],
            is_default: false,
            is_system: true,
            client_id: None,
            created_at: now,
            updated_at: now,
        };
        let operator_role = Role {
            id: Uuid::new_v4().to_string(),
            name: "Operator".to_string(),
            slug: PLATFORM_OPERATOR_ROLE_SLUG.to_string(),
            description: None,
            permissions: vec![],
            is_default: false,
            is_system: true,
            client_id: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<Role>(ROLES)
            .insert_one(&admin_role)
            .await
            .unwrap();
        db.collection::<Role>(ROLES)
            .insert_one(&operator_role)
            .await
            .unwrap();
        (admin_role.id.clone(), operator_role.id.clone())
    }

    async fn seed_person(db: &mongodb::Database, user_id: &str, email: &str) {
        let now = Utc::now();
        let user = User {
            id: user_id.to_string(),
            email: email.to_string(),
            password_hash: Some(crate::crypto::password::hash_password("password123").unwrap()),
            display_name: Some("Test".to_string()),
            slug: None,
            avatar_url: None,
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            is_operator: false,
            role_ids: vec![],
            group_ids: vec![],
            invite_code_id: None,
            mfa_enabled: false,
            social_provider: None,
            social_provider_id: None,
            user_type: crate::models::user::UserType::Person,
            primary_org_id: None,
            created_at: now,
            updated_at: now,
            last_login_at: None,
            profile_config: Default::default(),
        };
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_create_user_basic() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        let user = create_user(
            &db,
            "newuser@example.com",
            "strongpass123",
            Some("Alice"),
            "user",
        )
        .await
        .unwrap();

        assert_eq!(user.email, "newuser@example.com");
        assert_eq!(user.display_name.as_deref(), Some("Alice"));
        assert!(user.email_verified);
        assert!(user.is_active);
        assert!(!user.is_admin);
        assert!(user.password_hash.is_some());
    }

    #[tokio::test]
    async fn test_create_user_as_admin() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        let user = create_user(&db, "admin@example.com", "strongpass123", None, "admin")
            .await
            .unwrap();

        assert!(user.is_admin);
        assert!(!user.is_operator);
    }

    #[tokio::test]
    async fn test_create_user_as_operator() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        let user = create_user(
            &db,
            "operator@example.com",
            "strongpass123",
            None,
            "operator",
        )
        .await
        .unwrap();

        assert!(!user.is_admin);
        assert!(user.is_operator);
    }

    #[tokio::test]
    async fn test_create_user_rejects_invalid_email() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        let result = create_user(&db, "notanemail", "strongpass123", None, "user").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_user_rejects_short_password() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        let result = create_user(&db, "user@example.com", "short", None, "user").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_user_rejects_invalid_role() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        let result =
            create_user(&db, "user@example.com", "strongpass123", None, "superadmin").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_user_rejects_duplicate_email() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        create_user(&db, "dup@example.com", "strongpass123", None, "user")
            .await
            .unwrap();

        let result = create_user(&db, "DUP@EXAMPLE.COM", "strongpass123", None, "user").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_user_display_name() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "existing@example.com").await;

        let updated = update_user(&db, &user_id, Some("New Name"), None, None)
            .await
            .unwrap();
        assert_eq!(updated.display_name.as_deref(), Some("New Name"));
    }

    #[tokio::test]
    async fn test_update_user_email() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "oldemail@example.com").await;

        let updated = update_user(&db, &user_id, None, Some("newemail@example.com"), None)
            .await
            .unwrap();
        assert_eq!(updated.email, "newemail@example.com");
    }

    #[tokio::test]
    async fn test_update_user_rejects_invalid_email() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "valid@example.com").await;

        let result = update_user(&db, &user_id, None, Some("bad"), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_user_not_found() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let result = update_user(&db, &Uuid::new_v4().to_string(), Some("Name"), None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_platform_role_to_admin() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;
        let admin_id = Uuid::new_v4().to_string();
        let target_id = Uuid::new_v4().to_string();
        seed_person(&db, &admin_id, "admin-actor@example.com").await;
        seed_person(&db, &target_id, "target@example.com").await;

        let updated = set_platform_role(&db, &admin_id, &target_id, "admin")
            .await
            .unwrap();
        assert!(updated.is_admin);
    }

    #[tokio::test]
    async fn test_set_platform_role_self_protection() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "selfchange@example.com").await;

        let result = set_platform_role(&db, &user_id, &user_id, "admin").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_user_active_disables_and_revokes() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let admin_id = Uuid::new_v4().to_string();
        let target_id = Uuid::new_v4().to_string();
        seed_person(&db, &admin_id, "admin-disable@example.com").await;
        seed_person(&db, &target_id, "target-disable@example.com").await;

        let session = crate::models::session::Session {
            id: Uuid::new_v4().to_string(),
            user_id: target_id.clone(),
            token_hash: "hash".to_string(),
            ip_address: None,
            user_agent: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            revoked: false,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        };
        db.collection::<crate::models::session::Session>(SESSIONS)
            .insert_one(&session)
            .await
            .unwrap();

        set_user_active(&db, &admin_id, &target_id, false)
            .await
            .unwrap();

        let user = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &target_id })
            .await
            .unwrap()
            .unwrap();
        assert!(!user.is_active);

        let sess = db
            .collection::<crate::models::session::Session>(SESSIONS)
            .find_one(doc! { "_id": &session.id })
            .await
            .unwrap()
            .unwrap();
        assert!(sess.revoked);
    }

    #[tokio::test]
    async fn test_set_user_active_self_protection() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "self-disable@example.com").await;

        let result = set_user_active(&db, &user_id, &user_id, false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_user_cascade() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let admin_id = Uuid::new_v4().to_string();
        let target_id = Uuid::new_v4().to_string();
        seed_person(&db, &admin_id, "admin-del@example.com").await;
        seed_person(&db, &target_id, "target-del@example.com").await;

        let session = crate::models::session::Session {
            id: Uuid::new_v4().to_string(),
            user_id: target_id.clone(),
            token_hash: "hash".to_string(),
            ip_address: None,
            user_agent: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            revoked: false,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        };
        db.collection::<crate::models::session::Session>(SESSIONS)
            .insert_one(&session)
            .await
            .unwrap();

        delete_user_cascade(&db, &admin_id, &target_id)
            .await
            .unwrap();

        let user = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &target_id })
            .await
            .unwrap();
        assert!(user.is_none());

        let sess = db
            .collection::<crate::models::session::Session>(SESSIONS)
            .find_one(doc! { "_id": &session.id })
            .await
            .unwrap();
        assert!(sess.is_none());
    }

    #[tokio::test]
    async fn test_delete_user_cascade_self_protection() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "self-del@example.com").await;

        let result = delete_user_cascade(&db, &user_id, &user_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_user_cascade_not_found() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let admin_id = Uuid::new_v4().to_string();
        seed_person(&db, &admin_id, "admin-nf@example.com").await;

        let result = delete_user_cascade(&db, &admin_id, &Uuid::new_v4().to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_force_password_reset_for_password_user() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "reset-target@example.com").await;

        let session = crate::models::session::Session {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.clone(),
            token_hash: "hash".to_string(),
            ip_address: None,
            user_agent: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            revoked: false,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        };
        db.collection::<crate::models::session::Session>(SESSIONS)
            .insert_one(&session)
            .await
            .unwrap();

        force_password_reset(&db, &user_id).await.unwrap();

        let sess = db
            .collection::<crate::models::session::Session>(SESSIONS)
            .find_one(doc! { "_id": &session.id })
            .await
            .unwrap()
            .unwrap();
        assert!(sess.revoked);
    }

    #[tokio::test]
    async fn test_force_password_reset_rejects_social_only() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let user = User {
            id: user_id.clone(),
            email: "social@example.com".to_string(),
            password_hash: None,
            display_name: None,
            slug: None,
            avatar_url: None,
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            is_operator: false,
            role_ids: vec![],
            group_ids: vec![],
            invite_code_id: None,
            mfa_enabled: false,
            social_provider: Some("google".to_string()),
            social_provider_id: Some("google-123".to_string()),
            user_type: crate::models::user::UserType::Person,
            primary_org_id: None,
            created_at: now,
            updated_at: now,
            last_login_at: None,
            profile_config: Default::default(),
        };
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let result = force_password_reset(&db, &user_id).await;
        assert!(result.is_err());
    }
}
