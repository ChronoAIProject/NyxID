//! Result occurrences do not reopen the original stream/terminal-delivery claim.
use super::*;
use crate::models::{
    assistant_conversation::ActiveTurn,
    assistant_message::AssistantMessage,
    execution_permission_channel::{COLLECTION_NAME as DELIVERIES, Delivery, ResultPreparation},
};
use futures::TryStreamExt;

pub(crate) const VERSION: i32 = 5;
pub(crate) const SIDE_MESSAGE_VERSION: u32 = 6;
pub(crate) const PREPARATION_SECS: i64 = 180;
pub(crate) const SEND_SECS: i64 = 120;

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
        version: VERSION as u32,
        user_id: row.user_id.clone(),
        conversation_id: row.id.clone(),
        message_id: message.id.clone(),
        turn_id: turn.turn_id.clone(),
        task: task.clone(),
        state: "pending".into(),
        claim_id: None,
        preparation: None,
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

fn exact(d: &Delivery, state: impl Into<bson::Bson>) -> Document {
    doc! {"_id":&d.id,"version":d.version,"claim_id":&d.claim_id,"state":state.into()}
}
fn live() -> Document {
    doc! {"$and":[
        {"$eq":[{"$type":"$lease_expires_at"},"date"]},
        {"$gt":["$lease_expires_at","$$NOW"]},
        {"$gt":["$expires_at","$$NOW"]}
    ]}
}
fn elapsed(field: &str) -> Document {
    doc! {"$and":[{"$eq":[{"$type":field},"date"]},{"$lte":[field,"$$NOW"]}]}
}
fn deadline(seconds: i64) -> Document {
    doc! {"$min":[{"$add":["$$NOW",seconds*1000]},"$expires_at"]}
}
fn changed() -> AppError {
    AppError::Conflict("Channel result preparation changed".into())
}

pub(crate) async fn lease(db: &Database, id: &str) -> AppResult<Option<Delivery>> {
    // Version 4 uses the legacy dispatch protocol. Version 6 already has a
    // confirmed side message and must stay outside older workers' selectors.
    Ok(db.collection::<Delivery>(DELIVERIES).find_one_and_update(doc!{
        "_id":id,"version":{"$in":[4,VERSION,SIDE_MESSAGE_VERSION]},"expires_at":{"$type":"date"},
        "$expr":{"$gt":["$expires_at","$$NOW"]},"$or":[
            {"state":"pending"},{"state":"preparing","$expr":elapsed("$lease_expires_at")}
        ]
    },vec![doc!{"$set":{"version":{"$max":["$version",VERSION]},"state":"preparing","claim_id":uuid::Uuid::new_v4().to_string(),
        "lease_expires_at":deadline(PREPARATION_SECS)}}])
    .return_document(mongodb::options::ReturnDocument::After).await?)
}

pub(crate) async fn plan(
    db: &Database,
    d: &Delivery,
    fingerprint: &str,
    total: u32,
) -> AppResult<ResultPreparation> {
    if total == 0 || total as usize > super::super::channel_reply_delivery::MAX_REPLY_PARTS {
        return Err(changed());
    }
    let mut filter = exact(d, "preparing");
    filter.insert("$expr", live());
    filter.insert(
        "$or",
        bson::bson!([
            {"preparation":null}, {"preparation.fingerprint":fingerprint,"preparation.total":total}
        ]),
    );
    let prepared = ResultPreparation {
        fingerprint: fingerprint.into(),
        total,
        next: 0,
        receipts: vec![],
    };
    let saved=db.collection::<Delivery>(DELIVERIES).find_one_and_update(filter,
        vec![doc!{"$set":{"preparation":{"$ifNull":["$preparation",{"$literal":bson::to_bson(&prepared).map_err(encoding)?}]}}}])
        .return_document(mongodb::options::ReturnDocument::After).await?.ok_or_else(changed)?;
    let prepared = saved.preparation.ok_or_else(changed)?;
    if prepared.next > prepared.total || prepared.receipts.len() != prepared.next as usize {
        return Err(changed());
    }
    Ok(prepared)
}

