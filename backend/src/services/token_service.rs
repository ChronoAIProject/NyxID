use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::{self, JwtKeys};
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::refresh_token::{RefreshToken, COLLECTION_NAME as REFRESH_TOKENS};
use crate::models::session::{Session, COLLECTION_NAME as SESSIONS};

/// Tokens issued after successful authentication.
pub struct IssuedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub session_token: String,
    pub session_id: String,
    pub access_expires_in: i64,
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
    let user_uuid = Uuid::parse_str(user_id).map_err(|e| {
        AppError::Internal(format!("Invalid user_id: {e}"))
    })?;

    let session_token = generate_random_token();
    let session_token_hash = hash_token(&session_token);
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let session_expires = now + Duration::days(30);

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

    // Generate JWT access token
    let access_token =
        jwt::generate_access_token(jwt_keys, config, &user_uuid, "openid profile email")?;

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
        session_id: Some(session_id.clone()),
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        created_at: now,
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&new_refresh)
        .await?;

    Ok(IssuedTokens {
        access_token,
        refresh_token: refresh_token_jwt,
        session_token,
        session_id,
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
pub async fn refresh_tokens(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    refresh_token_str: &str,
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

    if stored.revoked {
        // Token reuse detected -- possible token theft.
        // Revoke the entire session as a security measure.
        tracing::warn!(
            user_id = %stored.user_id,
            jti = %claims.jti,
            "Refresh token reuse detected, revoking session"
        );

        if let Some(ref session_id) = stored.session_id {
            revoke_session(db, session_id).await?;
        }

        return Err(AppError::Unauthorized(
            "Refresh token has been revoked".to_string(),
        ));
    }

    let user_id_str = stored.user_id.clone();
    let user_id = Uuid::parse_str(&user_id_str).map_err(|e| {
        AppError::Internal(format!("Invalid user_id in refresh token: {e}"))
    })?;
    let session_id = stored.session_id.clone();
    let now = Utc::now();

    // Issue new access token
    let new_access = jwt::generate_access_token(jwt_keys, config, &user_id, "openid profile email")?;

    // Issue new refresh token (rotation)
    let (new_refresh_jwt, new_jti) = jwt::generate_refresh_token(jwt_keys, config, &user_id)?;
    let new_refresh_id = Uuid::new_v4().to_string();
    let refresh_expires = now + Duration::seconds(config.jwt_refresh_ttl_secs);

    // Revoke the old refresh token and set replaced_by
    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .update_one(
            doc! { "_id": &stored.id },
            doc! { "$set": {
                "revoked": true,
                "replaced_by": &new_refresh_id,
            }},
        )
        .await?;

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
        created_at: now,
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&new_refresh)
        .await?;

    // Update session last_active_at
    if let Some(ref sid) = session_id {
        db.collection::<Session>(SESSIONS)
            .update_one(
                doc! { "_id": sid },
                doc! { "$set": {
                    "last_active_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;
    }

    // Reuse the existing session token rather than generating a new orphan token.
    // The session cookie does not need to change on token refresh.
    // Return an empty session_token since the cookie should not be updated.
    Ok(IssuedTokens {
        access_token: new_access,
        refresh_token: new_refresh_jwt,
        session_token: String::new(), // Session token is not rotated on refresh
        session_id: session_id.unwrap_or_else(|| Uuid::nil().to_string()),
        access_expires_in: config.jwt_access_ttl_secs,
    })
}

/// Revoke a session and all its associated refresh tokens.
///
/// Uses batch update where possible to avoid N+1 queries.
pub async fn revoke_session(db: &mongodb::Database, session_id: &str) -> AppResult<()> {
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

    tracing::info!(session_id = %session_id, "Session revoked");

    Ok(())
}
