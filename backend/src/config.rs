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
    /// Additional CORS allowed origins (comma-separated, e.g. "http://localhost:5847,http://localhost:3000")
    pub cors_allowed_origins: Vec<String>,
    /// Additional origins trusted for browser CSRF (comma-separated).
    /// These are merged with `frontend_url` + `base_url` when checking the
    /// `Origin` / `Referer` header on cookie-authenticated state-changing
    /// requests. Keep this strictly narrower than `CORS_ALLOWED_ORIGINS`:
    /// only include origins that legitimately perform cookie-authenticated
    /// state changes. Bearer / API-key callers never need to be listed here.
    pub csrf_trusted_origins: Vec<String>,
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
    /// Relay reply token TTL in seconds (default: 1800 = 30 min)
    pub jwt_relay_reply_ttl_secs: i64,
    /// Relay callback token TTL in seconds (default: 300 = 5 min)
    pub jwt_relay_callback_ttl_secs: i64,
    /// Refresh token TTL in seconds (default: 604800 = 7 days)
    pub jwt_refresh_ttl_secs: i64,

    /// Host-configured URL for the independently published release-integrity
    /// manifest. Unset/empty disables admin verification and browser
    /// credential accept fails closed unless the owning org opts out.
    pub release_integrity_manifest_url: Option<String>,
    /// Local packaged standalone credential-accept build directory.
    pub credential_accept_dist_dir: String,

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
    /// Allowlist of reverse-proxy IPs whose `X-Forwarded-For` /
    /// `X-Real-IP` headers may be trusted for rate-limit keying.
    /// When the TCP peer is not in this list, forwarded headers are
    /// ignored and the peer IP is used instead — so a direct-exposure
    /// deployment can't be tricked into per-header buckets.
    ///
    /// Parsed from the comma-separated `TRUSTED_PROXY_IPS` env var.
    /// Empty (the default) is the strict mode: nothing trusted.
    pub trusted_proxy_ips: Vec<std::net::IpAddr>,

    /// Optional reverse-proxy-forwarded client certificate header used for
    /// RFC 8705 certificate-bound broker access tokens. Unset/empty disables
    /// mTLS binding even if a client sends `X-Client-Cert` directly.
    pub mtls_client_cert_header: Option<String>,

    /// Explicit HMAC key (64 hex chars = 32 bytes) used to derive
    /// `CliPairing.code_hash`. When unset, the backend derives the
    /// key from `ENCRYPTION_KEY` (if configured) or from the JWT
    /// private key PEM — see `derive_cli_pairing_hmac_key` in
    /// `main.rs` for the full priority chain. Set explicitly only
    /// when you want to rotate the pairing HMAC independently of
    /// both `ENCRYPTION_KEY` and the JWT signing key. Threaded
    /// through `AppConfig` (rather than read via `std::env::var`
    /// at the call site) so ops can introspect the resolved value
    /// via the same path as every other env-backed setting.
    pub cli_pairing_hmac_key: Option<String>,

    /// Service account token TTL in seconds (default: 3600 = 1 hour)
    pub sa_token_ttl_secs: i64,

    /// Telemetry DSN (e.g. PostHog project API key). When unset (default)
    /// telemetry is hard-off: `TelemetryClient::from_config` returns
    /// `None` and no events are captured.
    pub telemetry_dsn: Option<String>,
    /// Telemetry ingest host (defaults to EU PostHog if unset).
    pub telemetry_host: Option<String>,
    /// When true AND `telemetry_dsn` is empty, fall back to the
    /// compiled-in public share-back DSN. Self-hoster opt-in knob.
    pub share_analytics: bool,

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

    /// Interval in seconds between proactive OAuth token-refresh sweeps
    /// (default: 600 = 10 min). Set to 0 to disable the sweep entirely
    /// (lazy proxy-time refresh still applies). The sweep refreshes
    /// multi-connection OAuth access tokens that expire within
    /// `oauth_refresh_sweep_window_secs` so a token stays warm even for
    /// services that aren't proxied frequently.
    pub oauth_refresh_sweep_interval_secs: u64,

    /// How far ahead (seconds) the proactive refresh sweep looks for
    /// expiring OAuth access tokens (default: 900 = 15 min). Must be
    /// larger than the proxy-time 5-min buffer so the sweep wins the
    /// race for idle services.
    pub oauth_refresh_sweep_window_secs: i64,

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

    // Node Proxy
    /// Heartbeat ping interval in seconds (default: 30)
    pub node_heartbeat_interval_secs: u64,
    /// Mark node offline after this many seconds without heartbeat (default: 90)
    pub node_heartbeat_timeout_secs: u64,
    /// Timeout for proxy requests routed through nodes (default: 30)
    pub node_proxy_timeout_secs: u64,
    /// Registration token validity in seconds (default: 3600 = 1 hour)
    pub node_registration_token_ttl_secs: i64,
    /// Pending node credential metadata TTL in seconds (default: 86400 = 24 hours)
    pub node_pending_credential_ttl_secs: i64,
    /// Maximum nodes per user (default: 10)
    pub node_max_per_user: u32,
    /// Maximum concurrent WebSocket connections (default: 100)
    pub node_max_ws_connections: usize,
    /// Maximum duration for streaming proxy responses in seconds (default: 300)
    pub node_max_stream_duration_secs: u64,
    /// Enable HMAC request signing for node proxy requests (default: true)
    pub node_hmac_signing_enabled: bool,

    // Proxy streaming
    /// Maximum request body size for proxy routes in bytes (default: 100 MB)
    pub proxy_max_body_size: usize,
    /// Idle timeout for proxy streaming responses in seconds (default: 60).
    /// Stream is terminated if no data chunk arrives within this duration.
    pub proxy_stream_idle_timeout_secs: u64,
    /// Maximum concurrent SSH WebSocket tunnel sessions per user (default: 4)
    pub ssh_max_sessions_per_user: usize,
    /// Timeout for connecting to a downstream SSH target in seconds (default: 10)
    pub ssh_connect_timeout_secs: u64,
    /// Maximum duration for an SSH tunnel session in seconds (default: 3600)
    pub ssh_max_tunnel_duration_secs: u64,
    /// Maximum concurrent WebSocket passthrough connections (default: 200)
    pub ws_passthrough_max_connections: usize,
    /// Maximum request body size for anonymous public proxy/MCP routes.
    pub public_proxy_max_body_size: usize,
    /// Per-IP anonymous public proxy request cap per minute.
    pub public_proxy_rate_limit_per_minute: u32,
    /// Per-IP anonymous public MCP request cap per minute.
    pub public_mcp_rate_limit_per_minute: u32,

    // Channel Bot Relay
    /// Timeout in seconds for delivering inbound messages to agent callback URLs (default: 30)
    pub channel_relay_callback_timeout_secs: u32,
    /// Maximum number of channel bots a single user can register (default: 5)
    pub channel_relay_max_bots_per_user: u32,
    /// TTL in days for channel messages before automatic expiry (default: 30)
    pub channel_relay_message_ttl_days: u32,
    /// Per-message edit rate limit for channel relay replies (default: 10/s).
    pub channel_relay_edit_rate_limit_per_second: u32,
    /// Burst capacity for per-message edit rate limiting (default: 20).
    pub channel_relay_edit_rate_limit_burst: u32,

    // HTTP Event Gateway (NyxID#221 / ADR-013)
    /// Per-channel event rate limit (events per second, default 100).
    pub channel_event_rate_limit_per_second: u32,
    /// Per-channel event rate limit burst capacity (default 200).
    pub channel_event_rate_limit_burst: u32,
    /// Event dedup LRU cache capacity (default 10_000).
    pub channel_event_dedup_capacity: usize,
    /// Event dedup TTL in seconds (default 300 = 5 minutes).
    pub channel_event_dedup_ttl_secs: u64,

    /// Response-cache TTL (seconds) for the `aws_sigv4` proxy auth
    /// method. AWS Cost Explorer charges $0.01 per paginated request,
    /// so identical proxy requests in a short window get served from
    /// cache. **Defaults to 0 (disabled).** Operators should review
    /// the cache scoping (per-credential + per-operation-header keying
    /// via `CloudResponseCache::key`) and the bounds below before
    /// enabling. See NyxID#716 + Codex review REC 11.
    pub cloud_response_cache_ttl_secs: u64,
    /// Maximum bytes for a single cacheable response. Larger responses
    /// are forwarded uncached. Default 1 MiB. Override via
    /// `CLOUD_RESPONSE_CACHE_MAX_ENTRY_BYTES`.
    pub cloud_response_cache_max_entry_bytes: usize,
    /// Maximum number of cached entries. LRU-ish eviction by
    /// insertion timestamp when full. Override via
    /// `CLOUD_RESPONSE_CACHE_MAX_ENTRIES`.
    pub cloud_response_cache_max_entries: usize,

    // Registration gate
    /// When `true` (default), new-user registration requires a valid invite
    /// code and first-time social sign-ups are rejected. Set
    /// `INVITE_CODE_REQUIRED=false` to open public registration — used once
    /// the product launches publicly. See issue #179.
    pub invite_code_required: bool,

    /// When `true`, email/password auth UI is shown on `/login` and
    /// `/register`, and `POST /api/v1/auth/register` accepts new accounts.
    /// Defaults to `false` — the self-host quickstart in `README.md` is the
    /// only path that opts in by writing `EMAIL_AUTH_ENABLED=true` to
    /// `.env.dev`. Production, stock local `cargo run`, and any other
    /// environment without the flag get SSO-only signup. The login endpoint
    /// is NOT gated — existing users can still authenticate via direct API
    /// call even when the UI is hidden.
    pub email_auth_enabled: bool,

    // Dev convenience
    /// When `true`, newly registered users are marked as email-verified
    /// immediately. Only intended for local development — defaults to `false`.
    pub auto_verify_email: bool,
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
            .field("jwt_relay_reply_ttl_secs", &self.jwt_relay_reply_ttl_secs)
            .field(
                "jwt_relay_callback_ttl_secs",
                &self.jwt_relay_callback_ttl_secs,
            )
            .field("jwt_refresh_ttl_secs", &self.jwt_refresh_ttl_secs)
            .field(
                "release_integrity_manifest_url",
                &self.release_integrity_manifest_url,
            )
            .field(
                "credential_accept_dist_dir",
                &self.credential_accept_dist_dir,
            )
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
            .field("trusted_proxy_ips", &self.trusted_proxy_ips)
            .field("mtls_client_cert_header", &self.mtls_client_cert_header)
            .field(
                "cli_pairing_hmac_key",
                if self.cli_pairing_hmac_key.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
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
            .field(
                "oauth_refresh_sweep_interval_secs",
                &self.oauth_refresh_sweep_interval_secs,
            )
            .field(
                "oauth_refresh_sweep_window_secs",
                &self.oauth_refresh_sweep_window_secs,
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
            .field(
                "node_heartbeat_interval_secs",
                &self.node_heartbeat_interval_secs,
            )
            .field(
                "node_heartbeat_timeout_secs",
                &self.node_heartbeat_timeout_secs,
            )
            .field("node_proxy_timeout_secs", &self.node_proxy_timeout_secs)
            .field(
                "node_registration_token_ttl_secs",
                &self.node_registration_token_ttl_secs,
            )
            .field(
                "node_pending_credential_ttl_secs",
                &self.node_pending_credential_ttl_secs,
            )
            .field("node_max_per_user", &self.node_max_per_user)
            .field("node_max_ws_connections", &self.node_max_ws_connections)
            .field(
                "node_max_stream_duration_secs",
                &self.node_max_stream_duration_secs,
            )
            .field("node_hmac_signing_enabled", &self.node_hmac_signing_enabled)
            .field("proxy_max_body_size", &self.proxy_max_body_size)
            .field(
                "proxy_stream_idle_timeout_secs",
                &self.proxy_stream_idle_timeout_secs,
            )
            .field("ssh_max_sessions_per_user", &self.ssh_max_sessions_per_user)
            .field("ssh_connect_timeout_secs", &self.ssh_connect_timeout_secs)
            .field(
                "ssh_max_tunnel_duration_secs",
                &self.ssh_max_tunnel_duration_secs,
            )
            .field(
                "ws_passthrough_max_connections",
                &self.ws_passthrough_max_connections,
            )
            .field(
                "public_proxy_max_body_size",
                &self.public_proxy_max_body_size,
            )
            .field(
                "public_proxy_rate_limit_per_minute",
                &self.public_proxy_rate_limit_per_minute,
            )
            .field(
                "public_mcp_rate_limit_per_minute",
                &self.public_mcp_rate_limit_per_minute,
            )
            .field(
                "channel_relay_callback_timeout_secs",
                &self.channel_relay_callback_timeout_secs,
            )
            .field(
                "channel_relay_max_bots_per_user",
                &self.channel_relay_max_bots_per_user,
            )
            .field(
                "channel_relay_message_ttl_days",
                &self.channel_relay_message_ttl_days,
            )
            .field(
                "channel_relay_edit_rate_limit_per_second",
                &self.channel_relay_edit_rate_limit_per_second,
            )
            .field(
                "channel_relay_edit_rate_limit_burst",
                &self.channel_relay_edit_rate_limit_burst,
            )
            .field(
                "channel_event_rate_limit_per_second",
                &self.channel_event_rate_limit_per_second,
            )
            .field(
                "channel_event_rate_limit_burst",
                &self.channel_event_rate_limit_burst,
            )
            .field(
                "channel_event_dedup_capacity",
                &self.channel_event_dedup_capacity,
            )
            .field(
                "channel_event_dedup_ttl_secs",
                &self.channel_event_dedup_ttl_secs,
            )
            .finish()
    }
}

