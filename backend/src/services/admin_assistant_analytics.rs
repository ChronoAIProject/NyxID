//! Bounded, metadata-only NyxBot analytics. Never fetch transcript text.
use super::assistant_nyxagent::{ACTIVE_TURN_HEARTBEAT_STALE_SECS, ACTIVE_TURN_TTL_SECS};
use crate::models::{assistant_conversation, assistant_message};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use serde::{Deserialize, Serialize};

const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Default, Deserialize, Serialize)]
pub struct ActivityDay {
    pub day: String,
    pub sessions: i64,
}

#[derive(Default, Deserialize, Serialize)]
pub struct TurnCounts {
    pub completed: i64,
    pub failed: i64,
    pub stopped: i64,
    pub with_tools: i64,
    pub with_tool_errors: i64,
}

#[derive(Default, Deserialize, Serialize)]
pub struct TurnDay {
    pub day: String,
    pub turns: i64,
}

#[derive(Default, Deserialize)]
struct Count {
    count: i64,
}

#[derive(Default, Deserialize)]
struct ActivityResult {
    sessions: Vec<Count>,
    accounts: Vec<Count>,
    daily: Vec<ActivityDay>,
}

#[derive(Default, Deserialize)]
struct TurnsResult {
    totals: Vec<TurnCounts>,
    daily: Vec<TurnDay>,
}

pub struct Summary {
    pub sessions: i64,
    pub accounts: i64,
    pub running_now: u64,
    pub outcomes: TurnCounts,
    pub daily_sessions: Vec<ActivityDay>,
    pub daily_turns: Vec<TurnDay>,
}

async fn aggregate<T: serde::de::DeserializeOwned + Default>(
    db: &Database,
    pipeline: Vec<Document>,
) -> mongodb::error::Result<T> {
    let mut cursor = db
        .collection::<Document>(assistant_message::COLLECTION_NAME)
        .aggregate(pipeline)
        .max_time(QUERY_TIMEOUT)
        .await?;
    cursor
        .try_next()
        .await?
        .map(bson::from_document)
        .transpose()
        .map(|row| row.unwrap_or_default())
        .map_err(|_| mongodb::error::Error::custom("assistant analytics projection"))
}

fn running_filter(now: DateTime<Utc>) -> Document {
    doc! {
        "active_turn.stop_requested": {"$ne": true},
        "active_turn.heartbeat_at": {
            "$gte": bson::DateTime::from_chrono(now - chrono::Duration::seconds(ACTIVE_TURN_HEARTBEAT_STALE_SECS)),
            "$lte": bson::DateTime::from_chrono(now),
        },
        "$expr": {"$gt": [
            {"$ifNull": ["$active_turn.lease_expires_at", {"$add": ["$active_turn.started_at", ACTIVE_TURN_TTL_SECS * 1000]}]},
            bson::DateTime::from_chrono(now),
        ]},
    }
}

pub async fn summary(
    db: &Database,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> mongodb::error::Result<Summary> {
    let range =
        doc! {"$gte": bson::DateTime::from_chrono(from), "$lt": bson::DateTime::from_chrono(to)};
    let activity = vec![
        doc! {"$match": {"created_at": &range, "role": "user"}},
        doc! {"$project": {"conversation_id": 1, "user_id": 1, "created_at": 1}},
        doc! {"$facet": {
            "sessions": [{"$group": {"_id": "$conversation_id"}}, {"$count": "count"}],
            "accounts": [{"$group": {"_id": "$user_id"}}, {"$count": "count"}],
            "daily": [
                {"$group": {"_id": {"day": {"$dateToString": {"format": "%Y-%m-%d", "date": "$created_at", "timezone": "UTC"}}, "session": "$conversation_id"}}},
                {"$group": {"_id": "$_id.day", "sessions": {"$sum": 1}}},
                {"$project": {"_id": 0, "day": "$_id", "sessions": 1}},
                {"$sort": {"day": 1}},
            ],
        }},
    ];
    // Voice delivery can mirror the same settled turn into its parent thread.
    // Group by the durable turn id before counting outcomes or tool use.
    // A cancelled visible voice result overrides its hidden submit acknowledgement.
    let turns = vec![
        doc! {"$match": {"created_at": &range, "role": "assistant"}},
        doc! {"$project": {
            "turn_id": 1, "created_at": 1,
            "completed": {"$cond": [{"$eq": ["$status", "completed"]}, 1, 0]},
            "stopped": {"$cond": [{"$eq": ["$error_code", "cancelled"]}, 1, 0]},
            "with_tools": {"$cond": [{"$gt": [{"$size": {"$ifNull": ["$activities", []]}}, 0]}, 1, 0]},
            "with_tool_errors": {"$cond": [{"$in": ["error", {"$ifNull": ["$activities.status", []]}]}, 1, 0]},
        }},
        doc! {"$group": {
            "_id": "$turn_id", "created_at": {"$min": "$created_at"},
            "completed": {"$max": "$completed"}, "stopped": {"$max": "$stopped"},
            "with_tools": {"$max": "$with_tools"}, "with_tool_errors": {"$max": "$with_tool_errors"},
        }},
        doc! {"$facet": {
            "totals": [{"$group": {
                "_id": null,
                "completed": {"$sum": {"$cond": [{"$eq": ["$stopped", 1]}, 0, "$completed"]}},
                "stopped": {"$sum": "$stopped"},
                "failed": {"$sum": {"$cond": [{"$and": [{"$eq": ["$completed", 0]}, {"$eq": ["$stopped", 0]}]}, 1, 0]}},
                "with_tools": {"$sum": "$with_tools"}, "with_tool_errors": {"$sum": "$with_tool_errors"},
            }}],
            "daily": [
                {"$group": {"_id": {"$dateToString": {"format": "%Y-%m-%d", "date": "$created_at", "timezone": "UTC"}}, "turns": {"$sum": 1}}},
                {"$project": {"_id": 0, "day": "$_id", "turns": 1}},
                {"$sort": {"day": 1}},
            ],
        }},
    ];
    let (activity, turns, running_now) = tokio::try_join!(
        aggregate::<ActivityResult>(db, activity),
        aggregate::<TurnsResult>(db, turns),
        async {
            db.collection::<Document>(assistant_conversation::COLLECTION_NAME)
                .count_documents(running_filter(now))
                .max_time(QUERY_TIMEOUT)
                .await
        },
    )?;
    Ok(Summary {
        sessions: activity.sessions.first().map_or(0, |row| row.count),
        accounts: activity.accounts.first().map_or(0, |row| row.count),
        running_now,
        outcomes: turns.totals.into_iter().next().unwrap_or_default(),
        daily_sessions: activity.daily,
        daily_turns: turns.daily,
    })
}
