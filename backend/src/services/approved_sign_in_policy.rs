//! Account-controlled MFA exception for explicitly approved sign-in protocols.
use bson::{Document, doc};
use mongodb::{ClientSession, Database};

use super::session_authority::{self, DerivedMfaAuthority};
use crate::{
    errors::{AppError, AppResult},
    models::{
        session::{COLLECTION_NAME as SESSIONS, Session},
        user::{COLLECTION_NAME as USERS, SecurityPreferences, User},
    },
};

/// Missing preferences use this generation until the first explicit change.
pub const DEFAULT_REVISION: &str = "00000000-0000-0000-0000-000000000000";

pub fn effective_revision(policy: &SecurityPreferences) -> &str {
    policy.revision.as_deref().unwrap_or(DEFAULT_REVISION)
}

pub fn permits(user: &User, verified: bool, revision: Option<&str>) -> bool {
    let policy = &user.profile_config.security;
    match revision {
        Some(revision) => {
            policy.skip_mfa_for_approved_sign_ins && effective_revision(policy) == revision
        }
        None => session_authority::may_derive(user.mfa_enabled, verified),
    }
}

/// Resolve the live account policy; a bearer without a session cannot use it.
pub async fn authorize(
    db: &Database,
    actor: &str,
    authority: DerivedMfaAuthority,
) -> AppResult<Option<String>> {
    if authority.mfa_enabled.is_none() {
        return Err(AppError::Forbidden(
            "A first-party human session is required".into(),
        ));
    }
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": actor, "is_active": true})
        .await?
        .ok_or_else(|| AppError::Unauthorized("Account is unavailable".into()))?;
    super::auth_service::ensure_person_user(&user)?;
    if let Some(expected) = authority.app_policy_revision
        && !permits(&user, false, Some(&expected.to_string()))
    {
        return Err(AppError::MfaStepUpRequired);
    }
    if session_authority::may_derive(user.mfa_enabled, authority.verified) {
        return Ok(None);
    }
    let policy = &user.profile_config.security;
    if !policy.skip_mfa_for_approved_sign_ins {
        return Err(AppError::MfaStepUpRequired);
    }
    let revision = effective_revision(policy);
    if let Some(expected) = authority.app_policy_revision {
        if expected.to_string() != revision {
            return Err(AppError::MfaStepUpRequired);
        }
    } else if let Some(id) = authority.session_id {
        let parent = db
            .collection::<Session>(SESSIONS)
            .find_one(session_authority::authenticated(
                doc! {"_id": id.to_string(), "user_id": actor,
                "revoked": false, "expires_at": {"$gt": bson::DateTime::now()}},
            ))
            .await?;
        if !parent
            .as_ref()
            .is_some_and(session_authority::satisfies_mfa)
        {
            return Err(AppError::MfaStepUpRequired);
        }
    } else {
        return Err(AppError::MfaStepUpRequired);
    }
    Ok(Some(revision.to_owned()))
}

/// This write and the final grant claim must commit together. A timestamp can
/// be a no-op within one millisecond; a fresh nonce guarantees a write conflict
/// with a concurrent preference change even at that resolution.
pub async fn fence(
    db: &Database,
    actor: &str,
    revision: Option<&str>,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let Some(revision) = revision else {
        return Ok(());
    };
    let mut filter = doc! {
        "_id": actor, "is_active": true, "user_type": {"$ne": "org"},
        "$or": [
            {"profile_config.security.skip_mfa_for_approved_sign_ins": true},
            {"profile_config.security.skip_mfa_for_approved_sign_ins": {"$exists": false}},
        ],
    };
    if revision == DEFAULT_REVISION {
        filter.insert("profile_config.security.revision", bson::Bson::Null);
    } else {
        filter.insert("profile_config.security.revision", revision);
    }
    let result = db
        .collection::<Document>(USERS)
        .update_one(
            filter,
            doc! {"$set": {"profile_config.security.use_nonce": uuid::Uuid::new_v4().to_string()}},
        )
        .session(&mut *tx)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::AuthDeviceCodeDenied);
    }
    Ok(())
}

pub async fn fence_approval(
    db: &Database,
    actor: &str,
    authority: DerivedMfaAuthority,
    revision: Option<&str>,
    tx: &mut ClientSession,
) -> AppResult<()> {
    fence(db, actor, revision, tx)
        .await
        .map_err(|error| match error {
            AppError::AuthDeviceCodeDenied => AppError::MfaStepUpRequired,
            error => error,
        })?;
    if revision.is_some()
        && let Some(id) = authority.session_id
    {
        let result = db
            .collection::<Session>(SESSIONS)
            .update_one(
                session_authority::authenticated(doc! {"_id": id.to_string(), "user_id": actor,
                "revoked": false, "expires_at": {"$gt": bson::DateTime::now()}}),
                doc! {"$set": {"approved_sign_in_nonce": uuid::Uuid::new_v4().to_string()}},
            )
            .session(&mut *tx)
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::MfaStepUpRequired);
        }
    }
    Ok(())
}

