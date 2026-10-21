use bson::doc;
use serde_json::json;
use std::collections::BTreeMap;
use zeroize::Zeroizing;

use super::channel_adapters::telegram_new::{MANAGER_TOKEN, PLATFORM};
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
    /// Configure the manager as a first-party human administrator.
    pub async fn configure_manager(
        &self,
        actor: &str,
        fields: &BTreeMap<String, Option<Zeroizing<String>>>,
        regenerate: bool,
    ) -> AppResult<()> {
        self.configure_manager_for(actor, true, fields, regenerate)
            .await
    }

    /// `first_party` is checked under the manager operation, so a converted
    /// manager is never changed by an OAuth-app token that raced conversion.
    pub async fn configure_manager_for(
        &self,
        actor: &str,
        first_party: bool,
        fields: &BTreeMap<String, Option<Zeroizing<String>>>,
        regenerate: bool,
    ) -> AppResult<()> {
        super::telegram_new_service::with_operation(
            self.db,
            "telegram-new-manager-configuration",
            self.configure_manager_inner(actor, first_party, fields, regenerate),
        )
        .await
    }

    async fn configure_manager_inner(
        &self,
        actor: &str,
        first_party: bool,
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
        if self.manager_id.is_some() {
            if !first_party {
                return Err(AppError::Forbidden(
                    "First-party human administrator required".into(),
                ));
            }
            let mut session = self.db.client().start_session().await?;
            session.start_transaction().await?;
            super::platform_bot_config_service::fence_admin(self.db, actor, &mut session).await?;
            session.commit_transaction().await?;
        }
        let old = self.manager_values().await?;
        if old.get(MANAGER_TOKEN).is_none()
            && self
                .db
                .collection::<PlatformCredential>(CREDENTIALS)
                .count_documents(doc! {"provider":{"$regex":"^telegram-new(?:/[0-9]+)?$"}})
                .await?
                >= 100
        {
            return Err(AppError::Conflict(
                "At most 100 Telegram managers are supported.".into(),
            ));
        }
        if let Some(manager) = old.get("manager_bot_id") {
            super::platform_bot_config_service::admit_manager_admin(
                self.db,
                actor,
                first_party,
                manager,
            )
            .await?;
            super::platform_bot_config_service::invalidate_manager(self.db, manager).await?;
        }
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
        if self.manager_id.is_some_and(|expected| expected != id) {
            return Err(AppError::Conflict(
                "Manager token belongs to a different bot".into(),
            ));
        }
        super::telegram_new_service::with_operation(
            self.db,
            &format!("telegram-manager-identity:{id}"),
            async {
        if super::telegram_manager_registry::load(self.db, Some(&id.to_string())).await?
            .is_some_and(|r| r.provider != self.manager_provider()) {
            return Err(AppError::Conflict("This bot is already configured as a different manager entry.".into()));
        }
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
        super::platform_bot_config_service::admit_manager_admin(self.db, actor, first_party, &id.to_string()).await?;
        self.save_manager_fields(actor, fields).await?;
        let observation_id = old.get("observation_id").map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.db.collection::<PlatformCredential>(CREDENTIALS).update_one(doc! {"provider": self.manager_provider()}, doc! {"$set": {"fields.manager_bot_id": id.to_string(), "fields.manager_username": username, "fields.webhook_ready": "false", "fields.observation_id": observation_id}}).await?;
        let saved = self.manager_values().await?;
        let secret = saved
            .get(credentials::VERIFY_TOKEN_FIELD)
            .ok_or_else(|| AppError::Internal("Manager webhook secret is missing".into()))?;
        super::platform_bot_config_service::admit_manager_admin(self.db, actor, first_party, &id.to_string()).await?;
        self.api.call(token, "setWebhook", json!({"url": callback, "secret_token": secret, "allowed_updates": MANAGER_ALLOWED_UPDATES, "max_connections": 1})).await?;
        let webhook = self.api.call(token, "getWebhookInfo", json!({})).await?;
        validate_manager_webhook(&webhook, &callback)?;
        super::platform_bot_config_service::finalize_manager_delivery(self.db, actor, first_party, &id.to_string(),
            old.get("observation_started_at").map(str::to_owned).unwrap_or_else(|| chrono::Utc::now().timestamp().to_string())).await?;
        Ok(())
            },
        ).await
    }

    pub fn manager_callback(&self) -> String {
        format!(
            "{}/api/v1/webhooks/channel/telegram-new/manager{}",
            self.config.base_url.trim_end_matches('/'),
            self.manager_id
                .map(|id| format!("/{id}"))
                .unwrap_or_default()
        )
    }

    pub async fn clear_manager(&self) -> AppResult<()> {
        if self.manager_id.is_some() {
            return Err(AppError::Forbidden(
                "Additional manager cleanup requires a human administrator".into(),
            ));
        }
        super::telegram_new_service::with_operation(
            self.db,
            "telegram-new-manager-configuration",
            self.clear_manager_inner(None),
        )
        .await
    }

    pub async fn clear_manager_as(&self, actor: &str) -> AppResult<()> {
        super::telegram_new_service::with_operation(
            self.db,
            "telegram-new-manager-configuration",
            self.clear_manager_inner(Some(actor)),
        )
        .await
    }

    async fn clear_manager_inner(&self, actor: Option<&str>) -> AppResult<()> {
        if let Some(actor) = actor {
            let mut session = self.db.client().start_session().await?;
            session.start_transaction().await?;
            super::platform_bot_config_service::fence_admin(self.db, actor, &mut session).await?;
            session.commit_transaction().await?;
        }
        let row = self
            .db
            .collection::<PlatformCredential>(CREDENTIALS)
            .find_one(doc! {"provider":self.manager_provider()})
            .await?;
        let id = row
            .as_ref()
            .and_then(|row| row.fields.get("manager_bot_id"));
        if let Some(id) = id {
            return super::telegram_new_service::with_operation(
                self.db,
                &format!("telegram-manager-identity:{id}"),
                self.clear_manager_configuration(actor),
            )
            .await;
        }
        self.clear_manager_configuration(actor).await
    }

    async fn clear_manager_configuration(&self, actor: Option<&str>) -> AppResult<()> {
        self.expire().await?;
        let values = self.manager_values().await?;
        if let Some(manager) = values.get("manager_bot_id") {
            let id = manager
                .parse::<i64>()
                .map_err(|_| AppError::Conflict("Invalid manager identity".into()))?;
            if self.db.collection::<ChannelBot>(BOTS).find_one(doc! {
                "credential_source":"telegram_manager", "platform_bot_id":manager, "is_active":true
            }).await?.is_some() {
                return Err(AppError::Conflict("Delete this manager's channel connection before clearing its configuration.".into()));
            }
            let requests = self
                .db
                .collection::<bson::Document>(crate::models::telegram_bot_request::COLLECTION_NAME);
            if requests
                .find_one(doc! {"manager_bot_id":id,"active":true})
                .await?
                .is_some()
            {
                return Err(AppError::Conflict(
                    "Finish or cancel this manager's creation requests before clearing it.".into(),
                ));
            }
            // A created channel retains its setup request UUID after completion.
            let mut children = requests.aggregate(vec![
                doc! {"$match":{"manager_bot_id":id}},
                doc! {"$lookup":{"from":BOTS,"localField":"_id","foreignField":"_id","as":"channel"}},
                doc! {"$match":{"channel":{"$elemMatch":{"is_active":true,"platform":PLATFORM}}}},
                doc! {"$limit":1}, doc! {"$project":{"_id":1}}
            ]).await?;
            use futures::TryStreamExt;
            if children.try_next().await?.is_some() {
                return Err(AppError::Conflict(
                    "Delete this manager's managed channel bots before clearing it.".into(),
                ));
            }
        }
        if let Some(token) = values.get(MANAGER_TOKEN)
            && let Err(error) = self.remove_manager_webhook(token).await
        {
            tracing::warn!(%error, "Telegram manager webhook cleanup failed; clearing saved configuration");
        }
        if let Some(manager) = values.get("manager_bot_id") {
            super::platform_bot_config_service::invalidate_manager(self.db, manager).await?;
        }
        let mut session = self.db.client().start_session().await?;
        session.start_transaction().await?;
        if let Some(actor) = actor {
            super::platform_bot_config_service::fence_admin(self.db, actor, &mut session).await?;
        }
        self.db
            .collection::<PlatformCredential>(CREDENTIALS)
            .delete_one(doc! {"provider":self.manager_provider()})
            .session(&mut session)
            .await?;
        session.commit_transaction().await?;
        Ok(())
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
