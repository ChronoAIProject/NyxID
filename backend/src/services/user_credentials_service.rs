use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::provider_config::ProviderConfig;
use crate::models::user_api_key::UserApiKey;
use crate::models::user_provider_credentials::{COLLECTION_NAME, UserProviderCredentials};

/// Upsert per-user OAuth app credentials for a provider.
///
/// If credentials already exist for this (user, provider) pair, they are updated.
/// Otherwise, a new record is inserted.
pub async fn upsert_user_credentials(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_config_id: &str,
    client_id: &str,
    client_secret: Option<&str>,
    label: Option<&str>,
) -> AppResult<UserProviderCredentials> {
    upsert_user_credentials_with_expected_state_version(
        db,
        encryption_keys,
        user_id,
        provider_config_id,
        client_id,
        client_secret,
        label,
        None,
    )
    .await
}

/// Upsert per-user OAuth app credentials with an optional state-version
/// precondition. The browser assistant uses the precondition to fence the
/// secret-bearing write against evidence read before the dialog opened.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_user_credentials_with_expected_state_version(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    provider_config_id: &str,
    client_id: &str,
    client_secret: Option<&str>,
    label: Option<&str>,
    expected_state_version: Option<i64>,
) -> AppResult<UserProviderCredentials> {
    let collection = db.collection::<UserProviderCredentials>(COLLECTION_NAME);
    let now = Utc::now();

    // Allocate or load the authoritative record identity before KMS work.
    // Recheck the exact raw storage snapshot in the commit transaction.
    let raw_source = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"user_id":user_id,"provider_config_id":provider_config_id})
        .await?;
    let record_id = match &raw_source {
        Some(row) => row
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid OAuth client record".into()))?
            .to_owned(),
        None => Uuid::new_v4().to_string(),
    };
    let mut versions = std::collections::BTreeMap::new();
    // Public clients have no secret. Treat an explicitly empty optional secret
    // exactly like absence, including its storage dispatch marker.
    let client_secret = client_secret.filter(|value| !value.is_empty());
    super::credential_record_service::mark_version(
        &mut versions,
        encryption_keys,
        "client_id_encrypted",
    );
    if client_secret.is_some() {
        super::credential_record_service::mark_version(
            &mut versions,
            encryption_keys,
            "client_secret_encrypted",
        );
    }
    let client_id_enc = super::credential_record_service::encrypt(
        encryption_keys,
        COLLECTION_NAME,
        &record_id,
        user_id,
        "client_id_encrypted",
        client_id.as_bytes(),
    )
    .await?;
    let client_secret_enc = match client_secret {
        Some(s) => Some(
            super::credential_record_service::encrypt(
                encryption_keys,
                COLLECTION_NAME,
                &record_id,
                user_id,
                "client_secret_encrypted",
                s.as_bytes(),
            )
            .await?,
        ),
        None => None,
    };

    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<UserProviderCredentials> = async {
        super::erasure_admission::credential_change_in_session(db, user_id, &mut tx).await?;
        let owner = db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":user_id,"is_active":true,"platform_personal_deleting":{"$ne":true}},
                doc! {"$inc":{"credential_owner_fence":1_i64}},
            )
            .session(&mut tx)
            .await?;
        if owner.matched_count != 1 {
            return Err(AppError::Conflict(
                "Credential owner is unavailable or changed".into(),
            ));
        }
        let current_source = db.collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"user_id":user_id,"provider_config_id":provider_config_id}).session(&mut tx).await?;
        if current_source != raw_source { return Err(stale_credentials_conflict()); }
        let existing = current_source.map(bson::from_document::<UserProviderCredentials>)
            .transpose().map_err(|_| AppError::Internal("Invalid OAuth client storage".into()))?;

        if let Some(existing) = existing {
            if expected_state_version.is_some_and(|expected| existing.state_version != expected) {
                return Err(stale_credentials_conflict());
            }
            let mut set_doc = doc! {
                "client_id_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: client_id_enc,
                },
                "updated_at": bson::DateTime::from_chrono(now),
                "secret_context_versions": {"$literal":bson::to_bson(&versions).map_err(|_| AppError::Internal("Invalid credential versions".into()))?},
            };

            if let Some(enc) = client_secret_enc {
                set_doc.insert(
                    "client_secret_encrypted",
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: enc,
                    },
                );
            } else {
                set_doc.insert("client_secret_encrypted", bson::Bson::Null);
            }

            // PATCH semantics: None = "don't change", Some("") = "clear label"
            if let Some(l) = label {
                set_doc.insert("label", doc! {"$literal":l});
            }
            set_doc.insert(
                "state_version",
                doc! { "$add": [{ "$ifNull": ["$state_version", 0_i64] }, 1_i64] },
            );

            use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

            let mut filter = super::record_storage::source_predicate(raw_source.as_ref().unwrap(),
                &["user_id","provider_config_id","client_id_encrypted","client_secret_encrypted","secret_context_versions","state_version"]);
            if let Some(expected) = expected_state_version {
                let source_check = filter.remove("$expr").unwrap();
                filter.insert("$expr", doc! {"$and":[source_check,{"$eq":[{"$ifNull":["$state_version",0_i64]},expected]}]});
            }

            let updated = collection
                .find_one_and_update(filter, vec![doc! { "$set": set_doc }])
                .session(&mut tx)
                .with_options(
                    FindOneAndUpdateOptions::builder()
                        .return_document(ReturnDocument::After)
                        .build(),
                )
                .await?
                .ok_or_else(stale_credentials_conflict)?;

            tracing::info!(
                user_id = %user_id,
                provider_config_id = %provider_config_id,
                "User provider credentials updated"
            );

            Ok(updated)
        } else {
            if expected_state_version.is_some_and(|expected| expected != 0) {
                return Err(stale_credentials_conflict());
            }
            let cred = UserProviderCredentials {
                secret_context_versions: versions,
                id: record_id,
                user_id: user_id.to_string(),
                provider_config_id: provider_config_id.to_string(),
                client_id_encrypted: Some(client_id_enc),
                client_secret_encrypted: client_secret_enc,
                label: label.map(String::from),
                state_version: 1,
                created_at: now,
                updated_at: now,
            };

            collection.insert_one(&cred).session(&mut tx).await?;

            tracing::info!(
                user_id = %user_id,
                provider_config_id = %provider_config_id,
                "User provider credentials created"
            );

            Ok(cred)
        }
    }
    .await;
    super::org_membership_authority::finish(tx, result).await
}

