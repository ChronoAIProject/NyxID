use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::{
    crm_activity::{
        COLLECTION_NAME as ACTIVITIES, CrmActivity, CrmActivityKind, CrmActivitySummary,
    },
    crm_record::{CrmActor, CrmActorKind, CrmRecord},
};

use super::records::{ReceiptContext, insert_receipt, receipt_in_session, recover_duplicate};

const PAGE_SIZE: i64 = 50;
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NoteInput {
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityResponse {
    pub id: String,
    pub record_id: String,
    pub object_key: String,
    pub kind: CrmActivityKind,
    pub actor: ActivityActorResponse,
    pub summary: ActivitySummaryResponse,
    pub body: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityActorResponse {
    pub kind: CrmActorKind,
    pub user_id: String,
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    pub api_key_id: Option<String>,
    pub source: Option<ActivitySourceResponse>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivitySourceResponse {
    pub conversation_id: String,
    pub turn_id: String,
    pub run_id: Option<String>,
    pub ordered_at: String,
}

impl From<CrmActor> for ActivityActorResponse {
    fn from(actor: CrmActor) -> Self {
        Self {
            kind: actor.kind,
            user_id: actor.user_id,
            agent_id: actor.agent_id,
            agent_name: None,
            api_key_id: actor.api_key_id,
            source: actor.source.map(|source| ActivitySourceResponse {
                conversation_id: source.conversation_id,
                turn_id: source.turn_id,
                run_id: source.run_id,
                ordered_at: source.ordered_at.to_rfc3339(),
            }),
        }
    }
}

/// A single bounded lookup serves a history page or record detail. Actor IDs
/// remain readable when a specialist was removed, without exposing other owners.
pub(crate) async fn hydrate_agent_names(
    db: &Database,
    owner: &str,
    actors: &mut [&mut ActivityActorResponse],
) -> AppResult<()> {
    use crate::models::assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS};
    let pairs: Vec<_> = actors
        .iter()
        .filter_map(|actor| {
            actor.agent_id.as_ref().map(|id| {
                doc! {"_id":id,"user_id":{"$in":[owner,actor.user_id.as_str()]}}
            })
        })
        .collect();
    if pairs.is_empty() {
        return Ok(());
    }
    let rows: Vec<AssistantAgent> = db
        .collection::<AssistantAgent>(AGENTS)
        .find(doc! {"$or":pairs,"destroyed_at":bson::Bson::Null})
        .limit(100)
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_collect()
        .await?;
    for actor in actors {
        actor.agent_name = rows
            .iter()
            .find(|row| Some(&row.id) == actor.agent_id.as_ref())
            .map(|row| {
                row.display_name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(&row.name)
                    .to_owned()
            });
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivitySummaryResponse {
    pub changed_fields: Option<Vec<String>>,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl From<CrmActivitySummary> for ActivitySummaryResponse {
    fn from(summary: CrmActivitySummary) -> Self {
        Self {
            changed_fields: summary.changed_fields,
            from: summary.from,
            to: summary.to,
        }
    }
}

impl From<CrmActivity> for ActivityResponse {
    fn from(activity: CrmActivity) -> Self {
        Self {
            id: activity.id,
            record_id: activity.record_id,
            object_key: activity.object_key,
            kind: activity.kind,
            actor: activity.actor.into(),
            summary: activity.summary.into(),
            body: activity.body,
            created_at: activity.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ActivityPage {
    pub activities: Vec<ActivityResponse>,
    pub next_cursor: Option<String>,
}

pub async fn list(
    db: &Database,
    owner_id: &str,
    record_id: &str,
    cursor: Option<&str>,
) -> AppResult<ActivityPage> {
    let cursor = decode_cursor(cursor)?;
    ensure_record(db, owner_id, record_id).await?;
    let mut filter = doc! {"owner_id": owner_id, "record_id": record_id};
    if let Some(cursor) = &cursor {
        filter.insert(
            "$or",
            vec![
                doc! {"created_at": {"$lt": cursor.created_at}},
                doc! {"created_at": cursor.created_at, "_id": {"$lt": &cursor.id}},
            ],
        );
    }
    let mut rows: Vec<CrmActivity> = db
        .collection::<CrmActivity>(ACTIVITIES)
        .find(filter)
        .sort(doc! {"created_at": -1, "_id": -1})
        .limit(PAGE_SIZE + 1)
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_collect()
        .await?;
    let has_more = rows.len() > PAGE_SIZE as usize;
    rows.truncate(PAGE_SIZE as usize);
    let next_cursor = has_more
        .then(|| rows.last().map(ActivityCursor::from))
        .flatten()
        .map(|cursor| encode_cursor(&cursor))
        .transpose()?;
    let mut activities: Vec<ActivityResponse> = rows.into_iter().map(Into::into).collect();
    hydrate_agent_names(
        db,
        owner_id,
        &mut activities
            .iter_mut()
            .map(|row| &mut row.actor)
            .collect::<Vec<_>>(),
    )
    .await?;
    Ok(ActivityPage {
        activities,
        next_cursor,
    })
}

pub async fn add_note(
    db: &Database,
    owner_id: &str,
    record_id: &str,
    actor: CrmActor,
    input: NoteInput,
    receipt: Option<ReceiptContext>,
) -> AppResult<ActivityResponse> {
    if input.body.trim().is_empty() || input.body.chars().count() > 10_000 {
        return Err(super::validation_error(
            "body",
            "must contain 1 to 10000 characters",
        ));
    }
    let context = (
        db.clone(),
        owner_id.to_owned(),
        record_id.to_owned(),
        actor,
        input,
        receipt.clone(),
    );
    let result = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, owner, record_id, actor, input, receipt)| {
            Box::pin(async move {
                super::fence_owner(db, owner, tx).await?;
                if let Some(value) = receipt_in_session(db, receipt.as_ref(), tx).await? {
                    return serde_json::from_value(value).map_err(|_| {
                        AppError::Internal("Stored CRM note response is invalid".into())
                    });
                }
                let record = db
                    .collection::<CrmRecord>(crate::models::crm_record::COLLECTION_NAME)
                    .find_one(doc! {"_id": record_id, "owner_id": owner})
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(|| AppError::NotFound("CRM record not found".into()))?;
                let now = Utc::now();
                let activity = CrmActivity {
                    id: Uuid::new_v4().to_string(),
                    owner_id: owner.clone(),
                    record_id: record.id,
                    object_key: record.object_key,
                    kind: CrmActivityKind::Note,
                    actor: actor.clone(),
                    summary: CrmActivitySummary::default(),
                    body: Some(input.body.clone()),
                    created_at: now,
                };
                db.collection::<CrmActivity>(ACTIVITIES)
                    .insert_one(&activity)
                    .session(&mut *tx)
                    .await?;
                let response = ActivityResponse::from(activity);
                insert_receipt(db, tx, receipt.as_ref(), &response, now).await?;
                Ok(response)
            })
        },
    )
    .await;
    recover_duplicate(db, receipt.as_ref(), result).await
}

async fn ensure_record(db: &Database, owner_id: &str, record_id: &str) -> AppResult<()> {
    if db
        .collection::<CrmRecord>(crate::models::crm_record::COLLECTION_NAME)
        .find_one(doc! {"_id": record_id, "owner_id": owner_id})
        .max_time(QUERY_TIMEOUT)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("CRM record not found".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
struct ActivityCursor {
    created_at: bson::DateTime,
    id: String,
}

impl From<&CrmActivity> for ActivityCursor {
    fn from(activity: &CrmActivity) -> Self {
        Self {
            created_at: bson::DateTime::from_chrono(activity.created_at),
            id: activity.id.clone(),
        }
    }
}

fn encode_cursor(cursor: &ActivityCursor) -> AppResult<String> {
    bson::to_vec(cursor)
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
        .map_err(|_| super::validation_error("cursor", "is invalid"))
}

fn decode_cursor(cursor: Option<&str>) -> AppResult<Option<ActivityCursor>> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| super::validation_error("cursor", "is invalid"))?;
    let cursor: ActivityCursor =
        bson::from_slice(&bytes).map_err(|_| super::validation_error("cursor", "is invalid"))?;
    if uuid::Uuid::parse_str(&cursor.id).is_err() {
        return Err(super::validation_error("cursor", "is invalid"));
    }
    Ok(Some(cursor))
}

#[allow(dead_code)]
async fn _record_in_session(
    db: &Database,
    owner_id: &str,
    record_id: &str,
    tx: &mut ClientSession,
) -> AppResult<CrmRecord> {
    db.collection(crate::models::crm_record::COLLECTION_NAME)
        .find_one(doc! {"_id": record_id, "owner_id": owner_id})
        .session(tx)
        .await?
        .ok_or_else(|| AppError::NotFound("CRM record not found".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_cursor_round_trip_binds_timestamp_and_id() {
        let cursor = ActivityCursor {
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            id: Uuid::new_v4().to_string(),
        };
        let encoded = encode_cursor(&cursor).unwrap();
        let decoded = decode_cursor(Some(&encoded)).unwrap().unwrap();
        assert_eq!(decoded.created_at, cursor.created_at);
        assert_eq!(decoded.id, cursor.id);
        assert!(decode_cursor(Some("bad")).is_err());

        let invalid_id = encode_cursor(&ActivityCursor {
            created_at: cursor.created_at,
            id: "not-a-uuid".into(),
        })
        .unwrap();
        assert!(decode_cursor(Some(&invalid_id)).is_err());
    }
}
