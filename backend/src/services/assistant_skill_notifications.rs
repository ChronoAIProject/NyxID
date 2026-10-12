//! Durable delivery of owner-allowed skill requests to NyxBot.
use super::*;
use crate::models::assistant_conversation::AgentEvent;

/// Bounded retry work for the existing assistant sweep. Old rows without an
/// outbox marker retain their historical behavior.
pub async fn pending(db: &Database) -> AppResult<Vec<AssistantAcknowledgement>> {
    let rows: Vec<AssistantAcknowledgement> = db.collection::<AssistantAcknowledgement>(ACKS)
        .find(doc! {"skill_notification_pending": true, "kind": "skills", "status": "allowed", "decided_by": "user",
            "$or": [{"skill_notification_retry_after": {"$exists": false}},
                {"skill_notification_retry_after": {"$lte": bson::DateTime::now()}}]})
        .sort(doc! {"skill_notification_retry_at": 1, "_id": 1}).limit(100)
        .await?.try_collect().await?;
    // Rotate attempted work even when routing fails or its thread is parked.
    // A crash after this write delays recovery by one pass, never loses work.
    if !rows.is_empty() {
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        db.collection::<bson::Document>(ACKS)
            .update_many(
                doc! {"_id": {"$in": ids}, "skill_notification_pending": true},
                doc! {"$set": {"skill_notification_retry_at": bson::DateTime::now()}},
            )
            .await?;
    }
    Ok(rows)
}

/// A private, fenced delivery snapshot. No notification metadata is exposed
/// through the acknowledgement API.
#[derive(Clone)]
pub(crate) struct DeliveryAttempt {
    fence: i64,
    target: Option<String>,
    failures: i32,
}

impl DeliveryAttempt {
    pub(crate) fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    fn filter(&self, request: &AssistantAcknowledgement) -> bson::Document {
        let fence = if self.fence == 0 {
            doc! {"$or": [{"skill_notification_fence": 0_i64},
            {"skill_notification_fence": {"$exists": false}}]}
        } else {
            doc! {"skill_notification_fence": self.fence}
        };
        doc! {"_id": &request.id, "user_id": &request.user_id,
        "kind": "skills", "status": "allowed", "decided_by": "user",
        "skill_notification_pending": true,
        "skill_notification_target": self.target.as_deref().map(bson::Bson::from).unwrap_or(bson::Bson::Null),
        "$and": [fence, doc! {"$or": [
            {"skill_notification_retry_after": {"$exists": false}},
            {"skill_notification_retry_after": {"$lte": bson::DateTime::now()}}]}]}
    }
}

pub(crate) async fn attempt(
    db: &Database,
    request: &AssistantAcknowledgement,
) -> AppResult<Option<DeliveryAttempt>> {
    let Some(row) = db
        .collection::<bson::Document>(ACKS)
        .find_one(doc! {
            "_id": &request.id, "user_id": &request.user_id, "kind": "skills",
            "status": "allowed", "decided_by": "user", "skill_notification_pending": true
        })
        .await?
    else {
        return Ok(None);
    };
    if row
        .get_datetime("skill_notification_retry_after")
        .is_ok_and(|at| *at > bson::DateTime::now())
    {
        return Ok(None);
    }
    Ok(Some(DeliveryAttempt {
        fence: row
            .get_i64("skill_notification_fence")
            .or_else(|_| row.get_i32("skill_notification_fence").map(i64::from))
            .unwrap_or(0),
        target: row
            .get_str("skill_notification_target")
            .ok()
            .map(str::to_owned),
        failures: row
            .get_i32("skill_notification_authority_failures")
            .unwrap_or(0)
            .clamp(0, 7),
    }))
}

