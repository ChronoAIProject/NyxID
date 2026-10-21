use super::super::{api_key_mutation_service as transactions, assistant_upload_retention};
use super::*;
use crate::models::assistant_upload::{AssistantUpload, COLLECTION_NAME as UPLOADS};
use futures::TryStreamExt;
use mongodb::ClientSession;

pub(super) async fn reconcile_in_session(
    db: &Database,
    session: &mut ClientSession,
    receipt: &MediaBound,
) -> AppResult<()> {
    for item in receipt.ok.iter().chain(&receipt.failed) {
        let Some(index) = item.index else { continue };
        db.collection::<bson::Document>(EVENTS).update_one(
            doc! {"_id":&item.event_id,"media.key":&item.event_key},
            doc! {"$set":{"media.items.$[item].outcome":bson::to_bson(item).map_err(|_|AppError::AssistantTurnRequired)?}},
        ).array_filters(vec![doc! {"item.index":index as i64,"item.embedded":item.embedded}])
            .session(&mut *session).await?;
    }
    Ok(())
}

pub(super) async fn reconcile(db: &Database, receipt: &MediaBound) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let receipt = receipt.clone();
    session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(reconcile_in_session(&db, session, &receipt).await)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// Receipt-first replay is valid even after settlement or a successor turn.
pub(super) async fn bind(
    db: &Database,
    message: &AssistantMessage,
    proposed: &MediaBound,
) -> AppResult<MediaBound> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let message = message.clone();
    let proposed = proposed.clone();
    session.start_transaction().and_run2(async move |session| {
        let db = &db;
        let message = &message;
        let proposed = &proposed;
        let work: AppResult<_> = async {
            let current = db.collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"_id":&message.id,"user_id":&message.user_id,"turn_id":&message.turn_id})
                .session(&mut *session).await?.ok_or(AppError::AssistantTurnRequired)?;
            if current.media_event_ids != proposed.event_ids { return Err(AppError::AssistantTurnRequired); }
            if let Some(receipt) = current.media_bound { return Ok(receipt); }
            let fence = db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {
                "_id":&message.conversation_id,"user_id":&message.user_id,
                "active_turn.turn_id":&message.turn_id,"active_turn.stop_requested":{"$ne":true}
            },doc! {"$inc":{"upload_generation":1}}).session(&mut *session).await?;
            if fence.matched_count != 1 || current.media_pending.is_none() { return Err(AppError::AssistantTurnRequired); }
            let mut attachments = current.attachments;
            let mut ids = Vec::new();
            for item in &proposed.ok {
                let id = item.upload_id.as_deref().ok_or(AppError::AssistantTurnRequired)?;
                let root = db.collection::<AssistantUpload>(UPLOADS).find_one_and_update(doc! {
                    "_id":id,"user_id":&message.user_id,"conversation_id":&message.conversation_id,
                    "message_id":bson::Bson::Null,"source.event_key":&item.event_key,
                    "source.index":item.index.map(|i|i as i64),"source.embedded":item.embedded,
                    "source.turn_id":&message.turn_id
                },doc! {"$set":{"message_id":&message.id,"bound_at":bson::DateTime::now()},"$unset":{"expires_at":""}})
                    .session(&mut *session).await?.ok_or(AppError::AssistantTurnRequired)?;
                if !attachments.iter().any(|a|a.id==id) { attachments.push(uploads::metadata(&root)); }
                ids.push(id.to_owned());
            }
            Box::pin(assistant_upload_retention::used_in_turn(db,session,&message.user_id,&message.conversation_id,&message.turn_id,&ids)).await?;
            let text = if proposed.context.is_empty() {current.text} else {format!("{}\n\n{}",current.text,proposed.context)};
            db.collection::<bson::Document>(MESSAGES).update_one(doc! {"_id":&message.id,"media_pending":{"$ne":bson::Bson::Null}},doc! {
                "$set":{"attachments":bson::to_bson(&attachments).map_err(|_|AppError::AssistantTurnRequired)?,
                    "media_bound":bson::to_bson(proposed).map_err(|_|AppError::AssistantTurnRequired)?,"text":text},
                "$unset":{"media_pending":""}
            }).session(&mut *session).await?;
            #[cfg(test)]
            super::test_hooks::reach(db,&message.turn_id,"bind_before_commit").await;
            Ok(proposed.clone())
        }.await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

