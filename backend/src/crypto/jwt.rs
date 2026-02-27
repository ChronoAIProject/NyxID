use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::pkcs1::{DecodeRsaPublicKey, EncodeRsaPrivateKey, EncodeRsaPublicKey};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::errors::AppError;

/// Holds the RSA key pair used for JWT signing and verification.
#[derive(Clone)]
pub struct JwtKeys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
    /// Key ID included in JWT headers for key rotation support
    pub kid: String,
}

/// Standard JWT claims for NyxID tokens.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// Subject (user ID)
    pub sub: String,
    /// Issuer
    pub iss: String,
    /// Audience
    pub aud: String,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// JWT ID (unique per token)
    pub jti: String,
    /// Space-separated scopes
    pub scope: String,
    /// Token type: "access", "refresh", or "id"
    pub token_type: String,
    /// User's roles (present when "roles" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// User's groups (present when "groups" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    /// Flattened permissions from all roles (present when "roles" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
    /// Session ID (stable across token refreshes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    /// RFC 8693 actor claim -- identifies the service acting on behalf of the user.
    /// Present only in delegated tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act: Option<ActorClaim>,
    /// Flag indicating this is a delegated access token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated: Option<bool>,
    /// True if this token was issued to a service account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sa: Option<bool>,
}

/// Actor claim per RFC 8693 Section 4.1.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActorClaim {
    pub sub: String,
}

/// ID token claims following OpenID Connect Core.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IdTokenClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_hash: Option<String>,
    /// User's roles (present when "roles" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// User's groups (present when "groups" scope is requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    /// Authentication Context Class Reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acr: Option<String>,
    /// Authentication Methods References
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amr: Option<Vec<String>>,
    /// Time of authentication (Unix timestamp)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_time: Option<i64>,
    /// Session ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
}

impl JwtKeys {
    /// Load RSA keys from PEM files specified in the config.
    /// In development mode, auto-generates keys if they do not exist.
    /// In production, fails with a clear error when keys are missing.
    pub fn from_config(config: &AppConfig) -> Result<Self, AppError> {
        let private_path = Path::new(&config.jwt_private_key_path);
        let public_path = Path::new(&config.jwt_public_key_path);

        if !private_path.exists() || !public_path.exists() {
            if config.is_production() {
                return Err(AppError::Internal(format!(
                    "RSA key files not found at '{}' and '{}'. \
                     In production, keys must be pre-generated and mounted. \
                     Generate keys with: openssl genrsa -out private.pem 4096 && \
                     openssl rsa -in private.pem -pubout -out public.pem",
                    config.jwt_private_key_path, config.jwt_public_key_path
                )));
            }

            tracing::warn!(
                "RSA key files not found. Generating development key pair. \
                 This is NOT suitable for production use."
            );
            generate_rsa_keypair(&config.jwt_private_key_path, &config.jwt_public_key_path)?;
        }

        let private_pem = fs::read_to_string(private_path)
            .map_err(|e| AppError::Internal(format!("Failed to read private key: {e}")))?;
        let public_pem = fs::read_to_string(public_path)
            .map_err(|e| AppError::Internal(format!("Failed to read public key: {e}")))?;

        let encoding = EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .map_err(|e| AppError::Internal(format!("Invalid private key PEM: {e}")))?;
        let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .map_err(|e| AppError::Internal(format!("Invalid public key PEM: {e}")))?;

        // Compute a stable kid from the public key modulus
        let pub_key = RsaPublicKey::from_pkcs1_pem(&public_pem)
            .map_err(|e| AppError::Internal(format!("Failed to parse public key for kid: {e}")))?;
        let n_bytes = pub_key.n().to_bytes_be();
        let mut hasher = Sha256::new();
        hasher.update(&n_bytes);
        let kid = hex::encode(&hasher.finalize()[..8]);

        Ok(Self { encoding, decoding, kid })
    }
}