/// Retain decisions when live authority is unavailable. A newer enqueue or
/// consumption wins over this stale failure through the same marker fence.
pub(crate) async fn defer_authority(
    db: &Database,
    request: &AssistantAcknowledgement,
    attempt: &DeliveryAttempt,
    reason: &'static str,
) -> AppResult<()> {
    let failures = attempt.failures.saturating_add(1).min(7);
    let seconds = [60, 120, 240, 480, 960, 1920, 3600][(failures - 1) as usize];
    db.collection::<bson::Document>(ACKS).update_one(attempt.filter(request), doc! {
        "$inc": {"skill_notification_fence": 1_i64},
        "$set": {"skill_notification_authority_reason": reason,
            "skill_notification_authority_failures": failures,
            "skill_notification_retry_after": bson::DateTime::from_chrono(Utc::now() + Duration::seconds(seconds))}
    }).await?;
    Ok(())
}

/// Keep the marker until a turn consumes the event. The bounded general
/// event queue may evict old entries; the next sweep then restores them.
/// Concurrent delivery and replay never duplicate an event already queued.
pub(crate) async fn enqueue(
    db: &Database,
    request: &AssistantAcknowledgement,
    target: &str,
    attempt: &DeliveryAttempt,
    event: AgentEvent,
) -> AppResult<Option<String>> {
    let db = db.clone();
    let owner = request.user_id.clone();
    let request_id = request.id.clone();
    let filter = attempt.filter(request);
    let observed_target = attempt.target.clone();
    let target = target.to_owned();
    let entry = bson::to_bson(&event)
        .map_err(|_| AppError::Internal("Failed to encode skill event".into()))?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let operation = Box::pin(async {
            // The handler authorized this exact destination from the snapshot.
            // Migration is allowed only while the old destination is deleted.
            if let Some(stored) = observed_target.as_deref()
                && stored != target
                && db.collection::<bson::Document>(CONVERSATIONS)
                    .find_one(doc! {"_id": stored, "user_id": &owner})
                    .session(&mut *session).await?.is_some() {
                return Ok(None);
            }
            let Some(_) = db.collection::<bson::Document>(ACKS)
                .find_one_and_update(filter.clone(), doc! {
                    "$inc": {"skill_notification_fence": 1_i64},
                    "$set": {"skill_notification_target": &target},
                    "$unset": {"skill_notification_authority_reason": "",
                        "skill_notification_authority_failures": "", "skill_notification_retry_after": ""}
                }).session(&mut *session).await? else { return Ok(None); };
            let already_queued = db.collection::<bson::Document>(CONVERSATIONS)
                .find_one(doc! {"_id": &target, "user_id": &owner, "role": "orchestrator", "pending_events.id": &request_id})
                .session(&mut *session).await?;
            if already_queued.is_some() { return Ok(Some(target.clone())); }
            let queued = db.collection::<bson::Document>(CONVERSATIONS)
                .update_one(doc! {"_id": &target, "user_id": &owner, "role": "orchestrator"},
                    doc! {"$push": {"pending_events": {"$each": [&entry], "$slice": -(super::super::assistant_nyxagent::MAX_PENDING_EVENTS as i64)}}})
                .session(&mut *session).await?;
            if queued.matched_count != 1 { return Err(not_found()); }
            Ok(Some(target.clone()))
        }).await;
        mutations::transaction_result(operation)
    }).await.map_err(mutations::map_transaction_error)
}

/// Consumption and turn creation share one transaction, so a crash between
/// queueing and wake-up, or queue overflow, cannot lose a saved owner decision.
pub async fn consume_in_session(
    db: &Database,
    owner: &str,
    events: &[AgentEvent],
    session: &mut ClientSession,
) -> AppResult<()> {
    let ids: Vec<&str> = events
        .iter()
        .filter(|event| event.kind == "skill_request_allowed")
        .map(|event| event.id.as_str())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    db.collection::<bson::Document>(ACKS)
        .update_many(
            doc! {"_id": {"$in": ids}, "user_id": owner, "kind": "skills",
            "status": "allowed", "decided_by": "user", "skill_notification_pending": true},
            doc! {"$unset": {"skill_notification_pending": "",
            "skill_notification_authority_reason": "", "skill_notification_authority_failures": "",
            "skill_notification_retry_after": ""}},
        )
        .session(session)
        .await?;
    Ok(())
}
