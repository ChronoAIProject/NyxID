//! Exact identities for chat moves and removals. Provider partitions are reusable.
use crate::{
    errors::{AppError, AppResult},
    models::nyxbot_channel::{NyxbotChannel, NyxbotThread, THREADS_COLLECTION_NAME as THREADS},
};
use bson::{Bson, Document, doc};
use mongodb::{ClientSession, Database};

pub(crate) fn projection() -> Document {
    doc! {"_id":1,"channel_id":1,"partition":1,"kind":1,"relay_partition":1}
}

pub(crate) fn observed(row: &Document) -> AppResult<Document> {
    let id = row
        .get("_id")
        .ok_or_else(|| AppError::Internal("Chat identity unavailable".into()))?;
    let mut filter = doc! {"_id":{"$eq":id}};
    for field in ["channel_id", "partition", "kind", "relay_partition"] {
        filter.insert(field, doc! {"$eq":row.get(field).unwrap_or(&Bson::Null)});
    }
    Ok(filter)
}

pub(crate) fn exact(row: &NyxbotThread) -> Document {
    doc! {"_id":&row.id,"channel_id":&row.channel_id,"partition":&row.partition,
    "kind":&row.kind,"relay_partition":&row.relay_partition}
}

pub(crate) async fn delete_observed(
    db: &Database,
    tx: &mut ClientSession,
    rows: &[Document],
) -> AppResult<()> {
    for batch in rows.chunks(128) {
        let filters = batch.iter().map(observed).collect::<AppResult<Vec<_>>>()?;
        let result = db
            .collection::<Document>(THREADS)
            .delete_many(doc! {"$or":filters})
            .session(&mut *tx)
            .await?;
        if result.deleted_count != batch.len() as u64 {
            return Err(AppError::Conflict("Chat identity changed".into()));
        }
    }
    Ok(())
}

pub(crate) async fn adopt_personal(
    db: &Database,
    channel: &NyxbotChannel,
    relay: &str,
    gateway: &str,
) -> AppResult<()> {
    // Avoid no-op transactions; mutation authority is read again inside the transaction.
    let rows = db.collection::<Document>(THREADS);
    let source_filter = doc! {"channel_id":&channel.id,"$or":[
        {"partition":relay},{"relay_partition":relay,"kind":"private"}]
    };
    if rows
        .find_one(source_filter)
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(());
    }
    if rows
        .find_one(doc! {"channel_id":&channel.id,"partition":gateway})
        .projection(doc! {"kind":1})
        .await?
        .is_some_and(|row| !matches!(row.get("kind"), None | Some(Bson::Null)))
    {
        return Ok(());
    }
    #[cfg(test)]
    pause::transaction_started();
    let context = (
        db.clone(),
        channel.clone(),
        relay.to_owned(),
        gateway.to_owned(),
    );
    super::transaction_service::run_db_only(db, &context, |tx, (db, channel, relay, gateway)| {
        Box::pin(async move {
            let threads = db.collection::<NyxbotThread>(THREADS);
            let Some(source) = threads
                .find_one(doc! {"channel_id":&channel.id,"$or":[
                    {"partition":relay},{"relay_partition":relay,"kind":"private"}]
                })
                .session(&mut *tx)
                .await?
            else {
                return Ok(());
            };
            let destination = threads
                .find_one(doc! {"channel_id":&channel.id,"partition":gateway})
                .session(&mut *tx)
                .await?;
            if destination.as_ref().is_some_and(|row| row.kind.is_some()) {
                return Ok(());
            }
            #[cfg(test)]
            pause::after_read(
                "personal_adoption",
                &[exact(&source)],
                destination.as_ref().map(exact),
            )
            .await;
            if let Some(destination) = destination
                && threads
                    .delete_one(exact(&destination))
                    .session(&mut *tx)
                    .await?
                    .deleted_count
                    != 1
            {
                return Err(AppError::Conflict("Private chat changed".into()));
            }
            #[cfg(test)]
            pause::after_read("personal_destination_deleted", &[exact(&source)], None).await;
            let changed = threads.update_one(exact(&source), doc! {"$set":{
            "partition":gateway,"relay_partition":relay,"updated_at":bson::DateTime::now()
        }}).session(&mut *tx).await?;
            if changed.matched_count != 1 {
                return Err(AppError::Conflict("Private chat changed".into()));
            }
            for field in ["channel", "reply_channel"] {
                db.collection::<Document>("assistant_conversations").update_many(doc! {
                "user_id":&channel.user_id,format!("{field}.nyxbot_channel_id"):&channel.id,
                format!("{field}.partition"):&source.partition
            },doc! {"$set":{format!("{field}.partition"):gateway}}).session(&mut *tx).await?;
            }
            Ok(())
        })
    })
    .await
}

