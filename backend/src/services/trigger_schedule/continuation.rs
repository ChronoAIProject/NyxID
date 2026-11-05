//! Per-run coordinator enrollment and completion. This module never grants
//! execution authority and never reads process environment or migration state.
use super::*;
use crate::models::{
    assistant_conversation::COLLECTION_NAME as THREADS,
    assistant_direct_handoff::COLLECTION_NAME as HANDOFFS,
    assistant_group::COLLECTION_NAME as GROUPS,
    trigger_run::{AutomationWork, CoordinatorState},
};

pub const CONTINUATION_LIMIT: u32 = 32;
pub const DISPATCH_LIMIT: u32 = 32;
pub const COORDINATOR_DEADLINE_SECONDS: i64 = 60 * 60;
pub const ADMISSION_LEASE_SECONDS: i64 = 120;
pub const ADMISSION_ATTEMPT_SECONDS: u64 = 30;

pub async fn enrollment_ready(db: &Database, configured: bool) -> bool {
    if !configured {
        return false;
    }
    super::super::security_readiness_service::report(
        db,
        super::super::security_readiness_service::AUTOMATION_COMPLETION,
    )
    .await
    .is_ok_and(|report| report.ready)
}

pub fn state(now: DateTime<Utc>, expires_at: DateTime<Utc>) -> CoordinatorState {
    CoordinatorState {
        protocol: 2,
        deadline: (now + Duration::seconds(COORDINATOR_DEADLINE_SECONDS)).min(expires_at),
        continuations: 0,
        dispatches: 0,
        closed: false,
        version: 0,
        wait_kind: None,
    }
}

pub fn context(run: &TriggerRun, thread: &str) -> AutomationWork {
    AutomationWork {
        run_id: run.id.clone(),
        work_id: run.id.clone(),
        coordinator_thread_id: thread.into(),
        role: "coordinator".into(),
        confirmation_policy: run.confirmation_policy,
    }
}

