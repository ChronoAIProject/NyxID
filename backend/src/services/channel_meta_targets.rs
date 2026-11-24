//! Bounded durable retry state under one signed provider request. A deferred
//! target never holds the request's independent discovery cursor in place.
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database, options::ReturnDocument};

use super::{grants, provider::REQUESTS};
use crate::{errors::AppResult, models::channel_meta::PROVIDER_TARGETS};

const PAGE: usize = 100;
pub(crate) const ATTEMPTS: i64 = 8;

fn completed(row: &Document) -> AppResult<bool> {
    if row.get_str("status") != Ok("completed") {
        return Ok(false);
    }
    row.get_datetime("completed_at")
        .map_err(|_| grants::incomplete())?;
    for field in ["request_id", "bot_id", "grant_id"] {
        row.get_str(field).map_err(|_| grants::incomplete())?;
    }
    let id = row.get_str("_id").map_err(|_| grants::incomplete())?;
    if uuid::Uuid::parse_str(id)
        .ok()
        .is_none_or(|id| id.get_version() != Some(uuid::Version::Random))
    {
        return Err(grants::incomplete());
    }
    Ok(true)
}

async fn publish(
    db: &Database,
    tx: &mut ClientSession,
    request: &str,
    targets: &[bson::Bson],
) -> AppResult<()> {
    for target in targets {
        let target = target.as_document().ok_or_else(grants::incomplete)?;
        let bot = target.get_str("bot_id").map_err(|_| grants::incomplete())?;
        let grant = target
            .get_str("grant_id")
            .map_err(|_| grants::incomplete())?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut record = doc! {"_id":&id,"request_id":request,"bot_id":bot,"grant_id":grant,
        "status":"pending","retry_at":target.get_datetime("retry_at").copied().unwrap_or_else(|_|bson::DateTime::now()),
        "created_at":bson::DateTime::now()};
        if let Some(scope) = target.get("scope_id") {
            let scope = scope.as_str().ok_or_else(grants::incomplete)?;
            record.insert("scope_id", scope);
        }
        let row = db
            .collection::<Document>(PROVIDER_TARGETS)
            .find_one_and_update(
                doc! {"request_id":request,"bot_id":bot,"grant_id":grant},
                doc! {"$setOnInsert":record},
            )
            .upsert(true)
            .return_document(ReturnDocument::After)
            .session(&mut *tx)
            .await?
            .ok_or_else(grants::incomplete)?;
        if row.get_str("status") == Ok("completed") {
            completed(&row)?;
            db.collection::<Document>(PROVIDER_TARGETS)
                .update_one(
                    doc! {"_id":row.get("_id").cloned(),"request_id":request,
                    "status":"completed","completed_at":row.get("completed_at").cloned()},
                    doc! {"$unset":{"expires_at":""}},
                )
                .session(&mut *tx)
                .await?;
        } else if row.get_str("status") != Ok("pending") {
            return Err(grants::incomplete());
        }
    }
    Ok(())
}

/// Existing arrays and a page published with a cursor are durable handoffs.
/// Insert receipts and remove the exact observed page in one commit. Replaying
/// the transfer preserves a prior completed child and its proof.
pub(crate) async fn transfer_page(
    db: &Database,
    job: &Document,
    claim: &str,
) -> AppResult<Document> {
    let targets = job.get_array("targets").map_err(|_| grants::incomplete())?;
    if targets.is_empty() {
        return Ok(job.clone());
    }
    super::super::transaction_service::run_db_only(db,&(db,job,claim),|tx,&(db,job,claim)| {
        Box::pin(async move {
            let id=job.get_str("_id").map_err(|_|grants::incomplete())?;
            let targets=job.get_array("targets").map_err(|_|grants::incomplete())?;
            let count=targets.len().min(PAGE);
            publish(db,tx,id,&targets[..count]).await?;
            db.collection::<Document>(REQUESTS).find_one_and_update(doc! {
                "_id":id,"status":"pending","claim_id":claim,"targets":targets,
            },doc! {"$set":{"targets":targets[count..].to_vec()},"$inc":{"target_revision":1_i64},"$unset":{"expires_at":""}})
                .return_document(ReturnDocument::After).session(tx).await?.ok_or_else(grants::incomplete)
        })
    }).await
}

