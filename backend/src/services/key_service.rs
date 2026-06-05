use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use std::fmt;
use uuid::Uuid;

use crate::crypto::token::{generate_api_key, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::agent_service_binding::{
    AgentServiceBinding, COLLECTION_NAME as AGENT_BINDINGS,
};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::redaction::RedactedLen;

/// Result returned when a new API key is created.
/// The `full_key` is shown once and never stored.
pub struct CreatedApiKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub full_key: String,
    pub scopes: String,
    pub created_at: chrono::DateTime<Utc>,
    pub description: Option<String>,
    pub allowed_service_ids: Vec<String>,
    pub allowed_node_ids: Vec<String>,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    pub platform: Option<String>,
}

impl fmt::Debug for CreatedApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedApiKey")
            .field("id", &RedactedLen(self.id.len()))
            .field("name", &self.name)
            .field("key_prefix", &RedactedLen(self.key_prefix.len()))
            .field("full_key", &RedactedLen(self.full_key.len()))
            .field("scopes", &self.scopes)
            .field("created_at", &self.created_at)
            .field("description", &self.description)
            .field("allowed_service_ids", &self.allowed_service_ids)
            .field("allowed_node_ids", &self.allowed_node_ids)
            .field("allow_all_services", &self.allow_all_services)
            .field("allow_all_nodes", &self.allow_all_nodes)
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("rate_limit_burst", &self.rate_limit_burst)
            .field("platform", &self.platform)
            .finish()
    }
}

/// Valid scopes that can be assigned to API keys.
const VALID_API_KEY_SCOPES: &[&str] = &[
    "read",
    "write",
    "admin",
    "openid",
    "profile",
    "email",
    "services:read",
    "services:write",
    "proxy",
];

/// Valid platform identifiers for API keys.
const VALID_PLATFORMS: &[&str] = &[
    "claude-code",
    "cursor",
    "codex",
    "openclaw",
    "generic",
    "device-code",
    "device-onboard",
];

/// Validate the platform field if provided.
fn validate_platform(platform: Option<&str>) -> AppResult<()> {
    if let Some(p) = platform
        && !VALID_PLATFORMS.contains(&p)
    {
        return Err(AppError::ValidationError(format!(
            "Invalid platform '{}'. Valid platforms: {}",
            p,
            VALID_PLATFORMS.join(", ")
        )));
    }
    Ok(())
}

/// Validate that all requested scopes are from the allowed set.
fn validate_api_key_scopes(scopes: &str) -> AppResult<()> {
    if scopes.is_empty() {
        return Err(AppError::ValidationError(
            "At least one scope is required".to_string(),
        ));
    }

    for scope in scopes.split_whitespace() {
        if !VALID_API_KEY_SCOPES.contains(&scope) {
            return Err(AppError::ValidationError(format!(
                "Invalid scope '{}'. Valid scopes: {}",
                scope,
                VALID_API_KEY_SCOPES.join(", ")
            )));
        }
    }

    Ok(())
}

/// Validate that all service IDs belong to the user and are active.
async fn validate_service_ids(
    db: &mongodb::Database,
    user_id: &str,
    service_ids: &[String],
) -> AppResult<()> {
    for sid in service_ids {
        let exists = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": sid, "user_id": user_id, "is_active": true })
            .await?;
        if exists.is_none() {
            return Err(AppError::ValidationError(format!(
                "UserService '{}' not found or not owned by user",
                sid
            )));
        }
    }
    Ok(())
}

/// Validate that all node IDs belong to the user.
async fn validate_node_ids(
    db: &mongodb::Database,
    user_id: &str,
    node_ids: &[String],
) -> AppResult<()> {
    for nid in node_ids {
        let exists = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": nid, "user_id": user_id, "is_active": true })
            .await?;
        if exists.is_none() {
            return Err(AppError::ValidationError(format!(
                "Node '{}' not found or not owned by user",
                nid
            )));
        }
    }
    Ok(())
}

/// Determine whether the key should use the scoped `nyxid_ag_` prefix.
/// A key is scoped if either `allow_all` flag is false.
fn is_scoped_key(allow_all_services: bool, allow_all_nodes: bool) -> bool {
    !allow_all_services || !allow_all_nodes
}

