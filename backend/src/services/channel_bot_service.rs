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

/// A new bot or the owner's existing manager connection. Reused connections
/// require live verification and never return a webhook secret.
pub struct CreateBotResult {
    pub bot: ChannelBot,
    pub webhook_secret: String,
    pub reused: bool,
}

#[derive(Clone, Copy)]
pub enum SecretPatch<'a> {
    Unchanged,
    Clear,
    Set(&'a str),
}

pub struct UpdateBotParams<'a> {
    pub x_events: Option<&'a [crate::models::channel_bot::XChannelEvent]>,
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
                billing: None,
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

/// Register a channel bot or recover the owner's existing manager connection.
///
/// Verifies the token with the platform, encrypts it, generates a webhook
/// secret, and inserts a new bot in `pending` status. The caller must follow up
/// with [`register_webhook`] for a new bot or [`verify_telegram_bot`] for a reused
/// manager connection to activate it.
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
    if descriptor.managed_only {
        return Err(AppError::ValidationError(
            descriptor.managed_only_message.to_string(),
        ));
    }
    descriptor.validate(fields, false)?;
    // Validate label
    if label.is_empty() || label.len() > 200 {
        return Err(AppError::ValidationError(
            "Label must be between 1 and 200 characters".to_string(),
        ));
    }

    let effective_token = adapter.registration_token(fields)?;
    let identity = adapter
        .verify_bot_token(
            http_client,
            &BotCredentials {
                billing: None,
                token: &effective_token,
                platform_bot_id: descriptor.identity(fields),
                platform_secrets: None,
            },
        )
        .await?;

    persist_verified_bot(
        db,
        config,
        encryption_keys,
        adapter,
        user_id,
        label,
        fields,
        &effective_token,
        identity,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn persist_verified_bot(
    db: &mongodb::Database,
    config: &AppConfig,
    encryption_keys: &EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    user_id: &str,
    label: &str,
    fields: &RegistrationValues<'_>,
    effective_token: &str,
    identity: BotIdentity,
    managed: Option<(&str, &crate::models::channel_bot::ManagedBotSetup)>,
    connection: Option<(&str, &super::channel_platform::PollOutcome)>,
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

    if let Some(existing) = existing {
        if managed.is_none()
            && connection.is_none()
            && adapter.platform_id() == "telegram"
            && existing.user_id == user_id
            && existing.credential_source == "telegram_manager"
        {
            return Ok(CreateBotResult {
                bot: existing,
                webhook_secret: String::new(),
                reused: true,
            });
        }
        return Err(AppError::Conflict(format!(
            "Bot {} is already registered on {}. Open its existing connection in Channel Bots to complete setup. Check Personal and organization scopes, or ask the connection owner for access.",
            platform_bot_username,
            adapter.platform_id()
        )));
    }

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

    // Generate webhook secret: raw (hex-encoded random bytes) + SHA-256 hash
    let raw_secret = if descriptor.webhook_ingestion && connection.is_none() {
        hex::encode(rand::random::<[u8; 32]>())
    } else {
        String::new()
    };
    let secret_hash = if descriptor.webhook_ingestion && connection.is_none() {
        hex::encode(Sha256::digest(raw_secret.as_bytes()))
    } else {
        String::new()
    };

    let bot_token_encrypted = if connection.is_some() {
        Vec::new()
    } else {
        encryption_keys.encrypt(effective_token.as_bytes()).await?
    };

    let now = Utc::now();
    let bot = ChannelBot {
        x_events: None,
        last_verification: None,
        ownership_version: 0,
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        platform: adapter.platform_id().to_string(),
        label: label.to_string(),
        credential_source: if connection.is_some() {
            "connection"
        } else if managed.is_some() {
            "platform"
        } else {
            "user"
        }
        .to_string(),
        connection_id: connection.map(|(id, _)| id.to_string()),
        poll_cursor: connection.and_then(|(_, outcome)| outcome.cursor.clone()),
        poll_lease_until: None,
        last_polled_at: connection.and_then(|(_, outcome)| outcome.cursor.as_ref().map(|_| now)),
        poll_backoff_until: connection
            .and_then(|(_, outcome)| outcome.backoff)
            .map(|d| now + chrono::Duration::seconds(d.as_secs().min(86400) as i64)),
        poll_error_count: 0,
        last_poll_notice: None,
        error: None,
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
        status: if !descriptor.webhook_ingestion || connection.is_some() {
            "active"
        } else if managed.is_some() {
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
    insert_registered_bot(db, &bot, config.channel_relay_max_bots_per_user, None).await?;
    let bot = get_bot(db, &bot.id).await?;

    Ok(CreateBotResult {
        reused: false,
        webhook_secret: if bot.credential_source == "telegram_manager" {
            String::new()
        } else {
            raw_secret
        },
        bot,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn create_managed_bot(
    db: &mongodb::Database,
    billing: &super::billing::BillingService,
    config: &AppConfig,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    owner: &str,
    label: &str,
    input: &super::channel_managed::ManagedOnboardingInput,
    progress: &super::channel_managed::ManagedProgress,
) -> AppResult<CreateBotResult> {
    let managed = adapter
        .managed_onboarding()
        .ok_or_else(super::channel_managed::unavailable)?;
    let credential_descriptor = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?;
    if managed.provider != credential_descriptor.provider {
        return Err(super::channel_managed::unavailable());
    }
    let row = super::platform_credential_service::load(db, &credential_descriptor).await?;
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
    if let super::channel_platform::CredentialResolution::OAuthConnection {
        provider_slug,
        required_scopes,
    } = adapter.credential_resolution()
    {
        let connection_id = input.get("connection_id")?;
        let token = super::channel_credentials::connection_token(
            db,
            keys,
            owner,
            connection_id,
            provider_slug,
            required_scopes,
        )
        .await?;
        let platform =
            super::platform_credential_service::load_decrypted(db, keys, &credential_descriptor)
                .await?;
        if !adapter.connection_webhook_configured(&platform) {
            super::channel_billing_service::require_webhooks(config, adapter.platform_id())?;
        }
        progress.stage("verifying");
        let channel_billing = super::channel_billing_service::ChannelBilling::for_owner(
            db,
            billing,
            adapter.platform_id(),
            owner,
            None,
        );
        let mut credentials = BotCredentials::from(token.as_str());
        credentials.billing = channel_billing.as_ref();
        let identity = adapter.verify_bot_token(http, &credentials).await?;
        let outcome = if adapter.connection_webhook_configured(&platform) {
            super::channel_platform::PollOutcome {
                messages: vec![],
                cursor: None,
                backoff: None,
                notice: None,
            }
        } else {
            super::channel_billing_service::require_webhooks(config, adapter.platform_id())?;
            let outcome = adapter
                .poll_inbound(
                    http,
                    &BotCredentials {
                        billing: None,
                        token: &token,
                        platform_bot_id: Some(&identity.platform_bot_id),
                        platform_secrets: None,
                    },
                    None,
                )
                .await?;
            if outcome.cursor.is_none() {
                return Err(AppError::ChannelPlatformError(
                    "Initial channel poll was rate limited; retry after the provider's reset"
                        .to_string(),
                ));
            }
            outcome
        };
        let created = persist_verified_bot(
            db,
            config,
            keys,
            adapter,
            owner,
            label,
            &RegistrationValues::default(),
            "",
            identity,
            None,
            Some((connection_id, &outcome)),
        )
        .await?;
        progress.stage("subscribing");
        if super::channel_connection_webhook_service::configure(
            db,
            billing,
            keys,
            http,
            adapter,
            &created.bot,
            &config.base_url,
        )
        .await
        .is_err()
            && !billing.billing_enabled()
        {
            db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
                    doc! {"_id": &created.bot.id, "is_active": true, "status": "active"},
                    doc! {"$set": {"error": "Webhook setup did not complete. Polling remains enabled; select Verify to retry webhook setup."}},
            ).await?;
        }
        return Ok(CreateBotResult {
            bot: get_bot(db, &created.bot.id).await?,
            webhook_secret: created.webhook_secret,
            reused: created.reused,
        });
    }
    let platform =
        super::platform_credential_service::load_decrypted(db, keys, &credential_descriptor)
            .await?;
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
        config,
        keys,
        adapter,
        owner,
        label,
        &fields,
        &result.token,
        result.identity,
        Some((&result.registration_pin, &result.setup)),
        None,
    )
    .await?;
    let webhook_url = format!(
        "{}/api/v1/webhooks/channel/{}/{}",
        config.base_url,
        adapter.platform_id(),
        created.bot.id
    );
    let credentials = BotCredentials {
        billing: None,
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
        reused: created.reused,
    })
}

/// All registration paths serialize the identity and owner-limit check with
/// insertion. Telegram's manual and managed variants share one remote identity.
pub(crate) async fn insert_registered_bot(
    db: &mongodb::Database,
    bot: &ChannelBot,
    capacity: u32,
    telegram_request_revision: Option<i64>,
) -> AppResult<()> {
    if matches!(bot.platform.as_str(), "telegram" | "telegram-new") {
        super::telegram_new_service::with_operation(
            db,
            &format!("telegram-manager-identity:{}", bot.platform_bot_id),
            insert_registered_bot_inner(db, bot, capacity, telegram_request_revision),
        )
        .await
    } else {
        insert_registered_bot_inner(db, bot, capacity, telegram_request_revision).await
    }
}

async fn insert_registered_bot_inner(
    db: &mongodb::Database,
    bot: &ChannelBot,
    capacity: u32,
    telegram_request_revision: Option<i64>,
) -> AppResult<()> {
    use super::api_key_mutation_service::{map_transaction_error, transaction_result};
    use crate::models::platform_settings::{COLLECTION_NAME as SETTINGS, PLATFORM_SETTINGS_ID};
    use crate::models::telegram_bot_request::{COLLECTION_NAME as REQUESTS, MANAGED_BOTS};
    db.collection::<bson::Document>(SETTINGS)
        .update_one(
            doc! {"_id": PLATFORM_SETTINGS_ID},
            doc! {"$setOnInsert": {"channel_bot_registration_revision": 0_i64}},
        )
        .upsert(true)
        .await?;
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let mut bot = bot.clone();
    session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<()> = async {
            db.collection::<bson::Document>(SETTINGS).update_one(doc! {"_id": PLATFORM_SETTINGS_ID}, doc! {"$inc": {"channel_bot_registration_revision": 1_i64}}).session(&mut *session).await?;
            let bots = db.collection::<ChannelBot>(COLLECTION_NAME);
            if let Some(existing) = bots.find_one(doc! {"_id": &bot.id}).session(&mut *session).await? {
                if telegram_request_revision.is_some() && existing.is_active && existing.user_id == bot.user_id && existing.platform_bot_id == bot.platform_bot_id { return Ok(()); }
                return Err(AppError::Conflict("Bot registration already exists".into()));
            }
            let platform = if matches!(bot.platform.as_str(), "telegram" | "telegram-new") { bson::Bson::Document(doc! {"$in": ["telegram", "telegram-new"]}) } else { bson::Bson::String(bot.platform.clone()) };
            if bots.find_one(doc! {"platform": platform, "platform_bot_id": &bot.platform_bot_id, "is_active": true}).session(&mut *session).await?.is_some() {
                return Err(AppError::Conflict("This bot is already connected. Open its existing connection in Channel Bots to complete setup. Check Personal and organization scopes, or ask the connection owner for access.".into()));
            }
            if matches!(bot.platform.as_str(), "telegram" | "telegram-new") && let Some(manager) = db.collection::<crate::models::platform_credential::PlatformCredential>(crate::models::platform_credential::COLLECTION_NAME).find_one(doc! {"provider": "telegram-new", "fields.manager_bot_id": &bot.platform_bot_id}).session(&mut *session).await? {
                if bot.platform != "telegram" || manager.fields.get("webhook_ready").map(String::as_str) != Some("true") {
                    return Err(AppError::Conflict("Save the Telegram manager configuration successfully before connecting it with Telegram bot token.".into()));
                }
                bot.credential_source = "telegram_manager".into();
                bot.bot_token_encrypted.clear();
                bot.webhook_secret_hash.clear();
            }
            if bots.count_documents(doc! {"user_id": &bot.user_id, "is_active": true}).session(&mut *session).await? >= u64::from(capacity) {
                return Err(AppError::ChannelBotLimitReached(format!("maximum of {capacity} bots per owner reached")));
            }
            if let Some(revision) = telegram_request_revision {
                let request = db.collection::<crate::models::telegram_bot_request::TelegramBotRequest>(REQUESTS).find_one_and_update(
                    doc! {"_id": &bot.id, "owner_user_id": &bot.user_id, "revision": revision, "status": "ready", "active": true, "expires_at": {"$gt": bson::DateTime::now()}},
                    doc! {"$set": {"status": "provisioning"}, "$inc": {"revision": 1}},
                ).session(&mut *session).await?.ok_or_else(|| AppError::Conflict("The Telegram approval expired or changed. Refresh the request.".into()))?;
                let gate = db.collection::<bson::Document>(MANAGED_BOTS).update_one(
                    doc! {"manager_bot_id": request.manager_bot_id, "telegram_bot_id": request.telegram_bot_id, "telegram_user_id": request.telegram_user_id, "revision": request.manager_revision, "observation_id": &request.observation_id, "retired": {"$ne": true}},
                    doc! {"$inc": {"claim_revision": 1_i64}, "$set": {"retired": true}},
                ).session(&mut *session).await?;
                if gate.matched_count != 1 { return Err(AppError::Conflict("Telegram management changed after consent. Start a fresh connection request.".into())); }
            }
            if bot.credential_source == "connection" {
                let connection_id = bot.connection_id.as_deref().ok_or_else(|| {
                    AppError::ValidationError(
                        "Connection-backed bot is missing its OAuth credential".into(),
                    )
                })?;
                let fenced = crate::services::service_history::mutation::fence_backing_reference(
                    &db,
                    crate::models::user_api_key::COLLECTION_NAME,
                    connection_id,
                    &bot.user_id,
                    &mut *session,
                )
                .await?;
                if !fenced {
                    return Err(AppError::NotFound(
                        "Connected OAuth credential not found".into(),
                    ));
                }
            }
            bots.insert_one(&bot).session(&mut *session).await?;
            Ok(())
        }.await;
        transaction_result(operation)
    }).await.map_err(map_transaction_error)
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

pub async fn reconnect_bot(
    db: &mongodb::Database,
    billing: &super::billing::BillingService,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    connection_id: &str,
) -> AppResult<()> {
    super::channel_connection_webhook_service::serialized(
        db,
        adapter.platform_id(),
        reconnect_bot_inner(db, billing, keys, http, adapter, bot, connection_id),
    )
    .await
}

async fn reconnect_bot_inner(
    db: &mongodb::Database,
    billing: &super::billing::BillingService,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    connection_id: &str,
) -> AppResult<()> {
    let super::channel_platform::CredentialResolution::OAuthConnection {
        provider_slug,
        required_scopes,
    } = adapter.credential_resolution()
    else {
        return Err(super::channel_managed::unavailable());
    };
    if !bot.is_active || bot.credential_source != "connection" {
        return Err(super::channel_managed::unavailable());
    }
    let required_scopes =
        if bot.platform == "x" && super::channel_adapters::x::public_events_enabled(bot) {
            super::channel_adapters::x::PUBLIC_SCOPES
        } else {
            required_scopes
        };
    let token = super::channel_credentials::connection_token(
        db,
        keys,
        &bot.user_id,
        connection_id,
        provider_slug,
        required_scopes,
    )
    .await?;
    let channel_billing =
        super::channel_billing_service::ChannelBilling::for_bot(db, billing, bot, None);
    let mut credentials = BotCredentials::from(token.as_str());
    credentials.billing = channel_billing.as_ref();
    let identity = adapter.verify_bot_token(http, &credentials).await?;
    if identity.platform_bot_id != bot.platform_bot_id {
        return Err(AppError::ValidationError(
            "Reconnect the same platform account to preserve its conversation routes".to_string(),
        ));
    }
    let (cursor, backoff, last_polled_at) = if bot.webhook_registered
        || (bot.platform == "x"
            && (billing.billing_enabled()
                || super::channel_adapters::x::public_events_enabled(bot)))
    {
        (
            bot.poll_cursor.clone(),
            None,
            bot.last_polled_at.map(bson::DateTime::from_chrono),
        )
    } else if let Some(cursor) = &bot.poll_cursor {
        // Resume from the last committed event so DMs received during failure remain eligible.
        (
            Some(cursor.clone()),
            None,
            bot.last_polled_at.map(bson::DateTime::from_chrono),
        )
    } else {
        let outcome = adapter
            .poll_inbound(
                http,
                &BotCredentials {
                    billing: None,
                    token: &token,
                    platform_bot_id: Some(&identity.platform_bot_id),
                    platform_secrets: None,
                },
                None,
            )
            .await?;
        let cursor = outcome.cursor.ok_or_else(|| {
            AppError::ChannelPlatformError(
                "Initial channel poll was rate limited; retry later".to_string(),
            )
        })?;
        (Some(cursor), outcome.backoff, Some(bson::DateTime::now()))
    };
    let identity_username = identity.platform_bot_username;
    let connection_id = connection_id.to_string();
    let db = db.clone();
    let bot = bot.clone();
    let result = crate::services::service_history::transaction::run(&db.clone(), async move |transaction| {
        let operation: AppResult<()> = async {
            let session: &mut mongodb::ClientSession = transaction.into();
            let fenced = crate::services::service_history::mutation::fence_backing_reference(
                &db,
                crate::models::user_api_key::COLLECTION_NAME,
                &connection_id,
                &bot.user_id,
                session,
            )
            .await?;
            if !fenced {
                return Err(AppError::NotFound(
                    "Connected OAuth credential not found".into(),
                ));
            }
            let result = db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
                doc! { "_id": &bot.id, "user_id": &bot.user_id, "is_active": true, "connection_id": &bot.connection_id, "poll_cursor": &bot.poll_cursor, "updated_at": bson::DateTime::from_chrono(bot.updated_at) },
                doc! { "$set": {
                    "connection_id": &connection_id, "platform_bot_username": &identity_username,
                    "poll_cursor": &cursor, "poll_lease_until": null, "poll_error_count": 0,
                    "poll_backoff_until": backoff.map(|d| bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(d.as_secs().min(86400) as i64))),
                    "last_polled_at": last_polled_at, "status": "active", "error": null, "updated_at": bson::DateTime::now(),
                } },
            ).session(session).await?;
            if result.matched_count == 0 {
                return Err(AppError::Conflict(
                    "Channel bot changed during reconnect; retry".to_string(),
                ));
            }
            Ok(())
        }.await;
        super::api_key_mutation_service::transaction_result(operation)
    }).await;
    result.map_err(super::api_key_mutation_service::map_transaction_error)
}

