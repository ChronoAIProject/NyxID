use base64::Engine as _;
use chrono::{Duration, Utc};
use mongodb::bson::doc;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::JwtKeys;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::authorization_code::{AuthorizationCode, COLLECTION_NAME as AUTH_CODES};
use crate::models::oauth_client::{OauthClient, COLLECTION_NAME as OAUTH_CLIENTS};
use crate::models::refresh_token::{RefreshToken, COLLECTION_NAME as REFRESH_TOKENS};
use crate::models::user::{User, COLLECTION_NAME as USERS};

/// Validate an OAuth client and its redirect URI.
pub async fn validate_client(
    db: &mongodb::Database,
    client_id: &str,
    redirect_uri: &str,
) -> AppResult<OauthClient> {
    let client = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id })
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))?;

    if !client.is_active {
        return Err(AppError::BadRequest("OAuth client is inactive".to_string()));
    }

    // Check exact match first (works for all client types)
    if client.redirect_uris.iter().any(|uri| uri == redirect_uri) {
        return Ok(client);
    }

    // For public clients, also accept loopback redirect URIs (RFC 8252 s7.3)
    if client.client_type == "public" && is_loopback_redirect_uri(redirect_uri) {
        return Ok(client);
    }

    Err(AppError::InvalidRedirectUri)
}

/// Validate that the requested scopes are a subset of the client's allowed scopes.
pub fn validate_scopes(requested: &str, allowed: &str) -> AppResult<String> {
    let allowed_set: std::collections::HashSet<&str> = allowed.split_whitespace().collect();
    let requested_set: Vec<&str> = requested.split_whitespace().collect();

    for scope in &requested_set {
        if !allowed_set.contains(scope) {
            return Err(AppError::InvalidScope(format!(
                "Scope '{}' is not allowed for this client",
                scope
            )));
        }
    }

    Ok(requested_set.join(" "))
}

/// Create an authorization code for the OAuth authorization code flow.
#[allow(clippy::too_many_arguments)]
pub async fn create_authorization_code(
    db: &mongodb::Database,
    client_id: &str,
    user_id: &str,
    redirect_uri: &str,
    scope: &str,
    code_challenge: Option<&str>,
    code_challenge_method: Option<&str>,
    nonce: Option<&str>,
) -> AppResult<String> {
    let code = generate_random_token();
    let code_hash = hash_token(&code);
    let now = Utc::now();

    let new_code = AuthorizationCode {
        id: Uuid::new_v4().to_string(),
        code_hash,
        client_id: client_id.to_string(),
        user_id: user_id.to_string(),
        redirect_uri: redirect_uri.to_string(),
        scope: scope.to_string(),
        code_challenge: code_challenge.map(String::from),
        code_challenge_method: code_challenge_method.map(String::from),
        nonce: nonce.map(String::from),
        expires_at: now + Duration::minutes(5),
        used: false,
        created_at: now,
    };

    db.collection::<AuthorizationCode>(AUTH_CODES)
        .insert_one(&new_code)
        .await?;

    Ok(code)
}

/// Constant-time comparison using the `subtle` crate.
///
/// Prevents timing attacks by ensuring the comparison time does not
/// depend on the position of the first differing byte or input length.
fn constant_time_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// Validate client_secret for confidential clients.
///
/// Compares the SHA-256 hash of the provided secret against the stored hash.
/// Public clients (client_type == "public") do not require a secret.
fn validate_client_secret(
    client: &OauthClient,
    client_secret: Option<&str>,
) -> AppResult<()> {
    if client.client_type == "confidential" {
        let secret = client_secret.ok_or_else(|| {
            AppError::Unauthorized(
                "client_secret is required for confidential clients".to_string(),
            )
        })?;

        let provided_hash = hash_token(secret);

        if !constant_time_eq(&provided_hash, &client.client_secret_hash) {
            return Err(AppError::Unauthorized(
                "Invalid client_secret".to_string(),
            ));
        }
    }

    Ok(())
}

