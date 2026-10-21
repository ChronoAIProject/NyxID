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

/// Captured producer identity is service-owned, never persisted as ciphertext
/// metadata. Composite OAuth projections retain the original provider record
/// and auxiliary record so every field authenticates its actual source context.
#[derive(Clone)]
pub struct PlatformCredentialSnapshot {
    projected: PlatformCredential,
    oauth_source: Option<crate::models::provider_config::ProviderConfig>,
    auxiliary: Option<PlatformCredential>,
}
impl std::ops::Deref for PlatformCredentialSnapshot {
    type Target = PlatformCredential;
    fn deref(&self) -> &Self::Target {
        &self.projected
    }
}

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
) -> AppResult<Option<PlatformCredentialSnapshot>> {
    if let PlatformCredentialBacking::ProviderOAuth { provider_slug } = descriptor.backing {
        let providers = db.collection::<crate::models::provider_config::ProviderConfig>(
            crate::models::provider_config::COLLECTION_NAME,
        );
        let (row, auxiliary) = if has_auxiliary_fields(descriptor) {
            // Keep the snapshot pinned between reads even with no retained history window.
            let mut session = db.client().start_session().await?;
            session
                .start_transaction()
                .read_concern(mongodb::options::ReadConcern::snapshot())
                .and_run(
                    (
                        providers.clone(),
                        db.collection::<PlatformCredential>(COLLECTION_NAME),
                        provider_slug,
                        descriptor.provider,
                    ),
                    |session, (providers, auxiliary, slug, provider)| {
                        Box::pin(async move {
                            let row = providers
                                .find_one(doc! { "slug": *slug })
                                .session(&mut *session)
                                .await?;
                            let auxiliary = auxiliary
                                .find_one(doc! { "provider": *provider })
                                .session(&mut *session)
                                .await?;
                            Ok((row, auxiliary))
                        })
                    },
                )
                .await?
        } else {
            (
                providers.find_one(doc! { "slug": provider_slug }).await?,
                None,
            )
        };
        let oauth_source = row.clone();
        let auxiliary_source = auxiliary.clone();
        let mut projected = row.map(|row| PlatformCredential {
            secret_context_versions: Default::default(),
            channel_observation_version: 0,
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
        return Ok(projected.map(|projected| PlatformCredentialSnapshot {
            projected,
            oauth_source,
            auxiliary: auxiliary_source,
        }));
    }
    Ok(db
        .collection::<PlatformCredential>(COLLECTION_NAME)
        .find_one(doc! { "provider": descriptor.provider })
        .await?
        .map(|row| PlatformCredentialSnapshot {
            projected: row,
            oauth_source: None,
            auxiliary: None,
        }))
}

pub fn configured(
    row: Option<&PlatformCredentialSnapshot>,
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
    decrypt_snapshot(keys, load(db, descriptor).await?.as_ref()).await
}

pub(crate) async fn decrypt_snapshot(
    keys: &EncryptionKeys,
    row: Option<&PlatformCredentialSnapshot>,
) -> AppResult<PlatformVerifySecrets> {
    let Some(row) = row else {
        return Ok(PlatformVerifySecrets::default());
    };
    let Some(provider) = &row.oauth_source else {
        return decrypt_platform_snapshot(keys, Some(&row.projected)).await;
    };
    let mut result = decrypt_platform_snapshot(keys, row.auxiliary.as_ref()).await?;
    for (name, field, encrypted) in [
        (
            "client_id",
            "client_id_encrypted",
            provider.client_id_encrypted.as_deref(),
        ),
        (
            "client_secret",
            "client_secret_encrypted",
            provider.client_secret_encrypted.as_deref(),
        ),
    ] {
        if let Some(encrypted) = encrypted {
            let mut value = super::credential_record_service::CredentialRecord::provider(provider)
                .text(keys, field, encrypted)
                .await?;
            result.insert(name, std::mem::take(&mut *value));
        }
    }
    Ok(result)
}

// Plain metadata must never provide or mask a descriptor's secret input.
// Apply the same contract to execution, readiness and migration, including
// duplicates which would otherwise be hidden by the encrypted map merge.
fn validate_plain_fields(row: &PlatformCredential) -> AppResult<()> {
    static SECRET_FIELDS: std::sync::LazyLock<BTreeMap<&'static str, Vec<&'static str>>> =
        std::sync::LazyLock::new(|| {
            descriptors(&Arc::new(TokenExchangeCache::new()))
                .into_iter()
                .map(|(_, descriptor)| {
                    (
                        descriptor.provider,
                        descriptor
                            .fields
                            .iter()
                            .filter(|field| field.secret)
                            .map(|field| field.name)
                            .collect(),
                    )
                })
                .collect()
        });
    let descriptor_provider = if row.provider.starts_with("telegram-new/") {
        "telegram-new"
    } else {
        row.provider.as_str()
    };
    if row.fields.keys().any(|name| {
        name == VERIFY_TOKEN_FIELD
            || row.secrets.contains_key(name)
            || SECRET_FIELDS
                .get(descriptor_provider)
                .is_some_and(|fields| fields.contains(&name.as_str()))
    }) {
        return Err(AppError::Internal(
            "Platform secrets require encrypted storage".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn decrypt_platform_snapshot(
    keys: &EncryptionKeys,
    row: Option<&PlatformCredential>,
) -> AppResult<PlatformVerifySecrets> {
    let mut result = PlatformVerifySecrets::default();
    if let Some(row) = row {
        validate_plain_fields(row)?;
        if row
            .secret_context_versions
            .iter()
            .any(|(field, version)| *version != 3 || !row.secrets.contains_key(field))
        {
            return Err(AppError::Internal(
                "Invalid platform credential storage version".into(),
            ));
        }
        for (name, value) in &row.fields {
            result.insert(name, value.clone());
        }
        for (name, encrypted) in &row.secrets {
            let bytes = decrypt_platform_field(keys, row, name, &encrypted.bytes).await?;
            let value = std::str::from_utf8(&bytes)
                .map_err(|_| AppError::Internal("Invalid platform credential encoding".into()))?;
            result.insert(name, value.to_owned());
        }
    }
    Ok(result)
}
async fn decrypt_platform_field(
    keys: &EncryptionKeys,
    row: &PlatformCredential,
    name: &str,
    encrypted: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    match row.secret_context_versions.get(name) {
        Some(3) => {
            keys.decrypt_record(
                encrypted,
                &crate::crypto::aes::RecordContext::new(
                    COLLECTION_NAME,
                    &row.id,
                    super::credential_record_service::PLATFORM_OWNER,
                    &["secrets", name],
                )?,
            )
            .await
        }
        None if keys.allow_legacy_record_reads() => {
            keys.decrypt(encrypted).await.map(Zeroizing::new)
        }
        _ => Err(AppError::Internal(
            "Invalid platform credential storage version".into(),
        )),
    }
}
pub(crate) async fn encrypt_platform_field(
    keys: &EncryptionKeys,
    id: &str,
    name: &str,
    value: &[u8],
) -> AppResult<Vec<u8>> {
    if keys.contextual_record_writes() {
        keys.encrypt_record(
            value,
            &crate::crypto::aes::RecordContext::new(
                COLLECTION_NAME,
                id,
                super::credential_record_service::PLATFORM_OWNER,
                &["secrets", name],
            )?,
        )
        .await
    } else {
        keys.encrypt(value).await
    }
}

/// Inspect the actual storage row, rather than a composite OAuth projection.
/// OAuth-backed descriptors can read fields from two collections; rewrapping a
/// projection would assign its provider fields to the wrong record identity.
pub(crate) async fn verify_current_storage_snapshot(
    keys: &EncryptionKeys,
    source: &bson::Document,
) -> AppResult<()> {
    let row: PlatformCredential = bson::from_document(source.clone())
        .map_err(|_| AppError::Internal("Invalid platform credential storage".into()))?;
    validate_plain_fields(&row)?;
    if row.secret_context_versions.len() != row.secrets.len()
        || row
            .secret_context_versions
            .iter()
            .any(|(field, version)| *version != 3 || !row.secrets.contains_key(field))
    {
        return Err(AppError::Internal(
            "Invalid platform credential storage version".into(),
        ));
    }
    for (name, encrypted) in &row.secrets {
        let components = ["secrets", name];
        let context = crate::crypto::aes::RecordContext::new(
            COLLECTION_NAME,
            &row.id,
            super::credential_record_service::PLATFORM_OWNER,
            &components,
        )?;
        keys.authenticate_current_record(&encrypted.bytes, &context)
            .await?;
        let plain = decrypt_platform_field(keys, &row, name, &encrypted.bytes).await?;
        let value = std::str::from_utf8(&plain)
            .map_err(|_| AppError::Internal("Invalid platform credential encoding".into()))?;
        if value.is_empty() || value.len() > 4096 {
            return Err(AppError::Internal(
                "Invalid platform credential value".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) async fn migrate_storage_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &bson::Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    if verify_current_storage_snapshot(keys, source).await.is_ok() {
        return Ok(false);
    }
    let original: PlatformCredential = bson::from_document(source.clone())
        .map_err(|_| AppError::Internal("Invalid platform credential storage".into()))?;
    validate_plain_fields(&original)?;
    let mut row = original.clone();
    if row
        .secret_context_versions
        .iter()
        .any(|(field, version)| *version != 3 || !row.secrets.contains_key(field))
    {
        return Err(AppError::Internal(
            "Invalid platform credential storage version".into(),
        ));
    }
    for (name, encrypted) in &mut row.secrets {
        // Use the unmodified source for version dispatch throughout the row.
        let plain = decrypt_platform_field(keys, &original, name, &encrypted.bytes).await?;
        encrypted.bytes = encrypt_platform_field(keys, &row.id, name, &plain).await?;
        row.secret_context_versions.insert(name.clone(), 3);
    }
    let mut candidate = source.clone();
    candidate.insert(
        "secrets",
        bson::to_bson(&row.secrets)
            .map_err(|_| AppError::Internal("Invalid platform credential storage".into()))?,
    );
    candidate.insert(
        "secret_context_versions",
        bson::to_bson(&row.secret_context_versions)
            .map_err(|_| AppError::Internal("Invalid platform credential storage".into()))?,
    );
    verify_current_storage_snapshot(keys, &candidate).await?;
    Ok(db
        .collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            doc! {"$set":{
                "secrets":candidate.get("secrets").cloned(),
                "secret_context_versions":candidate.get("secret_context_versions").cloned()
            }},
        )
        .await?
        .matched_count
        == 1)
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
    let source = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"provider":descriptor.provider})
        .await?;
    let id = source
        .as_ref()
        .map(|row| row.get_str("_id").map(str::to_owned))
        .transpose()
        .map_err(|_| AppError::Internal("Invalid platform credential identity".into()))?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
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
                if field.secret {
                    unset.insert(format!("secret_context_versions.{name}"), "");
                }
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
                    super::credential_record_service::mark_classic_update(
                        &mut set, &mut unset, keys, name,
                    );
                    set.insert(
                        path,
                        bson::Binary {
                            subtype: bson::spec::BinarySubtype::Generic,
                            bytes: encrypt_platform_field(keys, &id, name, value.as_bytes())
                                .await?,
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
    expressions.insert("_id", doc! { "$literal": &id });
    expressions.insert("provider", doc! { "$literal": descriptor.provider });
    let has_verify_token = source
        .as_ref()
        .and_then(|row| row.get_document("secrets").ok())
        .is_some_and(|secrets| secrets.contains_key(VERIFY_TOKEN_FIELD));
    if regenerate_verify_token || (needs_token && !has_verify_token) {
        let token = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
        let encrypted = bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: encrypt_platform_field(keys, &id, VERIFY_TOKEN_FIELD, token.as_bytes()).await?,
        };
        let value = doc! { "$literal": encrypted };
        expressions.insert(&token_path, value);
        if keys.contextual_record_writes() {
            expressions.insert(
                format!("secret_context_versions.{VERIFY_TOKEN_FIELD}"),
                doc! {"$literal":3_i32},
            );
        } else {
            unset.insert(format!("secret_context_versions.{VERIFY_TOKEN_FIELD}"), "");
        }
    }
    let mut update = vec![doc! { "$set": expressions }];
    if !unset.is_empty() {
        update.push(doc! { "$unset": unset.keys().cloned().collect::<Vec<_>>() });
    }
    let filter = match &source {
        Some(source) => super::record_storage::exact_document_predicate(source),
        None => doc! {"_id":&id,"provider":descriptor.provider},
    };
    let db = super::transaction_service::durable_database(db);
    let context = (&db, &filter, &update, source.is_none());
    super::transaction_service::run_db_only(&db, &context, |tx, context| {
        let &(db, filter, update, upsert) = context;
        Box::pin(async move {
            let result = db
                .collection::<PlatformCredential>(COLLECTION_NAME)
                .update_one(filter.clone(), update.clone())
                .upsert(upsert)
                .session(&mut *tx)
                .await?;
            if result.matched_count == 0 && result.upserted_id.is_none() {
                return Err(AppError::Conflict(
                    "Platform credentials changed; reload and retry".into(),
                ));
            }
            Ok(())
        })
    })
    .await
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
                    "$unset": { "client_id_encrypted": "", "client_secret_encrypted": "", "secret_context_versions.client_id_encrypted": "", "secret_context_versions.client_secret_encrypted": "" },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
                None,
            )
            .await?;
            return Ok(());
        }
        db.collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
            .update_one(
                doc! { "slug": &provider_slug },
                doc! {
                    "$unset": { "client_id_encrypted": "", "client_secret_encrypted": "", "secret_context_versions.client_id_encrypted": "", "secret_context_versions.client_secret_encrypted": "" },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
            )
            .await?;
        return Ok(());
    }
    let db = super::transaction_service::durable_database(db);
    super::transaction_service::run_db_only(&db, &(&db, descriptor.provider), |tx, context| {
        let &(db, provider) = context;
        Box::pin(async move {
            db.collection::<PlatformCredential>(COLLECTION_NAME)
                .delete_one(doc! {"provider": provider})
                .session(&mut *tx)
                .await?;
            Ok(())
        })
    })
    .await
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
    let provider = db
        .collection::<crate::models::provider_config::ProviderConfig>(
            crate::models::provider_config::COLLECTION_NAME,
        )
        // The descriptor selects the backing provider identity. Some composite
        // credentials (Aurinko) deliberately belong to an API-key provider.
        .find_one(doc! {"slug":provider_slug})
        .await?
        .ok_or_else(|| AppError::NotFound("OAuth provider is not configured".into()))?;
    let auxiliary = db
        .collection::<PlatformCredential>(COLLECTION_NAME)
        .find_one(doc! {"provider":descriptor.provider})
        .await?;
    let auxiliary_id = auxiliary
        .as_ref()
        .map(|row| row.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
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
                let bytes = if is_oauth_field(name) {
                    super::credential_record_service::mark_classic_update(set, unset, keys, &path);
                    super::credential_record_service::encrypt(
                        keys,
                        crate::models::provider_config::COLLECTION_NAME,
                        &provider.id,
                        super::credential_record_service::PLATFORM_OWNER,
                        &path,
                        value.as_bytes(),
                    )
                    .await?
                } else {
                    super::credential_record_service::mark_classic_update(set, unset, keys, name);
                    encrypt_platform_field(keys, &auxiliary_id, name, value.as_bytes()).await?
                };
                set.insert(
                    path,
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes,
                    },
                );
            }
            None => {
                unset.insert(
                    format!(
                        "secret_context_versions.{}",
                        if is_oauth_field(name) {
                            format!("{name}_encrypted")
                        } else {
                            name.clone()
                        }
                    ),
                    "",
                );
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
            "$setOnInsert": { "_id": &auxiliary_id, "provider": descriptor.provider },
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
            Some((&provider.id, &auxiliary_id)),
        )
        .await?
        {
            return Err(AppError::NotFound("Provider is not configured".into()));
        }
        return Ok(());
    }
    let result = db
        .collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
        .update_one(doc! { "_id":&provider.id, "slug": provider_slug }, update)
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
    expected_ids: Option<(&str, &str)>,
) -> AppResult<bool> {
    let db = super::transaction_service::durable_database(db);
    let context = (
        &db,
        provider_slug,
        provider_name,
        &update,
        &auxiliary_update,
        expected_ids,
    );
    super::transaction_service::run_db_only(&db, &context, |session, context| {
        let &(db, slug, provider, update, auxiliary, expected_ids) = context;
        Box::pin(async move {
            let mut provider_filter = doc! {"slug":slug};
            if let Some((provider_id, _)) = expected_ids {
                provider_filter.insert("_id", provider_id);
            }
            let result = db
                .collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME)
                .update_one(provider_filter, update.clone())
                .session(&mut *session)
                .await?;
            let stored = db.collection::<PlatformCredential>(COLLECTION_NAME);
            if let Some(auxiliary) = auxiliary {
                if result.matched_count != 1 {
                    return Ok(false);
                }
                let mut auxiliary_filter = doc! {"provider":provider};
                if let Some((_, auxiliary_id)) = expected_ids {
                    auxiliary_filter.insert("_id", auxiliary_id);
                }
                stored
                    .update_one(auxiliary_filter, auxiliary.clone())
                    .upsert(true)
                    .session(&mut *session)
                    .await?;
            } else {
                stored
                    .delete_one(doc! {"provider":provider})
                    .session(&mut *session)
                    .await?;
            }
            Ok(true)
        })
    })
    .await
}

crate::errors::redacted_debug!(PlatformCredentialSnapshot);

#[cfg(test)]
mod storage_tests {
    use super::*;
    #[tokio::test]
    async fn secret_descriptor_names_in_plain_fields_never_pass_execution_or_readiness() {
        let keys = crate::test_utils::test_encryption_keys();
        let mut row = PlatformCredential {
            id: uuid::Uuid::new_v4().to_string(),
            provider: "telegram-new".into(),
            fields: BTreeMap::new(),
            secrets: BTreeMap::new(),
            secret_context_versions: BTreeMap::new(),
            updated_by: uuid::Uuid::new_v4().to_string(),
            updated_at: chrono::Utc::now(),
            channel_observation_version: 0,
        };
        row.fields
            .insert("manager_bot_token".into(), "misplaced-token".into());
        row.provider = "telegram-new/200".into();
        assert!(decrypt_platform_snapshot(&keys, Some(&row)).await.is_err());
        assert!(
            verify_current_storage_snapshot(&keys, &bson::to_document(&row).unwrap())
                .await
                .is_err()
        );
        row.provider = "telegram-new".into();
        let source = bson::to_document(&row).unwrap();
        assert!(decrypt_platform_snapshot(&keys, Some(&row)).await.is_err());
        assert!(
            verify_current_storage_snapshot(&keys, &source)
                .await
                .is_err()
        );
        row.secrets.insert(
            "manager_bot_token".into(),
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: encrypt_platform_field(
                    &keys,
                    &row.id,
                    "manager_bot_token",
                    b"encrypted-token",
                )
                .await
                .unwrap(),
            },
        );
        row.secret_context_versions
            .insert("manager_bot_token".into(), 3);
        assert!(decrypt_platform_snapshot(&keys, Some(&row)).await.is_err());
        assert!(
            verify_current_storage_snapshot(&keys, &bson::to_document(&row).unwrap())
                .await
                .is_err()
        );
        row.fields.clear();
        verify_current_storage_snapshot(&keys, &bson::to_document(&row).unwrap())
            .await
            .unwrap();
        assert_eq!(
            decrypt_platform_snapshot(&keys, Some(&row))
                .await
                .unwrap()
                .get("manager_bot_token"),
            Some("encrypted-token")
        );
    }
}
