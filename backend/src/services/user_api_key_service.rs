use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::oauth_state::{COLLECTION_NAME as OAUTH_STATES, OAuthState};
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
    "failed",
    "refresh_failed",
    "pending_auth",
];
const MAX_ERROR_MESSAGE_LENGTH: usize = 512;

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
    // Exclude terminal failure keys from provider-token sync. Without
    // this filter, a placeholder that was revoked via the
    // `only_if_pending` cleanup path (e.g. the cli-pair flow's
    // unload/cancel race against the OAuth callback) would be
    // resurrected as the callback's `sync_provider_token_to_api_keys`
    // blindly rewrites every key for the provider back to the
    // token's status. `revoked` and `failed` are terminal by design:
    // once explicit cleanup or provider denial took effect, a later
    // provider callback must not flip that row back to `active`.
    let keys: Vec<UserApiKey> = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .find(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
            "status": { "$nin": ["revoked", "failed"] },
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

/// Mark placeholder UserApiKey rows tied to a denied or failed OAuth
/// flow so the wizard's polling can exit immediately instead of waiting for
/// the 5-minute deadline.
///
/// The status filter keeps this race-safe: an OAuth callback that already
/// activated a credential is no longer `pending_auth`, so it will not be
/// overwritten by a late provider error callback.
pub async fn fail_pending_placeholders_for_provider(
    db: &mongodb::Database,
    user_id: &str,
    provider_config_id: &str,
    error_message: &str,
) -> AppResult<u64> {
    let message = normalize_error_message(error_message);
    let result = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_many(
            doc! {
                "user_id": user_id,
                "provider_config_id": provider_config_id,
                "status": { "$in": ["pending_auth"] },
                "credential_type": { "$ne": "node_managed" },
            },
            doc! {
                "$set": {
                    "status": "failed",
                    "error_message": message,
                    "credential_encrypted": bson::Bson::Null,
                    "access_token_encrypted": bson::Bson::Null,
                    "refresh_token_encrypted": bson::Bson::Null,
                    "token_scopes": bson::Bson::Null,
                    "expires_at": bson::Bson::Null,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    Ok(result.modified_count)
}

/// Lazy reconciliation of a single `pending_auth` OAuth placeholder. Called
/// from the wizard's polling endpoint (`GET /api/v1/keys/{id}`) so each poll
/// is a chance to converge the placeholder to a terminal status without
/// waiting on the OAuth callback to push the update.
///
/// Two passes:
///
/// 1. **Sync from a fresh token** (issue #653 success path). If a
///    `UserProviderToken` exists for this `(user_id, provider_config_id)`
///    AND it is fresher than this placeholder's `updated_at` (the OAuth
///    callback inserted/updated a token after the placeholder was created),
///    re-run `sync_provider_token_to_api_keys` so the placeholder is
///    promoted to `active`. This catches the race / silent-sync-failure
///    case while leaving previously-stored tokens alone.
///
///    The freshness check is critical: a `UserProviderToken` from a *prior*
///    successful OAuth (still in the DB because the user previously
///    connected this provider) must NOT be applied to a fresh `pending_
///    auth` placeholder created for a *new* OAuth attempt. Doing so would
///    flip the wizard to "Done" the moment it starts polling, before the
///    user has even completed the in-flight consent on the provider page.
///
/// 2. **Fail abandoned flows** (issue #653 cancel/deny path). If the row
///    is still `pending_auth` AND no live (non-expired) `OAuthState` row
///    remains for this `(user_id, provider_config_id)`, the OAuth flow has
///    been abandoned: the user cancelled on the provider page, the network
///    dropped before the callback landed, or the state TTL expired. Mark
///    the placeholder `failed` so the wizard's next poll exits with a
///    clear message instead of hanging until its 5-minute deadline.
///
/// Best-effort: any error here is bubbled but the caller (the read
/// handler) should log + ignore so the read still proceeds with whatever
/// state currently exists. No-op for non-pending rows, non-OAuth keys, or
/// keys without a `provider_config_id`.
pub async fn reconcile_pending_oauth_placeholder(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
) -> AppResult<()> {
    let api_key = match get_api_key(db, user_id, api_key_id).await {
        Ok(k) => k,
        // Caller (the read handler) will surface 404 next; nothing to
        // reconcile.
        Err(_) => return Ok(()),
    };
    if api_key.status != "pending_auth" {
        return Ok(());
    }
    let Some(provider_config_id) = api_key.provider_config_id.as_deref() else {
        return Ok(());
    };

    // Pass 1: pull forward only if a token landed AFTER this placeholder
    // was last touched. A token from a previous OAuth must not retroactively
    // mark a fresh placeholder active.
    let candidate_token = db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find_one(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
            "status": { "$in": ["active", "expired", "refresh_failed"] },
        })
        .await?;
    if let Some(token) = candidate_token
        && token.updated_at > api_key.updated_at
    {
        sync_provider_token_to_api_keys(db, user_id, provider_config_id).await?;
        // Re-read; if pass 1 promoted us to `active`, we're done.
        let api_key = match get_api_key(db, user_id, api_key_id).await {
            Ok(k) => k,
            Err(_) => return Ok(()),
        };
        if api_key.status != "pending_auth" {
            return Ok(());
        }
    }

    // Pass 2: mark failed if the OAuth state is gone (abandoned flow).
    //
    // The `$or` on `user_id` / `target_user_id` is critical for org-scoped
    // wizard flows. When an admin runs `nyxid service add --org X`, the
    // placeholder lives under the org user_id, but `OAuthState.user_id` is
    // the *actor* (admin) and the org user_id lives in
    // `OAuthState.target_user_id`. Querying only by `user_id` would never
    // find the live state for org flows, so Pass 2 would fire on the very
    // first poll and fail every legitimate org-scoped placeholder.
    let now = bson::DateTime::from_chrono(Utc::now());
    let live_state_count = db
        .collection::<OAuthState>(OAUTH_STATES)
        .count_documents(doc! {
            "$or": [
                { "user_id": user_id },
                { "target_user_id": user_id },
            ],
            "provider_config_id": provider_config_id,
            "expires_at": { "$gt": &now },
        })
        .await?;
    if live_state_count > 0 {
        return Ok(());
    }

    let message = normalize_error_message(
        "Authorization timed out or was cancelled. Cancel and re-run the wizard to try again.",
    );
    db.collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": api_key_id,
                "user_id": user_id,
                "status": "pending_auth",
            },
            doc! {
                "$set": {
                    "status": "failed",
                    "error_message": message,
                    "credential_encrypted": bson::Bson::Null,
                    "access_token_encrypted": bson::Bson::Null,
                    "refresh_token_encrypted": bson::Bson::Null,
                    "token_scopes": bson::Bson::Null,
                    "expires_at": bson::Bson::Null,
                    "updated_at": &now,
                },
            },
        )
        .await?;

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

/// Promote a `node_managed` `UserApiKey` to a direct credential type and
/// store a fresh encrypted credential. Used by PUT /keys when the caller
/// supplies a `credential` on a service whose backing key was previously
/// reconciled to `node_managed` (NyxID#418). `update_api_key` refuses to
/// touch node_managed records by design — this function is the explicit
/// opt-out for the server-held-credential flow.
///
/// Validates credential length and non-emptiness identically to
/// `update_api_key` so callers get consistent error messages.
pub async fn promote_node_managed_api_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    key_id: &str,
    credential_type: &str,
    credential: &str,
) -> AppResult<()> {
    if !VALID_CREDENTIAL_TYPES.contains(&credential_type) || credential_type == "node_managed" {
        return Err(AppError::ValidationError(format!(
            "Invalid target credential_type '{credential_type}' for promotion"
        )));
    }

    if credential.is_empty() {
        return Err(AppError::ValidationError(
            "Credential must not be empty".to_string(),
        ));
    }
    if credential.len() > MAX_CREDENTIAL_LENGTH {
        return Err(AppError::ValidationError(format!(
            "Credential exceeds maximum length of {MAX_CREDENTIAL_LENGTH} bytes"
        )));
    }

    // Reject provider-backed records: `sync_provider_token_to_api_keys`
    // refreshes every non-node_managed row that shares a provider_config_id
    // with the user's active `UserProviderToken`, so flipping the type
    // here would make the freshly stored credential eligible for
    // clobbering on the next provider callback/refresh. Callers who want
    // to rotate a provider-backed credential must go through the
    // provider's own OAuth / device-code / API-key flow. (Second Codex
    // review P2 of the NyxID#419 fix.)
    let existing = get_api_key(db, user_id, key_id).await?;
    if existing.credential_type != "node_managed" {
        return Err(AppError::NotFound(
            "Node-managed API key not found for this user".to_string(),
        ));
    }
    if existing.provider_config_id.is_some() {
        return Err(AppError::BadRequest(
            "Cannot store a server-held credential on a provider-backed service. \
             Use the provider's OAuth, device-code, or API-key flow to rotate \
             the credential, or register the endpoint as a custom service."
                .to_string(),
        ));
    }

    let encrypted = encryption_keys.encrypt(credential.as_bytes()).await?;
    let encrypted_bson = bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes: encrypted,
    };

    // `UserApiKey.provider_config_id` is serialized with
    // `skip_serializing_if = Option::is_none`, so normal (non-provider)
    // rows are stored with the field *absent*, not as `null`. Match
    // both states so promotion works for the common case where the
    // node_managed key was created without any provider link
    // (thirty-third-round Codex P1).
    let result = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": key_id,
                "user_id": user_id,
                "credential_type": "node_managed",
                "$or": [
                    { "provider_config_id": bson::Bson::Null },
                    { "provider_config_id": { "$exists": false } },
                ],
            },
            doc! {
                "$set": {
                    "credential_type": credential_type,
                    "credential_encrypted": encrypted_bson,
                    "access_token_encrypted": bson::Bson::Null,
                    "refresh_token_encrypted": bson::Bson::Null,
                    "token_scopes": bson::Bson::Null,
                    "expires_at": bson::Bson::Null,
                    "status": "active",
                    "error_message": bson::Bson::Null,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "Node-managed API key not found for this user".to_string(),
        ));
    }

    Ok(())
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

