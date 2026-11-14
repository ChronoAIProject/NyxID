// The assistant-actions golden manifest is a single `json!` literal covering
// every published descriptor (54 and growing), which exceeds the default
// macro recursion limit during test expansion.
#![recursion_limit = "1024"]

use axum::{extract::DefaultBodyLimit, extract::Extension, middleware as axum_mw};
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod api_docs;
mod cleanup_cli;
mod config;
mod crypto;
mod db;
mod downstream_disconnect;
mod errors;
mod handlers;
mod login_cli;
mod models;
mod mw;
mod redaction;
mod routes;
mod services;
mod shutdown_servers;
mod ssh_cli;
mod telemetry;
mod telemetry_erasure_cli;
mod turn_trace;

#[cfg(test)]
mod test_utils;

#[cfg(test)]
mod billing_integration_tests;

#[cfg(test)]
mod grant_visibility_tests;

#[cfg(test)]
mod credit_schedule_tests;

#[cfg(test)]
mod coordination_tests;

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
use crypto::hmac_keys::derive_hmac_key;
use crypto::jwks::JwksCache;
use crypto::jwt::JwtKeys;
use crypto::key_provider::KeyProvider;
use crypto::local_key_provider::LocalKeyProvider;
use models::mcp_session::McpSessionStore;

use services::node_ws_manager::NodeWsManager;
use services::platform_settings_service::BrokerPolicy;
use services::provider_token_exchange_service::TokenExchangeCache;
use services::push_service::{ApnsAuth, FcmAuth};
use services::ssh_service::SshSessionManager;

const BROKER_POLICY_REFRESH_INTERVAL_SECS: u64 = 5;
// Three refresh intervals tolerate two missed polls without an unbounded bypass.
const BROKER_POLICY_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(15);

pub struct CachedBrokerPolicy {
    policy: BrokerPolicy,
    verified_at: tokio::time::Instant,
    last_read_started_at: tokio::time::Instant,
    pending_reset: Option<(i64, tokio::time::Instant)>,
    reset_read_started_at: Option<tokio::time::Instant>,
    stale_reported: std::sync::atomic::AtomicBool,
}

impl CachedBrokerPolicy {
    fn new(policy: BrokerPolicy, verified_at: tokio::time::Instant) -> Self {
        Self {
            policy,
            verified_at,
            last_read_started_at: verified_at,
            pending_reset: None,
            reset_read_started_at: None,
            stale_reported: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn is_fresh(&self) -> bool {
        let stale = self.verified_at.elapsed() >= BROKER_POLICY_MAX_AGE;
        if stale
            && !self
                .stale_reported
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            tracing::error!(target: "nyxid::broker_policy", revision = self.policy.revision,
                "Broker rollout policy snapshot became stale");
        }
        !stale
    }

    // Only an accepted refresh under the write lock may report recovery. A
    // delayed reader that saw a fresh snapshot cannot undo a stale transition.
    fn report_recovery_if_fresh(&self) {
        if self.is_fresh()
            && self
                .stale_reported
                .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            tracing::info!(target: "nyxid::broker_policy", revision = self.policy.revision,
                "Broker rollout policy snapshot recovered");
        }
    }
}

impl From<BrokerPolicy> for CachedBrokerPolicy {
    fn from(policy: BrokerPolicy) -> Self {
        Self::new(policy, tokio::time::Instant::now())
    }
}

/// Shared application state available to all handlers via Axum's State extractor.
#[derive(Clone)]
pub struct AppState {
    pub shutdown: services::shutdown::Runtime,
    #[cfg(test)]
    pub(crate) local_machine_platform: Option<Arc<crate::test_utils::LocalMachinePlatform>>,
    pub db: DbHandle,
    pub config: AppConfig,
    pub jwt_keys: JwtKeys,
    pub http_client: reqwest::Client,
    /// Pre-computed JWK for the JWKS endpoint
    pub jwk_json: serde_json::Value,
    /// MongoDB-authoritative MCP session and notification store
    pub mcp_sessions: Arc<McpSessionStore>,
    /// JWKS cache for verifying external provider ID tokens (Google)
    pub jwks_cache: Arc<JwksCache>,
    /// FCM push notification auth (None if not configured)
    pub fcm_auth: Option<Arc<FcmAuth>>,
    /// APNs push notification auth (None if not configured)
    pub apns_auth: Option<Arc<ApnsAuth>>,
    /// Shared delivery runtime for connection-expiry notifications.
    pub connection_expiry_notifier:
        Arc<services::connection_expiry_service::ConnectionExpiryNotifier>,
    /// Signed lifecycle delivery runtime for registered developer apps.
    pub developer_webhook_dispatcher:
        Arc<services::developer_webhook_service::DeveloperWebhookDispatcher>,
    /// Versioned encryption keys for AES-256-GCM (current + optional previous for rotation)
    pub encryption_keys: Arc<EncryptionKeys>,
    /// Trusted workload-ticket verification for managed Node bootstrap. The
    /// default remains closed until a real provider is integrated and accepted.
    pub managed_bootstrap_verifier:
        Arc<dyn services::remote_machine_bootstrap::BootstrapTicketVerifier>,
    /// WebSocket connection manager for credential nodes
    pub node_ws_manager: Arc<NodeWsManager>,
    /// Process and address identity used to fence node ownership.
    pub replica_identity: Arc<services::node_owner_service::ReplicaIdentity>,
    /// Cluster-aware dispatch for local and remote credential-node sockets.
    pub node_dispatch: Arc<services::node_dispatch::NodeDispatch>,
    /// Concurrent SSH tunnel session limiter
    pub ssh_session_manager: Arc<SshSessionManager>,
    /// Renewable MongoDB capacity slots shared by cluster-wide session caps.
    pub cluster_slot_manager: services::cluster_slot_service::RenewableSlotManager,
    /// Per-agent rate limiter keyed by API key ID
    pub per_agent_limiter: mw::rate_limit::SharedPerAgentRateLimiter,
    /// Parsed per-user limit for each platform-owned downstream credential.
    pub platform_user_rate_limit: mw::rate_limit::PlatformUserRateLimitPolicy,
    /// Per-user request and in-flight limiter for direct assistant streams.
    pub direct_chat_limiter: mw::rate_limit::SharedDirectChatRateLimiter,
    /// Per-device-code public key limiter for headless provisioning.
    pub device_code_pubkey_limiter: mw::rate_limit::SharedPerPubkeyRateLimiter,
    /// Per-IP limiter for `/api/v1/devices/code/*`.
    pub device_code_ip_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Per-IP limiter for `POST /api/v1/auth/device/request` (5/min).
    pub auth_device_request_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Password changes: 20 attempts/IP/minute and 5 attempts/account/minute.
    pub password_change_ip_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    pub password_change_user_limiter: mw::rate_limit::SharedPerKeyRateLimiter,
    /// Per-IP limiter for `POST /api/v1/auth/device/poll` (60/min).
    pub auth_device_poll_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Per-IP limiter for `POST /api/v1/auth/device/approve` (10/min).
    pub auth_device_approve_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Per-user limiter for `POST /api/v1/auth/device/approve` (10/5min).
    pub auth_device_approve_per_user_limiter: mw::rate_limit::SharedPerKeyRateLimiter,
    /// Per-IP limiter for `POST /api/v1/auth/device/preview` (30/min).
    pub auth_device_preview_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    pub auth_agent_key_request_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    pub auth_agent_key_poll_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    pub auth_agent_key_approve_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    pub auth_agent_key_approve_per_user_limiter: mw::rate_limit::SharedPerKeyRateLimiter,
    pub auth_agent_key_preview_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Per-creator limiter for `POST /api/v1/connect-links` (10/min).
    pub connect_link_create_limiter: mw::rate_limit::SharedPerKeyRateLimiter,
    /// Per-IP limiter for public connect-link previews (30/min).
    pub connect_link_preview_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Per-IP limiter for connect-link completion attempts (30/min).
    pub connect_link_complete_limiter: mw::rate_limit::SharedPerIpRateLimiter,
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
    /// Dedicated per-IP limiter for anonymous public proxy requests.
    pub public_proxy_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Dedicated per-IP limiter for anonymous public MCP requests.
    pub public_mcp_limiter: mw::rate_limit::SharedPerIpRateLimiter,
    /// Effective broker rollout policy. Loaded from MongoDB at startup with
    /// env-derived `AppConfig` values as defaults, then refreshed by admin
    /// settings writes. Fresh or already-strict gates use the local snapshot;
    /// a stale permissive gate requires one successful live read.
    pub broker_policy: Arc<std::sync::RwLock<CachedBrokerPolicy>>,
    pub upload_retention: services::assistant_upload_retention::Cache,
    /// Server-side HMAC key used to derive `CliPairing.code_hash`.
    /// Lives in process memory only (never persisted), so a MongoDB
    /// snapshot alone doesn't let an attacker brute-force the 32^8
    /// code space offline. Derived at startup from `ENCRYPTION_KEY`
    /// (or `CLI_PAIRING_HMAC_KEY` when set); see
    /// `derive_cli_pairing_hmac_key` in `main.rs`.
    pub cli_pairing_hmac_key: std::sync::Arc<zeroize::Zeroizing<[u8; 32]>>,
    /// Server-side HMAC key used to derive auth-device login code hashes.
    /// Domain-separated from CLI pairing with the `auth-device` label.
    pub auth_device_hmac_key: std::sync::Arc<zeroize::Zeroizing<[u8; 32]>>,
    /// Server-side HMAC key used for tamper-evident audit-log hash chaining.
    /// Lives in process memory only and is never persisted to MongoDB.
    pub audit_chain_hmac_key: std::sync::Arc<zeroize::Zeroizing<[u8; 32]>>,
    /// Server-side HMAC key for tamper-evident billing-ledger hash chaining.
    /// Domain-separated from the audit chain with the `billing-ledger` label.
    pub billing_ledger_hmac_key: std::sync::Arc<zeroize::Zeroizing<[u8; 32]>>,
    /// Per-channel rate limiter keyed by conversation_id, for the HTTP Event
    /// Gateway (NyxID#221). Distinct from `per_agent_limiter`.
    pub per_channel_event_limiter: mw::rate_limit::SharedPerChannelEventLimiter,
    /// Per-conversation admission for unsolicited channel messages.
    pub per_conversation_initiate_limiter: mw::rate_limit::SharedPerChannelEventLimiter,
    /// Per-upstream-message edit limiter for progressive channel relay edits.
    pub per_message_edit_limiter: mw::rate_limit::SharedPerMessageEditRateLimiter,
    /// Per-trigger token bucket for public trigger ingress.
    pub per_trigger_limiter: mw::rate_limit::SharedPerChannelEventLimiter,
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
    /// Billing meter facade. P1 writes durable usage ledger rows only.
    pub billing: Arc<services::billing::BillingService>,
    /// Vendor-neutral telemetry client. `None` when no DSN is configured
    /// (the default hard-off state — see `docs/TELEMETRY.md` §3).
    pub telemetry: Option<Arc<telemetry::TelemetryClient>>,
    pub audit_event_types: Arc<services::admin_audit_service::EventTypeCache>,
    /// Live assistant changes from this process's MongoDB change stream:
    /// resumes chats waiting on connect links and new bots at once, and
    /// pushes thread changes to open browsers.
    pub assistant_live: Arc<services::assistant_live::AssistantLive>,
    /// Local rollout hint only; live admission integration follows Phase B.
    pub execution_enrollment: services::execution_permission_enrollment::snapshot::Cache,
}

