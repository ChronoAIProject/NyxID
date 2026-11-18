//! A preparation lease proves only that the next text part has not been sent.
//! Images retain their independent barriers. Direct sending parts are never
//! reclaimed; gateway reply-target recovery requires a frozen request.
#[path = "channel_turn_gateway_receipt.rs"]
pub mod gateway;

use crate::{
    errors::{AppError, AppResult},
    models::{
        channel_text_delivery::TextDeliveryNotice,
        channel_turn_preparation::{Preparation, Receipt},
        nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotEvent},
    },
};
use bson::{Document, doc};
use mongodb::{Database, options::ReturnDocument};

pub const PREPARATION_SECS: i64 = 180;
pub const SEND_SECS: i64 = 120;
// Old senders have no expiry and may split a reply across many provider calls.
// This horizon only makes their uncertainty visible; it never permits another send.
pub const LEGACY_UNCONFIRMED_SECS: i64 = 2100;

pub fn due() -> Document {
    doc! {"delivery.version":1, "$or":[
        {"delivery.state":{"$in":["waiting","pending"]}},
        {"delivery.state":"preparing_text","delivery.preparation.version":1,
         "$expr":{"$lte":["$delivery.preparation.lease_until","$$NOW"]}},
        stale_sending(),
    ]}
}
fn stale_sending() -> Document {
    doc! {"delivery.state":"sending", "$expr":{"$cond":[
        {"$eq":["$delivery.preparation.version",1]},
        {"$lte":["$delivery.preparation.lease_until","$$NOW"]},
        {"$and":[
            {"$eq":[{"$type":"$delivery.checked_at"},"date"]},
            {"$lte":["$delivery.checked_at",{"$subtract":["$$NOW",LEGACY_UNCONFIRMED_SECS*1000]}]}
        ]},
    ]}}
}
fn exact(event: &str, claim: &str, state: &str) -> Document {
    doc! {"_id":event,"delivery.version":1,"delivery.state":state,
    "delivery.claim_id":claim,"delivery.preparation.version":1}
}
fn live(event: &str, claim: &str) -> Document {
    let mut filter = exact(event, claim, "preparing_text");
    filter.insert(
        "$expr",
        doc! {"$gt":["$delivery.preparation.lease_until","$$NOW"]},
    );
    filter
}
pub fn lost() -> AppError {
    AppError::Conflict("Channel text preparation changed".into())
}

pub async fn acquire(
    db: &Database,
    event: &str,
    message: &str,
    claim: &str,
) -> AppResult<Option<Preparation>> {
    let row = db
        .collection::<Document>(EVENTS)
        .find_one_and_update(
            doc! {"_id":event,"delivery.version":1,"$or":[
                {"delivery.state":"pending"},
                {"delivery.state":"preparing_text","delivery.preparation.version":1,
                 "delivery.preparation.message_id":message,
                 "$expr":{"$lte":["$delivery.preparation.lease_until","$$NOW"]}},
            ]},
            vec![doc! {"$set":{
                "delivery.state":"preparing_text","delivery.claim_id":{"$literal":claim},
                "delivery.checked_at":"$$NOW","delivery.preparation.version":1,
                "delivery.preparation.message_id":{"$literal":message},
                "delivery.preparation.lease_until":{"$add":["$$NOW",PREPARATION_SECS*1000]},
            }}],
        )
        .return_document(ReturnDocument::After)
        .await?;
    row.map(|row| {
        let metadata = row
            .get_document("delivery")
            .and_then(|d| d.get_document("preparation"))
            .map_err(|_| lost())?;
        bson::from_document(metadata.clone()).map_err(|_| lost())
    })
    .transpose()
}

pub async fn plan(
    db: &Database,
    event: &str,
    claim: &str,
    fingerprint: &str,
    total: u32,
    notice: &TextDeliveryNotice,
) -> AppResult<()> {
    let mut filter = live(event, claim);
    filter.insert("delivery.preparation.fingerprint", bson::Bson::Null);
    let result=db.collection::<Document>(EVENTS).update_one(filter, doc! {"$set":{
        "delivery.preparation.fingerprint":fingerprint,"delivery.preparation.total":total,
        "delivery.preparation.next":0,"delivery.preparation.notice":bson::to_bson(notice).map_err(|_|lost())?,
    }}).await?;
    if result.matched_count == 1 {
        Ok(())
    } else {
        Err(lost())
    }
}

