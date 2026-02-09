use axum::{extract::DefaultBodyLimit, extract::Extension, middleware as axum_mw};
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
}

#[tokio::main]
async fn main() {
    // Load environment variables from .env file (if present)
    dotenvy::dotenv().ok();

    // Initialize structured logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("nyxid=info,tower_http=info")
        }))
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();

    tracing::info!("Starting NyxID authentication server");

    // Load configuration
    let config = AppConfig::from_env();
    tracing::info!(port = config.port, "Configuration loaded");

    // Validate encryption key at startup
    config.validate_encryption_key();

    // Connect to database
    let db = db::create_connection(&config)
        .await
        .expect("Failed to connect to database");

    // Load JWT signing keys
    let jwt_keys = JwtKeys::from_config(&config).expect("Failed to load JWT keys");
    tracing::info!("JWT keys loaded");

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