fn stale_credentials_conflict() -> AppError {
    AppError::Conflict(
        "the provider credentials changed since this action was prepared".to_string(),
    )
}

/// Get raw user credentials (for internal use, e.g. resolving OAuth creds).
pub async fn get_user_credentials(
    db: &mongodb::Database,
    user_id: &str,
    provider_config_id: &str,
) -> AppResult<Option<UserProviderCredentials>> {
    db.collection::<UserProviderCredentials>(COLLECTION_NAME)
        .find_one(doc! { "user_id": user_id, "provider_config_id": provider_config_id })
        .await
        .map_err(AppError::from)
}

/// Metadata about user credentials (without secrets) for API responses.
pub struct UserCredentialsMetadata {
    pub provider_config_id: String,
    pub label: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub state_version: i64,
}

/// Get credentials metadata without secrets (for API response).
pub async fn get_user_credentials_metadata(
    db: &mongodb::Database,
    user_id: &str,
    provider_config_id: &str,
) -> AppResult<Option<UserCredentialsMetadata>> {
    let cred = get_user_credentials(db, user_id, provider_config_id).await?;
    Ok(cred.map(|c| UserCredentialsMetadata {
        provider_config_id: c.provider_config_id,
        label: c.label,
        created_at: c.created_at,
        updated_at: c.updated_at,
        state_version: c.state_version,
    }))
}

/// Delete user credentials for a provider.
pub async fn delete_user_credentials(
    db: &mongodb::Database,
    user_id: &str,
    provider_config_id: &str,
) -> AppResult<()> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        super::erasure_admission::credential_change_in_session(db, user_id, &mut tx).await?;
        let removed = db
            .collection::<UserProviderCredentials>(COLLECTION_NAME)
            .delete_one(doc! {"user_id":user_id,"provider_config_id":provider_config_id})
            .session(&mut tx)
            .await?;
        if removed.deleted_count != 1 {
            return Err(AppError::NotFound("User credentials not found".into()));
        }
        Ok(())
    }
    .await;
    super::org_membership_authority::finish(tx, result).await
}

/// Resolved OAuth client credentials (decrypted).
pub struct ResolvedOAuthCredentials {
    pub app_source: super::oauth_app_source::OAuthAppSource,
    pub client_id: Zeroizing<String>,
    pub client_secret: Option<Zeroizing<String>>,
    /// Legacy user-provider credential lookup. Embedded connection apps also
    /// leave this empty; use `app_source` to identify the selected app source.
    pub credential_user_id: Option<String>,
}

impl std::fmt::Debug for ResolvedOAuthCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedOAuthCredentials")
            .field("app_source", &self.app_source)
            .field("client_id", &"[REDACTED]")
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("credential_user_id", &self.credential_user_id)
            .finish()
    }
}

pub fn provider_has_admin_oauth_credentials(provider: &ProviderConfig) -> bool {
    match provider.provider_type.as_str() {
        "oauth2" => {
            // OAuth2 requires both client_id and client_secret for admin credentials
            provider.client_id_encrypted.is_some() && provider.client_secret_encrypted.is_some()
        }
        "device_code" => {
            // Device code flow uses public clients -- only client_id is required
            // (no client_secret needed per RFC 8628 section 3.1)
            provider.client_id_encrypted.is_some()
        }
        _ => false,
    }
}

/// Resolve OAuth credentials based on the provider's `credential_mode`.
///
/// - `"admin"` -> use provider-level credentials (error if none)
/// - `"user"` -> use per-user credentials (error if none)
/// - `"both"` -> try per-user first, fall back to provider-level
pub async fn resolve_oauth_credentials(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
    user_id: &str,
) -> AppResult<ResolvedOAuthCredentials> {
    let mode = &provider.credential_mode;

    match mode.as_str() {
        "admin" => decrypt_provider_credentials(encryption_keys, provider).await,
        "user" => {
            resolve_user_credentials_for_owner(
                db,
                encryption_keys,
                provider,
                user_id,
                "This provider requires you to configure your own OAuth app credentials",
            )
            .await
        }
        "both" => {
            let user_creds = get_user_credentials(db, user_id, &provider.id).await?;
            if let Some(creds) = user_creds {
                decrypt_user_credentials(encryption_keys, &creds, user_id).await
            } else if provider_has_admin_oauth_credentials(provider) {
                decrypt_provider_credentials(encryption_keys, provider).await
            } else {
                Err(AppError::BadRequest(
                    "This provider requires either admin-configured OAuth app credentials or your own OAuth app credentials".to_string(),
                ))
            }
        }
        _ => {
            tracing::warn!(
                provider_id = %provider.id,
                mode = %mode,
                "Unknown credential_mode, falling back to admin"
            );
            decrypt_provider_credentials(encryption_keys, provider).await
        }
    }
}

/// Resolve the exact OAuth client credentials previously used to mint a token.
///
/// This bypasses the provider's current `credential_mode` so refreshes keep using
/// the same OAuth client that issued the refresh token.
pub async fn resolve_token_oauth_credentials(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
    credential_user_id: Option<&str>,
) -> AppResult<ResolvedOAuthCredentials> {
    match credential_user_id {
        Some(user_id) => {
            resolve_user_credentials_for_owner(
                db,
                encryption_keys,
                provider,
                user_id,
                "The OAuth app credentials used for this connection are no longer available. Reconnect after configuring them again.",
            )
            .await
        }
        None => {
            if provider_has_admin_oauth_credentials(provider) {
                decrypt_provider_credentials(encryption_keys, provider).await
            } else {
                Err(AppError::BadRequest(
                    "The provider's OAuth app credentials are no longer configured. Reconnect after an admin updates the provider."
                        .to_string(),
                ))
            }
        }
    }
}

