use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use std::{
    fmt,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

use crate::crypto::token::{generate_api_key, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::agent_service_binding::{
    AgentServiceBinding, COLLECTION_NAME as AGENT_BINDINGS,
};
use crate::models::api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS};
use crate::redaction::RedactedLen;
use crate::services::{
    api_key_mutation_service as key_mutations,
    api_key_scope_service::{self, ScopeAuthorization},
};

/// Result returned when a new API key is created.
/// The `full_key` is shown once and never stored.
pub struct CreatedApiKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub full_key: String,
    pub scopes: String,
    pub created_at: chrono::DateTime<Utc>,
    pub rotation_predecessor_id: Option<String>,
    pub state_version: i64,
    pub updated_at: chrono::DateTime<Utc>,
    pub description: Option<String>,
    pub allowed_service_ids: Vec<String>,
    pub allowed_node_ids: Vec<String>,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    pub platform: Option<String>,
    pub purpose: ApiKeyPurpose,
    pub scheduled_write_enabled: bool,
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
            .field("rotation_predecessor_id", &self.rotation_predecessor_id)
            .field("state_version", &self.state_version)
            .field("updated_at", &self.updated_at)
            .field("description", &self.description)
            .field("allowed_service_ids", &self.allowed_service_ids)
            .field("allowed_node_ids", &self.allowed_node_ids)
            .field("allow_all_services", &self.allow_all_services)
            .field("allow_all_nodes", &self.allow_all_nodes)
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("rate_limit_burst", &self.rate_limit_burst)
            .field("platform", &self.platform)
            .field("purpose", &self.purpose)
            .field("scheduled_write_enabled", &self.scheduled_write_enabled)
            .finish()
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum ApiKeyRotationOutcome {
    Created(CreatedApiKey),
    AlreadyCommitted(ApiKey),
}

#[derive(Clone)]
struct RotationMaterial {
    key_prefix: String,
    full_key: String,
    key_hash: String,
    created_at: chrono::DateTime<Utc>,
}

enum RotationTransactionOutcome {
    Created(ApiKey),
    AlreadyCommitted(ApiKey),
}

fn created_api_key_from_model(key: ApiKey, full_key: String) -> AppResult<CreatedApiKey> {
    let updated_at = key.updated_at.ok_or_else(|| {
        AppError::Internal("created API key lacks authoritative updated_at".to_string())
    })?;
    Ok(CreatedApiKey {
        id: key.id,
        name: key.name,
        key_prefix: key.key_prefix,
        full_key,
        scopes: key.scopes,
        created_at: key.created_at,
        rotation_predecessor_id: key.rotation_predecessor_id,
        state_version: key.state_version,
        updated_at,
        description: key.description,
        allowed_service_ids: key.allowed_service_ids,
        allowed_node_ids: key.allowed_node_ids,
        allow_all_services: key.allow_all_services,
        allow_all_nodes: key.allow_all_nodes,
        rate_limit_per_second: key.rate_limit_per_second,
        rate_limit_burst: key.rate_limit_burst,
        platform: key.platform,
        purpose: key.purpose,
        scheduled_write_enabled: key.scheduled_write_enabled,
    })
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
    create_api_key_with_scope_authorization(
        db,
        user_id,
        None,
        name,
        scopes,
        expires_at,
        description,
        allowed_service_ids,
        allowed_node_ids,
        allow_all_services,
        allow_all_nodes,
        rate_limit_per_second,
        rate_limit_burst,
        platform,
        callback_url,
        None,
    )
    .await
}

/// Create a new API key while validating explicit scope IDs against the
/// permissions of `scope_actor_user_id`. Personal keys may include permitted
/// org resources under #1133 semantics; org-owned keys remain owner-bound.
#[allow(clippy::too_many_arguments)]
pub async fn create_api_key_with_scope_authorization(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
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
    scope_plan_digest: Option<&str>,
) -> AppResult<CreatedApiKey> {
    create_api_key_with_security_class_and_id(
        db,
        user_id,
        scope_actor_user_id,
        None,
        name,
        scopes,
        expires_at,
        description,
        allowed_service_ids,
        allowed_node_ids,
        allow_all_services,
        allow_all_nodes,
        rate_limit_per_second,
        rate_limit_burst,
        platform,
        callback_url,
        scope_plan_digest,
        ApiKeyPurpose::General,
        false,
    )
    .await
}

/// Create an API key at a caller-reserved UUID.
///
/// The assistant action receipt owns this identifier before the effect starts,
/// so a retry can recover an already-created key without minting another one.
#[allow(clippy::too_many_arguments)]
pub async fn create_api_key_with_scope_authorization_and_id(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    resource_id: &str,
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
    scope_plan_digest: Option<&str>,
) -> AppResult<CreatedApiKey> {
    create_api_key_with_security_class_and_id(
        db,
        user_id,
        scope_actor_user_id,
        Some(resource_id),
        name,
        scopes,
        expires_at,
        description,
        allowed_service_ids,
        allowed_node_ids,
        allow_all_services,
        allow_all_nodes,
        rate_limit_per_second,
        rate_limit_burst,
        platform,
        callback_url,
        scope_plan_digest,
        ApiKeyPurpose::General,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_api_key_with_security_class(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
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
    scope_plan_digest: Option<&str>,
    purpose: ApiKeyPurpose,
    scheduled_write_enabled: bool,
) -> AppResult<CreatedApiKey> {
    create_api_key_with_security_class_and_id(
        db,
        user_id,
        scope_actor_user_id,
        None,
        name,
        scopes,
        expires_at,
        description,
        allowed_service_ids,
        allowed_node_ids,
        allow_all_services,
        allow_all_nodes,
        rate_limit_per_second,
        rate_limit_burst,
        platform,
        callback_url,
        scope_plan_digest,
        purpose,
        scheduled_write_enabled,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn create_api_key_with_security_class_and_id(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    resource_id: Option<&str>,
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
    scope_plan_digest: Option<&str>,
    purpose: ApiKeyPurpose,
    scheduled_write_enabled: bool,
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

    if purpose == ApiKeyPurpose::ScheduledInvocation && (all_svcs || all_nodes || scopes != "proxy")
    {
        return Err(AppError::DurableGrantMismatch(
            "scheduled_invocation keys require exact service/node scopes and scopes='proxy'"
                .to_string(),
        ));
    }

    if let Some(expected_digest) = scope_plan_digest {
        let actor_user_id = scope_actor_user_id.ok_or_else(|| {
            AppError::ApiKeyScopePlanOwnerUnsupported(
                "scope_plan_digest requires an authenticated actor context".to_string(),
            )
        })?;
        api_key_scope_service::verify_scope_plan_precondition(
            db,
            actor_user_id,
            user_id,
            &svc_ids,
            &node_ids,
            all_svcs,
            all_nodes,
            expected_digest,
        )
        .await?;
    }

    // Validate service/node IDs if restricted
    if !all_svcs {
        api_key_scope_service::validate_service_ids(
            db,
            user_id,
            &svc_ids,
            ScopeAuthorization::for_actor(scope_actor_user_id),
        )
        .await?;
    }
    if !all_nodes {
        api_key_scope_service::validate_node_ids(
            db,
            user_id,
            &node_ids,
            ScopeAuthorization::for_actor(scope_actor_user_id),
        )
        .await?;
    }

    let scoped = is_scoped_key(all_svcs, all_nodes);
    let (prefix, full_key, key_hash) = if scoped {
        generate_scoped_api_key()
    } else {
        generate_api_key()
    };

    let id = match resource_id {
        Some(value) => Uuid::parse_str(value)
            .map_err(|_| {
                AppError::ValidationError("reserved API key id must be a UUID".to_string())
            })?
            .to_string(),
        None => Uuid::new_v4().to_string(),
    };
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
        rotation_predecessor_id: None,
        state_version: 1,
        updated_at: Some(now),
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
        purpose,
        scheduled_write_enabled,
    };

    key_mutations::insert_one(db, &new_key, None).await?;

    Ok(CreatedApiKey {
        id,
        name: name.to_string(),
        key_prefix: prefix,
        full_key,
        scopes: scopes.to_string(),
        created_at: now,
        rotation_predecessor_id: None,
        state_version: 1,
        updated_at: now,
        description: description.map(|s| s.to_string()),
        allowed_service_ids: svc_ids,
        allowed_node_ids: node_ids,
        allow_all_services: all_svcs,
        allow_all_nodes: all_nodes,
        rate_limit_per_second,
        rate_limit_burst,
        platform: platform.map(|s| s.to_string()),
        purpose,
        scheduled_write_enabled,
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
    delete_api_key_with_expected_state_version(db, user_id, key_id, None).await
}

/// Delete an API key, optionally fencing the write on `expected_state_version`
/// so a stale precondition cannot commit.
pub async fn delete_api_key_with_expected_state_version(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
    expected_state_version: Option<i64>,
) -> AppResult<()> {
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;
    if let Some(expected) = expected_state_version
        && key.state_version != expected
    {
        return Err(stale_api_key_conflict());
    }

    let mut filter = doc! { "_id": &key.id, "user_id": user_id };
    if let Some(expected) = expected_state_version {
        filter.insert("state_version", expected);
    }

    let result =
        key_mutations::update_one(db, filter, doc! { "$set": { "is_active": false } }, None)
            .await?;
    if result.matched_count != 1 {
        return Err(unmatched_api_key_write(db, user_id, key_id, expected_state_version).await?);
    }

    tracing::info!(key_id = %key_id, user_id = %user_id, "API key deactivated");

    Ok(())
}

fn stale_api_key_conflict() -> AppError {
    AppError::Conflict("the API key changed since this action was prepared".to_string())
}

async fn unmatched_api_key_write(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
    expected_state_version: Option<i64>,
) -> AppResult<AppError> {
    let current = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "user_id": user_id })
        .await?;
    match current {
        None => Ok(AppError::NotFound("API key not found".to_string())),
        Some(key) if !key.is_active => Ok(AppError::NotFound("API key not found".to_string())),
        Some(key)
            if expected_state_version.is_some_and(|expected| key.state_version != expected) =>
        {
            Ok(stale_api_key_conflict())
        }
        Some(_) => Ok(AppError::Internal(
            "API key disappeared after update".to_string(),
        )),
    }
}

/// Rotate an API key: deactivate the old one and create a new one preserving name, scopes, and scope fields.
#[allow(dead_code)]
pub async fn rotate_api_key(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
) -> AppResult<CreatedApiKey> {
    rotate_api_key_with_scope_authorization(db, user_id, None, key_id).await
}

pub async fn rotate_api_key_with_scope_authorization(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    key_id: &str,
) -> AppResult<CreatedApiKey> {
    let successor_id = Uuid::new_v4().to_string();
    match rotate_api_key_with_scope_authorization_and_id(
        db,
        user_id,
        scope_actor_user_id,
        key_id,
        &successor_id,
    )
    .await?
    {
        ApiKeyRotationOutcome::Created(created) => Ok(created),
        ApiKeyRotationOutcome::AlreadyCommitted(_) => Err(AppError::Internal(
            "fresh rotation successor unexpectedly already existed".to_string(),
        )),
    }
}

/// Atomically rotate an exact predecessor to a caller-reserved successor UUID.
/// Replaying the same pair returns safe committed metadata and never replays
/// the one-time successor secret.
pub async fn rotate_api_key_with_scope_authorization_and_id(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    predecessor_id: &str,
    successor_id: &str,
) -> AppResult<ApiKeyRotationOutcome> {
    rotate_api_key_with_scope_authorization_and_id_inner(
        db,
        user_id,
        scope_actor_user_id,
        predecessor_id,
        successor_id,
        #[cfg(test)]
        None,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn rotate_api_key_with_scope_authorization_and_id_with_collision_hook(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    predecessor_id: &str,
    successor_id: &str,
    collision_hook: key_mutations::TransactionCollisionHook,
) -> AppResult<ApiKeyRotationOutcome> {
    rotate_api_key_with_scope_authorization_and_id_inner(
        db,
        user_id,
        scope_actor_user_id,
        predecessor_id,
        successor_id,
        Some(collision_hook),
    )
    .await
}

async fn rotate_api_key_with_scope_authorization_and_id_inner(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
    predecessor_id: &str,
    successor_id: &str,
    #[cfg(test)] collision_hook: Option<key_mutations::TransactionCollisionHook>,
) -> AppResult<ApiKeyRotationOutcome> {
    let successor_id = Uuid::parse_str(successor_id)
        .map_err(|_| AppError::ValidationError("successor_id must be a UUID".to_string()))?
        .to_string();
    if predecessor_id == successor_id {
        return Err(AppError::ValidationError(
            "rotation predecessor and successor must differ".to_string(),
        ));
    }

    let db = db.clone();
    let user_id = user_id.to_string();
    let predecessor_id = predecessor_id.to_string();
    let tracing_user_id = user_id.clone();
    let tracing_predecessor_id = predecessor_id.clone();
    let actor_id = scope_actor_user_id.map(str::to_string);
    let material: Arc<Mutex<Option<RotationMaterial>>> = Arc::new(Mutex::new(None));
    let material_for_transaction = Arc::clone(&material);
    let successor_for_transaction = successor_id.clone();
    let mut session = db.client().start_session().await?;

    let transaction = session
        .start_transaction()
        .and_run2(async move |session| {
            #[cfg(test)]
            if let Some(hook) = collision_hook.as_ref() {
                hook.begin_attempt();
            }
            let operation: AppResult<RotationTransactionOutcome> = async {
                let api_keys = db.collection::<ApiKey>(API_KEYS);

                let replay_successor = if let Some(existing) = api_keys
                    .find_one(doc! { "_id": &successor_for_transaction })
                    .session(&mut *session)
                    .await?
                {
                    if existing.user_id != user_id {
                        return Err(AppError::NotFound("API key not found".to_string()));
                    }
                    if existing.rotation_predecessor_id.as_deref() == Some(predecessor_id.as_str())
                        && existing.state_version > 0
                        && existing.updated_at.is_some()
                    {
                        Some(existing)
                    } else {
                        return Err(AppError::Conflict(
                            "reserved rotation successor is already in use".to_string(),
                        ));
                    }
                } else {
                    None
                };

                let old_key = api_keys
                    .find_one(doc! {
                        "_id": &predecessor_id,
                        "user_id": &user_id,
                    })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;
                if old_key.purpose == ApiKeyPurpose::ScheduledInvocation {
                    return Err(AppError::DurableGrantMismatch(
                        "scheduled_invocation keys must be reprovisioned from a fresh scope plan"
                            .to_string(),
                    ));
                }

                let authorization = ScopeAuthorization::for_actor(actor_id.as_deref());
                api_key_scope_service::validate_owner_write_with_session(
                    &db,
                    &user_id,
                    authorization,
                    &mut *session,
                )
                .await?;
                if !old_key.allow_all_services {
                    api_key_scope_service::validate_service_ids_with_session(
                        &db,
                        &user_id,
                        &old_key.allowed_service_ids,
                        authorization,
                        &mut *session,
                    )
                    .await?;
                }
                if !old_key.allow_all_nodes {
                    api_key_scope_service::validate_node_ids_with_session(
                        &db,
                        &user_id,
                        &old_key.allowed_node_ids,
                        authorization,
                        &mut *session,
                    )
                    .await?;
                }

                if let Some(existing) = replay_successor {
                    if old_key.is_active {
                        return Err(AppError::Conflict(
                            "rotation lineage is not committed".to_string(),
                        ));
                    }
                    return Ok(RotationTransactionOutcome::AlreadyCommitted(existing));
                }

                if let Some(existing_successor) = api_keys
                    .find_one(doc! {
                        "user_id": &user_id,
                        "rotation_predecessor_id": &predecessor_id,
                    })
                    .session(&mut *session)
                    .await?
                {
                    return Err(AppError::Conflict(format!(
                        "API key was already rotated to successor {}",
                        existing_successor.id
                    )));
                }
                if !old_key.is_active {
                    return Err(AppError::NotFound("API key not found".to_string()));
                }

                let mut cursor = db
                    .collection::<AgentServiceBinding>(AGENT_BINDINGS)
                    .find(doc! { "api_key_id": &old_key.id, "user_id": &user_id })
                    .session(&mut *session)
                    .await?;
                let old_bindings: Vec<AgentServiceBinding> =
                    cursor.stream(&mut *session).try_collect().await?;

                #[cfg(test)]
                if let Some(hook) = collision_hook.as_ref() {
                    hook.after_reads().await;
                }

                let rotation_material = {
                    let mut guard = material_for_transaction.lock().map_err(|_| {
                        AppError::Internal("rotation material lock poisoned".to_string())
                    })?;
                    guard
                        .get_or_insert_with(|| {
                            let (key_prefix, full_key, key_hash) = if is_scoped_key(
                                old_key.allow_all_services,
                                old_key.allow_all_nodes,
                            ) {
                                generate_scoped_api_key()
                            } else {
                                generate_api_key()
                            };
                            RotationMaterial {
                                key_prefix,
                                full_key,
                                key_hash,
                                created_at: Utc::now(),
                            }
                        })
                        .clone()
                };
                let successor = ApiKey {
                    id: successor_for_transaction.clone(),
                    user_id: old_key.user_id.clone(),
                    name: old_key.name.clone(),
                    key_prefix: rotation_material.key_prefix,
                    key_hash: rotation_material.key_hash,
                    scopes: old_key.scopes.clone(),
                    last_used_at: None,
                    expires_at: old_key.expires_at,
                    is_active: true,
                    created_at: rotation_material.created_at,
                    rotation_predecessor_id: Some(old_key.id.clone()),
                    state_version: 1,
                    updated_at: Some(rotation_material.created_at),
                    description: old_key.description.clone(),
                    allowed_service_ids: old_key.allowed_service_ids.clone(),
                    allowed_node_ids: old_key.allowed_node_ids.clone(),
                    allow_all_services: old_key.allow_all_services,
                    allow_all_nodes: old_key.allow_all_nodes,
                    rate_limit_per_second: old_key.rate_limit_per_second,
                    rate_limit_burst: old_key.rate_limit_burst,
                    platform: old_key.platform.clone(),
                    callback_url: old_key.callback_url.clone(),
                    purpose: old_key.purpose,
                    scheduled_write_enabled: old_key.scheduled_write_enabled,
                };

                let deactivated = key_mutations::update_one(
                    &db,
                    doc! {
                        "_id": &old_key.id,
                        "user_id": &user_id,
                        "is_active": true,
                    },
                    doc! { "$set": { "is_active": false } },
                    Some(&mut *session),
                )
                .await?;
                if deactivated.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "rotation predecessor changed concurrently".to_string(),
                    ));
                }
                key_mutations::insert_one(&db, &successor, Some(&mut *session)).await?;

                if !old_bindings.is_empty() {
                    let now = rotation_material.created_at;
                    let replacements: Vec<AgentServiceBinding> = old_bindings
                        .into_iter()
                        .map(|binding| AgentServiceBinding {
                            id: Uuid::new_v4().to_string(),
                            api_key_id: successor.id.clone(),
                            user_service_id: binding.user_service_id,
                            user_api_key_id: binding.user_api_key_id,
                            user_id: user_id.clone(),
                            created_at: now,
                            updated_at: now,
                        })
                        .collect();
                    db.collection::<AgentServiceBinding>(AGENT_BINDINGS)
                        .insert_many(replacements)
                        .session(&mut *session)
                        .await?;
                }

                Ok(RotationTransactionOutcome::Created(successor))
            }
            .await;
            key_mutations::transaction_result(operation)
        })
        .await
        .map_err(key_mutations::map_transaction_error)?;

    match transaction {
        RotationTransactionOutcome::AlreadyCommitted(key) => {
            Ok(ApiKeyRotationOutcome::AlreadyCommitted(key))
        }
        RotationTransactionOutcome::Created(key) => {
            let full_key = material
                .lock()
                .map_err(|_| AppError::Internal("rotation material lock poisoned".to_string()))?
                .as_ref()
                .map(|material| material.full_key.clone())
                .ok_or_else(|| {
                    AppError::Internal("rotation committed without key material".to_string())
                })?;
            tracing::info!(
                old_key_id = %tracing_predecessor_id,
                new_key_id = %key.id,
                user_id = %tracing_user_id,
                "API key rotated"
            );
            Ok(ApiKeyRotationOutcome::Created(created_api_key_from_model(
                key, full_key,
            )?))
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Update scope fields on an existing API key while validating explicit
/// service/node IDs against an optional actor permission context.
pub async fn update_api_key_scope_with_scope_authorization(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
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
    scope_plan_digest: Option<&str>,
) -> AppResult<ApiKey> {
    update_api_key_scope_with_expected_state_version(
        db,
        user_id,
        scope_actor_user_id,
        key_id,
        name,
        description,
        scopes,
        allowed_service_ids,
        allowed_node_ids,
        allow_all_services,
        allow_all_nodes,
        rate_limit_per_second,
        rate_limit_burst,
        platform,
        callback_url,
        scope_plan_digest,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
/// Like [`update_api_key_scope_with_scope_authorization`], but the write
/// filter includes `expected_state_version` when set so a stale read cannot
/// commit over a concurrent mutation.
pub async fn update_api_key_scope_with_expected_state_version(
    db: &mongodb::Database,
    user_id: &str,
    scope_actor_user_id: Option<&str>,
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
    scope_plan_digest: Option<&str>,
    expected_state_version: Option<i64>,
) -> AppResult<ApiKey> {
    let existing = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;
    if let Some(expected) = expected_state_version
        && existing.state_version != expected
    {
        return Err(stale_api_key_conflict());
    }

    if existing.purpose == ApiKeyPurpose::ScheduledInvocation
        && (scopes.is_some()
            || allowed_service_ids.is_some()
            || allowed_node_ids.is_some()
            || allow_all_services.is_some()
            || allow_all_nodes.is_some()
            || platform.is_some()
            || callback_url.is_some()
            || scope_plan_digest.is_some())
    {
        return Err(AppError::DurableGrantMismatch(
            "scheduled_invocation authority is immutable; use durable grant reauthorization"
                .to_string(),
        ));
    }

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

    if let Some(expected_digest) = scope_plan_digest {
        let actor_user_id = scope_actor_user_id.ok_or_else(|| {
            AppError::ApiKeyScopePlanOwnerUnsupported(
                "scope_plan_digest requires an authenticated actor context".to_string(),
            )
        })?;
        api_key_scope_service::verify_scope_plan_precondition(
            db,
            actor_user_id,
            user_id,
            allowed_service_ids.unwrap_or(&existing.allowed_service_ids),
            allowed_node_ids.unwrap_or(&existing.allowed_node_ids),
            effective_all_svcs,
            effective_all_nodes,
            expected_digest,
        )
        .await?;
    }

    if let Some(sids) = allowed_service_ids
        && !effective_all_svcs
    {
        api_key_scope_service::validate_service_ids(
            db,
            user_id,
            sids,
            ScopeAuthorization::for_actor(scope_actor_user_id),
        )
        .await?;
    }
    if let Some(nids) = allowed_node_ids
        && !effective_all_nodes
    {
        api_key_scope_service::validate_node_ids(
            db,
            user_id,
            nids,
            ScopeAuthorization::for_actor(scope_actor_user_id),
        )
        .await?;
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

    let mut filter = doc! { "_id": key_id, "user_id": user_id, "is_active": true };
    if let Some(expected) = expected_state_version {
        filter.insert("state_version", expected);
    }

    let result = key_mutations::update_one(db, filter, doc! { "$set": update }, None).await?;
    if result.matched_count != 1 {
        return Err(unmatched_api_key_write(db, user_id, key_id, expected_state_version).await?);
    }

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
    key_mutations::update_one(
        db,
        doc! { "_id": &key.id },
        doc! { "$set": { "last_used_at": bson::DateTime::from_chrono(now) } },
        None,
    )
    .await?;

    Ok((user_id, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{connect_test_database, connect_transaction_test_database};
    use crate::test_utils::{test_membership, test_user, test_user_service};

    fn test_node(owner_id: &str, name: &str) -> Node {
        let now = Utc::now();
        Node {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: name.to_string(),
            status: NodeStatus::Online,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: Some(now),
            connected_at: Some(now),
            metadata: None,
            metrics: NodeMetrics::default(),
            connection_owner: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    async fn insert_scope_fixture_user(db: &mongodb::Database, user_id: &str, user_type: UserType) {
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(user_id, user_type))
            .await
            .expect("insert user");
    }

    async fn insert_scope_fixture_service(
        db: &mongodb::Database,
        owner_id: &str,
        slug: &str,
        admin_only: bool,
    ) -> UserService {
        let service_id = Uuid::new_v4().to_string();
        let endpoint_id = Uuid::new_v4().to_string();
        let mut service = test_user_service(&service_id, owner_id, slug, &endpoint_id, None, None);
        service.admin_only = admin_only;
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert service");
        service
    }

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
        assert_eq!(created.rotation_predecessor_id, None);
        assert_eq!(created.state_version, 1);
        assert_eq!(created.updated_at, created.created_at);
    }

    #[tokio::test]
    async fn create_api_key_with_reserved_id_initializes_authority() {
        let Some(db) = connect_test_database("key_svc_create_reserved").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let reserved_id = Uuid::new_v4().to_string();
        let created = create_api_key_with_scope_authorization_and_id(
            &db,
            &user_id,
            Some(&user_id),
            &reserved_id,
            "reserved",
            "proxy",
            None,
            None,
            None,
            None,
            Some(true),
            Some(true),
            None,
            None,
            Some("codex"),
            None,
            None,
        )
        .await
        .expect("create reserved key");

        assert_eq!(created.id, reserved_id);
        assert_eq!(created.state_version, 1);
        assert_eq!(created.updated_at, created.created_at);
    }

    #[tokio::test]
    async fn personal_api_key_scope_accepts_member_permitted_org_service_and_node() {
        let Some(db) = connect_test_database("key_svc_scope_org_member_ok").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_scope_fixture_user(&db, &actor_id, UserType::Person).await;
        insert_scope_fixture_user(&db, &org_id, UserType::Org).await;

        let org_service = insert_scope_fixture_service(&db, &org_id, "org-proxyable", false).await;
        let org_node = test_node(&org_id, "org-node");
        db.collection::<Node>(NODES)
            .insert_one(&org_node)
            .await
            .expect("insert node");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &org_id,
                &actor_id,
                OrgRole::Member,
                Some(vec![org_service.id.clone()]),
            ))
            .await
            .expect("insert membership");

        let service_ids = [org_service.id.clone()];
        let node_ids = [org_node.id.clone()];
        let created = create_api_key_with_scope_authorization(
            &db,
            &actor_id,
            Some(&actor_id),
            "member-org-scope",
            "proxy",
            None,
            None,
            Some(&service_ids),
            Some(&node_ids),
            Some(false),
            Some(false),
            None,
            None,
            Some("codex"),
            None,
            None,
        )
        .await
        .expect("member can scope personal key to permitted org resources");

        assert_eq!(created.allowed_service_ids, vec![org_service.id]);
        assert_eq!(created.allowed_node_ids, vec![org_node.id]);
        assert!(!created.allow_all_services);
        assert!(!created.allow_all_nodes);
    }

    #[tokio::test]
    async fn personal_api_key_scope_rejects_member_admin_only_org_service() {
        let Some(db) = connect_test_database("key_svc_scope_org_admin_only_reject").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_scope_fixture_user(&db, &actor_id, UserType::Person).await;
        insert_scope_fixture_user(&db, &org_id, UserType::Org).await;

        let org_service = insert_scope_fixture_service(&db, &org_id, "admin-only", true).await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &org_id,
                &actor_id,
                OrgRole::Member,
                Some(vec![org_service.id.clone()]),
            ))
            .await
            .expect("insert membership");

        let service_ids = [org_service.id];
        let err = create_api_key_with_scope_authorization(
            &db,
            &actor_id,
            Some(&actor_id),
            "blocked-admin-only",
            "proxy",
            None,
            None,
            Some(&service_ids),
            None,
            Some(false),
            Some(true),
            None,
            None,
            Some("codex"),
            None,
            None,
        )
        .await
        .expect_err("members cannot scope to admin-only org services");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn personal_api_key_scope_rejects_viewer_org_node() {
        let Some(db) = connect_test_database("key_svc_scope_viewer_node_reject").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_scope_fixture_user(&db, &actor_id, UserType::Person).await;
        insert_scope_fixture_user(&db, &org_id, UserType::Org).await;

        let org_node = test_node(&org_id, "viewer-org-node");
        db.collection::<Node>(NODES)
            .insert_one(&org_node)
            .await
            .expect("insert node");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Viewer, None))
            .await
            .expect("insert membership");

        let node_ids = [org_node.id];
        let err = create_api_key_with_scope_authorization(
            &db,
            &actor_id,
            Some(&actor_id),
            "blocked-viewer-node",
            "proxy",
            None,
            None,
            None,
            Some(&node_ids),
            Some(true),
            Some(false),
            None,
            None,
            Some("codex"),
            None,
            None,
        )
        .await
        .expect_err("viewers cannot scope personal keys to org nodes");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn org_owned_api_key_scope_stays_owner_bound() {
        let Some(db) = connect_test_database("key_svc_scope_org_owner_bound").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_scope_fixture_user(&db, &actor_id, UserType::Person).await;
        insert_scope_fixture_user(&db, &org_id, UserType::Org).await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let personal_service =
            insert_scope_fixture_service(&db, &actor_id, "personal-service", false).await;
        let service_ids = [personal_service.id];
        let err = create_api_key_with_scope_authorization(
            &db,
            &org_id,
            None,
            "org-owned-cross-scope",
            "proxy",
            None,
            None,
            Some(&service_ids),
            None,
            Some(false),
            Some(true),
            None,
            None,
            Some("codex"),
            None,
            None,
        )
        .await
        .expect_err("org-owned keys must not scope to personal resources");

        assert!(matches!(err, AppError::ValidationError(_)));
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
        let deactivated = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &created.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!deactivated.is_active);
        assert_eq!(deactivated.state_version, 2);
        assert!(
            deactivated
                .updated_at
                .is_some_and(|value| value >= created.updated_at)
        );
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
        let db = connect_transaction_test_database("key_svc_rotate").await;
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
    async fn rotate_with_reserved_successor_is_atomic_and_replay_safe() {
        let db = connect_transaction_test_database("key_svc_rotate_reserved").await;
        crate::db::ensure_indexes(&db)
            .await
            .expect("create lineage index");

        let user_id = Uuid::new_v4().to_string();
        let original = create_api_key(
            &db,
            &user_id,
            "rotate-reserved",
            "read write",
            None,
            Some("lineage evidence"),
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
        .expect("create predecessor");
        let successor_id = Uuid::new_v4().to_string();

        let rotated = match rotate_api_key_with_scope_authorization_and_id(
            &db,
            &user_id,
            Some(&user_id),
            &original.id,
            &successor_id,
        )
        .await
        .expect("rotate exact lineage")
        {
            ApiKeyRotationOutcome::Created(created) => created,
            ApiKeyRotationOutcome::AlreadyCommitted(_) => {
                panic!("first rotation must return the one-time secret")
            }
        };
        assert_eq!(rotated.id, successor_id);
        assert_eq!(
            rotated.rotation_predecessor_id.as_deref(),
            Some(original.id.as_str())
        );
        assert_eq!(rotated.state_version, 1);
        assert_eq!(rotated.updated_at, rotated.created_at);
        assert_ne!(rotated.full_key, original.full_key);

        let predecessor = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &original.id })
            .await
            .expect("read predecessor")
            .expect("predecessor remains as lineage evidence");
        assert!(!predecessor.is_active);
        assert_eq!(predecessor.state_version, 2);
        assert!(
            predecessor
                .updated_at
                .is_some_and(|at| at >= original.updated_at)
        );

        let replay = rotate_api_key_with_scope_authorization_and_id(
            &db,
            &user_id,
            Some(&user_id),
            &original.id,
            &successor_id,
        )
        .await
        .expect("replay committed lineage");
        let replayed = match replay {
            ApiKeyRotationOutcome::AlreadyCommitted(key) => key,
            ApiKeyRotationOutcome::Created(_) => panic!("replay must not expose a second secret"),
        };
        assert_eq!(replayed.id, successor_id);
        assert_eq!(
            replayed.rotation_predecessor_id.as_deref(),
            Some(original.id.as_str())
        );
        assert_eq!(replayed.state_version, 1);

        let owner_isolation = rotate_api_key_with_scope_authorization_and_id(
            &db,
            &Uuid::new_v4().to_string(),
            None,
            &original.id,
            &successor_id,
        )
        .await
        .expect_err("another owner must not learn that the successor exists");
        assert!(matches!(owner_isolation, AppError::NotFound(_)));

        let conflict = rotate_api_key_with_scope_authorization_and_id(
            &db,
            &user_id,
            Some(&user_id),
            &original.id,
            &Uuid::new_v4().to_string(),
        )
        .await
        .expect_err("one predecessor cannot acquire another successor");
        assert!(matches!(conflict, AppError::Conflict(_)));

        delete_api_key(&db, &user_id, &successor_id)
            .await
            .expect("deactivate committed successor");
        let historical_replay = rotate_api_key_with_scope_authorization_and_id(
            &db,
            &user_id,
            Some(&user_id),
            &original.id,
            &successor_id,
        )
        .await
        .expect("committed lineage remains replayable after successor deactivation");
        let historical = match historical_replay {
            ApiKeyRotationOutcome::AlreadyCommitted(key) => key,
            ApiKeyRotationOutcome::Created(_) => {
                panic!("historical replay must not mint or expose another secret")
            }
        };
        assert!(!historical.is_active);
        assert_eq!(historical.state_version, 2);
    }

    #[tokio::test]
    async fn reserved_rotation_is_owner_isolated() {
        let db = connect_transaction_test_database("key_svc_rotate_owner_isolation").await;
        let owner_id = Uuid::new_v4().to_string();
        let other_user_id = Uuid::new_v4().to_string();
        let original = create_api_key(
            &db,
            &owner_id,
            "owner-key",
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
        .expect("create owner key");

        let error = rotate_api_key_with_scope_authorization_and_id(
            &db,
            &other_user_id,
            Some(&other_user_id),
            &original.id,
            &Uuid::new_v4().to_string(),
        )
        .await
        .expect_err("another owner cannot rotate the key");
        assert!(matches!(error, AppError::NotFound(_)));

        let untouched = get_api_key(&db, &owner_id, &original.id)
            .await
            .expect("failed rotation leaves predecessor active");
        assert_eq!(untouched.state_version, 1);
    }

    #[tokio::test]
    async fn rotation_revalidates_revoked_org_admin_authority() {
        let db = connect_transaction_test_database("key_svc_rotate_revoked_admin").await;
        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_scope_fixture_user(&db, &actor_id, UserType::Person).await;
        insert_scope_fixture_user(&db, &org_id, UserType::Org).await;
        let service = insert_scope_fixture_service(&db, &org_id, "org-service", false).await;
        let membership = test_membership(&org_id, &actor_id, OrgRole::Admin, None);
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(&membership)
            .await
            .expect("insert admin membership");

        let service_ids = [service.id];
        let original = create_api_key_with_scope_authorization(
            &db,
            &org_id,
            Some(&actor_id),
            "org-key",
            "proxy",
            None,
            None,
            Some(&service_ids),
            None,
            Some(false),
            Some(true),
            None,
            None,
            Some("codex"),
            None,
            None,
        )
        .await
        .expect("admin creates org key");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .update_one(
                doc! { "_id": &membership.id },
                doc! { "$set": { "revoked_at": bson::DateTime::now() } },
            )
            .await
            .expect("revoke membership");

        let error = rotate_api_key_with_scope_authorization_and_id(
            &db,
            &org_id,
            Some(&actor_id),
            &original.id,
            &Uuid::new_v4().to_string(),
        )
        .await
        .expect_err("rotation must revalidate current org authority");
        assert!(matches!(error, AppError::OrgRoleInsufficient(_)));

        let untouched = get_api_key(&db, &org_id, &original.id)
            .await
            .expect("revoked rotation leaves predecessor active");
        assert_eq!(untouched.state_version, 1);
    }

    #[tokio::test]
    async fn rotation_replay_revalidates_revoked_org_admin_authority() {
        let db = connect_transaction_test_database("key_svc_rotate_replay_revoked_admin").await;
        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_scope_fixture_user(&db, &actor_id, UserType::Person).await;
        insert_scope_fixture_user(&db, &org_id, UserType::Org).await;
        let service = insert_scope_fixture_service(&db, &org_id, "org-replay-service", false).await;
        let membership = test_membership(&org_id, &actor_id, OrgRole::Admin, None);
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(&membership)
            .await
            .expect("insert admin membership");

        let service_ids = [service.id];
        let original = create_api_key_with_scope_authorization(
            &db,
            &org_id,
            Some(&actor_id),
            "org-replay-key",
            "proxy",
            None,
            None,
            Some(&service_ids),
            None,
            Some(false),
            Some(true),
            None,
            None,
            Some("codex"),
            None,
            None,
        )
        .await
        .expect("admin creates org key");
        let successor_id = Uuid::new_v4().to_string();
        assert!(matches!(
            rotate_api_key_with_scope_authorization_and_id(
                &db,
                &org_id,
                Some(&actor_id),
                &original.id,
                &successor_id,
            )
            .await
            .expect("initial rotation"),
            ApiKeyRotationOutcome::Created(_)
        ));

        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .update_one(
                doc! { "_id": &membership.id },
                doc! { "$set": { "revoked_at": bson::DateTime::now() } },
            )
            .await
            .expect("revoke membership");
        let error = rotate_api_key_with_scope_authorization_and_id(
            &db,
            &org_id,
            Some(&actor_id),
            &original.id,
            &successor_id,
        )
        .await
        .expect_err("replay must revalidate current org authority");
        assert!(matches!(error, AppError::OrgRoleInsufficient(_)));
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
        let touched = get_api_key(&db, &user_id, &created.id)
            .await
            .expect("key remains active");
        assert_eq!(touched.state_version, 2);
        assert!(touched.last_used_at.is_some());
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
        let updated = update_api_key_scope_with_scope_authorization(
            &db,
            &user_id,
            None,
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
            None,
        )
        .await
        .expect("should update");
        assert_eq!(updated.name, "new-name");
        assert_eq!(updated.state_version, 2);
        assert!(
            updated
                .updated_at
                .is_some_and(|value| value >= created.updated_at)
        );
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
        let updated = update_api_key_scope_with_scope_authorization(
            &db,
            &user_id,
            None,
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
        let updated = update_api_key_scope_with_scope_authorization(
            &db,
            &user_id,
            None,
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
            None,
        )
        .await
        .expect("should update");
        assert_eq!(updated.rate_limit_per_second, None);
        assert_eq!(updated.rate_limit_burst, None);
    }

    #[tokio::test]
    async fn update_with_expected_state_version_does_not_clobber_concurrent_write() {
        let Some(db) = connect_test_database("key_svc_fence").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db,
            &user_id,
            "fence-key",
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
        assert_eq!(created.state_version, 1);

        let concurrent = update_api_key_scope_with_expected_state_version(
            &db,
            &user_id,
            None,
            &created.id,
            Some("from-concurrent"),
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
            None,
            Some(1),
        )
        .await
        .expect("concurrent authorized write at version 1");
        assert_eq!(concurrent.state_version, 2);
        assert_eq!(concurrent.name, "from-concurrent");

        let stale = update_api_key_scope_with_expected_state_version(
            &db,
            &user_id,
            None,
            &created.id,
            Some("from-stale"),
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
            None,
            Some(1),
        )
        .await
        .expect_err("stale expected version must not commit");
        assert!(matches!(stale, AppError::Conflict(_)));

        let stored = get_api_key(&db, &user_id, &created.id)
            .await
            .expect("key still active");
        assert_eq!(stored.state_version, 2);
        assert_eq!(
            stored.name, "from-concurrent",
            "stale write must not clobber the concurrent mutation"
        );
    }

    #[tokio::test]
    async fn delete_with_expected_state_version_rejects_stale_precondition() {
        let Some(db) = connect_test_database("key_svc_del_fence").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let created = create_api_key(
            &db,
            &user_id,
            "delete-fence",
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
        update_api_key_scope_with_scope_authorization(
            &db,
            &user_id,
            None,
            &created.id,
            Some("renamed"),
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
            None,
        )
        .await
        .expect("advance version");

        let stale = delete_api_key_with_expected_state_version(&db, &user_id, &created.id, Some(1))
            .await
            .expect_err("stale delete must not deactivate");
        assert!(matches!(stale, AppError::Conflict(_)));
        let stored = get_api_key(&db, &user_id, &created.id)
            .await
            .expect("key still active");
        assert!(stored.is_active);
        assert_eq!(stored.state_version, 2);
    }
}
