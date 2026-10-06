//! Durable arbitration between the provider stream and asynchronous channel replies.
//! Never reclaim a dispatch barrier: an interrupted external send is ambiguous.
use bson::doc;
use chrono::Utc;
use mongodb::{ClientSession, Database};

use crate::{
    errors::{AppError, AppResult},
    models::nyxbot_channel::EVENTS_COLLECTION_NAME as EVENTS,
};

pub async fn bind_in_session(
    db: &Database,
    session: &mut ClientSession,
    event: &str,
    owner: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<()> {
    let bound = db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "user_id": owner, "status": "running",
            "turn_id": bson::Bson::Null, "delivery.version": 1},
            doc! {"$set": {"conversation_id": conversation, "turn_id": turn}},
        )
        .session(session)
        .await?;
    if bound.matched_count != 1 {
        return Err(AppError::Conflict("Channel event already admitted".into()));
    }
    Ok(())
}

pub async fn detach(db: &Database, event: &str) -> AppResult<()> {
    db.collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "waiting"},
            doc! {"$set": {"delivery.state": "pending"}},
        )
        .await?;
    Ok(())
}

/// The write precedes the first committed message frame. Once claimed, a lost
/// stream cannot safely be retried without a gateway delivery acknowledgement.
pub async fn claim_stream(db: &Database, event: &str, status: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "waiting",
            "delivery.stream_deadline": {"$gt": bson::DateTime::from_chrono(Utc::now())}},
            doc! {"$set": {"delivery.state": "streamed", "status": status}},
        )
        .await?
        .modified_count
        == 1)
}

/// One irreversible outbound attempt, shared by the callback and all sweepers.
pub async fn claim_send(db: &Database, event: &str, claim: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "pending"},
            doc! {"$set": {"delivery.state": "sending", "delivery.claim_id": claim}},
        )
        .await?
        .modified_count
        == 1)
}

pub async fn finish(db: &Database, event: &str, claim: &str, status: &str) -> AppResult<()> {
    db.collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.state": "sending", "delivery.claim_id": claim},
            doc! {"$set": {"delivery.state": status}},
        )
        .await?;
    Ok(())
}
