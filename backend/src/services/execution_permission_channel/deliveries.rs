//! Result occurrences do not reopen the original stream/terminal-delivery claim.
use super::*;
use crate::models::{
    assistant_conversation::ActiveTurn,
    assistant_message::AssistantMessage,
    execution_permission_channel::{COLLECTION_NAME as DELIVERIES, Delivery},
};
use futures::TryStreamExt;

pub(crate) fn owns_result(row: &AssistantConversation, turn: &ActiveTurn) -> bool {
    turn.execution_task
        .as_ref()
        .and_then(|t| t.channel_source.as_ref())
        .is_some_and(|s| s.original_turn_id != turn.turn_id && s.conversation_id == row.id)
}

pub(crate) async fn enqueue(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
    turn: &ActiveTurn,
    message: &AssistantMessage,
) -> AppResult<()> {
    if !owns_result(row, turn)
        || turn.report_to.is_some()
        || (message.text.trim().is_empty() && message.attachments.is_empty())
    {
        return Ok(());
    }
    let task = turn.execution_task.as_ref().ok_or_else(refused)?;
    let source = task.channel_source.as_ref().ok_or_else(refused)?;
    let occurrence_key = fingerprint(
        b"execution-channel-result-v4",
        &(
            source.event_id.as_str(),
            &task.task,
            task.epoch,
            turn.turn_id.as_str(),
        ),
    )?;
    let delivery = Delivery {
        id: uuid::Uuid::new_v4().to_string(),
        occurrence_key: occurrence_key.clone(),
        version: 4,
        user_id: row.user_id.clone(),
        conversation_id: row.id.clone(),
        message_id: message.id.clone(),
        turn_id: turn.turn_id.clone(),
        task: task.clone(),
        state: "pending".into(),
        claim_id: None,
        lease_expires_at: None,
        created_at: Utc::now(),
        expires_at: source.expires_at.min(task.hard_deadline),
    };
    db.collection::<Document>(DELIVERIES)
        .update_one(
            doc! {"occurrence_key":occurrence_key},
            doc! {"$setOnInsert":bson::to_document(&delivery).map_err(encoding)?},
        )
        .upsert(true)
        .session(tx)
        .await?;
    Ok(())
}

pub(crate) async fn lease(db: &Database, id: &str) -> AppResult<Option<Delivery>> {
    let claim = uuid::Uuid::new_v4().to_string();
    Ok(db.collection::<Delivery>(DELIVERIES).find_one_and_update(doc!{
        "_id":id,"version":4,"expires_at":{"$gt":bson::DateTime::now()},"$or":[
            {"state":"pending"},{"state":"preparing","lease_expires_at":{"$lte":bson::DateTime::now()}}
        ]
    },doc!{"$set":{"state":"preparing","claim_id":claim,"lease_expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::seconds(60))}})
    .return_document(mongodb::options::ReturnDocument::After).await?)
}

async fn receipt_authority(
    db: &Database,
    tx: &mut ClientSession,
    d: &Delivery,
) -> AppResult<SourceRows> {
    let t = &d.task;
    let source = t.channel_source.as_ref().ok_or_else(refused)?;
    if d.version != 4
        || t.cancelled
        || t.hard_deadline <= Utc::now()
        || d.expires_at <= Utc::now()
        || d.user_id != t.actor_user_id
        || t.audience != audience(source)
        || source.conversation_id != d.conversation_id
    {
        return Err(refused());
    }
    if !super::super::execution_permission_enrollment::live_enabled(db, &d.user_id, Some(tx))
        .await?
    {
        return Err(refused());
    }
    // Completion is allowed; cancellation or epoch replacement is not. Do not
    // reopen the root or require a new model turn merely to release its receipt.
    let _:Document=load(db,tx,super::super::execution_permission_dispatch::tasks::collection(&t.task),doc!{
        "_id":&t.task.root_id,"execution_task.version":t.version as i32,"execution_task.epoch":t.epoch,
        "execution_task.actor_user_id":&d.user_id,"execution_task.cancelled":false,
        "execution_task.task":bson::to_bson(&t.task).map_err(encoding)?,
        "execution_task.hard_deadline":bson::DateTime::from_chrono(t.hard_deadline),
        "execution_task.channel_source":bson::to_bson(source).map_err(encoding)?,
        "execution_task.audience":bson::to_bson(&t.audience).map_err(encoding)?
    },true).await?;
    validate(db, tx, source, true).await
}

/// No network call in this retryable transaction. Crossing dispatching is final:
/// no worker can reclaim it, even if the process dies before receiving a receipt.
pub(crate) async fn claim(db: &Database, d: &Delivery) -> AppResult<bool> {
    let context = (db.clone(), d.clone());
    super::super::transaction_service::run_db_only(db,&context,|tx,(db,d)| Box::pin(async move {
        receipt_authority(db,tx,d).await?;
        Ok(db.collection::<Document>(DELIVERIES).update_one(doc!{
            "_id":&d.id,"version":4,"state":"preparing","claim_id":&d.claim_id,
            "lease_expires_at":{"$gt":bson::DateTime::now()},"expires_at":{"$gt":bson::DateTime::now()}
        },doc!{"$set":{"state":"dispatching"}}).session(tx).await?.matched_count==1)
    })).await
}

pub(crate) async fn finish(db: &Database, d: &Delivery, state: &str) -> AppResult<()> {
    if !matches!(state, "confirmed" | "rejected" | "unknown" | "refused") {
        return Err(encoding("state"));
    }
    let expected = if state == "refused" {
        "preparing"
    } else {
        "dispatching"
    };
    db.collection::<Document>(DELIVERIES)
        .update_one(
            doc! {"_id":&d.id,"claim_id":&d.claim_id,"state":expected},
            doc! {"$set":{"state":state},"$unset":{"lease_expires_at":""}},
        )
        .await?;
    Ok(())
}

pub(crate) async fn due(db: &Database) -> AppResult<Vec<String>> {
    let now = bson::DateTime::now();
    let rows:Vec<Delivery>=db.collection::<Delivery>(DELIVERIES).find(doc!{"version":4,"$or":[
        {"state":"pending"},
        {"state":"preparing","$or":[{"lease_expires_at":{"$lte":now}},{"expires_at":{"$lte":now}}]},
        {"state":"dispatching","lease_expires_at":{"$lte":now}}
    ]}).sort(doc!{"created_at":1,"_id":1}).limit(32).await?.try_collect().await?;
    let mut due = Vec::new();
    for row in rows {
        let terminal = if row.state == "dispatching" {
            Some("unknown")
        } else if row.expires_at <= Utc::now() {
            Some("expired")
        } else {
            None
        };
        if let Some(terminal) = terminal {
            db.collection::<Document>(DELIVERIES).update_one(doc!{"_id":&row.id,"version":4,"state":&row.state,"claim_id":&row.claim_id,"lease_expires_at":row.lease_expires_at.map(bson::DateTime::from_chrono)},doc!{"$set":{"state":terminal},"$unset":{"lease_expires_at":""}}).await?;
        } else {
            due.push(row.id);
        }
    }
    Ok(due)
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<Document>(DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"occurrence_key":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("execution_delivery_occurrence_unique".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"version":1,"state":1,"created_at":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"task.channel_source.channel_id":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"task.channel_source.thread_id":1})
                .build(),
        )
        .await?;
    // Keep terminal uncertainty visible for at most 24h; expiry is enforced above.
    db.collection::<Document>(DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"created_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::from_secs(86400))
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id":1,"conversation_id":1})
                .build(),
        )
        .await?;
    Ok(())
}
