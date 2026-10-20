use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use super::auth_service::{
    AccountAudit, default_role_ids_in_session, platform_role_ids_in_session,
};
use super::{
    audit_outbox_service,
    audit_runtime::{AuditRuntime, policy::AuditTier, transaction},
    token_service::SessionAudit,
};
use crate::crypto::password;
use crate::errors::{AppError, AppResult};
use crate::models::audit_outbox::{AuditDetail, AuditOwner, AuditPrincipal, SecurityFlag};
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
    runtime: &AuditRuntime,
    principal: AuditPrincipal,
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

    let normalized = email.to_lowercase();
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

    let audit = AccountAudit::new("admin.user.created");
    let context = (db, runtime, &principal, &new_user, &audit, &platform_role);
    transaction::run(
        db,
        &context,
        |session, (db, runtime, principal, prepared, audit, role)| {
            Box::pin(async move {
                let existing = db
                    .collection::<User>(USERS)
                    .find_one(doc! {"email": &prepared.email, "user_type": "person"})
                    .session(&mut *session)
                    .await?;
                if existing.is_some() {
                    return Err(AppError::Conflict(
                        "A user with this email already exists".into(),
                    ));
                }
                let mut user = (*prepared).clone();
                user.role_ids = default_role_ids_in_session(db, session).await?;
                let ids = platform_role_ids_in_session(db, session).await?;
                role_service::add_platform_role_id(&mut user.role_ids, **role, &ids);
                db.collection::<User>(USERS)
                    .insert_one(&user)
                    .session(&mut *session)
                    .await?;
                let event = audit
                    .event((*principal).clone(), &user, &user.id)
                    .with_detail(AuditDetail::PlatformRoleChange {
                        old: None,
                        new: role_service::resolve_platform_role_from_ids(&user, &ids),
                    });
                audit_outbox_service::enqueue_admitted_in_session(
                    db,
                    session,
                    runtime,
                    AuditTier::Standard,
                    &audit.event_id,
                    &event,
                )
                .await?;
                Ok(user)
            })
        },
    )
    .await
}

/// Update a user's profile fields (admin action).
///
/// Only provided fields are updated. Validates email uniqueness and
/// field constraints.
pub async fn update_user(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    principal: AuditPrincipal,
    user_id: &str,
    display_name: Option<&str>,
    email: Option<&str>,
    avatar_url: Option<&str>,
) -> AppResult<User> {
    let audit = AccountAudit::new("admin.user.updated");
    let context = (
        db,
        runtime,
        &principal,
        user_id,
        display_name,
        email,
        avatar_url,
        &audit,
    );
    transaction::run(
        db,
        &context,
        |session, (db, runtime, principal, user_id, display_name, email, avatar_url, audit)| {
            Box::pin(async move {
                let existing = db
                    .collection::<User>(USERS)
                    .find_one(doc! { "_id": user_id })
                    .session(&mut *session)
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
                        .session(&mut *session)
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
                    .session(&mut *session)
                    .await?;

                let updated = db
                    .collection::<User>(USERS)
                    .find_one(doc! { "_id": user_id })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| {
                        AppError::Internal("User disappeared after update".to_string())
                    })?;

                let event = audit.event((*principal).clone(), &updated, &updated.id);
                audit_outbox_service::enqueue_admitted_in_session(
                    db,
                    session,
                    runtime,
                    AuditTier::Standard,
                    &audit.event_id,
                    &event,
                )
                .await?;
                Ok(updated)
            })
        },
    )
    .await
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
    runtime: &AuditRuntime,
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
        super::platform_personal_owner_lifecycle::retire(db, runtime, target_user_id, false)
            .await?;
    }
    apply_user_security_state(
        db,
        runtime,
        admin_user_id,
        target_user_id,
        is_active,
        target.is_active,
    )
    .await?;

    if !is_active {
        super::platform_personal_owner_lifecycle::retire(db, runtime, target_user_id, false)
            .await?;
    }

    Ok(())
}