pub async fn reregister_managed_bot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<()> {
    if bot.credential_source != "platform"
        || !bot.is_active
        || bot.status == "suspended"
        || bot.platform == "telegram-new"
    {
        return Err(super::channel_managed::unavailable());
    }
    let descriptor = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?;
    let platform =
        super::platform_credential_service::load_decrypted(db, keys, &descriptor).await?;
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
        billing: None,
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
    if bot.credential_source != "platform"
        || !bot.is_active
        || bot.status == "suspended"
        || bot.platform == "telegram-new"
    {
        return Err(super::channel_managed::unavailable());
    }
    let descriptor = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?;
    let platform =
        super::platform_credential_service::load_decrypted(db, keys, &descriptor).await?;
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
        billing: None,
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
    if params.x_events.is_some() {
        return super::channel_connection_webhook_service::serialized(
            db,
            adapter.platform_id(),
            update_bot_inner(
                db,
                encryption_keys,
                http_client,
                adapter,
                bot_id,
                user_id,
                params,
            ),
        )
        .await;
    }
    super::channel_retry_ingress::with_lifecycle(
        db,
        adapter.serializes_lifecycle(),
        bot_id,
        update_bot_inner(
            db,
            encryption_keys,
            http_client,
            adapter,
            bot_id,
            user_id,
            params,
        ),
    )
    .await
}