pub async fn save(
    db: &Database,
    actor: &str,
    enabled: bool,
    authority: DerivedMfaAuthority,
    audit: &super::audit_service::AuditActor,
) -> AppResult<SecurityPreferences> {
    let context = (
        db.clone(),
        actor.to_owned(),
        enabled,
        authority,
        audit.clone(),
    );
    super::transaction_service::run_db_only(db, &context, |tx, c| Box::pin(async move {
        let (db, actor, enabled, authority, audit) = c;
        let user = db.collection::<User>(USERS).find_one(doc! {"_id": actor, "is_active": true})
            .session(&mut *tx).await?.ok_or_else(|| AppError::Unauthorized("Account is unavailable".into()))?;
        super::auth_service::ensure_person_user(&user)?;
        if user.profile_config.security.skip_mfa_for_approved_sign_ins == *enabled {
            return Ok(user.profile_config.security);
        }
        if *enabled && user.mfa_enabled {
            session_authority::require_derived_mfa(DerivedMfaAuthority::new(true, authority.verified))?;
            let id = authority.session_id.ok_or(AppError::MfaStepUpRequired)?;
            let result = db.collection::<Session>(SESSIONS).update_one(
                session_authority::authenticated(doc! {"_id": id.to_string(), "user_id": actor,
                    "mfa_assured": true, "revoked": false, "expires_at": {"$gt": bson::DateTime::now()}}),
                doc! {"$set": {"security_preference_nonce": uuid::Uuid::new_v4().to_string()}})
                .session(&mut *tx).await?;
            if result.matched_count != 1 { return Err(AppError::MfaStepUpRequired); }
        }
        let mut policy = user.profile_config.security;
        policy.skip_mfa_for_approved_sign_ins = *enabled;
        policy.revision = Some(uuid::Uuid::new_v4().to_string());
        policy.changed_at = Some(chrono::Utc::now());
        db.collection::<Document>(USERS).update_one(doc! {"_id": actor}, doc! {"$set": {
            "profile_config.security.skip_mfa_for_approved_sign_ins": enabled,
            "profile_config.security.revision": &policy.revision,
            "profile_config.security.changed_at": bson::DateTime::from_chrono(policy.changed_at.unwrap()),
        }}).session(&mut *tx).await?;
        super::audit_service::log_actor_event_in_transaction(db, tx, audit, "security_setting_changed",
            serde_json::json!({"setting": "skip_mfa_for_approved_sign_ins", "enabled": enabled})).await?;
        Ok(policy)
    })).await
}

/// Remove undelivered authority after a revoked policy is observed. The claim
/// prevents a losing poller from revoking a session already delivered elsewhere.
pub async fn deny_undelivered(
    db: &Database,
    collection: &str,
    id: &str,
    revision: &str,
) -> AppResult<()> {
    let context = (
        db.clone(),
        collection.to_owned(),
        id.to_owned(),
        revision.to_owned(),
    );
    super::transaction_service::run_db_only(db, &context, |tx, c| {
        Box::pin(async move {
            let (db, collection, id, revision) = c;
            let row = db.collection::<Document>(collection).find_one_and_update(
            doc! {"_id": id, "status": "approved", "mfa_policy_revision": revision},
            doc! {"$set": {"status": "denied", "denied_at": bson::DateTime::now(),
                "purge_at": bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::days(1))}, "$unset": {
                "delivery_access_token_encrypted": "", "delivery_refresh_token_encrypted": "",
                "delivery_credential_encrypted": "", "delivery_context_version": "",
                "agent_key_grant.delivery_credential_encrypted": ""}})
            .session(&mut *tx).await?;
            let Some(row) = row else {
                return Ok(());
            };
            if let Ok(id) = row.get_str("approved_session_id") {
                super::token_service::revoke_session_in_transaction(db, id, tx).await?;
            }
            let credential = row.get_str("credential_id").ok().or_else(|| {
                row.get_document("agent_key_grant")
                    .ok()?
                    .get_str("credential_id")
                    .ok()
            });
            if let Some(id) = credential {
                super::api_key_credential_service::revoke_in_session(
                    db,
                    id,
                    crate::models::api_key_credential::CredentialRevokedReason::UndeliveredExpired,
                    tx,
                )
                .await?;
            }
            if let Ok(actor) = row.get_str("approved_user_id") {
                super::audit_service::log_actor_event_in_transaction(db, tx,
                    &super::audit_service::AuditActor { user_id: actor.into(), ip_address: None,
                        user_agent: None, api_key_id: None, api_key_name: None },
                    "approved_sign_in_grant_revoked",
                    serde_json::json!({"request_id": id, "protocol": collection})).await?;
            }
            Ok(())
        })
    })
    .await
}
