//! Leased recovery for versioned chat and push results; message bodies stay in their thread.
use super::*;

pub const ATTEMPT_SECONDS: u64 = 300;
const LEASE_SECONDS: i64 = ATTEMPT_SECONDS as i64 + 60;
const MAX_ATTEMPTS: i32 = 128;

pub struct DeliveryClaim {
    pub run: TriggerRun,
    pub attempt: String,
    pub expires_at: DateTime<Utc>,
    pub lease_until: DateTime<Utc>,
}

pub enum Claim {
    Acquired(Box<DeliveryClaim>),
    Busy,
    Legacy,
    Expired,
}

pub async fn versioned(db: &Database, run: &TriggerRun) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(RUNS)
        .find_one(doc! {
            "_id": &run.id, "user_id": &run.user_id, "turn_id": &run.turn_id,
            "result_delivery.version": 1,
        })
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

pub async fn claim(db: &Database, run: &TriggerRun) -> AppResult<Claim> {
    let attempt = Uuid::new_v4().to_string();
    let document = db.collection::<Document>(RUNS).find_one_and_update(
        doc! {"_id":&run.id,"user_id":&run.user_id,"turn_id":&run.turn_id,"outcome":"started",
            "$or":[
                {"result_claimed":{"$ne":true}},
                {"result_delivery.version":1,"result_delivery.attempts":{"$lt":MAX_ATTEMPTS},
                 "$expr":{"$and":[
                    {"$eq":[{"$type":"$result_delivery.lease_until"},"date"]},
                    {"$eq":[{"$type":"$result_delivery.retry_at"},"date"]},
                    {"$eq":[{"$type":"$result_delivery.expires_at"},"date"]},
                    {"$lte":["$result_delivery.lease_until","$$NOW"]},
                    {"$lte":["$result_delivery.retry_at","$$NOW"]},
                    {"$gt":["$result_delivery.expires_at","$$NOW"]}
                 ]}}
            ]},
        vec![doc! {"$set":{
            "result_claimed":true,"result_delivery.version":1,"result_delivery.claim_id":&attempt,
            "result_delivery.lease_until":{"$add":["$$NOW",LEASE_SECONDS*1000]},
            "result_delivery.retry_at":"$$NOW",
            "result_delivery.expires_at":{"$ifNull":["$result_delivery.expires_at",{"$add":["$$NOW",30*60*1000]}]},
            "result_delivery.attempts":{"$add":[{"$ifNull":["$result_delivery.attempts",0]},1]}
        }}],
    ).return_document(ReturnDocument::After).await?;
    if let Some(document) = document {
        let expires_at = document
            .get_document("result_delivery")
            .and_then(|value| value.get_datetime("expires_at"))
            .map_err(|_| AppError::Internal("Result delivery deadline is unavailable".into()))?
            .to_chrono();
        let lease_until = document
            .get_document("result_delivery")
            .and_then(|value| value.get_datetime("lease_until"))
            .map_err(|_| AppError::Internal("Result delivery lease is unavailable".into()))?
            .to_chrono();
        let run = bson::from_document(document)
            .map_err(|_| AppError::Internal("Result delivery identity is unavailable".into()))?;
        return Ok(Claim::Acquired(Box::new(DeliveryClaim {
            run,
            attempt,
            expires_at,
            lease_until,
        })));
    }
    let Some(document) = db
        .collection::<Document>(RUNS)
        .find_one(doc! {
            "_id":&run.id,"user_id":&run.user_id,"turn_id":&run.turn_id,"outcome":"started"
        })
        .projection(
            doc! {"result_delivery":1,"result_claimed":1,"expired":{"$and":[
                {"$eq":[{"$type":"$result_delivery.lease_until"},"date"]},
                {"$eq":[{"$type":"$result_delivery.expires_at"},"date"]},
                {"$lte":["$result_delivery.lease_until","$$NOW"]},
                {"$or":[{"$lte":["$result_delivery.expires_at","$$NOW"]},
                    {"$gte":["$result_delivery.attempts",MAX_ATTEMPTS]}]}
            ]}},
        )
        .await?
    else {
        return Ok(Claim::Busy);
    };
    let Ok(delivery) = document.get_document("result_delivery") else {
        return Ok(if document.get_bool("result_claimed").unwrap_or(false) {
            Claim::Legacy
        } else {
            Claim::Busy
        });
    };
    if delivery.get_i32("version") != Ok(1) {
        return Ok(Claim::Legacy);
    }
    if document.get_bool("expired").unwrap_or(false) {
        return Ok(Claim::Expired);
    }
    Ok(Claim::Busy)
}

