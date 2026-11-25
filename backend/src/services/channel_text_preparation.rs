//! A pending part proves no send. Old writers can take it only by changing it
//! to Sending, which also fences a new writer's exact preparation attempt.
use super::{api_key_mutation_service as tx, channel_text_delivery as journal};
use crate::{
    errors::{AppError, AppResult},
    models::channel_text_delivery::{COLLECTION_NAME, ChannelTextDelivery, TextPartState},
};
use bson::{Document, doc};
use mongodb::Database;
use std::{future::Future, sync::Arc};

pub const PREPARATION_SECS: i64 = 180;
pub const SEND_SECS: i64 = 120;
pub const RETRY_SECS: u64 = 5;
pub const MANAGER_LEASE_SECS: i64 = 120;

#[derive(Clone)]
pub enum Parent {
    SavedReply { id: String, claim: String },
    Manager { id: String, claim: String },
    Scheduled { id: String, claim: String },
}
impl Parent {
    fn collection(&self) -> &'static str {
        match self {
            Self::SavedReply { .. } => crate::models::assistant_message::COLLECTION_NAME,
            Self::Manager { .. } => crate::models::platform_manager_delivery::COLLECTION_NAME,
            Self::Scheduled { .. } => crate::models::trigger_run::COLLECTION_NAME,
        }
    }
    fn filter(&self) -> Document {
        match self {
            Self::SavedReply { id, claim } => {
                doc! {"_id":id,"role":"assistant","handoff_delivery_lease":claim,"$expr":{"$gt":["$handoff_delivery_due_at","$$NOW"]}}
            }
            Self::Manager { id, claim } => doc! {"_id":id,"state":"dispatching","claim_id":claim,
            "$expr":{"$and":[{"$gt":["$expires_at","$$NOW"]},
                {"$gt":["$updated_at",{"$subtract":["$$NOW",MANAGER_LEASE_SECS*1000]}]}]}},
            Self::Scheduled { id, claim } => {
                doc! {"_id":id,"outcome":"started","result_delivery.version":1,
                "result_delivery.claim_id":claim,"$expr":{"$and":[
                    {"$gt":["$result_delivery.lease_until","$$NOW"]},
                    {"$gt":["$result_delivery.expires_at","$$NOW"]}]}}
            }
        }
    }
    fn touch(&self) -> Vec<Document> {
        let field = match self {
            Self::SavedReply { .. } => {
                return vec![doc! {"$set":{"handoff_delivery_due_at":{"$add":["$$NOW",120_000]}}}];
            }
            Self::Manager { .. } => "updated_at",
            Self::Scheduled { .. } => "result_delivery.effect_checked_at",
        };
        vec![doc! {"$set":{field:"$$NOW"}}]
    }
}

struct Boundary {
    db: Database,
    parent: Parent,
    id: String,
    fingerprint: String,
    index: usize,
    attempt: String,
}
tokio::task_local! {
    static PARENT: Parent;
    static BOUNDARY: Arc<Boundary>;
}
pub fn active() -> bool {
    PARENT.try_with(|_| ()).is_ok()
}
pub async fn with_parent<F: Future>(parent: Parent, work: F) -> F::Output {
    PARENT.scope(parent, work).await
}
pub async fn with_part<F: Future>(
    db: &Database,
    row: &ChannelTextDelivery,
    index: usize,
    attempt: &str,
    work: F,
) -> F::Output {
    if let Ok(parent) = PARENT.try_with(Clone::clone) {
        BOUNDARY
            .scope(
                Arc::new(Boundary {
                    db: db.clone(),
                    parent,
                    id: row.id.clone(),
                    fingerprint: row.fingerprint.clone(),
                    index,
                    attempt: attempt.into(),
                }),
                work,
            )
            .await
    } else {
        work.await
    }
}
fn busy() -> AppError {
    AppError::RateLimitedWithRetry {
        retry_after: RETRY_SECS,
    }
}