pub fn expired(run: &TriggerRun, now: DateTime<Utc>) -> bool {
    run.coordinator
        .as_ref()
        .is_some_and(|c| c.protocol == 2 && c.deadline <= now)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Barrier {
    Waiting,
    ContinueTerminal,
    Superseded,
}

/// Registration and settlement both perform real thread -> run writes. An
/// unrelated active owner turn must survive and cannot block this barrier.
pub async fn barrier(db: &Database, run: &TriggerRun) -> AppResult<Barrier> {
    let Some(protocol) = run.coordinator.as_ref() else {
        return Ok(Barrier::ContinueTerminal);
    };
    if protocol.protocol != 2 {
        return Err(AppError::Conflict(
            "coordinator_protocol_unsupported".into(),
        ));
    }
    let ctx = (db.clone(), run.clone());
    super::super::transaction_service::run_db_only(db, &ctx, |session, (db, run)| {
        Box::pin(async move {
            let Some(live) = db.collection::<TriggerRun>(RUNS)
                .find_one(doc! {"_id": &run.id, "user_id": &run.user_id,
                    "turn_id": &run.turn_id, "coordinator.protocol": 2})
                .session(&mut *session).await? else { return Ok(Barrier::Superseded); };
            if live.outcome == RunOutcome::Waiting {
                return Ok(if live.coordinator.as_ref().and_then(|c| c.wait_kind.as_deref()) == Some("coordinator") {
                    Barrier::Waiting
                } else { Barrier::Superseded });
            }
            if live.outcome != RunOutcome::Started { return Ok(Barrier::Superseded); }
            // Recovery after terminal_result must reach main's result-claim path.
            if live.terminal_result_recorded || live.result_claimed {
                return Ok(Barrier::ContinueTerminal);
            }
            let thread = live.thread_id.as_deref().ok_or_else(|| AppError::Conflict("trigger_thread_missing".into()))?;
            if db.collection::<Document>(THREADS).update_one(
                doc! {"_id": thread, "user_id": &live.user_id},
                doc! {"$inc": {"automation_handoff_version": 1_i64}},
            ).session(&mut *session).await?.matched_count != 1 {
                return Err(AppError::Conflict("trigger_thread_changed".into()));
            }
            db.collection::<Document>(RUNS).update_one(
                doc! {"_id": &live.id},
                doc! {"$inc": {"coordinator.version": 1_i64}, "$set": {"coordinator.closed": true}},
            ).session(&mut *session).await?;
            if !has_source(db, &live, session).await? { return Ok(Barrier::ContinueTerminal); }
            db.collection::<Document>(RUNS).update_one(doc! {"_id": &live.id},
                doc! {"$set": {"outcome": "waiting", "reason": "coordinator_pending", "coordinator.wait_kind": "coordinator", "waiting_for": []}},
            ).session(&mut *session).await?;
            wake_in_session(db, &live.id, session).await?;
            audit_in_session(db, &live, "trigger_run_waiting", "coordinator_pending", session).await?;
            Ok(Barrier::Waiting)
        })
    }).await
}

pub async fn has_source(
    db: &Database,
    run: &TriggerRun,
    session: &mut ClientSession,
) -> AppResult<bool> {
    if let Some(thread) = run.thread_id.as_deref()
        && db
            .collection::<Document>(THREADS)
            .find_one(doc! {
                "_id": thread, "user_id": &run.user_id,
                "pending_events.automation_work.run_id": &run.id,
            })
            .projection(doc! {"_id": 1})
            .session(&mut *session)
            .await?
            .is_some()
    {
        return Ok(true);
    }
    if db
        .collection::<Document>(HANDOFFS)
        .find_one(doc! {
            "user_id": &run.user_id, "run_id": &run.id, "state": "pending",
        })
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    Ok(db
        .collection::<Document>(GROUPS)
        .find_one(doc! {
            "user_id": &run.user_id, "followers.automation.work.run_id": &run.id,
        })
        .projection(doc! {"_id": 1})
        .session(session)
        .await?
        .is_some())
}

/// An event cannot steal a live admission lease. The claimant compares the
/// generation when deferring so it cannot overwrite a concurrently queued wake.
pub async fn wake_in_session(
    db: &Database,
    run: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(WORK).update_one(doc! {"_id": run}, vec![doc! {"$set": {
        "automation_v2": true,
        "wake_generation": {"$add": [{"$ifNull": ["$wake_generation", 0]}, 1]},
        "at": {"$cond": [{"$gt": ["$admission_lease_until", date(Utc::now())]}, "$at", date(Utc::now())]},
    }}]).session(session).await?;
    Ok(())
}

/// Extend the exact claimed job before bounded v2 admission, including its
/// first enrollment. The run remains legacy until the admission commits.
pub async fn reserve_admission(db: &Database, job: &Document) -> AppResult<()> {
    let now = Utc::now();
    let changed = db
        .collection::<Document>(WORK)
        .update_one(
            doc! {
                "_id": job.get_str("_id").unwrap_or_default(),
                "fence": job.get_str("fence").unwrap_or_default(), "at": {"$gt": date(now)},
            },
            doc! {"$set": {"at": date(now + Duration::seconds(ADMISSION_LEASE_SECONDS))}},
        )
        .await?
        .matched_count;
    if changed != 1 {
        return Err(AppError::Conflict("trigger_lease_lost".into()));
    }
    Ok(())
}

pub fn backstop(deferrals: i32) -> Duration {
    Duration::seconds((60_i64 << deferrals.clamp(0, 3)).min(300))
}

/// Only a verified live source turn can add work. The run-wide counter counts
/// accepted dispatches, including nested direct and group handoffs, exactly once.
pub async fn register_in_session(
    db: &Database,
    owner: &str,
    thread: &str,
    turn: &str,
    work: &AutomationWork,
    count: u32,
    session: &mut ClientSession,
) -> AppResult<TriggerRun> {
    let matched = db.collection::<Document>(THREADS).update_one(doc! {
        "_id": thread, "user_id": owner, "active_turn.turn_id": turn,
        "active_turn.stop_requested": {"$ne": true},
        "active_turn.automation_work": bson::to_bson(work).map_err(|_| AppError::Internal("Encode automation work".into()))?,
    }, doc! {"$inc": {"automation_handoff_version": 1_i64}}).session(&mut *session).await?.matched_count;
    if matched != 1 {
        return Err(AppError::AssistantTurnRequired);
    }
    let mut filter = doc! {"_id": &work.run_id, "user_id": owner, "coordinator.protocol": 2,
    "coordinator.deadline": {"$gt": date(Utc::now())},
    "coordinator.dispatches": {"$lte": i64::from(DISPATCH_LIMIT.saturating_sub(count))},
    "outcome": {"$in": ["started", "waiting"]}};
    if count > DISPATCH_LIMIT {
        return Err(AppError::Conflict("automation_dispatch_limit".into()));
    }
    if work.role == "coordinator" {
        filter.insert("turn_id", turn);
        filter.insert("coordinator.closed", false);
    }
    db.collection::<TriggerRun>(RUNS).find_one_and_update(filter,
        doc! {"$inc": {"coordinator.version": 1_i64, "coordinator.dispatches": i64::from(count)}})
        .return_document(ReturnDocument::After).session(session).await?
        .ok_or_else(|| AppError::Conflict("automation_dispatch_limit_or_run_closed".into()))
}

pub async fn has_queued_result(db: &Database, run: &TriggerRun) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(THREADS)
        .find_one(doc! {
            "_id": &run.thread_id, "user_id": &run.user_id,
            "pending_events.automation_work.run_id": &run.id,
        })
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

/// Human Cancel and hard expiry share one exact-run transaction. Disabling new
/// admission does not disable this drain path. Repeated owned terminal cancels
/// are idempotent; cross-owner/trigger identities are not found.
pub async fn cancel(
    db: &Database,
    owner: &str,
    trigger: &str,
    id: &str,
    reason: &str,
    frontend: &str,
) -> AppResult<TriggerRun> {
    let ctx = (
        db.clone(),
        owner.to_owned(),
        trigger.to_owned(),
        id.to_owned(),
        reason.to_owned(),
        frontend.to_owned(),
    );
    let (run, changed) = super::super::transaction_service::run_db_only(
        db,
        &ctx,
        |session, (db, owner, trigger, id, reason, frontend)| {
            Box::pin(async move {
                let run = db
                    .collection::<TriggerRun>(RUNS)
                    .find_one(doc! {"_id": id, "trigger_id": trigger, "user_id": owner})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Automation run not found".into()))?;
                if !matches!(
                    run.outcome,
                    RunOutcome::Pending | RunOutcome::Started | RunOutcome::Waiting
                ) {
                    return Ok((run, false));
                }
                Ok((
                    cancel_in_session(db, &run, reason, frontend, session).await?,
                    true,
                ))
            })
        },
    )
    .await?;
    if changed {
        finish_record(db, &run).await?;
    }
    Ok(run)
}

async fn cancel_in_session(
    db: &Database,
    run: &TriggerRun,
    reason: &str,
    frontend: &str,
    session: &mut ClientSession,
) -> AppResult<TriggerRun> {
    if run.coordinator.as_ref().is_none_or(|c| c.protocol != 2) {
        return Err(AppError::Conflict(
            "Run does not support coordinator cancellation".into(),
        ));
    }
    let owner = &run.user_id;
    let id = &run.id;
    if let Some(thread) = &run.thread_id {
        db.collection::<Document>(THREADS)
            .update_one(
                doc! {"_id": thread, "user_id": owner},
                doc! {"$inc": {"automation_handoff_version": 1_i64}},
            )
            .session(&mut *session)
            .await?;
    }
    db.collection::<Document>(RUNS)
        .update_one(
            doc! {"_id": id},
            doc! {"$inc": {"coordinator.version": 1_i64}, "$set": {"coordinator.closed": true}},
        )
        .session(&mut *session)
        .await?;
    let result = finish_update(
        db,
        doc! {"_id": id, "outcome": {"$in": ["pending", "started", "waiting"]}},
        RunOutcome::Failed,
        Some(reason),
        frontend,
        session,
    )
    .await?
    .ok_or_else(|| AppError::Conflict("Automation run changed".into()))?;
    audit_in_session(db, run, "trigger_run_cancelled", reason, session).await?;
    Ok(result)
}

async fn audit_in_session(
    db: &Database,
    run: &TriggerRun,
    event: &str,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let key = super::super::audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit chain unavailable".into()))?;
    super::super::audit_service::log_system_event_in_session(
        db,
        session,
        key,
        event,
        serde_json::json!({"run_id": run.id, "trigger_id": run.trigger_id, "reason": reason}),
    )
    .await
}

#[derive(serde::Serialize)]
pub struct Diagnostic {
    pub configured: bool,
    pub readiness: super::super::security_readiness_service::Readiness,
    pub rollback_safe: bool,
    pub drain_blockers: Vec<String>,
}

pub async fn diagnostic(db: &Database, configured: bool) -> AppResult<Diagnostic> {
    let readiness = super::super::security_readiness_service::report(
        db,
        super::super::security_readiness_service::AUTOMATION_COMPLETION,
    )
    .await?;
    let mut drain_blockers = Vec::new();
    for (collection, filter, hint, reason) in drain_queries() {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id": 1})
            .hint(mongodb::options::Hint::Name(hint.into()))
            .max_time(std::time::Duration::from_secs(2))
            .await?
            .is_some()
        {
            drain_blockers.push(reason.into());
        }
    }
    Ok(Diagnostic {
        configured,
        readiness,
        rollback_safe: drain_blockers.is_empty(),
        drain_blockers,
    })
}

fn drain_queries() -> Vec<(&'static str, Document, &'static str, &'static str)> {
    vec![
        (
            RUNS,
            doc! {"coordinator.protocol": 2, "outcome": {"$in": ["pending", "started", "waiting"]}},
            "automation_coordinator_drain",
            "nonterminal_runs",
        ),
        (
            HANDOFFS,
            doc! {"state": "pending"},
            "automation_direct_drain",
            "pending_handoffs",
        ),
        (
            THREADS,
            doc! {"active_turn.automation_work.run_id": {"$gt": ""}},
            "automation_active_drain",
            "active_children",
        ),
        (
            THREADS,
            doc! {"pending_events.automation_work.run_id": {"$gt": ""}},
            "automation_event_drain",
            "queued_results",
        ),
        (
            GROUPS,
            doc! {"followers.automation.work.run_id": {"$gt": ""}},
            "automation_group_drain",
            "group_work",
        ),
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"automation_work.run_id": {"$gt": ""}, "status": {"$in": ["pending", "allowed"]}},
            "automation_card_drain",
            "confirmation_cards",
        ),
        (
            crate::models::async_service_operation::COLLECTION_NAME,
            doc! {"delivery.automation_work.run_id": {"$gt": ""}, "state": {"$in": ["submitting", "waiting", "ready", "queued", "cancelling"]}},
            "automation_async_drain",
            "async_work",
        ),
    ]
}

