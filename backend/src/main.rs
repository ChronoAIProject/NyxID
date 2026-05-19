use axum::{extract::DefaultBodyLimit, extract::Extension, middleware as axum_mw};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod api_docs;
mod cleanup_cli;
mod config;
mod crypto;
mod db;
mod errors;
mod handlers;
mod login_cli;
mod models;
mod mw;
mod routes;
mod services;
mod ssh_cli;
mod telemetry;

#[cfg(test)]
mod test_utils;

use std::sync::Arc;

/// Install `aws_lc_rs` as the rustls process-wide crypto provider.
/// Called once at startup; no-op if a provider is already installed
/// (e.g. by a library that lost the race to ours).
///
/// Without this, rustls can panic on first TLS use when feature
/// unification leaves multiple crypto providers in the dependency graph.
fn install_rustls_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return;
    }
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        // Another thread already installed a provider in between our
        // `get_default` check and the install attempt. Either way, a
        // default is now installed.
        debug_assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}

use crate::db::DbHandle;
use config::AppConfig;
use crypto::aes::EncryptionKeys;
use crypto::jwks::JwksCache;
use crypto::jwt::JwtKeys;
use crypto::key_provider::KeyProvider;
use crypto::local_key_provider::LocalKeyProvider;
use models::mcp_session::McpSessionStore;

use services::dpop_jti_cache::{DPOP_JTI_CACHE_CAPACITY, DPOP_JTI_CACHE_TTL_SECS, DpopJtiCache};
use services::event_dedup_cache::EventDedupCache;
use services::node_ws_manager::NodeWsManager;
use services::provider_token_exchange_service::TokenExchangeCache;
use services::push_service::{ApnsAuth, FcmAuth};
use services::ssh_service::SshSessionManager;

/// Shared application state available to all handlers via Axum's State extractor.
#[derive(Clone)]
pub struct AppState {
    pub db: DbHandle,
    pub config: AppConfig,
    pub jwt_keys: JwtKeys,
    pub http_client: reqwest::Client,
    /// Pre-computed JWK for the JWKS endpoint
    pub jwk_json: serde_json::Value,
    /// Hybrid in-memory + MongoDB MCP session store
    pub mcp_sessions: Arc<McpSessionStore>,
    /// JWKS cache for verifying external provider ID tokens (Google)
    pub jwks_cache: Arc<JwksCache>,
    /// FCM push notification auth (None if not configured)
    pub fcm_auth: Option<Arc<FcmAuth>>,
    /// APNs push notification auth (None if not configured)
    pub apns_auth: Option<Arc<ApnsAuth>>,
    /// Versioned encryption keys for AES-256-GCM (current + optional previous for rotation)
    pub encryption_keys: Arc<EncryptionKeys>,
    /// WebSocket connection manager for credential nodes
    pub node_ws_manager: Arc<NodeWsManager>,
    /// Concurrent SSH tunnel session limiter
    pub ssh_session_manager: Arc<SshSessionManager>,
    /// Per-agent rate limiter keyed by API key ID
    pub per_agent_limiter: mw::rate_limit::SharedPerAgentRateLimiter,
    /// Per-IP rate limiter for `POST /cli-pairings/claim`. Tighter than
    /// the global rate limiter (5 attempts per 60s per IP) so brute
    /// forcing the 8-char pairing code is infeasible even from a
    /// compromised session. The key is the TCP peer IP by default;
    /// deployments that configure `TRUSTED_PROXY_IPS` get per-real-
    /// client keying by reading the forwarded headers from the listed
    /// proxies. Mobile users behind CGNAT may collide, but the floor
    /// is generous enough (1/12s avg) that legitimate retypes aren't
    /// blocked.
    pub cli_pairing_claim_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Server-side HMAC key used to derive `CliPairing.code_hash`.
    /// Lives in process memory only (never persisted), so a MongoDB
    /// snapshot alone doesn't let an attacker brute-force the 32^8
    /// code space offline. Derived at startup from `ENCRYPTION_KEY`
    /// (or `CLI_PAIRING_HMAC_KEY` when set); see
    /// `derive_cli_pairing_hmac_key` in `main.rs`.
    pub cli_pairing_hmac_key: std::sync::Arc<zeroize::Zeroizing<[u8; 32]>>,
    /// Per-channel rate limiter keyed by conversation_id, for the HTTP Event
    /// Gateway (NyxID#221). Distinct from `per_agent_limiter`.
    pub per_channel_event_limiter: mw::rate_limit::SharedPerChannelEventLimiter,
    /// Per-upstream-message edit limiter for progressive channel relay edits.
    pub per_message_edit_limiter: mw::rate_limit::SharedPerMessageEditRateLimiter,
    /// Best-effort idempotency cache for inbound channel events.
    pub event_dedup_cache: Arc<EventDedupCache>,
    /// Per-process DPoP proof replay cache keyed by proof jti.
    pub dpop_jti_cache: Arc<DpopJtiCache>,
    /// Active WebSocket passthrough connection count (for resource limiting)
    pub ws_passthrough_count: Arc<std::sync::atomic::AtomicUsize>,
    /// Generic downstream-provider token exchange cache with per-key
    /// single-flight. Backs the `token_exchange` auth method (Lark/Feishu
    /// tenant tokens, OAuth 2.0 client_credentials, etc.) and the channel
    /// bot adapter's outbound replies.
    pub token_exchange_cache: Arc<TokenExchangeCache>,
    /// Response cache for the `aws_sigv4` auth method. AWS Cost Explorer
    /// charges per request, so identical proxy calls in a short window get
    /// replayed from cache. TTL is driven by
    /// `cloud_response_cache_ttl_secs`. NyxID#716.
    pub cloud_response_cache: Arc<crate::services::cloud_response_cache::CloudResponseCache>,
    /// Vendor-neutral telemetry client. `None` when no DSN is configured
    /// (the default hard-off state — see `docs/TELEMETRY.md` §3).
    pub telemetry: Option<Arc<telemetry::TelemetryClient>>,
}

