use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::bson::{self, doc};
use rand::RngCore;
use serde::Serialize;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::{self, JwtKeys};
use crate::crypto::token::{constant_time_eq, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
use crate::models::service_account_token::{COLLECTION_NAME as SA_TOKENS, ServiceAccountToken};

#[derive(Debug, Serialize)]
pub struct ClientCredentialsResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
}

/// Generate a client_id: "sa_" + 24 hex chars (12 random bytes).
fn generate_client_id() -> String {
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("sa_{}", hex::encode(bytes))
}

/// Generate a client_secret: "sas_" + 64 hex chars (32 random bytes).
fn generate_client_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("sas_{}", hex::encode(bytes))
}

/// Create a new service account. Returns (ServiceAccount, raw_client_secret).
///
/// Note: Duplicate names are intentionally allowed. The `client_id` is the
/// unique identifier; names are for human display only.
pub async fn create_service_account(
    db: &Database,
    name: &str,
    description: Option<&str>,
    allowed_scopes: &str,
    role_ids: &[String],
    rate_limit_override: Option<u64>,
    created_by: &str,
) -> AppResult<(ServiceAccount, String)> {
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::ValidationError(
            "Service account name must be between 1 and 100 characters".to_string(),
        ));
    }

    if let Some(d) = description
        && d.len() > 500
    {
        return Err(AppError::ValidationError(
            "Description must be 500 characters or less".to_string(),
        ));
    }

    // Scopes are free-form strings. Validation against a known scope vocabulary
    // is not enforced here; unrecognized scopes will simply not match any
    // access control rules at request time.
    if allowed_scopes.is_empty() {
        return Err(AppError::ValidationError(
            "At least one scope is required".to_string(),
        ));
    }

    if let Some(rl) = rate_limit_override
        && rl == 0
    {
        return Err(AppError::ValidationError(
            "Rate limit override must be greater than 0".to_string(),
        ));
    }

    if !role_ids.is_empty() {
        let existing_count = db
            .collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME)
            .count_documents(doc! { "_id": { "$in": role_ids } })
            .await?;
        if existing_count != role_ids.len() as u64 {
            return Err(AppError::ValidationError(
                "One or more role IDs do not exist".to_string(),
            ));
        }
    }

    let id = Uuid::new_v4().to_string();
    let client_id = generate_client_id();
    let raw_secret = generate_client_secret();
    let secret_hash = hash_token(&raw_secret);
    let secret_prefix = raw_secret[..8].to_string();
    let now = Utc::now();

    let sa = ServiceAccount {
        id,
        name: name.to_string(),
        description: description.map(String::from),
        client_id,
        client_secret_hash: secret_hash,
        secret_prefix,
        role_ids: role_ids.to_vec(),
        allowed_scopes: allowed_scopes.to_string(),
        is_active: true,
        rate_limit_override,
        created_by: created_by.to_string(),
        owner_user_id: Some(created_by.to_string()),
        created_at: now,
        updated_at: now,
        last_authenticated_at: None,
    };

    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .insert_one(&sa)
        .await?;

    Ok((sa, raw_secret))
}

/// List all service accounts (paginated).
pub async fn list_service_accounts(
    db: &Database,
    page: u64,
    per_page: u64,
    search: Option<&str>,
) -> AppResult<(Vec<ServiceAccount>, u64)> {
    let offset = (page - 1) * per_page;

    let filter = match search {
        Some(s) if !s.is_empty() => {
            let escaped = regex::escape(s);
            doc! { "name": { "$regex": &escaped, "$options": "i" } }
        }
        _ => doc! {},
    };

    let total = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .count_documents(filter.clone())
        .await?;

    let accounts: Vec<ServiceAccount> = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    Ok((accounts, total))
}

/// Get a service account by ID.
pub async fn get_service_account(db: &Database, sa_id: &str) -> AppResult<ServiceAccount> {
    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "_id": sa_id })
        .await?
        .ok_or_else(|| AppError::ServiceAccountNotFound(sa_id.to_string()))
}

/// Update a service account's mutable fields.
#[allow(clippy::too_many_arguments)]
pub async fn update_service_account(
    db: &Database,
    sa_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    allowed_scopes: Option<&str>,
    role_ids: Option<&[String]>,
    rate_limit_override: Option<Option<u64>>,
    is_active: Option<bool>,
) -> AppResult<ServiceAccount> {
    // Verify it exists first
    let _existing = get_service_account(db, sa_id).await?;

    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(n) = name {
        if n.is_empty() || n.len() > 100 {
            return Err(AppError::ValidationError(
                "Service account name must be between 1 and 100 characters".to_string(),
            ));
        }
        set_doc.insert("name", n);
    }

    if let Some(d) = description {
        if d.len() > 500 {
            return Err(AppError::ValidationError(
                "Description must be 500 characters or less".to_string(),
            ));
        }
        if d.is_empty() {
            set_doc.insert("description", bson::Bson::Null);
        } else {
            set_doc.insert("description", d);
        }
    }

    if let Some(s) = allowed_scopes {
        if s.is_empty() {
            return Err(AppError::ValidationError(
                "At least one scope is required".to_string(),
            ));
        }
        set_doc.insert("allowed_scopes", s);
    }

    if let Some(roles) = role_ids {
        if !roles.is_empty() {
            let existing_count = db
                .collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME)
                .count_documents(doc! { "_id": { "$in": roles.iter().map(|r| r.as_str()).collect::<Vec<&str>>() } })
                .await?;
            if existing_count != roles.len() as u64 {
                return Err(AppError::ValidationError(
                    "One or more role IDs do not exist".to_string(),
                ));
            }
        }
        set_doc.insert(
            "role_ids",
            roles.iter().map(|r| r.as_str()).collect::<Vec<&str>>(),
        );
    }

    if let Some(rl) = rate_limit_override {
        match rl {
            Some(val) => {
                if val == 0 {
                    return Err(AppError::ValidationError(
                        "Rate limit override must be greater than 0".to_string(),
                    ));
                }
                set_doc.insert("rate_limit_override", val as i64);
            }
            None => {
                set_doc.insert("rate_limit_override", bson::Bson::Null);
            }
        }
    }

    if let Some(active) = is_active {
        set_doc.insert("is_active", active);
    }

    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .update_one(doc! { "_id": sa_id }, doc! { "$set": set_doc })
        .await?;

    get_service_account(db, sa_id).await
}

