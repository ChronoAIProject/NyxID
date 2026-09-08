//! Channel bot lifecycle management service.
//!
//! Handles bot registration, webhook setup, token encryption/decryption,
//! listing, deletion, and platform-specific bot lookup.

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use sha2::{Digest, Sha256};

use crate::config::AppConfig;
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{COLLECTION_NAME, ChannelBot};
use crate::models::channel_conversation::COLLECTION_NAME as CONVERSATIONS;
use crate::services::channel_platform::{
    BotCredentials, BotIdentity, PlatformAdapter, RegistrationDescriptor, RegistrationValues,
};

/// Result of creating a bot: the persisted record plus the raw webhook secret
/// (shown once, never stored in cleartext).
pub struct CreateBotResult {
    pub bot: ChannelBot,
    pub webhook_secret: String,
}

#[derive(Clone, Copy)]
pub enum SecretPatch<'a> {
    Unchanged,
    Clear,
    Set(&'a str),
}

pub struct UpdateBotParams<'a> {
    pub bot_token: Option<&'a str>,
    pub label: Option<&'a str>,
    pub verification_token: Option<&'a str>,
    pub encrypt_key: SecretPatch<'a>,
    pub app_id: Option<&'a str>,
    pub app_secret: Option<&'a str>,
}

impl<'a> UpdateBotParams<'a> {
    pub fn fields(&self) -> RegistrationValues<'a> {
        RegistrationValues(
            [
                ("bot_token", self.bot_token),
                ("app_id", self.app_id),
                ("app_secret", self.app_secret),
                ("verification_token", self.verification_token),
                (
                    "encrypt_key",
                    match self.encrypt_key {
                        SecretPatch::Unchanged => None,
                        SecretPatch::Clear => Some(""),
                        SecretPatch::Set(value) => Some(value),
                    },
                ),
            ]
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect(),
        )
    }
}

async fn maybe_rebuild_bot_token(
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    params: &UpdateBotParams<'_>,
) -> AppResult<Option<Vec<u8>>> {
    let fields = params.fields();
    if !adapter
        .registration()
        .token_fields
        .iter()
        .any(|field| fields.get(field).is_some())
    {
        return Ok(None);
    }
    let current = zeroize::Zeroizing::new(decrypt_bot_token(encryption_keys, bot).await?);
    let Some(token) = adapter.updated_token(&current, &fields)? else {
        return Ok(None);
    };
    adapter
        .verify_bot_token(
            http_client,
            &BotCredentials {
                token: &token,
                platform_bot_id: Some(&bot.platform_bot_id),
                platform_secrets: None,
            },
        )
        .await
        .map_err(|error| adapter.updated_token_error(error))?;
    Ok(Some(encryption_keys.encrypt(token.as_bytes()).await?))
}

async fn write_registration_fields(
    keys: &EncryptionKeys,
    descriptor: &RegistrationDescriptor,
    fields: &RegistrationValues<'_>,
    set: &mut bson::Document,
    unset: &mut bson::Document,
) -> AppResult<()> {
    for field in descriptor.all_fields() {
        // Token construction and verification are handled by the adapter.
        if field.name == "bot_token" {
            continue;
        }
        if let Some(value) = fields.get(field.name) {
            if value.is_empty() && field.clearable {
                unset.insert(field.storage, "");
            } else if field.secret {
                set.insert(
                    field.storage,
                    bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: keys.encrypt(value.as_bytes()).await?,
                    },
                );
            } else {
                set.insert(field.storage, value);
            }
        }
    }
    Ok(())
}

