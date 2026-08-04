use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::provider_config::ProviderConfig;
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
    let collection = db.collection::<UserProviderCredentials>(COLLECTION_NAME);
    let now = Utc::now();

    let client_id_enc = encryption_keys.encrypt(client_id.as_bytes()).await?;
    let client_secret_enc = match client_secret {
        Some(s) => Some(encryption_keys.encrypt(s.as_bytes()).await?),
        None => None,
    };

    let existing = collection
        .find_one(doc! { "user_id": user_id, "provider_config_id": provider_config_id })
        .await?;

    if let Some(existing) = existing {
        let mut set_doc = doc! {
            "client_id_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: client_id_enc,
            },
            "updated_at": bson::DateTime::from_chrono(now),
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
            set_doc.insert("label", l);
        }

        use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

        let updated = collection
            .find_one_and_update(doc! { "_id": &existing.id }, doc! { "$set": set_doc })
            .with_options(
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?
            .ok_or_else(|| {
                AppError::Internal("Credential disappeared during update".to_string())
            })?;

        tracing::info!(
            user_id = %user_id,
            provider_config_id = %provider_config_id,
            "User provider credentials updated"
        );

        Ok(updated)
    } else {
        let cred = UserProviderCredentials {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            provider_config_id: provider_config_id.to_string(),
            client_id_encrypted: Some(client_id_enc),
            client_secret_encrypted: client_secret_enc,
            label: label.map(String::from),
            created_at: now,
            updated_at: now,
        };

        collection.insert_one(&cred).await?;

        tracing::info!(
            user_id = %user_id,
            provider_config_id = %provider_config_id,
            "User provider credentials created"
        );

        Ok(cred)
    }
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
    }))
}

/// Delete user credentials for a provider.
pub async fn delete_user_credentials(
    db: &mongodb::Database,
    user_id: &str,
    provider_config_id: &str,
) -> AppResult<()> {
    let result = db
        .collection::<UserProviderCredentials>(COLLECTION_NAME)
        .delete_one(doc! { "user_id": user_id, "provider_config_id": provider_config_id })
        .await?;

    if result.deleted_count == 0 {
        return Err(AppError::NotFound("User credentials not found".to_string()));
    }

    tracing::info!(
        user_id = %user_id,
        provider_config_id = %provider_config_id,
        "User provider credentials deleted"
    );

    Ok(())
}

