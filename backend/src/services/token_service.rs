use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::{self, JwtKeys};
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::mcp_session::McpSessionStore;
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};

/// Grace period (in seconds) after refresh token rotation during which
/// reuse of the old token is treated as a legitimate retry rather than theft.
///
/// **Security trade-off**: A longer window gives clients more time to recover
/// from network failures during rotation (e.g., the response with the new
/// token was lost), but also gives an attacker who stole the old token a
/// window to use it before it is flagged as theft.
///
/// 120 seconds was chosen because:
/// - Network retries typically happen within seconds, not minutes.
/// - It covers slow mobile connections and client-side retry back-off.
/// - It is short enough that a stolen token has minimal usable window
///   (the attacker must also have the JWT, which has its own short TTL).
const REUSE_GRACE_PERIOD_SECS: i64 = 120;

/// Maximum depth when following a replacement chain.
/// Prevents infinite loops if the database has a cycle, and bounds the
/// number of DB round-trips for concurrent-rotation scenarios.
const MAX_REPLACEMENT_CHAIN_DEPTH: usize = 5;

/// Session lifetime for first-party sessions.
pub const SESSION_TTL_SECS: i64 = 30 * 24 * 3600;

/// Default bearer-token scopes for first-party NyxID clients.
///
/// Includes `proxy` so first-party token clients retain access to NyxID's
/// proxy, LLM gateway, and MCP surfaces after route-level scope enforcement.
pub const FIRST_PARTY_ACCESS_SCOPES: &str = "openid profile email proxy";

/// Tokens issued after successful authentication.
pub struct IssuedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: String,
    pub access_expires_in: i64,
    pub resource_uris: Vec<String>,
}

/// Session issued for browser-based authentication flows.
pub struct IssuedSession {
    pub session_token: String,
    pub session_id: String,
}

enum RefreshTokenDisposition {
    Rotate(RefreshToken),
    Reuse(RefreshToken),
}

async fn follow_active_replacement_chain(
    db: &mongodb::Database,
    first_replacement_id: &str,
    user_id: &str,
    request_jti: &str,
) -> AppResult<Option<RefreshToken>> {
    let mut current_id = first_replacement_id.to_string();

    for depth in 0..MAX_REPLACEMENT_CHAIN_DEPTH {
        let candidate = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "_id": &current_id })
            .await?;

        match candidate {
            Some(r) if !r.revoked && r.expires_at > Utc::now() => {
                tracing::info!(
                    user_id = %user_id,
                    jti = %request_jti,
                    replacement_id = %current_id,
                    chain_depth = depth,
                    "Refresh token retry resolved to active replacement token"
                );
                return Ok(Some(r));
            }
            Some(RefreshToken {
                replaced_by: Some(next_id),
                ..
            }) => {
                tracing::debug!(
                    user_id = %user_id,
                    replacement_id = %current_id,
                    chain_depth = depth,
                    "Following replacement chain (concurrent rotation)"
                );
                current_id = next_id;
            }
            _ => return Ok(None),
        }
    }

    Ok(None)
}