async fn update_bot_inner(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    user_id: &str,
    params: UpdateBotParams<'_>,
) -> AppResult<ChannelBot> {
    let bot = get_bot_for_user(db, bot_id, user_id).await?;
    if bot.credential_source == "telegram_manager" && !params.fields().0.is_empty() {
        return Err(AppError::ValidationError(
            "Update this bot's token in Admin > Platform Credentials > Telegram — bot creation."
                .into(),
        ));
    }
    if matches!(bot.credential_source.as_str(), "platform" | "connection")
        && !params.fields().0.is_empty()
    {
        return Err(AppError::ValidationError(
            "Platform-managed credentials cannot be edited. Reconnect through managed onboarding."
                .to_string(),
        ));
    }
    if adapter.serializes_lifecycle() && !bot.is_active {
        return Err(AppError::ChannelBotInactive("Bot has been deleted".into()));
    }
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    let mut unset_doc = doc! {};

    if let Some(events) = params.x_events {
        super::channel_adapters::x::validate_events(&bot.platform, events)?;
        if !bot.is_active || bot.credential_source != "connection" {
            return Err(AppError::ValidationError(
                "X event selection requires an active connected account".into(),
            ));
        }
        let descriptor = adapter
            .platform_credentials()
            .ok_or_else(super::channel_managed::unavailable)?;
        let platform =
            super::platform_credential_service::load_decrypted(db, encryption_keys, &descriptor)
                .await?;
        if !adapter.connection_webhook_configured(&platform) {
            return Err(AppError::ValidationError(
                "Configure X platform webhook credentials before selecting events".into(),
            ));
        }
        let mut proposed = bot.clone();
        proposed.x_events = Some(events.to_vec());
        let scopes = if super::channel_adapters::x::public_events_enabled(&proposed) {
            super::channel_adapters::x::PUBLIC_SCOPES
        } else {
            super::channel_adapters::x::REQUIRED_SCOPES
        };
        super::channel_credentials::connection_token(
            db,
            encryption_keys,
            &bot.user_id,
            bot.connection_id
                .as_deref()
                .ok_or_else(super::channel_managed::unavailable)?,
            "twitter",
            scopes,
        )
        .await?;
        set_doc.insert(
            "x_events",
            bson::to_bson(events)
                .map_err(|_| AppError::Internal("Unable to encode X events".into()))?,
        );
    }

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
    if !fields.0.is_empty() {
        unset_doc.insert("last_verification", "");
    }
    if adapter.serializes_lifecycle() && fields.get("app_secret").is_some() {
        set_doc.insert("status", "pending_webhook");
        set_doc.insert("webhook_registered", false);
    }

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

pub fn webhook_url(base_url: &str, bot: &ChannelBot) -> String {
    let base = base_url.trim_end_matches('/');
    if bot.credential_source == "telegram_manager" {
        format!("{base}/api/v1/webhooks/channel/telegram-new/manager")
    } else {
        format!("{base}/api/v1/webhooks/channel/{}/{}", bot.platform, bot.id)
    }
}

fn manager_channel_ready(
    bot: &ChannelBot,
    manager: Option<&crate::models::platform_credential::PlatformCredential>,
) -> bool {
    bot.platform == "telegram"
        && manager.is_some_and(|manager| {
            manager.fields.get("manager_bot_id") == Some(&bot.platform_bot_id)
                && manager.fields.get("webhook_ready").map(String::as_str) == Some("true")
        })
}

/// Project saved manager availability for responses without changing stored bots.
pub async fn apply_manager_configuration_status(
    db: &mongodb::Database,
    bots: &mut [ChannelBot],
) -> AppResult<()> {
    if !bots
        .iter()
        .any(|bot| bot.credential_source == "telegram_manager" && bot.is_active)
    {
        return Ok(());
    }
    let manager = super::platform_credential_service::load(
        db,
        &super::channel_adapters::telegram_new::credential_descriptor(),
    )
    .await?;
    for bot in bots {
        if bot.credential_source == "telegram_manager"
            && bot.is_active
            && !manager_channel_ready(bot, manager.as_ref())
        {
            bot.status = "failed".into();
            bot.webhook_registered = false;
            bot.error = Some(super::telegram_new_admin::MANAGER_WEBHOOK_ERROR.into());
        }
    }
    Ok(())
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
    register_webhook_with_telegram_api(
        db,
        &super::telegram_new_api::TelegramApi::new(http_client),
        adapter,
        bot_id,
        bot_token,
        webhook_url,
        webhook_secret,
    )
    .await
}

pub(crate) async fn register_webhook_with_telegram_api(
    db: &mongodb::Database,
    telegram_api: &super::telegram_new_api::TelegramApi<'_>,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    bot_token: &str,
    webhook_url: &str,
    webhook_secret: &str,
) -> AppResult<()> {
    if adapter.serializes_lifecycle() {
        return super::channel_retry_ingress::with_lifecycle(
            db,
            adapter.serializes_lifecycle(),
            bot_id,
            register_webhook_inner(
                db,
                telegram_api.http,
                adapter,
                bot_id,
                bot_token,
                webhook_url,
                webhook_secret,
            ),
        )
        .await;
    }
    let bot = get_bot(db, bot_id).await?;
    if bot.platform == "telegram" {
        return super::telegram_new_service::with_operation(
            db,
            &format!("telegram-manager-identity:{}", bot.platform_bot_id),
            async {
                let bot = get_bot(db, bot_id).await?;
                ensure_telegram_webhook_owner(db, &bot).await?;
                if bot.credential_source != "telegram_manager" {
                    verify_telegram_token_identity(telegram_api.http, adapter, &bot, bot_token)
                        .await?;
                }
                register_telegram_webhook_locked(
                    db,
                    telegram_api,
                    adapter,
                    &bot,
                    bot_token,
                    webhook_url,
                    webhook_secret,
                )
                .await
            },
        )
        .await;
    }
    adapter
        .register_webhook(telegram_api.http, bot_token, webhook_url, webhook_secret)
        .await?;
    store_webhook_registration(db, adapter, doc! { "_id": bot_id }).await
}

async fn is_telegram_manager_identity(db: &mongodb::Database, remote_id: &str) -> AppResult<bool> {
    let manager = super::platform_credential_service::load(
        db,
        &super::channel_adapters::telegram_new::credential_descriptor(),
    )
    .await?;
    Ok(manager.is_some_and(|manager| {
        manager.fields.get("manager_bot_id").map(String::as_str) == Some(remote_id)
    }))
}

async fn ensure_telegram_webhook_owner(db: &mongodb::Database, bot: &ChannelBot) -> AppResult<()> {
    if !bot.is_active {
        return Err(AppError::ChannelBotInactive("Bot has been deleted".into()));
    }
    if bot.credential_source != "telegram_manager"
        && is_telegram_manager_identity(db, &bot.platform_bot_id).await?
    {
        return Err(AppError::Conflict(
            "This Telegram identity is configured as the manager. Reconnect it as a manager channel.".into(),
        ));
    }
    Ok(())
}

async fn verify_telegram_token_identity(
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    token: &str,
) -> AppResult<()> {
    let identity = adapter
        .verify_bot_token(
            http,
            &BotCredentials {
                billing: None,
                token,
                platform_bot_id: Some(&bot.platform_bot_id),
                platform_secrets: None,
            },
        )
        .await?;
    if identity.platform_bot_id != bot.platform_bot_id {
        return Err(AppError::ValidationError(
            "Telegram token belongs to a different bot. Reconnect the channel with its own token."
                .into(),
        ));
    }
    Ok(())
}

// The identity lease covers the provider effect and its matching local secret/status.
async fn register_telegram_webhook_locked(
    db: &mongodb::Database,
    telegram_api: &super::telegram_new_api::TelegramApi<'_>,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    bot_token: &str,
    webhook_url: &str,
    webhook_secret: &str,
) -> AppResult<()> {
    if bot.credential_source == "telegram_manager" {
        let verification = async {
            let manager = super::platform_credential_service::load(
                db,
                &super::channel_adapters::telegram_new::credential_descriptor(),
            )
            .await?;
            if !manager_channel_ready(bot, manager.as_ref()) {
                return Err(AppError::Conflict(
                    super::telegram_new_admin::MANAGER_WEBHOOK_ERROR.into(),
                ));
            }
            let webhook = telegram_api
                .call(bot_token, "getWebhookInfo", serde_json::json!({}))
                .await?;
            super::telegram_new_admin::validate_manager_webhook(&webhook, webhook_url)
        }
        .await;
        if let Err(error) = verification {
            mark_manager_channel_failed(db, &bot.id).await?;
            return Err(error);
        }
    } else {
        adapter
            .register_webhook(telegram_api.http, bot_token, webhook_url, webhook_secret)
            .await?;
    }
    let mut fields = doc! {
        "status": "active", "webhook_registered": true, "error": bson::Bson::Null,
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    if bot.credential_source != "telegram_manager" {
        fields.insert(
            "webhook_secret_hash",
            hex::encode(Sha256::digest(webhook_secret.as_bytes())),
        );
    }
    let mut filter = doc! {
        "_id": &bot.id,
        "is_active": true,
        "credential_source": &bot.credential_source,
        "platform_bot_id": &bot.platform_bot_id,
    };
    if bot.credential_source == "user" {
        filter.insert(
            "credential_source",
            doc! { "$in": ["user", bson::Bson::Null] },
        );
    }
    let result = db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .update_one(filter, doc! { "$set": fields })
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::Conflict(
            "Telegram channel changed during verification. Try again.".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn verify_telegram_bot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    telegram_api: &super::telegram_new_api::TelegramApi<'_>,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    owner_id: &str,
    base_url: &str,
) -> AppResult<ChannelBot> {
    let bot = get_bot_for_user(db, bot_id, owner_id).await?;
    super::telegram_new_service::with_operation(
        db,
        &format!("telegram-manager-identity:{}", bot.platform_bot_id),
        async {
            let bot = get_bot_for_user(db, bot_id, owner_id).await?;
            ensure_telegram_webhook_owner(db, &bot).await?;
            let verified_token = async {
                let token = super::channel_credentials::resolve_bot_token(db, keys, adapter, &bot).await?;
                verify_telegram_token_identity(telegram_api.http, adapter, &bot, &token).await?;
                adapter.validate_stored_verification(&bot)?;
                Ok(token)
            }.await;
            let token = match verified_token {
                Ok(token) => token,
                Err(error) => {
                    if bot.credential_source == "telegram_manager" {
                        mark_manager_channel_failed(db, bot_id).await?;
                    }
                    return Err(error);
                }
            };
            let secret = if bot.credential_source == "telegram_manager" {
                String::new()
            } else {
                hex::encode(rand::random::<[u8; 32]>())
            };
            if let Err(error) = register_telegram_webhook_locked(
                db, telegram_api, adapter, &bot, &token, &webhook_url(base_url, &bot), &secret,
            ).await {
                if bot.credential_source == "telegram_manager" {
                    return Err(error);
                }
                db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
                    doc! {"_id": bot_id, "is_active": true},
                    doc! {"$set": {"status": "failed", "webhook_registered": false, "updated_at": bson::DateTime::now()}},
                ).await?;
            }
            get_bot_for_user(db, bot_id, owner_id).await
        },
    ).await
}

async fn register_webhook_inner(
    db: &mongodb::Database,
    http_client: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    bot_token: &str,
    webhook_url: &str,
    webhook_secret: &str,
) -> AppResult<()> {
    let bot = get_bot(db, bot_id).await?;
    if !bot.is_active {
        return Err(AppError::ChannelBotInactive("Bot has been deleted".into()));
    }
    let setup = adapter
        .setup_bot_webhook(
            db,
            http_client,
            &bot,
            bot_token,
            webhook_url,
            webhook_secret,
        )
        .await;
    if let Err(error) = setup {
        if adapter.serializes_lifecycle() {
            db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
                doc! { "_id": bot_id, "is_active": true, "updated_at": bson::DateTime::from_chrono(bot.updated_at) },
                doc! { "$set": { "status": "failed", "webhook_registered": false, "updated_at": bson::DateTime::now() } },
            ).await?;
        }
        return Err(error);
    }

    store_webhook_registration(
        db,
        adapter,
        doc! { "_id": bot_id, "is_active": true, "updated_at": bson::DateTime::from_chrono(bot.updated_at) },
    ).await
}

async fn store_webhook_registration(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    filter: bson::Document,
) -> AppResult<()> {
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
            filter,
            doc! { "$set": {
                "status": status,
                "webhook_registered": registered,
                "updated_at": now,
            }},
        )
        .await?;

    Ok(())
}

