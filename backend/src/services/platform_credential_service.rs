use std::collections::BTreeMap;
use std::sync::Arc;

use bson::doc;
use zeroize::Zeroizing;

use super::channel_managed::{PlatformCredentialBacking, PlatformCredentialDescriptor};
use super::channel_platform::PlatformVerifySecrets;
use super::provider_token_exchange_service::TokenExchangeCache;
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::platform_credential::{COLLECTION_NAME, PlatformCredential};

pub const VERIFY_TOKEN_FIELD: &str = "webhook_verify_token";

pub fn descriptors(cache: &Arc<TokenExchangeCache>) -> Vec<(String, PlatformCredentialDescriptor)> {
    let mut providers = BTreeMap::new();
    for adapter in super::channel_adapters::registered_adapters(cache) {
        if let Some(descriptor) = adapter.platform_credentials() {
            providers
                .entry(descriptor.provider)
                .or_insert_with(|| (adapter.platform_id().to_string(), descriptor));
        }
    }
    providers.into_values().collect()
}

pub fn descriptor(
    cache: &Arc<TokenExchangeCache>,
    provider: &str,
) -> AppResult<(String, PlatformCredentialDescriptor)> {
    descriptors(cache)
        .into_iter()
        .find(|(_, d)| d.provider == provider)
        .ok_or_else(|| AppError::NotFound("Unknown platform credential provider".to_string()))
}

pub async fn load(db: &mongodb::Database, provider: &str) -> AppResult<Option<PlatformCredential>> {
    let (_, descriptor) = descriptor(&Arc::new(TokenExchangeCache::new()), provider)?;
    if let PlatformCredentialBacking::ProviderOAuth { provider_slug } = descriptor.backing {
        let row = db
            .collection::<crate::models::provider_config::ProviderConfig>(
                crate::models::provider_config::COLLECTION_NAME,
            )
            .find_one(doc! { "slug": provider_slug })
            .await?;
        return Ok(row.map(|row| PlatformCredential {
            id: row.id,
            provider: provider.to_string(),
            fields: BTreeMap::new(),
            secrets: [
                ("client_id", row.client_id_encrypted),
                ("client_secret", row.client_secret_encrypted),
            ]
            .into_iter()
            .filter_map(|(name, bytes)| {
                bytes.map(|bytes| {
                    (
                        name.to_string(),
                        bson::Binary {
                            subtype: bson::spec::BinarySubtype::Generic,
                            bytes,
                        },
                    )
                })
            })
            .collect(),
            updated_by: row.created_by,
            updated_at: row.updated_at,
        }));
    }
    Ok(db
        .collection::<PlatformCredential>(COLLECTION_NAME)
        .find_one(doc! { "provider": provider })
        .await?)
}

pub fn configured(
    row: Option<&PlatformCredential>,
    descriptor: &PlatformCredentialDescriptor,
) -> bool {
    descriptor
        .fields
        .iter()
        .filter(|field| field.required)
        .all(|field| {
            row.is_some_and(|row| {
                if field.secret {
                    row.secrets.contains_key(field.name)
                } else {
                    row.fields.contains_key(field.name)
                }
            })
        })
}

pub async fn load_decrypted(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    provider: &str,
) -> AppResult<PlatformVerifySecrets> {
    let mut result = PlatformVerifySecrets::default();
    if let Some(row) = load(db, provider).await? {
        for (name, value) in row.fields {
            result.insert(&name, value);
        }
        for (name, encrypted) in row.secrets {
            let bytes = Zeroizing::new(keys.decrypt(&encrypted.bytes).await?);
            let value = std::str::from_utf8(&bytes).map_err(|_| {
                AppError::Internal("Invalid platform credential encoding".to_string())
            })?;
            result.insert(&name, value.to_string());
        }
    }
    Ok(result)
}

