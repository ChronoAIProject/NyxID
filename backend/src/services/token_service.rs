use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::{self, JwtKeys};
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::mcp_session::McpSessionStore;
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

/// Tokens issued after successful authentication.
pub struct IssuedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: String,
    pub access_expires_in: i64,
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
    })
}

/// Create a new session and issue JWT tokens.
pub async fn create_session(
    db: &mongodb::Database,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<IssuedSession> {
    Uuid::parse_str(user_id).map_err(|e| AppError::Internal(format!("Invalid user_id: {e}")))?;

    let session_token = generate_random_token();
    let session_token_hash = hash_token(&session_token);
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let session_expires = now + Duration::seconds(SESSION_TTL_SECS);

    // Create session record
    let new_session = Session {
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

    db.collection::<Session>(SESSIONS)
        .insert_one(&new_session)
        .await?;

    Ok(IssuedSession {
        session_token,
        session_id,
    })
}

/// Create a new session and issue JWT tokens.
pub async fn create_session_and_issue_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    user_id: &str,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<IssuedTokens> {
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id: {e}")))?;

    let session = create_session(db, user_id, ip_address, user_agent).await?;
    let now = Utc::now();

    // Resolve RBAC data and inject into the access token based on scope
    let scope = "openid profile email";
    let rbac_data =
        crate::services::rbac_helpers::build_rbac_claim_data(db, user_id, scope).await?;
    let access_token =
        jwt::generate_access_token(jwt_keys, config, &user_uuid, scope, Some(&rbac_data))?;

    // Generate refresh token
    let (refresh_token_jwt, refresh_jti) =
        jwt::generate_refresh_token(jwt_keys, config, &user_uuid)?;

    let refresh_id = Uuid::new_v4().to_string();
    let refresh_expires = now + Duration::seconds(config.jwt_refresh_ttl_secs);

    // Persist refresh token metadata
    let new_refresh = RefreshToken {
        id: refresh_id,
        jti: refresh_jti,
        client_id: Uuid::nil().to_string(), // first-party client
        user_id: user_id.to_string(),
        session_id: Some(session.session_id.clone()),
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        revoked_at: None,
        created_at: now,
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&new_refresh)
        .await?;

    Ok(IssuedTokens {
        access_token,
        refresh_token: refresh_token_jwt,
        session_id: session.session_id,
        access_expires_in: config.jwt_access_ttl_secs,
    })
}

/// Create a short-lived pending MFA session.
///
/// This binds a temporary token hash to the user_id so the MFA verification
/// step can validate that the user already passed password authentication.
/// The session expires in 5 minutes and is marked with a specific user_agent
/// to distinguish it from real sessions.
pub async fn create_mfa_pending_session(
    db: &mongodb::Database,
    user_id: &str,
    temp_token_hash: &str,
) -> AppResult<String> {
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires = now + Duration::minutes(5);

    let pending_session = Session {
        id: session_id.clone(),
        user_id: user_id.to_string(),
        token_hash: temp_token_hash.to_string(),
        ip_address: None,
        user_agent: Some("mfa_pending".to_string()),
        expires_at: expires,
        revoked: false,
        created_at: now,
        last_active_at: now,
    };

    db.collection::<Session>(SESSIONS)
        .insert_one(&pending_session)
        .await?;

    Ok(session_id)
}

/// Refresh an expired access token using a valid refresh token.
///
/// Implements refresh token rotation: the old token is revoked and
/// a new refresh token is issued alongside the new access token.
/// Does NOT generate a new session token (reuses the existing session).
///
/// When `mcp_sessions` is provided, session revocations also cascade
/// to MCP sessions for the affected user.
pub async fn refresh_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    refresh_token_str: &str,
    mcp_sessions: Option<&McpSessionStore>,
) -> AppResult<IssuedTokens> {
    // Verify the refresh JWT
    let claims = jwt::verify_token(jwt_keys, config, refresh_token_str)?;

    if claims.token_type != "refresh" {
        return Err(AppError::Unauthorized("Expected refresh token".to_string()));
    }

    // Look up the refresh token record by JTI
    let stored = db
        .collection::<RefreshToken>(REFRESH_TOKENS)
        .find_one(doc! { "jti": &claims.jti })
        .await?
        .ok_or_else(|| AppError::Unauthorized("Refresh token not found".to_string()))?;

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

    let (active_token, reuse_existing_refresh_token) = match disposition {
        RefreshTokenDisposition::Rotate(token) => (token, false),
        RefreshTokenDisposition::Reuse(token) => (token, true),
    };
    let user_id_str = active_token.user_id.clone();
    let user_id = Uuid::parse_str(&user_id_str)
        .map_err(|e| AppError::Internal(format!("Invalid user_id in refresh token: {e}")))?;
    let session_id = active_token.session_id.clone();
    let now = Utc::now();

    // Resolve RBAC data and inject into the refreshed access token
    let scope = "openid profile email";
    let rbac_data =
        crate::services::rbac_helpers::build_rbac_claim_data(db, &user_id_str, scope).await?;
    let new_access =
        jwt::generate_access_token(jwt_keys, config, &user_id, scope, Some(&rbac_data))?;

    if reuse_existing_refresh_token {
        touch_session_last_active(db, session_id.as_deref(), now).await?;
        return build_reused_refresh_response(
            jwt_keys,
            config,
            &user_id,
            &active_token,
            new_access,
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
            touch_session_last_active(db, session_id.as_deref(), now).await?;
            return build_reused_refresh_response(
                jwt_keys, config, &user_id, &recovered, new_access,
            );
        }

        return Err(AppError::Conflict(
            "Refresh token was concurrently rotated, please retry".to_string(),
        ));
    }

    // Persist new refresh token
    let new_refresh = RefreshToken {
        id: new_refresh_id,
        jti: new_jti,
        client_id: Uuid::nil().to_string(),
        user_id: user_id_str,
        session_id: session_id.clone(),
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        revoked_at: None,
        created_at: now,
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&new_refresh)
        .await?;

    touch_session_last_active(db, session_id.as_deref(), now).await?;

    // Reuse the existing session token rather than generating a new orphan token.
    // The session cookie does not need to change on token refresh.
    // Return an empty session_token since the cookie should not be updated.
    Ok(IssuedTokens {
        access_token: new_access,
        refresh_token: new_refresh_jwt,
        session_id: session_id.unwrap_or_else(|| Uuid::nil().to_string()),
        access_expires_in: config.jwt_access_ttl_secs,
    })
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
        .await?;

    // Revoke all refresh tokens for this session in a batch
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_many(
            doc! { "session_id": session_id, "revoked": false },
            doc! { "$set": { "revoked": true } },
        )
        .await?;

    // Cascade: remove MCP sessions for the affected user
    if let (Some(mcp), Some(session)) = (mcp_sessions, &session_doc) {
        mcp.remove_by_user_id(&session.user_id);
    }

    tracing::info!(session_id = %session_id, "Session revoked");

    Ok(())
}