pub(crate) async fn mark_manager_channel_failed(
    db: &mongodb::Database,
    bot_id: &str,
) -> AppResult<()> {
    db.collection::<ChannelBot>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": bot_id, "is_active": true, "credential_source": "telegram_manager" },
            doc! { "$set": {
                "status": "failed", "webhook_registered": false,
                "error": super::telegram_new_admin::MANAGER_WEBHOOK_ERROR,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
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
            doc! { "_id": bot_id, "$or": [
                { "platform": { "$ne": "telegram" } },
                { "is_active": true, "status": "pending", "webhook_registered": false }
            ] },
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

/// List active personal and administered-org bots in one newest-first list.
pub async fn list_all_bots(db: &mongodb::Database, actor: &str) -> AppResult<Vec<ChannelBot>> {
    use crate::models::user::{COLLECTION_NAME as USERS, User};

    let memberships = super::org_service::list_memberships_for_member(db, actor, false).await?;
    let org_ids: Vec<_> = memberships
        .into_iter()
        .filter(|membership| membership.role.can_admin())
        .map(|membership| membership.org_user_id)
        .collect();
    let mut owner_ids = vec![actor.to_string()];
    if !org_ids.is_empty() {
        // Match the single-org ACL: a membership must still point to an org.
        let orgs: Vec<User> = db
            .collection::<User>(USERS)
            .find(doc! { "_id": { "$in": org_ids }, "user_type": "org" })
            .await?
            .try_collect()
            .await?;
        owner_ids.extend(orgs.into_iter().map(|org| org.id));
    }

    Ok(db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .find(doc! { "user_id": { "$in": owner_ids }, "is_active": true })
        .sort(doc! { "created_at": -1, "_id": 1 })
        .await?
        .try_collect()
        .await?)
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
#[allow(clippy::too_many_arguments)]
pub async fn delete_bot(
    db: &mongodb::Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    encryption_keys: &EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    user_id: &str,
) -> AppResult<Option<&'static str>> {
    let bot = get_bot_for_user(db, bot_id, user_id).await?;
    if bot.platform == "telegram" {
        return super::telegram_new_service::with_operation(
            db,
            &format!("telegram-manager-identity:{}", bot.platform_bot_id),
            delete_bot_inner(db, http_client, encryption_keys, adapter, bot_id, user_id),
        )
        .await;
    }
    if bot.platform == "telegram-new" {
        return super::telegram_new_service::with_operation(db, &format!("telegram-child:{}", bot.platform_bot_id), async {
            db.collection::<bson::Document>(crate::models::telegram_bot_request::MANAGED_BOTS).update_many(doc! {"telegram_bot_id": bot.platform_bot_id.parse::<i64>().unwrap_or_default()}, doc! {"$set": {"retired": true}}).await?;
            delete_bot_inner(db, http_client, encryption_keys, adapter, bot_id, user_id).await?;
            db.collection::<bson::Document>(crate::models::telegram_bot_request::COLLECTION_NAME).update_one(doc! {"_id": bot_id}, doc! {"$set": {"status": "cancelled", "active": false}, "$inc": {"revision": 1}}).await?;
            let service = super::telegram_new_service::TelegramNewService {
                db, config, keys: encryption_keys, api: super::telegram_new_api::TelegramApi::new(http_client),
            };
            Ok(Some(service.remove_owned_webhook(&bot).await.unwrap_or("failed")))
        }).await;
    }
    delete_bot_serialized(db, http_client, encryption_keys, adapter, bot_id, user_id).await
}

async fn delete_bot_serialized(
    db: &mongodb::Database,
    http_client: &reqwest::Client,
    encryption_keys: &EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    user_id: &str,
) -> AppResult<Option<&'static str>> {
    super::channel_retry_ingress::with_lifecycle(
        db,
        adapter.serializes_lifecycle(),
        bot_id,
        async {
            let work = delete_bot_inner(db, http_client, encryption_keys, adapter, bot_id, user_id);
            if adapter.serializes_lifecycle() {
                super::channel_retry_ingress::with_ingress(db, bot_id, work).await
            } else {
                work.await
            }
        },
    )
    .await
}

async fn delete_bot_inner(
    db: &mongodb::Database,
    http_client: &reqwest::Client,
    encryption_keys: &EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    user_id: &str,
) -> AppResult<Option<&'static str>> {
    let bot = get_bot_for_user(db, bot_id, user_id).await?;

    // Best-effort webhook deregistration
    if bot.platform != "telegram-new"
        && bot.credential_source != "connection"
        && bot.credential_source != "telegram_manager"
        && bot.webhook_registered
        && !adapter.serializes_lifecycle()
        && (bot.platform != "telegram" || bot.is_active)
        && (bot.platform != "telegram"
            || !is_telegram_manager_identity(db, &bot.platform_bot_id).await?)
        && let Ok(token) = decrypt_bot_token(encryption_keys, &bot).await
        && (bot.platform != "telegram"
            || verify_telegram_token_identity(http_client, adapter, &bot, &token)
                .await
                .is_ok())
    {
        let _ = adapter
            .remove_bot_webhook(db, http_client, &bot, &token)
            .await;
    }

    let now = bson::DateTime::from_chrono(Utc::now());

    let connection_webhook = bot.credential_source == "connection"
        && super::channel_connection_webhook_service::supports(adapter);

    // Keep possible remote subscriptions eligible for cleanup after deletion.
    // A handover or edit during webhook cleanup must not deactivate the new owner’s bot.
    let deleted = db.collection::<ChannelBot>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": bot_id, "user_id": user_id, "updated_at": bson::DateTime::from_chrono(bot.updated_at) },
            doc! { "$set": {
                "is_active": false,
                "status": "inactive",
                "webhook_registered": connection_webhook,
                "updated_at": now,
            }},
        )
        .await?;
    if deleted.matched_count != 1 {
        return Err(AppError::Conflict(
            "Channel bot changed during deletion; retry with its current state".into(),
        ));
    }

    // Deactivate all conversations tied to this bot
    db.collection::<mongodb::bson::Document>(CONVERSATIONS)
        .update_many(
            doc! {
                "channel_bot_id": bot_id,
                "$or": [
                    { "user_id": user_id },
                    { "user_id": { "$exists": false } },
                    { "user_id": bson::Bson::Null },
                ],
            },
            doc! { "$set": {
                "is_active": false,
                "updated_at": now,
            }},
        )
        .await?;

    if adapter.serializes_lifecycle() {
        for collection in [
            crate::models::channel_email::SENDS,
            crate::models::channel_email::RECEIPTS,
            crate::models::channel_email::BATCHES,
        ] {
            db.collection::<bson::Document>(collection)
                .delete_many(doc! { "bot_id": bot_id, "user_id": user_id })
                .await?;
        }
    }
    let cleanup = if connection_webhook {
        let result = super::channel_connection_webhook_service::remove(
            db,
            encryption_keys,
            http_client,
            adapter,
            &bot,
        )
        .await;
        if result.is_ok() {
            db.collection::<ChannelBot>(COLLECTION_NAME)
                .update_one(
                    doc! { "_id": bot_id, "is_active": false },
                    doc! { "$set": { "webhook_registered": false } },
                )
                .await?;
        }
        Some(if result.is_ok() { "removed" } else { "failed" })
    } else if adapter.serializes_lifecycle() {
        let result = async {
            let token = zeroize::Zeroizing::new(decrypt_bot_token(encryption_keys, &bot).await?);
            adapter
                .remove_bot_webhook(db, http_client, &bot, &token)
                .await
        }
        .await;
        Some(if result.is_ok() { "removed" } else { "failed" })
    } else if bot.credential_source == "platform" && bot.platform != "telegram-new" {
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
    let descriptor = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?;
    let platform =
        super::platform_credential_service::load_decrypted(db, keys, &descriptor).await?;
    let token = zeroize::Zeroizing::new(decrypt_bot_token(keys, bot).await?);
    let credentials = BotCredentials {
        billing: None,
        token: &token,
        platform_bot_id: Some(&bot.platform_bot_id),
        platform_secrets: Some(&platform),
    };
    adapter
        .remove_managed_webhook_override(http, &credentials, bot)
        .await?;
    Ok("removed")
}

/// Verify/repair under the same fence as credential rotation and deletion.
pub async fn verify_serialized_bot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    owner_id: &str,
    url: &str,
) -> AppResult<ChannelBot> {
    super::channel_retry_ingress::with_lifecycle(db, true, bot_id, async {
        let bot = get_bot_for_user(db, bot_id, owner_id).await?;
        if !bot.is_active {
            return Err(AppError::ChannelBotInactive("Bot has been deleted".into()));
        }
        let token = zeroize::Zeroizing::new(decrypt_bot_token(keys, &bot).await?);
        adapter
            .verify_bot_token(
                http,
                &BotCredentials {
                    billing: None,
                    token: &token,
                    platform_bot_id: Some(&bot.platform_bot_id),
                    platform_secrets: None,
                },
            )
            .await?;
        adapter.validate_stored_verification(&bot)?;
        register_webhook_inner(db, http, adapter, bot_id, &token, url, "").await?;
        get_bot_for_user(db, bot_id, owner_id).await
    })
    .await
}

