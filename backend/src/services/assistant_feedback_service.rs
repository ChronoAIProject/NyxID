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
    pub context: Option<crate::models::assistant_message_feedback::FeedbackReplyContext>,
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
            context: r.context,
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

pub struct FeedbackWrite {
    pub user_id: String,
    pub message_id: String,
    pub conversation_id: Option<String>,
    pub group_id: Option<String>,
    pub turn_id: Option<String>,
    pub vote: String,
    pub comment: Option<String>,
    pub comment_present: bool,
    pub context: Option<crate::models::assistant_message_feedback::FeedbackReplyContext>,
}

pub async fn upsert_feedback(
    db: &Database,
    write: FeedbackWrite,
) -> mongodb::error::Result<(AssistantMessageFeedback, Option<AssistantMessageFeedback>)> {
    let filter = doc! {"user_id": &write.user_id, "message_id": &write.message_id};
    let existing = db
        .collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .find_one(filter.clone())
        .await?;
    let now = chrono::Utc::now();
    let mut set = doc! {"vote": &write.vote, "updated_at": bson::DateTime::from_chrono(now), "conversation_id": write.conversation_id.clone(), "group_id": write.group_id.clone(), "turn_id": write.turn_id.clone()};
    if write.comment_present {
        if let Some(comment) = write.comment.clone() {
            set.insert("comment", comment);
        } else {
            set.insert("comment", bson::Bson::Null);
        }
    }
    if let Some(context) = write.context {
        set.insert(
            "context",
            bson::to_bson(&context).unwrap_or(bson::Bson::Null),
        );
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
) -> mongodb::error::Result<Option<AssistantMessageFeedback>> {
    db.collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .find_one_and_delete(doc! {"user_id": user_id, "message_id": message_id})
        .await
}

pub async fn reply_context(
    db: &Database,
    surface: &str,
    reply: &AssistantMessage,
    conversation: Option<&crate::models::assistant_conversation::AssistantConversation>,
) -> mongodb::error::Result<crate::models::assistant_message_feedback::FeedbackReplyContext> {
    let user_time = db.collection::<AssistantMessage>(MESSAGE_COLLECTION).find_one(doc! {"conversation_id": &reply.conversation_id, "turn_id": &reply.turn_id, "role":"user"}).await?.map(|row| row.created_at);
    let latency_ms = user_time.map(|created| {
        reply
            .created_at
            .signed_duration_since(created)
            .num_milliseconds()
    });
    let labels = reply
        .activities
        .iter()
        .map(|a| a.label.clone())
        .filter(|v| !v.is_empty())
        .take(20)
        .collect::<Vec<_>>();
    Ok(
        crate::models::assistant_message_feedback::FeedbackReplyContext {
            surface: Some(surface.to_owned()),
            agent_id: conversation.and_then(|c| c.agent_id.clone()),
            agent_kind: conversation.map(|c| {
                if c.agent_id.is_some() {
                    "specialist".into()
                } else {
                    "nyxbot".into()
                }
            }),
            model: conversation.map(|c| c.model.clone()),
            access_mode: conversation.map(|c| format!("{:?}", c.access_mode)),
            reply_status: Some(reply.status.clone()),
            error_code: reply.error_code.clone(),
            origin: reply.origin.as_ref().map(|v| format!("{v:?}")),
            via: reply.via.clone(),
            turn_id: Some(reply.turn_id.clone()),
            reply_seq: Some(reply.seq),
            reply_created_at: Some(reply.created_at),
            latency_ms,
            tool_calls: reply.activities.len() as u32,
            tool_errors: reply
                .activities
                .iter()
                .filter(|a| a.status == "error")
                .count() as u32,
            tool_labels: labels,
            machine_used: reply.activities.iter().any(|a| a.machine.is_some()),
            attachments: reply.attachments.len() as u32,
            steered: reply.steering.is_some(),
            voice: reply.voice.is_some(),
            reply_chars: reply.text.chars().count() as u32,
            context_reset_since_reply: conversation
                .and_then(|c| c.context_reset_at)
                .is_some_and(|at| at > reply.created_at),
            org_group: false,
        },
    )
}

pub fn group_reply_context(
    reply: &GroupMessage,
) -> crate::models::assistant_message_feedback::FeedbackReplyContext {
    crate::models::assistant_message_feedback::FeedbackReplyContext {
        surface: Some("group".into()),
        agent_id: reply.agent_id.clone(),
        agent_kind: None,
        model: None,
        access_mode: None,
        reply_status: None,
        error_code: None,
        origin: None,
        via: None,
        turn_id: None,
        reply_seq: Some(reply.seq),
        reply_created_at: Some(reply.created_at),
        latency_ms: None,
        tool_calls: reply.activities.len() as u32,
        tool_errors: reply
            .activities
            .iter()
            .filter(|a| a.status == "error")
            .count() as u32,
        tool_labels: reply
            .activities
            .iter()
            .map(|a| a.label.clone())
            .take(20)
            .collect(),
        machine_used: reply.activities.iter().any(|a| a.machine.is_some()),
        attachments: reply.attachments.len() as u32,
        steered: false,
        voice: false,
        reply_chars: reply.text.chars().count() as u32,
        context_reset_since_reply: false,
        org_group: reply.org_group,
    }
}