pub async fn indexes(db: &Database) -> mongodb::error::Result<()> {
    for (collection, keys, name) in [
        (
            RUNS,
            doc! {"user_id": 1, "thread_id": 1, "coordinator.protocol": 1, "outcome": 1},
            "automation_coordinator_thread",
        ),
        (
            HANDOFFS,
            doc! {"state": 1, "run_id": 1},
            "automation_direct_drain",
        ),
        (
            THREADS,
            doc! {"active_turn.automation_work.run_id": 1},
            "automation_active_drain",
        ),
        (
            THREADS,
            doc! {"pending_events.automation_work.run_id": 1},
            "automation_event_drain",
        ),
        (
            GROUPS,
            doc! {"followers.automation.work.run_id": 1},
            "automation_group_drain",
        ),
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"status": 1, "automation_work.run_id": 1},
            "automation_card_drain",
        ),
        (
            crate::models::async_service_operation::COLLECTION_NAME,
            doc! {"state": 1, "delivery.automation_work.run_id": 1},
            "automation_async_drain",
        ),
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"user_id": 1, "automation_work.work_id": 1, "status": 1},
            "automation_card_work",
        ),
        (
            crate::models::async_service_operation::COLLECTION_NAME,
            doc! {"user_id": 1, "delivery.automation_work.work_id": 1, "state": 1},
            "automation_async_work",
        ),
        (
            GROUPS,
            doc! {"user_id": 1, "followers.automation.work.work_id": 1},
            "automation_group_parent_work",
        ),
        (
            crate::models::assistant_group::MESSAGES_COLLECTION_NAME,
            doc! {"group_id": 1, "user_id": 1, "request_id": 1, "seq": -1},
            "automation_group_transcript",
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().name(name.to_owned()).build())
                    .build(),
            )
            .await?;
    }
    Ok(())
}