pub(crate) async fn due(
    db: &Database,
    request: &str,
    now: bson::DateTime,
) -> AppResult<Vec<Document>> {
    Ok(db
        .collection::<Document>(PROVIDER_TARGETS)
        .find(doc! {
            "request_id":request,"status":"pending","retry_at":{"$lte":now},
        })
        .sort(doc! {"retry_at":1,"_id":1})
        .limit(ATTEMPTS)
        .await?
        .try_collect()
        .await?)
}

pub(crate) async fn settle(
    db: &Database,
    request: &str,
    claim: &str,
    target: &Document,
    result: Result<bool, u32>,
) -> AppResult<()> {
    super::super::transaction_service::run_db_only(db,&(db,request,claim,target,result),|tx,&(db,request,claim,target,result)| {
        Box::pin(async move {
            if db.collection::<Document>(REQUESTS).update_one(doc! {"_id":request,"status":"pending","claim_id":claim},
                doc! {"$inc":{"target_revision":1_i64}}).session(&mut *tx).await?.matched_count!=1 {return Err(grants::incomplete());}
            let update=if let Err(code)=result {
                doc! {"$set":{"retry_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(15)),"last_error_code":code}}
            } else if !result.unwrap_or(false) {
                doc! {"$set":{"retry_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::seconds(1))},"$unset":{"last_error_code":""}}
            } else {doc! {"$set":{"status":"completed","completed_at":bson::DateTime::now()},"$unset":{"retry_at":"","last_error_code":""}}};
            let changed=db.collection::<Document>(PROVIDER_TARGETS).update_one(doc! {
                "_id":target.get("_id").cloned(),"request_id":request,"bot_id":target.get("bot_id").cloned(),
                "grant_id":target.get("grant_id").cloned(),"status":"pending",
            },update).session(tx).await?;
            if changed.matched_count!=1 {return Err(grants::incomplete());}
            Ok(())
        })
    }).await
}

fn discovered(job: &Document) -> bool {
    let compatibility = matches!(
        job.get("compatibility_pending"),
        None | Some(bson::Bson::Boolean(false))
    );
    let scope = match job.get("scope_capture") {
        None => true,
        Some(bson::Bson::Document(capture)) => {
            capture.get_bool("done") == Ok(true)
                && matches!(
                    capture.get("sessions_done"),
                    None | Some(bson::Bson::Boolean(true))
                )
        }
        _ => false,
    };
    job.get_array("targets").is_ok_and(Vec::is_empty) && compatibility && scope
}

pub(crate) async fn finish_or_reschedule(
    db: &Database,
    job: &Document,
    claim: &str,
) -> AppResult<()> {
    super::super::transaction_service::run_db_only(db,&(db,job,claim),|tx,&(db,job,claim)| {
        Box::pin(async move {
            let id=job.get_str("_id").map_err(|_|grants::incomplete())?;
            let filter=doc! {"_id":id,"status":"pending","claim_id":claim};
            let live=db.collection::<Document>(REQUESTS).find_one(filter.clone()).session(&mut *tx).await?.ok_or_else(grants::incomplete)?;
            let pending=db.collection::<Document>(PROVIDER_TARGETS).find_one(doc! {"request_id":id,"status":{"$ne":"completed"}})
                .projection(doc! {"retry_at":1,"status":1}).sort(doc! {"retry_at":1,"_id":1}).session(&mut *tx).await?;
            let update=if discovered(&live) && pending.is_none() {
                doc! {"$set":{"status":"completed","completed_at":bson::DateTime::now(),"target_receipts_pending":true,
                    "proof_expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::days(90))},
                    "$unset":{"targets":"","claim_id":"","next_attempt_at":"","scope_capture":"","compatibility_pending":"","receipt_cleanup_after":"","expires_at":""}}
            } else {
                let next=if !discovered(&live) {bson::DateTime::now()} else {
                    pending.and_then(|row|row.get_datetime("retry_at").copied().ok()).unwrap_or_else(||
                        bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(15)))
                };
                doc! {"$set":{"next_attempt_at":next},"$unset":{"claim_id":""}}
            };
            if db.collection::<Document>(REQUESTS).update_one(filter,update).session(tx).await?.matched_count!=1 {return Err(grants::incomplete());}
            Ok(())
        })
    }).await
}

/// Pending child proof has no TTL. After parent completion, apply its original
/// 90-day proof deadline in indexed pages before enabling the parent's TTL.
pub(crate) async fn expire_completed_receipts(db: &Database) -> AppResult<()> {
    let Some(job) = db
        .collection::<Document>(REQUESTS)
        .find_one(doc! {"status":"completed","target_receipts_pending":true})
        .sort(doc! {"_id":1})
        .await?
    else {
        return Ok(());
    };
    super::super::transaction_service::run_db_only(db, &(db, &job), |tx, &(db, job)| {
        Box::pin(async move {
            let id = job.get_str("_id").map_err(|_| grants::incomplete())?;
            let expires = *job
                .get_datetime("proof_expires_at")
                .map_err(|_| grants::incomplete())?;
            let mut filter = doc! {"request_id":id};
            if let Ok(after) = job.get_str("receipt_cleanup_after") {
                filter.insert("_id", doc! {"$gt":after});
            }
            let mut cursor = db
                .collection::<Document>(PROVIDER_TARGETS)
                .find(filter)
                .sort(doc! {"_id":1})
                .limit(PAGE as i64)
                .session(&mut *tx)
                .await?;
            let rows: Vec<Document> = cursor.stream(&mut *tx).try_collect().await?;
            for row in &rows {
                if !completed(row)? {
                    return Err(grants::incomplete());
                }
                db.collection::<Document>(PROVIDER_TARGETS)
                    .update_one(
                        doc! {"_id":row.get("_id").cloned(),"request_id":id,"status":"completed"},
                        doc! {"$set":{"expires_at":expires}},
                    )
                    .session(&mut *tx)
                    .await?;
            }
            let mut fields = doc! {};
            if let Some(last) = rows.last() {
                fields.insert("receipt_cleanup_after", last.get("_id").cloned());
            }
            if rows.len() < PAGE {
                fields.insert("target_receipts_pending", false);
                fields.insert("expires_at", expires);
            }
            let changed = db
                .collection::<Document>(REQUESTS)
                .update_one(
                    doc! {"_id":id,"status":"completed","target_receipts_pending":true,
                    "receipt_cleanup_after":job.get("receipt_cleanup_after").cloned()},
                    doc! {"$set":fields},
                )
                .session(tx)
                .await?;
            if changed.matched_count != 1 {
                return Err(grants::incomplete());
            }
            Ok(())
        })
    })
    .await
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<Document>(PROVIDER_TARGETS)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"request_id":1,"bot_id":1,"grant_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"request_id":1,"status":1,"retry_at":1,"_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"request_id":1,"_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        ])
        .await?;
    db.collection::<Document>(REQUESTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"status":1,"target_receipts_pending":1,"_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_array_handoff_is_atomic_idempotent_and_retains_pending_child_proof() {
        let db = crate::test_utils::connect_transaction_test_database("meta_target_handoff").await;
        ensure_indexes(&db).await.unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let a = doc! {"bot_id":uuid::Uuid::new_v4().to_string(),"grant_id":uuid::Uuid::new_v4().to_string()};
        let b = doc! {"bot_id":uuid::Uuid::new_v4().to_string(),"grant_id":uuid::Uuid::new_v4().to_string()};
        let array = vec![
            bson::Bson::Document(a.clone()),
            bson::Bson::Document(a.clone()),
            bson::Bson::Document(b.clone()),
        ];
        let jobs = db.collection::<Document>(REQUESTS);
        jobs.insert_one(doc! {"_id":&id,"status":"pending","claim_id":"claim","targets":&array,
            "scope_capture":{"identity":"keyed-subject","done":false,"sessions_done":true,"after":"durable-cursor"}}).await.unwrap();
        let job = jobs.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        let transferred = transfer_page(&db, &job, "claim").await.unwrap();
        assert!(transferred.get_array("targets").unwrap().is_empty());
        assert_eq!(
            transferred
                .get_document("scope_capture")
                .unwrap()
                .get_str("after"),
            Ok("durable-cursor")
        );
        let rows = db.collection::<Document>(PROVIDER_TARGETS);
        assert_eq!(
            rows.count_documents(doc! {"request_id":&id}).await.unwrap(),
            2
        );
        assert_eq!(
            rows.count_documents(doc! {"request_id":&id,"expires_at":{"$exists":true}})
                .await
                .unwrap(),
            0
        );
        let first = rows
            .find_one(doc! {"request_id":&id,"bot_id":a.get_str("bot_id").unwrap()})
            .await
            .unwrap()
            .unwrap();
        settle(&db, &id, "claim", &first, Ok(true)).await.unwrap();
        let completed_at = *rows
            .find_one(doc! {"_id":first.get("_id").cloned()})
            .await
            .unwrap()
            .unwrap()
            .get_datetime("completed_at")
            .unwrap();
        jobs.update_one(doc! {"_id":&id}, doc! {"$set":{"targets":array}})
            .await
            .unwrap();
        let replay = jobs.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        transfer_page(&db, &replay, "claim").await.unwrap();
        let proof = rows
            .find_one(doc! {"_id":first.get("_id").cloned()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(proof.get_datetime("completed_at"), Ok(&completed_at));
        assert!(!proof.contains_key("expires_at"));
        finish_or_reschedule(&db, &replay, "claim").await.unwrap();
        assert_eq!(
            jobs.find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap()
                .get_str("status"),
            Ok("pending")
        );
        jobs.update_one(
            doc! {"_id":&id},
            doc! {"$set":{"claim_id":"next","scope_capture.done":true}},
        )
        .await
        .unwrap();
        let due = due(&db, &id, bson::DateTime::now()).await.unwrap();
        assert_eq!(due.len(), 1);
        settle(&db, &id, "next", &due[0], Ok(true)).await.unwrap();
        let current = jobs.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        finish_or_reschedule(&db, &current, "next").await.unwrap();
        let done = jobs.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        assert_eq!(done.get_str("status"), Ok("completed"));
        assert!(!done.contains_key("expires_at"));
        assert_eq!(
            rows.count_documents(doc! {"request_id":&id,"expires_at":{"$exists":true}})
                .await
                .unwrap(),
            0
        );
        expire_completed_receipts(&db).await.unwrap();
        let expired = jobs.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        assert_eq!(
            expired.get_datetime("expires_at"),
            expired.get_datetime("proof_expires_at")
        );
        assert_eq!(
            rows.count_documents(
                doc! {"request_id":&id,"expires_at":expired.get("proof_expires_at").cloned()}
            )
            .await
            .unwrap(),
            2
        );
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;

    #[tokio::test]
    async fn completed_target_population_does_not_scan_for_due_or_remaining_work() {
        let db =
            crate::test_utils::connect_transaction_test_database("meta_target_query_bound").await;
        ensure_indexes(&db).await.unwrap();
        let request = uuid::Uuid::new_v4().to_string();
        let rows:Vec<_>=(0..1200).map(|_|doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"request_id":&request,"bot_id":uuid::Uuid::new_v4().to_string(),
            "grant_id":uuid::Uuid::new_v4().to_string(),"status":"completed","completed_at":bson::DateTime::now(),
        }).collect();
        db.collection::<Document>(PROVIDER_TARGETS)
            .insert_many(rows)
            .await
            .unwrap();
        for query in [
            doc! {"find":PROVIDER_TARGETS,"filter":{"request_id":&request,"status":"pending","retry_at":{"$lte":bson::DateTime::now()}},"sort":{"retry_at":1,"_id":1},"limit":ATTEMPTS},
            doc! {"find":PROVIDER_TARGETS,"filter":{"request_id":&request,"status":{"$ne":"completed"}},"projection":{"retry_at":1,"status":1},"sort":{"retry_at":1,"_id":1},"limit":1},
        ] {
            let explain = db
                .run_command(doc! {"explain":query,"verbosity":"executionStats"})
                .await
                .unwrap();
            let stats = explain.get_document("executionStats").unwrap();
            let count = |field| {
                stats
                    .get_i64(field)
                    .unwrap_or_else(|_| i64::from(stats.get_i32(field).unwrap()))
            };
            assert_eq!(count("nReturned"), 0);
            assert!(
                count("totalDocsExamined") <= 2 && count("totalKeysExamined") <= 4,
                "{stats:?}"
            );
        }
        db.drop().await.unwrap();
    }
}
