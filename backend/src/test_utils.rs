use std::sync::{Arc, atomic::AtomicUsize};
use std::time::Duration;

use mongodb::bson::doc;
use uuid::Uuid;

use crate::AppState;
use crate::config::AppConfig;
use crate::crypto::aes::EncryptionKeys;
use crate::crypto::jwks::JwksCache;
use crate::crypto::jwt::JwtKeys;
use crate::models::mcp_session::McpSessionStore;
use crate::models::org_membership::{OrgMembership, OrgRole};
use crate::models::user::{User, UserType};
use crate::models::user_endpoint::UserEndpoint;
use crate::models::user_service::UserService;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::event_dedup_cache::EventDedupCache;
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::provider_token_exchange_service::TokenExchangeCache;
use crate::services::ssh_service::SshSessionManager;

/// Connect to a fresh per-test MongoDB database. Tries the docker-compose
/// instance first, then a plain local mongod. Returns `None` if neither is
/// reachable so integration tests can skip cleanly in environments without
/// a running MongoDB.
pub(crate) async fn connect_test_database(prefix: &str) -> Option<mongodb::Database> {
    let db_name = format!("nyxid_test_{prefix}_{}", uuid::Uuid::new_v4());
    let candidates = [
        format!("mongodb://nyxid:nyxid_dev_password@127.0.0.1:27018/{db_name}?authSource=admin"),
        format!("mongodb://127.0.0.1:27017/{db_name}"),
    ];

    for uri in candidates {
        let Ok(client) = mongodb::Client::with_uri_str(&uri).await else {
            continue;
        };
        let db = client.database(&db_name);
        if db.run_command(doc! { "ping": 1 }).await.is_ok() {
            return Some(db);
        }
    }

    None
}

/// Build an `AppConfig` suitable for unit tests that need access to the
/// config's side-effect-free fields (encryption key, limits, feature flags).
pub(crate) fn test_app_config() -> AppConfig {
    AppConfig {
        port: 3001,
        base_url: "http://localhost:3001".to_string(),
        frontend_url: "http://localhost:3000".to_string(),
        cors_allowed_origins: vec![],
        csrf_trusted_origins: vec![],
        database_url: "mongodb://ignored-for-test".to_string(),
        database_max_connections: 10,
        environment: "test".to_string(),
        jwt_private_key_path: "keys/private.pem".to_string(),
        jwt_public_key_path: "keys/public.pem".to_string(),
        jwt_issuer: "nyxid".to_string(),
        jwt_access_ttl_secs: 900,
        jwt_relay_reply_ttl_secs: 1800,
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
        encryption_key: Some("11".repeat(32)),
        encryption_key_previous: None,
        rate_limit_per_second: 10,
        rate_limit_burst: 30,
        trusted_proxy_ips: vec![],
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
        node_heartbeat_interval_secs: 30,
        node_heartbeat_timeout_secs: 90,
        node_proxy_timeout_secs: 30,
        node_registration_token_ttl_secs: 3600,
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
        channel_relay_callback_timeout_secs: 30,
        channel_relay_max_bots_per_user: 5,
        channel_relay_message_ttl_days: 30,
        channel_event_rate_limit_per_second: 100,
        channel_event_rate_limit_burst: 200,
        channel_event_dedup_capacity: 32_768,
        channel_event_dedup_ttl_secs: 300,
        invite_code_required: false,
        email_auth_enabled: false,
        auto_verify_email: false,
        telemetry_dsn: None,
        telemetry_host: None,
        share_analytics: false,
    }
}

pub(crate) fn test_encryption_keys() -> EncryptionKeys {
    EncryptionKeys::from_config(&test_app_config())
}

