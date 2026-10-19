//! Durable expiry notices are transcript rows, independent of channel source TTL.
use super::super::api_key_mutation_service as transactions;
use super::*;
use crate::models::assistant_conversation::AgentEvent;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{ClientSession, options::ReturnDocument};

pub enum NoticeReason {
    Expired,
    Revoked,
}

pub async fn notice_in_session(
    db: &Database,
    session: &mut ClientSession,
    user: &str,
    conversation: &str,
    event: &AgentEvent,
    reason: NoticeReason,
) -> AppResult<()> {
    let key =
        doc! {"user_id":user,"conversation_id":conversation,"media_queue_notice_id":&event.id};
    let messages = db.collection::<bson::Document>(MESSAGES);
    if messages
        .find_one(key.clone())
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let parent = db
        .collection::<bson::Document>(CONVERSATIONS)
        .find_one_and_update(
            doc! {"_id":conversation,"user_id":user},
            doc! {"$inc":{"message_count":1}},
        )
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    let seq = parent
        .get_i64("message_count")
        .or_else(|_| parent.get_i32("message_count").map(i64::from))
        .unwrap_or(1);
    let sent = event
        .media_sent_at
        .unwrap_or(event.created_at)
        .format("%H:%M");
    let quoted: String = event.text.chars().take(3000).collect();
    let reason = match reason {
        NoticeReason::Expired => "expired before it could be processed",
        NoticeReason::Revoked => "could no longer be processed",
    };
    let text = format!(
        "a photo sent at {sent} UTC {reason}; please resend\n\nQuoted history (not a new request):\n> {}",
        quoted.replace('\n', "\n> ")
    );
    messages.update_one(key,doc! {"$setOnInsert":{
        "_id":uuid::Uuid::new_v4().to_string(),"user_id":user,"conversation_id":conversation,
        "seq":seq,"turn_id":&event.id,"role":"assistant","text":text,"status":"completed",
        "media_queue_notice_id":&event.id,"created_at":bson::DateTime::now(),
    }}).upsert(true).session(&mut *session).await?;
    Ok(())
}

/// Bounded admin/maintenance entry point. Passing `now` explicitly performs an
/// early rollback drain. No feature flag, provider call or owner request runs.
pub async fn convert_queued(db: &Database, before: DateTime<Utc>) -> AppResult<u64> {
    let cutoff = bson::DateTime::from_chrono(before);
    let candidates:Vec<AssistantConversation> = db.collection::<AssistantConversation>(CONVERSATIONS).find(doc! {
        "pending_events":{"$elemMatch":{"media_event_ids.0":{"$exists":true},"media_sent_at":{"$lte":cutoff}}}
    }).sort(doc! {"pending_events.media_sent_at":1,"_id":1}).limit(50).await?.try_collect().await?;
    let mut removed = 0;
    for row in candidates {
        let mut session = db.client().start_session().await?;
        let db = db.clone();
        removed += session.start_transaction().and_run2(async move |session| {
            let db=&db;
            let work: AppResult<u64> = async {
                let Some(current) = db.collection::<AssistantConversation>(CONVERSATIONS).find_one(doc! {"_id":&row.id,"user_id":&row.user_id})
                    .session(&mut *session).await? else {return Ok(0)};
                let mut count = 0;
                for event in current.pending_events.iter().filter(|e|!e.media_event_ids.is_empty() && e.media_sent_at.unwrap_or(e.created_at)<=before).take(50) {
                    notice_in_session(db,session,&row.user_id,&row.id,event,NoticeReason::Expired).await?;
                    // Project the terminal class before removing the queue's
                    // durable evidence, in the same transaction as the notice.
                    let mut receipt = MediaBound {
                        event_ids: event.media_event_ids.clone(), ok: Vec::new(),
                        failed: Vec::new(), context: String::new(),
                    };
                    for source_id in &event.media_event_ids {
                        if let Some(source) = db.collection::<NyxbotEvent>(EVENTS)
                            .find_one(doc! {"_id":source_id,"user_id":&row.user_id,"conversation_id":&row.id})
                            .session(&mut *session).await?
                            && let Some(media) = &source.media
                        {
                            for item in &media.items {
                                receipt.failed.push(outcome(&source,Some(item.index),item.embedded,"not_processed: expired",None));
                            }
                        }
                    }
                    binding::reconcile_in_session(db,session,&receipt).await?;
                    count += db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&row.id,"user_id":&row.user_id},
                        doc! {"$pull":{"pending_events":{"id":&event.id,"media_event_ids":&event.media_event_ids}}})
                        .session(&mut *session).await?.modified_count;
                }
                Ok(count)
            }.await;
            transactions::transaction_result(work)
        }).await.map_err(transactions::map_transaction_error)?;
    }
    Ok(removed)
}

pub async fn sweep(db: &Database) -> AppResult<u64> {
    // Select inactive turns before applying the batch limit so old active
    // messages cannot starve orphan cleanup indefinitely.
    let pending:Vec<bson::Document>=db.collection::<bson::Document>(MESSAGES).aggregate(vec![
        doc! {"$match":{"media_pending":{"$ne":bson::Bson::Null}}},
        doc! {"$lookup":{"from":CONVERSATIONS,"localField":"conversation_id","foreignField":"_id","as":"parent"}},
        doc! {"$match":{"$expr":{"$not":{"$in":["$turn_id","$parent.active_turn.turn_id"]}}}},
        doc! {"$sort":{"created_at":1,"_id":1}},doc! {"$limit":50},doc! {"$unset":"parent"}
    ]).await?.try_collect().await?;
    let mut finalized = 0;
    for document in pending {
        let message: AssistantMessage = bson::from_document(document)
            .map_err(|_| AppError::Internal("Invalid media message".into()))?;
        finalized += u64::from(
            media_finalize(
                db,
                &message.user_id,
                &message.conversation_id,
                &message.turn_id,
            )
            .await?,
        );
    }
    Ok(finalized + convert_queued(db, Utc::now() - chrono::Duration::days(6)).await?)
}