pub async fn before_effect(db: &Database, event: &str, claim: &str, part: u32) -> AppResult<()> {
    // Repeated provider calls inside a part are allowed only for this still-live
    // irreversible claim (for example Telegram's explicit formatting refusal).
    let filter = doc! {"_id":event,"delivery.version":1,"delivery.claim_id":claim,
    "delivery.preparation.version":1,"delivery.preparation.next":part,
    "delivery.state":{"$in":["preparing_text","sending"]},
    "$expr":{"$gt":["$delivery.preparation.lease_until","$$NOW"]}};
    let result=db.collection::<Document>(EVENTS).update_one(filter,vec![doc! {"$set":{
        "delivery.preparation.lease_until":{"$cond":[{"$eq":["$delivery.state","preparing_text"]},
            {"$add":["$$NOW",SEND_SECS*1000]},"$delivery.preparation.lease_until"]},
        "delivery.state":"sending","delivery.checked_at":"$$NOW",
        "delivery.gateway_attempted":{"$cond":[{"$eq":["$delivery.transport","gateway"]},true,"$$REMOVE"]},
    }}]).await?;
    if result.matched_count == 1 {
        Ok(())
    } else {
        Err(lost())
    }
}

pub async fn accepted(
    db: &Database,
    event: &str,
    claim: &str,
    part: u32,
    total: u32,
    receipt: &Receipt,
) -> AppResult<()> {
    let mut filter = exact(event, claim, "sending");
    filter.insert("delivery.preparation.next", part);
    let receipt = bson::to_bson(receipt).map_err(|_| lost())?;
    let result=db.collection::<Document>(EVENTS).update_one(filter,vec![doc! {"$set":{
        "delivery.state":if part+1==total {"sent"} else {"preparing_text"},
        "delivery.checked_at":"$$NOW","delivery.preparation.next":part+1,
        "delivery.preparation.receipts":{"$concatArrays":[{"$ifNull":["$delivery.preparation.receipts",[]]},{"$literal":[receipt]}]},
        "delivery.preparation.lease_until":{"$add":["$$NOW",PREPARATION_SECS*1000]},
    }}]).await?;
    if result.matched_count == 1 {
        Ok(())
    } else {
        Err(lost())
    }
}

pub async fn empty(db: &Database, event: &str, claim: &str, status: &str) -> AppResult<bool> {
    if status == "unknown" {
        return finish_unknown(db, live(event, claim)).await;
    }
    Ok(db
        .collection::<Document>(EVENTS)
        .update_one(live(event, claim), doc! {"$set":{"delivery.state":status}})
        .await?
        .matched_count
        == 1)
}

/// The terminal transition and owner-thread notice commit together. A stale
/// sender cannot finish after this wins; concurrent sweepers append only once.
pub async fn unconfirmed(db: &Database, event: &str, claim: Option<&str>) -> AppResult<bool> {
    let mut filter = match claim {
        Some(claim) => exact(event, claim, "sending"),
        None => stale_sending(),
    };
    filter.insert("_id", event);
    filter.insert("delivery.version", 1);
    finish_unknown(db, filter).await
}

async fn finish_unknown(db: &Database, filter: Document) -> AppResult<bool> {
    use super::api_key_mutation_service as tx;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run((db, filter), |session, (db, filter)| {
            Box::pin(async move {
                let operation: AppResult<bool> = async {
                    let Some(event) = db
                        .collection::<NyxbotEvent>(EVENTS)
                        .find_one_and_update(
                            filter.clone(),
                            doc! {"$set":{"delivery.state":"unknown"}},
                        )
                        .session(&mut *session)
                        .await?
                    else {
                        return Ok(false);
                    };
                    super::channel_reply_notice::late_unconfirmed(db, session, &event).await?;
                    Ok(true)
                }
                .await;
                tx::transaction_result(operation)
            })
        })
        .await
        .map_err(tx::map_transaction_error)
}

tokio::task_local! { static EFFECT: super::channel_platform::PostEffectGuard; }
pub async fn with_effect<F: std::future::Future>(
    guard: super::channel_platform::PostEffectGuard,
    work: F,
) -> F::Output {
    EFFECT.scope(guard, work).await
}
pub async fn effect() -> AppResult<()> {
    if let Ok(guard) = EFFECT.try_with(Clone::clone) {
        #[cfg(test)]
        super::durable_wake_tests::after("late_text_prepared").await;
        guard().await?;
        #[cfg(test)]
        super::durable_wake_tests::after("late_text_dispatch_committed").await;
    }
    Ok(())
}
