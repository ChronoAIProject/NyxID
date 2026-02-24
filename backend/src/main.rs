use std::net::SocketAddr;
use axum::{extract::DefaultBodyLimit, extract::Extension, middleware as axum_mw};
use clap::Parser;
use tokio::net::TcpListener;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod config;
mod crypto;
mod db;
mod errors;
mod handlers;
mod models;
mod mw;
mod routes;
mod services;

use std::sync::Arc;

use crate::db::DbHandle;
use config::AppConfig;
use crypto::jwt::JwtKeys;
use models::mcp_session::McpSessionStore;

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
}

/// NyxID authentication and SSO platform.
#[derive(Parser)]
#[command(name = "nyxid", version, about)]
struct Cli {
    /// Promote an existing user to admin by email address, then exit.
    #[arg(long = "promote-admin", value_name = "EMAIL")]
    promote_admin: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Load environment variables from .env file (if present)
    dotenvy::dotenv().ok();

    // Initialize structured logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("nyxid=info,tower_http=info")
        }))
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();

    // Load configuration
    let config = AppConfig::from_env();

    // Connect to database
    let db = db::create_connection(&config)
        .await
        .expect("Failed to connect to database");

    // Handle CLI commands (exit without starting server)
    if let Some(email) = cli.promote_admin {
        run_promote_admin(&db, &email).await;
        return;
    }

    // Seed default OAuth clients (idempotent)
    services::oauth_client_service::seed_default_clients(&db)
        .await
        .expect("Failed to seed default OAuth clients");

    // Seed default AI provider configurations (idempotent)
    services::provider_service::seed_default_providers(&db, &config.encryption_key)
        .await
        .expect("Failed to seed default providers");

    // Seed downstream services for LLM providers (idempotent)
    services::provider_service::seed_default_llm_services(&db, &config.encryption_key)
        .await
        .expect("Failed to seed default LLM services");

    // Seed system roles for RBAC (idempotent)
    services::role_service::seed_system_roles(&db)
        .await
        .expect("Failed to seed system roles");

    // --- Server startup ---
    tracing::info!("Starting NyxID authentication server");
    tracing::info!(port = config.port, issuer = %config.jwt_issuer, "Configuration loaded");
    config.warn_if_non_url_issuer();

    // Validate encryption key at startup
    config.validate_encryption_key();

    // Load JWT signing keys
    let jwt_keys = JwtKeys::from_config(&config).expect("Failed to load JWT keys");
    tracing::info!("JWT keys loaded (kid={})", jwt_keys.kid);

    // Compute JWK from the public key for the JWKS endpoint
    let public_pem = std::fs::read_to_string(&config.jwt_public_key_path)
        .expect("Failed to read public key for JWK");
    let jwk_json = crypto::jwt::public_key_jwk(&public_pem)
        .expect("Failed to compute JWK from public key");

    // Create a shared reqwest client for connection reuse
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
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

    // Create shared state
    let state = AppState {
        db,
        config: config.clone(),
        jwt_keys,
        http_client,
        jwk_json,
        mcp_sessions: mcp_sessions.clone(),
    };

    // Create rate limiters
    let global_rate_limiter = mw::rate_limit::create_rate_limiter(
        config.rate_limit_per_second,
        config.rate_limit_burst,
    );
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

    // Configure CORS - restrict methods and headers explicitly
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::exact(
            config.frontend_url.parse().expect("Invalid FRONTEND_URL"),
        ))
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
        ]))
        .allow_credentials(true);

    // Build router — public OAuth routes get open CORS (per RFC 9207),
    // private API routes get restricted CORS (FRONTEND_URL only).
    let (public_oauth, private_api) = routes::build_router();

    let app = public_oauth
        .merge(private_api.layer(cors))
        .with_state(state)
        .layer(DefaultBodyLimit::max(1_048_576))
        .layer(axum_mw::from_fn(
            mw::security_headers::security_headers_middleware,
        ))
        .layer(axum_mw::from_fn(
            mw::rate_limit::rate_limit_middleware,
        ))
        .layer(Extension(per_ip_rate_limiter))
        .layer(Extension(global_rate_limiter))
        .layer(TraceLayer::new_for_http());

    // Bind and serve
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    tracing::info!("Listening on {addr}");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("Server error");
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