pub async fn claim(
    db: &Database,
    row: &ChannelTextDelivery,
    index: usize,
) -> AppResult<journal::PartClaim> {
    if !active() {
        return journal::claim(db, row, index).await;
    }
    let prefix = format!("parts.{index}");
    let attempt = uuid::Uuid::new_v4().to_string();
    let mut filter = doc! {"_id":&row.id,"fingerprint":&row.fingerprint,format!("{prefix}.state"):"pending",
    "$expr":{"$and":[
        {"$lte":[{"$ifNull":["$retry_at",bson::DateTime::from_millis(0)]},"$$NOW"]},
        {"$lte":[{"$ifNull":[journal::part_field(index, "preparation.lease_until"),bson::DateTime::from_millis(0)]},"$$NOW"]}
    ]}};
    for previous in 0..index {
        filter.insert(format!("parts.{previous}.state"), "sent");
    }
    let set = journal::part_update(
        index,
        doc! {
            "attempt_id": &attempt,
            "preparation": {"version":1, "lease_until":{"$add":["$$NOW", PREPARATION_SECS*1000]}},
        },
    );
    let changed = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(filter, vec![doc! {"$set": set}])
        .await?;
    if changed.matched_count == 1 {
        #[cfg(test)]
        {
            super::durable_wake_tests::after("text_part_prepared").await;
            if index == 1 {
                super::durable_wake_tests::after("text_suffix_prepared").await;
            }
        }
        return Ok(journal::PartClaim::Send(attempt));
    }
    let current = journal::load(db, &row.logical_key)
        .await?
        .ok_or_else(journal::unconfirmed)?;
    match current.parts.get(index) {
        Some(part) if part.state == TextPartState::Sent => {
            Ok(journal::PartClaim::Sent(part.clone()))
        }
        Some(part) if part.state == TextPartState::Pending => Err(busy()),
        _ => Err(journal::unconfirmed()),
    }
}

pub async fn effect() -> AppResult<()> {
    let Ok(boundary) = BOUNDARY.try_with(Arc::clone) else {
        return Ok(());
    };
    Box::pin(commit_effect(boundary)).await
}

async fn commit_effect(boundary: Arc<Boundary>) -> AppResult<()> {
    let mut session = boundary.db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(boundary, |session, boundary| {
            Box::pin(async move {
                let result: AppResult<()> = async {
                    // The parent write conflicts with recovery, so a survivor cannot
                    // replace its claim between this check and the part transition.
                    let parent = boundary
                        .db
                        .collection::<Document>(boundary.parent.collection());
                    if parent
                        .update_one(boundary.parent.filter(), boundary.parent.touch())
                        .session(&mut *session)
                        .await?
                        .matched_count
                        != 1
                    {
                        return Err(journal::unconfirmed());
                    }
                    let prefix = format!("parts.{}", boundary.index);
                    let filter = doc! {
                        "_id": &boundary.id, "fingerprint": &boundary.fingerprint,
                        format!("{prefix}.attempt_id"): &boundary.attempt,
                        format!("{prefix}.state"): {"$in": ["pending", "sending"]},
                        format!("{prefix}.preparation.version"): 1,
                        "$expr": {"$gt": [journal::part_field(boundary.index, "preparation.lease_until"), "$$NOW"]},
                    };
                    // Adapters may have a documented refusal fallback or multiple
                    // effects. They keep the same attempt and never extend its send
                    // deadline. Sending is never reclaimed, even after expiry.
                    let set = journal::part_update(boundary.index, doc! {
                        "state": "sending",
                        "preparation": {"version": 1, "lease_until": {"$cond": [
                            {"$eq": [journal::part_field(boundary.index, "state"), "pending"]},
                            {"$add": ["$$NOW", SEND_SECS * 1000]},
                            journal::part_field(boundary.index, "preparation.lease_until"),
                        ]}},
                    });
                    let update = vec![doc! {"$set": set}];
                    if boundary
                        .db
                        .collection::<Document>(COLLECTION_NAME)
                        .update_one(filter, update)
                        .session(&mut *session)
                        .await?
                        .matched_count
                        != 1
                    {
                        // A retry error here could reopen a Sending part after an
                        // earlier effect. Callers may retry only a still-Pending CAS.
                        return Err(journal::unconfirmed());
                    }
                    Ok(())
                }
                .await;
                tx::transaction_result(result)
            })
        })
        .await
        .map_err(tx::map_transaction_error)?;
    #[cfg(test)]
    super::durable_wake_tests::after("text_part_dispatch_committed").await;
    Ok(())
}

