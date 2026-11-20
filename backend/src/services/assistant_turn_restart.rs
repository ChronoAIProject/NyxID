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
    models::assistant_conversation::{
        AssistantConversation, COLLECTION_NAME, RestartSource, TeamRestartSource, TurnOrigin,
    },
};

pub(crate) enum Loss<'a> {
    Shutdown(&'a str),
    Stale(DateTime<Utc>),
    Expiry(DateTime<Utc>),
}

fn source_matches(row: &AssistantConversation) -> bool {
    let Some(turn) = &row.active_turn else {
        return false;
    };
    let Some(source) = &turn.restart_admission else {
        return false;
    };
    match (source.version, &source.team_source) {
        (1 | 2, None) => true,
        (3, Some(team))
            if source.source == RestartSource::FirstPartyOwner
                && turn.automation_work.is_none()
                && turn.trigger_run_id.is_none() =>
        {
            match team {
                TeamRestartSource::Assignment { report_to } => {
                    turn.origin == TurnOrigin::Orchestrator
                        && row.is_subagent()
                        && row.group_id.is_none()
                        && turn.report_to.as_ref() == Some(report_to)
                }
                TeamRestartSource::GroupMember {
                    group_id,
                    upper_seq,
                } => {
                    turn.origin == TurnOrigin::Group
                        && turn.group_reply_in_settlement
                        && row.group_id.as_ref() == Some(group_id)
                        && row.group_seen_seq == *upper_seq
                }
            }
        }
        _ => false,
    }
}

pub(crate) fn awaiting_reply(row: &AssistantConversation, turn_id: &str) -> bool {
    row.active_turn.as_ref().is_some_and(|turn| {
        turn.turn_id == turn_id
            && (turn.restart_admission.is_some()
                || super::assistant_nyxagent::live_turn(row, Utc::now()).is_some())
    })
}

pub(crate) async fn confirm_window(
    db: &Database,
    row: &AssistantConversation,
    window_key: &str,
) -> AppResult<bool> {
    let filter = WorkerFence::matching(
        row,
        doc! {
            "credential_api_key_id": &row.credential_api_key_id,
            "active_turn.restart_admission.window.key": window_key,
        },
    )?;
    Ok(db
        .collection::<AssistantConversation>(COLLECTION_NAME)
        .update_one(
            filter,
            doc! {"$set": {
                "active_turn.restart_admission.window.terminal": true,
                "active_turn.running_response": bson::Bson::Null,
            }},
        )
        .write_concern(WriteConcern::majority())
        .await?
        .matched_count
        == 1)
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
    if !source_matches(row) || source.reattach.is_none() || turn.stop_requested {
        return Ok(None);
    }
    let Some(next_epoch) = previous.epoch.checked_add(1) else {
        return Ok(None);
    };
    let mut filter = WorkerFence::capture(row)?.filter();
    filter.extend(
        doc! {"active_turn.restart_admission.version": source.version,
        "active_turn.stop_requested": false, "credential_api_key_id": &source.credential_id },
    );
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
        "active_turn.restart_offer": {"from_worker": &worker.id, "generation": &source.generation, "reason": reason, "offered_at": now, "due_at": now},
        "active_turn.execution_attempt": {"epoch": next_epoch, "claim_id": Uuid::new_v4().to_string()},
    }};
    Ok(Some((filter, update)))
}

pub(crate) fn offer(
    db: &Database,
    row: &AssistantConversation,
    loss: Loss<'_>,
) -> futures::future::BoxFuture<'static, AppResult<bool>> {
    let change = offer_change(row, loss);
    let db = db.clone();
    Box::pin(async move {
        let Some((filter, update)) = change? else {
            return Ok(false);
        };
        let context = (db.clone(), filter, update);
        super::transaction_service::run_db_only(&db, &context, |session, (db, filter, update)| {
            Box::pin(save_offer(db, session, filter.clone(), update.clone()))
        })
        .await
    })
}

pub(crate) async fn audit(
    db: &Database,
    session: &mut mongodb::ClientSession,
    owner: &str,
    turn_id: &str,
    offer: &crate::models::assistant_conversation::RestartOffer,
    transition: &str,
    outcome: &str,
) -> AppResult<()> {
    let actor = super::audit_service::AuditActor {
        user_id: owner.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    let metadata = serde_json::json!({
        "turn_id": turn_id, "generation": offer.generation, "from_worker": offer.from_worker,
        "reason": offer.reason, "transition": transition, "outcome": outcome,
    });
    super::audit_service::log_actor_event_in_transaction(
        db,
        session,
        &actor,
        "assistant_turn_handoff",
        metadata,
    )
    .await
}

async fn save_offer(
    db: &Database,
    session: &mut mongodb::ClientSession,
    filter: bson::Document,
    update: bson::Document,
) -> AppResult<bool> {
    let Some(row) = db
        .collection::<AssistantConversation>(COLLECTION_NAME)
        .find_one_and_update(filter, update)
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *session)
        .await?
    else {
        return Ok(false);
    };
    let turn = row
        .active_turn
        .ok_or(crate::errors::AppError::AssistantTurnRequired)?;
    audit(
        db,
        session,
        &row.user_id,
        &turn.turn_id,
        turn.restart_offer
            .as_ref()
            .ok_or(crate::errors::AppError::AssistantTurnRequired)?,
        "offered",
        "pending",
    )
    .await?;
    Ok(true)
}

