use super::inbox::{InboxError, advance_head, commit, decode_stored};
use super::inbox_cursor::MAX_PENDING_DISTANCE;
use crate::models::sandbox_usage_inbox::{EVENTS, Event as StoredEvent, STREAMS, Stream};
use bson::doc;
use futures::TryStreamExt;
use mongodb::{
    Database,
    options::{Acknowledgment, ReadConcern, WriteConcern},
};

pub(super) async fn promote_stream(
    db: &Database,
    namespace: &str,
    stream_id: &str,
) -> Result<(), InboxError> {
    for _ in 0..MAX_PENDING_DISTANCE {
        if !promote_one(db, namespace, stream_id).await? {
            break;
        }
    }
    Ok(())
}

async fn promote_one(db: &Database, namespace: &str, stream_id: &str) -> Result<bool, InboxError> {
    let streams = db.collection::<Stream>(STREAMS);
    let events = db.collection::<StoredEvent>(EVENTS);
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
        .await?;
    let result = async {
        let Some(mut stream) = streams
            .find_one(doc! {
                "namespace":namespace, "stream_id":stream_id, "promotion_ready":true,
            })
            .max_time(std::time::Duration::from_secs(5))
            .session(&mut session)
            .await?
        else {
            return Ok(false);
        };
        let next = stream.sequence.checked_add(1).ok_or(InboxError::Corrupt)?;
        let accepted = stream
            .pending
            .first()
            .filter(|item| item.sequence == next)
            .ok_or(InboxError::Corrupt)?;
        let row = events
            .find_one(doc! {
                "namespace":namespace, "stream_id":stream_id, "sequence":next,
                "validation":"pending", "digest":&accepted.digest,
            })
            .session(&mut session)
            .await?
            .ok_or(InboxError::Corrupt)?;
        let previous = events
            .find_one(doc! {"_id":&stream.head_id, "digest":&stream.head_digest})
            .session(&mut session)
            .await?
            .ok_or(InboxError::Corrupt)?;
        let event = decode_stored(&row)?;
        let head = decode_stored(&previous)?;
        let old_head = stream.head_id.clone();
        stream.pending.remove(0);
        let advanced_head =
            advance_head(db, &mut session, &mut stream, &event, &head, &row.id).await;
        if let Err(InboxError::Conflict(reason)) = advanced_head {
            let changed = events
                .update_one(
                    doc! {"_id":&row.id,"validation":"pending"},
                    doc! {"$set":{"validation":"rejected","validation_reason":reason}},
                )
                .session(&mut session)
                .await?
                .matched_count;
            let blocked = streams
                .update_one(
                    doc! {"_id":&stream.id,"head_id":old_head},
                    doc! {"$set":{"promotion_ready":false}},
                )
                .session(&mut session)
                .await?
                .matched_count;
            if changed != 1 || blocked != 1 {
                return Err(InboxError::Unavailable);
            }
            commit(&mut session).await?;
            return Ok(false);
        }
        advanced_head?;
        let more = stream.promotion_ready;
        let changed = events
            .update_one(
                doc! {"_id":&row.id,"validation":"pending"},
                doc! {"$set":{"validation":"validated"}},
            )
            .session(&mut session)
            .await?
            .matched_count;
        let advanced = streams
            .replace_one(doc! {"_id":&stream.id,"head_id":old_head}, stream)
            .session(&mut session)
            .await?
            .matched_count;
        if changed != 1 || advanced != 1 {
            return Err(InboxError::Unavailable);
        }
        commit(&mut session).await?;
        Ok(more)
    }
    .await;
    if !matches!(&result, Err(InboxError::Database(error)) if error.contains_label("UnknownTransactionCommitResult"))
        && !matches!(result, Ok(true))
    {
        let _ = session.abort_transaction().await;
    }
    result
}

async fn reconcile_pending(db: &Database, after: &str) -> Result<Option<String>, InboxError> {
    let mut streams = db
        .collection::<Stream>(STREAMS)
        .find(doc! {"promotion_ready":true,"_id":{"$gt":after}})
        .sort(doc! {"_id":1})
        .limit(16)
        .await?;
    let mut last = None;
    while let Some(stream) = streams.try_next().await? {
        if let Err(error) = promote_stream(db, &stream.namespace, &stream.stream_id).await {
            tracing::warn!(error = %error, "Usage custody validation remains pending");
        }
        last = Some(stream.id);
    }
    Ok(last)
}

pub fn spawn_reconciler(db: Database) {
    crate::services::shutdown::process().spawn_worker(async move {
        let mut after = String::new();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while let Some(_work) = crate::services::shutdown::process().tick(&mut tick).await {
            match reconcile_pending(&db, &after).await {
                Ok(next) => after = next.unwrap_or_default(),
                Err(error) => tracing::warn!(error = %error, "Usage custody reconciliation failed"),
            }
        }
    });
}
