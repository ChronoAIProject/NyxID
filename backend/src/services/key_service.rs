use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::token::{generate_api_key, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};

/// Result returned when a new API key is created.
/// The `full_key` is shown once and never stored.
pub struct CreatedApiKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub full_key: String,
    pub scopes: String,
    pub created_at: chrono::DateTime<Utc>,
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

/// Create a new API key for a user.
pub async fn create_api_key(
    db: &mongodb::Database,
    user_id: &str,
    name: &str,
    scopes: &str,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> AppResult<CreatedApiKey> {
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::ValidationError(
            "API key name must be between 1 and 100 characters".to_string(),
        ));
    }

    // Validate scopes against allowed set
    validate_api_key_scopes(scopes)?;

    let (prefix, full_key, key_hash) = generate_api_key();
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
    })
}

/// List all API keys for a user (without exposing the full key).
pub async fn list_api_keys(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<ApiKey>> {
    let keys: Vec<ApiKey> = db
        .collection::<ApiKey>(API_KEYS)
        .find(doc! { "user_id": user_id, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(keys)
}

/// Delete (deactivate) an API key.
pub async fn delete_api_key(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
) -> AppResult<()> {
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

/// Rotate an API key: deactivate the old one and create a new one with the same name and scopes.
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

    let name = old_key.name.clone();
    let scopes = old_key.scopes.clone();
    let expires_at = old_key.expires_at;

    // Deactivate old key
    db.collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! { "_id": &old_key.id },
            doc! { "$set": { "is_active": false } },
        )
        .await?;

    // Create new key
    let new_key = create_api_key(db, user_id, &name, &scopes, expires_at).await?;

    tracing::info!(
        old_key_id = %key_id,
        new_key_id = %new_key.id,
        user_id = %user_id,
        "API key rotated"
    );

    Ok(new_key)
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
        && expires_at < Utc::now() {
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
