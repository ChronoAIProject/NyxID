//! Metadata-only engine attempts, capacity waiting and durable settlement diagnostics.
use chrono::{DateTime, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde_json::json;

use super::{
    assistant_nyxagent::TurnError,
    audit_service::{self, AuditActor},
};
use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::{ActiveTurn, AssistantConversation, COLLECTION_NAME, TurnOrigin},
        assistant_message::TurnMetadata,
    },
};

use super::assistant_engine_policy as policy;
pub use super::assistant_engine_policy::CAPACITY_ACKNOWLEDGEMENT;

pub fn origin(turn: &ActiveTurn) -> &'static str {
    match turn.origin {
        TurnOrigin::Channel => "channel",
        TurnOrigin::Event => "event",
        TurnOrigin::Trigger => "trigger",
        _ => "chat",
    }
}

pub fn metadata(turn: &ActiveTurn, now: DateTime<Utc>) -> TurnMetadata {
    let active_wait_ms = turn
        .capacity_wait_started_at
        .filter(|_| turn.waiting_for_capacity)
        .map_or(0, |started| {
            (now - started).num_milliseconds().max(0) as u64
        });
    TurnMetadata {
        attempts: Some(turn.engine_attempts),
        capacity_wait_ms: Some(turn.capacity_wait_ms.saturating_add(active_wait_ms)),
    }
}

pub async fn attempt(db: &Database, row: &AssistantConversation) -> AppResult<u32> {
    let turn = row.active_turn.as_ref().expect("claimed turn");
    let saved = db.collection::<bson::Document>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id": &row.id, "user_id": &row.user_id, "active_turn.turn_id": &turn.turn_id, "active_turn.stop_requested": {"$ne": true}},
            doc! {"$inc": {"active_turn.engine_attempts": 1}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .projection(doc! {"active_turn.engine_attempts": 1})
        .await?
        .ok_or_else(|| crate::errors::AppError::Conflict("Assistant turn ended".into()))?;
    Ok(saved
        .get_document("active_turn")
        .ok()
        .and_then(|turn| {
            turn.get_i64("engine_attempts")
                .ok()
                .or_else(|| turn.get_i32("engine_attempts").ok().map(i64::from))
        })
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(1))
}

pub fn accepted<'a>(
    db: &'a Database,
    row: &'a AssistantConversation,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let turn_id = row
            .active_turn
            .as_ref()
            .expect("claimed turn")
            .turn_id
            .clone();
        let accepted_at = bson::DateTime::now();
        // The common acceptance path needs no audit or transaction. Read the
        // persisted wait flag: the executor's conversation snapshot is stale.
        let accepted = doc! {"$set": {
            "active_turn.waiting_for_capacity": false,
            "active_turn.capacity_accepted_at": {"$ifNull": ["$active_turn.capacity_accepted_at", accepted_at]},
            "active_turn.capacity_wait_note_origin": bson::Bson::Null,
        }};
        if db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &row.id, "user_id": &row.user_id, "active_turn.turn_id": &turn_id,
                "$or": [{"active_turn.capacity_wait_started_at": bson::Bson::Null},
                    {"active_turn.waiting_for_capacity": {"$ne": true}}]},
                vec![accepted.clone()],
            )
            .await?
            .matched_count
            == 1
        {
            return Ok(());
        }
        let db = db.clone();
        let row = row.clone();
        let mut session = db.client().start_session().await?;
        session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let Some(saved) = db.collection::<AssistantConversation>(COLLECTION_NAME)
                .find_one_and_update(
                    doc! {"_id": &row.id, "user_id": &row.user_id, "active_turn.turn_id": &turn_id,
                        "active_turn.waiting_for_capacity": true, "active_turn.capacity_wait_started_at": {"$ne": bson::Bson::Null}},
                    vec![doc! {"$set": {"active_turn.capacity_wait_ms": {"$add": [
                        {"$ifNull": ["$active_turn.capacity_wait_ms", 0]},
                        {"$max": [0, {"$subtract": [accepted_at, "$active_turn.capacity_wait_started_at"]}]},
                    ]}}}, accepted.clone()],
                ).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?
            else { return Ok(()); };
            let turn = saved.active_turn.as_ref().expect("matched turn");
            let data = metadata(turn, Utc::now());
            if data.capacity_wait_ms.is_some_and(|ms| ms > 0) {
                audit_service::log_actor_event_in_transaction(&db, session, &actor(&row.user_id), "assistant_turn_capacity_wait", json!({
                    "conversation_id": row.id, "turn_id": turn_id, "waited_ms": data.capacity_wait_ms,
                    "attempts": data.attempts, "origin": origin(turn),
                })).await?;
            }
            Ok(())
        }.await;
        super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)
    })
}

