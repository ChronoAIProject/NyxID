use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Serialize;

use crate::errors::{AppError, AppResult};
use crate::models::{
    crm_link::COLLECTION_NAME as LINKS,
    crm_record::{COLLECTION_NAME as RECORDS, CrmRecord},
};

use super::records::{RecordResponse, hydrate_records};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize)]
pub struct RelatedResponse {
    pub groups: Vec<RelatedGroup>,
}

#[derive(Debug, Serialize)]
pub struct RelatedGroup {
    pub object_key: String,
    pub field_key: String,
    pub count: u64,
    pub records: Vec<RecordResponse>,
}

pub async fn related(
    db: &Database,
    owner_id: &str,
    target_record_id: &str,
) -> AppResult<RelatedResponse> {
    if db
        .collection::<CrmRecord>(RECORDS)
        .find_one(doc! {"_id": target_record_id, "owner_id": owner_id})
        .max_time(QUERY_TIMEOUT)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("CRM record not found".into()));
    }
    let grouped: Vec<mongodb::bson::Document> = db
        .collection::<mongodb::bson::Document>(LINKS)
        .aggregate(vec![
            doc! {"$match": {"owner_id": owner_id, "target_record_id": target_record_id}},
            doc! {"$lookup": {
                "from": RECORDS,
                "localField": "source_record_id",
                "foreignField": "_id",
                "as": "source",
            }},
            doc! {"$unwind": "$source"},
            doc! {"$match": {
                "source.owner_id": owner_id,
                "source.archived_at": mongodb::bson::Bson::Null,
            }},
            doc! {"$group": {
                "_id": {"object_key": "$source_object", "field_key": "$field_key"},
                "count": {"$sum": 1_i64},
                "source_ids": {"$topN": {
                    "n": 50_i64,
                    "sortBy": {"source_record_id": 1_i64},
                    "output": "$source_record_id",
                }},
            }},
            doc! {"$sort": {"_id.object_key": 1_i64, "_id.field_key": 1_i64}},
        ])
        .max_time(QUERY_TIMEOUT)
        .await?
        .try_collect()
        .await?;
    let source_ids: Vec<String> = grouped
        .iter()
        .filter_map(|group| group.get_array("source_ids").ok())
        .flatten()
        .filter_map(mongodb::bson::Bson::as_str)
        .map(str::to_owned)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let source_rows: Vec<CrmRecord> = if source_ids.is_empty() {
        vec![]
    } else {
        db.collection::<CrmRecord>(RECORDS)
            .find(doc! {
                "_id": {"$in": &source_ids},
                "owner_id": owner_id,
                "archived_at": mongodb::bson::Bson::Null,
            })
            .max_time(QUERY_TIMEOUT)
            .await?
            .try_collect()
            .await?
    };
    let mut session = db.client().start_session().await?;
    let responses = hydrate_records(db, &source_rows, &mut session).await?;
    let by_id: HashMap<_, _> = responses
        .into_iter()
        .map(|record| (record.id.clone(), record))
        .collect();
    let mut groups = Vec::with_capacity(grouped.len());
    for group in grouped {
        let key = group
            .get_document("_id")
            .map_err(|_| AppError::Internal("Invalid CRM related group".into()))?;
        let object_key = key
            .get_str("object_key")
            .map_err(|_| AppError::Internal("Invalid CRM related object".into()))?
            .to_owned();
        let field_key = key
            .get_str("field_key")
            .map_err(|_| AppError::Internal("Invalid CRM related field".into()))?
            .to_owned();
        let count = group
            .get_i64("count")
            .ok()
            .and_then(|count| u64::try_from(count).ok())
            .ok_or_else(|| AppError::Internal("Invalid CRM related count".into()))?;
        let records = group
            .get_array("source_ids")
            .map_err(|_| AppError::Internal("Invalid CRM related ids".into()))?
            .iter()
            .filter_map(mongodb::bson::Bson::as_str)
            .filter_map(|id| by_id.get(id).cloned())
            .collect();
        groups.push(RelatedGroup {
            object_key,
            field_key,
            count,
            records,
        });
    }
    Ok(RelatedResponse { groups })
}
