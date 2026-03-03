use std::collections::HashMap;
use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::errors::{AppError, AppResult};

const GOOGLE_JWKS_URI: &str = "https://www.googleapis.com/oauth2/v3/certs";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";
const JWKS_DEFAULT_TTL_SECS: u64 = 3600;
const JWKS_MIN_TTL_SECS: u64 = 300;
const JWKS_MAX_TTL_SECS: u64 = 86400;
const GOOGLE_ID_TOKEN_MAX_AGE_SECS: i64 = 600;
const CLOCK_SKEW_TOLERANCE_SECS: i64 = 30;

/// Cached JWKS entry for a single provider endpoint.
struct CachedJwks {
    keys: Vec<CachedKey>,
    fetched_at: Instant,
    max_age: Duration,
}

/// A single cached JWK with its decoded key material.
struct CachedKey {
    kid: Option<String>,
    decoding_key: DecodingKey,
    algorithm: Algorithm,
}

/// Google ID token claims extracted after verification.
#[derive(Debug, Deserialize)]
pub struct GoogleIdTokenClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub picture: Option<String>,
}

/// Thread-safe JWKS cache that fetches and caches remote JWKS endpoints.
pub struct JwksCache {
    inner: RwLock<HashMap<String, CachedJwks>>,
    http_client: reqwest::Client,
}

/// Raw JWKS JSON response from a provider endpoint.
#[derive(Deserialize)]
struct JwksResponse {
    keys: Vec<JwkKey>,
}

/// A single JWK from the JWKS endpoint.
#[derive(Deserialize)]
struct JwkKey {
    kty: String,
    kid: Option<String>,
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
    #[serde(rename = "use")]
    key_use: Option<String>,
}

/// Classification of JWT verification failures to decide whether JWKS refresh
/// might help.
#[derive(Debug)]
enum VerifyError {
    NoMatchingKey,
    InvalidSignature,
    Expired,
    InvalidAudience,
    InvalidIssuer,
    Other(String),
}

impl VerifyError {
    fn should_refresh_keys(&self) -> bool {
        matches!(self, Self::NoMatchingKey | Self::InvalidSignature)
    }

    fn into_app_error(self) -> AppError {
        match self {
            Self::NoMatchingKey => {
                AppError::ExternalTokenInvalid("No matching key found for token".to_string())
            }
            Self::InvalidSignature => {
                AppError::ExternalTokenInvalid("Token signature is invalid".to_string())
            }
            Self::Expired => AppError::ExternalTokenInvalid("Token has expired".to_string()),
            Self::InvalidAudience => AppError::ExternalTokenInvalid("Invalid audience".to_string()),
            Self::InvalidIssuer => AppError::ExternalTokenInvalid("Invalid issuer".to_string()),
            Self::Other(message) => {
                AppError::ExternalTokenInvalid(format!("Token verification failed: {message}"))
            }
        }
    }
}