pub async fn failed_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    turn: &ActiveTurn,
    error: &TurnError,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let data = metadata(turn, now);
    audit_service::log_actor_event_in_transaction(db, session, &actor(&row.user_id), "assistant_turn_failed", json!({
        "conversation_id": row.id, "turn_id": turn.turn_id, "code": error.code, "stage": error.stage,
        "upstream_code": error.upstream_code, "http_status": error.http_status, "origin": origin(turn),
        "attempts": data.attempts, "capacity_wait_ms": data.capacity_wait_ms, "trigger_run_id": turn.trigger_run_id,
        "worker_instance": turn.worker_instance, "settled_by_worker_instance": super::assistant_engine_worker::instance(),
        "heartbeat_age_ms": heartbeat_age(turn, Some(error), now),
    })).await
}

pub async fn run_metadata_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    turn: &ActiveTurn,
    data: &TurnMetadata,
) -> AppResult<()> {
    if let Some(run_id) = turn.trigger_run_id.as_ref() {
        db.collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME)
            .update_one(
                doc! {"_id": run_id, "user_id": &row.user_id, "turn_id": &turn.turn_id},
                vec![doc! {"$set": {
                    "attempts": {"$add": [{"$ifNull": ["$attempts", 0]}, i64::from(turn.engine_attempts)]},
                    "capacity_wait_ms": {"$add": [{"$ifNull": ["$capacity_wait_ms", 0]}, data.capacity_wait_ms.unwrap_or(0) as i64]},
                }}],
            )
            .session(session)
            .await?;
    }
    Ok(())
}