/// Parse a boolean env var with a default value.
fn parse_bool_env(name: &str, default: bool) -> bool {
    match env::var(name)
        .ok()
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => default,
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on"),
    }
}

/// Parse the comma-separated `TRUSTED_PROXY_IPS` env var into a Vec of
/// IP addresses. Entries that fail to parse are dropped with a warning
/// so a typo can't silently extend trust to unparsed input; startup
/// still succeeds because direct-exposure deployments are the common
/// case and don't need this set.
fn parse_trusted_proxy_ips(raw: Option<String>) -> Vec<std::net::IpAddr> {
    let Some(raw) = raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    let mut ips = Vec::new();
    for entry in raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match entry.parse::<std::net::IpAddr>() {
            Ok(ip) => ips.push(ip),
            Err(err) => tracing::warn!(
                entry = %entry,
                error = %err,
                "TRUSTED_PROXY_IPS entry is not a valid IP address; dropping",
            ),
        }
    }
    ips
}

/// Parse the `INVITE_CODE_REQUIRED` env var.
///
/// Defaults to `true` (invite codes required) when the variable is unset or
/// empty. Accepts the usual boolean-ish spellings case-insensitively.
fn parse_invite_code_required(raw: Option<String>) -> bool {
    match raw.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => true,
        Some(v) => !matches!(
            v.to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        ),
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
            cors_allowed_origins: env::var("CORS_ALLOWED_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            csrf_trusted_origins: env::var("CSRF_TRUSTED_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
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
            jwt_relay_reply_ttl_secs: env::var("JWT_RELAY_REPLY_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1800),
            jwt_relay_callback_ttl_secs: env::var("JWT_RELAY_CALLBACK_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            jwt_refresh_ttl_secs: env::var("JWT_REFRESH_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(604800),
            release_integrity_manifest_url: env::var("RELEASE_INTEGRITY_MANIFEST_URL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            credential_accept_dist_dir: env::var("CREDENTIAL_ACCEPT_DIST_DIR")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "frontend/dist/credential-accept".to_string()),

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
            trusted_proxy_ips: parse_trusted_proxy_ips(env::var("TRUSTED_PROXY_IPS").ok()),
            mtls_client_cert_header: env::var("MTLS_CLIENT_CERT_HEADER")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            cli_pairing_hmac_key: env::var("CLI_PAIRING_HMAC_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty()),

            sa_token_ttl_secs: env::var("SA_TOKEN_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),

            telemetry_dsn: env::var("NYXID_TELEMETRY_DSN")
                .ok()
                .filter(|s| !s.is_empty()),
            telemetry_host: env::var("NYXID_TELEMETRY_HOST")
                .ok()
                .filter(|s| !s.is_empty()),
            share_analytics: parse_bool_env("NYXID_SHARE_ANALYTICS", false),

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

            oauth_refresh_sweep_interval_secs: env::var("OAUTH_REFRESH_SWEEP_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),

            oauth_refresh_sweep_window_secs: env::var("OAUTH_REFRESH_SWEEP_WINDOW_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),

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

            node_heartbeat_interval_secs: env::var("NODE_HEARTBEAT_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            node_heartbeat_timeout_secs: env::var("NODE_HEARTBEAT_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(90),
            node_proxy_timeout_secs: env::var("NODE_PROXY_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            node_registration_token_ttl_secs: env::var("NODE_REGISTRATION_TOKEN_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            node_pending_credential_ttl_secs: env::var("NODE_PENDING_CREDENTIAL_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(86_400),
            node_max_per_user: env::var("NODE_MAX_PER_USER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            node_max_ws_connections: env::var("NODE_MAX_WS_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
            node_max_stream_duration_secs: env::var("NODE_MAX_STREAM_DURATION_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            node_hmac_signing_enabled: env::var("NODE_HMAC_SIGNING_ENABLED")
                .ok()
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
            proxy_max_body_size: env::var("PROXY_MAX_BODY_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100 * 1024 * 1024),
            proxy_stream_idle_timeout_secs: env::var("PROXY_STREAM_IDLE_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            ssh_max_sessions_per_user: env::var("SSH_MAX_SESSIONS_PER_USER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            ssh_connect_timeout_secs: env::var("SSH_CONNECT_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            ssh_max_tunnel_duration_secs: env::var("SSH_MAX_TUNNEL_DURATION_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            ws_passthrough_max_connections: env::var("WS_PASSTHROUGH_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(200),
            public_proxy_max_body_size: env::var("PUBLIC_PROXY_MAX_BODY_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(
                    crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_PROXY_MAX_BODY_SIZE,
                ),
            public_proxy_rate_limit_per_minute: env::var("PUBLIC_PROXY_RATE_LIMIT_PER_MINUTE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(
                    crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_PROXY_RATE_LIMIT_PER_MINUTE,
                ),
            public_mcp_rate_limit_per_minute: env::var("PUBLIC_MCP_RATE_LIMIT_PER_MINUTE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(
                    crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_MCP_RATE_LIMIT_PER_MINUTE,
                ),
            channel_relay_callback_timeout_secs: env::var("CHANNEL_RELAY_CALLBACK_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            channel_relay_max_bots_per_user: env::var("CHANNEL_RELAY_MAX_BOTS_PER_USER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            channel_relay_message_ttl_days: env::var("CHANNEL_RELAY_MESSAGE_TTL_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            channel_relay_edit_rate_limit_per_second: env::var(
                "CHANNEL_RELAY_EDIT_RATE_LIMIT_PER_SECOND",
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10),
            channel_relay_edit_rate_limit_burst: env::var("CHANNEL_RELAY_EDIT_RATE_LIMIT_BURST")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            channel_event_rate_limit_per_second: env::var("CHANNEL_EVENT_RATE_LIMIT_PER_SECOND")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
            channel_event_rate_limit_burst: env::var("CHANNEL_EVENT_RATE_LIMIT_BURST")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(200),
            channel_event_dedup_capacity: env::var("CHANNEL_EVENT_DEDUP_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                // Default sized to honor the 5-min TTL window under the
                // default rate limit: 100 events/s × 300s = 30,000 entries
                // for a single saturated channel. 32_768 leaves headroom
                // and is a power of two. Operators with many concurrent
                // high-throughput channels should tune this up.
                .unwrap_or(32_768),
            channel_event_dedup_ttl_secs: env::var("CHANNEL_EVENT_DEDUP_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            cloud_response_cache_ttl_secs: env::var("CLOUD_RESPONSE_CACHE_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            cloud_response_cache_max_entry_bytes: env::var("CLOUD_RESPONSE_CACHE_MAX_ENTRY_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::services::cloud_response_cache::DEFAULT_MAX_ENTRY_BYTES),
            cloud_response_cache_max_entries: env::var("CLOUD_RESPONSE_CACHE_MAX_ENTRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::services::cloud_response_cache::DEFAULT_MAX_ENTRIES),

            invite_code_required: parse_invite_code_required(env::var("INVITE_CODE_REQUIRED").ok()),
            email_auth_enabled: parse_bool_env("EMAIL_AUTH_ENABLED", false),
            auto_verify_email: parse_bool_env("AUTO_VERIFY_EMAIL", false),
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
                #[allow(unused_mut, clippy::useless_vec)]
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

    pub fn validate_ssh_runtime_config(&self) {
        if self.ssh_max_sessions_per_user == 0 {
            panic!("SSH_MAX_SESSIONS_PER_USER must be greater than 0");
        }
        if self.ssh_connect_timeout_secs == 0 {
            panic!("SSH_CONNECT_TIMEOUT_SECS must be greater than 0");
        }
        if self.ssh_max_tunnel_duration_secs == 0 {
            panic!("SSH_MAX_TUNNEL_DURATION_SECS must be greater than 0");
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
            cors_allowed_origins: vec![],
            csrf_trusted_origins: vec![],
            database_url: "mongodb://localhost:27017/nyxid".to_string(),
            database_max_connections: 10,
            environment: environment.to_string(),
            jwt_private_key_path: "keys/private.pem".to_string(),
            jwt_public_key_path: "keys/public.pem".to_string(),
            jwt_issuer: base_url.to_string(),
            jwt_access_ttl_secs: 900,
            jwt_relay_reply_ttl_secs: 1800,
            jwt_relay_callback_ttl_secs: 300,
            jwt_refresh_ttl_secs: 604800,
            release_integrity_manifest_url: None,
            credential_accept_dist_dir: "frontend/dist/credential-accept".to_string(),
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
            trusted_proxy_ips: vec![],
            mtls_client_cert_header: None,
            cli_pairing_hmac_key: None,
            sa_token_ttl_secs: 3600,
            telemetry_dsn: None,
            telemetry_host: None,
            share_analytics: false,
            cookie_domain: None,
            telegram_bot_token: None,
            telegram_webhook_secret: None,
            telegram_webhook_url: None,
            telegram_bot_username: None,
            approval_expiry_interval_secs: 5,
            oauth_refresh_sweep_interval_secs: 600,
            oauth_refresh_sweep_window_secs: 900,
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
            node_heartbeat_interval_secs: 30,
            node_heartbeat_timeout_secs: 90,
            node_proxy_timeout_secs: 30,
            node_registration_token_ttl_secs: 3600,
            node_pending_credential_ttl_secs: 86_400,
            node_max_per_user: 10,
            node_max_ws_connections: 100,
            node_max_stream_duration_secs: 300,
            node_hmac_signing_enabled: true,
            proxy_max_body_size: 100 * 1024 * 1024,
            proxy_stream_idle_timeout_secs: 60,
            ssh_max_sessions_per_user: 4,
            ssh_connect_timeout_secs: 10,
            ssh_max_tunnel_duration_secs: 3600,
            ws_passthrough_max_connections: 200,
            public_proxy_max_body_size:
                crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_PROXY_MAX_BODY_SIZE,
            public_proxy_rate_limit_per_minute:
                crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_PROXY_RATE_LIMIT_PER_MINUTE,
            public_mcp_rate_limit_per_minute:
                crate::services::anonymous_endpoint_service::DEFAULT_PUBLIC_MCP_RATE_LIMIT_PER_MINUTE,
            channel_relay_callback_timeout_secs: 30,
            channel_relay_max_bots_per_user: 5,
            channel_relay_message_ttl_days: 30,
            channel_relay_edit_rate_limit_per_second: 10,
            channel_relay_edit_rate_limit_burst: 20,
            channel_event_rate_limit_per_second: 100,
            channel_event_rate_limit_burst: 200,
            channel_event_dedup_capacity: 32_768,
            channel_event_dedup_ttl_secs: 300,
            cloud_response_cache_ttl_secs: 0,
            cloud_response_cache_max_entry_bytes:
                crate::services::cloud_response_cache::DEFAULT_MAX_ENTRY_BYTES,
            cloud_response_cache_max_entries:
                crate::services::cloud_response_cache::DEFAULT_MAX_ENTRIES,
            invite_code_required: true,
            email_auth_enabled: false,
            auto_verify_email: false,
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

    #[test]
    #[should_panic(expected = "SSH_MAX_SESSIONS_PER_USER must be greater than 0")]
    fn validate_ssh_runtime_config_rejects_zero_max_sessions() {
        let mut cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        cfg.ssh_max_sessions_per_user = 0;
        cfg.validate_ssh_runtime_config();
    }

    #[test]
    #[should_panic(expected = "SSH_CONNECT_TIMEOUT_SECS must be greater than 0")]
    fn validate_ssh_runtime_config_rejects_zero_connect_timeout() {
        let mut cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        cfg.ssh_connect_timeout_secs = 0;
        cfg.validate_ssh_runtime_config();
    }

    #[test]
    fn validate_ssh_runtime_config_accepts_valid_values() {
        let cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        cfg.validate_ssh_runtime_config();
    }

    #[test]
    fn trusted_proxy_ips_unset_defaults_to_empty() {
        assert!(parse_trusted_proxy_ips(None).is_empty());
        assert!(parse_trusted_proxy_ips(Some(String::new())).is_empty());
        assert!(parse_trusted_proxy_ips(Some("   ".to_string())).is_empty());
    }

    #[test]
    fn trusted_proxy_ips_parses_single_ipv4() {
        let parsed = parse_trusted_proxy_ips(Some("10.0.0.1".to_string()));
        assert_eq!(
            parsed,
            vec![std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1))]
        );
    }

    #[test]
    fn trusted_proxy_ips_parses_comma_separated_list() {
        let parsed = parse_trusted_proxy_ips(Some("10.0.0.1, 127.0.0.1 ,::1".to_string()));
        assert_eq!(
            parsed,
            vec![
                std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1)),
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            ]
        );
    }

    #[test]
    fn trusted_proxy_ips_drops_invalid_entries() {
        let parsed = parse_trusted_proxy_ips(Some("10.0.0.1, not-an-ip, 127.0.0.1".to_string()));
        assert_eq!(
            parsed,
            vec![
                std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1)),
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            ]
        );
    }

    #[test]
    fn invite_code_required_defaults_to_true_when_unset() {
        assert!(parse_invite_code_required(None));
        assert!(parse_invite_code_required(Some(String::new())));
        assert!(parse_invite_code_required(Some("   ".to_string())));
    }

    #[test]
    fn invite_code_required_false_for_falsy_values() {
        for v in ["false", "FALSE", "False", "0", "no", "NO", "off", "OFF"] {
            assert!(
                !parse_invite_code_required(Some(v.to_string())),
                "{v} should disable the gate"
            );
        }
    }

    #[test]
    fn invite_code_required_true_for_truthy_values() {
        for v in ["true", "TRUE", "1", "yes", "on", "anything-else"] {
            assert!(
                parse_invite_code_required(Some(v.to_string())),
                "{v} should leave the gate enabled"
            );
        }
    }

    #[test]
    fn parse_bool_env_defaults() {
        assert!(parse_bool_env("NONEXISTENT_VAR_XYZZY_12345", true));
        assert!(!parse_bool_env("NONEXISTENT_VAR_XYZZY_12345", false));
    }

    #[test]
    fn cookie_domain_returns_configured_value() {
        let mut cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        assert!(cfg.cookie_domain().is_none());
        cfg.cookie_domain = Some(".example.com".to_string());
        assert_eq!(cfg.cookie_domain(), Some(".example.com"));
    }

    #[test]
    fn apple_configured_requires_all_fields() {
        let mut cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        assert!(!cfg.apple_configured());
        cfg.apple_client_id = Some("id".to_string());
        cfg.apple_team_id = Some("team".to_string());
        cfg.apple_key_id = Some("key".to_string());
        assert!(!cfg.apple_configured());
        cfg.apple_private_key_path = Some("/path".to_string());
        assert!(cfg.apple_configured());
    }

    #[test]
    fn debug_redacts_secrets() {
        let cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        let debug = format!("{:?}", cfg);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(&"ab".repeat(32)));
    }

    #[test]
    #[should_panic(expected = "SSH_MAX_TUNNEL_DURATION_SECS must be greater than 0")]
    fn validate_ssh_runtime_config_rejects_zero_tunnel_duration() {
        let mut cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        cfg.ssh_max_tunnel_duration_secs = 0;
        cfg.validate_ssh_runtime_config();
    }

    #[test]
    fn validate_key_provider_local() {
        let cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        cfg.validate_key_provider();
    }

    #[test]
    #[should_panic(expected = "Unsupported KEY_PROVIDER")]
    fn validate_key_provider_unsupported() {
        let mut cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        cfg.key_provider = "invalid-provider".to_string();
        cfg.validate_key_provider();
    }

    #[test]
    fn warn_if_non_url_issuer_does_not_panic() {
        let mut cfg = make_config("http://localhost:3001", "dev", &"ab".repeat(32));
        cfg.jwt_issuer = "nyxid".to_string();
        cfg.warn_if_non_url_issuer();
        cfg.jwt_issuer = "https://auth.example.com".to_string();
        cfg.warn_if_non_url_issuer();
    }
}