impl AppState {
    fn cached_broker_policy(&self) -> std::sync::RwLockReadGuard<'_, CachedBrokerPolicy> {
        self.broker_policy.read().unwrap_or_else(|poisoned| {
            tracing::error!("broker policy lock poisoned; checking snapshot age");
            poisoned.into_inner()
        })
    }

    /// Last observed settings for administrative display, not enforcement.
    pub fn broker_policy(&self) -> BrokerPolicy {
        self.cached_broker_policy().policy
    }

    /// Ignore one older revision; confirm a reset with a second successful read.
    pub fn set_broker_policy_if_fresh(&self, policy: BrokerPolicy) -> bool {
        self.set_broker_policy_verified_at(policy, tokio::time::Instant::now())
    }

    fn set_broker_policy_verified_at(
        &self,
        policy: BrokerPolicy,
        verified_at: tokio::time::Instant,
    ) -> bool {
        let mut current = self.broker_policy.write().unwrap_or_else(|poisoned| {
            tracing::error!("broker policy lock poisoned while updating; checking snapshot");
            poisoned.into_inner()
        });
        current.is_fresh();
        // A confirmed store reset invalidates every read begun before its
        // confirming read, including a delayed response with a higher revision.
        if current
            .reset_read_started_at
            .is_some_and(|reset| verified_at < reset)
        {
            return false;
        }
        if policy.revision < current.policy.revision {
            // A read started before the newest observation cannot confirm a reset.
            if verified_at < current.last_read_started_at {
                return false;
            }
            let confirmed = current
                .pending_reset
                .is_some_and(|(revision, finished_at)| {
                    revision == policy.revision && verified_at >= finished_at
                });
            current.last_read_started_at = verified_at;
            if !confirmed {
                current.pending_reset = Some((policy.revision, tokio::time::Instant::now()));
                return false;
            }
            current.reset_read_started_at = Some(verified_at);
            tracing::warn!(target: "nyxid::broker_policy", previous_revision = current.policy.revision,
                reset_revision = policy.revision, "Confirmed broker rollout policy revision reset");
        }
        current.last_read_started_at = current.last_read_started_at.max(verified_at);
        current.pending_reset = None;
        let changed = current.policy != policy;
        current.policy = policy;
        current.verified_at = if changed {
            verified_at
        } else {
            current.verified_at.max(verified_at)
        };
        current.report_recovery_if_fresh();
        changed
    }

    async fn refresh_broker_policy(&self) -> errors::AppResult<bool> {
        self.cached_broker_policy().is_fresh();
        let started_at = tokio::time::Instant::now();
        match services::platform_settings_service::load_broker_policy(&self.db, &self.config).await
        {
            Ok(policy) => Ok(self.set_broker_policy_verified_at(policy, started_at)),
            Err(error) => {
                self.broker_policy
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .pending_reset = None;
                self.cached_broker_policy().is_fresh();
                Err(error)
            }
        }
    }

    async fn broker_requirement(
        &self,
        required: fn(&BrokerPolicy) -> bool,
    ) -> errors::AppResult<bool> {
        {
            let cached = self.cached_broker_policy();
            if cached.is_fresh() || required(&cached.policy) {
                return Ok(required(&cached.policy));
            }
        }
        // At most one inline load per decision. Concurrent callers may each read;
        // the successful refresh makes subsequent decisions local again.
        self.refresh_broker_policy()
            .await
            .map_err(|_| errors::AppError::OAuthAuthorityUnavailable)?;
        let cached = self.cached_broker_policy();
        if cached.is_fresh() || required(&cached.policy) {
            Ok(required(&cached.policy))
        } else {
            // A rejected older revision or a read lasting beyond the age bound
            // cannot authorize using the still-permissive cached value.
            Err(errors::AppError::OAuthAuthorityUnavailable)
        }
    }

    fn code_broker_requirements(&self) -> services::oauth_service::CodeBrokerRequirements {
        use services::oauth_service::CodeBrokerRequirements;
        let cached = self.cached_broker_policy();
        let policy = &cached.policy;
        if cached.is_fresh() {
            CodeBrokerRequirements::known(
                policy.broker_require_admin_capability,
                policy.broker_require_sender_constraint,
            )
        } else {
            CodeBrokerRequirements {
                require_admin_capability: policy.broker_require_admin_capability.then_some(true),
                require_sender_constraint: policy.broker_require_sender_constraint.then_some(true),
            }
        }
    }

    async fn refresh_code_broker_requirements(
        &self,
    ) -> services::oauth_service::CodeBrokerRequirements {
        // The exchange polls this future only when an unknown gate matters to
        // its loaded client. Failure leaves that gate unknown and the code unused.
        let _ = self.refresh_broker_policy().await;
        self.code_broker_requirements()
    }

    pub async fn broker_require_sender_constraint(&self) -> errors::AppResult<bool> {
        self.broker_requirement(|policy| policy.broker_require_sender_constraint)
            .await
    }

    pub async fn broker_require_admin_capability(&self) -> errors::AppResult<bool> {
        self.broker_requirement(|policy| policy.broker_require_admin_capability)
            .await
    }
}

fn spawn_broker_policy_refresh_task(state: AppState) {
    services::shutdown::process().spawn_worker(async move {
        loop {
            match state.refresh_broker_policy().await {
                Ok(changed) => {
                    if changed {
                        let policy = state.broker_policy();
                        tracing::info!(
                            broker_policy_revision = policy.revision,
                            broker_require_sender_constraint =
                                policy.broker_require_sender_constraint,
                            broker_require_admin_capability =
                                policy.broker_require_admin_capability,
                            "Refreshed broker rollout policy from platform settings"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "Failed to refresh broker rollout policy from platform settings"
                    );
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(
                BROKER_POLICY_REFRESH_INTERVAL_SECS,
            ))
            .await;
        }
    });
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
    /// Inspect and reconcile durable analytics-erasure operator dependencies.
    TelemetryErasure(telemetry_erasure_cli::ErasureArgs),
    /// Export existing final HMAC keys without changing history; never prints secrets.
    ExportHmacKeyset {
        /// New local file, created exclusively with restricted permissions.
        #[arg(long)]
        path: std::path::PathBuf,
    },
}

async fn load_prepared_hmac_keyset(
    db: &mongodb::Database,
    config: &AppConfig,
) -> crypto::hmac_lifecycle::HmacKeyset {
    // Existing final domain values are pinned independently from mutable RSA
    // and encryption sources. A file is authoritative: never fall back on a
    // parse/read failure, and do not read the active RSA PEM for HMAC then.
    let hmac_keys = if let Some(path) = config.hmac_keyset_path.as_deref() {
        crypto::hmac_lifecycle::HmacKeyset::load(std::path::Path::new(path))
            .expect("Failed to load pinned HMAC keyset")
    } else {
        let pem = zeroize::Zeroizing::new(
            std::fs::read(&config.jwt_private_key_path)
                .expect("Failed to read legacy JWT HMAC source"),
        );
        tracing::warn!(
            "Legacy HMAC sources remain coupled; export and deploy a pinned HMAC keyset before rotating JWT or encryption keys"
        );
        legacy_hmac_keyset(config, &pem)
    };
    let legacy_for_adoption = if services::hmac_lifecycle_service::is_prepared(db)
        .await
        .expect("Failed to read HMAC preparation state")
    {
        None
    } else {
        let pem = zeroize::Zeroizing::new(
            std::fs::read(&config.jwt_private_key_path)
                .expect("Failed to read legacy HMAC preparation source"),
        );
        Some(legacy_hmac_keyset(config, &pem))
    };
    services::hmac_lifecycle_service::ensure_consistent(
        db,
        &hmac_keys,
        legacy_for_adoption.as_ref(),
    )
    .await
    .expect("HMAC lifecycle consistency check failed");
    drop(legacy_for_adoption);
    hmac_keys
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--attachment-worker") {
        services::attachment_extraction::worker();
        return;
    }
    server_main(Cli::parse());
}

fn server_main(cli: Cli) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Cannot start backend runtime: {error}");
            std::process::exit(1);
        }
    };
    runtime.block_on(server_run(cli));
    if services::shutdown::process().is_draining() {
        // Blocking parsers/password hashes cannot be aborted once running. They
        // must not extend an explicitly bounded process shutdown.
        runtime.shutdown_timeout(std::time::Duration::ZERO);
    }
}