fn generate_scoped_api_key() -> (String, String, String) {
    use rand::RngCore;
    use sha2::{Digest, Sha256};

    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);

    let hex_encoded = hex::encode(bytes);
    let full_key = format!("nyxid_ag_{hex_encoded}");
    let prefix = format!("nyxid_ag_{}", &hex_encoded[..8]);
    let mut hasher = Sha256::new();
    hasher.update(full_key.as_bytes());
    let hash = hex::encode(hasher.finalize());

    (prefix, full_key, hash)
}

/// Create a new API key for a user, optionally with service/node scope.
#[allow(clippy::too_many_arguments)]
pub async fn create_api_key(
    db: &mongodb::Database,
    user_id: &str,
    name: &str,
    scopes: &str,
    expires_at: Option<chrono::DateTime<Utc>>,
    description: Option<&str>,
    allowed_service_ids: Option<&[String]>,
    allowed_node_ids: Option<&[String]>,
    allow_all_services: Option<bool>,
    allow_all_nodes: Option<bool>,
    rate_limit_per_second: Option<u32>,
    rate_limit_burst: Option<u32>,
    platform: Option<&str>,
    callback_url: Option<&str>,
) -> AppResult<CreatedApiKey> {
    if name.is_empty() || name.len() > 200 {
        return Err(AppError::ValidationError(
            "API key name must be between 1 and 200 characters".to_string(),
        ));
    }

    validate_api_key_scopes(scopes)?;
    validate_platform(platform)?;

    let svc_ids = allowed_service_ids.unwrap_or(&[]).to_vec();
    let node_ids = allowed_node_ids.unwrap_or(&[]).to_vec();
    let all_svcs = allow_all_services.unwrap_or(true);
    let all_nodes = allow_all_nodes.unwrap_or(true);

    // Validate service/node IDs if restricted
    if !all_svcs {
        validate_service_ids(db, user_id, &svc_ids).await?;
    }
    if !all_nodes {
        validate_node_ids(db, user_id, &node_ids).await?;
    }

    let scoped = is_scoped_key(all_svcs, all_nodes);
    let (prefix, full_key, key_hash) = if scoped {
        generate_scoped_api_key()
    } else {
        generate_api_key()
    };

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let new_key = ApiKey {
        id: id.clone(),
        user_id: user_id.to_string(),
        name: name.to_string(),
        key_prefix: prefix.clone(),
        key_hash,
        scopes: scopes.to_string(),
        last_used_at: None,
        expires_at,
        is_active: true,
        created_at: now,
        description: description.map(|s| s.to_string()),
        allowed_service_ids: svc_ids.clone(),
        allowed_node_ids: node_ids.clone(),
        allow_all_services: all_svcs,
        allow_all_nodes: all_nodes,
        rate_limit_per_second,
        rate_limit_burst,
        platform: platform.map(|s| s.to_string()),
        callback_url: {
            if let Some(url) = callback_url {
                crate::services::url_validation::validate_base_url(url)?;
                Some(url.to_string())
            } else {
                None
            }
        },
    };

    db.collection::<ApiKey>(API_KEYS)
        .insert_one(&new_key)
        .await?;

    Ok(CreatedApiKey {
        id,
        name: name.to_string(),
        key_prefix: prefix,
        full_key,
        scopes: scopes.to_string(),
        created_at: now,
        description: description.map(|s| s.to_string()),
        allowed_service_ids: svc_ids,
        allowed_node_ids: node_ids,
        allow_all_services: all_svcs,
        allow_all_nodes: all_nodes,
        rate_limit_per_second,
        rate_limit_burst,
        platform: platform.map(|s| s.to_string()),
    })
}

/// List all API keys for a user (without exposing the full key).
pub async fn list_api_keys(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<ApiKey>> {
    let keys: Vec<ApiKey> = db
        .collection::<ApiKey>(API_KEYS)
        .find(doc! { "user_id": user_id, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(keys)
}

/// Get a single API key by ID, verifying ownership.
pub async fn get_api_key(db: &mongodb::Database, user_id: &str, key_id: &str) -> AppResult<ApiKey> {
    db.collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))
}

/// Delete (deactivate) an API key.
pub async fn delete_api_key(db: &mongodb::Database, user_id: &str, key_id: &str) -> AppResult<()> {
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! { "_id": &key.id },
            doc! { "$set": { "is_active": false } },
        )
        .await?;

    tracing::info!(key_id = %key_id, user_id = %user_id, "API key deactivated");

    Ok(())
}

