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
    /// Per-add OAuth connection identifier. Set when this key is being
    /// minted as part of a fresh OAuth/device-code add (so the eventual
    /// callback can scope the token write to this key). `None` for
    /// non-OAuth keys (api_key / bearer / basic / ssh / node-managed).
    pub connection_id: Option<&'a str>,
    /// User-provided OAuth Custom App client_id for BYO providers
    /// (`credential_mode: "user"` — Lark, Feishu, Twitter/X). When
    /// supplied, encrypted into `UserApiKey.user_oauth_client_id_encrypted`
    /// so this connection's authorize / token-exchange / refresh paths
    /// resolve the client credentials from the key itself instead of
    /// `user_provider_credentials` (which is single-row per
    /// `(user, provider)` and can't represent multiple Custom Apps).
    /// Must be supplied together with `oauth_client_secret` or neither.
    pub oauth_client_id: Option<&'a str>,
    pub oauth_client_secret: Option<&'a str>,
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

    // BYO Custom App credentials (Lark / Feishu / Twitter multi-connection).
    // Caller has already validated paired presence; we treat them as
    // independent optional secrets here so `None` rows simply skip the
    // field, preserving backward-compat with existing keys.
    match (params.oauth_client_id, params.oauth_client_secret) {
        (Some(_), None) | (None, Some(_)) => {
            return Err(AppError::ValidationError(
                "oauth_client_id and oauth_client_secret must be supplied together".to_string(),
            ));
        }
        _ => {}
    }
    let user_oauth_client_id_encrypted =
        encrypt_optional_secret(encryption_keys, params.oauth_client_id.unwrap_or("")).await?;
    let user_oauth_client_secret_encrypted =
        encrypt_optional_secret(encryption_keys, params.oauth_client_secret.unwrap_or("")).await?;

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
        connection_id: params.connection_id.map(str::to_string),
        user_oauth_client_id_encrypted,
        user_oauth_client_secret_encrypted,
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
        connection_id: provider_token.connection_id.clone(),
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
///
/// Legacy-only fan-out: this function operates on keys with no
/// `connection_id` (the pre-multi-connection storage model where a
/// single `(user, provider)` token is shared by every key for that
/// pair). Multi-connection keys (`connection_id: Some(uuid)`) own their
/// tokens directly on the `UserApiKey` row and are written by
/// [`write_oauth_tokens_to_key`] / refreshed by
/// [`crate::services::user_token_service::refresh_user_api_key_in_place`];
/// they must NOT be touched here, otherwise a legacy refresh on one
/// connection would clobber a sibling connection's independently-managed
/// token (the "B2" failure mode in the design doc).
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
    //
    // `connection_id: null` filter scopes this to legacy keys only
    // (B2 fix). Multi-connection keys deliberately have `connection_id:
    // Some(uuid)` and are excluded from the legacy fan-out path.
    let keys: Vec<UserApiKey> = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .find(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
            "status": { "$nin": ["revoked", "failed"] },
            "connection_id": null,
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

/// Multi-connection OAuth callback write path.
///
/// Writes a freshly-minted OAuth/device-code token directly onto the
/// `UserApiKey` row identified by `connection_id`, encrypting tokens
/// before storage and flipping the row from `pending_auth` to `active`.
/// Unlike [`sync_provider_token_to_api_keys`] (which fans out per
/// `(user, provider)`), this function is scoped to a single key — one
/// connection's authorization can never clobber a sibling connection's
/// token, even when both belong to the same `(user, provider)` pair.
///
/// Callers (`handle_oauth_callback`, `store_device_code_tokens`) reach
/// this path only when `OAuthState.connection_id` is `Some`. The
/// `connection_id` is unique across `user_api_keys` (enforced by the
/// partial unique index added in `db.rs::ensure_indexes`).
///
/// Errors:
/// - `AppError::NotFound` if no `UserApiKey` matches the connection_id
///   (e.g. the user deleted the pending placeholder mid-flow).
/// - Encryption / database errors bubble up unchanged.
pub async fn write_oauth_tokens_to_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    connection_id: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    token_scopes: Option<&str>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> AppResult<()> {
    let access_enc = encryption_keys.encrypt(access_token.as_bytes()).await?;
    let refresh_enc = match refresh_token {
        Some(rt) if !rt.is_empty() => Some(encryption_keys.encrypt(rt.as_bytes()).await?),
        _ => None,
    };

    let now = bson::DateTime::from_chrono(Utc::now());

    let mut set_doc = doc! {
        "credential_type": "oauth2",
        "access_token_encrypted": optional_binary_bson(Some(&access_enc)),
        "refresh_token_encrypted": optional_binary_bson(refresh_enc.as_ref()),
        "token_scopes": optional_string_bson(token_scopes),
        "expires_at": optional_datetime_bson(expires_at),
        "status": "active",
        "error_message": bson::Bson::Null,
        "updated_at": &now,
    };
    // Preserve the existing `credential_encrypted` (which is None for
    // OAuth keys); explicit unset so a previously-set credential cannot
    // mask the new access_token at proxy time.
    set_doc.insert("credential_encrypted", bson::Bson::Null);

    // Exclude terminal-status rows from the write. `revoked` / `failed`
    // are terminal by design (matching `sync_provider_token_to_api_keys`
    // and `fail_pending_placeholders_for_provider`): once a user has
    // revoked the key or it failed, a duplicate or late provider
    // callback must not resurrect it back to `active`. A fresh add
    // (`pending_auth`) and a re-authorization of a live connection
    // (`active` / `expired`) are both still matched.
    let result = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! {
                "connection_id": connection_id,
                "status": { "$nin": ["revoked", "failed"] },
            },
            doc! { "$set": set_doc },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(format!(
            "No writable UserApiKey found for connection_id {connection_id} \
             (it may not exist, or may have been revoked / already failed)"
        )));
    }

    tracing::info!(
        connection_id = %connection_id,
        "OAuth tokens written to UserApiKey (multi-connection path)"
    );

    Ok(())
}

