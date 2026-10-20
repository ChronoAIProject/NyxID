use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::password;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::audit_outbox::{
    AuditDetail, AuditOwner, AuditPrincipal, AuditStage, SecurityFlag, SystemAuditActor,
};
use crate::models::user::{COLLECTION_NAME as USERS, PlatformRole, User, UserType};
use crate::services::{
    audit_outbox_service::{self, AuditEvent},
    audit_runtime::{AuditRuntime, policy::AuditTier, transaction},
    role_service,
};

/// Stable audit identity prepared by the operation owner before transaction retries.
pub(super) struct AccountAudit {
    pub event_id: Uuid,
    action: &'static str,
}

impl AccountAudit {
    pub(super) fn new(action: &'static str) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            action,
        }
    }

    pub(super) fn event(
        &self,
        principal: AuditPrincipal,
        owner: &User,
        target: &str,
    ) -> AuditEvent {
        AuditEvent {
            operation_id: self.event_id.to_string(),
            stage: AuditStage::Committed,
            principal,
            owner: if owner.user_type.is_org() {
                AuditOwner::Org {
                    id: owner.id.clone(),
                }
            } else {
                AuditOwner::Person {
                    id: owner.id.clone(),
                }
            },
            action: self.action,
            target_id: Some(target.to_owned()),
        }
    }
}

fn authentication_principal() -> AuditPrincipal {
    AuditPrincipal::System {
        actor: SystemAuditActor::Authentication,
    }
}

/// Session-aware RBAC reads; role decisions still use role_service's shared logic.
pub(super) async fn platform_role_ids_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
) -> AppResult<role_service::PlatformRoleIds> {
    use crate::models::role::{
        COLLECTION_NAME, PLATFORM_ADMIN_ROLE_SLUG, PLATFORM_OPERATOR_ROLE_SLUG, Role,
    };
    let mut ids = Vec::with_capacity(2);
    for slug in [PLATFORM_ADMIN_ROLE_SLUG, PLATFORM_OPERATOR_ROLE_SLUG] {
        let role = db
            .collection::<Role>(COLLECTION_NAME)
            .find_one(doc! { "slug": slug, "client_id": null, "is_system": true })
            .session(&mut *session)
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "Platform system role '{slug}' is missing; startup seed did not complete"
                ))
            })?;
        ids.push(role.id);
    }
    Ok(role_service::PlatformRoleIds {
        admin: ids.remove(0),
        operator: ids.remove(0),
    })
}

pub(super) async fn default_role_ids_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
) -> AppResult<Vec<String>> {
    use crate::models::role::{COLLECTION_NAME, Role};
    use futures::TryStreamExt;
    let mut cursor = db
        .collection::<Role>(COLLECTION_NAME)
        .find(doc! { "is_default": true })
        .session(&mut *session)
        .await?;
    let roles: Vec<Role> = cursor.stream(session).try_collect().await?;
    Ok(roles.into_iter().map(|role| role.id).collect())
}

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
    /// Raw material is exposed only to verification regressions, never handlers.
    #[cfg(test)]
    pub email_verification_token: Zeroizing<String>,
    /// `true` when a new user was actually inserted; `false` when the email
    /// already existed and a fake success was returned for email-enumeration
    /// protection. Invitation consumption commits with actual account creation;
    /// callers use this to avoid provisioning or telemetry for fake identities.
    pub actually_created: bool,
    pub invite_code_id: Option<String>,
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
    runtime: &AuditRuntime,
    email: &str,
    password_raw: &str,
    display_name: Option<&str>,
    invite_code: Option<&str>,
    auto_verify_email: bool,
) -> AppResult<RegisterResult> {
    let prepared = prepare_registration(email, password_raw, display_name, auto_verify_email)?;
    let plan = RegistrationPlan {
        invite_code: invite_code.map(str::to_owned),
        bootstrap: false,
        audit: AccountAudit::new("register"),
        invite_audit: AccountAudit::new("invite.code_redeemed"),
    };
    commit_registration(db, runtime, &prepared, &plan).await
}

/// Bootstrap the first administrator in one transaction, including the empty
/// account check, RBAC assignment and system audit. Concurrent winners conflict
/// on the shared audit reservation and retry the complete empty-account check.
pub async fn bootstrap_admin(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    email: &str,
    password_raw: &str,
    display_name: Option<&str>,
) -> AppResult<RegisterResult> {
    let prepared = prepare_registration(email, password_raw, display_name, true)?;
    let plan = RegistrationPlan {
        invite_code: None,
        bootstrap: true,
        audit: AccountAudit::new("admin_setup"),
        invite_audit: AccountAudit::new("invite.code_redeemed"),
    };
    commit_registration(db, runtime, &prepared, &plan).await
}

struct PreparedRegistration {
    user: User,
    #[cfg(test)]
    verification_token: Zeroizing<String>,
}

