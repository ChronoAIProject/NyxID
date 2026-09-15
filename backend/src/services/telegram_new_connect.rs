use bson::doc;
use chrono::Utc;
use serde_json::json;
use zeroize::Zeroizing;

use super::channel_adapters::telegram_new::{MANAGER_TOKEN, PLATFORM};
use super::telegram_new_service::{TelegramNewService, bot_identity, hash};
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{COLLECTION_NAME as BOTS, ChannelBot};
use crate::models::telegram_bot_request::{
    COLLECTION_NAME as REQUESTS, MANAGED_BOTS, TelegramBotRequest, TelegramRequestStatus as Status,
};

impl TelegramNewService<'_> {
    pub async fn connect(
        &self,
        actor: &str,
        id: &str,
        expected_bot: i64,
        revision: i64,
    ) -> AppResult<ChannelBot> {
        super::telegram_new_service::with_operation(
            self.db,
            &format!("telegram-child:{expected_bot}"),
            self.connect_inner(actor, id, expected_bot, revision),
        )
        .await
    }

    async fn connect_inner(
        &self,
        actor: &str,
        id: &str,
        expected_bot: i64,
        revision: i64,
    ) -> AppResult<ChannelBot> {
        let request = self.get(actor, id).await?;
        if request.telegram_bot_id != Some(expected_bot)
            || !matches!(
                request.status,
                Status::Ready | Status::Provisioning | Status::Connected
            )
        {
            return Err(AppError::Conflict(
                "Complete the bot creation in Telegram before connecting it.".into(),
            ));
        }
        if request.status == Status::Ready && request.revision != revision {
            return Err(AppError::Conflict(
                "The request changed. Refresh before confirming.".into(),
            ));
        }
        let bot = if let Some(saved) = self
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id": id})
            .await?
        {
            saved
        } else {
            let (manager_id, _, values) = self.manager().await?;
            if manager_id != request.manager_bot_id
                || values.get("observation_id") != Some(request.observation_id.as_str())
            {
                return Err(AppError::Conflict(
                    "The manager changed. Start a new request.".into(),
                ));
            }
            let manager_token = values
                .get(MANAGER_TOKEN)
                .ok_or_else(|| AppError::Conflict("Manager token is unavailable".into()))?;
            self.verify_creation_delivery(&request, manager_token)
                .await?;
            let token = self
                .api
                .token(
                    values
                        .get(MANAGER_TOKEN)
                        .ok_or_else(|| AppError::Conflict("Manager token is unavailable".into()))?,
                    expected_bot,
                )
                .await?;
            let identity = self.api.call(&token, "getMe", json!({})).await?;
            let (verified_id, username) = bot_identity(&identity).ok_or_else(|| {
                AppError::ChannelPlatformError("Telegram returned an invalid bot identity".into())
            })?;
            if verified_id != expected_bot {
                return Err(AppError::Conflict(
                    "Telegram returned a different bot identity".into(),
                ));
            }
            let secret = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
            let bot = ChannelBot {
                id: id.into(),
                user_id: request.owner_user_id.clone(),
                platform: PLATFORM.into(),
                label: request.label.clone(),
                credential_source: "platform".into(),
                connection_id: None,
                poll_cursor: None,
                poll_lease_until: None,
                last_polled_at: None,
                poll_backoff_until: None,
                poll_error_count: 0,
                last_poll_notice: None,
                error: None,
                registration_pin_encrypted: None,
                webhook_secret_encrypted: Some(self.keys.encrypt(secret.as_bytes()).await?),
                managed_setup: None,
                bot_token_encrypted: self.keys.encrypt(token.as_bytes()).await?,
                platform_bot_id: verified_id.to_string(),
                platform_bot_username: username,
                webhook_registered: false,
                webhook_secret_hash: hash(&secret),
                app_id: None,
                app_secret_encrypted: None,
                lark_verification_token_encrypted: None,
                lark_encrypt_key_encrypted: None,
                public_key: None,
                status: "pending".into(),
                is_active: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            self.check_owner(actor, &request.owner_user_id).await?;
            super::channel_bot_service::insert_registered_bot(
                self.db,
                &bot,
                self.config.channel_relay_max_bots_per_user,
                Some(revision),
            )
            .await?;
            super::channel_bot_service::get_bot(self.db, id).await?
        };
        if !bot.is_active || !matches!(bot.status.as_str(), "pending" | "active") {
            return Err(AppError::Conflict(
                "This bot was deleted or suspended. It cannot be reactivated by retrying setup."
                    .into(),
            ));
        }
        if bot.status == "active" {
            self.finish_request(id).await?;
            return Ok(bot);
        }
        let event = self.db.collection::<bson::Document>(MANAGED_BOTS).find_one(doc! {"manager_bot_id": request.manager_bot_id, "telegram_bot_id": expected_bot, "telegram_user_id": request.telegram_user_id, "revision": request.manager_revision}).await?;
        if event.is_none() {
            return Err(AppError::Conflict(
                "Telegram management changed. This connection needs fresh approval.".into(),
            ));
        }
        self.check_owner(actor, &bot.user_id).await?;
        let token =
            Zeroizing::new(super::channel_bot_service::decrypt_bot_token(self.keys, &bot).await?);
        let encrypted = bot
            .webhook_secret_encrypted
            .as_deref()
            .ok_or_else(|| AppError::Internal("Saved webhook secret is missing".into()))?;
        let secret = Zeroizing::new(self.keys.decrypt(encrypted).await?);
        let secret = std::str::from_utf8(&secret)
            .map_err(|_| AppError::Internal("Invalid webhook secret encoding".into()))?;
        let callback = format!(
            "{}/api/v1/webhooks/channel/{PLATFORM}/{}",
            self.config.base_url.trim_end_matches('/'),
            bot.id
        );
        let webhook = self.api.call(&token, "getWebhookInfo", json!({})).await?;
        if webhook["url"]
            .as_str()
            .is_some_and(|url| !url.is_empty() && url != callback)
        {
            return Err(AppError::Conflict("This bot has a webhook for another application. Clear that webhook before retrying.".into()));
        }
        self.api.call(&token, "setWebhook", json!({"url": callback, "secret_token": secret, "allowed_updates": ["message", "edited_message", "channel_post"]})).await?;
        let webhook = self.api.call(&token, "getWebhookInfo", json!({})).await?;
        if webhook["url"] != callback {
            return Err(AppError::ChannelPlatformError(
                "Telegram webhook was not confirmed. Retry connecting the saved bot.".into(),
            ));
        }
        let result = self.db.collection::<ChannelBot>(BOTS).update_one(doc! {"_id": id, "is_active": true, "status": "pending"}, doc! {"$set": {"status": "active", "webhook_registered": true, "updated_at": bson::DateTime::now()}}).await?;
        if result.matched_count != 1 {
            let saved = super::channel_bot_service::get_bot(self.db, id).await?;
            if !saved.is_active || saved.status != "active" {
                let _ = self.remove_owned_webhook(&bot).await;
                return Err(AppError::Conflict(
                    "The bot state changed during setup. Refresh its saved state.".into(),
                ));
            }
        }
        self.finish_request(id).await?;
        super::channel_bot_service::get_bot(self.db, id).await
    }

    async fn finish_request(&self, id: &str) -> AppResult<()> {
        let result = self.db
            .collection::<TelegramBotRequest>(REQUESTS)
            .update_one(
                doc! {"_id": id, "status": "provisioning"},
                doc! {"$set": {"status": "connected", "active": false}, "$unset": {"connection_error": "", "next_connection_attempt_at": ""}, "$inc": {"revision": 1}},
            )
            .await?;
        if result.modified_count == 1
            && let Some(request) = self
                .db
                .collection::<TelegramBotRequest>(REQUESTS)
                .find_one(doc! {"_id": id, "auto_connect": true})
                .await?
        {
            super::audit_service::log_async(
                self.db.clone(),
                Some(request.actor_user_id.clone()),
                "channel_bot_created".into(),
                Some(
                    json!({"bot_id": id, "platform": PLATFORM, "owner_user_id": request.owner_user_id, "source": "telegram_creation"}),
                ),
                None,
                None,
                None,
                None,
            );
            if let Ok((_, _, values)) = self.manager().await
                && let Some(token) = values.get(MANAGER_TOKEN)
            {
                let username = request.bot_username.as_deref().unwrap_or_default();
                let _ = self.api.call(token, "sendMessage", json!({
                    "chat_id": request.telegram_user_id,
                    "text": format!("@{username} is connected to NyxID. Setup is complete. You can choose an AI agent in the bot's settings."),
                    "reply_markup": {"inline_keyboard": [[{"text": "Bot settings", "url": format!("{}/channel-bots/{id}", self.config.frontend_url.trim_end_matches('/'))}]]},
                    "link_preview_options": {"is_disabled": true},
                })).await;
            }
        }
        Ok(())
    }
}