async fn touch_session_last_active(
    db: &mongodb::Database,
    session_id: Option<&str>,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    if let Some(sid) = session_id {
        db.collection::<Session>(SESSIONS)
            .update_one(
                doc! { "_id": sid },
                doc! { "$set": {
                    "last_active_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;
    }

    Ok(())
}

fn build_reused_refresh_response(
    jwt_keys: &JwtKeys,
    config: &AppConfig,
    user_id: &Uuid,
    active_token: &RefreshToken,
    access_token: String,
    resource_uris: Vec<String>,
) -> AppResult<IssuedTokens> {
    let refresh_token = jwt::reissue_refresh_token(
        jwt_keys,
        config,
        user_id,
        &active_token.jti,
        active_token.created_at.timestamp(),
        active_token.expires_at.timestamp(),
    )?;

    Ok(IssuedTokens {
        access_token,
        refresh_token,
        session_id: active_token
            .session_id
            .clone()
            .unwrap_or_else(|| Uuid::nil().to_string()),
        access_expires_in: config.jwt_access_ttl_secs,
        resource_uris,
    })
}

/// Create a new session and issue JWT tokens.
pub async fn create_session(
    db: &mongodb::Database,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<IssuedSession> {
    create_session_with_transaction(db, user_id, ip_address, user_agent, None).await
}

pub(crate) async fn create_session_with_transaction(
    db: &mongodb::Database,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    transaction: Option<&mut mongodb::ClientSession>,
) -> AppResult<IssuedSession> {
    create_session_with_transaction_and_assurance(
        db,
        user_id,
        ip_address,
        user_agent,
        transaction,
        false,
    )
    .await
}

pub(crate) async fn create_session_with_transaction_and_assurance(
    db: &mongodb::Database,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    transaction: Option<&mut mongodb::ClientSession>,
    mfa_assured: bool,
) -> AppResult<IssuedSession> {
    create_session_with_transaction_and_mfa_authority(
        db,
        user_id,
        ip_address,
        user_agent,
        transaction,
        mfa_assured,
        None,
    )
    .await
}

pub(crate) async fn create_session_with_transaction_and_mfa_authority(
    db: &mongodb::Database,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    transaction: Option<&mut mongodb::ClientSession>,
    mfa_assured: bool,
    mfa_exemption: Option<crate::models::session::MfaExemption>,
) -> AppResult<IssuedSession> {
    Uuid::parse_str(user_id).map_err(|e| AppError::Internal(format!("Invalid user_id: {e}")))?;

    let session_token = generate_random_token();
    let session_token_hash = hash_token(&session_token);
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let session_expires = now + Duration::seconds(SESSION_TTL_SECS);

    // Create session record
    let new_session = Session {
        purpose: Some(crate::models::session::SessionPurpose::Authenticated),
        mfa_method: None,
        mfa_assured: Some(mfa_assured),
        mfa_exemption,
        id: session_id.clone(),
        user_id: user_id.to_string(),
        token_hash: session_token_hash,
        ip_address: ip_address.map(String::from),
        user_agent: user_agent.map(String::from),
        expires_at: session_expires,
        revoked: false,
        created_at: now,
        last_active_at: now,
    };

    let collection = db.collection::<Session>(SESSIONS);
    let insert = collection.insert_one(&new_session);
    if let Some(transaction) = transaction {
        // The transaction owner aborts definite preparation failures. Helpers
        // must not abort a borrowed transaction behind its owner's back.
        insert.session(&mut *transaction).await?;
    } else {
        insert.await?;
    }

    Ok(IssuedSession {
        session_token,
        session_id,
    })
}

/// Create a new session and issue JWT tokens.
#[cfg(test)]
pub async fn create_session_and_issue_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<IssuedTokens> {
    issue_session_tokens(db, config, jwt_keys, user_id, ip_address, user_agent, None).await
}

pub async fn create_derived_session_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    authority: super::session_authority::DerivedMfaAuthority,
) -> AppResult<IssuedTokens> {
    super::session_authority::require_derived_mfa(authority)?;
    let prepared = prepare_session_tokens_with_assurance(
        db,
        config,
        jwt_keys,
        user_id,
        ip_address,
        user_agent,
        authority.verified,
    )
    .await?;
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    insert_prepared_session(db, &prepared, &mut transaction).await?;
    super::transaction_service::commit(&mut transaction).await?;
    Ok(prepared.tokens)
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
async fn issue_session_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    transaction: Option<&mut mongodb::ClientSession>,
) -> AppResult<IssuedTokens> {
    let prepared =
        prepare_session_tokens(db, config, jwt_keys, user_id, ip_address, user_agent).await?;
    if let Some(transaction) = transaction {
        insert_prepared_session(db, &prepared, transaction).await?;
    } else {
        db.collection::<Session>(SESSIONS)
            .insert_one(&prepared.session)
            .await?;
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(&prepared.refresh)
            .await?;
    }
    Ok(prepared.tokens)
}

pub struct PreparedSessionTokens {
    pub tokens: IssuedTokens,
    session: Session,
    refresh: RefreshToken,
}

impl PreparedSessionTokens {
    pub(crate) fn set_app_policy_exempt(&mut self, exempt: bool) {
        self.session.mfa_exemption =
            exempt.then_some(crate::models::session::MfaExemption::ApprovedSignInPolicy);
    }

    pub(crate) fn set_mfa_assured(&mut self, assured: bool) {
        self.session.mfa_assured = Some(assured);
    }
}

pub async fn insert_prepared_session(
    db: &mongodb::Database,
    prepared: &PreparedSessionTokens,
    transaction: &mut mongodb::ClientSession,
) -> AppResult<()> {
    // Leave commit/abort ownership with the enclosing authentication workflow.
    db.collection::<Session>(SESSIONS)
        .insert_one(&prepared.session)
        .session(&mut *transaction)
        .await?;
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&prepared.refresh)
        .session(&mut *transaction)
        .await?;
    Ok(())
}

pub async fn prepare_session_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<PreparedSessionTokens> {
    prepare_session_tokens_with_assurance(
        db, config, jwt_keys, user_id, ip_address, user_agent, false,
    )
    .await
}

pub async fn prepare_session_tokens_with_assurance(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    mfa_assured: bool,
) -> AppResult<PreparedSessionTokens> {
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id: {e}")))?;

    let now = Utc::now();
    let session = Session {
        purpose: Some(crate::models::session::SessionPurpose::Authenticated),
        mfa_method: None,
        mfa_assured: Some(mfa_assured),
        mfa_exemption: None,
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_owned(),
        token_hash: hash_token(&generate_random_token()),
        ip_address: ip_address.map(str::to_owned),
        user_agent: user_agent.map(str::to_owned),
        expires_at: now + Duration::seconds(SESSION_TTL_SECS),
        revoked: false,
        created_at: now,
        last_active_at: now,
    };

    // Resolve RBAC data and inject into the access token based on scope
    let scope = FIRST_PARTY_ACCESS_SCOPES;
    let mut rbac_data =
        crate::services::rbac_helpers::build_rbac_claim_data(db, user_id, scope).await?;
    rbac_data.sid = Some(session.id.clone());
    let access_token = jwt::generate_access_token(
        jwt_keys,
        config,
        &user_uuid,
        scope,
        Some(&rbac_data),
        None,
        None,
        None,
        None,
    )?;

    // Generate refresh token
    let (refresh_token_jwt, refresh_jti) =
        jwt::generate_refresh_token(jwt_keys, config, &user_uuid)?;

    let refresh_id = Uuid::new_v4().to_string();
    let refresh_expires = now + Duration::seconds(config.jwt_refresh_ttl_secs);

    // Persist refresh token metadata
    let new_refresh = RefreshToken {
        protected_consent: None,
        id: refresh_id,
        jti: refresh_jti,
        client_id: Uuid::nil().to_string(), // first-party client
        user_id: user_id.to_string(),
        session_id: Some(session.id.clone()),
        scope: None,
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        revoked_at: None,
        resource_uris: Vec::new(),
        allowed_service_ids: Vec::new(),
        allow_all_services: true,
        created_at: now,
    };

    let tokens = IssuedTokens {
        access_token,
        refresh_token: refresh_token_jwt,
        session_id: session.id.clone(),
        access_expires_in: config.jwt_access_ttl_secs,
        resource_uris: Vec::new(),
    };
    Ok(PreparedSessionTokens {
        tokens,
        session,
        refresh: new_refresh,
    })
}

/// Create a short-lived pending MFA session.
///
/// This binds a temporary token hash to the user_id so the MFA verification
/// step can validate that the user already passed password authentication.
/// The challenge expires in 5 minutes. Its dedicated collection and explicit
/// server-controlled purpose separate it from completed login authority.
pub async fn create_mfa_pending_session_for_password(
    db: &mongodb::Database,
    user: &crate::models::user::User,
    temp_token_hash: &str,
) -> AppResult<String> {
    if !user.mfa_enabled {
        return Err(AppError::AuthenticationFailed("MFA state changed".into()));
    }
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    super::auth_service::fence_password_proof(db, user, &mut transaction).await?;
    let id = insert_mfa_pending_session_with_method(
        db,
        &user.id,
        temp_token_hash,
        Some("password"),
        Some(&mut transaction),
    )
    .await?;
    super::transaction_service::commit(&mut transaction).await?;
    Ok(id)
}
/// Create a first-factor MFA challenge and retain the non-secret method label
/// used for audit/telemetry after the challenge is redeemed. The raw token is
/// never persisted; only its hash is stored in the dedicated challenge store.
pub async fn create_mfa_pending_session_with_method(
    db: &mongodb::Database,
    user_id: &str,
    temp_token_hash: &str,
    method: Option<&str>,
) -> AppResult<String> {
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id": user_id, "is_active": true, "mfa_enabled": true,
            "user_type": {"$ne": "org"}, "platform_personal_deleting": {"$ne": true}},
            doc! {"$set": {"social_mfa_challenge_fence": uuid::Uuid::new_v4().to_string()}},
        )
        .session(&mut transaction)
        .await?
        .ok_or_else(|| AppError::AuthenticationFailed("MFA state changed".into()))?;
    let id = insert_mfa_pending_session_with_method(
        db,
        user_id,
        temp_token_hash,
        method,
        Some(&mut transaction),
    )
    .await?;
    super::transaction_service::commit(&mut transaction).await?;
    Ok(id)
}

