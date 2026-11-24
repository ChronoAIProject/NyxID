use crate::models::reply_token_use::ReplyTokenUse;
use crate::{
    errors::{AppError, AppResult},
    models::{
        channel_bot::ChannelBot,
        channel_message::ChannelMessage,
        channel_reply_delivery::{COLLECTION_NAME, ChannelReplyDelivery, PartStatus, ReplyPart},
    },
};
use bson::doc;

pub const MAX_REPLY_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_REPLY_PARTS: usize = 64;

pub struct Delivery {
    meta_grant: Option<crate::models::channel_meta::GrantRef>,
    id: String,
    bot_id: String,
    inbound_id: String,
    telegram: bool,
    parts: Vec<ReplyPart>,
    trace: Option<crate::turn_trace::TurnContext>,
    operations: Vec<Option<crate::turn_trace::Operation>>,
}

impl Delivery {
    pub async fn begin(
        db: &mongodb::Database,
        bot: &ChannelBot,
        original: &ChannelMessage,
        count: usize,
    ) -> AppResult<Self> {
        let record = Self::record(uuid::Uuid::new_v4().to_string(), bot, original, count)?;
        if super::channel_adapters::meta::is_meta(&bot.platform) {
            super::transaction_service::run_db_only(
                db,
                &(db, &record, bot, original, bot.platform.as_str()),
                |tx, &(db, record, bot, original, platform)| {
                    Box::pin(async move {
                        super::channel_relay_service::fence_meta_observation(
                            db,
                            tx,
                            &record.channel_bot_id,
                            platform,
                            true,
                        )
                        .await?;
                        let binding=bot.meta_binding.as_ref().ok_or_else(unconfirmed)?;
                        let source=original.meta_authority.as_ref().ok_or_else(unconfirmed)?;
                        if source.generation != binding.generation || source.grant_id.as_ref().is_some_and(|grant|grant != &binding.grant_id) {
                            return Err(unconfirmed());
                        }
                        let mut filter=doc! {"_id":&bot.id,"user_id":&bot.user_id,"is_active":true,"meta_binding.generation":binding.generation};
                        filter.extend(super::channel_meta_service::grants::grant_filter(&binding.grant_id));
                        if db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME).find_one(filter).session(&mut *tx).await?.is_none() {return Err(unconfirmed());}
                        super::channel_meta_service::scopes::fence_current(db,tx,bot).await?;
                        db.collection::<ChannelReplyDelivery>(COLLECTION_NAME)
                            .insert_one(record)
                            .session(&mut *tx)
                            .await?;
                        Ok(())
                    })
                },
            )
            .await?;
        } else {
            db.collection::<ChannelReplyDelivery>(COLLECTION_NAME)
                .insert_one(&record)
                .await?;
        }
        Ok(Self::from_record(record, bot))
    }

    /// The adapter is ready to POST a single fixed notice. Its caller commits
    /// this dispatch barrier together with the final authority check.
    pub(crate) async fn claim_org_notice(
        db: &mongodb::Database,
        session: &mut mongodb::ClientSession,
        bot: &ChannelBot,
        source: &ChannelMessage,
        token: Option<&ReplyTokenUse>,
    ) -> AppResult<Self> {
        let identity = format!("org-thread-notice:v1:{}:{}", bot.id, source.id);
        let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, identity.as_bytes());
        let mut record = Self::record(id.to_string(), bot, source, 1)?;
        record.parts[0].status = PartStatus::Uncertain;
        db.collection::<ChannelReplyDelivery>(COLLECTION_NAME)
            .insert_one(&record)
            .session(&mut *session)
            .await
            .map_err(claim_error)?;
        if let Some(token) = token {
            super::channel_relay_service::fence_reply_token_owners(
                db,
                token,
                &source.user_id,
                session,
            )
            .await?;
            db.collection::<ReplyTokenUse>(crate::models::reply_token_use::COLLECTION_NAME)
                .insert_one(token)
                .session(session)
                .await
                .map_err(claim_error)?;
        }
        Ok(Self::from_record(record, bot))
    }

    fn record(
        id: String,
        bot: &ChannelBot,
        original: &ChannelMessage,
        count: usize,
    ) -> AppResult<ChannelReplyDelivery> {
        if count == 0 || count > MAX_REPLY_PARTS {
            return Err(AppError::ValidationError(
                "Reply exceeds the 64-part limit".into(),
            ));
        }
        Ok(ChannelReplyDelivery {
            meta_grant: if super::channel_adapters::meta::is_meta(&bot.platform) {
                Some(super::channel_meta_service::scopes::reference(bot)?)
            } else {
                None
            },
            id,
            user_id: bot.user_id.clone(),
            channel_bot_id: bot.id.clone(),
            inbound_message_id: original.id.clone(),
            expected_parts: count as u32,
            complete: false,
            parts: vec![
                ReplyPart {
                    formatting_fallback: None,
                    status: PartStatus::NotAttempted,
                    platform_message_ids: Vec::new(),
                    outbound_message_id: None,
                    failure_code: None,
                };
                count
            ],
            created_at: chrono::Utc::now(),
        })
    }

    fn from_record(record: ChannelReplyDelivery, bot: &ChannelBot) -> Self {
        let count = record.parts.len();
        let trace = crate::turn_trace::TurnContext::current();
        Self {
            meta_grant: record.meta_grant,
            id: record.id,
            bot_id: bot.id.clone(),
            inbound_id: record.inbound_message_id,
            telegram: matches!(bot.platform.as_str(), "telegram" | "telegram-new"),
            parts: record.parts,
            trace,
            operations: if trace.is_some() {
                (0..count).map(|_| None).collect()
            } else {
                Vec::new()
            },
        }
    }

    pub fn formatting_receipt(&self, index: usize) -> super::telegram_formatting::Receipt {
        super::telegram_formatting::Receipt {
            collection: COLLECTION_NAME,
            filter: doc! {"_id": &self.id, "complete":false, format!("parts.{index}.status"): "uncertain"},
            field: format!("parts.{index}.formatting_fallback"),
            plain: false,
        }
    }

    pub async fn attempting(&mut self, db: &mongodb::Database, index: usize) -> AppResult<()> {
        let previous = self.parts[index].status;
        self.parts[index].status = PartStatus::Uncertain;
        if let Err(error) = self.save_part(db, index, true).await {
            self.parts[index].status = previous;
            return Err(error);
        }
        self.trace_attempt(index);
        Ok(())
    }

    pub(crate) fn trace_attempt(&mut self, index: usize) {
        if let Some(operation) = self.operations.get_mut(index) {
            *operation = Some(crate::turn_trace::Operation::part(self.trace, index));
        }
    }

    pub async fn accepted(
        &mut self,
        db: &mongodb::Database,
        index: usize,
        ids: Vec<String>,
        outbound_message_id: &str,
    ) -> AppResult<()> {
        self.parts[index] = ReplyPart {
            formatting_fallback: None,
            status: PartStatus::Accepted,
            platform_message_ids: ids,
            outbound_message_id: Some(outbound_message_id.into()),
            failure_code: None,
        };
        if let Some(Some(operation)) = self.operations.get_mut(index) {
            operation.finish(crate::turn_trace::Outcome::Confirmed, None, None);
        }
        self.save_part(db, index, false).await
    }

    pub async fn failed(
        &mut self,
        db: &mongodb::Database,
        index: usize,
        error: &AppError,
    ) -> AppResult<()> {
        let part = &mut self.parts[index];
        let fresh = part.status == PartStatus::NotAttempted;
        if part.status == PartStatus::NotAttempted
            || (self.telegram && super::telegram_formatting::definite_refusal(error))
        {
            part.status = PartStatus::Failed;
        }
        part.failure_code = Some(error.error_code());
        if let Some(Some(operation)) = self.operations.get_mut(index) {
            operation.finish(
                if part.status == PartStatus::Failed {
                    crate::turn_trace::Outcome::Rejected
                } else {
                    crate::turn_trace::Outcome::Unknown
                },
                None,
                Some(error.error_code()),
            );
        }
        self.save_part(db, index, fresh).await?;
        if self.telegram {
            super::channel_reply_notice::failed(db, &self.id, &self.bot_id, &self.inbound_id)
                .await?;
        }
        Ok(())
    }

    async fn save_part(&self, db: &mongodb::Database, index: usize, fresh: bool) -> AppResult<()> {
        let fields = bson::to_document(&self.parts[index]).map_err(|_| unconfirmed())?;
        let fields: bson::Document = fields
            .into_iter()
            .map(|(key, value)| (format!("parts.{index}.{key}"), value))
            .collect();
        let mut filter = doc! {"_id":&self.id,"complete":false};
        if self.meta_grant.is_some() && !fresh {
            filter.insert(format!("parts.{index}.status"), "uncertain");
        }
        let result = super::channel_meta_service::scopes::update_record(
            db,
            self.meta_grant.as_ref(),
            fresh,
            COLLECTION_NAME,
            filter,
            doc! {"$set":fields},
        )
        .await?;
        if result.matched_count != 1 {
            return Err(unconfirmed());
        }
        Ok(())
    }

    pub async fn complete(self, db: &mongodb::Database) -> AppResult<()> {
        if !self
            .parts
            .iter()
            .all(|part| part.status == PartStatus::Accepted)
        {
            return Err(unconfirmed());
        }
        let result = super::channel_meta_service::scopes::update_record(
            db,
            self.meta_grant.as_ref(),
            false,
            COLLECTION_NAME,
            doc! {"_id":&self.id,"complete":false},
            doc! {"$set":{"complete":true}},
        )
        .await?;
        if result.matched_count != 1 {
            return Err(unconfirmed());
        }
        if let Some(trace) = self.trace {
            crate::turn_trace::milestone(
                &trace.turn_id(),
                crate::turn_trace::Stage::Channel,
                None,
                Some(crate::turn_trace::Outcome::Confirmed),
            );
        }
        Ok(())
    }
}

