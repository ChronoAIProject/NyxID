use chrono::Utc;
use mongodb::bson::{self, doc};
use totp_rs::{Algorithm, Secret, TOTP};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::password;
use crate::crypto::token::generate_random_token;
use crate::errors::{AppError, AppResult};
use crate::models::mfa_factor::{COLLECTION_NAME as MFA_FACTORS, MfaFactor};

/// Result from initiating TOTP setup.
pub struct TotpSetupResult {
    pub factor_id: String,
    pub secret: Zeroizing<String>,
    pub qr_code_url: Zeroizing<String>,
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
    .map_err(|_| AppError::Internal("Failed to create TOTP".into()))
}

async fn encrypt_factor_secret(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    plaintext: &[u8],
) -> AppResult<Vec<u8>> {
    if keys.contextual_record_writes() {
        let field = ["secret_encrypted"];
        let context = crate::crypto::aes::RecordContext::new(MFA_FACTORS, id, owner, &field)?;
        keys.encrypt_record(plaintext, &context).await
    } else {
        keys.encrypt(plaintext).await
    }
}

async fn decrypt_factor_secret(
    keys: &EncryptionKeys,
    factor: &MfaFactor,
) -> AppResult<Zeroizing<Vec<u8>>> {
    if factor.factor_type != "totp" {
        return Err(AppError::Internal(
            "Unsupported encrypted MFA factor".into(),
        ));
    }
    let encrypted = factor
        .secret_encrypted
        .as_deref()
        .ok_or_else(|| AppError::Internal("Missing MFA secret".into()))?;
    let field = ["secret_encrypted"];
    let context =
        crate::crypto::aes::RecordContext::new(MFA_FACTORS, &factor.id, &factor.user_id, &field)?;
    let plaintext = match factor.secret_context_version {
        Some(3) => keys.decrypt_record(encrypted, &context).await?,
        None if keys.allow_legacy_record_reads() => Zeroizing::new(keys.decrypt(encrypted).await?),
        _ => {
            return Err(AppError::Internal(
                "MFA secret format is unavailable".into(),
            ));
        }
    };
    // Readiness requires a usable TOTP seed, including the library's key-size
    // validation. Authentication of an empty or malformed secret is insufficient.
    if plaintext.len() > 1024 {
        return Err(AppError::Internal("Invalid MFA secret".into()));
    }
    let text = std::str::from_utf8(&plaintext)
        .map_err(|_| AppError::Internal("Invalid MFA secret".into()))?;
    let secret = Secret::Encoded(text.to_owned());
    let _totp = create_totp(
        secret
            .to_bytes()
            .map_err(|_| AppError::Internal("Invalid MFA secret".into()))?,
        "NyxID",
        &factor.user_id,
    )?;
    Ok(plaintext)
}

pub(crate) async fn verify_current_secret(
    keys: &EncryptionKeys,
    factor: &MfaFactor,
) -> AppResult<()> {
    if factor.secret_context_version != Some(3) {
        return Err(AppError::Conflict(
            "MFA secret requires contextual migration".into(),
        ));
    }
    let field = ["secret_encrypted"];
    let context =
        crate::crypto::aes::RecordContext::new(MFA_FACTORS, &factor.id, &factor.user_id, &field)?;
    keys.authenticate_current_record(
        factor
            .secret_encrypted
            .as_deref()
            .ok_or_else(|| AppError::Internal("Missing MFA secret".into()))?,
        &context,
    )
    .await?;
    drop(decrypt_factor_secret(keys, factor).await?);
    Ok(())
}

pub(crate) async fn migrate_secret_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &bson::Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let factor: MfaFactor = bson::from_document(source.clone())
        .map_err(|_| AppError::Internal("Invalid MFA storage".into()))?;
    let plaintext = decrypt_factor_secret(keys, &factor).await?;
    let encrypted = encrypt_factor_secret(keys, &factor.id, &factor.user_id, &plaintext).await?;
    let mut target = factor.clone();
    target.secret_encrypted = Some(encrypted);
    target.secret_context_version = Some(3);
    verify_current_secret(keys, &target).await?;
    let replacement =
        bson::to_document(&target).map_err(|_| AppError::Internal("Invalid MFA storage".into()))?;
    let db = super::transaction_service::durable_database(db);
    let mut tx = super::erasure_admission::begin_write(&db, &factor.user_id).await?;
    let predicate = super::record_storage::source_predicate(
        source,
        &[
            "user_id",
            "secret_encrypted",
            "secret_context_version",
            "factor_type",
            "is_verified",
            "is_active",
            "last_totp_counter",
            "recovery_codes",
            "updated_at",
        ],
    );
    // Factor state/counter/recovery authority is preserved. A concurrent login,
    // enrollment, disable or replacement wins over this exact snapshot.
    let result = db.collection::<bson::Document>(MFA_FACTORS).update_one(
        predicate,
        doc! {"$set":{"secret_encrypted":replacement.get("secret_encrypted").cloned(),"secret_context_version":3_i32}},
    ).session(&mut tx).await.map(|r|r.matched_count==1).map_err(AppError::from);
    super::org_membership_authority::finish(tx, result).await
}

/// Start TOTP enrollment for a user.
///
/// Generates a new TOTP secret, encrypts it, and stores it as an unverified
/// factor. Returns the secret and QR code URL for the authenticator app.
pub async fn setup_totp(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    user_email: &str,
) -> AppResult<TotpSetupResult> {
    let secret = Secret::generate_secret();
    let secret_base32 = Zeroizing::new(secret.to_encoded().to_string());

    let totp = create_totp(
        secret
            .to_bytes()
            .map_err(|_| AppError::Internal("Invalid MFA secret".into()))?,
        "NyxID",
        user_email,
    )?;

    let qr_code_url = Zeroizing::new(totp.get_url());

    let factor_id = Uuid::new_v4().to_string();
    let encrypted_secret = encrypt_factor_secret(
        encryption_keys,
        &factor_id,
        user_id,
        secret_base32.as_bytes(),
    )
    .await?;
    let now = Utc::now();

    let new_factor = MfaFactor {
        id: factor_id.clone(),
        user_id: user_id.to_string(),
        factor_type: "totp".to_string(),
        secret_encrypted: Some(encrypted_secret),
        secret_context_version: encryption_keys.contextual_record_writes().then_some(3),
        recovery_codes: None,
        last_totp_counter: None,
        is_verified: false,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    fence_live_account(db, &mut session, user_id, None).await?;
    super::session_authority::revoke_pending(db, user_id, &mut session).await?;
    if db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "user_id": user_id, "factor_type": "totp", "is_active": true, "is_verified": true
        })
        .session(&mut session)
        .await
        .map_err(mfa_db_error)?
        .is_some()
    {
        session.abort_transaction().await?;
        return Err(AppError::Conflict(
            "TOTP is already configured for this account".into(),
        ));
    }
    db.collection::<MfaFactor>(MFA_FACTORS).update_many(
        doc! {"user_id": user_id, "factor_type": "totp", "is_active": true, "is_verified": false},
        doc! {"$set": {"is_active": false, "updated_at": bson::DateTime::now()}}
    ).session(&mut session).await.map_err(mfa_db_error)?;
    db.collection::<MfaFactor>(MFA_FACTORS)
        .insert_one(&new_factor)
        .session(&mut session)
        .await
        .map_err(mfa_db_error)?;
    commit_mfa(&mut session).await?;

    Ok(TotpSetupResult {
        factor_id,
        secret: secret_base32,
        qr_code_url,
    })
}