/// All social first factors share the same transactionally fenced challenge.
pub async fn start_social_mfa_challenge(
    db: &mongodb::Database,
    user_id: &str,
    provider: &str,
) -> AppResult<zeroize::Zeroizing<String>> {
    let challenge = zeroize::Zeroizing::new(generate_random_token());
    create_mfa_pending_session_with_method(db, user_id, &hash_token(&challenge), Some(provider))
        .await?;
    Ok(challenge)
}

async fn insert_mfa_pending_session_with_method(
    db: &mongodb::Database,
    user_id: &str,
    temp_token_hash: &str,
    method: Option<&str>,
    transaction: Option<&mut mongodb::ClientSession>,
) -> AppResult<String> {
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires = now + Duration::minutes(5);

    let pending_session = Session {
        purpose: Some(crate::models::session::SessionPurpose::MfaPending),
        mfa_method: method.map(str::to_owned),
        mfa_assured: Some(false),
        mfa_exemption: None,
        id: session_id.clone(),
        user_id: user_id.to_string(),
        token_hash: temp_token_hash.to_string(),
        ip_address: None,
        user_agent: None,
        expires_at: expires,
        revoked: false,
        created_at: now,
        last_active_at: now,
    };

    let document = bson::to_document(&pending_session)
        .map_err(|_| AppError::Internal("Failed to encode MFA challenge".into()))?;
    let collection = db.collection::<bson::Document>(crate::models::session::MFA_CHALLENGES);
    let insert = collection.insert_one(document);
    if let Some(transaction) = transaction {
        insert.session(transaction).await?;
    } else {
        insert.await?;
    }

    Ok(session_id)
}