/// Acquire all Aurinko effect fences before changing owner or bot state.
/// A busy request leaves the owner able to authenticate and retry deletion.
/// Hard deletion removes credentials locally; upstream subscriptions become inert.
pub async fn delete_owner_aurinko_channels(db: &mongodb::Database, owner: &str) -> AppResult<()> {
    use super::coordination_service::{EventDedupClaimResult, EventDedupStore};
    let bots: Vec<ChannelBot> = db
        .collection::<ChannelBot>(COLLECTION_NAME)
        .find(doc! { "user_id": owner, "platform": "aurinko" })
        .await?
        .try_collect()
        .await?;
    let mut claims = Vec::new();
    let result = async {
        for bot in &bots {
            for namespace in ["channel-bot-lifecycle", "channel-bot-ingress"] {
                match EventDedupStore::claim(
                    db,
                    namespace,
                    &bot.id,
                    "mutation",
                    std::time::Duration::from_secs(120),
                )
                .await?
                {
                    EventDedupClaimResult::Claimed(claim) => claims.push(claim),
                    EventDedupClaimResult::Duplicate => {
                        return Err(super::channel_retry_ingress::retry_later());
                    }
                }
            }
        }
        db.collection::<ChannelBot>(COLLECTION_NAME)
            .delete_many(doc! { "user_id": owner, "platform": "aurinko" })
            .await?;
        for collection in [
            CONVERSATIONS,
            crate::models::channel_message::COLLECTION_NAME,
        ] {
            db.collection::<bson::Document>(collection)
                .delete_many(doc! { "user_id": owner, "platform": "aurinko" })
                .await?;
        }
        Ok(())
    }
    .await;
    for claim in claims {
        EventDedupStore::release(db, &claim).await?;
    }
    result
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
        fn media_capabilities(&self) -> crate::services::channel_platform::MediaCapabilities {
            crate::services::channel_platform::MediaCapabilities::NONE
        }
        fn outbound_capabilities(&self) -> crate::services::channel_platform::OutboundCapabilities {
            crate::services::channel_platform::OutboundCapabilities::NONE
        }
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
            x_events: None,
            last_verification: None,
            ownership_version: 0,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "lark".to_string(),
            label: "Test Bot".to_string(),
            credential_source: "user".to_string(),
            connection_id: None,
            poll_cursor: None,
            poll_lease_until: None,
            last_polled_at: None,
            poll_backoff_until: None,
            poll_error_count: 0,
            last_poll_notice: None,
            error: None,
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
    async fn legacy_lifecycle_preserves_inactive_patch_and_manual_registration() {
        let db = crate::test_utils::connect_test_database("legacy_channel_lifecycle")
            .await
            .expect("real Mongo required");
        let keys = test_encryption_keys();
        let http = reqwest::Client::new();
        let mut bot = make_lark_bot(&keys, "app:secret").await;
        bot.is_active = false;
        db.collection::<ChannelBot>(COLLECTION_NAME)
            .insert_one(&bot)
            .await
            .unwrap();
        let adapter = crate::services::channel_adapters::lark::LarkFamilyAdapter::lark(Arc::new(
            crate::services::provider_token_exchange_service::TokenExchangeCache::new(),
        ));
        let patched = update_bot(
            &db,
            &keys,
            &http,
            &adapter,
            &bot.id,
            &bot.user_id,
            UpdateBotParams {
                x_events: None,
                label: Some("Renamed"),
                bot_token: None,
                app_id: None,
                app_secret: None,
                verification_token: None,
                encrypt_key: SecretPatch::Unchanged,
            },
        )
        .await
        .unwrap();
        assert_eq!(patched.label, "Renamed");
        assert!(!patched.is_active);
        register_webhook(
            &db,
            &http,
            &adapter,
            &bot.id,
            "app:secret",
            "https://nyxid.test/hook",
            "unused",
        )
        .await
        .unwrap();
        let registered = get_bot(&db, &bot.id).await.unwrap();
        assert_eq!(registered.status, "pending_webhook");
        assert!(!registered.webhook_registered);
        assert!(!registered.is_active);
        assert_eq!(registered.label, "Renamed");
        assert_eq!(
            db.collection::<bson::Document>("coordination_event_dedup")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
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
            x_events: None,
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
            x_events: None,
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
            x_events: None,
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
            x_events: None,
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
            fn media_capabilities(&self) -> crate::services::channel_platform::MediaCapabilities {
                crate::services::channel_platform::MediaCapabilities::NONE
            }
            fn outbound_capabilities(
                &self,
            ) -> crate::services::channel_platform::OutboundCapabilities {
                crate::services::channel_platform::OutboundCapabilities::NONE
            }
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
            x_events: None,
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