/// Verify a TOTP code to complete enrollment.
pub async fn verify_totp_setup(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    factor_id: &str,
    user_id: &str,
    code: &str,
    current_session_id: Option<&str>,
    actor: Option<&super::audit_service::AuditActor>,
) -> AppResult<Zeroizing<Vec<String>>> {
    let factor = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "_id": factor_id,
            "user_id": user_id,
            "factor_type": "totp",
            "is_active": true,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("TOTP factor not found".to_string()))?;

    if factor.is_verified {
        return Err(AppError::BadRequest("Factor already verified".to_string()));
    }

    let secret_bytes = decrypt_factor_secret(encryption_keys, &factor).await?;
    let secret_str = std::str::from_utf8(&secret_bytes)
        .map_err(|_| AppError::Internal("Invalid MFA secret encoding".into()))?;
    // Secret and TOTP own these copies and zeroize on drop via the library's
    // enabled zeroize feature, including invalid-secret constructor failures.
    let secret = Secret::Encoded(secret_str.to_owned());
    let totp = create_totp(
        secret
            .to_bytes()
            .map_err(|_| AppError::Internal("Invalid MFA secret".into()))?,
        "NyxID",
        user_id,
    )?;

    let counter = matching_totp_counter(&totp, code, Utc::now().timestamp())
        .ok_or_else(|| AppError::AuthenticationFailed("Invalid TOTP code".into()))?;

    // Generate recovery codes
    let recovery_codes = Zeroizing::new(
        (0..10)
            .map(|_| {
                let token = Zeroizing::new(generate_random_token());
                token[..32].to_string()
            })
            .collect::<Vec<String>>(),
    );

    let codes_bson = bson::to_bson(&hash_recovery_codes(recovery_codes.clone()).await?)
        .map_err(|_| AppError::Internal("Recovery code encoding failed".into()))?;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    fence_live_account(db, &mut session, user_id, Some(true)).await?;
    if db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "user_id": user_id, "factor_type": "totp", "is_active": true, "is_verified": true
        })
        .session(&mut session)
        .await
        .map_err(mfa_db_error)?
        .is_some()
    {
        session.abort_transaction().await?;
        return Err(AppError::Conflict(
            "An active verified MFA factor already exists".into(),
        ));
    }
    let changed = db.collection::<MfaFactor>(MFA_FACTORS).update_one(
        doc! {"_id": factor_id, "user_id": user_id, "factor_type": "totp", "is_active": true, "is_verified": false},
        doc! {"$set": {"is_verified": true, "last_totp_counter": counter, "recovery_codes": codes_bson, "updated_at": bson::DateTime::now()}}
    ).session(&mut session).await.map_err(mfa_db_error)?;
    if changed.modified_count != 1 {
        session.abort_transaction().await?;
        return Err(AppError::Conflict("MFA setup changed; start again".into()));
    }
    if matching_totp_counter(&totp, code, Utc::now().timestamp()) != Some(counter) {
        session.abort_transaction().await?;
        return Err(AppError::AuthenticationFailed("TOTP code expired".into()));
    }
    super::session_authority::revoke_other_sessions_in_transaction(
        db,
        user_id,
        current_session_id,
        &mut session,
    )
    .await?;
    if let Some(current_session_id) = current_session_id {
        let retained = db
            .collection::<crate::models::session::Session>(crate::models::session::COLLECTION_NAME)
            .update_one(
                super::session_authority::authenticated(
                    doc! {"_id": current_session_id, "user_id": user_id,
                    "revoked": false, "expires_at": {"$gt": bson::DateTime::now()}},
                ),
                doc! {"$set": {"mfa_assured": true}},
            )
            .session(&mut session)
            .await
            .map_err(mfa_db_error)?;
        if retained.matched_count != 1 {
            session.abort_transaction().await?;
            return Err(AppError::Unauthorized(
                "Session expired; sign in again".into(),
            ));
        }
    }
    super::session_authority::revoke_pending(db, user_id, &mut session).await?;
    super::audit_service::log_actor_event_in_transaction(
        db,
        &mut session,
        &super::audit_service::AuditActor {
            user_id: user_id.into(),
            ip_address: actor.and_then(|a| a.ip_address.clone()),
            user_agent: actor.and_then(|a| a.user_agent.clone()),
            api_key_id: None,
            api_key_name: None,
        },
        "mfa_enabled",
        serde_json::json!({"factor_type": "totp", "other_sessions_signed_out": true}),
    )
    .await?;
    commit_mfa(&mut session).await?;

    Ok(recovery_codes)
}

use super::transaction_service::{commit as commit_mfa, map_error as mfa_db_error};

/// Serialize factor transitions against account disable/deletion and each other.
/// All enrollment/disable/recovery mutations write this same live account row.
async fn fence_live_account(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    user_id: &str,
    enabled: Option<bool>,
) -> AppResult<()> {
    let mut update = doc! {"$inc": {"mfa_state_version": 1_i64}};
    if let Some(enabled) = enabled {
        update.insert(
            "$set",
            doc! {"mfa_enabled": enabled, "updated_at": bson::DateTime::now()},
        );
    }
    let live = db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME).update_one(
        doc! {"_id": user_id, "is_active": true, "user_type": {"$ne": "org"}, "platform_personal_deleting": {"$ne": true}}, update
    ).session(session).await.map_err(mfa_db_error)?;
    if live.matched_count != 1 {
        return Err(AppError::AuthenticationFailed(
            "Account is unavailable".into(),
        ));
    }
    Ok(())
}

/// Verified password and exact MFA generation captured in one account snapshot.
/// Fields are private so callers cannot manufacture a first-factor proof.
pub struct TotpDisableProof {
    user: crate::models::user::User,
    generation: Option<i64>,
}

pub async fn prepare_totp_disable(
    db: &mongodb::Database,
    user_id: &str,
    password: Option<&str>,
) -> AppResult<TotpDisableProof> {
    let row = db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": user_id})
        .await?
        .ok_or_else(|| AppError::AuthenticationFailed("Account is unavailable".into()))?;
    let generation = match row.get("mfa_state_version") {
        None | Some(bson::Bson::Null) => None,
        Some(bson::Bson::Int64(value)) => Some(*value),
        Some(bson::Bson::Int32(value)) => Some(i64::from(*value)),
        _ => {
            return Err(AppError::AuthenticationFailed(
                "MFA state is unavailable".into(),
            ));
        }
    };
    let user: crate::models::user::User = bson::from_document(row)
        .map_err(|_| AppError::Internal("Account state is unavailable".into()))?;
    if let Some(hash) = user.password_hash.as_deref() {
        let supplied = password
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::AuthenticationFailed("Password is required".into()))?;
        if !crate::crypto::password::verify_password(supplied, hash)? {
            return Err(AppError::AuthenticationFailed("Invalid password".into()));
        }
    }
    Ok(TotpDisableProof { user, generation })
}