pub async fn update(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    descriptor: &PlatformCredentialDescriptor,
    actor: &str,
    fields: &BTreeMap<String, Option<Zeroizing<String>>>,
    regenerate_verify_token: bool,
) -> AppResult<()> {
    if let PlatformCredentialBacking::ProviderOAuth { provider_slug } = descriptor.backing {
        return update_provider_oauth(
            db,
            keys,
            descriptor,
            provider_slug,
            fields,
            regenerate_verify_token,
        )
        .await;
    }
    let mut set = doc! { "updated_by": actor, "updated_at": bson::DateTime::now() };
    let mut unset = doc! {};
    for (name, value) in fields {
        let field = descriptor
            .fields
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| {
                AppError::ValidationError("Unknown platform credential field".to_string())
            })?;
        let path = format!(
            "{}.{}",
            if field.secret { "secrets" } else { "fields" },
            name
        );
        match value {
            None => {
                unset.insert(path, "");
            }
            Some(value) => {
                let value = value.trim();
                if value.is_empty()
                    || value.len() > 4096
                    || (field.numeric
                        && (value.len() > 32 || !value.bytes().all(|b| b.is_ascii_digit())))
                {
                    return Err(AppError::ValidationError(format!(
                        "Invalid {}",
                        field.label
                    )));
                }
                if field.secret {
                    set.insert(
                        path,
                        bson::Binary {
                            subtype: bson::spec::BinarySubtype::Generic,
                            bytes: keys.encrypt(value.as_bytes()).await?,
                        },
                    );
                } else {
                    set.insert(path, value);
                }
            }
        }
    }
    let token_path = format!("secrets.{VERIFY_TOKEN_FIELD}");
    let needs_token = descriptor
        .webhook_secret_field
        .is_some_and(|field| fields.get(field).is_some_and(Option::is_some));
    if regenerate_verify_token && descriptor.webhook_secret_field.is_none() {
        return Err(AppError::ValidationError(
            "Platform webhooks are not supported".to_string(),
        ));
    }
    // A pipeline atomically preserves an existing verify token during rotation.
    // Every supplied value is literal, including values beginning with '$'.
    let mut expressions: bson::Document = set
        .into_iter()
        .map(|(name, value)| (name, bson::Bson::Document(doc! { "$literal": value })))
        .collect();
    expressions.insert(
        "_id",
        doc! { "$ifNull": ["$_id", { "$literal": uuid::Uuid::new_v4().to_string() }] },
    );
    expressions.insert("provider", doc! { "$literal": descriptor.provider });
    if needs_token || regenerate_verify_token {
        let token = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
        let encrypted = bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: keys.encrypt(token.as_bytes()).await?,
        };
        let value = doc! { "$literal": encrypted };
        expressions.insert(
            &token_path,
            if regenerate_verify_token {
                value
            } else {
                doc! { "$ifNull": [format!("${token_path}"), value] }
            },
        );
    }
    let mut update = vec![doc! { "$set": expressions }];
    if !unset.is_empty() {
        update.push(doc! { "$unset": unset.keys().cloned().collect::<Vec<_>>() });
    }
    db.collection::<PlatformCredential>(COLLECTION_NAME)
        .update_one(doc! { "provider": descriptor.provider }, update)
        .upsert(true)
        .await?;
    Ok(())
}

pub async fn delete(db: &mongodb::Database, provider: &str) -> AppResult<()> {
    let (_, descriptor) = descriptor(&Arc::new(TokenExchangeCache::new()), provider)?;
    if let PlatformCredentialBacking::ProviderOAuth { provider_slug } = descriptor.backing {
        db.collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
            .update_one(
                doc! { "slug": provider_slug },
                doc! {
                    "$unset": { "client_id_encrypted": "", "client_secret_encrypted": "" },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
            )
            .await?;
        return Ok(());
    }
    db.collection::<PlatformCredential>(COLLECTION_NAME)
        .delete_one(doc! { "provider": provider })
        .await?;
    Ok(())
}

async fn update_provider_oauth(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    descriptor: &PlatformCredentialDescriptor,
    provider_slug: &str,
    fields: &BTreeMap<String, Option<Zeroizing<String>>>,
    regenerate_verify_token: bool,
) -> AppResult<()> {
    if regenerate_verify_token {
        return Err(AppError::ValidationError(
            "Platform webhooks are not supported".to_string(),
        ));
    }
    let mut set = doc! { "updated_at": bson::DateTime::now() };
    let mut unset = doc! {};
    for (name, value) in fields {
        if !matches!(name.as_str(), "client_id" | "client_secret")
            || !descriptor.fields.iter().any(|field| field.name == name)
        {
            return Err(AppError::ValidationError(
                "Unknown platform credential field".to_string(),
            ));
        }
        let path = format!("{name}_encrypted");
        match value {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() || value.len() > 4096 {
                    return Err(AppError::ValidationError(format!("Invalid {name}")));
                }
                set.insert(
                    path,
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: keys.encrypt(value.as_bytes()).await?,
                    },
                );
            }
            None => {
                unset.insert(path, "");
            }
        }
    }
    let mut update = doc! { "$set": set };
    if !unset.is_empty() {
        update.insert("$unset", unset);
    }
    let result = db
        .collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
        .update_one(
            doc! { "slug": provider_slug, "provider_type": "oauth2" },
            update,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "OAuth provider is not configured".to_string(),
        ));
    }
    Ok(())
}