/// Account state, personal-retirement markers and Machine source-epoch/outbox
/// fences commit together. The epoch remains after re-enable, so an unswept old
/// grant cannot revive.
async fn apply_user_security_state(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    admin_user_id: &str,
    user_id: &str,
    active: bool,
    expected_active: bool,
) -> AppResult<()> {
    let audit = AccountAudit::new("admin.user.status_changed");
    let principal = AuditPrincipal::Person {
        id: admin_user_id.to_owned(),
    };
    let revocation = SessionAudit::new(principal.clone(), user_id, "sessions_revoked", user_id);
    let context = (
        db,
        runtime,
        &principal,
        user_id,
        active,
        expected_active,
        &audit,
        &revocation,
    );
    transaction::run(
        db,
        &context,
        |session,
         (db, runtime, principal, owner_id, active, expected_active, audit, revocation)| {
            Box::pin(async move {
                let before = db
                    .collection::<User>(USERS)
                    .find_one(doc! {"_id": owner_id})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("User not found".into()))?;
                apply_user_security_state_in_session(
                    db,
                    session,
                    owner_id,
                    *active,
                    false,
                    None,
                    Some(*expected_active),
                )
                .await?;
                if !*active {
                    revoke_all_user_sessions_in_session(db, runtime, session, owner_id, revocation)
                        .await?;
                }
                let after = db
                    .collection::<User>(USERS)
                    .find_one(doc! {"_id": owner_id})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("User not found".into()))?;
                let event = audit
                    .event((*principal).clone(), &after, &after.id)
                    .with_detail(AuditDetail::SecurityFlagChange {
                        flag: SecurityFlag::AccountActive,
                        old: before.is_active,
                        new: after.is_active,
                    });
                audit_outbox_service::enqueue_admitted_in_session(
                    db,
                    session,
                    runtime,
                    if *active {
                        AuditTier::Standard
                    } else {
                        AuditTier::Narrowing
                    },
                    &audit.event_id,
                    &event,
                )
                .await?;
                Ok(())
            })
        },
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
    let context = (
        db,
        user_id,
        active,
        deleting,
        owner_deletion_policy,
        expected_active,
    );
    transaction::run(
        db,
        &context,
        |session, (db, user_id, active, deleting, policy, expected)| {
            Box::pin(apply_user_security_state_in_session(
                db, session, user_id, *active, *deleting, *policy, *expected,
            ))
        },
    )
    .await
}

async fn apply_user_security_state_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    user_id: &str,
    active: bool,
    deleting: bool,
    owner_deletion_policy: Option<&str>,
    expected_active: Option<bool>,
) -> AppResult<()> {
    let current = db
        .collection::<bson::Document>(USERS)
        .find_one(doc! {"_id":user_id})
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
    let mut filter = doc! {"_id":user_id};
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
    if let Some(policy) = owner_deletion_policy {
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
            policy_version: policy.to_owned(),
            accepted_at: accepted_at.to_chrono(),
        };
        let basis = bson::to_bson(&basis)
            .map_err(|_| AppError::Internal("Machine deletion basis encoding failed".into()))?;
        db.collection::<bson::Document>(crate::models::remote_machine::COLLECTION_NAME)
            .update_many(
                doc! {"owner_id":user_id,"deletion_basis":bson::Bson::Null},
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
    runtime: &AuditRuntime,
    principal: AuditPrincipal,
    user_id: &str,
) -> AppResult<Option<String>> {
    let token = zeroize::Zeroizing::new(crate::crypto::token::generate_random_token());
    let token_hash = crate::crypto::token::hash_token(&token);
    let expires_at = bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1));
    let audit = AccountAudit::new("admin.user.password_reset");
    let revocation = SessionAudit::new(principal.clone(), user_id, "sessions_revoked", user_id);
    let context = (
        db,
        runtime,
        &principal,
        user_id,
        &token_hash,
        &expires_at,
        &audit,
        &revocation,
    );
    let issued = transaction::run(
        db,
        &context,
        |session, (db, runtime, principal, user_id, token_hash, expires_at, audit, revocation)| {
            Box::pin(async move {
                let user = db
                    .collection::<User>(USERS)
                    .find_one(doc! {"_id": user_id})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("User not found".into()))?;
                if user.password_hash.is_none() {
                    return Err(AppError::BadRequest(
                        "User has no password (social login only)".into(),
                    ));
                }
                let issued = !user.user_type.is_org();
                if issued {
                    super::auth_service::write_password_reset_in_session(
                        db,
                        session,
                        &user,
                        token_hash,
                        **expires_at,
                    )
                    .await?;
                }
                revoke_all_user_sessions_in_session(db, runtime, session, &user.id, revocation)
                    .await?;
                let event = audit.event((*principal).clone(), &user, &user.id);
                audit_outbox_service::enqueue_admitted_in_session(
                    db,
                    session,
                    runtime,
                    AuditTier::Availability,
                    &audit.event_id,
                    &event,
                )
                .await?;
                Ok(issued)
            })
        },
    )
    .await?;
    Ok(issued.then(|| token.to_string()))
}

/// Delete a user and cascade-delete all related documents.
///
/// Self-protection: admin_user_id must differ from target_user_id.
/// Audit log entries are retained (orphaned reference).
pub async fn delete_user_cascade(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    admin_user_id: &str,
    target_user_id: &str,
) -> AppResult<()> {
    if admin_user_id == target_user_id {
        return Err(AppError::ValidationError(
            "Cannot delete yourself".to_string(),
        ));
    }

    delete_user_cascade_internal(db, runtime, target_user_id, None).await
}

/// Delete the currently authenticated user and cascade-delete all related documents.
///
/// This is intended for self-service account deletion flows (e.g. DELETE /users/me).
pub const ACCOUNT_DELETION_POLICY_VERSION: &str = "machine-data";
pub const ACCOUNT_DELETION_POLICY_HEADER: &str = "x-nyxid-account-deletion-policy";