/// Refresh an expired access token using a valid refresh token.
///
/// Implements refresh token rotation: the old token is revoked and
/// a new refresh token is issued alongside the new access token.
/// Does NOT generate a new session token (reuses the existing session).
///
/// When `mcp_sessions` is provided, session revocations also cascade
/// to the affected user's bearer-less (session-fallback) MCP sessions.
pub async fn refresh_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    refresh_token_str: &str,
    mcp_sessions: Option<&McpSessionStore>,
    requested_resources: Option<&[String]>,
) -> AppResult<IssuedTokens> {
    // Verify the refresh JWT
    let claims = jwt::verify_token(jwt_keys, config, refresh_token_str)?;

    if claims.token_type != "refresh" {
        return Err(AppError::Unauthorized("Expected refresh token".to_string()));
    }

    // Look up the refresh token record by JTI
    let mut stored = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! { "jti": &claims.jti })
        .await?
        .ok_or_else(|| AppError::Unauthorized("Refresh token not found".to_string()))?;

    // A signed-out device is no longer authority, even when its refresh row
    // was batch-revoked. Reject before reuse detection can cascade again.
    let bound_first_party =
        stored.client_id == Uuid::nil().to_string() && stored.session_id.is_some();
    if bound_first_party {
        validate_refresh_session(db, &stored).await?;
    }

    // Re-validate the issuing OAuth client. First-party login flows
    // (`auth_service::login_with_password`, `refresh_session_with_token`)
    // store `client_id = Uuid::nil()` as a sentinel and have no
    // `OauthClient` row -- skip the lookup for those. Real OAuth clients
    // must still exist AND be active; without this check, a refresh
    // token minted by a developer app would remain usable forever after
    // the app (or its owning org) is deleted, because the standard
    // delete path is a soft-delete that leaves the row in the
    // collection. The auth-code path already filters by `is_active`
    // (see `oauth_service::exchange_authorization_code`), so this is
    // the matching gate on the refresh side.
    if stored.client_id != Uuid::nil().to_string() {
        let client_active = db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": &stored.client_id, "is_active": true })
            .await?
            .is_some();
        if !client_active {
            tracing::warn!(
                client_id = %stored.client_id,
                reason = "client_inactive",
                route_class = "refresh",
                "Protected OAuth authority rejected"
            );
            // Revoke the token in place so subsequent retries follow
            // the existing reuse-detection path instead of leaking the
            // window between deletion and the next refresh attempt.
            db.collection::<RefreshToken>(REFRESH_TOKENS)
                .update_one(
                    doc! { "_id": &stored.id, "revoked": false },
                    doc! { "$set": {
                        "revoked": true,
                        "revoked_at": bson::DateTime::from_chrono(Utc::now()),
                    }},
                )
                .await?;
            // A distinct variant so `/oauth/token` keeps reporting this as
            // `invalid_client` (the client must re-register) while every
            // other refresh failure becomes `invalid_grant`. Same HTTP 401.
            return Err(AppError::AuthenticationFailed(
                "Issuing OAuth client is no longer active".to_string(),
            ));
        }
    }

    // PD-3: only an unused legacy OAuth record demonstrably newer than the
    // current grant may acquire its revision. Keep the original row unbound:
    // once consumed, it must never enter the normal rotation retry grace path.
    let legacy_upgrade =
        stored.client_id != Uuid::nil().to_string() && stored.protected_consent.is_none();
    if legacy_upgrade {
        if stored.revoked || stored.expires_at <= Utc::now() {
            return Err(AppError::Unauthorized(
                "Refresh token has expired or been revoked".into(),
            ));
        }
        let consent =
            super::consent_service::check_consent(db, &stored.user_id, &stored.client_id, "")
                .await?
                .ok_or_else(super::consent_service::grant_changed)?;
        if stored.created_at <= consent.granted_at + Duration::seconds(60) {
            tracing::warn!(client_id = %stored.client_id, route_class = "refresh", reason = "legacy_unbound", "Protected OAuth authority rejected");
            return Err(super::consent_service::grant_changed());
        }
        let scope = stored
            .scope
            .as_deref()
            .unwrap_or(FIRST_PARTY_ACCESS_SCOPES)
            .split_whitespace()
            .filter(|scope| {
                *scope != "machine"
                    && consent
                        .scopes
                        .split_whitespace()
                        .any(|granted| granted == *scope)
            })
            .collect::<Vec<_>>()
            .join(" ");
        if scope.is_empty() {
            return Err(super::consent_service::grant_changed());
        }
        // Pre-default-deny consent has no service authority, including for a
        // legacy resource-bearing record. Preserve identity-only refreshes.
        if !consent.allow_all_services
            && consent.allowed_service_ids.is_none()
            && !stored.resource_uris.is_empty()
        {
            return Err(super::consent_service::grant_changed());
        }
        let consent_services = consent.allowed_service_ids.as_deref().unwrap_or_default();
        let allowed_service_ids = match (stored.allow_all_services, consent.allow_all_services) {
            (true, true) => Vec::new(),
            (true, false) => consent_services.to_vec(),
            (false, true) => stored.allowed_service_ids.clone(),
            (false, false) => stored
                .allowed_service_ids
                .iter()
                .filter(|id| consent_services.contains(id))
                .cloned()
                .collect(),
        };
        let allow_all_services = stored.allow_all_services && consent.allow_all_services;
        // Validate every retained URI before ensure_issuance_reference can write
        // an unversioned consent or rotation can consume the legacy record.
        if !stored.resource_uris.is_empty() {
            let resolved = super::oauth_resource_service::resolve_requested_resources(
                db,
                config,
                &stored.user_id,
                Some(&stored.resource_uris),
            )
            .await
            .map_err(|error| match error {
                AppError::InvalidTarget(_)
                | AppError::RequiredServiceNotConnected { .. }
                | AppError::Forbidden(_)
                | AppError::NotFound(_) => super::consent_service::grant_changed(),
                error => error,
            })?
            .ok_or_else(super::consent_service::grant_changed)?;
            if !allow_all_services
                && resolved
                    .service_ids
                    .iter()
                    .any(|id| !allowed_service_ids.contains(id))
            {
                return Err(super::consent_service::grant_changed());
            }
        }
        stored.protected_consent =
            Some(super::consent_service::ensure_issuance_reference(db, &consent).await?);
        stored.scope = Some(scope);
        stored.allowed_service_ids = allowed_service_ids;
        stored.allow_all_services = allow_all_services;
    }
    let original_protected_consent = stored.protected_consent.clone();
    // If the token is revoked, check if this is a post-rotation retry
    // (client retried with old token after restart) vs actual token reuse.
    //
    // The primary indicator of rotation is `replaced_by` being set -- batch
    // revocations (revoke_session, explicit revoke) never set `replaced_by`.
    // The time-based grace period is a secondary constraint: if `revoked_at`
    // is present, we only allow retries within REUSE_GRACE_PERIOD_SECS.
    // If `revoked_at` is `None` (tokens rotated before this field was added),
    // we still check the replacement chain for a valid unused token.
    //
    // When multiple concurrent requests use the same old token, the first
    // succeeds and rotates the replacement. Subsequent requests must converge
    // on the already-active replacement token rather than rotate the chain
    // again, otherwise stale responses can overwrite the browser cookie with
    // an older token and eventually trigger false theft detection.
    let disposition = if stored.revoked {
        let within_grace = stored
            .revoked_at
            .map(|ra| (Utc::now() - ra).num_seconds() <= REUSE_GRACE_PERIOD_SECS)
            .unwrap_or(true); // None means pre-migration token; allow chain check

        match (&stored.replaced_by, within_grace) {
            (Some(first_replacement_id), true) => {
                match follow_active_replacement_chain(
                    db,
                    first_replacement_id,
                    &stored.user_id,
                    &claims.jti,
                )
                .await?
                {
                    Some(token) => RefreshTokenDisposition::Reuse(token),
                    None => {
                        // Chain exhausted without finding a valid token.
                        // This is actual token reuse -- revoke the session.
                        tracing::warn!(
                            user_id = %stored.user_id,
                            jti = %claims.jti,
                            "Refresh token reuse detected, revoking session"
                        );
                        if let Some(ref session_id) = stored.session_id {
                            revoke_session(db, session_id, mcp_sessions).await?;
                        }
                        return Err(AppError::Unauthorized(
                            "Refresh token has been revoked".to_string(),
                        ));
                    }
                }
            }
            (None, _) => {
                // No replacement -- this was a batch/explicit revocation, not rotation.
                tracing::warn!(
                    user_id = %stored.user_id,
                    jti = %claims.jti,
                    "Refresh token reuse detected (explicitly revoked), revoking session"
                );
                if let Some(ref session_id) = stored.session_id {
                    revoke_session(db, session_id, mcp_sessions).await?;
                }
                return Err(AppError::Unauthorized(
                    "Refresh token has been revoked".to_string(),
                ));
            }
            (Some(_), false) => {
                // Outside grace period -- too old to be a legitimate retry.
                tracing::warn!(
                    user_id = %stored.user_id,
                    jti = %claims.jti,
                    "Refresh token reuse detected (outside grace period), revoking session"
                );
                if let Some(ref session_id) = stored.session_id {
                    revoke_session(db, session_id, mcp_sessions).await?;
                }
                return Err(AppError::Unauthorized(
                    "Refresh token has been revoked".to_string(),
                ));
            }
        }
    } else {
        RefreshTokenDisposition::Rotate(stored)
    };

    let (mut active_token, reuse_existing_refresh_token) = match disposition {
        RefreshTokenDisposition::Rotate(token) => (token, false),
        RefreshTokenDisposition::Reuse(token) => (token, true),
    };
    if active_token.protected_consent != original_protected_consent {
        return Err(super::consent_service::grant_changed());
    }
    super::consent_service::validate_protected_reference_for_route(
        db,
        &active_token.user_id,
        &active_token.client_id,
        active_token
            .scope
            .as_deref()
            .unwrap_or(FIRST_PARTY_ACCESS_SCOPES),
        active_token.protected_consent.as_ref(),
        "refresh",
    )
    .await?;
    if active_token.client_id == Uuid::nil().to_string() {
        if active_token.session_id.is_none() {
            active_token.session_id = Some(bind_legacy_refresh_session(db, &active_token).await?);
        }
        if !bound_first_party {
            validate_refresh_session(db, &active_token).await?;
        }
    }
    let user_id_str = active_token.user_id.clone();
    let user_id = Uuid::parse_str(&user_id_str)
        .map_err(|e| AppError::Internal(format!("Invalid user_id in refresh token: {e}")))?;
    let session_id = active_token.session_id.clone();
    let now = Utc::now();

    // Resolve RBAC data and inject into the refreshed access token. OAuth
    // refresh-token rows persist the originally granted scope; legacy and
    // first-party rows fall back to the historical first-party scope string.
    let scope = active_token
        .scope
        .as_deref()
        .unwrap_or(FIRST_PARTY_ACCESS_SCOPES);
    let token_resource_scope =
        crate::services::oauth_resource_service::resolve_token_resource_scope(
            db,
            config,
            &user_id_str,
            requested_resources,
            &active_token.resource_uris,
            &active_token.allowed_service_ids,
            active_token.allow_all_services,
        )
        .await?;
    let mut rbac_data =
        crate::services::rbac_helpers::build_rbac_claim_data(db, &user_id_str, scope).await?;
    if active_token.client_id == Uuid::nil().to_string() {
        rbac_data.sid = session_id.clone();
    }
    let restrictions =
        (!token_resource_scope.allow_all_services).then_some(jwt::AccessTokenRestrictions {
            resources: &token_resource_scope.resource_uris,
            allowed_service_ids: &token_resource_scope.allowed_service_ids,
            allowed_node_ids: &[],
            allow_all_nodes: true,
        });
    let new_access = if active_token.client_id == Uuid::nil().to_string() {
        jwt::generate_access_token(
            jwt_keys,
            config,
            &user_id,
            scope,
            Some(&rbac_data),
            None,
            None,
            None,
            restrictions,
        )?
    } else {
        jwt::generate_oauth_access_token_with_consent(
            jwt_keys,
            config,
            &user_id,
            scope,
            Some(&rbac_data),
            None,
            None,
            None,
            restrictions,
            &active_token.client_id,
            active_token.protected_consent.as_ref(),
        )?
    };

    if reuse_existing_refresh_token {
        super::consent_service::fence_protected_reference_for_route(
            db,
            &active_token.user_id,
            &active_token.client_id,
            scope,
            active_token.protected_consent.as_ref(),
            "refresh",
        )
        .await?;
        touch_session_last_active(db, session_id.as_deref(), now).await?;
        return build_reused_refresh_response(
            jwt_keys,
            config,
            &user_id,
            &active_token,
            new_access,
            token_resource_scope.resource_uris,
        );
    }

    // Issue new refresh token (rotation)
    let (new_refresh_jwt, new_jti) = jwt::generate_refresh_token(jwt_keys, config, &user_id)?;
    let new_refresh_id = Uuid::new_v4().to_string();
    let refresh_expires = now + Duration::seconds(config.jwt_refresh_ttl_secs);

    // Atomically revoke the active refresh token using find_one_and_update
    // with a "revoked: false" guard. This prevents two concurrent rotation
    // requests from both succeeding (only the first CAS wins).
    let revoked = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one_and_update(
            doc! { "_id": &active_token.id, "revoked": false },
            doc! { "$set": {
                "revoked": true,
                "revoked_at": bson::DateTime::from_chrono(now),
                "replaced_by": &new_refresh_id,
            }},
        )
        .await?;

    if revoked.is_none() {
        if legacy_upgrade {
            return Err(super::consent_service::grant_changed());
        }
        // Another concurrent request rotated this token after we loaded it.
        // Recover the new active replacement token and return it so clients
        // converge on the same cookie value instead of logging out.
        let current = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "_id": &active_token.id })
            .await?;

        if let Some(current_token) = current
            && let Some(first_replacement_id) = current_token.replaced_by.as_deref()
            && let Some(recovered) = follow_active_replacement_chain(
                db,
                first_replacement_id,
                &active_token.user_id,
                &active_token.jti,
            )
            .await?
        {
            tracing::info!(
                user_id = %active_token.user_id,
                jti = %active_token.jti,
                replacement_id = %recovered.id,
                "Concurrent refresh rotation resolved to active replacement token"
            );
            if recovered.protected_consent != active_token.protected_consent {
                return Err(super::consent_service::grant_changed());
            }
            super::consent_service::fence_protected_reference_for_route(
                db,
                &active_token.user_id,
                &active_token.client_id,
                scope,
                active_token.protected_consent.as_ref(),
                "refresh",
            )
            .await?;
            touch_session_last_active(db, session_id.as_deref(), now).await?;
            return build_reused_refresh_response(
                jwt_keys,
                config,
                &user_id,
                &recovered,
                new_access,
                token_resource_scope.resource_uris,
            );
        }

        return Err(AppError::Conflict(
            "Refresh token was concurrently rotated, please retry".to_string(),
        ));
    }

    // Persist new refresh token
    let new_refresh = RefreshToken {
        protected_consent: active_token.protected_consent.clone(),
        id: new_refresh_id,
        jti: new_jti,
        client_id: active_token.client_id.clone(),
        user_id: user_id_str,
        session_id: session_id.clone(),
        scope: active_token.scope.clone(),
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        revoked_at: None,
        resource_uris: active_token.resource_uris.clone(),
        allowed_service_ids: active_token.allowed_service_ids.clone(),
        allow_all_services: active_token.allow_all_services,
        created_at: now,
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&new_refresh)
        .await?;

    if let Err(error) = super::consent_service::fence_protected_reference_for_route(
        db,
        &active_token.user_id,
        &active_token.client_id,
        scope,
        active_token.protected_consent.as_ref(),
        "refresh",
    )
    .await
    {
        super::oauth_service::revoke_issued_refresh(db, &new_refresh.jti).await?;
        return Err(error);
    }
    touch_session_last_active(db, session_id.as_deref(), now).await?;

    // Reuse the existing session token rather than generating a new orphan token.
    // The session cookie does not need to change on token refresh.
    // Return an empty session_token since the cookie should not be updated.
    Ok(IssuedTokens {
        access_token: new_access,
        refresh_token: new_refresh_jwt,
        session_id: session_id.unwrap_or_else(|| Uuid::nil().to_string()),
        access_expires_in: config.jwt_access_ttl_secs,
        resource_uris: token_resource_scope.resource_uris,
    })
}

