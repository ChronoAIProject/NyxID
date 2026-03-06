use chrono::Utc;
use mongodb::bson::{self, doc};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::provider_config::ProviderConfig;
use crate::models::user_provider_credentials::{COLLECTION_NAME, UserProviderCredentials};

/// Upsert per-user OAuth app credentials for a provider.
///
/// If credentials already exist for this (user, provider) pair, they are updated.
/// Otherwise, a new record is inserted.
pub async fn upsert_user_credentials(
    db: &mongodb::Database,
    encryption_key: &[u8],
    user_id: &str,
    provider_config_id: &str,
    client_id: &str,
    client_secret: Option<&str>,
    label: Option<&str>,
) -> AppResult<UserProviderCredentials> {
    let collection = db.collection::<UserProviderCredentials>(COLLECTION_NAME);
    let now = Utc::now();

    let client_id_enc = aes::encrypt(client_id.as_bytes(), encryption_key)?;
    let client_secret_enc = match client_secret {
        Some(s) => Some(aes::encrypt(s.as_bytes(), encryption_key)?),
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
            provider.client_id_encrypted.is_some() && provider.client_secret_encrypted.is_some()
        }
        "device_code" => provider.client_id_encrypted.is_some(),
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
    encryption_key: &[u8],
    provider: &ProviderConfig,
    user_id: &str,
) -> AppResult<ResolvedOAuthCredentials> {
    let mode = &provider.credential_mode;

    match mode.as_str() {
        "admin" => decrypt_provider_credentials(encryption_key, provider),
        "user" => {
            resolve_user_credentials_for_owner(
                db,
                encryption_key,
                provider,
                user_id,
                "This provider requires you to configure your own OAuth app credentials",
            )
            .await
        }
        "both" => {
            let user_creds = get_user_credentials(db, user_id, &provider.id).await?;
            if let Some(creds) = user_creds {
                decrypt_user_credentials(encryption_key, &creds, user_id)
            } else if provider_has_admin_oauth_credentials(provider) {
                decrypt_provider_credentials(encryption_key, provider)
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
            decrypt_provider_credentials(encryption_key, provider)
        }
    }
}

/// Resolve the exact OAuth client credentials previously used to mint a token.
///
/// This bypasses the provider's current `credential_mode` so refreshes keep using
/// the same OAuth client that issued the refresh token.
pub async fn resolve_token_oauth_credentials(
    db: &mongodb::Database,
    encryption_key: &[u8],
    provider: &ProviderConfig,
    credential_user_id: Option<&str>,
) -> AppResult<ResolvedOAuthCredentials> {
    match credential_user_id {
        Some(user_id) => {
            resolve_user_credentials_for_owner(
                db,
                encryption_key,
                provider,
                user_id,
                "The OAuth app credentials used for this connection are no longer available. Reconnect after configuring them again.",
            )
            .await
        }
        None => {
            if provider_has_admin_oauth_credentials(provider) {
                decrypt_provider_credentials(encryption_key, provider)
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
    encryption_key: &[u8],
    provider: &ProviderConfig,
    user_id: &str,
    missing_message: &str,
) -> AppResult<ResolvedOAuthCredentials> {
    let user_creds = get_user_credentials(db, user_id, &provider.id).await?;
    match user_creds {
        Some(creds) => decrypt_user_credentials(encryption_key, &creds, user_id),
        None => Err(AppError::BadRequest(missing_message.to_string())),
    }
}

/// Decrypt provider-level (admin) OAuth credentials.
///
/// Note: `Zeroizing` is best-effort here — the `String::from_utf8` clone means the
/// plaintext remains in memory until deallocated. Acceptable for our threat model
/// (encrypted at rest, decrypted in-memory only when needed).
fn decrypt_provider_credentials(
    encryption_key: &[u8],
    provider: &ProviderConfig,
) -> AppResult<ResolvedOAuthCredentials> {
    let encrypted_cid = provider.client_id_encrypted.as_ref().ok_or_else(|| {
        AppError::Internal(format!("Provider {} missing client_id", provider.slug))
    })?;

    let decrypted_cid = Zeroizing::new(aes::decrypt(encrypted_cid, encryption_key)?);
    let client_id = String::from_utf8((*decrypted_cid).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode client_id: {e}")))?;

    let client_secret = if let Some(ref encrypted) = provider.client_secret_encrypted {
        let decrypted = Zeroizing::new(aes::decrypt(encrypted, encryption_key)?);
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
fn decrypt_user_credentials(
    encryption_key: &[u8],
    creds: &UserProviderCredentials,
    credential_user_id: &str,
) -> AppResult<ResolvedOAuthCredentials> {
    let encrypted_cid = creds
        .client_id_encrypted
        .as_ref()
        .ok_or_else(|| AppError::Internal("User credentials missing client_id".to_string()))?;

    let decrypted_cid = Zeroizing::new(aes::decrypt(encrypted_cid, encryption_key)?);
    let client_id = String::from_utf8((*decrypted_cid).clone())
        .map_err(|e| AppError::Internal(format!("Failed to decode user client_id: {e}")))?;

    let client_secret =
        if let Some(ref encrypted) = creds.client_secret_encrypted {
            let decrypted = Zeroizing::new(aes::decrypt(encrypted, encryption_key)?);
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