/// Register a new channel bot for the given user.
///
/// Verifies the token with the platform, encrypts it, generates a webhook
/// secret, and inserts the bot in `pending` status. The caller must follow up
/// with [`register_webhook`] to activate the bot.
#[allow(clippy::too_many_arguments)]
pub async fn create_bot(
    db: &mongodb::Database,
    config: &AppConfig,
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    user_id: &str,
    label: &str,
    fields: &RegistrationValues<'_>,
) -> AppResult<CreateBotResult> {
    let descriptor = adapter.registration();
    descriptor.validate(fields, false)?;
    // Validate label
    if label.is_empty() || label.len() > 200 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 200 characters".to_string(),
        ));
    }

    // Enforce per-user bot limit
    let active_count = db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .count_documents(doc! { "user_id": user_id, "is_active": true })
        .await?;

    if active_count >= u64::from(config.channel_relay_max_bots_per_user) {
        return Err(AppError::ChannelBotLimitReached(format!(
            "maximum of {} bots per user reached",
            config.channel_relay_max_bots_per_user
        )));
    }

    let effective_token = adapter.registration_token(fields)?;
    let identity = adapter
        .verify_bot_token(
            http_client,
            &BotCredentials {
                token: &effective_token,
                platform_bot_id: descriptor.identity(fields),
                platform_secrets: None,
            },
        )
        .await?;

    persist_verified_bot(
        db,
        encryption_keys,
        adapter,
        user_id,
        label,
        fields,
        &effective_token,
        identity,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn persist_verified_bot(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    user_id: &str,
    label: &str,
    fields: &RegistrationValues<'_>,
    effective_token: &str,
    identity: BotIdentity,
    managed: Option<(&str, &crate::models::channel_bot::ManagedBotSetup)>,
) -> AppResult<CreateBotResult> {
    let descriptor = adapter.registration();
    let BotIdentity {
        platform_bot_id,
        platform_bot_username,
    } = identity;

    // Check for duplicate platform bot
    let existing = db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .find_one(doc! {
            "platform": adapter.platform_id(),
            "platform_bot_id": &platform_bot_id,
            "is_active": true,
        })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(format!(
            "Bot {} is already registered on {}",
            platform_bot_username,
            adapter.platform_id()
        )));
    }

    // Generate webhook secret: raw (hex-encoded random bytes) + SHA-256 hash
    let raw_secret = hex::encode(rand::random::<[u8; 32]>());
    let secret_hash = hex::encode(Sha256::digest(raw_secret.as_bytes()));

    let bot_token_encrypted = encryption_keys.encrypt(effective_token.as_bytes()).await?;

    let now = Utc::now();
    let bot = ChannelBot {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        platform: adapter.platform_id().to_string(),
        label: label.to_string(),
        credential_source: if managed.is_some() {
            "platform"
        } else {
            "user"
        }
        .to_string(),
        registration_pin_encrypted: match managed {
            Some((pin, _)) => Some(encryption_keys.encrypt(pin.as_bytes()).await?),
            None => None,
        },
        webhook_secret_encrypted: if managed.is_some() {
            Some(encryption_keys.encrypt(raw_secret.as_bytes()).await?)
        } else {
            None
        },
        managed_setup: managed.map(|(_, setup)| setup.clone()),
        bot_token_encrypted,
        platform_bot_id,
        platform_bot_username,
        webhook_registered: false,
        webhook_secret_hash: secret_hash,
        app_id: None,
        app_secret_encrypted: None,
        lark_verification_token_encrypted: None,
        lark_encrypt_key_encrypted: None,
        public_key: None,
        status: if managed.is_some() {
            "pending_webhook"
        } else {
            "pending"
        }
        .to_string(),
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    let mut document = bson::to_document(&bot)
        .map_err(|_| AppError::Internal("Unable to serialize channel bot".to_string()))?;
    write_registration_fields(
        encryption_keys,
        &descriptor,
        fields,
        &mut document,
        &mut doc! {},
    )
    .await?;
    let bot: ChannelBot = bson::from_document(document)
        .map_err(|_| AppError::Internal("Invalid channel bot storage fields".to_string()))?;
    db.collection::<ChannelBot>(COLLECTION_NAME)
        .insert_one(&bot)
        .await?;

    Ok(CreateBotResult {
        bot,
        webhook_secret: raw_secret,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn create_managed_bot(
    db: &mongodb::Database,
    config: &AppConfig,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    owner: &str,
    label: &str,
    input: &super::channel_managed::ManagedOnboardingInput,
    progress: &super::channel_managed::ManagedProgress,
) -> AppResult<CreateBotResult> {
    let descriptor = adapter
        .managed_onboarding()
        .ok_or_else(super::channel_managed::unavailable)?;
    let credential_descriptor = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?;
    let row = super::platform_credential_service::load(db, descriptor.provider).await?;
    if !super::platform_credential_service::configured(row.as_ref(), &credential_descriptor) {
        return Err(super::channel_managed::unavailable());
    }
    if label.trim().is_empty() || label.len() > 128 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 128 characters".to_string(),
        ));
    }
    let active_count = db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .count_documents(doc! { "user_id": owner, "is_active": true })
        .await?;
    if active_count >= u64::from(config.channel_relay_max_bots_per_user) {
        return Err(AppError::ChannelBotLimitReached(format!(
            "maximum of {} bots per user reached",
            config.channel_relay_max_bots_per_user
        )));
    }
    let platform =
        super::platform_credential_service::load_decrypted(db, keys, descriptor.provider).await?;
    progress.stage("exchanging");
    let result = adapter
        .complete_managed_onboarding(http, &platform, input)
        .await?;
    let fields = RegistrationValues(
        result
            .fields
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect(),
    );
    let created = persist_verified_bot(
        db,
        keys,
        adapter,
        owner,
        label,
        &fields,
        &result.token,
        result.identity,
        Some((&result.registration_pin, &result.setup)),
    )
    .await?;
    let webhook_url = format!(
        "{}/api/v1/webhooks/channel/{}/{}",
        config.base_url,
        adapter.platform_id(),
        created.bot.id
    );
    let credentials = BotCredentials {
        token: &result.token,
        platform_bot_id: Some(&created.bot.platform_bot_id),
        platform_secrets: Some(&platform),
    };
    let setup = adapter
        .setup_managed_bot(
            http,
            &credentials,
            &created.bot,
            &webhook_url,
            &created.webhook_secret,
            &result.registration_pin,
            progress,
        )
        .await?;
    store_managed_setup(db, &created.bot.id, &setup).await?;
    Ok(CreateBotResult {
        bot: get_bot(db, &created.bot.id).await?,
        webhook_secret: created.webhook_secret,
    })
}

