use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database, bson::doc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::{
    crm_activity::{
        COLLECTION_NAME as ACTIVITIES, CrmActivity, CrmActivityKind, CrmActivitySummary,
    },
    crm_record::{CrmActor, CrmRecord},
};

use super::records::{ReceiptContext, insert_receipt, receipt_in_session, recover_duplicate};

const PAGE_SIZE: i64 = 50;

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
    pub actor: CrmActor,
    pub summary: CrmActivitySummary,
    pub body: Option<String>,
    pub created_at: String,
}

impl From<CrmActivity> for ActivityResponse {
    fn from(activity: CrmActivity) -> Self {
        Self {
            id: activity.id,
            record_id: activity.record_id,
            object_key: activity.object_key,
            kind: activity.kind,
            actor: activity.actor,
            summary: activity.summary,
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
    let offset = decode_cursor(cursor)?;
    ensure_record(db, owner_id, record_id).await?;
    let mut rows: Vec<CrmActivity> = db
        .collection::<CrmActivity>(ACTIVITIES)
        .find(doc! {"owner_id": owner_id, "record_id": record_id})
        .sort(doc! {"created_at": -1, "_id": -1})
        .skip(offset)
        .limit(PAGE_SIZE + 1)
        .await?
        .try_collect()
        .await?;
    let has_more = rows.len() > PAGE_SIZE as usize;
    rows.truncate(PAGE_SIZE as usize);
    Ok(ActivityPage {
        activities: rows.into_iter().map(Into::into).collect(),
        next_cursor: has_more.then(|| encode_cursor(offset + PAGE_SIZE as u64)),
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
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("CRM record not found".into()));
    }
    Ok(())
}

fn encode_cursor(offset: u64) -> String {
    URL_SAFE_NO_PAD.encode(offset.to_be_bytes())
}

fn decode_cursor(cursor: Option<&str>) -> AppResult<u64> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| super::validation_error("cursor", "is invalid"))?;
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| super::validation_error("cursor", "is invalid"))?;
    Ok(u64::from_be_bytes(bytes))
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
