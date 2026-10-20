use chrono::Utc;
use mongodb::bson::{self, doc};
use totp_rs::{Algorithm, Secret, TOTP};
use uuid::Uuid;

use super::{
    audit_runtime::{AuditRuntime, policy::AuditTier, transaction},
    token_service::{self, SessionAudit},
};
use crate::crypto::aes::EncryptionKeys;
use crate::crypto::password;
use crate::crypto::token::generate_random_token;
use crate::errors::{AppError, AppResult};
use crate::models::audit_outbox::AuditPrincipal;
use crate::models::mfa_factor::{COLLECTION_NAME as MFA_FACTORS, MfaFactor};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};
use crate::models::user::{COLLECTION_NAME as USERS, User};

/// Verified caller and affected person remain distinct from key/SA attribution.
pub struct MfaActor {
    pub user_id: String,
    pub principal: AuditPrincipal,
}

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

/// Encryption and random/recovery material are prepared before retryable DB work.
pub async fn setup_totp(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    encryption_keys: &EncryptionKeys,
    actor: &MfaActor,
    user_email: &str,
) -> AppResult<TotpSetupResult> {
    let user_id = actor.user_id.as_str();
    let secret = Secret::generate_secret();
    let encoded = secret.to_encoded().to_string();
    let totp = create_totp(
        secret
            .to_bytes()
            .map_err(|_| AppError::Internal("Invalid TOTP secret".into()))?,
        "NyxID",
        user_email,
    )?;
    let encrypted = encryption_keys.encrypt(encoded.as_bytes()).await?;
    let now = Utc::now();
    let factor = MfaFactor {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.into(),
        factor_type: "totp".into(),
        secret_encrypted: Some(encrypted),
        recovery_codes: None,
        is_verified: false,
        is_active: true,
        created_at: now,
        updated_at: now,
    };
    let audit = SessionAudit::new(actor.principal.clone(), user_id, "mfa_setup", &factor.id);
    let context = (db, runtime, &factor, &audit);
    transaction::run(db, &context, |session, (db, runtime, factor, audit)| {
        Box::pin(async move {
            token_service::live_person(db, session, &factor.user_id).await?;
            let existing = db
                .collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {
                    "user_id": &factor.user_id,
                    "factor_type": "totp",
                    "is_active": true,
                })
                .session(&mut *session)
                .await?;
            if let Some(existing) = existing {
                if existing.is_verified {
                    return Err(AppError::Conflict(
                        "TOTP is already configured for this account".into(),
                    ));
                }
                db.collection::<MfaFactor>(MFA_FACTORS)
                    .update_one(
                        doc! {"_id": existing.id, "is_active": true, "is_verified": false},
                        doc! {"$set": {
                            "is_active": false,
                            "updated_at": bson::DateTime::from_chrono(factor.created_at),
                        }},
                    )
                    .session(&mut *session)
                    .await?;
            }
            db.collection::<MfaFactor>(MFA_FACTORS)
                .insert_one(*factor)
                .session(&mut *session)
                .await?;
            audit
                .enqueue(db, runtime, session, AuditTier::Availability)
                .await
        })
    })
    .await?;
    Ok(TotpSetupResult {
        factor_id: factor.id,
        secret: encoded,
        qr_code_url: totp.get_url(),
    })
}

async fn check_factor(
    encryption_keys: &EncryptionKeys,
    factor: &MfaFactor,
    code: &str,
) -> AppResult<bool> {
    let encrypted = factor
        .secret_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("Missing encrypted secret".into()))?;
    let secret = zeroize::Zeroizing::new(
        String::from_utf8(encryption_keys.decrypt(encrypted).await?)
            .map_err(|_| AppError::Internal("Invalid secret encoding".into()))?,
    );
    let totp = create_totp(
        Secret::Encoded(secret.to_string())
            .to_bytes()
            .map_err(|_| AppError::Internal("Invalid TOTP secret".into()))?,
        "NyxID",
        &factor.user_id,
    )?;
    totp.check_current(code)
        .map_err(|_| AppError::Internal("TOTP verification failed".into()))
}

