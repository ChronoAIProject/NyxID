use std::sync::{Arc, OnceLock, atomic::AtomicUsize};
use std::time::Duration;

use base64::Engine;
use mongodb::bson::doc;
use uuid::Uuid;

use crate::AppState;
use crate::config::AppConfig;
use crate::crypto::aes::EncryptionKeys;
use crate::crypto::jwks::JwksCache;
use crate::crypto::jwt::JwtKeys;
use crate::models::audit_log::AuditLog;
use crate::models::mcp_session::McpSessionStore;
use crate::models::node_pending_credential::NodePendingCredential;
use crate::models::org_membership::{MemberScopeSource, OrgMembership, OrgRole};
use crate::models::user::{User, UserType};
use crate::models::user_endpoint::UserEndpoint;
use crate::models::user_service::UserService;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::dpop_jti_cache::{
    DPOP_JTI_CACHE_CAPACITY, DPOP_JTI_CACHE_TTL_SECS, DpopJtiCache,
};
use crate::services::event_dedup_cache::EventDedupCache;
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::provider_token_exchange_service::TokenExchangeCache;
use crate::services::ssh_service::SshSessionManager;

const TEST_DB_NAME_PREFIX: &str = "nyxid_test_";
const TEST_DB_UUID_LEN: usize = 36;
const MAX_TEST_DB_PREFIX_LEN: usize = 63 - TEST_DB_NAME_PREFIX.len() - 1 - TEST_DB_UUID_LEN;

/// Connect to a fresh per-test MongoDB database.
///
/// Probes the dev docker-compose mongod on `127.0.0.1:27018` first (published by
/// `docker-compose.override.yml`), then the CI-style mongod on `127.0.0.1:27017`
/// (the GitHub Actions service container). Each candidate is gated by a fast TCP
/// reachability check, so a port with no listener — e.g. 27018 in CI, or both
/// ports when no mongod is running locally — is skipped in milliseconds instead
/// of stalling on the driver's server-selection timeout. Without that pre-check
/// the dead 27018 probe cost ~10s of server selection on *every* DB-backed test
/// in CI (where only 27017 exists), which dominated the suite's wall-clock.
/// Returns `None` when neither is reachable so integration tests skip cleanly.
///
/// Deliberately NOT cached: a per-test client is required for correct llvm-cov
/// coverage measurement — a shared client broke under the runtime-per-test
/// harness (see #864). The TCP pre-check keeps per-test connects cheap.
pub(crate) async fn connect_test_database(prefix: &str) -> Option<mongodb::Database> {
    let client = probe_test_mongo_client().await?;
    let db_name = format!(
        "{TEST_DB_NAME_PREFIX}{}_{}",
        sanitize_test_db_prefix(prefix),
        uuid::Uuid::new_v4()
    );

    Some(client.database(&db_name))
}

/// Returns `true` when a TCP connection to `addr` succeeds quickly. A closed
/// local port returns `ECONNREFUSED` almost immediately, so this rejects a dead
/// probe candidate in ~milliseconds rather than paying the mongo server-selection
/// timeout. The timeout is only an upper bound for the pathological case of a
/// port that neither accepts nor refuses (not expected on loopback).
async fn test_mongo_port_reachable(addr: &str) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect(addr),
        )
        .await,
        Ok(Ok(_))
    )
}