fn claim_error(error: mongodb::error::Error) -> AppError {
    if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
    {
        AppError::Unauthorized("Reply already claimed".into())
    } else {
        error.into()
    }
}

pub fn unconfirmed() -> AppError {
    AppError::ChannelPlatformError("Complete reply delivery was not confirmed".into())
}

/// Relay clients may retry HTTP 429. Once dispatch is possible, that could
/// duplicate an accepted prefix; keep the original numeric code in the journal.
pub fn failure_after_dispatch(error: AppError) -> AppError {
    match error {
        AppError::RateLimited
        | AppError::RateLimitedWithRetry { .. }
        | AppError::ServiceConcurrencyLimited => unconfirmed(),
        other => other,
    }
}

/// Authentication has already admitted this logical request. Recheck revocation
/// between parts without consuming the single-use reply token again.
#[allow(clippy::too_many_arguments)]
pub async fn send_parts(
    db: &mongodb::Database,
    http: &reqwest::Client,
    adapter: &dyn super::channel_platform::PlatformAdapter,
    bot: &ChannelBot,
    original: &ChannelMessage,
    attributed_key: &str,
    chat: &str,
    credentials: &super::channel_platform::BotCredentials<'_>,
    parts: Vec<super::channel_platform::OutboundReply>,
) -> AppResult<(ChannelMessage, super::channel_platform::SendOutcome)> {
    let mut delivery = Delivery::begin(db, bot, original, parts.len()).await?;
    let mut last = None;
    for (index, part) in parts.into_iter().enumerate() {
        let provider_attempted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sent = Box::pin(async {
            super::ownership_transfer_service::require_current_bot(db, bot).await?;
            let now = bson::DateTime::now();
            if db
                .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                .find_one(doc! {"_id": attributed_key, "is_active": true,
                "$or": [{"expires_at": bson::Bson::Null}, {"expires_at": {"$gt": now}}]})
                .await?
                .is_none()
            {
                return Err(AppError::Unauthorized(
                    "Reply authority is no longer active".into(),
                ));
            }
            delivery.attempting(db, index).await?;
            let mut outcome = super::telegram_formatting::scope(
                db,
                delivery.formatting_receipt(index),
                Some(&bot.id),
                super::channel_platform::track_provider_attempt(
                    provider_attempted.clone(),
                    adapter.send_bound_reply_outcome(
                        db,
                        http,
                        bot,
                        original,
                        credentials,
                        chat,
                        &part,
                        None,
                    ),
                ),
            )
            .await?;
            if let Some(record) = &mut outcome.observation {
                record.waba_id = bot.app_id.clone();
            }
            let stored = super::channel_relay_service::store_outbound_message(
                db,
                &bot.id,
                &original.conversation_id,
                &bot.user_id,
                &bot.platform,
                attributed_key,
                Some(&original.id),
                outcome.final_id.as_deref(),
                Some(chat),
                "text",
                outcome.observation.clone(),
            )
            .await?;
            let ids = outcome.observation.as_ref().map_or_else(
                || outcome.final_id.iter().cloned().collect(),
                |record| record.component_ids.clone(),
            );
            if let Some(error) = outcome.error.take() {
                if outcome
                    .observation
                    .as_ref()
                    .is_some_and(|record| !record.uncertain && record.component_ids.is_empty())
                {
                    delivery.parts[index].status = PartStatus::Failed;
                }
                delivery.parts[index].platform_message_ids = ids;
                delivery.parts[index].outbound_message_id = Some(stored.id);
                // An issued Meta attempt has one terminal transition. failed()
                // publishes both the observation and the provider's refusal code.
                if delivery.meta_grant.is_none() {
                    delivery.save_part(db, index, false).await?;
                }
                return Err(error);
            }
            outcome
                .final_id
                .as_deref()
                .filter(|id| super::channel_thread_service::valid_id(id))
                .ok_or_else(unconfirmed)?;
            if outcome.observation.as_ref().is_some_and(|record| {
                !record.complete || record.uncertain || record.component_ids.is_empty()
            }) {
                return Err(unconfirmed());
            }
            delivery.accepted(db, index, ids, &stored.id).await?;
            Ok((stored, outcome))
        })
        .await;
        match sent {
            Ok(sent) => last = Some(sent),
            Err(error) => {
                if super::channel_adapters::meta::is_meta(&bot.platform)
                    && delivery.parts[index].status == PartStatus::Uncertain
                    && !provider_attempted.load(std::sync::atomic::Ordering::Relaxed)
                {
                    delivery.parts[index].status = PartStatus::Failed;
                }
                delivery.failed(db, index, &error).await?;
                return Err(failure_after_dispatch(error));
            }
        }
    }
    delivery.complete(db).await?;
    last.ok_or_else(unconfirmed)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_saved_parts(
    key: &str,
    db: &mongodb::Database,
    http: &reqwest::Client,
    adapter: &dyn super::channel_platform::PlatformAdapter,
    bot: &ChannelBot,
    original: &ChannelMessage,
    attributed_key: &str,
    chat: &str,
    credentials: &super::channel_platform::BotCredentials<'_>,
    parts: Vec<super::channel_platform::OutboundReply>,
) -> AppResult<(ChannelMessage, super::channel_platform::SendOutcome)> {
    use super::{channel_text_delivery as journal, channel_text_preparation as preparation};
    let plan = journal::begin_bound(
        db,
        key,
        bot,
        &original.id,
        &super::channel_reply_plan::fingerprint(&parts),
        parts.len(),
    )
    .await?;
    let mut last = None;
    for (index, part) in parts.into_iter().enumerate() {
        match preparation::claim(db, &plan, index).await? {
            journal::PartClaim::Sent(previous) => {
                let id = previous.outbound_message_id.ok_or_else(unconfirmed)?;
                last = Some((
                    super::channel_relay_service::get_message(db, &id).await?,
                    super::channel_platform::SendOutcome::legacy(previous.platform_message_id),
                ));
            }
            journal::PartClaim::Send(attempt) => {
                let (stored, outcome) =
                    Box::pin(preparation::with_part(db, &plan, index, &attempt, async {
                        preparation::effect().await?;
                        send_parts(
                            db,
                            http,
                            adapter,
                            bot,
                            original,
                            attributed_key,
                            chat,
                            credentials,
                            vec![part],
                        )
                        .await
                    }))
                    .await?;
                journal::finish(
                    db,
                    &plan,
                    index,
                    &attempt,
                    crate::models::channel_text_delivery::TextPartState::Sent,
                    outcome.final_id.as_deref(),
                    Some(&stored.id),
                )
                .await?;
                last = Some((stored, outcome));
            }
        }
    }
    last.ok_or_else(unconfirmed)
}

pub async fn ensure_indexes(db: &mongodb::Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    let collection = db.collection::<ChannelReplyDelivery>(COLLECTION_NAME);
    collection
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"created_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::from_secs(30 * 86400))
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"channel_bot_id": 1, "inbound_message_id": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id": 1, "inbound_message_id": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"parts.outbound_message_id": 1})
                .build(),
        ])
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatched_rate_limit_cannot_invite_a_whole_reply_retry() {
        for error in [
            AppError::RateLimited,
            AppError::RateLimitedWithRetry { retry_after: 3 },
            AppError::ServiceConcurrencyLimited,
        ] {
            assert!(matches!(
                failure_after_dispatch(error),
                AppError::ChannelPlatformError(_)
            ));
        }
    }
}
