use std::sync::OnceLock;

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};

/// TTL for the generated Apple client secret JWT (5 minutes).
/// Apple allows up to 6 months, but we regenerate per-request.
const CLIENT_SECRET_TTL_SECS: i64 = 300;

const APPLE_AUD: &str = "https://appleid.apple.com";

/// Cached encoding key so we only read and parse the .p8 file once.
/// Wrapped in Result to avoid re-attempting failed initialization.
static APPLE_ENCODING_KEY: OnceLock<Result<EncodingKey, String>> = OnceLock::new();

#[derive(Debug, Serialize)]
struct AppleClientSecretClaims {
    iss: String,
    iat: i64,
    exp: i64,
    aud: String,
    sub: String,
}

/// Get or initialize the cached Apple encoding key from the .p8 file.
fn get_encoding_key(key_path: &str) -> AppResult<&'static EncodingKey> {
    let result = APPLE_ENCODING_KEY.get_or_init(|| {
        let pem = match std::fs::read(key_path) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(path = %key_path, error = %e, "Failed to read Apple private key");
                return Err("Failed to read Apple private key".to_string());
            }
        };

        EncodingKey::from_ec_pem(&pem).map_err(|e| {
            tracing::error!(error = %e, "Failed to parse Apple private key as EC PEM");
            "Failed to parse Apple private key".to_string()
        })
    });

    result
        .as_ref()
        .map_err(|msg| AppError::Internal(msg.clone()))
}