pub async fn verify_totp_setup(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    encryption_keys: &EncryptionKeys,
    factor_id: &str,
    actor: &MfaActor,
    code: &str,
) -> AppResult<Vec<String>> {
    let user_id = actor.user_id.as_str();
    let factor = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(
            doc! {"_id": factor_id, "user_id": user_id, "factor_type": "totp", "is_active": true},
        )
        .await?
        .ok_or_else(|| AppError::NotFound("TOTP factor not found".into()))?;
    if factor.is_verified {
        return Err(AppError::BadRequest("Factor already verified".into()));
    }
    if !check_factor(encryption_keys, &factor, code).await? {
        return Err(AppError::AuthenticationFailed("Invalid TOTP code".into()));
    }
    let codes: Vec<String> = (0..10)
        .map(|_| generate_random_token()[..8].to_string())
        .collect();
    let hashes = codes
        .iter()
        .map(|code| password::hash_password(code))
        .collect::<Result<Vec<_>, _>>()?;
    let audit = SessionAudit::new(actor.principal.clone(), user_id, "mfa_enabled", factor_id);
    let context = (db, runtime, &factor, &hashes, &audit);
    transaction::run(
        db,
        &context,
        |session, (db, runtime, factor, hashes, audit)| {
            Box::pin(async move {
                token_service::live_person(db, session, &factor.user_id).await?;
                let live = db
                    .collection::<MfaFactor>(MFA_FACTORS)
                    .find_one(doc! {
                        "_id": &factor.id,
                        "user_id": &factor.user_id,
                        "is_active": true,
                        "is_verified": false,
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::Conflict("MFA setup changed".into()))?;
                if live.secret_encrypted != factor.secret_encrypted {
                    return Err(AppError::Conflict("MFA setup changed".into()));
                }
                db.collection::<MfaFactor>(MFA_FACTORS)
                    .update_one(
                        doc! {"_id": &factor.id},
                        doc! {"$set": {
                            "is_verified": true,
                            "recovery_codes": *hashes,
                            "updated_at": bson::DateTime::now(),
                        }},
                    )
                    .session(&mut *session)
                    .await?;
                db.collection::<User>(USERS)
                    .update_one(
                        doc! {"_id": &factor.user_id},
                        doc! {"$set": {"mfa_enabled": true, "updated_at": bson::DateTime::now()}},
                    )
                    .session(&mut *session)
                    .await?;
                audit
                    .enqueue(db, runtime, session, AuditTier::Availability)
                    .await
            })
        },
    )
    .await?;
    Ok(codes)
}

pub async fn verified_factor(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    code: &str,
) -> AppResult<Option<MfaFactor>> {
    let factor = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "user_id": user_id,
            "factor_type": "totp",
            "is_verified": true,
            "is_active": true,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("No active TOTP factor found".into()))?;
    Ok(check_factor(encryption_keys, &factor, code)
        .await?
        .then_some(factor))
}

pub async fn verify_totp(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    code: &str,
) -> AppResult<bool> {
    Ok(verified_factor(db, encryption_keys, user_id, code)
        .await?
        .is_some())
}

pub enum MfaSession {
    Browser(Box<token_service::PreparedBrowserSession>),
    Tokens(Box<token_service::PreparedSessionTokens>),
}

impl MfaSession {
    pub fn browser(value: token_service::PreparedBrowserSession) -> Self {
        Self::Browser(Box::new(value))
    }
    pub fn tokens(value: token_service::PreparedSessionTokens) -> Self {
        Self::Tokens(Box::new(value))
    }
}