async fn probe_test_mongo_client() -> Option<mongodb::Client> {
    let db_name = format!("nyxid_test_probe_{}", uuid::Uuid::new_v4());
    // (tcp address, client URI). 27018 is the dev docker-compose port; 27017 is
    // the CI service-container port. Probe order is no longer load-bearing — the
    // TCP pre-check below skips whichever candidate has no listener.
    let candidates = [
        (
            "127.0.0.1:27018",
            format!(
                "mongodb://nyxid:nyxid_dev_password@127.0.0.1:27018/{db_name}?authSource=admin&directConnection=true"
            ),
        ),
        (
            "127.0.0.1:27017",
            format!("mongodb://127.0.0.1:27017/{db_name}?directConnection=true"),
        ),
    ];

    for (addr, uri) in candidates {
        // Fast-fail a port with no listener instead of blocking on server
        // selection before falling over to the next candidate.
        if !test_mongo_port_reachable(addr).await {
            continue;
        }

        let Ok(mut options) = mongodb::options::ClientOptions::parse(&uri).await else {
            continue;
        };
        // The TCP pre-check guards against dead-port stalls in milliseconds, so
        // these generous driver timeouts only bound the real-mongod-present
        // case. Under cargo llvm-cov, argon2 plus instrumentation can starve the
        // driver's heartbeat monitor long enough to otherwise trigger
        // ConnectionPoolCleared("server monitor timeout") flakes, observed on
        // reset_password_happy_path.
        options.server_selection_timeout = Some(Duration::from_secs(30));
        options.connect_timeout = Some(Duration::from_secs(20));
        options.max_pool_size = Some(4);
        let Ok(client) = mongodb::Client::with_options(options) else {
            continue;
        };
        let db = client.database(&db_name);
        if db.run_command(doc! { "ping": 1 }).await.is_err() {
            continue;
        }

        let probe = db.collection::<mongodb::bson::Document>("__probe");
        let write_ready = tokio::time::timeout(
            Duration::from_secs(5),
            probe.insert_one(doc! { "_id": "probe" }),
        )
        .await;
        if matches!(write_ready, Ok(Ok(_))) {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                probe.delete_one(doc! { "_id": "probe" }),
            )
            .await;
            return Some(client);
        }
    }

    None
}

