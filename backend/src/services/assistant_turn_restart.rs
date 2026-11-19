use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
    options::WriteConcern,
};
use uuid::Uuid;

use super::assistant_nyxagent::WorkerFence;
use crate::{
    errors::AppResult,
    models::assistant_conversation::{AssistantConversation, COLLECTION_NAME},
};

pub(crate) enum Loss<'a> {
    Shutdown(&'a str),
    Stale(DateTime<Utc>),
    Expiry(DateTime<Utc>),
}

pub(crate) fn owns_subscriber(row: &AssistantConversation, worker: &str) -> bool {
    let Some(turn) = row
        .active_turn
        .as_ref()
        .filter(|turn| turn.restart_admission.is_some())
    else {
        return true;
    };
    turn.worker_instance
        .as_ref()
        .is_some_and(|owner| owner.id == worker)
        && turn
            .restart_offer
            .as_ref()
            .is_none_or(|offer| offer.claimed_by.as_deref() == Some(worker))
}

pub(crate) fn absence_retry_before(
    admitted_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    retention_seconds: u32,
) -> Option<DateTime<Utc>> {
    let usable = retention_seconds.checked_sub(300)?;
    let cutoff = admitted_at.checked_add_signed(chrono::Duration::seconds(i64::from(usable)))?;
    (admitted_at <= observed_at && observed_at < cutoff).then_some(cutoff)
}

fn offer_change(
    row: &AssistantConversation,
    loss: Loss<'_>,
) -> AppResult<Option<(bson::Document, bson::Document)>> {
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(None);
    };
    let (Some(source), Some(previous), Some(worker)) = (
        &turn.restart_admission,
        &turn.execution_attempt,
        &turn.worker_instance,
    ) else {
        return Ok(None);
    };
    if source.version != 1 || source.reattach.is_none() || turn.stop_requested {
        return Ok(None);
    }
    let Some(next_epoch) = previous.epoch.checked_add(1) else {
        return Ok(None);
    };
    let mut filter = WorkerFence::capture(row)?.filter();
    filter.extend(doc! {"active_turn.restart_admission.version": 1,
    "active_turn.stop_requested": false, "credential_api_key_id": &source.credential_id });
    match &turn.restart_offer {
        None => filter.insert("active_turn.restart_offer", bson::Bson::Null),
        Some(offer) if offer.claimed_by.as_ref() == Some(&worker.id) => {
            filter.insert("active_turn.restart_offer.claimed_by", &worker.id)
        }
        Some(_) => return Ok(None),
    };
    let reason = match loss {
        Loss::Shutdown(exiting) if worker.id == exiting => "shutdown",
        Loss::Shutdown(_) => return Ok(None),
        Loss::Stale(before) => {
            filter.insert(
                "active_turn.heartbeat_at",
                doc! {"$type":"date", "$lte": bson::DateTime::from_chrono(before)},
            );
            "sweep"
        }
        Loss::Expiry(now) => {
            if super::assistant_nyxagent::live_turn(row, now).is_some() {
                return Ok(None);
            }
            filter.insert(
                "active_turn.lease_expires_at",
                turn.lease_expires_at.map(bson::DateTime::from_chrono),
            );
            "lease_expiry"
        }
    };
    let now = bson::DateTime::now();
    let update = doc! {"$set": {
        "active_turn.restart_offer": {"from_worker": &worker.id, "reason": reason, "offered_at": now, "due_at": now},
        "active_turn.execution_attempt": {"epoch": next_epoch, "claim_id": Uuid::new_v4().to_string()},
    }};
    Ok(Some((filter, update)))
}

pub(crate) async fn offer(
    db: &Database,
    row: &AssistantConversation,
    loss: Loss<'_>,
) -> AppResult<bool> {
    let Some((filter, update)) = offer_change(row, loss)? else {
        return Ok(false);
    };
    Ok(db
        .collection::<AssistantConversation>(COLLECTION_NAME)
        .update_one(filter, update)
        .write_concern(WriteConcern::majority())
        .await?
        .matched_count
        == 1)
}