async fn resolve_user_credentials_for_owner(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
    user_id: &str,
    missing_message: &str,
) -> AppResult<ResolvedOAuthCredentials> {
    let user_creds = get_user_credentials(db, user_id, &provider.id).await?;
    match user_creds {
        Some(creds) => decrypt_user_credentials(encryption_keys, &creds, user_id).await,
        None => Err(AppError::BadRequest(missing_message.to_string())),
    }
}

/// Decrypt provider-level (admin) OAuth credentials.
///
pub(crate) async fn decrypt_provider_credentials(
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
) -> AppResult<ResolvedOAuthCredentials> {
    let encrypted_cid = provider.client_id_encrypted.as_ref().ok_or_else(|| {
        AppError::Internal(format!("Provider {} missing client_id", provider.slug))
    })?;

    let client_id = super::credential_record_service::CredentialRecord::provider(provider)
        .text(encryption_keys, "client_id_encrypted", encrypted_cid)
        .await?;
    let client_secret = match provider.client_secret_encrypted.as_deref() {
        Some(value) => Some(
            super::credential_record_service::CredentialRecord::provider(provider)
                .text(encryption_keys, "client_secret_encrypted", value)
                .await?,
        ),
        None => None,
    };

    Ok(ResolvedOAuthCredentials {
        app_source: super::oauth_app_source::OAuthAppSource::Platform,
        client_id,
        client_secret: client_secret.filter(|secret| !secret.is_empty()),
        credential_user_id: None,
    })
}

/// Resolve the OAuth app credentials carried by a claimed modern key. BYO
/// credentials are decrypted from the pre-image itself; rows without embedded
/// credentials use the provider-level app and never query the deleted key.
pub async fn decrypt_claimed_key_oauth_credentials(
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
    key: &UserApiKey,
) -> AppResult<ResolvedOAuthCredentials> {
    let Some(encrypted_client_id) = key.user_oauth_client_id_encrypted.as_ref() else {
        return decrypt_provider_credentials(encryption_keys, provider).await;
    };

    let client_id = super::credential_record_service::CredentialRecord::key(key)
        .text(
            encryption_keys,
            "user_oauth_client_id_encrypted",
            encrypted_client_id,
        )
        .await?;
    let client_secret = match key.user_oauth_client_secret_encrypted.as_deref() {
        Some(value) => Some(
            super::credential_record_service::CredentialRecord::key(key)
                .text(encryption_keys, "user_oauth_client_secret_encrypted", value)
                .await?,
        ),
        None => None,
    };

    Ok(ResolvedOAuthCredentials {
        app_source: super::oauth_app_source::OAuthAppSource::Byo,
        client_id,
        client_secret: client_secret.filter(|secret| !secret.is_empty()),
        credential_user_id: Some(key.user_id.clone()),
    })
}

/// Decrypt user-level OAuth credentials.
///
/// The returned client material owns zeroizing strings.
async fn decrypt_user_credentials(
    encryption_keys: &EncryptionKeys,
    creds: &UserProviderCredentials,
    credential_user_id: &str,
) -> AppResult<ResolvedOAuthCredentials> {
    let encrypted_cid = creds
        .client_id_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("User credentials missing client_id".to_string()))?;

    let client_id = super::credential_record_service::CredentialRecord::user_credentials(creds)
        .text(encryption_keys, "client_id_encrypted", encrypted_cid)
        .await?;
    let client_secret = match creds.client_secret_encrypted.as_deref() {
        Some(value) => Some(
            super::credential_record_service::CredentialRecord::user_credentials(creds)
                .text(encryption_keys, "client_secret_encrypted", value)
                .await?,
        ),
        None => None,
    };

    Ok(ResolvedOAuthCredentials {
        app_source: super::oauth_app_source::OAuthAppSource::Byo,
        client_id,
        client_secret: client_secret.filter(|secret| !secret.is_empty()),
        credential_user_id: Some(credential_user_id.to_string()),
    })
}

/// Check if a provider supports user-level credentials.
pub fn supports_user_credentials(provider: &ProviderConfig) -> bool {
    provider.credential_mode == "user" || provider.credential_mode == "both"
}

/// Resolve OAuth client credentials directly from a multi-connection
/// `UserApiKey` identified by `connection_id`.
///
/// Returns `Ok(Some(...))` when a `UserApiKey` with this `connection_id`
/// exists and carries `user_oauth_client_id_encrypted` (and a paired
/// secret). Returns `Ok(None)` when there is no such key, or when the
/// matched key has no BYO credentials — in which case the caller falls
/// back to the legacy resolution paths (`resolve_oauth_credentials` /
/// `resolve_token_oauth_credentials`). This is the multi-connection
/// cousin of `resolve_oauth_credentials`: it lets the per-connection
/// authorize / exchange path use the connection's own Custom App
/// credentials rather than the single-row `user_provider_credentials`
/// table, which can't represent two Custom Apps for the same
/// `(user, provider)` pair.
///
/// Note: `refresh_user_api_key_in_place` does **not** use this helper —
/// it already operates on the `UserApiKey` directly (the
/// `credential_user_id` field is unused there). Keeping the refresh
/// path's existing implementation avoids needing to deserialize twice
/// in the hot path.
/// The authoritative OAuth-client source recorded on a connection's key
/// (`"platform"` | `"byo"` | `None` for legacy). Drives forced-platform
/// resolution at initiation/reconnect so a "NyxID managed" choice is never
/// overridden by a user's legacy provider-level BYO credentials.
pub async fn connection_credential_source(
    db: &mongodb::Database,
    connection_id: &str,
) -> AppResult<Option<String>> {
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use mongodb::bson::doc;

    let key = crate::services::service_history::collection::<UserApiKey>(db, USER_API_KEYS)
        .find_one(doc! { "connection_id": connection_id })
        .await?;
    Ok(key.and_then(|k| k.credential_source))
}