/// Rotate an API key: deactivate the old one and create a new one preserving name, scopes, and scope fields.
pub async fn rotate_api_key(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
) -> AppResult<CreatedApiKey> {
    let old_key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    // Snapshot old bindings BEFORE deactivating so we can clone them onto the new key.
    let old_bindings: Vec<AgentServiceBinding> = db
        .collection::<AgentServiceBinding>(AGENT_BINDINGS)
        .find(doc! { "api_key_id": &old_key.id, "user_id": user_id })
        .await?
        .try_collect()
        .await?;

    // Deactivate old key
    db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! { "_id": &old_key.id },
            doc! { "$set": { "is_active": false } },
        )
        .await?;

    // Create new key preserving all fields
    let new_key = create_api_key(
        db,
        user_id,
        &old_key.name,
        &old_key.scopes,
        old_key.expires_at,
        old_key.description.as_deref(),
        Some(&old_key.allowed_service_ids),
        Some(&old_key.allowed_node_ids),
        Some(old_key.allow_all_services),
        Some(old_key.allow_all_nodes),
        old_key.rate_limit_per_second,
        old_key.rate_limit_burst,
        old_key.platform.as_deref(),
        old_key.callback_url.as_deref(),
    )
    .await?;

    // Clone agent_service_bindings from the old key to the new key so per-service
    // credential overrides survive rotation. The (api_key_id, user_service_id)
    // unique index is satisfied because new_key.id differs from old_key.id.
    let cloned_count = if old_bindings.is_empty() {
        0
    } else {
        let now = Utc::now();
        let new_bindings: Vec<AgentServiceBinding> = old_bindings
            .iter()
            .map(|b| AgentServiceBinding {
                id: Uuid::new_v4().to_string(),
                api_key_id: new_key.id.clone(),
                user_service_id: b.user_service_id.clone(),
                user_api_key_id: b.user_api_key_id.clone(),
                user_id: user_id.to_string(),
                created_at: now,
                updated_at: now,
            })
            .collect();
        let count = new_bindings.len();
        db.collection::<AgentServiceBinding>(AGENT_BINDINGS)
            .insert_many(&new_bindings)
            .await?;
        count
    };

    tracing::info!(
        old_key_id = %key_id,
        new_key_id = %new_key.id,
        user_id = %user_id,
        cloned_bindings = cloned_count,
        "API key rotated"
    );

    Ok(new_key)
}