pub async fn store_managed_setup(
    db: &mongodb::Database,
    bot_id: &str,
    setup: &crate::models::channel_bot::ManagedBotSetup,
) -> AppResult<()> {
    let setup = bson::to_bson(setup)
        .map_err(|_| AppError::Internal("Unable to serialize managed setup".to_string()))?;
    db.collection::<ChannelBot>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": bot_id },
            doc! { "$set": { "managed_setup": setup, "updated_at": bson::DateTime::now() } },
        )
        .await?;
    Ok(())
}

pub async fn reregister_managed_bot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<()> {
    if bot.credential_source != "platform" || !bot.is_active {
        return Err(super::channel_managed::unavailable());
    }
    let provider = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?
        .provider;
    let platform = super::platform_credential_service::load_decrypted(db, keys, provider).await?;
    let token = zeroize::Zeroizing::new(decrypt_bot_token(keys, bot).await?);
    let pin_bytes = zeroize::Zeroizing::new(
        keys.decrypt(
            bot.registration_pin_encrypted
                .as_deref()
                .ok_or_else(super::channel_managed::unavailable)?,
        )
        .await?,
    );
    let pin = std::str::from_utf8(&pin_bytes)
        .map_err(|_| AppError::Internal("Invalid registration PIN encoding".to_string()))?;
    let credentials = BotCredentials {
        token: &token,
        platform_bot_id: Some(&bot.platform_bot_id),
        platform_secrets: Some(&platform),
    };
    let mut setup = bot
        .managed_setup
        .clone()
        .ok_or_else(super::channel_managed::unavailable)?;
    let result = adapter
        .reregister_managed_bot(http, &credentials, pin)
        .await;
    setup.registration = result
        .as_ref()
        .cloned()
        .unwrap_or_else(|_| "failed".to_string());
    store_managed_setup(db, &bot.id, &setup).await?;
    result.map(|_| ())
}