async fn server_run(cli: Cli) {
    // Pick a rustls `CryptoProvider` explicitly before ANY TLS use.
    // Feature unification can compile multiple providers into the
    // backend (notably aws_lc_rs and ring), and rustls cannot
    // auto-select in that state. The CLI uses the same pattern; see
    // cli/src/main.rs for the canonical version.
    install_rustls_crypto_provider();

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
    turn_trace::init();

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

    services::assistant_engine_worker::initialize();

    // Load configuration
    let mut config = AppConfig::from_env();
    config.validate_ssh_runtime_config();
    config.validate_cluster_runtime_config();
    if config.trusted_proxy_ips.is_empty() {
        tracing::warn!(
            "TRUSTED_PROXY_IPS is empty; forwarded client IPs and Cloudflare country headers cannot be verified. Requester attribution will be unavailable when the observed peer is an internal proxy, and strict public per-IP limits may collapse to that proxy address."
        );
    }

    // Connect to database
    let db = db::create_connection(&config)
        .await
        .expect("Failed to connect to database");
    // Preparation reads the unchanged legacy bytes and bounded chain evidence.
    // It neither loads a signer nor starts normal server writers, so an older
    // weak signer can be exported before the strict JWT cutover.
    if let Some(Commands::ExportHmacKeyset { path }) = post_db_command.as_ref() {
        let keys = load_prepared_hmac_keyset(&db, &config).await;
        keys.export_new(path)
            .expect("Failed to export existing HMAC keyset");
        tracing::info!("Existing HMAC domain keys exported to a restricted file");
        return;
    }
    services::platform_help_service::ensure(&db)
        .await
        .expect("Failed to initialize public help");
    services::assistant_nyxagent::warn_at_startup(&db).await;
    services::assistant_access_policy::ensure_cutover(&db)
        .await
        .expect("Failed to read the immutable assistant skill-routing cutover");

    // Load JWT signing keys early: DB-backed CLI subcommands may audit-log
    // before the server state is built, and the audit-chain key can fall back
    // to the JWT private key in KMS deployments.
    let jwt_keys = JwtKeys::from_config(&config).expect("Failed to load JWT keys");
    tracing::info!("JWT signing and verification keys loaded");

    // Publish the exact validated startup snapshot; do not reread a PEM that
    // could have changed independently of the active signer after validation.
    let jwk_json = jwt_keys
        .public_jwk
        .clone()
        .expect("validated active JWT public key");

    let hmac_keys = load_prepared_hmac_keyset(&db, &config).await;
    let cli_pairing = hmac_keys.cli_pairing().clone();
    let auth_device = hmac_keys.auth_device().clone();
    let audit_chain = hmac_keys.audit_chain().clone();
    let action_fingerprint = hmac_keys.assistant_action().clone();
    let billing_ledger = hmac_keys.billing_ledger().clone();
    let internal_dispatch_hmac_key = hmac_keys.internal_dispatch().clone();
    let cli_pairing_hmac_key = Arc::new(cli_pairing);
    let auth_device_hmac_key = Arc::new(auth_device);
    services::audit_service::init_audit_chain_hmac_key(audit_chain.clone());
    let audit_chain_hmac_key = Arc::new(audit_chain);
    services::assistant_action_receipts::init_action_fingerprint_hmac_key(action_fingerprint);
    services::billing::ledger::init_billing_ledger_hmac_key(billing_ledger.clone());
    let billing_ledger_hmac_key = Arc::new(billing_ledger);
    services::billing::exact_migration::spawn(db.clone(), config.billing_exact_cutover_drained);
    services::billing::account_reconciliation::spawn(
        db.clone(),
        config.billing_enabled,
        config.chain_verify_interval_secs,
    );
    services::chain_verify_service::spawn_chain_verify_worker(
        db.clone(),
        audit_chain_hmac_key.clone(),
        billing_ledger_hmac_key.clone(),
        config.chain_verify_interval_secs,
    );

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
    if let Some(Commands::TelemetryErasure(args)) = post_db_command {
        if let Err(error) = telemetry_erasure_cli::run(&db, args).await {
            eprintln!("Erasure reconciliation failed: {error}");
            std::process::exit(1);
        }
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
        ek.set_allow_legacy_record_reads(config.allow_legacy_record_reads);
        ek.set_contextual_record_writes(config.contextual_record_writes);
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

    if services::voice::catalog::seed(&db, &config.base_url)
        .await
        .is_err()
    {
        tracing::error!(
            slug = services::voice::access::SERVICE_SLUG,
            "Failed to seed NyxBot Voice catalog service; continuing startup"
        );
    }

    services::inference_service::backfill(&db)
        .await
        .expect("Failed to backfill inference metadata");
    services::utility_inference_service::seed(&db)
        .await
        .expect("Failed to seed utility inference defaults");

    services::retired_service_service::retire_legacy_vendors(&db)
        .await
        .expect("Failed to retire legacy vendor credential stores");

    Box::pin(services::domain_alias_service::normalize_hosted_specs(
        &db, &config,
    ))
    .await
    .expect("Failed to normalize hosted catalog spec URLs");

    // Materialize ServiceEndpoint rows for seeded catalog services from the
    // hosted overlay specs so /api/v1/mcp/config publishes concrete
    // service_id + endpoint_id operations for workflow consumers (#1290).
    services::catalog_spec_sync::sync_seeded_service_endpoints(&db)
        .await
        .expect("Failed to sync seeded catalog spec endpoints");

    // Auto-run endpoint discovery for admin catalog services that have an
    // openapi_spec_url but never had discover-endpoints run (#1290
    // follow-up). Remote spec fetches run in the background so a slow or
    // unreachable URL never blocks startup.
    {
        let db = db.clone();
        services::shutdown::process().spawn_worker(async move {
            if let Err(error) =
                services::catalog_spec_sync::sync_spec_backed_service_endpoints(&db).await
            {
                tracing::error!(%error, "Spec-backed endpoint sync sweep failed");
            }
        });
    }

    // Heal UserService rows whose `auth_method` was snapshotted as the raw
    // catalog `"none"` instead of the SPR-derived injection config, which
    // stops the proxy from injecting the caller's stored credential.
    services::user_service_service::backfill_stale_catalog_auth_snapshots(&db)
        .await
        .expect("Failed to backfill stale UserService auth_method snapshots");

    // Remove org-owned public platform rows created by the pre-0.26.1
    // provisioning bug. The sweep deletes orphan resources before each row so
    // retries work on standalone MongoDB too. Personal rows, explicit bindings,
    // and restricted grants are untouched; cleanup failures do not stop startup.
    if let Err(error) =
        services::user_service_service::cleanup_public_org_auto_provisions(&db).await
    {
        tracing::warn!(%error, "Failed to clean up stale public platform org auto-provisions");
    }

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
    if let Err(e) = db::migrate_to_unified_collections(&db, &encryption_keys).await {
        tracing::warn!("Unified collection migration encountered errors: {e}");
    }

    // Live proxy resolution follows the catalog while bounded background
    // repair refreshes stored snapshots after collection migration.
    services::catalog_endpoint_service::spawn_reconcile(db.clone(), None);

    // Retire pre-upgrade owner asks in bounded pending-row batches. The sweep is
    // idempotent; failures retry on policy writes or the next startup.
    if services::assistant_acknowledgement_service::retire_full_access_confirmations(
        &db, None, None,
    )
    .await
    .is_err()
    {
        tracing::error!(
            code = "assistant_full_access_retirement_failed",
            "Failed to retire obsolete Full-access confirmations; continuing startup"
        );
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

    // Create a shared reqwest client for connection reuse.
    // Use connect_timeout (not global timeout) so SSE streaming responses
    // from LLM services are not killed after 30 seconds.
    let http_client = reqwest::Client::builder()
        .redirect(services::telegram_new_api::app_redirect_policy())
        .connect_timeout(std::time::Duration::from_secs(10))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .build()
        .expect("Failed to create HTTP client");

    // Create MCP session store with MongoDB persistence
    let mcp_sessions = Arc::new(McpSessionStore::with_db_options(
        db.clone(),
        std::time::Duration::from_secs(config.mcp_notification_ttl_secs),
        std::time::Duration::from_millis(config.mcp_notification_poll_interval_ms),
    ));

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
    let replica_identity = Arc::new(services::node_owner_service::ReplicaIdentity::new(
        config.instance_name.clone(),
        config.internal_advertise_url.clone(),
    ));
    // Register before serving. Reuse the process generation, not node socket
    // heartbeats: a replica can enforce policy without owning any node sockets.
    services::security_readiness_service::ensure_indexes(&db)
        .await
        .expect("security replica indexes failed");
    let readiness_runtime = services::security_readiness_service::Runtime::new(
        (*replica_identity).clone(),
        config.security_rollout.clone(),
        services::proxy_service::actor_addressed_policy_required(),
    );
    services::security_readiness_service::initialize(readiness_runtime.clone());
    services::security_readiness_service::observe_with_automation(
        &db,
        &readiness_runtime,
        config.automation_continuation_v2_enabled,
    )
    .await
    .expect("security replica observation failed");
    let readiness_db = db.clone();
    let automation_enabled = config.automation_continuation_v2_enabled;
    services::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            if let Err(error) = services::security_readiness_service::observe_with_automation(
                &readiness_db,
                &readiness_runtime,
                automation_enabled,
            )
            .await
            {
                tracing::error!(%error, "Security replica observation failed");
            }
        }
    });
    let internal_auth = services::internal_auth::InternalAuth::new(
        db.clone(),
        internal_dispatch_hmac_key,
        std::time::Duration::from_secs(config.internal_auth_max_skew_secs),
        std::time::Duration::from_secs(config.internal_nonce_ttl_secs),
    );
    let node_dispatch = Arc::new(services::node_dispatch::NodeDispatch::new(
        db.clone(),
        node_ws_manager.clone(),
        replica_identity.clone(),
        http_client.clone(),
        internal_auth,
        services::node_ws_manager::node_proxy_ws_message_size_limit(config.proxy_max_body_size),
        std::time::Duration::from_secs(config.internal_duplex_handshake_timeout_secs),
    ));
    node_ws_manager.attach_cluster_dispatch(Arc::downgrade(&node_dispatch));
    services::coordination_service::initialize_cluster_lease_runtime(
        services::coordination_service::ClusterLeaseRuntime::new(
            models::coordination::CoordinationHolder {
                instance_id: replica_identity.instance_name.clone(),
                generation_id: replica_identity.generation_id.clone(),
            },
            std::time::Duration::from_secs(config.cluster_lease_ttl_secs),
            std::time::Duration::from_secs(config.cluster_lease_renew_secs),
        ),
    );
    let cluster_slot_manager = services::cluster_slot_service::RenewableSlotManager::new(
        db.clone(),
        replica_identity.coordination_holder(),
        std::time::Duration::from_secs(config.cluster_slot_ttl_secs),
        std::time::Duration::from_secs(config.cluster_slot_renew_secs),
    );
    let ssh_session_manager = Arc::new(SshSessionManager::new(
        cluster_slot_manager.clone(),
        config.ssh_max_sessions_per_user,
    ));

    // HTTP Event Gateway state (NyxID#221).
    let per_channel_event_limiter = Arc::new(mw::rate_limit::PerChannelEventLimiter::with_db(
        db.clone(),
        "channel_event",
        config.channel_event_rate_limit_per_second,
        config.channel_event_rate_limit_burst,
    ));
    let per_trigger_limiter = Arc::new(mw::rate_limit::PerChannelEventLimiter::with_db(
        db.clone(),
        "trigger_ingress",
        config.trigger_rate_limit_per_second,
        config.trigger_rate_limit_burst,
    ));
    let per_message_edit_limiter = Arc::new(mw::rate_limit::PerMessageEditRateLimiter::with_db(
        db.clone(),
        "channel_message_edit",
        config.channel_relay_edit_rate_limit_per_second,
        config.channel_relay_edit_rate_limit_burst,
    ));

    let per_conversation_initiate_limiter =
        Arc::new(mw::rate_limit::PerChannelEventLimiter::with_db(
            db.clone(),
            "channel_initiate",
            config.channel_relay_initiate_rate_limit_per_second,
            config.channel_relay_initiate_rate_limit_burst,
        ));

    // Create shared state
    services::billing::sandbox_rates::seed(
        &db,
        &config.lago_plan_code,
        audit_chain_hmac_key.as_slice(),
    )
    .await
    .expect("Failed to seed initial CMA rate card");
    let billing = Arc::new(services::billing::BillingService::new(
        db.clone(),
        Arc::new(config.clone()),
    ));
    if config.billing_enabled {
        if billing.lago_configured() {
            match billing.backfill_existing_owner_wallets().await {
                Ok(stats) => tracing::info!(
                    scanned = stats.scanned,
                    provisioned = stats.provisioned,
                    existing = stats.existing,
                    failed = stats.failed,
                    "Billing wallet backfill complete"
                ),
                Err(error) => {
                    tracing::warn!(error = %error, "Billing wallet backfill failed");
                }
            }
        } else {
            tracing::warn!(
                "BILLING_ENABLED=true but Lago is not configured; billing wallet backfill skipped"
            );
        }
    }
    let broker_policy_verified_at = tokio::time::Instant::now();
    let broker_policy = services::platform_settings_service::load_broker_policy(&db, &config)
        .await
        .expect("Failed to load platform broker policy");
    let developer_webhook_dispatcher = Arc::new(
        services::developer_webhook_service::DeveloperWebhookDispatcher::new(
            http_client.clone(),
            encryption_keys.clone(),
        ),
    );
    services::channel_bot_health::install_failure_notifier(
        services::channel_bot_health::ChannelBotFailureNotifier::new(
            Arc::new(config.clone()),
            http_client.clone(),
            fcm_auth.clone(),
            apns_auth.clone(),
        ),
    );
    let connection_expiry_notifier = Arc::new(
        services::connection_expiry_service::ConnectionExpiryNotifier::new(
            Arc::new(config.clone()),
            http_client.clone(),
            fcm_auth.clone(),
            apns_auth.clone(),
            Some(developer_webhook_dispatcher.clone()),
        ),
    );
    // Erasure is independent of current capture settings and uses authenticated
    // vendor APIs. Historical destinations remain in the durable registry.
    let erasure_clients = telemetry::erasure::ErasureClients::from_env()
        .expect("Invalid telemetry erasure configuration");
    services::telemetry_erasure_service::register_destinations(&db, &erasure_clients)
        .await
        .expect("Telemetry erasure destination registration failed");

    let execution_enrollment =
        crate::services::execution_permission_enrollment::snapshot::Cache::for_database(&db);
    let state = AppState {
        shutdown: services::shutdown::process(),
        #[cfg(test)]
        local_machine_platform: None,
        db: db.clone(),
        config: config.clone(),
        jwt_keys,
        http_client: http_client.clone(),
        jwk_json,
        mcp_sessions: mcp_sessions.clone(),
        jwks_cache,
        fcm_auth: fcm_auth.clone(),
        apns_auth: apns_auth.clone(),
        connection_expiry_notifier,
        developer_webhook_dispatcher,
        encryption_keys: encryption_keys.clone(),
        managed_bootstrap_verifier:
            services::remote_machine_bootstrap_verifier::configured_verifier(),
        node_ws_manager,
        replica_identity,
        node_dispatch,
        ssh_session_manager,
        cluster_slot_manager: cluster_slot_manager.clone(),
        per_agent_limiter: Arc::new(mw::rate_limit::PerAgentRateLimiter::with_db(
            db.clone(),
            "agent",
        )),
        platform_user_rate_limit: mw::rate_limit::PlatformUserRateLimitPolicy::new(
            config.platform_service_rate_limit_per_second,
            config.platform_service_rate_limit_burst,
        ),
        direct_chat_limiter: mw::rate_limit::create_direct_chat_rate_limiter(
            db.clone(),
            cluster_slot_manager,
        ),
        device_code_pubkey_limiter: mw::rate_limit::create_per_pubkey_rate_limiter(
            db.clone(),
            "device_code_pubkey",
        ),
        device_code_ip_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "device_code_ip",
            5,
            60,
        ),
        auth_device_request_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_device_request",
            5,
            60,
        ),
        password_change_ip_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "password_change_ip",
            20,
            60,
        ),
        password_change_user_limiter: mw::rate_limit::create_per_key_rate_limiter(
            db.clone(),
            "password_change_user",
            5,
            60,
        ),
        auth_device_poll_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_device_poll",
            60,
            60,
        ),
        auth_device_approve_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_device_approve",
            10,
            60,
        ),
        auth_device_approve_per_user_limiter: mw::rate_limit::create_per_key_rate_limiter(
            db.clone(),
            "auth_device_approve_user",
            10,
            300,
        ),
        auth_device_preview_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_device_preview",
            30,
            60,
        ),
        auth_agent_key_request_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_agent_key_request",
            5,
            60,
        ),
        auth_agent_key_poll_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_agent_key_poll",
            60,
            60,
        ),
        auth_agent_key_approve_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_agent_key_approve",
            10,
            60,
        ),
        auth_agent_key_approve_per_user_limiter: mw::rate_limit::create_per_key_rate_limiter(
            db.clone(),
            "auth_agent_key_approve_user",
            10,
            300,
        ),
        auth_agent_key_preview_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "auth_agent_key_preview",
            30,
            60,
        ),
        connect_link_create_limiter: mw::rate_limit::create_per_key_rate_limiter(
            db.clone(),
            "connect_link_create",
            10,
            60,
        ),
        connect_link_preview_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "connect_link_preview",
            30,
            60,
        ),
        connect_link_complete_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "connect_link_complete",
            30,
            60,
        ),
        // 5 claim attempts per 60 seconds per IP; window-based, not token
        // bucket, because we want a hard cap on guesses per unit time.
        cli_pairing_claim_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "cli_pairing_claim",
            5,
            60,
        ),
        public_proxy_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "public_proxy",
            config.public_proxy_rate_limit_per_minute,
            60,
        ),
        public_mcp_limiter: mw::rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "public_mcp",
            config.public_mcp_rate_limit_per_minute,
            60,
        ),
        broker_policy: Arc::new(std::sync::RwLock::new(CachedBrokerPolicy::new(
            broker_policy,
            broker_policy_verified_at,
        ))),
        upload_retention: Default::default(),
        cli_pairing_hmac_key,
        auth_device_hmac_key,
        audit_chain_hmac_key,
        billing_ledger_hmac_key,
        per_channel_event_limiter,
        per_message_edit_limiter,
        per_conversation_initiate_limiter,
        per_trigger_limiter,
        token_exchange_cache: Arc::new(TokenExchangeCache::new()),
        cloud_response_cache: Arc::new(
            services::cloud_response_cache::CloudResponseCache::with_bounds(
                config.cloud_response_cache_ttl_secs,
                config.cloud_response_cache_max_entry_bytes,
                config.cloud_response_cache_max_entries,
            ),
        ),
        billing,
        telemetry: telemetry::TelemetryClient::from_config(&config, Some(db.clone())),
        audit_event_types: Arc::default(),
        assistant_live: Arc::default(),
        execution_enrollment,
    };

    services::personal_erasure_service::ensure_indexes(&state.db)
        .await
        .expect("Personal erasure indexes");
    let _personal_erasure_worker = services::personal_erasure_service::spawn_worker(state.clone());
    services::telemetry_erasure_service::spawn_worker(state.db.clone(), erasure_clients);
    let _usage_rollup_worker =
        services::billing::usage_rollup::spawn_worker(state.db.clone(), Arc::new(config.clone()));
    let _billing_reconcile_worker = services::billing::reconcile::spawn_reconcile_worker(
        state.billing.reconciler(),
        config.billing_reconcile_interval_secs,
    );
    state.billing.spawn_refresh_worker();
    services::assistant_upload_retention::refresh(&state.db, &state.upload_retention)
        .await
        .expect("Upload retention policy");
    services::assistant_upload_retention::spawn(state.clone());
    services::assistant_agent_learning_review::start_publication_migration(&state).await;
    services::assistant_agent_learning::spawn(state.clone());
    spawn_broker_policy_refresh_task(state.clone());

    let lifecycle_cleanup_db = state.db.clone();
    services::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                break;
            };
            if let Err(error) =
                services::oracle_pool_service::cleanup_deleted_pools(&lifecycle_cleanup_db).await
            {
                tracing::warn!(%error, "Oracle lifecycle cleanup pass failed");
            }
        }
    });
    let login_cleanup_db = state.db.clone();
    services::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                break;
            };
            if let Err(error) =
                services::oracle_login_profile_service::purge_expired(&login_cleanup_db).await
            {
                tracing::warn!(%error, "Oracle saved login cleanup failed");
            }
        }
    });

    // Create rate limiters
    let global_rate_limiter = mw::rate_limit::create_rate_limiter(
        state.db.clone(),
        config.rate_limit_per_second,
        config.rate_limit_burst,
    );
    let per_ip_rate_limiter = mw::rate_limit::create_per_ip_rate_limiter(
        state.db.clone(),
        "global_ip",
        config.rate_limit_burst, // per-IP max requests per window
        1,                       // 1-second window
    );

    // NyxBot: destroy idle subagents and retry deferred team wake-ups.
    handlers::assistant_team::spawn_sweeps(state.clone());
    services::call_channels::coordinator::spawn_sweep(state.clone());
    services::platform_manager_transport::spawn_sweeps(
        state.clone(),
        services::platform_manager_transport::compose(
            state.clone(),
            handlers::assistant_team::owner_turn_settled_hook(state.clone()),
            handlers::agent_skills::preload_context(state.clone()),
        ),
    );
    handlers::trigger_scheduler::spawn(state.clone());
    // Live assistant changes: one change stream for this process, and the
    // NyxBot reaction to finished links and new bots.
    handlers::nyxbot::spawn_live_dispatch(state.clone());
    handlers::machine_update::spawn(state.clone());
    services::machine_access_service::spawn(state.clone());
    services::remote_machine_manager::spawn(state.clone());
    {
        let live = state.assistant_live.clone();
        let db = state.db.clone();
        services::shutdown::process().spawn_worker(async move { live.run(db).await });
    }

    {
        let cache = state.execution_enrollment.clone();
        let db = state.db.clone();
        services::shutdown::process().spawn_worker(async move { cache.run(db).await });
        services::shutdown::process().spawn_worker(
            services::execution_permission_enrollment::jobs::run(state.db.clone()),
        );
    }

    // Revoke abandoned Agent Key exchanges even when the CLI stops polling.
    if config.agent_key_login_sweep_interval_secs > 0 {
        let sweep_db = state.db.clone();
        let sweep_interval = config.agent_key_login_sweep_interval_secs;
        services::shutdown::process().spawn_worker(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(sweep_interval));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                    break;
                };
                if let Err(error) = services::auth_device_service::sweep_expired(&sweep_db).await {
                    tracing::error!(
                        error_code = error.error_code(),
                        "Device login expiry sweep failed"
                    );
                }
                if let Err(error) =
                    services::auth_agent_key_login_service::sweep_expired(&sweep_db).await
                {
                    tracing::error!(
                        error_code = error.error_code(),
                        "Agent Key login expiry sweep failed"
                    );
                }
            }
        });
    }

    // Expire abandoned app-bound connect links even when their creator has
    // stopped polling. Disabled when the interval is 0.
    if config.connect_link_expiry_sweep_interval_secs > 0 {
        let connect_link_expiry_db = state.db.clone();
        let connect_link_expiry_dispatcher = state.developer_webhook_dispatcher.clone();
        let connect_link_expiry_interval = config.connect_link_expiry_sweep_interval_secs;
        services::shutdown::process().spawn_worker(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(connect_link_expiry_interval));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                    break;
                };
                if let Err(error) = services::connect_link_service::expire_pending_app_links(
                    &connect_link_expiry_db,
                    &connect_link_expiry_dispatcher,
                )
                .await
                {
                    tracing::warn!(%error, "Connect-link expiry sweep error");
                }
            }
        });
    }

    // Bot links use their own sweep so slow receivers cannot delay connector links.
    if config.connect_link_expiry_sweep_interval_secs > 0 {
        let bot_link_state = state.clone();
        let interval_secs = config.connect_link_expiry_sweep_interval_secs;
        services::shutdown::process().spawn_worker(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                    break;
                };
                if let Err(error) = services::channel_connect_link_service::sweep(
                    &bot_link_state.db,
                    &bot_link_state.encryption_keys,
                    &bot_link_state.developer_webhook_dispatcher,
                )
                .await
                {
                    tracing::warn!(%error, "Channel connect-link sweep failed");
                }
            }
        });
    }

    // Budget holds must recover independently of external Telegram notice latency.
    let platform_budget_db = state.db.clone();
    services::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                break;
            };
            if let Err(error) =
                services::platform_inference_service::sweep_abandoned(&platform_budget_db).await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Platform budget recovery sweep failed"
                );
            }
        }
    });

    let platform_cleanup_state = state.clone();
    services::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                break;
            };
            if services::shutdown::process().is_draining() {
                break;
            }
            if let Err(error) =
                handlers::platform_bots::sweep_cleanup(&platform_cleanup_state).await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Platform conversion cleanup sweep failed"
                );
            }
            if services::shutdown::process().is_draining() {
                break;
            }
            if let Err(error) =
                services::platform_provisioning_service::sweep(&platform_cleanup_state).await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Platform setup provisioning sweep failed"
                );
            }
            if services::shutdown::process().is_draining() {
                break;
            }
            if let Err(error) =
                services::platform_setup_notice_service::sweep(&platform_cleanup_state).await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Platform setup status notice sweep failed"
                );
            }
            if services::shutdown::process().is_draining() {
                break;
            }
            if let Err(error) =
                services::platform_native_receipt_service::sweep(&platform_cleanup_state).await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Native platform recovery sweep failed"
                );
            }
            if services::shutdown::process().is_draining() {
                break;
            }
            if let Err(error) =
                services::platform_setup_service::expire(&platform_cleanup_state.db).await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Platform setup expiry sweep failed"
                );
            }
        }
    });

    // Spawn background cleanup task for MCP session reaper.
    // Sessions live up to 30 days (extended on every request via touch()).
    // Reaper runs every 5 minutes to clean up sessions idle longer than 30 days.
    let mcp_sessions_for_reaper = mcp_sessions.clone();
    services::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                break;
            };
            if let Err(error) = mcp_sessions_for_reaper
                .reap_expired(std::time::Duration::from_secs(
                    models::mcp_session::MCP_SESSION_MAX_IDLE_SECS,
                ))
                .await
            {
                tracing::warn!(%error, "MCP session reaper failed");
            }
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
    services::shutdown::process().spawn_worker(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(expiry_interval_secs));
        // Prevent rapid-fire catch-up sweeps when a tick is delayed (e.g. under
        // database backpressure). Burst mode (the default) would fire consecutive
        // sweeps immediately, widening the race window between find() and
        // update_many() in expire_pending_requests.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                break;
            };
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

    // Spawn proactive OAuth token-refresh sweep. Keeps multi-connection
    // OAuth access tokens warm (Google / Lark / GitHub BYO etc.) so a
    // token stays valid even for services that aren't proxied often, and
    // a dead refresh token surfaces as status="failed" promptly instead
    // of on the user's next proxy attempt. Lazy proxy-time refresh still
    // covers everything this misses. Disabled when the interval is 0.
    if config.oauth_refresh_sweep_interval_secs > 0 {
        let refresh_db = state.db.clone();
        let refresh_keys = state.encryption_keys.clone();
        let refresh_notifier = state.connection_expiry_notifier.clone();
        let refresh_interval = config.oauth_refresh_sweep_interval_secs;
        let refresh_window =
            chrono::Duration::seconds(config.oauth_refresh_sweep_window_secs.max(0));
        services::shutdown::process().spawn_worker(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(refresh_interval));
            // Delay catch-up bursts under DB backpressure: a stacked sweep
            // would re-query the same expiring rows and double the refresh
            // calls. One sweep per interval is plenty.
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Skip the immediate first tick so startup isn't competing with
            // index creation / seeding for the connection pool.
            interval.tick().await;
            loop {
                let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                    break;
                };
                if let Err(e) = services::user_token_service::refresh_expiring_oauth_keys(
                    &refresh_db,
                    &refresh_keys,
                    refresh_window,
                    Some(&refresh_notifier),
                )
                .await
                {
                    tracing::warn!("OAuth refresh sweep error: {e}");
                }
            }
        });
    }

    let discord_shutdown = tokio_util::sync::CancellationToken::new();
    let discord_worker = services::shutdown::process().spawn_worker(
        services::discord_gateway::run(state.clone(), discord_shutdown.clone()),
    );

    {
        let poll_state = state.clone();
        let poll_interval = if config.channel_poll_interval_secs > 0 {
            config.channel_poll_interval_secs
        } else {
            60
        };
        services::shutdown::process().spawn_worker(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(poll_interval));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;
            loop {
                let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                    break;
                };
                if services::channel_billing_service::sweep(&poll_state)
                    .await
                    .is_err()
                {
                    tracing::warn!(
                        "Channel subscription cleanup failed; retrying on the next tick"
                    );
                }
                if poll_state.config.channel_poll_interval_secs > 0
                    && services::channel_poll_service::sweep(&poll_state)
                        .await
                        .is_err()
                {
                    tracing::warn!("Channel poll sweep failed; retrying on the next tick");
                }
            }
        });
    }

    {
        let creation_state = state.clone();
        services::shutdown::process().spawn_worker(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                    break;
                };
                let service = services::telegram_new_service::TelegramNewService {
                    manager_id: None,
                    db: &creation_state.db,
                    keys: &creation_state.encryption_keys,
                    config: &creation_state.config,
                    api: services::telegram_new_api::TelegramApi::new(&creation_state.http_client),
                };
                if service.complete_pending_creations().await.is_err() {
                    tracing::warn!("Telegram creation sweep failed; retrying on the next tick");
                }
            }
        });
    }

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
        services::shutdown::process().spawn_worker(async move {
            // Check every 5 minutes
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            // Skip the first tick (we just registered above)
            interval.tick().await;
            loop {
                let Some(_work) = services::shutdown::process().tick(&mut interval).await else {
                    break;
                };
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
        services::shutdown::process().spawn_worker(async move {
            services::telegram_poller::run_polling_loop(polling_state).await;
        });
    }

    // Spawn background heartbeat sweep for node WebSocket connections
    let heartbeat_db = state.db.clone();
    let heartbeat_ws = state.node_ws_manager.clone();
    let heartbeat_identity = state.replica_identity.clone();
    let heartbeat_interval = config.node_heartbeat_interval_secs;
    let heartbeat_timeout = config.node_heartbeat_timeout_secs;
    let node_owner_lease_ttl_secs = config.node_owner_lease_ttl_secs;
    let node_owner_lease_renew_secs = config.node_owner_lease_renew_secs;
    let owner_lease_db = heartbeat_db.clone();
    let owner_lease_ws = heartbeat_ws.clone();
    let owner_lease_identity = heartbeat_identity.clone();
    services::shutdown::process().spawn_worker(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(node_owner_lease_renew_secs));
        loop {
            interval.tick().await;
            handlers::node_ws::node_ws_manager_owner_lease_sweep(
                &owner_lease_db,
                &owner_lease_ws,
                &owner_lease_identity,
                node_owner_lease_ttl_secs,
            )
            .await;
        }
    });
    services::shutdown::process().spawn_worker(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(heartbeat_interval));
        loop {
            interval.tick().await;
            handlers::node_ws::node_ws_manager_heartbeat_sweep(
                &heartbeat_db,
                &heartbeat_ws,
                &heartbeat_identity,
                heartbeat_timeout,
            )
            .await;
        }
    });

    let allowed_origins = configured_cors_origins(&config);
    tracing::info!(
        origins = ?allowed_origins.iter().map(|h| h.to_str().unwrap_or("?")).collect::<Vec<_>>(),
        "CORS allowed origins"
    );

    let cors = private_cors(allowed_origins);

    // Build router — public OAuth routes get open CORS (per RFC 9207),
    // private API routes get restricted CORS (FRONTEND_URL only).
    services::service_history::relay::start(state.db.clone());

    let (public_oauth, private_api) = routes::build_router_with_state(state.clone());

    let csrf_state = state.clone();
    let private_api = private_api.layer(cors).layer(axum_mw::from_fn_with_state(
        csrf_state,
        mw::csrf::browser_csrf_middleware,
    ));
    let device_code_rate_limiters = mw::rate_limit::DeviceCodeRateLimiters {
        per_ip: state.device_code_ip_limiter.clone(),
        per_pubkey: state.device_code_pubkey_limiter.clone(),
        db: Some(state.db.clone()),
        trusted_proxies: Arc::new(state.config.trusted_proxy_ips.clone()),
    };
    let trusted_proxy_ranges = Arc::new(state.config.trusted_proxy_ips.clone());
    let rate_limit_exempt_ips =
        mw::rate_limit::RateLimitExemptIps(Arc::new(state.config.rate_limit_exempt_ips.clone()));

    // Global response-header policy (security headers + the SSE
    // anti-buffering mark) wraps the FULLY MERGED router, so every route
    // inherits it — public OAuth, `/api/v1`, proxy, LLM gateway, `/mcp`.
    // Applying it to one branch, or merging a router after it, would
    // silently exempt those routes.
    let internal_node_dispatch = state.node_dispatch.clone();
    let public_backend_url = state.config.base_url.clone();
    let shutdown_state = state.clone();
    let app = public_oauth
        .merge(private_api)
        .with_state(state)
        .layer(DefaultBodyLimit::max(1_048_576))
        .layer(axum_mw::from_fn_with_state::<
            _,
            _,
            (
                axum::extract::State<mw::rate_limit::DeviceCodeRateLimiters>,
                axum::http::Request<axum::body::Body>,
            ),
        >(
            device_code_rate_limiters,
            mw::rate_limit::device_code_rate_limit_middleware,
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
        .layer(Extension(trusted_proxy_ranges))
        .layer(Extension(rate_limit_exempt_ips))
        .layer(
            TraceLayer::new_for_http().make_span_with(mw::request_trace::span::<axum::body::Body>),
        )
        .layer(axum_mw::from_fn(mw::assistant_trace::assistant_trace));
    // Wrap last so early middleware errors (including rate limits) receive
    // the same deployment-controlled policy as handler responses.
    let app = mw::security_headers::with_response_headers_for_origin(app, &public_backend_url)
        .expect("BASE_URL must be a valid public HTTP(S) URL without credentials")
        .layer(axum_mw::from_fn_with_state(
            shutdown_state.shutdown.clone(),
            mw::shutdown::track,
        ));

    // Bind both listeners before serving. Internal routes never enter the
    // public router and therefore cannot be exposed by an ingress path rule.
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");
    let internal_listener = TcpListener::bind(&config.internal_bind_addr)
        .await
        .expect("Failed to bind internal dispatch address");
    let internal_body_limit =
        services::node_ws_manager::node_proxy_ws_message_size_limit(config.proxy_max_body_size);
    let internal_app = services::node_dispatch::internal_router(
        internal_node_dispatch,
        internal_body_limit,
        std::time::Duration::from_secs(config.internal_duplex_handshake_timeout_secs),
    )
    .layer(axum_mw::from_fn_with_state(
        shutdown_state.shutdown.clone(),
        mw::shutdown::track,
    ));

    tracing::info!("Listening on {addr}");
    tracing::info!(
        bind = %config.internal_bind_addr,
        advertise_url = %config.internal_advertise_url,
        "Internal replica listener started"
    );

    let mut signals = match services::shutdown::signals::Signals::new() {
        Ok(signals) => signals,
        Err(error) => {
            tracing::error!(%error, "Cannot install shutdown signal handlers");
            return;
        }
    };
    let mut servers = shutdown_servers::Servers::new(
        shutdown_state.shutdown.clone(),
        listener,
        internal_listener,
        app,
        internal_app,
    );
    let signal = tokio::select! {
        () = servers.failed() => None,
        signal = signals.next() => Some(signal),
    };
    if let Some(signal) = signal
        && let Some(drain) = services::shutdown::drain::Drain::begin(
            &shutdown_state.shutdown,
            std::time::Duration::from_secs(config.shutdown_drain_timeout_secs),
            signal,
        )
    {
        use services::shutdown::drain::End;
        // Keep the configured timeout a hard exit budget. Short budgets reserve
        // ten percent; ordinary budgets reserve at most five seconds.
        let reserve = drain.cleanup_reserve();
        let work_deadline = drain.work_deadline();
        let end = loop {
            let end = tokio::select! {
                () = servers.failed() => End::ListenerFailure,
                end = drain.wait(&shutdown_state, work_deadline, signals.next()) => end,
            };
            if end != End::Idle {
                break end;
            }
            let idle = tokio::select! {
                biased;
                signal = signals.next() => break drain.second_signal(signal),
                () = tokio::time::sleep_until(work_deadline) => break End::Deadline,
                idle = servers.finish_idle() => idle,
            };
            if idle {
                break End::Idle;
            }
            servers.resume();
        };
        servers.quiesce();
        let cleanup_deadline = drain.deadline.min(tokio::time::Instant::now() + reserve);
        finish_shutdown(
            &shutdown_state,
            &drain,
            end,
            &discord_shutdown,
            cleanup_deadline,
        )
        .await;
        return;
    }
    // Zero preserves the old signal path: no drain state, settlement or release
    // pass, and the same bounded Discord cancellation before runtime teardown.
    drop(servers);
    discord_shutdown.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), discord_worker).await;
}