pub async fn finish_unrecoverable(
    db: &Database,
    run: &TriggerRun,
    frontend: &str,
) -> AppResult<()> {
    let filter = doc! {
        "_id":&run.id,"user_id":&run.user_id,"turn_id":&run.turn_id,"outcome":"started",
        "result_claimed":true,
        "$or":[
            {"result_delivery":{"$exists":false}},
            {"result_delivery.version":1,"$expr":{"$and":[
                {"$eq":[{"$type":"$result_delivery.lease_until"},"date"]},
                {"$eq":[{"$type":"$result_delivery.expires_at"},"date"]},
                {"$lte":["$result_delivery.lease_until","$$NOW"]},
                {"$or":[{"$lte":["$result_delivery.expires_at","$$NOW"]},
                    {"$gte":["$result_delivery.attempts",MAX_ATTEMPTS]}]}
            ]}}
        ]
    };
    let mut session = db.client().start_session().await?;
    let saved = session
        .start_transaction()
        .and_run((db, filter, frontend), |session, (db, filter, frontend)| {
            Box::pin(async move {
                let operation: AppResult<_> = async {
                    super::notification_delivery::unrecoverable_in_session(db, filter, session)
                        .await?;
                    let saved = db
                        .collection::<TriggerRun>(RUNS)
                        .find_one(filter.clone())
                        .session(&mut *session)
                        .await?;
                    let status = saved
                        .as_ref()
                        .and_then(|run| run.notification_delivery.as_ref());
                    let accepted = status.is_some_and(|s| s.state == "accepted")
                        && saved.as_ref().is_some_and(|run| {
                            run.execution_outcome == Some(RunOutcome::Completed)
                        });
                    super::finish_update(
                        db,
                        filter.clone(),
                        if accepted {
                            RunOutcome::Completed
                        } else {
                            RunOutcome::Failed
                        },
                        FinishReason::Scheduler(if accepted {
                            None
                        } else {
                            Some(
                                status
                                    .and_then(|s| s.reason.as_deref())
                                    .unwrap_or("result_delivery_interrupted"),
                            )
                        }),
                        frontend,
                        session,
                    )
                    .await
                }
                .await;
                transactions::transaction_result(operation)
            })
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Some(run) = saved {
        super::finish_record(db, &run).await?;
    }
    Ok(())
}

pub async fn defer(db: &Database, claim: &DeliveryClaim, retry_after: u64) -> AppResult<()> {
    let delay = retry_after.clamp(1, 86400) as i64 * 1000;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db,claim,delay), |session,(db,claim,delay)| Box::pin(async move {
        let result:AppResult<()> = async {
            let Some(saved) = db.collection::<Document>(RUNS).find_one_and_update(
                claim_filter(claim),
                vec![doc! {"$set":{"result_delivery.retry_at":{"$min":[{"$add":["$$NOW",*delay]},"$result_delivery.expires_at"]},
                    "result_delivery.lease_until":"$$NOW"}}],
            ).return_document(ReturnDocument::After).session(&mut *session).await? else {return Ok(());};
            let at=saved.get_document("result_delivery").and_then(|d|d.get_datetime("retry_at"))
                .map_err(|_|AppError::Internal("Result retry deadline is unavailable".into()))?;
            db.collection::<Document>(WORK).update_one(
                doc! {"_id":&claim.run.id},
                doc! {"$set":{"kind":"run","trigger_id":&claim.run.trigger_id,"at":*at,"fence":Uuid::new_v4().to_string()}},
            ).upsert(true).session(session).await?;
            Ok(())
        }.await;
        transactions::transaction_result(result)
    })).await.map_err(transactions::map_transaction_error)
}

