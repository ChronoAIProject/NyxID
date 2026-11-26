//! Existing bot Health evidence for authenticated org thread notice refusals.
use crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME as EVENTS;
use crate::services::channel_thread_service::gateway::{self, ReplyFailure};
use bson::{Document, doc};
use futures::{TryStreamExt, future::BoxFuture};
use mongodb::{Database, options::Hint};
use std::time::Duration;

pub(crate) fn record_refusal<'a>(
    db: &'a Database,
    channel_id: &str,
    source_id: &str,
) -> BoxFuture<'a, ()> {
    let filter = doc! {"channel_id": channel_id, "event_id": source_id, "org_notice.pending": true};
    Box::pin(async move {
        record_matching_refusal(db, filter).await;
    })
}

/// Earlier relay preflight has authenticated the token/key against retained ingress,
/// but might no longer be able to load the retired route or channel.
pub(crate) fn record_source_refusal<'a>(
    db: &'a Database,
    bot_id: &str,
    source_id: &str,
) -> BoxFuture<'a, bool> {
    let filter = doc! {"org_notice.bot_id": bot_id, "event_id": source_id};
    Box::pin(record_matching_refusal(db, filter))
}

async fn record_matching_refusal(db: &Database, mut filter: Document) -> bool {
    filter.extend(
        doc! {"org_notice.version":1,"turn_id":null,"conversation_id":null,"status":"refused"},
    );
    let events = db.collection::<Document>(EVENTS);
    let error = crate::errors::AppError::Conflict("thread_target_unavailable".into());
    let mut unreported = filter.clone();
    unreported.extend(doc! {"org_notice.refusal_recorded":{"$ne":true}});
    match events
        .find_one_and_update(
            unreported,
            doc! {"$set":{"org_notice.refusal_recorded":true}},
        )
        .projection(
            doc! {"event_id":1,"delivery.origin.org_execution.org_id":1,"org_notice.bot_id":1},
        )
        .await
    {
        Ok(Some(event)) => {
            let org = event
                .get_document("delivery")
                .and_then(|d| d.get_document("origin"))
                .and_then(|d| d.get_document("org_execution"))
                .and_then(|d| d.get_str("org_id"));
            if let (Ok(source), Ok(owner), Some(bot)) = (
                event.get_str("event_id"),
                org,
                event
                    .get_document("org_notice")
                    .ok()
                    .and_then(|notice| notice.get_str("bot_id").ok()),
            ) {
                gateway::record_outcome(db, bot, owner, source, ReplyFailure::Target, &error).await;
                return true;
            }
            false
        }
        Ok(None) => {
            let prior = events
                .find_one(filter)
                .projection(doc! {"org_notice.bot_id":1})
                .await;
            if let Ok(Some(event)) = prior
                && let Ok(bot) = event
                    .get_document("org_notice")
                    .and_then(|notice| notice.get_str("bot_id"))
            {
                // An intervening success can clear Health. Preserve the single audit
                // while showing this authenticated refusal as the latest outcome.
                super::channel_bot_service::record_outbound_outcome_with_reason(
                    db,
                    bot,
                    Some(&error),
                    Some(ReplyFailure::Target),
                )
                .await;
                return true;
            }
            false
        }
        Err(_) => false,
    }
}

/// Retirement has already committed. Inspect at most one page on the partial
/// pending-notice index. Bot Health needs one representative failure even when
/// incoming traffic has left a large backlog.
pub(crate) fn after_retirement(db: Database, channel_id: String) {
    super::shutdown::process().spawn_cleanup(async move {
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            let mut pending = db
                .collection::<Document>(EVENTS)
                .find(doc! {
                    "channel_id":&channel_id,"org_notice.pending":true,
                    "org_notice.expires_at":{"$gt":bson::DateTime::now()},
                })
                .sort(doc! {"org_notice.expires_at":-1})
                .limit(1)
                .hint(Hint::Name("org_notice_pending".into()))
                .projection(doc! {"event_id":1,"_id":0})
                .max_time(Duration::from_millis(250))
                .await?;
            if let Some(row) = pending.try_next().await?
                && let Ok(source) = row.get_str("event_id")
            {
                record_refusal(&db, &channel_id, source).await;
            }
            mongodb::error::Result::Ok(())
        })
        .await;
    });
}