/// NyxID authentication and SSO platform.
#[derive(Parser)]
#[command(name = "nyxid", version, about)]
struct Cli {
    /// Promote an existing user to admin by email address, then exit.
    #[arg(long = "promote-admin", value_name = "EMAIL")]
    promote_admin: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with a NyxID server and print an access token.
    Login(login_cli::LoginArgs),
    /// SSH client helper commands for certificate issuance and ProxyCommand integration.
    Ssh(ssh_cli::SshCli),
    /// Scan for and hard-delete orphaned user_endpoints and user_api_keys left
    /// over by pre-fix revoke flows. Prints a preview and prompts before deleting.
    CleanupOrphans(cleanup_cli::CleanupArgs),
}

#[tokio::main]
async fn main() {
    // Pick a rustls `CryptoProvider` explicitly before ANY TLS use.
    // Feature unification can compile multiple providers into the
    // backend (notably aws_lc_rs and ring), and rustls cannot
    // auto-select in that state. The CLI uses the same pattern; see
    // cli/src/main.rs for the canonical version.
    install_rustls_crypto_provider();

    let cli = Cli::parse();

    // Load environment variables from .env file (if present)
    dotenvy::dotenv().ok();

    // Initialize structured logging
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("nyxid=info,tower_http=info")),
        )
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();

    // Login and Ssh don't touch the database; handle them before we connect.
    // Other subcommands (like CleanupOrphans) fall through to the post-DB path.
    let post_db_command = match cli.command {
        Some(Commands::Login(args)) => {
            if let Err(error) = login_cli::run(args).await {
                eprintln!("Login failed: {error}");
                std::process::exit(1);
            }
            return;
        }
        Some(Commands::Ssh(ssh_cli)) => {
            if let Err(error) = ssh_cli::run(ssh_cli).await {
                eprintln!("SSH helper failed: {error}");
                std::process::exit(1);
            }
            return;
        }
        other => other,
    };

    // Load configuration
    let mut config = AppConfig::from_env();
    config.validate_ssh_runtime_config();

    // Connect to database
    let db = db::create_connection(&config)
        .await
        .expect("Failed to connect to database");

    // Handle CLI commands (exit without starting server)
    if let Some(email) = cli.promote_admin {
        services::role_service::seed_system_roles(&db)
            .await
            .expect("Failed to seed system roles");
        services::role_service::backfill_platform_role_memberships(&db)
            .await
            .expect("Failed to backfill platform role memberships");
        run_promote_admin(&db, &email).await;
        return;
    }
    if let Some(Commands::CleanupOrphans(args)) = post_db_command {
        if let Err(error) = cleanup_cli::run(&db, args).await {
            eprintln!("Cleanup failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    // Validate provider-specific encryption config before any seed calls that use it.
    config.validate_key_provider();

    // Build key provider(s)
    let (provider, fallback_provider): (Arc<dyn KeyProvider>, Option<Arc<dyn KeyProvider>>) =
        match config.key_provider.as_str() {
            "local" => {
                let local = Arc::new(LocalKeyProvider::from_config(&config));
                (local, None)
            }
            #[cfg(feature = "aws-kms")]
            "aws-kms" => {
                let kms =
                    Arc::new(crypto::aws_kms_provider::AwsKmsProvider::from_config(&config).await);
                let fallback = config.encryption_key.as_ref().map(|_| {
                    Arc::new(LocalKeyProvider::from_config(&config)) as Arc<dyn KeyProvider>
                });
                (kms, fallback)
            }
            #[cfg(feature = "gcp-kms")]
            "gcp-kms" => {
                let kms =
                    Arc::new(crypto::gcp_kms_provider::GcpKmsProvider::from_config(&config).await);
                let fallback = config.encryption_key.as_ref().map(|_| {
                    Arc::new(LocalKeyProvider::from_config(&config)) as Arc<dyn KeyProvider>
                });
                (kms, fallback)
            }
            other => panic!("Unsupported KEY_PROVIDER: {other}"),
        };

    // Cross-provider key_id collision check (H2): during migration, the primary
    // KMS provider and fallback local provider derive key_ids from different
    // inputs. A collision (1-in-256 chance) would cause decrypt failures.
    if let Some(ref fallback) = fallback_provider
        && fallback.has_key_id(provider.current_key_id())
    {
        panic!(
            "Primary and fallback providers have colliding key IDs (0x{:02x}). \
             This is a 1-in-256 hash collision. Use a different KMS key.",
            provider.current_key_id()
        );
    }

    // Build EncryptionKeys with provider and optional fallback
    let legacy = if config.encryption_key.is_some() {
        Some(crypto::aes::LegacyKeys::from_config(&config))
    } else {
        None
    };

    let encryption_keys = Arc::new({
        let mut ek = EncryptionKeys::with_provider_and_fallback(provider, fallback_provider);
        if let Some(l) = legacy {
            ek.set_legacy(l);
        }
        ek
    });
    if encryption_keys.has_previous() {
        tracing::warn!(
            "ENCRYPTION_KEY_PREVIOUS is configured. Only one previous key is supported; do not rotate again until all old-key ciphertexts have been re-wrapped or re-encrypted."
        );
    }

    // Seed default OAuth clients (idempotent)
    services::oauth_client_service::seed_default_clients(&db)
        .await
        .expect("Failed to seed default OAuth clients");

    // Backfill default MCP scopes on dynamic-registration clients whenever
    // the default set grows (e.g. issue #434 added `roles`/`groups`).
    // Idempotent.
    services::oauth_client_service::migrate_dynamic_clients_grant_default_mcp_scopes(&db)
        .await
        .expect("Failed to migrate dynamic OAuth clients");

    // Seed default AI provider configurations (idempotent)
    services::provider_service::seed_default_providers(&db, encryption_keys.as_ref())
        .await
        .expect("Failed to seed default providers");

    // Seed downstream services for default providers (idempotent)
    services::provider_service::seed_default_services(&db, encryption_keys.as_ref())
        .await
        .expect("Failed to seed default services");

    // Heal UserService rows whose `auth_method` was snapshotted as the raw
    // catalog `"none"` instead of the SPR-derived injection config, which
    // stops the proxy from injecting the caller's stored credential.
    services::user_service_service::backfill_stale_catalog_auth_snapshots(&db)
        .await
        .expect("Failed to backfill stale UserService auth_method snapshots");

    // Seed system roles for RBAC (idempotent)
    services::role_service::seed_system_roles(&db)
        .await
        .expect("Failed to seed system roles");
    let platform_role_backfill = services::role_service::backfill_platform_role_memberships(&db)
        .await
        .expect("Failed to backfill platform role memberships");
    tracing::info!(
        admin_role_id = %platform_role_backfill.admin_role_id,
        operator_role_id = %platform_role_backfill.operator_role_id,
        admin_users_modified = platform_role_backfill.admin_users_modified,
        operator_users_modified = platform_role_backfill.operator_users_modified,
        "Platform role membership backfill complete"
    );

    // Run unified collection migration (idempotent, non-fatal)
    if let Err(e) = db::migrate_to_unified_collections(&db).await {
        tracing::warn!("Unified collection migration encountered errors: {e}");
    }

    // --- Server startup ---
    tracing::info!("Starting NyxID authentication server");
    tracing::info!(port = config.port, issuer = %config.jwt_issuer, "Configuration loaded");
    config.warn_if_non_url_issuer();

    // Validate and initialize push notification config (reads FCM JSON, verifies APNs key)
    config.validate_push_config();

    // Initialize push notification auth
    let fcm_auth = if config.fcm_service_account_path.is_some() {
        match FcmAuth::from_service_account_file(
            config.fcm_service_account_path.as_deref().unwrap(),
        ) {
            Ok(auth) => Some(Arc::new(auth)),
            Err(e) => {
                tracing::error!("Failed to initialize FCM auth: {e}");
                None
            }
        }
    } else {
        None
    };

    let apns_auth = if config.apns_key_path.is_some() {
        match ApnsAuth::new(
            config.apns_key_path.as_deref().unwrap(),
            config.apns_key_id.as_deref().expect("APNS_KEY_ID required"),
            config
                .apns_team_id
                .as_deref()
                .expect("APNS_TEAM_ID required"),
        ) {
            Ok(auth) => Some(Arc::new(auth)),
            Err(e) => {
                tracing::error!("Failed to initialize APNs auth: {e}");
                None
            }
        }
    } else {
        None
    };

    // Load JWT signing keys
    let jwt_keys = JwtKeys::from_config(&config).expect("Failed to load JWT keys");
    tracing::info!("JWT keys loaded (kid={})", jwt_keys.kid);

    // Compute JWK from the public key for the JWKS endpoint
    let public_pem = std::fs::read_to_string(&config.jwt_public_key_path)
        .expect("Failed to read public key for JWK");
    let jwk_json =
        crypto::jwt::public_key_jwk(&public_pem).expect("Failed to compute JWK from public key");

    // Create a shared reqwest client for connection reuse.
    // Use connect_timeout (not global timeout) so SSE streaming responses
    // from LLM services are not killed after 30 seconds.
    let http_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .build()
        .expect("Failed to create HTTP client");

    // Create MCP session store with MongoDB persistence
    let mcp_sessions = Arc::new(McpSessionStore::with_db(db.clone()));

    // Recover MCP sessions from MongoDB (survives server restarts)
    match mcp_sessions.load_from_db().await {
        Ok(0) => tracing::info!("No MCP sessions to recover"),
        Ok(count) => tracing::info!(count, "Recovered MCP sessions from database"),
        Err(e) => tracing::warn!("Failed to load MCP sessions from database: {e}"),
    }

    // Create JWKS cache for external provider token verification
    let jwks_cache = Arc::new(JwksCache::new(http_client.clone()));

    // Create node WebSocket connection manager
    let node_ws_manager = Arc::new(NodeWsManager::new(
        config.node_proxy_timeout_secs,
        config.node_max_ws_connections,
    ));
    let ssh_session_manager = Arc::new(SshSessionManager::new(config.ssh_max_sessions_per_user));

    // HTTP Event Gateway state (NyxID#221).
    let per_channel_event_limiter = Arc::new(mw::rate_limit::PerChannelEventLimiter::new(
        config.channel_event_rate_limit_per_second,
        config.channel_event_rate_limit_burst,
    ));
    let event_dedup_cache = Arc::new(EventDedupCache::new(
        config.channel_event_dedup_capacity,
        std::time::Duration::from_secs(config.channel_event_dedup_ttl_secs),
    ));
    let dpop_jti_cache = Arc::new(DpopJtiCache::new(
        DPOP_JTI_CACHE_CAPACITY,
        std::time::Duration::from_secs(DPOP_JTI_CACHE_TTL_SECS),
    ));
    let per_message_edit_limiter = Arc::new(mw::rate_limit::PerMessageEditRateLimiter::new(
        config.channel_relay_edit_rate_limit_per_second,
        config.channel_relay_edit_rate_limit_burst,
    ));

    // Derive the CLI-pairing HMAC key. Kept in process memory
    // only; see `derive_cli_pairing_hmac_key` for the key source.
    // The JWT private key file contents are the universal
    // fallback so KMS deployments without `ENCRYPTION_KEY` still
    // start — see the function's doc for the full priority chain.
    let jwt_private_key_pem = std::fs::read(&config.jwt_private_key_path)
        .expect("Failed to read JWT private key for CLI-pairing HMAC seed");
    let cli_pairing_hmac_key = Arc::new(derive_cli_pairing_hmac_key(
        config.cli_pairing_hmac_key.as_deref(),
        config.encryption_key.as_deref(),
        Some(&jwt_private_key_pem),
    ));
    // JWT private key bytes carry no secret beyond what's
    // already in JwtKeys; drop them immediately after derivation.
    drop(jwt_private_key_pem);

    // Create shared state
    let state = AppState {
        db,
        config: config.clone(),
        jwt_keys,
        http_client,
        jwk_json,
        mcp_sessions: mcp_sessions.clone(),
        jwks_cache,
        fcm_auth: fcm_auth.clone(),
        apns_auth: apns_auth.clone(),
        encryption_keys: encryption_keys.clone(),
        node_ws_manager,
        ssh_session_manager,
        per_agent_limiter: Arc::new(mw::rate_limit::PerAgentRateLimiter::new()),
        // 5 claim attempts per 60 seconds per IP; window-based, not token
        // bucket, because we want a hard cap on guesses per unit time.
        cli_pairing_claim_limiter: mw::rate_limit::create_per_ip_rate_limiter(5, 60),
        cli_pairing_hmac_key,
        per_channel_event_limiter,
        per_message_edit_limiter,
        event_dedup_cache,
        dpop_jti_cache,
        ws_passthrough_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        token_exchange_cache: Arc::new(TokenExchangeCache::new()),
        cloud_response_cache: Arc::new(
            services::cloud_response_cache::CloudResponseCache::with_bounds(
                config.cloud_response_cache_ttl_secs,
                config.cloud_response_cache_max_entry_bytes,
                config.cloud_response_cache_max_entries,
            ),
        ),
        telemetry: telemetry::TelemetryClient::from_config(&config),
    };

    // Spawn the telemetry-erasure worker. No-op when `state.telemetry`
    // is `None` (hard-off mode); the function logs + returns.
    services::telemetry_erasure_service::spawn_worker(state.db.clone(), state.telemetry.clone());

    // Create rate limiters
    let global_rate_limiter =
        mw::rate_limit::create_rate_limiter(config.rate_limit_per_second, config.rate_limit_burst);
    let per_ip_rate_limiter = mw::rate_limit::create_per_ip_rate_limiter(
        config.rate_limit_burst, // per-IP max requests per window
        1,                       // 1-second window
    );

    // Spawn background cleanup task for per-IP rate limiter
    let cleanup_limiter = per_ip_rate_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup_limiter.cleanup();
        }
    });

    // Spawn background cleanup task for the CLI-pairing claim limiter.
    // Same cadence as the global per-IP limiter.
    let cleanup_pairing_claim_limiter = state.cli_pairing_claim_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup_pairing_claim_limiter.cleanup();
        }
    });

    // Spawn background cleanup task for the per-agent token bucket limiter.
    // Entries are retained for 120 seconds to avoid re-allocation for agents
    // that send bursts slightly apart.
    // 60-second cleanup interval is intentionally coarse to minimize lock contention.
    let cleanup_agent_limiter = state.per_agent_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup_agent_limiter.cleanup();
        }
    });

    // Spawn background cleanup for the per-channel event limiter and the
    // event idempotency LRU. Same cadence as the per-agent limiter.
    let cleanup_event_limiter = state.per_channel_event_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup_event_limiter.cleanup();
        }
    });
    let cleanup_edit_limiter = state.per_message_edit_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup_edit_limiter.cleanup();
        }
    });
    let cleanup_event_dedup = state.event_dedup_cache.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup_event_dedup.cleanup();
        }
    });

    // Spawn background cleanup task for MCP session reaper.
    // Sessions live up to 30 days (extended on every request via touch()).
    // Reaper runs every 5 minutes to clean up sessions idle longer than 30 days.
    let mcp_sessions_for_reaper = mcp_sessions.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            mcp_sessions_for_reaper.reap_expired(std::time::Duration::from_secs(
                models::mcp_session::MCP_SESSION_MAX_IDLE_SECS,
            ));
        }
    });

    // Spawn background task to expire timed-out approval requests
    let db_for_expiry = state.db.clone();
    let config_for_expiry = state.config.clone();
    let http_for_expiry = state.http_client.clone();
    let fcm_for_expiry = state.fcm_auth.clone();
    let apns_for_expiry = state.apns_auth.clone();
    let telemetry_for_expiry = state.telemetry.clone();
    let expiry_interval_secs = config.approval_expiry_interval_secs;
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(expiry_interval_secs));
        // Prevent rapid-fire catch-up sweeps when a tick is delayed (e.g. under
        // database backpressure). Burst mode (the default) would fire consecutive
        // sweeps immediately, widening the race window between find() and
        // update_many() in expire_pending_requests.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(e) = services::approval_service::expire_pending_requests(
                &db_for_expiry,
                &config_for_expiry,
                &http_for_expiry,
                fcm_for_expiry.as_deref(),
                apns_for_expiry.as_deref(),
                telemetry_for_expiry.as_ref(),
            )
            .await
            {
                tracing::warn!("Approval expiry task error: {e}");
            }
        }
    });

    // Telegram integration: webhook mode (production) or polling mode (development)
    if let (Some(bot_token), Some(webhook_url), Some(webhook_secret)) = (
        &config.telegram_bot_token,
        &config.telegram_webhook_url,
        &config.telegram_webhook_secret,
    ) {
        // Production: register webhook with Telegram
        match services::telegram_service::set_webhook(
            &state.http_client,
            bot_token,
            webhook_url,
            webhook_secret,
        )
        .await
        {
            Ok(()) => tracing::info!("Telegram webhook registered: {webhook_url}"),
            Err(e) => tracing::error!("Failed to register Telegram webhook: {e}"),
        }

        // Periodically verify the webhook is healthy and re-register if needed.
        // Telegram can silently drop webhooks after sustained delivery failures.
        let wh_http = state.http_client.clone();
        let wh_token = bot_token.clone();
        let wh_url = webhook_url.clone();
        let wh_secret = webhook_secret.clone();
        tokio::spawn(async move {
            // Check every 5 minutes
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            // Skip the first tick (we just registered above)
            interval.tick().await;
            loop {
                interval.tick().await;
                if !services::telegram_service::is_webhook_healthy(&wh_http, &wh_token, &wh_url)
                    .await
                {
                    tracing::warn!("Telegram webhook unhealthy, re-registering");
                    match services::telegram_service::set_webhook(
                        &wh_http, &wh_token, &wh_url, &wh_secret,
                    )
                    .await
                    {
                        Ok(()) => tracing::info!("Telegram webhook re-registered"),
                        Err(e) => tracing::error!("Telegram webhook re-registration failed: {e}"),
                    }
                }
            }
        });
    } else if config.telegram_bot_token.is_some() {
        // Development fallback: poll getUpdates when no webhook URL is configured
        let polling_state = state.clone();
        tokio::spawn(async move {
            services::telegram_poller::run_polling_loop(polling_state).await;
        });
    }

    // Spawn background heartbeat sweep for node WebSocket connections
    let heartbeat_db = state.db.clone();
    let heartbeat_ws = state.node_ws_manager.clone();
    let heartbeat_interval = config.node_heartbeat_interval_secs;
    let heartbeat_timeout = config.node_heartbeat_timeout_secs;
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(heartbeat_interval));
        loop {
            interval.tick().await;
            handlers::node_ws::node_ws_manager_heartbeat_sweep(
                &heartbeat_db,
                &heartbeat_ws,
                heartbeat_timeout,
            )
            .await;
        }
    });

    // Build set of allowed CORS origins: frontend_url + any extra from CORS_ALLOWED_ORIGINS
    let mut allowed_origins: std::collections::HashSet<axum::http::HeaderValue> =
        std::collections::HashSet::new();
    allowed_origins.insert(config.frontend_url.parse().expect("Invalid FRONTEND_URL"));
    for origin in &config.cors_allowed_origins {
        if let Ok(hv) = origin.parse() {
            allowed_origins.insert(hv);
        } else {
            tracing::warn!(origin, "Ignoring invalid CORS_ALLOWED_ORIGINS entry");
        }
    }
    tracing::info!(
        origins = ?allowed_origins.iter().map(|h| h.to_str().unwrap_or("?")).collect::<Vec<_>>(),
        "CORS allowed origins"
    );

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _| {
            allowed_origins.contains(origin)
        }))
        .allow_methods(AllowMethods::list([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::PATCH,
            axum::http::Method::OPTIONS,
        ]))
        .allow_headers(AllowHeaders::list([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::header::ACCEPT,
            axum::http::header::ORIGIN,
            axum::http::header::COOKIE,
            "X-User-Email".parse().unwrap(),
            "X-User-Display-Name".parse().unwrap(),
            "X-API-Key".parse().unwrap(),
            "X-NyxID-Client".parse().unwrap(),
            "X-NyxID-Client-Version".parse().unwrap(),
        ]))
        .allow_credentials(true);

    // Build router — public OAuth routes get open CORS (per RFC 9207),
    // private API routes get restricted CORS (FRONTEND_URL only).
    let (public_oauth, private_api) = routes::build_router(config.proxy_max_body_size);

    let csrf_state = state.clone();
    let private_api = private_api.layer(cors).layer(axum_mw::from_fn_with_state(
        csrf_state,
        mw::csrf::browser_csrf_middleware,
    ));

    let app = public_oauth
        .merge(private_api)
        .with_state(state)
        .layer(DefaultBodyLimit::max(1_048_576))
        .layer(axum_mw::from_fn(
            mw::security_headers::security_headers_middleware,
        ))
        .layer(axum_mw::from_fn(mw::rate_limit::rate_limit_middleware))
        // Derive `TelemetryContext` from the `X-NyxID-Client` headers on
        // every request and stash it in request extensions so handlers
        // can read it when they emit events. Header-only; the
        // `surface="agent"` override for api-key auth happens at emit
        // time in `emit_event` (see `docs/TELEMETRY.md` §5.1).
        .layer(axum_mw::from_fn(mw::telemetry::telemetry_mw))
        .layer(Extension(per_ip_rate_limiter))
        .layer(Extension(global_rate_limiter))
        .layer(TraceLayer::new_for_http());

    // Bind and serve
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    tracing::info!("Listening on {addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("Server error");
}

