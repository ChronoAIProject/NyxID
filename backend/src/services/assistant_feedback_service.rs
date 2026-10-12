use crate::models::{
    assistant_agent::{AgentKind, AssistantAgent, COLLECTION_NAME as AGENTS},
    assistant_conversation::{AgentRole, AssistantConversation},
    assistant_group::{GroupMessage, MESSAGES_COLLECTION_NAME},
    assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGE_COLLECTION},
    assistant_message_feedback::{AssistantMessageFeedback, COLLECTION_NAME, FeedbackReplyContext},
};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use serde::{Deserialize, Serialize};

/// Log target for every feedback state change. Lines carry identifiers and
/// reply metadata only: never the comment, reply, prompt or tool text.
pub const LOG_TARGET: &str = "nyxid::assistant_feedback";
/// Bound on distinct tool labels kept in a reply context.
pub const MAX_TOOL_LABELS: usize = 20;

#[derive(Debug, Serialize)]
pub struct AdminCount {
    pub day: String,
    pub up: i64,
    pub down: i64,
}

/// Admin read of recent rows. Returns the stored rows unchanged; handlers
/// project them into response DTOs.
pub async fn admin_recent(
    db: &Database,
    filter: bson::Document,
    limit: i64,
) -> mongodb::error::Result<Vec<AssistantMessageFeedback>> {
    db.collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"created_at": -1, "_id": -1})
        .limit(limit)
        .await?
        .try_collect::<Vec<_>>()
        .await
}

pub async fn admin_counts(
    db: &Database,
    filter: bson::Document,
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

/// The vote state a transcript shows beside a reply. Projected so a malformed
/// or newer `context` on a stored row can never fail a history read.
#[derive(Clone, Debug, Deserialize)]
pub struct FeedbackState {
    pub message_id: String,
    pub vote: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// The acting person's votes on one page of transcript messages.
pub async fn states_for_messages(
    db: &Database,
    user_id: &str,
    message_ids: &[String],
) -> mongodb::error::Result<Vec<FeedbackState>> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    db.collection::<FeedbackState>(COLLECTION_NAME)
        .find(doc! {"user_id": user_id, "message_id": {"$in": message_ids}})
        .projection(doc! {"_id": 0, "message_id": 1, "vote": 1, "comment": 1, "updated_at": 1})
        .await?
        .try_collect()
        .await
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
    /// The sanitized comment. Applied only when `comment_present`.
    pub comment: Option<String>,
    /// The request carried a `comment` field: a vote-only write keeps the
    /// stored comment, while an explicit empty comment clears it.
    pub comment_present: bool,
    pub context: Option<FeedbackReplyContext>,
}

/// What a write changed, named as the log `event` and audit `change`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackEvent {
    Submitted,
    Changed,
    CommentUpdated,
    Withdrawn,
}

impl FeedbackEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Changed => "changed",
            Self::CommentUpdated => "comment_updated",
            Self::Withdrawn => "withdrawn",
        }
    }
}

/// Classify an upsert against the row it replaced. `None` is an idempotent
/// repeat (same vote, no comment change), which logs and audits nothing.
pub fn classify(
    previous: Option<&AssistantMessageFeedback>,
    row: &AssistantMessageFeedback,
) -> Option<FeedbackEvent> {
    match previous {
        None => Some(FeedbackEvent::Submitted),
        Some(previous) if previous.vote != row.vote => Some(FeedbackEvent::Changed),
        Some(previous) if previous.comment != row.comment => Some(FeedbackEvent::CommentUpdated),
        Some(_) => None,
    }
}