fn sanitize_test_db_prefix(prefix: &str) -> String {
    let sanitized: String = prefix
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .take(MAX_TEST_DB_PREFIX_LEN)
        .collect();

    if sanitized.is_empty() {
        "db".to_string()
    } else {
        sanitized
    }
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
        encryption_key: Some("11".repeat(32)),
        encryption_key_previous: None,
        rate_limit_per_second: 10,
        rate_limit_burst: 30,
        trusted_proxy_ips: vec![],
        mtls_client_cert_header: None,
        cli_pairing_hmac_key: None,
        sa_token_ttl_secs: 3600,
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
        oracle_task_retention_days: 30,
        cloud_response_cache_ttl_secs: 0,
        cloud_response_cache_max_entry_bytes: 1024 * 1024,
        cloud_response_cache_max_entries: 256,
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

/// A throwaway 2048-bit RSA private key (PKCS#8 PEM) for GCP service-account
/// mint tests. A mock token server never verifies the signature, so this
/// only needs to be a parseable, signable key.
pub(crate) const TEST_GCP_SA_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCxOfGio6jS5FhN\nxWOq8diF22dhiHhJ+IHxHM7NP40+ljQri6sRnfFzbEZoS2JcXgX7vuWBwjopYgR0\nawMK+fjhOzuy1bEltJ940ZyFgtVIMxgAVosI9fz38faLd1hqc1X/S2KADLYFdt2I\nTucnPg3W5eLlwXrggCBR5TuGBkSGO2uX4H48pZ54vEVrT4APz3GF6kn378lM/04G\nXKfuR3VBCQtQ1N1t+uSDHVEZCOXqnOm1KDgBuBvGCwn+nDAo8X7vSUZ53CvzIsgX\nmHCf7u2cHdYw9LRYlZdMeNuuIRX/2pH5chuIGoVKgywG3svb3/STJG6jT2oUpM7c\nCYu0p7SVAgMBAAECggEAFcPQdZFUy+WIJLDvnxBJb5L03MkGQMtYpfRMP2+lGIEY\n0ho6fZTgkLTE5s0PPNm9MWANzoQ8YVWsx2FXA9OUKZD9MWbF9SP8C7nuV4UsTUwd\nD/mQ5J5VHVwlU5ZqENSuRIaNB73H4t7osPNDtxGLYI9l8KJ0xTpm/bfBuiFt6/AO\nvJoCT12m5gZzF7cLHk3Gb8a9YSlj86rM3eJJF+L0UZ0gpob//RDqnX58SaqeW3sM\nIRXPL9ZHUsKZ9i2Ke68DMox9ACi3gFmnsyaiB0yhBjOiBvTpIjgAT7ucmdFKP15D\ndPgphTxM6cnxGLRE37PiSqW6GDzA7itly8zPRi2OpwKBgQDls4wyeaMNQtgSvXWM\nvSImzgyk7/KagmWtniSYw8Kh8pAL0vHUz38XL8PpVDBTCp8N7pSHnu8brxXOwwYU\n/a9kJzgmncYHogkrcsDXskx4czUx6BO7p8qMBSYh2dCI9iHIJejl3Be+tWmWdEPk\nXn7WCOzq3mzJVfubdMuAqGTpEwKBgQDFhGAJHSMIsEInEHMDCmHy7cX5pDOJoX9K\nB2SjQTpHXmTS6LjrpAFSodyuM3lr/M/coVk8FAwwGfNAlViaEotQBlbkU/HkekkM\n+iNvlMKm8YL2fMpCHQNDI/S9sjiI0Yi7unPFnlbmpCY7NDCWGJsm0x5IsDs4sKfF\nQ8ISheGItwKBgQDgOu3ZODSbdW1InfpqcRctmmdte27wtepcGczP9AnD3e4QHNRG\nUmhWUiKFW9HwvqWWDBiia9wuwjQfqvH8+8iDlGWUDOCMAvnAmDz4Uu2jh5OeLFdX\nEO0A0uXulZqkmOFRaPB5sujbGm0Amm7MOBLJDd15SbgYsv7zOoiOB9S6UQKBgCDZ\nx288nVsQlbARmE9lJq1Uxpyipr+5UIZrfF16t8qu9G3vrvHiMSYhLab7gLJpNdko\nLMNFQlGtvzt6m2Xkt67znvgSziSGAihaYhJo14cUnAeK8cjVMnm0PTxfq+91ihxP\nAnpXv3RU0Nb/8yTDqupmKp9EUFU5bG3uuxSBl+U5AoGBAL+NOw9adup24YiPJ/Gc\nMC3YWJLHTMmWthhQl2zoST3B2qyF59herT0OapF9uvSA/3R7l2/hjY7Y62qHdvlp\nyvwM98ObxwlT/Cip3pDK1E/cek9QwqxyAsRDdy/Tr1PnISowhaNRtv/6yjpjDMRq\n36i//64vyzDNvwtlnvGWhsCs\n-----END PRIVATE KEY-----\n";

/// Build a Google service-account key JSON whose `token_uri` points at a
/// (test) token endpoint. Used by GCP service-account mint/handler tests.
pub(crate) fn test_gcp_sa_json(token_uri: &str) -> String {
    serde_json::json!({
        "type": "service_account",
        "project_id": "test-project",
        "private_key_id": "abc123",
        "private_key": TEST_GCP_SA_PRIVATE_KEY,
        "client_email": "svc@test-project.iam.gserviceaccount.com",
        "client_id": "1234567890",
        "token_uri": token_uri,
    })
    .to_string()
}

/// Spawn a one-route mock OAuth token endpoint on localhost. Returns the
/// `/token` URL (usable as a service-account `token_uri` under `cfg(test)`)
/// and the server task handle.
pub(crate) async fn spawn_mock_token_server(
    response: serde_json::Value,
    status: axum::http::StatusCode,
) -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new().route(
        "/token",
        axum::routing::post(move || {
            let resp = response.clone();
            async move { (status, axum::Json(resp)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/token"), handle)
}

/// Returns a process-wide RSA `JwtKeys` for tests, generated lazily once.
///
/// Generating an RSA keypair via the pure-Rust `rsa` crate is the dominant
/// cost in many test paths (tens of seconds per call, even at 2048 bits, in
/// debug profiles). Tests don't need production-grade key sizes or unique
/// keys per test, so we share one 2048-bit pair across the entire test
/// binary and clone the cheap `JwtKeys` handle for each caller.
pub(crate) fn cached_test_jwt_keys() -> JwtKeys {
    static CACHED: OnceLock<JwtKeys> = OnceLock::new();
    CACHED.get_or_init(generate_test_jwt_keys).clone()
}

fn generate_test_jwt_keys() -> JwtKeys {
    use jsonwebtoken::{DecodingKey, EncodingKey};
    use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey};
    use rsa::traits::PublicKeyParts;
    use sha2::{Digest, Sha256};

    let mut rng = rand::thread_rng();
    let private_key =
        rsa::RsaPrivateKey::new(&mut rng, 2048).expect("generate test RSA private key");
    let public_key = private_key.to_public_key();

    let private_pem = private_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .expect("encode test RSA private PEM");
    let public_pem = public_key
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .expect("encode test RSA public PEM");

    let n_bytes = public_key.n().to_bytes_be();
    let kid = hex::encode(&Sha256::digest(&n_bytes)[..8]);

    JwtKeys {
        encoding: EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .expect("build test RSA encoding key"),
        decoding: DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .expect("build test RSA decoding key"),
        kid,
    }
}

/// Build a minimal `AppState` for handler tests.
pub(crate) fn test_app_state(db: mongodb::Database) -> AppState {
    let config = test_app_config();
    test_app_state_with_config(db, config)
}

/// Build an `AppState` with a caller-provided config for pure handler tests.
pub(crate) fn test_app_state_with_config(db: mongodb::Database, config: AppConfig) -> AppState {
    let http_client = reqwest::Client::new();
    let jwt_keys = cached_test_jwt_keys();

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
        device_code_pubkey_limiter: crate::mw::rate_limit::create_per_pubkey_rate_limiter(),
        device_code_ip_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(5, 60),
        public_proxy_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            config.public_proxy_rate_limit_per_minute,
            60,
        ),
        public_mcp_limiter: crate::mw::rate_limit::create_per_ip_rate_limiter(
            config.public_mcp_rate_limit_per_minute,
            60,
        ),
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
        per_message_edit_limiter: Arc::new(crate::mw::rate_limit::PerMessageEditRateLimiter::new(
            config.channel_relay_edit_rate_limit_per_second,
            config.channel_relay_edit_rate_limit_burst,
        )),
        event_dedup_cache: Arc::new(EventDedupCache::new(
            config.channel_event_dedup_capacity,
            Duration::from_secs(config.channel_event_dedup_ttl_secs),
        )),
        dpop_jti_cache: Arc::new(DpopJtiCache::new(
            DPOP_JTI_CACHE_CAPACITY,
            Duration::from_secs(DPOP_JTI_CACHE_TTL_SECS),
        )),
        ws_passthrough_count: Arc::new(AtomicUsize::new(0)),
        token_exchange_cache: Arc::new(TokenExchangeCache::new()),
        cloud_response_cache: Arc::new(
            crate::services::cloud_response_cache::CloudResponseCache::new(0),
        ),
        telemetry: None,
    }
}

/// Build an `AppState` for tests that never perform MongoDB operations.
pub(crate) async fn test_app_state_no_db() -> AppState {
    let client = mongodb::Client::with_uri_str("mongodb://localhost:27017")
        .await
        .expect("build inert test MongoDB client");
    test_app_state(client.database("nyxid_unit_unused"))
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
        ip_address: None,
        user_agent: None,
    }
}

fn sorted_strings(values: &[&str]) -> Vec<String> {
    let mut values: Vec<String> = values.iter().map(|value| value.to_string()).collect();
    values.sort();
    values
}

fn b64url_fixture(byte: u8, len: usize) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![byte; len])
}

