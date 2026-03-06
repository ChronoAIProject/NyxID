use std::time::Duration;

use mongodb::bson::doc;
use mongodb::options::{ClientOptions, IndexOptions};
use mongodb::{Client, Database, IndexModel};

use crate::config::AppConfig;

/// Type alias for the MongoDB database handle used throughout the application.
pub type DbHandle = Database;

/// Create a configured MongoDB connection and return the database handle.
///
/// Parses the connection string, configures the connection pool, verifies
/// connectivity with a ping, and ensures all required indexes exist.
pub async fn create_connection(config: &AppConfig) -> Result<DbHandle, mongodb::error::Error> {
    let mut client_options = ClientOptions::parse(&config.database_url).await?;

    client_options.max_pool_size = Some(config.database_max_connections);
    client_options.min_pool_size = Some(2);
    client_options.connect_timeout = Some(Duration::from_secs(10));
    client_options.server_selection_timeout = Some(Duration::from_secs(10));
    client_options.max_idle_time = Some(Duration::from_secs(600));

    let client = Client::with_options(client_options)?;

    // Extract database name from the connection string, default to "nyxid"
    let db_name = client
        .default_database()
        .map(|db| db.name().to_string())
        .unwrap_or_else(|| "nyxid".to_string());

    let db = client.database(&db_name);

    // Verify connectivity
    db.run_command(doc! { "ping": 1 }).await?;
    tracing::info!("MongoDB connection established");

    ensure_indexes(&db).await?;
    tracing::info!("MongoDB indexes verified");

    Ok(db)
}