fn actor(user_id: &str) -> AuditActor {
    AuditActor {
        user_id: user_id.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}

pub fn settled(
    row: &AssistantConversation,
    turn: &ActiveTurn,
    error: Option<&TurnError>,
    now: DateTime<Utc>,
) {
    let data = metadata(turn, now);
    let worker = super::assistant_engine_worker::instance();
    tracing::info!(target: "nyxid::assistant_engine", conversation_id = %row.id, turn_id = %turn.turn_id,
        worker_instance = %worker.id, worker_hostname = worker.hostname.as_deref(),
        turn_worker_instance = turn.worker_instance.as_ref().map(|worker| worker.id.as_str()),
        turn_worker_hostname = turn.worker_instance.as_ref().and_then(|worker| worker.hostname.as_deref()),
        heartbeat_age_ms = heartbeat_age(turn, error, now),
        code = error.map_or("completed", |e| e.code), stage = error.map(|e| e.stage),
        upstream_code = error.and_then(|e| e.upstream_code.as_deref()), http_status = error.and_then(|e| e.http_status),
        attempts = error.and_then(TurnError::attempts).or(data.attempts),
        capacity_wait_ms = error.and_then(TurnError::capacity_wait_ms).or(data.capacity_wait_ms).unwrap_or(0),
        duration_ms = (now - turn.started_at).num_milliseconds().max(0), origin = origin(turn),
        trigger_run_id = turn.trigger_run_id.as_deref(), "Assistant turn settled");
}

fn heartbeat_age(turn: &ActiveTurn, error: Option<&TurnError>, now: DateTime<Utc>) -> Option<i64> {
    error?;
    turn.heartbeat_at
        .map(|at| (now - at).num_milliseconds().max(0))
}

pub async fn request_started(
    db: &Database,
    row: &AssistantConversation,
    started: DateTime<Utc>,
    deadline: DateTime<Utc>,
) -> AppResult<()> {
    let turn = row.active_turn.as_ref().expect("claimed turn");
    db.collection::<bson::Document>(COLLECTION_NAME).update_one(
        doc! {"_id": &row.id, "active_turn.turn_id": &turn.turn_id, "active_turn.stop_requested": {"$ne": true}},
        vec![doc! {"$set": {
            "active_turn.engine_request_started_at": {"$ifNull": ["$active_turn.engine_request_started_at", bson::DateTime::from_chrono(started)]},
            "active_turn.capacity_wait_deadline": {"$ifNull": ["$active_turn.capacity_wait_deadline", bson::DateTime::from_chrono(deadline)]},
        }}],
    ).await?;
    Ok(())
}

pub async fn acknowledge(
    db: &Database,
    row: &AssistantConversation,
    now: DateTime<Utc>,
) -> AppResult<Option<ActiveTurn>> {
    let turn = row.active_turn.as_ref().expect("claimed turn");
    Ok(db.collection::<AssistantConversation>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id": &row.id, "user_id": &row.user_id, "active_turn.turn_id": &turn.turn_id,
            "active_turn.origin": {"$ne": "trigger"}, "active_turn.trigger_run_id": bson::Bson::Null,
            "active_turn.capacity_accepted_at": bson::Bson::Null, "active_turn.stop_requested": {"$ne": true},
            "active_turn.engine_request_started_at": {"$lte": bson::DateTime::from_chrono(now - chrono::Duration::from_std(policy::ACK_DELAY).expect("bounded acknowledgement delay"))},
            "active_turn.capacity_wait_deadline": {"$gt": bson::DateTime::from_chrono(now)},
        },
        doc! {"$set": {"active_turn.capacity_wait_note_origin": "nyxid_system"}},
    ).return_document(mongodb::options::ReturnDocument::After).await?.and_then(|row| row.active_turn))
}

pub async fn claim_notice(
    db: &Database,
    row: &AssistantConversation,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let turn = row.active_turn.as_ref().expect("claimed turn");
    Ok(db.collection::<AssistantConversation>(COLLECTION_NAME).update_one(
        doc! {"_id": &row.id, "user_id": &row.user_id, "active_turn.turn_id": &turn.turn_id,
            "active_turn.capacity_wait_note_origin": "nyxid_system", "active_turn.stop_requested": {"$ne": true},
            "active_turn.capacity_accepted_at": bson::Bson::Null,
            "active_turn.origin": {"$ne": "trigger"}, "active_turn.trigger_run_id": bson::Bson::Null,
            "active_turn.engine_request_started_at": {"$lte": bson::DateTime::from_chrono(now - chrono::Duration::from_std(policy::ACK_DELAY).expect("bounded acknowledgement delay"))},
            "active_turn.capacity_notice_claimed": {"$ne": true}},
        doc! {"$set": {"active_turn.capacity_notice_claimed": true}},
    ).await?.modified_count == 1)
}

/// Human-readable elapsed time, rounded up to a whole second on every surface.
pub fn wait_duration(wait_ms: u64) -> String {
    let seconds = wait_ms.div_ceil(1_000);
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    let mut parts = Vec::with_capacity(2);
    if minutes > 0 {
        parts.push(format!(
            "{minutes} minute{}",
            if minutes == 1 { "" } else { "s" }
        ));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!(
            "{seconds} second{}",
            if seconds == 1 { "" } else { "s" }
        ));
    }
    parts.join(" ")
}

pub fn capacity_message(wait_ms: Option<u64>) -> String {
    let waited = wait_ms
        .filter(|ms| *ms > 0)
        .map_or_else(String::new, |ms| format!(" (waited {})", wait_duration(ms)));
    format!("NyxBot is at capacity right now{waited}. Try again shortly.")
}