pub async fn repair_managed_bot(
    db: &mongodb::Database,
    config: &AppConfig,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<crate::models::channel_bot::ManagedBotSetup> {
    if bot.credential_source != "platform" || !bot.is_active {
        return Err(super::channel_managed::unavailable());
    }
    let provider = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?
        .provider;
    let platform = super::platform_credential_service::load_decrypted(db, keys, provider).await?;
    let token = zeroize::Zeroizing::new(decrypt_bot_token(keys, bot).await?);
    let pin = zeroize::Zeroizing::new(
        keys.decrypt(
            bot.registration_pin_encrypted
                .as_deref()
                .ok_or_else(super::channel_managed::unavailable)?,
        )
        .await?,
    );
    let pin = std::str::from_utf8(&pin)
        .map_err(|_| AppError::Internal("Invalid registration PIN encoding".to_string()))?;
    let credentials = BotCredentials {
        token: &token,
        platform_bot_id: Some(&bot.platform_bot_id),
        platform_secrets: Some(&platform),
    };

    // Earlier managed rows held only the hash. Atomically install a reusable
    // encrypted token before Meta can perform the callback handshake.
    if bot.webhook_secret_encrypted.is_none() {
        let secret = zeroize::Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
        let encrypted = keys.encrypt(secret.as_bytes()).await?;
        db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
            doc! { "_id": &bot.id, "is_active": true, "webhook_secret_encrypted": bson::Bson::Null },
            doc! { "$set": {
                "webhook_secret_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted },
                "webhook_secret_hash": hex::encode(Sha256::digest(secret.as_bytes())),
                "updated_at": bson::DateTime::now(),
            } },
        ).await?;
    }
    let bot = get_bot(db, &bot.id).await?;
    if !bot.is_active {
        return Err(super::channel_managed::unavailable());
    }
    let verify_token = zeroize::Zeroizing::new(
        keys.decrypt(
            bot.webhook_secret_encrypted
                .as_deref()
                .ok_or_else(super::channel_managed::unavailable)?,
        )
        .await?,
    );
    let verify_token = std::str::from_utf8(&verify_token)
        .map_err(|_| AppError::Internal("Invalid webhook token encoding".to_string()))?;
    let webhook_url = format!(
        "{}/api/v1/webhooks/channel/{}/{}",
        config.base_url,
        adapter.platform_id(),
        bot.id
    );
    let setup = adapter
        .setup_managed_bot(
            http,
            &credentials,
            &bot,
            &webhook_url,
            verify_token,
            pin,
            &super::channel_managed::ManagedProgress::default(),
        )
        .await?;
    store_managed_setup(db, &bot.id, &setup).await?;
    Ok(setup)
}

pub async fn update_bot(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    user_id: &str,
    params: UpdateBotParams<'_>,
) -> AppResult<ChannelBot> {
    let bot = get_bot_for_user(db, bot_id, user_id).await?;
    if bot.credential_source == "platform" && !params.fields().0.is_empty() {
        return Err(AppError::ValidationError(
            "Platform-managed credentials cannot be edited. Reconnect through managed onboarding."
                .to_string(),
        ));
    }
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    let mut unset_doc = doc! {};

    if let Some(label) = params.label {
        if label.is_empty() || label.len() > 200 {
            return Err(AppError::ValidationError(
                "Label must be between 1 and 200 characters".to_string(),
            ));
        }
        set_doc.insert("label", label);
    }

    let descriptor = adapter.registration();
    let fields = params.fields();
    descriptor.validate(&fields, true)?;
    write_registration_fields(
        encryption_keys,
        &descriptor,
        &fields,
        &mut set_doc,
        &mut unset_doc,
    )
    .await?;

    if let Some(bot_token_encrypted) =
        maybe_rebuild_bot_token(encryption_keys, http_client, adapter, &bot, &params).await?
    {
        set_doc.insert(
            "bot_token_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: bot_token_encrypted,
            },
        );
    }

    let mut update_doc = doc! { "$set": set_doc };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }

    db.collection::<ChannelBot>(COLLECTION_NAME)
        .update_one(doc! { "_id": bot_id, "user_id": user_id }, update_doc)
        .await?;

    get_bot_for_user(db, bot_id, user_id).await
}

/// Register the webhook URL with the platform and activate the bot.
///
/// The `webhook_secret` must be the raw secret returned from [`create_bot`].
pub async fn register_webhook(
    db: &mongodb::Database,
    http_client: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    bot_token: &str,
    webhook_url: &str,
    webhook_secret: &str,
) -> AppResult<()> {
    adapter
        .register_webhook(http_client, bot_token, webhook_url, webhook_secret)
        .await?;

    // Platforms with manual webhook setup (Discord, Lark, Feishu) return Ok
    // from register_webhook but the user must configure the URL themselves.
    // Only mark as fully registered for platforms where we actually set the URL.
    let auto_registered = adapter.registration().automatic_webhook;

    let (status, registered) = if auto_registered {
        ("active", true)
    } else {
        ("pending_webhook", false)
    };

    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<ChannelBot>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": bot_id },
            doc! { "$set": {
                "status": status,
                "webhook_registered": registered,
                "updated_at": now,
            }},
        )
        .await?;

    Ok(())
}

