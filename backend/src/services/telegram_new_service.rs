use bson::doc;
use chrono::{Duration, Utc};
use mongodb::{
    IndexModel,
    options::{IndexOptions, ReturnDocument},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::channel_adapters::telegram_new::MANAGER_TOKEN;
use super::platform_credential_service as credentials;
use super::telegram_new_api::TelegramApi;
use crate::config::AppConfig;
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{COLLECTION_NAME as BOTS, ChannelBot};
use crate::models::telegram_bot_request::{
    COLLECTION_NAME as REQUESTS, TelegramBotRequest, TelegramRequestStatus as Status,
};
use crate::models::telegram_bot_request::{MANAGED_BOTS, ManagedBotEvents};
use crate::models::user::{COLLECTION_NAME as USERS, User};

pub struct TelegramNewService<'a> {
    pub db: &'a mongodb::Database,
    pub keys: &'a EncryptionKeys,
    pub config: &'a AppConfig,
    pub api: TelegramApi<'a>,
}

pub fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
const CREATION_RECOVERY_MINUTES: i64 = 60;

fn suggested_bot_username(label: &str) -> String {
    let mut stem = label
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
        .to_ascii_lowercase();
    if stem.is_empty() {
        stem.push_str("nyx");
    } else if !stem.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        stem.insert_str(0, "nyx_");
    }
    if stem.ends_with("bot") && (5..=32).contains(&stem.len()) {
        return stem;
    }
    stem.truncate(28);
    format!("{}_bot", stem.trim_end_matches('_'))
}

fn nonce() -> Zeroizing<String> {
    Zeroizing::new(hex::encode(rand::random::<[u8; 24]>()))
}
fn conflict(message: &str) -> AppError {
    AppError::Conflict(message.into())
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match error.kind.as_ref() {
        ErrorKind::Write(WriteFailure::WriteError(error)) => error.code == 11000,
        ErrorKind::Command(error) => error.code == 11000,
        _ => false,
    }
}

pub(crate) async fn with_operation<T>(
    db: &mongodb::Database,
    name: &str,
    operation: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    use super::coordination_service::{LeaseStore, cluster_lease_runtime};
    let runtime = cluster_lease_runtime();
    let lease = runtime
        .acquire(db, name)
        .await?
        .ok_or_else(|| conflict("Another Telegram operation is running. Try again shortly."))?;
    let result = runtime.run_while_renewed(db, &lease, operation).await;
    let _ = LeaseStore::release(db, &lease).await;
    result.unwrap_or_else(|| {
        Err(conflict(
            "Telegram operation interrupted. Reopen Add Channel Bot → Telegram to continue the saved request; for manager configuration, save it again.",
        ))
    })
}

