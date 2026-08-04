use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::password;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, PlatformRole, User, UserType};
use crate::services::role_service;

/// Maximum password length to prevent Argon2 DoS via extremely long passwords.
const MAX_PASSWORD_LENGTH: usize = 128;

/// Reject any auth flow that lands on an org-type user.
///
/// Org users (`user_type = Org`) exist purely as the owner record for shared
/// resources. They have no password, no MFA, no email verification flow,
/// and must never be allowed to log in. This guard is called from every
/// path that loads a user during an authentication operation.
pub fn ensure_person_user(user: &User) -> AppResult<()> {
    match user.user_type {
        UserType::Person => Ok(()),
        UserType::Org => Err(AppError::OrgCannotAuthenticate),
    }
}

/// Result of a successful registration.
pub struct RegisterResult {
    pub user_id: String,
    /// Used in debug builds to log the verification token.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub email_verification_token: String,
    /// `true` when a new user was actually inserted; `false` when the email
    /// already existed and a fake success was returned for email-enumeration
    /// protection. Callers that hold a reserved invite code must use this to
    /// know whether to record or release the reservation.
    pub actually_created: bool,
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
    invite_code_id: Option<&str>,
    auto_verify_email: bool,
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

    // Check for existing person user - return fake success to prevent email
    // enumeration. We deliberately scope to user_type=person so that an org
    // happening to share an email does not block person registration.
    let existing = db
        .collection::<User>(USERS)
        .find_one(doc! {
            "email": email.to_lowercase(),
            "user_type": "person",
        })
        .await?;

    if existing.is_some() {
        // Return a fake success result to prevent email enumeration.
        // In production, send an email to the existing user informing them
        // that someone attempted to register with their address.
        tracing::warn!(email = %email, "Registration attempt for existing email");
        return Ok(RegisterResult {
            user_id: Uuid::new_v4().to_string(), // Fake ID, not stored
            email_verification_token: generate_random_token(), // Fake token
            actually_created: false,
        });
    }

    let password_hash = password::hash_password(password_raw)?;
    let verification_token = generate_random_token();
    // Store the hash of the verification token, not the raw token
    let verification_token_hash = hash_token(&verification_token);
    let now = Utc::now();
    let user_id = Uuid::new_v4().to_string();

    // Auto-assign default roles to new users
    let default_role_ids = role_service::get_default_role_ids(db).await?;