fn normalize_error_message(message: &str) -> String {
    let trimmed = message.trim();
    let message = if trimmed.is_empty() {
        "OAuth authorization failed"
    } else {
        trimmed
    };
    message.chars().take(MAX_ERROR_MESSAGE_LENGTH).collect()
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

/// Atomic conditional revoke used by the `only_if_pending=true`
/// cleanup path. Flips status `pending_auth -> revoked` in a
/// single MongoDB update with the status in the filter, so the
/// provider OAuth/device-code callback cannot slip a
/// `pending_auth -> active` write in between our status read and
/// the destructive update. Returns `Ok(true)` when the revoke
/// happened, `Ok(false)` when the status had already changed
/// (callback won the race — leave the newly-authorized credential
/// alone). Returns `NotFound` if the record does not exist at all.
pub async fn revoke_api_key_if_pending(
    db: &mongodb::Database,
    user_id: &str,
    key_id: &str,
) -> AppResult<bool> {
    // Verify the key exists and belongs to this user before the
    // conditional update, so `matched_count == 0` on the main
    // update can be unambiguously interpreted as "status already
    // changed" (not "key doesn't exist").
    let existing = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .find_one(doc! { "_id": key_id, "user_id": user_id })
        .await?;
    if existing.is_none() {
        return Err(AppError::NotFound("API key not found".to_string()));
    }

    let result = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": key_id,
                "user_id": user_id,
                "status": { "$in": ["pending_auth"] },
            },
            doc! {
                "$set": {
                    "status": "revoked",
                    "credential_encrypted": bson::Bson::Null,
                    "access_token_encrypted": bson::Bson::Null,
                    "refresh_token_encrypted": bson::Bson::Null,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    // Defense-in-depth against a late OAuth /
                    // device-code callback racing this revoke.
                    // `sync_provider_token_to_api_keys` already
                    // filters out `status: "revoked"`, but
                    // clearing the provider link here means the
                    // callback's find() can't locate this row
                    // at all — so even if the status filter
                    // were ever removed by mistake, the
                    // revoked key stays revoked.
                    "provider_config_id": bson::Bson::Null,
                }
            },
        )
        .await?;

    Ok(result.matched_count > 0)
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
    use mongodb::bson::doc;

    use super::{
        OAUTH_STATES, USER_PROVIDER_TOKENS, fail_pending_placeholders_for_provider,
        has_server_credential, reconcile_pending_oauth_placeholder,
        sync_provider_token_to_api_keys,
    };
    use crate::models::oauth_state::OAuthState;
    use crate::models::user_api_key::UserApiKey;
    use crate::models::user_provider_token::UserProviderToken;
    use crate::test_utils::connect_test_database;
    use chrono::Duration;

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

    fn provider_key(
        key_id: &str,
        user_id: &str,
        provider_id: &str,
        status: &str,
        credential_type: &str,
    ) -> UserApiKey {
        let mut key = sample_key(credential_type);
        key.id = key_id.to_string();
        key.user_id = user_id.to_string();
        key.provider_config_id = Some(provider_id.to_string());
        key.status = status.to_string();
        key
    }

    async fn get_key(db: &mongodb::Database, key_id: &str) -> UserApiKey {
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .find_one(doc! { "_id": key_id })
            .await
            .unwrap()
            .unwrap()
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

    #[tokio::test]
    async fn sync_provider_token_uses_effective_org_owner() {
        let Some(db) = connect_test_database("user_api_key_sync_org").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let now = Utc::now();
        let admin_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(UserApiKey {
                id: api_key_id.clone(),
                user_id: org_id.clone(),
                label: "Org Codex".to_string(),
                credential_type: "oauth2".to_string(),
                credential_encrypted: None,
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: Some(provider_id.clone()),
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                status: "pending_auth".to_string(),
                last_used_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: org_id.clone(),
                provider_config_id: provider_id.clone(),
                credential_user_id: None,
                token_type: "oauth2".to_string(),
                access_token_encrypted: Some(vec![1, 2, 3]),
                refresh_token_encrypted: Some(vec![4, 5, 6]),
                token_scopes: Some("openid profile".to_string()),
                expires_at: None,
                api_key_encrypted: None,
                status: "active".to_string(),
                last_refreshed_at: None,
                last_used_at: None,
                error_message: None,
                label: None,
                metadata: None,
                gateway_url: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        sync_provider_token_to_api_keys(&db, &admin_id, &provider_id)
            .await
            .unwrap();
        let key_after_admin_sync = db
            .collection::<UserApiKey>(super::COLLECTION_NAME)
            .find_one(doc! { "_id": &api_key_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key_after_admin_sync.status, "pending_auth");
        assert!(key_after_admin_sync.access_token_encrypted.is_none());

        sync_provider_token_to_api_keys(&db, &org_id, &provider_id)
            .await
            .unwrap();
        let key_after_org_sync = db
            .collection::<UserApiKey>(super::COLLECTION_NAME)
            .find_one(doc! { "_id": &api_key_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key_after_org_sync.status, "active");
        assert_eq!(
            key_after_org_sync.access_token_encrypted,
            Some(vec![1, 2, 3])
        );
        assert_eq!(
            key_after_org_sync.refresh_token_encrypted,
            Some(vec![4, 5, 6])
        );
    }

    #[tokio::test]
    async fn sync_provider_token_does_not_reactivate_failed_key() {
        let Some(db) = connect_test_database("user_api_key_sync_skips_failed").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let now = Utc::now();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        let mut key = provider_key(&key_id, &user_id, &provider_id, "failed", "oauth2");
        key.error_message = Some("access_denied".to_string());
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(key)
            .await
            .unwrap();

        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: user_id.clone(),
                provider_config_id: provider_id.clone(),
                credential_user_id: None,
                token_type: "oauth2".to_string(),
                access_token_encrypted: Some(vec![1, 2, 3]),
                refresh_token_encrypted: Some(vec![4, 5, 6]),
                token_scopes: Some("openid profile".to_string()),
                expires_at: None,
                api_key_encrypted: None,
                status: "active".to_string(),
                last_refreshed_at: None,
                last_used_at: None,
                error_message: None,
                label: None,
                metadata: None,
                gateway_url: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        sync_provider_token_to_api_keys(&db, &user_id, &provider_id)
            .await
            .unwrap();

        let key_after_sync = get_key(&db, &key_id).await;
        assert_eq!(key_after_sync.status, "failed");
        assert_eq!(
            key_after_sync.error_message.as_deref(),
            Some("access_denied")
        );
        assert!(key_after_sync.access_token_encrypted.is_none());
    }

    #[tokio::test]
    async fn fail_pending_placeholders_for_provider_marks_pending_match_failed() {
        let Some(db) = connect_test_database("user_api_key_fail_provider_pending_matches").await
        else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_1 = uuid::Uuid::new_v4().to_string();
        let key_2 = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_many(vec![
                provider_key(&key_1, &user_id, &provider_id, "pending_auth", "oauth2"),
                provider_key(&key_2, &user_id, &provider_id, "pending_auth", "oauth2"),
            ])
            .await
            .unwrap();

        let failed =
            fail_pending_placeholders_for_provider(&db, &user_id, &provider_id, "access_denied")
                .await
                .unwrap();

        assert_eq!(failed, 2);
        let key_1 = get_key(&db, &key_1).await;
        let key_2 = get_key(&db, &key_2).await;
        assert_eq!(key_1.status, "failed");
        assert_eq!(key_1.error_message.as_deref(), Some("access_denied"));
        assert_eq!(key_2.status, "failed");
        assert_eq!(key_2.error_message.as_deref(), Some("access_denied"));
    }

    #[tokio::test]
    async fn fail_pending_placeholders_for_provider_skips_active() {
        let Some(db) = connect_test_database("user_api_key_fail_provider_skips_active").await
        else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let active_key = uuid::Uuid::new_v4().to_string();
        let pending_key = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_many(vec![
                provider_key(&active_key, &user_id, &provider_id, "active", "oauth2"),
                provider_key(
                    &pending_key,
                    &user_id,
                    &provider_id,
                    "pending_auth",
                    "oauth2",
                ),
            ])
            .await
            .unwrap();

        let failed =
            fail_pending_placeholders_for_provider(&db, &user_id, &provider_id, "access_denied")
                .await
                .unwrap();

        assert_eq!(failed, 1);
        assert_eq!(get_key(&db, &active_key).await.status, "active");
        assert_eq!(get_key(&db, &pending_key).await.status, "failed");
    }

    #[tokio::test]
    async fn fail_pending_placeholders_for_provider_skips_node_managed() {
        let Some(db) = connect_test_database("user_api_key_fail_provider_skips_node_managed").await
        else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let node_key = uuid::Uuid::new_v4().to_string();
        let oauth_key = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_many(vec![
                provider_key(
                    &node_key,
                    &user_id,
                    &provider_id,
                    "pending_auth",
                    "node_managed",
                ),
                provider_key(&oauth_key, &user_id, &provider_id, "pending_auth", "oauth2"),
            ])
            .await
            .unwrap();

        let failed =
            fail_pending_placeholders_for_provider(&db, &user_id, &provider_id, "access_denied")
                .await
                .unwrap();

        assert_eq!(failed, 1);
        assert_eq!(get_key(&db, &node_key).await.status, "pending_auth");
        assert_eq!(get_key(&db, &oauth_key).await.status, "failed");
    }

    #[tokio::test]
    async fn fail_pending_placeholders_for_provider_no_matches_returns_zero() {
        let Some(db) = connect_test_database("user_api_key_fail_provider_no_matches").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();

        let failed =
            fail_pending_placeholders_for_provider(&db, &user_id, &provider_id, "access_denied")
                .await
                .unwrap();

        assert_eq!(failed, 0);
    }

    fn live_oauth_state(user_id: &str, provider_id: &str) -> OAuthState {
        live_oauth_state_full(user_id, None, provider_id)
    }

    fn live_oauth_state_for_org(actor_id: &str, org_id: &str, provider_id: &str) -> OAuthState {
        live_oauth_state_full(actor_id, Some(org_id), provider_id)
    }

    fn live_oauth_state_full(
        actor_id: &str,
        target_user_id: Option<&str>,
        provider_id: &str,
    ) -> OAuthState {
        let now = Utc::now();
        OAuthState {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: actor_id.to_string(),
            provider_config_id: provider_id.to_string(),
            code_verifier: None,
            device_code_encrypted: None,
            user_code_encrypted: None,
            poll_interval: None,
            target_user_id: target_user_id.map(str::to_string),
            credential_user_id: None,
            redirect_path: None,
            consumed: false,
            expires_at: now + Duration::minutes(10),
            created_at: now,
        }
    }

    fn provider_token(user_id: &str, provider_id: &str) -> UserProviderToken {
        let now = Utc::now();
        UserProviderToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            provider_config_id: provider_id.to_string(),
            credential_user_id: None,
            token_type: "oauth2".to_string(),
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: Some(vec![4, 5, 6]),
            token_scopes: Some("openid profile".to_string()),
            expires_at: None,
            api_key_encrypted: None,
            status: "active".to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Issue #653 success path: a `UserProviderToken` lands but the original
    /// callback's sync didn't update this placeholder. The next wizard poll
    /// triggers reconcile, which re-runs sync and brings the row to `active`.
    /// The token must be FRESHER than the placeholder for promotion (see the
    /// regression test `reconcile_does_not_resurrect_placeholder_with_stale_
    /// token` for why).
    #[tokio::test]
    async fn reconcile_promotes_pending_to_active_when_fresh_token_lands() {
        let Some(db) = connect_test_database("user_api_key_reconcile_promotes").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        let placeholder_at = Utc::now() - Duration::seconds(30);

        let mut placeholder =
            provider_key(&key_id, &user_id, &provider_id, "pending_auth", "oauth2");
        placeholder.created_at = placeholder_at;
        placeholder.updated_at = placeholder_at;
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(placeholder)
            .await
            .unwrap();
        // Token created AFTER the placeholder — simulates the OAuth callback
        // landing a fresh token that the original sync missed (race / silent
        // sync error).
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(provider_token(&user_id, &provider_id))
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &key_id)
            .await
            .unwrap();

        let key = get_key(&db, &key_id).await;
        assert_eq!(key.status, "active");
        assert_eq!(key.access_token_encrypted, Some(vec![1, 2, 3]));
        assert_eq!(key.refresh_token_encrypted, Some(vec![4, 5, 6]));
    }

    /// Regression test for the local-test bug we hit on first ship: a user
    /// who previously connected the provider has a stale `UserProviderToken`
    /// row from that earlier OAuth. When they start a NEW OAuth attempt, a
    /// fresh `pending_auth` placeholder is created; the wizard polls; the
    /// reconcile must NOT use the stale token to flip the placeholder to
    /// active. Otherwise the wizard reports "Done" before the user has even
    /// completed the in-flight consent on the provider page.
    #[tokio::test]
    async fn reconcile_does_not_resurrect_placeholder_with_stale_token() {
        let Some(db) = connect_test_database("user_api_key_reconcile_skips_stale_token").await
        else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        // Stale token from a previous OAuth (10 minutes ago).
        let mut stale_token = provider_token(&user_id, &provider_id);
        let stale_at = Utc::now() - Duration::minutes(10);
        stale_token.created_at = stale_at;
        stale_token.updated_at = stale_at;
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(stale_token)
            .await
            .unwrap();

        // Fresh placeholder for a new OAuth attempt (just now). User is
        // currently on the provider's consent page; OAuth has NOT completed.
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(provider_key(
                &key_id,
                &user_id,
                &provider_id,
                "pending_auth",
                "oauth2",
            ))
            .await
            .unwrap();
        // A live OAuth state (the in-flight attempt) keeps the abandon-fail
        // pass from firing, so this test is purely about Pass 1 behaviour.
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(live_oauth_state(&user_id, &provider_id))
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &key_id)
            .await
            .unwrap();

        let key = get_key(&db, &key_id).await;
        assert_eq!(
            key.status, "pending_auth",
            "stale token must not retroactively promote a fresh placeholder"
        );
        assert!(key.access_token_encrypted.is_none());
    }

    /// Issue #653 cancel/deny path: user closed the Lark page or denied
    /// without redirecting back with `?error=`. No callback ever arrives, no
    /// OAuth state remains. Reconcile flips the placeholder to `failed` so
    /// the wizard's next poll exits with an actionable message instead of
    /// hanging until the 5-minute deadline.
    #[tokio::test]
    async fn reconcile_marks_failed_when_no_live_oauth_state() {
        let Some(db) = connect_test_database("user_api_key_reconcile_fails_abandoned").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(provider_key(
                &key_id,
                &user_id,
                &provider_id,
                "pending_auth",
                "oauth2",
            ))
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &key_id)
            .await
            .unwrap();

        let key = get_key(&db, &key_id).await;
        assert_eq!(key.status, "failed");
        assert!(
            key.error_message
                .as_deref()
                .is_some_and(|m| m.contains("timed out") || m.contains("cancelled"))
        );
    }

    /// Don't fail prematurely while the OAuth flow is still in flight.
    /// A live OAuth state means the user is mid-consent on the provider page.
    #[tokio::test]
    async fn reconcile_keeps_pending_when_oauth_state_still_alive() {
        let Some(db) = connect_test_database("user_api_key_reconcile_keeps_pending").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(provider_key(
                &key_id,
                &user_id,
                &provider_id,
                "pending_auth",
                "oauth2",
            ))
            .await
            .unwrap();
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(live_oauth_state(&user_id, &provider_id))
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &key_id)
            .await
            .unwrap();

        let key = get_key(&db, &key_id).await;
        assert_eq!(key.status, "pending_auth");
    }

    /// Active rows are terminal-success and must be left alone.
    #[tokio::test]
    async fn reconcile_noop_for_already_active_key() {
        let Some(db) = connect_test_database("user_api_key_reconcile_noop_active").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(provider_key(
                &key_id,
                &user_id,
                &provider_id,
                "active",
                "oauth2",
            ))
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &key_id)
            .await
            .unwrap();

        let key = get_key(&db, &key_id).await;
        assert_eq!(key.status, "active");
    }

    /// Issue #653 PR #723 review — Critical #1: in-flight OAuth callback
    /// race. After `handle_oauth_callback` atomically claims the OAuth
    /// state (`consumed: true`) but before the token-exchange roundtrip
    /// to the provider completes (~1+ s for Lark), reconcile must NOT
    /// see the in-progress flow as "abandoned". The state row is still
    /// present (with consumed=true and a live expires_at), so Pass 2's
    /// "no live state" check should treat it as live.
    #[tokio::test]
    async fn reconcile_keeps_pending_when_oauth_state_is_consumed_but_not_yet_deleted() {
        let Some(db) = connect_test_database("user_api_key_reconcile_in_flight_callback").await
        else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(provider_key(
                &key_id,
                &user_id,
                &provider_id,
                "pending_auth",
                "oauth2",
            ))
            .await
            .unwrap();
        // Simulate a callback that has atomically claimed the state but
        // hasn't yet finished the token exchange + insert. The row is
        // alive (not yet deleted by the cleanup at the end of
        // `handle_oauth_callback`) and within `expires_at`.
        let mut in_flight = live_oauth_state(&user_id, &provider_id);
        in_flight.consumed = true;
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(in_flight)
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &key_id)
            .await
            .unwrap();

        let key = get_key(&db, &key_id).await;
        assert_eq!(
            key.status, "pending_auth",
            "in-flight callback (consumed state, no token yet) must not be reported as abandoned"
        );
    }

    /// Issue #653 PR #723 review — Critical #2: org-scoped wizard flows.
    /// `OAuthState.user_id` is the actor (admin); the org user_id lives
    /// in `target_user_id`. The placeholder is owned by the org user.
    /// Reconcile must match the live state via `target_user_id` so that
    /// org-scoped placeholders aren't immediately failed on the first
    /// poll.
    #[tokio::test]
    async fn reconcile_finds_live_state_for_org_scoped_placeholder() {
        let Some(db) = connect_test_database("user_api_key_reconcile_org_scope_state_lookup").await
        else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let admin_id = uuid::Uuid::new_v4().to_string();
        let org_user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        // Placeholder lives under the org user id (the row owner).
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(provider_key(
                &key_id,
                &org_user_id,
                &provider_id,
                "pending_auth",
                "oauth2",
            ))
            .await
            .unwrap();
        // OAuth state is initiated by the admin actor on behalf of the
        // org — admin id in `user_id`, org id in `target_user_id`.
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(live_oauth_state_for_org(
                &admin_id,
                &org_user_id,
                &provider_id,
            ))
            .await
            .unwrap();

        // Reconcile is invoked with the row owner (the org user id).
        reconcile_pending_oauth_placeholder(&db, &org_user_id, &key_id)
            .await
            .unwrap();

        let key = get_key(&db, &key_id).await;
        assert_eq!(
            key.status, "pending_auth",
            "org-scoped placeholder must not be failed when an OAuth state exists under an admin actor with target_user_id matching the row owner"
        );
    }

    /// Direct-credential keys (no provider_config_id) aren't OAuth-flow rows;
    /// they shouldn't be touched even when status happens to be pending_auth.
    #[tokio::test]
    async fn reconcile_noop_for_key_without_provider_config_id() {
        let Some(db) = connect_test_database("user_api_key_reconcile_noop_non_oauth").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        let mut key = sample_key("api_key");
        key.id = key_id.clone();
        key.user_id = user_id.clone();
        key.provider_config_id = None;
        key.status = "pending_auth".to_string();
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(key)
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &key_id)
            .await
            .unwrap();

        let key = get_key(&db, &key_id).await;
        assert_eq!(key.status, "pending_auth");
    }

    #[tokio::test]
    async fn fail_pending_placeholders_for_provider_scopes_by_provider() {
        let Some(db) = connect_test_database("user_api_key_fail_provider_scopes").await else {
            eprintln!("skipping user_api_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let other_provider_id = uuid::Uuid::new_v4().to_string();
        let matching_key = uuid::Uuid::new_v4().to_string();
        let other_key = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_many(vec![
                provider_key(
                    &matching_key,
                    &user_id,
                    &provider_id,
                    "pending_auth",
                    "oauth2",
                ),
                provider_key(
                    &other_key,
                    &user_id,
                    &other_provider_id,
                    "pending_auth",
                    "oauth2",
                ),
            ])
            .await
            .unwrap();

        let failed =
            fail_pending_placeholders_for_provider(&db, &user_id, &provider_id, "access_denied")
                .await
                .unwrap();

        assert_eq!(failed, 1);
        assert_eq!(get_key(&db, &matching_key).await.status, "failed");
        assert_eq!(get_key(&db, &other_key).await.status, "pending_auth");
    }
}
