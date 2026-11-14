//! Order cache mutations against provider snapshots without changing price rows.
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_rate_cache::{BillingRateCache, COLLECTION_NAME as RATES},
        billing_rate_epoch::{COLLECTION_NAME as EPOCH, RateEpoch, SINGLETON_ID},
    },
    services::transaction_service,
};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};

/// At most this many replacements and rejected metric codes per transaction.
pub(super) const PUBLICATION_BATCH_SIZE: usize = 100;

#[derive(Debug)]
pub(crate) struct EpochCreationRace;
#[derive(Debug)]
struct Superseded;

pub(super) async fn capture(db: &Database) -> AppResult<i64> {
    Ok(db
        .collection::<RateEpoch>(EPOCH)
        .find_one(doc! {"_id":SINGLETON_ID})
        .await?
        .map_or(0, |epoch| epoch.generation))
}

/// Join an existing explicit writer's transaction, including absent retirements.
pub(super) async fn advance_in_session(
    db: &Database,
    session: &mut ClientSession,
) -> AppResult<()> {
    let result = db
        .collection::<RateEpoch>(EPOCH)
        .find_one_and_update(
            doc! {"_id":SINGLETON_ID},
            doc! {"$inc":{"generation":1_i64}},
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .session(session)
        .await;
    match result {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(AppError::Internal("Rate epoch was not advanced".into())),
        // MongoDB 8 does not label every transactional upsert collision as
        // transient. Only this singleton creation collision retries the body.
        Err(error) if super::ledger::is_duplicate_key_error(&error) => Err(
            AppError::DatabaseError(mongodb::error::Error::custom(EpochCreationRace)),
        ),
        Err(error) => Err(error.into()),
    }
}

async fn advance_captured(
    db: &Database,
    session: &mut ClientSession,
    captured: i64,
) -> AppResult<i64> {
    let next = captured
        .checked_add(1)
        .ok_or_else(|| AppError::Internal("Rate epoch exhausted".into()))?;
    let mut filter = doc! {"_id":SINGLETON_ID};
    if captured == 0 {
        filter.insert(
            "$or",
            bson::bson!([{ "generation": 0_i64 }, { "generation": {"$exists":false} }]),
        );
    } else {
        filter.insert("generation", captured);
    }
    let result = db
        .collection::<Document>(EPOCH)
        .update_one(filter, doc! {"$inc":{"generation":1_i64}})
        .upsert(captured == 0)
        .session(session)
        .await;
    match result {
        Ok(result) if result.matched_count == 1 || result.upserted_id.is_some() => Ok(next),
        Ok(_) => Err(superseded()),
        // A concurrent initializer already advanced zero. Discard this fetch;
        // do not adopt its generation and replay the captured provider data.
        Err(error) if super::ledger::is_duplicate_key_error(&error) => Err(superseded()),
        Err(error) => Err(error.into()),
    }
}
fn superseded() -> AppError {
    AppError::DatabaseError(mongodb::error::Error::custom(Superseded))
}

pub(super) async fn replace(db: &Database, row: &BillingRateCache) -> AppResult<()> {
    let context = (db.clone(), row.clone());
    transaction_service::run_db_only(db, &context, |session, (db, row)| {
        Box::pin(replace_in_session(db, session, row))
    })
    .await
}
async fn replace_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &BillingRateCache,
) -> AppResult<()> {
    advance_in_session(db, session).await?;
    db.collection::<BillingRateCache>(RATES)
        .replace_one(doc! {"_id":&row.id}, row)
        .upsert(true)
        .session(session)
        .await?;
    Ok(())
}

pub(super) async fn retire(db: &Database, code: &str) -> AppResult<()> {
    let context = (db.clone(), code.to_owned(), bson::DateTime::now());
    transaction_service::run_db_only(db, &context, |session, (db, code, now)| {
        Box::pin(retire_in_session(db, session, code, *now))
    })
    .await
}
async fn retire_in_session(
    db: &Database,
    session: &mut ClientSession,
    code: &str,
    now: bson::DateTime,
) -> AppResult<()> {
    advance_in_session(db, session).await?;
    db.collection::<BillingRateCache>(RATES)
        .update_many(
            doc! {"lago_metric_code":code},
            doc! {"$set":{"retired_at":now}},
        )
        .session(session)
        .await?;
    Ok(())
}

pub(super) struct Chunk<'a> {
    pub captured: i64,
    pub plan: &'a str,
    pub diagnostics: Option<&'a [String]>,
    pub rejected: &'a [String],
    pub rows: &'a [BillingRateCache],
}

/// None discards the rest of this fetch. Transaction retries keep this exact
/// captured generation and payload; only a committed chunk advances the caller.
pub(super) async fn publish_chunk(
    db: &Database,
    chunk: Chunk<'_>,
) -> AppResult<Option<(i64, u64)>> {
    let context = (db.clone(), chunk);
    match transaction_service::run_db_only(db, &context, |session, (db, chunk)| {
        Box::pin(publish_in_session(db, session, chunk))
    })
    .await
    {
        Ok(result) => Ok(Some(result)),
        Err(AppError::DatabaseError(error)) if error.get_custom::<Superseded>().is_some() => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
async fn publish_in_session(
    db: &Database,
    session: &mut ClientSession,
    chunk: &Chunk<'_>,
) -> AppResult<(i64, u64)> {
    let next = advance_captured(db, session, chunk.captured).await?;
    if let Some(rejected) = chunk.diagnostics {
        db.collection::<Document>("billing_rate_diagnostics")
            .update_one(
                doc! {"_id":chunk.plan},
                doc! {"$set":{
                    "rejected_metrics":rejected, "updated_at":bson::DateTime::now(),
                }},
            )
            .upsert(true)
            .session(&mut *session)
            .await?;
    }
    let rates = db.collection::<Document>(RATES);
    if !chunk.rejected.is_empty() {
        rates
            .delete_many(
                doc! {"lago_metric_code":{"$in":chunk.rejected}, "retired_at":bson::Bson::Null},
            )
            .session(&mut *session)
            .await?;
    }
    let mut written = 0;
    for row in chunk.rows {
        let replacement = bson::to_document(row)
            .map_err(|_| AppError::Internal("Rate cache serialization failed".into()))?;
        // A fetch begun AFTER retirement may still observe provider lag. Keep
        // the complete historical row; only an explicit sync may revive it.
        let result = rates.update_one(doc! {"_id":&row.id}, vec![doc! {"$replaceWith":{"$cond":[
            {"$ne":[{"$ifNull":["$retired_at",null]},null]}, "$$ROOT", {"$literal":replacement},
        ]}}]).upsert(true).session(&mut *session).await?;
        written += result.modified_count + u64::from(result.upserted_id.is_some());
    }
    Ok((next, written))
}

#[cfg(test)]
#[path = "rate_epoch_tests.rs"]
mod tests;