/// Generate a 4096-bit RSA key pair and write PEM files with restrictive permissions.
pub fn generate_rsa_keypair(private_path: &str, public_path: &str) -> Result<(), AppError> {
    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 4096)
        .map_err(|e| AppError::Internal(format!("RSA key generation failed: {e}")))?;

    let public_key = private_key.to_public_key();

    // Ensure parent directories exist
    if let Some(parent) = Path::new(private_path).parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Internal(format!("Failed to create key directory: {e}")))?;
    }

    let private_pem = private_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .map_err(|e| AppError::Internal(format!("Failed to encode private key: {e}")))?;

    let public_pem = public_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .map_err(|e| AppError::Internal(format!("Failed to encode public key: {e}")))?;

    fs::write(private_path, private_pem.as_bytes())
        .map_err(|e| AppError::Internal(format!("Failed to write private key: {e}")))?;
    fs::write(public_path, public_pem.as_bytes())
        .map_err(|e| AppError::Internal(format!("Failed to write public key: {e}")))?;

    // Set restrictive permissions on the private key (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(private_path, perms)
            .map_err(|e| AppError::Internal(format!("Failed to set key permissions: {e}")))?;
    }

    tracing::info!("Generated 4096-bit RSA key pair at {private_path} and {public_path}");

    Ok(())
}

/// Optional RBAC data to embed in JWT claims.
pub struct RbacClaimData {
    pub roles: Option<Vec<String>>,
    pub groups: Option<Vec<String>>,
    pub permissions: Option<Vec<String>>,
    pub sid: Option<String>,
}

/// Generate an access token for the given user.
pub fn generate_access_token(
    keys: &JwtKeys,
    config: &AppConfig,
    user_id: &Uuid,
    scope: &str,
    rbac: Option<&RbacClaimData>,
) -> Result<String, AppError> {
    let now = Utc::now().timestamp();

    let claims = Claims {
        sub: user_id.to_string(),
        iss: config.jwt_issuer.clone(),
        aud: config.base_url.clone(),
        exp: now + config.jwt_access_ttl_secs,
        iat: now,
        jti: Uuid::new_v4().to_string(),
        scope: scope.to_string(),
        token_type: "access".to_string(),
        roles: rbac.and_then(|r| r.roles.clone()),
        groups: rbac.and_then(|r| r.groups.clone()),
        permissions: rbac.and_then(|r| r.permissions.clone()),
        sid: rbac.and_then(|r| r.sid.clone()),
        act: None,
        delegated: None,
        sa: None,
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());

    encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode access token: {e}")))
}

/// Generate a refresh token for the given user.
pub fn generate_refresh_token(
    keys: &JwtKeys,
    config: &AppConfig,
    user_id: &Uuid,
) -> Result<(String, String), AppError> {
    let now = Utc::now().timestamp();
    let jti = Uuid::new_v4().to_string();

    let claims = Claims {
        sub: user_id.to_string(),
        iss: config.jwt_issuer.clone(),
        aud: config.base_url.clone(),
        exp: now + config.jwt_refresh_ttl_secs,
        iat: now,
        jti: jti.clone(),
        scope: String::new(),
        token_type: "refresh".to_string(),
        roles: None,
        groups: None,
        permissions: None,
        sid: None,
        act: None,
        delegated: None,
        sa: None,
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());

    let token = encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode refresh token: {e}")))?;

    Ok((token, jti))
}

/// TTL for delegation tokens issued via Token Exchange (5 minutes).
pub const DELEGATED_TOKEN_TTL_SECS: i64 = 300;

/// TTL for delegation tokens injected via MCP proxy (5 minutes).
/// Downstream services can refresh these tokens via `POST /api/v1/delegation/refresh`
/// for long-running/agentic workflows.
pub const MCP_DELEGATION_TOKEN_TTL_SECS: i64 = 300;