/// Create all required indexes for every collection.
///
/// Uses `create_index` which is idempotent -- if the index already exists
/// with the same specification it is a no-op.
pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    // ── users ──
    let users = db.collection::<mongodb::bson::Document>("users");
    users
        .create_index(
            IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    users
        .create_index(
            IndexModel::builder()
                .keys(doc! { "email_verification_token": 1 })
                .build(),
        )
        .await?;
    users
        .create_index(
            IndexModel::builder()
                .keys(doc! { "password_reset_token": 1 })
                .build(),
        )
        .await?;
    // Social login lookup: find user by (provider, provider_id)
    // Drop old sparse index if it exists (sparse doesn't work with null values from serde)
    let _ = users
        .drop_index("social_provider_1_social_provider_id_1")
        .await;
    users
        .create_index(
            IndexModel::builder()
                .keys(doc! { "social_provider": 1, "social_provider_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {
                            "social_provider": { "$type": "string" },
                            "social_provider_id": { "$type": "string" },
                        })
                        .build(),
                )
                .build(),
        )
        .await?;

    // ── sessions ──
    let sessions = db.collection::<mongodb::bson::Document>("sessions");
    sessions
        .create_index(IndexModel::builder().keys(doc! { "token_hash": 1 }).build())
        .await?;
    sessions
        .create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;
    sessions
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(0))
                        .build(),
                )
                .build(),
        )
        .await?;

    // ── authorization_codes ──
    let auth_codes = db.collection::<mongodb::bson::Document>("authorization_codes");
    auth_codes
        .create_index(IndexModel::builder().keys(doc! { "code_hash": 1 }).build())
        .await?;
    auth_codes
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(0))
                        .build(),
                )
                .build(),
        )
        .await?;

    // ── refresh_tokens ──
    let refresh_tokens = db.collection::<mongodb::bson::Document>("refresh_tokens");
    refresh_tokens
        .create_index(
            IndexModel::builder()
                .keys(doc! { "jti": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    refresh_tokens
        .create_index(IndexModel::builder().keys(doc! { "session_id": 1 }).build())
        .await?;
    refresh_tokens
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(0))
                        .build(),
                )
                .build(),
        )
        .await?;

    // ── api_keys ──
    let api_keys = db.collection::<mongodb::bson::Document>("api_keys");
    api_keys
        .create_index(IndexModel::builder().keys(doc! { "key_hash": 1 }).build())
        .await?;
    api_keys
        .create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;

    // ── mfa_factors ──
    let mfa = db.collection::<mongodb::bson::Document>("mfa_factors");
    mfa.create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;

    // ── downstream_services ──
    let services = db.collection::<mongodb::bson::Document>("downstream_services");
    services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "slug": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "service_category": 1, "is_active": 1 })
                .build(),
        )
        .await?;

    services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "provider_config_id": 1 })
                .options(IndexOptions::builder().sparse(true).unique(true).build())
                .build(),
        )
        .await?;

    // ── user_service_connections ──
    let usc = db.collection::<mongodb::bson::Document>("user_service_connections");
    usc.create_index(
        IndexModel::builder()
            .keys(doc! { "user_id": 1, "service_id": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;

    // ── audit_log ──
    let audit = db.collection::<mongodb::bson::Document>("audit_log");
    audit
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "created_at": -1 })
                .build(),
        )
        .await?;
    audit
        .create_index(
            IndexModel::builder()
                .keys(doc! { "event_type": 1, "created_at": -1 })
                .build(),
        )
        .await?;

    // ── oauth_clients ── (no special indexes beyond _id)

    // ── service_endpoints ──
    let endpoints = db.collection::<mongodb::bson::Document>("service_endpoints");
    endpoints
        .create_index(
            IndexModel::builder()
                .keys(doc! { "service_id": 1, "is_active": 1 })
                .build(),
        )
        .await?;
    endpoints
        .create_index(
            IndexModel::builder()
                .keys(doc! { "service_id": 1, "name": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

    // ── provider_configs ──
    let provider_configs = db.collection::<mongodb::bson::Document>("provider_configs");
    provider_configs
        .create_index(
            IndexModel::builder()
                .keys(doc! { "slug": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    provider_configs
        .create_index(
            IndexModel::builder()
                .keys(doc! { "provider_type": 1, "is_active": 1 })
                .build(),
        )
        .await?;

    // ── user_provider_tokens ──
    let user_tokens = db.collection::<mongodb::bson::Document>("user_provider_tokens");
    user_tokens
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "provider_config_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    user_tokens
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "status": 1 })
                .build(),
        )
        .await?;
    user_tokens
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "expires_at": 1 })
                .build(),
        )
        .await?;

    // ── service_provider_requirements ──
    let spr = db.collection::<mongodb::bson::Document>("service_provider_requirements");
    spr.create_index(
        IndexModel::builder()
            .keys(doc! { "service_id": 1, "provider_config_id": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;
    spr.create_index(IndexModel::builder().keys(doc! { "service_id": 1 }).build())
        .await?;
    spr.create_index(
        IndexModel::builder()
            .keys(doc! { "provider_config_id": 1 })
            .build(),
    )
    .await?;

    // ── oauth_states ──
    let oauth_states = db.collection::<mongodb::bson::Document>("oauth_states");
    oauth_states
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(0))
                        .build(),
                )
                .build(),
        )
        .await?;
    oauth_states
        .create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;

    // ── roles ──
    let roles = db.collection::<mongodb::bson::Document>("roles");
    roles
        .create_index(
            IndexModel::builder()
                .keys(doc! { "slug": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    roles
        .create_index(
            IndexModel::builder()
                .keys(doc! { "client_id": 1 })
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
        )
        .await?;

    // ── groups ──
    let groups = db.collection::<mongodb::bson::Document>("groups");
    groups
        .create_index(
            IndexModel::builder()
                .keys(doc! { "slug": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    groups
        .create_index(
            IndexModel::builder()
                .keys(doc! { "parent_group_id": 1 })
                .build(),
        )
        .await?;

    // ── consents ──
    let consents = db.collection::<mongodb::bson::Document>("consents");
    consents
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "client_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    consents
        .create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;

    // ── service_accounts ──
    let sa = db.collection::<mongodb::bson::Document>("service_accounts");
    sa.create_index(
        IndexModel::builder()
            .keys(doc! { "client_id": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;
    sa.create_index(IndexModel::builder().keys(doc! { "is_active": 1 }).build())
        .await?;
    sa.create_index(IndexModel::builder().keys(doc! { "created_by": 1 }).build())
        .await?;

    // ── service_account_tokens ──
    let sat = db.collection::<mongodb::bson::Document>("service_account_tokens");
    sat.create_index(
        IndexModel::builder()
            .keys(doc! { "jti": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;
    sat.create_index(
        IndexModel::builder()
            .keys(doc! { "service_account_id": 1 })
            .build(),
    )
    .await?;
    sat.create_index(
        IndexModel::builder()
            .keys(doc! { "expires_at": 1 })
            .options(
                IndexOptions::builder()
                    .expire_after(Duration::from_secs(0))
                    .build(),
            )
            .build(),
    )
    .await?;

    // ── mcp_sessions ──
    let mcp_sessions = db.collection::<mongodb::bson::Document>("mcp_sessions");
    mcp_sessions
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(0))
                        .build(),
                )
                .build(),
        )
        .await?;
    mcp_sessions
        .create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;

    // ── approval_requests ──
    let approval_requests = db.collection::<mongodb::bson::Document>("approval_requests");
    approval_requests
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "status": 1 })
                .build(),
        )
        .await?;
    // Migration: drop the legacy auto-named index on idempotency_key if it exists.
    // The current index uses a stable explicit name to avoid accidental drop/recreate
    // loops on startup.
    let _ = approval_requests.drop_index("idempotency_key_1").await;
    approval_requests
        .create_index(
            IndexModel::builder()
                .keys(doc! { "idempotency_key": 1 })
                .options(
                    IndexOptions::builder()
                        .name("idempotency_key_pending_unique".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "status": "pending" })
                        .build(),
                )
                .build(),
        )
        .await?;
    approval_requests
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(90 * 24 * 60 * 60))
                        .build(),
                )
                .build(),
        )
        .await?;

    // ── approval_grants ──
    let approval_grants = db.collection::<mongodb::bson::Document>("approval_grants");
    approval_grants
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "user_id": 1,
                    "service_id": 1,
                    "requester_type": 1,
                    "requester_id": 1,
                })
                .build(),
        )
        .await?;
    approval_grants
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(0))
                        .build(),
                )
                .build(),
        )
        .await?;
    approval_grants
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "granted_at": -1 })
                .build(),
        )
        .await?;

    // ── service_approval_configs ──
    let sac = db.collection::<mongodb::bson::Document>("service_approval_configs");
    sac.create_index(
        IndexModel::builder()
            .keys(doc! { "user_id": 1, "service_id": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build(),
    )
    .await?;
    sac.create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;

    // ── user_provider_credentials ──
    let user_creds = db.collection::<mongodb::bson::Document>("user_provider_credentials");
    user_creds
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "provider_config_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

    // ── notification_channels ──
    let notification_channels = db.collection::<mongodb::bson::Document>("notification_channels");
    notification_channels
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    notification_channels
        .create_index(
            IndexModel::builder()
                .keys(doc! { "telegram_link_code": 1 })
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
        )
        .await?;
    notification_channels
        .create_index(
            IndexModel::builder()
                .keys(doc! { "telegram_chat_id": 1 })
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
        )
        .await?;
    // Supports token-based cleanup paths (e.g. account switching and logout detach).
    notification_channels
        .create_index(
            IndexModel::builder()
                .keys(doc! { "push_devices.token": 1 })
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
        )
        .await?;

    Ok(())
}
