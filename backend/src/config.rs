use std::env;

/// Application configuration loaded from environment variables.
#[derive(Clone)]
pub struct AppConfig {
    /// Server port (default: 3001)
    pub port: u16,
    /// Base URL for the backend (e.g. https://auth.nyxid.dev)
    pub base_url: String,
    /// Frontend URL for CORS and redirects (e.g. https://nyxid.dev)
    pub frontend_url: String,
    /// MongoDB connection string
    pub database_url: String,
    /// Maximum database connection pool size
    pub database_max_connections: u32,

    /// Environment: "development", "staging", "production"
    pub environment: String,

    // JWT configuration
    /// Path to RSA private key PEM file for signing JWTs
    pub jwt_private_key_path: String,
    /// Path to RSA public key PEM file for verifying JWTs
    pub jwt_public_key_path: String,
    /// JWT issuer claim
    pub jwt_issuer: String,
    /// Access token TTL in seconds (default: 900 = 15 min)
    pub jwt_access_ttl_secs: i64,
    /// Refresh token TTL in seconds (default: 604800 = 7 days)
    pub jwt_refresh_ttl_secs: i64,

    // Social login providers
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,

    // Apple Sign In
    pub apple_client_id: Option<String>,
    pub apple_team_id: Option<String>,
    pub apple_key_id: Option<String>,
    pub apple_private_key_path: Option<String>,

    // SMTP configuration
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_from_address: Option<String>,

    // Encryption
    /// 32-byte hex-encoded AES-256 key for local envelope encryption and
    /// legacy v0/v1 decrypt fallback.
    ///
    /// Required when `KEY_PROVIDER=local`. Optional for other providers.
    pub encryption_key: Option<String>,
    /// Optional previous encryption key for key rotation (same format as
    /// `encryption_key`).
    pub encryption_key_previous: Option<String>,

    // Rate limiting
    /// Max requests per second per IP for general endpoints
    pub rate_limit_per_second: u64,
    /// Max burst size for rate limiter
    pub rate_limit_burst: u32,

    /// Service account token TTL in seconds (default: 3600 = 1 hour)
    pub sa_token_ttl_secs: i64,

    /// Optional cookie domain for cross-subdomain auth (e.g. ".chrono-ai.fun").
    /// When set, cookies include `Domain=<value>` so they are shared across
    /// subdomains. Leave unset for single-domain / localhost development.
    pub cookie_domain: Option<String>,

    /// Telegram Bot API token for sending approval notifications.
    pub telegram_bot_token: Option<String>,

    /// Secret token for verifying Telegram webhook callbacks.
    pub telegram_webhook_secret: Option<String>,

    /// Public URL where Telegram sends webhook callbacks.
    pub telegram_webhook_url: Option<String>,

    /// Telegram bot username (without @) for link instructions.
    pub telegram_bot_username: Option<String>,

    /// Interval in seconds between approval expiry sweeps (default: 5).
    pub approval_expiry_interval_secs: u64,

    // -- FCM (Firebase Cloud Messaging) --
    /// Path to FCM service account JSON file.
    pub fcm_service_account_path: Option<String>,

    /// FCM project ID (extracted from service account JSON at startup).
    pub fcm_project_id: Option<String>,

    // -- APNs (Apple Push Notification service) --
    /// Path to APNs .p8 private key file.
    pub apns_key_path: Option<String>,

    /// APNs Key ID (from Apple Developer portal).
    pub apns_key_id: Option<String>,

    /// APNs Team ID (from Apple Developer portal).
    pub apns_team_id: Option<String>,

    /// APNs topic (bundle ID of the iOS app, e.g. "dev.nyxid.app").
    pub apns_topic: Option<String>,

    /// Use APNs sandbox instead of production.
    /// Default: true in development, false otherwise.
    pub apns_sandbox: bool,

    /// Key provider type for envelope encryption KEK operations.
    /// Supported: "local", "aws-kms" (feature aws-kms), "gcp-kms" (feature gcp-kms).
    pub key_provider: String,