/// Password proof, MFA generation and current factor are redeemed together.
/// The login verifier writes the same account row and increments its generation;
/// comparing the immediately following generation also rejects disable/re-enroll.
pub async fn disable_totp(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    proof: TotpDisableProof,
    code: &str,
    actor: Option<&super::audit_service::AuditActor>,
) -> AppResult<LoginFactor> {
    let user_id = &proof.user.id;
    let mut completion = begin_login_completion(db, encryption_keys, user_id, code, None)
        .await?
        .ok_or_else(|| AppError::AuthenticationFailed("Invalid MFA code".into()))?;
    let expected = proof
        .generation
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| AppError::AuthenticationFailed("MFA state is unavailable".into()))?;
    let changed = db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": user_id, "is_active": true, "mfa_enabled": true,
                "user_type": {"$ne": "org"}, "mfa_state_version": expected,
                "password_hash": &proof.user.password_hash,
            },
            doc! {
                "$set": {"mfa_enabled": false, "updated_at": bson::DateTime::now()},
                "$inc": {"mfa_state_version": 1_i64},
            },
        )
        .session(&mut completion.transaction)
        .await
        .map_err(mfa_db_error)?;
    if changed.matched_count != 1 {
        completion.abort().await?;
        return Err(AppError::AuthenticationFailed(
            "MFA or password changed; verify again".into(),
        ));
    }
    db.collection::<MfaFactor>(MFA_FACTORS)
        .update_many(
            doc! {"user_id": user_id, "is_active": true},
            doc! {"$set": {"is_active": false, "updated_at": bson::DateTime::now()}},
        )
        .session(&mut completion.transaction)
        .await
        .map_err(mfa_db_error)?;
    super::session_authority::revoke_pending(db, user_id, &mut completion.transaction).await?;
    super::audit_service::log_actor_event_in_transaction(
        db,
        &mut completion.transaction,
        &super::audit_service::AuditActor {
            user_id: user_id.clone(),
            ip_address: actor.and_then(|a| a.ip_address.clone()),
            user_agent: actor.and_then(|a| a.user_agent.clone()),
            api_key_id: None,
            api_key_name: None,
        },
        "mfa_disabled",
        serde_json::json!({"factor_kind": completion.factor.as_str()}),
    )
    .await?;
    completion.commit().await
}

pub async fn disable_totp_with_code(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    code: &str,
    password: Option<&str>,
    actor: Option<&super::audit_service::AuditActor>,
) -> AppResult<LoginFactor> {
    let proof = prepare_totp_disable(db, user_id, password).await?;
    disable_totp(db, encryption_keys, proof, code, actor).await
}

static RECOVERY_WORK: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)));
fn busy() -> AppError {
    AppError::Conflict("MFA verification is busy; retry shortly".into())
}
async fn hash_recovery_codes(codes: Zeroizing<Vec<String>>) -> AppResult<Vec<String>> {
    let permit = RECOVERY_WORK
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    // Hold admission until the blocking task completes; no unbounded spawn queue.
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        codes
            .iter()
            .map(|code| password::hash_password(code))
            .collect()
    })
    .await
    .map_err(|_| AppError::Internal("MFA hashing failed".into()))?
}
fn valid_recovery_hash(hash: &str) -> bool {
    use argon2::password_hash::PasswordHash;
    if hash.len() > 256 {
        return false;
    }
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    parsed.algorithm.as_str() == "argon2id"
        && parsed.version == Some(19)
        && matches!(parsed.params.get_decimal("m"), Some(8..=65536))
        && matches!(parsed.params.get_decimal("t"), Some(1..=3))
        && matches!(parsed.params.get_decimal("p"), Some(1..=4))
        && parsed.hash.is_some_and(|value| value.len() == 32)
        && parsed.salt.is_some()
}
async fn matching_recovery_hash(
    code: &str,
    stored: Option<&serde_json::Value>,
) -> AppResult<Option<String>> {
    let Some(values) = stored.and_then(|value| value.as_array()) else {
        return Ok(None);
    };
    if values.len() > 10 {
        return Ok(None);
    }
    let hashes = values
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if hashes.len() != values.len() || !hashes.iter().all(|hash| valid_recovery_hash(hash)) {
        return Ok(None);
    }
    let permit = RECOVERY_WORK
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    let code = zeroize::Zeroizing::new(code.to_owned());
    tokio::task::spawn_blocking(move || -> AppResult<Option<String>> {
        let _permit = permit;
        for hash in hashes {
            if password::verify_password(&code, &hash)? {
                return Ok(Some(hash));
            }
        }
        Ok(None)
    })
    .await
    .map_err(|_| AppError::Internal("MFA verification failed".into()))?
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginFactor {
    Totp,
    Recovery,
}
impl LoginFactor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Totp => "totp",
            Self::Recovery => "recovery",
        }
    }
}

/// Match the documented one-step skew and retain the highest matching counter.
fn matching_totp_counter(totp: &TOTP, code: &str, now: i64) -> Option<i64> {
    let current = now.checked_div(30)?;
    [current + 1, current, current - 1]
        .into_iter()
        .find(|counter| {
            if *counter < 0 {
                return false;
            }
            let mut exact = totp.clone();
            exact.skew = 0;
            exact.check(code, (*counter as u64) * 30)
        })
}

pub struct ReservedChallenge {
    pub session: crate::models::session::Session,
    pub legacy: bool,
}
pub async fn reserve_pending_challenge(
    db: &mongodb::Database,
    token_hash: &str,
) -> AppResult<ReservedChallenge> {
    use crate::models::session::{MFA_CHALLENGES, Session};
    for (collection, legacy) in [
        (MFA_CHALLENGES, false),
        (crate::models::session::COLLECTION_NAME, true),
    ] {
        let row = db
            .collection::<Session>(collection)
            .find_one_and_update(
                super::session_authority::pending(
                    doc! {"token_hash": token_hash,"revoked":false,
                    "$expr": {"$gt": ["$expires_at", "$$NOW"]},
                    "$or": [{"mfa_attempts": {"$exists":false}}, {"mfa_attempts": {"$lt":5}}]},
                    legacy,
                ),
                doc! {"$inc": {"mfa_attempts":1}},
            )
            .await?;
        if let Some(session) = row {
            return Ok(ReservedChallenge { session, legacy });
        }
    }
    Err(AppError::AuthenticationFailed(
        "Invalid or expired MFA challenge".into(),
    ))
}