    let new_user = User {
        id: user_id.clone(),
        email: email.to_lowercase(),
        password_hash: Some(password_hash),
        display_name: display_name.map(String::from),
        slug: None,
        avatar_url: None,
        email_verified: auto_verify_email,
        email_verification_token: if auto_verify_email {
            None
        } else {
            Some(verification_token_hash)
        },
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin: false,
        is_operator: false,
        role_ids: default_role_ids,
        group_ids: vec![],
        invite_code_id: invite_code_id.map(String::from),
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

    tracing::info!(user_id = %user_id, "User registered");

    Ok(RegisterResult {
        user_id,
        email_verification_token: verification_token,
        actually_created: true,
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
        .find_one(doc! {
            "email": email.to_lowercase(),
            "user_type": "person",
        })
        .await?
        .ok_or_else(|| AppError::AuthenticationFailed("Invalid email or password".to_string()))?;

    // Belt-and-suspenders: the partial-unique email index already excludes
    // org users, but we double-check here so any code path that bypassed
    // the index filter still gets blocked.
    ensure_person_user(&user)?;

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

    ensure_person_user(&user)?;

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
        .find_one(doc! {
            "email": email.to_lowercase(),
            "user_type": "person",
        })
        .await?;

    // Always return Ok to prevent email enumeration
    let Some(user) = user else {
        return Ok(None);
    };

    // The query already filters to person, but be defensive against
    // hand-crafted documents that bypass the index filter.
    if user.user_type.is_org() {
        return Ok(None);
    }

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

    ensure_person_user(&user)?;

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
/// Assigns the platform admin RBAC role, mirrors `is_admin = true`, and sets
/// `email_verified = true` on the user.
/// Returns the user ID on success.
pub async fn promote_user_to_admin(db: &mongodb::Database, email: &str) -> AppResult<String> {
    let normalized = email.to_lowercase();

    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {
            "email": &normalized,
            "user_type": "person",
        })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No user found with email: {}", normalized)))?;

    ensure_person_user(&user)?;

    if role_service::resolve_platform_role(db, &user)
        .await?
        .is_admin()
    {
        return Err(AppError::Conflict(format!(
            "User {} is already an admin",
            normalized
        )));
    }

    let platform_role_ids = role_service::get_platform_role_ids(db).await?;
    let mut pipeline = role_service::set_platform_role_update(
        PlatformRole::Admin,
        &platform_role_ids,
        bson::DateTime::from_chrono(Utc::now()),
    );
    pipeline.push(doc! { "$set": { "email_verified": true } });
    db.collection::<User>(USERS)
        .update_one(doc! { "_id": &user.id }, pipeline)
        .await?;

    tracing::info!(user_id = %user.id, email = %normalized, "User promoted to admin");

    Ok(user.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_person() -> User {
        let now = Utc::now();
        User {
            id: Uuid::new_v4().to_string(),
            email: "alice@example.com".to_string(),
            password_hash: Some("$argon2id$hash".to_string()),
            display_name: Some("Alice".to_string()),
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
            user_type: UserType::Person,
            primary_org_id: None,
            created_at: now,
            updated_at: now,
            last_login_at: None,
            profile_config: Default::default(),
        }
    }

    #[test]
    fn ensure_person_user_allows_person() {
        let user = make_person();
        assert!(ensure_person_user(&user).is_ok());
    }

    #[test]
    fn ensure_person_user_rejects_org() {
        let mut user = make_person();
        user.user_type = UserType::Org;
        let err = ensure_person_user(&user).expect_err("must reject org");
        assert!(matches!(err, AppError::OrgCannotAuthenticate));
    }

    // Regression tests for issue #424 — see docs/ORG_MODEL.md "Public vs
    // authenticated surfaces". Public auth endpoints (login, forgot-password)
    // must make org-owner emails indistinguishable from unknown emails so
    // unauthenticated callers cannot enumerate which emails belong to orgs.
    // `OrgCannotAuthenticate` is intentionally NOT surfaced on these paths.

    #[tokio::test]
    async fn authenticate_user_returns_generic_failure_for_org_email() {
        let Some(db) = crate::test_utils::connect_test_database("auth_org").await else {
            eprintln!("skipping auth_service integration test: no local MongoDB available");
            return;
        };

        let mut org = make_person();
        org.email = "org-owner@example.com".to_string();
        org.user_type = UserType::Org;
        db.collection::<User>(USERS).insert_one(&org).await.unwrap();

        let err = authenticate_user(&db, &org.email, "irrelevant-password")
            .await
            .expect_err("org email must not authenticate");

        match err {
            AppError::AuthenticationFailed(_) => {}
            other => panic!("expected AuthenticationFailed for anti-enumeration, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn initiate_password_reset_returns_none_for_org_email_and_persists_no_token() {
        let Some(db) = crate::test_utils::connect_test_database("auth_org").await else {
            eprintln!("skipping auth_service integration test: no local MongoDB available");
            return;
        };

        let mut org = make_person();
        org.email = "org-owner@example.com".to_string();
        org.user_type = UserType::Org;
        db.collection::<User>(USERS).insert_one(&org).await.unwrap();

        let result = initiate_password_reset(&db, &org.email)
            .await
            .expect("forgot-password must not error for org email");
        assert!(result.is_none(), "no reset token should be issued for org");

        let stored = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &org.id })
            .await
            .unwrap()
            .expect("org user still exists");
        assert!(
            stored.password_reset_token.is_none(),
            "no reset token should be persisted on the org user"
        );
        assert!(stored.password_reset_expires_at.is_none());
    }

    #[tokio::test]
    async fn register_user_happy_path() {
        let Some(db) = crate::test_utils::connect_test_database("auth_reg_ok").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let result = register_user(
            &db,
            "new@example.com",
            "password123",
            Some("New"),
            None,
            false,
        )
        .await
        .expect("register");

        assert!(result.actually_created);
        assert!(!result.user_id.is_empty());
        assert!(!result.email_verification_token.is_empty());

        let stored = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &result.user_id })
            .await
            .unwrap()
            .expect("user in db");
        assert_eq!(stored.email, "new@example.com");
        assert!(stored.password_hash.is_some());
        assert!(!stored.email_verified);
        assert!(stored.email_verification_token.is_some());
    }

    #[tokio::test]
    async fn register_user_short_password_error() {
        let Some(db) = crate::test_utils::connect_test_database("auth_reg_short").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        match register_user(&db, "short@example.com", "1234567", None, None, false).await {
            Err(AppError::ValidationError(_)) => {}
            Err(other) => panic!("expected ValidationError, got: {other:?}"),
            Ok(_) => panic!("expected error for short password"),
        }
    }

    #[tokio::test]
    async fn register_user_long_password_error() {
        let Some(db) = crate::test_utils::connect_test_database("auth_reg_long").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let long_pw = "a".repeat(129);
        match register_user(&db, "long@example.com", &long_pw, None, None, false).await {
            Err(AppError::ValidationError(_)) => {}
            Err(other) => panic!("expected ValidationError, got: {other:?}"),
            Ok(_) => panic!("expected error for long password"),
        }
    }

    #[tokio::test]
    async fn register_user_duplicate_email_returns_fake_success() {
        let Some(db) = crate::test_utils::connect_test_database("auth_reg_dup").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(&db, "dup@example.com", "password123", None, None, false)
            .await
            .expect("first register");

        let result = register_user(&db, "dup@example.com", "password456", None, None, false)
            .await
            .expect("duplicate should return fake success");

        assert!(!result.actually_created);
    }

    #[tokio::test]
    async fn register_user_auto_verify_sets_email_verified() {
        let Some(db) = crate::test_utils::connect_test_database("auth_reg_auto").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let result = register_user(&db, "auto@example.com", "password123", None, None, true)
            .await
            .expect("register with auto-verify");

        let stored = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &result.user_id })
            .await
            .unwrap()
            .expect("user in db");
        assert!(stored.email_verified);
        assert!(stored.email_verification_token.is_none());
    }