/// Old first-party issuers had no sid. The first refresh creates one fixed
/// 30-day anchor and binds the active token in the same transaction. Retries
/// and every successor reuse it, so rolling refresh cannot extend legacy MFA.
async fn bind_legacy_refresh_session(
    db: &mongodb::Database,
    token: &RefreshToken,
) -> AppResult<String> {
    let context = (db.clone(), token.id.clone(), token.user_id.clone());
    super::transaction_service::run_db_only(db, &context, |tx, c| Box::pin(async move {
        let (db, id, user) = c;
        let row = db.collection::<RefreshToken>(REFRESH_TOKENS).find_one(doc! {
            "_id": id, "user_id": user, "revoked": false, "expires_at": {"$gt": bson::DateTime::now()}
        }).session(&mut *tx).await?.ok_or_else(|| AppError::Unauthorized("Refresh token has expired or been revoked".into()))?;
        if let Some(sid) = row.session_id { return Ok(sid); }
        let issued = create_session_with_transaction(db, user, None, None, Some(&mut *tx)).await?;
        db.collection::<bson::Document>(SESSIONS).update_one(doc! {"_id": &issued.session_id},
            doc! {"$set": {"mfa_exemption": "legacy_migration"}}).session(&mut *tx).await?;
        db.collection::<RefreshToken>(REFRESH_TOKENS).update_one(doc! {"_id": id},
            doc! {"$set": {"session_id": &issued.session_id}}).session(&mut *tx).await?;
        Ok(issued.session_id)
    })).await
}

async fn validate_refresh_session(db: &mongodb::Database, token: &RefreshToken) -> AppResult<()> {
    let user = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": &token.user_id, "is_active": true})
        .await?
        .ok_or_else(|| AppError::Unauthorized("User account is inactive".into()))?;
    // Refresh is always bounded by the session, including non-MFA accounts.
    let live = db
        .collection::<Session>(SESSIONS)
        .find_one(super::session_authority::authenticated(doc! {
            "_id": &token.session_id, "user_id": &token.user_id, "revoked": false,
            "expires_at": {"$gt": bson::DateTime::now()}
        }))
        .await?;
    let live = live.ok_or_else(|| AppError::Unauthorized("Session expired or revoked".into()))?;
    if user.mfa_enabled && !super::session_authority::satisfies_mfa(&live) {
        super::session_authority::reject_unassured(
            db,
            &token.user_id,
            token.session_id.as_deref(),
            "refresh",
            None,
            None,
        )
        .await?;
        return Err(AppError::Unauthorized(
            "MFA verification required; sign in again".into(),
        ));
    }
    Ok(())
}