pub(crate) async fn offer_expired_in_session(
    db: &Database,
    row: &AssistantConversation,
    session: &mut mongodb::ClientSession,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if let Some((filter, update)) = offer_change(row, Loss::Expiry(now))? {
        save_offer(db, session, filter, update).await?;
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
    let context = (db.clone(), offered.clone(), survivor.clone());
    super::transaction_service::run_db_only(db, &context, |session, (db, offered, survivor)| {
        Box::pin(claim_capacity(db, offered, survivor, None, session))
    })
    .await
}

pub(crate) async fn claim_capacity(
    db: &Database,
    offered: &AssistantConversation,
    survivor: &crate::models::assistant_conversation::EngineWorkerInstance,
    capacity: Option<&crate::models::coordination::SlotToken>,
    session: &mut mongodb::ClientSession,
) -> AppResult<Option<AssistantConversation>> {
    let Some(turn) = &offered.active_turn else {
        return Ok(None);
    };
    let Some(offer) = &turn.restart_offer else {
        return Ok(None);
    };
    if offer.claimed_by.is_some() || offer.from_worker == survivor.id || !source_matches(offered) {
        return Ok(None);
    }
    let mut filter = WorkerFence::capture(offered)?.filter();
    let source = turn.restart_admission.as_deref().expect("validated source");
    filter.insert(
        "active_turn.restart_admission.team_source",
        bson::to_bson(&source.team_source)
            .map_err(|err| crate::errors::AppError::Internal(err.to_string()))?,
    );
    if let Some(team) = &source.team_source {
        filter.insert("agent_id", &source.agent_id);
        filter.insert("model", &source.model);
        if !source.credential_ended {
            filter.insert("credential_api_key_id", &source.credential_id);
        }
        match team {
            TeamRestartSource::Assignment { report_to } => {
                filter.insert("active_turn.report_to", report_to);
            }
            TeamRestartSource::GroupMember {
                group_id,
                upper_seq,
            } => {
                filter.insert("group_id", group_id);
                filter.insert("group_seen_seq", *upper_seq);
                filter.insert("active_turn.group_reply_in_settlement", true);
            }
        }
    }
    filter.extend(doc!{"active_turn.restart_admission.version":source.version, "active_turn.restart_offer.from_worker":&offer.from_worker,
        "active_turn.restart_offer.claimed_by":null, "active_turn.stop_requested":false,
        "active_turn.restart_offer.due_at":{"$lte":bson::DateTime::now()}});
    let mut fields = doc! {"active_turn.restart_offer.claimed_by": &survivor.id,
    "active_turn.restart_admission.continued_after_restart":true,
    "active_turn.restart_admission.generation":super::security_readiness_service::local_generation(),
    "active_turn.worker_instance": {"id":&survivor.id, "hostname":&survivor.hostname},
    "active_turn.execution_attempt.claim_id":Uuid::new_v4().to_string(), "active_turn.heartbeat_at":bson::DateTime::now()};
    if let Some(capacity) = capacity {
        fields.insert(
            "active_turn.restart_admission.capacity",
            bson::to_bson(capacity)
                .map_err(|error| crate::errors::AppError::Internal(error.to_string()))?,
        );
    }
    let collection = db.collection::<AssistantConversation>(COLLECTION_NAME);
    let change = collection
        .find_one_and_update(filter, doc! {"$set":fields})
        .return_document(mongodb::options::ReturnDocument::After);
    let claimed = change.session(&mut *session).await?;
    if let Some(active) = claimed.as_ref().and_then(|row| row.active_turn.as_ref()) {
        if let (Some(source), Some(attempt)) =
            (&active.restart_admission, &active.execution_attempt)
        {
            db.collection::<bson::Document>(super::assistant_service_execution::COLLECTION)
                .update_one(doc! {"_id":&source.reply_id,"user_id":&offered.user_id,"conversation_id":&offered.id,"turn_id":&turn.turn_id,"kind":"model_attempt"},
                    doc! {"$set":{"active":true,"claim_id":&attempt.claim_id,"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(35))}})
                .session(&mut *session).await?;
        }
        audit(
            db,
            session,
            &offered.user_id,
            &turn.turn_id,
            offer,
            "claimed",
            "running",
        )
        .await?;
    }
    Ok(claimed)
}

pub(crate) async fn due(db: &Database, worker: &str) -> AppResult<Vec<AssistantConversation>> {
    use futures::TryStreamExt;
    Ok(db.collection::<AssistantConversation>(COLLECTION_NAME).find(doc! {
        "active_turn.restart_admission.version":{"$in":[1,2,3]}, "active_turn.restart_offer.due_at":{"$type":"date","$lte":bson::DateTime::now()},
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