/// One atomic upsert. `ReturnDocument::Before` gives the replaced row, so two
/// concurrent first votes cannot both be classified `submitted`: MongoDB
/// retries the losing duplicate-key upsert as an update of the winner's row.
pub async fn upsert_feedback(
    db: &Database,
    write: FeedbackWrite,
) -> mongodb::error::Result<(AssistantMessageFeedback, Option<AssistantMessageFeedback>)> {
    let filter = doc! {"user_id": &write.user_id, "message_id": &write.message_id};
    let now = chrono::Utc::now();
    let new_id = uuid::Uuid::new_v4().to_string();
    let mut set = doc! {
        "vote": &write.vote,
        "updated_at": bson::DateTime::from_chrono(now),
        "conversation_id": write.conversation_id.clone(),
        "group_id": write.group_id.clone(),
        "turn_id": write.turn_id.clone(),
    };
    if write.comment_present {
        set.insert("comment", write.comment.clone());
    }
    if let Some(context) = write.context.as_ref() {
        set.insert(
            "context",
            bson::to_bson(context)
                .map_err(|_| mongodb::error::Error::custom("feedback context"))?,
        );
    }
    let update = doc! {
        "$set": set,
        "$setOnInsert": {"_id": &new_id, "created_at": bson::DateTime::from_chrono(now)},
    };
    let previous = db
        .collection::<AssistantMessageFeedback>(COLLECTION_NAME)
        .find_one_and_update(filter, update)
        .upsert(true)
        .return_document(ReturnDocument::Before)
        .await?;
    let row = AssistantMessageFeedback {
        id: previous.as_ref().map_or(new_id, |p| p.id.clone()),
        user_id: write.user_id,
        conversation_id: write.conversation_id,
        group_id: write.group_id,
        message_id: write.message_id,
        turn_id: write.turn_id,
        vote: write.vote,
        comment: if write.comment_present {
            write.comment
        } else {
            previous.as_ref().and_then(|p| p.comment.clone())
        },
        context: write
            .context
            .or_else(|| previous.as_ref().and_then(|p| p.context.clone())),
        created_at: previous.as_ref().map_or(now, |p| p.created_at),
        updated_at: now,
    };
    Ok((row, previous))
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

fn tool_labels<'a>(labels: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for label in labels.filter(|label| !label.is_empty()) {
        if out.len() == MAX_TOOL_LABELS {
            break;
        }
        if !out.iter().any(|seen| seen == label) {
            out.push(label.to_owned());
        }
    }
    out
}

fn serde_name<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
}

/// Snapshot of a conversation reply. Besides the reply row the handler has
/// loaded, it reads the turn's user message (latency), the thread's agent
/// (kind and access override) and the owner's settings (effective access
/// mode): point lookups by key.
pub async fn reply_context(
    db: &Database,
    reply: &AssistantMessage,
    conversation: &AssistantConversation,
) -> mongodb::error::Result<FeedbackReplyContext> {
    let user_time = db
        .collection::<AssistantMessage>(MESSAGE_COLLECTION)
        .find_one(doc! {"conversation_id": &reply.conversation_id, "user_id": &reply.user_id, "turn_id": &reply.turn_id, "role": "user"})
        .await?
        .map(|row| row.created_at);
    let latency_ms = user_time.map(|created| {
        reply
            .created_at
            .signed_duration_since(created)
            .num_milliseconds()
    });
    let agent_owner = conversation
        .agent_owner_id
        .as_deref()
        .unwrap_or(&conversation.user_id);
    let agent = match conversation.agent_id.as_deref() {
        Some(agent_id) => {
            db.collection::<AssistantAgent>(AGENTS)
                .find_one(doc! {"_id": agent_id, "user_id": agent_owner})
                .await?
        }
        None => None,
    };
    let agent_kind = match agent.as_ref().map(|agent| agent.kind) {
        Some(AgentKind::Nyxbot) => "nyxbot",
        Some(AgentKind::Specialist) => "specialist",
        None => match conversation.role {
            AgentRole::Orchestrator => "nyxbot",
            AgentRole::Subagent => "specialist",
        },
    };
    // The same resolution the owner gate applies: a per-agent override, else
    // the acting person's owner-level mode.
    let access_mode = match agent.as_ref().and_then(|agent| agent.access_mode_override) {
        Some(mode) => mode,
        None => {
            crate::services::assistant_settings_service::get(db, &conversation.user_id)
                .await
                .map_err(|_| mongodb::error::Error::custom("assistant settings"))?
                .access_mode
        }
    };
    Ok(FeedbackReplyContext {
        surface: Some("conversation".into()),
        agent_id: conversation.agent_id.clone(),
        agent_kind: Some(agent_kind.into()),
        model: Some(conversation.model.clone()),
        access_mode: serde_name(&access_mode),
        reply_status: Some(reply.status.clone()),
        error_code: reply.error_code.clone(),
        origin: reply.origin.as_ref().and_then(serde_name),
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
        tool_labels: tool_labels(reply.activities.iter().map(|a| a.label.as_str())),
        machine_used: reply.activities.iter().any(|a| a.machine.is_some()),
        attachments: reply.attachments.len() as u32,
        steered: reply.steering.is_some(),
        voice: reply.voice.is_some(),
        reply_chars: reply.text.chars().count() as u32,
        context_reset_since_reply: conversation
            .context_reset_at
            .is_some_and(|at| at > reply.created_at),
        org_group: false,
    })
}

