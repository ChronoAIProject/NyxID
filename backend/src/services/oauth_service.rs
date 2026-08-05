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
use crate::models::authorization_code::{
    AuthorizationCode, COLLECTION_NAME as AUTH_CODES, ExternalSubjectRef,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::services::oauth_resource_service;

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

    // For public clients, also accept:
    // - Loopback redirect URIs per RFC 8252 s7.3 (http://127.0.0.1:PORT/...)
    // - Private-use URI scheme redirects per RFC 8252 s7.1 (cursor://, vscode://, etc.)
    //
    // Both are safe because our authorize endpoint requires PKCE (S256), which
    // prevents authorization code interception attacks.
    if client.client_type == "public"
        && (is_loopback_redirect_uri(redirect_uri) || is_private_use_uri_scheme(redirect_uri))
    {
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

/// Resolve the effective authorize scope for an OAuth client.
///
/// If the request omits `scope`, the client's configured `allowed_scopes` are
/// used as the default so narrowed clients still work without an explicit
/// override.
pub fn resolve_authorize_scope(requested: Option<&str>, allowed: &str) -> AppResult<String> {
    let requested = requested.unwrap_or(allowed);
    validate_scopes(requested, allowed)
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
    external_subject: Option<&ExternalSubjectRef>,
    binding_grant_id: Option<&str>,
    resource_uris: &[String],
    allowed_service_ids: &[String],
    allow_all_services: bool,
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
        external_subject: external_subject.cloned(),
        binding_grant_id: binding_grant_id.map(String::from),
        resource_uris: resource_uris.to_vec(),
        allowed_service_ids: allowed_service_ids.to_vec(),
        allow_all_services,
        expires_at: now + Duration::minutes(5),
        used: false,
        created_at: now,
    };

    db.collection::<AuthorizationCode>(AUTH_CODES)
        .insert_one(&new_code)
        .await?;

    Ok(code)
}

/// Authenticate a client by client_id and client_secret.
///
/// Used by introspection (RFC 7662) and revocation (RFC 7009) endpoints.
/// Public clients (PKCE-based, no secret) only need a valid client_id.
pub async fn authenticate_client(
    db: &mongodb::Database,
    client_id: &str,
    client_secret: Option<&str>,
) -> AppResult<OauthClient> {
    let client = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id })
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid client credentials".to_string()))?;

    if !client.is_active {
        return Err(AppError::Unauthorized(
            "OAuth client is inactive".to_string(),
        ));
    }

    validate_client_secret(&client, client_secret)?;

    Ok(client)
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
fn validate_client_secret(client: &OauthClient, client_secret: Option<&str>) -> AppResult<()> {
    if client.client_type == "confidential" {
        let secret = client_secret.ok_or_else(|| {
            AppError::Unauthorized("client_secret is required for confidential clients".to_string())
        })?;

        let provided_hash = hash_token(secret);

        if !constant_time_eq(&provided_hash, &client.client_secret_hash) {
            return Err(AppError::Unauthorized("Invalid client_secret".to_string()));
        }
    }

    Ok(())
}

/// Exchange an authorization code for tokens.
///
/// Implements PKCE verification (S256 only) and client_secret validation
/// for confidential clients. Persists the refresh token to the database
/// and implements code replay detection.
///
/// Returns the minted token strings plus metadata needed by the OAuth broker path.
pub struct ExchangedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub refresh_token_jti: String,
    pub id_token: Option<String>,
    pub granted_scope: String,
    pub user_id: String,
    pub external_subject: Option<ExternalSubjectRef>,
    pub binding_grant_id: Option<String>,
    pub broker_capability_enabled: bool,
    pub resource_uris: Vec<String>,
    // Populated by the token exchange for the granted per-service access, but not
    // yet consumed on the access-token path (only the refresh_token_* siblings are
    // read today). Allowed rather than removed so the completing work (#1111) can
    // wire consumption without re-adding them.
    #[allow(dead_code)]
    pub allowed_service_ids: Vec<String>,
    #[allow(dead_code)]
    pub allow_all_services: bool,
    pub refresh_token_resource_uris: Vec<String>,
    pub refresh_token_allowed_service_ids: Vec<String>,
    pub refresh_token_allow_all_services: bool,
}