/// Issue a mobile social login with the rollout decision persisted before any
/// credentials are returned. Other first-factor flows do not use this entry.
pub async fn create_mobile_social_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    keys: &JwtKeys,
    user_id: &str,
    ip: Option<&str>,
    ua: Option<&str>,
    exempt: bool,
) -> AppResult<IssuedTokens> {
    let mut prepared = prepare_session_tokens(db, config, keys, user_id, ip, ua).await?;
    if exempt {
        prepared.session.mfa_exemption = Some(crate::models::session::MfaExemption::MobileRollout);
    }
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    insert_prepared_session(db, &prepared, &mut tx).await?;
    super::transaction_service::commit(&mut tx).await?;
    Ok(prepared.tokens)
}

/// Transactional session/refresh revocation for an undelivered login grant.
/// Such a session has not been returned to a client and cannot have MCP children.
pub(crate) async fn revoke_session_in_transaction(
    db: &mongodb::Database,
    id: &str,
    tx: &mut mongodb::ClientSession,
) -> AppResult<()> {
    db.collection::<Session>(SESSIONS)
        .update_one(
            doc! {"_id": id},
            doc! {"$set": {"revoked": true, "last_active_at": bson::DateTime::now()}},
        )
        .session(&mut *tx)
        .await?;
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_many(
            doc! {"session_id": id, "revoked": false},
            doc! {"$set": {"revoked": true}},
        )
        .session(&mut *tx)
        .await?;
    Ok(())
}