/// Resolve the provider's own (platform/admin) OAuth client credentials,
/// bypassing every user-credential lookup. Used when a connection is pinned
/// to `credential_source = "platform"`; the credentials stay centrally
/// rotatable on `ProviderConfig` (never copied onto the key).
pub async fn resolve_platform_oauth_credentials(
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
) -> AppResult<ResolvedOAuthCredentials> {
    decrypt_provider_credentials(encryption_keys, provider).await
}

pub async fn resolve_connection_oauth_credentials(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    connection_id: &str,
) -> AppResult<Option<ResolvedOAuthCredentials>> {
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use mongodb::bson::doc;

    let key = crate::services::service_history::collection::<UserApiKey>(db, USER_API_KEYS)
        .find_one(doc! { "connection_id": connection_id })
        .await?;
    let Some(key) = key else {
        return Ok(None);
    };
    let Some(enc_cid) = key.user_oauth_client_id_encrypted.as_ref() else {
        return Ok(None);
    };

    let client_id = super::credential_record_service::CredentialRecord::key(&key)
        .text(encryption_keys, "user_oauth_client_id_encrypted", enc_cid)
        .await?;
    let client_secret = match key.user_oauth_client_secret_encrypted.as_deref() {
        Some(value) => Some(
            super::credential_record_service::CredentialRecord::key(&key)
                .text(encryption_keys, "user_oauth_client_secret_encrypted", value)
                .await?,
        ),
        None => None,
    };

    Ok(Some(ResolvedOAuthCredentials {
        app_source: super::oauth_app_source::OAuthAppSource::Byo,
        client_id,
        client_secret: client_secret.filter(|secret| !secret.is_empty()),
        // The `credential_user_id` field on `ResolvedOAuthCredentials`
        // is load-bearing only for the legacy `user_provider_credentials`
        // path (it identifies which user-owned cred row a token was
        // minted against, so a future refresh can re-resolve from the
        // same row). For the multi-connection path, the refresh code
        // reads creds directly off the `UserApiKey` row by id, so this
        // back-reference is unused. Leaving it `None` keeps
        // `OAuthState.credential_user_id` empty on the multi-connection
        // path — verified consistent with §12 of the design doc.
        credential_user_id: None,
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn resolved_oauth_credentials_debug_redacts_client_material() {
        let credentials = super::ResolvedOAuthCredentials {
            app_source: super::super::oauth_app_source::OAuthAppSource::Byo,
            client_id: "distinctive-resolved-client-id".to_owned().into(),
            client_secret: Some("distinctive-resolved-client-secret".to_owned().into()),
            credential_user_id: Some("credential-owner".into()),
        };
        for output in [format!("{credentials:?}"), format!("{credentials:#?}")] {
            assert!(!output.contains(credentials.client_id.as_str()));
            assert!(!output.contains(credentials.client_secret.as_ref().unwrap().as_str()));
            assert!(output.contains("Byo"));
        }
    }
    use chrono::Utc;

    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::test_utils::{connect_test_database, test_encryption_keys};

    use super::resolve_connection_oauth_credentials;
    use crate::errors::AppError;
    use crate::models::user_provider_credentials::COLLECTION_NAME;
    use mongodb::bson::doc;

    fn placeholder_key(connection_id: &str) -> UserApiKey {
        UserApiKey {
            secret_context_versions: Default::default(),
            oauth_app_observation: None,
            credential_source: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            label: "test".to_string(),
            credential_type: "oauth2".to_string(),
            credential_encrypted: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: Some(uuid::Uuid::new_v4().to_string()),
            connection_id: Some(connection_id.to_string()),
            oauth_attempt_nonce: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "pending_auth".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            credential_epoch: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── Pure function tests (no MongoDB) ──────────────────────────

    #[test]
    fn provider_has_admin_oauth_credentials_oauth2_with_both() {
        let provider = make_provider("oauth2", true, true);
        assert!(super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn provider_has_admin_oauth_credentials_oauth2_missing_secret() {
        let provider = make_provider("oauth2", true, false);
        assert!(!super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn provider_has_admin_oauth_credentials_oauth2_missing_id() {
        let provider = make_provider("oauth2", false, true);
        assert!(!super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn provider_has_admin_oauth_credentials_oauth2_missing_both() {
        let provider = make_provider("oauth2", false, false);
        assert!(!super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn provider_has_admin_oauth_credentials_device_code_only_needs_client_id() {
        let provider = make_provider("device_code", true, false);
        assert!(super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn provider_has_admin_oauth_credentials_device_code_missing_client_id() {
        let provider = make_provider("device_code", false, false);
        assert!(!super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn provider_has_admin_oauth_credentials_device_code_with_both() {
        let provider = make_provider("device_code", true, true);
        assert!(super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn provider_has_admin_oauth_credentials_api_key_always_false() {
        let provider = make_provider("api_key", true, true);
        assert!(!super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn provider_has_admin_oauth_credentials_unknown_type_false() {
        let provider = make_provider("unknown_type", true, true);
        assert!(!super::provider_has_admin_oauth_credentials(&provider));
    }

    #[test]
    fn supports_user_credentials_user_mode() {
        let mut provider = make_provider("oauth2", false, false);
        provider.credential_mode = "user".to_string();
        assert!(super::supports_user_credentials(&provider));
    }

    #[test]
    fn supports_user_credentials_both_mode() {
        let mut provider = make_provider("oauth2", false, false);
        provider.credential_mode = "both".to_string();
        assert!(super::supports_user_credentials(&provider));
    }

    #[test]
    fn supports_user_credentials_admin_mode_is_false() {
        let mut provider = make_provider("oauth2", false, false);
        provider.credential_mode = "admin".to_string();
        assert!(!super::supports_user_credentials(&provider));
    }

    #[test]
    fn supports_user_credentials_unknown_mode_is_false() {
        let mut provider = make_provider("oauth2", false, false);
        provider.credential_mode = "something_else".to_string();
        assert!(!super::supports_user_credentials(&provider));
    }

    fn make_provider(
        provider_type: &str,
        has_client_id: bool,
        has_client_secret: bool,
    ) -> crate::models::provider_config::ProviderConfig {
        let now = Utc::now();
        crate::models::provider_config::ProviderConfig {
            secret_context_versions: Default::default(),
            id: uuid::Uuid::new_v4().to_string(),
            slug: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            description: None,
            provider_type: provider_type.to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
            revocation: None,
            default_scopes: None,
            client_id_encrypted: if has_client_id {
                Some(vec![1, 2, 3])
            } else {
                None
            },
            client_secret_encrypted: if has_client_secret {
                Some(vec![4, 5, 6])
            } else {
                None
            },
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            token_request_encoding: None,
            oauth_request_headers: Default::default(),
            supports_oauth_scopes: true,
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            authorization_nonce_required: false,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            revocation_seed_version: 0,
            created_at: now,
            updated_at: now,
        }
    }

    async fn seed_live_owner(db: &mongodb::Database, id: &str) {
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                id,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn credential_label_update_preserves_expression_looking_literal() {
        let db =
            crate::test_utils::connect_transaction_test_database("credential_label_literal").await;
        let owner = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &owner).await;
        let keys = test_encryption_keys();
        let provider = uuid::Uuid::new_v4().to_string();
        super::upsert_user_credentials(
            &db,
            &keys,
            &owner,
            &provider,
            "id",
            Some("secret"),
            Some("initial"),
        )
        .await
        .unwrap();
        let updated = super::upsert_user_credentials(
            &db,
            &keys,
            &owner,
            &provider,
            "id",
            Some("secret"),
            Some("$state_version"),
        )
        .await
        .unwrap();
        assert_eq!(updated.label.as_deref(), Some("$state_version"));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn credential_upsert_cannot_recreate_rows_after_owner_closure() {
        let db =
            crate::test_utils::connect_transaction_test_database("credential_owner_closure").await;
        let owner = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &owner).await;
        let keys = test_encryption_keys();
        let provider = uuid::Uuid::new_v4().to_string();
        super::upsert_user_credentials(
            &db,
            &keys,
            &owner,
            &provider,
            "first-id",
            Some("first-secret"),
            None,
        )
        .await
        .unwrap();
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&owner},
                doc! {"$set":{"is_active":false,"platform_personal_deleting":true}},
            )
            .await
            .unwrap();
        db.collection::<bson::Document>(COLLECTION_NAME)
            .delete_many(doc! {"user_id":&owner})
            .await
            .unwrap();
        assert!(matches!(
            super::upsert_user_credentials(
                &db,
                &keys,
                &owner,
                &provider,
                "late-id",
                Some("late-secret"),
                None
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .delete_one(doc! {"_id":&owner})
            .await
            .unwrap();
        assert!(matches!(
            super::upsert_user_credentials(
                &db,
                &keys,
                &owner,
                &provider,
                "late-id",
                Some("late-secret"),
                None
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    // ── Integration tests: upsert / get / delete / resolve credentials ──

    #[tokio::test]
    async fn upsert_user_credentials_creates_new_record() {
        let Some(db) = connect_test_database("uc_svc_upsert_create").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let provider_id = uuid::Uuid::new_v4().to_string();

        let cred = super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "my-client-id",
            Some("my-client-secret"),
            Some("My Lark App"),
        )
        .await
        .expect("insert should succeed");

        assert_eq!(cred.user_id, user_id);
        assert_eq!(cred.provider_config_id, provider_id);
        assert_eq!(cred.label.as_deref(), Some("My Lark App"));
        assert!(cred.client_id_encrypted.is_some());
        assert!(cred.client_secret_encrypted.is_some());
    }

    #[tokio::test]
    async fn upsert_user_credentials_updates_existing() {
        let Some(db) = connect_test_database("uc_svc_upsert_update").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let provider_id = uuid::Uuid::new_v4().to_string();

        let first = super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "cid-v1",
            Some("sec-v1"),
            Some("Label v1"),
        )
        .await
        .unwrap();

        let second = super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "cid-v2",
            Some("sec-v2"),
            Some("Label v2"),
        )
        .await
        .unwrap();

        // Same record updated (same _id)
        assert_eq!(first.id, second.id);
        assert_eq!(second.label.as_deref(), Some("Label v2"));
        assert!(second.updated_at >= first.updated_at);
    }

    #[tokio::test]
    async fn public_client_none_empty_set_clear_strict_reads_and_migration() {
        use super::super::credential_record_service as records;
        use crate::models::user_provider_credentials::UserProviderCredentials;
        let db =
            crate::test_utils::connect_transaction_test_database("byo_public_client_matrix").await;
        let owner = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &owner).await;
        let provider = uuid::Uuid::new_v4().to_string();
        let mut strict = test_encryption_keys();
        strict.set_allow_legacy_record_reads(false);
        let migration = test_encryption_keys();
        let mut version = 0;
        for secret in [
            None,
            Some(""),
            Some("distinctive-client-secret"),
            Some(""),
            Some("replacement-secret"),
            None,
        ] {
            let row = super::upsert_user_credentials_with_expected_state_version(
                &db,
                &strict,
                &owner,
                &provider,
                "public-client",
                secret,
                None,
                Some(version),
            )
            .await
            .unwrap();
            version += 1;
            assert_eq!(row.state_version, version);
            let expected = secret.filter(|value| !value.is_empty());
            assert_eq!(row.client_secret_encrypted.is_some(), expected.is_some());
            assert_eq!(
                row.secret_context_versions
                    .contains_key("client_secret_encrypted"),
                expected.is_some()
            );
            let resolved = super::decrypt_user_credentials(&strict, &row, &owner)
                .await
                .unwrap();
            assert_eq!(
                resolved
                    .client_secret
                    .as_deref()
                    .map(|value| value.as_str()),
                expected
            );
            let raw = db
                .collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            records::verify_current_snapshot(&strict, COLLECTION_NAME, &raw)
                .await
                .unwrap();
            assert!(
                !records::migrate_snapshot(&db, &migration, COLLECTION_NAME, &raw)
                    .await
                    .unwrap()
            );
            let migrated = db
                .collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {"_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            records::verify_current_snapshot(&strict, COLLECTION_NAME, &migrated)
                .await
                .unwrap();
            let migrated: UserProviderCredentials = bson::from_document(migrated).unwrap();
            assert_eq!(
                super::decrypt_user_credentials(&strict, &migrated, &owner)
                    .await
                    .unwrap()
                    .client_secret
                    .as_deref()
                    .map(|value| value.as_str()),
                expected
            );
        }

        // Exercise the former accepted encrypted-empty storage contract,
        // rather than fabricating a contextual marker over absent material.
        let mut legacy = test_encryption_keys();
        legacy.set_contextual_record_writes(false);
        let row = super::upsert_user_credentials(
            &db,
            &legacy,
            &owner,
            &provider,
            "legacy-public-client",
            None,
            None,
        )
        .await
        .unwrap();
        let encrypted_empty = legacy.encrypt(b"").await.unwrap();
        db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc! {"_id":&row.id}, doc! {"$set":{
            "client_secret_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted_empty}
        }}).await.unwrap();
        let raw = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        let historical: UserProviderCredentials = bson::from_document(raw.clone()).unwrap();
        assert!(
            super::decrypt_user_credentials(&legacy, &historical, &owner)
                .await
                .unwrap()
                .client_secret
                .is_none()
        );
        assert!(
            records::migrate_snapshot(&db, &migration, COLLECTION_NAME, &raw)
                .await
                .unwrap()
        );
        let current = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        records::verify_current_snapshot(&strict, COLLECTION_NAME, &current)
            .await
            .unwrap();
        let current: UserProviderCredentials = bson::from_document(current).unwrap();
        assert!(current.client_secret_encrypted.is_none());
        assert!(
            !current
                .secret_context_versions
                .contains_key("client_secret_encrypted")
        );
        assert!(
            super::decrypt_user_credentials(&strict, &current, &owner)
                .await
                .unwrap()
                .client_secret
                .is_none()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn upsert_user_credentials_without_secret() {
        let Some(db) = connect_test_database("uc_svc_upsert_no_secret").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let provider_id = uuid::Uuid::new_v4().to_string();

        let cred = super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "cid-only",
            None,
            None,
        )
        .await
        .unwrap();

        assert!(cred.client_id_encrypted.is_some());
        assert!(cred.client_secret_encrypted.is_none());
        assert!(cred.label.is_none());
    }

    #[tokio::test]
    async fn upsert_user_credentials_label_none_preserves_existing_label() {
        let Some(db) = connect_test_database("uc_svc_upsert_label_preserve").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let provider_id = uuid::Uuid::new_v4().to_string();

        // Create with label
        super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "cid",
            Some("sec"),
            Some("Original"),
        )
        .await
        .unwrap();

        // Update without label (None = "don't change")
        let updated = super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "cid-new",
            Some("sec-new"),
            None,
        )
        .await
        .unwrap();

        assert_eq!(updated.label.as_deref(), Some("Original"));
    }

    #[tokio::test]
    async fn get_user_credentials_returns_none_when_missing() {
        let Some(db) = connect_test_database("uc_svc_get_missing").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let result = super::get_user_credentials(
            &db,
            &uuid::Uuid::new_v4().to_string(),
            &uuid::Uuid::new_v4().to_string(),
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_user_credentials_returns_existing() {
        let Some(db) = connect_test_database("uc_svc_get_existing").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let provider_id = uuid::Uuid::new_v4().to_string();

        super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "cid",
            Some("sec"),
            Some("label"),
        )
        .await
        .unwrap();

        let result = super::get_user_credentials(&db, &user_id, &provider_id)
            .await
            .unwrap();
        assert!(result.is_some());
        let cred = result.unwrap();
        assert_eq!(cred.user_id, user_id);
        assert_eq!(cred.provider_config_id, provider_id);
    }

    #[tokio::test]
    async fn get_user_credentials_metadata_returns_without_secrets() {
        let Some(db) = connect_test_database("uc_svc_get_metadata").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let provider_id = uuid::Uuid::new_v4().to_string();

        super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "cid",
            Some("sec"),
            Some("Metadata Label"),
        )
        .await
        .unwrap();

        let meta = super::get_user_credentials_metadata(&db, &user_id, &provider_id)
            .await
            .unwrap()
            .expect("metadata should be Some");
        assert_eq!(meta.provider_config_id, provider_id);
        assert_eq!(meta.label.as_deref(), Some("Metadata Label"));
    }

    #[tokio::test]
    async fn get_user_credentials_metadata_returns_none_when_missing() {
        let Some(db) = connect_test_database("uc_svc_get_metadata_missing").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let result = super::get_user_credentials_metadata(
            &db,
            &uuid::Uuid::new_v4().to_string(),
            &uuid::Uuid::new_v4().to_string(),
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn delete_user_credentials_removes_record() {
        let Some(db) = connect_test_database("uc_svc_delete_ok").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let provider_id = uuid::Uuid::new_v4().to_string();

        super::upsert_user_credentials(&db, &enc, &user_id, &provider_id, "cid", Some("sec"), None)
            .await
            .unwrap();

        super::delete_user_credentials(&db, &user_id, &provider_id)
            .await
            .expect("delete should succeed");

        let after = super::get_user_credentials(&db, &user_id, &provider_id)
            .await
            .unwrap();
        assert!(after.is_none(), "credentials should be gone after delete");
    }

    #[tokio::test]
    async fn delete_user_credentials_not_found_when_missing() {
        let Some(db) = connect_test_database("uc_svc_delete_not_found").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let err = super::delete_user_credentials(
            &db,
            &uuid::Uuid::new_v4().to_string(),
            &uuid::Uuid::new_v4().to_string(),
        )
        .await
        .expect_err("should return NotFound");
        assert!(matches!(err, crate::errors::AppError::NotFound(_)));
    }

    // ── resolve_oauth_credentials integration tests ──────────────────

    #[tokio::test]
    async fn resolve_oauth_credentials_admin_mode_decrypts_provider_creds() {
        let Some(db) = connect_test_database("uc_svc_resolve_admin").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let cid_enc = enc.encrypt(b"admin-client-id").await.unwrap();
        let sec_enc = enc.encrypt(b"admin-secret").await.unwrap();

        let mut provider = make_provider("oauth2", true, true);
        provider.client_id_encrypted = Some(cid_enc);
        provider.client_secret_encrypted = Some(sec_enc);
        provider.credential_mode = "admin".to_string();

        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let resolved = super::resolve_oauth_credentials(&db, &enc, &provider, &user_id)
            .await
            .unwrap();

        assert_eq!(resolved.client_id.as_str(), "admin-client-id");
        assert_eq!(
            resolved
                .client_secret
                .as_ref()
                .map(|secret| secret.as_str()),
            Some("admin-secret")
        );
        assert!(resolved.credential_user_id.is_none());
    }

    #[tokio::test]
    async fn resolve_oauth_credentials_user_mode_requires_user_creds() {
        let Some(db) = connect_test_database("uc_svc_resolve_user_missing").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let mut provider = make_provider("oauth2", false, false);
        provider.credential_mode = "user".to_string();

        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let result = super::resolve_oauth_credentials(&db, &enc, &provider, &user_id).await;
        assert!(result.is_err(), "should fail without user creds");
        assert!(matches!(
            result.unwrap_err(),
            crate::errors::AppError::BadRequest(_)
        ));
    }

    #[tokio::test]
    async fn resolve_oauth_credentials_user_mode_with_stored_creds() {
        let Some(db) = connect_test_database("uc_svc_resolve_user_ok").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let mut provider = make_provider("oauth2", false, false);
        provider.credential_mode = "user".to_string();

        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;

        // Store user credentials
        super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider.id,
            "user-cid",
            Some("user-secret"),
            None,
        )
        .await
        .unwrap();

        let resolved = super::resolve_oauth_credentials(&db, &enc, &provider, &user_id)
            .await
            .unwrap();

        assert_eq!(resolved.client_id.as_str(), "user-cid");
        assert_eq!(
            resolved
                .client_secret
                .as_ref()
                .map(|secret| secret.as_str()),
            Some("user-secret")
        );
        assert_eq!(
            resolved.credential_user_id.as_deref(),
            Some(user_id.as_str())
        );
    }

    #[tokio::test]
    async fn resolve_oauth_credentials_both_mode_prefers_user_creds() {
        let Some(db) = connect_test_database("uc_svc_resolve_both_user_pref").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let admin_cid_enc = enc.encrypt(b"admin-cid").await.unwrap();
        let admin_sec_enc = enc.encrypt(b"admin-sec").await.unwrap();

        let mut provider = make_provider("oauth2", true, true);
        provider.client_id_encrypted = Some(admin_cid_enc);
        provider.client_secret_encrypted = Some(admin_sec_enc);
        provider.credential_mode = "both".to_string();

        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;

        // Store user credentials (should be preferred over admin)
        super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider.id,
            "user-cid-both",
            Some("user-sec-both"),
            None,
        )
        .await
        .unwrap();

        let resolved = super::resolve_oauth_credentials(&db, &enc, &provider, &user_id)
            .await
            .unwrap();

        assert_eq!(resolved.client_id.as_str(), "user-cid-both");
        assert_eq!(
            resolved
                .client_secret
                .as_ref()
                .map(|secret| secret.as_str()),
            Some("user-sec-both")
        );
        assert_eq!(
            resolved.credential_user_id.as_deref(),
            Some(user_id.as_str())
        );
    }

    #[tokio::test]
    async fn resolve_oauth_credentials_both_mode_falls_back_to_admin() {
        let Some(db) = connect_test_database("uc_svc_resolve_both_admin_fb").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let admin_cid_enc = enc.encrypt(b"admin-fallback-cid").await.unwrap();
        let admin_sec_enc = enc.encrypt(b"admin-fallback-sec").await.unwrap();

        let mut provider = make_provider("oauth2", true, true);
        provider.client_id_encrypted = Some(admin_cid_enc);
        provider.client_secret_encrypted = Some(admin_sec_enc);
        provider.credential_mode = "both".to_string();

        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        // No user credentials stored, should fall back to admin

        let resolved = super::resolve_oauth_credentials(&db, &enc, &provider, &user_id)
            .await
            .unwrap();

        assert_eq!(resolved.client_id.as_str(), "admin-fallback-cid");
        assert_eq!(
            resolved
                .client_secret
                .as_ref()
                .map(|secret| secret.as_str()),
            Some("admin-fallback-sec")
        );
        assert!(resolved.credential_user_id.is_none());
    }

    #[tokio::test]
    async fn resolve_oauth_credentials_both_mode_fails_when_neither_available() {
        let Some(db) = connect_test_database("uc_svc_resolve_both_neither").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let mut provider = make_provider("oauth2", false, false);
        provider.credential_mode = "both".to_string();

        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;

        let result = super::resolve_oauth_credentials(&db, &enc, &provider, &user_id).await;
        assert!(result.is_err(), "should fail with no creds in 'both' mode");
        assert!(matches!(
            result.unwrap_err(),
            crate::errors::AppError::BadRequest(_)
        ));
    }

    // ── resolve_token_oauth_credentials integration tests ────────────

    #[tokio::test]
    async fn resolve_token_oauth_credentials_admin_path() {
        let Some(db) = connect_test_database("uc_svc_resolve_tok_admin").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let cid_enc = enc.encrypt(b"tok-admin-cid").await.unwrap();
        let sec_enc = enc.encrypt(b"tok-admin-sec").await.unwrap();

        let mut provider = make_provider("oauth2", true, true);
        provider.client_id_encrypted = Some(cid_enc);
        provider.client_secret_encrypted = Some(sec_enc);

        let resolved = super::resolve_token_oauth_credentials(&db, &enc, &provider, None)
            .await
            .unwrap();

        assert_eq!(resolved.client_id.as_str(), "tok-admin-cid");
        assert_eq!(
            resolved
                .client_secret
                .as_ref()
                .map(|secret| secret.as_str()),
            Some("tok-admin-sec")
        );
    }

    #[tokio::test]
    async fn resolve_token_oauth_credentials_user_path() {
        let Some(db) = connect_test_database("uc_svc_resolve_tok_user").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let provider = make_provider("oauth2", false, false);
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;

        super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider.id,
            "tok-user-cid",
            Some("tok-user-sec"),
            None,
        )
        .await
        .unwrap();

        let resolved = super::resolve_token_oauth_credentials(&db, &enc, &provider, Some(&user_id))
            .await
            .unwrap();

        assert_eq!(resolved.client_id.as_str(), "tok-user-cid");
        assert_eq!(
            resolved
                .client_secret
                .as_ref()
                .map(|secret| secret.as_str()),
            Some("tok-user-sec")
        );
        assert_eq!(
            resolved.credential_user_id.as_deref(),
            Some(user_id.as_str())
        );
    }

    #[tokio::test]
    async fn resolve_token_oauth_credentials_fails_when_admin_creds_missing() {
        let Some(db) = connect_test_database("uc_svc_resolve_tok_admin_miss").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let provider = make_provider("oauth2", false, false);

        let result = super::resolve_token_oauth_credentials(&db, &enc, &provider, None).await;
        assert!(result.is_err(), "should fail without admin creds");
        assert!(matches!(
            result.unwrap_err(),
            crate::errors::AppError::BadRequest(_)
        ));
    }

    // ── CRUD round-trip integration test ─────────────────────────────

    #[tokio::test]
    async fn credentials_full_lifecycle() {
        let Some(db) = connect_test_database("uc_svc_full_lifecycle").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        seed_live_owner(&db, &user_id).await;
        let provider_id = uuid::Uuid::new_v4().to_string();

        // 1. Create
        let cred = super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "lifecycle-cid",
            Some("lifecycle-sec"),
            Some("v1"),
        )
        .await
        .unwrap();
        assert_eq!(cred.label.as_deref(), Some("v1"));

        // 2. Read
        let fetched = super::get_user_credentials(&db, &user_id, &provider_id)
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(fetched.id, cred.id);

        // 3. Update
        let updated = super::upsert_user_credentials(
            &db,
            &enc,
            &user_id,
            &provider_id,
            "lifecycle-cid-v2",
            Some("lifecycle-sec-v2"),
            Some("v2"),
        )
        .await
        .unwrap();
        assert_eq!(updated.id, cred.id);
        assert_eq!(updated.label.as_deref(), Some("v2"));

        // 4. Read metadata
        let meta = super::get_user_credentials_metadata(&db, &user_id, &provider_id)
            .await
            .unwrap()
            .expect("metadata should exist");
        assert_eq!(meta.label.as_deref(), Some("v2"));

        // 5. Delete
        super::delete_user_credentials(&db, &user_id, &provider_id)
            .await
            .unwrap();

        // 6. Verify gone
        let gone = super::get_user_credentials(&db, &user_id, &provider_id)
            .await
            .unwrap();
        assert!(gone.is_none());

        // 7. Double-delete returns NotFound
        let err = super::delete_user_credentials(&db, &user_id, &provider_id)
            .await
            .expect_err("double delete should fail");
        assert!(matches!(err, crate::errors::AppError::NotFound(_)));
    }

    // ── Existing connection-resolution tests ─────────────────────────

    #[tokio::test]
    async fn returns_none_when_no_key_matches_connection_id() {
        let Some(db) = connect_test_database("user_creds_resolve_no_match").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let enc = test_encryption_keys();
        let bogus = uuid::Uuid::new_v4().to_string();
        let resolved = resolve_connection_oauth_credentials(&db, &enc, &bogus)
            .await
            .unwrap();
        assert!(resolved.is_none(), "expected None when no key matches");
    }

    #[tokio::test]
    async fn returns_none_when_matched_key_has_no_byo_creds() {
        // A multi-connection key that doesn't carry BYO credentials
        // (e.g. codex via device_code) must NOT short-circuit
        // credential resolution. The caller falls back to the legacy
        // resolution path which reads `ProviderConfig` / `user_provider_credentials`.
        let Some(db) = connect_test_database("user_creds_resolve_no_byo").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let enc = test_encryption_keys();
        let connection_id = uuid::Uuid::new_v4().to_string();
        let key = placeholder_key(&connection_id);
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();

        let resolved = resolve_connection_oauth_credentials(&db, &enc, &connection_id)
            .await
            .unwrap();
        assert!(
            resolved.is_none(),
            "key without BYO creds must resolve to None"
        );
    }

    #[tokio::test]
    async fn returns_creds_when_matched_key_has_byo_pair() {
        let Some(db) = connect_test_database("user_creds_resolve_byo_ok").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let enc = test_encryption_keys();
        let connection_id = uuid::Uuid::new_v4().to_string();
        let mut key = placeholder_key(&connection_id);
        key.user_oauth_client_id_encrypted = Some(enc.encrypt(b"cli_marketing").await.unwrap());
        key.user_oauth_client_secret_encrypted = Some(enc.encrypt(b"super-secret").await.unwrap());

        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();

        let resolved = resolve_connection_oauth_credentials(&db, &enc, &connection_id)
            .await
            .unwrap()
            .expect("BYO creds should resolve to Some");
        assert_eq!(resolved.client_id.as_str(), "cli_marketing");
        assert_eq!(
            resolved
                .client_secret
                .as_ref()
                .map(|secret| secret.as_str()),
            Some("super-secret")
        );
        // Per §12: the multi-connection branch leaves credential_user_id None.
        assert!(resolved.credential_user_id.is_none());
        assert_eq!(
            resolved.app_source,
            crate::services::oauth_app_source::OAuthAppSource::Byo
        );
    }
}