pub(crate) async fn offer_expired_in_session(
    db: &Database,
    row: &AssistantConversation,
    session: &mut mongodb::ClientSession,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if let Some((filter, update)) = offer_change(row, Loss::Expiry(now))? {
        db.collection::<AssistantConversation>(COLLECTION_NAME)
            .update_one(filter, update)
            .session(session)
            .await?;
    }
    Ok(())
}

pub(crate) async fn yield_on_shutdown(
    db: &Database,
    row: &AssistantConversation,
    worker: &str,
) -> AppResult<bool> {
    if offer(db, row, Loss::Shutdown(worker)).await? {
        return Ok(true);
    }
    let turns = db.collection::<bson::Document>(COLLECTION_NAME);
    if turns
        .find_one(WorkerFence::capture(row)?.filter())
        .projection(doc! {"_id":1})
        .await?
        .is_some()
    {
        return Ok(false);
    }
    let Some(turn) = &row.active_turn else {
        return Ok(true);
    };
    let (Some(source), Some(attempt)) = (&turn.restart_admission, &turn.execution_attempt) else {
        return Ok(true);
    };
    let saved = db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(
            doc! {"_id":&source.reply_id, "user_id":&row.user_id, "conversation_id":&row.id,
            "turn_id":&turn.turn_id, "settled_by_attempt.epoch":attempt.epoch,
            "settled_by_attempt.claim_id":&attempt.claim_id},
        )
        .projection(doc! {"_id":1})
        .await?;
    Ok(saved.is_none())
}

pub(crate) async fn claim(
    db: &Database,
    offered: &AssistantConversation,
    survivor: &crate::models::assistant_conversation::EngineWorkerInstance,
) -> AppResult<Option<AssistantConversation>> {
    let Some(turn) = &offered.active_turn else {
        return Ok(None);
    };
    let Some(offer) = &turn.restart_offer else {
        return Ok(None);
    };
    if offer.claimed_by.is_some() || offer.from_worker == survivor.id {
        return Ok(None);
    }
    let mut filter = WorkerFence::capture(offered)?.filter();
    filter.extend(doc!{"active_turn.restart_admission.version":1, "active_turn.restart_offer.from_worker":&offer.from_worker,
        "active_turn.restart_offer.claimed_by":null, "active_turn.stop_requested":false,
        "active_turn.restart_offer.due_at":{"$lte":bson::DateTime::now()}});
    Ok(db.collection::<AssistantConversation>(COLLECTION_NAME).find_one_and_update(filter,
        doc!{"$set": {"active_turn.restart_offer.claimed_by": &survivor.id,
            "active_turn.worker_instance": {"id":&survivor.id, "hostname":&survivor.hostname},
            "active_turn.execution_attempt.claim_id":Uuid::new_v4().to_string(), "active_turn.heartbeat_at":bson::DateTime::now()}})
        .write_concern(WriteConcern::majority()).return_document(mongodb::options::ReturnDocument::After).await?)
}

pub(crate) async fn due(db: &Database, worker: &str) -> AppResult<Vec<AssistantConversation>> {
    use futures::TryStreamExt;
    Ok(db.collection::<AssistantConversation>(COLLECTION_NAME).find(doc! {
        "active_turn.restart_admission.version":1, "active_turn.restart_offer.due_at":{"$type":"date","$lte":bson::DateTime::now()},
        "active_turn.restart_offer.claimed_by":null,"active_turn.restart_offer.from_worker":{"$ne":worker}, "active_turn.stop_requested":false
    }).sort(doc!{"active_turn.restart_offer.due_at":1,"_id":1}).limit(32).await?.try_collect().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absence_evidence_ages_from_admission_and_excludes_the_cutoff() {
        let admitted = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let retention = 7 * 24 * 3600;
        let cutoff = admitted + chrono::Duration::seconds(i64::from(retention) - 300);
        let tick = chrono::Duration::milliseconds(1);
        assert_eq!(
            absence_retry_before(admitted, admitted, retention),
            Some(cutoff)
        );
        assert_eq!(
            absence_retry_before(admitted, cutoff - tick, retention),
            Some(cutoff)
        );
        for observed in [admitted - tick, cutoff, cutoff + tick] {
            assert_eq!(absence_retry_before(admitted, observed, retention), None);
        }
        for short in [0, 299, 300] {
            assert_eq!(absence_retry_before(admitted, admitted, short), None);
        }
    }
}