/// Generate a delegated access token (RFC 8693).
///
/// Like a regular access token, but with:
/// - `act.sub` claim identifying the acting service
/// - `delegated: true` flag
/// - Constrained scope (only delegation-specific scopes)
/// - Configurable short TTL
pub fn generate_delegated_access_token(
    keys: &JwtKeys,
    config: &AppConfig,
    user_id: &Uuid,
    scope: &str,
    acting_client_id: &str,
    ttl_secs: i64,
) -> Result<String, AppError> {
    let now = Utc::now().timestamp();

    let claims = Claims {
        sub: user_id.to_string(),
        iss: config.jwt_issuer.clone(),
        aud: config.base_url.clone(),
        exp: now + ttl_secs,
        iat: now,
        jti: Uuid::new_v4().to_string(),
        scope: scope.to_string(),
        token_type: "access".to_string(),
        roles: None,
        groups: None,
        permissions: None,
        sid: None,
        act: Some(ActorClaim {
            sub: acting_client_id.to_string(),
        }),
        delegated: Some(true),
        sa: None,
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());

    encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode delegated token: {e}")))
}

/// Optional auth context data to embed in ID token claims.
pub struct IdTokenAuthContext {
    pub roles: Option<Vec<String>>,
    pub groups: Option<Vec<String>>,
    pub acr: Option<String>,
    pub amr: Option<Vec<String>>,
    pub auth_time: Option<i64>,
    pub sid: Option<String>,
}

/// Generate an OIDC ID token.
#[allow(clippy::too_many_arguments)]
pub fn generate_id_token(
    keys: &JwtKeys,
    config: &AppConfig,
    user_id: &Uuid,
    email: Option<&str>,
    email_verified: Option<bool>,
    name: Option<&str>,
    picture: Option<&str>,
    audience: &str,
    nonce: Option<&str>,
    access_token: Option<&str>,
    auth_ctx: Option<&IdTokenAuthContext>,
) -> Result<String, AppError> {
    let now = Utc::now().timestamp();

    // Compute at_hash per OIDC Core Section 3.1.3.6: left half of SHA-256
    // of the access token, base64url-encoded.
    let at_hash = access_token.map(|token| {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        let full_hash = hasher.finalize();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&full_hash[..16])
    });

    let claims = IdTokenClaims {
        sub: user_id.to_string(),
        iss: config.jwt_issuer.clone(),
        aud: audience.to_string(),
        exp: now + 3600, // ID tokens are valid for 1 hour
        iat: now,
        email: email.map(String::from),
        email_verified,
        name: name.map(String::from),
        picture: picture.map(String::from),
        nonce: nonce.map(String::from),
        at_hash,
        roles: auth_ctx.and_then(|c| c.roles.clone()),
        groups: auth_ctx.and_then(|c| c.groups.clone()),
        acr: auth_ctx.and_then(|c| c.acr.clone()),
        amr: auth_ctx.and_then(|c| c.amr.clone()),
        auth_time: auth_ctx.and_then(|c| c.auth_time),
        sid: auth_ctx.and_then(|c| c.sid.clone()),
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());

    encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode ID token: {e}")))
}

/// Extract the RSA public key as a JWK (JSON Web Key) for the JWKS endpoint.
pub fn public_key_jwk(public_pem: &str) -> Result<serde_json::Value, AppError> {
    use base64::Engine as _;

    let pub_key = RsaPublicKey::from_pkcs1_pem(public_pem)
        .map_err(|e| AppError::Internal(format!("Failed to parse public key for JWK: {e}")))?;

    let n_bytes = pub_key.n().to_bytes_be();
    let e_bytes = pub_key.e().to_bytes_be();

    let n_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&n_bytes);
    let e_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&e_bytes);

    // Stable kid derived from SHA-256 of the modulus
    let mut hasher = Sha256::new();
    hasher.update(&n_bytes);
    let kid = hex::encode(&hasher.finalize()[..8]);

    Ok(serde_json::json!({
        "kty": "RSA",
        "use": "sig",
        "alg": "RS256",
        "kid": kid,
        "n": n_b64,
        "e": e_b64,
    }))
}

