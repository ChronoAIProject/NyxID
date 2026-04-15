use std::collections::HashSet;
use std::time::Duration;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use mongodb::options::{ClientOptions, IndexOptions};
use mongodb::{Client, Database, IndexModel};

use crate::config::AppConfig;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::node_service_binding::{
    COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_provider_credentials::{
    COLLECTION_NAME as USER_PROVIDER_CREDENTIALS, UserProviderCredentials,
};
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::models::user_service_connection::{
    COLLECTION_NAME as USER_SERVICE_CONNECTIONS, UserServiceConnection,
};

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

    backfill_downstream_service_types(&db).await?;
    migrate_legacy_api_spec_url(&db).await?;

    Ok(db)
}

/// Create all required indexes for every collection.
///
/// Uses `create_index` which is idempotent -- if the index already exists
/// with the same specification it is a no-op.
pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    // ── users ──
    let users = db.collection::<mongodb::bson::Document>("users");
    // Backfill user_type before changing the email index. Without this, legacy
    // rows would not be matched by the new partial-unique filter and a
    // duplicate person email could slip in (the index wouldn't see the legacy row).
    backfill_user_type(db).await?;
    // Migration: drop legacy non-partial unique index on email so the new
    // partial-unique index (filtered to user_type=person) can be created.
    // Org users do not need a unique email; they often share contact emails
    // or have none at all. The drop is best-effort -- on fresh DBs the index
    // does not exist yet, which is fine.
    let _ = users.drop_index("email_1").await;
    users
        .create_index(
            IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "user_type": "person" })
                        .build(),
                )
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
    // Migration: drop legacy non-partial unique index on slug so the new partial index can be created
    let _ = services.drop_index("slug_1").await;
    services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "slug": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "is_active": true })
                        .build(),
                )
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
                .keys(doc! { "service_type": 1, "service_category": 1, "is_active": 1 })
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
    audit
        .create_index(
            IndexModel::builder()
                .keys(doc! { "api_key_id": 1, "created_at": -1 })
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
    approval_requests
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "expires_at": 1 })
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

    // ── nodes ──
    let nodes = db.collection::<mongodb::bson::Document>("nodes");
    // Drop legacy index without partial filter (if it exists) to replace with soft-delete-safe version
    let _ = nodes.drop_index("user_id_1_name_1").await;
    nodes
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "name": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "is_active": true })
                        .build(),
                )
                .build(),
        )
        .await?;
    nodes
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "is_active": 1 })
                .build(),
        )
        .await?;
    nodes
        .create_index(
            IndexModel::builder()
                .keys(doc! { "auth_token_hash": 1 })
                .build(),
        )
        .await?;

    // ── node_service_bindings ──
    let nsb = db.collection::<mongodb::bson::Document>("node_service_bindings");
    // Drop legacy index without partial filter (if it exists) to replace with soft-delete-safe version
    let _ = nsb.drop_index("node_id_1_service_id_1").await;
    nsb.create_index(
        IndexModel::builder()
            .keys(doc! { "node_id": 1, "service_id": 1 })
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .partial_filter_expression(doc! { "is_active": true })
                    .build(),
            )
            .build(),
    )
    .await?;
    nsb.create_index(
        IndexModel::builder()
            .keys(doc! { "user_id": 1, "service_id": 1, "is_active": 1 })
            .build(),
    )
    .await?;
    nsb.create_index(
        IndexModel::builder()
            .keys(doc! { "node_id": 1, "is_active": 1 })
            .build(),
    )
    .await?;

    // ── node_registration_tokens ──
    let nrt = db.collection::<mongodb::bson::Document>("node_registration_tokens");
    nrt.create_index(IndexModel::builder().keys(doc! { "token_hash": 1 }).build())
        .await?;
    nrt.create_index(
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

    // Drop old sparse unique indexes that conflict with partial filter indexes
    // (MongoDB won't replace an index with different options on the same keys)
    let _ = db
        .collection::<mongodb::bson::Document>("user_api_keys")
        .drop_index("source_1_source_id_1")
        .await;
    let _ = db
        .collection::<mongodb::bson::Document>("user_services")
        .drop_index("source_1_source_id_1")
        .await;

    // -- user_endpoints --
    let user_endpoints = db.collection::<mongodb::bson::Document>("user_endpoints");
    user_endpoints
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "catalog_service_id": 1 })
                .build(),
        )
        .await?;
    user_endpoints
        .create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;

    // -- user_api_keys --
    let user_api_keys = db.collection::<mongodb::bson::Document>("user_api_keys");
    user_api_keys
        .create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;
    user_api_keys
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "provider_config_id": 1 })
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
        )
        .await?;
    user_api_keys
        .create_index(
            IndexModel::builder()
                .keys(doc! { "source": 1, "source_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {
                            "source_id": { "$type": "string" }
                        })
                        .build(),
                )
                .build(),
        )
        .await?;

    // -- user_services --
    let user_services = db.collection::<mongodb::bson::Document>("user_services");
    user_services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "slug": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "is_active": true })
                        .build(),
                )
                .build(),
        )
        .await?;
    user_services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "catalog_service_id": 1 })
                .options(IndexOptions::builder().sparse(true).build())
                .build(),
        )
        .await?;
    user_services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "is_active": 1 })
                .build(),
        )
        .await?;
    user_services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "endpoint_id": 1 })
                .build(),
        )
        .await?;
    user_services
        .create_index(IndexModel::builder().keys(doc! { "api_key_id": 1 }).build())
        .await?;
    user_services
        .create_index(
            IndexModel::builder()
                .keys(doc! { "source": 1, "source_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {
                            "source_id": { "$type": "string" }
                        })
                        .build(),
                )
                .build(),
        )
        .await?;

    // -- agent_service_bindings --
    let agent_bindings = db.collection::<Document>("agent_service_bindings");
    agent_bindings
        .create_index(
            IndexModel::builder()
                .keys(doc! { "api_key_id": 1, "user_service_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    agent_bindings
        .create_index(IndexModel::builder().keys(doc! { "api_key_id": 1 }).build())
        .await?;
    agent_bindings
        .create_index(IndexModel::builder().keys(doc! { "user_id": 1 }).build())
        .await?;

    // ── channel_bots ──
    let channel_bots = db.collection::<mongodb::bson::Document>("channel_bots");
    channel_bots
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "platform": 1 })
                .build(),
        )
        .await?;
    channel_bots
        .create_index(
            IndexModel::builder()
                .keys(doc! { "platform": 1, "platform_bot_id": 1 })
                .build(),
        )
        .await?;

    // ── channel_conversations ──
    let channel_convos = db.collection::<mongodb::bson::Document>("channel_conversations");
    channel_convos
        .create_index(
            IndexModel::builder()
                .keys(doc! { "channel_bot_id": 1, "platform_conversation_id": 1, "platform_sender_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "is_active": true })
                        .build(),
                )
                .build(),
        )
        .await?;
    channel_convos
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "platform": 1 })
                .build(),
        )
        .await?;
    channel_convos
        .create_index(
            IndexModel::builder()
                .keys(doc! { "agent_api_key_id": 1 })
                .build(),
        )
        .await?;

    // ── channel_messages ──
    let channel_msgs = db.collection::<mongodb::bson::Document>("channel_messages");
    channel_msgs
        .create_index(
            IndexModel::builder()
                .keys(doc! { "conversation_id": 1, "created_at": -1 })
                .build(),
        )
        .await?;
    channel_msgs
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": 1 })
                // NOTE: To honor CHANNEL_RELAY_MESSAGE_TTL_DAYS, use collMod
                // after startup: db.runCommand({ collMod: "channel_messages",
                // index: { keyPattern: { created_at: 1 }, expireAfterSeconds: N } })
                // ensure_indexes only takes &Database, so the config value
                // cannot be injected here at compile time.
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(30 * 24 * 60 * 60))
                        .build(),
                )
                .build(),
        )
        .await?;

    // ── channel_event_logs ──
    // ADR-013 metadata-only event forwarding ledger. No payload content is
    // stored; see models::channel_event_log::ChannelEventLog.
    //
    // Important: the (conversation_id, event_id) index is deliberately
    // **non-unique**. The collection is an append-only audit trail, so we
    // want one row per forward attempt — including dedup hits and retries
    // following a transient callback failure. A unique constraint would
    // silently swallow later outcome rows for the same event_id.
    //
    // The new non-unique index carries an explicit name so we can safely
    // drop the legacy default-named unique index without colliding with
    // the new one. After the first boot (which cleans up the legacy name)
    // the drop becomes a cheap no-op returning `IndexNotFound`.
    let channel_event_logs = db.collection::<mongodb::bson::Document>("channel_event_logs");
    let _ = channel_event_logs
        .drop_index("conversation_id_1_event_id_1")
        .await;
    channel_event_logs
        .create_index(
            IndexModel::builder()
                .keys(doc! { "conversation_id": 1, "event_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("channel_event_logs_convid_eventid_lookup".to_string())
                        .build(),
                )
                .build(),
        )
        .await?;
    channel_event_logs
        .create_index(
            IndexModel::builder()
                .keys(doc! { "conversation_id": 1, "forwarded_at": -1 })
                .build(),
        )
        .await?;
    channel_event_logs
        .create_index(
            IndexModel::builder()
                .keys(doc! { "forwarded_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(30 * 24 * 60 * 60))
                        .build(),
                )
                .build(),
        )
        .await?;

    // ── invite_codes ──
    let invite_codes = db.collection::<mongodb::bson::Document>("invite_codes");
    invite_codes
        .create_index(
            IndexModel::builder()
                .keys(doc! { "code": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;

    // ── org_memberships ──
    let org_memberships = db.collection::<Document>("org_memberships");
    // Lookup: "all active orgs for this member" -- proxy fallback path
    org_memberships
        .create_index(
            IndexModel::builder()
                .keys(doc! { "member_user_id": 1, "revoked_at": 1 })
                .build(),
        )
        .await?;
    // Uniqueness: a person can only have one membership row per org
    org_memberships
        .create_index(
            IndexModel::builder()
                .keys(doc! { "org_user_id": 1, "member_user_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    // Lookup: "list members of this org" (filterable by revoked_at)
    org_memberships
        .create_index(
            IndexModel::builder()
                .keys(doc! { "org_user_id": 1, "revoked_at": 1 })
                .build(),
        )
        .await?;

    // ── org_invites ──
    let org_invites = db.collection::<Document>("org_invites");
    org_invites
        .create_index(
            IndexModel::builder()
                .keys(doc! { "nonce": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    // TTL: invites are removed automatically after expiry
    org_invites
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
    org_invites
        .create_index(
            IndexModel::builder()
                .keys(doc! { "org_user_id": 1 })
                .build(),
        )
        .await?;

    backfill_downstream_service_types(db).await?;
    purge_legacy_channel_message_content(db).await?;

    Ok(())
}

/// Name of the `schema_migrations` collection used to track one-off
/// data migrations that are too expensive to replay on every boot.
const SCHEMA_MIGRATIONS: &str = "schema_migrations";

/// Migration marker id for `purge_legacy_channel_message_content`.
const PURGE_CHANNEL_MESSAGE_CONTENT_MIGRATION: &str = "purge_channel_message_content_v1";

/// Enforce ADR-013 on any historical `channel_messages` documents that were
/// written before the metadata-only refactor. Unsets `text`, `attachments`,
/// and `raw_platform_data` from matching rows.
///
/// Gated behind a `schema_migrations` marker so the full-collection scan
/// (the `$exists` filter cannot use an index) runs exactly once per
/// deployment. Subsequent boots are a single indexed `find_one` against
/// the marker document.
async fn purge_legacy_channel_message_content(db: &Database) -> Result<(), mongodb::error::Error> {
    let migrations = db.collection::<Document>(SCHEMA_MIGRATIONS);

    // Skip the scan entirely if this migration has already been applied.
    let already_applied = migrations
        .find_one(doc! { "_id": PURGE_CHANNEL_MESSAGE_CONTENT_MIGRATION })
        .await?;
    if already_applied.is_some() {
        return Ok(());
    }

    let messages = db.collection::<Document>("channel_messages");
    let result = messages
        .update_many(
            doc! {
                "$or": [
                    { "text": { "$exists": true } },
                    { "attachments": { "$exists": true } },
                    { "raw_platform_data": { "$exists": true } },
                ],
            },
            doc! {
                "$unset": {
                    "text": "",
                    "attachments": "",
                    "raw_platform_data": "",
                },
            },
        )
        .await?;

    if result.modified_count > 0 {
        tracing::info!(
            count = result.modified_count,
            "Purged legacy content fields from channel_messages (ADR-013)"
        );
    } else {
        tracing::debug!("Legacy channel_messages content purge found no matching rows");
    }

    // Record the marker so future boots skip the scan. If this insert
    // fails (e.g. transient write error), the next boot will re-run the
    // purge — which is idempotent and cheap when no rows match.
    let marker = doc! {
        "_id": PURGE_CHANNEL_MESSAGE_CONTENT_MIGRATION,
        "applied_at": bson::DateTime::now(),
        "modified_count": result.modified_count as i64,
    };
    if let Err(err) = migrations.insert_one(&marker).await {
        tracing::warn!(
            error = %err,
            "Failed to write schema_migrations marker; purge will re-run on next boot"
        );
    }

    Ok(())
}

/// Backfill `user_type = "person"` on legacy user rows that pre-date the
/// org-model migration. Required before creating the partial-unique email
/// index, because the index only matches docs that satisfy the filter --
/// rows missing the field are invisible to the index.
async fn backfill_user_type(db: &Database) -> Result<(), mongodb::error::Error> {
    let users = db.collection::<Document>("users");
    let result = users
        .update_many(
            doc! { "user_type": { "$exists": false } },
            doc! { "$set": { "user_type": "person" } },
        )
        .await?;

    if result.modified_count > 0 {
        tracing::info!(
            count = result.modified_count,
            "Backfilled missing user_type to 'person'"
        );
    }

    Ok(())
}

async fn backfill_downstream_service_types(db: &Database) -> Result<(), mongodb::error::Error> {
    let services = db.collection::<Document>("downstream_services");
    let migration = services
        .update_many(
            doc! { "service_type": { "$exists": false } },
            doc! { "$set": { "service_type": "http" } },
        )
        .await?;

    if migration.modified_count > 0 {
        tracing::info!(
            count = migration.modified_count,
            "Backfilled missing downstream service_type to http"
        );
    }

    Ok(())
}

/// Migrate legacy `api_spec_url` field to `openapi_spec_url` on downstream_services.
///
/// Documents created before the field rename may have `api_spec_url`. If a later
/// update wrote `openapi_spec_url` without removing `api_spec_url`, the document
/// ends up with both keys, causing a deserialization error (serde alias treats
/// them as the same field). This migration:
/// 1. Removes `api_spec_url` from documents that have both fields (duplicate).
/// 2. Renames `api_spec_url` to `openapi_spec_url` on documents that only have the old field.
async fn migrate_legacy_api_spec_url(db: &Database) -> Result<(), mongodb::error::Error> {
    let services = db.collection::<Document>("downstream_services");

    // Step 1: Remove stale api_spec_url from documents that have both fields
    let dedup = services
        .update_many(
            doc! {
                "api_spec_url": { "$exists": true },
                "openapi_spec_url": { "$exists": true },
            },
            doc! { "$unset": { "api_spec_url": "" } },
        )
        .await?;
    if dedup.modified_count > 0 {
        tracing::info!(
            count = dedup.modified_count,
            "Removed duplicate api_spec_url from downstream services"
        );
    }

    // Step 2: Rename api_spec_url -> openapi_spec_url for remaining legacy documents
    let rename = services
        .update_many(
            doc! { "api_spec_url": { "$exists": true } },
            doc! { "$rename": { "api_spec_url": "openapi_spec_url" } },
        )
        .await?;
    if rename.modified_count > 0 {
        tracing::info!(
            count = rename.modified_count,
            "Renamed api_spec_url to openapi_spec_url on downstream services"
        );
    }

    // Step 3: Post-migration verification -- no documents should have api_spec_url
    let remaining = services
        .count_documents(doc! { "api_spec_url": { "$exists": true } })
        .await?;
    if remaining > 0 {
        tracing::error!(
            count = remaining,
            "Migration incomplete: downstream_services documents still have api_spec_url"
        );
    }

    Ok(())
}

/// Migrate existing user data to the new unified collections.
/// Idempotent: uses source + source_id to skip already-migrated records.
pub async fn migrate_to_unified_collections(
    db: &Database,
) -> Result<(), Box<dyn std::error::Error>> {
    cleanup_duplicate_migration_services(db).await?;
    migrate_provider_tokens(db).await?;
    migrate_service_connections(db).await?;
    migrate_node_service_bindings(db).await?;
    tracing::info!("Unified collection migration complete");
    Ok(())
}

/// Remove UserService records that were created by the slug-suffix migration path
/// (e.g., "api-lark-2") when the base slug already had an active record for the
/// same user + catalog_service_id. Only targets suffixed migration artifacts, not
/// legitimate multi-key setups where a user intentionally has two connections to
/// the same provider.
async fn cleanup_duplicate_migration_services(
    db: &Database,
) -> Result<(), Box<dyn std::error::Error>> {
    // Find active migration-sourced UserService records with suffixed slugs
    // (the "-N" pattern produced by the slug collision resolver).
    let migration_services: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! {
            "is_active": true,
            "source": { "$regex": "^migration_" },
            "catalog_service_id": { "$ne": null },
        })
        .await?
        .try_collect()
        .await?;

    let mut cleaned = 0u64;
    for svc in &migration_services {
        let csid = match &svc.catalog_service_id {
            Some(id) => id,
            None => continue,
        };

        // Only target slugs that look like migration suffixes (e.g., "api-lark-2").
        // Extract the base slug by stripping a trailing "-N" where N is 2..=100.
        let base_slug = match svc.slug.rfind('-') {
            Some(pos) => {
                let suffix = &svc.slug[pos + 1..];
                match suffix.parse::<u32>() {
                    Ok(n) if (2..=100).contains(&n) => &svc.slug[..pos],
                    _ => continue, // Not a migration suffix
                }
            }
            None => continue, // No hyphen, not a suffixed slug
        };

        // Verify the base slug record exists and is active for the same user + catalog service
        let base_exists = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! {
                "user_id": &svc.user_id,
                "slug": base_slug,
                "catalog_service_id": csid,
                "is_active": true,
            })
            .await?;

        if base_exists.is_none() {
            continue;
        }

        // This is a migration-created suffix duplicate -- delete it and its associated records
        let _ = db
            .collection::<UserService>(USER_SERVICES)
            .delete_one(doc! { "_id": &svc.id })
            .await;
        let _ = db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .delete_one(doc! { "_id": &svc.endpoint_id })
            .await;
        if let Some(ref ak_id) = svc.api_key_id {
            let _ = db
                .collection::<UserApiKey>(USER_API_KEYS)
                .delete_one(doc! { "_id": ak_id })
                .await;
        }

        tracing::info!(
            user_id = %svc.user_id,
            slug = %svc.slug,
            base_slug = %base_slug,
            service_id = %svc.id,
            catalog_service_id = %csid,
            "Cleaned up suffixed migration duplicate"
        );
        cleaned += 1;
    }

    if cleaned > 0 {
        tracing::info!(count = cleaned, "Cleaned up duplicate migration services");
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InheritedIdentityFields {
    identity_propagation_mode: String,
    identity_include_user_id: bool,
    identity_include_email: bool,
    identity_include_name: bool,
    identity_jwt_audience: Option<String>,
    forward_access_token: bool,
    inject_delegation_token: bool,
    delegation_token_scope: String,
}

fn inherited_identity_fields(service: Option<&DownstreamService>) -> InheritedIdentityFields {
    match service {
        Some(service) => InheritedIdentityFields {
            identity_propagation_mode: service.identity_propagation_mode.clone(),
            identity_include_user_id: service.identity_include_user_id,
            identity_include_email: service.identity_include_email,
            identity_include_name: service.identity_include_name,
            identity_jwt_audience: service.identity_jwt_audience.clone(),
            forward_access_token: service.forward_access_token,
            inject_delegation_token: service.inject_delegation_token,
            delegation_token_scope: service.delegation_token_scope.clone(),
        },
        None => InheritedIdentityFields {
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
        },
    }
}

fn resolve_available_slug_from_existing(
    base_slug: &str,
    active_slugs: &HashSet<String>,
) -> Option<String> {
    if !active_slugs.contains(base_slug) {
        return Some(base_slug.to_string());
    }

    for n in 2..=100 {
        let candidate = format!("{base_slug}-{n}");
        if !active_slugs.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

async fn resolve_migration_user_service_slug(
    db: &Database,
    user_id: &str,
    base_slug: &str,
) -> Result<Option<String>, mongodb::error::Error> {
    let mut candidate_slugs = HashSet::new();
    candidate_slugs.insert(base_slug.to_string());
    for n in 2..=100 {
        candidate_slugs.insert(format!("{base_slug}-{n}"));
    }

    let slug_values: Vec<bson::Bson> = candidate_slugs
        .iter()
        .cloned()
        .map(bson::Bson::String)
        .collect();
    let existing: Vec<Document> = db
        .collection::<Document>(USER_SERVICES)
        .find(doc! {
            "user_id": user_id,
            "is_active": true,
            "slug": { "$in": slug_values },
        })
        .await?
        .try_collect()
        .await?;

    let existing_slugs: HashSet<String> = existing
        .into_iter()
        .filter_map(|doc| doc.get_str("slug").ok().map(str::to_owned))
        .collect();

    Ok(resolve_available_slug_from_existing(
        base_slug,
        &existing_slugs,
    ))
}

/// Migrate UserProviderTokens to the unified UserEndpoint + UserApiKey + UserService model.
async fn migrate_provider_tokens(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let tokens: Vec<UserProviderToken> = db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find(doc! {})
        .await?
        .try_collect()
        .await?;

    let mut migrated = 0u64;
    for token in &tokens {
        // Check idempotency: skip if already migrated
        let existing = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! {
                "source": "migration_provider_token",
                "source_id": &token.id,
            })
            .await?;
        if existing.is_some() {
            continue;
        }

        // Find the DownstreamService linked to this provider
        let service = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "provider_config_id": &token.provider_config_id, "is_active": true })
            .await?;

        // Load ProviderConfig for name
        let provider = db
            .collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find_one(doc! { "_id": &token.provider_config_id })
            .await?;

        let provider_name = provider
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Unknown Provider".to_string());
        let provider_slug = provider
            .as_ref()
            .map(|p| p.slug.clone())
            .unwrap_or_else(|| format!("provider-{}", &token.provider_config_id));

        let now = Utc::now();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Determine endpoint URL and catalog info
        let (endpoint_url, catalog_service_id, slug, auth_method, auth_key_name) =
            if let Some(ref svc) = service {
                let url = token
                    .gateway_url
                    .as_deref()
                    .filter(|u| !u.is_empty())
                    .unwrap_or(&svc.base_url);
                (
                    url.to_string(),
                    Some(svc.id.clone()),
                    svc.slug.clone(),
                    svc.auth_method.clone(),
                    svc.auth_key_name.clone(),
                )
            } else {
                let url = token
                    .gateway_url
                    .as_deref()
                    .filter(|u| !u.is_empty())
                    .unwrap_or("https://placeholder.invalid");
                (
                    url.to_string(),
                    None,
                    provider_slug,
                    "bearer".to_string(),
                    "Authorization".to_string(),
                )
            };

        // Skip if a UserService already exists for this user + catalog service
        // (e.g., created by an earlier token for the same provider)
        if let Some(ref csid) = catalog_service_id {
            let already_has_service = db
                .collection::<UserService>(USER_SERVICES)
                .find_one(doc! {
                    "user_id": &token.user_id,
                    "catalog_service_id": csid,
                    "is_active": true,
                })
                .await?;
            if already_has_service.is_some() {
                continue;
            }
        }

        let base_slug = slug;
        let slug = match resolve_migration_user_service_slug(db, &token.user_id, &base_slug).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(
                    user_id = %token.user_id,
                    source_id = %token.id,
                    base_slug = %base_slug,
                    "Skipping provider token migration: active user service slug space exhausted"
                );
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if slug != base_slug {
            tracing::info!(
                user_id = %token.user_id,
                source_id = %token.id,
                original_slug = %base_slug,
                resolved_slug = %slug,
                "Provider token migration resolved active user service slug collision"
            );
        }

        // Create UserEndpoint
        let endpoint = UserEndpoint {
            id: endpoint_id.clone(),
            user_id: token.user_id.clone(),
            label: provider_name.clone(),
            url: endpoint_url,
            catalog_service_id: catalog_service_id.clone(),
            openapi_spec_url: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(&endpoint)
            .await?;

        // Look up user provider credentials for OAuth app credentials
        let user_creds = db
            .collection::<UserProviderCredentials>(USER_PROVIDER_CREDENTIALS)
            .find_one(doc! {
                "user_id": &token.user_id,
                "provider_config_id": &token.provider_config_id,
            })
            .await?;

        // Create UserApiKey -- clean up endpoint on failure
        let api_key = UserApiKey {
            id: api_key_id.clone(),
            user_id: token.user_id.clone(),
            label: token.label.clone().unwrap_or(provider_name),
            credential_type: token.token_type.clone(),
            credential_encrypted: token.api_key_encrypted.clone(),
            access_token_encrypted: token.access_token_encrypted.clone(),
            refresh_token_encrypted: token.refresh_token_encrypted.clone(),
            token_scopes: token.token_scopes.clone(),
            expires_at: token.expires_at,
            provider_config_id: Some(token.provider_config_id.clone()),
            user_oauth_client_id_encrypted: user_creds
                .as_ref()
                .and_then(|c| c.client_id_encrypted.clone()),
            user_oauth_client_secret_encrypted: user_creds
                .as_ref()
                .and_then(|c| c.client_secret_encrypted.clone()),
            status: token.status.clone(),
            last_used_at: token.last_used_at,
            error_message: token.error_message.clone(),
            source: Some("migration_provider_token".to_string()),
            source_id: Some(token.id.clone()),
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&api_key)
            .await
        {
            // Clean up orphaned endpoint
            let _ = db
                .collection::<UserEndpoint>(USER_ENDPOINTS)
                .delete_one(doc! { "_id": &endpoint_id })
                .await;
            return Err(e.into());
        }

        // Create UserService -- clean up endpoint + api_key on failure
        let inherited_identity = inherited_identity_fields(service.as_ref());
        let user_service = UserService {
            id: service_id,
            user_id: token.user_id.clone(),
            slug,
            endpoint_id: endpoint_id.clone(),
            api_key_id: Some(api_key_id.clone()),
            auth_method,
            auth_key_name,
            catalog_service_id,
            node_id: None,
            node_priority: 0,
            service_type: if let Some(ref svc) = service {
                svc.service_type.clone()
            } else {
                "http".to_string()
            },
            identity_propagation_mode: inherited_identity.identity_propagation_mode,
            identity_include_user_id: inherited_identity.identity_include_user_id,
            identity_include_email: inherited_identity.identity_include_email,
            identity_include_name: inherited_identity.identity_include_name,
            identity_jwt_audience: inherited_identity.identity_jwt_audience,
            forward_access_token: inherited_identity.forward_access_token,
            inject_delegation_token: inherited_identity.inject_delegation_token,
            delegation_token_scope: inherited_identity.delegation_token_scope,
            custom_user_agent: None,
            is_active: true,
            source: Some("migration_provider_token".to_string()),
            source_id: Some(token.id.clone()),
            source_app_id: None,
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = db
            .collection::<UserService>(USER_SERVICES)
            .insert_one(&user_service)
            .await
        {
            // Clean up orphaned endpoint and api_key
            let _ = db
                .collection::<UserEndpoint>(USER_ENDPOINTS)
                .delete_one(doc! { "_id": &endpoint_id })
                .await;
            let _ = db
                .collection::<UserApiKey>(USER_API_KEYS)
                .delete_one(doc! { "_id": &api_key_id })
                .await;
            return Err(e.into());
        }

        migrated += 1;
    }

    if migrated > 0 {
        tracing::info!(
            count = migrated,
            "Migrated provider tokens to unified collections"
        );
    }
    Ok(())
}

/// Migrate UserServiceConnections to the unified model.
async fn migrate_service_connections(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let connections: Vec<UserServiceConnection> = db
        .collection::<UserServiceConnection>(USER_SERVICE_CONNECTIONS)
        .find(doc! { "is_active": true })
        .await?
        .try_collect()
        .await?;

    let mut migrated = 0u64;
    for conn in &connections {
        // Check idempotency
        let existing = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! {
                "source": "migration_connection",
                "source_id": &conn.id,
            })
            .await?;
        if existing.is_some() {
            continue;
        }

        // Load the downstream service
        let service = match db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": &conn.service_id })
            .await?
        {
            Some(s) => s,
            None => {
                tracing::warn!(
                    conn_id = %conn.id,
                    service_id = %conn.service_id,
                    "Skipping connection migration: downstream service not found"
                );
                continue;
            }
        };

        // Check if already migrated via provider token path
        let already_has_service = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! {
                "user_id": &conn.user_id,
                "catalog_service_id": &service.id,
                "is_active": true,
            })
            .await?;
        if already_has_service.is_some() {
            continue;
        }

        let slug = match resolve_migration_user_service_slug(db, &conn.user_id, &service.slug).await
        {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!(
                    user_id = %conn.user_id,
                    source_id = %conn.id,
                    base_slug = %service.slug,
                    "Skipping service connection migration: active user service slug space exhausted"
                );
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if slug != service.slug {
            tracing::info!(
                user_id = %conn.user_id,
                source_id = %conn.id,
                original_slug = %service.slug,
                resolved_slug = %slug,
                "Service connection migration resolved active user service slug collision"
            );
        }

        let now = Utc::now();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Create UserEndpoint
        let endpoint = UserEndpoint {
            id: endpoint_id.clone(),
            user_id: conn.user_id.clone(),
            label: service.name.clone(),
            url: service.base_url.clone(),
            catalog_service_id: Some(service.id.clone()),
            openapi_spec_url: service.openapi_spec_url.clone(),
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(&endpoint)
            .await?;

        // Create UserApiKey -- clean up endpoint on failure
        let cred_type = conn
            .credential_type
            .clone()
            .or_else(|| service.auth_type.clone())
            .unwrap_or_else(|| "api_key".to_string());
        let api_key = UserApiKey {
            id: api_key_id.clone(),
            user_id: conn.user_id.clone(),
            label: conn
                .credential_label
                .clone()
                .unwrap_or_else(|| service.name.clone()),
            credential_type: cred_type,
            credential_encrypted: conn.credential_encrypted.clone(),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: Some("migration_connection".to_string()),
            source_id: Some(conn.id.clone()),
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&api_key)
            .await
        {
            let _ = db
                .collection::<UserEndpoint>(USER_ENDPOINTS)
                .delete_one(doc! { "_id": &endpoint_id })
                .await;
            return Err(e.into());
        }

        // Create UserService -- clean up endpoint + api_key on failure
        let inherited_identity = inherited_identity_fields(Some(&service));
        let user_service = UserService {
            id: service_id,
            user_id: conn.user_id.clone(),
            slug,
            endpoint_id: endpoint_id.clone(),
            api_key_id: Some(api_key_id.clone()),
            auth_method: service.auth_method.clone(),
            auth_key_name: service.auth_key_name.clone(),
            catalog_service_id: Some(service.id.clone()),
            node_id: None,
            node_priority: 0,
            service_type: service.service_type.clone(),
            identity_propagation_mode: inherited_identity.identity_propagation_mode,
            identity_include_user_id: inherited_identity.identity_include_user_id,
            identity_include_email: inherited_identity.identity_include_email,
            identity_include_name: inherited_identity.identity_include_name,
            identity_jwt_audience: inherited_identity.identity_jwt_audience,
            forward_access_token: inherited_identity.forward_access_token,
            inject_delegation_token: inherited_identity.inject_delegation_token,
            delegation_token_scope: inherited_identity.delegation_token_scope,
            custom_user_agent: None,
            is_active: true,
            source: Some("migration_connection".to_string()),
            source_id: Some(conn.id.clone()),
            source_app_id: None,
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = db
            .collection::<UserService>(USER_SERVICES)
            .insert_one(&user_service)
            .await
        {
            let _ = db
                .collection::<UserEndpoint>(USER_ENDPOINTS)
                .delete_one(doc! { "_id": &endpoint_id })
                .await;
            let _ = db
                .collection::<UserApiKey>(USER_API_KEYS)
                .delete_one(doc! { "_id": &api_key_id })
                .await;
            return Err(e.into());
        }

        migrated += 1;
    }

    if migrated > 0 {
        tracing::info!(
            count = migrated,
            "Migrated service connections to unified collections"
        );
    }
    Ok(())
}

/// Migrate NodeServiceBindings: attach node_id + priority to matching UserService records.
/// For bindings with no existing UserService (e.g. SSH-only services configured purely
/// through NodeServiceBinding), create the full record set (endpoint + api_key + service).
async fn migrate_node_service_bindings(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let bindings: Vec<NodeServiceBinding> = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find(doc! { "is_active": true })
        .await?
        .try_collect()
        .await?;

    let mut updated = 0u64;
    let mut created = 0u64;
    for binding in &bindings {
        // Try to update existing UserService (created by provider_token or connection migration)
        let result = db
            .collection::<Document>(USER_SERVICES)
            .update_one(
                doc! {
                    "user_id": &binding.user_id,
                    "catalog_service_id": &binding.service_id,
                    "is_active": true,
                },
                doc! {
                    "$set": {
                        "node_id": &binding.node_id,
                        "node_priority": binding.priority,
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }
                },
            )
            .await?;

        if result.modified_count > 0 {
            updated += 1;
            continue;
        }

        // No existing UserService was updated -- check if one already exists
        let already_exists = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! {
                "user_id": &binding.user_id,
                "catalog_service_id": &binding.service_id,
                "is_active": true,
            })
            .await?;
        if already_exists.is_some() {
            continue;
        }

        // Check idempotency by source
        let migrated_before = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! {
                "source": "migration_node_binding",
                "source_id": &binding.id,
            })
            .await?;
        if migrated_before.is_some() {
            continue;
        }

        // Load DownstreamService for this binding
        let service = match db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": &binding.service_id })
            .await?
        {
            Some(s) => s,
            None => {
                tracing::warn!(
                    binding_id = %binding.id,
                    service_id = %binding.service_id,
                    "Skipping node binding migration: downstream service not found"
                );
                continue;
            }
        };

        let slug =
            match resolve_migration_user_service_slug(db, &binding.user_id, &service.slug).await {
                Ok(Some(s)) => s,
                Ok(None) => {
                    tracing::warn!(
                        user_id = %binding.user_id,
                        source_id = %binding.id,
                        base_slug = %service.slug,
                        "Skipping node binding migration: active user service slug space exhausted"
                    );
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
        if slug != service.slug {
            tracing::info!(
                user_id = %binding.user_id,
                source_id = %binding.id,
                original_slug = %service.slug,
                resolved_slug = %slug,
                "Node binding migration resolved active user service slug collision"
            );
        }

        let now = Utc::now();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let is_ssh = service.service_type == "ssh";
        let ep_url = if is_ssh {
            service
                .ssh_config
                .as_ref()
                .map(|c| format!("ssh://{}:{}", c.host, c.port))
                .unwrap_or_default()
        } else {
            service.base_url.clone()
        };

        let credential_type = if is_ssh {
            "ssh_certificate".to_string()
        } else {
            "node_managed".to_string()
        };

        // Create UserEndpoint
        let endpoint = UserEndpoint {
            id: endpoint_id.clone(),
            user_id: binding.user_id.clone(),
            label: service.name.clone(),
            url: ep_url,
            catalog_service_id: Some(service.id.clone()),
            openapi_spec_url: service.openapi_spec_url.clone(),
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(&endpoint)
            .await?;

        // Create UserApiKey (placeholder -- node-managed or SSH certificate)
        let api_key = UserApiKey {
            id: api_key_id.clone(),
            user_id: binding.user_id.clone(),
            label: service.name.clone(),
            credential_type,
            credential_encrypted: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: Some("migration_node_binding".to_string()),
            source_id: Some(binding.id.clone()),
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&api_key)
            .await
        {
            let _ = db
                .collection::<UserEndpoint>(USER_ENDPOINTS)
                .delete_one(doc! { "_id": &endpoint_id })
                .await;
            return Err(e.into());
        }

        // Create UserService with node routing
        let inherited_identity = inherited_identity_fields(Some(&service));
        let user_service = UserService {
            id: service_id,
            user_id: binding.user_id.clone(),
            slug,
            endpoint_id: endpoint_id.clone(),
            api_key_id: Some(api_key_id.clone()),
            auth_method: service.auth_method.clone(),
            auth_key_name: service.auth_key_name.clone(),
            catalog_service_id: Some(service.id.clone()),
            node_id: Some(binding.node_id.clone()),
            node_priority: binding.priority,
            service_type: service.service_type.clone(),
            identity_propagation_mode: inherited_identity.identity_propagation_mode,
            identity_include_user_id: inherited_identity.identity_include_user_id,
            identity_include_email: inherited_identity.identity_include_email,
            identity_include_name: inherited_identity.identity_include_name,
            identity_jwt_audience: inherited_identity.identity_jwt_audience,
            forward_access_token: inherited_identity.forward_access_token,
            inject_delegation_token: inherited_identity.inject_delegation_token,
            delegation_token_scope: inherited_identity.delegation_token_scope,
            custom_user_agent: None,
            is_active: true,
            source: Some("migration_node_binding".to_string()),
            source_id: Some(binding.id.clone()),
            source_app_id: None,
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = db
            .collection::<UserService>(USER_SERVICES)
            .insert_one(&user_service)
            .await
        {
            let _ = db
                .collection::<UserEndpoint>(USER_ENDPOINTS)
                .delete_one(doc! { "_id": &endpoint_id })
                .await;
            let _ = db
                .collection::<UserApiKey>(USER_API_KEYS)
                .delete_one(doc! { "_id": &api_key_id })
                .await;
            return Err(e.into());
        }

        created += 1;
    }

    if updated > 0 || created > 0 {
        tracing::info!(
            updated,
            created,
            "Migrated node service bindings to unified collections"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_downstream_service() -> DownstreamService {
        DownstreamService {
            id: "svc-1".to_string(),
            name: "Test".to_string(),
            slug: "test".to_string(),
            description: None,
            base_url: "https://example.com".to_string(),
            service_type: "http".to_string(),
            visibility: "public".to_string(),
            auth_method: "header".to_string(),
            auth_key_name: "Authorization".to_string(),
            credential_encrypted: vec![],
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
            oauth_client_id: None,
            service_category: "connection".to_string(),
            requires_user_credential: true,
            is_active: true,
            created_by: "system".to_string(),
            identity_propagation_mode: "both".to_string(),
            identity_include_user_id: true,
            identity_include_email: true,
            identity_include_name: false,
            identity_jwt_audience: Some("https://aud.example.com".to_string()),
            forward_access_token: false,
            inject_delegation_token: true,
            delegation_token_scope: "proxy:* llm:status".to_string(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn inherited_identity_fields_default_when_catalog_service_missing() {
        let fields = inherited_identity_fields(None);
        assert_eq!(fields.identity_propagation_mode, "none");
        assert!(!fields.identity_include_user_id);
        assert!(!fields.forward_access_token);
        assert!(!fields.inject_delegation_token);
        assert_eq!(fields.delegation_token_scope, "llm:proxy");
    }

    #[test]
    fn inherited_identity_fields_preserve_catalog_settings() {
        let service = sample_downstream_service();

        let fields = inherited_identity_fields(Some(&service));
        assert_eq!(fields.identity_propagation_mode, "both");
        assert!(fields.identity_include_user_id);
        assert!(fields.identity_include_email);
        assert_eq!(
            fields.identity_jwt_audience.as_deref(),
            Some("https://aud.example.com")
        );
        assert!(!fields.forward_access_token);
        assert!(fields.inject_delegation_token);
        assert_eq!(fields.delegation_token_scope, "proxy:* llm:status");
    }

    #[test]
    fn resolve_available_slug_uses_base_when_available() {
        let resolved = resolve_available_slug_from_existing("llm-openai", &HashSet::new());
        assert_eq!(resolved.as_deref(), Some("llm-openai"));
    }

    #[test]
    fn resolve_available_slug_suffixes_from_active_conflicts() {
        let active_slugs = HashSet::from([
            "llm-openai".to_string(),
            "llm-openai-2".to_string(),
            "llm-openai-4".to_string(),
        ]);

        let resolved = resolve_available_slug_from_existing("llm-openai", &active_slugs);
        assert_eq!(resolved.as_deref(), Some("llm-openai-3"));
    }

    #[test]
    fn resolve_available_slug_returns_none_when_suffix_space_exhausted() {
        let active_slugs: HashSet<String> = std::iter::once("llm-openai".to_string())
            .chain((2..=100).map(|n| format!("llm-openai-{n}")))
            .collect();

        let resolved = resolve_available_slug_from_existing("llm-openai", &active_slugs);
        assert!(resolved.is_none());
    }
}
