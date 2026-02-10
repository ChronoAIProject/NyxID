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

use crate::db::DbHandle;
use config::AppConfig;
use crypto::jwt::JwtKeys;

/// Shared application state available to all handlers via Axum's State extractor.
#[derive(Clone)]
pub struct AppState {
    pub db: DbHandle,
    pub config: AppConfig,
    pub jwt_keys: JwtKeys,
    pub http_client: reqwest::Client,
    /// Pre-computed JWK for the JWKS endpoint
    pub jwk_json: serde_json::Value,
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

    // --- Server startup ---
    tracing::info!("Starting NyxID authentication server");
    tracing::info!(port = config.port, "Configuration loaded");

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

    // Create shared state
    let state = AppState {
        db,
        config: config.clone(),
        jwt_keys,
        http_client,
        jwk_json,
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

    // Build router with all middleware layers
    let app = routes::build_router()
        .with_state(state)
        .layer(DefaultBodyLimit::max(1_048_576)) // 1MB global body limit
        .layer(axum_mw::from_fn(
            mw::security_headers::security_headers_middleware,
        ))
        .layer(axum_mw::from_fn(
            mw::rate_limit::rate_limit_middleware,
        ))
        .layer(Extension(per_ip_rate_limiter))
        .layer(Extension(global_rate_limiter))
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    // Bind and serve
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    tracing::info!("Listening on {addr}");

    axum::serve(listener, app)
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
