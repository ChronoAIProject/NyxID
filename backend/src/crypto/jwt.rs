use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey};
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
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

        Ok(Self { encoding, decoding })
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

/// Generate an access token for the given user.
pub fn generate_access_token(
    keys: &JwtKeys,
    config: &AppConfig,
    user_id: &Uuid,
    scope: &str,
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
    };

    let header = Header::new(Algorithm::RS256);

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
    };

    let header = Header::new(Algorithm::RS256);

    let token = encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode refresh token: {e}")))?;

    Ok((token, jti))
}

/// Generate an OIDC ID token.
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
) -> Result<String, AppError> {
    let now = Utc::now().timestamp();

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
    };

    let header = Header::new(Algorithm::RS256);

    encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode ID token: {e}")))
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