/// Rotate the client secret. Revokes all outstanding tokens.
/// Returns (updated ServiceAccount, new raw_client_secret).
pub async fn rotate_secret(db: &Database, sa_id: &str) -> AppResult<(ServiceAccount, String)> {
    let _existing = get_service_account(db, sa_id).await?;

    let raw_secret = generate_client_secret();
    let secret_hash = hash_token(&raw_secret);
    let secret_prefix = raw_secret[..8].to_string();

    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .update_one(
            doc! { "_id": sa_id },
            doc! {
                "$set": {
                    "client_secret_hash": &secret_hash,
                    "secret_prefix": &secret_prefix,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    // Revoke all outstanding tokens
    revoke_all_tokens(db, sa_id).await?;

    let updated = get_service_account(db, sa_id).await?;
    Ok((updated, raw_secret))
}

/// Soft-delete (deactivate) a service account and revoke all tokens.
pub async fn delete_service_account(db: &Database, sa_id: &str) -> AppResult<()> {
    let _existing = get_service_account(db, sa_id).await?;

    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .update_one(
            doc! { "_id": sa_id },
            doc! {
                "$set": {
                    "is_active": false,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    revoke_all_tokens(db, sa_id).await?;

    Ok(())
}

/// Revoke all active tokens for a service account.
pub async fn revoke_all_tokens(db: &Database, sa_id: &str) -> AppResult<u64> {
    let result = db
        .collection::<ServiceAccountToken>(SA_TOKENS)
        .update_many(
            doc! { "service_account_id": sa_id, "revoked": false },
            doc! { "$set": { "revoked": true } },
        )
        .await?;

    Ok(result.modified_count)
}

/// Authenticate via client credentials: validate client_id + client_secret,
/// issue a JWT, and persist a token record.
pub async fn authenticate_client_credentials(
    db: &Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    client_id: &str,
    client_secret: &str,
    requested_scope: Option<&str>,
) -> AppResult<ClientCredentialsResponse> {
    let secret_hash = hash_token(client_secret);

    let sa = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "client_id": client_id })
        .await?
        .ok_or_else(|| AppError::AuthenticationFailed("Invalid client credentials".to_string()))?;

    if !sa.is_active {
        return Err(AppError::AuthenticationFailed(
            "Invalid client credentials".to_string(),
        ));
    }

    if !constant_time_eq(sa.client_secret_hash.as_bytes(), secret_hash.as_bytes()) {
        return Err(AppError::AuthenticationFailed(
            "Invalid client credentials".to_string(),
        ));
    }

    // Validate requested scopes are a subset of allowed_scopes
    let granted_scope = match requested_scope {
        Some(req) if !req.is_empty() => {
            let allowed: std::collections::HashSet<&str> =
                sa.allowed_scopes.split_whitespace().collect();
            for s in req.split_whitespace() {
                if !allowed.contains(s) {
                    return Err(AppError::InvalidScope(format!(
                        "Scope '{}' is not allowed for this service account",
                        s
                    )));
                }
            }
            req.to_string()
        }
        _ => sa.allowed_scopes.clone(),
    };

    let ttl = config.sa_token_ttl_secs;

    let (token, jti) =
        jwt::generate_service_account_token(jwt_keys, config, &sa.id, &granted_scope, ttl)?;

    // Persist token record for revocation support
    let token_record = ServiceAccountToken {
        id: Uuid::new_v4().to_string(),
        jti,
        service_account_id: sa.id.clone(),
        scope: granted_scope.clone(),
        expires_at: Utc::now() + Duration::seconds(ttl),
        revoked: false,
        created_at: Utc::now(),
    };

    db.collection::<ServiceAccountToken>(SA_TOKENS)
        .insert_one(&token_record)
        .await?;

    // Update last_authenticated_at
    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .update_one(
            doc! { "_id": &sa.id },
            doc! { "$set": { "last_authenticated_at": bson::DateTime::from_chrono(Utc::now()) } },
        )
        .await?;

    Ok(ClientCredentialsResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: ttl,
        scope: granted_scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_format() {
        let id = generate_client_id();
        assert!(id.starts_with("sa_"));
        assert_eq!(id.len(), 3 + 24); // "sa_" + 24 hex chars
    }

    #[test]
    fn client_secret_format() {
        let secret = generate_client_secret();
        assert!(secret.starts_with("sas_"));
        assert_eq!(secret.len(), 4 + 64); // "sas_" + 64 hex chars
    }

    #[test]
    fn client_ids_are_unique() {
        let id1 = generate_client_id();
        let id2 = generate_client_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn client_secrets_are_unique() {
        let s1 = generate_client_secret();
        let s2 = generate_client_secret();
        assert_ne!(s1, s2);
    }

    #[test]
    fn secret_hash_matches() {
        let secret = generate_client_secret();
        let hash1 = hash_token(&secret);
        let hash2 = hash_token(&secret);
        assert_eq!(hash1, hash2);
    }
}
