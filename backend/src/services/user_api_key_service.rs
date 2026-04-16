use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::user_api_key::{COLLECTION_NAME, UserApiKey};
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
use crate::services::agent_binding_service;

/// Maximum credential length in bytes to prevent abuse.
const MAX_CREDENTIAL_LENGTH: usize = 8192;
const VALID_CREDENTIAL_TYPES: &[&str] = &[
    "api_key",
    "oauth2",
    "bearer",
    "basic",
    "ssh_certificate",
    "node_managed",
];
const VALID_STATUSES: &[&str] = &[
    "active",
    "expired",
    "revoked",
    "refresh_failed",
    "pending_auth",
];

pub struct CreateApiKeyParams<'a> {
    pub label: &'a str,
    pub credential_type: &'a str,
    pub credential: &'a str,
    pub access_token: Option<&'a str>,
    pub refresh_token: Option<&'a str>,
    pub token_scopes: Option<&'a str>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub provider_config_id: Option<&'a str>,
    pub status: &'a str,
    pub source: Option<&'a str>,
    pub source_id: Option<&'a str>,
}

pub fn has_server_credential(api_key: &UserApiKey) -> bool {
    match api_key.credential_type.as_str() {
        "oauth2" => api_key
            .access_token_encrypted
            .as_ref()
            .is_some_and(|value| !value.is_empty()),
        "node_managed" | "ssh_certificate" => false,
        _ => api_key
            .credential_encrypted
            .as_ref()
            .is_some_and(|value| !value.is_empty()),
    }
}

/// List all API keys for a user (summary only, no decrypted values).
pub async fn list_api_keys(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<UserApiKey>> {
    let keys: Vec<UserApiKey> = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;
    Ok(keys)
}

/// Get single API key by ID, verifying ownership.
pub async fn get_api_key(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
) -> AppResult<UserApiKey> {
    db.collection::<UserApiKey>(COLLECTION_NAME)
        .find_one(doc! { "_id": key_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))
}

/// Create a new API key with an encrypted credential.
pub async fn create_api_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    params: CreateApiKeyParams<'_>,
) -> AppResult<UserApiKey> {
    if params.label.is_empty() || params.label.len() > 200 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 200 characters".to_string(),
        ));
    }

    if !VALID_CREDENTIAL_TYPES.contains(&params.credential_type) {
        return Err(AppError::ValidationError(format!(
            "Invalid credential_type '{}'. Valid: {}",
            params.credential_type,
            VALID_CREDENTIAL_TYPES.join(", ")
        )));
    }

    if !VALID_STATUSES.contains(&params.status) {
        return Err(AppError::ValidationError(format!(
            "Invalid status '{}'. Valid: {}",
            params.status,
            VALID_STATUSES.join(", ")
        )));
    }

    let access_token = params.access_token.or_else(|| {
        (params.credential_type == "oauth2" && !params.credential.is_empty())
            .then_some(params.credential)
    });
    let credential_value = if params.credential_type == "oauth2" {
        ""
    } else {
        params.credential
    };

    let credential_encrypted = encrypt_optional_secret(encryption_keys, credential_value).await?;
    let access_token_encrypted =
        encrypt_optional_secret(encryption_keys, access_token.unwrap_or("")).await?;
    let refresh_token_encrypted =
        encrypt_optional_secret(encryption_keys, params.refresh_token.unwrap_or("")).await?;
    let now = Utc::now();

    let api_key = UserApiKey {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        label: params.label.to_string(),
        credential_type: params.credential_type.to_string(),
        credential_encrypted,
        access_token_encrypted,
        refresh_token_encrypted,
        token_scopes: params.token_scopes.map(str::to_string),
        expires_at: params.expires_at,
        provider_config_id: params.provider_config_id.map(str::to_string),
        user_oauth_client_id_encrypted: None,
        user_oauth_client_secret_encrypted: None,
        status: params.status.to_string(),
        last_used_at: None,
        error_message: None,
        source: Some(params.source.unwrap_or("user_created").to_string()),
        source_id: params.source_id.map(str::to_string),
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserApiKey>(COLLECTION_NAME)
        .insert_one(&api_key)
        .await?;

    Ok(api_key)
}