/// Generate an Apple client_secret JWT (ES256-signed).
///
/// The JWT contains:
/// - iss: Apple Team ID
/// - sub: Apple Services ID (client_id)
/// - aud: https://appleid.apple.com
/// - iat/exp: current time + 5 minutes
///
/// Header includes kid (Apple Key ID) and alg (ES256).
/// The .p8 private key is read once and cached for the process lifetime.
pub fn generate_apple_client_secret(config: &AppConfig) -> AppResult<String> {
    let team_id = config.apple_team_id.as_deref().ok_or_else(|| {
        AppError::ExternalProviderNotConfigured(
            "Apple provider not configured (missing APPLE_TEAM_ID)".to_string(),
        )
    })?;
    let client_id = config.apple_client_id.as_deref().ok_or_else(|| {
        AppError::ExternalProviderNotConfigured(
            "Apple provider not configured (missing APPLE_CLIENT_ID)".to_string(),
        )
    })?;
    let key_id = config.apple_key_id.as_deref().ok_or_else(|| {
        AppError::ExternalProviderNotConfigured(
            "Apple provider not configured (missing APPLE_KEY_ID)".to_string(),
        )
    })?;
    let key_path = config.apple_private_key_path.as_deref().ok_or_else(|| {
        AppError::ExternalProviderNotConfigured(
            "Apple provider not configured (missing APPLE_PRIVATE_KEY_PATH)".to_string(),
        )
    })?;

    let encoding_key = get_encoding_key(key_path)?;

    let now = chrono::Utc::now().timestamp();
    let claims = AppleClientSecretClaims {
        iss: team_id.to_string(),
        iat: now,
        exp: now + CLIENT_SECRET_TTL_SECS,
        aud: APPLE_AUD.to_string(),
        sub: client_id.to_string(),
    };

    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key_id.to_string());

    encode(&header, &claims, encoding_key).map_err(|e| {
        tracing::error!(error = %e, "Failed to sign Apple client secret JWT");
        AppError::Internal("Failed to generate Apple client secret".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use serde::Deserialize;
    use uuid::Uuid;

    const TEST_APPLE_P8: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg+X37GqD34I6Hski4\n\
OBTYEbs17zyU5fioiG0K2vj9+2qhRANCAASOPZbqdBl6KWu50dBPA6B8Z3htIql2\n\
ci0O2dgc19c2/sLtanU7P2KAzhEo8O0tIc0Dwe/nMqKfue82eGVL3DqM\n\
-----END PRIVATE KEY-----\n";

    #[derive(Debug, Deserialize)]
    struct ParsedClaims {
        iss: String,
        sub: String,
        aud: String,
        iat: i64,
        exp: i64,
    }

    fn make_test_config(private_key_path: Option<String>) -> AppConfig {
        AppConfig {
            port: 3001,
            base_url: "https://auth.example.com".to_string(),
            frontend_url: "https://app.example.com".to_string(),
            database_url: "mongodb://localhost:27017/nyxid".to_string(),
            database_max_connections: 10,
            environment: "test".to_string(),
            jwt_private_key_path: "keys/private.pem".to_string(),
            jwt_public_key_path: "keys/public.pem".to_string(),
            jwt_issuer: "https://auth.example.com".to_string(),
            jwt_access_ttl_secs: 900,
            jwt_refresh_ttl_secs: 604800,
            google_client_id: None,
            google_client_secret: None,
            github_client_id: None,
            github_client_secret: None,
            apple_client_id: Some("com.example.nyxid".to_string()),
            apple_team_id: Some("TEAM123".to_string()),
            apple_key_id: Some("KEY123".to_string()),
            apple_private_key_path: private_key_path,
            smtp_host: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_from_address: None,
            encryption_key: Some("ab".repeat(32)),
            encryption_key_previous: None,
            rate_limit_per_second: 10,
            rate_limit_burst: 30,
            sa_token_ttl_secs: 3600,
            cookie_domain: None,
            telegram_bot_token: None,
            telegram_webhook_secret: None,
            telegram_webhook_url: None,
            telegram_bot_username: None,
            approval_expiry_interval_secs: 5,
            fcm_service_account_path: None,
            fcm_project_id: None,
            apns_key_path: None,
            apns_key_id: None,
            apns_team_id: None,
            apns_topic: None,
            apns_sandbox: true,
            key_provider: "local".to_string(),
            aws_kms_key_arn: None,
            aws_kms_key_arn_previous: None,
            gcp_kms_key_name: None,
            gcp_kms_key_name_previous: None,
            cors_allowed_origins: vec![],
            node_heartbeat_interval_secs: 30,
            node_heartbeat_timeout_secs: 90,
            node_proxy_timeout_secs: 30,
            node_registration_token_ttl_secs: 3600,
            node_max_per_user: 10,
            node_max_ws_connections: 100,
            node_max_stream_duration_secs: 300,
            node_hmac_signing_enabled: true,
        }
    }

    fn parse_claims_unverified(jwt: &str) -> ParsedClaims {
        let payload = jwt
            .split('.')
            .nth(1)
            .expect("JWT should have payload segment");
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .expect("payload should be base64url");
        serde_json::from_slice(&decoded).expect("payload should be valid JSON claims")
    }

    #[test]
    fn generate_client_secret_includes_expected_header_and_claims() {
        let key_path = std::env::temp_dir().join(format!("nyxid-apple-{}.p8", Uuid::new_v4()));
        std::fs::write(&key_path, TEST_APPLE_P8).expect("failed to write test key");

        let config = make_test_config(Some(
            key_path
                .to_str()
                .expect("tmp key path should be UTF-8")
                .to_string(),
        ));
        let jwt = generate_apple_client_secret(&config).expect("client secret should generate");

        let header = jsonwebtoken::decode_header(&jwt).expect("header should decode");
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("KEY123"));

        let claims = parse_claims_unverified(&jwt);
        assert_eq!(claims.iss, "TEAM123");
        assert_eq!(claims.sub, "com.example.nyxid");
        assert_eq!(claims.aud, APPLE_AUD);
        assert!(claims.exp > claims.iat);
        assert!(claims.exp - claims.iat <= CLIENT_SECRET_TTL_SECS + 1);

        let _ = std::fs::remove_file(key_path);
    }

    #[test]
    fn generate_client_secret_fails_when_key_path_missing() {
        let config = make_test_config(None);
        let err = generate_apple_client_secret(&config).expect_err("missing key path should fail");
        assert!(matches!(err, AppError::ExternalProviderNotConfigured(_)));
    }
}