struct RegistrationPlan {
    invite_code: Option<String>,
    bootstrap: bool,
    audit: AccountAudit,
    invite_audit: AccountAudit,
}

fn prepare_registration(
    email: &str,
    password_raw: &str,
    display_name: Option<&str>,
    auto_verify_email: bool,
) -> AppResult<PreparedRegistration> {
    if password_raw.len() < 8 {
        return Err(AppError::ValidationError(
            "Password must be at least 8 characters".into(),
        ));
    }
    if password_raw.len() > MAX_PASSWORD_LENGTH {
        return Err(AppError::ValidationError(format!(
            "Password must be at most {MAX_PASSWORD_LENGTH} characters"
        )));
    }
    let password_hash = password::hash_password(password_raw)?;
    let verification_token = Zeroizing::new(generate_random_token());
    let now = Utc::now();
    let user = User {
        id: Uuid::new_v4().to_string(),
        email: email.to_lowercase(),
        password_hash: Some(password_hash),
        display_name: display_name.map(str::to_owned),
        slug: None,
        avatar_url: None,
        email_verified: auto_verify_email,
        email_verification_token: if auto_verify_email {
            None
        } else {
            Some(hash_token(&verification_token))
        },
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
    };
    Ok(PreparedRegistration {
        user,
        #[cfg(test)]
        verification_token,
    })
}

async fn commit_registration(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    prepared: &PreparedRegistration,
    plan: &RegistrationPlan,
) -> AppResult<RegisterResult> {
    let context = (db, runtime, prepared, plan);
    let (actually_created, invite_code_id) =
        transaction::run(db, &context, |session, (db, runtime, prepared, plan)| {
            Box::pin(register_prepared_in_session(
                db, runtime, session, prepared, plan,
            ))
        })
        .await?;
    Ok(RegisterResult {
        user_id: prepared.user.id.clone(),
        #[cfg(test)]
        email_verification_token: prepared.verification_token.clone(),
        actually_created,
        invite_code_id,
    })
}

async fn register_prepared_in_session(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    session: &mut mongodb::ClientSession,
    prepared: &PreparedRegistration,
    plan: &RegistrationPlan,
) -> AppResult<(bool, Option<String>)> {
    if plan.bootstrap
        && db
            .collection::<User>(USERS)
            .count_documents(doc! {})
            .session(&mut *session)
            .await?
            != 0
    {
        return Err(AppError::Forbidden(
            "Setup has already been completed. Use the CLI --promote-admin flag to promote existing users.".into()
        ));
    }
    let invite = match &plan.invite_code {
        Some(code) => Some(
            super::invite_code_service::registration_invite_in_session(
                db,
                session,
                code,
                &prepared.user.email,
            )
            .await?,
        ),
        None => None,
    };
    if db
        .collection::<User>(USERS)
        .find_one(doc! {
            "email": &prepared.user.email, "user_type": "person",
        })
        .session(&mut *session)
        .await?
        .is_some()
    {
        // No authority mutation, reservation or invented-person audit for fake success.
        return Ok((false, None));
    }
    let mut user = prepared.user.clone();
    user.role_ids = default_role_ids_in_session(db, session).await?;
    let platform_role_ids = platform_role_ids_in_session(db, session).await?;
    if plan.bootstrap {
        role_service::add_platform_role_id(
            &mut user.role_ids,
            PlatformRole::Admin,
            &platform_role_ids,
        );
        (user.is_admin, user.is_operator) = PlatformRole::Admin.legacy_flags();
    }
    user.invite_code_id = invite.as_ref().map(|row| row.id.clone());
    db.collection::<User>(USERS)
        .insert_one(&user)
        .session(&mut *session)
        .await?;
    let principal = if plan.bootstrap {
        AuditPrincipal::System {
            actor: SystemAuditActor::Startup,
        }
    } else {
        authentication_principal()
    };
    if let Some(invite) = &invite {
        super::invite_code_service::consume_registration_invite_in_session(
            db, session, invite, &user,
        )
        .await?;
        let event = plan
            .invite_audit
            .event(authentication_principal(), &user, &invite.id);
        audit_outbox_service::enqueue_admitted_in_session(
            db,
            session,
            runtime,
            AuditTier::Standard,
            &plan.invite_audit.event_id,
            &event,
        )
        .await?;
    }
    let event =
        plan.audit
            .event(principal, &user, &user.id)
            .with_detail(AuditDetail::PlatformRoleChange {
                old: None,
                new: role_service::resolve_platform_role_from_ids(&user, &platform_role_ids),
            });
    audit_outbox_service::enqueue_admitted_in_session(
        db,
        session,
        runtime,
        AuditTier::Standard,
        &plan.audit.event_id,
        &event,
    )
    .await?;
    Ok((true, user.invite_code_id))
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
pub async fn verify_email(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    token: &str,
) -> AppResult<String> {
    let token_hash = hash_token(token);
    let audit = AccountAudit::new("email_verified");
    let context = (db, runtime, &token_hash, &audit);
    transaction::run(db, &context, |session, (db, runtime, token_hash, audit)| {
        Box::pin(async move {
            let user = db.collection::<User>(USERS)
                .find_one(doc! { "email_verification_token": token_hash })
                .session(&mut *session).await?
                .ok_or_else(|| AppError::BadRequest("Invalid or expired verification token".into()))?;
            ensure_person_user(&user)?;
            if user.email_verified { return Err(AppError::BadRequest("Email already verified".into())); }
            db.collection::<User>(USERS).update_one(
                doc! { "_id": &user.id, "email_verification_token": token_hash },
                doc! { "$set": { "email_verified": true, "email_verification_token": null, "updated_at": bson::DateTime::now() } },
            ).session(&mut *session).await?;
            let event = audit.event(AuditPrincipal::Person { id: user.id.clone() }, &user, &user.id)
                .with_detail(AuditDetail::SecurityFlagChange { flag: SecurityFlag::EmailVerified, old: user.email_verified, new: true });
            audit_outbox_service::enqueue_admitted_in_session(db, session, runtime, AuditTier::Availability, &audit.event_id, &event).await?;
            Ok(user.id)
        })
    }).await
}

/// Initiate a password reset by generating a reset token.
///
/// Stores the hash of the token, not the raw token.
pub async fn initiate_password_reset(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    email: &str,
) -> AppResult<Option<String>> {
    let token = Zeroizing::new(generate_random_token());
    let token_hash = hash_token(&token);
    let email = email.to_lowercase();
    let expires_at = bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1));
    let audit = AccountAudit::new("password_reset_requested");
    let context = (db, runtime, &email, &token_hash, &expires_at, &audit);
    let issued = transaction::run(
        db,
        &context,
        |session, (db, runtime, email, token_hash, expires_at, audit)| {
            Box::pin(async move {
                let Some(user) = db
                    .collection::<User>(USERS)
                    .find_one(doc! { "email": email, "user_type": "person" })
                    .session(&mut *session)
                    .await?
                else {
                    return Ok(false);
                };
                ensure_person_user(&user)?;
                write_password_reset_in_session(db, session, &user, token_hash, **expires_at)
                    .await?;
                let event = audit.event(authentication_principal(), &user, &user.id);
                audit_outbox_service::enqueue_admitted_in_session(
                    db,
                    session,
                    runtime,
                    AuditTier::Availability,
                    &audit.event_id,
                    &event,
                )
                .await?;
                Ok(true)
            })
        },
    )
    .await?;
    Ok(issued.then(|| token.to_string()))
}

