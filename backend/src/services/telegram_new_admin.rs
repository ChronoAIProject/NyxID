use bson::doc;
use serde_json::json;
use std::collections::BTreeMap;
use zeroize::Zeroizing;

use super::channel_adapters::telegram_new::{MANAGER_TOKEN, PLATFORM, credential_descriptor};
use super::{platform_credential_service as credentials, telegram_new_service::TelegramNewService};
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{COLLECTION_NAME as BOTS, ChannelBot};
use crate::models::platform_credential::{COLLECTION_NAME as CREDENTIALS, PlatformCredential};

pub(super) const MANAGER_WEBHOOK_ERROR: &str = "The Telegram manager webhook is unavailable. Ask an administrator to save its Platform Credentials configuration again.";

pub(super) const MANAGER_ALLOWED_UPDATES: [&str; 5] = [
    "message",
    "edited_message",
    "channel_post",
    "callback_query",
    "managed_bot",
];

pub(super) fn validate_manager_webhook(
    webhook: &serde_json::Value,
    callback: &str,
) -> AppResult<()> {
    if webhook["url"] != callback
        || !MANAGER_ALLOWED_UPDATES.iter().all(|kind| {
            webhook["allowed_updates"]
                .as_array()
                .is_some_and(|types| types.iter().any(|value| value == kind))
        })
    {
        return Err(AppError::ChannelPlatformError(MANAGER_WEBHOOK_ERROR.into()));
    }
    Ok(())
}

