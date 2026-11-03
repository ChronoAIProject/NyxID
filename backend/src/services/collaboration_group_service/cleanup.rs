//! Retire lost authority immediately; remove hidden artifacts only after the
//! provider worker has settled. No public transcript is removed on departure.
use super::*;
use crate::{
    models::assistant_conversation::AssistantConversation,
    services::api_key_mutation_service as transactions,
};

pub async fn sweep(db: &Database) -> AppResult<()> {
    let threads: Vec<AssistantConversation> = db
        .collection(crate::models::assistant_conversation::COLLECTION_NAME)
        .find(doc! {"collaboration.version":1})
        .sort(doc! {"collaboration_checked_at":1})
        .limit(64)
        .await?
        .try_collect()
        .await?;
    for thread in threads {
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .and_run((db.clone(), thread), |session, context| {
                let (db, thread) = &*context;
                Box::pin(async move {
                    let result: AppResult<()> = async {
                        let row = db
                            .collection::<AssistantConversation>(
                                crate::models::assistant_conversation::COLLECTION_NAME,
                            )
                            .find_one(doc! {"_id":&thread.id})
                            .session(&mut *session)
                            .await?;
                        let Some(row) = row else {
                            return Ok(());
                        };
                        match runtime::access_in_session(db, &row, session).await {
                            Ok(_) => {}
                            Err(error)
                                if error.is_forbidden()
                                    || matches!(
                                        error,
                                        AppError::NotFound(_) | AppError::Conflict(_)
                                    ) =>
                            {
                                runtime::retire_thread(db, &row.id, session).await?;
                                if row.active_turn.is_none() {
                                    erase_thread(db, &row, session).await?;
                                    return Ok(());
                                }
                            }
                            Err(error) => return Err(error),
                        }
                        db.collection::<bson::Document>(
                            crate::models::assistant_conversation::COLLECTION_NAME,
                        )
                        .update_one(
                            doc! {"_id":&row.id},
                            doc! {"$set":{"collaboration_checked_at":bson::DateTime::now()}},
                        )
                        .session(session)
                        .await?;
                        Ok(())
                    }
                    .await;
                    transactions::transaction_result(result)
                })
            })
            .await
            .map_err(transactions::map_transaction_error)?;
    }
    db.collection::<model::ExecutionSlot>(model::SLOTS)
        .delete_many(doc! {"revoked":true,"conversation_id":null})
        .await?;
    // Erasure may already have removed the hidden row. Clear only the exact
    // orphan pointer; a concurrently replaced slot cannot be deleted here.
    let orphans:Vec<bson::Document>=db.collection::<model::ExecutionSlot>(model::SLOTS).aggregate(vec![
        doc! {"$match":{"conversation_id":{"$type":"string"}}},
        doc! {"$lookup":{"from":crate::models::assistant_conversation::COLLECTION_NAME,"localField":"conversation_id","foreignField":"_id","as":"thread"}},
        doc! {"$match":{"thread.0":{"$exists":false}}},doc! {"$limit":64},doc! {"$project":{"_id":1,"conversation_id":1}}
    ]).await?.try_collect().await?;
    for orphan in orphans {
        db.collection::<model::ExecutionSlot>(model::SLOTS).delete_one(doc! {"_id":orphan.get_str("_id").map_err(|_|missing())?,"conversation_id":orphan.get_str("conversation_id").map_err(|_|missing())?}).await?;
    }
    Ok(())
}
async fn erase_thread(
    db: &Database,
    row: &AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<()> {
    // Keys are already inactive. Delete encrypted material and every artifact
    // indexed by the exact hidden thread, independently of agent ownership.
    use crate::models as m;
    match super::super::key_service::delete_api_key_in_session(
        db,
        &row.user_id,
        &row.credential_api_key_id,
        None,
        Some(&mut *session),
    )
    .await
    {
        Ok(_) | Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    for collection in [
        m::assistant_agent_credential::COLLECTION_NAME,
        m::assistant_acknowledgement::COLLECTION_NAME,
        m::assistant_sharing::CONSENTS_COLLECTION_NAME,
        m::assistant_message::COLLECTION_NAME,
        m::assistant_attachment::COLLECTION_NAME,
        m::assistant_resource::COLLECTION_NAME,
        m::async_service_operation::COLLECTION_NAME,
        super::super::assistant_service_execution::COLLECTION,
        m::nyxbot_image_delivery::COLLECTION_NAME,
        m::nyxbot_image_delivery::LEGACY_COLLECTION_NAME,
        m::assistant_message_feedback::COLLECTION_NAME,
    ] {
        db.collection::<bson::Document>(collection)
            .delete_many(doc! {"conversation_id":&row.id,"collaboration_group_id":null})
            .session(&mut *session)
            .await?;
    }
    db.collection::<bson::Document>(m::assistant_checkpoint::COLLECTION_NAME)
        .delete_many(doc! {"_id":&row.id})
        .session(&mut *session)
        .await?;
    db.collection::<model::ExecutionSlot>(model::SLOTS)
        .update_many(
            doc! {"conversation_id":&row.id},
            doc! {"$set":{"revoked":true,"conversation_id":null}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<AssistantConversation>(m::assistant_conversation::COLLECTION_NAME)
        .delete_one(doc! {"_id":&row.id,"active_turn":null})
        .session(session)
        .await?;
    Ok(())
}

/// Personal erasure removes creator-owned groups and participation before the
/// ordinary owner inventory can destroy their parent joins. One group per pass.
pub(crate) async fn erase_person(db: &Database, owner: &str) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot()).and_run((db.clone(), owner.to_owned()), |session, context| {let (db, owner)=&*context;let owner=owner.as_str();Box::pin(async move {
        let result:AppResult<bool>=async {
            let Some(mut group)=db.collection::<AssistantGroup>(GROUPS).find_one(doc! {"participant_user_ids":owner,"collaboration.version":1}).session(&mut *session).await? else {return Ok(false);};
            fence_group(db,&group,session).await?;
            if group.user_id==owner {membership::delete(db,&group,session).await?;} else {
                let previous=marker(&group)?.membership_revision;
                group.participant_user_ids.retain(|id|id!=owner);
                invalidate(db,&group.id,session).await?;
                if db.collection::<AssistantGroup>(GROUPS).update_one(doc! {"_id":&group.id,"collaboration.membership_revision":previous},
                    doc! {"$set":{"participant_user_ids":group.participant_user_ids},"$unset":{format!("last_recipient_ids_by_actor.{owner}"):""},"$inc":{"collaboration.membership_revision":1}}).session(&mut *session).await?.matched_count!=1 {return Err(stale());}
                db.collection::<bson::Document>(model::TRANSFERS).delete_many(doc! {"_id":&group.id}).session(&mut *session).await?;
            }
            Ok(true)
        }.await;
        transactions::transaction_result(result)
    })}).await.map_err(transactions::map_transaction_error)
}
