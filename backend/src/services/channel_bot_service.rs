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

#[derive(Clone, Copy)]
pub enum SecretPatch<'a> {
    Unchanged,
    Clear,
    Set(&'a str),
}

pub struct UpdateBotParams<'a> {
    pub label: Option<&'a str>,
    pub verification_token: Option<&'a str>,
    pub encrypt_key: SecretPatch<'a>,
    pub app_id: Option<&'a str>,
    pub app_secret: Option<&'a str>,
}

fn parse_lark_bot_credentials(bot_token: &str) -> AppResult<(&str, &str)> {
    bot_token
        .split_once(':')
        .ok_or_else(|| AppError::Internal("stored Lark/Feishu bot token is malformed".to_string()))
}

async fn maybe_rebuild_lark_bot_token(
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    params: &UpdateBotParams<'_>,
) -> AppResult<Option<Vec<u8>>> {
    if !matches!(bot.platform.as_str(), "lark" | "feishu")
        || (params.app_id.is_none() && params.app_secret.is_none())
    {
        return Ok(None);
    }

    let current_bot_token = decrypt_bot_token(encryption_keys, bot).await?;
    let (current_app_id, current_app_secret) = parse_lark_bot_credentials(&current_bot_token)?;
    let effective_app_id = params.app_id.unwrap_or(current_app_id);
    let effective_app_secret = params.app_secret.unwrap_or(current_app_secret);
    let composite = format!("{effective_app_id}:{effective_app_secret}");

    adapter
        .verify_bot_token(http_client, &composite)
        .await
        .map_err(|e| {
            AppError::ValidationError(format!("invalid {} app credentials: {e}", bot.platform))
        })?;

    Ok(Some(encryption_keys.encrypt(composite.as_bytes()).await?))
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
    verification_token: Option<&str>,
    encrypt_key: Option<&str>,
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

    if matches!(adapter.platform_id(), "lark" | "feishu")
        && verification_token
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(AppError::ValidationError(
            "Lark/Feishu Verification Token is required".to_string(),
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
    let lark_verification_token_encrypted = match verification_token {
        Some(token) => Some(encryption_keys.encrypt(token.as_bytes()).await?),
        None => None,
    };
    let lark_encrypt_key_encrypted = match encrypt_key {
        Some(key) => Some(encryption_keys.encrypt(key.as_bytes()).await?),
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
        lark_verification_token_encrypted,
        lark_encrypt_key_encrypted,
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

    if let Some(verification_token) = params.verification_token {
        let encrypted = encryption_keys
            .encrypt(verification_token.as_bytes())
            .await?;
        set_doc.insert(
            "lark_verification_token_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: encrypted,
            },
        );
    }

    match params.encrypt_key {
        SecretPatch::Unchanged => {}
        SecretPatch::Clear => {
            unset_doc.insert("lark_encrypt_key_encrypted", "");
        }
        SecretPatch::Set(value) => {
            let encrypted = encryption_keys.encrypt(value.as_bytes()).await?;
            set_doc.insert(
                "lark_encrypt_key_encrypted",
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: encrypted,
                },
            );
        }
    }

    if let Some(app_id) = params.app_id {
        set_doc.insert("app_id", app_id);
    }

    if let Some(app_secret) = params.app_secret {
        let encrypted = encryption_keys.encrypt(app_secret.as_bytes()).await?;
        set_doc.insert(
            "app_secret_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: encrypted,
            },
        );
    }

    if let Some(bot_token_encrypted) =
        maybe_rebuild_lark_bot_token(encryption_keys, http_client, adapter, &bot, &params).await?
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

    struct RecordingAdapter {
        seen_tokens: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl PlatformAdapter for RecordingAdapter {
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
            _bot_token: &str,
            _conversation_id: &str,
            _reply: &OutboundReply,
        ) -> AppResult<Option<String>> {
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
            bot_token: &str,
        ) -> AppResult<BotIdentity> {
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
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: Some("new_app"),
            app_secret: None,
        };

        let rebuilt =
            maybe_rebuild_lark_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
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
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: None,
            app_secret: Some("new_secret"),
        };

        let rebuilt =
            maybe_rebuild_lark_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
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
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: Some("new_app"),
            app_secret: Some("new_secret"),
        };

        let rebuilt =
            maybe_rebuild_lark_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
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
            label: Some("New Label"),
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: None,
            app_secret: None,
        };

        let rebuilt =
            maybe_rebuild_lark_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
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

    // ---- maybe_rebuild_lark_bot_token (non-lark platform) ----

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
                _bot_token: &str,
                _conversation_id: &str,
                _reply: &OutboundReply,
            ) -> AppResult<Option<String>> {
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
                _bot_token: &str,
            ) -> AppResult<BotIdentity> {
                unimplemented!()
            }
        }

        let adapter = TelegramAdapter;
        let mut bot = make_lark_bot(&encryption_keys, "some_token").await;
        bot.platform = "telegram".to_string();

        let params = UpdateBotParams {
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: Some("new_app"),
            app_secret: Some("new_secret"),
        };

        let result =
            maybe_rebuild_lark_bot_token(&encryption_keys, &http_client, &adapter, &bot, &params)
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
