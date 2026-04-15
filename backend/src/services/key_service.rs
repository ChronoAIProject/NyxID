use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::token::{generate_api_key, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};

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
const VALID_PLATFORMS: &[&str] = &["claude-code", "cursor", "codex", "openclaw", "generic"];

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

    tracing::info!(
        old_key_id = %key_id,
        new_key_id = %new_key.id,
        user_id = %user_id,
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