impl JwksCache {
    /// Create a new JWKS cache with the given HTTP client.
    pub fn new(http_client: reqwest::Client) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            http_client,
        }
    }

    /// Fetch JWKS keys for a given URI. Returns cached keys if still fresh.
    async fn get_keys(&self, jwks_uri: &str) -> AppResult<Vec<CachedKey>> {
        // Check cache (read lock)
        {
            let cache = self.inner.read().await;
            if let Some(entry) = cache.get(jwks_uri)
                && entry.fetched_at.elapsed() < entry.max_age
            {
                return Ok(entry
                    .keys
                    .iter()
                    .map(|k| CachedKey {
                        kid: k.kid.clone(),
                        decoding_key: k.decoding_key.clone(),
                        algorithm: k.algorithm,
                    })
                    .collect());
            }
        }

        // Cache miss or stale -- fetch fresh keys
        self.fetch_and_cache(jwks_uri, false).await
    }

    /// Force-refresh keys for a given URI (used on verification failure).
    async fn force_refresh(&self, jwks_uri: &str) -> AppResult<Vec<CachedKey>> {
        self.fetch_and_cache(jwks_uri, true).await
    }

    /// Fetch JWKS from the remote endpoint, parse keys, and update cache.
    ///
    /// When `force` is false, performs double-checked locking to avoid cache
    /// stampede: re-checks cache freshness before issuing the HTTP request.
    async fn fetch_and_cache(&self, jwks_uri: &str, force: bool) -> AppResult<Vec<CachedKey>> {
        // Double-checked locking: another task may have refreshed while we waited
        if !force {
            let cache = self.inner.read().await;
            if let Some(entry) = cache.get(jwks_uri)
                && entry.fetched_at.elapsed() < entry.max_age
            {
                return Ok(entry
                    .keys
                    .iter()
                    .map(|k| CachedKey {
                        kid: k.kid.clone(),
                        decoding_key: k.decoding_key.clone(),
                        algorithm: k.algorithm,
                    })
                    .collect());
            }
        }

        let response = self.http_client.get(jwks_uri).send().await.map_err(|e| {
            tracing::error!(uri = %jwks_uri, error = %e, "JWKS fetch failed");
            AppError::ExternalTokenInvalid("Failed to fetch provider signing keys".to_string())
        })?;

        // Parse Cache-Control max-age for TTL
        let max_age = parse_cache_control_max_age(
            response
                .headers()
                .get(reqwest::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
        );

        let jwks: JwksResponse = response.json().await.map_err(|e| {
            tracing::error!(uri = %jwks_uri, error = %e, "JWKS JSON parse failed");
            AppError::ExternalTokenInvalid("Failed to parse provider signing keys".to_string())
        })?;

        let mut cached_keys = Vec::new();
        for jwk in &jwks.keys {
            if let Some(key) = parse_jwk(jwk) {
                cached_keys.push(key);
            }
        }

        if cached_keys.is_empty() {
            return Err(AppError::ExternalTokenInvalid(
                "No usable signing keys from provider".to_string(),
            ));
        }

        // Clone keys for return before moving into cache
        let result: Vec<CachedKey> = cached_keys
            .iter()
            .map(|k| CachedKey {
                kid: k.kid.clone(),
                decoding_key: k.decoding_key.clone(),
                algorithm: k.algorithm,
            })
            .collect();

        // Update cache (write lock)
        {
            let mut cache = self.inner.write().await;
            cache.insert(
                jwks_uri.to_string(),
                CachedJwks {
                    keys: cached_keys,
                    fetched_at: Instant::now(),
                    max_age,
                },
            );
        }

        Ok(result)
    }

    /// Verify a Google ID token and return the parsed claims.
    ///
    /// Performs full JWT verification: RS256 signature via JWKS, issuer, audience,
    /// expiry, and iat freshness check.
    pub async fn verify_google_id_token(
        &self,
        token: &str,
        expected_audience: &str,
    ) -> AppResult<GoogleIdTokenClaims> {
        // Decode header to get kid (without verifying signature yet)
        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| AppError::ExternalTokenInvalid(format!("Invalid JWT header: {e}")))?;

        // Only accept RS256
        if header.alg != Algorithm::RS256 {
            return Err(AppError::ExternalTokenInvalid(format!(
                "Unsupported algorithm: {:?} (expected RS256)",
                header.alg
            )));
        }

        let kid = header.kid.as_deref();

        // Try cached keys first
        let keys = self.get_keys(GOOGLE_JWKS_URI).await?;
        match verify_with_keys(token, kid, &keys, expected_audience) {
            Ok(claims) => return validate_google_claims(claims),
            Err(err) if err.should_refresh_keys() => {
                // Key not found or signature mismatch -- key rotation may have happened.
                tracing::debug!(kid = ?kid, error = ?err, "JWKS refresh candidate");
            }
            Err(err) => return Err(err.into_app_error()),
        }

        // Force refresh and retry once
        let keys = self.force_refresh(GOOGLE_JWKS_URI).await?;
        let claims = verify_with_keys(token, kid, &keys, expected_audience).map_err(|err| {
            tracing::debug!(kid = ?kid, error = ?err, "JWKS refresh did not resolve verification");
            err.into_app_error()
        })?;
        validate_google_claims(claims)
    }
}

/// Attempt to verify the token against the cached keys.
fn verify_with_keys(
    token: &str,
    kid: Option<&str>,
    keys: &[CachedKey],
    expected_audience: &str,
) -> Result<GoogleIdTokenClaims, VerifyError> {
    // Find matching key by kid, or try all keys if no kid
    let matching_keys: Vec<&CachedKey> = if let Some(kid) = kid {
        keys.iter()
            .filter(|k| k.kid.as_deref() == Some(kid))
            .collect()
    } else {
        keys.iter().collect()
    };

    if matching_keys.is_empty() {
        return Err(VerifyError::NoMatchingKey);
    }

    let mut last_err = None;
    for key in matching_keys {
        let mut validation = Validation::new(key.algorithm);
        validation.set_issuer(&[GOOGLE_ISSUER]);
        validation.set_audience(&[expected_audience]);

        match decode::<GoogleIdTokenClaims>(token, &key.decoding_key, &validation) {
            Ok(token_data) => return Ok(token_data.claims),
            Err(e) => {
                last_err = Some(e);
            }
        }
    }

    let err = last_err.unwrap_or_else(|| {
        jsonwebtoken::errors::Error::from(jsonwebtoken::errors::ErrorKind::InvalidToken)
    });
    Err(match err.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => VerifyError::Expired,
        jsonwebtoken::errors::ErrorKind::InvalidAudience => VerifyError::InvalidAudience,
        jsonwebtoken::errors::ErrorKind::InvalidIssuer => VerifyError::InvalidIssuer,
        jsonwebtoken::errors::ErrorKind::InvalidSignature => VerifyError::InvalidSignature,
        _ => VerifyError::Other(err.to_string()),
    })
}