pub struct IssuedOAuthRefreshToken {
    pub refresh_token: String,
    pub refresh_token_jti: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn issue_oauth_refresh_token(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    client_id: &str,
    user_id: &str,
    scope: &str,
    resource_uris: &[String],
    allowed_service_ids: &[String],
    allow_all_services: bool,
) -> AppResult<IssuedOAuthRefreshToken> {
    let user_uuid = Uuid::parse_str(user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id for refresh token: {e}")))?;
    let (refresh_token_jwt, refresh_token_jti) =
        crate::crypto::jwt::generate_refresh_token(jwt_keys, config, &user_uuid)?;

    let now = Utc::now();
    let refresh_expires = now + Duration::seconds(config.jwt_refresh_ttl_secs);
    let new_refresh = RefreshToken {
        id: Uuid::new_v4().to_string(),
        jti: refresh_token_jti.clone(),
        client_id: client_id.to_string(),
        user_id: user_id.to_string(),
        session_id: None,
        scope: Some(scope.to_string()),
        expires_at: refresh_expires,
        revoked: false,
        replaced_by: None,
        revoked_at: None,
        resource_uris: resource_uris.to_vec(),
        allowed_service_ids: allowed_service_ids.to_vec(),
        allow_all_services,
        created_at: now,
    };

    db.collection::<RefreshToken>(REFRESH_TOKENS)
        .insert_one(&new_refresh)
        .await?;

    Ok(IssuedOAuthRefreshToken {
        refresh_token: refresh_token_jwt,
        refresh_token_jti,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn exchange_authorization_code(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    broker_require_admin_capability: bool,
    code: &str,
    client_id: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    client_secret: Option<&str>,
    access_token_ttl_override_secs: Option<i64>,
    requested_resources: Option<&[String]>,
    dpop_jkt: Option<&str>,
    mtls_x5t_s256: Option<&str>,
) -> AppResult<ExchangedTokens> {
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
                && used_code.used
            {
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

    // Validate client_secret for confidential clients. Reject auth
    // codes against soft-deleted clients (`is_active = false`) so an
    // already-issued code cannot be exchanged after the developer
    // app -- or its owning org -- has been deleted. The legacy
    // delete path on `oauth_clients` is a soft-delete, so the row
    // can linger with `is_active = false` until the org cascade
    // sweeps it; without this filter the code window is up to the
    // auth-code TTL after the delete.
    let client = db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .find_one(doc! { "_id": client_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::BadRequest("OAuth client not found".to_string()))?;

    validate_client_secret(&client, client_secret)?;
    let broker_capability_enabled =
        crate::services::oauth_broker_service::is_broker_client_with_policy(
            &client,
            broker_require_admin_capability,
        );

    // PKCE verification (S256 only)
    if let Some(challenge) = &stored.code_challenge {
        let verifier = code_verifier.ok_or(AppError::PkceVerificationFailed)?;

        let method = stored.code_challenge_method.as_deref().unwrap_or("S256");

        // Only S256 is supported -- reject any other method
        if method != "S256" {
            return Err(AppError::BadRequest(
                "Only S256 code_challenge_method is supported".to_string(),
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let computed_challenge =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

        // Use constant-time comparison to prevent timing attacks
        if !constant_time_eq(&computed_challenge, challenge) {
            return Err(AppError::PkceVerificationFailed);
        }
    }

    // Parse user_id back to Uuid for JWT generation
    let user_uuid = Uuid::parse_str(&stored.user_id)
        .map_err(|e| AppError::Internal(format!("Invalid user_id in authorization code: {e}")))?;
    let token_resource_scope = oauth_resource_service::resolve_token_resource_scope(
        db,
        config,
        &stored.user_id,
        requested_resources,
        &stored.resource_uris,
        &stored.allowed_service_ids,
        stored.allow_all_services,
    )
    .await?;

    // Resolve RBAC data filtered by the granted scope
    let rbac_data =
        crate::services::rbac_helpers::build_rbac_claim_data(db, &stored.user_id, &stored.scope)
            .await?;

    // Generate tokens with RBAC claims
    let access_token = crate::crypto::jwt::generate_oauth_access_token(
        jwt_keys,
        config,
        &user_uuid,
        &stored.scope,
        Some(&rbac_data),
        broker_capability_enabled
            .then_some(access_token_ttl_override_secs)
            .flatten(),
        dpop_jkt,
        mtls_x5t_s256,
        (!token_resource_scope.allow_all_services).then_some(
            crate::crypto::jwt::AccessTokenRestrictions {
                resources: &token_resource_scope.resource_uris,
                allowed_service_ids: &token_resource_scope.allowed_service_ids,
            },
        ),
        &stored.client_id,
    )?;

    let issued_refresh = issue_oauth_refresh_token(
        db,
        config,
        jwt_keys,
        client_id,
        &stored.user_id,
        &stored.scope,
        &stored.resource_uris,
        &stored.allowed_service_ids,
        stored.allow_all_services,
    )
    .await?;

    // Generate ID token if openid scope was requested
    let id_token = if stored.scope.split_whitespace().any(|s| s == "openid") {
        // Fetch user to populate claims
        let user = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &stored.user_id })
            .await?
            .ok_or_else(|| AppError::Internal("User not found for ID token".to_string()))?;

        // Build RBAC auth context for the ID token
        let auth_ctx = crate::services::rbac_helpers::build_id_token_auth_context(
            db,
            &stored.user_id,
            &stored.scope,
        )
        .await?;

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
            Some(&auth_ctx),
        )?)
    } else {
        None
    };

    let granted_scope = stored.scope.clone();
    Ok(ExchangedTokens {
        access_token,
        refresh_token: issued_refresh.refresh_token,
        refresh_token_jti: issued_refresh.refresh_token_jti,
        id_token,
        granted_scope,
        user_id: stored.user_id,
        external_subject: stored.external_subject,
        binding_grant_id: stored.binding_grant_id,
        broker_capability_enabled,
        resource_uris: token_resource_scope.resource_uris,
        allowed_service_ids: token_resource_scope.allowed_service_ids,
        allow_all_services: token_resource_scope.allow_all_services,
        refresh_token_resource_uris: stored.resource_uris,
        refresh_token_allowed_service_ids: stored.allowed_service_ids,
        refresh_token_allow_all_services: stored.allow_all_services,
    })
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
        && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        && parsed.port().is_some()
}

/// Check whether a redirect URI uses a private-use URI scheme per RFC 8252
/// section 7.1.
///
/// Native/desktop OAuth clients (Cursor, Claude Code, VS Code, etc.) register
/// custom URI schemes like `cursor://...` or `vscode://...` with the OS to
/// receive authorization callbacks. These are safe for public clients because
/// PKCE prevents authorization code interception attacks.
///
/// Only non-standard schemes are accepted (`http` and `https` require explicit
/// registration to prevent open-redirector vulnerabilities).
fn is_private_use_uri_scheme(uri: &str) -> bool {
    let Ok(parsed) = url::Url::parse(uri) else {
        return false;
    };
    !matches!(parsed.scheme(), "http" | "https")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::token::hash_token;
    use crate::models::oauth_client::OauthClient;
    use crate::models::refresh_token::RefreshToken;
    use crate::test_utils::{connect_test_database, test_user_service};
    use mongodb::bson::doc;

    fn test_client(client_id: &str, client_type: &str, secret: &str) -> OauthClient {
        let now = Utc::now();
        OauthClient {
            id: client_id.to_string(),
            client_name: format!("client {client_id}"),
            client_secret_hash: hash_token(secret),
            redirect_uris: vec!["https://app.example/callback".to_string()],
            allowed_scopes: "openid profile email".to_string(),
            scope_provenance: Default::default(),
            grant_types: "authorization_code".to_string(),
            client_type: client_type.to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            created_by: Some("test".to_string()),
            created_at: now,
            updated_at: now,
        }
    }

    async fn insert_client(db: &mongodb::Database, client: &OauthClient) {
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(client)
            .await
            .expect("insert oauth client");
    }

    #[test]
    fn authorize_scope_defaults_to_client_allowed_scopes() {
        let result = resolve_authorize_scope(None, "openid roles").unwrap();
        assert_eq!(result, "openid roles");
    }

    #[test]
    fn authorize_scope_keeps_valid_requested_subset() {
        let result = resolve_authorize_scope(Some("openid"), "openid profile email").unwrap();
        assert_eq!(result, "openid");
    }

    #[test]
    fn authorize_scope_rejects_scope_outside_allowed_set() {
        let result = resolve_authorize_scope(None, "openid");
        assert_eq!(result.unwrap(), "openid");

        let invalid = resolve_authorize_scope(Some("openid email"), "openid");
        assert!(invalid.is_err());
    }

    #[test]
    fn validate_scopes_preserves_requested_order_and_duplicates() {
        let result = validate_scopes("email openid email", "openid profile email").unwrap();
        assert_eq!(result, "email openid email");
    }

    #[test]
    fn loopback_redirect_uri_requires_http_loopback_host_and_port() {
        assert!(is_loopback_redirect_uri("http://127.0.0.1:49152/callback"));
        assert!(is_loopback_redirect_uri("http://localhost:3000/callback"));
        assert!(is_loopback_redirect_uri("http://[::1]:31337/callback"));

        assert!(!is_loopback_redirect_uri(
            "https://127.0.0.1:49152/callback"
        ));
        assert!(!is_loopback_redirect_uri("http://127.0.0.1/callback"));
        assert!(!is_loopback_redirect_uri(
            "http://192.168.1.10:3000/callback"
        ));
        assert!(!is_loopback_redirect_uri("not a uri"));
    }

    #[test]
    fn private_use_redirect_uri_rejects_standard_web_schemes() {
        assert!(is_private_use_uri_scheme("cursor://oauth/callback"));
        assert!(is_private_use_uri_scheme("vscode://nyxid/auth"));

        assert!(!is_private_use_uri_scheme("http://localhost:3000/callback"));
        assert!(!is_private_use_uri_scheme("https://app.example/callback"));
        assert!(!is_private_use_uri_scheme("not a uri"));
    }

    #[tokio::test]
    async fn validate_client_accepts_exact_and_public_native_redirects() {
        let db = connect_test_database("oauth_val_ok")
            .await
            .expect("local MongoDB required for oauth_service tests");
        let public = test_client("public-client", "public", "");
        insert_client(&db, &public).await;

        let exact = validate_client(&db, &public.id, "https://app.example/callback")
            .await
            .expect("registered redirect accepted");
        assert_eq!(exact.id, public.id);

        let loopback = validate_client(&db, &public.id, "http://127.0.0.1:54321/callback")
            .await
            .expect("public loopback redirect accepted");
        assert_eq!(loopback.id, public.id);

        let private_scheme = validate_client(&db, &public.id, "cursor://nyxid/callback")
            .await
            .expect("public private-use redirect accepted");
        assert_eq!(private_scheme.id, public.id);
    }

    #[tokio::test]
    async fn validate_client_rejects_inactive_confidential_and_unknown_redirects() {
        let db = connect_test_database("oauth_val_bad")
            .await
            .expect("local MongoDB required for oauth_service tests");

        let confidential = test_client("confidential-client", "confidential", "secret");
        insert_client(&db, &confidential).await;
        let err = validate_client(&db, &confidential.id, "http://127.0.0.1:54321/callback")
            .await
            .expect_err("confidential clients must not get dynamic native redirects");
        assert!(matches!(err, AppError::InvalidRedirectUri));

        let mut inactive = test_client("inactive-client", "public", "");
        inactive.is_active = false;
        insert_client(&db, &inactive).await;
        let err = validate_client(&db, &inactive.id, "https://app.example/callback")
            .await
            .expect_err("inactive client rejected");
        assert!(
            matches!(err, AppError::BadRequest(message) if message == "OAuth client is inactive")
        );

        let err = validate_client(&db, "missing-client", "https://app.example/callback")
            .await
            .expect_err("missing client rejected");
        assert!(matches!(err, AppError::NotFound(message) if message == "OAuth client not found"));
    }

    #[tokio::test]
    async fn authenticate_client_handles_public_and_confidential_secret_paths() {
        let db = connect_test_database("oauth_auth")
            .await
            .expect("local MongoDB required for oauth_service tests");

        let public = test_client("public-auth", "public", "");
        let confidential = test_client("conf-auth", "confidential", "correct-secret");
        insert_client(&db, &public).await;
        insert_client(&db, &confidential).await;

        let authenticated_public = authenticate_client(&db, &public.id, None)
            .await
            .expect("public client does not require secret");
        assert_eq!(authenticated_public.id, public.id);

        let authenticated_confidential =
            authenticate_client(&db, &confidential.id, Some("correct-secret"))
                .await
                .expect("confidential client accepts matching secret");
        assert_eq!(authenticated_confidential.id, confidential.id);

        let missing = authenticate_client(&db, &confidential.id, None)
            .await
            .expect_err("missing secret rejected");
        assert!(matches!(
            missing,
            AppError::Unauthorized(message)
                if message == "client_secret is required for confidential clients"
        ));

        let wrong = authenticate_client(&db, &confidential.id, Some("wrong-secret"))
            .await
            .expect_err("wrong secret rejected");
        assert!(
            matches!(wrong, AppError::Unauthorized(message) if message == "Invalid client_secret")
        );
    }

    #[tokio::test]
    async fn create_authorization_code_persists_hashed_code_and_metadata() {
        let db = connect_test_database("oauth_code")
            .await
            .expect("local MongoDB required for oauth_service tests");
        let external = ExternalSubjectRef {
            platform: "lark".to_string(),
            tenant: Some("tenant-1".to_string()),
            external_user_id: "ou_123".to_string(),
        };

        let code = create_authorization_code(
            &db,
            "client-1",
            "user-1",
            "https://app.example/callback",
            "openid profile",
            Some("challenge"),
            Some("S256"),
            Some("nonce-1"),
            Some(&external),
            Some(&"a".repeat(64)),
            &["https://nyx.example/api/v1/proxy/s/openai".to_string()],
            &["svc-1".to_string()],
            false,
        )
        .await
        .expect("create authorization code");

        assert!(!code.is_empty());
        let stored = db
            .collection::<AuthorizationCode>(AUTH_CODES)
            .find_one(doc! { "code_hash": hash_token(&code) })
            .await
            .expect("query auth code")
            .expect("stored auth code");
        assert_ne!(stored.code_hash, code);
        assert_eq!(stored.client_id, "client-1");
        assert_eq!(stored.user_id, "user-1");
        assert_eq!(stored.redirect_uri, "https://app.example/callback");
        assert_eq!(stored.scope, "openid profile");
        assert_eq!(stored.code_challenge.as_deref(), Some("challenge"));
        assert_eq!(stored.code_challenge_method.as_deref(), Some("S256"));
        assert_eq!(stored.nonce.as_deref(), Some("nonce-1"));
        assert_eq!(stored.external_subject, Some(external));
        assert_eq!(stored.binding_grant_id, Some("a".repeat(64)));
        assert_eq!(
            stored.resource_uris,
            vec!["https://nyx.example/api/v1/proxy/s/openai".to_string()]
        );
        assert_eq!(stored.allowed_service_ids, vec!["svc-1".to_string()]);
        assert!(!stored.allow_all_services);
        assert!(!stored.used);
        assert!(stored.expires_at > Utc::now());
    }

    #[tokio::test]
    async fn exchange_authorization_code_preserves_empty_service_grant() {
        let Some(db) = connect_test_database("oauth_empty_service_grant").await else {
            return;
        };
        let client = test_client("client-empty-service-grant", "public", "");
        insert_client(&db, &client).await;
        let user_id = Uuid::new_v4().to_string();
        let code = "empty-service-grant-code";
        let now = Utc::now();

        db.collection::<AuthorizationCode>(AUTH_CODES)
            .insert_one(AuthorizationCode {
                id: Uuid::new_v4().to_string(),
                code_hash: hash_token(code),
                client_id: client.id.clone(),
                user_id: user_id.clone(),
                redirect_uri: "https://app.example/callback".to_string(),
                scope: "openid".to_string(),
                code_challenge: None,
                code_challenge_method: None,
                nonce: None,
                external_subject: None,
                binding_grant_id: None,
                resource_uris: Vec::new(),
                allowed_service_ids: Vec::new(),
                allow_all_services: false,
                expires_at: now + Duration::minutes(5),
                used: false,
                created_at: now,
            })
            .await
            .expect("insert authorization code");

        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &user_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert user");

        let config = crate::test_utils::test_app_config();
        let jwt_keys = crate::test_utils::cached_test_jwt_keys();
        let exchanged = exchange_authorization_code(
            &db,
            &config,
            &jwt_keys,
            config.broker_require_admin_capability(),
            code,
            &client.id,
            "https://app.example/callback",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("exchange authorization code");

        assert!(!exchanged.allow_all_services);
        assert!(exchanged.allowed_service_ids.is_empty());
        let claims = crate::crypto::jwt::verify_token(&jwt_keys, &config, &exchanged.access_token)
            .expect("verify access token");
        assert_eq!(claims.allow_all_services, Some(false));
        assert_eq!(claims.allowed_service_ids, Some(Vec::new()));
        assert_eq!(claims.resources, Some(Vec::new()));
        assert_eq!(claims.client_id.as_deref(), Some(client.id.as_str()));

        let stored_refresh = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &exchanged.refresh_token_jti })
            .await
            .expect("query refresh")
            .expect("refresh stored");
        assert!(!stored_refresh.allow_all_services);
        assert!(stored_refresh.allowed_service_ids.is_empty());
    }

    #[tokio::test]
    async fn exchange_authorization_code_accepts_resource_within_consent_only_service_grant() {
        let Some(db) = connect_test_database("oauth_consent_only_resource").await else {
            return;
        };
        let client = test_client("client-consent-only-resource", "public", "");
        insert_client(&db, &client).await;
        let user_id = Uuid::new_v4().to_string();
        let service_a_id = Uuid::new_v4().to_string();
        let service_b_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &user_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert user");
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_many([
            test_user_service(&service_a_id, &user_id, "svc-a", "endpoint-a", None, None),
            test_user_service(&service_b_id, &user_id, "svc-b", "endpoint-b", None, None),
        ])
        .await
        .expect("insert user services");

        let code = "consent-only-resource-code";
        db.collection::<AuthorizationCode>(AUTH_CODES)
            .insert_one(AuthorizationCode {
                id: Uuid::new_v4().to_string(),
                code_hash: hash_token(code),
                client_id: client.id.clone(),
                user_id: user_id.clone(),
                redirect_uri: "https://app.example/callback".to_string(),
                scope: "openid".to_string(),
                code_challenge: None,
                code_challenge_method: None,
                nonce: None,
                external_subject: None,
                binding_grant_id: None,
                resource_uris: Vec::new(),
                allowed_service_ids: vec![service_a_id.clone()],
                allow_all_services: false,
                expires_at: now + Duration::minutes(5),
                used: false,
                created_at: now,
            })
            .await
            .expect("insert authorization code");

        let config = crate::test_utils::test_app_config();
        let jwt_keys = crate::test_utils::cached_test_jwt_keys();
        let resource_a = oauth_resource_service::user_service_resource_uri(&config, "svc-a");
        let requested = vec![resource_a.clone()];
        let exchanged = exchange_authorization_code(
            &db,
            &config,
            &jwt_keys,
            config.broker_require_admin_capability(),
            code,
            &client.id,
            "https://app.example/callback",
            None,
            None,
            None,
            Some(&requested),
            None,
            None,
        )
        .await
        .expect("exchange authorization code");

        assert!(!exchanged.allow_all_services);
        assert_eq!(exchanged.resource_uris, vec![resource_a.clone()]);
        assert_eq!(exchanged.allowed_service_ids, vec![service_a_id.clone()]);
        assert_eq!(exchanged.refresh_token_resource_uris, Vec::<String>::new());
        assert_eq!(
            exchanged.refresh_token_allowed_service_ids,
            vec![service_a_id.clone()]
        );
        assert!(!exchanged.refresh_token_allow_all_services);

        let claims = crate::crypto::jwt::verify_token(&jwt_keys, &config, &exchanged.access_token)
            .expect("verify access token");
        assert_eq!(claims.resources, Some(vec![resource_a]));
        assert_eq!(claims.allowed_service_ids, Some(vec![service_a_id.clone()]));
        assert_eq!(claims.allow_all_services, Some(false));

        let stored_refresh = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &exchanged.refresh_token_jti })
            .await
            .expect("query refresh")
            .expect("refresh stored");
        assert!(!stored_refresh.allow_all_services);
        assert!(stored_refresh.resource_uris.is_empty());
        assert_eq!(stored_refresh.allowed_service_ids, vec![service_a_id]);
    }

    #[tokio::test]
    async fn exchange_authorization_code_rejects_resource_outside_consent_only_service_grant() {
        let Some(db) = connect_test_database("oauth_consent_only_resource_reject").await else {
            return;
        };
        let client = test_client("client-consent-only-resource-reject", "public", "");
        insert_client(&db, &client).await;
        let user_id = Uuid::new_v4().to_string();
        let service_a_id = Uuid::new_v4().to_string();
        let service_b_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &user_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert user");
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_many([
            test_user_service(&service_a_id, &user_id, "svc-a", "endpoint-a", None, None),
            test_user_service(&service_b_id, &user_id, "svc-b", "endpoint-b", None, None),
        ])
        .await
        .expect("insert user services");

        let code = "consent-only-resource-reject-code";
        db.collection::<AuthorizationCode>(AUTH_CODES)
            .insert_one(AuthorizationCode {
                id: Uuid::new_v4().to_string(),
                code_hash: hash_token(code),
                client_id: client.id.clone(),
                user_id: user_id.clone(),
                redirect_uri: "https://app.example/callback".to_string(),
                scope: "openid".to_string(),
                code_challenge: None,
                code_challenge_method: None,
                nonce: None,
                external_subject: None,
                binding_grant_id: None,
                resource_uris: Vec::new(),
                allowed_service_ids: vec![service_a_id],
                allow_all_services: false,
                expires_at: now + Duration::minutes(5),
                used: false,
                created_at: now,
            })
            .await
            .expect("insert authorization code");

        let config = crate::test_utils::test_app_config();
        let jwt_keys = crate::test_utils::cached_test_jwt_keys();
        let resource_b = oauth_resource_service::user_service_resource_uri(&config, "svc-b");
        let requested = vec![resource_b];
        let err = match exchange_authorization_code(
            &db,
            &config,
            &jwt_keys,
            config.broker_require_admin_capability(),
            code,
            &client.id,
            "https://app.example/callback",
            None,
            None,
            None,
            Some(&requested),
            None,
            None,
        )
        .await
        {
            Ok(_) => panic!("resource outside consented services is rejected"),
            Err(err) => err,
        };
        assert!(matches!(err, AppError::InvalidTarget(_)));

        let refresh_count = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .count_documents(doc! { "client_id": &client.id, "user_id": &user_id })
            .await
            .expect("count refresh tokens");
        assert_eq!(refresh_count, 0);
    }

    #[tokio::test]
    async fn exchange_authorization_code_replay_revokes_existing_refresh_tokens() {
        let db = connect_test_database("oauth_replay")
            .await
            .expect("local MongoDB required for oauth_service tests");
        let code = "used-code";
        let user_id = Uuid::new_v4().to_string();
        let client_id = "client-replay";
        let now = Utc::now();

        db.collection::<AuthorizationCode>(AUTH_CODES)
            .insert_one(AuthorizationCode {
                id: Uuid::new_v4().to_string(),
                code_hash: hash_token(code),
                client_id: client_id.to_string(),
                user_id: user_id.clone(),
                redirect_uri: "https://app.example/callback".to_string(),
                scope: "openid".to_string(),
                code_challenge: None,
                code_challenge_method: None,
                nonce: None,
                external_subject: None,
                binding_grant_id: None,
                resource_uris: Vec::new(),
                allowed_service_ids: Vec::new(),
                allow_all_services: true,
                expires_at: now + Duration::minutes(5),
                used: true,
                created_at: now,
            })
            .await
            .expect("insert used auth code");

        let refresh = RefreshToken {
            id: Uuid::new_v4().to_string(),
            jti: "jti-replay".to_string(),
            client_id: client_id.to_string(),
            user_id: user_id.clone(),
            session_id: None,
            scope: Some("openid".to_string()),
            expires_at: now + Duration::days(1),
            revoked: false,
            replaced_by: None,
            revoked_at: None,
            resource_uris: Vec::new(),
            allowed_service_ids: Vec::new(),
            allow_all_services: true,
            created_at: now,
        };
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_one(&refresh)
            .await
            .expect("insert refresh token");

        let config = crate::test_utils::test_app_config();
        let jwt_keys = crate::test_utils::cached_test_jwt_keys();
        let err = match exchange_authorization_code(
            &db,
            &config,
            &jwt_keys,
            config.broker_require_admin_capability(),
            code,
            client_id,
            "https://app.example/callback",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        {
            Ok(_) => panic!("used code rejected as replay"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            AppError::BadRequest(message) if message == "Authorization code has already been used"
        ));

        let revoked = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "_id": &refresh.id })
            .await
            .expect("query refresh")
            .expect("refresh token remains");
        assert!(revoked.revoked);
    }
}