/// Mark a bot as failed (e.g. after webhook registration fails).
pub async fn mark_bot_failed(db: &mongodb::Database, bot_id: &str) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<ChannelBot>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": bot_id },
            doc! { "$set": {
                "status": "failed",
                "updated_at": now,
            }},
        )
        .await?;
    Ok(())
}

/// List all active bots for a user, newest first.
pub async fn list_bots(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<ChannelBot>> {
    let bots: Vec<ChannelBot> = db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;
    Ok(bots)
}

/// Get a bot by ID regardless of ownership.
pub async fn get_bot(db: &mongodb::Database, bot_id: &str) -> AppResult<ChannelBot> {
    db.collection::<ChannelBot>(COLLECTION_NAME)
        .find_one(doc! { "_id": bot_id })
        .await?
        .ok_or_else(|| AppError::ChannelBotNotFound(bot_id.to_string()))
}

/// Get a bot by ID with ownership verification.
pub async fn get_bot_for_user(
    db: &mongodb::Database,
    bot_id: &str,
    user_id: &str,
) -> AppResult<ChannelBot> {
    db.collection::<ChannelBot>(COLLECTION_NAME)
        .find_one(doc! { "_id": bot_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::ChannelBotNotFound(bot_id.to_string()))
}

/// Decrypt the encrypted bot token.
pub async fn decrypt_bot_token(
    encryption_keys: &EncryptionKeys,
    bot: &ChannelBot,
) -> AppResult<String> {
    let bytes = encryption_keys.decrypt(&bot.bot_token_encrypted).await?;
    String::from_utf8(bytes).map_err(|e| {
        AppError::Internal(format!("bot token decryption produced invalid UTF-8: {e}"))
    })
}

/// Soft-delete a bot: deregister webhook, deactivate bot and its conversations.
///
/// Webhook deregistration errors are logged but do not fail the operation,
/// because the bot token may have already been revoked on the platform side.
pub async fn delete_bot(
    db: &mongodb::Database,
    http_client: &reqwest::Client,
    encryption_keys: &EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    user_id: &str,
) -> AppResult<Option<&'static str>> {
    let bot = get_bot_for_user(db, bot_id, user_id).await?;

    // Best-effort webhook deregistration
    if bot.webhook_registered
        && let Ok(token) = decrypt_bot_token(encryption_keys, &bot).await
    {
        // Register with an empty URL to remove the webhook
        let _ = adapter.register_webhook(http_client, &token, "", "").await;
    }

    let now = bson::DateTime::from_chrono(Utc::now());

    // Soft-delete the bot
    db.collection::<ChannelBot>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": bot_id, "user_id": user_id },
            doc! { "$set": {
                "is_active": false,
                "status": "inactive",
                "updated_at": now,
            }},
        )
        .await?;

    // Deactivate all conversations tied to this bot
    db.collection::<mongodb::bson::Document>(CONVERSATIONS)
        .update_many(
            doc! { "channel_bot_id": bot_id },
            doc! { "$set": {
                "is_active": false,
                "updated_at": now,
            }},
        )
        .await?;

    let cleanup = if bot.credential_source == "platform" {
        Some(
            cleanup_managed_webhook(db, http_client, encryption_keys, adapter, &bot)
                .await
                .unwrap_or("failed"),
        )
    } else {
        None
    };
    Ok(cleanup)
}