/// Create a new API key by copying encrypted fields from an existing provider token.
pub async fn create_api_key_from_provider_token(
    db: &mongodb::Database,
    user_id: &str,
    label: &str,
    provider_config_id: &str,
    provider_token: &UserProviderToken,
) -> AppResult<UserApiKey> {
    if label.is_empty() || label.len() > 200 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 200 characters".to_string(),
        ));
    }

    let collection = db.collection::<UserApiKey>(COLLECTION_NAME);

    // Reuse an existing UserApiKey that already points at this provider token.
    // The `{source, source_id}` unique partial index enforces the invariant
    // "one UserApiKey per UserProviderToken per user", so a second call would
    // otherwise hit a duplicate-key error and surface as HTTP 500. Multiple
    // UserService rows can share the returned api_key, which is the desired
    // behaviour when a user registers several services backed by the same
    // provider credential (e.g. two `llm-openai` services with different
    // endpoint URLs).
    if let Some(existing) = collection
        .find_one(doc! {
            "user_id": user_id,
            "source": "user_created",
            "source_id": &provider_token.id,
        })
        .await?
    {
        return Ok(existing);
    }

    let credential_type = provider_token_type_to_api_key_type(&provider_token.token_type)?;
    let now = Utc::now();

    let api_key = UserApiKey {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        label: label.to_string(),
        credential_type: credential_type.to_string(),
        credential_encrypted: provider_token.api_key_encrypted.clone(),
        access_token_encrypted: provider_token.access_token_encrypted.clone(),
        refresh_token_encrypted: provider_token.refresh_token_encrypted.clone(),
        token_scopes: provider_token.token_scopes.clone(),
        expires_at: provider_token.expires_at,
        provider_config_id: Some(provider_config_id.to_string()),
        user_oauth_client_id_encrypted: None,
        user_oauth_client_secret_encrypted: None,
        status: provider_token.status.clone(),
        last_used_at: provider_token.last_used_at,
        error_message: provider_token.error_message.clone(),
        source: Some("user_created".to_string()),
        source_id: Some(provider_token.id.clone()),
        created_at: now,
        updated_at: now,
    };

    collection.insert_one(&api_key).await?;

    Ok(api_key)
}

/// Synchronize any provider-linked unified keys with the latest provider token state.
pub async fn sync_provider_token_to_api_keys(
    db: &mongodb::Database,
    user_id: &str,
    provider_config_id: &str,
) -> AppResult<()> {
    let keys: Vec<UserApiKey> = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .find(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
        })
        .await?
        .try_collect()
        .await?;

    if keys.is_empty() {
        return Ok(());
    }

    let provider_token = db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
            "status": { "$in": ["active", "expired", "refresh_failed"] },
        })
        .await?;

    let now = bson::DateTime::from_chrono(Utc::now());

    for key in keys {
        if key.credential_type == "node_managed" {
            continue;
        }

        let set_doc = if let Some(ref token) = provider_token {
            doc! {
                "credential_type": provider_token_type_to_api_key_type(&token.token_type)?,
                "credential_encrypted": optional_binary_bson(token.api_key_encrypted.as_ref()),
                "access_token_encrypted": optional_binary_bson(token.access_token_encrypted.as_ref()),
                "refresh_token_encrypted": optional_binary_bson(token.refresh_token_encrypted.as_ref()),
                "token_scopes": optional_string_bson(token.token_scopes.as_deref()),
                "expires_at": optional_datetime_bson(token.expires_at),
                "status": token.status.as_str(),
                "last_used_at": optional_datetime_bson(token.last_used_at),
                "error_message": optional_string_bson(token.error_message.as_deref()),
                "updated_at": &now,
            }
        } else {
            doc! {
                "credential_encrypted": bson::Bson::Null,
                "access_token_encrypted": bson::Bson::Null,
                "refresh_token_encrypted": bson::Bson::Null,
                "token_scopes": bson::Bson::Null,
                "expires_at": bson::Bson::Null,
                "status": "revoked",
                "error_message": bson::Bson::Null,
                "updated_at": &now,
            }
        };

        db.collection::<UserApiKey>(COLLECTION_NAME)
            .update_one(doc! { "_id": &key.id }, doc! { "$set": set_doc })
            .await?;
    }

    Ok(())
}

