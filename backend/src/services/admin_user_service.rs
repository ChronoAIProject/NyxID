use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::password;
use crate::errors::{AppError, AppResult};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};
use crate::models::user::{COLLECTION_NAME as USERS, PlatformRole, User};
use crate::services::{api_key_mutation_service as key_mutations, role_service};

/// Maximum password length to prevent Argon2 DoS via extremely long passwords.
const MAX_PASSWORD_LENGTH: usize = 128;

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
        let name = name.trim();
        if name.len() > 200 {
            return Err(AppError::ValidationError(
                "Display name must be 200 characters or less".to_string(),
            ));
        }
        set_doc.insert(
            "display_name",
            if name.is_empty() {
                bson::Bson::Null
            } else {
                bson::Bson::String(name.to_string())
            },
        );
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
        let url = url.trim();
        if url.len() > 2048 {
            return Err(AppError::ValidationError(
                "Avatar URL must be 2048 characters or less".to_string(),
            ));
        }
        if !url.is_empty() && !url.starts_with("https://") {
            return Err(AppError::ValidationError(
                "Avatar URL must use https:// scheme".to_string(),
            ));
        }
        set_doc.insert(
            "avatar_url",
            if url.is_empty() {
                bson::Bson::Null
            } else {
                bson::Bson::String(url.to_string())
            },
        );
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

    let target = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": target_user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    if is_active && !target.is_active {
        super::platform_personal_owner_lifecycle::retire(db, target_user_id, false).await?;
    }
    apply_user_security_state(db, target_user_id, is_active, false, target.is_active).await?;

    if !is_active {
        super::platform_personal_owner_lifecycle::retire(db, target_user_id, false).await?;
    }

    Ok(())
}

/// Account state, personal-retirement markers and Machine source-epoch/outbox
/// fences commit together. The epoch remains after re-enable, so an unswept old
/// grant cannot revive.
async fn apply_user_security_state(
    db: &mongodb::Database,
    user_id: &str,
    active: bool,
    deleting: bool,
    expected_active: bool,
) -> AppResult<()> {
    apply_user_security_state_with_deletion_policy(
        db,
        user_id,
        active,
        deleting,
        None,
        Some(expected_active),
    )
    .await
}

async fn apply_user_security_state_with_deletion_policy(
    db: &mongodb::Database,
    user_id: &str,
    active: bool,
    deleting: bool,
    owner_deletion_policy: Option<&str>,
    expected_active: Option<bool>,
) -> AppResult<()> {
    if let Some(policy) = owner_deletion_policy {
        validate_account_deletion_policy(policy)?;
        if active || !deleting {
            return Err(AppError::ValidationError(
                "Owner deletion policy requires account deletion".into(),
            ));
        }
    }
    let db = db.clone();
    let user_id = user_id.to_owned();
    let owner_deletion_policy = owner_deletion_policy.map(str::to_owned);
    db.client()
        .start_session()
        .await?
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result = apply_user_security_state_in_session(
                &db,
                session,
                &user_id,
                active,
                deleting,
                owner_deletion_policy.as_deref(),
                expected_active,
            )
            .await;
            key_mutations::transaction_result(result)
        })
        .await
        .map_err(key_mutations::map_transaction_error)
}