pub async fn pending_challenge_is_live(
    db: &mongodb::Database,
    token_hash: &str,
) -> AppResult<bool> {
    for (collection, legacy) in [
        (crate::models::session::MFA_CHALLENGES, false),
        (crate::models::session::COLLECTION_NAME, true),
    ] {
        if db
            .collection::<bson::Document>(collection)
            .find_one(super::session_authority::pending(
                doc! {
                    "token_hash": token_hash, "revoked": false,
                    "$expr": {"$gt": ["$expires_at", "$$NOW"]},
                    "$or": [{"mfa_attempts": {"$exists": false}}, {"mfa_attempts": {"$lt": 5}}],
                },
                legacy,
            ))
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn cancel_pending_challenge(db: &mongodb::Database, token_hash: &str) -> AppResult<()> {
    for (collection, legacy) in [
        (crate::models::session::MFA_CHALLENGES, false),
        (crate::models::session::COLLECTION_NAME, true),
    ] {
        if let Some(challenge) = db
            .collection::<crate::models::session::Session>(collection)
            .find_one_and_delete(super::session_authority::pending(
                doc! {"token_hash": token_hash, "revoked": false},
                legacy,
            ))
            .await?
        {
            super::audit_service::log_async(
                db.clone(),
                Some(challenge.user_id),
                "mfa_challenge_cancelled".into(),
                Some(serde_json::json!({"source": "cookie"})),
                None,
                None,
                None,
                None,
            );
        }
    }
    Ok(())
}

pub fn valid_login_code(code: &str) -> bool {
    (code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()))
        || (code.len() == 8
            && code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        || (code.len() == 32 && code.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Verify a login factor. Recovery redemption and optional pending challenge
/// consumption commit together with the live-account fence. Enrollment never
/// accepts recovery codes. A matching hash alone is not a successful redemption.
/// An uncommitted proof. Authenticated session/approval insertion must use this
/// transaction before commit; dropping it aborts consumption of the proof.
pub struct LoginCompletion {
    pub transaction: mongodb::ClientSession,
    pub factor: LoginFactor,
    totp_proof: Option<(TOTP, zeroize::Zeroizing<String>, i64)>,
    challenge_expires_at: Option<chrono::DateTime<Utc>>,
}
impl LoginCompletion {
    pub async fn abort(mut self) -> AppResult<()> {
        self.transaction.abort_transaction().await?;
        Ok(())
    }
    pub async fn commit(mut self) -> AppResult<LoginFactor> {
        if self
            .challenge_expires_at
            .is_some_and(|expiry| expiry <= Utc::now())
        {
            self.transaction.abort_transaction().await?;
            return Err(AppError::AuthenticationFailed(
                "MFA challenge expired".into(),
            ));
        }
        if let Some((totp, code, counter)) = &self.totp_proof
            && (matching_totp_counter(totp, code, Utc::now().timestamp()) != Some(*counter))
        {
            self.transaction.abort_transaction().await?;
            return Err(AppError::AuthenticationFailed("TOTP code expired".into()));
        }
        commit_mfa(&mut self.transaction).await?;
        Ok(self.factor)
    }
}

#[cfg(test)]
pub async fn verify_login_factor(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    user_id: &str,
    code: &str,
    pending: Option<&ReservedChallenge>,
) -> AppResult<Option<LoginFactor>> {
    match begin_login_completion(db, keys, user_id, code, pending).await? {
        Some(completion) => Ok(Some(completion.commit().await?)),
        None => Ok(None),
    }
}

pub async fn begin_login_completion(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    user_id: &str,
    code: &str,
    pending: Option<&ReservedChallenge>,
) -> AppResult<Option<LoginCompletion>> {
    // The account budget survives challenge replacement, inline login and
    // replica/IP changes. Charge before parsing or expensive recovery hashing;
    // rollback of verification never refunds an unsuccessful online attempt.
    let rate_db = db.client().database_with_options(
        db.name(),
        mongodb::options::DatabaseOptions::builder()
            .write_concern(mongodb::options::WriteConcern::majority())
            .build(),
    );
    if !super::coordination_service::RateWindowStore::admit(
        &rate_db,
        "mfa_login_account",
        user_id,
        10,
        std::time::Duration::from_secs(300),
    )
    .await?
    .allowed
    {
        return Err(AppError::RateLimited);
    }
    if !valid_login_code(code) {
        return Ok(None);
    }
    use futures::TryStreamExt;
    let mut factors: Vec<MfaFactor> = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find(doc! {
            "user_id": user_id, "factor_type": "totp", "is_verified": true, "is_active": true
        })
        .limit(2)
        .await?
        .try_collect()
        .await?;
    if factors.len() > 1 {
        return Err(AppError::Conflict(
            "Multiple active MFA factors require account recovery support".into(),
        ));
    }
    let factor = factors
        .pop()
        .ok_or_else(|| AppError::NotFound("No active TOTP factor found".into()))?;
    let (kind, hash, totp, counter) = if code.len() == 6 {
        let bytes = decrypt_factor_secret(keys, &factor).await?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| AppError::Internal("Invalid MFA secret".into()))?;
        let secret = Secret::Encoded(text.to_owned());
        let totp = create_totp(
            secret
                .to_bytes()
                .map_err(|_| AppError::Internal("Invalid MFA secret".into()))?,
            "NyxID",
            user_id,
        )?;
        let Some(counter) = matching_totp_counter(&totp, code, Utc::now().timestamp()) else {
            return Ok(None);
        };
        (LoginFactor::Totp, None, Some(totp), Some(counter))
    } else {
        let Some(hash) = matching_recovery_hash(code, factor.recovery_codes.as_ref()).await? else {
            return Ok(None);
        };
        (LoginFactor::Recovery, Some(hash), None, None)
    };
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    fence_live_account(db, &mut session, user_id, None).await?;
    if db
        .collection::<MfaFactor>(MFA_FACTORS)
        .count_documents(
            doc! {"user_id":user_id,"factor_type":"totp","is_verified":true,"is_active":true},
        )
        .limit(2)
        .session(&mut session)
        .await
        .map_err(mfa_db_error)?
        != 1
    {
        session.abort_transaction().await?;
        return Err(AppError::Conflict("MFA factor state is ambiguous".into()));
    }
    let mut predicate =
        doc! {"_id": &factor.id, "user_id": user_id, "is_active": true, "is_verified": true};
    let mut update = doc! {"$set": {"last_verified_at": bson::DateTime::now()}};
    if let Some(counter) = counter {
        predicate.insert("$or", bson::bson!([{"last_totp_counter": {"$exists": false}}, {"last_totp_counter": {"$lt": counter}}]));
        update
            .get_document_mut("$set")
            .expect("set update")
            .insert("last_totp_counter", counter);
    }
    if let Some(hash) = hash {
        predicate.insert("recovery_codes", &hash);
        update.insert("$pull", doc! {"recovery_codes": hash});
    }
    let changed = db
        .collection::<MfaFactor>(MFA_FACTORS)
        .update_one(predicate, update)
        .session(&mut session)
        .await
        .map_err(mfa_db_error)?;
    if changed.matched_count != 1 {
        session.abort_transaction().await?;
        return Ok(None);
    }
    if let Some(challenge) = pending {
        let collection = if challenge.legacy {
            "sessions"
        } else {
            crate::models::session::MFA_CHALLENGES
        };
        let consumed = db
            .collection::<bson::Document>(collection)
            .update_one(
                super::session_authority::pending(
                    doc! {"_id": &challenge.session.id, "user_id": user_id, "revoked": false,
                    "$expr": {"$gt": ["$expires_at", "$$NOW"]}, "mfa_attempts": {"$lte": 5}},
                    challenge.legacy,
                ),
                doc! {"$set": {"revoked": true}},
            )
            .session(&mut session)
            .await
            .map_err(mfa_db_error)?;
        if consumed.modified_count != 1 {
            session.abort_transaction().await?;
            return Err(AppError::AuthenticationFailed(
                "Invalid or expired MFA challenge".into(),
            ));
        }
    }
    if kind == LoginFactor::Recovery {
        super::audit_service::log_actor_event_in_transaction(
            db,
            &mut session,
            &super::audit_service::AuditActor {
                user_id: user_id.into(),
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
            },
            "mfa_recovery_used",
            serde_json::json!({"factor_type": "recovery"}),
        )
        .await?;
    }
    if let Some(totp) = &totp
        && (matching_totp_counter(totp, code, Utc::now().timestamp()) != counter)
    {
        session.abort_transaction().await?;
        return Ok(None);
    }
    if pending.is_some_and(|challenge| challenge.session.expires_at <= Utc::now()) {
        session.abort_transaction().await?;
        return Err(AppError::AuthenticationFailed(
            "MFA challenge expired".into(),
        ));
    }
    Ok(Some(LoginCompletion {
        transaction: session,
        factor: kind,
        totp_proof: totp
            .zip(counter)
            .map(|(totp, counter)| (totp, zeroize::Zeroizing::new(code.to_owned()), counter)),
        challenge_expires_at: pending.map(|challenge| challenge.session.expires_at),
    }))
}

const RECOVERY_NOTICE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
static RECOVERY_NOTICES: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(8)));

fn schedule_recovery_notice(
    permit: tokio::sync::OwnedSemaphorePermit,
    notice: impl std::future::Future<Output = AppResult<()>> + Send + 'static,
) {
    tokio::spawn(async move {
        let _permit = permit;
        match tokio::time::timeout(RECOVERY_NOTICE_TIMEOUT, notice).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("MFA recovery notification delivery failed"),
            Err(_) => tracing::warn!("MFA recovery notification delivery timed out"),
        }
    });
}