pub fn validate_account_deletion_policy(policy: &str) -> AppResult<()> {
    if policy != ACCOUNT_DELETION_POLICY_VERSION {
        return Err(AppError::ValidationError(
            "Confirm the current account deletion policy, including all managed Machine data"
                .into(),
        ));
    }
    Ok(())
}

pub async fn delete_current_user_cascade(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    user_id: &str,
    deletion_policy_version: &str,
) -> AppResult<()> {
    validate_account_deletion_policy(deletion_policy_version)?;
    delete_user_cascade_internal(db, runtime, user_id, Some(deletion_policy_version)).await
}

async fn release_nyxbot_org_links(db: &mongodb::Database, user_id: &str) -> AppResult<()> {
    use futures::TryStreamExt;
    let links: Vec<bson::Document> = db
        .collection::<bson::Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find(doc! {"user_id": user_id, "bot_owner_id": {"$type": "string"}})
        .projection(doc! {"bot_owner_id": 1, "route_id": 1, "route_api_key_id": 1})
        .await?
        .try_collect()
        .await?;
    for link in links {
        let Ok(org) = link.get_str("bot_owner_id") else {
            continue;
        };
        if let Ok(route) = link.get_str("route_id") {
            match crate::services::channel_routing_service::delete_conversation(db, route, org)
                .await
            {
                Ok(()) | Err(AppError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        if let Ok(key) = link.get_str("route_api_key_id") {
            match crate::services::key_service::delete_api_key(db, org, key).await {
                Ok(()) | Err(AppError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

async fn delete_user_cascade_internal(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    target_user_id: &str,
    owner_deletion_policy: Option<&str>,
) -> AppResult<()> {
    let _target = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": target_user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    // Stop admission before erasing credentials. Unsettled provider evidence
    // retains its owner until cleanup/settlement; deletion is safely retryable.
    super::call_channels::owner_admission::close(db, target_user_id).await?;
    db.collection::<bson::Document>(crate::models::channel_adapter::BINDINGS)
        .update_many(
            doc! {"user_id":target_user_id,"enabled":true},
            doc! {"$set":{"enabled":false},"$inc":{"revision":1}},
        )
        .await?;
    db.collection::<bson::Document>(crate::models::channel_adapter::SESSIONS)
        .update_many(
            doc! {"user_id":target_user_id,"state":{"$ne":"closed"}},
            doc! {"$inc":{"generation":1},"$set":{"lease_expires_at":bson::DateTime::now()}},
        )
        .await?;
    if db.collection::<bson::Document>(crate::models::channel_adapter::SESSIONS).find_one(doc!{"user_id":target_user_id,"$or":[{"state":{"$ne":"closed"}},{"billing_finalized":false}]}).await?.is_some() || db.collection::<bson::Document>(crate::models::channel_adapter::EVENTS).find_one(doc!{"user_id":target_user_id,"event_kind":"sms_send","billing_finalized":false}).await?.is_some(){return Err(AppError::Conflict("External call or message usage is still being reconciled. Retry deletion after it settles.".into()));}

    // Busy email effects must leave the user able to authenticate and retry.
    crate::services::channel_bot_service::delete_owner_aurinko_channels(db, target_user_id).await?;

    // Only self-service owner confirmation grants Machine erasure authority.
    // Admin removal fences execution but preserves indefinite workspace data.
    apply_user_security_state_with_deletion_policy(
        db,
        target_user_id,
        false,
        true,
        owner_deletion_policy,
        None,
    )
    .await?;

    super::platform_personal_owner_lifecycle::retire(db, runtime, target_user_id, true).await?;

    // Org bots the user linked to their NyxBot keep an org-owned route and
    // route key; remove them so the org's bot is free for its admins.
    release_nyxbot_org_links(db, target_user_id).await?;

    // Phase 2: cascade delete user-owned documents keyed by user_id
    let user_filter = doc! { "user_id": target_user_id };

    let user_scoped_collections = [
        crate::models::channel_adapter::BINDINGS,
        crate::models::channel_adapter::ALLOCATIONS,
        crate::models::channel_adapter::SESSIONS,
        crate::models::channel_adapter::EVENTS,
        crate::models::channel_adapter::CLAIMS,
        crate::models::async_service_operation::COLLECTION_NAME,
        crate::models::async_service_operation::QUOTAS_COLLECTION_NAME,
        crate::models::saved_login::COLLECTION_NAME,
        crate::models::machine_setup::COLLECTION_NAME,
        crate::models::machine_desktop::COLLECTION_NAME,
        crate::models::machine_update::COLLECTION_NAME,
        crate::models::machine_update::ATTEMPTS_COLLECTION_NAME,
        crate::models::channel_activity::NOTIFICATIONS_COLLECTION,
        crate::models::channel_email::SUBSCRIPTIONS,
        crate::models::channel_email::SENDS,
        crate::models::channel_email::BATCHES,
        crate::models::channel_email::RECEIPTS,
        SESSIONS,
        REFRESH_TOKENS,
        API_KEYS,
        crate::models::permission_policy::COLLECTION_NAME,
        crate::models::assistant_acknowledgement::COLLECTION_NAME,
        crate::models::assistant_agent::COLLECTION_NAME,
        crate::models::assistant_group::COLLECTION_NAME,
        crate::models::assistant_group::MESSAGES_COLLECTION_NAME,
        crate::models::assistant_attachment::COLLECTION_NAME,
        crate::models::assistant_resource::COLLECTION_NAME,
        crate::models::nyxbot_image_delivery::COLLECTION_NAME,
        crate::models::nyxbot_image_delivery::LEGACY_COLLECTION_NAME,
        crate::models::assistant_upload_retention::TOMBSTONES,
        crate::models::assistant_agent_credential::COLLECTION_NAME,
        crate::models::assistant_conversation::COLLECTION_NAME,
        crate::models::assistant_message::COLLECTION_NAME,
        crate::models::assistant_invocation::COLLECTION_NAME,
        crate::models::assistant_invocation::TURNS,
        crate::models::assistant_message_feedback::COLLECTION_NAME,
        crate::models::assistant_checkpoint::COLLECTION_NAME,
        super::assistant_service_execution::COLLECTION,
        crate::models::nyxbot_channel::COLLECTION_NAME,
        crate::models::nyxbot_channel::THREADS_COLLECTION_NAME,
        crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME,
        crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
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
    // Machine job metadata remains until its existing retention worker can
    // retire it. Revocation consumers still need the exact persisted binding
    // to cancel submitted work after the account row has been removed.
    db.collection::<bson::Document>(
        crate::models::assistant_agent_learning::AUTHORED_CLAIMS_COLLECTION_NAME,
    )
    .delete_many(doc! {"owner_id":target_user_id})
    .await?;
    // NyxBot settings are keyed by the user ID itself.
    db.collection::<bson::Document>(crate::models::assistant_voice_session::COLLECTION_NAME)
        .update_many(
            doc! {"user_id":target_user_id},
            doc! {"$set":{"end_requested":true,"purge_requested":true}},
        )
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_voice::REQUESTS)
        .delete_many(doc! {"user_id": target_user_id})
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_settings::COLLECTION_NAME)
        .delete_one(doc! {"_id": target_user_id})
        .await?;

    // Consume the source owner's existing fenced deactivation API before
    // removing authored clients. Each bounded pass preserves exact revocation
    // receipts for other users who had granted Machine access to that client.
    loop {
        use futures::TryStreamExt;
        let clients: Vec<bson::Document> = db
            .collection::<bson::Document>(OAUTH_CLIENTS)
            .find(doc! {"created_by":target_user_id})
            .projection(doc! {"_id":1})
            .sort(doc! {"_id":1})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        if clients.is_empty() {
            break;
        }
        for client in clients {
            let id = client
                .get_str("_id")
                .map_err(|_| AppError::Internal("OAuth client identity invalid".into()))?;
            super::oauth_client_service::delete_client_for_creator(db, id, target_user_id).await?;
            db.collection::<bson::Document>(OAUTH_CLIENTS)
                .delete_one(doc! {"_id":id,"created_by":target_user_id,"is_active":false})
                .await?;
        }
    }

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
        .delete_one(doc! { "_id": target_user_id, "is_active": false,
        "platform_personal_deleting": true,
        "platform_personal_retirement_pending": {"$ne": true} })
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
pub async fn verify_email(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    principal: AuditPrincipal,
    user_id: &str,
) -> AppResult<()> {
    let audit = AccountAudit::new("admin.user.email_verified");
    let context = (db, runtime, &principal, user_id, &audit);
    transaction::run(db, &context, |session, (db, runtime, principal, user_id, audit)| {
        Box::pin(async move {
            let user = db.collection::<User>(USERS).find_one(doc! {"_id": user_id})
                .session(&mut *session).await?.ok_or_else(|| AppError::NotFound("User not found".into()))?;
            if user.email_verified { return Err(AppError::BadRequest("Email already verified".into())); }
            db.collection::<User>(USERS).update_one(
                doc! {"_id": &user.id}, doc! {"$set": {"email_verified": true, "email_verification_token": null, "updated_at": bson::DateTime::now()}}
            ).session(&mut *session).await?;
            let event = audit.event((*principal).clone(), &user, &user.id).with_detail(
                crate::models::audit_outbox::AuditDetail::SecurityFlagChange { flag: crate::models::audit_outbox::SecurityFlag::EmailVerified, old: user.email_verified, new: true }
            );
            audit_outbox_service::enqueue_admitted_in_session(db, session, runtime, AuditTier::Availability, &audit.event_id, &event).await?;
            Ok(())
        })
    }).await
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
pub async fn revoke_all_user_sessions(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    principal: AuditPrincipal,
    user_id: &str,
) -> AppResult<u64> {
    let audit = SessionAudit::new(principal, user_id, "sessions_revoked", user_id);
    let context = (db, runtime, user_id, &audit);
    transaction::run(db, &context, |session, (db, runtime, owner, audit)| {
        Box::pin(async move {
            revoke_all_user_sessions_in_session(db, runtime, session, owner, audit).await
        })
    })
    .await
}

/// Join the caller's complete account/retirement transaction.
/// The prepared identity belongs to the caller; the affected owner is loaded here.
pub(super) async fn revoke_all_user_sessions_in_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    session: &mut mongodb::ClientSession,
    user_id: &str,
    audit: &SessionAudit,
) -> AppResult<u64> {
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": user_id})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    // Session creation also writes this row, so a stale issuance conflicts.
    db.collection::<User>(USERS)
        .update_one(
            doc! {"_id": &owner.id},
            doc! {"$set": {"updated_at": bson::DateTime::now()}},
        )
        .session(&mut *session)
        .await?;
    let sessions = db
        .collection::<Session>(SESSIONS)
        .update_many(
            doc! {"user_id": &owner.id, "revoked": false},
            doc! {"$set": {"revoked": true, "last_active_at": bson::DateTime::now()}},
        )
        .session(&mut *session)
        .await?;
    let tokens = db
        .collection::<bson::Document>(REFRESH_TOKENS)
        .update_many(
            doc! {"user_id": &owner.id, "revoked": false},
            doc! {"$set": {"revoked": true}},
        )
        .session(&mut *session)
        .await?;
    if sessions.modified_count != 0 || tokens.modified_count != 0 {
        let event = audit_outbox_service::AuditEvent {
            owner: if owner.user_type.is_org() {
                AuditOwner::Org {
                    id: owner.id.clone(),
                }
            } else {
                AuditOwner::Person {
                    id: owner.id.clone(),
                }
            },
            target_id: Some(owner.id),
            ..audit.event.clone()
        }
        .with_detail(audit_outbox_service::session_revocation_count(
            sessions.modified_count,
        ));
        audit_outbox_service::enqueue_admitted_in_session(
            db,
            session,
            runtime,
            AuditTier::Narrowing,
            &audit.event_id,
            &event,
        )
        .await?;
    }
    Ok(sessions.modified_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::role::{
        COLLECTION_NAME as ROLES, PLATFORM_ADMIN_ROLE_SLUG, PLATFORM_OPERATOR_ROLE_SLUG, Role,
    };
    use crate::test_utils::*;

    #[tokio::test]
    async fn w8_id_revoke_all_joins_owner_transaction_and_keeps_admin_attribution() {
        use crate::models::audit_outbox::{PENDING_COLLECTION_NAME as PENDING, PendingAuditEvent};
        let db = connect_transaction_test_database("w8_id_revoke_all").await;
        let state = test_app_state(db.clone());
        let admin = Uuid::new_v4().to_string();
        let owner = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let issued = super::super::token_service::create_session_and_issue_tokens(
            &db,
            &state.audit_runtime,
            &state.config,
            &state.jwt_keys,
            &owner,
            None,
            None,
        )
        .await
        .unwrap();
        let principal = AuditPrincipal::Person { id: admin.clone() };
        let audit = SessionAudit::new(principal.clone(), &owner, "sessions_revoked", &owner);
        let context = (&db, &state.audit_runtime, &owner, &audit);
        let aborted: AppResult<()> =
            transaction::run(&db, &context, |session, (db, runtime, owner, audit)| {
                Box::pin(async move {
                    assert_eq!(
                        revoke_all_user_sessions_in_session(db, runtime, session, owner, audit)
                            .await?,
                        1
                    );
                    Err(AppError::Conflict("abort retirement".into()))
                })
            })
            .await;
        assert!(matches!(aborted, Err(AppError::Conflict(_))));
        assert!(
            !db.collection::<Session>(SESSIONS)
                .find_one(doc! {"_id":&issued.session_id})
                .await
                .unwrap()
                .unwrap()
                .revoked
        );
        assert_eq!(
            db.collection::<bson::Document>(REFRESH_TOKENS)
                .count_documents(doc! {"revoked":true})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING)
                .count_documents(doc! {"event.action":"sessions_revoked"})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            revoke_all_user_sessions(&db, &state.audit_runtime, principal, &owner)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<bson::Document>(REFRESH_TOKENS)
                .count_documents(doc! {"revoked":true})
                .await
                .unwrap(),
            1
        );
        let event = db
            .collection::<PendingAuditEvent>(PENDING)
            .find_one(doc! {"event.action":"sessions_revoked"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.event.principal, AuditPrincipal::Person { id: admin });
        assert_eq!(event.event.owner, AuditOwner::Person { id: owner.clone() });
        assert_eq!(
            event.event.detail,
            Some(
                crate::models::audit_outbox::AuditDetail::SessionRevocationCount {
                    count: 1,
                    saturated: false,
                }
            )
        );
        assert_eq!(
            revoke_all_user_sessions(&db, &state.audit_runtime, event.event.principal, &owner)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING)
                .count_documents(doc! {"event.action":"sessions_revoked"})
                .await
                .unwrap(),
            1
        );
    }

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
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        seed_platform_roles(&db).await;

        let user = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
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
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        seed_platform_roles(&db).await;

        let user = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "admin@example.com",
            "strongpass123",
            None,
            "admin",
        )
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
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        seed_platform_roles(&db).await;

        let user = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
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
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        seed_platform_roles(&db).await;

        let result = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "notanemail",
            "strongpass123",
            None,
            "user",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_user_rejects_short_password() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        seed_platform_roles(&db).await;

        let result = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "user@example.com",
            "short",
            None,
            "user",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_user_rejects_invalid_role() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        seed_platform_roles(&db).await;

        let result = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "user@example.com",
            "strongpass123",
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
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        seed_platform_roles(&db).await;

        create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "dup@example.com",
            "strongpass123",
            None,
            "user",
        )
        .await
        .unwrap();

        let result = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "DUP@EXAMPLE.COM",
            "strongpass123",
            None,
            "user",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_user_display_name() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "existing@example.com").await;

        let updated = update_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            &user_id,
            Some("New Name"),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(updated.display_name.as_deref(), Some("New Name"));
    }

    #[tokio::test]
    async fn test_update_user_email() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "oldemail@example.com").await;

        let updated = update_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            &user_id,
            None,
            Some("newemail@example.com"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(updated.email, "newemail@example.com");
    }

    #[tokio::test]
    async fn test_update_user_rejects_invalid_email() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "valid@example.com").await;

        let result = update_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            &user_id,
            None,
            Some("bad"),
            None,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_user_not_found() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        let result = update_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            &Uuid::new_v4().to_string(),
            Some("Name"),
            None,
            None,
        )
        .await;
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
        set_user_active(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
            &Uuid::new_v4().to_string(),
            &user,
            false,
        )
        .await
        .unwrap();
        set_user_active(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
            &Uuid::new_v4().to_string(),
            &user,
            true,
        )
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
            set_user_active(
                &db,
                &crate::test_utils::test_app_state(db.clone()).audit_runtime,
                &Uuid::new_v4().to_string(),
                &user,
                true
            )
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

        set_user_active(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
            &admin_id,
            &target_id,
            false,
        )
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
            .collection::<crate::models::api_key::ApiKey>(API_KEYS)
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

        let result = set_user_active(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
            &user_id,
            &user_id,
            false,
        )
        .await;
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

        delete_user_cascade(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
            &admin_id,
            &target_id,
        )
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
    async fn test_delete_user_cascade_self_protection() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_person(&db, &user_id, "self-del@example.com").await;

        let result = delete_user_cascade(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
            &user_id,
            &user_id,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_user_cascade_not_found() {
        let Some(db) = connect_test_database("admin_user").await else {
            return;
        };
        let admin_id = Uuid::new_v4().to_string();
        seed_person(&db, &admin_id, "admin-nf@example.com").await;

        let result = delete_user_cascade(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
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

        force_password_reset(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
            AuditPrincipal::System {
                actor: crate::models::audit_outbox::SystemAuditActor::Maintenance,
            },
            &user_id,
        )
        .await
        .unwrap();

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

        let result = force_password_reset(
            &db,
            &crate::test_utils::test_app_state(db.clone()).audit_runtime,
            AuditPrincipal::System {
                actor: crate::models::audit_outbox::SystemAuditActor::Maintenance,
            },
            &user_id,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn admin_form_empty_user_fields_are_null() {
        let db = connect_test_database("admin_form_user_clear")
            .await
            .expect("Mongo required");
        let state = crate::test_utils::test_app_state(db.clone());
        let principal = AuditPrincipal::Person {
            id: Uuid::new_v4().to_string(),
        };
        let id = Uuid::new_v4().to_string();
        seed_person(&db, &id, "clear@example.com").await;
        let updated = update_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            &id,
            Some("   "),
            None,
            Some("  "),
        )
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
    #[tokio::test]
    async fn w8_id_admin_lifecycle_uses_admin_principal_and_person_or_org_owner() {
        use crate::models::{
            audit_outbox::{AuditDetail, PENDING_COLLECTION_NAME, PendingAuditEvent},
            user::UserType,
        };
        let db = connect_transaction_test_database("w8_id_admin_lifecycle_owners").await;
        let state = test_app_state(db.clone());
        role_service::seed_system_roles(&db).await.unwrap();
        let admin = Uuid::new_v4().to_string();
        let mut administrator = test_user(&admin, UserType::Person);
        let roles = role_service::get_platform_role_ids(&db).await.unwrap();
        administrator.role_ids.push(roles.admin);
        administrator.is_admin = true;
        db.collection::<User>(USERS)
            .insert_one(administrator)
            .await
            .unwrap();
        let principal = AuditPrincipal::Person { id: admin.clone() };
        let person = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "w8-id-admin-sentinel@example.invalid",
            "w8-id-admin-password",
            None,
            "user",
        )
        .await
        .unwrap();
        let second = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "w8-id-admin-two@example.invalid",
            "w8-id-second-password",
            None,
            "operator",
        )
        .await
        .unwrap();
        update_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            &person.id,
            Some("private-display-sentinel"),
            None,
            None,
        )
        .await
        .unwrap();
        db.collection::<User>(USERS).update_one(doc! {"_id": &person.id},doc! {"$set":{"email_verified":false,"email_verification_token":"private-hash-sentinel"}}).await.unwrap();
        verify_email(&db, &state.audit_runtime, principal.clone(), &person.id)
            .await
            .unwrap();
        let issued = super::super::token_service::create_session_and_issue_tokens(
            &db,
            &state.audit_runtime,
            &state.config,
            &state.jwt_keys,
            &person.id,
            None,
            None,
        )
        .await
        .unwrap();
        let reset_token =
            force_password_reset(&db, &state.audit_runtime, principal.clone(), &person.id)
                .await
                .unwrap()
                .unwrap();
        assert!(
            db.collection::<Session>(SESSIONS)
                .find_one(doc! {"_id": &issued.session_id})
                .await
                .unwrap()
                .unwrap()
                .revoked
        );
        let retained = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &person.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retained.password_reset_token.as_deref(),
            Some(crate::crypto::token::hash_token(&reset_token).as_str())
        );
        let count = db
            .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
            .find_one(doc! {"event.action":"sessions_revoked","event.owner.id":&person.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            count.event.detail,
            Some(AuditDetail::SessionRevocationCount {
                count: 1,
                saturated: false
            })
        );
        for action in [
            "admin.user.created",
            "admin.user.updated",
            "admin.user.email_verified",
            "admin.user.password_reset",
        ] {
            let row = db
                .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .find_one(doc! {"event.action":action,"event.owner.id":&person.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.event.principal, principal);
            assert_eq!(
                row.event.owner,
                AuditOwner::Person {
                    id: person.id.clone()
                }
            );
            let bytes = bson::to_vec(&row).unwrap();
            for secret in [
                person.email.as_str(),
                "w8-id-admin-password",
                "private-display-sentinel",
                "private-hash-sentinel",
                reset_token.as_str(),
                retained.password_reset_token.as_deref().unwrap(),
                retained.password_hash.as_deref().unwrap(),
            ] {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes())
                );
            }
        }
        for _ in 0..2 {
            let org = test_user(&Uuid::new_v4().to_string(), UserType::Org);
            db.collection::<User>(USERS).insert_one(&org).await.unwrap();
            set_user_active(&db, &state.audit_runtime, &admin, &org.id, false)
                .await
                .unwrap();
            let row = db
                .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .find_one(
                    doc! {"event.action":"admin.user.status_changed","event.owner.id":&org.id},
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.event.principal, principal);
            assert_eq!(row.event.owner, AuditOwner::Org { id: org.id });
            assert_eq!(
                row.event.detail,
                Some(AuditDetail::SecurityFlagChange {
                    flag: SecurityFlag::AccountActive,
                    old: true,
                    new: false,
                })
            );
        }
        for (id, role) in [
            (&person.id, PlatformRole::User),
            (&second.id, PlatformRole::Operator),
        ] {
            let created = db
                .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .find_one(doc! {"event.action": "admin.user.created", "event.owner.id": id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                created.event.detail,
                Some(AuditDetail::PlatformRoleChange {
                    old: None,
                    new: role
                })
            );
        }
        let other = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &second.id})
            .await
            .unwrap()
            .unwrap();
        assert!(other.is_active);
        assert!(other.password_reset_token.is_none());
        assert_eq!(
            db.collection::<bson::Document>("notification_deliveries_v1")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn w8_id_forced_reset_and_admin_verification_roll_back_with_their_audits() {
        use crate::models::{
            audit_outbox::{PENDING_COLLECTION_NAME, PendingAuditEvent},
            refresh_token::{COLLECTION_NAME as REFRESH, RefreshToken},
        };
        use crate::test_utils::fail_command::{FailCommandController, Guard};
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{
            Arc, OnceLock,
            atomic::{AtomicBool, Ordering},
        };
        let base = connect_transaction_test_database("w8_id_admin_reset_atomic").await;
        let controller = Arc::new(OnceLock::<FailCommandController>::new());
        let observed_controller = controller.clone();
        let armed = Arc::new(AtomicBool::new(false));
        let observed_armed = armed.clone();
        let mut guard =
            Guard::acquire(
                &base,
                Some(EventHandler::callback(move |event| {
                    if let CommandEvent::Started(event) = event
                        && event.command_name == "insert"
                        && event.command.get_str("insert").ok() == Some(PENDING_COLLECTION_NAME)
                        && event.command.get_array("documents").is_ok_and(|rows| {
                            rows.iter().any(|row| {
                                matches!(
                                    row.as_document()
                                        .and_then(|row| row.get_document("event").ok())
                                        .and_then(|event| event.get_str("action").ok()),
                                    Some("admin.user.password_reset" | "admin.user.email_verified")
                                )
                            })
                        })
                        && !observed_armed.swap(true, Ordering::SeqCst)
                    {
                        observed_controller.get().unwrap().arm(
                    doc! {"times": 1},
                    doc! {"failCommands": ["insert"], "errorCode": 121, "errorLabels": []},
                ).unwrap();
                    }
                })),
            )
            .await;
        controller
            .set(guard.controller())
            .unwrap_or_else(|_| panic!("controller set twice"));
        let db = guard.database().clone();
        let state = test_app_state(db.clone());
        role_service::seed_system_roles(&db).await.unwrap();
        let admin = Uuid::new_v4().to_string();
        let principal = AuditPrincipal::Person { id: admin.clone() };
        let person = create_user(
            &db,
            &state.audit_runtime,
            principal.clone(),
            "w8-id-forced-reset@example.invalid",
            "w8-id-original-password",
            None,
            "user",
        )
        .await
        .unwrap();
        let mut issued = Vec::new();
        for _ in 0..2 {
            issued.push(
                super::super::token_service::create_session_and_issue_tokens(
                    &db,
                    &state.audit_runtime,
                    &state.config,
                    &state.jwt_keys,
                    &person.id,
                    None,
                    None,
                )
                .await
                .unwrap(),
            );
        }
        let before = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &person.id})
            .await
            .unwrap()
            .unwrap();
        let failed =
            force_password_reset(&db, &state.audit_runtime, principal.clone(), &person.id).await;
        assert!(
            armed.load(Ordering::SeqCst),
            "fault must follow token issuance and revocation"
        );
        assert!(matches!(failed, Err(AppError::DatabaseError(_))));
        guard.disarm().await.unwrap();
        let after = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &person.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.password_reset_token, before.password_reset_token);
        assert_eq!(
            after.password_reset_expires_at,
            before.password_reset_expires_at
        );
        for collection in [SESSIONS, REFRESH] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {"user_id": &person.id, "revoked": false},)
                    .await
                    .unwrap(),
                2
            );
        }
        assert_eq!(db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME).count_documents(
            doc! {"event.action": {"$in": ["admin.user.password_reset", "sessions_revoked"]}},
        ).await.unwrap(), 0);
        let token = force_password_reset(&db, &state.audit_runtime, principal.clone(), &person.id)
            .await
            .unwrap()
            .unwrap();
        let updated = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &person.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            updated.password_reset_token,
            Some(crate::crypto::token::hash_token(&token))
        );
        assert_eq!(updated.password_hash, before.password_hash);
        for item in issued {
            assert!(
                db.collection::<Session>(SESSIONS)
                    .find_one(doc! {"_id": &item.session_id})
                    .await
                    .unwrap()
                    .unwrap()
                    .revoked
            );
        }
        assert_eq!(
            db.collection::<RefreshToken>(REFRESH)
                .count_documents(doc! {"user_id": &person.id, "revoked": false},)
                .await
                .unwrap(),
            0
        );
        let revocation = db
            .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
            .find_one(doc! {"event.action": "sessions_revoked"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            revocation.event.detail,
            Some(AuditDetail::SessionRevocationCount {
                count: 2,
                saturated: false
            })
        );
        assert_eq!(revocation.event.principal, principal);
        assert_eq!(
            revocation.event.owner,
            AuditOwner::Person {
                id: person.id.clone()
            }
        );

        db.collection::<User>(USERS).update_one(
            doc! {"_id": &person.id},
            doc! {"$set": {"email_verified": false, "email_verification_token": "w8-id-verification-hash-sentinel"}},
        ).await.unwrap();
        armed.store(false, Ordering::SeqCst);
        let failed = verify_email(&db, &state.audit_runtime, principal.clone(), &person.id).await;
        assert!(armed.load(Ordering::SeqCst));
        assert!(matches!(failed, Err(AppError::DatabaseError(_))));
        guard.disarm().await.unwrap();
        let unverified = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &person.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!unverified.email_verified);
        assert_eq!(
            unverified.email_verification_token.as_deref(),
            Some("w8-id-verification-hash-sentinel")
        );
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .count_documents(doc! {"event.action": "admin.user.email_verified"},)
                .await
                .unwrap(),
            0
        );
        verify_email(&db, &state.audit_runtime, principal.clone(), &person.id)
            .await
            .unwrap();
        let verified = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &person.id})
            .await
            .unwrap()
            .unwrap();
        assert!(verified.email_verified);
        assert!(verified.email_verification_token.is_none());
        let event = db
            .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
            .find_one(doc! {"event.action": "admin.user.email_verified"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.event.principal, principal);
        assert_eq!(
            event.event.detail,
            Some(AuditDetail::SecurityFlagChange {
                flag: SecurityFlag::EmailVerified,
                old: false,
                new: true,
            })
        );
        assert_eq!(
            db.collection::<bson::Document>("notification_deliveries_v1")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
}
