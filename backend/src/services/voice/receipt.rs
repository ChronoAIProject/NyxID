//! Deterministic call-end snapshot on the ordinary, transactionally sequenced timeline.
use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        assistant_voice::REQUESTS,
        assistant_voice_session::{COLLECTION_NAME as CALLS, VoiceSession},
    },
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};

/// Called in the same transaction that releases the live slot. Recovery cannot
/// overwrite this snapshot or allocate a second timeline sequence.
pub async fn append(db: &Database, tx: &mut ClientSession, call: &VoiceSession) -> AppResult<()> {
    let Some(thread) = db
        .collection::<AssistantConversation>(THREADS)
        .find_one_and_update(
            doc! {"_id":&call.conversation_id,"user_id":&call.user_id},
            doc! {"$inc":{"message_count":1},"$set":{"updated_at":bson::DateTime::now()}},
        )
        .return_document(ReturnDocument::After)
        .session(&mut *tx)
        .await?
    else {
        return Ok(());
    };
    let mut cursor = db.collection::<Document>(REQUESTS).aggregate(vec![
        doc! {"$match":{"session_id":&call.id,"user_id":&call.user_id,"conversation_id":&call.conversation_id}},
        doc! {"$group":{"_id":{"$ifNull":["$root_request_id","$_id"]},
            "states":{"$addToSet":"$state"},"started":{"$max":{"$cond":[{"$ne":[{"$ifNull":["$started_at",null]},null]},1,0]}}}},
        doc! {"$group":{"_id":null,"started":{"$sum":"$started"},
            "completed":{"$sum":{"$cond":[{"$setEquals":["$states",["completed"]]},1,0]}},
            "queued":{"$sum":{"$cond":[{"$in":["queued","$states"]},1,0]}},
            "cancelled":{"$sum":{"$cond":[{"$in":["cancelled","$states"]},1,0]}}}},
    ]).session(&mut *tx).await?;
    let counts = cursor
        .stream(&mut *tx)
        .try_next()
        .await?
        .unwrap_or_default();
    let mut cards = db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .aggregate(vec![
            doc! {"$match":{"user_id":&call.user_id,"conversation_id":&call.conversation_id,
                "status":{"$in":["allowed","denied","used"]},"decided_at":{"$lte":bson::DateTime::from_chrono(call.closed_at.unwrap_or_else(Utc::now))}}},
            doc! {"$lookup":{"from":REQUESTS,"localField":"voice_request_id","foreignField":"_id","as":"request"}},
            doc! {"$match":{"request.session_id":&call.id}},doc! {"$count":"count"},
        ]).session(&mut *tx).await?;
    let confirmations = cards
        .stream(&mut *tx)
        .try_next()
        .await?
        .map_or(0, |d| integer(&d, "count"));
    let mut results = db.collection::<Document>(REQUESTS).aggregate(vec![
        doc! {"$match":{"session_id":&call.id,"user_id":&call.user_id,"state":{"$in":["completed","cancelled"]}}},
        doc! {"$sort":{"message_seq":-1}},doc! {"$limit":8},
        doc! {"$lookup":{"from":MESSAGES,"localField":"turn_id","foreignField":"turn_id","as":"reply"}},
        doc! {"$unwind":"$reply"},
        doc! {"$match":{"reply.role":"assistant","reply.execution_pending":{"$ne":true},
            "reply.user_id":&call.user_id,"reply.conversation_id":&call.conversation_id}},
        doc! {"$project":{"_id":"$reply._id","seq":"$reply.seq"}},doc! {"$sort":{"seq":1}},doc! {"$limit":8},
    ]).session(&mut *tx).await?;
    let results: Vec<Document> = results.stream(&mut *tx).try_collect().await?;
    let seconds = (call.closed_at.unwrap_or_else(Utc::now) - call.created_at)
        .num_seconds()
        .clamp(0, 1800);
    let text = render(seconds, &counts, confirmations, &results);
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<AssistantMessage>(MESSAGES)
        .insert_one(AssistantMessage {
            id: id.clone(),
            conversation_id: call.conversation_id.clone(),
            user_id: call.user_id.clone(),
            seq: thread.message_count,
            turn_id: call.id.clone(),
            role: "assistant".into(),
            text,
            status: "completed".into(),
            error_code: None,
            created_at: call.closed_at.unwrap_or_else(Utc::now),
            activities: vec![],
            attachments: vec![],
            origin: None,
            via: Some("voice".into()),
            voice: None,
            execution_pending: true,
        })
        .session(&mut *tx)
        .await?;
    db.collection::<Document>(CALLS)
        .update_one(
            doc! {"_id":&call.id},
            doc! {"$set":{"receipt_message_id":id}},
        )
        .session(&mut *tx)
        .await?;
    Ok(())
}
fn integer(d: &Document, key: &str) -> i64 {
    d.get_i64(key)
        .or_else(|_| d.get_i32(key).map(i64::from))
        .unwrap_or(0)
}
fn render(seconds: i64, counts: &Document, confirmations: i64, results: &[Document]) -> String {
    let mut text = format!(
        "**Call receipt** · {}:{:02}\n\nRequests: {} started · {} completed · {} queued · {} cancelled. Confirmations decided: {}.",
        seconds / 60,
        seconds % 60,
        integer(counts, "started"),
        integer(counts, "completed"),
        integer(counts, "queued"),
        integer(counts, "cancelled"),
        confirmations
    );
    let links: Vec<_> = results
        .iter()
        .filter_map(|r| {
            Some(format!(
                "[Result {}](#message-{})",
                integer(r, "seq"),
                uuid::Uuid::parse_str(r.get_str("_id").ok()?).ok()?
            ))
        })
        .collect();
    if !links.is_empty() {
        text.push_str("\n\n");
        text.push_str(&links.join(" · "));
    }
    text
}