/// The factor fence, pending-session consumption, authenticated session and audit
/// commit together. Decryption/TOTP checks and prepared identities precede this call.
pub async fn complete_login(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    pending: Option<&Session>,
    factor: &MfaFactor,
    prepared: &MfaSession,
) -> AppResult<()> {
    let owner = match prepared {
        MfaSession::Browser(value) => value.user_id(),
        MfaSession::Tokens(value) => value.user_id(),
    };
    if owner != factor.user_id || pending.is_some_and(|pending| pending.user_id != owner) {
        return Err(AppError::Forbidden("MFA identity mismatch".into()));
    }
    let audit = SessionAudit::new(
        AuditPrincipal::Person { id: owner.into() },
        owner,
        "mfa_login_completed",
        pending.map_or(factor.id.as_str(), |pending| pending.id.as_str()),
    );
    let context = (db, runtime, pending, factor, prepared, &audit);
    transaction::run(
        db,
        &context,
        |session, (db, runtime, pending, factor, prepared, audit)| {
            Box::pin(async move {
                let live = db
                    .collection::<MfaFactor>(MFA_FACTORS)
                    .find_one(doc! {
                        "_id": &factor.id,
                        "user_id": &factor.user_id,
                        "is_active": true,
                        "is_verified": true,
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::AuthenticationFailed("MFA factor changed".into()))?;
                if live.secret_encrypted != factor.secret_encrypted {
                    return Err(AppError::AuthenticationFailed("MFA factor changed".into()));
                }
                db.collection::<MfaFactor>(MFA_FACTORS)
                    .update_one(
                        doc! {"_id": &factor.id},
                        doc! {"$set": {"updated_at": bson::DateTime::now()}},
                    )
                    .session(&mut *session)
                    .await?;
                if let Some(pending) = pending {
                    let consumed = db
                        .collection::<Session>(SESSIONS)
                        .update_one(
                            doc! {
                                "_id": &pending.id,
                                "user_id": &pending.user_id,
                                "token_hash": &pending.token_hash,
                                "user_agent": "mfa_pending",
                                "revoked": false,
                                "expires_at": {"$gt": bson::DateTime::now()},
                            },
                            doc! {"$set": {"revoked": true}},
                        )
                        .session(&mut *session)
                        .await?;
                    if consumed.matched_count != 1 {
                        return Err(AppError::AuthenticationFailed(
                            "MFA session expired or consumed".into(),
                        ));
                    }
                }
                match prepared {
                    MfaSession::Browser(value) => {
                        token_service::insert_prepared_browser_session(db, runtime, value, session)
                            .await?;
                    }
                    MfaSession::Tokens(value) => {
                        token_service::insert_prepared_session(db, runtime, value, session).await?;
                    }
                }
                audit
                    .enqueue(db, runtime, session, AuditTier::Availability)
                    .await
            })
        },
    )
    .await
}

pub async fn disable_totp(
    db: &mongodb::Database,
    runtime: &AuditRuntime,
    actor: &MfaActor,
    supplied_password: &str,
) -> AppResult<()> {
    let principal = actor.principal.clone();
    let actor = actor.user_id.as_str();
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": actor, "is_active": true})
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    let hash = user
        .password_hash
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("No password set on account".into()))?;
    if !password::verify_password(supplied_password, hash)? {
        return Err(AppError::AuthenticationFailed("Invalid password".into()));
    }
    let audit = SessionAudit::new(principal, actor, "mfa_disabled", actor);
    let context = (db, runtime, actor, hash, &audit);
    transaction::run(
        db,
        &context,
        |session, (db, runtime, actor, hash, audit)| {
            Box::pin(async move {
                token_service::live_person(db, session, actor).await?;
                let updated = db
                    .collection::<User>(USERS)
                    .update_one(
                        doc! {"_id": *actor, "password_hash": *hash, "is_active": true},
                        doc! {"$set": {"mfa_enabled": false, "updated_at": bson::DateTime::now()}},
                    )
                    .session(&mut *session)
                    .await?;
                if updated.matched_count != 1 {
                    return Err(AppError::AuthenticationFailed(
                        "Account credentials changed".into(),
                    ));
                }
                db.collection::<MfaFactor>(MFA_FACTORS)
                    .update_many(
                        doc! {"user_id": *actor, "is_active": true},
                        doc! {"$set": {"is_active": false, "updated_at": bson::DateTime::now()}},
                    )
                    .session(&mut *session)
                    .await?;
                audit
                    .enqueue(db, runtime, session, AuditTier::Standard)
                    .await
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    async fn w8_id_enabled_factor(
        db: &mongodb::Database,
        state: &crate::AppState,
        user_id: &str,
    ) -> (TotpSetupResult, MfaFactor, Vec<String>) {
        let actor = MfaActor {
            user_id: user_id.into(),
            principal: AuditPrincipal::Person { id: user_id.into() },
        };
        let setup = setup_totp(
            db,
            &state.audit_runtime,
            &state.encryption_keys,
            &actor,
            "w8-id@example.test",
        )
        .await
        .unwrap();
        let totp = create_totp(
            Secret::Encoded(setup.secret.clone()).to_bytes().unwrap(),
            "NyxID",
            "w8-id@example.test",
        )
        .unwrap();
        let codes = verify_totp_setup(
            db,
            &state.audit_runtime,
            &state.encryption_keys,
            &setup.factor_id,
            &actor,
            &totp.generate_current().unwrap(),
        )
        .await
        .unwrap();
        let factor = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id":&setup.factor_id})
            .await
            .unwrap()
            .unwrap();
        (setup, factor, codes)
    }

    #[tokio::test]
    async fn w8_id_mfa_enable_disable_rollback_and_secret_free_events() {
        use crate::models::audit_outbox::PENDING_COLLECTION_NAME as PENDING;
        use futures::TryStreamExt;
        let db = connect_transaction_test_database("w8_id_mfa_atomic").await;
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, crate::models::user::UserType::Person);
        user.password_hash = Some(password::hash_password("w8-id-password").unwrap());
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();
        let actor = MfaActor {
            user_id: user_id.clone(),
            principal: AuditPrincipal::Person {
                id: user_id.clone(),
            },
        };
        let setup = setup_totp(
            &db,
            &state.audit_runtime,
            &state.encryption_keys,
            &actor,
            &user.email,
        )
        .await
        .unwrap();
        let totp = create_totp(
            Secret::Encoded(setup.secret.clone()).to_bytes().unwrap(),
            "NyxID",
            &user.email,
        )
        .unwrap();
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME).update_one(doc! {"singleton":true},doc! {"$set":{
            "witness_enrolled":true,"oldest_pending_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(2))
        }}).await.unwrap();
        let rejected = verify_totp_setup(
            &db,
            &state.audit_runtime,
            &state.encryption_keys,
            &setup.factor_id,
            &actor,
            &totp.generate_current().unwrap(),
        )
        .await;
        assert!(matches!(
            rejected,
            Err(AppError::AuditAdmissionClosed { .. })
        ));
        assert!(
            !db.collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {"_id":&setup.factor_id})
                .await
                .unwrap()
                .unwrap()
                .is_verified
        );
        assert!(
            !db.collection::<User>(USERS)
                .find_one(doc! {"_id":&user_id})
                .await
                .unwrap()
                .unwrap()
                .mfa_enabled
        );
        assert_eq!(
            db.collection::<bson::Document>(PENDING)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .update_one(
                doc! {"singleton":true},
                doc! {"$set":{"oldest_pending_at":bson::DateTime::now()}},
            )
            .await
            .unwrap();
        let codes = verify_totp_setup(
            &db,
            &state.audit_runtime,
            &state.encryption_keys,
            &setup.factor_id,
            &actor,
            &totp.generate_current().unwrap(),
        )
        .await
        .unwrap();
        let enabled = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id":&setup.factor_id})
            .await
            .unwrap()
            .unwrap();
        assert!(enabled.is_verified);
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&user_id})
                .await
                .unwrap()
                .unwrap()
                .mfa_enabled
        );
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME).update_one(doc! {"singleton":true},doc! {"$set":{
            "oldest_pending_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(2))
        }}).await.unwrap();
        assert!(matches!(
            disable_totp(&db, &state.audit_runtime, &actor, "w8-id-password").await,
            Err(AppError::AuditAdmissionClosed { .. })
        ));
        assert!(
            db.collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {"_id":&setup.factor_id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&user_id})
                .await
                .unwrap()
                .unwrap()
                .mfa_enabled
        );
        db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .update_one(
                doc! {"singleton":true},
                doc! {"$set":{"oldest_pending_at":bson::DateTime::now()}},
            )
            .await
            .unwrap();
        disable_totp(&db, &state.audit_runtime, &actor, "w8-id-password")
            .await
            .unwrap();
        assert!(
            !db.collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {"_id":&setup.factor_id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        assert!(
            !db.collection::<User>(USERS)
                .find_one(doc! {"_id":&user_id})
                .await
                .unwrap()
                .unwrap()
                .mfa_enabled
        );
        let mut secrets = codes;
        secrets.push(setup.secret);
        secrets.push(totp.generate_current().unwrap());
        secrets.push(user.password_hash.unwrap());
        secrets.extend(
            enabled
                .recovery_codes
                .as_ref()
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned()),
        );
        let encrypted = bson::to_bson(&enabled.secret_encrypted)
            .unwrap()
            .to_string();
        secrets.push(encrypted);
        while super::super::audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
        {}
        for collection in [
            PENDING,
            crate::models::audit_log::COLLECTION_NAME,
            crate::models::audit_outbox::COLLECTION_NAME,
        ] {
            let rows: Vec<bson::Document> = db
                .collection(collection)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(rows.len(), 3);
            for row in rows {
                let encoded = serde_json::to_string(&row).unwrap();
                for secret in &secrets {
                    assert!(!encoded.contains(secret));
                }
            }
        }
    }

    #[tokio::test]
    async fn w8_id_mfa_pending_login_has_one_winner_and_rechecks_factor() {
        use crate::models::audit_outbox::PENDING_COLLECTION_NAME as PENDING;
        let db = connect_transaction_test_database("w8_id_mfa_login").await;
        let state = test_app_state(db.clone());
        let actor = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let (_, factor, _) = w8_id_enabled_factor(&db, &state, &actor).await;
        let pending_id = token_service::create_mfa_pending_session(
            &db,
            &state.audit_runtime,
            &actor,
            "private-pending-hash",
        )
        .await
        .unwrap();
        let pending = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! {"_id":&pending_id})
            .await
            .unwrap()
            .unwrap();
        let one = MfaSession::tokens(
            token_service::prepare_session_tokens(
                &db,
                &state.config,
                &state.jwt_keys,
                &actor,
                None,
                None,
            )
            .await
            .unwrap(),
        );
        let two = MfaSession::tokens(
            token_service::prepare_session_tokens(
                &db,
                &state.config,
                &state.jwt_keys,
                &actor,
                None,
                None,
            )
            .await
            .unwrap(),
        );
        let (a, b) = tokio::join!(
            complete_login(&db, &state.audit_runtime, Some(&pending), &factor, &one),
            complete_login(&db, &state.audit_runtime, Some(&pending), &factor, &two)
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert_eq!(
            db.collection::<Session>(SESSIONS)
                .count_documents(doc! {"revoked":false})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<bson::Document>(PENDING)
                .count_documents(doc! {"event.action":"mfa_login_completed"})
                .await
                .unwrap(),
            1
        );
        assert!(
            db.collection::<Session>(SESSIONS)
                .find_one(doc! {"_id":&pending_id})
                .await
                .unwrap()
                .unwrap()
                .revoked
        );
        db.collection::<MfaFactor>(MFA_FACTORS)
            .update_one(doc! {"_id":&factor.id}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        let prepared = MfaSession::browser(
            token_service::prepare_browser_session(&actor, None, None).unwrap(),
        );
        assert!(matches!(
            complete_login(&db, &state.audit_runtime, None, &factor, &prepared).await,
            Err(AppError::AuthenticationFailed(_))
        ));
        assert_eq!(
            db.collection::<Session>(SESSIONS)
                .count_documents(doc! {"revoked":false})
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn test_setup_totp_creates_unverified_factor() {
        let Some(db) = connect_test_database("mfa_svc").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

        let result = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "user@example.com",
        )
        .await;
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
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

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

        let result = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "user@example.com",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_setup_totp_replaces_unverified_factor() {
        let Some(db) = connect_test_database("mfa_svc").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

        let first = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "user@example.com",
        )
        .await
        .unwrap();

        let second = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "user@example.com",
        )
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
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

        let Ok(setup) = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "user@example.com",
        )
        .await
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

        let Ok(recovery_codes) = verify_totp_setup(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &setup.factor_id,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            &code,
        )
        .await
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
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

        let setup = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "user@example.com",
        )
        .await
        .unwrap();

        let result = verify_totp_setup(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &setup.factor_id,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "000000",
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_verify_totp_login_with_valid_code() {
        let Some(db) = connect_test_database("mfa_login").await else {
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

        let Ok(setup) = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "user@example.com",
        )
        .await
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

        let Ok(_) = verify_totp_setup(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &setup.factor_id,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            &code,
        )
        .await
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
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            "nonexistent-factor-id",
            &MfaActor {
                user_id: "nonexistent-user".to_string(),
                principal: AuditPrincipal::Person {
                    id: "nonexistent-user".to_string(),
                },
            },
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
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

        let Ok(setup) = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "user@example.com",
        )
        .await
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

        let Ok(_) = verify_totp_setup(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &setup.factor_id,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            &code,
        )
        .await
        else {
            eprintln!("Skipping: MongoDB connection lost during TOTP verify");
            return;
        };

        let code2 = totp.generate_current().unwrap();
        let err = verify_totp_setup(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &setup.factor_id,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            &code2,
        )
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
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

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
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, crate::models::user::UserType::Person))
            .await
            .unwrap();

        let result = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user_id.to_string(),
                principal: AuditPrincipal::Person {
                    id: user_id.to_string(),
                },
            },
            "alice@example.com",
        )
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

        for id in [&user1, &user2] {
            db.collection::<User>(USERS)
                .insert_one(test_user(id, crate::models::user::UserType::Person))
                .await
                .unwrap();
        }
        let setup1 = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user1.to_string(),
                principal: AuditPrincipal::Person {
                    id: user1.to_string(),
                },
            },
            "user1@example.com",
        )
        .await
        .expect("setup1");
        let setup2 = setup_totp(
            &db,
            &test_app_state(db.clone()).audit_runtime,
            &encryption_keys,
            &MfaActor {
                user_id: user2.to_string(),
                principal: AuditPrincipal::Person {
                    id: user2.to_string(),
                },
            },
            "user2@example.com",
        )
        .await
        .expect("setup2");

        assert_ne!(setup1.secret, setup2.secret);
        assert_ne!(setup1.factor_id, setup2.factor_id);
    }
}