async fn cleanup_managed_webhook(
    db: &mongodb::Database,
    http: &reqwest::Client,
    keys: &EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<&'static str> {
    let Some((field, value)) = adapter.managed_webhook_scope(bot) else {
        return Ok("not_applicable");
    };
    let mut filter = doc! { "_id": { "$ne": &bot.id }, "platform": &bot.platform, "credential_source": "platform", "is_active": true };
    filter.insert(field, value);
    if db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .find_one(filter)
        .await?
        .is_some()
    {
        return Ok("retained_shared");
    }
    let provider = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?
        .provider;
    let platform = super::platform_credential_service::load_decrypted(db, keys, provider).await?;
    let token = zeroize::Zeroizing::new(decrypt_bot_token(keys, bot).await?);
    let credentials = BotCredentials {
        token: &token,
        platform_bot_id: Some(&bot.platform_bot_id),
        platform_secrets: Some(&platform),
    };
    adapter
        .remove_managed_webhook_override(http, &credentials, bot)
        .await?;
    Ok("removed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::crypto::local_key_provider::LocalKeyProvider;
    use crate::services::channel_platform::{InboundMessage, OutboundReply};

    #[test]
    fn webhook_secret_hash_matches_sha256() {
        let raw = "test_secret_value";
        let hash = hex::encode(Sha256::digest(raw.as_bytes()));
        // Verify we get a 64-char hex string (256 bits)
        assert_eq!(hash.len(), 64);
        // Verify deterministic
        let hash2 = hex::encode(Sha256::digest(raw.as_bytes()));
        assert_eq!(hash, hash2);
    }

    #[test]
    fn webhook_secret_different_inputs_different_hashes() {
        let hash_a = hex::encode(Sha256::digest(b"secret_a"));
        let hash_b = hex::encode(Sha256::digest(b"secret_b"));
        assert_ne!(hash_a, hash_b);
    }

    use crate::services::channel_adapters::lark::parse_lark_bot_credentials;

    struct RecordingAdapter {
        seen_tokens: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl PlatformAdapter for RecordingAdapter {
        fn registration(&self) -> RegistrationDescriptor {
            crate::services::channel_adapters::lark::lark_registration()
        }
        fn updated_token(
            &self,
            current: &str,
            fields: &RegistrationValues<'_>,
        ) -> AppResult<Option<zeroize::Zeroizing<String>>> {
            crate::services::channel_adapters::lark::updated_lark_token(current, fields)
        }
        fn platform_id(&self) -> &str {
            "lark"
        }

        async fn verify_webhook(
            &self,
            _bot: &ChannelBot,
            _secrets: Option<&crate::services::channel_platform::PlatformVerifySecrets>,
            _headers: &axum::http::HeaderMap,
            _body: &[u8],
        ) -> AppResult<()> {
            unimplemented!("verify_webhook is not used in these tests")
        }

        async fn parse_inbound(&self, _body: &[u8]) -> AppResult<Vec<InboundMessage>> {
            unimplemented!("parse_inbound is not used in these tests")
        }

        async fn send_reply(
            &self,
            _http: &reqwest::Client,
            credentials: &crate::services::channel_platform::BotCredentials<'_>,
            _conversation_id: &str,
            _reply: &OutboundReply,
        ) -> AppResult<Option<String>> {
            let _bot_token = credentials.token;
            unimplemented!("send_reply is not used in these tests")
        }

        async fn register_webhook(
            &self,
            _http: &reqwest::Client,
            _bot_token: &str,
            _webhook_url: &str,
            _secret: &str,
        ) -> AppResult<()> {
            unimplemented!("register_webhook is not used in these tests")
        }

        async fn verify_bot_token(
            &self,
            _http: &reqwest::Client,
            credentials: &crate::services::channel_platform::BotCredentials<'_>,
        ) -> AppResult<BotIdentity> {
            let bot_token = credentials.token;
            self.seen_tokens.lock().unwrap().push(bot_token.to_string());
            Ok(BotIdentity {
                platform_bot_id: "cli_test".to_string(),
                platform_bot_username: "testbot".to_string(),
            })
        }
    }

    fn test_encryption_keys() -> EncryptionKeys {
        EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([0x11; 32], None)))
    }

    async fn make_lark_bot(encryption_keys: &EncryptionKeys, bot_token: &str) -> ChannelBot {
        ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "lark".to_string(),
            label: "Test Bot".to_string(),
            credential_source: "user".to_string(),
            registration_pin_encrypted: None,
            webhook_secret_encrypted: None,
            managed_setup: None,
            bot_token_encrypted: encryption_keys.encrypt(bot_token.as_bytes()).await.unwrap(),
            platform_bot_id: "cli_test".to_string(),
            platform_bot_username: "testbot".to_string(),
            webhook_registered: false,
            webhook_secret_hash: "unused".to_string(),
            app_id: Some("old_app".to_string()),
            app_secret_encrypted: None,
            lark_verification_token_encrypted: None,
            lark_encrypt_key_encrypted: None,
            public_key: None,
            status: "pending_webhook".to_string(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn rebuilds_composite_bot_token_when_only_app_id_changes() {
        let encryption_keys = test_encryption_keys();
        let http_client = reqwest::Client::new();
        let seen_tokens = Arc::new(Mutex::new(Vec::new()));
        let adapter = RecordingAdapter {
            seen_tokens: seen_tokens.clone(),
        };
        let bot = make_lark_bot(&encryption_keys, "old_app:old_secret").await;
        let params = UpdateBotParams {
            bot_token: None,
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: Some("new_app"),
            app_secret: None,
        };

        let rebuilt =
            maybe_rebuild_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
                .await
                .unwrap()
                .unwrap();

        assert_eq!(
            seen_tokens.lock().unwrap().as_slice(),
            &["new_app:old_secret".to_string()]
        );
        let decrypted = encryption_keys.decrypt(&rebuilt).await.unwrap();
        assert_eq!(String::from_utf8(decrypted).unwrap(), "new_app:old_secret");
    }

    #[tokio::test]
    async fn rebuilds_composite_bot_token_when_only_app_secret_changes() {
        let encryption_keys = test_encryption_keys();
        let http_client = reqwest::Client::new();
        let seen_tokens = Arc::new(Mutex::new(Vec::new()));
        let adapter = RecordingAdapter {
            seen_tokens: seen_tokens.clone(),
        };
        let bot = make_lark_bot(&encryption_keys, "old_app:old_secret").await;
        let params = UpdateBotParams {
            bot_token: None,
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: None,
            app_secret: Some("new_secret"),
        };

        let rebuilt =
            maybe_rebuild_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
                .await
                .unwrap()
                .unwrap();

        assert_eq!(
            seen_tokens.lock().unwrap().as_slice(),
            &["old_app:new_secret".to_string()]
        );
        let decrypted = encryption_keys.decrypt(&rebuilt).await.unwrap();
        assert_eq!(String::from_utf8(decrypted).unwrap(), "old_app:new_secret");
    }

    #[tokio::test]
    async fn rebuilds_composite_bot_token_when_both_lark_credentials_change() {
        let encryption_keys = test_encryption_keys();
        let http_client = reqwest::Client::new();
        let seen_tokens = Arc::new(Mutex::new(Vec::new()));
        let adapter = RecordingAdapter {
            seen_tokens: seen_tokens.clone(),
        };
        let bot = make_lark_bot(&encryption_keys, "old_app:old_secret").await;
        let params = UpdateBotParams {
            bot_token: None,
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: Some("new_app"),
            app_secret: Some("new_secret"),
        };

        let rebuilt =
            maybe_rebuild_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
                .await
                .unwrap()
                .unwrap();

        assert_eq!(
            seen_tokens.lock().unwrap().as_slice(),
            &["new_app:new_secret".to_string()]
        );
        let decrypted = encryption_keys.decrypt(&rebuilt).await.unwrap();
        assert_eq!(String::from_utf8(decrypted).unwrap(), "new_app:new_secret");
    }

    #[tokio::test]
    async fn leaves_composite_bot_token_unchanged_when_lark_credentials_are_not_patched() {
        let encryption_keys = test_encryption_keys();
        let http_client = reqwest::Client::new();
        let seen_tokens = Arc::new(Mutex::new(Vec::new()));
        let adapter = RecordingAdapter {
            seen_tokens: seen_tokens.clone(),
        };
        let bot = make_lark_bot(&encryption_keys, "old_app:old_secret").await;
        let params = UpdateBotParams {
            bot_token: None,
            label: Some("New Label"),
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: None,
            app_secret: None,
        };

        let rebuilt =
            maybe_rebuild_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
                .await
                .unwrap();

        assert!(rebuilt.is_none());
        assert!(seen_tokens.lock().unwrap().is_empty());
    }

    // ---- parse_lark_bot_credentials ----

    #[test]
    fn parse_lark_bot_credentials_valid_format() {
        let (app_id, app_secret) = parse_lark_bot_credentials("cli_abc123:secret_xyz").unwrap();
        assert_eq!(app_id, "cli_abc123");
        assert_eq!(app_secret, "secret_xyz");
    }

    #[test]
    fn parse_lark_bot_credentials_multiple_colons() {
        // split_once splits at the first colon, so the secret can contain colons
        let (app_id, app_secret) = parse_lark_bot_credentials("app:secret:with:colons").unwrap();
        assert_eq!(app_id, "app");
        assert_eq!(app_secret, "secret:with:colons");
    }

    #[test]
    fn parse_lark_bot_credentials_empty_app_id() {
        let (app_id, app_secret) = parse_lark_bot_credentials(":secret").unwrap();
        assert_eq!(app_id, "");
        assert_eq!(app_secret, "secret");
    }

    #[test]
    fn parse_lark_bot_credentials_empty_secret() {
        let (app_id, app_secret) = parse_lark_bot_credentials("app_id:").unwrap();
        assert_eq!(app_id, "app_id");
        assert_eq!(app_secret, "");
    }

    #[test]
    fn parse_lark_bot_credentials_missing_colon_returns_error() {
        let result = parse_lark_bot_credentials("no_colon_here");
        assert!(result.is_err());
    }

    #[test]
    fn parse_lark_bot_credentials_empty_string_returns_error() {
        let result = parse_lark_bot_credentials("");
        assert!(result.is_err());
    }

    // ---- SecretPatch ----

    #[test]
    fn secret_patch_unchanged_is_copy() {
        let patch = SecretPatch::Unchanged;
        let _copy = patch; // SecretPatch is Copy
        assert!(matches!(_copy, SecretPatch::Unchanged));
    }

    #[test]
    fn secret_patch_variants_are_distinct() {
        let unchanged = SecretPatch::Unchanged;
        let clear = SecretPatch::Clear;
        let set = SecretPatch::Set("value");

        assert!(matches!(unchanged, SecretPatch::Unchanged));
        assert!(matches!(clear, SecretPatch::Clear));
        assert!(matches!(set, SecretPatch::Set("value")));
    }

    // ---- maybe_rebuild_bot_token (non-lark platform) ----

    #[tokio::test]
    async fn maybe_rebuild_returns_none_for_non_lark_platform() {
        let encryption_keys = test_encryption_keys();
        let http_client = reqwest::Client::new();

        // Use a Telegram adapter instead of Lark
        struct TelegramAdapter;

        #[async_trait::async_trait]
        impl PlatformAdapter for TelegramAdapter {
            fn platform_id(&self) -> &str {
                "telegram"
            }

            async fn verify_webhook(
                &self,
                _bot: &ChannelBot,
                _secrets: Option<&crate::services::channel_platform::PlatformVerifySecrets>,
                _headers: &axum::http::HeaderMap,
                _body: &[u8],
            ) -> AppResult<()> {
                unimplemented!()
            }

            async fn parse_inbound(&self, _body: &[u8]) -> AppResult<Vec<InboundMessage>> {
                unimplemented!()
            }

            async fn send_reply(
                &self,
                _http: &reqwest::Client,
                credentials: &crate::services::channel_platform::BotCredentials<'_>,
                _conversation_id: &str,
                _reply: &OutboundReply,
            ) -> AppResult<Option<String>> {
                let _bot_token = credentials.token;
                unimplemented!()
            }

            async fn register_webhook(
                &self,
                _http: &reqwest::Client,
                _bot_token: &str,
                _webhook_url: &str,
                _secret: &str,
            ) -> AppResult<()> {
                unimplemented!()
            }

            async fn verify_bot_token(
                &self,
                _http: &reqwest::Client,
                credentials: &crate::services::channel_platform::BotCredentials<'_>,
            ) -> AppResult<BotIdentity> {
                let _bot_token = credentials.token;
                unimplemented!()
            }
        }

        let adapter = TelegramAdapter;
        let mut bot = make_lark_bot(&encryption_keys, "some_token").await;
        bot.platform = "telegram".to_string();

        let params = UpdateBotParams {
            bot_token: None,
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: Some("new_app"),
            app_secret: Some("new_secret"),
        };

        let result =
            maybe_rebuild_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
                .await
                .unwrap();

        assert!(result.is_none());
    }

    // ---- decrypt_bot_token ----

    #[tokio::test]
    async fn decrypt_bot_token_roundtrips() {
        let encryption_keys = test_encryption_keys();
        let bot = make_lark_bot(&encryption_keys, "test_app:test_secret").await;
        let decrypted = decrypt_bot_token(&encryption_keys, &bot).await.unwrap();
        assert_eq!(decrypted, "test_app:test_secret");
    }

    // ---- webhook_secret_hash properties ----

    #[test]
    fn webhook_secret_hash_is_64_hex_chars() {
        let raw = hex::encode(rand::random::<[u8; 32]>());
        let hash = hex::encode(Sha256::digest(raw.as_bytes()));
        assert_eq!(hash.len(), 64);
        // all chars are hex
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