/// Resolved OAuth client credentials (decrypted).
#[derive(Debug)]
pub struct ResolvedOAuthCredentials {
    pub client_id: String,
    pub client_secret: Option<String>,
    /// `Some(user_id)` when user-provided OAuth app credentials were used.
    /// `None` means provider-level credentials were used.
    pub credential_user_id: Option<String>,
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
/// Note: `Zeroizing` is best-effort here — the `String::from_utf8` clone means the
/// plaintext remains in memory until deallocated. Acceptable for our threat model
/// (encrypted at rest, decrypted in-memory only when needed).
async fn decrypt_provider_credentials(
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
) -> AppResult<ResolvedOAuthCredentials> {
    let encrypted_cid = provider.client_id_encrypted.as_ref().ok_or_else(|| {
        AppError::Internal(format!("Provider {} missing client_id", provider.slug))
    })?;

    let decrypted_cid = Zeroizing::new(encryption_keys.decrypt(encrypted_cid).await?);
    let client_id = String::from_utf8((*decrypted_cid).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode client_id: {e}")))?;

    let client_secret = if let Some(ref encrypted) = provider.client_secret_encrypted {
        let decrypted = Zeroizing::new(encryption_keys.decrypt(encrypted).await?);
        Some(
            String::from_utf8((*decrypted).clone())
                .map_err(|e| AppError::Internal(format!("Failed to decode client_secret: {e}")))?,
        )
    } else {
        None
    };

    Ok(ResolvedOAuthCredentials {
        client_id,
        client_secret,
        credential_user_id: None,
    })
}

/// Decrypt user-level OAuth credentials.
///
/// Note: `Zeroizing` is best-effort — see `decrypt_provider_credentials` doc comment.
async fn decrypt_user_credentials(
    encryption_keys: &EncryptionKeys,
    creds: &UserProviderCredentials,
    credential_user_id: &str,
) -> AppResult<ResolvedOAuthCredentials> {
    let encrypted_cid = creds
        .client_id_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("User credentials missing client_id".to_string()))?;

    let decrypted_cid = Zeroizing::new(encryption_keys.decrypt(encrypted_cid).await?);
    let client_id = String::from_utf8((*decrypted_cid).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode user client_id: {e}")))?;

    let client_secret =
        if let Some(ref encrypted) = creds.client_secret_encrypted {
            let decrypted = Zeroizing::new(encryption_keys.decrypt(encrypted).await?);
            Some(String::from_utf8((*decrypted).clone()).map_err(|e| {
                AppError::Internal(format!("Failed to decode user client_secret: {e}"))
            })?)
        } else {
            None
        };

    Ok(ResolvedOAuthCredentials {
        client_id,
        client_secret,
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
pub async fn resolve_connection_oauth_credentials(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    connection_id: &str,
) -> AppResult<Option<ResolvedOAuthCredentials>> {
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use mongodb::bson::doc;

    let key = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! { "connection_id": connection_id })
        .await?;
    let Some(key) = key else {
        return Ok(None);
    };
    let Some(enc_cid) = key.user_oauth_client_id_encrypted.as_ref() else {
        return Ok(None);
    };

    let dec_cid = Zeroizing::new(encryption_keys.decrypt(enc_cid).await?);
    let client_id = String::from_utf8((*dec_cid).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode connection client_id: {e}")))?;

    let client_secret = if let Some(enc_sec) = key.user_oauth_client_secret_encrypted.as_ref() {
        let dec_sec = Zeroizing::new(encryption_keys.decrypt(enc_sec).await?);
        Some(String::from_utf8((*dec_sec).clone()).map_err(|e| {
            AppError::Internal(format!("Failed to decode connection client_secret: {e}"))
        })?)
    } else {
        None
    };

    Ok(Some(ResolvedOAuthCredentials {
        client_id,
        client_secret,
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
    use chrono::Utc;

    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::test_utils::{connect_test_database, test_encryption_keys};

    use super::resolve_connection_oauth_credentials;

    fn placeholder_key(connection_id: &str) -> UserApiKey {
        UserApiKey {
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
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "pending_auth".to_string(),
            last_used_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
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
            id: uuid::Uuid::new_v4().to_string(),
            slug: "test-provider".to_string(),
            name: "Test Provider".to_string(),
            description: None,
            provider_type: provider_type.to_string(),
            authorization_url: None,
            token_url: None,
            revocation_url: None,
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
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        }
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
    async fn upsert_user_credentials_without_secret() {
        let Some(db) = connect_test_database("uc_svc_upsert_no_secret").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let enc = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
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
        let resolved = super::resolve_oauth_credentials(&db, &enc, &provider, &user_id)
            .await
            .unwrap();

        assert_eq!(resolved.client_id, "admin-client-id");
        assert_eq!(resolved.client_secret.as_deref(), Some("admin-secret"));
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

        assert_eq!(resolved.client_id, "user-cid");
        assert_eq!(resolved.client_secret.as_deref(), Some("user-secret"));
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

        assert_eq!(resolved.client_id, "user-cid-both");
        assert_eq!(resolved.client_secret.as_deref(), Some("user-sec-both"));
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
        // No user credentials stored, should fall back to admin

        let resolved = super::resolve_oauth_credentials(&db, &enc, &provider, &user_id)
            .await
            .unwrap();

        assert_eq!(resolved.client_id, "admin-fallback-cid");
        assert_eq!(
            resolved.client_secret.as_deref(),
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

        assert_eq!(resolved.client_id, "tok-admin-cid");
        assert_eq!(resolved.client_secret.as_deref(), Some("tok-admin-sec"));
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

        assert_eq!(resolved.client_id, "tok-user-cid");
        assert_eq!(resolved.client_secret.as_deref(), Some("tok-user-sec"));
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
        assert_eq!(resolved.client_id, "cli_marketing");
        assert_eq!(resolved.client_secret.as_deref(), Some("super-secret"));
        // Per §12: the multi-connection branch leaves credential_user_id None.
        assert!(resolved.credential_user_id.is_none());
    }
}