pub(super) async fn write_password_reset_in_session(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    user: &User,
    token_hash: &str,
    expires_at: bson::DateTime,
) -> AppResult<()> {
    db.collection::<User>(USERS).update_one(
        doc! { "_id": &user.id },
        doc! { "$set": { "password_reset_token": token_hash, "password_reset_expires_at": expires_at, "updated_at": bson::DateTime::now() } },
    ).session(session).await?;
    Ok(())
}

/// Complete a password reset with the token and new password.
///
/// Hashes the incoming token for comparison against stored hash.
pub async fn reset_password(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    token: &str,
    new_password: &str,
) -> AppResult<()> {
    if new_password.len() < 8 {
        return Err(AppError::ValidationError(
            "Password must be at least 8 characters".into(),
        ));
    }
    if new_password.len() > MAX_PASSWORD_LENGTH {
        return Err(AppError::ValidationError(format!(
            "Password must be at most {MAX_PASSWORD_LENGTH} characters"
        )));
    }
    let token_hash = hash_token(token);
    let new_hash = password::hash_password(new_password)?;
    let audit = AccountAudit::new("password_reset");
    let revoke_id = Uuid::new_v4();
    let context = (db, runtime, &token_hash, &new_hash, &audit, &revoke_id);
    transaction::run(db, &context, |session, (db, runtime, token_hash, new_hash, audit, revoke_id)| {
        Box::pin(async move {
            let user = db.collection::<User>(USERS)
                .find_one(doc! { "password_reset_token": token_hash })
                .session(&mut *session).await?
                .ok_or_else(|| AppError::BadRequest("Invalid or expired reset token".into()))?;
            ensure_person_user(&user)?;
            match user.password_reset_expires_at {
                Some(expires_at) if expires_at < Utc::now() => return Err(AppError::BadRequest("Reset token has expired".into())),
                None => return Err(AppError::BadRequest("Invalid reset token".into())),
                _ => (),
            }
            db.collection::<User>(USERS).update_one(
                doc! { "_id": &user.id, "password_reset_token": token_hash },
                doc! { "$set": { "password_hash": new_hash, "password_reset_token": null, "password_reset_expires_at": null, "updated_at": bson::DateTime::now() } },
            ).session(&mut *session).await?;
            let principal = AuditPrincipal::Person { id: user.id.clone() };
            let revoke_audit = super::token_service::SessionAudit {
                event_id: **revoke_id,
                event: AuditEvent { operation_id: revoke_id.to_string(), stage: AuditStage::Committed, principal: principal.clone(), owner: AuditOwner::Person { id: user.id.clone() }, action: "sessions_revoked", target_id: Some(user.id.clone()) },
            };
            super::admin_user_service::revoke_all_user_sessions_in_session(db, runtime, session, &user.id, &revoke_audit).await?;
            let event = audit.event(principal, &user, &user.id);
            audit_outbox_service::enqueue_admitted_in_session(db, session, runtime, AuditTier::Availability, &audit.event_id, &event).await?;
            Ok(())
        })
    }).await
}