/// Derive the HMAC key used to key `CliPairing.code_hash`.
///
/// Priority:
///   1. `env_override` (sourced from `CLI_PAIRING_HMAC_KEY` in
///      production). 64 hex chars = 32 bytes. Use this in multi-
///      instance deployments so all workers agree on the HMAC
///      output for the same code.
///   2. Derived from `encryption_key` (`ENCRYPTION_KEY`) via
///      HMAC-SHA256 with a domain-separated label. Stable across
///      restarts and instances that share `ENCRYPTION_KEY`. This
///      is the expected production path for the local key provider.
///   3. Derived from `jwt_private_key_pem` via HMAC-SHA256 with a
///      distinct domain-separated label. The JWT signing key is
///      always loaded at startup (the service won't come up
///      without it) and is by deployment practice shared across
///      workers. This branch is what keeps `KEY_PROVIDER=aws-kms`
///      or `gcp-kms` deployments (which may omit `ENCRYPTION_KEY`)
///      booting without operators having to configure an extra
///      env var up front — and it still yields the same HMAC on
///      every worker, so CLI remote pairing keeps working without
///      sticky sessions.
///   4. Panic. Should be unreachable in practice because the JWT
///      private key is required; the branch exists purely as a
///      defensive stop against a future refactor silently
///      dropping the JWT fallback.
///
/// The key never touches MongoDB; an attacker with DB-only access
/// cannot derive it and therefore cannot brute-force the ~2^40
/// code space offline.
///
/// All inputs are passed in (rather than read via `std::env::var`,
/// through `&AppConfig`, or from disk here) so unit tests can pin
/// precedence without racing on process-wide environment state.
fn derive_cli_pairing_hmac_key(
    env_override: Option<&str>,
    encryption_key: Option<&str>,
    jwt_private_key_pem: Option<&[u8]>,
) -> zeroize::Zeroizing<[u8; 32]> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    // 1. Explicit env override takes precedence. A set-but-empty
    //    value is treated as unset (dotenv / docker-compose often
    //    emit empty strings for unused vars).
    if let Some(raw) = env_override {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            match hex::decode(trimmed) {
                Ok(bytes) if bytes.len() == 32 => {
                    let mut out = [0u8; 32];
                    out.copy_from_slice(&bytes);
                    tracing::info!("cli-pairing HMAC key loaded from CLI_PAIRING_HMAC_KEY");
                    return zeroize::Zeroizing::new(out);
                }
                _ => {
                    panic!("CLI_PAIRING_HMAC_KEY must be 64 hex characters (32 bytes)");
                }
            }
        }
    }

    // 2. Derive from ENCRYPTION_KEY if configured. ENCRYPTION_KEY
    //    is 32 bytes of random material for the local AES provider
    //    and is the natural shared-secret in single-region
    //    deployments. Domain-separate the output so we can't
    //    accidentally collide with any future HMAC use.
    if let Some(hex_key) = encryption_key
        && let Ok(master) = hex::decode(hex_key.trim())
        && master.len() == 32
    {
        let mut mac =
            HmacSha256::new_from_slice(&master).expect("HMAC-SHA256 accepts any key length");
        mac.update(b"nyxid:cli-pairing-code-hmac-v1");
        let digest = mac.finalize().into_bytes();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        tracing::info!("cli-pairing HMAC key derived from ENCRYPTION_KEY");
        return zeroize::Zeroizing::new(out);
    }

    // 3. Universal JWT-private-key fallback. Lets KMS
    //    deployments (which may legitimately omit
    //    `ENCRYPTION_KEY`) boot without requiring ops to set
    //    `CLI_PAIRING_HMAC_KEY` up front — and still gives every
    //    worker the same HMAC because the JWT private key is the
    //    same PEM file on every instance. Domain-separated with
    //    a distinct label so the derivation can't collide with
    //    any other HMAC use of the JWT key.
    if let Some(pem) = jwt_private_key_pem
        && !pem.is_empty()
    {
        let mut mac = HmacSha256::new_from_slice(pem).expect("HMAC-SHA256 accepts any key length");
        mac.update(b"nyxid:cli-pairing-code-hmac-v1:jwt");
        let digest = mac.finalize().into_bytes();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        tracing::info!(
            "cli-pairing HMAC key derived from JWT private key \
             (neither CLI_PAIRING_HMAC_KEY nor ENCRYPTION_KEY is \
             set). Set CLI_PAIRING_HMAC_KEY explicitly if you need \
             to rotate it independently of the JWT signing key."
        );
        return zeroize::Zeroizing::new(out);
    }

    // 4. Defensive stop. Unreachable in practice because the JWT
    //    private key is loaded before this function is called —
    //    the service would have panicked earlier. Keep the
    //    message actionable in case a future refactor severs the
    //    JWT input.
    panic!(
        "cli-pairing HMAC key has no source: CLI_PAIRING_HMAC_KEY, \
         ENCRYPTION_KEY, and the JWT private key are all missing. \
         Set CLI_PAIRING_HMAC_KEY to a 64-hex-char value \
         (`openssl rand -hex 32`) — see docs/ENV.md."
    );
}