#[allow(clippy::too_many_arguments)]
/// Update scope fields on an existing API key.
pub async fn update_api_key_scope(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    scopes: Option<&str>,
    allowed_service_ids: Option<&[String]>,
    allowed_node_ids: Option<&[String]>,
    allow_all_services: Option<bool>,
    allow_all_nodes: Option<bool>,
    rate_limit_per_second: Option<Option<u32>>,
    rate_limit_burst: Option<Option<u32>>,
    platform: Option<Option<&str>>,
    callback_url: Option<Option<&str>>,
) -> AppResult<ApiKey> {
    let existing = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    if let Some(n) = name
        && (n.is_empty() || n.len() > 200)
    {
        return Err(AppError::ValidationError(
            "API key name must be between 1 and 200 characters".to_string(),
        ));
    }
    if let Some(platform) = platform {
        validate_platform(platform)?;
    }

    let effective_all_svcs = allow_all_services.unwrap_or(existing.allow_all_services);
    let effective_all_nodes = allow_all_nodes.unwrap_or(existing.allow_all_nodes);

    if let Some(sids) = allowed_service_ids
        && !effective_all_svcs
    {
        validate_service_ids(db, user_id, sids).await?;
    }
    if let Some(nids) = allowed_node_ids
        && !effective_all_nodes
    {
        validate_node_ids(db, user_id, nids).await?;
    }

    let mut update = doc! {};

    if let Some(n) = name {
        update.insert("name", n);
    }
    if let Some(d) = description {
        update.insert("description", d);
    }
    if let Some(s) = scopes {
        update.insert("scopes", s);
    }
    if let Some(sids) = allowed_service_ids {
        update.insert("allowed_service_ids", sids);
    }
    if let Some(nids) = allowed_node_ids {
        update.insert("allowed_node_ids", nids);
    }
    if let Some(v) = allow_all_services {
        update.insert("allow_all_services", v);
    }
    if let Some(v) = allow_all_nodes {
        update.insert("allow_all_nodes", v);
    }
    if let Some(rps) = rate_limit_per_second {
        match rps {
            Some(v) => {
                update.insert("rate_limit_per_second", v as i32);
            }
            None => {
                update.insert("rate_limit_per_second", bson::Bson::Null);
            }
        }
    }
    if let Some(burst) = rate_limit_burst {
        match burst {
            Some(v) => {
                update.insert("rate_limit_burst", v as i32);
            }
            None => {
                update.insert("rate_limit_burst", bson::Bson::Null);
            }
        }
    }
    if let Some(platform) = platform {
        match platform {
            Some(value) => {
                update.insert("platform", value);
            }
            None => {
                update.insert("platform", bson::Bson::Null);
            }
        }
    }
    if let Some(url) = callback_url {
        match url {
            Some(value) if !value.trim().is_empty() => {
                crate::services::url_validation::validate_base_url(value)?;
                update.insert("callback_url", value);
            }
            _ => {
                update.insert("callback_url", bson::Bson::Null);
            }
        }
    }

    if update.is_empty() {
        return Ok(existing);
    }

    db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! { "_id": key_id, "user_id": user_id },
            doc! { "$set": update },
        )
        .await?;

    db.collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::Internal("API key disappeared after update".to_string()))
}

