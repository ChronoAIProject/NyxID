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
    id: String,
    parts: Vec<ReplyPart>,
}

impl Delivery {
    pub async fn begin(
        db: &mongodb::Database,
        bot: &ChannelBot,
        original: &ChannelMessage,
        count: usize,
    ) -> AppResult<Self> {
        if count == 0 || count > MAX_REPLY_PARTS {
            return Err(AppError::ValidationError(
                "Reply exceeds the 64-part limit".into(),
            ));
        }
        let record = ChannelReplyDelivery {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: bot.user_id.clone(),
            channel_bot_id: bot.id.clone(),
            inbound_message_id: original.id.clone(),
            expected_parts: count as u32,
            complete: false,
            parts: vec![
                ReplyPart {
                    status: PartStatus::NotAttempted,
                    platform_message_ids: Vec::new(),
                    outbound_message_id: None,
                    failure_code: None,
                };
                count
            ],
            created_at: chrono::Utc::now(),
        };
        db.collection::<ChannelReplyDelivery>(COLLECTION_NAME)
            .insert_one(&record)
            .await?;
        Ok(Self {
            id: record.id,
            parts: record.parts,
        })
    }

    pub async fn attempting(&mut self, db: &mongodb::Database, index: usize) -> AppResult<()> {
        self.parts[index].status = PartStatus::Uncertain;
        self.save_part(db, index).await
    }

    pub async fn accepted(
        &mut self,
        db: &mongodb::Database,
        index: usize,
        ids: Vec<String>,
        outbound_message_id: &str,
    ) -> AppResult<()> {
        self.parts[index] = ReplyPart {
            status: PartStatus::Accepted,
            platform_message_ids: ids,
            outbound_message_id: Some(outbound_message_id.into()),
            failure_code: None,
        };
        self.save_part(db, index).await
    }

    pub async fn failed(
        &mut self,
        db: &mongodb::Database,
        index: usize,
        error: &AppError,
    ) -> AppResult<()> {
        let part = &mut self.parts[index];
        if part.status == PartStatus::NotAttempted {
            part.status = PartStatus::Failed;
        }
        part.failure_code = Some(error.error_code());
        self.save_part(db, index).await
    }

    pub async fn observed(
        &mut self,
        db: &mongodb::Database,
        index: usize,
        ids: Vec<String>,
        outbound_message_id: &str,
    ) -> AppResult<()> {
        self.parts[index].platform_message_ids = ids;
        self.parts[index].outbound_message_id = Some(outbound_message_id.into());
        self.save_part(db, index).await
    }

    async fn save_part(&self, db: &mongodb::Database, index: usize) -> AppResult<()> {
        let result = db
            .collection::<ChannelReplyDelivery>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &self.id, "complete": false},
                doc! {"$set": {format!("parts.{index}"): bson::to_bson(&self.parts[index]).map_err(|_| unconfirmed())?}},
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
        let result = db
            .collection::<ChannelReplyDelivery>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &self.id, "complete": false},
                doc! {"$set": {"complete": true}},
            )
            .await?;
        if result.matched_count != 1 {
            return Err(unconfirmed());
        }
        Ok(())
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
            let mut outcome = adapter
                .send_bound_reply_outcome(db, http, bot, original, credentials, chat, &part, None)
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
                delivery.observed(db, index, ids, &stored.id).await?;
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
                delivery.failed(db, index, &error).await?;
                return Err(failure_after_dispatch(error));
            }
        }
    }
    delivery.complete(db).await?;
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