/// Validate Google-specific claims beyond basic JWT verification.
fn validate_google_claims(claims: GoogleIdTokenClaims) -> AppResult<GoogleIdTokenClaims> {
    let now = chrono::Utc::now().timestamp();

    // Reject tokens with future iat (with clock skew tolerance)
    if claims.iat > now + CLOCK_SKEW_TOLERANCE_SECS {
        return Err(AppError::ExternalTokenInvalid(
            "Token issued_at is in the future".to_string(),
        ));
    }

    // Check iat freshness (reject tokens older than 10 minutes + clock skew tolerance)
    if now - claims.iat > GOOGLE_ID_TOKEN_MAX_AGE_SECS + CLOCK_SKEW_TOLERANCE_SECS {
        return Err(AppError::ExternalTokenInvalid(
            "Token is too old (iat exceeds maximum age)".to_string(),
        ));
    }

    // Require sub to be present and non-empty
    if claims.sub.is_empty() {
        return Err(AppError::ExternalTokenInvalid(
            "Missing subject claim".to_string(),
        ));
    }

    Ok(claims)
}

/// Parse a single JWK into a CachedKey. Returns None for unsupported key types.
fn parse_jwk(jwk: &JwkKey) -> Option<CachedKey> {
    // Only RSA signing keys
    if jwk.kty != "RSA" {
        return None;
    }

    // Only signature keys (not encryption)
    if jwk.key_use.as_deref() == Some("enc") {
        return None;
    }

    let alg = match jwk.alg.as_deref() {
        Some("RS256") => Algorithm::RS256,
        _ => return None,
    };

    let n = jwk.n.as_deref()?;
    let e = jwk.e.as_deref()?;

    let decoding_key = DecodingKey::from_rsa_components(n, e).ok()?;

    Some(CachedKey {
        kid: jwk.kid.clone(),
        decoding_key,
        algorithm: alg,
    })
}

