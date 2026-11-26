//! First-party organization oversight. This read never constructs execution authority.
use bson::{Bson, Document, doc};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::Database;
use serde::{Deserialize, Serialize};

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::COLLECTION_NAME as AGENTS,
        assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
        assistant_message::COLLECTION_NAME as MESSAGES,
        channel_bot::COLLECTION_NAME as BOTS,
        channel_meta::GRANT_SCOPES,
        nyxbot_channel::{COLLECTION_NAME as CHANNELS, THREADS_COLLECTION_NAME as THREADS},
        org_channel::LANES,
    },
    services::assistant_nyxagent::{TurnError, valid_id},
};

const HANDLE: &str = "org-guest-";

#[derive(Serialize)]
pub(crate) struct Conversation {
    pub id: String,
    title: &'static str,
    pub last_message_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    pub agent_id: String,
    pub read_only: bool,
    pub channel: Source,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Source {
    pub nyxbot_channel_id: String,
    pub chat_id: String,
    pub parent_chat_id: Option<String>,
    pub thread_id: Option<String>,
    platform: String,
    bot_label: String,
    chat_kind: String,
    chat_title: Option<String>,
    parent_title: Option<String>,
}

#[derive(Deserialize)]
struct Row {
    #[serde(rename = "_id")]
    id: String,
    agent_id: String,
    created_at: bson::DateTime,
    updated_at: bson::DateTime,
    channel: Source,
}

impl From<Row> for Conversation {
    fn from(row: Row) -> Self {
        Self {
            id: format!("{HANDLE}{}", row.id.trim_start_matches("nyxa-")),
            title: "Guest conversation",
            last_message_at: row.updated_at.to_chrono(),
            created_at: row.created_at.to_chrono(),
            agent_id: row.agent_id,
            read_only: true,
            channel: row.channel,
        }
    }
}

fn missing() -> AppError {
    AppError::NotFound("Guest conversation not found".into())
}

fn conversation_id(handle: &str) -> Option<String> {
    let id = format!("nyxa-{}", handle.strip_prefix(HANDLE)?);
    valid_id(&id).then_some(id)
}

async fn admin_org(db: &Database, actor: &str, agent: &str) -> AppResult<String> {
    let row = db
        .collection::<Document>(AGENTS)
        .find_one(doc! {"_id": agent, "kind": "specialist", "destroyed_at": Bson::Null})
        .projection(doc! {"user_id": 1})
        .await?
        .ok_or_else(missing)?;
    let org = row.get_str("user_id").map_err(|_| missing())?;
    match super::require_admin(db, actor, org).await {
        Ok(()) => Ok(org.to_owned()),
        Err(AppError::Forbidden(_)) => Err(missing()),
        Err(error) => Err(error),
    }
}

fn guest_input() -> Document {
    doc! {
        "role": "user", "origin": "channel", "author": "guest", "status": "completed",
        "steering": Bson::Null, "voice": Bson::Null, "execution_pending": {"$ne": true},
    }
}

// Resolve the current channel revision, exact chat partition, effective agent
// and retained guest input before applying the page limit. A group may have
// several guest lanes and an independent member transcript.
fn eligible(org: &str, selectors: Document) -> Vec<Document> {
    let mut guest = doc! {
        "user_id": org, "guest_turn": true, "guest_model_setup": {"$ne": true},
        "collaboration": Bson::Null, "group_id": Bson::Null,
        "channel.org_execution.org_id": org, "channel.org_execution.actor_id": org,
        "channel.org_execution.guest": true, "channel.org_execution.version": 1,
        "channel.org_execution.sender_hash": {"$type": "string", "$ne": ""},
        "channel.org_execution.link_id": Bson::Null,
        "channel.org_execution.link_revision": Bson::Null,
    };
    guest.extend(selectors);
    vec![
        doc! {"$match": guest},
        doc! {"$match": {"$expr": {"$eq": ["$channel.org_execution.agent_id", "$agent_id"]}}},
        doc! {"$lookup": {
            "from": CHANNELS, "localField": "channel.nyxbot_channel_id", "foreignField": "_id",
            "let": {"revision": "$channel.org_execution.revision"},
            "pipeline": [
                {"$match": {"status": "active", "bot_owner_id": org,
                    "org_specialist.org_id": org, "org_specialist.version": 1,
                    "$expr": {"$eq": ["$org_specialist.revision", "$$revision"]}}},
                {"$project": {"user_id": 1, "channel_bot_id": 1, "agent_id": 1, "platform": 1, "bot_label": 1}},
            ],
            "as": "binding",
        }},
        doc! {"$unwind": "$binding"},
        doc! {"$lookup": {
            "from": LANES, "localField": "_id", "foreignField": "conversation_id",
            "let": {"channel": "$binding._id", "partition": "$channel.partition", "agent": "$agent_id",
                "revision": "$channel.org_execution.revision", "sender": "$channel.org_execution.sender_hash",
                "guest_revision": {"$ifNull": ["$channel.org_execution.guest_revision", Bson::Null]}},
            "pipeline": [
                {"$match": {"org_id": org, "actor_id": org, "$expr": {"$and": [
                    {"$eq": ["$channel_id", "$$channel"]},
                    {"$eq": ["$partition", "$$partition"]},
                    {"$eq": ["$agent_id", "$$agent"]},
                    {"$eq": ["$binding_revision", "$$revision"]},
                    {"$eq": ["$sender_hash", "$$sender"]},
                    {"$eq": [{"$ifNull": ["$guest_revision", Bson::Null]}, "$$guest_revision"]},
                ]}}},
                {"$limit": 1},
                {"$project": {"_id": 1}},
            ],
            "as": "lane",
        }},
        doc! {"$match": {"lane.0": {"$exists": true}}},
        doc! {"$lookup": {
            "from": BOTS, "localField": "binding.channel_bot_id", "foreignField": "_id",
            "pipeline": [
                {"$match": {"user_id": org, "is_active": true}},
                {"$project": {"_id": 1}},
            ],
            "as": "bot",
        }},
        doc! {"$match": {"bot.0": {"$exists": true}}},
        doc! {"$lookup": {
            "from": THREADS, "localField": "binding._id", "foreignField": "channel_id",
            "let": {"partition": "$channel.partition", "owner": "$binding.user_id"},
            "pipeline": [
                {"$match": {"$expr": {"$and": [
                    {"$eq": ["$partition", "$$partition"]},
                    {"$eq": ["$user_id", "$$owner"]},
                ]}}},
                {"$project": {"kind": 1, "title": 1, "agent_id": 1, "record_scope": 1,
                    "settings_chat_id": 1, "parent_chat_id": 1}},
            ],
            "as": "source",
        }},
        doc! {"$unwind": "$source"},
        doc! {"$lookup": {
            "from": THREADS, "localField": "source.settings_chat_id", "foreignField": "_id",
            "let": {"channel": "$binding._id", "owner": "$binding.user_id"},
            "pipeline": [
                {"$match": {"$expr": {"$and": [
                    {"$eq": ["$channel_id", "$$channel"]},
                    {"$eq": ["$user_id", "$$owner"]},
                ]}}},
                {"$project": {"kind": 1, "title": 1, "agent_id": 1}},
            ],
            "as": "settings",
        }},
        doc! {"$set": {"settings": {"$first": "$settings"}}},
        doc! {"$match": {
            "$or": [
                {"source.record_scope": {"$ne": "platform_thread"}, "source.kind": {"$in": ["private", "group", "channel"]}},
                {"source.record_scope": "platform_thread", "settings.kind": {"$in": ["group", "channel"]}},
            ],
            "$expr": {"$eq": [
                {"$ifNull": ["$settings.agent_id", "$source.agent_id", "$binding.agent_id"]},
                "$agent_id",
            ]},
        }},
        doc! {"$lookup": {
            "from": GRANT_SCOPES, "localField": "meta_grant.bot_id", "foreignField": "bot_id",
            "let": {"grant": "$meta_grant.grant_id", "generation": "$meta_grant.generation", "bot": "$binding.channel_bot_id"},
            "pipeline": [
                {"$match": {"status": {"$in": ["current", "sealed", "retired"]}, "erasure_requested": {"$ne": true},
                    "$expr": {"$and": [
                        {"$eq": ["$bot_id", "$$bot"]},
                        {"$eq": ["$grant_id", "$$grant"]},
                        {"$lte": ["$from_generation", "$$generation"]},
                        {"$or": [
                            {"$eq": [{"$ifNull": ["$to_generation", Bson::Null]}, Bson::Null]},
                            {"$gte": ["$to_generation", "$$generation"]},
                        ]},
                    ]}}},
                {"$project": {"_id": 1}},
            ],
            "as": "retained_grant",
        }},
        doc! {"$match": {"$or": [{"meta_grant": Bson::Null}, {"retained_grant.0": {"$exists": true}}]}},
        doc! {"$lookup": {
            "from": MESSAGES, "localField": "_id", "foreignField": "conversation_id",
            "pipeline": [
                {"$match": {"user_id": org}},
                {"$match": guest_input()},
                {"$limit": 1},
                {"$project": {"_id": 1}},
            ],
            "as": "guest_inputs",
        }},
        doc! {"$match": {"guest_inputs.0": {"$exists": true}}},
    ]
}

fn projection() -> Document {
    doc! {"$project": {
        "agent_id": 1, "created_at": 1, "updated_at": 1,
        "channel": {
            "nyxbot_channel_id": "$binding._id", "chat_id": "$source._id",
            "parent_chat_id": "$settings._id",
            "thread_id": {"$cond": [{"$eq": ["$source.record_scope", "platform_thread"]}, "$source._id", Bson::Null]},
            "platform": "$binding.platform", "bot_label": "$binding.bot_label",
            "chat_kind": {"$ifNull": ["$settings.kind", "$source.kind"]},
            "chat_title": {"$cond": [{"$eq": ["$source.kind", "private"]}, Bson::Null, "$source.title"]},
            "parent_title": "$settings.title",
        },
    }}
}

async fn rows(db: &Database, mut pipeline: Vec<Document>) -> AppResult<Vec<Row>> {
    pipeline.push(projection());
    Ok(db
        .collection::<Document>(CONVERSATIONS)
        .aggregate(pipeline)
        .with_type::<Row>()
        .await?
        .try_collect()
        .await?)
}

pub(crate) async fn list(
    db: &Database,
    actor: &str,
    agent: &str,
    limit: i64,
    before: Option<&str>,
) -> AppResult<(Vec<Conversation>, Option<String>)> {
    let org = admin_org(db, actor, agent).await?;
    let mut selectors = doc! {"agent_id": agent};
    if let Some(cursor) = before {
        let bad = || AppError::BadRequest("Invalid guest conversation cursor".into());
        let (time, handle) = cursor.split_once(':').ok_or_else(bad)?;
        let id = conversation_id(handle).ok_or_else(bad)?;
        let time = bson::DateTime::from_millis(time.parse().map_err(|_| bad())?);
        selectors.insert(
            "$or",
            bson::bson!([
                {"updated_at": {"$lt": time}},
                {"updated_at": time, "_id": {"$lt": id}},
            ]),
        );
    }
    let mut pipeline = eligible(&org, selectors);
    pipeline.insert(1, doc! {"$sort": {"updated_at": -1, "_id": -1}});
    pipeline.push(doc! {"$limit": limit + 1});
    let mut result = rows(db, pipeline).await?;
    if admin_org(db, actor, agent).await? != org {
        return Err(missing());
    }
    let more = result.len() > limit as usize;
    result.truncate(limit as usize);
    let result: Vec<Conversation> = result.into_iter().map(Into::into).collect();
    let next = more.then(|| {
        let last = result.last().expect("nonempty page");
        format!("{}:{}", last.last_message_at.timestamp_millis(), last.id)
    });
    Ok((result, next))
}

pub(crate) async fn latest_by_chat(
    db: &Database,
    actor: &str,
    org: &str,
    channel: &str,
    chat_ids: &[&str],
) -> AppResult<std::collections::HashMap<String, String>> {
    if chat_ids.is_empty() {
        return Ok(Default::default());
    }
    super::require_admin(db, actor, org).await?;
    let mut pipeline = eligible(org, doc! {"channel.nyxbot_channel_id": channel});
    pipeline.extend([
        doc! {"$match": {"source._id": {"$in": chat_ids}, "source.record_scope": {"$ne": "platform_thread"}}},
        doc! {"$lookup": {
            "from": AGENTS, "localField": "agent_id", "foreignField": "_id",
            "pipeline": [
                {"$match": {"user_id": org, "kind": "specialist", "destroyed_at": Bson::Null}},
                {"$project": {"_id": 1}},
            ],
            "as": "specialist",
        }},
        doc! {"$match": {"specialist.0": {"$exists": true}}},
        doc! {"$sort": {"updated_at": -1, "_id": -1}},
        doc! {"$group": {"_id": "$source._id", "conversation": {"$first": "$$ROOT"}}},
        doc! {"$replaceWith": "$conversation"},
    ]);
    Ok(rows(db, pipeline)
        .await?
        .into_iter()
        .map(|row| {
            let conversation = Conversation::from(row);
            (conversation.channel.chat_id, conversation.id)
        })
        .collect())
}

#[derive(Deserialize)]
struct MessageRow {
    seq: i64,
    role: String,
    text: String,
    status: String,
    created_at: bson::DateTime,
    error_code: Option<String>,
    delivery: Option<String>,
    #[serde(default)]
    failures: Vec<crate::models::assistant_conversation::ToolFailure>,
}

#[derive(Serialize)]
pub(crate) struct Message {
    pub seq: i64,
    role: String,
    text: String,
    status: String,
    created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery_status: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_failures: Vec<ToolDiagnostic>,
}

#[derive(Serialize)]
struct ToolDiagnostic {
    kind: String,
    reason: Option<String>,
    status: Option<u16>,
}

impl From<MessageRow> for Message {
    fn from(row: MessageRow) -> Self {
        let error_code = row
            .error_code
            .as_deref()
            .map(|value| TurnError::new(value).code);
        let delivery_status = row.delivery.filter(|value| {
            matches!(
                value.as_str(),
                "delivered"
                    | "failed"
                    | "generated_not_confirmed"
                    | "interrupted_prefix"
                    | "unconfirmed"
            )
        });
        Self {
            seq: row.seq,
            role: row.role,
            text: if row.status == "completed" {
                row.text.chars().take(12_000).collect()
            } else {
                String::new()
            },
            status: row.status,
            created_at: row.created_at.to_chrono(),
            error_code,
            delivery_status,
            tool_failures: row
                .failures
                .into_iter()
                .filter(|value| {
                    matches!(
                        value.kind.as_str(),
                        "tool_error" | "service_status" | "internal" | "refused"
                    )
                })
                .take(16)
                .map(|value| ToolDiagnostic {
                    kind: value.kind,
                    reason: value.reason.filter(|reason| {
                        matches!(
                            reason.as_str(),
                            "downstream_http_error"
                                | "approval_required"
                                | "approval_denied"
                                | "guest_operation_denied"
                                | "invalid_tool_arguments"
                        )
                    }),
                    status: value.status.filter(|status| (400..=599).contains(status)),
                })
                .collect(),
        }
    }
}

pub(crate) async fn history(
    db: &Database,
    actor: &str,
    handle: &str,
    limit: i64,
    before: Option<i64>,
) -> AppResult<(String, Conversation, Vec<Message>, Option<i64>)> {
    let id = conversation_id(handle).ok_or_else(missing)?;
    let row = db
        .collection::<Document>(CONVERSATIONS)
        .find_one(doc! {"_id": &id, "guest_turn": true, "channel.org_execution.guest": true})
        .projection(doc! {"agent_id": 1})
        .await?
        .ok_or_else(missing)?;
    let agent = row.get_str("agent_id").map_err(|_| missing())?;
    let org = admin_org(db, actor, agent).await?;
    let scope = || eligible(&org, doc! {"_id": &id, "agent_id": agent});
    if rows(db, scope()).await?.is_empty() {
        return Err(missing());
    }
    let mut selectors = doc! {"conversation_id": &id, "user_id": &org, "origin": "channel",
    "$or": [
        {"role": "user", "author": "guest", "status": "completed",
            "steering": Bson::Null, "voice": Bson::Null, "execution_pending": {"$ne": true}},
        {"role": "assistant", "author": Bson::Null, "continuation_of": Bson::Null,
            "status": {"$in": ["completed", "failed"]}},
    ]};
    if let Some(seq) = before {
        selectors.insert("seq", doc! {"$lt": seq});
    }
    let messages = db.collection::<Document>(MESSAGES)
        .aggregate([
            doc! {"$match": selectors},
            doc! {"$sort": {"seq": -1}},
            doc! {"$lookup": {
                "from": MESSAGES, "localField": "conversation_id", "foreignField": "conversation_id",
                "let": {"turn": "$turn_id"},
                "pipeline": [
                    {"$match": {"user_id": &org, "$expr": {"$eq": ["$turn_id", "$$turn"]}}},
                    {"$match": guest_input()},
                    {"$limit": 1},
                    {"$project": {"_id": 1}},
                ],
                "as": "guest_inputs",
            }},
            doc! {"$match": {"guest_inputs.0": {"$exists": true}}},
            doc! {"$limit": limit + 1},
            doc! {"$project": {
                "seq": 1, "role": 1, "text": 1, "status": 1, "created_at": 1, "error_code": 1,
                "delivery": "$turn_metadata.channel_delivery.status",
                "failures": {"$cond": [{"$eq": ["$role", "assistant"]},
                    {"$map": {"input": {"$filter": {"input": {"$ifNull": ["$activities", []]},
                        "as": "activity", "cond": {"$ne": [{"$ifNull": ["$$activity.failure", Bson::Null]}, Bson::Null]}}},
                        "as": "activity", "in": "$$activity.failure"}}, []]},
            }},
        ])
        .with_type::<MessageRow>()
        .await?
        .try_collect::<Vec<_>>()
        .await?;

    // Recheck live scope after loading the page, including during binding retirement
    // and erasure. Neither read invokes execution, delivery or write fences.
    let conversation = rows(db, scope()).await?.pop().ok_or_else(missing)?;
    if admin_org(db, actor, agent).await? != org {
        return Err(missing());
    }
    let more = messages.len() > limit as usize;
    let mut messages: Vec<_> = messages
        .into_iter()
        .take(limit as usize)
        .map(Message::from)
        .collect();
    messages.reverse();
    let next = more.then(|| messages[0].seq);
    Ok((org, conversation.into(), messages, next))
}
