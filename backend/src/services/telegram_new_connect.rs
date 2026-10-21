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
            self.connect_inner(actor, id, expected_bot, revision, None),
        )
        .await
    }

    pub(crate) async fn connect_claimed(
        &self,
        request: &TelegramBotRequest,
        bot: i64,
    ) -> AppResult<ChannelBot> {
        super::telegram_new_service::with_operation(
            self.db,
            &format!("telegram-child:{bot}"),
            self.connect_inner(
                &request.actor_user_id,
                &request.id,
                bot,
                request.revision,
                Some(request.connection_generation),
            ),
        )
        .await
    }

    async fn connect_inner(
        &self,
        actor: &str,
        id: &str,
        expected_bot: i64,
        revision: i64,
        claimed_generation: Option<i64>,
    ) -> AppResult<ChannelBot> {
        let request = self.get(actor, id).await?;
        if claimed_generation.is_some_and(|generation| generation != request.connection_generation)
        {
            return Err(AppError::Conflict("The connection claim changed".into()));
        }
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
        super::platform_provisioning_service::admit_request(self.db, &request, true).await?;
        let bot = if let Some(saved) = self
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id": id})
            .await?
        {
            saved
        } else {
            let (manager_id, _, values) = self
                .for_manager(request.manager_bot_id)
                .await?
                .manager()
                .await?;
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
            super::platform_provisioning_service::admit_request(self.db, &request, true).await?;
            let token = self
                .api
                .token(
                    values
                        .get(MANAGER_TOKEN)
                        .ok_or_else(|| AppError::Conflict("Manager token is unavailable".into()))?,
                    expected_bot,
                )
                .await?;
            super::platform_provisioning_service::admit_request(self.db, &request, true).await?;
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
                manager_instance: None,
                diagnostics: Default::default(),
                x_events: None,
                last_verification: None,
                ownership_version: 0,
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
            super::channel_bot_service::insert_approved_telegram_bot(
                self.db,
                &bot,
                self.config.channel_relay_max_bots_per_user,
                &request,
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
            self.finish_request(&request).await?;
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
        super::platform_provisioning_service::admit_request(self.db, &request, true).await?;
        let webhook = self.api.call(&token, "getWebhookInfo", json!({})).await?;
        if webhook["url"]
            .as_str()
            .is_some_and(|url| !url.is_empty() && url != callback)
        {
            return Err(AppError::Conflict("This bot has a webhook for another application. Clear that webhook before retrying.".into()));
        }
        super::platform_provisioning_service::admit_request(self.db, &request, true).await?;
        self.api.call(&token, "setWebhook", json!({"url": callback, "secret_token": secret, "allowed_updates": ["message", "edited_message", "channel_post"]})).await?;
        super::platform_provisioning_service::admit_request(self.db, &request, true).await?;
        let webhook = self.api.call(&token, "getWebhookInfo", json!({})).await?;
        if webhook["url"] != callback {
            return Err(AppError::ChannelPlatformError(
                "Telegram webhook was not confirmed. Retry connecting the saved bot.".into(),
            ));
        }
        let db = self.db.clone();
        let captured = request.clone();
        let mut session = db.client().start_session().await?;
        let result = session.start_transaction().and_run2(async move |session| {
            let outcome: AppResult<_> = async {
                super::platform_provisioning_service::fence_request(&db, &captured, true, session).await?;
                Ok(db.collection::<ChannelBot>(BOTS).update_one(doc! {"_id": &captured.id, "is_active": true, "status": "pending"}, doc! {"$set": {"status": "active", "webhook_registered": true, "updated_at": bson::DateTime::now()}}).session(&mut *session).await?)
            }.await;
            super::api_key_mutation_service::transaction_result(outcome)
        }).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
        if result.matched_count != 1 {
            let saved = super::channel_bot_service::get_bot(self.db, id).await?;
            if !saved.is_active || saved.status != "active" {
                let _ = self.remove_owned_webhook(&bot).await;
                return Err(AppError::Conflict(
                    "The bot state changed during setup. Refresh its saved state.".into(),
                ));
            }
        }
        self.finish_request(&request).await?;
        // Tracked requests reserve their completion notification even when the
        // browser has gone away. The bot-link sweep recovers any transient error.
        if let Err(error) = super::channel_connect_link_service::complete(self.db, id).await {
            tracing::warn!(channel_connect_link_id = id, %error, "Channel setup completion remains pending");
        }
        super::channel_bot_service::get_bot(self.db, id).await
    }

    async fn finish_request(&self, captured: &TelegramBotRequest) -> AppResult<()> {
        let id = captured.id.as_str();
        if captured.setup_authority.is_some() {
            let db = self.db.clone();
            let request = captured.clone();
            let mut session = db.client().start_session().await?;
            session.start_transaction().and_run2(async move |session| {
                let result: AppResult<()> = async {
                    super::platform_provisioning_service::fence_request(&db, &request, true, session).await?;
                    db.collection::<TelegramBotRequest>(REQUESTS).update_one(
                        doc! {"_id": &request.id, "status": "provisioning", "connection_generation": request.connection_generation},
                        doc! {"$set": {"status": "connected", "active": false}, "$unset": {"connection_error": "", "next_connection_attempt_at": ""}, "$inc": {"revision": 1_i64}}
                    ).session(&mut *session).await?;
                    if let Some(a) = &request.setup_authority {
                        let transition = db.collection::<bson::Document>(crate::models::platform_setup::COLLECTION_NAME).update_one(
                            doc! {"_id": &a.setup_id, "revision": a.setup_revision, "approved_digest": &a.digest, "state": "provisioning"},
                            doc! {"$set": {"state": "ready_for_test", "updated_at": bson::DateTime::now()}}
                        ).session(&mut *session).await?;
                        if transition.modified_count == 1 {
                        let installation = db.collection::<bson::Document>(crate::models::platform_installation::COLLECTION_NAME).update_one(
                            doc! {"setup_id": &a.setup_id, "channel_bot_id": &request.id, "state": "provisioning"},
                            doc! {"$set": {"state": "ready_for_test", "updated_at": bson::DateTime::now()}}
                        ).session(&mut *session).await?;
                        if installation.modified_count != 1 {
                            return Err(crate::errors::AppError::Conflict("Installation transition changed".into()));
                        }
                        super::platform_setup_notice_service::queue(&db, a, "ready_for_test", session).await?;
                        }
                    }
                    Ok(())
                }.await;
                super::api_key_mutation_service::transaction_result(result)
            }).await.map_err(super::api_key_mutation_service::map_transaction_error)?;
            return Ok(());
        }
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
            if let Ok((_, _, values)) = self
                .for_manager(request.manager_bot_id)
                .await?
                .manager()
                .await
                && let Some(token) = values.get(MANAGER_TOKEN)
            {
                let username = request.bot_username.as_deref().unwrap_or_default();
                let _ = self.api.call(token, "sendMessage", json!({
                    "chat_id": request.telegram_user_id,
                    "text": format!("@{username} is connected to NyxID. Setup is complete. Tap Open your bot and press Start to talk to it. You can choose an AI agent for replies in Bot settings."),
                    "reply_markup": {"inline_keyboard": [
                        [{"text": "Open your bot", "url": format!("https://t.me/{username}")}],
                        [{"text": "Bot settings", "url": format!("{}/channel-bots/{id}", self.config.frontend_url.trim_end_matches('/'))}],
                    ]},
                    "link_preview_options": {"is_disabled": true},
                })).await;
            }
        }
        Ok(())
    }
}