async fn finish_shutdown(
    state: &AppState,
    drain: &services::shutdown::drain::Drain,
    end: services::shutdown::drain::End,
    discord_shutdown: &tokio_util::sync::CancellationToken,
    deadline: tokio::time::Instant,
) {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    let settle_deadline = tokio::time::Instant::now() + remaining * 2 / 3;
    let release_deadline = deadline - remaining / 10;
    state.shutdown.interrupt();
    // Let the runners use their ordinary settlement and delivery path first.
    let _ = tokio::time::timeout(remaining / 5, state.shutdown.idle()).await;
    state.shutdown.stop();
    discord_shutdown.cancel();
    let stopped = tokio::time::timeout_at(settle_deadline, state.shutdown.execution_stopped())
        .await
        .is_ok();
    if stopped {
        if let Ok(Ok(rows)) = tokio::time::timeout_at(
            settle_deadline,
            services::shutdown::drain::settle_remaining(state),
        )
        .await
        {
            // The transaction already persisted terminal state. Existing hooks
            // publish channel/group/trigger delivery, under the same bound.
            let deliveries = async {
                use futures::StreamExt;
                futures::stream::iter(rows)
                    .for_each_concurrent(16, |(row, error)| {
                        handlers::assistant_team::after_turn_boxed(
                            state.clone(),
                            row,
                            String::new(),
                            Some(error),
                        )
                    })
                    .await;
            };
            let _ = tokio::time::timeout_at(settle_deadline, deliveries).await;
        }
    } else {
        tracing::warn!("Shutdown tasks did not stop before cleanup deadline");
    }
    if stopped
        && !matches!(
            tokio::time::timeout_at(settle_deadline, services::shutdown::release::nodes(state))
                .await,
            Ok(Ok(_))
        )
    {
        tracing::warn!("Shutdown node ownership release unavailable");
    }
    let nodes_stopped =
        tokio::time::timeout_at(settle_deadline, state.shutdown.node_sockets_closed())
            .await
            .is_ok();
    // Never free capacity under an executor which has not stopped.
    let cleanup_stopped = stopped
        && nodes_stopped
        && tokio::time::timeout_at(settle_deadline, state.shutdown.tasks_stopped())
            .await
            .is_ok();
    if cleanup_stopped {
        let _ =
            tokio::time::timeout_at(release_deadline, state.shutdown.close_completion_workers())
                .await;
    }
    let mut releases = if cleanup_stopped {
        services::shutdown::drain::release_owned(state, release_deadline).await
    } else {
        std::collections::BTreeMap::new()
    };
    // The node release path counts the winning write, including a socket
    // teardown racing this coordinator's exact-fence release.
    releases.insert("node_owners", 0);
    // The observation writer is joined even if an unrelated completion task
    // could not finish. Retiring it cannot transfer an external-effect claim.
    if stopped {
        let observation = tokio::time::timeout_at(
            release_deadline,
            services::shutdown::release::observation(&state.db, &state.replica_identity),
        )
        .await;
        releases.insert(
            "security_observation",
            observation.ok().and_then(Result::ok).unwrap_or_default(),
        );
    }
    drain.audit(state, end, releases, deadline).await;
}

