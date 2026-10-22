//! Leased recovery for versioned chat results; message bodies stay in their thread.
use super::*;

pub const ATTEMPT_SECONDS: u64 = 300;
const LEASE_SECONDS: i64 = ATTEMPT_SECONDS as i64 + 60;
const MAX_ATTEMPTS: i32 = 128;

pub struct DeliveryClaim {
    pub run: TriggerRun,
    pub attempt: String,
    pub expires_at: DateTime<Utc>,
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
    let now = Utc::now();
    let attempt = Uuid::new_v4().to_string();
    let document = db.collection::<Document>(RUNS).find_one_and_update(
        doc! {"_id":&run.id,"user_id":&run.user_id,"turn_id":&run.turn_id,"outcome":"started",
            "$or":[
                {"result_claimed":{"$ne":true}},
                {"result_delivery.version":1,"result_delivery.lease_until":{"$lte":date(now)},
                 "result_delivery.retry_at":{"$lte":date(now)},"result_delivery.expires_at":{"$gt":date(now)},
                 "result_delivery.attempts":{"$lt":MAX_ATTEMPTS}}
            ]},
        vec![doc! {"$set":{
            "result_claimed":true,"result_delivery.version":1,"result_delivery.claim_id":&attempt,
            "result_delivery.lease_until":date(now+Duration::seconds(LEASE_SECONDS)),
            "result_delivery.retry_at":date(now),
            "result_delivery.expires_at":{"$ifNull":["$result_delivery.expires_at",date(now+Duration::minutes(30))]},
            "result_delivery.attempts":{"$add":[{"$ifNull":["$result_delivery.attempts",0]},1]}
        }}],
    ).return_document(ReturnDocument::After).await?;
    if let Some(document) = document {
        let expires_at = document
            .get_document("result_delivery")
            .and_then(|value| value.get_datetime("expires_at"))
            .map_err(|_| AppError::Internal("Result delivery deadline is unavailable".into()))?
            .to_chrono();
        let run = bson::from_document(document)
            .map_err(|_| AppError::Internal("Result delivery identity is unavailable".into()))?;
        return Ok(Claim::Acquired(Box::new(DeliveryClaim {
            run,
            attempt,
            expires_at,
        })));
    }
    let Some(document) = db
        .collection::<Document>(RUNS)
        .find_one(doc! {
            "_id":&run.id,"user_id":&run.user_id,"turn_id":&run.turn_id,"outcome":"started"
        })
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
    if delivery
        .get_datetime("lease_until")
        .is_ok_and(|at| at.to_chrono() <= now)
        && (delivery
            .get_datetime("expires_at")
            .is_ok_and(|at| at.to_chrono() <= now)
            || (delivery.get_i32("attempts").unwrap_or(MAX_ATTEMPTS) >= MAX_ATTEMPTS))
    {
        return Ok(Claim::Expired);
    }
    Ok(Claim::Busy)
}

pub async fn finish_unrecoverable(
    db: &Database,
    run: &TriggerRun,
    frontend: &str,
) -> AppResult<()> {
    let now = date(Utc::now());
    let filter = doc! {
        "_id":&run.id,"user_id":&run.user_id,"turn_id":&run.turn_id,"outcome":"started",
        "result_claimed":true,
        "$or":[
            {"result_delivery":{"$exists":false}},
            {"result_delivery.version":1,"result_delivery.lease_until":{"$lte":now},
                "$or":[{"result_delivery.expires_at":{"$lte":now}},
                    {"result_delivery.attempts":{"$gte":MAX_ATTEMPTS}}]}
        ]
    };
    let mut session = db.client().start_session().await?;
    let saved = session
        .start_transaction()
        .and_run((db, filter, frontend), |session, (db, filter, frontend)| {
            Box::pin(async move {
                transactions::transaction_result(
                    super::finish_update(
                        db,
                        filter.clone(),
                        RunOutcome::Failed,
                        Some("result_delivery_interrupted"),
                        frontend,
                        session,
                    )
                    .await,
                )
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
    let at =
        (Utc::now() + Duration::seconds(retry_after.clamp(1, 86400) as i64)).min(claim.expires_at);
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db,claim,at), |session,(db,claim,at)| Box::pin(async move {
        let result:AppResult<()> = async {
            let changed = db.collection::<Document>(RUNS).update_one(
                claim_filter(claim),
                doc! {"$set":{"result_delivery.retry_at":date(*at),"result_delivery.lease_until":date(Utc::now())}},
            ).session(&mut *session).await?;
            if changed.matched_count == 0 {return Ok(());}
            db.collection::<Document>(WORK).update_one(
                doc! {"_id":&claim.run.id},
                doc! {"$set":{"kind":"run","trigger_id":&claim.run.trigger_id,"at":date(*at),"fence":Uuid::new_v4().to_string()}},
            ).upsert(true).session(session).await?;
            Ok(())
        }.await;
        transactions::transaction_result(result)
    })).await.map_err(transactions::map_transaction_error)
}

fn claim_filter(claim: &DeliveryClaim) -> Document {
    doc! {"_id":&claim.run.id,"user_id":&claim.run.user_id,"turn_id":&claim.run.turn_id,"outcome":"started",
    "result_delivery.version":1,"result_delivery.claim_id":&claim.attempt,
    "result_delivery.lease_until":{"$gt":date(Utc::now())}}
}

pub async fn finish(
    db: &Database,
    claim: &DeliveryClaim,
    outcome: RunOutcome,
    reason: Option<&str>,
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