/// Promote an existing user to admin by email address.
///
/// Assigns the platform admin RBAC role, mirrors `is_admin = true`, and sets
/// `email_verified = true` on the user.
/// Returns the user ID on success.
pub async fn promote_user_to_admin(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    principal: AuditPrincipal,
    email: &str,
) -> AppResult<String> {
    let normalized = email.to_lowercase();
    let audit = AccountAudit::new("admin_promoted");
    let context = (db, runtime, &principal, &normalized, &audit);
    transaction::run(
        db,
        &context,
        |session, (db, runtime, principal, normalized, audit)| {
            Box::pin(async move {
                let user = db
                    .collection::<User>(USERS)
                    .find_one(doc! { "email": normalized, "user_type": "person" })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("No user found with email: {normalized}"))
                    })?;
                ensure_person_user(&user)?;
                let role_ids = platform_role_ids_in_session(db, session).await?;
                let old_role = role_service::resolve_platform_role_from_ids(&user, &role_ids);
                if old_role.is_admin() {
                    return Err(AppError::Conflict(format!(
                        "User {normalized} is already an admin"
                    )));
                }
                let mut pipeline = role_service::set_platform_role_update(
                    PlatformRole::Admin,
                    &role_ids,
                    bson::DateTime::now(),
                );
                pipeline.push(doc! { "$set": { "email_verified": true } });
                db.collection::<User>(USERS)
                    .update_one(doc! { "_id": &user.id }, pipeline)
                    .session(&mut *session)
                    .await?;
                let updated = db
                    .collection::<User>(USERS)
                    .find_one(doc! {"_id": &user.id})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::Internal("Promoted user disappeared".into()))?;
                let event = audit
                    .event((*principal).clone(), &updated, &updated.id)
                    .with_detail(AuditDetail::PlatformRoleChange {
                        old: Some(old_role),
                        new: role_service::resolve_platform_role_from_ids(&updated, &role_ids),
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
                Ok(user.id)
            })
        },
    )
    .await
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
        let state = crate::test_utils::test_app_state(db.clone());

        let mut org = make_person();
        org.email = "org-owner@example.com".to_string();
        org.user_type = UserType::Org;
        db.collection::<User>(USERS).insert_one(&org).await.unwrap();

        let result = initiate_password_reset(&db, &state.audit_runtime, &org.email)
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let result = register_user(
            &db,
            &state.audit_runtime,
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
        let state = crate::test_utils::test_app_state(db.clone());

        match register_user(
            &db,
            &state.audit_runtime,
            "short@example.com",
            "1234567",
            None,
            None,
            false,
        )
        .await
        {
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
        let state = crate::test_utils::test_app_state(db.clone());

        let long_pw = "a".repeat(129);
        match register_user(
            &db,
            &state.audit_runtime,
            "long@example.com",
            &long_pw,
            None,
            None,
            false,
        )
        .await
        {
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(
            &db,
            &state.audit_runtime,
            "dup@example.com",
            "password123",
            None,
            None,
            false,
        )
        .await
        .expect("first register");

        let result = register_user(
            &db,
            &state.audit_runtime,
            "dup@example.com",
            "password456",
            None,
            None,
            false,
        )
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let result = register_user(
            &db,
            &state.audit_runtime,
            "auto@example.com",
            "password123",
            None,
            None,
            true,
        )
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(
            &db,
            &state.audit_runtime,
            "user@example.com",
            "correct-pw-1",
            None,
            None,
            true,
        )
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let result = register_user(
            &db,
            &state.audit_runtime,
            "inactive@example.com",
            "password123",
            None,
            None,
            true,
        )
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(
            &db,
            &state.audit_runtime,
            "good@example.com",
            "password123",
            None,
            None,
            true,
        )
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
        let state = crate::test_utils::test_app_state(db.clone());

        let err = verify_email(&db, &state.audit_runtime, "invalid-token-value")
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let reg = register_user(
            &db,
            &state.audit_runtime,
            "verify@example.com",
            "password123",
            None,
            None,
            false,
        )
        .await
        .expect("register");

        verify_email(&db, &state.audit_runtime, &reg.email_verification_token)
            .await
            .expect("first verify");

        let err = verify_email(&db, &state.audit_runtime, &reg.email_verification_token)
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let reg = register_user(
            &db,
            &state.audit_runtime,
            "hvp@example.com",
            "password123",
            None,
            None,
            false,
        )
        .await
        .expect("register");
        assert!(reg.actually_created);

        let user_id = verify_email(&db, &state.audit_runtime, &reg.email_verification_token)
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
        let state = crate::test_utils::test_app_state(db.clone());

        let result = initiate_password_reset(&db, &state.audit_runtime, "unknown@example.com")
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(
            &db,
            &state.audit_runtime,
            "reset@example.com",
            "password123",
            None,
            None,
            true,
        )
        .await
        .expect("register");

        let token = initiate_password_reset(&db, &state.audit_runtime, "reset@example.com")
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
        let state = crate::test_utils::test_app_state(db.clone());

        let err = reset_password(&db, &state.audit_runtime, "some-token", "short")
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
        let state = crate::test_utils::test_app_state(db.clone());

        let long_pw = "a".repeat(129);
        let err = reset_password(&db, &state.audit_runtime, "some-token", &long_pw)
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
        let state = crate::test_utils::test_app_state(db.clone());

        let err = reset_password(
            &db,
            &state.audit_runtime,
            "invalid-token-value",
            "newpassword1",
        )
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        let reg = register_user(
            &db,
            &state.audit_runtime,
            "expired@example.com",
            "password123",
            None,
            None,
            true,
        )
        .await
        .expect("register");

        let token = initiate_password_reset(&db, &state.audit_runtime, "expired@example.com")
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

        let err = reset_password(&db, &state.audit_runtime, &token, "newpassword1")
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
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();

        register_user(
            &db,
            &state.audit_runtime,
            "rp-ok@example.com",
            "oldpassword1",
            None,
            None,
            true,
        )
        .await
        .expect("register");

        let token = initiate_password_reset(&db, &state.audit_runtime, "rp-ok@example.com")
            .await
            .expect("initiate")
            .expect("token");

        reset_password(&db, &state.audit_runtime, &token, "newpassword1")
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
    #[tokio::test]
    async fn w8_id_registration_invite_and_audits_abort_and_commit_together() {
        use crate::models::{
            audit_outbox::{PENDING_COLLECTION_NAME, PendingAuditEvent},
            invite_code::{COLLECTION_NAME as INVITES, InviteCode},
        };
        let db =
            crate::test_utils::connect_transaction_test_database("w8_id_registration_atomic").await;
        let state = crate::test_utils::test_app_state(db.clone());
        role_service::seed_system_roles(&db).await.unwrap();
        let invite = super::super::invite_code_service::create_invite_code(
            &db,
            &Uuid::new_v4().to_string(),
            1,
            None,
        )
        .await
        .unwrap();
        let email = "w8-id-registration-sentinel@example.invalid";
        let password = "w8-id-registration-secret";
        let prepared =
            prepare_registration(email, password, Some("w8-id-display-sentinel"), false).unwrap();
        let plan = RegistrationPlan {
            invite_code: Some(invite.code.clone()),
            bootstrap: false,
            audit: AccountAudit::new("register"),
            invite_audit: AccountAudit::new("invite.code_redeemed"),
        };
        let context = (&db, &state.audit_runtime, &prepared, &plan);
        let aborted: AppResult<()> =
            transaction::run(&db, &context, |session, (db, runtime, prepared, plan)| {
                Box::pin(async move {
                    register_prepared_in_session(db, runtime, session, prepared, plan).await?;
                    Err(AppError::Conflict("abort account creation".into()))
                })
            })
            .await;
        assert!(matches!(aborted, Err(AppError::Conflict(_))));
        assert_eq!(
            db.collection::<User>(USERS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let unchanged = db
            .collection::<InviteCode>(INVITES)
            .find_one(doc! {"_id": &invite.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.used_count, 0);
        assert!(unchanged.usages.is_empty());
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .count_documents(
                    doc! {"event.action": {"$in": ["register", "invite.code_redeemed"]}}
                )
                .await
                .unwrap(),
            0
        );
        let result = commit_registration(&db, &state.audit_runtime, &prepared, &plan)
            .await
            .unwrap();
        assert!(result.actually_created);
        assert_eq!(result.invite_code_id.as_deref(), Some(invite.id.as_str()));
        let consumed = db
            .collection::<InviteCode>(INVITES)
            .find_one(doc! {"_id": &invite.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(consumed.used_count, 1);
        assert_eq!(consumed.usages.len(), 1);
        assert_eq!(consumed.usages[0].user_id, result.user_id);
        for action in ["register", "invite.code_redeemed"] {
            let pending = db
                .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .find_one(doc! {"event.action": action})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(pending.event.principal, authentication_principal());
            assert_eq!(
                pending.event.owner,
                AuditOwner::Person {
                    id: result.user_id.clone()
                }
            );
        }
        while audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
        {}
        for collection in [
            PENDING_COLLECTION_NAME,
            crate::models::audit_log::COLLECTION_NAME,
            "audit_outbox",
        ] {
            use futures::TryStreamExt;
            let rows: Vec<bson::Document> = db
                .collection::<bson::Document>(collection)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            for row in rows {
                let bytes = bson::to_vec(&row).unwrap();
                for secret in [
                    email,
                    password,
                    prepared.verification_token.as_str(),
                    prepared.user.password_hash.as_deref().unwrap(),
                    prepared.user.email_verification_token.as_deref().unwrap(),
                    invite.code.as_str(),
                    "w8-id-display-sentinel",
                ] {
                    assert!(
                        !bytes
                            .windows(secret.len())
                            .any(|window| window == secret.as_bytes()),
                        "{collection} contained account material"
                    );
                }
            }
        }
        assert_eq!(
            db.collection::<bson::Document>("notification_deliveries_v1")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<bson::Document>("audit_effect_intents")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn w8_id_registration_unknown_commit_retries_commit_without_double_consumption() {
        use crate::models::{
            audit_outbox::{PENDING_COLLECTION_NAME, PendingAuditEvent},
            invite_code::{COLLECTION_NAME as INVITES, InviteCode},
        };
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        let base =
            crate::test_utils::connect_transaction_test_database("w8_id_registration_unknown")
                .await;
        let commands = Arc::new(Mutex::new(Vec::new()));
        let captured = commands.clone();
        let mut fault = crate::test_utils::fail_command::Guard::acquire(
            &base,
            Some(EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event
                    && event.command_name == "commitTransaction"
                {
                    captured.lock().unwrap().push(event.command);
                }
            })),
        )
        .await;
        let db = fault.database().clone();
        let state = crate::test_utils::test_app_state(db.clone());
        role_service::seed_system_roles(&db).await.unwrap();
        let invite = super::super::invite_code_service::create_invite_code(
            &db,
            &Uuid::new_v4().to_string(),
            1,
            None,
        )
        .await
        .unwrap();
        let prepared = prepare_registration(
            "w8-id-unknown@example.invalid",
            "w8-id-secret-password",
            None,
            true,
        )
        .unwrap();
        let plan = RegistrationPlan {
            invite_code: Some(invite.code.clone()),
            bootstrap: false,
            audit: AccountAudit::new("register"),
            invite_audit: AccountAudit::new("invite.code_redeemed"),
        };
        commands.lock().unwrap().clear();
        fault.arm(doc! {"times": 1}, doc! {"failCommands": ["commitTransaction"],
            "writeConcernError": {"code": 64, "errmsg": "injected uncertain registration commit"},
            "errorLabels": ["UnknownTransactionCommitResult"]}).await.unwrap();
        let attempts = AtomicUsize::new(0);
        let context = (&db, &state.audit_runtime, &prepared, &plan, &attempts);
        transaction::run(
            &db,
            &context,
            |session, (db, runtime, prepared, plan, attempts)| {
                Box::pin(async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    register_prepared_in_session(db, runtime, session, prepared, plan).await
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        fault.disarm().await.unwrap();
        let commits = commands.lock().unwrap().clone();
        assert!(commits.len() >= 2);
        for commit in &commits[1..] {
            assert_eq!(commit.get("lsid"), commits[0].get("lsid"));
            assert_eq!(commit.get("txnNumber"), commits[0].get("txnNumber"));
        }
        assert_eq!(
            db.collection::<User>(USERS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        let consumed = db
            .collection::<InviteCode>(INVITES)
            .find_one(doc! {"_id": &invite.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(consumed.used_count, 1);
        assert_eq!(consumed.usages.len(), 1);
        for action in ["register", "invite.code_redeemed"] {
            assert_eq!(
                db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                    .count_documents(doc! {"event.action": action})
                    .await
                    .unwrap(),
                1
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn w8_id_bootstrap_has_one_winner_and_a_system_attributed_role_grant() {
        use crate::models::audit_outbox::{PENDING_COLLECTION_NAME, PendingAuditEvent};
        let db = crate::test_utils::connect_transaction_test_database("w8_id_bootstrap_one_winner")
            .await;
        let state = crate::test_utils::test_app_state(db.clone());
        role_service::seed_system_roles(&db).await.unwrap();
        let (first, second) = tokio::join!(
            bootstrap_admin(
                &db,
                &state.audit_runtime,
                "w8-id-first@example.invalid",
                "w8-id-password-one",
                None
            ),
            bootstrap_admin(
                &db,
                &state.audit_runtime,
                "w8-id-second@example.invalid",
                "w8-id-password-two",
                None
            ),
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let winner = first.or(second).unwrap();
        let owner = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &winner.user_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            role_service::resolve_platform_role(&db, &owner)
                .await
                .unwrap(),
            PlatformRole::Admin
        );
        assert!(owner.email_verified);
        assert_eq!(
            db.collection::<User>(USERS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        let event = db
            .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
            .find_one(doc! {"event.action": "admin_setup"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            event.event.principal,
            AuditPrincipal::System {
                actor: SystemAuditActor::Startup
            }
        );
        assert_eq!(event.event.owner, AuditOwner::Person { id: owner.id });
        assert_eq!(
            event.event.detail,
            Some(AuditDetail::PlatformRoleChange {
                old: None,
                new: PlatformRole::Admin,
            })
        );
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .count_documents(doc! {"event.action": "admin_setup"})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn w8_id_password_redemption_rolls_back_after_consumption_and_revokes_on_success() {
        use crate::models::{
            audit_outbox::{AuditDetail, PENDING_COLLECTION_NAME, PendingAuditEvent},
            refresh_token::{COLLECTION_NAME as REFRESH, RefreshToken},
            session::{COLLECTION_NAME as SESSIONS, Session},
        };
        use crate::test_utils::fail_command::{FailCommandController, Guard};
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{
            Arc, OnceLock,
            atomic::{AtomicBool, Ordering},
        };
        let base = crate::test_utils::connect_transaction_test_database("w8_id_reset_atomic").await;
        let controller = Arc::new(OnceLock::<FailCommandController>::new());
        let observed_controller = controller.clone();
        let armed = Arc::new(AtomicBool::new(false));
        let observed_armed = armed.clone();
        let mut fault = Guard::acquire(
            &base,
            Some(EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event
                    && event.command_name == "insert"
                    && event.command.get_str("insert").ok() == Some(PENDING_COLLECTION_NAME)
                    && event.command.get_array("documents").is_ok_and(|documents| {
                        documents.iter().any(|row| {
                            row.as_document()
                                .and_then(|row| row.get_document("event").ok())
                                .and_then(|event| event.get_str("action").ok())
                                == Some("password_reset")
                        })
                    })
                    && !observed_armed.swap(true, Ordering::SeqCst)
                {
                    observed_controller
                        .get()
                        .unwrap()
                        .arm(
                            doc! {"times":1},
                            doc! {"failCommands":["insert"], "errorCode":121, "errorLabels": []},
                        )
                        .unwrap();
                }
            })),
        )
        .await;
        controller
            .set(fault.controller())
            .unwrap_or_else(|_| panic!("fault controller already set"));
        let db = fault.database().clone();
        let state = crate::test_utils::test_app_state(db.clone());
        role_service::seed_system_roles(&db).await.unwrap();
        let registered = register_user(
            &db,
            &state.audit_runtime,
            "w8-id-reset@example.invalid",
            "w8-id-old-password",
            None,
            None,
            true,
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
                    &registered.user_id,
                    None,
                    None,
                )
                .await
                .unwrap(),
            );
        }
        let token =
            initiate_password_reset(&db, &state.audit_runtime, "w8-id-reset@example.invalid")
                .await
                .unwrap()
                .unwrap();
        let before = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &registered.user_id})
            .await
            .unwrap()
            .unwrap();
        let failure = reset_password(&db, &state.audit_runtime, &token, "w8-id-new-password").await;
        assert!(
            armed.load(Ordering::SeqCst),
            "fault must occur after consuming the reset token and updating sessions"
        );
        assert!(
            matches!(failure, Err(AppError::DatabaseError(_))),
            "{failure:?}"
        );
        fault.disarm().await.unwrap();
        let after = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &registered.user_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.password_hash, before.password_hash);
        assert_eq!(after.password_reset_token, before.password_reset_token);
        assert_eq!(
            after.password_reset_expires_at,
            before.password_reset_expires_at
        );
        for collection in [SESSIONS, REFRESH] {
            assert_eq!(
                db.collection::<bson::Document>(collection)
                    .count_documents(doc! {"user_id": &registered.user_id, "revoked": false})
                    .await
                    .unwrap(),
                2
            );
        }
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .count_documents(
                    doc! {"event.action":{"$in":["password_reset","sessions_revoked"]}}
                )
                .await
                .unwrap(),
            0
        );
        reset_password(&db, &state.audit_runtime, &token, "w8-id-new-password")
            .await
            .unwrap();
        let after = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": &registered.user_id})
            .await
            .unwrap()
            .unwrap();
        assert!(after.password_reset_token.is_none());
        assert!(after.password_reset_expires_at.is_none());
        assert!(
            password::verify_password(
                "w8-id-new-password",
                after.password_hash.as_deref().unwrap()
            )
            .unwrap()
        );
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
                .count_documents(doc! {"user_id": &registered.user_id, "revoked": false})
                .await
                .unwrap(),
            0
        );
        let revoke = db
            .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
            .find_one(doc! {"event.action":"sessions_revoked"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            revoke.event.detail,
            Some(AuditDetail::SessionRevocationCount {
                count: 2,
                saturated: false
            })
        );
        assert_eq!(
            revoke.event.principal,
            AuditPrincipal::Person {
                id: registered.user_id.clone()
            }
        );
        let fresh = super::super::token_service::create_session(
            &db,
            &state.audit_runtime,
            &registered.user_id,
            None,
            None,
        )
        .await
        .unwrap();
        let audit_count = db
            .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap();
        assert!(matches!(
            reset_password(&db, &state.audit_runtime, &token, "w8-id-replay-password").await,
            Err(AppError::BadRequest(_))
        ));
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            audit_count
        );
        assert!(
            !db.collection::<Session>(SESSIONS)
                .find_one(doc! {"_id": &fresh.session_id})
                .await
                .unwrap()
                .unwrap()
                .revoked
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn w8_id_account_lifecycle_c2_tiers_preserve_narrowing() {
        let hash = password::hash_password("w8-id-password").unwrap();
        for (label, age, count, diverged, availability, standard) in [
            ("stale", 0, 1, false, true, true),
            ("standard_age", 7200, 1, false, true, false),
            ("availability_age", 90000, 1, false, false, false),
            ("standard_count", 0, 100000, false, true, false),
            ("availability_count", 0, 1000000, false, false, false),
            ("diverged", 0, 1, true, true, false),
        ] {
            let db = crate::test_utils::connect_transaction_test_database(&format!(
                "w8_id_account_c2_{label}"
            ))
            .await;
            let state = crate::test_utils::test_app_state(db.clone());
            role_service::seed_system_roles(&db).await.unwrap();
            let owner = Uuid::new_v4().to_string();
            let actor = Uuid::new_v4().to_string();
            let mut user = crate::test_utils::test_user(&owner, UserType::Person);
            user.email = format!("{owner}@example.invalid");
            user.password_hash = Some(hash.clone());
            db.collection::<User>(USERS)
                .insert_one(&user)
                .await
                .unwrap();
            let initial = super::super::token_service::create_session_and_issue_tokens(
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
            db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME).update_one(doc! {"singleton":true}, doc! {"$set": {
                "witness_enrolled":true,"pending_count":count as i64,"next_order":count as i64,
                "oldest_pending_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(age))
            }}).await.unwrap();
            if diverged {
                let chain = super::super::audit_state_service::load(&db)
                    .await
                    .unwrap()
                    .id;
                db.collection::<bson::Document>(crate::models::audit_witness_delivery::COLLECTION_NAME).insert_one(doc! {"_id": &chain, "divergence": {"seq":1_i64,"local_hash":"a".repeat(64),"witnessed_hash":"b".repeat(64)}}).await.unwrap();
                state
                    .audit_runtime
                    .restore_incident(&db, &chain)
                    .await
                    .unwrap();
            }
            let reset = initiate_password_reset(&db, &state.audit_runtime, &user.email).await;
            assert_eq!(
                reset.is_ok(),
                availability,
                "{label}: Availability: {:?}",
                reset.as_ref().err()
            );
            let reset_owner = db
                .collection::<User>(USERS)
                .find_one(doc! {"_id": &owner})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                reset_owner.password_reset_token.is_some(),
                availability,
                "{label}: refused issuance must not store a token"
            );
            assert_eq!(
                reset_owner.password_reset_expires_at.is_some(),
                availability
            );
            let registration = register_user(
                &db,
                &state.audit_runtime,
                "w8-id-new-person@example.invalid",
                "w8-id-password",
                None,
                None,
                true,
            )
            .await;
            assert_eq!(
                registration.is_ok(),
                standard,
                "{label}: Standard: {:?}",
                registration.as_ref().err()
            );
            assert_eq!(
                db.collection::<User>(USERS)
                    .count_documents(doc! {"email": "w8-id-new-person@example.invalid"})
                    .await
                    .unwrap(),
                u64::from(standard),
                "{label}: refused registration must not create a person"
            );
            super::super::admin_user_service::set_user_active(
                &db,
                &state.audit_runtime,
                &actor,
                &owner,
                false,
            )
            .await
            .unwrap();
            let after = db
                .collection::<User>(USERS)
                .find_one(doc! {"_id": &owner})
                .await
                .unwrap()
                .unwrap();
            assert!(!after.is_active, "{label}: Narrowing must commit");
            assert!(
                db.collection::<crate::models::session::Session>(
                    crate::models::session::COLLECTION_NAME
                )
                .find_one(doc! {"_id": &initial.session_id})
                .await
                .unwrap()
                .unwrap()
                .revoked
            );
            db.drop().await.unwrap();
        }
    }
}
