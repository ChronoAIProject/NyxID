//! Leased, adapter-driven polling. No message bodies or credentials are persisted here.

use std::time::Duration;

use bson::doc;
use chrono::Utc;
use futures::{StreamExt, TryStreamExt, stream};
use mongodb::options::ReturnDocument;

use super::channel_inbound_service::{InboundDeps, process_inbound_messages};
use super::channel_platform::{BotCredentials, Ingestion, PlatformAdapter};
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{COLLECTION_NAME, ChannelBot};

const LEASE_SECS: i64 = 90;
pub const ERROR_THRESHOLD: u32 = 5;

pub async fn sweep(state: &crate::AppState) -> AppResult<()> {
    sweep_with_adapters(
        state,
        super::channel_adapters::registered_adapters(&state.token_exchange_cache),
    )
    .await
}

pub(crate) async fn sweep_with_adapters(
    state: &crate::AppState,
    adapters: Vec<Box<dyn PlatformAdapter>>,
) -> AppResult<()> {
    for adapter in adapters {
        let Ingestion::Poll { min_interval_secs } = adapter.ingestion() else {
            continue;
        };
        let now = Utc::now();
        let filter = due_filter(adapter.platform_id(), min_interval_secs, now);
        let bots: Vec<ChannelBot> = state
            .db
            .collection::<ChannelBot>(COLLECTION_NAME)
            .find(filter)
            .sort(doc! { "last_polled_at": 1, "_id": 1 })
            .limit(100)
            .await?
            .try_collect()
            .await?;
        let adapter = adapter.as_ref();
        stream::iter(bots)
            .for_each_concurrent(8, |bot| async move {
                if poll_bot(state, adapter, &bot.id, min_interval_secs)
                    .await
                    .is_err()
                {
                    tracing::warn!(bot_id = %bot.id, "Channel poll failed; continuing sweep");
                }
            })
            .await;
    }
    Ok(())
}

fn due_filter(
    platform: &str,
    min_interval_secs: u64,
    now: chrono::DateTime<Utc>,
) -> bson::Document {
    let now_bson = bson::DateTime::from_chrono(now);
    doc! {
        "platform": platform, "is_active": true, "status": "active",
        "$and": [
            { "$or": [{ "poll_lease_until": null }, { "poll_lease_until": { "$lte": now_bson } }] },
            { "$or": [{ "poll_backoff_until": null }, { "poll_backoff_until": { "$lte": now_bson } }] },
            { "$or": [{ "last_polled_at": null }, { "last_polled_at": { "$lte": bson::DateTime::from_chrono(now - chrono::Duration::seconds(min_interval_secs.min(i64::MAX as u64) as i64)) } }] },
        ],
    }
}

pub(crate) async fn poll_bot(
    state: &crate::AppState,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    min_interval_secs: u64,
) -> AppResult<()> {
    let collection = state.db.collection::<ChannelBot>(COLLECTION_NAME);
    let mut filter = due_filter(adapter.platform_id(), min_interval_secs, Utc::now());
    filter.insert("_id", bot_id);
    let mut lease_until =
        bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(LEASE_SECS));
    let Some(bot) = collection
        .find_one_and_update(filter, doc! { "$set": { "poll_lease_until": lease_until } })
        .return_document(ReturnDocument::After)
        .await?
    else {
        return Ok(());
    };

    let work = async {
        let token = super::channel_credentials::resolve_bot_token(
            &state.db,
            &state.encryption_keys,
            adapter,
            &bot,
        )
        .await?;
        let credentials = BotCredentials {
            token: &token,
            platform_bot_id: Some(&bot.platform_bot_id),
            platform_secrets: None,
        };
        let outcome = adapter
            .poll_inbound(&state.http_client, &credentials, bot.poll_cursor.as_deref())
            .await?;
        let complete =
            process_inbound_messages(InboundDeps::from(state), &bot, adapter, &outcome.messages)
                .await
                .map_err(|_| {
                    AppError::ChannelPlatformError(
                        "Inbound channel processing failed; the cursor was retained".to_string(),
                    )
                })?;
        if !complete {
            return Err(AppError::ChannelPlatformError(
                "Inbound channel processing failed; the cursor was retained".to_string(),
            ));
        }
        Ok(outcome)
    };
    let work = tokio::time::timeout(Duration::from_secs(15 * 60), work);
    tokio::pin!(work);
    let mut renew = tokio::time::interval(Duration::from_secs(20));
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renew.tick().await;
    let result = loop {
        tokio::select! {
            result = &mut work => break result.unwrap_or_else(|_| Err(AppError::ChannelPlatformError("Channel poll timed out; retry later".to_string()))),
            _ = renew.tick() => {
                let next = bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(LEASE_SECS));
                match collection.update_one(doc! { "_id": &bot.id, "is_active": true, "status": "active", "connection_id": &bot.connection_id,
                    "poll_lease_until": { "$eq": lease_until, "$gt": bson::DateTime::now() } },
                    doc! { "$set": { "poll_lease_until": next } }).await {
                    Ok(result) if result.modified_count == 1 => lease_until = next,
                    _ => break Err(AppError::ChannelPlatformError("Channel polling lease was lost; retry later".to_string())),
                }
            }
        }
    };

    // Every outcome releases only this claim; a reconnect or replacement claim fences the write.
    let fence = doc! { "_id": &bot.id, "poll_lease_until": lease_until, "connection_id": &bot.connection_id };
    let mut set = doc! { "last_polled_at": bson::DateTime::now(), "poll_lease_until": null };
    let failure = match result {
        Ok(outcome) => {
            if let Some(notice) = outcome.notice {
                set.insert("last_poll_notice", notice);
            }
            set.insert("poll_cursor", outcome.cursor);
            set.insert(
                "poll_backoff_until",
                outcome.backoff.map(|d| {
                    bson::DateTime::from_chrono(
                        Utc::now() + chrono::Duration::seconds(d.as_secs().min(86400) as i64),
                    )
                }),
            );
            set.insert("poll_error_count", 0);
            set.insert("error", bson::Bson::Null);
            None
        }
        Err(error) => {
            let cause = match error {
                AppError::ChannelPlatformError(cause) | AppError::ValidationError(cause) => cause,
                _ => "Channel polling is unavailable; retry or reconnect the account".to_string(),
            };
            let count = bot.poll_error_count.saturating_add(1);
            set.insert("poll_error_count", i64::from(count));
            set.insert("error", &cause);
            set.insert(
                "poll_backoff_until",
                bson::DateTime::from_chrono(
                    Utc::now() + chrono::Duration::seconds(30 * 2_i64.pow(count.min(6))),
                ),
            );
            (count >= ERROR_THRESHOLD).then_some(cause)
        }
    };
    // Check expiry at commit as well as renewal. An expired worker cannot advance the cursor.
    let mut active_fence = fence.clone();
    active_fence.insert("is_active", true);
    active_fence.insert("status", "active");
    active_fence.insert(
        "poll_lease_until",
        doc! { "$eq": lease_until, "$gt": bson::DateTime::now() },
    );
    if failure.is_some() {
        set.insert("status", "failed");
        set.insert("updated_at", bson::DateTime::now());
    }
    let committed = collection
        .update_one(active_fence, doc! { "$set": set })
        .await;
    let released = collection
        .update_one(fence, doc! { "$set": { "poll_lease_until": null } })
        .await;
    let committed = committed?;
    released?;
    if committed.modified_count > 0
        && let Some(cause) = failure
    {
        super::channel_credentials::audit_failure(&state.db, &bot, &cause).await?;
    }
    Ok(())
}