/// Bounded best-effort delivery never withholds a committed login response.
/// The transactional metadata-only recovery audit remains durable even when
/// channels stall, admission is full, or this process terminates before delivery.
pub fn notify_recovery_use(state: &crate::AppState, user_id: &str, factor: LoginFactor) {
    if factor != LoginFactor::Recovery {
        return;
    }
    let Ok(permit) = RECOVERY_NOTICES.clone().try_acquire_owned() else {
        tracing::warn!("MFA recovery notification capacity exhausted");
        return;
    };
    let state = state.clone();
    let user_id = user_id.to_owned();
    schedule_recovery_notice(permit, async move {
        super::notification_service::send_mfa_recovery_notification(&state, &user_id)
            .await
            .map(|_| ())
    });
}

pub fn notify_mfa_disabled(state: &crate::AppState, user_id: &str) {
    let Ok(permit) = RECOVERY_NOTICES.clone().try_acquire_owned() else {
        return;
    };
    let state = state.clone();
    let user_id = user_id.to_owned();
    schedule_recovery_notice(permit, async move {
        super::notification_service::send_mfa_disabled_notification(&state, &user_id)
            .await
            .map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{User, UserType};
    use crate::test_utils::{connect_test_database, test_encryption_keys, test_user};
    async fn fixture(name: &str) -> (mongodb::Database, EncryptionKeys, String) {
        let db = connect_test_database(name)
            .await
            .expect("isolated MongoDB required");
        let id = Uuid::new_v4().to_string();
        db.collection::<User>("users")
            .insert_one(test_user(&id, UserType::Person))
            .await
            .unwrap();
        (db, test_encryption_keys(), id)
    }
    #[tokio::test]
    async fn contextual_mfa_migration_preserves_authority_and_rejects_swaps_and_stale_snapshots() {
        let (db, mut keys, user) = fixture("mfa_context_migration").await;
        let legacy = factor(&db, &keys, &user, &["00112233445566778899001122334455"]).await;
        let legacy_secret = decrypt_factor_secret(&keys, &legacy).await.unwrap();
        assert!(
            migrate_secret_snapshot(&db, &keys, &bson::to_document(&legacy).unwrap())
                .await
                .unwrap()
        );
        let migrated = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id":&legacy.id})
            .await
            .unwrap()
            .unwrap();
        verify_current_secret(&keys, &migrated).await.unwrap();
        assert_eq!(
            *decrypt_factor_secret(&keys, &migrated).await.unwrap(),
            *legacy_secret
        );
        assert_eq!(migrated.is_verified, legacy.is_verified);
        assert_eq!(migrated.is_active, legacy.is_active);
        assert_eq!(migrated.recovery_codes, legacy.recovery_codes);
        assert_eq!(migrated.last_totp_counter, legacy.last_totp_counter);
        assert_eq!(
            migrated.updated_at.timestamp_millis(),
            legacy.updated_at.timestamp_millis()
        );
        let mut swapped = migrated.clone();
        swapped.id = Uuid::new_v4().to_string();
        assert!(decrypt_factor_secret(&keys, &swapped).await.is_err());
        swapped = migrated.clone();
        swapped.user_id = Uuid::new_v4().to_string();
        assert!(decrypt_factor_secret(&keys, &swapped).await.is_err());
        keys.set_allow_legacy_record_reads(false);
        assert!(decrypt_factor_secret(&keys, &legacy).await.is_err());
        verify_current_secret(&keys, &migrated).await.unwrap();
        for value in [b"".as_slice(), b"bad-base32!", b"JBSWY3DP"] {
            let mut malformed = migrated.clone();
            malformed.secret_encrypted = Some(
                encrypt_factor_secret(&keys, &migrated.id, &user, value)
                    .await
                    .unwrap(),
            );
            assert!(verify_current_secret(&keys, &malformed).await.is_err());
            assert!(
                migrate_secret_snapshot(&db, &keys, &bson::to_document(&malformed).unwrap())
                    .await
                    .is_err()
            );
        }
        // A concurrent accepted counter must never be reset by storage migration.
        db.collection::<MfaFactor>(MFA_FACTORS)
            .update_one(
                doc! {"_id":&migrated.id},
                doc! {"$set":{"last_totp_counter":42_i64}},
            )
            .await
            .unwrap();
        assert!(
            !migrate_secret_snapshot(&db, &keys, &bson::to_document(&migrated).unwrap())
                .await
                .unwrap()
        );
        let successor = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id":&migrated.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(successor.last_totp_counter, Some(42));
        assert_eq!(successor.secret_encrypted, migrated.secret_encrypted);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn compatible_mfa_setup_remains_old_reader_compatible_until_writer_cutover() {
        let (db, mut keys, user) = fixture("mfa_context_staging").await;
        keys.set_contextual_record_writes(false);
        let setup = setup_totp(&db, &keys, &user, "mfa@example.test")
            .await
            .unwrap();
        let legacy = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id":&setup.factor_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(legacy.secret_context_version, None);
        assert_eq!(
            keys.decrypt(legacy.secret_encrypted.as_ref().unwrap())
                .await
                .unwrap(),
            setup.secret.as_bytes()
        );
        assert!(
            migrate_secret_snapshot(&db, &keys, &bson::to_document(&legacy).unwrap())
                .await
                .is_err()
        );
        keys.set_contextual_record_writes(true);
        let successor = setup_totp(&db, &keys, &user, "mfa@example.test")
            .await
            .unwrap();
        let current_factor = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id":&successor.factor_id})
            .await
            .unwrap()
            .unwrap();
        verify_current_secret(&keys, &current_factor).await.unwrap();
        let code = current(&successor.secret);
        verify_totp_setup(&db, &keys, &successor.factor_id, &user, &code, None, None)
            .await
            .unwrap();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_array_secret_migrates_from_raw_snapshot_without_normalizing_cas() {
        let (db, keys, user) = fixture("mfa_array_migration").await;
        let legacy = factor(&db, &keys, &user, &[]).await;
        let mut source = db
            .collection::<bson::Document>(MFA_FACTORS)
            .find_one(doc! {"_id":&legacy.id})
            .await
            .unwrap()
            .unwrap();
        source.remove("secret_context_version");
        source.insert(
            "secret_encrypted",
            legacy
                .secret_encrypted
                .as_ref()
                .unwrap()
                .iter()
                .map(|byte| bson::Bson::Int64((*byte).into()))
                .collect::<Vec<_>>(),
        );
        db.collection::<bson::Document>(MFA_FACTORS)
            .replace_one(doc! {"_id":&legacy.id}, &source)
            .await
            .unwrap();
        assert!(migrate_secret_snapshot(&db, &keys, &source).await.unwrap());
        let migrated = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id":&legacy.id})
            .await
            .unwrap()
            .unwrap();
        verify_current_secret(&keys, &migrated).await.unwrap();
        assert_eq!(
            *decrypt_factor_secret(&keys, &migrated).await.unwrap(),
            *decrypt_factor_secret(&keys, &legacy).await.unwrap()
        );
        assert!(!migrate_secret_snapshot(&db, &keys, &source).await.unwrap());
        db.drop().await.unwrap();
    }

    fn current(secret: &str) -> String {
        create_totp(
            Secret::Encoded(secret.into()).to_bytes().unwrap(),
            "NyxID",
            "test",
        )
        .unwrap()
        .generate_current()
        .unwrap()
    }
    async fn factor(
        db: &mongodb::Database,
        keys: &EncryptionKeys,
        id: &str,
        codes: &[&str],
    ) -> MfaFactor {
        let secret = Secret::generate_secret().to_encoded().to_string();
        let mut hashes = Vec::new();
        for code in codes {
            hashes.push(password::hash_password(code).unwrap());
        }
        let factor = MfaFactor {
            id: Uuid::new_v4().to_string(),
            user_id: id.into(),
            factor_type: "totp".into(),
            secret_encrypted: Some(keys.encrypt(secret.as_bytes()).await.unwrap()),
            secret_context_version: None,
            recovery_codes: Some(serde_json::json!(hashes)),
            last_totp_counter: None,
            is_verified: true,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.collection::<MfaFactor>(MFA_FACTORS)
            .insert_one(&factor)
            .await
            .unwrap();
        factor
    }
    #[tokio::test]
    async fn disable_password_proof_held_across_reset_or_factor_generation_is_rejected() {
        use crate::services::{auth_service, token_service};
        for change in ["password_reset", "factor_generation"] {
            let (db, keys, id) = fixture("mfa_disable_proof").await;
            // Runtime test credentials; never fixed passwords.
            let old_password = crate::crypto::token::generate_random_token();
            let successor_password = crate::crypto::token::generate_random_token();
            db.collection::<bson::Document>("users").update_one(doc! {"_id":&id},
                doc! {"$set":{"password_hash":password::hash_password(&old_password).unwrap(),"mfa_enabled":true}}).await.unwrap();
            let active = factor(&db, &keys, &id, &["Recovery"]).await;
            // This is the production seam after verification and before mutation.
            let held = prepare_totp_disable(&db, &id, Some(&old_password))
                .await
                .unwrap();
            let new_password = if change == "password_reset" {
                let actor = db
                    .collection::<User>("users")
                    .find_one(doc! {"_id":&id})
                    .await
                    .unwrap()
                    .unwrap();
                let reset = auth_service::initiate_password_reset(&db, &actor.email)
                    .await
                    .unwrap()
                    .unwrap();
                auth_service::reset_password(&db, &reset, &successor_password)
                    .await
                    .unwrap();
                successor_password.as_str()
            } else {
                // Enrollment/disable use this same shared generation fence.
                db.collection::<bson::Document>("users")
                    .update_one(doc! {"_id":&id}, doc! {"$inc":{"mfa_state_version":1_i64}})
                    .await
                    .unwrap();
                old_password.as_str()
            };
            let token = crate::crypto::token::hash_token("successor-pending-proof");
            token_service::create_mfa_pending_session_with_method(
                &db,
                &id,
                &token,
                Some("password"),
            )
            .await
            .unwrap();
            assert!(matches!(
                disable_totp(&db, &keys, held, "Recovery", None).await,
                Err(AppError::AuthenticationFailed(_))
            ));
            assert_eq!(
                db.collection::<MfaFactor>(MFA_FACTORS)
                    .count_documents(doc! {"_id":&active.id,"is_active":true})
                    .await
                    .unwrap(),
                1
            );
            assert_eq!(
                db.collection::<bson::Document>(crate::models::session::MFA_CHALLENGES)
                    .count_documents(doc! {"user_id":&id,"revoked":false})
                    .await
                    .unwrap(),
                1,
                "stale proof invalidated successor pending authority"
            );
            let fresh = prepare_totp_disable(&db, &id, Some(new_password))
                .await
                .unwrap();
            disable_totp(&db, &keys, fresh, "Recovery", None)
                .await
                .unwrap();
            assert_eq!(
                db.collection::<MfaFactor>(MFA_FACTORS)
                    .count_documents(doc! {"user_id":&id,"is_active":true})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                db.collection::<bson::Document>(crate::models::session::MFA_CHALLENGES)
                    .count_documents(doc! {"user_id":&id,"revoked":false})
                    .await
                    .unwrap(),
                0
            );
            db.drop().await.unwrap();
        }
    }
    #[tokio::test]
    async fn account_factor_admission_is_atomic_isolated_and_resets_after_window() {
        use crate::models::coordination::RATE_WINDOW_COLLECTION_NAME;
        use std::time::Duration;
        crate::test_utils::ensure_rate_window_headroom(
            Duration::from_secs(300),
            Duration::from_secs(15),
        )
        .await;
        let (db, keys, id) = fixture("mfa_account_admission").await;
        let keys = std::sync::Arc::new(keys);
        let other =
            mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap()
                .database(db.name());
        // Warm the row: this assertion is an admission, even for malformed input.
        assert!(
            begin_login_completion(&db, &keys, &id, "invalid", None)
                .await
                .unwrap()
                .is_none()
        );
        let mut tasks = Vec::new();
        for n in 0..20 {
            let database = if n % 2 == 0 {
                db.clone()
            } else {
                other.clone()
            };
            let keys = keys.clone();
            let id = id.clone();
            tasks.push(tokio::spawn(async move {
                begin_login_completion(&database, &keys, &id, "invalid", None).await
            }));
        }
        let mut admitted = 1;
        for task in tasks {
            match task.await.unwrap() {
                Ok(None) => admitted += 1,
                Err(AppError::RateLimited) => {}
                _ => panic!("unexpected admission result; database/internal errors are failures"),
            }
        }
        assert_eq!(admitted, 10);
        assert!(
            begin_login_completion(&other, &keys, &Uuid::new_v4().to_string(), "invalid", None)
                .await
                .unwrap()
                .is_none()
        );
        // Advance only this fixture's persisted window to exercise the production
        // DB-clock reset pipeline without a five-minute wall-clock sleep.
        db.collection::<bson::Document>(RATE_WINDOW_COLLECTION_NAME).update_many(
            doc! {"namespace":"mfa_login_account"},
            doc! {"$set":{"window_start":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(10))}}
        ).await.unwrap();
        assert!(
            begin_login_completion(&other, &keys, &id, "invalid", None)
                .await
                .unwrap()
                .is_none()
        );
        db.drop().await.unwrap();
    }

    #[test]
    fn totp_library_owners_zeroize_on_drop_and_invalid_constructor_diagnostics_are_fixed() {
        fn zeroizes<T: zeroize::ZeroizeOnDrop>() {}
        zeroizes::<TOTP>();
        zeroizes::<Secret>();
        let secret = "distinctive-totp-constructor-secret";
        let error = create_totp(vec![1; 2], secret, secret).err().unwrap();
        assert!(!format!("{error:?} {error}").contains(secret));
    }

    #[test]
    fn login_input_and_phc_work_bounds() {
        assert!(valid_login_code("123456"));
        assert!(valid_login_code("Ab_12-xy"));
        for code in [
            "",
            "12345",
            "abcdef",
            "1234567",
            "123456789",
            "1234 678",
            "éééé",
        ] {
            assert!(!valid_login_code(code));
        }
        let hash = password::hash_password(&crate::crypto::token::generate_random_token()).unwrap();
        assert!(valid_recovery_hash(&hash));
        for malicious in [
            hash.replace("m=65536", "m=4294967295"),
            hash.replace("t=3", "t=999"),
            hash.replace("p=4", "p=99"),
            hash.replace("argon2id", "argon2i"),
        ] {
            assert!(!valid_recovery_hash(&malicious));
        }
    }
    #[tokio::test]
    async fn enrollment_replacement_and_confirm_preserve_active_state() {
        let (db, keys, id) = fixture("mfa_enroll").await;
        let old = setup_totp(&db, &keys, &id, "user@example.com")
            .await
            .unwrap();
        let new = setup_totp(&db, &keys, &id, "user@example.com")
            .await
            .unwrap();
        assert!(
            verify_totp_setup(
                &db,
                &keys,
                &old.factor_id,
                &id,
                &current(&old.secret),
                None,
                None
            )
            .await
            .is_err()
        );
        let enrolled_code = current(&new.secret);
        let codes = verify_totp_setup(&db, &keys, &new.factor_id, &id, &enrolled_code, None, None)
            .await
            .unwrap();
        assert_eq!(codes.len(), 10);
        assert!(
            codes
                .iter()
                .all(|code| code.len() == 32 && code.bytes().all(|b| b.is_ascii_hexdigit()))
        );
        assert!(
            db.collection::<User>("users")
                .find_one(doc! {"_id": &id})
                .await
                .unwrap()
                .unwrap()
                .mfa_enabled
        );
        assert!(
            setup_totp(&db, &keys, &id, "user@example.com")
                .await
                .is_err()
        );
        assert!(
            verify_totp_setup(&db, &keys, &new.factor_id, &id, &codes[0], None, None)
                .await
                .is_err(),
            "recovery cannot enroll"
        );
        assert_eq!(
            verify_login_factor(&db, &keys, &id, &enrolled_code, None)
                .await
                .unwrap(),
            None,
            "enrollment consumes its counter"
        );
        let enrolled = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id": &new.factor_id})
            .await
            .unwrap()
            .unwrap();
        let next_counter =
            (enrolled.last_totp_counter.unwrap() + 1).max(Utc::now().timestamp() / 30);
        let next = create_totp(
            Secret::Encoded(new.secret.to_string()).to_bytes().unwrap(),
            "NyxID",
            "test",
        )
        .unwrap()
        .generate(next_counter as u64 * 30);
        assert_eq!(
            verify_login_factor(&db, &keys, &id, &next, None)
                .await
                .unwrap(),
            Some(LoginFactor::Totp)
        );
        assert_eq!(
            verify_login_factor(&db, &keys, &id, &next, None)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            verify_login_factor(&db, &keys, &id, &codes[0], None)
                .await
                .unwrap(),
            Some(LoginFactor::Recovery)
        );
        assert_eq!(
            verify_login_factor(&db, &keys, &id, &codes[0], None)
                .await
                .unwrap(),
            None
        );
    }
    #[test]
    fn totp_skew_matches_exact_counter_and_excludes_expired_steps() {
        let totp = create_totp(vec![0x42; 20], "NyxID", "skew-test").unwrap();
        let now = 1_800_000_000;
        let counter = now / 30;
        for offset in [-1_i64, 0, 1] {
            let code = totp.generate(((counter + offset) * 30) as u64);
            assert_eq!(
                matching_totp_counter(&totp, &code, now),
                Some(counter + offset)
            );
        }
        for offset in [-2_i64, 2] {
            let code = totp.generate(((counter + offset) * 30) as u64);
            assert_eq!(matching_totp_counter(&totp, &code, now), None);
        }
    }

    #[tokio::test]
    async fn totp_counter_is_single_use_across_clients_and_monotonic_with_skew() {
        let (db, keys, id) = fixture("mfa_totp_replay").await;
        let stored = factor(&db, &keys, &id, &[]).await;
        let plaintext = keys
            .decrypt(stored.secret_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        let secret = Secret::Encoded(String::from_utf8(plaintext).unwrap())
            .to_bytes()
            .unwrap();
        let totp = create_totp(secret, "NyxID", "test").unwrap();
        let counter = Utc::now().timestamp() / 30;
        let code = totp.generate((counter * 30) as u64);
        let client = mongodb::Client::with_uri_str(
            std::env::var("NYXID_TEST_DATABASE_URL").expect("test URI required"),
        )
        .await
        .unwrap();
        let independent = client.database(db.name());
        let (a, b) = tokio::join!(
            verify_login_factor(&db, &keys, &id, &code, None),
            verify_login_factor(&independent, &keys, &id, &code, None),
        );
        for result in [&a, &b] {
            assert!(
                matches!(result, Ok(_) | Err(AppError::Conflict(_))),
                "unexpected result: {result:?}"
            );
        }
        assert_eq!(
            [a, b]
                .iter()
                .filter(|result| matches!(result, Ok(Some(LoginFactor::Totp))))
                .count(),
            1
        );
        assert_eq!(
            verify_login_factor(&db, &keys, &id, &code, None)
                .await
                .unwrap(),
            None
        );
        let previous = totp.generate(((counter - 1) * 30) as u64);
        assert_eq!(
            verify_login_factor(&db, &keys, &id, &previous, None)
                .await
                .unwrap(),
            None
        );
        let next = totp.generate(((counter + 1) * 30) as u64);
        assert_eq!(
            verify_login_factor(&db, &keys, &id, &next, None)
                .await
                .unwrap(),
            Some(LoginFactor::Totp)
        );
        assert_eq!(
            verify_login_factor(&independent, &keys, &id, &next, None)
                .await
                .unwrap(),
            None
        );
        let updated = db
            .collection::<bson::Document>(MFA_FACTORS)
            .find_one(doc! {"_id":stored.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.get_i64("last_totp_counter").unwrap(), counter + 1);
    }

    #[tokio::test]
    async fn recovery_single_use_across_independent_clients_and_metadata_only_audit() {
        let (db, keys, id) = fixture("mfa_recovery").await;
        let code = "R3c-v_12";
        factor(&db, &keys, &id, &[code]).await;
        let client = mongodb::Client::with_uri_str(
            std::env::var("NYXID_TEST_DATABASE_URL").expect("test URI required"),
        )
        .await
        .unwrap();
        let independent = client.database(db.name());
        let (a, b) = tokio::join!(
            verify_login_factor(&db, &keys, &id, code, None),
            verify_login_factor(&independent, &keys, &id, code, None)
        );
        for result in [&a, &b] {
            assert!(
                matches!(
                    result,
                    Ok(_) | Err(AppError::Conflict(_)) | Err(AppError::AuthenticationFailed(_))
                ),
                "unexpected result: {result:?}"
            );
        }
        assert_eq!(
            [a, b]
                .iter()
                .filter(|v| matches!(v, Ok(Some(LoginFactor::Recovery))))
                .count(),
            1
        );
        assert_eq!(
            verify_login_factor(&db, &keys, &id, code, None)
                .await
                .unwrap(),
            None
        );
        let audit = db
            .collection::<bson::Document>("audit_log")
            .find_one(doc! {"user_id": &id, "event_type": "mfa_recovery_used"})
            .await
            .unwrap()
            .expect("durable recovery audit");
        assert!(!format!("{audit:?}").contains(code));
    }
    #[tokio::test]
    async fn wrong_owner_disabled_factor_and_deleted_account_keep_recovery_unspent() {
        let (db, keys, id) = fixture("mfa_boundaries").await;
        let row = factor(&db, &keys, &id, &["Recovery"]).await;
        assert!(
            verify_login_factor(&db, &keys, &Uuid::new_v4().to_string(), "Recovery", None)
                .await
                .is_err()
        );
        assert_eq!(
            verify_login_factor(&db, &keys, &id, "Invalid_", None)
                .await
                .unwrap(),
            None
        );
        db.collection::<bson::Document>("users")
            .update_one(doc! {"_id": &id}, doc! {"$set": {"is_active": false}})
            .await
            .unwrap();
        assert!(
            verify_login_factor(&db, &keys, &id, "Recovery", None)
                .await
                .is_err()
        );
        let retained = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retained.recovery_codes.unwrap().as_array().unwrap().len(),
            1
        );
    }
    #[tokio::test]
    async fn pending_challenge_is_single_use_expired_and_attempt_limits_fail_closed() {
        let (db, keys, id) = fixture("mfa_pending").await;
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id": &id}, doc! {"$set": {"mfa_enabled": true}})
            .await
            .unwrap();
        factor(&db, &keys, &id, &["Recovery", "An0ther_"]).await;
        let token_hash = crate::crypto::token::hash_token("pending-token");
        let pending_id = crate::services::token_service::create_mfa_pending_session_with_method(
            &db,
            &id,
            &token_hash,
            Some("password"),
        )
        .await
        .unwrap();
        // The handler reserves an attempt before bounded password work.
        db.collection::<bson::Document>(crate::models::session::MFA_CHALLENGES)
            .update_one(
                doc! {"_id": &pending_id},
                doc! {"$set": {"mfa_attempts": 1}},
            )
            .await
            .unwrap();
        let challenge = reserve_pending_challenge(&db, &token_hash).await.unwrap();
        let (a, b) = tokio::join!(
            verify_login_factor(&db, &keys, &id, "Recovery", Some(&challenge)),
            verify_login_factor(&db, &keys, &id, "An0ther_", Some(&challenge))
        );
        for result in [&a, &b] {
            assert!(
                matches!(
                    result,
                    Ok(_) | Err(AppError::Conflict(_)) | Err(AppError::AuthenticationFailed(_))
                ),
                "unexpected result: {result:?}"
            );
        }
        assert_eq!(
            [a, b].iter().filter(|v| matches!(v, Ok(Some(_)))).count(),
            1
        );
        let stored = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"user_id": &id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored
                .recovery_codes
                .as_ref()
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1,
            "losing challenge must not consume its code"
        );
        db.collection::<bson::Document>(crate::models::session::MFA_CHALLENGES)
            .update_one(
                doc! {"_id": &pending_id},
                doc! {"$set": {"revoked": false,"mfa_attempts": 6}},
            )
            .await
            .unwrap();
        for code in ["Recovery", "An0ther_"] {
            assert!(!matches!(
                verify_login_factor(&db, &keys, &id, code, Some(&challenge)).await,
                Ok(Some(_))
            ));
        }
        db.collection::<bson::Document>(crate::models::session::MFA_CHALLENGES)
            .update_one(
                doc! {"_id": &pending_id},
                doc! {"$set": {"mfa_attempts": 1,"expires_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        for code in ["Recovery", "An0ther_"] {
            assert!(!matches!(
                verify_login_factor(&db, &keys, &id, code, Some(&challenge)).await,
                Ok(Some(_))
            ));
        }
    }
    #[tokio::test]
    async fn concurrent_setup_confirm_disable_never_reactivates_stale_factor() {
        let (db, keys, id) = fixture("mfa_cas").await;
        let setup = setup_totp(&db, &keys, &id, "user@example.com")
            .await
            .unwrap();
        let code = current(&setup.secret);
        let (confirm, replace) = tokio::join!(
            verify_totp_setup(&db, &keys, &setup.factor_id, &id, &code, None, None),
            setup_totp(&db, &keys, &id, "user@example.com")
        );
        assert_eq!(
            usize::from(confirm.is_ok()) + usize::from(replace.is_ok()),
            1
        );
        let recovery_codes = match confirm {
            Ok(codes) => codes,
            Err(error) => {
                assert!(
                    matches!(error, AppError::Conflict(_) | AppError::NotFound(_)),
                    "unexpected confirmation error: {error:?}"
                );
                let replacement = replace.as_ref().unwrap();
                verify_totp_setup(
                    &db,
                    &keys,
                    &replacement.factor_id,
                    &id,
                    &current(&replacement.secret),
                    None,
                    None,
                )
                .await
                .unwrap()
            }
        };
        if let Err(error) = replace {
            assert!(
                matches!(error, AppError::Conflict(_)),
                "unexpected replacement error: {error:?}"
            );
        }
        // Either confirmation wins or replacement invalidates its old id.
        let live = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .count_documents(doc! {"user_id": &id,"is_active":true})
            .await
            .unwrap();
        assert_eq!(live, 1);
        db.collection::<bson::Document>("users").update_one(doc! {"_id":&id},
            doc! {"$set":{"password_hash":password::hash_password(&crate::crypto::token::generate_random_token()).unwrap()}}).await.unwrap();
        let user = db
            .collection::<crate::models::user::User>("users")
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        let generation = db
            .collection::<bson::Document>("users")
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap()
            .get_i64("mfa_state_version")
            .ok();
        disable_totp(
            &db,
            &keys,
            TotpDisableProof { user, generation },
            &recovery_codes[0],
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            db.collection::<MfaFactor>(MFA_FACTORS)
                .count_documents(doc! {"user_id": &id,"is_active":true})
                .await
                .unwrap(),
            0
        );
        assert!(
            !db.collection::<User>("users")
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap()
                .mfa_enabled
        );
        assert!(
            verify_totp_setup(&db, &keys, &setup.factor_id, &id, &code, None, None)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn legacy_duplicate_factors_are_never_arbitrarily_selected_or_destroyed() {
        let (db, keys, id) = fixture("mfa_duplicates").await;
        let pending = setup_totp(&db, &keys, &id, "user@example.com")
            .await
            .unwrap();
        let first = factor(&db, &keys, &id, &["Recovery"]).await;
        assert!(matches!(
            setup_totp(&db, &keys, &id, "user@example.com").await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            verify_totp_setup(
                &db,
                &keys,
                &pending.factor_id,
                &id,
                &current(&pending.secret),
                None,
                None
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        factor(&db, &keys, &id, &["An0ther_"]).await;
        assert!(matches!(
            verify_login_factor(&db, &keys, &id, "Recovery", None).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            db.collection::<MfaFactor>(MFA_FACTORS)
                .count_documents(doc! {"user_id":&id,"is_active":true,"is_verified":true})
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            db.collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {"_id":first.id})
                .await
                .unwrap()
                .unwrap()
                .recovery_codes
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