pub async fn activate_node_managed_api_key(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
) -> AppResult<()> {
    reset_provider_api_key_state(db, user_id, key_id, "node_managed", "active").await
}

pub async fn mark_provider_connection_pending(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
    credential_type: &str,
) -> AppResult<()> {
    reset_provider_api_key_state(db, user_id, key_id, credential_type, "pending_auth").await
}

async fn reset_provider_api_key_state(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
    credential_type: &str,
    status: &str,
) -> AppResult<()> {
    let result = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": key_id, "user_id": user_id },
            doc! {
                "$set": {
                    "credential_type": credential_type,
                    "credential_encrypted": bson::Bson::Null,
                    "access_token_encrypted": bson::Bson::Null,
                    "refresh_token_encrypted": bson::Bson::Null,
                    "token_scopes": bson::Bson::Null,
                    "expires_at": bson::Bson::Null,
                    "status": status,
                    "error_message": bson::Bson::Null,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("API key not found".to_string()));
    }

    Ok(())
}

async fn encrypt_optional_secret(
    encryption_keys: &EncryptionKeys,
    secret: &str,
) -> AppResult<Option<Vec<u8>>> {
    if secret.is_empty() {
        Ok(None)
    } else {
        if secret.len() > MAX_CREDENTIAL_LENGTH {
            return Err(AppError::ValidationError(format!(
                "Credential exceeds maximum length of {MAX_CREDENTIAL_LENGTH} bytes"
            )));
        }
        Ok(Some(encryption_keys.encrypt(secret.as_bytes()).await?))
    }
}

fn provider_token_type_to_api_key_type(token_type: &str) -> AppResult<&'static str> {
    match token_type {
        "api_key" => Ok("api_key"),
        "oauth2" => Ok("oauth2"),
        other => Err(AppError::Internal(format!(
            "Unsupported provider token type '{other}'"
        ))),
    }
}

fn optional_binary_bson(bytes: Option<&Vec<u8>>) -> bson::Bson {
    match bytes {
        Some(value) => bson::Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: value.clone(),
        }),
        None => bson::Bson::Null,
    }
}

fn optional_string_bson(value: Option<&str>) -> bson::Bson {
    match value {
        Some(text) => bson::Bson::String(text.to_string()),
        None => bson::Bson::Null,
    }
}

fn optional_datetime_bson(value: Option<chrono::DateTime<chrono::Utc>>) -> bson::Bson {
    match value {
        Some(dt) => bson::Bson::DateTime(bson::DateTime::from_chrono(dt)),
        None => bson::Bson::Null,
    }
}

/// Update label or rotate credential.
pub async fn update_api_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    key_id: &str,
    label: Option<&str>,
    credential: Option<&str>,
) -> AppResult<()> {
    if label.is_none() && credential.is_none() {
        return Err(AppError::BadRequest(
            "At least one field must be provided".to_string(),
        ));
    }

    let existing = get_api_key(db, user_id, key_id).await?;
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(l) = label {
        if l.is_empty() || l.len() > 200 {
            return Err(AppError::ValidationError(
                "Label must be between 1 and 200 characters".to_string(),
            ));
        }
        set_doc.insert("label", l);
    }

    if let Some(cred) = credential {
        if existing.credential_type == "node_managed" {
            return Err(AppError::BadRequest(
                "Credential is managed on the node agent. Update it on the node instead."
                    .to_string(),
            ));
        }

        if cred.is_empty() {
            return Err(AppError::ValidationError(
                "Credential must not be empty".to_string(),
            ));
        }
        if cred.len() > MAX_CREDENTIAL_LENGTH {
            return Err(AppError::ValidationError(format!(
                "Credential exceeds maximum length of {MAX_CREDENTIAL_LENGTH} bytes"
            )));
        }
        let encrypted = encryption_keys.encrypt(cred.as_bytes()).await?;
        let encrypted_bson = bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: encrypted,
        };
        if existing.credential_type == "oauth2" {
            set_doc.insert("credential_encrypted", bson::Bson::Null);
            set_doc.insert("access_token_encrypted", encrypted_bson);
            set_doc.insert("refresh_token_encrypted", bson::Bson::Null);
            set_doc.insert("expires_at", bson::Bson::Null);
            set_doc.insert("token_scopes", bson::Bson::Null);
        } else {
            set_doc.insert("credential_encrypted", encrypted_bson);
        }
        set_doc.insert("status", "active");
        set_doc.insert("error_message", bson::Bson::Null);
    }

    let result = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": key_id, "user_id": user_id },
            doc! { "$set": set_doc },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("API key not found".to_string()));
    }

    Ok(())
}