/// Called by settlement before it clears the same conversation fence. Kept
/// outside the run future so Stop cannot cancel its cleanup.
pub(crate) fn finalize_in_session<'a>(
    db: &'a Database,
    session: &'a mut ClientSession,
    user: &'a str,
    conversation: &'a str,
    turn: &'a str,
    reason: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let Some(message) = db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {
                "user_id":user,"conversation_id":conversation,"turn_id":turn,
                "role":{"$ne":"assistant"},"media_event_ids.0":{"$exists":true}
            })
            .session(&mut *session)
            .await?
        else {
            return Ok(());
        };
        let receipt = if let Some(receipt) = message.media_bound {
            receipt
        } else {
            let receipt = pending_receipt(
                db,
                &message.media_event_ids,
                &format!("not_processed: {reason}"),
            )
            .await?;
            db.collection::<bson::Document>(MESSAGES).update_one(doc! {"_id":&message.id,"media_bound":bson::Bson::Null},doc! {
            "$set":{"media_bound":bson::to_bson(&receipt).map_err(|_|AppError::AssistantTurnRequired)?,
                "text":format!("{}\n\n{}",message.text,receipt.context)},"$unset":{"media_pending":""}
        }).session(&mut *session).await?;
            receipt
        };
        reconcile_in_session(db, session, &receipt).await?;
        // A late unknown upload commit may arrive later; it remains unbound and
        // is never adopted on replay. Pending retention handles that later row.
        let mut cursor = db.collection::<bson::Document>(UPLOADS).find(doc! {
        "user_id":user,"conversation_id":conversation,"source.turn_id":turn,"message_id":bson::Bson::Null,
    }).projection(doc! {"_id":1}).session(&mut *session).await?;
        let roots: Vec<bson::Document> = cursor.stream(&mut *session).try_collect().await?;
        let ids: Vec<_> = roots.iter().filter_map(|r| r.get_str("_id").ok()).collect();
        if !ids.is_empty() {
            db.collection::<bson::Document>(UPLOADS)
                .delete_many(doc! {"user_id":user,
                    "$or":[{"_id":{"$in":&ids}},{"parent_attachment_id":{"$in":&ids}}]
                })
                .session(&mut *session)
                .await?;
        }
        Ok(())
    })
}

/// Orphan-message maintenance only wins when the original turn is no longer
/// active. The parent write conflicts with binding and successor admission.
pub(crate) async fn media_finalize(
    db: &Database,
    user: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let user = user.to_owned();
    let conversation = conversation.to_owned();
    let turn = turn.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &db;
            let user = user.as_str();
            let conversation = conversation.as_str();
            let turn = turn.as_str();
            let work: AppResult<_> = async {
                let fence = db
                    .collection::<bson::Document>(CONVERSATIONS)
                    .update_one(
                        doc! {
                            "_id":conversation,"user_id":user,"active_turn.turn_id":{"$ne":turn}
                        },
                        doc! {"$inc":{"upload_generation":1}},
                    )
                    .session(&mut *session)
                    .await?;
                if fence.matched_count == 0 {
                    return Ok(false);
                }
                finalize_in_session(db, session, user, conversation, turn, "failed").await?;
                assistant_upload_retention::settled_in_session(
                    db,
                    session,
                    user,
                    conversation,
                    turn,
                )
                .await?;
                Ok(true)
            }
            .await;
            transactions::transaction_result(work)
        })
        .await
        .map_err(transactions::map_transaction_error)
}