/// Revoke a session and all its associated refresh tokens.
///
/// Uses batch update where possible to avoid N+1 queries.
/// When `mcp_sessions` is provided, also cascades to MCP sessions for the user.
pub async fn revoke_session(
    db: &mongodb::Database,
    session_id: &str,
    mcp_sessions: Option<&McpSessionStore>,
) -> AppResult<()> {
    // Look up the session to get the user_id for MCP cascade
    let session_doc = db
        .collection::<Session>(SESSIONS)
        .find_one(doc! { "_id": session_id })
        .await?;

    // Revoke the session
    db.collection::<Session>(SESSIONS)
        .update_one(
            doc! { "_id": session_id },
            doc! { "$set": { "revoked": true } },
        )
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;

    // Revoke all refresh tokens for this session in a batch
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_many(
            doc! { "session_id": session_id, "revoked": false },
            doc! { "$set": { "revoked": true } },
        )
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;

    // Cascade: end the user's MCP sessions that authenticate without a bearer
    if let (Some(mcp), Some(session)) = (mcp_sessions, &session_doc) {
        mcp.remove_fallback_sessions_by_user_id(&session.user_id)
            .await?;
    }

    tracing::info!(session_id = %session_id, "Session revoked");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    async fn seed_user(db: &mongodb::Database, user_id: &str) {
        let user = test_user(user_id, crate::models::user::UserType::Person);
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(&user)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dead_bound_refresh_rejects_before_reuse_without_cascading_again() {
        let db = connect_transaction_test_database("refresh_early_gate").await;
        let state = test_app_state(db.clone());
        let id = Uuid::new_v4().to_string();
        seed_user(&db, &id).await;
        for expired in [false, true] {
            let tokens = create_session_and_issue_tokens(
                &db,
                &state.config,
                &state.jwt_keys,
                &id,
                None,
                None,
            )
            .await
            .unwrap();
            revoke_session(&db, &tokens.session_id, None).await.unwrap();
            if expired {
                db.collection::<Session>(SESSIONS).update_one(doc! {"_id": &tokens.session_id},
                    doc! {"$set": {"revoked": false, "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
            }
            // A repeated reuse cascade would remove this unrelated fallback.
            let sentinel = state
                .mcp_sessions
                .create_with_parent_session(&id, true, true, None)
                .await
                .unwrap()
                .unwrap();
            let before = db
                .collection::<RefreshToken>(REFRESH_TOKENS)
                .find_one(doc! {"session_id": &tokens.session_id})
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(refresh_tokens(&db, &state.config, &state.jwt_keys, &tokens.refresh_token,
                Some(&state.mcp_sessions), None).await,
                Err(AppError::Unauthorized(message)) if message == "Session expired or revoked")
            );
            assert!(
                state
                    .mcp_sessions
                    .get_for_auth(&sentinel)
                    .await
                    .unwrap()
                    .is_some()
            );
            let after = db
                .collection::<RefreshToken>(REFRESH_TOKENS)
                .find_one(doc! {"_id": &before.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                bson::to_document(&before).unwrap(),
                bson::to_document(&after).unwrap()
            );
        }
        // MFA policy is also checked before reuse disposition. A live explicit
        // unverified session must take the single policy-rejection path even
        // if its refresh row has already been revoked.
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id": &id}, doc! {"$set": {"mfa_enabled": true}})
            .await
            .unwrap();
        let tokens =
            create_session_and_issue_tokens(&db, &state.config, &state.jwt_keys, &id, None, None)
                .await
                .unwrap();
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_one(
                doc! {"session_id": &tokens.session_id},
                doc! {"$set": {"revoked": true}},
            )
            .await
            .unwrap();
        assert!(
            matches!(refresh_tokens(&db, &state.config, &state.jwt_keys, &tokens.refresh_token, None, None).await,
            Err(AppError::Unauthorized(message)) if message == "MFA verification required; sign in again")
        );
        assert!(
            matches!(refresh_tokens(&db, &state.config, &state.jwt_keys, &tokens.refresh_token, None, None).await,
            Err(AppError::Unauthorized(message)) if message == "Session expired or revoked")
        );
        assert_eq!(db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"user_id": &id, "event_type": "mfa_unassured_session_rejected", "event_data.session_id": &tokens.session_id}).await.unwrap(), 1);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn mobile_social_without_exemption_writes_explicit_unverified_authority() {
        let db = connect_transaction_test_database("mobile_no_exemption").await;
        let state = test_app_state(db.clone());
        let id = Uuid::new_v4().to_string();
        seed_user(&db, &id).await;
        let tokens = create_mobile_social_tokens(
            &db,
            &state.config,
            &state.jwt_keys,
            &id,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        let row = db
            .collection::<bson::Document>(SESSIONS)
            .find_one(doc! {"_id": &tokens.session_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.get_bool("mfa_assured"), Ok(false));
        assert!(!row.contains_key("mfa_exemption"));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn test_create_session_inserts_record() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let result = create_session(&db, &user_id, Some("127.0.0.1"), Some("test-agent")).await;
        assert!(result.is_ok());

        let issued = result.unwrap();
        assert!(!issued.session_token.is_empty());
        assert!(!issued.session_id.is_empty());

        let stored = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &issued.session_id })
            .await
            .unwrap();
        assert!(stored.is_some());
        let stored = stored.unwrap();
        assert_eq!(stored.user_id, user_id);
        assert!(!stored.revoked);
        assert_eq!(stored.ip_address.as_deref(), Some("127.0.0.1"));
        assert_eq!(stored.user_agent.as_deref(), Some("test-agent"));
    }

    #[tokio::test]
    async fn test_create_session_rejects_invalid_uuid() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let result = create_session(&db, "not-a-uuid", None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_session_with_none_optional_fields() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session(&db, &user_id, None, None).await.unwrap();
        let stored = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &issued.session_id })
            .await
            .unwrap()
            .unwrap();
        assert!(stored.ip_address.is_none());
        assert!(stored.user_agent.is_none());
    }

    #[tokio::test]
    async fn test_create_session_and_issue_tokens_returns_valid_tokens() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let result = create_session_and_issue_tokens(
            &db,
            &config,
            &jwt_keys,
            &user_id,
            Some("10.0.0.1"),
            Some("browser/1.0"),
        )
        .await;
        assert!(result.is_ok());

        let issued = result.unwrap();
        assert!(!issued.access_token.is_empty());
        assert!(!issued.refresh_token.is_empty());
        assert!(!issued.session_id.is_empty());
        assert_eq!(issued.access_expires_in, config.jwt_access_ttl_secs);

        let access_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.access_token).unwrap();
        assert_eq!(access_claims.sub, user_id);
        assert_eq!(access_claims.token_type, "access");

        let refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        assert_eq!(refresh_claims.sub, user_id);
        assert_eq!(refresh_claims.token_type, "refresh");
    }

    #[tokio::test]
    async fn test_create_session_and_issue_tokens_persists_refresh_token() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session_and_issue_tokens(&db, &config, &jwt_keys, &user_id, None, None)
            .await
            .unwrap();

        let refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        let stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &refresh_claims.jti })
            .await
            .unwrap();
        assert!(stored.is_some());
        let stored = stored.unwrap();
        assert_eq!(stored.user_id, user_id);
        assert!(!stored.revoked);
        assert_eq!(
            stored.session_id.as_deref(),
            Some(issued.session_id.as_str())
        );
    }

    #[tokio::test]
    async fn test_refresh_tokens_rotates_refresh_token() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session_and_issue_tokens(&db, &config, &jwt_keys, &user_id, None, None)
            .await
            .unwrap();

        let refreshed = refresh_tokens(&db, &config, &jwt_keys, &issued.refresh_token, None, None)
            .await
            .unwrap();

        assert!(!refreshed.access_token.is_empty());
        assert!(!refreshed.refresh_token.is_empty());
        assert_ne!(refreshed.refresh_token, issued.refresh_token);

        let old_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        let old_stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &old_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(old_stored.revoked);
        assert!(old_stored.replaced_by.is_some());
    }

    #[tokio::test]
    async fn test_refresh_tokens_preserves_oauth_service_scope() {
        let Some(db) = connect_test_database("token_svc_oauth_scope").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let client_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.clone(),
                client_name: "Scoped OAuth Client".to_string(),
                client_secret_hash: String::new(),
                redirect_uris: vec!["https://app.example/callback".to_string()],
                allowed_scopes: "openid proxy".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                default_service_catalog_slugs: Vec::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: Some(user_id.clone()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let user_uuid = Uuid::parse_str(&user_id).unwrap();
        let (refresh_jwt, refresh_jti) =
            crate::crypto::jwt::generate_refresh_token(&jwt_keys, &config, &user_uuid).unwrap();
        let allowed_service_ids = vec!["svc-1".to_string()];
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(RefreshToken {
                protected_consent: None,
                id: Uuid::new_v4().to_string(),
                jti: refresh_jti,
                client_id: client_id.clone(),
                user_id: user_id.clone(),
                session_id: None,
                scope: Some("openid proxy".to_string()),
                expires_at: now + Duration::days(7),
                revoked: false,
                replaced_by: None,
                revoked_at: None,
                resource_uris: Vec::new(),
                allowed_service_ids: allowed_service_ids.clone(),
                allow_all_services: false,
                created_at: now,
            })
            .await
            .unwrap();

        // Explicit legacy rejection: even a currently active client cannot
        // upgrade an unbound refresh token using a later consent grant.
        let consent = super::super::consent_service::grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid proxy",
            Some(allowed_service_ids.clone()),
        )
        .await
        .unwrap();
        let protected = super::super::consent_service::ensure_issuance_reference(&db, &consent)
            .await
            .unwrap();
        assert!(matches!(
            refresh_tokens(&db, &config, &jwt_keys, &refresh_jwt, None, None).await,
            Err(AppError::Conflict(_))
        ));
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_one(
                doc! {"user_id":&user_id,"client_id":&client_id,"revoked":false},
                doc! {"$set":{"protected_consent":bson::to_bson(&protected).unwrap()}},
            )
            .await
            .unwrap();

        let refreshed = refresh_tokens(&db, &config, &jwt_keys, &refresh_jwt, None, None)
            .await
            .unwrap();
        let access_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refreshed.access_token).unwrap();
        assert_eq!(access_claims.scope, "openid proxy");
        assert_eq!(access_claims.client_id.as_deref(), Some(client_id.as_str()));
        assert_eq!(access_claims.allow_all_services, Some(false));
        assert_eq!(access_claims.allowed_service_ids, Some(allowed_service_ids));

        let new_refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refreshed.refresh_token).unwrap();
        let new_stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": new_refresh_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(new_stored.scope.as_deref(), Some("openid proxy"));
        assert!(!new_stored.allow_all_services);
        assert_eq!(new_stored.allowed_service_ids, vec!["svc-1".to_string()]);
    }

    #[tokio::test]
    async fn test_refresh_resource_narrowing_is_per_request() {
        let Some(db) = connect_test_database("token_svc_resource_per_request").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let service_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(test_user_service(
            &service_id,
            &user_id,
            "svc-a",
            "endpoint-a",
            None,
            None,
        ))
        .await
        .unwrap();
        let resource_a =
            crate::services::oauth_resource_service::user_service_resource_uri(&config, "svc-a");

        let issued = create_session_and_issue_tokens(&db, &config, &jwt_keys, &user_id, None, None)
            .await
            .unwrap();

        let requested = vec![resource_a.clone()];
        let narrowed = refresh_tokens(
            &db,
            &config,
            &jwt_keys,
            &issued.refresh_token,
            None,
            Some(&requested),
        )
        .await
        .unwrap();

        let access_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &narrowed.access_token).unwrap();
        assert_eq!(access_claims.resources, Some(vec![resource_a.clone()]));
        assert_eq!(access_claims.allowed_service_ids, Some(vec![service_id]));
        assert_eq!(access_claims.allow_all_services, Some(false));

        let narrowed_refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &narrowed.refresh_token).unwrap();
        let narrowed_stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": narrowed_refresh_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(narrowed_stored.allow_all_services);
        assert!(narrowed_stored.resource_uris.is_empty());
        assert!(narrowed_stored.allowed_service_ids.is_empty());

        let unrestricted =
            refresh_tokens(&db, &config, &jwt_keys, &narrowed.refresh_token, None, None)
                .await
                .unwrap();
        let unrestricted_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &unrestricted.access_token)
                .unwrap();
        assert!(unrestricted_claims.resources.is_none());
        assert!(unrestricted_claims.allowed_service_ids.is_none());
        assert!(unrestricted_claims.allow_all_services.is_none());
    }

    #[tokio::test]
    async fn test_refresh_accepts_resource_within_consent_only_service_grant() {
        let Some(db) = connect_test_database("token_svc_consent_only_resource").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;
        let client_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.clone(),
                client_name: "Consent Only OAuth Client".to_string(),
                client_secret_hash: String::new(),
                redirect_uris: vec!["https://app.example/callback".to_string()],
                allowed_scopes: "openid proxy".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: Some(user_id.clone()),
                default_service_catalog_slugs: Vec::new(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let service_a_id = Uuid::new_v4().to_string();
        let service_b_id = Uuid::new_v4().to_string();
        let services = db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        );
        services
            .insert_many([
                test_user_service(&service_a_id, &user_id, "svc-a", "endpoint-a", None, None),
                test_user_service(&service_b_id, &user_id, "svc-b", "endpoint-b", None, None),
            ])
            .await
            .unwrap();
        let resource_a =
            crate::services::oauth_resource_service::user_service_resource_uri(&config, "svc-a");
        let resource_b =
            crate::services::oauth_resource_service::user_service_resource_uri(&config, "svc-b");

        let consent = super::super::consent_service::grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid proxy",
            Some(vec![service_a_id.clone()]),
        )
        .await
        .unwrap();
        let protected = super::super::consent_service::ensure_issuance_reference(&db, &consent)
            .await
            .unwrap();

        let user_uuid = Uuid::parse_str(&user_id).unwrap();
        let (refresh_jwt, refresh_jti) =
            crate::crypto::jwt::generate_refresh_token(&jwt_keys, &config, &user_uuid).unwrap();
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(RefreshToken {
                protected_consent: Some(protected),
                id: Uuid::new_v4().to_string(),
                jti: refresh_jti,
                client_id: client_id.clone(),
                user_id: user_id.clone(),
                session_id: None,
                scope: Some("openid proxy".to_string()),
                expires_at: now + Duration::days(7),
                revoked: false,
                replaced_by: None,
                revoked_at: None,
                resource_uris: Vec::new(),
                allowed_service_ids: vec![service_a_id.clone()],
                allow_all_services: false,
                created_at: now,
            })
            .await
            .unwrap();

        let requested_a = vec![resource_a.clone()];
        let refreshed = refresh_tokens(
            &db,
            &config,
            &jwt_keys,
            &refresh_jwt,
            None,
            Some(&requested_a),
        )
        .await
        .unwrap();
        let access_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refreshed.access_token).unwrap();
        assert_eq!(access_claims.resources, Some(vec![resource_a]));
        assert_eq!(
            access_claims.allowed_service_ids,
            Some(vec![service_a_id.clone()])
        );
        assert_eq!(access_claims.allow_all_services, Some(false));

        let new_refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &refreshed.refresh_token).unwrap();
        let new_stored = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": new_refresh_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(!new_stored.allow_all_services);
        assert!(new_stored.resource_uris.is_empty());
        assert_eq!(new_stored.allowed_service_ids, vec![service_a_id]);

        let requested_b = vec![resource_b];
        let err = match refresh_tokens(
            &db,
            &config,
            &jwt_keys,
            &refreshed.refresh_token,
            None,
            Some(&requested_b),
        )
        .await
        {
            Ok(_) => panic!("resource outside consented services is rejected"),
            Err(err) => err,
        };
        assert!(matches!(err, AppError::InvalidTarget(_)));
    }

    #[tokio::test]
    async fn test_refresh_tokens_rejects_non_refresh_token() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4();

        let access_token = crate::crypto::jwt::generate_access_token(
            &jwt_keys, &config, &user_id, "openid", None, None, None, None, None,
        )
        .unwrap();

        let result = refresh_tokens(&db, &config, &jwt_keys, &access_token, None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_refresh_tokens_rejects_revoked_token_no_replacement() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session_and_issue_tokens(&db, &config, &jwt_keys, &user_id, None, None)
            .await
            .unwrap();

        let claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_one(
                doc! { "jti": &claims.jti },
                doc! { "$set": { "revoked": true } },
            )
            .await
            .unwrap();

        let result =
            refresh_tokens(&db, &config, &jwt_keys, &issued.refresh_token, None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_revoke_session_marks_session_and_tokens() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        seed_user(&db, &user_id).await;

        let issued = create_session_and_issue_tokens(&db, &config, &jwt_keys, &user_id, None, None)
            .await
            .unwrap();

        revoke_session(&db, &issued.session_id, None).await.unwrap();

        let session = db
            .collection::<Session>(SESSIONS)
            .find_one(doc! { "_id": &issued.session_id })
            .await
            .unwrap()
            .unwrap();
        assert!(session.revoked);

        let refresh_claims =
            crate::crypto::jwt::verify_token(&jwt_keys, &config, &issued.refresh_token).unwrap();
        let refresh = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &refresh_claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(refresh.revoked);
    }

    #[tokio::test]
    async fn test_create_mfa_pending_session_with_method() {
        let Some(db) = connect_test_database("token_svc").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, crate::models::user::UserType::Person);
        user.mfa_enabled = true;
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(user)
            .await
            .unwrap();

        let temp_hash = "abc123hash";
        let session_id =
            create_mfa_pending_session_with_method(&db, &user_id, temp_hash, Some("password"))
                .await
                .unwrap();

        let session = db
            .collection::<Session>(crate::models::session::MFA_CHALLENGES)
            .find_one(doc! { "_id": &session_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.user_id, user_id);
        assert_eq!(session.token_hash, temp_hash);
        assert_eq!(
            session.purpose,
            Some(crate::models::session::SessionPurpose::MfaPending)
        );
        assert_eq!(session.mfa_method.as_deref(), Some("password"));
        assert!(session.user_agent.is_none());
        assert!(!session.revoked);
    }
}

#[cfg(test)]
#[path = "token_service/legacy_tests.rs"]
mod legacy_tests;