/// Run the --promote-admin CLI command, then return.
async fn run_promote_admin(db: &mongodb::Database, email: &str) {
    use services::{audit_service, auth_service};

    match auth_service::promote_user_to_admin(db, email).await {
        Ok(user_id) => {
            audit_service::log_async(
                db.clone(),
                Some(user_id.clone()),
                "admin_promoted".to_string(),
                Some(serde_json::json!({
                    "email": email,
                    "method": "cli"
                })),
                None,
                None,
                None,
                None,
            );

            // Brief sleep to allow the async audit log write to complete
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            println!("Successfully promoted user to admin:");
            println!("  Email:   {email}");
            println!("  User ID: {user_id}");
        }
        Err(e) => {
            eprintln!("Failed to promote admin: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for startup-time helpers that sit in `main.rs`.
    //!
    //! Keeping derivation-ordering invariants pinned here because a
    //! silent precedence flip (e.g. a refactor that read
    //! `config.encryption_key` before the env override) would be a
    //! security regression: the explicit override is the escape
    //! hatch ops use when policy says "do not reuse encryption-key
    //! material for other HMAC purposes," and it must win every
    //! time.

    use super::*;

    fn enc_hex(byte: u8) -> String {
        hex::encode([byte; 32])
    }

    fn jwt_bytes(byte: u8) -> Vec<u8> {
        // Stand-in for a PEM file's bytes; the derivation reads
        // raw bytes and doesn't care about PEM structure.
        vec![byte; 1_024]
    }

    #[test]
    fn cli_pairing_key_prefers_env_override_over_encryption_key() {
        // Both set. The explicit env override must win — ops use
        // this to decouple the pairing HMAC key from ENCRYPTION_KEY
        // (for independent rotation, or for policies that forbid
        // reusing key material across purposes).
        let env_override_hex = hex::encode([0xAAu8; 32]);
        let enc = enc_hex(0x55);
        let jwt = jwt_bytes(0x33);

        let key_with_override =
            derive_cli_pairing_hmac_key(Some(&env_override_hex), Some(&enc), Some(&jwt));
        let key_from_enc_only = derive_cli_pairing_hmac_key(None, Some(&enc), Some(&jwt));

        assert_eq!(
            key_with_override.as_slice(),
            &[0xAAu8; 32],
            "env override must be used verbatim, not derived"
        );
        assert_ne!(
            key_with_override.as_slice(),
            key_from_enc_only.as_slice(),
            "env override must NOT match the ENCRYPTION_KEY-derived key"
        );
    }

    #[test]
    fn cli_pairing_key_prefers_encryption_key_over_jwt_fallback() {
        // With ENCRYPTION_KEY set we prefer that derivation over
        // the JWT fallback, so operators migrating from local to
        // KMS don't get a silent HMAC-key change.
        let enc = enc_hex(0x55);
        let jwt = jwt_bytes(0x33);
        let with_enc = derive_cli_pairing_hmac_key(None, Some(&enc), Some(&jwt));
        let jwt_only = derive_cli_pairing_hmac_key(None, None, Some(&jwt));
        assert_ne!(
            with_enc.as_slice(),
            jwt_only.as_slice(),
            "ENCRYPTION_KEY derivation must differ from JWT fallback"
        );
    }

    #[test]
    fn cli_pairing_key_falls_back_to_encryption_key_when_env_absent() {
        // Unset and empty-string overrides both fall through to
        // the ENCRYPTION_KEY derivation — dotenv often emits empty
        // strings for unused vars.
        let enc = enc_hex(0x55);
        let jwt = jwt_bytes(0x33);
        let unset = derive_cli_pairing_hmac_key(None, Some(&enc), Some(&jwt));
        let empty = derive_cli_pairing_hmac_key(Some(""), Some(&enc), Some(&jwt));
        let whitespace_only = derive_cli_pairing_hmac_key(Some("   "), Some(&enc), Some(&jwt));

        assert_eq!(unset.as_slice(), empty.as_slice());
        assert_eq!(unset.as_slice(), whitespace_only.as_slice());
    }

    #[test]
    fn cli_pairing_key_falls_back_to_jwt_when_encryption_key_absent() {
        // KMS deployments without ENCRYPTION_KEY must still boot.
        // The JWT private key is always loaded at startup so
        // deriving the pairing HMAC from its PEM bytes is the
        // zero-configuration path for those deployments. The
        // derivation must still be deterministic so all workers
        // agree on the HMAC output.
        let jwt = jwt_bytes(0x33);
        let a = derive_cli_pairing_hmac_key(None, None, Some(&jwt));
        let b = derive_cli_pairing_hmac_key(None, None, Some(&jwt));
        assert_eq!(
            a.as_slice(),
            b.as_slice(),
            "JWT fallback must be deterministic across calls"
        );
    }

    #[test]
    fn cli_pairing_key_jwt_fallback_is_keyed_by_jwt_contents() {
        // Different JWT PEM → different derived HMAC, so rotating
        // the JWT signing key rotates the pairing HMAC in lockstep.
        let a = derive_cli_pairing_hmac_key(None, None, Some(&jwt_bytes(0x11)));
        let b = derive_cli_pairing_hmac_key(None, None, Some(&jwt_bytes(0x22)));
        assert_ne!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn cli_pairing_key_derivation_is_deterministic_across_calls() {
        // Two calls with the same inputs must produce the same
        // derived key — otherwise multi-instance deployments
        // sharing ENCRYPTION_KEY would disagree on HMACs.
        let enc = enc_hex(0x77);
        let jwt = jwt_bytes(0x33);
        let a = derive_cli_pairing_hmac_key(None, Some(&enc), Some(&jwt));
        let b = derive_cli_pairing_hmac_key(None, Some(&enc), Some(&jwt));
        assert_eq!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn cli_pairing_key_differs_per_encryption_key() {
        // Different ENCRYPTION_KEY → different derived HMAC key,
        // so the domain-separated derivation actually does depend
        // on the master.
        let jwt = jwt_bytes(0x33);
        let a = derive_cli_pairing_hmac_key(None, Some(&enc_hex(0x11)), Some(&jwt));
        let b = derive_cli_pairing_hmac_key(None, Some(&enc_hex(0x22)), Some(&jwt));
        assert_ne!(a.as_slice(), b.as_slice());
    }

    #[test]
    #[should_panic(expected = "CLI_PAIRING_HMAC_KEY must be 64 hex characters")]
    fn cli_pairing_key_panics_on_malformed_env_override() {
        // A typo must fail loudly at startup — silently falling
        // through to derivation would hide operator intent
        // ("use THIS key, not one derived from ENCRYPTION_KEY").
        let _ = derive_cli_pairing_hmac_key(
            Some("not-valid-hex"),
            Some(&enc_hex(0x55)),
            Some(&jwt_bytes(0x33)),
        );
    }

    #[test]
    #[should_panic(expected = "CLI_PAIRING_HMAC_KEY must be 64 hex characters")]
    fn cli_pairing_key_panics_on_wrong_length_env_override() {
        // Hex parses fine but the byte count is wrong — same
        // failure mode, still loud.
        let short = hex::encode([0xCCu8; 16]);
        let _ =
            derive_cli_pairing_hmac_key(Some(&short), Some(&enc_hex(0x55)), Some(&jwt_bytes(0x33)));
    }

    #[test]
    #[should_panic(expected = "cli-pairing HMAC key has no source")]
    fn cli_pairing_key_panics_when_no_source_at_all() {
        // JWT input is required in production; if none of the
        // three sources resolve, refuse to start.
        let _ = derive_cli_pairing_hmac_key(None, None, None);
    }

    #[test]
    #[should_panic(expected = "cli-pairing HMAC key has no source")]
    fn cli_pairing_key_panics_on_all_empty_inputs() {
        // Empty strings / empty slice are treated like unset.
        let _ = derive_cli_pairing_hmac_key(Some(""), Some(""), Some(&[]));
    }
}