/// Assert an RCI audit row is metadata-only and has exactly the expected keys.
pub(crate) fn assert_rci_audit_row(
    entry: &AuditLog,
    expected_event_type: &str,
    pending: &NodePendingCredential,
    expected_remote_state: Option<&str>,
    extra_keys: &[&str],
) {
    assert_eq!(entry.event_type, expected_event_type);
    assert_eq!(
        entry.user_id.as_deref(),
        Some(pending.owner_user_id.as_str())
    );

    let event_data = entry.event_data.as_ref().expect("audit event data");
    let object = event_data.as_object().expect("audit event data object");
    let mut expected_keys = vec![
        "event_at",
        "flow",
        "node_id",
        "owner_user_id",
        "pending_created_at",
        "pending_credential_id",
        "pending_expires_at",
        "routed_via",
        "service_slug",
    ];
    if expected_remote_state.is_some() {
        expected_keys.push("remote_state");
    }
    expected_keys.extend(extra_keys.iter().copied());

    let mut actual_keys: Vec<String> = object.keys().cloned().collect();
    actual_keys.sort();
    assert_eq!(actual_keys, sorted_strings(&expected_keys));

    assert_eq!(object["flow"], "remote_credential_injection");
    assert_eq!(object["routed_via"], "node");
    assert_eq!(object["node_id"], pending.node_id);
    assert_eq!(object["pending_credential_id"], pending.id);
    assert_eq!(object["service_slug"], pending.service_slug);
    assert_eq!(object["owner_user_id"], pending.owner_user_id);
    assert_eq!(
        object["pending_created_at"],
        pending.created_at.to_rfc3339()
    );
    assert_eq!(
        object["pending_expires_at"],
        pending.expires_at.to_rfc3339()
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(object["event_at"].as_str().expect("event_at string"))
            .is_ok()
    );

    if let Some(remote_state) = expected_remote_state {
        assert_eq!(object["remote_state"], remote_state);
    } else {
        assert!(object.get("remote_state").is_none());
    }

    if let Some(queued_at) = object.get("ciphertext_queued_at") {
        assert_eq!(
            queued_at.as_str().expect("ciphertext_queued_at string"),
            pending
                .ciphertext_queued_at
                .expect("pending has queued timestamp")
                .to_rfc3339()
        );
    }
    if let Some(expires_at) = object.get("ciphertext_expires_at") {
        assert_eq!(
            expires_at.as_str().expect("ciphertext_expires_at string"),
            pending
                .ciphertext_expires_at
                .expect("pending has ciphertext expiry")
                .to_rfc3339()
        );
    }

    for forbidden_key in [
        "plaintext",
        "secret",
        "ciphertext",
        "nonce",
        "node_pubkey",
        "admin_pubkey",
        "sealed_privkey",
        "private_key",
        "hash",
        "fingerprint",
        "length",
        "bytes",
        "target_url",
        "field_name",
        "injection_method",
        "raw_version",
        "raw_status",
        "raw_node_error",
        "raw_decrypt_error",
        "raw_decline_reason",
        "decrypt_error",
        "queue_count",
        "queued_pending_ids",
    ] {
        assert!(
            !object.contains_key(forbidden_key),
            "{expected_event_type}: {forbidden_key}"
        );
    }

    let event_json = event_data.to_string();
    let forbidden_values = [
        b64url_fixture(5, 32),
        b64url_fixture(6, 32),
        b64url_fixture(7, 24),
        b64url_fixture(8, 32),
        b64url_fixture(9, 31),
        b64url_fixture(10, 32),
        b64url_fixture(11, 24),
        b64url_fixture(12, 32),
        b64url_fixture(13, 32),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3]),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1, 2, 3, 4]),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([42]),
        "super-secret-plaintext-fixture".to_string(),
        "secret-value-fixture".to_string(),
        "raw-node-error-fixture".to_string(),
        "decline-reason-fixture".to_string(),
        "raw-decline-reason-fixture".to_string(),
    ];
    for forbidden_value in forbidden_values {
        assert!(!event_json.contains(&forbidden_value), "{forbidden_value}");
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
        // Derive a unique slug from the user_id so tests that create multiple
        // org fixtures and call `ensure_indexes` (which builds the partial
        // unique slug index) don't collide on a shared "test-org" value.
        slug: match user_type {
            UserType::Person => None,
            UserType::Org => Some(format!(
                "test-org-{}",
                &user_id.replace('-', "").chars().take(8).collect::<String>()
            )),
        },
        avatar_url: None,
        email_verified: true,
        email_verification_token: None,
        password_reset_token: None,
        password_reset_expires_at: None,
        is_active: true,
        is_admin: false,
        is_operator: false,
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
        profile_config: Default::default(),
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
        scope_source: MemberScopeSource::Override,
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
        ssh_auth_mode: crate::models::ssh_auth_mode::SshAuthMode::ProxyOnly,
        ssh_node_keys_stale: false,
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
        ws_frame_injections: Vec::new(),
        is_active: true,
        source: None,
        source_id: None,
        source_app_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Guards the per-test mongo probe against the CI slowdown regression: a
    /// candidate port with no listener must fail over in ~milliseconds, not stall
    /// on the driver's server-selection timeout. (In CI only 27017 is published,
    /// so the 27018 candidate is dead — without the TCP pre-check every DB test
    /// paid ~10s here.) Uses a freshly-closed ephemeral port so the assertion is
    /// deterministic and needs no running mongod.
    #[tokio::test]
    async fn closed_port_probe_fails_fast() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local addr").to_string();
        drop(listener); // port now has no listener -> connect refused immediately

        let start = Instant::now();
        let reachable = test_mongo_port_reachable(&addr).await;
        let elapsed = start.elapsed();

        assert!(!reachable, "a closed port must be reported unreachable");
        assert!(
            elapsed < Duration::from_secs(2),
            "closed-port probe must fail fast (got {elapsed:?}); a dead candidate \
             must not block on the mongo server-selection timeout"
        );
    }
}
