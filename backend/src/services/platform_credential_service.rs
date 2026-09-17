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

pub async fn load(
    db: &mongodb::Database,
    descriptor: &PlatformCredentialDescriptor,
) -> AppResult<Option<PlatformCredential>> {
    if let PlatformCredentialBacking::ProviderOAuth { provider_slug } = descriptor.backing {
        let providers = db.collection::<crate::models::provider_config::ProviderConfig>(
            crate::models::provider_config::COLLECTION_NAME,
        );
        let (row, auxiliary) = if has_auxiliary_fields(descriptor) {
            // A writer transaction alone cannot prevent readers mixing two rotations.
            let mut session = db.client().start_session().snapshot(true).await?;
            let row = providers
                .find_one(doc! { "slug": provider_slug })
                .session(&mut session)
                .await?;
            let auxiliary = db
                .collection::<PlatformCredential>(COLLECTION_NAME)
                .find_one(doc! { "provider": descriptor.provider })
                .session(&mut session)
                .await?;
            (row, auxiliary)
        } else {
            (
                providers.find_one(doc! { "slug": provider_slug }).await?,
                None,
            )
        };
        let mut projected = row.map(|row| PlatformCredential {
            id: row.id,
            provider: descriptor.provider.to_string(),
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
        });
        if let Some(stored) = auxiliary
            && let Some(projected) = projected.as_mut()
        {
            for field in descriptor
                .fields
                .iter()
                .filter(|field| !is_oauth_field(field.name))
            {
                if let Some(secret) = stored.secrets.get(field.name) {
                    projected.secrets.insert(field.name.into(), secret.clone());
                }
            }
            if stored.updated_at >= projected.updated_at {
                projected.updated_at = stored.updated_at;
                projected.updated_by = stored.updated_by;
            }
        }
        return Ok(projected);
    }
    Ok(db
        .collection::<PlatformCredential>(COLLECTION_NAME)
        .find_one(doc! { "provider": descriptor.provider })
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
    descriptor: &PlatformCredentialDescriptor,
) -> AppResult<PlatformVerifySecrets> {
    let mut result = PlatformVerifySecrets::default();
    if let Some(row) = load(db, descriptor).await? {
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
            actor,
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

pub async fn delete(
    db: &mongodb::Database,
    descriptor: &PlatformCredentialDescriptor,
) -> AppResult<()> {
    if let PlatformCredentialBacking::ProviderOAuth { provider_slug } = descriptor.backing {
        if has_auxiliary_fields(descriptor) {
            write_composite_provider(
                db,
                provider_slug,
                descriptor.provider,
                doc! {
                    "$unset": { "client_id_encrypted": "", "client_secret_encrypted": "" },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
            return Ok(());
        }
        db.collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
            .update_one(
                doc! { "slug": &provider_slug },
                doc! {
                    "$unset": { "client_id_encrypted": "", "client_secret_encrypted": "" },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
            )
            .await?;
        return Ok(());
    }
    db.collection::<PlatformCredential>(COLLECTION_NAME)
        .delete_one(doc! { "provider": descriptor.provider })
        .await?;
    Ok(())
}

async fn update_provider_oauth(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    descriptor: &PlatformCredentialDescriptor,
    provider_slug: &str,
    actor: &str,
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
    let mut auxiliary_set = doc! { "updated_by": actor, "updated_at": bson::DateTime::now() };
    let mut auxiliary_unset = doc! {};
    for (name, value) in fields {
        if !descriptor
            .fields
            .iter()
            .any(|field| field.name == name && field.secret)
        {
            return Err(AppError::ValidationError(
                "Unknown platform credential field".to_string(),
            ));
        }
        let (path, set, unset) = if is_oauth_field(name) {
            (format!("{name}_encrypted"), &mut set, &mut unset)
        } else {
            (
                format!("secrets.{name}"),
                &mut auxiliary_set,
                &mut auxiliary_unset,
            )
        };
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
    if has_auxiliary_fields(descriptor) {
        let mut auxiliary_update = doc! {
            "$set": auxiliary_set,
            "$setOnInsert": { "_id": uuid::Uuid::new_v4().to_string(), "provider": descriptor.provider },
        };
        if !auxiliary_unset.is_empty() {
            auxiliary_update.insert("$unset", auxiliary_unset);
        }
        if !write_composite_provider(
            db,
            provider_slug,
            descriptor.provider,
            update,
            Some(auxiliary_update),
        )
        .await?
        {
            return Err(AppError::NotFound("Provider is not configured".into()));
        }
        return Ok(());
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

fn is_oauth_field(name: &str) -> bool {
    matches!(name, "client_id" | "client_secret")
}

fn has_auxiliary_fields(descriptor: &PlatformCredentialDescriptor) -> bool {
    descriptor
        .fields
        .iter()
        .any(|field| !is_oauth_field(field.name))
}

async fn write_composite_provider(
    db: &mongodb::Database,
    provider_slug: &str,
    provider_name: &'static str,
    update: bson::Document,
    auxiliary_update: Option<bson::Document>,
) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    let context = (
        db.clone(),
        provider_slug.to_string(),
        provider_name,
        update,
        auxiliary_update,
    );
    Ok(session
        .start_transaction()
        .and_run(
            context,
            |session, (db, slug, provider, update, auxiliary)| {
                Box::pin(async move {
                    let result = db
                        .collection::<bson::Document>(
                            crate::models::provider_config::COLLECTION_NAME,
                        )
                        .update_one(doc! { "slug": slug.as_str() }, update.clone())
                        .session(&mut *session)
                        .await?;
                    let stored = db.collection::<PlatformCredential>(COLLECTION_NAME);
                    if let Some(auxiliary) = auxiliary {
                        if result.matched_count != 1 {
                            return Ok(false);
                        }
                        stored
                            .update_one(doc! { "provider": *provider }, auxiliary.clone())
                            .upsert(true)
                            .session(&mut *session)
                            .await?;
                    } else {
                        stored
                            .delete_one(doc! { "provider": *provider })
                            .session(&mut *session)
                            .await?;
                    }
                    Ok(true)
                })
            },
        )
        .await?)
}
