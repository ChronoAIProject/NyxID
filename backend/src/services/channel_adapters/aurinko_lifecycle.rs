use super::*;
use crate::models::channel_email::{
    BATCHES, EmailBatch, EmailReceipt, EmailSend, EmailSubscription, RECEIPTS, SENDS, SUBSCRIPTIONS,
};
use zeroize::Zeroizing;

impl AurinkoAdapter {
    pub(super) async fn signing_fingerprint(
        &self,
        db: &mongodb::Database,
        bot: &ChannelBot,
    ) -> AppResult<String> {
        if bot.credential_source == "connection" {
            let descriptor = self.platform_credentials().ok_or_else(invalid)?;
            let row = crate::services::platform_credential_service::load(db, &descriptor)
                .await?
                .ok_or_else(invalid)?;
            let secret = row.secrets.get("signing_secret").ok_or_else(invalid)?;
            Ok(hex::encode(Sha256::digest(&secret.bytes)))
        } else {
            Ok(hex::encode(Sha256::digest(
                bot.app_secret_encrypted.as_deref().ok_or_else(invalid)?,
            )))
        }
    }
    pub(super) async fn signing_verification(
        &self,
        db: &mongodb::Database,
        keys: &crate::crypto::aes::EncryptionKeys,
        bot: &ChannelBot,
    ) -> AppResult<(
        crate::services::channel_platform::PlatformVerifySecrets,
        String,
    )> {
        if bot.credential_source != "connection" {
            return Ok((
                self.build_verify_secrets(keys, bot).await?,
                self.signing_fingerprint(db, bot).await?,
            ));
        }
        let descriptor = self.platform_credentials().ok_or_else(invalid)?;
        let row = crate::services::platform_credential_service::load(db, &descriptor)
            .await?
            .ok_or_else(invalid)?;
        let encrypted = &row.secrets.get("signing_secret").ok_or_else(invalid)?.bytes;
        let fingerprint = hex::encode(Sha256::digest(encrypted));
        let bytes = Zeroizing::new(keys.decrypt(encrypted).await?);
        let secret = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
        let mut secrets = crate::services::channel_platform::PlatformVerifySecrets::default();
        secrets.insert("app_secret", secret.to_owned());
        Ok((secrets, fingerprint))
    }
    async fn subscriptions(&self, token: &str, url: &str) -> AppResult<Vec<Value>> {
        let mut found = Vec::new();
        for offset in (0..1000).step_by(100) {
            let (status, page) = self
                .request(
                    Method::GET,
                    token,
                    &["subscriptions"],
                    &[
                        ("limit", "100"),
                        ("offset", &offset.to_string()),
                        ("includeInactive", "true"),
                    ],
                    None,
                )
                .await?;
            if !status.is_success() {
                return Err(upstream());
            }
            let records = page["records"].as_array().ok_or_else(upstream)?;
            found.extend(
                records
                    .iter()
                    .filter(|row| {
                        row["resource"] == "/email/messages" && row["notificationUrl"] == url
                    })
                    .cloned(),
            );
            if page["done"] == true || records.len() < 100 {
                return Ok(found);
            }
        }
        Err(upstream())
    }
    async fn unsubscribe(&self, token: &str, id: &str) -> AppResult<()> {
        let (status, _) = self
            .request(Method::DELETE, token, &["subscriptions", id], &[], None)
            .await?;
        if status.is_success() || status == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(upstream())
        }
    }
    pub(super) async fn setup_subscription(
        &self,
        db: &mongodb::Database,
        bot: &ChannelBot,
        token: &str,
        url: &str,
    ) -> AppResult<()> {
        if !bot.is_active {
            return Err(AppError::ChannelBotInactive("Bot has been deleted".into()));
        }
        if bot.credential_source == "connection" {
            crate::services::aurinko_oauth_service::require_live_connection(
                db,
                &bot.user_id,
                bot.connection_id.as_deref().ok_or_else(invalid)?,
                true,
            )
            .await?;
        }
        self.account(token, Some(&bot.platform_bot_id)).await?;
        let collection = db.collection::<EmailSubscription>(SUBSCRIPTIONS);
        let binding = match collection.find_one(doc! { "_id": &bot.id }).await? {
            Some(binding) => binding,
            None => {
                let binding = EmailSubscription {
                    id: bot.id.clone(),
                    user_id: bot.user_id.clone(),
                    account_id: bot.platform_bot_id.clone(),
                    notification_url: url.into(),
                    subscription_id: None,
                    verified_secret_fingerprint: None,
                    created_at: Utc::now(),
                };
                collection.insert_one(&binding).await?;
                binding
            }
        };
        if binding.user_id != bot.user_id
            || binding.account_id != bot.platform_bot_id
            || binding.notification_url != url
        {
            return Err(AppError::Conflict(
                "Aurinko callback binding changed; delete and recreate the bot".into(),
            ));
        }
        // Recover a successful subscribe whose response (or subsequent DB write)
        // was lost. Never subscribe a second time without checking the exact URL.
        let rows = self.subscriptions(token, url).await?;
        let fingerprint = self.signing_fingerprint(db, bot).await?;
        let active = rows.iter().find(|row| {
            binding.verified_secret_fingerprint.as_deref() == Some(&fingerprint)
                && row["active"] == true
                && row["disabledByBilling"] != true
                && row["detailLevel"] == "status"
                && row["filters"]
                    .as_array()
                    .is_some_and(|filters| filters.iter().any(|filter| filter == "withoutDrafts"))
        });
        let id = if let Some(row) = active {
            identity(&row["id"]).ok_or_else(upstream)?
        } else {
            for row in &rows {
                self.unsubscribe(token, &identity(&row["id"]).ok_or_else(upstream)?)
                    .await?;
            }
            let (status, row) = self.request(Method::POST, token, &["subscriptions"], &[], Some(&json!({
                "resource": "/email/messages", "notificationUrl": url, "filters": ["withoutDrafts"], "detailLevel": "status"
            }))).await?;
            if !status.is_success() {
                return Err(upstream());
            }
            identity(&row["id"]).ok_or_else(upstream)?
        };
        if self.signing_fingerprint(db, bot).await? != fingerprint {
            return Err(AppError::Conflict(
                "Platform signing secret changed; retry Verify".into(),
            ));
        }
        let persisted = collection
            .update_one(
                doc! { "_id": &bot.id, "user_id": &bot.user_id,
                "account_id": &bot.platform_bot_id, "verified_secret_fingerprint": &fingerprint },
                doc! { "$set": { "subscription_id": &id } },
            )
            .await?;
        if persisted.matched_count != 1 {
            return Err(AppError::ChannelPlatformError(
                "Aurinko did not verify the current signing secret; retry Verify".into(),
            ));
        }
        if self.signing_fingerprint(db, bot).await? != fingerprint {
            return Err(AppError::Conflict(
                "Platform signing secret changed; retry Verify".into(),
            ));
        }
        for row in rows {
            let other = identity(&row["id"]).ok_or_else(upstream)?;
            if other != id {
                self.unsubscribe(token, &other).await?;
            }
        }
        Ok(())
    }
    pub(super) async fn remove_subscription(
        &self,
        db: &mongodb::Database,
        bot: &ChannelBot,
        token: &str,
    ) -> AppResult<()> {
        let Some(binding) = db
            .collection::<EmailSubscription>(SUBSCRIPTIONS)
            .find_one(doc! {
                "_id": &bot.id, "user_id": &bot.user_id, "account_id": &bot.platform_bot_id,
            })
            .await?
        else {
            return Ok(());
        };
        // Listing by the stored exact URL also cleans up uncertain subscribe
        // attempts; other consumers of this mailbox are never touched.
        for row in self.subscriptions(token, &binding.notification_url).await? {
            self.unsubscribe(token, &identity(&row["id"]).ok_or_else(upstream)?)
                .await?;
        }
        db.collection::<EmailSubscription>(SUBSCRIPTIONS)
            .delete_one(doc! { "_id": &bot.id })
            .await?;
        Ok(())
    }
    pub(super) async fn receive(
        &self,
        context: &IngressContext<'_>,
        bot_id: &str,
        headers: &HeaderMap,
        query: &HashMap<String, String>,
        body: &[u8],
    ) -> AppResult<Option<String>> {
        if body.len() > MAX_BODY {
            return Err(malformed());
        }
        let bot = crate::services::channel_bot_service::get_bot(context.db, bot_id).await?;
        if bot.platform != "aurinko" || !bot.is_active {
            return Err(invalid());
        }
        let (secrets, fingerprint) = self
            .signing_verification(context.db, context.encryption_keys, &bot)
            .await?;
        self.verify_webhook(&bot, Some(&secrets), headers, body)
            .await?;
        if let Some(challenge) = query.get("validationToken") {
            if challenge.is_empty()
                || challenge.len() > 4096
                || challenge.chars().any(char::is_control)
            {
                return Err(malformed());
            }
            // No account payload exists in Aurinko's signed text/plain challenge.
            // The per-bot URL and persisted verified account bind setup here.
            context
                .db
                .collection::<EmailSubscription>(SUBSCRIPTIONS)
                .update_one(
                    doc! { "_id": &bot.id, "user_id": &bot.user_id },
                    doc! { "$set": { "verified_secret_fingerprint": fingerprint } },
                )
                .await?;
            return Ok(Some(challenge.clone()));
        }
        if bot.status != "active" || !bot.webhook_registered {
            return Err(channel_retry_ingress::retry_later());
        }
        let binding = context
            .db
            .collection::<EmailSubscription>(SUBSCRIPTIONS)
            .find_one(doc! {
                "_id": &bot.id, "user_id": &bot.user_id, "account_id": &bot.platform_bot_id,
            })
            .await?
            .ok_or_else(channel_retry_ingress::retry_later)?;
        let event: Value = serde_json::from_slice(body).map_err(|_| malformed())?;
        if identity(&event["accountId"]).as_deref() != Some(bot.platform_bot_id.as_str())
            || identity(&event["subscription"]) != binding.subscription_id
            || binding.subscription_id.is_none()
        {
            return Err(invalid());
        }
        if let Some(kind) = event["lifecycleEvent"].as_str() {
            crate::services::audit_service::log_async(
                context.db.clone(),
                Some(bot.user_id.clone()),
                "channel_email_lifecycle".into(),
                Some(
                    json!({"bot_id":bot.id, "kind": if kind == "error" { "error" } else { "status" }}),
                ),
                None,
                None,
                None,
                None,
            );
            return Ok(None);
        }
        // Lifecycle, tracking and other resource notifications are not mail.
        if event["resource"] != "/email/messages" {
            return Ok(None);
        }
        let Some(payloads) = event["payloads"].as_array() else {
            // Signal-only delivery cannot be processed without delta sync.
            return Err(channel_retry_ingress::retry_later());
        };
        if payloads.len() > MAX_BATCH {
            return Err(malformed());
        }
        if payloads.is_empty() {
            return Ok(None);
        }
        use sha2::Digest;
        let digest = hex::encode(Sha256::digest(body));
        let EventDedupClaimResult::Claimed(batch_claim) = EventDedupStore::claim(
            context.db,
            "aurinko-batch",
            &bot.id,
            &digest,
            Duration::from_secs(60),
        )
        .await?
        else {
            return Err(channel_retry_ingress::retry_later());
        };
        let result = channel_retry_ingress::with_ingress(context.db, &bot.id, async {
            let live = crate::services::channel_bot_service::get_bot(context.db, &bot.id).await?;
            if !live.is_active || live.status != "active" || live.updated_at != bot.updated_at {
                return Err(channel_retry_ingress::retry_later());
            }
            self.receive_batch(context, &bot, payloads, &digest).await
        })
        .await;
        EventDedupStore::release(context.db, &batch_claim).await?;
        result
    }
    async fn receive_batch(
        &self,
        context: &IngressContext<'_>,
        bot: &ChannelBot,
        payloads: &[Value],
        digest: &str,
    ) -> AppResult<Option<String>> {
        let batches = context.db.collection::<EmailBatch>(BATCHES);
        let filter = doc! { "bot_id": &bot.id, "digest": digest };
        let batch = batches.find_one_and_update(filter.clone(), doc! { "$setOnInsert": {
            "_id": uuid::Uuid::new_v4().to_string(), "bot_id": &bot.id, "user_id": &bot.user_id,
            "digest": digest, "next_offset": 0_i64, "expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::days(31)),
        }}).upsert(true).return_document(mongodb::options::ReturnDocument::After).await?.ok_or_else(upstream)?;
        let token = crate::services::channel_credentials::resolve_bot_token(
            context.db,
            context.encryption_keys,
            self,
            bot,
        )
        .await?;
        let account = self.account(&token, Some(&bot.platform_bot_id)).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let mut failed = false;
        for step in 0..payloads.len() {
            let offset = (batch.next_offset as usize + step) % payloads.len();
            let payload = &payloads[offset];
            if !matches!(payload["changeType"].as_str(), Some("created" | "updated")) {
                continue;
            }
            // A signed but unsupported identifier is a permanent input-policy
            // rejection; it must not prevent later entries from progressing.
            let Some(id) = payload["id"].as_str().and_then(|id| opaque(id).ok()) else {
                continue;
            };
            if tokio::time::Instant::now() >= deadline {
                failed = true;
                break;
            }
            // Advance before the bounded fetch so a timeout/crash gives later
            // items a turn on the producer's next redelivery.
            batches
                .update_one(
                    filter.clone(),
                    doc! { "$set": { "next_offset": ((offset + 1) % payloads.len()) as i64 } },
                )
                .await?;
            let receipts = context.db.collection::<EmailReceipt>(RECEIPTS);
            let receipt_filter = doc! { "bot_id": &bot.id, "platform_message_id": id };
            if receipts
                .find_one(receipt_filter.clone())
                .await?
                .is_some_and(|r| r.completed)
            {
                continue;
            }
            let claim = match EventDedupStore::claim(
                context.db,
                "aurinko-inbound",
                &bot.id,
                id,
                Duration::from_secs(60),
            )
            .await?
            {
                EventDedupClaimResult::Claimed(claim) => claim,
                EventDedupClaimResult::Duplicate => {
                    if !EventDedupStore::is_committed(context.db, "aurinko-inbound", &bot.id, id)
                        .await?
                    {
                        failed = true;
                    }
                    continue;
                }
            };
            let receipt = receipts.find_one_and_update(receipt_filter, doc! { "$setOnInsert": {
                "_id": uuid::Uuid::new_v4().to_string(), "bot_id": &bot.id, "user_id": &bot.user_id,
                "platform_message_id": id, "completed": false, "created_at": bson::DateTime::now(),
            }}).upsert(true).return_document(mongodb::options::ReturnDocument::After).await?.ok_or_else(upstream)?;
            let result = tokio::time::timeout_at(deadline, async {
                if let Some(message) = self.message(&token, id).await?
                    && let Some(inbound) = normalize(bot, &account, &message)?
                {
                    channel_retry_ingress::deliver(context, bot, &inbound, &receipt.id).await?;
                }
                Ok::<(), AppError>(())
            })
            .await
            .unwrap_or_else(|_| Err(channel_retry_ingress::retry_later()));
            if result.is_ok() {
                receipts
                    .update_one(
                        doc! { "_id": &receipt.id },
                        doc! { "$set": { "completed": true } },
                    )
                    .await?;
                if !EventDedupStore::commit(context.db, &claim, COMMIT_TTL).await? {
                    failed = true;
                }
            } else {
                EventDedupStore::release(context.db, &claim).await?;
                failed = true;
            }
        }
        if failed {
            Err(channel_retry_ingress::retry_later())
        } else {
            Ok(None)
        }
    }
    pub(super) async fn reply_once(
        &self,
        db: &mongodb::Database,
        bot: &ChannelBot,
        original: &ChannelMessage,
        credentials: &BotCredentials<'_>,
        conversation: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        if bot.credential_source == "connection" {
            crate::services::aurinko_oauth_service::require_live_connection(
                db,
                &bot.user_id,
                bot.connection_id.as_deref().ok_or_else(invalid)?,
                true,
            )
            .await?;
        }
        let live = crate::services::channel_bot_service::get_bot(db, &bot.id).await?;
        if !live.is_active
            || live.status != "active"
            || live.updated_at != bot.updated_at
            || original.channel_bot_id.as_deref() != Some(&bot.id)
            || original.user_id != bot.user_id
            || original.platform != "aurinko"
            || original.direction != "inbound"
        {
            return Err(AppError::Conflict(
                "Aurinko bot or reply binding changed".into(),
            ));
        }
        let id = original
            .platform_message_id
            .as_deref()
            .ok_or_else(upstream)?;
        let existing = db
            .collection::<EmailSend>(SENDS)
            .find_one(doc! { "bot_id": &bot.id, "platform_message_id": id })
            .await?;
        if let Some(existing) = existing {
            if existing.status == "submitted" {
                return Ok(existing.platform_reply_message_id);
            }
            return Err(AppError::Conflict("Aurinko reply was already attempted; delivery may be uncertain. Check the mailbox before taking further action.".into()));
        }
        let text = reply
            .text
            .as_deref()
            .filter(|v| !v.trim().is_empty() && v.len() <= MAX_BODY)
            .ok_or_else(|| {
                AppError::ValidationError("Email reply must contain 1–262144 bytes of text".into())
            })?;
        if !reply.attachments.is_empty()
            || reply
                .metadata
                .as_ref()
                .is_some_and(|v| v.as_object().is_none_or(|m| !m.is_empty()))
        {
            return Err(AppError::ValidationError("Aurinko channel replies accept text only; recipient overrides and attachments are not supported".into()));
        }
        let account = self
            .account(credentials.token, Some(&bot.platform_bot_id))
            .await?;
        let message = self
            .message(credentials.token, id)
            .await?
            .ok_or_else(|| AppError::NotFound("Original email is no longer available".into()))?;
        let normalized = normalize(bot, &account, &message)?.ok_or_else(|| {
            AppError::Forbidden("Original email is excluded by inbound mail policy".into())
        })?;
        if normalized.conversation_id != conversation
            || normalized.thread_id != original.thread_id
            || original.sender_platform_id.as_deref() != Some(&normalized.sender_platform_id)
        {
            return Err(AppError::Conflict(
                "Aurinko mailbox thread binding changed".into(),
            ));
        }
        let recipient = match message["replyTo"].as_array() {
            Some(values) if values.len() == 1 => address(&values[0]).ok_or_else(upstream)?,
            Some(values) if values.len() > 1 => {
                return Err(AppError::ValidationError(
                    "Multiple Reply-To addresses require manual mailbox review".into(),
                ));
            }
            _ => address(&message["from"]).ok_or_else(upstream)?,
        };
        if mailbox_addresses(&account)
            .iter()
            .any(|own| own.eq_ignore_ascii_case(recipient))
        {
            return Err(AppError::Forbidden(
                "Refusing a reply to this mailbox itself".into(),
            ));
        }
        let subject = message["subject"].as_str().unwrap_or_default();
        let body = json!({"subject": if subject.to_ascii_lowercase().starts_with("re:") { subject.to_string() } else { format!("Re: {subject}") },
            "body": text, "to": [{"address": recipient}], "cc": [], "bcc": [],
            "xHeaders": [{"name": "X-NyxID-Auto-Reply", "value": "true"}, {"name": "X-Auto-Response-Suppress", "value": "All"}]});
        let now = Utc::now();
        let live = crate::services::channel_bot_service::get_bot(db, &bot.id).await?;
        if !live.is_active || live.status != "active" || live.updated_at != bot.updated_at {
            return Err(AppError::Conflict(
                "Aurinko bot changed during reply preflight".into(),
            ));
        }
        // Route/key revocation can happen while the provider preflight is in
        // progress. Reload both bindings before the irreversible send barrier.
        let key_id = original.agent_api_key_id.as_deref().ok_or_else(invalid)?;
        let current = db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .find_one(
                doc! { "_id": &original.id, "user_id": &bot.user_id, "channel_bot_id": &bot.id,
                "conversation_id": &original.conversation_id, "agent_api_key_id": key_id },
            )
            .await?;
        let route = db.collection::<crate::models::channel_conversation::ChannelConversation>(crate::models::channel_conversation::COLLECTION_NAME)
            .find_one(doc! { "_id": &original.conversation_id, "user_id": &bot.user_id, "channel_bot_id": &bot.id,
                "is_active": true, "agent_api_key_id": key_id }).await?;
        let key = db
            .collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
            .find_one(doc! { "_id": key_id, "user_id": &bot.user_id, "is_active": true })
            .await?;
        if current.is_none()
            || route.is_none()
            || key.is_none_or(|key| key.expires_at.is_some_and(|expiry| expiry <= Utc::now()))
        {
            return Err(AppError::Forbidden(
                "Aurinko reply routing authority changed".into(),
            ));
        }
        let send = EmailSend {
            id: uuid::Uuid::new_v4().to_string(),
            bot_id: bot.id.clone(),
            user_id: bot.user_id.clone(),
            account_id: bot.platform_bot_id.clone(),
            platform_message_id: id.into(),
            inbound_message_id: original.id.clone(),
            status: "submitting".into(),
            platform_reply_message_id: None,
            created_at: now,
            updated_at: now,
        };
        // A unique index on (bot_id, platform_message_id) fences JWT and API-key
        // callers and survives process crashes. There is deliberately no TTL.
        db.collection::<EmailSend>(SENDS).insert_one(&send).await?;
        let response = self
            .request(
                Method::POST,
                credentials.token,
                &["email", "messages", id, "reply"],
                &[("bodyType", "text"), ("returnIds", "true")],
                Some(&body),
            )
            .await;
        let result = match response {
            Ok((status, value)) if status.is_success() && value["status"] == "Ok" => Ok(value["id"].as_str().map(String::from)),
            _ => Err(AppError::Conflict("Aurinko reply submission is uncertain; it will not be automatically retried. Check the mailbox.".into())),
        };
        let platform_id = result.as_ref().ok().and_then(|v| v.clone());
        db.collection::<EmailSend>(SENDS).update_one(doc! { "_id": &send.id }, doc! { "$set": {
            "status": if result.is_ok() { "submitted" } else { "uncertain" },
            "platform_reply_message_id": platform_id, "updated_at": bson::DateTime::from_chrono(Utc::now()),
        }}).await?;
        crate::services::audit_service::log_async(
            db.clone(),
            Some(bot.user_id.clone()),
            "channel_email_reply_attempted".into(),
            Some(
                json!({"bot_id": bot.id, "message_id": original.id, "outcome": if result.is_ok() { "submitted" } else { "uncertain" }}),
            ),
            None,
            None,
            original.agent_api_key_id.clone(),
            None,
        );
        result
    }
}
