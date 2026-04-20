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
use crate::services::channel_platform::{BotIdentity, PlatformAdapter};

/// Result of creating a bot: the persisted record plus the raw webhook secret
/// (shown once, never stored in cleartext).
pub struct CreateBotResult {
    pub bot: ChannelBot,
    pub webhook_secret: String,
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
    bot_token: &str,
    label: &str,
    app_id: Option<&str>,
    app_secret: Option<&str>,
    public_key: Option<&str>,
) -> AppResult<CreateBotResult> {
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

    // Slack requires the app's signing secret to verify webhook signatures.
    // Without it the bot can never receive any inbound message, so fail fast
    // at registration time. Mirrors the Lark/Feishu requirement of
    // `app_id`+`app_secret` enforced at the handler layer. Treat blank /
    // whitespace-only secrets as missing so non-frontend clients (CLI with
    // an empty env var, API callers passing `""`) can't register an unusable
    // bot.
    if adapter.platform_id() == "slack" && app_secret.map(|s| s.trim().is_empty()).unwrap_or(true) {
        return Err(AppError::ValidationError(
            "Slack signing secret is required (pass via app_secret)".to_string(),
        ));
    }

    // For Lark/Feishu, verify_bot_token expects "app_id:app_secret" format.
    // Build the effective token for verification matching what we'll store.
    let verify_token = if matches!(adapter.platform_id(), "lark" | "feishu") {
        match (app_id, app_secret) {
            (Some(id), Some(secret)) => format!("{id}:{secret}"),
            _ => bot_token.to_string(),
        }
    } else {
        bot_token.to_string()
    };

    // Verify the token with the platform to obtain bot identity
    let BotIdentity {
        platform_bot_id,
        platform_bot_username,
    } = adapter.verify_bot_token(http_client, &verify_token).await?;

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

    // For Lark/Feishu, store "app_id:app_secret" as the bot token so that
    // send_reply can exchange it for a tenant_access_token at send time.
    let effective_token = if matches!(adapter.platform_id(), "lark" | "feishu") {
        match (app_id, app_secret) {
            (Some(id), Some(secret)) => format!("{id}:{secret}"),
            _ => bot_token.to_string(),
        }
    } else {
        bot_token.to_string()
    };

    // Encrypt the bot token
    let bot_token_encrypted = encryption_keys.encrypt(effective_token.as_bytes()).await?;

    // Encrypt app secret if provided (Lark/Feishu)
    let app_secret_encrypted = match app_secret {
        Some(secret) => Some(encryption_keys.encrypt(secret.as_bytes()).await?),
        None => None,
    };

    let now = Utc::now();
    let bot = ChannelBot {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        platform: adapter.platform_id().to_string(),
        label: label.to_string(),
        bot_token_encrypted,
        platform_bot_id,
        platform_bot_username,
        webhook_registered: false,
        webhook_secret_hash: secret_hash,
        app_id: app_id.map(String::from),
        app_secret_encrypted,
        public_key: public_key.map(String::from),
        status: "pending".to_string(),
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    db.collection::<ChannelBot>(COLLECTION_NAME)
        .insert_one(&bot)
        .await?;

    Ok(CreateBotResult {
        bot,
        webhook_secret: raw_secret,
    })
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
    let auto_registered = matches!(adapter.platform_id(), "telegram");

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
) -> AppResult<()> {
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