/// Snapshot of a group reply. Conversation-only fields stay absent.
/// The group feedback PUT path uses this to persist the same metadata-only
/// context as conversation feedback.
pub fn group_reply_context(reply: &GroupMessage) -> FeedbackReplyContext {
    FeedbackReplyContext {
        surface: Some("group".into()),
        agent_id: reply.agent_id.clone(),
        reply_seq: Some(reply.seq),
        reply_created_at: Some(reply.created_at),
        tool_calls: reply.activities.len() as u32,
        tool_errors: reply
            .activities
            .iter()
            .filter(|a| a.status == "error")
            .count() as u32,
        tool_labels: tool_labels(reply.activities.iter().map(|a| a.label.as_str())),
        machine_used: reply.activities.iter().any(|a| a.machine.is_some()),
        attachments: reply.attachments.len() as u32,
        reply_chars: reply.text.chars().count() as u32,
        org_group: reply.org_group,
        ..FeedbackReplyContext::default()
    }
}

fn opt(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("")
}

/// Emit the one structured line for a state change. `row` is the stored row
/// after a write, or the deleted row for `withdrawn`. Comment text is never
/// logged: only whether one exists and its length.
pub fn log_event(
    event: FeedbackEvent,
    row: &AssistantMessageFeedback,
    previous: Option<&AssistantMessageFeedback>,
) {
    let empty = FeedbackReplyContext::default();
    let context = row.context.as_ref().unwrap_or(&empty);
    let labels = context.tool_labels.join(",");
    tracing::info!(
        target: LOG_TARGET,
        event = event.as_str(),
        feedback_id = %row.id,
        user_id = %row.user_id,
        message_id = %row.message_id,
        conversation_id = opt(&row.conversation_id),
        group_id = opt(&row.group_id),
        vote = %row.vote,
        previous_vote = previous.map_or("", |p| p.vote.as_str()),
        has_comment = row.comment.is_some(),
        comment_chars = row.comment.as_ref().map_or(0, |c| c.chars().count()),
        has_context = row.context.is_some(),
        surface = opt(&context.surface),
        agent_id = opt(&context.agent_id),
        agent_kind = opt(&context.agent_kind),
        model = opt(&context.model),
        access_mode = opt(&context.access_mode),
        reply_status = opt(&context.reply_status),
        error_code = opt(&context.error_code),
        origin = opt(&context.origin),
        via = opt(&context.via),
        turn_id = opt(&context.turn_id),
        reply_seq = context.reply_seq.unwrap_or(0),
        reply_created_at = %context.reply_created_at.map(|at| at.to_rfc3339()).unwrap_or_default(),
        latency_ms = context.latency_ms.unwrap_or(-1),
        tool_calls = context.tool_calls,
        tool_errors = context.tool_errors,
        tool_labels = %labels,
        machine_used = context.machine_used,
        attachments = context.attachments,
        steered = context.steered,
        voice = context.voice,
        reply_chars = context.reply_chars,
        context_reset_since_reply = context.context_reset_since_reply,
        org_group = context.org_group,
        "assistant feedback state changed"
    );
}

/// Metadata-only audit details shared by submitted/changed/withdrawn events.
pub fn audit_details(
    event: FeedbackEvent,
    row: &AssistantMessageFeedback,
    previous: Option<&AssistantMessageFeedback>,
) -> serde_json::Value {
    let context = row.context.as_ref();
    serde_json::json!({
        "message_id": row.message_id,
        "vote": row.vote,
        "conversation_id": row.conversation_id,
        "group_id": row.group_id,
        "group": row.group_id.is_some(),
        "previous_vote": previous.map(|p| p.vote.clone()),
        "change": event.as_str(),
        "has_comment": row.comment.is_some(),
        "agent_id": context.and_then(|c| c.agent_id.clone()),
        "agent_kind": context.and_then(|c| c.agent_kind.clone()),
        "model": context.and_then(|c| c.model.clone()),
        "error_code": context.and_then(|c| c.error_code.clone()),
        "tool_errors": context.map(|c| c.tool_errors),
    })
}