pub async fn stop_root(db: &Database, owner: &str, id: &str, turn: &str) -> AppResult<()> {
    let ctx = (db.clone(), owner.to_owned(), id.to_owned(), turn.to_owned());
    let changed = super::super::transaction_service::run_db_only(db, &ctx, |session, (db, owner, id, turn)| Box::pin(async move {
        let Some(run) = db.collection::<TriggerRun>(RUNS).find_one(doc! {"_id": id, "user_id": owner, "turn_id": turn, "outcome": "started", "coordinator.protocol": 2}).session(&mut *session).await? else { return Ok(None); };
        cancel_in_session(db, &run, "cancelled", "", session).await.map(Some)
    })).await?;
    if let Some(run) = changed {
        finish_record(db, &run).await?;
    }
    Ok(())
}

/// Transport cancellation follows a successful exact-root Stop. A stale Stop
/// that lost to settlement must not cancel descendants of a now-waiting run.
pub async fn stopped_targets(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> AppResult<Vec<crate::models::assistant_conversation::AssistantConversation>> {
    let Some(turn) = &row.active_turn else {
        return Ok(Vec::new());
    };
    let Some(work) = turn
        .automation_work
        .as_ref()
        .filter(|w| w.role == "coordinator")
    else {
        return Ok(Vec::new());
    };
    let Some(run) = db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {
            "_id": &work.run_id, "user_id": &row.user_id, "turn_id": &turn.turn_id,
            "coordinator.protocol": 2, "outcome": "failed", "reason": "cancelled",
        })
        .await?
    else {
        return Ok(Vec::new());
    };
    active_work(db, &run).await
}