/// Shared atomic account fence. Personal-erasure acceptance owns its transaction
/// and persists its intent beside this state transition before any cleanup.
pub(crate) async fn apply_user_security_state_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    user_id: &str,
    active: bool,
    deleting: bool,
    owner_deletion_policy: Option<&str>,
    expected_active: Option<bool>,
) -> AppResult<()> {
    if let Some(policy) = owner_deletion_policy {
        validate_account_deletion_policy(policy)?;
        if active || !deleting {
            return Err(AppError::ValidationError(
                "Owner deletion policy requires account deletion".into(),
            ));
        }
    }
    let owner_deletion_policy = owner_deletion_policy.map(str::to_owned);
    let current = db
        .collection::<bson::Document>(USERS)
        .find_one(doc! {"_id":&user_id})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    if active
        && current
            .get("deletion_requested_at")
            .is_some_and(|value| !matches!(value, bson::Bson::Null))
    {
        return Err(AppError::Conflict(
            "Account deletion has already been accepted".into(),
        ));
    }
    if !active {
        super::org_membership_authority::fence_person_authority_loss(
            db, user_id, session, deleting,
        )
        .await?;
    }
    let mut filter = doc! {"_id":&user_id};
    if let Some(expected_active) = expected_active {
        filter.insert("is_active", expected_active);
    }
    if active {
        filter.insert("platform_personal_retirement_pending", doc! {"$ne":true});
        filter.insert("platform_personal_deleting", doc! {"$ne":true});
    }
    let now = bson::DateTime::now();
    let mut set = doc! {"is_active":active,"updated_at":now};
    if !active {
        set.insert("platform_personal_retirement_pending", true);
    }
    if deleting {
        set.insert("platform_personal_deleting", true);
        let requested = current
            .get("deletion_requested_at")
            .filter(|value| !matches!(value, bson::Bson::Null))
            .cloned()
            .unwrap_or(bson::Bson::DateTime(now));
        set.insert("deletion_requested_at", requested);
    }
    let changed = db
        .collection::<bson::Document>(USERS)
        .update_one(
            filter,
            doc! {"$set":set,"$inc":{"platform_personal_effect_revision":1_i64}},
        )
        .session(&mut *session)
        .await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict(
            "Account state changed; refresh before retrying".into(),
        ));
    }
    if let Some(policy) = &owner_deletion_policy {
        // Same transaction as the account/source fence. These
        // retained lifecycle records outlive the user cascade;
        // replay preserves the original consent and timestamp.
        let accepted_at = current
            .get_datetime("deletion_requested_at")
            .copied()
            .unwrap_or(now);
        let basis = crate::models::remote_machine::MachineDeletionBasis {
            owner_id: user_id.to_owned(),
            source: crate::models::remote_machine::MachineDeletionSource::AccountOwner,
            policy_version: policy.clone(),
            accepted_at: accepted_at.to_chrono(),
        };
        let basis = bson::to_bson(&basis)
            .map_err(|_| AppError::Internal("Machine deletion basis encoding failed".into()))?;
        db.collection::<bson::Document>(crate::models::remote_machine::COLLECTION_NAME)
            .update_many(
                doc! {"owner_id":&user_id,"deletion_basis":bson::Bson::Null},
                doc! {"$set":{"deletion_basis":basis}},
            )
            .session(&mut *session)
            .await?;
    }
    if !active {
        super::machine_external_grant_service::user_security_changed_in_session(
            db,
            user_id,
            if deleting {
                "user_deletion_requested"
            } else {
                "user_disabled"
            },
            session,
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

/// Explicit owner acknowledgement required before accepting Machine erasure.
pub const ACCOUNT_DELETION_POLICY_VERSION: &str = "machine-data-provider-revocation-v2";
pub const ACCOUNT_DELETION_POLICY_HEADER: &str = "x-nyxid-account-deletion-policy";

pub fn validate_account_deletion_policy(policy: &str) -> AppResult<()> {
    if policy != ACCOUNT_DELETION_POLICY_VERSION {
        return Err(AppError::ValidationError(
            "Confirm the current account deletion policy, including provider revocation and all managed Machine data"
                .into(),
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
        .find(super::session_authority::authenticated(
            doc! { "user_id": user_id },
        ))
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
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let count =
        match super::session_authority::revoke_all_in_transaction(db, user_id, &mut transaction)
            .await
        {
            Ok(count) => count,
            Err(error) => {
                transaction.abort_transaction().await?;
                return Err(error);
            }
        };
    super::transaction_service::commit(&mut transaction).await?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runtime test credential; never a fixed password.
    fn test_password() -> String {
        crate::crypto::token::generate_random_token()
    }
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
            password_hash: Some(crate::crypto::password::hash_password(&test_password()).unwrap()),
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
            &test_password(),
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

        let user = create_user(&db, "admin@example.com", &test_password(), None, "admin")
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
            &test_password(),
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

        let result = create_user(&db, "notanemail", &test_password(), None, "user").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_user_rejects_short_password() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        let result =
            create_user(&db, "user@example.com", &test_password()[..5], None, "user").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_user_rejects_invalid_role() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        let result = create_user(
            &db,
            "user@example.com",
            &test_password(),
            None,
            "superadmin",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_user_rejects_duplicate_email() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        seed_platform_roles(&db).await;

        create_user(&db, "dup@example.com", &test_password(), None, "user")
            .await
            .unwrap();

        let result = create_user(&db, "DUP@EXAMPLE.COM", &test_password(), None, "user").await;
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
    async fn machine_source_disable_fences_grant_and_outbox_before_reenable() {
        let db = connect_transaction_test_database("machine_user_disable").await;
        let user = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let grant = Uuid::new_v4().to_string();
        let grants = crate::models::machine_external_grant::COLLECTION_NAME;
        db.collection::<bson::Document>(grants)
            .insert_one(doc! {
                "_id":&grant,"user_id":&user,"revision":1_i64,"admission_fence":0_i64,
                "status":"active","user_security_revision":0_i64,
            })
            .await
            .unwrap();
        set_user_active(&db, &Uuid::new_v4().to_string(), &user, false)
            .await
            .unwrap();
        set_user_active(&db, &Uuid::new_v4().to_string(), &user, true)
            .await
            .unwrap();
        let source = db
            .collection::<bson::Document>(USERS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert!(source.get_bool("is_active").unwrap());
        assert_eq!(
            source
                .get_i64("machine_external_security_revision")
                .unwrap(),
            1
        );
        let revoked = db
            .collection::<bson::Document>(grants)
            .find_one(doc! {"_id":&grant})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(revoked.get_str("status").unwrap(), "revoked");
        assert_eq!(revoked.get_i64("revision").unwrap(), 2);
        assert_eq!(
            db.collection::<bson::Document>(crate::models::machine_access::OUTBOX)
                .count_documents(
                    doc! {"kind":"external","reason":"user_disabled","selector.grant_id":&grant}
                )
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn machine_source_hook_failure_aborts_account_deletion_marker() {
        let db = connect_transaction_test_database("machine_user_rollback").await;
        let user = Uuid::new_v4().to_string();
        let machine_id = Uuid::new_v4().to_string();
        let machines = crate::models::remote_machine::COLLECTION_NAME;
        db.collection::<bson::Document>(machines)
            .insert_one(doc! {"_id":&machine_id,"owner_id":&user,"deletion_basis":bson::Bson::Null})
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user, crate::models::user::UserType::Person))
            .await
            .unwrap();
        db.collection::<bson::Document>(crate::models::machine_external_grant::COLLECTION_NAME)
            .insert_one(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&user,
                "revision":i64::MAX,"admission_fence":0_i64,"status":"active","user_security_revision":0_i64})
            .await.unwrap();
        assert!(
            apply_user_security_state_with_deletion_policy(
                &db,
                &user,
                false,
                true,
                Some(ACCOUNT_DELETION_POLICY_VERSION),
                None,
            )
            .await
            .is_err()
        );
        let unchanged = db
            .collection::<bson::Document>(USERS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert!(unchanged.get_bool("is_active").unwrap());
        assert!(!unchanged.contains_key("deletion_requested_at"));
        assert!(!unchanged.contains_key("machine_external_security_revision"));
        for field in [
            "platform_personal_retirement_pending",
            "platform_personal_deleting",
            "platform_personal_effect_revision",
        ] {
            assert!(
                !unchanged.contains_key(field),
                "aborted source fence must also abort {field}"
            );
        }
        let machine = db
            .collection::<bson::Document>(machines)
            .find_one(doc! {"_id":&machine_id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(machine.get("deletion_basis"), Some(bson::Bson::Null)),
            "aborted source fence must also abort erasure authority"
        );
        assert_eq!(
            db.collection::<bson::Document>(crate::models::machine_access::OUTBOX)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn machine_account_deletion_marker_survives_interruption_and_rejects_reenable() {
        let db = connect_transaction_test_database("machine_user_marker").await;
        let user = Uuid::new_v4().to_string();
        let machine_id = Uuid::new_v4().to_string();
        let machines = crate::models::remote_machine::COLLECTION_NAME;
        db.collection::<bson::Document>(machines)
            .insert_one(doc! {"_id":&machine_id,"owner_id":&user,"deletion_basis":bson::Bson::Null})
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user, crate::models::user::UserType::Person))
            .await
            .unwrap();
        apply_user_security_state_with_deletion_policy(
            &db,
            &user,
            false,
            true,
            Some(ACCOUNT_DELETION_POLICY_VERSION),
            None,
        )
        .await
        .unwrap();
        let requested = db
            .collection::<bson::Document>(USERS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert!(requested.get_datetime("deletion_requested_at").is_ok());
        assert!(
            requested
                .get_bool("platform_personal_retirement_pending")
                .unwrap()
        );
        assert!(requested.get_bool("platform_personal_deleting").unwrap());
        assert_eq!(
            requested
                .get_i64("platform_personal_effect_revision")
                .unwrap(),
            1
        );
        let first_machine = db
            .collection::<bson::Document>(machines)
            .find_one(doc! {"_id":&machine_id})
            .await
            .unwrap()
            .unwrap();
        let first_basis = first_machine
            .get_document("deletion_basis")
            .unwrap()
            .clone();
        assert_eq!(first_basis.get_str("owner_id").unwrap(), user);
        assert_eq!(first_basis.get_str("source").unwrap(), "account_owner");
        assert_eq!(
            first_basis.get_str("policy_version").unwrap(),
            ACCOUNT_DELETION_POLICY_VERSION
        );
        assert_eq!(
            first_basis.get_datetime("accepted_at").unwrap(),
            requested.get_datetime("deletion_requested_at").unwrap()
        );
        assert!(
            set_user_active(&db, &Uuid::new_v4().to_string(), &user, true)
                .await
                .is_err()
        );
        apply_user_security_state_with_deletion_policy(
            &db,
            &user,
            false,
            true,
            Some(ACCOUNT_DELETION_POLICY_VERSION),
            None,
        )
        .await
        .unwrap();
        let replayed = db
            .collection::<bson::Document>(USERS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            replayed.get_datetime("deletion_requested_at").unwrap(),
            requested.get_datetime("deletion_requested_at").unwrap()
        );
        assert!(!replayed.get_bool("is_active").unwrap());
        db.collection::<bson::Document>(USERS)
            .delete_one(doc! {"_id":&user})
            .await
            .unwrap();
        let retained = db
            .collection::<bson::Document>(machines)
            .find_one(doc! {"_id":&machine_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retained.get_document("deletion_basis").unwrap(),
            &first_basis
        );
        db.drop().await.unwrap();
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
            purpose: Some(crate::models::session::SessionPurpose::Authenticated),
            mfa_method: None,
            mfa_assured: false,
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
        db.collection::<bson::Document>("assistant_message_feedback")
            .insert_one(doc! {"_id":"cascade-feedback", "user_id": &target_id, "message_id":"cascade-message", "vote":"up"})
            .await
            .unwrap();
        let api_key = crate::services::key_service::create_api_key(
            &db,
            &target_id,
            "disable-me",
            "read",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
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

        let disabled_key = db
            .collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! { "_id": &api_key.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!disabled_key.is_active);
        assert_eq!(disabled_key.state_version, 2);
        assert!(
            disabled_key
                .updated_at
                .is_some_and(|value| value >= api_key.updated_at)
        );
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
    async fn personal_erasure_rejects_org_without_mutation_through_service_and_admin() {
        use crate::models::user::UserType;
        use axum::extract::{Path, State};
        use axum::http::HeaderMap;
        use futures::TryStreamExt;

        async fn snapshot(
            db: &mongodb::Database,
        ) -> std::collections::BTreeMap<String, Vec<bson::Document>> {
            let mut result = std::collections::BTreeMap::new();
            for collection in db.list_collection_names().await.unwrap() {
                let rows = db
                    .collection::<bson::Document>(&collection)
                    .find(doc! {})
                    .sort(doc! {"_id":1})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                result.insert(collection, rows);
            }
            result
        }

        let db = connect_transaction_test_database("person_erasure_org_boundary").await;
        let (admin_role, _) = seed_platform_roles(&db).await;
        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let mut admin = test_user(&admin_id, UserType::Person);
        admin.role_ids.push(admin_role);
        db.collection::<User>(USERS)
            .insert_many([admin, test_user(&org_id, UserType::Org)])
            .await
            .unwrap();
        // Creator attribution does not make these organization-owned resources
        // eligible for personal erasure, even when a platform admin requests it.
        for collection in [
            crate::models::user_service::COLLECTION_NAME,
            crate::models::user_api_key::COLLECTION_NAME,
            crate::models::node::COLLECTION_NAME,
            crate::models::channel_adapter::BINDINGS,
        ] {
            db.collection::<bson::Document>(collection)
                .insert_one(doc! {
                    "_id":Uuid::new_v4().to_string(), "user_id":&org_id,
                    "created_by":&admin_id, "is_active":true, "enabled":true,
                    "revision":1, "credential":"unchanged-ciphertext-fixture"
                })
                .await
                .unwrap();
        }
        let state = test_app_state(db.clone());
        // Startup installs erasure indexes before requests. Include that empty
        // bookkeeping schema in the baseline so every request mutation remains
        // covered by the exact snapshot comparison below.
        crate::services::personal_erasure_service::ensure_indexes(&db)
            .await
            .unwrap();
        let before = snapshot(&db).await;
        assert!(matches!(
            crate::test_utils::accept_and_process_admin_erasure(&db, &admin_id, &org_id).await,
            Err(AppError::ValidationError(_))
        ));
        assert_eq!(snapshot(&db).await, before);
        assert!(matches!(
            crate::test_utils::accept_and_process_owner_erasure(
                &db,
                &org_id,
                ACCOUNT_DELETION_POLICY_VERSION
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        assert_eq!(snapshot(&db).await, before);
        assert!(matches!(
            crate::handlers::admin::delete_user(
                State(state),
                test_auth_user(&admin_id),
                HeaderMap::new(),
                Path(org_id)
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        assert_eq!(snapshot(&db).await, before);
    }

    #[tokio::test]
    async fn test_accepted_admin_erasure() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let admin_id = Uuid::new_v4().to_string();
        let target_id = Uuid::new_v4().to_string();
        seed_person(&db, &admin_id, "admin-del@example.com").await;
        seed_person(&db, &target_id, "target-del@example.com").await;

        let session = crate::models::session::Session {
            purpose: Some(crate::models::session::SessionPurpose::Authenticated),
            mfa_method: None,
            mfa_assured: false,
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

        db.collection::<bson::Document>("assistant_message_feedback")
            .insert_one(doc! {"_id":"cascade-feedback", "user_id": &target_id, "message_id":"cascade-message", "vote":"up", "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()})
            .await
            .unwrap();
        assert!(
            db.collection::<bson::Document>("assistant_message_feedback")
                .find_one(doc! {"_id":"cascade-feedback"})
                .await
                .unwrap()
                .is_some()
        );

        let claims = db.collection::<bson::Document>(
            crate::models::assistant_agent_learning::AUTHORED_CLAIMS_COLLECTION_NAME,
        );
        claims
            .insert_many([
                doc! {"_id":"target-draft","owner_id":&target_id},
                doc! {"_id":"other-draft","owner_id":&admin_id},
            ])
            .await
            .unwrap();

        crate::test_utils::accept_and_process_admin_erasure(&db, &admin_id, &target_id)
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
        assert!(
            db.collection::<bson::Document>("assistant_message_feedback")
                .find_one(doc! {"user_id": &target_id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            claims
                .find_one(doc! {"owner_id":&target_id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            claims
                .find_one(doc! {"owner_id":&admin_id})
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn lark_base_write_owner_purge_deletes_active_terminal_and_photo_rows_only_for_that_owner()
     {
        let db = crate::test_utils::connect_transaction_test_database("base_owner_purge").await;
        let admin = Uuid::new_v4().to_string();
        let owner = Uuid::new_v4().to_string();
        seed_person(&db, &admin, "base-admin@example.com").await;
        seed_person(&db, &owner, "base-owner@example.com").await;
        for collection in [
            crate::models::lark_base_write::COLLECTION_NAME,
            crate::models::lark_base_write_photo::COLLECTION_NAME,
            crate::models::assistant_attachment::COLLECTION_NAME,
            crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME,
        ] {
            for state in [
                "uploading",
                "dispatching",
                "succeeded",
                "uncertain",
                "failed_pre_dispatch",
                "failed_definite",
            ] {
                db.collection::<bson::Document>(collection)
                    .insert_one(
                        doc! {"_id":Uuid::new_v4().to_string(),"user_id":&owner,"state":state},
                    )
                    .await
                    .unwrap();
            }
            db.collection::<bson::Document>(collection)
                .insert_one(doc! {"_id":Uuid::new_v4().to_string(),"user_id":&admin})
                .await
                .unwrap();
        }
        crate::test_utils::accept_and_process_admin_erasure(&db, &admin, &owner)
            .await
            .unwrap();
        for collection in [
            crate::models::lark_base_write::COLLECTION_NAME,
            crate::models::lark_base_write_photo::COLLECTION_NAME,
            crate::models::assistant_attachment::COLLECTION_NAME,
            crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME,
        ] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {"user_id":&owner})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {"user_id":&admin})
                    .await
                    .unwrap(),
                1
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn test_delete_user_cascade_self_protection() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "self-del@example.com").await;

        let result =
            crate::test_utils::accept_and_process_admin_erasure(&db, &user_id, &user_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_user_cascade_not_found() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let admin_id = Uuid::new_v4().to_string();
        seed_person(&db, &admin_id, "admin-nf@example.com").await;

        let result = crate::test_utils::accept_and_process_admin_erasure(
            &db,
            &admin_id,
            &Uuid::new_v4().to_string(),
        )
        .await;
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
            purpose: Some(crate::models::session::SessionPurpose::Authenticated),
            mfa_method: None,
            mfa_assured: false,
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

    #[tokio::test]
    async fn admin_form_empty_user_fields_are_null() {
        let db = connect_test_database("admin_form_user_clear")
            .await
            .expect("Mongo required");
        let id = Uuid::new_v4().to_string();
        seed_person(&db, &id, "clear@example.com").await;
        let updated = update_user(&db, &id, Some("   "), None, Some("  "))
            .await
            .unwrap();
        assert!(updated.display_name.is_none());
        assert!(updated.avatar_url.is_none());
        let raw = db
            .collection::<bson::Document>(USERS)
            .find_one(doc! { "_id": id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(raw.get("display_name"), Some(&bson::Bson::Null));
        assert_eq!(raw.get("avatar_url"), Some(&bson::Bson::Null));
    }
}