/// Revoke an API key (sets status = "revoked", clears credential).
pub async fn revoke_api_key(db: &mongodb::Database, user_id: &str, key_id: &str) -> AppResult<()> {
    let result = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": key_id, "user_id": user_id },
            doc! {
                "$set": {
                    "status": "revoked",
                    "credential_encrypted": bson::Bson::Null,
                    "access_token_encrypted": bson::Bson::Null,
                    "refresh_token_encrypted": bson::Bson::Null,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("API key not found".to_string()));
    }

    Ok(())
}

/// Delete an API key. Fails if any active UserService references it.
pub async fn delete_api_key(db: &mongodb::Database, user_id: &str, key_id: &str) -> AppResult<()> {
    // Verify ownership
    let _ = get_api_key(db, user_id, key_id).await?;

    let ref_count = db
        .collection::<mongodb::bson::Document>(USER_SERVICES)
        .count_documents(doc! {
            "api_key_id": key_id,
            "is_active": true,
        })
        .await?;

    if ref_count > 0 {
        return Err(AppError::Conflict(
            "API key is in use by active services".to_string(),
        ));
    }

    db.collection::<UserApiKey>(COLLECTION_NAME)
        .delete_one(doc! { "_id": key_id, "user_id": user_id })
        .await?;

    // Cascade-clean any agent service bindings that used this credential
    // as an override. Without this, the Agent Key detail page keeps the
    // row around with the credential label degraded to a raw UUID
    // (issue #324). Safe to run after delete because bindings are keyed
    // by `user_api_key_id` and don't need the credential row to exist.
    agent_binding_service::cleanup_bindings_for_credential(db, user_id, key_id).await?;

    Ok(())
}

/// Update last_used_at and updated_at timestamps (fire-and-forget, called from proxy).
pub async fn touch_last_used(db: &mongodb::Database, key_id: &str) {
    let now = bson::DateTime::from_chrono(Utc::now());
    let _ = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": key_id },
            doc! { "$set": { "last_used_at": &now, "updated_at": &now } },
        )
        .await;
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::has_server_credential;
    use crate::models::user_api_key::UserApiKey;

    fn sample_key(credential_type: &str) -> UserApiKey {
        UserApiKey {
            id: "key-1".to_string(),
            user_id: "user-1".to_string(),
            label: "Sample".to_string(),
            credential_type: credential_type.to_string(),
            credential_encrypted: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: Some("provider-1".to_string()),
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn detects_server_credential_for_oauth_keys() {
        let mut key = sample_key("oauth2");
        assert!(!has_server_credential(&key));
        key.access_token_encrypted = Some(vec![1, 2, 3]);
        assert!(has_server_credential(&key));
    }

    #[test]
    fn node_managed_keys_never_report_server_credentials() {
        let mut key = sample_key("node_managed");
        key.credential_encrypted = Some(vec![1, 2, 3]);
        key.access_token_encrypted = Some(vec![4, 5, 6]);
        assert!(!has_server_credential(&key));
    }
}
