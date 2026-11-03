use std::collections::{BTreeMap, HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Serialize;

use crate::errors::{AppError, AppResult};
use crate::models::{
    crm_link::{COLLECTION_NAME as LINKS, CrmLink},
    crm_record::{COLLECTION_NAME as RECORDS, CrmRecord},
};

use super::records::{RecordResponse, hydrate_record};

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
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("CRM record not found".into()));
    }
    let links: Vec<CrmLink> = db
        .collection::<CrmLink>(LINKS)
        .find(doc! {"owner_id": owner_id, "target_record_id": target_record_id})
        .await?
        .try_collect()
        .await?;
    let source_ids: Vec<String> = links
        .iter()
        .map(|link| link.source_record_id.clone())
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
            .await?
            .try_collect()
            .await?
    };
    let live_ids: HashSet<&str> = source_rows.iter().map(|row| row.id.as_str()).collect();
    let by_id: HashMap<_, _> = source_rows
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();
    let mut grouped: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for link in links {
        if live_ids.contains(link.source_record_id.as_str()) {
            grouped
                .entry((link.source_object, link.field_key))
                .or_default()
                .push(link.source_record_id);
        }
    }
    let mut session = db.client().start_session().await?;
    let mut groups = Vec::with_capacity(grouped.len());
    for ((object_key, field_key), ids) in grouped {
        let mut unique: Vec<String> = ids
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        unique.sort();
        let count = unique.len() as u64;
        let mut records = Vec::new();
        for id in unique.iter().take(50) {
            if let Some(row) = by_id.get(id.as_str()) {
                records.push(hydrate_record(db, row, &mut session).await?);
            }
        }
        groups.push(RelatedGroup {
            object_key,
            field_key,
            count,
            records,
        });
    }
    Ok(RelatedResponse { groups })
}