/// A logical child remains outstanding across cards, nested work and external
/// async results. Callers already hold the receipt/group transaction fence.
pub async fn work_pending(
    db: &Database,
    owner: &str,
    thread: &str,
    work: &AutomationWork,
    session: &mut ClientSession,
) -> AppResult<bool> {
    for (collection, filter) in [
        (
            THREADS,
            doc! {"_id": thread, "user_id": owner, "$or": [{"active_turn.automation_work.work_id": &work.work_id}, {"pending_events.automation_work.work_id": &work.work_id}]},
        ),
        (
            HANDOFFS,
            doc! {"user_id": owner, "run_id": &work.run_id, "parent_work.work_id": &work.work_id, "state": "pending"},
        ),
        (
            GROUPS,
            doc! {"user_id": owner, "followers.automation.work.work_id": &work.work_id},
        ),
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"user_id": owner, "automation_work.work_id": &work.work_id, "status": {"$in": ["pending", "allowed"]}, "expires_at": {"$gt": date(Utc::now())}},
        ),
        (
            crate::models::async_service_operation::COLLECTION_NAME,
            doc! {"user_id": owner, "delivery.automation_work.work_id": &work.work_id, "state": {"$in": ["submitting", "waiting", "ready", "queued", "cancelling"]}},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id": 1})
            .session(&mut *session)
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Exact persisted lineage is a lifecycle fence, never execution permission.
/// Absent contexts do not call this function or add any hot-path database work.
pub async fn fence_work(
    db: &Database,
    owner: &str,
    work: &AutomationWork,
    session: &mut ClientSession,
) -> AppResult<()> {
    if db.collection::<Document>(RUNS).update_one(doc! {"_id": &work.run_id, "user_id": owner, "coordinator.protocol": 2, "coordinator.deadline": {"$gt": date(Utc::now())}, "outcome": {"$in": ["started", "waiting"]}}, doc! {"$inc": {"coordinator.version": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(AppError::AssistantTurnRequired); }
    let pending = match work.role.as_str() {
        "coordinator" => true,
        "direct" => db.collection::<Document>(HANDOFFS).find_one(doc! {"_id": &work.work_id, "run_id": &work.run_id, "user_id": owner, "state": "pending"}).projection(doc! {"_id": 1}).session(&mut *session).await?.is_some(),
        "group" => db.collection::<Document>(GROUPS).find_one(doc! {"user_id": owner, "followers": {"$elemMatch": {"automation.work.run_id": &work.run_id, "automation.dispatches": {"$elemMatch": {"id": &work.work_id, "state": "running"}}}}}).projection(doc! {"_id": 1}).session(&mut *session).await?.is_some(),
        _ => false,
    };
    if !pending {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(())
}

/// Only live persisted work may bypass ordinary event loop budgets. Selection
/// uses the same partition as admission; arbitrary tags cannot grant a bypass.
pub async fn event_can_resume(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> AppResult<bool> {
    if !row
        .pending_events
        .iter()
        .any(|event| event.automation_work.is_some())
    {
        return Ok(false);
    }
    let mut preview = row.clone();
    let events = super::super::async_service_operation::select_events(
        &mut preview,
        crate::models::assistant_conversation::TurnOrigin::Event,
    );
    let Some(work) = events
        .first()
        .and_then(|event| event.automation_work.as_ref())
    else {
        return Ok(false);
    };
    let Some(run) = db.collection::<TriggerRun>(RUNS).find_one(doc! {
        "_id": &work.run_id, "user_id": &row.user_id, "coordinator.protocol": 2,
        "coordinator.deadline": {"$gt": date(Utc::now())}, "outcome": {"$in": ["started", "waiting"]},
    }).await? else { return Ok(false); };
    Ok(match work.role.as_str() {
        "coordinator" => work.coordinator_thread_id == row.id && work.work_id == run.id
            && run.coordinator.as_ref().is_some_and(|c| c.continuations < CONTINUATION_LIMIT),
        "direct" => db.collection::<Document>(HANDOFFS).find_one(doc! {
            "_id": &work.work_id, "run_id": &run.id, "user_id": &row.user_id,
            "child_thread_id": &row.id, "state": "pending",
        }).projection(doc! {"_id": 1}).await?.is_some(),
        "group" => db.collection::<Document>(GROUPS).find_one(doc! {
            "_id": &row.group_id, "user_id": &row.user_id,
            "followers": {"$elemMatch": {"automation.work.run_id": &run.id,
                "automation.dispatches": {"$elemMatch": {"id": &work.work_id, "thread_id": &row.id, "state": "running"}}}},
        }).projection(doc! {"_id": 1}).await?.is_some(),
        _ => false,
    })
}

pub async fn active_work(
    db: &Database,
    run: &TriggerRun,
) -> AppResult<Vec<crate::models::assistant_conversation::AssistantConversation>> {
    Ok(db
        .collection(crate::models::assistant_conversation::COLLECTION_NAME)
        .find(doc! {"user_id": &run.user_id, "active_turn.automation_work.run_id": &run.id})
        .limit(i64::from(DISPATCH_LIMIT) + 1)
        .await?
        .try_collect()
        .await?)
}

pub async fn event_threads(db: &Database, run: &TriggerRun) -> AppResult<Vec<String>> {
    let rows: Vec<Document> = db.collection::<Document>(THREADS).find(doc! {"user_id": &run.user_id, "pending_events": {"$elemMatch": {"automation_work.run_id": &run.id, "automation_work.role": {"$in": ["direct", "group"]}}}}).projection(doc! {"_id": 1}).limit(i64::from(DISPATCH_LIMIT)).await?.try_collect().await?;
    Ok(rows
        .iter()
        .filter_map(|r| r.get_str("_id").ok().map(str::to_owned))
        .collect())
}

pub async fn owned_targets(
    db: &Database,
    owner: &str,
    trigger: &str,
    id: &str,
) -> AppResult<Vec<crate::models::assistant_conversation::AssistantConversation>> {
    let run = db
        .collection::<TriggerRun>(RUNS)
        .find_one(
            doc! {"_id": id, "trigger_id": trigger, "user_id": owner, "coordinator.protocol": 2},
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Automation run not found".into()))?;
    active_work(db, &run).await
}

/// Deleting an enrolled coordinator fences its run and descendants in the
/// same transaction. Receipt cleanup remains unconditional in the caller.
pub async fn delete_coordinator_in_session(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<()> {
    let mut cursor = db
        .collection::<TriggerRun>(RUNS)
        .find(doc! {
            "user_id": &row.user_id, "thread_id": &row.id, "coordinator.protocol": 2,
            "outcome": {"$in": ["started", "waiting"]},
        })
        .limit(129)
        .session(&mut *session)
        .await?;
    let runs: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
    if runs.len() > 128 {
        return Err(AppError::Conflict(
            "Cancel waiting automations before deleting this conversation".into(),
        ));
    }
    for run in runs {
        let run = cancel_in_session(db, &run, "thread_deleted", "", session).await?;
        db.collection::<Document>(WORK)
            .delete_one(doc! {"_id": &run.id})
            .session(&mut *session)
            .await?;
        db.collection::<Document>(TRIGGERS).update_one(doc! {
            "_id": &run.trigger_id, "$or": [{"schedule_state.last_run": null},
                {"schedule_state.last_run.scheduled_at": {"$lte": date(run.scheduled_at)}}],
        }, doc! {"$set": {"schedule_state.last_run": bson::to_bson(&run).map_err(|_| AppError::Internal("Encode deleted automation run".into()))?}}).session(&mut *session).await?;
    }
    Ok(())
}

/// Every v2 terminal path retires descendants, including thread deletion and
/// coordinator failure. Legacy finalization performs no new cleanup commands.
pub async fn retire_work_in_session(
    db: &Database,
    run: &TriggerRun,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let owner = &run.user_id;
    let id = &run.id;
    db.collection::<Document>(THREADS)
        .update_many(
            doc! {"user_id": owner, "active_turn.automation_work.run_id": id},
            doc! {"$set": {"active_turn.stop_requested": true}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<Document>(THREADS)
        .update_many(
            doc! {"user_id": owner, "pending_events.automation_work.run_id": id},
            doc! {"$pull": {"pending_events": {"automation_work.run_id": id}}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<Document>(HANDOFFS)
        .update_many(
            doc! {"user_id": owner, "run_id": id, "state": "pending"},
            doc! {"$set": {"state": "cancelled"}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<Document>(GROUPS)
        .update_many(
            doc! {"user_id": owner, "followers.automation.work.run_id": id},
            doc! {"$pull": {"followers": {"automation.work.run_id": id}}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME).update_many(doc! {"user_id": owner, "automation_work.run_id": id, "status": {"$in": ["pending", "allowed"]}}, doc! {"$set": {"status": "expired"}}).session(&mut *session).await?;
    db.collection::<Document>(crate::models::async_service_operation::COLLECTION_NAME).update_many(doc! {"user_id": owner, "delivery.automation_work.run_id": id, "state": {"$in": ["submitting", "waiting", "ready", "queued"]}}, doc! {"$unset": {"secret_context_versions.result_encrypted": ""}, "$set": {"state": "cancelling", "reason": reason, "result_encrypted": null, "lease_id": null, "lease_until": null, "poll_at": date(Utc::now())}}).session(&mut *session).await?;
    Ok(())
}

pub async fn child_failure(
    db: &Database,
    owner: &str,
    work: &AutomationWork,
    session: &mut ClientSession,
) -> AppResult<Option<&'static str>> {
    let card = db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME).find_one(doc! {"user_id": owner, "automation_work.work_id": &work.work_id, "$or": [{"status": {"$in": ["denied", "expired"]}}, {"status": {"$in": ["pending", "allowed"]}, "expires_at": {"$lte": date(Utc::now())}}]}).projection(doc! {"status": 1}).session(&mut *session).await?;
    if let Some(card) = card {
        return Ok(Some(if card.get_str("status").ok() == Some("denied") {
            "confirmation_denied"
        } else {
            "confirmation_expired"
        }));
    }
    if db.collection::<Document>(crate::models::async_service_operation::COLLECTION_NAME).find_one(doc! {"user_id": owner, "delivery.automation_work.work_id": &work.work_id, "state": {"$in": ["cancelled", "expired", "failed"]}}).projection(doc! {"_id": 1}).session(session).await?.is_some() {return Ok(Some("async_operation_failed"));}
    Ok(None)
}