    #[tokio::test]
    async fn authenticate_user_wrong_password() {
        let Some(db) = crate::test_utils::connect_test_database("auth_pw_wrong").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(&db, "user@example.com", "correct-pw-1", None, None, true)
            .await
            .expect("register");

        let err = authenticate_user(&db, "user@example.com", "wrong-password")
            .await
            .expect_err("wrong password");
        assert!(matches!(err, AppError::AuthenticationFailed(_)));
    }

    #[tokio::test]
    async fn authenticate_user_inactive_user() {
        let Some(db) = crate::test_utils::connect_test_database("auth_inactive").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let result = register_user(&db, "inactive@example.com", "password123", None, None, true)
            .await
            .expect("register");

        db.collection::<User>(USERS)
            .update_one(
                doc! { "_id": &result.user_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let err = authenticate_user(&db, "inactive@example.com", "password123")
            .await
            .expect_err("inactive user");
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn authenticate_user_social_only_no_password() {
        let Some(db) = crate::test_utils::connect_test_database("auth_social").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let mut user = make_person();
        user.email = "social@example.com".to_string();
        user.password_hash = None;
        user.social_provider = Some("google".to_string());
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();

        let err = authenticate_user(&db, "social@example.com", "anything")
            .await
            .expect_err("social-only user");
        assert!(matches!(err, AppError::AuthenticationFailed(_)));
    }

    #[tokio::test]
    async fn authenticate_user_happy_path() {
        let Some(db) = crate::test_utils::connect_test_database("auth_login_ok").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(&db, "good@example.com", "password123", None, None, true)
            .await
            .expect("register");

        let user = authenticate_user(&db, "good@example.com", "password123")
            .await
            .expect("login");
        assert_eq!(user.email, "good@example.com");
    }

    #[tokio::test]
    async fn authenticate_user_nonexistent_email() {
        let Some(db) = crate::test_utils::connect_test_database("auth_nouser").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = authenticate_user(&db, "nobody@example.com", "password123")
            .await
            .expect_err("nonexistent email");
        assert!(matches!(err, AppError::AuthenticationFailed(_)));
    }

    #[tokio::test]
    async fn authenticate_user_over_max_length_password() {
        let Some(db) = crate::test_utils::connect_test_database("auth_maxpw").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let long_pw = "a".repeat(129);
        let err = authenticate_user(&db, "any@example.com", &long_pw)
            .await
            .expect_err("over max length");
        assert!(matches!(err, AppError::AuthenticationFailed(_)));
    }

    #[tokio::test]
    async fn verify_email_invalid_token() {
        let Some(db) = crate::test_utils::connect_test_database("auth_verify_bad").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = verify_email(&db, "invalid-token-value")
            .await
            .expect_err("invalid token");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn verify_email_already_verified() {
        let Some(db) = crate::test_utils::connect_test_database("auth_verify_dup").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let reg = register_user(&db, "verify@example.com", "password123", None, None, false)
            .await
            .expect("register");

        verify_email(&db, &reg.email_verification_token)
            .await
            .expect("first verify");

        let err = verify_email(&db, &reg.email_verification_token)
            .await
            .expect_err("already verified");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn verify_email_happy_path() {
        let Some(db) = crate::test_utils::connect_test_database("auth_verify_ok").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let reg = register_user(&db, "hvp@example.com", "password123", None, None, false)
            .await
            .expect("register");
        assert!(reg.actually_created);

        let user_id = verify_email(&db, &reg.email_verification_token)
            .await
            .expect("verify");
        assert_eq!(user_id, reg.user_id);

        let stored = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &reg.user_id })
            .await
            .unwrap()
            .expect("user");
        assert!(stored.email_verified);
        assert!(stored.email_verification_token.is_none());
    }

    #[tokio::test]
    async fn initiate_password_reset_unknown_email_returns_none() {
        let Some(db) = crate::test_utils::connect_test_database("auth_reset_none").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let result = initiate_password_reset(&db, "unknown@example.com")
            .await
            .expect("should not error");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn initiate_password_reset_happy_path() {
        let Some(db) = crate::test_utils::connect_test_database("auth_reset_ok").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(&db, "reset@example.com", "password123", None, None, true)
            .await
            .expect("register");

        let token = initiate_password_reset(&db, "reset@example.com")
            .await
            .expect("initiate")
            .expect("should return token");
        assert!(!token.is_empty());
    }

    #[tokio::test]
    async fn reset_password_short_password() {
        let Some(db) = crate::test_utils::connect_test_database("auth_rp_short").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = reset_password(&db, "some-token", "short")
            .await
            .expect_err("short password");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn reset_password_long_password() {
        let Some(db) = crate::test_utils::connect_test_database("auth_rp_long").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let long_pw = "a".repeat(129);
        let err = reset_password(&db, "some-token", &long_pw)
            .await
            .expect_err("long password");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn reset_password_invalid_token() {
        let Some(db) = crate::test_utils::connect_test_database("auth_rp_bad").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = reset_password(&db, "invalid-token-value", "newpassword1")
            .await
            .expect_err("invalid token");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn reset_password_expired_token() {
        let Some(db) = crate::test_utils::connect_test_database("auth_rp_exp").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let reg = register_user(&db, "expired@example.com", "password123", None, None, true)
            .await
            .expect("register");

        let token = initiate_password_reset(&db, "expired@example.com")
            .await
            .expect("initiate")
            .expect("token");

        let past = Utc::now() - chrono::Duration::hours(2);
        db.collection::<User>(USERS)
            .update_one(
                doc! { "_id": &reg.user_id },
                doc! { "$set": {
                    "password_reset_expires_at": bson::DateTime::from_chrono(past),
                }},
            )
            .await
            .unwrap();

        let err = reset_password(&db, &token, "newpassword1")
            .await
            .expect_err("expired token");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn reset_password_happy_path() {
        let Some(db) = crate::test_utils::connect_test_database("auth_rp_ok").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(&db, "rp-ok@example.com", "oldpassword1", None, None, true)
            .await
            .expect("register");

        let token = initiate_password_reset(&db, "rp-ok@example.com")
            .await
            .expect("initiate")
            .expect("token");

        reset_password(&db, &token, "newpassword1")
            .await
            .expect("reset");

        let err = authenticate_user(&db, "rp-ok@example.com", "oldpassword1")
            .await
            .expect_err("old password should fail");
        assert!(matches!(err, AppError::AuthenticationFailed(_)));

        let user = authenticate_user(&db, "rp-ok@example.com", "newpassword1")
            .await
            .expect("new password should work");
        assert_eq!(user.email, "rp-ok@example.com");
    }
}