/// Preserve each legacy domain, including its historical source-byte convention.
fn legacy_hmac_keyset(config: &AppConfig, pem: &[u8]) -> crypto::hmac_lifecycle::HmacKeyset {
    crypto::hmac_lifecycle::HmacKeyset([
        derive_cli_pairing_hmac_key(
            config.cli_pairing_hmac_key.as_deref(),
            config.encryption_key.as_deref(),
            Some(pem),
        ),
        derive_auth_device_hmac_key(config.encryption_key.as_deref(), Some(pem)),
        services::audit_chain_service::derive_audit_chain_hmac_key(
            config.audit_chain_hmac_key.as_deref(),
            config.encryption_key.as_deref(),
            Some(pem),
        ),
        // These two legacy callers use hex STRING bytes; do not normalize.
        crypto::hmac_keys::derive_hmac_key(
            "assistant-action-fingerprint",
            config.encryption_key.as_deref().map(str::as_bytes),
            pem,
        ),
        services::billing::ledger::derive_billing_ledger_hmac_key(
            config.billing_ledger_hmac_key.as_deref(),
            config.encryption_key.as_deref(),
            Some(pem),
        ),
        services::internal_auth::derive_key(
            config.internal_dispatch_hmac_key.as_deref(),
            config.encryption_key.as_deref().map(str::as_bytes),
            pem,
        ),
    ])
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
    if let Some(master) = decode_hmac_seed(encryption_key) {
        tracing::info!("cli-pairing HMAC key derived from ENCRYPTION_KEY");
        return derive_hmac_key(
            "cli-pairing",
            Some(&master),
            jwt_private_key_pem.unwrap_or_default(),
        );
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
        tracing::info!(
            "cli-pairing HMAC key derived from JWT private key \
             (neither CLI_PAIRING_HMAC_KEY nor ENCRYPTION_KEY is \
             set). Set CLI_PAIRING_HMAC_KEY explicitly if you need \
             to rotate it independently of the JWT signing key."
        );
        return derive_hmac_key("cli-pairing", None, pem);
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

fn derive_auth_device_hmac_key(
    encryption_key: Option<&str>,
    jwt_private_key_pem: Option<&[u8]>,
) -> zeroize::Zeroizing<[u8; 32]> {
    let jwt_private_key_pem = jwt_private_key_pem.unwrap_or_default();
    if let Some(master) = decode_hmac_seed(encryption_key) {
        tracing::info!("auth-device HMAC key derived from ENCRYPTION_KEY");
        return derive_hmac_key("auth-device", Some(&master), jwt_private_key_pem);
    }

    if !jwt_private_key_pem.is_empty() {
        tracing::info!("auth-device HMAC key derived from JWT private key");
        return derive_hmac_key("auth-device", None, jwt_private_key_pem);
    }

    panic!(
        "auth-device HMAC key has no source: ENCRYPTION_KEY and the JWT private key \
         are both missing."
    );
}

fn decode_hmac_seed(encryption_key: Option<&str>) -> Option<zeroize::Zeroizing<Vec<u8>>> {
    let hex_key = encryption_key?;
    let Ok(master) = hex::decode(hex_key.trim()) else {
        return None;
    };
    if master.len() == 32 {
        Some(zeroize::Zeroizing::new(master))
    } else {
        None
    }
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

fn configured_cors_origins(
    config: &AppConfig,
) -> std::collections::HashSet<axum::http::HeaderValue> {
    // Build set of allowed CORS origins: frontend_url + any extra from CORS_ALLOWED_ORIGINS
    let mut allowed_origins: std::collections::HashSet<axum::http::HeaderValue> =
        std::collections::HashSet::new();
    allowed_origins.insert(config.frontend_url.parse().expect("Invalid FRONTEND_URL"));
    for origin in config
        .cors_allowed_origins
        .iter()
        .chain(&config.domain_aliases.frontend_urls)
    {
        if let Ok(hv) = origin.parse() {
            allowed_origins.insert(hv);
        } else {
            tracing::warn!(origin, "Ignoring invalid CORS_ALLOWED_ORIGINS entry");
        }
    }
    allowed_origins
}

fn private_cors(allowed_origins: std::collections::HashSet<axum::http::HeaderValue>) -> CorsLayer {
    CorsLayer::new()
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
            "DPoP".parse().unwrap(),
            "X-User-Email".parse().unwrap(),
            "X-User-Display-Name".parse().unwrap(),
            "X-API-Key".parse().unwrap(),
            "X-NyxID-Client".parse().unwrap(),
            "X-NyxID-Client-Version".parse().unwrap(),
            "X-NyxID-Analytics".parse().unwrap(),
            "X-NyxID-Analytics-Account".parse().unwrap(),
            "DNT".parse().unwrap(),
            "X-NyxID-Account-Deletion-Policy".parse().unwrap(),
        ]))
        .expose_headers([
            axum::http::HeaderName::from_static(mw::assistant_trace::TRACE_HEADER),
            axum::http::HeaderName::from_static(handlers::invocation_diagnostics::HANDLE_HEADER),
            axum::http::HeaderName::from_static(handlers::invocation_diagnostics::EXPIRES_HEADER),
            axum::http::HeaderName::from_static(turn_trace::REQUEST_HEADER),
        ])
        .allow_credentials(true)
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

    #[tokio::test]
    async fn round2_broker_background_refresh_starts_immediately() {
        let (state, reads, _) = broker_round2_fixture().await;
        spawn_broker_policy_refresh_task(state.clone());
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while reads.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the first refresh must not wait five seconds");
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round2_broker_freshness_logs_once_per_transition() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tracing::instrument::WithSubscriber;
        struct Transitions(Arc<AtomicUsize>, Arc<AtomicUsize>);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Transitions {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                if event.metadata().target() == "nyxid::broker_policy" {
                    if *event.metadata().level() == tracing::Level::ERROR {
                        self.0.fetch_add(1, Ordering::SeqCst);
                    }
                    if *event.metadata().level() == tracing::Level::INFO {
                        self.1.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
        }
        let stale = Arc::new(AtomicUsize::new(0));
        let recovered = Arc::new(AtomicUsize::new(0));
        let subscriber =
            tracing_subscriber::registry().with(Transitions(stale.clone(), recovered.clone()));
        let (state, _, _) = broker_round2_fixture().await;
        let mut strict = state.broker_policy();
        strict.broker_require_admin_capability = true;
        strict.broker_require_sender_constraint = true;
        state.set_broker_policy_if_fresh(strict);
        async {
            expire_broker_snapshot(&state);
            for _ in 0..10 {
                assert!(state.broker_require_admin_capability().await.unwrap());
                assert!(state.broker_require_sender_constraint().await.unwrap());
            }
            state.set_broker_policy_if_fresh(strict);
            for _ in 0..10 {
                assert!(state.broker_require_admin_capability().await.unwrap());
            }
        }
        .with_subscriber(subscriber)
        .await;
        assert_eq!(
            stale.load(Ordering::SeqCst),
            1,
            "one error per stale transition"
        );
        assert_eq!(
            recovered.load(Ordering::SeqCst),
            1,
            "one info per recovery transition"
        );
        state.db.drop().await.unwrap();
    }

    async fn broker_round2_fixture() -> (AppState, Arc<std::sync::atomic::AtomicUsize>, String) {
        use mongodb::{
            event::{EventHandler, command::CommandEvent},
            options::ClientOptions,
        };
        let db = test_utils::connect_transaction_test_database("broker_round2").await;
        let app = format!("broker-round2-{}", uuid::Uuid::new_v4());
        let mut options = ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        options.app_name = Some(app.clone());
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = reads.clone();
        options.command_event_handler = Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command.get_str("find") == Ok("platform_settings")
            {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }));
        let client = mongodb::Client::with_options(options).unwrap();
        (
            test_utils::test_app_state(client.database(db.name())),
            reads,
            app,
        )
    }

    fn expire_broker_snapshot(state: &AppState) {
        state.broker_policy.write().unwrap().verified_at =
            tokio::time::Instant::now() - BROKER_POLICY_MAX_AGE;
    }

    pub(crate) static BROKER_FAILPOINT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn round2_stale_permissive_broker_read_failure_is_retryable() {
        let _guard = BROKER_FAILPOINT.lock().await;
        use mongodb::bson::doc;
        let (state, reads, app) = broker_round2_fixture().await;
        let mut high = state.broker_policy();
        high.revision = 10;
        state.set_broker_policy_if_fresh(high);
        let mut low = high;
        low.revision = 0;
        assert!(!state.set_broker_policy_if_fresh(low));
        expire_broker_snapshot(&state);
        state
            .db
            .client()
            .database("admin")
            .run_command(doc! {
                "configureFailPoint":"failCommand", "mode":{"times":2},
                "data":{"failCommands":["find"],"appName":app,"errorCode":13}
            })
            .await
            .unwrap();
        let sender = state.broker_require_sender_constraint().await;
        let admin = state.broker_require_admin_capability().await;
        assert!(
            matches!(sender, Err(errors::AppError::OAuthAuthorityUnavailable)),
            "stale uncertainty must be retryable, got {sender:?}"
        );
        assert!(
            matches!(admin, Err(errors::AppError::OAuthAuthorityUnavailable)),
            "stale uncertainty must be retryable, got {admin:?}"
        );
        let error = sender.unwrap_err();
        assert_eq!(error.error_code(), 12900);
        assert_eq!(
            error.status_code(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(
            !state.set_broker_policy_if_fresh(low),
            "failed loads interrupt reset confirmation"
        );
        assert_eq!(state.broker_policy(), high);
        assert!(state.set_broker_policy_if_fresh(low));
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round2_stale_permissive_broker_reads_once_and_recovers() {
        let (state, reads, _) = broker_round2_fixture().await;
        expire_broker_snapshot(&state);
        assert!(
            !state.broker_require_sender_constraint().await.unwrap(),
            "a healthy permissive store must allow after its inline refresh"
        );
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!state.broker_require_admin_capability().await.unwrap());
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round2_fresh_and_stale_strict_broker_checks_are_local() {
        let (state, reads, _) = broker_round2_fixture().await;
        assert!(!state.broker_require_sender_constraint().await.unwrap());
        assert!(!state.broker_require_admin_capability().await.unwrap());
        let mut strict = state.broker_policy();
        strict.broker_require_sender_constraint = true;
        strict.broker_require_admin_capability = true;
        state.set_broker_policy_if_fresh(strict);
        expire_broker_snapshot(&state);
        assert!(state.broker_require_sender_constraint().await.unwrap());
        assert!(state.broker_require_admin_capability().await.unwrap());
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0);
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round2_ten_stale_broker_requests_do_at_most_ten_reads() {
        let (state, reads, _) = broker_round2_fixture().await;
        expire_broker_snapshot(&state);
        let outcomes =
            futures::future::join_all((0..10).map(|_| state.broker_require_sender_constraint()))
                .await;
        assert!(
            outcomes.iter().all(|result| matches!(result, Ok(false))),
            "{outcomes:?}"
        );
        let count = reads.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            (1..=10).contains(&count),
            "{count} policy reads for ten requests"
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round2_two_consecutive_lower_broker_revisions_follow_store_reset() {
        let (state, _, _) = broker_round2_fixture().await;
        let mut high = state.broker_policy();
        high.revision = 10;
        state.set_broker_policy_if_fresh(high);
        expire_broker_snapshot(&state);
        let age = state.cached_broker_policy().verified_at;
        let mut low = high;
        low.revision = 0;
        assert!(!state.set_broker_policy_if_fresh(low));
        assert_eq!(state.broker_policy(), high);
        assert_eq!(
            state.cached_broker_policy().verified_at,
            age,
            "one stray read does not renew freshness"
        );
        assert!(
            state.set_broker_policy_if_fresh(low),
            "the same lower revision twice confirms the store reset"
        );
        assert_eq!(state.broker_policy(), low);
        assert!(state.cached_broker_policy().verified_at > age);
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn round3_confirmed_reset_rejects_delayed_pre_reset_higher_revision() {
        let state = test_utils::test_app_state_no_db().await;
        tokio::time::pause();
        let mut high = state.broker_policy();
        high.revision = 10;
        state.set_broker_policy_if_fresh(high);
        let normal_read_started = tokio::time::Instant::now();
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        state.set_broker_policy_if_fresh(high);
        high.revision = 11;
        assert!(
            state.set_broker_policy_verified_at(high, normal_read_started),
            "without a reset a higher revision still wins regardless of read start"
        );
        let mut low = high;
        low.revision = 0;
        assert!(!state.set_broker_policy_if_fresh(low));
        let delayed_read_started = tokio::time::Instant::now();
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        assert!(state.set_broker_policy_if_fresh(low));
        let reset_verified_at = state.cached_broker_policy().verified_at;
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        assert!(
            !state.set_broker_policy_verified_at(high, delayed_read_started),
            "a delayed pre-reset read must not undo the confirmed store reset"
        );
        assert_eq!(state.broker_policy(), low);
        assert_eq!(
            state.cached_broker_policy().verified_at,
            reset_verified_at,
            "an ignored pre-reset read must not renew freshness"
        );
        let post_reset_read_started = tokio::time::Instant::now();
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        assert!(!state.set_broker_policy_if_fresh(low));
        high.revision = 12;
        assert!(
            state.set_broker_policy_verified_at(high, post_reset_read_started),
            "a higher revision still wins when its read started after the reset"
        );
        assert_eq!(state.broker_policy(), high);
        tokio::time::resume();
    }

    #[tokio::test]
    async fn broker_reset_confirmation_rejects_overlapping_and_interrupted_reads() {
        let (state, _, _) = broker_round2_fixture().await;
        let mut high = state.broker_policy();
        high.revision = 10;
        state.set_broker_policy_if_fresh(high);
        let old_start = tokio::time::Instant::now();
        let mut low = high;
        low.revision = 0;
        assert!(!state.set_broker_policy_if_fresh(low));
        assert!(!state.set_broker_policy_verified_at(low, old_start));
        assert_eq!(state.broker_policy(), high);
        state.set_broker_policy_if_fresh(high);
        assert!(!state.set_broker_policy_if_fresh(low));
        assert_eq!(state.broker_policy(), high);
        assert!(state.set_broker_policy_if_fresh(low));
        assert_eq!(state.broker_policy(), low);
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn healthy_broker_refreshes_verify_equal_revisions_without_request_reads() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let commands = Arc::new(AtomicUsize::new(0));
        let seen = commands.clone();
        let db = test_utils::connect_transaction_test_database_with_command_handler(
            "healthy_broker_policy",
            mongodb::event::EventHandler::callback(move |event| {
                if matches!(event, mongodb::event::command::CommandEvent::Started(_)) {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
            }),
        )
        .await;
        let state = test_utils::test_app_state(db.clone());
        let policy = services::platform_settings_service::load_broker_policy(&db, &state.config)
            .await
            .unwrap();
        state.set_broker_policy_if_fresh(policy);
        tokio::time::pause();
        let before = commands.load(Ordering::SeqCst);
        for _ in 0..8 {
            tokio::time::advance(std::time::Duration::from_secs(5)).await;
            assert!(
                !state.set_broker_policy_if_fresh(policy),
                "unchanged successful refresh is not a policy change"
            );
            assert!(!state.broker_require_sender_constraint().await.unwrap());
            assert!(!state.broker_require_admin_capability().await.unwrap());
        }
        assert_eq!(
            commands.load(Ordering::SeqCst),
            before,
            "healthy enforcement stays entirely local"
        );
        tokio::time::advance(BROKER_POLICY_MAX_AGE).await;
        let age = state.cached_broker_policy().verified_at;
        let mut rollback = policy;
        rollback.revision -= 1;
        assert!(!state.set_broker_policy_if_fresh(rollback));
        assert_eq!(
            state.cached_broker_policy().verified_at,
            age,
            "a rollback cannot renew freshness"
        );
        state.set_broker_policy_if_fresh(policy);
        assert!(!state.broker_require_sender_constraint().await.unwrap());
        let mut newer = policy;
        newer.revision += 1;
        state.set_broker_policy_verified_at(
            newer,
            tokio::time::Instant::now() - BROKER_POLICY_MAX_AGE,
        );
        assert!(
            !state.cached_broker_policy().is_fresh(),
            "a delayed read retains its observation age"
        );
        assert_eq!(commands.load(Ordering::SeqCst), before);
        tokio::time::resume();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn broker_policy_reload_failure_expires_only_the_unhealthy_replica() {
        let _guard = BROKER_FAILPOINT.lock().await;
        use mongodb::{bson::doc, options::ClientOptions};
        use services::platform_settings_service as settings;
        let db = test_utils::connect_transaction_test_database("broker_policy_age").await;
        let mut options = ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        let app = format!("broker-stale-{}", uuid::Uuid::new_v4());
        options.app_name = Some(app.clone());
        let commands = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let recorded = commands.clone();
        options.command_event_handler =
            Some(mongodb::event::EventHandler::callback(move |event| {
                if matches!(event, mongodb::event::command::CommandEvent::Started(_)) {
                    recorded.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }));
        let client = mongodb::Client::with_options(options).unwrap();
        let a = test_utils::test_app_state(client.database(db.name()));
        let b = test_utils::test_app_state(db.clone());
        assert!(!Arc::ptr_eq(&a.broker_policy, &b.broker_policy));
        let original = settings::load_broker_policy(&a.db, &a.config)
            .await
            .unwrap();
        a.set_broker_policy_if_fresh(original);
        let tightened = settings::update_broker_settings(
            &b.db,
            settings::BrokerSettingsPatch {
                broker_require_sender_constraint: Some(Some(true)),
                broker_require_admin_capability: Some(Some(true)),
            },
        )
        .await
        .unwrap();
        b.set_broker_policy_if_fresh(BrokerPolicy::from_settings(&b.config, &tightened));
        assert!(b.broker_require_sender_constraint().await.unwrap());
        assert!(!a.broker_require_sender_constraint().await.unwrap());
        db.client()
            .database("admin")
            .run_command(doc! {
                "configureFailPoint":"failCommand", "mode":{"times":1},
                "data":{"failCommands":["find"],"appName":&app,"errorCode":13},
            })
            .await
            .unwrap();
        let reload = settings::load_broker_policy(&a.db, &a.config).await;
        assert!(reload.is_err(), "only A's policy reload is rejected");
        assert!(
            a.db.collection::<mongodb::bson::Document>("users")
                .find_one(doc! {})
                .await
                .is_ok()
        );
        expire_broker_snapshot(&a);
        db.client()
            .database("admin")
            .run_command(doc! {
                "configureFailPoint":"failCommand", "mode":{"times":2},
                "data":{"failCommands":["find"],"appName":&app,"errorCode":13},
            })
            .await
            .unwrap();
        let before = commands.load(std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            a.broker_require_sender_constraint().await,
            Err(errors::AppError::OAuthAuthorityUnavailable)
        ));
        assert!(matches!(
            a.broker_require_admin_capability().await,
            Err(errors::AppError::OAuthAuthorityUnavailable)
        ));
        assert_eq!(
            commands.load(std::sync::atomic::Ordering::SeqCst),
            before + 2
        );
        a.set_broker_policy_if_fresh(
            settings::load_broker_policy(&a.db, &a.config)
                .await
                .unwrap(),
        );
        assert_eq!(a.broker_policy(), b.broker_policy());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn hmac_preparation_exports_legacy_2048_pem_without_loading_a_signer() {
        use rsa::{
            RsaPrivateKey, RsaPublicKey,
            pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
        };
        let db = test_utils::connect_test_database("hmac_weak_signer_export")
            .await
            .expect("isolated MongoDB required");
        let dir = tempfile::tempdir().unwrap();
        let private = RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
        let pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
        let private_path = dir.path().join("legacy-private.pem");
        let public_path = dir.path().join("legacy-public.pem");
        std::fs::write(&private_path, pem.as_bytes()).unwrap();
        std::fs::write(
            &public_path,
            RsaPublicKey::from(&private)
                .to_public_key_pem(LineEnding::LF)
                .unwrap(),
        )
        .unwrap();
        let mut config = test_utils::test_app_config();
        config.encryption_key = None;
        config.hmac_keyset_path = None;
        config.jwt_private_key_path = private_path.to_str().unwrap().into();
        config.jwt_public_key_path = public_path.to_str().unwrap().into();
        assert!(
            JwtKeys::from_config(&config).is_err(),
            "normal signing must remain strict"
        );
        let expected = legacy_hmac_keyset(&config, pem.as_bytes());
        let prepared = load_prepared_hmac_keyset(&db, &config).await;
        let exported = dir.path().join("hmac.json");
        prepared.export_new(&exported).unwrap();
        let loaded = crypto::hmac_lifecycle::HmacKeyset::load(&exported).unwrap();
        assert_eq!(loaded.commitments(), expected.commitments());
        for (actual, expected) in loaded.0.iter().zip(expected.0.iter()) {
            assert_eq!(actual.as_slice(), expected.as_slice());
        }
        assert_eq!(
            std::fs::read(&private_path).unwrap(),
            pem.as_bytes(),
            "preparation must never replace old PEM bytes"
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn hmac_adoption_rejects_wrong_first_keys_and_preserves_history_after_source_rotation() {
        use models::{
            audit_log::AuditLog,
            billing_ledger::{BillingLedgerEntry, BillingLedgerEventType},
        };
        use services::{
            audit_chain_service as audit, audit_service, billing::ledger,
            hmac_lifecycle_service as lifecycle,
        };
        let db = test_utils::connect_test_database("hmac_adoption_history")
            .await
            .expect("isolated MongoDB required");
        let mut config = test_utils::test_app_config();
        config.encryption_key = Some("1a".repeat(32));
        config.cli_pairing_hmac_key = None;
        config.audit_chain_hmac_key = None;
        config.billing_ledger_hmac_key = None;
        config.internal_dispatch_hmac_key = None;
        let old = legacy_hmac_keyset(&config, b"legacy-pem-for-hmac-fixture");
        let audit_row = audit::append_chained_entry(
            &db,
            AuditLog {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: Some("retained-owner".into()),
                event_type: "hmac_history_fixture".into(),
                event_data: Some(serde_json::json!({"retained":"immutable"})),
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
                seq: None,
                prev_hash: None,
                entry_hash: None,
                created_at: chrono::Utc::now(),
            },
            old.audit_chain().as_slice(),
        )
        .await
        .unwrap();
        let legacy_billing = ledger::append_chained_entry(
            &db,
            BillingLedgerEntry {
                id: uuid::Uuid::new_v4().to_string(),
                seq: 0,
                prev_hash: String::new(),
                entry_hash: String::new(),
                event_type: BillingLedgerEventType::UsageSettled,
                movement: None,
                postings: Vec::new(),
                owner_id: "retained-owner".into(),
                reference_id: "old-billing".into(),
                transaction_id: None,
                layer: None,
                metric: None,
                service_slug: None,
                model: None,
                quantity: Some(1),
                amount_credits: Some(5),
                amount_micros: None,
                balance_credits: None,
                dedupe_key: None,
                wallet_id: None,
                created_at: chrono::Utc::now(),
            },
            old.billing_ledger().as_slice(),
        )
        .await
        .unwrap();
        let exact_billing = ledger::append_chained_entry(
            &db,
            ledger::exact_entry(
                "retained-owner",
                "exact-billing",
                "hmac_fixture",
                "hmac-exact".into(),
                ledger::transfer(
                    "owner:retained".into(),
                    "platform:retained".into(),
                    models::credits::Credits::from_whole(1),
                ),
                None,
            ),
            old.billing_ledger().as_slice(),
        )
        .await
        .unwrap();
        // Store a pre-adoption fingerprint using the same keyed API as delayed
        // callbacks/instruction identity; source rotation must preserve it.
        let fingerprint = audit_service::keyed_fingerprint(
            Some(old.audit_chain().as_slice()),
            b"assistant-stable-instructions-v1",
            b"pending operation immutable context",
        )
        .unwrap();
        db.collection::<bson::Document>("hmac_pending_fixture")
            .insert_one(bson::doc! {"_id":"pending","fingerprint":&fingerprint})
            .await
            .unwrap();
        let mut changed_config = config.clone();
        changed_config.encryption_key = Some("2b".repeat(32));
        let wrong = legacy_hmac_keyset(&changed_config, b"rotated-pem-for-hmac-fixture");
        // Simulate mistakenly changed sources, even with the same wrong
        // candidate presented as legacy preparation: real history rejects it.
        assert!(
            lifecycle::ensure_consistent(&db, &wrong, Some(&wrong))
                .await
                .is_err()
        );
        assert!(!lifecycle::is_prepared(&db).await.unwrap());
        assert_eq!(
            db.collection::<bson::Document>(models::hmac_keyset::COLLECTION_NAME)
                .count_documents(bson::doc! {})
                .await
                .unwrap(),
            0
        );
        // Billing alone must also reject a bad first candidate after audit
        // verification succeeds.
        let wrong_billing = old.with_changed_domain("billing_ledger");
        assert!(
            lifecycle::ensure_consistent(&db, &wrong_billing, Some(&wrong_billing))
                .await
                .is_err()
        );
        assert!(!lifecycle::is_prepared(&db).await.unwrap());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hmac.json");
        old.export_new(&path).unwrap();
        let adopted = crypto::hmac_lifecycle::HmacKeyset::load(&path).unwrap();
        lifecycle::ensure_consistent(&db, &adopted, Some(&old))
            .await
            .unwrap();
        // Source keys can now change; configured adoption does not derive from
        // them and the durable pin rejects accidental replacement.
        lifecycle::ensure_consistent(&db, &adopted, None)
            .await
            .unwrap();
        assert!(
            lifecycle::ensure_consistent(&db, &wrong, None)
                .await
                .is_err()
        );
        assert_eq!(
            audit::verify_chain(&db, adopted.audit_chain().as_slice(), None, None, None)
                .await
                .unwrap()
                .status,
            audit::AuditChainStatus::Ok
        );
        assert_eq!(
            ledger::verify_chain(&db, adopted.billing_ledger().as_slice(), None, None, None)
                .await
                .unwrap()
                .status,
            ledger::BillingLedgerStatus::Ok
        );
        assert_eq!(
            audit::compute_entry_hash(&audit_row, adopted.audit_chain().as_slice()).unwrap(),
            audit_row.entry_hash.unwrap()
        );
        for row in [legacy_billing, exact_billing] {
            assert_eq!(
                ledger::compute_entry_hash(&row, adopted.billing_ledger().as_slice()),
                row.entry_hash
            );
        }
        let pending = db
            .collection::<bson::Document>("hmac_pending_fixture")
            .find_one(bson::doc! {"_id":"pending"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            pending.get_str("fingerprint").unwrap(),
            audit_service::keyed_fingerprint(
                Some(adopted.audit_chain().as_slice()),
                b"assistant-stable-instructions-v1",
                b"pending operation immutable context"
            )
            .unwrap()
        );
        assert_eq!(old.commitments(), adopted.commitments());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn domain_alias_cors_preflight_only_allows_configured_frontends() {
        use tower::ServiceExt;
        let mut config = crate::test_utils::test_app_config();
        config.frontend_url = "https://new.example".into();
        for aliases in [false, true] {
            config.domain_aliases.frontend_urls = if aliases {
                vec!["https://old.example".into()]
            } else {
                vec![]
            };
            let app = axum::Router::new()
                .route("/test", axum::routing::post(|| async {}))
                .layer(private_cors(configured_cors_origins(&config)));
            for origin in [
                "https://new.example",
                "https://old.example",
                "https://old.example.evil",
            ] {
                let response = app
                    .clone()
                    .oneshot(
                        axum::http::Request::builder()
                            .method("OPTIONS")
                            .uri("/test")
                            .header("Origin", origin)
                            .header("Access-Control-Request-Method", "POST")
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response
                        .headers()
                        .contains_key("access-control-allow-origin"),
                    origin == "https://new.example" || (aliases && origin == "https://old.example")
                );
            }
        }
    }

    #[tokio::test]
    async fn private_cors_allows_analytics_narrowing_header_only_for_trusted_origin() {
        use tower::ServiceExt;
        let app = axum::Router::new()
            .route(
                "/api/v1/users/me/preferences/analytics",
                axum::routing::put(|| async {}),
            )
            .layer(private_cors(std::collections::HashSet::from([
                "https://app.example".parse().unwrap(),
            ])));
        for origin in ["https://app.example", "https://untrusted.example"] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("OPTIONS")
                        .uri("/api/v1/users/me/preferences/analytics")
                        .header("Origin", origin)
                        .header("Access-Control-Request-Method", "PUT")
                        .header(
                            "Access-Control-Request-Headers",
                            "content-type,x-nyxid-analytics,x-nyxid-analytics-account,dnt",
                        )
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            if origin == "https://app.example" {
                assert_eq!(
                    response
                        .headers()
                        .get("access-control-allow-origin")
                        .unwrap(),
                    origin
                );
                let allowed = response
                    .headers()
                    .get("access-control-allow-headers")
                    .unwrap()
                    .to_str()
                    .unwrap();
                assert!(allowed.contains("x-nyxid-analytics"));
                assert!(allowed.contains("x-nyxid-analytics-account"));
                assert!(allowed.contains("dnt"));
            } else {
                assert!(
                    !response
                        .headers()
                        .contains_key("access-control-allow-origin")
                );
            }
        }
    }

    #[tokio::test]
    async fn private_cors_exposes_invocation_diagnostic_headers() {
        use tower::ServiceExt;

        let origin = "https://app.example";
        let app = axum::Router::new()
            .route("/test", axum::routing::get(|| async { "ok" }))
            .layer(private_cors(std::collections::HashSet::from([origin
                .parse()
                .unwrap()])));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/test")
                    .header("Origin", origin)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let exposed = response
            .headers()
            .get("access-control-expose-headers")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(exposed.contains(handlers::invocation_diagnostics::HANDLE_HEADER));
        assert!(exposed.contains(handlers::invocation_diagnostics::EXPIRES_HEADER));
    }

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
