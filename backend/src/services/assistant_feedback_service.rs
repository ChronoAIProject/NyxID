use crate::models::{
    assistant_group::{GroupMessage, MESSAGES_COLLECTION_NAME},
    assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGE_COLLECTION},
    assistant_message_feedback::{AssistantMessageFeedback, COLLECTION_NAME},
};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AdminCount {
    pub day: String,
    pub up: i64,
    pub down: i64,
}
pub struct AdminRow {
    pub message_id: String,
    pub vote: String,
    pub comment: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub id: String,
}
pub async fn admin_recent(
    db: &Database,
    filter: mongodb::bson::Document,
    limit: i64,
) -> mongodb::error::Result<Vec<AdminRow>> {
    let rows = db
        .collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"created_at": -1, "_id": -1})
        .limit(limit)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| AdminRow {
            message_id: r.message_id,
            vote: r.vote,
            comment: r.comment,
            created_at: r.created_at,
            updated_at: r.updated_at,
            id: r.id,
        })
        .collect())
}

pub async fn admin_counts(
    db: &Database,
    filter: mongodb::bson::Document,
) -> mongodb::error::Result<Vec<AdminCount>> {
    let pipeline = vec![
        doc! {"$match": filter},
        doc! {"$group": {"_id": {"day": {"$dateToString": {"format": "%Y-%m-%d", "date": "$created_at"}}, "vote": "$vote"}, "count": {"$sum": 1}}},
    ];
    let mut cursor = db
        .collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .aggregate(pipeline)
        .await?;
    let mut out = std::collections::BTreeMap::<String, AdminCount>::new();
    while let Some(row) = cursor.try_next().await? {
        let Ok(id) = row.get_document("_id") else {
            continue;
        };
        let Ok(day_value) = id.get_str("day") else {
            continue;
        };
        let day: String = day_value.to_owned();
        let Ok(vote) = id.get_str("vote") else {
            continue;
        };
        let count: i64 = match row.get("count") {
            Some(bson::Bson::Int32(v)) => i64::from(*v),
            Some(bson::Bson::Int64(v)) => *v,
            _ => continue,
        };
        let entry = out.entry(day.clone()).or_insert(AdminCount {
            day,
            up: 0,
            down: 0,
        });
        if vote == "up" {
            entry.up += count;
        } else {
            entry.down += count;
        }
    }
    Ok(out.into_values().collect())
}

/// Single-row lookup keeps eligibility bounded for long-lived conversations.
pub async fn conversation_message(
    db: &Database,
    user_id: &str,
    conversation_id: &str,
    message_id: &str,
) -> mongodb::error::Result<Option<AssistantMessage>> {
    db.collection::<AssistantMessage>(MESSAGE_COLLECTION)
        .find_one(doc! {"_id": message_id, "conversation_id": conversation_id, "user_id": user_id})
        .await
}

pub async fn group_message(
    db: &Database,
    group_id: &str,
    message_id: &str,
) -> mongodb::error::Result<Option<GroupMessage>> {
    db.collection::<GroupMessage>(MESSAGES_COLLECTION_NAME)
        .find_one(doc! {"_id": message_id, "group_id": group_id})
        .await
}

pub async fn upsert_feedback(
    db: &Database,
    user_id: &str,
    message_id: &str,
    conversation_id: Option<String>,
    group_id: Option<String>,
    turn_id: Option<String>,
    vote: &str,
    comment: Option<String>,
) -> mongodb::error::Result<(AssistantMessageFeedback, Option<AssistantMessageFeedback>)> {
    let filter = doc! {"user_id": user_id, "message_id": message_id};
    let existing = db
        .collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .find_one(filter.clone())
        .await?;
    let now = chrono::Utc::now();
    let mut set = doc! {"vote": vote, "updated_at": bson::DateTime::from_chrono(now), "conversation_id": conversation_id, "group_id": group_id, "turn_id": turn_id};
    if comment.is_some() {
        set.insert("comment", comment);
    }
    let update = doc! {"$set": set, "$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "created_at": bson::DateTime::from_chrono(now)}};
    let row = db
        .collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .find_one_and_update(filter, update)
        .upsert(true)
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .ok_or_else(|| mongodb::error::Error::custom("feedback write failed"))?;
    Ok((row, existing))
}

pub async fn delete_feedback(
    db: &Database,
    user_id: &str,
    message_id: &str,
) -> mongodb::error::Result<bool> {
    Ok(db
        .collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .delete_one(doc! {"user_id": user_id, "message_id": message_id})
        .await?
        .deleted_count
        > 0)
}