/// Exchange an authorization code for tokens.
///
/// Implements PKCE verification (S256 only) and client_secret validation
/// for confidential clients. Persists the refresh token to the database
/// and implements code replay detection.
#[allow(clippy::too_many_arguments)]
pub async fn exchange_authorization_code(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    code: &str,
    client_id: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    client_secret: Option<&str>,
) -> AppResult<(String, String, Option<String>)> {
    let code_hash = hash_token(code);

    // Atomically claim the authorization code (prevents TOCTOU race condition).
    // find_one_and_update returns the document BEFORE the update, and only
    // matches if used == false, ensuring exactly one request can claim a code.
    let stored = db
        .collection::<AuthorizationCode>(AUTH_CODES)
        .find_one_and_update(
            doc! { "code_hash": &code_hash, "client_id": client_id, "used": false },
            doc! { "$set": { "used": true } },
        )
        .await?;

    let stored = match stored {
        Some(code) => code,
        None => {
            // Either code doesn't exist OR it was already used.
            // Check if it exists-but-used to trigger replay detection.
            let maybe_used = db
                .collection::<AuthorizationCode>(AUTH_CODES)
                .find_one(doc! { "code_hash": &code_hash, "client_id": client_id })
                .await?;

            if let Some(ref used_code) = maybe_used
                && used_code.used {
                    tracing::warn!(
                        client_id = %client_id,
                        user_id = %used_code.user_id,
                        "Authorization code replay detected, revoking associated refresh tokens"
                    );

                    // Revoke all refresh tokens for this client + user combination
                    db.collection::<RefreshToken>(REFRESH_TOKENS)
                        .update_many(
                            doc! {
                                "client_id": client_id,
                                "user_id": &used_code.user_id,
                                "revoked": false,
                            },
                            doc! { "$set": { "revoked": true } },
                        )
                        .await?;

                    return Err(AppError::BadRequest(
                        "Authorization code has already been used".to_string(),
                    ));
                }

            return Err(AppError::BadRequest(
                "Invalid authorization code".to_string(),
            ));
        }
    };

    if stored.expires_at < Utc::now() {
        return Err(AppError::BadRequest(
            "Authorization code has expired".to_string(),
        ));
    }

    if stored.redirect_uri != redirect_uri {
        return Err(AppError::InvalidRedirectUri);
    }

    // Validate client_secret for confidential clients
    let client = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id })
        .await?
        .ok_or_else(|| AppError::BadRequest("OAuth client not found".to_string()))?;

    validate_client_secret(&client, client_secret)?;

    // PKCE verification (S256 only)
    if let Some(challenge) = &stored.code_challenge {
        let verifier = code_verifier
            .ok_or(AppError::PkceVerificationFailed)?;

        let method = stored
            .code_challenge_method
            .as_deref()
            .unwrap_or("S256");

        // Only S256 is supported -- reject any other method
        if method != "S256" {
            return Err(AppError::BadRequest(
                "Only S256 code_challenge_method is supported".to_string(),
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let computed_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(hasher.finalize());

        // Use constant-time comparison to prevent timing attacks
        if !constant_time_eq(&computed_challenge, challenge) {
            return Err(AppError::PkceVerificationFailed);
        }
    }

    // Parse user_id back to Uuid for JWT generation
    let user_uuid = Uuid::parse_str(&stored.user_id).map_err(|e| {
        AppError::Internal(format!("Invalid user_id in authorization code: {e}"))
    })?;

    // Generate tokens
    let access_token = crate::crypto::jwt::generate_access_token(
        jwt_keys, config, &user_uuid, &stored.scope,
    )?;

    let (refresh_token_jwt, refresh_jti) = crate::crypto::jwt::generate_refresh_token(
        jwt_keys, config, &user_uuid,
    )?;

    // Persist OAuth refresh token to database for revocation support
    let refresh_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let refresh_expires = now + Duration::seconds(config.jwt_refresh_ttl_secs);

    let new_refresh = RefreshToken {
        id: refresh_id,
        jti: refresh_jti,
        client_id: client_id.to_string(),
        user_id: stored.user_id.clone(),
        session_id: None, // OAuth flow has no session
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        created_at: now,
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&new_refresh)
        .await?;

    // Generate ID token if openid scope was requested
    let id_token = if stored.scope.split_whitespace().any(|s| s == "openid") {
        // Fetch user to populate claims
        let user = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &stored.user_id })
            .await?
            .ok_or_else(|| AppError::Internal("User not found for ID token".to_string()))?;

        Some(crate::crypto::jwt::generate_id_token(
            jwt_keys,
            config,
            &user_uuid,
            Some(&user.email),
            Some(user.email_verified),
            user.display_name.as_deref(),
            user.avatar_url.as_deref(),
            client_id,
            stored.nonce.as_deref(),
            Some(&access_token),
        )?)
    } else {
        None
    };

    Ok((access_token, refresh_token_jwt, id_token))
}

/// Check whether a redirect URI is a loopback address per RFC 8252 section 7.3.
///
/// For native/desktop OAuth clients (like MCP clients), the redirect URI is
/// `http://localhost:{random_port}/callback`. The port varies per session because
/// the client binds an ephemeral port. Only `http` scheme is accepted (loopback
/// connections never leave the machine, so TLS is unnecessary).
fn is_loopback_redirect_uri(uri: &str) -> bool {
    let Ok(parsed) = url::Url::parse(uri) else {
        return false;
    };
    parsed.scheme() == "http"
        && matches!(parsed.host_str(), Some("127.0.0.1" | "[::1]"))
        && parsed.port().is_some()
}