    // AWS KMS (Phase 4)
    /// AWS KMS key ARN for DEK wrapping. Required when KEY_PROVIDER=aws-kms.
    pub aws_kms_key_arn: Option<String>,
    /// Optional previous AWS KMS key ARN for multi-key migration.
    pub aws_kms_key_arn_previous: Option<String>,

    // GCP KMS (Phase 4)
    /// GCP Cloud KMS key resource name. Required when KEY_PROVIDER=gcp-kms.
    pub gcp_kms_key_name: Option<String>,
    /// Optional previous GCP KMS key name for multi-key migration.
    pub gcp_kms_key_name_previous: Option<String>,
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConfig")
            .field("port", &self.port)
            .field("base_url", &self.base_url)
            .field("frontend_url", &self.frontend_url)
            .field("database_url", &self.database_url)
            .field("database_max_connections", &self.database_max_connections)
            .field("environment", &self.environment)
            .field("jwt_private_key_path", &self.jwt_private_key_path)
            .field("jwt_public_key_path", &self.jwt_public_key_path)
            .field("jwt_issuer", &self.jwt_issuer)
            .field("jwt_access_ttl_secs", &self.jwt_access_ttl_secs)
            .field("jwt_refresh_ttl_secs", &self.jwt_refresh_ttl_secs)
            .field("google_client_id", &self.google_client_id)
            .field("google_client_secret", &"[REDACTED]")
            .field("github_client_id", &self.github_client_id)
            .field("github_client_secret", &"[REDACTED]")
            .field("apple_client_id", &self.apple_client_id)
            .field("apple_team_id", &self.apple_team_id)
            .field("apple_key_id", &self.apple_key_id)
            .field("apple_private_key_path", &self.apple_private_key_path)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("smtp_username", &self.smtp_username)
            .field("smtp_password", &"[REDACTED]")
            .field("smtp_from_address", &self.smtp_from_address)
            .field(
                "encryption_key",
                if self.encryption_key.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .field(
                "encryption_key_previous",
                if self.encryption_key_previous.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("rate_limit_burst", &self.rate_limit_burst)
            .field("sa_token_ttl_secs", &self.sa_token_ttl_secs)
            .field("cookie_domain", &self.cookie_domain)
            .field("telegram_bot_token", &"[REDACTED]")
            .field("telegram_webhook_secret", &"[REDACTED]")
            .field("telegram_webhook_url", &self.telegram_webhook_url)
            .field("telegram_bot_username", &self.telegram_bot_username)
            .field(
                "approval_expiry_interval_secs",
                &self.approval_expiry_interval_secs,
            )
            .field("fcm_service_account_path", &self.fcm_service_account_path)
            .field("fcm_project_id", &self.fcm_project_id)
            .field("apns_key_path", &self.apns_key_path)
            .field("apns_key_id", &self.apns_key_id)
            .field("apns_team_id", &self.apns_team_id)
            .field("apns_topic", &self.apns_topic)
            .field("apns_sandbox", &self.apns_sandbox)
            .field("key_provider", &self.key_provider)
            .field(
                "aws_kms_key_arn",
                if self.aws_kms_key_arn.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .field(
                "aws_kms_key_arn_previous",
                if self.aws_kms_key_arn_previous.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .field(
                "gcp_kms_key_name",
                if self.gcp_kms_key_name.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .field(
                "gcp_kms_key_name_previous",
                if self.gcp_kms_key_name_previous.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

impl AppConfig {
    /// Load configuration from environment variables.
    /// Panics on missing required variables to fail fast at startup.
    pub fn from_env() -> Self {
        let environment = env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string());
        let is_dev = environment == "development" || environment == "dev";

        let base_url = env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:3001".to_string());

        Self {
            port: env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3001),
            frontend_url: env::var("FRONTEND_URL")
                .unwrap_or_else(|_| "http://localhost:3000".to_string()),
            database_url: env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            database_max_connections: env::var("DATABASE_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),

            environment,

            jwt_private_key_path: env::var("JWT_PRIVATE_KEY_PATH")
                .unwrap_or_else(|_| "keys/private.pem".to_string()),
            jwt_public_key_path: env::var("JWT_PUBLIC_KEY_PATH")
                .unwrap_or_else(|_| "keys/public.pem".to_string()),
            jwt_issuer: env::var("JWT_ISSUER").unwrap_or_else(|_| base_url.clone()),

            base_url,
            jwt_access_ttl_secs: env::var("JWT_ACCESS_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
            jwt_refresh_ttl_secs: env::var("JWT_REFRESH_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(604800),

            google_client_id: env::var("GOOGLE_CLIENT_ID").ok(),
            google_client_secret: env::var("GOOGLE_CLIENT_SECRET").ok(),
            github_client_id: env::var("GITHUB_CLIENT_ID").ok(),
            github_client_secret: env::var("GITHUB_CLIENT_SECRET").ok(),

            apple_client_id: env::var("APPLE_CLIENT_ID").ok().filter(|s| !s.is_empty()),
            apple_team_id: env::var("APPLE_TEAM_ID").ok().filter(|s| !s.is_empty()),
            apple_key_id: env::var("APPLE_KEY_ID").ok().filter(|s| !s.is_empty()),
            apple_private_key_path: env::var("APPLE_PRIVATE_KEY_PATH")
                .ok()
                .filter(|s| !s.is_empty()),

            smtp_host: env::var("SMTP_HOST").ok(),
            smtp_port: env::var("SMTP_PORT").ok().and_then(|v| v.parse().ok()),
            smtp_username: env::var("SMTP_USERNAME").ok(),
            smtp_password: env::var("SMTP_PASSWORD").ok(),
            smtp_from_address: env::var("SMTP_FROM_ADDRESS").ok(),

            encryption_key: env::var("ENCRYPTION_KEY").ok().filter(|s| !s.is_empty()),
            encryption_key_previous: env::var("ENCRYPTION_KEY_PREVIOUS")
                .ok()
                .filter(|s| !s.is_empty()),

            rate_limit_per_second: env::var("RATE_LIMIT_PER_SECOND")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            rate_limit_burst: env::var("RATE_LIMIT_BURST")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),

            sa_token_ttl_secs: env::var("SA_TOKEN_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),

            cookie_domain: env::var("COOKIE_DOMAIN").ok().filter(|s| !s.is_empty()),

            telegram_bot_token: env::var("TELEGRAM_BOT_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
            telegram_webhook_secret: env::var("TELEGRAM_WEBHOOK_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            telegram_webhook_url: env::var("TELEGRAM_WEBHOOK_URL")
                .ok()
                .filter(|s| !s.is_empty()),

            telegram_bot_username: env::var("TELEGRAM_BOT_USERNAME")
                .ok()
                .filter(|s| !s.is_empty()),

            approval_expiry_interval_secs: env::var("APPROVAL_EXPIRY_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),

            fcm_service_account_path: env::var("FCM_SERVICE_ACCOUNT_PATH")
                .ok()
                .filter(|s| !s.is_empty()),
            fcm_project_id: None, // derived from service account JSON at startup

            apns_key_path: env::var("APNS_KEY_PATH").ok().filter(|s| !s.is_empty()),
            apns_key_id: env::var("APNS_KEY_ID").ok().filter(|s| !s.is_empty()),
            apns_team_id: env::var("APNS_TEAM_ID").ok().filter(|s| !s.is_empty()),
            apns_topic: env::var("APNS_TOPIC").ok().filter(|s| !s.is_empty()),
            apns_sandbox: env::var("APNS_SANDBOX")
                .ok()
                .map(|v| v == "true" || v == "1")
                .unwrap_or(is_dev),

            key_provider: env::var("KEY_PROVIDER").unwrap_or_else(|_| "local".to_string()),

            aws_kms_key_arn: env::var("AWS_KMS_KEY_ARN").ok().filter(|s| !s.is_empty()),
            aws_kms_key_arn_previous: env::var("AWS_KMS_KEY_ARN_PREVIOUS")
                .ok()
                .filter(|s| !s.is_empty()),
            gcp_kms_key_name: env::var("GCP_KMS_KEY_NAME").ok().filter(|s| !s.is_empty()),
            gcp_kms_key_name_previous: env::var("GCP_KMS_KEY_NAME_PREVIOUS")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }

    /// Returns true if running in development mode.
    pub fn is_development(&self) -> bool {
        self.environment == "development" || self.environment == "dev"
    }

    /// Returns true if running in production mode.
    pub fn is_production(&self) -> bool {
        self.environment == "production"
    }

    /// Validate the local encryption key at startup.
    /// Panics if the key is missing, invalid, all-zeros, or the wrong length.
    pub fn validate_encryption_key(&self) {
        let encryption_key = self
            .encryption_key
            .as_ref()
            .expect("ENCRYPTION_KEY must be set when KEY_PROVIDER=local");

        if encryption_key.len() != 64 {
            panic!(
                "ENCRYPTION_KEY must be exactly 64 hex characters (32 bytes), got {} characters",
                encryption_key.len()
            );
        }

        let key_bytes =
            hex::decode(encryption_key).expect("ENCRYPTION_KEY is not valid hexadecimal");

        if key_bytes.len() != 32 {
            panic!("ENCRYPTION_KEY must decode to exactly 32 bytes");
        }

        // Reject all-zeros key (likely copied from .env.example)
        if key_bytes.iter().all(|&b| b == 0) {
            panic!(
                "ENCRYPTION_KEY is all zeros. This is insecure. \
                 Generate a proper key with: openssl rand -hex 32"
            );
        }

        // Validate previous key if present
        if let Some(ref prev_key) = self.encryption_key_previous {
            if prev_key.len() != 64 {
                panic!(
                    "ENCRYPTION_KEY_PREVIOUS must be exactly 64 hex characters (32 bytes), got {} characters",
                    prev_key.len()
                );
            }

            let prev_bytes =
                hex::decode(prev_key).expect("ENCRYPTION_KEY_PREVIOUS is not valid hexadecimal");

            if prev_bytes.len() != 32 {
                panic!("ENCRYPTION_KEY_PREVIOUS must decode to exactly 32 bytes");
            }

            if prev_bytes.iter().all(|&b| b == 0) {
                panic!(
                    "ENCRYPTION_KEY_PREVIOUS is all zeros. This is insecure. \
                     Generate a proper key with: openssl rand -hex 32"
                );
            }

            if prev_key == encryption_key {
                tracing::warn!(
                    "ENCRYPTION_KEY_PREVIOUS is the same as ENCRYPTION_KEY. \
                     This is valid but means no rotation is in progress."
                );
            }
        }
    }

    /// Validate the configured key provider at startup.
    /// Panics if an unsupported provider is specified.
    pub fn validate_key_provider(&self) {
        match self.key_provider.as_str() {
            "local" => self.validate_encryption_key(),
            #[cfg(feature = "aws-kms")]
            "aws-kms" => {
                self.aws_kms_key_arn.as_ref().unwrap_or_else(|| {
                    panic!("AWS_KMS_KEY_ARN must be set when KEY_PROVIDER=aws-kms")
                });
                // ENCRYPTION_KEY is optional (for migration fallback)
                if self.encryption_key.is_some() {
                    self.validate_encryption_key();
                }
            }
            #[cfg(feature = "gcp-kms")]
            "gcp-kms" => {
                self.gcp_kms_key_name.as_ref().unwrap_or_else(|| {
                    panic!("GCP_KMS_KEY_NAME must be set when KEY_PROVIDER=gcp-kms")
                });
                if self.encryption_key.is_some() {
                    self.validate_encryption_key();
                }
            }
            other => {
                #[allow(unused_mut)]
                let mut supported = vec!["local"];
                #[cfg(feature = "aws-kms")]
                supported.push("aws-kms");
                #[cfg(feature = "gcp-kms")]
                supported.push("gcp-kms");
                panic!(
                    "Unsupported KEY_PROVIDER: {other}. Supported providers: {}",
                    supported.join(", ")
                );
            }
        }
    }

    /// Log a warning if the OIDC issuer is not a URL.
    /// The OIDC spec requires the issuer to be an https:// URL
    /// (http:// is acceptable for localhost development).
    pub fn warn_if_non_url_issuer(&self) {
        if !self.jwt_issuer.starts_with("http://") && !self.jwt_issuer.starts_with("https://") {
            tracing::warn!(
                issuer = %self.jwt_issuer,
                "JWT_ISSUER is not a URL. OIDC spec requires the issuer to be an https:// URL \
                 (http:// is acceptable for localhost development). Consider removing JWT_ISSUER \
                 to use BASE_URL as the default, or set it to your public URL."
            );
        }
    }

    /// Returns true if the Secure cookie flag should be set.
    /// Disabled for localhost HTTP development.
    pub fn use_secure_cookies(&self) -> bool {
        !self.base_url.starts_with("http://localhost")
            && !self.base_url.starts_with("http://127.0.0.1")
    }

    /// Returns the configured cookie domain, if any.
    pub fn cookie_domain(&self) -> Option<&str> {
        self.cookie_domain.as_deref()
    }

    /// Returns true if all Apple Sign In credentials are configured.
    pub fn apple_configured(&self) -> bool {
        self.apple_client_id.is_some()
            && self.apple_team_id.is_some()
            && self.apple_key_id.is_some()
            && self.apple_private_key_path.is_some()
    }

    /// Validate and initialize push notification config at startup.
    /// Reads the FCM service account JSON to extract `project_id`.
    /// Verifies APNs key and required companion fields.
    pub fn validate_push_config(&mut self) {
        // FCM validation
        if let Some(path) = &self.fcm_service_account_path {
            let content = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("Failed to read FCM service account at {path}: {e}"));
            let json: serde_json::Value = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("Invalid JSON in FCM service account at {path}: {e}"));

            let project_id = json
                .get("project_id")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("FCM service account JSON missing 'project_id' field"));

            // Verify required fields exist
            json.get("client_email")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| panic!("FCM service account JSON missing 'client_email' field"));

            json.get("private_key")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| panic!("FCM service account JSON missing 'private_key' field"));

            self.fcm_project_id = Some(project_id.to_string());
            tracing::info!(
                project_id = %project_id,
                "FCM push notifications enabled"
            );
        }

        // APNs validation
        if let Some(path) = &self.apns_key_path {
            std::fs::metadata(path)
                .unwrap_or_else(|e| panic!("APNs key file not readable at {path}: {e}"));

            if self.apns_key_id.is_none() {
                panic!("APNS_KEY_ID is required when APNS_KEY_PATH is set");
            }
            if self.apns_team_id.is_none() {
                panic!("APNS_TEAM_ID is required when APNS_KEY_PATH is set");
            }

            let team_id = self.apns_team_id.as_deref().unwrap();
            let sandbox_label = if self.apns_sandbox {
                "sandbox"
            } else {
                "production"
            };
            tracing::info!(
                team_id = %team_id,
                environment = %sandbox_label,
                "APNs push notifications enabled"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal AppConfig for testing pure methods.
    fn make_config(base_url: &str, environment: &str, encryption_key: &str) -> AppConfig {
        AppConfig {
            port: 3001,
            base_url: base_url.to_string(),
            frontend_url: "http://localhost:3000".to_string(),
            database_url: "mongodb://localhost:27017/nyxid".to_string(),
            database_max_connections: 10,
            environment: environment.to_string(),
            jwt_private_key_path: "keys/private.pem".to_string(),
            jwt_public_key_path: "keys/public.pem".to_string(),
            jwt_issuer: base_url.to_string(),
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
            encryption_key: Some(encryption_key.to_string()),
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
        }
    }

    #[test]
    fn is_development_true() {
        let cfg = make_config(
            "http://localhost:3001",
            "development",
            "aa".repeat(32).as_str(),
        );
        assert!(cfg.is_development());
        let cfg2 = make_config("http://localhost:3001", "dev", "aa".repeat(32).as_str());
        assert!(cfg2.is_development());
    }

    #[test]
    fn is_development_false_for_production() {
        let cfg = make_config(
            "https://auth.example.com",
            "production",
            "aa".repeat(32).as_str(),
        );
        assert!(!cfg.is_development());
    }

    #[test]
    fn is_production_true() {
        let cfg = make_config(
            "https://auth.example.com",
            "production",
            "aa".repeat(32).as_str(),
        );
        assert!(cfg.is_production());
    }

    #[test]
    fn is_production_false() {
        let cfg = make_config(
            "http://localhost:3001",
            "development",
            "aa".repeat(32).as_str(),
        );
        assert!(!cfg.is_production());
    }

    #[test]
    fn secure_cookies_for_https() {
        let cfg = make_config(
            "https://auth.example.com",
            "production",
            "aa".repeat(32).as_str(),
        );
        assert!(cfg.use_secure_cookies());
    }

    #[test]
    fn no_secure_cookies_for_localhost() {
        let cfg = make_config(
            "http://localhost:3001",
            "development",
            "aa".repeat(32).as_str(),
        );
        assert!(!cfg.use_secure_cookies());
    }

    #[test]
    fn no_secure_cookies_for_127_0_0_1() {
        let cfg = make_config(
            "http://127.0.0.1:3001",
            "development",
            "aa".repeat(32).as_str(),
        );
        assert!(!cfg.use_secure_cookies());
    }

    #[test]
    fn validate_encryption_key_valid() {
        // 64 hex chars = 32 bytes, not all zeros
        let key = "ab".repeat(32);
        let cfg = make_config("http://localhost:3001", "dev", &key);
        cfg.validate_encryption_key(); // should not panic
    }

    #[test]
    #[should_panic(expected = "ENCRYPTION_KEY must be set when KEY_PROVIDER=local")]
    fn validate_encryption_key_missing() {
        let mut cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        cfg.encryption_key = None;
        cfg.validate_encryption_key();
    }

    #[test]
    #[should_panic(expected = "must be exactly 64 hex characters")]
    fn validate_encryption_key_too_short() {
        let cfg = make_config("http://localhost:3001", "dev", "abcd");
        cfg.validate_encryption_key();
    }

    #[test]
    #[should_panic(expected = "not valid hexadecimal")]
    fn validate_encryption_key_not_hex() {
        let key = "zz".repeat(32); // not valid hex
        let cfg = make_config("http://localhost:3001", "dev", &key);
        cfg.validate_encryption_key();
    }

    #[test]
    #[should_panic(expected = "all zeros")]
    fn validate_encryption_key_all_zeros() {
        let key = "00".repeat(32);
        let cfg = make_config("http://localhost:3001", "dev", &key);
        cfg.validate_encryption_key();
    }

    #[test]
    fn validate_encryption_key_with_valid_previous() {
        let key = "ab".repeat(32);
        let mut cfg = make_config("http://localhost:3001", "dev", &key);
        cfg.encryption_key_previous = Some("cd".repeat(32));
        cfg.validate_encryption_key(); // should not panic
    }

    #[test]
    #[should_panic(expected = "ENCRYPTION_KEY_PREVIOUS must be exactly 64 hex characters")]
    fn validate_previous_key_too_short() {
        let key = "ab".repeat(32);
        let mut cfg = make_config("http://localhost:3001", "dev", &key);
        cfg.encryption_key_previous = Some("abcd".to_string());
        cfg.validate_encryption_key();
    }

    #[test]
    #[should_panic(expected = "ENCRYPTION_KEY_PREVIOUS is not valid hexadecimal")]
    fn validate_previous_key_not_hex() {
        let key = "ab".repeat(32);
        let mut cfg = make_config("http://localhost:3001", "dev", &key);
        cfg.encryption_key_previous = Some("zz".repeat(32));
        cfg.validate_encryption_key();
    }

    #[test]
    #[should_panic(expected = "ENCRYPTION_KEY_PREVIOUS is all zeros")]
    fn validate_previous_key_all_zeros() {
        let key = "ab".repeat(32);
        let mut cfg = make_config("http://localhost:3001", "dev", &key);
        cfg.encryption_key_previous = Some("00".repeat(32));
        cfg.validate_encryption_key();
    }
}