/// Parse Cache-Control header for max-age value, clamped to [min, max] TTL.
fn parse_cache_control_max_age(header: Option<&str>) -> Duration {
    let secs = header
        .and_then(|v| {
            v.split(',')
                .map(|s| s.trim())
                .find(|s| s.starts_with("max-age="))
                .and_then(|s| s.strip_prefix("max-age="))
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(JWKS_DEFAULT_TTL_SECS);

    let clamped = secs.clamp(JWKS_MIN_TTL_SECS, JWKS_MAX_TTL_SECS);
    Duration::from_secs(clamped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cache_control_max_age_basic() {
        let duration = parse_cache_control_max_age(Some("public, max-age=19291, must-revalidate"));
        assert_eq!(duration, Duration::from_secs(19291));
    }

    #[test]
    fn parse_cache_control_max_age_clamped_low() {
        let duration = parse_cache_control_max_age(Some("max-age=10"));
        assert_eq!(duration, Duration::from_secs(JWKS_MIN_TTL_SECS));
    }

    #[test]
    fn parse_cache_control_max_age_clamped_high() {
        let duration = parse_cache_control_max_age(Some("max-age=999999"));
        assert_eq!(duration, Duration::from_secs(JWKS_MAX_TTL_SECS));
    }

    #[test]
    fn parse_cache_control_max_age_missing() {
        let duration = parse_cache_control_max_age(None);
        assert_eq!(duration, Duration::from_secs(JWKS_DEFAULT_TTL_SECS));
    }

    #[test]
    fn parse_cache_control_max_age_no_max_age() {
        let duration = parse_cache_control_max_age(Some("no-cache, no-store"));
        assert_eq!(duration, Duration::from_secs(JWKS_DEFAULT_TTL_SECS));
    }

    #[test]
    fn parse_jwk_valid_rsa() {
        let jwk = JwkKey {
            kty: "RSA".to_string(),
            kid: Some("test-kid".to_string()),
            alg: Some("RS256".to_string()),
            n: Some("0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw".to_string()),
            e: Some("AQAB".to_string()),
            key_use: Some("sig".to_string()),
        };

        let result = parse_jwk(&jwk);
        assert!(result.is_some());
        let key = result.unwrap();
        assert_eq!(key.kid, Some("test-kid".to_string()));
        assert_eq!(key.algorithm, Algorithm::RS256);
    }

    #[test]
    fn parse_jwk_rejects_non_rsa() {
        let jwk = JwkKey {
            kty: "EC".to_string(),
            kid: Some("ec-kid".to_string()),
            alg: Some("ES256".to_string()),
            n: None,
            e: None,
            key_use: Some("sig".to_string()),
        };

        assert!(parse_jwk(&jwk).is_none());
    }

    #[test]
    fn parse_jwk_rejects_enc_use() {
        let jwk = JwkKey {
            kty: "RSA".to_string(),
            kid: Some("enc-kid".to_string()),
            alg: Some("RS256".to_string()),
            n: Some("AQAB".to_string()),
            e: Some("AQAB".to_string()),
            key_use: Some("enc".to_string()),
        };

        assert!(parse_jwk(&jwk).is_none());
    }

    #[test]
    fn parse_jwk_rejects_non_rs256_alg() {
        let jwk = JwkKey {
            kty: "RSA".to_string(),
            kid: Some("kid".to_string()),
            alg: Some("RS384".to_string()),
            n: Some("AQAB".to_string()),
            e: Some("AQAB".to_string()),
            key_use: Some("sig".to_string()),
        };

        assert!(parse_jwk(&jwk).is_none());
    }

    #[test]
    fn validate_google_claims_rejects_old_iat() {
        let old_iat = chrono::Utc::now().timestamp() - 700; // 700s ago, > 600s + 30s tolerance
        let claims = GoogleIdTokenClaims {
            sub: "123".to_string(),
            iss: GOOGLE_ISSUER.to_string(),
            aud: "test".to_string(),
            exp: chrono::Utc::now().timestamp() + 3600,
            iat: old_iat,
            email: Some("user@example.com".to_string()),
            email_verified: Some(true),
            name: None,
            picture: None,
        };

        let result = validate_google_claims(claims);
        assert!(result.is_err());
    }

    #[test]
    fn validate_google_claims_rejects_empty_sub() {
        let now = chrono::Utc::now().timestamp();
        let claims = GoogleIdTokenClaims {
            sub: String::new(),
            iss: GOOGLE_ISSUER.to_string(),
            aud: "test".to_string(),
            exp: now + 3600,
            iat: now,
            email: Some("user@example.com".to_string()),
            email_verified: Some(true),
            name: None,
            picture: None,
        };

        let result = validate_google_claims(claims);
        assert!(result.is_err());
    }

    #[test]
    fn validate_google_claims_rejects_future_iat() {
        let future_iat = chrono::Utc::now().timestamp() + 60; // 60s in future, > 30s tolerance
        let claims = GoogleIdTokenClaims {
            sub: "123".to_string(),
            iss: GOOGLE_ISSUER.to_string(),
            aud: "test".to_string(),
            exp: chrono::Utc::now().timestamp() + 3600,
            iat: future_iat,
            email: Some("user@example.com".to_string()),
            email_verified: Some(true),
            name: None,
            picture: None,
        };

        let result = validate_google_claims(claims);
        assert!(result.is_err());
    }

    #[test]
    fn parse_jwk_rejects_missing_alg() {
        let jwk = JwkKey {
            kty: "RSA".to_string(),
            kid: Some("no-alg-kid".to_string()),
            alg: None,
            n: Some("0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw".to_string()),
            e: Some("AQAB".to_string()),
            key_use: Some("sig".to_string()),
        };

        assert!(parse_jwk(&jwk).is_none());
    }

    #[test]
    fn validate_google_claims_accepts_valid() {
        let now = chrono::Utc::now().timestamp();
        let claims = GoogleIdTokenClaims {
            sub: "google-user-123".to_string(),
            iss: GOOGLE_ISSUER.to_string(),
            aud: "test-client".to_string(),
            exp: now + 3600,
            iat: now - 30,
            email: Some("user@example.com".to_string()),
            email_verified: Some(true),
            name: Some("Test User".to_string()),
            picture: None,
        };

        let result = validate_google_claims(claims);
        assert!(result.is_ok());
    }

    #[test]
    fn verify_error_refresh_policy() {
        assert!(VerifyError::NoMatchingKey.should_refresh_keys());
        assert!(VerifyError::InvalidSignature.should_refresh_keys());
        assert!(!VerifyError::InvalidAudience.should_refresh_keys());
        assert!(!VerifyError::InvalidIssuer.should_refresh_keys());
        assert!(!VerifyError::Expired.should_refresh_keys());
        assert!(!VerifyError::Other("x".to_string()).should_refresh_keys());
    }
}