/// Mark placeholder UserApiKey rows tied to a denied or failed OAuth
/// flow so the wizard's polling can exit immediately instead of waiting for
/// the 5-minute deadline.
///
/// The status filter keeps this race-safe: an OAuth callback that already
/// activated a credential is no longer `pending_auth`, so it will not be
/// overwritten by a late provider error callback.
///
/// Legacy-only fan-out: scoped to `connection_id: null` so a denial on
/// one legacy flow cannot mark a concurrent multi-connection placeholder
/// (different `connection_id`) as failed. Multi-connection failures are
/// recorded directly on their own `UserApiKey` row by
/// [`crate::services::user_token_service::refresh_user_api_key_in_place`]
/// or the OAuth callback's multi-connection branch.
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
                "connection_id": null,
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

/// Mark a single multi-connection OAuth placeholder (identified by its
/// `connection_id`) as `failed` with an actionable message. Used by the
/// OAuth callback failure arms for wizard / BYO-Custom-App flows, where the
/// placeholder carries a non-null `connection_id` and is therefore skipped by
/// the legacy `connection_id: null` fan-out in
/// `fail_pending_placeholders_for_provider`.
///
/// Race-safe: only `pending_auth` rows are touched, so a callback that
/// already activated the credential, or a row the user revoked, is never
/// overwritten by a late provider-error callback.
pub async fn fail_connection_placeholder(
    db: &mongodb::Database,
    connection_id: &str,
    error_message: &str,
) -> AppResult<u64> {
    let message = normalize_error_message(error_message);
    let result = db
        .collection::<UserApiKey>(COLLECTION_NAME)
        .update_one(
            doc! {
                "connection_id": connection_id,
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

/// Fail the OAuth placeholder(s) for a denied/failed callback, routing to the
/// right strategy: multi-connection flows (non-null `connection_id`) fail
/// their specific row; legacy flows fan out across `connection_id: null`
/// placeholders for the provider.
pub async fn fail_oauth_placeholders(
    db: &mongodb::Database,
    owner_id: &str,
    provider_config_id: &str,
    connection_id: Option<&str>,
    error_message: &str,
) -> AppResult<u64> {
    match connection_id {
        Some(conn_id) => fail_connection_placeholder(db, conn_id, error_message).await,
        None => {
            fail_pending_placeholders_for_provider(db, owner_id, provider_config_id, error_message)
                .await
        }
    }
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
    //
    // LEGACY-ONLY. Multi-connection keys are activated directly by their
    // own OAuth callback (`write_oauth_tokens_to_key`) and never read from
    // `user_provider_tokens`. Running Pass 1 for them would be the "B1"
    // silent-failure mode: a legacy *sibling* token's refresh bumps
    // `user_provider_tokens.updated_at` past this pending key's
    // `updated_at`, Pass 1 inherits that unrelated token, and the
    // multi-connection placeholder flips to `active` with the wrong
    // credentials before the user even authorizes on the provider page.
    // Pass 2 below still runs for multi-connection keys (scoped by
    // `connection_id`) so abandoned flows are still swept to `failed`.
    if api_key.connection_id.is_none() {
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
    }

    // Pass 2: mark failed if the OAuth state is gone (abandoned flow).
    // Runs for BOTH legacy and multi-connection placeholders.
    //
    // The `$or` on `user_id` / `target_user_id` is critical for org-scoped
    // wizard flows. When an admin runs `nyxid service add --org X`, the
    // placeholder lives under the org user_id, but `OAuthState.user_id` is
    // the *actor* (admin) and the org user_id lives in
    // `OAuthState.target_user_id`. Querying only by `user_id` would never
    // find the live state for org flows, so Pass 2 would fire on the very
    // first poll and fail every legitimate org-scoped placeholder.
    //
    // For multi-connection placeholders the live-state lookup is further
    // narrowed by `connection_id`: each connection's OAuth flow has its
    // own `OAuthState` row carrying that id (threaded through
    // `initiate_oauth_connect` / `request_device_code`). Without this
    // narrowing, a *sibling* connection's in-flight `OAuthState` for the
    // same `(user, provider)` would keep this placeholder pending forever
    // — abandonment would never be detected.
    //
    // ORDERING REQUIREMENT (see design doc §4.2/§4.3): the multi-connection
    // `create_key` path MUST insert the `OAuthState` (carrying the
    // connection_id) before — or in the same logical unit as — minting
    // the `pending_auth` placeholder. Otherwise a `GET /keys/:id` poll in
    // the gap between placeholder-mint and state-insert would find no
    // matching state and fail the placeholder prematurely.
    let now = bson::DateTime::from_chrono(Utc::now());
    let mut state_filter = doc! {
        "$or": [
            { "user_id": user_id },
            { "target_user_id": user_id },
        ],
        "provider_config_id": provider_config_id,
        "expires_at": { "$gt": &now },
    };
    if let Some(ref conn_id) = api_key.connection_id {
        state_filter.insert("connection_id", conn_id.as_str());
    }
    let live_state_count = db
        .collection::<OAuthState>(OAUTH_STATES)
        .count_documents(state_filter)
        .await?;
    if live_state_count > 0 {
        return Ok(());
    }

    // Grace window for fresh multi-connection placeholders.
    //
    // `create_key` mints a multi-connection placeholder (`connection_id:
    // Some(..)`), and the wizard's *subsequent, separate* OAuth-initiate
    // request creates the connection-scoped `OAuthState` a beat later.
    // Between those two requests a `GET /keys/:id` poll would find no
    // matching state and — because Pass 2 is connection-scoped — fail
    // the placeholder prematurely. Don't fail a multi-connection
    // placeholder younger than this window; its `OAuthState` is expected
    // imminently. Legacy placeholders are exempt: their Pass 2 lookup is
    // by `(user, provider)`, so a sibling's leftover state already
    // bridges the gap (and this path is unchanged from pre-multi-
    // connection behavior).
    let fresh_grace = chrono::Duration::seconds(60);
    if api_key.connection_id.is_some()
        && Utc::now().signed_duration_since(api_key.created_at) < fresh_grace
    {
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
        OAUTH_STATES, USER_PROVIDER_TOKENS, fail_connection_placeholder,
        fail_pending_placeholders_for_provider, has_server_credential,
        reconcile_pending_oauth_placeholder, sync_provider_token_to_api_keys,
        write_oauth_tokens_to_key,
    };
    use crate::models::oauth_state::OAuthState;
    use crate::models::user_api_key::UserApiKey;
    use crate::models::user_provider_token::UserProviderToken;
    use crate::test_utils::connect_test_database;
    use crate::test_utils::test_encryption_keys;
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
            connection_id: None,
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
                connection_id: None,
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
                connection_id: None,
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
                connection_id: None,
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
            connection_id: None,
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
            connection_id: None,
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

    #[tokio::test]
    async fn write_oauth_tokens_to_key_activates_pending_key() {
        let Some(db) = connect_test_database("user_api_key_write_oauth_basic").await else {
            eprintln!("skipping write_oauth_tokens_to_key test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();

        let now = Utc::now();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let connection_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(UserApiKey {
                id: key_id.clone(),
                user_id: user_id.clone(),
                label: "Multi-conn Codex".to_string(),
                credential_type: "oauth2".to_string(),
                credential_encrypted: None,
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: Some(provider_id.clone()),
                connection_id: Some(connection_id.clone()),
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

        let expires = now + Duration::seconds(3600);
        write_oauth_tokens_to_key(
            &db,
            &encryption_keys,
            &connection_id,
            "access-token-123",
            Some("refresh-token-456"),
            Some("openid profile"),
            Some(expires),
        )
        .await
        .unwrap();

        let restored = get_key(&db, &key_id).await;
        assert_eq!(restored.status, "active");
        assert_eq!(restored.credential_type, "oauth2");
        assert!(restored.access_token_encrypted.is_some());
        assert!(restored.refresh_token_encrypted.is_some());
        assert_eq!(restored.token_scopes.as_deref(), Some("openid profile"));
        assert!(restored.expires_at.is_some());
        assert!(restored.error_message.is_none());

        // Decrypt and verify the stored access token is the plaintext we wrote.
        let access_bytes = encryption_keys
            .decrypt(restored.access_token_encrypted.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(access_bytes).unwrap(), "access-token-123");
    }

    #[tokio::test]
    async fn sync_provider_token_skips_multi_connection_keys() {
        // B2 isolation test: a legacy `user_provider_tokens` refresh
        // sync must NOT touch any UserApiKey that carries its own
        // connection_id. Otherwise a legacy refresh on connection-null
        // keys would silently overwrite a multi-connection sibling's
        // independent token (the silent-alias bug reborn).
        let Some(db) = connect_test_database("user_api_key_sync_skips_multi_conn").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };

        let now = Utc::now();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let legacy_key_id = uuid::Uuid::new_v4().to_string();
        let multi_conn_key_id = uuid::Uuid::new_v4().to_string();
        let connection_id = uuid::Uuid::new_v4().to_string();

        // Legacy key: connection_id=None, pending_auth.
        let mut legacy = provider_key(
            &legacy_key_id,
            &user_id,
            &provider_id,
            "pending_auth",
            "oauth2",
        );
        legacy.access_token_encrypted = None;
        // Multi-connection key: connection_id=Some, also pending_auth.
        let mut multi = provider_key(
            &multi_conn_key_id,
            &user_id,
            &provider_id,
            "pending_auth",
            "oauth2",
        );
        multi.connection_id = Some(connection_id);
        multi.access_token_encrypted = None;

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_many(vec![legacy, multi])
            .await
            .unwrap();

        // Insert a legacy provider token to sync from.
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: user_id.clone(),
                provider_config_id: provider_id.clone(),
                connection_id: None,
                credential_user_id: None,
                token_type: "oauth2".to_string(),
                access_token_encrypted: Some(vec![7, 7, 7]),
                refresh_token_encrypted: Some(vec![8, 8, 8]),
                token_scopes: Some("legacy".to_string()),
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

        // Legacy key: tokens copied in, flipped to active.
        let legacy_after = get_key(&db, &legacy_key_id).await;
        assert_eq!(legacy_after.status, "active");
        assert_eq!(legacy_after.access_token_encrypted, Some(vec![7, 7, 7]));

        // Multi-connection key: completely untouched.
        let multi_after = get_key(&db, &multi_conn_key_id).await;
        assert_eq!(
            multi_after.status, "pending_auth",
            "multi-connection key must not be activated by a legacy sync"
        );
        assert!(
            multi_after.access_token_encrypted.is_none(),
            "multi-connection key must not inherit legacy tokens"
        );
    }

    #[tokio::test]
    async fn reconcile_skips_pass1_for_multi_connection_keys() {
        // B1 isolation: scenario from the design doc.
        //   1. Legacy codex A exists with a `user_provider_tokens` row.
        //   2. User adds codex B (multi-connection, pending_auth).
        //   3. A's background refresh bumps user_provider_tokens.updated_at
        //      past B's updated_at.
        //   4. GET /keys/{B.id} triggers reconcile on B. WITHOUT the fix,
        //      Pass 1 finds the newer token and activates B with A's
        //      credentials — the silent-alias bug.
        // With the fix, Pass 1 is skipped for multi-connection keys. Pass 2
        // still runs, but here B's own OAuthState is live, so the
        // placeholder is correctly left `pending_auth` for its own callback.
        let Some(db) = connect_test_database("user_api_key_reconcile_skips_pass1_multi").await
        else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };

        let now = Utc::now();
        let earlier = now - Duration::minutes(10);
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let multi_key_id = uuid::Uuid::new_v4().to_string();
        let conn_id = uuid::Uuid::new_v4().to_string();

        // Pending multi-connection placeholder created BEFORE the
        // legacy token's refresh bumped its updated_at.
        let mut multi = provider_key(
            &multi_key_id,
            &user_id,
            &provider_id,
            "pending_auth",
            "oauth2",
        );
        multi.connection_id = Some(conn_id.clone());
        multi.access_token_encrypted = None;
        multi.refresh_token_encrypted = None;
        multi.updated_at = earlier;
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(multi)
            .await
            .unwrap();

        // Legacy provider-token row, with a FRESHER updated_at — this is
        // the Pass 1 trap the reconcile logic must NOT take.
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: user_id.clone(),
                provider_config_id: provider_id.clone(),
                connection_id: None,
                credential_user_id: None,
                token_type: "oauth2".to_string(),
                access_token_encrypted: Some(vec![9, 9, 9]),
                refresh_token_encrypted: Some(vec![8, 8, 8]),
                token_scopes: Some("legacy".to_string()),
                expires_at: None,
                api_key_encrypted: None,
                status: "active".to_string(),
                last_refreshed_at: None,
                last_used_at: None,
                error_message: None,
                label: None,
                metadata: None,
                gateway_url: None,
                created_at: earlier,
                updated_at: now,
            })
            .await
            .unwrap();

        // B's own OAuthState is live (carries the matching connection_id)
        // so Pass 2 must NOT fail it — the flow is still in progress.
        let mut state = live_oauth_state(&user_id, &provider_id);
        state.connection_id = Some(conn_id.clone());
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(state)
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &multi_key_id)
            .await
            .unwrap();

        let after = get_key(&db, &multi_key_id).await;
        assert_eq!(
            after.status, "pending_auth",
            "multi-connection placeholder must stay pending: Pass 1 skipped, Pass 2 sees live state"
        );
        assert!(
            after.access_token_encrypted.is_none(),
            "multi-connection placeholder must not inherit legacy access_token"
        );
    }

    #[tokio::test]
    async fn reconcile_fails_abandoned_multi_connection_placeholder() {
        // Abandonment sweep: a multi-connection placeholder whose own
        // OAuthState is gone (user closed the provider tab, TTL swept the
        // state) must be marked `failed` by Pass 2 — same cleanup
        // guarantee legacy keys have. Without Pass 2 running for
        // multi-connection keys, the row would be a permanent orphan.
        let Some(db) = connect_test_database("user_api_key_reconcile_fails_abandoned_multi").await
        else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let multi_key_id = uuid::Uuid::new_v4().to_string();
        let conn_id = uuid::Uuid::new_v4().to_string();

        let mut multi = provider_key(
            &multi_key_id,
            &user_id,
            &provider_id,
            "pending_auth",
            "oauth2",
        );
        multi.connection_id = Some(conn_id.clone());
        multi.access_token_encrypted = None;
        // Age the placeholder past the 60s fresh-placeholder grace window
        // so Pass 2's abandonment sweep actually fires. A freshly-minted
        // multi-connection placeholder is deliberately spared (its
        // OAuthState insert is a beat behind `POST /keys`).
        multi.created_at = Utc::now() - Duration::minutes(5);
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(multi)
            .await
            .unwrap();

        // A live OAuthState exists for the SAME (user, provider) but a
        // DIFFERENT connection_id — a sibling connection's in-flight
        // flow. It must NOT keep this placeholder alive: Pass 2 is
        // scoped by connection_id.
        let mut sibling_state = live_oauth_state(&user_id, &provider_id);
        sibling_state.connection_id = Some(uuid::Uuid::new_v4().to_string());
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(sibling_state)
            .await
            .unwrap();

        reconcile_pending_oauth_placeholder(&db, &user_id, &multi_key_id)
            .await
            .unwrap();

        let after = get_key(&db, &multi_key_id).await;
        assert_eq!(
            after.status, "failed",
            "abandoned multi-connection placeholder must be swept to failed by Pass 2"
        );
        assert!(
            after.error_message.is_some(),
            "failed placeholder should carry a user-facing error message"
        );
    }

    #[tokio::test]
    async fn reconcile_spares_fresh_multi_connection_placeholder_without_state() {
        // Race-window guard: between `POST /keys` (mints the multi-
        // connection placeholder) and the wizard's separate
        // OAuth-initiate request (creates the connection-scoped
        // OAuthState), a `GET /keys/:id` poll sees a placeholder with NO
        // matching live state. Pass 2 must NOT fail it — the state
        // insert is a beat behind. A placeholder younger than the 60s
        // grace window is spared.
        let Some(db) = connect_test_database("user_api_key_reconcile_spares_fresh_multi").await
        else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let multi_key_id = uuid::Uuid::new_v4().to_string();
        let conn_id = uuid::Uuid::new_v4().to_string();

        // Fresh placeholder: `provider_key` sets created_at = now, which
        // is well within the 60s grace window.
        let mut multi = provider_key(
            &multi_key_id,
            &user_id,
            &provider_id,
            "pending_auth",
            "oauth2",
        );
        multi.connection_id = Some(conn_id);
        multi.access_token_encrypted = None;
        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(multi)
            .await
            .unwrap();

        // No OAuthState at all — simulating the gap before the
        // OAuth-initiate request lands.
        reconcile_pending_oauth_placeholder(&db, &user_id, &multi_key_id)
            .await
            .unwrap();

        let after = get_key(&db, &multi_key_id).await;
        assert_eq!(
            after.status, "pending_auth",
            "a fresh multi-connection placeholder must be spared by the grace window"
        );
    }

    #[tokio::test]
    async fn fail_pending_placeholders_skips_multi_connection_keys() {
        // B2 isolation test for the failure path: a legacy provider
        // denial must NOT mark a concurrent multi-connection placeholder
        // as failed. Otherwise an unrelated codex / Lark add could be
        // sabotaged by a sibling's bad refresh.
        let Some(db) = connect_test_database("user_api_key_fail_skips_multi_conn").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let legacy_key_id = uuid::Uuid::new_v4().to_string();
        let multi_key_id = uuid::Uuid::new_v4().to_string();
        let conn_id = uuid::Uuid::new_v4().to_string();

        let legacy = provider_key(
            &legacy_key_id,
            &user_id,
            &provider_id,
            "pending_auth",
            "oauth2",
        );
        let mut multi = provider_key(
            &multi_key_id,
            &user_id,
            &provider_id,
            "pending_auth",
            "oauth2",
        );
        multi.connection_id = Some(conn_id);

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_many(vec![legacy, multi])
            .await
            .unwrap();

        let failed =
            fail_pending_placeholders_for_provider(&db, &user_id, &provider_id, "access_denied")
                .await
                .unwrap();

        assert_eq!(failed, 1, "only the legacy key should be failed");
        assert_eq!(get_key(&db, &legacy_key_id).await.status, "failed");
        assert_eq!(
            get_key(&db, &multi_key_id).await.status,
            "pending_auth",
            "multi-connection key must not be failed by a legacy denial"
        );
    }

    #[tokio::test]
    async fn fail_connection_placeholder_marks_matching_placeholder_failed() {
        let Some(db) = connect_test_database("user_api_key_fail_conn_marks_matching").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        let conn_id = uuid::Uuid::new_v4().to_string();

        let mut key = provider_key(&key_id, &user_id, &provider_id, "pending_auth", "oauth2");
        key.connection_id = Some(conn_id.clone());
        key.credential_encrypted = Some(vec![1, 2]);
        key.access_token_encrypted = Some(vec![3, 4]);
        key.refresh_token_encrypted = Some(vec![5, 6]);
        key.token_scopes = Some("scope1".to_string());
        key.expires_at = Some(Utc::now());

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(key)
            .await
            .unwrap();

        let modified = fail_connection_placeholder(&db, &conn_id, "test_error_message")
            .await
            .unwrap();

        assert_eq!(modified, 1);

        let updated = get_key(&db, &key_id).await;
        assert_eq!(updated.status, "failed");
        assert_eq!(
            updated.error_message,
            Some("test_error_message".to_string())
        );
        assert!(updated.credential_encrypted.is_none());
        assert!(updated.access_token_encrypted.is_none());
        assert!(updated.refresh_token_encrypted.is_none());
        assert!(updated.token_scopes.is_none());
        assert!(updated.expires_at.is_none());
    }

    #[tokio::test]
    async fn fail_connection_placeholder_isolation() {
        let Some(db) = connect_test_database("user_api_key_fail_conn_isolation").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_x_id = uuid::Uuid::new_v4().to_string();
        let key_y_id = uuid::Uuid::new_v4().to_string();
        let conn_x = uuid::Uuid::new_v4().to_string();
        let conn_y = uuid::Uuid::new_v4().to_string();

        let mut key_x = provider_key(&key_x_id, &user_id, &provider_id, "pending_auth", "oauth2");
        key_x.connection_id = Some(conn_x.clone());

        let mut key_y = provider_key(&key_y_id, &user_id, &provider_id, "pending_auth", "oauth2");
        key_y.connection_id = Some(conn_y.clone());

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_many(vec![key_x, key_y])
            .await
            .unwrap();

        let modified = fail_connection_placeholder(&db, &conn_x, "error_x")
            .await
            .unwrap();

        assert_eq!(modified, 1);

        assert_eq!(get_key(&db, &key_x_id).await.status, "failed");
        assert_eq!(get_key(&db, &key_y_id).await.status, "pending_auth");
    }

    #[tokio::test]
    async fn fail_connection_placeholder_race_safety() {
        let Some(db) = connect_test_database("user_api_key_fail_conn_race").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        let conn_id = uuid::Uuid::new_v4().to_string();

        let mut key = provider_key(&key_id, &user_id, &provider_id, "active", "oauth2");
        key.connection_id = Some(conn_id.clone());

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(key)
            .await
            .unwrap();

        let modified = fail_connection_placeholder(&db, &conn_id, "late_error")
            .await
            .unwrap();

        assert_eq!(modified, 0);

        let updated = get_key(&db, &key_id).await;
        assert_eq!(updated.status, "active");
        assert!(updated.error_message.is_none());
    }

    #[tokio::test]
    async fn write_oauth_tokens_to_key_treats_empty_refresh_as_absent() {
        let Some(db) = connect_test_database("user_api_key_write_oauth_empty_refresh").await else {
            eprintln!("skipping write_oauth_tokens_to_key test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let now = Utc::now();
        let connection_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(UserApiKey {
                id: key_id.clone(),
                user_id: uuid::Uuid::new_v4().to_string(),
                label: "k".to_string(),
                credential_type: "oauth2".to_string(),
                credential_encrypted: None,
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: Some(uuid::Uuid::new_v4().to_string()),
                connection_id: Some(connection_id.clone()),
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

        // Empty-string refresh should serialize as Null (same as None),
        // not encrypt and store an empty ciphertext blob. Also test that
        // `token_scopes: None` round-trips as None.
        write_oauth_tokens_to_key(
            &db,
            &encryption_keys,
            &connection_id,
            "access-token",
            Some(""),
            None,
            None,
        )
        .await
        .unwrap();

        let restored = get_key(&db, &key_id).await;
        assert_eq!(restored.status, "active");
        assert!(restored.refresh_token_encrypted.is_none());
        assert!(restored.token_scopes.is_none());
    }

    #[tokio::test]
    async fn write_oauth_tokens_to_key_returns_not_found_for_unknown_connection() {
        let Some(db) = connect_test_database("user_api_key_write_oauth_not_found").await else {
            eprintln!("skipping write_oauth_tokens_to_key test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();

        let bogus_connection_id = uuid::Uuid::new_v4().to_string();
        let result = write_oauth_tokens_to_key(
            &db,
            &encryption_keys,
            &bogus_connection_id,
            "access-token",
            None,
            None,
            None,
        )
        .await;

        match result {
            Err(crate::errors::AppError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_oauth_tokens_to_key_refuses_to_resurrect_revoked_key() {
        // A duplicate or late provider callback for a connection the user
        // has since revoked must NOT flip the key back to `active`.
        // `revoked` / `failed` are terminal — same contract as
        // `sync_provider_token_to_api_keys`.
        let Some(db) = connect_test_database("user_api_key_write_oauth_revoked").await else {
            eprintln!("skipping write_oauth_tokens_to_key test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();

        let now = Utc::now();
        let connection_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(super::COLLECTION_NAME)
            .insert_one(UserApiKey {
                id: key_id.clone(),
                user_id: uuid::Uuid::new_v4().to_string(),
                label: "revoked codex".to_string(),
                credential_type: "oauth2".to_string(),
                credential_encrypted: None,
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: Some(uuid::Uuid::new_v4().to_string()),
                connection_id: Some(connection_id.clone()),
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                status: "revoked".to_string(),
                last_used_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let result = write_oauth_tokens_to_key(
            &db,
            &encryption_keys,
            &connection_id,
            "access-token",
            None,
            None,
            None,
        )
        .await;

        // The terminal-status filter means no row matches → NotFound.
        match result {
            Err(crate::errors::AppError::NotFound(_)) => {}
            other => panic!("expected NotFound for revoked key, got {other:?}"),
        }

        // The revoked key is untouched — not resurrected to `active`.
        let after = get_key(&db, &key_id).await;
        assert_eq!(after.status, "revoked");
        assert!(after.access_token_encrypted.is_none());
    }

    #[tokio::test]
    async fn write_oauth_tokens_to_key_only_touches_matching_connection() {
        let Some(db) = connect_test_database("user_api_key_write_oauth_isolated").await else {
            eprintln!("skipping write_oauth_tokens_to_key test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();

        let now = Utc::now();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let conn_a = uuid::Uuid::new_v4().to_string();
        let conn_b = uuid::Uuid::new_v4().to_string();
        let key_a = uuid::Uuid::new_v4().to_string();
        let key_b = uuid::Uuid::new_v4().to_string();

        // Insert two pending keys for the same user+provider but different
        // connection_ids — this is the multi-add scenario (e.g. user adds
        // a second codex; both keys exist concurrently while one is
        // authorizing).
        for (key_id, conn_id) in [(&key_a, &conn_a), (&key_b, &conn_b)] {
            db.collection::<UserApiKey>(super::COLLECTION_NAME)
                .insert_one(UserApiKey {
                    id: key_id.clone(),
                    user_id: user_id.clone(),
                    label: format!("Key for {conn_id}"),
                    credential_type: "oauth2".to_string(),
                    credential_encrypted: None,
                    access_token_encrypted: None,
                    refresh_token_encrypted: None,
                    token_scopes: None,
                    expires_at: None,
                    provider_config_id: Some(provider_id.clone()),
                    connection_id: Some(conn_id.clone()),
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
        }

        // Authorize only connection A.
        write_oauth_tokens_to_key(
            &db,
            &encryption_keys,
            &conn_a,
            "token-for-A",
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let restored_a = get_key(&db, &key_a).await;
        let restored_b = get_key(&db, &key_b).await;

        assert_eq!(restored_a.status, "active");
        assert!(restored_a.access_token_encrypted.is_some());

        // Connection B must be untouched — proves the new write path
        // does NOT fan out across a `(user, provider)` pair.
        assert_eq!(restored_b.status, "pending_auth");
        assert!(restored_b.access_token_encrypted.is_none());
    }

    // ──────────────────────────────────────────────────────────────────
    // BYO Custom App credentials (Lark / Feishu / Twitter multi-connection).
    // These prove that `create_api_key` actually persists the user-
    // provided OAuth client_id/secret onto the new `UserApiKey`, which
    // `refresh_user_api_key_in_place` already knows how to read.
    // Without this write path, multi-connection Lark refresh fails on
    // first expiry.
    // ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_api_key_stores_byo_oauth_client_credentials() {
        let Some(db) = connect_test_database("user_api_key_create_with_byo_creds").await else {
            eprintln!("skipping create_api_key BYO test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let connection_id = uuid::Uuid::new_v4().to_string();

        let key = super::create_api_key(
            &db,
            &encryption_keys,
            &user_id,
            super::CreateApiKeyParams {
                label: "Marketing Lark",
                credential_type: "oauth2",
                credential: "",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: Some(&provider_id),
                connection_id: Some(&connection_id),
                oauth_client_id: Some("cli_marketing_app"),
                oauth_client_secret: Some("super-secret-marketing"),
                status: "pending_auth",
                source: Some("user_created"),
                source_id: None,
            },
        )
        .await
        .unwrap();

        let stored = get_key(&db, &key.id).await;
        assert_eq!(
            stored.connection_id.as_deref(),
            Some(connection_id.as_str())
        );
        let enc_cid = stored
            .user_oauth_client_id_encrypted
            .expect("client_id must be persisted");
        let enc_sec = stored
            .user_oauth_client_secret_encrypted
            .expect("client_secret must be persisted");
        let dec_cid = encryption_keys.decrypt(&enc_cid).await.unwrap();
        let dec_sec = encryption_keys.decrypt(&enc_sec).await.unwrap();
        assert_eq!(String::from_utf8(dec_cid).unwrap(), "cli_marketing_app");
        assert_eq!(
            String::from_utf8(dec_sec).unwrap(),
            "super-secret-marketing"
        );
    }

    #[tokio::test]
    async fn create_api_key_rejects_unpaired_byo_credentials() {
        let Some(db) = connect_test_database("user_api_key_create_unpaired_byo").await else {
            eprintln!("skipping unpaired-BYO test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();

        // Only client_id supplied — missing secret half.
        let err = super::create_api_key(
            &db,
            &encryption_keys,
            &user_id,
            super::CreateApiKeyParams {
                label: "Bad",
                credential_type: "oauth2",
                credential: "",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: Some("cli_orphan"),
                oauth_client_secret: None,
                status: "pending_auth",
                source: Some("user_created"),
                source_id: None,
            },
        )
        .await
        .expect_err("unpaired BYO should be rejected");
        assert!(matches!(err, crate::errors::AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_api_key_omits_byo_when_none_supplied() {
        // Regression guard: existing call sites that don't supply BYO
        // creds must continue to store `None` for both encrypted halves,
        // so legacy keys aren't accidentally tagged as BYO.
        let Some(db) = connect_test_database("user_api_key_create_no_byo").await else {
            eprintln!("skipping no-BYO test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();

        let key = super::create_api_key(
            &db,
            &encryption_keys,
            &user_id,
            super::CreateApiKeyParams {
                label: "Plain API key",
                credential_type: "api_key",
                credential: "sk-test-123",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: Some("user_created"),
                source_id: None,
            },
        )
        .await
        .unwrap();

        let stored = get_key(&db, &key.id).await;
        assert!(stored.user_oauth_client_id_encrypted.is_none());
        assert!(stored.user_oauth_client_secret_encrypted.is_none());
    }
}
