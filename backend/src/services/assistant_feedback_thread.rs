//! Bounded, human-visible transcript excerpts for privileged feedback review.
use crate::{
    errors::{AppError, AppResult},
    models::assistant_message_feedback::COLLECTION_NAME as FEEDBACK,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{Bson, Document, doc},
};
use serde::Serialize;
use std::time::Duration;

const PAGE_SIZE: usize = 6;
const DEADLINE: Duration = Duration::from_secs(10);
#[derive(Serialize, utoipa::ToSchema)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub rated: bool,
    pub truncated: bool,
}
pub struct Page {
    pub messages: Vec<Message>,
    pub next_cursor: Option<String>,
    pub unavailable: bool,
}
fn missing() -> Page {
    Page {
        messages: vec![],
        next_cursor: None,
        unavailable: true,
    }
}
fn invalid() -> AppError {
    AppError::BadRequest("feedback_thread_cursor_invalid".into())
}
fn position(row: &Document) -> AppResult<(i64, String)> {
    Ok((
        row.get_i64("seq")
            .or_else(|_| row.get_i32("seq").map(i64::from))
            .map_err(|_| AppError::Internal("Invalid message sequence".into()))?,
        row.get_str("_id")
            .map_err(|_| AppError::Internal("Invalid message identity".into()))?
            .into(),
    ))
}
pub async fn preview(db: &Database, feedback_id: &str, cursor: Option<&str>) -> AppResult<Page> {
    tokio::time::timeout(
        DEADLINE + Duration::from_secs(2),
        read(db, feedback_id, cursor),
    )
    .await
    .map_err(|_| AppError::AdminUsageQueryTimeout)?
}
async fn read(db: &Database, feedback_id: &str, cursor: Option<&str>) -> AppResult<Page> {
    // Every page observes feedback, parent, anchor and messages in one snapshot.
    let mut session = db.client().start_session().snapshot(true).await?;
    let feedback = db
        .collection::<Document>(FEEDBACK)
        .find_one(doc! {"_id": feedback_id})
        .projection(doc! {"conversation_id": 1, "group_id": 1, "message_id": 1})
        .max_time(DEADLINE)
        .session(&mut session)
        .await?
        .ok_or_else(|| AppError::NotFound("feedback not found".into()))?;
    let anchor_id = feedback
        .get_str("message_id")
        .map_err(|_| AppError::Internal("Invalid feedback anchor".into()))?;
    let (scope_key, scope_id, parents, messages, roles) = match (
        feedback.get_str("conversation_id").ok(),
        feedback.get_str("group_id").ok(),
    ) {
        (Some(id), None) => (
            "conversation_id",
            id,
            "assistant_conversations",
            "assistant_messages",
            vec!["user", "assistant"],
        ),
        (None, Some(id)) => (
            "group_id",
            id,
            "assistant_groups",
            "assistant_group_messages",
            vec!["user", "agent"],
        ),
        _ => return Ok(missing()),
    };
    if db
        .collection::<Document>(parents)
        .find_one(doc! {"_id": scope_id})
        .projection(doc! {"_id": 1})
        .max_time(DEADLINE)
        .session(&mut session)
        .await?
        .is_none()
    {
        return Ok(missing());
    }
    let mut scope = doc! {scope_key: scope_id, "role": {"$in": roles}};
    if scope_key == "conversation_id" {
        // Synthetic user instructions and internal wake-ups are not human input.
        scope.insert("$or", vec![doc! {"role": "assistant"}, doc! {
            "role": "user", "origin": {"$in": [Bson::Null, Bson::String("user".into()), Bson::String("channel".into())]},
            "continuation_of": Bson::Null,
        }]);
    }
    let collection = db.collection::<Document>(messages);
    let mut anchor_filter = scope.clone();
    anchor_filter.insert("_id", anchor_id);
    let Some(anchor) = collection
        .find_one(anchor_filter)
        .projection(doc! {"seq": 1})
        .max_time(DEADLINE)
        .session(&mut session)
        .await?
    else {
        return Ok(missing());
    };
    let anchor_position = position(&anchor)?;
    let before = if let Some(cursor) = cursor {
        if cursor.len() > 1024 {
            return Err(invalid());
        }
        let raw = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
        let decoded: Vec<String> = serde_json::from_slice(&raw).map_err(|_| invalid())?;
        if decoded.len() != 3 || decoded[0] != feedback_id || decoded[1] != anchor_id {
            return Err(invalid());
        }
        let mut filter = scope.clone();
        filter.insert("_id", &decoded[2]);
        let row = collection
            .find_one(filter)
            .projection(doc! {"seq": 1})
            .max_time(DEADLINE)
            .session(&mut session)
            .await?
            .ok_or_else(invalid)?;
        let pos = position(&row)?;
        if pos > anchor_position {
            return Err(invalid());
        }
        Some(pos)
    } else {
        None
    };
    let boundary = before.as_ref().unwrap_or(&anchor_position);
    // Each parent has a unique (parent, seq) index. Seek directly through it;
    // adding _id to the sort would force a blocking sort over the whole history.
    // The cursor still resolves and validates its boundary ID within this parent.
    scope.insert(
        "seq",
        doc! {if before.is_some() {"$lt"} else {"$lte"}: boundary.0},
    );
    let mut rows = collection
        .find(scope)
        .sort(doc! {"seq": -1})
        // Deliberate allowlist: no tool, attachment, system, or credential fields.
        .projection(doc! {"seq": 1, "role": 1, "text": 1, "created_at": 1})
        .limit((PAGE_SIZE + 1) as i64)
        .max_time(DEADLINE)
        .session(&mut session)
        .await?;
    let mut docs = Vec::new();
    while rows.advance(&mut session).await? {
        docs.push(rows.deserialize_current()?);
    }
    let more = docs.len() > PAGE_SIZE;
    docs.truncate(PAGE_SIZE);
    let next_cursor = if more {
        docs.last().map(|d| {
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&[feedback_id, anchor_id, d.get_str("_id").unwrap_or("")])
                    .expect("string cursor"),
            )
        })
    } else {
        None
    };
    let mut output = Vec::new();
    for row in docs.into_iter().rev() {
        let (_, id) = position(&row)?;
        let raw = row.get_str("text").unwrap_or("");
        let excerpt = super::assistant_agent_learning::redacted_excerpt(raw);
        output.push(Message {
            rated: id == anchor_id,
            id,
            role: row.get_str("role").unwrap_or("user").into(),
            truncated: excerpt.truncated,
            text: excerpt.text,
            created_at: row
                .get_datetime("created_at")
                .map_err(|_| AppError::Internal("Invalid message time".into()))?
                .to_chrono(),
        });
    }
    Ok(Page {
        messages: output,
        next_cursor,
        unavailable: false,
    })
}