/// Validate an API key from a request. Returns the user_id if valid.
pub async fn validate_api_key(
    db: &mongodb::Database,
    raw_key: &str,
) -> AppResult<(String, ApiKey)> {
    let key_hash = hash_token(raw_key);

    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "key_hash": &key_hash, "is_active": true })
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid API key".to_string()))?;

    // Check expiration
    if let Some(expires_at) = key.expires_at
        && expires_at < Utc::now()
    {
        return Err(AppError::Unauthorized("API key has expired".to_string()));
    }

    // Update last_used_at
    let user_id = key.user_id.clone();
    let now = Utc::now();
    db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! { "_id": &key.id },
            doc! { "$set": { "last_used_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;

    Ok((user_id, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::connect_test_database;

    // ---------------------------------------------------------------
    // Pure function tests (no MongoDB needed)
    // ---------------------------------------------------------------

    #[test]
    fn validate_platform_accepts_none() {
        assert!(validate_platform(None).is_ok());
    }

    #[test]
    fn validate_platform_accepts_all_valid_values() {
        for p in &["claude-code", "cursor", "codex", "openclaw", "generic"] {
            assert!(validate_platform(Some(p)).is_ok(), "should accept {p}");
        }
    }

    #[test]
    fn validate_platform_rejects_invalid() {
        let result = validate_platform(Some("unknown-platform"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ValidationError(_)));
    }

    #[test]
    fn validate_platform_rejects_empty_string() {
        let result = validate_platform(Some(""));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ValidationError(_)));
    }

    #[test]
    fn validate_scopes_accepts_single_valid() {
        assert!(validate_api_key_scopes("read").is_ok());
    }

    #[test]
    fn validate_scopes_accepts_multiple_valid() {
        assert!(validate_api_key_scopes("read write proxy").is_ok());
    }

    #[test]
    fn validate_scopes_accepts_all_valid_scopes() {
        assert!(
            validate_api_key_scopes(
                "read write admin openid profile email services:read services:write proxy"
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_scopes_rejects_empty() {
        let result = validate_api_key_scopes("");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ValidationError(_)));
    }

    #[test]
    fn validate_scopes_rejects_invalid_scope() {
        let result = validate_api_key_scopes("read bogus write");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ValidationError(_)));
    }

    #[test]
    fn is_scoped_key_both_true_returns_false() {
        assert!(!is_scoped_key(true, true));
    }

    #[test]
    fn is_scoped_key_services_false_returns_true() {
        assert!(is_scoped_key(false, true));
    }

    #[test]
    fn is_scoped_key_nodes_false_returns_true() {
        assert!(is_scoped_key(true, false));
    }

    #[test]
    fn is_scoped_key_both_false_returns_true() {
        assert!(is_scoped_key(false, false));
    }

    #[test]
    fn generate_scoped_api_key_format() {
        let (prefix, full_key, hash) = generate_scoped_api_key();
        assert!(
            prefix.starts_with("nyxid_ag_"),
            "prefix should start with nyxid_ag_, got: {prefix}"
        );
        assert!(
            full_key.starts_with("nyxid_ag_"),
            "full_key should start with nyxid_ag_, got: {full_key}"
        );
        assert_eq!(hash.len(), 64, "hash should be 64 hex chars");
        assert!(
            hex::decode(&hash).is_ok(),
            "hash should be valid hex: {hash}"
        );
    }

    #[test]
    fn generate_scoped_api_key_unique() {
        let (_, key_a, hash_a) = generate_scoped_api_key();
        let (_, key_b, hash_b) = generate_scoped_api_key();
        assert_ne!(key_a, key_b, "two generated keys should differ");
        assert_ne!(hash_a, hash_b, "two generated hashes should differ");
    }

    #[test]
    fn generate_scoped_api_key_prefix_is_subset_of_full_key() {
        let (prefix, full_key, _) = generate_scoped_api_key();
        assert!(
            full_key.starts_with(&prefix),
            "full_key should start with prefix"
        );
    }

    // ---------------------------------------------------------------
    // Integration tests (require MongoDB)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn create_api_key_rejects_empty_name() {
        let Some(db) = connect_test_database("key_svc_create_empty").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let result = create_api_key(
            &db, &user_id, "", "read", None, None, None, None, None, None, None, None, None, None,
        )
        .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_api_key_rejects_too_long_name() {
        let Some(db) = connect_test_database("key_svc_create_long").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let long_name = "a".repeat(201);
        let result = create_api_key(
            &db, &user_id, &long_name, "read", None, None, None, None, None, None, None, None,
            None, None,
        )
        .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_api_key_rejects_invalid_scope() {
        let Some(db) = connect_test_database("key_svc_create_scope").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let result = create_api_key(
            &db,
            &user_id,
            "test",
            "invalid_scope",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_api_key_happy_path() {
        let Some(db) = connect_test_database("key_svc_create_ok").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db,
            &user_id,
            "my-key",
            "read write",
            None,
            Some("test key"),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("claude-code"),
            None,
        )
        .await
        .expect("should create key");
        assert_eq!(created.name, "my-key");
        assert_eq!(created.scopes, "read write");
        assert_eq!(created.description.as_deref(), Some("test key"));
        assert_eq!(created.platform.as_deref(), Some("claude-code"));
        assert!(created.allow_all_services);
        assert!(created.allow_all_nodes);
        assert!(!created.full_key.is_empty());
    }

    #[tokio::test]
    async fn list_api_keys_empty() {
        let Some(db) = connect_test_database("key_svc_list_empty").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let keys = list_api_keys(&db, &user_id).await.expect("should list");
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn list_api_keys_returns_created_keys() {
        let Some(db) = connect_test_database("key_svc_list").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        create_api_key(
            &db, &user_id, "key-1", "read", None, None, None, None, None, None, None, None, None,
            None,
        )
        .await
        .expect("create key-1");
        create_api_key(
            &db, &user_id, "key-2", "write", None, None, None, None, None, None, None, None, None,
            None,
        )
        .await
        .expect("create key-2");
        let keys = list_api_keys(&db, &user_id).await.expect("should list");
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn get_api_key_not_found() {
        let Some(db) = connect_test_database("key_svc_get_nf").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let result = get_api_key(&db, &user_id, "nonexistent-id").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_api_key_happy_path() {
        let Some(db) = connect_test_database("key_svc_get_ok").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db,
            &user_id,
            "look-me-up",
            "proxy",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create key");
        let fetched = get_api_key(&db, &user_id, &created.id)
            .await
            .expect("should find");
        assert_eq!(fetched.name, "look-me-up");
        assert_eq!(fetched.scopes, "proxy");
    }

    #[tokio::test]
    async fn delete_api_key_deactivates() {
        let Some(db) = connect_test_database("key_svc_del").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db,
            &user_id,
            "to-delete",
            "read",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create key");
        delete_api_key(&db, &user_id, &created.id)
            .await
            .expect("should deactivate");
        let result = get_api_key(&db, &user_id, &created.id).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_api_key_not_found() {
        let Some(db) = connect_test_database("key_svc_del_nf").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let result = delete_api_key(&db, &user_id, "ghost-id").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn rotate_api_key_preserves_fields() {
        let Some(db) = connect_test_database("key_svc_rotate").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let original = create_api_key(
            &db,
            &user_id,
            "rotate-me",
            "read write",
            None,
            Some("rotatable"),
            None,
            None,
            None,
            None,
            Some(50),
            Some(100),
            Some("codex"),
            None,
        )
        .await
        .expect("create key");
        let rotated = rotate_api_key(&db, &user_id, &original.id)
            .await
            .expect("should rotate");
        assert_ne!(rotated.id, original.id);
        assert_ne!(rotated.full_key, original.full_key);
        assert_eq!(rotated.name, "rotate-me");
        assert_eq!(rotated.scopes, "read write");
        assert_eq!(rotated.description.as_deref(), Some("rotatable"));
        assert_eq!(rotated.platform.as_deref(), Some("codex"));
        assert_eq!(rotated.rate_limit_per_second, Some(50));
        assert_eq!(rotated.rate_limit_burst, Some(100));
        // Old key should be deactivated
        let result = get_api_key(&db, &user_id, &original.id).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn validate_api_key_happy_path() {
        let Some(db) = connect_test_database("key_svc_val_ok").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db,
            &user_id,
            "validate-me",
            "read",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create key");
        let (returned_uid, key) = validate_api_key(&db, &created.full_key)
            .await
            .expect("should validate");
        assert_eq!(returned_uid, user_id);
        assert_eq!(key.name, "validate-me");
    }

    #[tokio::test]
    async fn validate_api_key_invalid_key_errors() {
        let Some(db) = connect_test_database("key_svc_val_bad").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let result = validate_api_key(&db, "totally-bogus-key").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn validate_api_key_expired_key_errors() {
        let Some(db) = connect_test_database("key_svc_val_exp").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let past = Utc::now() - chrono::Duration::hours(1);
        let created = create_api_key(
            &db,
            &user_id,
            "expired",
            "read",
            Some(past),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create key");
        let result = validate_api_key(&db, &created.full_key).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn update_api_key_scope_name() {
        let Some(db) = connect_test_database("key_svc_upd_name").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db, &user_id, "old-name", "read", None, None, None, None, None, None, None, None,
            None, None,
        )
        .await
        .expect("create key");
        let updated = update_api_key_scope(
            &db,
            &user_id,
            &created.id,
            Some("new-name"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("should update");
        assert_eq!(updated.name, "new-name");
    }

    #[tokio::test]
    async fn update_api_key_scope_platform() {
        let Some(db) = connect_test_database("key_svc_upd_plat").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db,
            &user_id,
            "plat-test",
            "read",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create key");
        let updated = update_api_key_scope(
            &db,
            &user_id,
            &created.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(Some("cursor")),
            None,
        )
        .await
        .expect("should update");
        assert_eq!(updated.platform.as_deref(), Some("cursor"));
    }

    #[tokio::test]
    async fn update_api_key_scope_clear_rate_limit() {
        let Some(db) = connect_test_database("key_svc_upd_rl").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db,
            &user_id,
            "rl-test",
            "read",
            None,
            None,
            None,
            None,
            None,
            None,
            Some(10),
            Some(20),
            None,
            None,
        )
        .await
        .expect("create key");
        assert_eq!(created.rate_limit_per_second, Some(10));
        let updated = update_api_key_scope(
            &db,
            &user_id,
            &created.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(None), // clear rate_limit_per_second
            Some(None), // clear rate_limit_burst
            None,
            None,
        )
        .await
        .expect("should update");
        assert_eq!(updated.rate_limit_per_second, None);
        assert_eq!(updated.rate_limit_burst, None);
    }
}