async fn receipt_authority(
    db: &Database,
    tx: &mut ClientSession,
    d: &Delivery,
) -> AppResult<SourceRows> {
    let t = &d.task;
    let source = t.channel_source.as_ref().ok_or_else(refused)?;
    if !matches!(d.version, 4 | 5 | SIDE_MESSAGE_VERSION)
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

/// No network call in this retryable transaction. Kept for the legacy claim
/// contract; ordinary result sends use the exact part boundary below.
#[cfg(test)]
pub(crate) async fn claim(db: &Database, d: &Delivery) -> AppResult<bool> {
    dispatch(db, d, None).await
}

async fn dispatch(db: &Database, d: &Delivery, part: Option<u32>) -> AppResult<bool> {
    let context = (db.clone(), d.clone(), part);
    super::super::transaction_service::run_db_only(db,&context,|tx,(db,d,part)|Box::pin(async move {
        receipt_authority(db,tx,d).await?;
        let mut filter=exact(d,if part.is_some(){bson::bson!({"$in":["preparing","dispatching"]})}else{bson::bson!("preparing")});
        filter.insert("$expr",live());
        if let Some(part)=part {
            filter.insert("preparation.next",*part);
        }
        Ok(db.collection::<Document>(DELIVERIES).update_one(filter,vec![doc!{"$set":{
            "state":"dispatching","lease_expires_at":{"$cond":[{"$eq":["$state","preparing"]},deadline(SEND_SECS),"$lease_expires_at"]}
        }}]).session(tx).await?.matched_count==1)
    })).await
}

pub(crate) async fn before_effect(db: &Database, d: &Delivery, part: u32) -> AppResult<()> {
    if !dispatch(db, d, Some(part)).await? {
        return Err(changed());
    }
    #[cfg(test)]
    crate::services::durable_wake_tests::after("execution_result_dispatch_committed").await;
    Ok(())
}

pub(crate) async fn accepted(
    db: &Database,
    d: &mut Delivery,
    part: u32,
    total: u32,
    receipt: &crate::models::channel_turn_preparation::Receipt,
    side_message: bool,
) -> AppResult<bool> {
    let mut filter = exact(d, "dispatching");
    filter.insert("preparation.next", part);
    filter.insert("preparation.total", total);
    let mut changes = doc! {
        "state":if part+1==total {"confirmed"}else{"preparing"},
        "preparation.next":part+1,
        "preparation.receipts":{"$concatArrays":["$preparation.receipts",{"$literal":[bson::to_bson(receipt).map_err(encoding)?]}]},
        "lease_expires_at":if part+1==total {bson::bson!("$$REMOVE")}else{bson::bson!(deadline(PREPARATION_SECS))},
    };
    if side_message {
        changes.insert("version", SIDE_MESSAGE_VERSION);
    }
    let accepted = db
        .collection::<Document>(DELIVERIES)
        .update_one(filter, vec![doc! {"$set":changes}])
        .await?
        .matched_count
        == 1;
    if accepted && side_message {
        d.version = SIDE_MESSAGE_VERSION;
    }
    Ok(accepted)
}

pub(crate) async fn finish_owned(db: &Database, d: &Delivery, state: &str) -> AppResult<bool> {
    if !matches!(state, "confirmed" | "rejected" | "unknown" | "refused") {
        return Err(encoding("state"));
    }
    let expected = if state == "refused" {
        "preparing"
    } else {
        "dispatching"
    };
    Ok(db
        .collection::<Document>(DELIVERIES)
        .update_one(
            exact(d, expected),
            doc! {"$set":{"state":state},"$unset":{"lease_expires_at":""}},
        )
        .await?
        .matched_count
        == 1)
}

#[cfg(test)]
pub(crate) async fn finish(db: &Database, d: &Delivery, state: &str) -> AppResult<()> {
    finish_owned(db, d, state).await.map(|_| ())
}

pub(crate) fn due_pipeline() -> Vec<Document> {
    vec![
        doc! {"$match":{"version":{"$in":[4,VERSION,SIDE_MESSAGE_VERSION]},"$or":[
            {"state":"pending"},
            {"state":"preparing","$expr":{"$or":[elapsed("$lease_expires_at"),elapsed("$expires_at")]}},
            {"state":"dispatching","$expr":elapsed("$lease_expires_at")}
        ]}},
        doc! {"$sort":{"created_at":1,"_id":1}},
        doc! {"$limit":32},
        doc! {"$set":{"expired":elapsed("$expires_at")}},
    ]
}

pub(crate) async fn due(db: &Database) -> AppResult<Vec<String>> {
    // One indexed read on idle passes. Computed expiry uses the same database
    // clock as acquisition and the provider fence, never the replica's clock.
    let rows: Vec<Document> = db
        .collection::<Document>(DELIVERIES)
        .aggregate(due_pipeline())
        .await?
        .try_collect()
        .await?;
    let mut due = Vec::new();
    for raw in rows {
        let expired = raw.get_bool("expired").unwrap_or(false);
        let row: Delivery = bson::from_document(raw).map_err(encoding)?;
        let terminal = if row.state == "dispatching" {
            Some("unknown")
        } else if expired {
            Some("expired")
        } else {
            None
        };
        if let Some(terminal) = terminal {
            let mut filter = exact(&row, row.state.as_str());
            filter.insert(
                "lease_expires_at",
                row.lease_expires_at.map(bson::DateTime::from_chrono),
            );
            filter.insert(
                "$expr",
                elapsed(if terminal == "unknown" {
                    "$lease_expires_at"
                } else {
                    "$expires_at"
                }),
            );
            db.collection::<Document>(DELIVERIES)
                .update_one(
                    filter,
                    doc! {"$set":{"state":terminal},"$unset":{"lease_expires_at":""}},
                )
                .await?;
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