/// Generate an access token for a service account.
///
/// Like a regular access token, but with `sa: true` and no RBAC claims
/// embedded (RBAC is resolved at request time for service accounts).
pub fn generate_service_account_token(
    keys: &JwtKeys,
    config: &AppConfig,
    service_account_id: &str,
    scope: &str,
    ttl_secs: i64,
) -> Result<(String, String), AppError> {
    let now = Utc::now().timestamp();
    let jti = Uuid::new_v4().to_string();

    let claims = Claims {
        sub: service_account_id.to_string(),
        iss: config.jwt_issuer.clone(),
        aud: config.base_url.clone(),
        exp: now + ttl_secs,
        iat: now,
        jti: jti.clone(),
        scope: scope.to_string(),
        token_type: "access".to_string(),
        roles: None,
        groups: None,
        permissions: None,
        sid: None,
        act: None,
        delegated: None,
        sa: Some(true),
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());

    let token = encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode SA token: {e}")))?;

    Ok((token, jti))
}

/// Verify and decode an access or refresh token.
pub fn verify_token(keys: &JwtKeys, config: &AppConfig, token: &str) -> Result<Claims, AppError> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[&config.jwt_issuer]);
    validation.set_audience(&[&config.base_url]);

    let token_data = decode::<Claims>(token, &keys.decoding, &validation).map_err(|e| {
        match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => AppError::TokenExpired,
            _ => AppError::Unauthorized("Invalid token".to_string()),
        }
    })?;

    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1::EncodeRsaPrivateKey;

    /// Generate a test RSA key pair (2048-bit for speed) and return JwtKeys + AppConfig.
    fn test_keys_and_config() -> (JwtKeys, AppConfig) {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = private_key.to_public_key();

        let private_pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap();
        let public_pem = public_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap();

        let encoding = EncodingKey::from_rsa_pem(private_pem.as_bytes()).unwrap();
        let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap();

        let n_bytes = public_key.n().to_bytes_be();
        let mut hasher = Sha256::new();
        hasher.update(&n_bytes);
        let kid = hex::encode(&hasher.finalize()[..8]);

        let keys = JwtKeys { encoding, decoding, kid };

        let config = AppConfig {
            port: 3001,
            base_url: "http://localhost:3001".to_string(),
            frontend_url: "http://localhost:3000".to_string(),
            database_url: "mongodb://localhost:27017/nyxid".to_string(),
            database_max_connections: 10,
            environment: "development".to_string(),
            jwt_private_key_path: "keys/private.pem".to_string(),
            jwt_public_key_path: "keys/public.pem".to_string(),
            jwt_issuer: "http://localhost:3001".to_string(),
            jwt_access_ttl_secs: 900,
            jwt_refresh_ttl_secs: 604800,
            google_client_id: None,
            google_client_secret: None,
            github_client_id: None,
            github_client_secret: None,
            apple_client_id: None,
            apple_team_id: None,
            apple_key_id: None,
            apple_private_key_path: None,
            smtp_host: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_from_address: None,
            encryption_key: "ab".repeat(32),
            rate_limit_per_second: 10,
            rate_limit_burst: 30,
            sa_token_ttl_secs: 3600,
            cookie_domain: None,
        };

        (keys, config)
    }

    #[test]
    fn generate_and_verify_access_token() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let token = generate_access_token(&keys, &config, &user_id, "openid profile", None).unwrap();

        let claims = verify_token(&keys, &config, &token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.token_type, "access");
        assert_eq!(claims.scope, "openid profile");
        assert_eq!(claims.iss, "http://localhost:3001");
    }

    #[test]
    fn generate_and_verify_refresh_token() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let (token, jti) = generate_refresh_token(&keys, &config, &user_id).unwrap();

        let claims = verify_token(&keys, &config, &token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.token_type, "refresh");
        assert_eq!(claims.jti, jti);
        assert!(claims.scope.is_empty());
    }

    #[test]
    fn verify_token_rejects_invalid_token() {
        let (keys, config) = test_keys_and_config();
        let result = verify_token(&keys, &config, "invalid.jwt.token");
        assert!(result.is_err());
    }

    #[test]
    fn verify_token_rejects_expired_token() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let now = Utc::now().timestamp();

        let claims = Claims {
            sub: user_id.to_string(),
            iss: config.jwt_issuer.clone(),
            aud: config.base_url.clone(),
            exp: now - 3600, // expired 1 hour ago
            iat: now - 7200,
            jti: Uuid::new_v4().to_string(),
            scope: String::new(),
            token_type: "access".to_string(),
            roles: None,
            groups: None,
            permissions: None,
            sid: None,
            act: None,
            delegated: None,
            sa: None,
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(keys.kid.clone());
        let token = encode(&header, &claims, &keys.encoding).unwrap();

        let result = verify_token(&keys, &config, &token);
        assert!(matches!(result, Err(AppError::TokenExpired)));
    }

    #[test]
    fn access_token_has_kid_header() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let token = generate_access_token(&keys, &config, &user_id, "openid", None).unwrap();

        // Decode header without validation to check kid
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.kid, Some(keys.kid.clone()));
        assert_eq!(header.alg, Algorithm::RS256);
    }

    #[test]
    fn generate_id_token_basic() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let token = generate_id_token(
            &keys,
            &config,
            &user_id,
            Some("user@example.com"),
            Some(true),
            Some("Test User"),
            None,
            "test-client",
            Some("nonce123"),
            None,
            None,
        )
        .unwrap();

        // Verify we can decode it (use lenient validation since audience differs)
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&config.jwt_issuer]);
        validation.set_audience(&["test-client"]);
        let decoded = decode::<IdTokenClaims>(&token, &keys.decoding, &validation).unwrap();
        assert_eq!(decoded.claims.sub, user_id.to_string());
        assert_eq!(decoded.claims.email, Some("user@example.com".to_string()));
        assert_eq!(decoded.claims.nonce, Some("nonce123".to_string()));
    }

    #[test]
    fn generate_id_token_with_at_hash() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let access_token = generate_access_token(&keys, &config, &user_id, "openid", None).unwrap();

        let id_token = generate_id_token(
            &keys,
            &config,
            &user_id,
            None,
            None,
            None,
            None,
            "test-client",
            None,
            Some(&access_token),
            None,
        )
        .unwrap();

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&config.jwt_issuer]);
        validation.set_audience(&["test-client"]);
        let decoded = decode::<IdTokenClaims>(&id_token, &keys.decoding, &validation).unwrap();
        assert!(decoded.claims.at_hash.is_some());
    }

    #[test]
    fn claims_serde_roundtrip() {
        let claims = Claims {
            sub: "user-123".to_string(),
            iss: "nyxid".to_string(),
            aud: "http://localhost:3001".to_string(),
            exp: 1700000000,
            iat: 1699999000,
            jti: "jti-abc".to_string(),
            scope: "openid profile".to_string(),
            token_type: "access".to_string(),
            roles: None,
            groups: None,
            permissions: None,
            sid: None,
            act: None,
            delegated: None,
            sa: None,
        };
        let json = serde_json::to_string(&claims).unwrap();
        let restored: Claims = serde_json::from_str(&json).unwrap();
        assert_eq!(claims.sub, restored.sub);
        assert_eq!(claims.token_type, restored.token_type);
    }

    #[test]
    fn generate_and_verify_delegated_token() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let token = generate_delegated_access_token(
            &keys, &config, &user_id, "llm:proxy", "test-client-id", 300,
        )
        .unwrap();

        let claims = verify_token(&keys, &config, &token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.token_type, "access");
        assert_eq!(claims.scope, "llm:proxy");
        assert_eq!(claims.delegated, Some(true));
        let act = claims.act.expect("act claim should be present");
        assert_eq!(act.sub, "test-client-id");
    }

    #[test]
    fn delegated_token_respects_ttl() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let token = generate_delegated_access_token(
            &keys, &config, &user_id, "llm:proxy", "svc", 60,
        )
        .unwrap();

        let claims = verify_token(&keys, &config, &token).unwrap();
        // TTL should be ~60 seconds (allow 2s tolerance for test execution time)
        let ttl = claims.exp - claims.iat;
        assert_eq!(ttl, 60);
    }

    #[test]
    fn claims_without_act_deserialize_fine() {
        // Verify that tokens without act/delegated fields still deserialize
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let token = generate_access_token(&keys, &config, &user_id, "openid", None).unwrap();

        let claims = verify_token(&keys, &config, &token).unwrap();
        assert!(claims.act.is_none());
        assert!(claims.delegated.is_none());
    }

    #[test]
    fn id_token_claims_skip_none_at_hash() {
        let claims = IdTokenClaims {
            sub: "user-123".to_string(),
            iss: "nyxid".to_string(),
            aud: "client-1".to_string(),
            exp: 1700000000,
            iat: 1699999000,
            email: None,
            email_verified: None,
            name: None,
            picture: None,
            nonce: None,
            at_hash: None,
            roles: None,
            groups: None,
            acr: None,
            amr: None,
            auth_time: None,
            sid: None,
        };
        let json = serde_json::to_value(&claims).unwrap();
        // at_hash should be absent when None (skip_serializing_if)
        assert!(json.get("at_hash").is_none());
        // New optional fields should also be absent when None
        assert!(json.get("roles").is_none());
        assert!(json.get("groups").is_none());
        assert!(json.get("acr").is_none());
        assert!(json.get("amr").is_none());
        assert!(json.get("auth_time").is_none());
        assert!(json.get("sid").is_none());
    }

    #[test]
    fn generate_and_verify_service_account_token() {
        let (keys, config) = test_keys_and_config();
        let sa_id = Uuid::new_v4().to_string();
        let (token, jti) = generate_service_account_token(
            &keys, &config, &sa_id, "proxy:* llm:proxy", 3600,
        )
        .unwrap();

        let claims = verify_token(&keys, &config, &token).unwrap();
        assert_eq!(claims.sub, sa_id);
        assert_eq!(claims.token_type, "access");
        assert_eq!(claims.scope, "proxy:* llm:proxy");
        assert_eq!(claims.sa, Some(true));
        assert_eq!(claims.jti, jti);
        assert!(claims.act.is_none());
        assert!(claims.delegated.is_none());
        assert!(claims.roles.is_none());
        assert!(claims.groups.is_none());
        assert!(claims.sid.is_none());
    }

    #[test]
    fn service_account_token_respects_ttl() {
        let (keys, config) = test_keys_and_config();
        let sa_id = Uuid::new_v4().to_string();
        let (token, _) = generate_service_account_token(
            &keys, &config, &sa_id, "proxy:*", 120,
        )
        .unwrap();

        let claims = verify_token(&keys, &config, &token).unwrap();
        let ttl = claims.exp - claims.iat;
        assert_eq!(ttl, 120);
    }

    #[test]
    fn sa_claim_skipped_when_none() {
        let (keys, config) = test_keys_and_config();
        let user_id = Uuid::new_v4();
        let token = generate_access_token(&keys, &config, &user_id, "openid", None).unwrap();

        let claims = verify_token(&keys, &config, &token).unwrap();
        assert!(claims.sa.is_none());

        // Verify the JSON doesn't include "sa" when None
        let json = serde_json::to_string(&claims).unwrap();
        assert!(!json.contains("\"sa\""));
    }
}