pub(super) fn claim_filter(claim: &DeliveryClaim) -> Document {
    doc! {"_id":&claim.run.id,"user_id":&claim.run.user_id,"turn_id":&claim.run.turn_id,"outcome":"started",
    "result_delivery.version":1,"result_delivery.claim_id":&claim.attempt,
    "$expr":{"$gt":["$result_delivery.lease_until","$$NOW"]}}
}

pub async fn finish(
    db: &Database,
    claim: &DeliveryClaim,
    outcome: RunOutcome,
    reason: FinishReason<'_>,
    frontend: &str,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let saved = session
        .start_transaction()
        .and_run(
            (db, claim, outcome, reason, frontend),
            |session, (db, claim, outcome, reason, frontend)| {
                Box::pin(async move {
                    transactions::transaction_result(
                        super::finish_update(
                            db,
                            claim_filter(claim),
                            *outcome,
                            *reason,
                            frontend,
                            session,
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Some(run) = saved {
        super::finish_record(db, &run).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> (Database, TriggerRun) {
        let db =
            crate::test_utils::connect_transaction_test_database("result_preparation_lease").await;
        let run:TriggerRun=bson::from_document(doc! {
            "_id":"run","trigger_id":"trigger","user_id":"owner","scheduled_at":bson::DateTime::now(),
            "deadline":bson::DateTime::from_chrono(Utc::now()+Duration::hours(1)),"outcome":"started",
            "thread_id":"thread","turn_id":"turn","deliver_to":{"type":"chat","chat_id":"chat"},
            "fence":"old","lease_until":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(7)),
        }).unwrap();
        db.collection::<TriggerRun>(RUNS)
            .insert_one(&run)
            .await
            .unwrap();
        (db, run)
    }
    async fn now(db: &Database) -> i64 {
        db.run_command(doc! {"hello":1})
            .await
            .unwrap()
            .get_datetime("localTime")
            .unwrap()
            .timestamp_millis()
    }
    #[tokio::test]
    async fn scheduled_text_preparation_result_claim_and_retry_use_database_time() {
        let (db, run) = fixture().await;
        let before = now(&db).await;
        let Claim::Acquired(claim) = claim(&db, &run).await.unwrap() else {
            panic!("new claim")
        };
        let after = now(&db).await;
        assert!(
            (before + LEASE_SECONDS * 1000..=after + LEASE_SECONDS * 1000)
                .contains(&claim.lease_until.timestamp_millis())
        );
        assert!(
            (before + 30 * 60 * 1000..=after + 30 * 60 * 1000)
                .contains(&claim.expires_at.timestamp_millis())
        );
        let before = now(&db).await;
        defer(&db, &claim, 5).await.unwrap();
        let after = now(&db).await;
        let saved = db
            .collection::<Document>(RUNS)
            .find_one(doc! {"_id":&run.id})
            .await
            .unwrap()
            .unwrap();
        let retry = saved
            .get_document("result_delivery")
            .unwrap()
            .get_datetime("retry_at")
            .unwrap();
        assert!((before + 5000..=after + 5000).contains(&retry.timestamp_millis()));
        let work = db
            .collection::<Document>(WORK)
            .find_one(doc! {"_id":&run.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(work.get_datetime("at").unwrap(), retry);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn scheduled_text_preparation_invalid_lease_is_not_reclaimed_or_expired() {
        let (db, run) = fixture().await;
        let Claim::Acquired(_) = claim(&db, &run).await.unwrap() else {
            panic!("new claim")
        };
        for value in [
            bson::Bson::Null,
            bson::Bson::Int64(0),
            bson::Bson::String("old".into()),
        ] {
            db.collection::<Document>(RUNS).update_one(doc! {"_id":&run.id},doc! {"$set":{
                "result_delivery.lease_until":value,"result_delivery.retry_at":bson::DateTime::from_millis(0),
                "result_delivery.expires_at":bson::DateTime::from_millis(0),
            }}).await.unwrap();
            assert!(matches!(claim(&db, &run).await.unwrap(), Claim::Busy));
            finish_unrecoverable(&db, &run, "https://app.nyxid.test")
                .await
                .unwrap();
            let saved = db
                .collection::<Document>(RUNS)
                .find_one(doc! {"_id":&run.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.get_str("outcome").unwrap(), "started");
        }
        db.drop().await.unwrap();
    }
}