impl TelegramNewService<'_> {
    pub async fn configure_manager(
        &self,
        actor: &str,
        fields: &BTreeMap<String, Option<Zeroizing<String>>>,
        regenerate: bool,
    ) -> AppResult<()> {
        super::telegram_new_service::with_operation(
            self.db,
            "telegram-new-manager-configuration",
            self.configure_manager_inner(actor, fields, regenerate),
        )
        .await
    }

    async fn configure_manager_inner(
        &self,
        actor: &str,
        fields: &BTreeMap<String, Option<Zeroizing<String>>>,
        regenerate: bool,
    ) -> AppResult<()> {
        if regenerate {
            return Err(AppError::ValidationError("Telegram bot creation keeps a stable webhook secret. Use manager-token rotation without regenerating the webhook secret.".into()));
        }
        if fields.keys().any(|key| key != MANAGER_TOKEN)
            || fields.get(MANAGER_TOKEN).is_some_and(Option::is_none)
        {
            return Err(AppError::ValidationError("Supply a manager bot token, or use Clear configuration when no managed bots remain.".into()));
        }
        let old = credentials::load_decrypted(
            self.db,
            self.keys,
            &super::channel_adapters::telegram_new::credential_descriptor(),
        )
        .await?;
        let token = fields
            .get(MANAGER_TOKEN)
            .and_then(Option::as_ref)
            .map(|v| v.trim())
            .or_else(|| old.get(MANAGER_TOKEN))
            .ok_or_else(|| AppError::ValidationError("Manager bot token is required".into()))?;
        if token.is_empty() || token.len() > 512 {
            return Err(AppError::ValidationError(
                "Invalid manager bot token".into(),
            ));
        }
        let identity = self.api.call(token, "getMe", json!({})).await?;
        let (id, username) =
            super::telegram_new_service::bot_identity(&identity).ok_or_else(|| {
                AppError::ValidationError("Telegram returned an invalid manager identity".into())
            })?;
        super::telegram_new_service::with_operation(
            self.db,
            &format!("telegram-manager-identity:{id}"),
            async {
        if identity["can_manage_bots"] != true {
            return Err(AppError::ValidationError(
                "Enable management of other bots in BotFather for this manager first.".into(),
            ));
        }
        if old
            .get("manager_bot_id")
            .is_some_and(|value| value != id.to_string())
        {
            return Err(AppError::Conflict("This configuration belongs to a different manager bot. Clear it before switching managers.".into()));
        }
        if self
            .config
            .telegram_bot_token
            .as_deref()
            .and_then(|token| token.split(':').next())
            .and_then(|id| id.parse::<i64>().ok())
            == Some(id)
        {
            return Err(AppError::Conflict(
                "The notification bot cannot also be the Telegram bot creation manager.".into(),
            ));
        }
        if self.db.collection::<ChannelBot>(BOTS).find_one(doc! {"platform": {"$in": ["telegram", PLATFORM]}, "platform_bot_id": id.to_string(), "is_active": true, "credential_source": {"$ne": "telegram_manager"}}).await?.is_some() {
            return Err(AppError::Conflict("A registered channel bot cannot also be the manager.".into()));
        }
        let callback = self.manager_callback();
        let url = reqwest::Url::parse(&callback)
            .map_err(|_| AppError::ValidationError("Invalid public callback URL".into()))?;
        if url.scheme() != "https" {
            return Err(AppError::ValidationError(
                "Telegram bot creation requires an HTTPS public backend URL.".into(),
            ));
        }
        let webhook = self.api.call(token, "getWebhookInfo", json!({})).await?;
        if webhook["url"]
            .as_str()
            .is_some_and(|existing| !existing.is_empty() && existing != callback)
        {
            return Err(AppError::Conflict("This manager already has a webhook for another application or environment. Use a dedicated manager.".into()));
        }
        credentials::update(
            self.db,
            self.keys,
            &credential_descriptor(),
            actor,
            fields,
            false,
        )
        .await?;
        let observation_id = old.get("observation_id").map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.db.collection::<PlatformCredential>(CREDENTIALS).update_one(doc! {"provider": PLATFORM}, doc! {"$set": {"fields.manager_bot_id": id.to_string(), "fields.manager_username": username, "fields.webhook_ready": "false", "fields.observation_id": observation_id}}).await?;
        let saved = credentials::load_decrypted(self.db, self.keys, &super::channel_adapters::telegram_new::credential_descriptor()).await?;
        let secret = saved
            .get(credentials::VERIFY_TOKEN_FIELD)
            .ok_or_else(|| AppError::Internal("Manager webhook secret is missing".into()))?;
        self.api.call(token, "setWebhook", json!({"url": callback, "secret_token": secret, "allowed_updates": MANAGER_ALLOWED_UPDATES, "max_connections": 1})).await?;
        let webhook = self.api.call(token, "getWebhookInfo", json!({})).await?;
        validate_manager_webhook(&webhook, &callback)?;
        self.db
            .collection::<PlatformCredential>(CREDENTIALS)
            .update_one(
                doc! {"provider": PLATFORM, "fields.manager_bot_id": id.to_string()},
                doc! {"$set": {"fields.webhook_ready": "true", "fields.observation_started_at": old.get("observation_started_at").map(str::to_owned).unwrap_or_else(|| chrono::Utc::now().timestamp().to_string())}},
            )
            .await?;
        Ok(())
            },
        ).await
    }

    pub fn manager_callback(&self) -> String {
        format!(
            "{}/api/v1/webhooks/channel/telegram-new/manager",
            self.config.base_url.trim_end_matches('/')
        )
    }

    pub async fn clear_manager(&self) -> AppResult<()> {
        super::telegram_new_service::with_operation(
            self.db,
            "telegram-new-manager-configuration",
            self.clear_manager_inner(),
        )
        .await
    }

    async fn clear_manager_inner(&self) -> AppResult<()> {
        let row = credentials::load(self.db, &credential_descriptor()).await?;
        let id = row
            .as_ref()
            .and_then(|row| row.fields.get("manager_bot_id"));
        if let Some(id) = id {
            return super::telegram_new_service::with_operation(
                self.db,
                &format!("telegram-manager-identity:{id}"),
                self.clear_manager_configuration(),
            )
            .await;
        }
        self.clear_manager_configuration().await
    }

    async fn clear_manager_configuration(&self) -> AppResult<()> {
        self.expire().await?;
        if self
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {
                "credential_source": "telegram_manager", "is_active": true,
            })
            .await?
            .is_some()
        {
            return Err(AppError::Conflict("Delete the manager's channel connection before clearing the Telegram manager configuration. Its channel routes depend on this configuration.".into()));
        }
        if self
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {"platform": PLATFORM, "is_active": true})
            .await?
            > 0
            || self
                .db
                .collection::<bson::Document>(crate::models::telegram_bot_request::COLLECTION_NAME)
                .count_documents(doc! {"active": true})
                .await?
                > 0
        {
            return Err(AppError::Conflict("Delete the managed channel bots and finish or cancel creation requests before clearing the manager.".into()));
        }
        let values = credentials::load_decrypted(
            self.db,
            self.keys,
            &super::channel_adapters::telegram_new::credential_descriptor(),
        )
        .await?;
        if let Some(token) = values.get(MANAGER_TOKEN)
            && let Err(error) = self.remove_manager_webhook(token).await
        {
            tracing::warn!(%error, "Telegram manager webhook cleanup failed; clearing saved configuration");
        }
        credentials::delete(self.db, &credential_descriptor()).await
    }

    async fn remove_manager_webhook(&self, token: &str) -> AppResult<()> {
        let webhook = self.api.call(token, "getWebhookInfo", json!({})).await?;
        if webhook["url"] == self.manager_callback() {
            self.api
                .call(
                    token,
                    "deleteWebhook",
                    json!({"drop_pending_updates": false}),
                )
                .await?;
        }
        Ok(())
    }
}