pub(crate) async fn record(
    db: &Database,
    partition: Document,
    update: Vec<Document>,
    obsolete: Option<Document>,
) -> AppResult<NyxbotThread> {
    let has_obsolete = match &obsolete {
        Some(filter) => db
            .collection::<Document>(THREADS)
            .find_one(filter.clone())
            .projection(doc! {"_id":1})
            .await?
            .is_some(),
        None => false,
    };
    if !has_obsolete {
        return db
            .collection::<NyxbotThread>(THREADS)
            .find_one_and_update(partition, update)
            .upsert(true)
            .return_document(mongodb::options::ReturnDocument::After)
            .await?
            .ok_or_else(|| AppError::Internal("Channel chat unavailable".into()));
    }
    #[cfg(test)]
    pause::transaction_started();
    let context = (db.clone(), partition, update, obsolete);
    super::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, partition, update, obsolete)| {
            Box::pin(async move {
                let rows = db.collection::<Document>(THREADS);
                let destination = rows
                    .find_one(partition.clone())
                    .projection(projection())
                    .session(&mut *tx)
                    .await?;
                if let Some(obsolete) = obsolete {
                    let mut cursor = rows
                        .find(obsolete.clone())
                        .projection(projection())
                        .batch_size(128)
                        .session(&mut *tx)
                        .await?;
                    loop {
                        let mut stale = Vec::with_capacity(128);
                        for _ in 0..128 {
                            let Some(row) = cursor.next(&mut *tx).await.transpose()? else {
                                break;
                            };
                            stale.push(row);
                        }
                        if stale.is_empty() {
                            break;
                        }
                        #[cfg(test)]
                        pause::after_read(
                            "group_rediscovery",
                            &stale.iter().map(observed).collect::<AppResult<Vec<_>>>()?,
                            destination.as_ref().map(observed).transpose()?,
                        )
                        .await;
                        delete_observed(db, tx, &stale).await?;
                    }
                }
                let filter = destination
                    .as_ref()
                    .map(observed)
                    .transpose()?
                    .unwrap_or_else(|| partition.clone());
                db.collection::<NyxbotThread>(THREADS)
                    .find_one_and_update(filter, update.clone())
                    .upsert(destination.is_none())
                    .return_document(mongodb::options::ReturnDocument::After)
                    .session(tx)
                    .await?
                    .ok_or_else(|| AppError::Conflict("Chat identity changed".into()))
            })
        },
    )
    .await
}

#[cfg(test)]
pub(crate) mod pause {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Clone)]
    pub(crate) struct Hook {
        pub point: &'static str,
        pub reached: Arc<tokio::sync::Notify>,
        pub resume: Arc<tokio::sync::Notify>,
        pub observations: Arc<tokio::sync::Mutex<Vec<Document>>>,
        once: Arc<AtomicBool>,
    }
    impl Hook {
        pub(crate) fn new(point: &'static str) -> Self {
            Self {
                point,
                reached: Default::default(),
                resume: Default::default(),
                observations: Default::default(),
                once: Default::default(),
            }
        }
    }
    tokio::task_local! {pub(crate) static HOOK: Hook;}
    tokio::task_local! {pub(crate) static TRANSACTIONS: Arc<std::sync::atomic::AtomicUsize>;}
    pub(crate) fn transaction_started() {
        let _ = TRANSACTIONS.try_with(|count| count.fetch_add(1, Ordering::SeqCst));
    }
    pub(crate) async fn after_read(point: &str, rows: &[Document], destination: Option<Document>) {
        let Ok(hook) = HOOK.try_with(Clone::clone) else {
            return;
        };
        if hook.point != point || hook.once.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut observations = hook.observations.lock().await;
        observations.extend_from_slice(rows);
        observations.extend(destination);
        drop(observations);
        hook.reached.notify_one();
        hook.resume.notified().await;
    }
}