/// Build a minimal `AppState` for handler tests.
pub(crate) fn test_app_state(db: mongodb::Database) -> AppState {
    let mut config = test_app_config();
    let temp_dir = tempfile::tempdir().expect("create temp dir for jwt keys");
    config.jwt_private_key_path = temp_dir.path().join("private.pem").display().to_string();
    config.jwt_public_key_path = temp_dir.path().join("public.pem").display().to_string();

    let http_client = reqwest::Client::new();
    let jwt_keys = JwtKeys::from_config(&config).expect("build test jwt keys");

    AppState {
        db,
        config: config.clone(),
        jwt_keys,
        http_client: http_client.clone(),
        jwk_json: serde_json::json!({}),
        mcp_sessions: Arc::new(McpSessionStore::new()),
        jwks_cache: Arc::new(JwksCache::new(http_client)),
        fcm_auth: None,
        apns_auth: None,
        encryption_keys: Arc::new(test_encryption_keys()),
        node_ws_manager: Arc::new(NodeWsManager::new(
            config.node_proxy_timeout_secs,
            config.node_max_ws_connections,
        )),
        ssh_session_manager: Arc::new(SshSessionManager::new(config.ssh_max_sessions_per_user)),
        per_agent_limiter: Arc::new(crate::mw::rate_limit::PerAgentRateLimiter::new()),
        // Production default from backend/src/main.rs — 5 claims per
        // 60s per IP; mirror here so claim-rate-limit tests see the
        // same shape.
        cli_pairing_claim_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(5, 60),
        // Tests don't exercise pairing-code HMAC verification; a
        // zero-filled key is deterministic and never touches prod data.
        cli_pairing_hmac_key: Arc::new(zeroize::Zeroizing::new([0u8; 32])),
        per_channel_event_limiter: Arc::new(crate::mw::rate_limit::PerChannelEventLimiter::new(
            config.channel_event_rate_limit_per_second,
            config.channel_event_rate_limit_burst,
        )),
        event_dedup_cache: Arc::new(EventDedupCache::new(
            config.channel_event_dedup_capacity,
            Duration::from_secs(config.channel_event_dedup_ttl_secs),
        )),
        ws_passthrough_count: Arc::new(AtomicUsize::new(0)),
        token_exchange_cache: Arc::new(TokenExchangeCache::new()),
        telemetry: None,
    }
}

/// Build a permissive session-auth `AuthUser` for handler tests.
pub(crate) fn test_auth_user(user_id: &str) -> AuthUser {
    AuthUser {
        user_id: Uuid::parse_str(user_id).expect("valid uuid user id"),
        session_id: None,
        scope: String::new(),
        acting_client_id: None,
        approval_owner_user_id: None,
        auth_method: AuthMethod::Session,
        allow_all_services: true,
        allow_all_nodes: true,
        allowed_service_ids: vec![],
        allowed_node_ids: vec![],
        api_key_id: None,
        api_key_name: None,
        rate_limit_per_second: None,
        rate_limit_burst: None,
    }
}

pub(crate) fn test_user(user_id: &str, user_type: UserType) -> User {
    let now = chrono::Utc::now();
    User {
        id: user_id.to_string(),
        email: format!("{user_id}@example.com"),
        password_hash: None,
        display_name: Some(match user_type {
            UserType::Person => "Test User".to_string(),
            UserType::Org => "Test Org".to_string(),
        }),
        avatar_url: None,
        email_verified: true,
        email_verification_token: None,
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin: false,
        role_ids: vec![],
        group_ids: vec![],
        invite_code_id: None,
        mfa_enabled: false,
        social_provider: None,
        social_provider_id: None,
        user_type,
        primary_org_id: None,
        created_at: now,
        updated_at: now,
        last_login_at: None,
    }
}

pub(crate) fn test_membership(
    org_user_id: &str,
    member_user_id: &str,
    role: OrgRole,
    allowed_service_ids: Option<Vec<String>>,
) -> OrgMembership {
    OrgMembership {
        id: Uuid::new_v4().to_string(),
        org_user_id: org_user_id.to_string(),
        member_user_id: member_user_id.to_string(),
        role,
        allowed_service_ids,
        created_at: chrono::Utc::now(),
        revoked_at: None,
    }
}

pub(crate) fn test_user_endpoint(
    endpoint_id: &str,
    user_id: &str,
    label: &str,
    url: &str,
    openapi_spec_url: Option<&str>,
    catalog_service_id: Option<&str>,
) -> UserEndpoint {
    UserEndpoint {
        id: endpoint_id.to_string(),
        user_id: user_id.to_string(),
        label: label.to_string(),
        url: url.to_string(),
        catalog_service_id: catalog_service_id.map(str::to_string),
        openapi_spec_url: openapi_spec_url.map(str::to_string),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

pub(crate) fn test_user_service(
    service_id: &str,
    user_id: &str,
    slug: &str,
    endpoint_id: &str,
    catalog_service_id: Option<&str>,
    node_id: Option<&str>,
) -> UserService {
    UserService {
        id: service_id.to_string(),
        user_id: user_id.to_string(),
        slug: slug.to_string(),
        endpoint_id: endpoint_id.to_string(),
        api_key_id: None,
        auth_method: "none".to_string(),
        auth_key_name: String::new(),
        catalog_service_id: catalog_service_id.map(str::to_string),
        node_id: node_id.map(str::to_string),
        node_priority: 0,
        service_type: "http".to_string(),
        identity_propagation_mode: "none".to_string(),
        identity_include_user_id: false,
        identity_include_email: false,
        identity_include_name: false,
        identity_jwt_audience: None,
        forward_access_token: false,
        inject_delegation_token: false,
        delegation_token_scope: "llm:proxy".to_string(),
        custom_user_agent: None,
        default_request_headers: None,
        is_active: true,
        source: None,
        source_id: None,
        source_app_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}