/// A failed preparation is retryable only while its exact part is still pending.
pub async fn release(
    db: &Database,
    row: &ChannelTextDelivery,
    index: usize,
    attempt: &str,
) -> AppResult<bool> {
    let prefix = format!("parts.{index}");
    Ok(db.collection::<Document>(COLLECTION_NAME).update_one(
        doc! {"_id":&row.id,format!("{prefix}.state"):"pending",format!("{prefix}.attempt_id"):attempt,format!("{prefix}.preparation.version"):1},
        doc! {"$unset":{format!("{prefix}.preparation"):""}},
    ).await?.matched_count==1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::platform_manager_delivery::COLLECTION_NAME as PARENTS;
    async fn fixture() -> (Database, ChannelTextDelivery, Parent) {
        let db = crate::test_utils::connect_transaction_test_database("text_preparation").await;
        journal::ensure_indexes(&db).await.unwrap();
        db.collection::<Document>(PARENTS).insert_one(doc! {"_id":"parent","claim_id":"owner","state":"dispatching",
            "updated_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::hours(1))}).await.unwrap();
        let row = journal::begin(&db, "logical", "person", "source", "digest", 2)
            .await
            .unwrap();
        (
            db,
            row,
            Parent::Manager {
                id: "parent".into(),
                claim: "owner".into(),
            },
        )
    }
    async fn prepare(
        db: &Database,
        row: &ChannelTextDelivery,
        parent: &Parent,
        index: usize,
    ) -> String {
        match with_parent(parent.clone(), claim(db, row, index))
            .await
            .unwrap()
        {
            journal::PartClaim::Send(attempt) => attempt,
            _ => panic!("expected preparation"),
        }
    }
    async fn boundary(
        db: &Database,
        row: &ChannelTextDelivery,
        parent: &Parent,
        index: usize,
        attempt: &str,
    ) -> AppResult<()> {
        with_parent(parent.clone(), with_part(db, row, index, attempt, effect())).await
    }
    async fn expire(db: &Database, row: &ChannelTextDelivery, index: usize) {
        db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&row.id},doc! {"$set":{format!("parts.{index}.preparation.lease_until"):bson::DateTime::from_millis(0)}}).await.unwrap();
    }
    #[tokio::test]
    async fn text_preparation_two_survivors_and_slow_original_have_one_effect_winner() {
        let (db, row, parent) = fixture().await;
        let old = prepare(&db, &row, &parent, 0).await;
        expire(&db, &row, 0).await;
        let (a, b) = tokio::join!(
            with_parent(parent.clone(), claim(&db, &row, 0)),
            with_parent(parent.clone(), claim(&db, &row, 0))
        );
        let next = match (a, b) {
            (Ok(journal::PartClaim::Send(attempt)), Err(_))
            | (Err(_), Ok(journal::PartClaim::Send(attempt))) => attempt,
            _ => panic!("exactly one surviving preparation"),
        };
        assert!(boundary(&db, &row, &parent, 0, &old).await.is_err());
        assert!(
            journal::finish(&db, &row, 0, &old, TextPartState::Sent, Some("wrong"), None)
                .await
                .is_err()
        );
        boundary(&db, &row, &parent, 0, &next).await.unwrap();
        expire(&db, &row, 0).await;
        let error = boundary(&db, &row, &parent, 0, &next).await.unwrap_err();
        assert!(
            error.retry_after_seconds().is_none(),
            "an expired possible effect cannot ask callers to reopen Sending"
        );
        assert!(with_parent(parent, claim(&db, &row, 0)).await.is_err());
        assert!(!release(&db, &row, 0, &next).await.unwrap());
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn text_preparation_old_writer_preempts_pending_but_neither_writer_reopens_sending() {
        let (db, row, parent) = fixture().await;
        let prepared = prepare(&db, &row, &parent, 0).await;
        let raw = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        let old_view: bson::Document = raw.clone();
        let stored: ChannelTextDelivery = bson::from_document(old_view).unwrap();
        assert_eq!(stored.parts[0].state, TextPartState::Pending);
        let journal::PartClaim::Send(legacy) = journal::claim(&db, &row, 0).await.unwrap() else {
            panic!("old writer must still work")
        };
        assert!(boundary(&db, &row, &parent, 0, &prepared).await.is_err());
        expire(&db, &row, 0).await;
        assert!(
            with_parent(parent.clone(), claim(&db, &row, 0))
                .await
                .is_err()
        );
        journal::finish(
            &db,
            &row,
            0,
            &legacy,
            TextPartState::Sent,
            Some("legacy-receipt"),
            None,
        )
        .await
        .unwrap();
        let next = prepare(&db, &row, &parent, 1).await;
        boundary(&db, &row, &parent, 1, &next).await.unwrap();
        assert!(journal::claim(&db, &row, 1).await.is_err());
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn text_preparation_lost_parent_fences_effect_without_consuming_the_pending_part() {
        let (db, row, parent) = fixture().await;
        let attempt = prepare(&db, &row, &parent, 0).await;
        db.collection::<Document>(PARENTS)
            .update_one(doc! {"_id":"parent"}, doc! {"$set":{"claim_id":"survivor"}})
            .await
            .unwrap();
        assert!(boundary(&db, &row, &parent, 0, &attempt).await.is_err());
        let current = journal::load(&db, &row.logical_key).await.unwrap().unwrap();
        assert_eq!(current.parts[0].state, TextPartState::Pending);
        expire(&db, &row, 0).await;
        let survivor = Parent::Manager {
            id: "parent".into(),
            claim: "survivor".into(),
        };
        let next = prepare(&db, &row, &survivor, 0).await;
        boundary(&db, &row, &survivor, 0, &next).await.unwrap();
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn text_preparation_database_time_and_historical_sending_remain_distinct() {
        let (db, row, parent) = fixture().await;
        let before = db
            .run_command(doc! {"hello":1})
            .await
            .unwrap()
            .get_datetime("localTime")
            .unwrap()
            .timestamp_millis();
        let attempt = prepare(&db, &row, &parent, 0).await;
        let after = db
            .run_command(doc! {"hello":1})
            .await
            .unwrap()
            .get_datetime("localTime")
            .unwrap()
            .timestamp_millis();
        let current = journal::load(&db, &row.logical_key).await.unwrap().unwrap();
        let at = current.parts[0]
            .preparation
            .as_ref()
            .unwrap()
            .lease_until
            .timestamp_millis();
        assert!((before + PREPARATION_SECS * 1000..=after + PREPARATION_SECS * 1000).contains(&at));
        assert!(release(&db, &row, 0, &attempt).await.unwrap());
        let journal::PartClaim::Send(old) = journal::claim(&db, &row, 0).await.unwrap() else {
            panic!("legacy")
        };
        assert!(with_parent(parent, claim(&db, &row, 0)).await.is_err());
        journal::finish(&db, &row, 0, &old, TextPartState::Sent, Some("done"), None)
            .await
            .unwrap();
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn text_preparation_scheduled_parent_and_part_both_fence_the_slow_worker() {
        let (db, row, _) = fixture().await;
        let runs = db.collection::<Document>(crate::models::trigger_run::COLLECTION_NAME);
        runs.insert_one(doc! {"_id":"run", "outcome":"started", "result_delivery": {
            "version":1, "claim_id":"original",
            "lease_until":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(6)),
            "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(30)),
        }}).await.unwrap();
        let original = Parent::Scheduled {
            id: "run".into(),
            claim: "original".into(),
        };
        let attempt = prepare(&db, &row, &original, 0).await;
        runs.update_one(
            doc! {"_id":"run"},
            doc! {"$set":{"result_delivery.claim_id":"survivor"}},
        )
        .await
        .unwrap();
        assert!(boundary(&db, &row, &original, 0, &attempt).await.is_err());
        expire(&db, &row, 0).await;
        let survivor = Parent::Scheduled {
            id: "run".into(),
            claim: "survivor".into(),
        };
        let (a, b) = tokio::join!(
            with_parent(survivor.clone(), claim(&db, &row, 0)),
            with_parent(survivor.clone(), claim(&db, &row, 0)),
        );
        let current = match (a, b) {
            (Ok(journal::PartClaim::Send(attempt)), Err(_))
            | (Err(_), Ok(journal::PartClaim::Send(attempt))) => attempt,
            _ => panic!("one survivor"),
        };
        assert!(boundary(&db, &row, &original, 0, &attempt).await.is_err());
        boundary(&db, &row, &survivor, 0, &current).await.unwrap();
        assert!(!release(&db, &row, 0, &current).await.unwrap());
        db.drop().await.unwrap();
    }
}