pub async fn ensure_indexes(db: &mongodb::Database) -> Result<(), mongodb::error::Error> {
    db.collection::<ManagedBotEvents>(MANAGED_BOTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"manager_bot_id": 1, "telegram_bot_id": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    let collection = db.collection::<TelegramBotRequest>(REQUESTS);
    for (keys, options) in [
        (
            doc! {"challenge_hash": 1},
            IndexOptions::builder().unique(true).build(),
        ),
        (
            doc! {"actor_user_id": 1},
            IndexOptions::builder()
                .unique(true)
                .partial_filter_expression(doc! {"active": true})
                .build(),
        ),
        (
            doc! {"manager_bot_id": 1, "telegram_user_id": 1},
            IndexOptions::builder()
                .unique(true)
                .partial_filter_expression(
                    doc! {"active": true, "telegram_user_id": {"$type": "long"}},
                )
                .build(),
        ),
        (
            doc! {"manager_bot_id": 1, "telegram_bot_id": 1},
            IndexOptions::default(),
        ),
        (
            doc! {"purge_after": 1},
            IndexOptions::builder()
                .expire_after(std::time::Duration::ZERO)
                .build(),
        ),
    ] {
        collection
            .create_index(IndexModel::builder().keys(keys).options(options).build())
            .await?;
    }
    Ok(())
}

impl TelegramNewService<'_> {
    pub async fn check_owner(&self, actor: &str, owner: &str) -> AppResult<()> {
        let person = self
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": actor, "is_active": true})
            .await?
            .ok_or_else(|| AppError::Forbidden("Account is unavailable".into()))?;
        if person.user_type.is_org()
            || !super::org_service::resolve_owner_access(self.db, actor, owner)
                .await?
                .can_write()
        {
            return Err(AppError::Forbidden(
                "An active account and organization administrator access are required".into(),
            ));
        }
        if self
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": owner, "is_active": true})
            .await?
            .is_none()
        {
            return Err(AppError::Forbidden(
                "Destination account is unavailable".into(),
            ));
        }
        Ok(())
    }

    pub async fn manager(
        &self,
    ) -> AppResult<(i64, String, super::channel_platform::PlatformVerifySecrets)> {
        let values = credentials::load_decrypted(
            self.db,
            self.keys,
            &super::channel_adapters::telegram_new::credential_descriptor(),
        )
        .await?;
        let id = values
            .get("manager_bot_id")
            .and_then(|v| v.parse::<i64>().ok());
        match (id, values.get("manager_username"), values.get(MANAGER_TOKEN), values.get("webhook_ready")) {
            (Some(id), Some(name), Some(_), Some("true")) => Ok((id, name.to_owned(), values)),
            _ => Err(AppError::ValidationError("Telegram bot creation is not configured. Ask an administrator to save a manager bot in Platform Credentials.".into())),
        }
    }

    pub(super) async fn expire(&self) -> AppResult<()> {
        self.db.collection::<TelegramBotRequest>(REQUESTS).update_many(
            doc! {"active": true, "expires_at": {"$lte": bson::DateTime::now()}, "status": {"$nin": ["provisioning", "connected"]}},
            doc! {"$set": {"active": false, "status": "expired", "purge_after": bson::DateTime::from_chrono(Utc::now() + Duration::days(1))}, "$inc": {"revision": 1}, "$unset": {"consent_hash": ""}},
        ).await?;
        Ok(())
    }

    pub async fn current(&self, actor: &str) -> AppResult<Option<TelegramBotRequest>> {
        self.expire().await?;
        let request = self
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(doc! {"actor_user_id": actor, "active": true})
            .await?;
        if let Some(request) = &request
            && !self.owner_still_authorized(request).await?
            && request.status != Status::Provisioning
        {
            return Ok(None);
        }
        Ok(request)
    }

    async fn cancel_unstarted(&self, request: &TelegramBotRequest) -> AppResult<bool> {
        let result = self.db.collection::<TelegramBotRequest>(REQUESTS).update_one(
            doc! {"_id": &request.id, "revision": request.revision, "status": {"$in": ["waiting_telegram", "waiting_bot", "waiting_consent", "ready"]}},
            doc! {"$set": {"active": false, "status": "cancelled", "purge_after": bson::DateTime::from_chrono(Utc::now() + Duration::days(1))}, "$inc": {"revision": 1}, "$unset": {"consent_hash": ""}},
        ).await?;
        Ok(result.modified_count == 1)
    }

    async fn owner_still_authorized(&self, request: &TelegramBotRequest) -> AppResult<bool> {
        match self
            .check_owner(&request.actor_user_id, &request.owner_user_id)
            .await
        {
            Ok(()) => Ok(true),
            Err(AppError::Forbidden(_) | AppError::NotFound(_)) => {
                self.cancel_unstarted(request).await?;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    pub async fn get(&self, actor: &str, id: &str) -> AppResult<TelegramBotRequest> {
        self.expire().await?;
        let request = self
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(doc! {"_id": id, "actor_user_id": actor})
            .await?
            .ok_or_else(|| AppError::NotFound("Telegram creation request not found".into()))?;
        if !self.owner_still_authorized(&request).await? {
            return Err(AppError::Forbidden(
                "Destination access is no longer available".into(),
            ));
        }
        Ok(request)
    }

    pub async fn begin(
        &self,
        actor: &str,
        owner: &str,
        label: &str,
    ) -> AppResult<(TelegramBotRequest, String)> {
        with_operation(
            self.db,
            "telegram-new-manager-configuration",
            self.begin_inner(actor, owner, label),
        )
        .await
    }

    async fn begin_inner(
        &self,
        actor: &str,
        owner: &str,
        label: &str,
    ) -> AppResult<(TelegramBotRequest, String)> {
        self.check_owner(actor, owner).await?;
        let label = label.trim();
        if label.is_empty() || label.len() > 128 {
            return Err(AppError::ValidationError(
                "Label must be between 1 and 128 characters".into(),
            ));
        }
        let (manager_id, manager_name, manager_values) = self.manager().await?;
        let observation_id = manager_values
            .get("observation_id")
            .ok_or_else(|| conflict("Save the manager configuration before creating bots"))?;
        if self.current(actor).await?.is_some() {
            return Err(conflict(
                "Finish or cancel your existing Telegram creation request first",
            ));
        }
        let destination = self
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": owner})
            .await?
            .ok_or_else(|| AppError::NotFound("Destination account not found".into()))?;
        let challenge = nonce();
        let request = TelegramBotRequest {
            id: uuid::Uuid::new_v4().to_string(),
            actor_user_id: actor.into(),
            owner_user_id: owner.into(),
            manager_bot_id: manager_id,
            observation_id: observation_id.into(),
            label: label.into(),
            destination: format!(
                "{}\nDestination ID: {}\nInitiating account ID: {}\nWebsite: {}",
                destination.slug.as_deref().unwrap_or("Personal account"),
                owner,
                actor,
                self.config.frontend_url
            ),
            status: Status::WaitingTelegram,
            active: true,
            revision: 0,
            challenge_hash: hash(&challenge),
            telegram_user_id: None,
            telegram_bot_id: None,
            bot_username: None,
            consent_hash: None,
            manager_revision: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(15),
            purge_after: None,
        };
        self.db
            .collection::<TelegramBotRequest>(REQUESTS)
            .insert_one(&request)
            .await?;
        Ok((
            request,
            format!("https://t.me/{manager_name}?start={}", challenge.as_str()),
        ))
    }

    pub async fn launch(&self, actor: &str, id: &str) -> AppResult<String> {
        let request = self.get(actor, id).await?;
        if !request.active
            || !matches!(
                request.status,
                Status::WaitingTelegram | Status::WaitingBot | Status::WaitingConsent
            )
        {
            return Err(conflict("This creation request cannot be opened again"));
        }
        let (manager_id, manager_name, _) = self.manager().await?;
        if manager_id != request.manager_bot_id {
            return Err(conflict(
                "The manager configuration changed; start a new request",
            ));
        }
        let challenge = nonce();
        let result = self
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .update_one(
                doc! {"_id": id, "revision": request.revision, "active": true},
                doc! {"$set": {"challenge_hash": hash(&challenge)}, "$inc": {"revision": 1}},
            )
            .await?;
        if result.modified_count != 1 {
            return Err(conflict("The request changed; refresh and try again"));
        }
        Ok(format!(
            "https://t.me/{manager_name}?start={}",
            challenge.as_str()
        ))
    }

    pub async fn cancel(&self, actor: &str, id: &str) -> AppResult<()> {
        let request = self
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(doc! {"_id": id, "actor_user_id": actor})
            .await?
            .ok_or_else(|| AppError::NotFound("Telegram creation request not found".into()))?;
        if matches!(request.status, Status::Cancelled | Status::Expired) {
            return Ok(());
        }
        if matches!(
            request.status,
            Status::Provisioning | Status::Connected | Status::Suspended
        ) {
            return Err(conflict(
                "Bot connection has started; manage the saved bot instead",
            ));
        }
        if !self.cancel_unstarted(&request).await? {
            return Err(conflict("The request changed; refresh before cancelling"));
        }
        if let Some(user) = request.telegram_user_id
            && let Ok((_, _, values)) = self.manager().await
            && let Some(token) = values.get(MANAGER_TOKEN)
        {
            let _ = self.api.call(token, "sendMessage", json!({"chat_id": user, "text": "Creation request cancelled. A bot already created in Telegram still belongs to you.", "reply_markup": {"remove_keyboard": true}})).await;
        }
        Ok(())
    }

    pub async fn webhook(&self, headers: &axum::http::HeaderMap, body: &[u8]) -> AppResult<()> {
        if body.len() > 256 * 1024 {
            return Err(AppError::RequestBodyTooLarge {
                max_bytes: 256 * 1024,
                context: "Telegram manager webhook".into(),
            });
        }
        let (manager_id, _, values) = self.manager().await?;
        let provided = headers
            .get("x-telegram-bot-api-secret-token")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let expected = values
            .get(credentials::VERIFY_TOKEN_FIELD)
            .ok_or_else(|| conflict("Manager webhook secret is unavailable"))?;
        if !bool::from(hash(provided).as_bytes().ct_eq(hash(expected).as_bytes())) {
            return Err(AppError::ChannelWebhookVerificationFailed(
                "Invalid manager webhook secret".into(),
            ));
        }
        let update: Value = serde_json::from_slice(body)
            .map_err(|_| AppError::BadRequest("Invalid Telegram update".into()))?;
        let token = values
            .get(MANAGER_TOKEN)
            .ok_or_else(|| conflict("Manager token is unavailable"))?;
        self.expire().await?;
        if let Some(change) = update.get("managed_bot") {
            if let (Some(bot_id), Some(user), Some(update_id)) = (
                bot_identity(&change["bot"]).map(|b| b.0),
                change["user"]["id"].as_i64(),
                update["update_id"].as_i64(),
            ) {
                let events = self.db.collection::<ManagedBotEvents>(MANAGED_BOTS);
                events.update_one(doc! {"manager_bot_id": manager_id, "telegram_bot_id": bot_id}, doc! {"$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "manager_bot_id": manager_id, "telegram_bot_id": bot_id, "telegram_user_id": user, "revision": 0_i64, "update_ids": []}}).upsert(true).await?;
                let event = events.find_one_and_update(doc! {"manager_bot_id": manager_id, "telegram_bot_id": bot_id, "update_ids": {"$ne": update_id}}, doc! {"$set": {"telegram_user_id": user}, "$inc": {"revision": 1_i64}, "$push": {"update_ids": {"$each": [update_id], "$slice": -64}}}).return_document(ReturnDocument::After).await?;
                let event = match event {
                    Some(event) => Some(event),
                    None => {
                        events
                            .find_one(
                                doc! {"manager_bot_id": manager_id, "telegram_bot_id": bot_id},
                            )
                            .await?
                    }
                };
                if let Some(event) = event {
                    self.suspend_changed_bot(manager_id, bot_id, event.revision)
                        .await?;
                }
            }
            return Ok(());
        }
        if let Some(callback) = update.get("callback_query") {
            return self.consent_callback(manager_id, token, callback).await;
        }
        let Some(message) = update.get("message") else {
            return Ok(());
        };
        let Some(user_id) = private_sender(message) else {
            return Ok(());
        };
        if let Some(challenge) = message["text"]
            .as_str()
            .and_then(|t| t.strip_prefix("/start "))
        {
            if challenge.len() != 48 || !challenge.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(());
            }
            let request = self.db.collection::<TelegramBotRequest>(REQUESTS).find_one_and_update(
                doc! {"manager_bot_id": manager_id, "challenge_hash": hash(challenge), "active": true, "expires_at": {"$gt": bson::DateTime::now()}, "status": {"$in": ["waiting_telegram", "waiting_bot", "waiting_consent"]}, "$or": [{"telegram_user_id": null}, {"telegram_user_id": user_id}]},
                doc! {"$set": {"telegram_user_id": user_id}, "$inc": {"revision": 1}},
            ).return_document(ReturnDocument::After).await;
            let request = match request {
                Ok(request) => request,
                Err(error) if is_duplicate_key_error(&error) => {
                    self.api.call(token, "sendMessage", json!({"chat_id": user_id, "text": "Your Telegram account already has an active creation request. Finish or cancel that request in NyxID, then reopen this setup link."})).await?;
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };
            if let Some(request) = request {
                if !self.owner_still_authorized(&request).await? {
                    return Ok(());
                }
                if request.status == Status::WaitingConsent {
                    self.send_consent(token, &request).await?;
                } else {
                    self.db
                        .collection::<TelegramBotRequest>(REQUESTS)
                        .update_one(
                            doc! {"_id": &request.id, "revision": request.revision},
                            doc! {"$set": {"status": "waiting_bot"}},
                        )
                        .await?;
                    self.api.call(token, "sendMessage", json!({"chat_id": user_id, "text": format!("Create a Telegram bot to connect to:\n{}\n\nOnly continue if you started this setup and recognize these identifiers. You own the bot; NyxID will manage its connection. You will approve the exact bot after creation.\n\nTo recover a bot created in an earlier attempt, send /recover @YourBotUsername here.", request.destination), "reply_markup": {"keyboard": [[{"text": "Create bot", "request_managed_bot": {"request_id": 1, "suggested_name": request.label.chars().take(64).collect::<String>(), "suggested_username": suggested_bot_username(&request.label)}}]], "resize_keyboard": true, "one_time_keyboard": true}})).await?;
                }
            }
            return Ok(());
        }
        let creation = message
            .get("managed_bot_created")
            .and_then(|v| bot_identity(&v["bot"]));
        let observation_id = values
            .get("observation_id")
            .ok_or_else(|| conflict("Manager observation is unavailable"))?;
        let observation_start = values
            .get("observation_started_at")
            .and_then(|date| date.parse::<i64>().ok())
            .ok_or_else(|| conflict("Manager observation is unavailable"))?;
        let (bot_id, username) = if let Some((bot_id, username)) = creation {
            let created_at = message["date"]
                .as_i64()
                .and_then(chrono::DateTime::from_timestamp_secs);
            let Some(created_at) = created_at.filter(|date| {
                date.timestamp() >= observation_start
                    && *date >= Utc::now() - Duration::minutes(CREATION_RECOVERY_MINUTES)
                    && *date <= Utc::now() + Duration::seconds(30)
            }) else {
                self.api.call(token, "sendMessage", json!({"chat_id": user_id, "text": "This creation is too old or has no verifiable creation time. Create a new bot from the current setup request, or use the Telegram bot token option with its current token."})).await?;
                return Ok(());
            };
            let events = self.db.collection::<ManagedBotEvents>(MANAGED_BOTS);
            events.update_one(doc! {"manager_bot_id": manager_id, "telegram_bot_id": bot_id}, doc! {"$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "manager_bot_id": manager_id, "telegram_bot_id": bot_id, "telegram_user_id": user_id, "revision": 0_i64, "update_ids": []}}).upsert(true).await?;
            // Provenance is immutable, including across reconfiguration and redelivery.
            events.update_one(doc! {"manager_bot_id": manager_id, "telegram_bot_id": bot_id, "created_by": null, "retired": {"$ne": true}, "revision": {"$lte": 1_i64}}, doc! {"$set": {"created_by": user_id, "bot_username": &username, "created_at": bson::DateTime::from_chrono(created_at), "observation_id": observation_id}}).await?;
            (bot_id, username)
        } else if let Some(name) = message["text"]
            .as_str()
            .and_then(|text| text.strip_prefix("/recover "))
            .map(|name| name.trim().trim_start_matches('@'))
        {
            let Some(candidate) = self.db.collection::<ManagedBotEvents>(MANAGED_BOTS).find_one(doc! {"manager_bot_id": manager_id, "created_by": user_id, "bot_username": name, "observation_id": observation_id, "retired": {"$ne": true}, "created_at": {"$gt": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(CREATION_RECOVERY_MINUTES))}}).await? else {
                self.api.call(token, "sendMessage", json!({"chat_id": user_id, "text": "No recent, unconnected bot creation was recorded for your Telegram account. Recovery is available for 60 minutes after creation. Create a new bot or use the Telegram bot token option with its current token."})).await?;
                return Ok(());
            };
            (
                candidate.telegram_bot_id,
                candidate.bot_username.unwrap_or_default(),
            )
        } else {
            return Ok(());
        };
        let request = self.db.collection::<TelegramBotRequest>(REQUESTS).find_one_and_update(
            doc! {"manager_bot_id": manager_id, "telegram_user_id": user_id, "active": true, "status": "waiting_bot", "expires_at": {"$gt": bson::DateTime::now()}},
            doc! {"$set": {"telegram_bot_id": bot_id, "bot_username": &username, "status": "waiting_consent"}, "$inc": {"revision": 1}},
        ).return_document(ReturnDocument::After).await?;
        if let Some(request) = request { self.send_consent(token, &request).await?; }
        else if let Some(request) = self.db.collection::<TelegramBotRequest>(REQUESTS).find_one(doc! {"manager_bot_id": manager_id, "telegram_user_id": user_id, "telegram_bot_id": bot_id, "active": true, "status": "waiting_consent"}).await? {
            self.send_consent(token, &request).await?;
        } else {
            self.api.call(token, "sendMessage", json!({"chat_id": user_id, "text": format!("@{username} exists in Telegram but this event has no waiting creation request. Return to NyxID → Add Channel Bot → Telegram, start a request, then send /recover @{username} here. No bot was connected by this message."), "reply_markup": {"remove_keyboard": true}})).await?;
        }
        Ok(())
    }

    async fn send_consent(&self, token: &str, request: &TelegramBotRequest) -> AppResult<()> {
        if !self.owner_still_authorized(request).await? {
            return Ok(());
        }
        let consent = nonce();
        let result = self.db.collection::<TelegramBotRequest>(REQUESTS).update_one(
            doc! {"_id": &request.id, "status": "waiting_consent", "revision": request.revision, "active": true},
            doc! {"$set": {"consent_hash": hash(&consent)}, "$inc": {"revision": 1}},
        ).await?;
        if result.modified_count != 1 {
            return Ok(());
        }
        self.api.call(token, "sendMessage", json!({"chat_id": request.telegram_user_id, "text": "Review the exact bot and destination below before approving.", "reply_markup": {"remove_keyboard": true}})).await?;
        self.api.call(token, "sendMessage", json!({"chat_id": request.telegram_user_id, "text": format!("Connect @{} (bot ID {}) to:\n{}\n\nNyxID has not verified a prior relationship between this Telegram account and the destination. Only approve if you started this setup and recognize these identifiers. NyxID will obtain this bot's token to receive messages and send replies.", request.bot_username.as_deref().unwrap_or(""), request.telegram_bot_id.unwrap_or_default(), request.destination), "reply_markup": {"inline_keyboard": [[{"text": "Decline", "callback_data": format!("no:{}", consent.as_str())}, {"text": "Approve this bot", "callback_data": format!("ok:{}", consent.as_str())}]]}})).await?;
        Ok(())
    }

    async fn consent_callback(&self, manager: i64, token: &str, callback: &Value) -> AppResult<()> {
        let Some(user) = callback["from"]["id"].as_i64().filter(|id| *id > 0) else {
            return Ok(());
        };
        if callback["from"]["is_bot"] != false
            || callback["message"]["chat"]["type"] != "private"
            || callback["message"]["chat"]["id"].as_i64() != Some(user)
        {
            return Ok(());
        }
        let Some(data) = callback["data"].as_str() else {
            return Ok(());
        };
        let (approve, proof) = if let Some(proof) = data.strip_prefix("ok:") {
            (true, proof)
        } else if let Some(proof) = data.strip_prefix("no:") {
            (false, proof)
        } else {
            return Ok(());
        };
        if proof.len() != 48 {
            return Ok(());
        }
        let pending = self.db.collection::<TelegramBotRequest>(REQUESTS).find_one(doc! {"manager_bot_id": manager, "telegram_user_id": user, "consent_hash": hash(proof), "active": true, "status": "waiting_consent"}).await?;
        let Some(pending) = pending else {
            return Ok(());
        };
        if !self.owner_still_authorized(&pending).await? {
            return Ok(());
        }
        let event = self
            .db
            .collection::<ManagedBotEvents>(MANAGED_BOTS)
            .find_one(doc! {"manager_bot_id": manager, "telegram_bot_id": pending.telegram_bot_id})
            .await?;
        // The provider identifies the creator, not necessarily the current owner.
        // Further management events require an independent credential proof.
        if approve
            && event.as_ref().is_some_and(|event| {
                event.revision > 1
                    || event.telegram_user_id != user
                    || event.created_by != Some(user)
                    || event.retired
                    || event.observation_id.as_deref() != Some(&pending.observation_id)
                    || event.created_at.is_none_or(|date| {
                        date < Utc::now() - Duration::minutes(CREATION_RECOVERY_MINUTES)
                    })
            })
        {
            self.cancel_unstarted(&pending).await?;
            self.api.call(token, "sendMessage", json!({"chat_id": user, "text": "This bot's management changed after creation. This request has been cancelled. Create a new bot, or use the Telegram bot token option with the current bot token after deleting any saved connection.", "reply_markup": {"remove_keyboard": true}})).await?;
            return Ok(());
        }
        if approve && event.as_ref().is_none_or(|event| event.revision == 0) {
            self.api.call(token, "answerCallbackQuery", json!({"callback_query_id": callback["id"], "text": "Telegram is still confirming creation. Please tap Approve again shortly.", "show_alert": true})).await?;
            return Ok(());
        }
        let request = self.db.collection::<TelegramBotRequest>(REQUESTS).find_one_and_update(
            doc! {"manager_bot_id": manager, "telegram_user_id": user, "consent_hash": hash(proof), "active": true, "status": "waiting_consent", "expires_at": {"$gt": bson::DateTime::now()}},
            doc! {"$set": {"status": if approve {"ready"} else {"cancelled"}, "active": approve, "manager_revision": event.map(|e| e.revision)}, "$unset": {"consent_hash": ""}, "$inc": {"revision": 1}},
        ).return_document(ReturnDocument::After).await?;
        if let Some(request) = request {
            let return_url = format!(
                "{}/channel-bots?connect=telegram-new",
                self.config.frontend_url.trim_end_matches('/')
            );
            let markup = if approve {
                json!({"inline_keyboard": [[{"text": "Return to NyxID", "url": return_url}]]})
            } else {
                json!({"remove_keyboard": true})
            };
            self.api.call(token, "sendMessage", json!({"chat_id": user, "text": if approve {"Approved. Return to NyxID and confirm Connect bot to finish."} else {"Connection declined. Your Telegram bot has not been connected."}, "reply_markup": markup})).await?;
            let _ = self.api.call(token, "answerCallbackQuery", json!({"callback_query_id": callback["id"], "text": if request.status == Status::Ready {"Approved"} else {"Declined"}})).await;
        }
        Ok(())
    }

    pub(crate) async fn verify_creation_delivery(
        &self,
        request: &TelegramBotRequest,
        token: &str,
    ) -> AppResult<()> {
        let event = self.db.collection::<ManagedBotEvents>(MANAGED_BOTS).find_one(doc! {"manager_bot_id": request.manager_bot_id, "telegram_bot_id": request.telegram_bot_id, "revision": 1_i64, "created_by": request.telegram_user_id, "observation_id": &request.observation_id, "retired": {"$ne": true}}).await?.ok_or_else(|| conflict("Bot management changed. Create a new bot or use its current token with the Telegram bot token option."))?;
        let created_at = event.created_at.filter(|date| *date >= Utc::now() - Duration::minutes(CREATION_RECOVERY_MINUTES)).ok_or_else(|| conflict("This creation is too old to connect automatically. Use the Telegram bot token option with its current token."))?;
        let webhook = self.api.call(token, "getWebhookInfo", json!({})).await?;
        if webhook["url"] != self.manager_callback()
            || webhook["pending_update_count"].as_i64() != Some(0)
            || webhook["last_error_date"]
                .as_i64()
                .is_some_and(|date| date >= created_at.timestamp())
        {
            return Err(conflict(
                "Telegram management updates are pending or delivery was interrupted. Retry after delivery recovers; a recorded delivery error requires a new bot or the Telegram bot token option.",
            ));
        }
        let unchanged = self
            .db
            .collection::<ManagedBotEvents>(MANAGED_BOTS)
            .find_one(doc! {"_id": event.id, "revision": 1_i64, "retired": {"$ne": true}})
            .await?;
        if unchanged.is_none() {
            return Err(conflict(
                "Telegram management changed while checking delivery. Start a new bot or use its current token with the Telegram bot token option.",
            ));
        }
        Ok(())
    }

    pub(crate) async fn remove_owned_webhook(&self, bot: &ChannelBot) -> AppResult<&'static str> {
        let token =
            Zeroizing::new(super::channel_bot_service::decrypt_bot_token(self.keys, bot).await?);
        let callback = format!(
            "{}/api/v1/webhooks/channel/{}/{}",
            self.config.base_url.trim_end_matches('/'),
            bot.platform,
            bot.id
        );
        let webhook = self.api.call(&token, "getWebhookInfo", json!({})).await?;
        if webhook["url"] != callback {
            return Ok("retained_external");
        }
        self.api
            .call(
                &token,
                "deleteWebhook",
                json!({"drop_pending_updates": false}),
            )
            .await?;
        Ok("removed")
    }

    async fn suspend_changed_bot(
        &self,
        manager: i64,
        telegram_bot: i64,
        revision: i64,
    ) -> AppResult<()> {
        use futures::TryStreamExt;
        let mut requests = self.db.collection::<TelegramBotRequest>(REQUESTS).find(doc! {"manager_bot_id": manager, "telegram_bot_id": telegram_bot, "manager_revision": {"$ne": revision}, "status": {"$in": ["provisioning", "connected", "suspended"]}}).await?;
        while let Some(request) = requests.try_next().await? {
            self.db
                .collection::<ChannelBot>(BOTS)
                .update_one(
                    doc! {"_id": &request.id, "is_active": true},
                    doc! {"$set": {"status": "suspended", "updated_at": bson::DateTime::now()}},
                )
                .await?;
            self.db.collection::<TelegramBotRequest>(REQUESTS).update_one(doc! {"_id": &request.id}, doc! {"$set": {"status": "suspended", "active": false}, "$inc": {"revision": 1}}).await?;
            if let Some(bot) = self
                .db
                .collection::<ChannelBot>(BOTS)
                .find_one(doc! {"_id": &request.id, "is_active": true})
                .await?
            {
                let _ = self.remove_owned_webhook(&bot).await;
            }
        }
        Ok(())
    }
}

pub fn private_sender(message: &Value) -> Option<i64> {
    if message["chat"]["type"] != "private"
        || ["forward_origin", "forward_date", "sender_chat", "via_bot"]
            .iter()
            .any(|key| message.get(*key).is_some())
        || message["from"]["is_bot"] != false
    {
        return None;
    }
    let user = message["from"]["id"].as_i64().filter(|id| *id > 0)?;
    (message["chat"]["id"].as_i64() == Some(user)).then_some(user)
}

pub fn bot_identity(value: &Value) -> Option<(i64, String)> {
    if value["is_bot"] != true {
        return None;
    }
    let id = value["id"].as_i64().filter(|id| *id > 0)?;
    let username = value["username"].as_str()?;
    if !(5..=32).contains(&username.len())
        || !username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return None;
    }
    Some((id, username.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::suggested_bot_username;

    #[test]
    fn telegram_new_username_suggestions_follow_telegram_constraints() {
        for (label, expected) in [
            ("nyx_test_123456", "nyx_test_123456_bot"),
            ("Support Team!", "support_team_bot"),
            ("CustomerBot", "customerbot"),
            ("Customer_bot", "customer_bot"),
            ("123 Support", "nyx_123_support_bot"),
            ("x", "x_bot"),
            ("客服 🤖", "nyx_bot"),
            ("---", "nyx_bot"),
        ] {
            assert_eq!(suggested_bot_username(label), expected);
        }
        for label in [
            "a".repeat(128),
            format!("{}bot", "a".repeat(32)),
            format!("{} extra", "a".repeat(27)),
            "🤖".repeat(32),
            "9".repeat(128),
        ] {
            let username = suggested_bot_username(&label);
            assert!((5..=32).contains(&username.len()));
            assert!(username.starts_with(|ch: char| ch.is_ascii_alphabetic()));
            assert!(username.ends_with("bot"));
            assert!(
                username
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
            );
        }
    }
}
