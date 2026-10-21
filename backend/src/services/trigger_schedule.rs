//! Trigger schedules: derived leased work, durable UUID-v5 occurrence identity,
//! and admission fenced in the same transaction as an assistant turn.
pub mod compute;
use super::api_key_mutation_service as transactions;
use super::assistant_engine_policy as policy;
use crate::models::trigger_run::RunOutcome;
use crate::{
    errors::{AppError, AppResult},
    models::{
        trigger::{COLLECTION_NAME as TRIGGERS, Trigger, TriggerDelivery, TriggerStatus},
        trigger_run::{COLLECTION_NAME as RUNS, TriggerRun},
        trigger_schedule::{ThreadPolicy, TriggerSource},
    },
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use uuid::Uuid;

pub const OWNER_LIMITS: &str = "trigger_owner_limits";
pub const WORK: &str = "trigger_work";
pub const BUDGETS: &str = "trigger_run_budgets";
pub const LEASE_SECONDS: i64 = 30;
pub const HISTORY_LIMIT: u64 = 1000;

pub fn date(at: DateTime<Utc>) -> bson::DateTime {
    bson::DateTime::from_chrono(at)
}

pub fn run_id(trigger_id: &str, at: DateTime<Utc>) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("{trigger_id}:{}", at.timestamp_millis()).as_bytes(),
    )
    .to_string()
}

pub async fn indexes(db: &Database) -> mongodb::error::Result<()> {
    for (collection, keys, name, ttl) in [
        (WORK, doc! { "at": 1, "_id": 1 }, "trigger_work_due", false),
        (
            WORK,
            doc! { "trigger_id": 1, "_id": 1 },
            "trigger_work_parent",
            false,
        ),
        (
            RUNS,
            doc! { "trigger_id": 1, "scheduled_at": -1, "_id": -1 },
            "trigger_run_history",
            false,
        ),
        (
            RUNS,
            doc! { "trigger_id": 1, "outcome": 1 },
            "trigger_run_overlap",
            false,
        ),
        (RUNS, doc! { "expires_at": 1 }, "trigger_run_ttl", true),
        (
            BUDGETS,
            doc! { "expires_at": 1 },
            "trigger_budget_ttl",
            true,
        ),
        (
            TRIGGERS,
            doc! { "user_id": 1, "setup_watch_id": 1 },
            "trigger_setup_watch",
            false,
        ),
        (
            TRIGGERS,
            doc! { "user_id": 1, "source": 1 },
            "trigger_owner_source",
            false,
        ),
    ] {
        let mut options = IndexOptions::builder().name(name.to_string()).build();
        if ttl {
            options.expire_after = Some(std::time::Duration::ZERO);
        }
        db.collection::<Document>(collection)
            .create_index(IndexModel::builder().keys(keys).options(options).build())
            .await?;
    }
    Ok(())
}

/// The only discovery query in a tick. Both the predicate and projection are
/// covered by trigger_work_due; fetching/mutating a claimed document follows.
pub async fn due(db: &Database, now: DateTime<Utc>) -> AppResult<Vec<Document>> {
    Ok(db
        .collection::<Document>(WORK)
        .find(doc! { "at": {"$lte":date(now)} })
        .projection(doc! { "_id": 1, "at": 1 })
        .sort(doc! { "at": 1, "_id": 1 })
        .limit(100)
        .hint(mongodb::options::Hint::Name("trigger_work_due".into()))
        .await?
        .try_collect()
        .await?)
}

pub async fn claim(db: &Database, id: &str, now: DateTime<Utc>) -> AppResult<Option<Document>> {
    Ok(db
        .collection::<Document>(WORK)
        .find_one_and_update(
            doc! { "_id": id, "at": {"$lte":date(now)} },
            doc! {
                "$set": {
                    "at": date(now+Duration::seconds(LEASE_SECONDS)),
                    "fence": Uuid::new_v4().to_string(),
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?)
}

pub async fn reschedule(db: &Database, job: &Document, at: Option<DateTime<Utc>>) -> AppResult<()> {
    let filter = doc! {
        "_id": job.get_str("_id").unwrap_or_default(),
        "fence": job.get_str("fence").unwrap_or_default(),
    };
    if let Some(at) = at {
        db.collection::<Document>(WORK)
            .update_one(filter, doc! { "$set": {"at":date(at)} })
            .await?;
    } else {
        db.collection::<Document>(WORK).delete_one(filter).await?;
    }
    Ok(())
}

pub async fn set_schedule_job(
    db: &Database,
    trigger: &Trigger,
    session: &mut ClientSession,
) -> AppResult<()> {
    let id = format!("schedule:{}", trigger.id);
    if trigger.source == TriggerSource::Schedule
        && trigger.status == TriggerStatus::Active
        && let Some(at) = trigger.schedule_state.next_due_at
    {
        super::erasure_admission::fence_in_session(db, &trigger.user_id, session).await?;
        db.collection::<Document>(WORK)
            .replace_one(
                doc! { "_id": &id },
                doc! {
                    "_id": &id,
                    "kind": "schedule",
                    "trigger_id": &trigger.id,
                    "user_id": &trigger.user_id,
                    "at": date(at),
                    "fence": "",
                },
            )
            .upsert(true)
            .session(session)
            .await?;
    } else {
        db.collection::<Document>(WORK)
            .delete_one(doc! { "_id": id })
            .session(session)
            .await?;
    }
    Ok(())
}

pub fn new_run(trigger: &Trigger, at: DateTime<Utc>, deadline: DateTime<Utc>) -> TriggerRun {
    TriggerRun {
        trigger_generation: None,
        dispatch_at: None,
        terminal_result_recorded: false,
        attempts: None,
        capacity_wait_ms: None,
        id: run_id(&trigger.id, at),
        trigger_id: trigger.id.clone(),
        user_id: trigger.user_id.clone(),
        scheduled_at: at,
        deadline,
        outcome: RunOutcome::Pending,
        confirmation_policy: None,
        reason: None,
        started_at: None,
        finished_at: None,
        thread_id: None,
        turn_id: None,
        duration_ms: None,
        missed: None,
        waiting_for: Vec::new(),
        result_claimed: false,
        image_refs: Vec::new(),
        image_refs_truncated: 0,
        agent_id: None,
        deliver_to: Default::default(),
        label: trigger.label.clone(),
        fence: String::new(),
        lease_until: Utc::now(),
        expires_at: Utc::now() + Duration::days(90),
    }
}

pub(crate) async fn insert_run(
    db: &Database,
    run: &TriggerRun,
    session: &mut ClientSession,
) -> AppResult<()> {
    super::erasure_admission::fence_in_session(db, &run.user_id, session).await?;
    let stored = db
        .collection::<Document>(TRIGGERS)
        .find_one_and_update(
            doc! { "_id": &run.trigger_id, "status": "active" },
            doc! { "$inc": {"run_version":1,"history_count":1} },
        )
        .session(&mut *session)
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    if !stored.contains_key("history_count") {
        let count = db
            .collection::<Document>(RUNS)
            .count_documents(doc! { "trigger_id": &run.trigger_id })
            .session(&mut *session)
            .await?;
        db.collection::<Document>(TRIGGERS)
            .update_one(
                doc! { "_id": &run.trigger_id },
                doc! { "$set": {"history_count": count as i64 + 1} },
            )
            .session(&mut *session)
            .await?;
    }
    let trigger: Trigger = bson::from_document(stored)
        .map_err(|error| AppError::Internal(format!("Read trigger during admission: {error}")))?;
    let outstanding = db
        .collection::<TriggerRun>(RUNS)
        .count_documents(doc! {
            "trigger_id": &run.trigger_id,
            "outcome": {
                "$in": [
                    RunOutcome::Pending.as_str(),
                    RunOutcome::Started.as_str(),
                    RunOutcome::Waiting.as_str(),
                ],
            },
            "reason": { "$nin": ["overlap_skip", "queue_full"] },
        })
        .session(&mut *session)
        .await?;
    let mut run = run.clone();
    if outstanding > 0 && trigger.overlap == crate::models::trigger_schedule::OverlapPolicy::Skip {
        run.reason = Some("overlap_skip".into());
    } else if outstanding >= 2 {
        run.reason = Some("queue_full".into());
    }
    db.collection::<TriggerRun>(RUNS)
        .insert_one(&run)
        .session(&mut *session)
        .await?;
    db.collection::<Document>(WORK)
        .insert_one(doc! {
            "_id": &run.id,
            "kind": "run",
            "trigger_id": &run.trigger_id,
            "user_id": &run.user_id,
            "at": date(run.dispatch_at.unwrap_or_else(Utc::now)),
            "fence": "",
        })
        .session(session)
        .await?;
    Ok(())
}

/// Advance the schedule and create its occurrence in one transaction. The job's
/// fence rejects an expired worker even if it computed the same next time.
pub async fn claim_occurrence(
    db: &Database,
    job: &Document,
    now: DateTime<Utc>,
) -> AppResult<Option<TriggerRun>> {
    let mut session = db.client().start_session().await?;
    let job = job.clone();
    let audit_db = db.clone();
    let db = db.clone();
    let claimed = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = async {
                let fence = doc! {
                    "_id": job.get_str("_id").unwrap_or_default(),
                    "fence": job.get_str("fence").unwrap_or_default(),
                    "at": { "$gt": date(now) },
                };
                if db
                    .collection::<Document>(WORK)
                    .find_one(fence.clone())
                    .session(&mut *session)
                    .await?
                    .is_none()
                {
                    return Ok(None);
                }
                let Some(mut trigger) = db
                    .collection::<Trigger>(TRIGGERS)
                    .find_one(doc! { "_id": job.get_str("trigger_id").unwrap_or_default() })
                    .session(&mut *session)
                    .await?
                else {
                    db.collection::<Document>(WORK)
                        .delete_one(fence)
                        .session(&mut *session)
                        .await?;
                    return Ok(None);
                };
                if trigger.status != TriggerStatus::Active
                    || trigger.source != TriggerSource::Schedule
                {
                    db.collection::<Document>(WORK)
                        .delete_one(fence)
                        .session(&mut *session)
                        .await?;
                    return Ok(None);
                }
                let spec = trigger
                    .schedule
                    .as_ref()
                    .ok_or_else(|| AppError::Internal("Schedule specification missing".into()))?;
                let Some(due) = trigger.schedule_state.next_due_at.filter(|due| *due <= now) else {
                    return Ok(None);
                };
                let at = compute::latest(spec, now)?.unwrap_or(due).max(due);
                let mut run = new_run(&trigger, at, at + compute::grace(spec, at)?);
                run.dispatch_at = Some(at + dispatch_offset(&trigger.id, spec, at)?);
                insert_run(&db, &run, session).await?;
                // Catch-up executes only the newest occurrence. All missed work is
                // one metadata-only summary with one deterministic identity/audit.
                let missed = if due < at {
                    let through =
                        compute::latest(spec, at - Duration::milliseconds(1))?.unwrap_or(due);
                    let mut summary = new_run(&trigger, due, due);
                    summary.outcome = RunOutcome::Skipped;
                    summary.reason = Some("missed_window".into());
                    summary.finished_at = Some(now);
                    summary.missed = Some(crate::models::trigger_run::MissedOccurrences {
                        count: compute::missed_count(spec, due, at)?,
                        from: due,
                        through,
                    });
                    db.collection::<TriggerRun>(RUNS)
                        .insert_one(&summary)
                        .session(&mut *session)
                        .await?;
                    db.collection::<Trigger>(TRIGGERS)
                        .update_one(
                            doc! { "_id": &trigger.id },
                            doc! { "$inc": {"history_count": 1} },
                        )
                        .session(&mut *session)
                        .await?;
                    Some(summary)
                } else {
                    None
                };
                trigger.schedule_state.occurrences =
                    trigger.schedule_state.occurrences.saturating_add(1);
                trigger.schedule_state.next_due_at = if spec
                    .max_runs
                    .is_some_and(|max| trigger.schedule_state.occurrences >= max)
                {
                    None
                } else {
                    compute::next(spec, now)?
                };
                db.collection::<Trigger>(TRIGGERS).update_one(doc! { "_id": &trigger.id },doc! {
                    "$set": {
                        "schedule_state.next_due_at": trigger.schedule_state.next_due_at.map(date),
                        "schedule_state.occurrences": i64::from(trigger.schedule_state.occurrences),
                    },
                }).session(&mut *session).await?;
                set_schedule_job(&db, &trigger, session).await?;
                Ok(Some((run, missed)))
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Some((run, skipped)) = claimed {
        if let Some(item) = skipped {
            let missed = item.missed.as_ref().expect("summary metadata");
            audit(
                &audit_db,
                &item.user_id,
                "trigger_occurrences_missed",
                doc! {
                    "trigger_id": &item.trigger_id,
                    "run_id": &item.id,
                    "count": missed.count as i64,
                    "from": date(missed.from),
                    "through": date(missed.through),
                },
            )
            .await;
        }
        trim_history(&audit_db, &run.trigger_id).await?;
        Ok(Some(run))
    } else {
        Ok(None)
    }
}

pub async fn enqueue_now(db: &Database, trigger: &Trigger) -> AppResult<TriggerRun> {
    if trigger.status != TriggerStatus::Active {
        return Err(AppError::ValidationError(
            "Resume this trigger before running it".into(),
        ));
    }
    let now = Utc::now();
    let grace = trigger
        .schedule
        .as_ref()
        .map(|s| compute::grace(s, now))
        .transpose()?
        .unwrap_or(Duration::hours(1));
    let run = new_run(trigger, now, now + grace);
    let db = db.clone();
    let saved = run.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(insert_run(&db, &saved, session).await)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    Ok(run)
}

pub async fn history(
    db: &Database,
    trigger: &Trigger,
    before: Option<DateTime<Utc>>,
    before_id: Option<&str>,
) -> AppResult<Vec<TriggerRun>> {
    let mut filter = doc! { "trigger_id": &trigger.id, "user_id": &trigger.user_id };
    if let Some(before) = before {
        if let Some(id) = before_id {
            filter.insert(
                "$or",
                vec![
                    doc! { "scheduled_at": {"$lt":date(before)} },
                    doc! { "scheduled_at": date(before), "_id": {"$lt":id} },
                ],
            );
        } else {
            filter.insert("scheduled_at", doc! { "$lt": date(before) });
        }
    }
    Ok(db
        .collection::<TriggerRun>(RUNS)
        .find(filter)
        .sort(doc! { "scheduled_at": -1, "_id": -1 })
        .limit(100)
        .await?
        .try_collect()
        .await?)
}

pub async fn finish(
    db: &Database,
    run_id: &str,
    outcome: RunOutcome,
    reason: Option<&str>,
    frontend: &str,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let work_db = db.clone();
    let run_id = run_id.to_owned();
    let reason = reason.map(str::to_owned);
    let frontend = frontend.to_owned();
    let run = session
        .start_transaction()
        .and_run2(async move |session| {
            let result = Box::pin(finish_update(
                &work_db,
                doc! {"_id": &run_id, "outcome": {"$in": ["pending", "started", "waiting"]}},
                outcome,
                reason.as_deref(),
                &frontend,
                session,
            ))
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Some(run) = run {
        finish_record(db, &run).await?;
    }
    Ok(())
}

/// Fenced terminal decisions belong only to the current worker.
pub fn finish_claimed<'a>(
    db: &'a Database,
    job: &'a Document,
    run_id: &'a str,
    outcome: RunOutcome,
    reason: Option<&'a str>,
    frontend: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    // The scheduler settles many exit paths. Keep each transaction's state
    // out of its polling frame while it starts a complete assistant turn.
    Box::pin(finish_claimed_inner(
        db, job, run_id, outcome, reason, frontend,
    ))
}

async fn finish_claimed_inner(
    db: &Database,
    job: &Document,
    run_id: &str,
    outcome: RunOutcome,
    reason: Option<&str>,
    frontend: &str,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let work_db = db.clone();
    let job = job.clone();
    let id = run_id.to_owned();

    let reason = reason.map(str::to_owned);
    let frontend = frontend.to_owned();
    let run = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = async {
                if work_db
                    .collection::<Document>(WORK)
                    .update_one(
                        doc! {
                            "_id": &id,
                            "fence": job.get_str("fence").unwrap_or_default(),
                            "at": { "$gt": date(Utc::now()) },
                        },
                        doc! { "$inc": {"settlement_version":1} },
                    )
                    .session(&mut *session)
                    .await?
                    .matched_count
                    == 0
                {
                    return Ok(None);
                }
                Box::pin(finish_update(
                    &work_db,
                    doc! {
                        "_id": &id,
                        "outcome": { "$in": [RunOutcome::Pending.as_str(),RunOutcome::Waiting.as_str()] },
                    },
                    outcome,
                    reason.as_deref(),
                    &frontend,
                    session,
                ))
                .await
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Some(run) = run {
        finish_record(db, &run).await?;
    }
    Ok(())
}

async fn finish_update(
    db: &Database,
    filter: Document,
    outcome: RunOutcome,
    reason: Option<&str>,
    frontend: &str,
    session: &mut ClientSession,
) -> AppResult<Option<TriggerRun>> {
    let now = Utc::now();
    let collection = db.collection::<TriggerRun>(RUNS);
    let action = collection
        .find_one_and_update(
            filter,
            vec![doc! {
                "$set": {
                    "outcome": outcome.as_str(),
                    "reason": reason,
                    "finished_at": date(now),
                    "duration_ms": {
                        "$cond": [
                            { "$ne": ["$started_at", null] },
                            { "$subtract": [date(now), "$started_at"] },
                            null,
                        ],
                    },
                },
            }],
        )
        .return_document(ReturnDocument::After);
    let mut run = action.session(&mut *session).await?;
    if let Some(run) = &mut run {
        Box::pin(record_terminal_in_session(
            db, session, run, reason, frontend,
        ))
        .await?;
        run.terminal_result_recorded = true;
    }
    Ok(run)
}

async fn finish_record(db: &Database, run: &TriggerRun) -> AppResult<()> {
    db.collection::<Document>(WORK)
        .delete_one(doc! { "_id": &run.id })
        .await?;
    let last_run = bson::to_bson(run)
        .map_err(|error| AppError::Internal(format!("Serialize trigger run: {error}")))?;
    db.collection::<Trigger>(TRIGGERS)
        .update_one(
            doc! {
                "_id": &run.trigger_id,
                "$or": [
                    { "schedule_state.last_run": null },
                    { "schedule_state.last_run.scheduled_at": { "$lte": date(run.scheduled_at) } },
                ],
            },
            doc! {
                "$set": {
                    "schedule_state.last_run": last_run,
                },
            },
        )
        .await?;
    audit(
        db,
        &run.user_id,
        "trigger_run_outcome",
        doc! {
            "trigger_id": &run.trigger_id,
            "run_id": &run.id,
            "outcome": run.outcome.as_str(),
            "reason": &run.reason,
        },
    )
    .await;
    trim_history(db, &run.trigger_id).await
}

async fn trim_history(db: &Database, trigger_id: &str) -> AppResult<()> {
    // One projected point read on the common path. Inserts increment this
    // counter transactionally; TTL deletion may overestimate, never underestimate.
    let Some(trigger) = db
        .collection::<Document>(TRIGGERS)
        .find_one(doc! { "_id": trigger_id })
        .projection(doc! { "history_count": 1 })
        .await?
    else {
        return Ok(());
    };
    let count = trigger
        .get_i64("history_count")
        .ok()
        .or_else(|| trigger.get_i32("history_count").ok().map(i64::from));
    if count.is_some_and(|count| count <= HISTORY_LIMIT as i64) {
        return Ok(());
    }
    let db = db.clone();
    let trigger_id = trigger_id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                db.collection::<Document>(TRIGGERS)
                    .update_one(
                        doc! { "_id": &trigger_id },
                        doc! { "$inc": {"history_version": 1} },
                    )
                    .session(&mut *session)
                    .await?;
                let count = db
                    .collection::<Document>(RUNS)
                    .count_documents(doc! { "trigger_id": &trigger_id })
                    .session(&mut *session)
                    .await?;
                let mut deleted = 0;
                if count > HISTORY_LIMIT {
                    let mut cursor = db
                        .collection::<Document>(RUNS)
                        .find(doc! {
                            "trigger_id": &trigger_id,
                            "outcome": {
                                "$in": [
                                    RunOutcome::Completed.as_str(),
                                    RunOutcome::Failed.as_str(),
                                    RunOutcome::Skipped.as_str(),
                                ],
                            },
                        })
                        .sort(doc! { "scheduled_at": 1, "_id": 1 })
                        .limit((count - HISTORY_LIMIT).min(HISTORY_LIMIT) as i64)
                        .projection(doc! { "_id": 1 })
                        .session(&mut *session)
                        .await?;
                    let mut ids = Vec::new();
                    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
                        if let Ok(id) = row.get_str("_id") {
                            ids.push(id.to_owned());
                        }
                    }
                    if !ids.is_empty() {
                        deleted = db
                            .collection::<Document>(RUNS)
                            .delete_many(doc! { "_id": {"$in": ids} })
                            .session(&mut *session)
                            .await?
                            .deleted_count;
                    }
                }
                db.collection::<Document>(TRIGGERS)
                    .update_one(
                        doc! { "_id": &trigger_id },
                        doc! { "$set": {"history_count": (count - deleted) as i64} },
                    )
                    .session(session)
                    .await?;
                Ok(())
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

pub async fn audit(db: &Database, owner: &str, event: &str, metadata: Document) {
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        &super::audit_service::AuditActor {
            user_id: owner.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        event,
        Some(serde_json::to_value(metadata).unwrap_or_default()),
    )
    .await;
}

#[derive(Clone, Debug)]
pub struct TurnClaim {
    pub run_id: String,
    pub fence: String,
    pub trigger_updated_at: DateTime<Utc>,
    pub continuation: bool,
}

/// Called ONLY for trigger turns, inside begin_turn's transaction. A committed
/// turn and its run barrier cannot be separated by a crash or a lease expiry.
pub fn admit_turn<'a>(
    db: &'a Database,
    owner: &'a str,
    claim: &'a TurnClaim,
    thread: &'a str,
    turn: &'a str,
    session: &'a mut ClientSession,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    // Keep the schedule's transaction state out of the enclosing owner-turn
    // admission future, which also carries the assistant's authority fences.
    Box::pin(admit_turn_inner(db, owner, claim, thread, turn, session))
}

async fn admit_turn_inner(
    db: &Database,
    owner: &str,
    claim: &TurnClaim,
    thread: &str,
    turn: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let now = Utc::now();
    let mut run_filter = doc! {
        "_id": &claim.run_id,
        "user_id": owner,
        "outcome": if claim.continuation {
        RunOutcome::Waiting.as_str()
    } else {
        RunOutcome::Pending.as_str()
    },
    };
    if !claim.continuation {
        run_filter.insert("deadline", doc! { "$gte": date(now) });
    }
    let run = db
        .collection::<TriggerRun>(RUNS)
        .find_one(run_filter)
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Conflict("trigger_run_unavailable".into()))?;
    if db
        .collection::<Document>(WORK)
        .update_one(
            doc! { "_id": &run.id, "fence": &claim.fence, "at": {"$gt":date(now)} },
            doc! {
                "$set": {
                    "at": date(now+Duration::seconds(super::assistant_nyxagent::ACTIVE_TURN_TTL_SECS)),
                },
            },
        )
        .session(&mut *session)
        .await?
        .matched_count
        == 0
    {
        return Err(AppError::Conflict("trigger_lease_lost".into()));
    }
    let trigger = db
        .collection::<Trigger>(TRIGGERS)
        .find_one_and_update(
            doc! {
                "_id": &run.trigger_id,
                "user_id": owner,
                "status": "active",
                "updated_at": date(claim.trigger_updated_at),
            },
            doc! { "$inc": {"run_version":1} },
        )
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Conflict("trigger_paused".into()))?;
    let running = db
        .collection::<TriggerRun>(RUNS)
        .count_documents(doc! {
            "trigger_id": &run.trigger_id,
            "_id": { "$ne": &run.id },
            "outcome": { "$in": [RunOutcome::Started.as_str(), RunOutcome::Waiting.as_str()] },
        })
        .session(&mut *session)
        .await?;
    if running > 0 {
        return Err(AppError::Conflict(
            match trigger.overlap {
                crate::models::trigger_schedule::OverlapPolicy::Skip => "trigger_overlap_skip",
                _ => "trigger_overlap_queue",
            }
            .into(),
        ));
    }
    if !claim.continuation {
        let oldest = db
            .collection::<TriggerRun>(RUNS)
            .find_one(doc! {
                "trigger_id": &run.trigger_id,
                "outcome": RunOutcome::Pending.as_str(),
                "reason": { "$nin": ["overlap_skip", "queue_full"] },
            })
            .sort(doc! { "scheduled_at": 1, "_id": 1 })
            .session(&mut *session)
            .await?;
        if oldest.is_some_and(|oldest| oldest.id != run.id) {
            return Err(AppError::Conflict("trigger_overlap_queue".into()));
        }
    }
    let owner_row = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! { "_id": owner, "is_active": true })
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Forbidden("Automation owner unavailable".into()))?;
    if !owner_row.user_type.is_person() {
        return Err(AppError::Forbidden("Automations belong to a person".into()));
    }
    if !claim.continuation {
        let settings = db
            .collection::<crate::models::assistant_settings::AssistantSettings>(
                crate::models::assistant_settings::COLLECTION_NAME,
            )
            .find_one(doc! { "_id": owner })
            .session(&mut *session)
            .await?
            .unwrap_or_else(|| {
                crate::models::assistant_settings::AssistantSettings::defaults(owner)
            });
        let hour = now.timestamp().div_euclid(3600);
        let day = now.timestamp().div_euclid(86400);
        let current = db
            .collection::<Document>(BUDGETS)
            .find_one(doc! { "_id": owner })
            .session(&mut *session)
            .await?
            .unwrap_or_default();
        let hourly = if current.get_i64("hour").ok() == Some(hour) {
            current.get_i32("hourly").unwrap_or_default()
        } else {
            0
        };
        let daily = if current.get_i64("day").ok() == Some(day) {
            current.get_i32("daily").unwrap_or_default()
        } else {
            0
        };
        if hourly >= settings.trigger_runs_per_hour || daily >= settings.trigger_runs_per_day {
            return Err(AppError::TriggerRateLimited);
        }
        db.collection::<Document>(BUDGETS)
            .update_one(
                doc! { "_id": owner },
                doc! {
                    "$set": {
                        "hour": hour,
                        "day": day,
                        "hourly": hourly+1,
                        "daily": daily+1,
                        "expires_at": date(now+Duration::days(2)),
                    },
                },
            )
            .upsert(true)
            .session(&mut *session)
            .await?;
    }
    let mut set = doc! {
        "outcome": RunOutcome::Started.as_str(),
        "reason": null,
        "thread_id": thread,
        "turn_id": turn,
        "waiting_for": [],
    };
    if !claim.continuation {
        set.insert("trigger_generation", date(trigger.updated_at));
        set.insert("started_at", date(now));
        if let TriggerDelivery::Assistant {
            agent_id,
            deliver_to,
            confirmation_policy,
            ..
        } = &trigger.delivery
        {
            if trigger.source == TriggerSource::Webhook {
                set.insert(
                    "confirmation_policy",
                    bson::to_bson(confirmation_policy).map_err(|error| {
                        AppError::Internal(format!("Serialize confirmation policy: {error}"))
                    })?,
                );
            }
            set.insert("agent_id", agent_id);
            set.insert(
                "deliver_to",
                bson::to_bson(deliver_to).map_err(|error| {
                    AppError::Internal(format!("Serialize trigger delivery: {error}"))
                })?,
            );
        }
    }
    db.collection::<TriggerRun>(RUNS)
        .update_one(doc! { "_id": &run.id }, doc! { "$set": set })
        .session(session)
        .await?;
    Ok(())
}

/// Webhook data stays out of the owner-facing home context unless explicitly chosen.
pub fn default_thread_policy(
    source: TriggerSource,
    agent: &crate::models::assistant_agent::AssistantAgent,
) -> ThreadPolicy {
    if source == TriggerSource::Schedule && agent.is_nyxbot() {
        ThreadPolicy::Home
    } else {
        ThreadPolicy::Dedicated
    }
}

/// Choose and reserve the run thread transactionally, so competing/recovering
/// workers never create a second dedicated or per-run thread.
pub fn run_thread<'a>(
    db: &'a Database,
    keys: &'a std::sync::Arc<crate::crypto::aes::EncryptionKeys>,
    run: &'a TriggerRun,
    trigger: &'a Trigger,
) -> futures::future::BoxFuture<
    'a,
    AppResult<crate::models::assistant_conversation::AssistantConversation>,
> {
    // Thread creation and its authority transaction precede a full assistant
    // turn; do not embed their state in the scheduler's polling frame.
    Box::pin(run_thread_inner(db, keys, run, trigger))
}

async fn run_thread_inner(
    db: &Database,
    keys: &std::sync::Arc<crate::crypto::aes::EncryptionKeys>,
    run: &TriggerRun,
    trigger: &Trigger,
) -> AppResult<crate::models::assistant_conversation::AssistantConversation> {
    use crate::models::trigger_schedule::ThreadPolicy;
    let TriggerDelivery::Assistant {
        agent_id,
        thread_policy,
        ..
    } = &trigger.delivery
    else {
        return Err(AppError::TriggerDeliveryUnsupported);
    };
    let agent = super::assistant_team_service::agent(db, &trigger.user_id, agent_id).await?;
    let policy = thread_policy.unwrap_or_else(|| default_thread_policy(trigger.source, &agent));
    let home = if policy == ThreadPolicy::Home {
        Some(
            super::assistant_team_service::home_thread_for(db, keys, &trigger.user_id, &agent)
                .await?,
        )
    } else {
        None
    };
    let db = db.clone();
    let keys = keys.clone();
    let run = run.clone();
    let trigger = trigger.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(
                run_thread_in_session(&db, &keys, &run, &trigger, &agent, home.as_ref(), session)
                    .await,
            )
        })
        .await
        .map_err(transactions::map_transaction_error)
}

pub(crate) async fn run_thread_in_session(
    db: &Database,
    keys: &std::sync::Arc<crate::crypto::aes::EncryptionKeys>,
    run: &TriggerRun,
    trigger: &Trigger,
    agent: &crate::models::assistant_agent::AssistantAgent,
    home: Option<&crate::models::assistant_conversation::AssistantConversation>,
    session: &mut ClientSession,
) -> AppResult<crate::models::assistant_conversation::AssistantConversation> {
    use crate::models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS},
        trigger_schedule::ThreadPolicy,
    };
    let TriggerDelivery::Assistant { thread_policy, .. } = &trigger.delivery else {
        return Err(AppError::TriggerDeliveryUnsupported);
    };
    let policy = thread_policy.unwrap_or_else(|| default_thread_policy(trigger.source, agent));
    let current = db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! { "_id": &run.id })
        .session(&mut *session)
        .await?
        .ok_or(AppError::TriggerDeliveryRecordNotFound)?;
    // This write serializes simultaneous first runs and fences edited targets.
    let current_trigger = db
        .collection::<Trigger>(TRIGGERS)
        .find_one_and_update(
            doc! { "_id": &trigger.id, "status": "active", "updated_at": date(trigger.updated_at) },
            doc! { "$inc": {"run_version":1} },
        )
        .session(&mut *session)
        .await?
        .ok_or(AppError::TriggerNotFound)?;
    let id = current.thread_id.as_ref().or_else(|| {
        (policy == ThreadPolicy::Dedicated)
            .then_some(current_trigger.schedule_state.dedicated_thread_id.as_ref())
            .flatten()
    });
    let existing = if let Some(id) = id {
        db.collection::<AssistantConversation>(THREADS)
            .find_one(doc! {
                "_id": id,
                "user_id": &run.user_id,
                "agent_id": &agent.id,
                "channel": null,
                "group_id": null,
            })
            .session(&mut *session)
            .await?
    } else {
        None
    };
    let row = if let Some(row) = existing.or_else(|| home.cloned()) {
        row
    } else {
        super::assistant_team_service::create_automation_thread(
            db,
            keys,
            &run.user_id,
            agent,
            &trigger.label,
            session,
        )
        .await?
    };
    db.collection::<TriggerRun>(RUNS)
        .update_one(
            doc! { "_id": &run.id },
            doc! { "$set": {"thread_id":&row.id} },
        )
        .session(&mut *session)
        .await?;
    if policy == ThreadPolicy::Dedicated {
        db.collection::<Trigger>(TRIGGERS)
            .update_one(
                doc! { "_id": &trigger.id },
                doc! { "$set": {"schedule_state.dedicated_thread_id":&row.id} },
            )
            .session(&mut *session)
            .await?;
    }
    Ok(row)
}

pub async fn pause_target(db: &Database, trigger: &Trigger, reason: &str) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let work_db = db.clone();
    let saved = trigger.clone();
    let saved_reason = reason.to_owned();
    let changed = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = async {
                let modified =
                    pause_target_in_session(&work_db, &saved, &saved_reason, session).await?;
                Ok(modified)
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if changed {
        audit(
            db,
            &trigger.user_id,
            "trigger_paused",
            doc! { "trigger_id": &trigger.id, "reason": reason },
        )
        .await;
    }
    Ok(())
}

async fn pause_target_in_session(
    db: &Database,
    trigger: &Trigger,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<bool> {
    let modified = db
        .collection::<Trigger>(TRIGGERS)
        .update_one(
            doc! {"_id": &trigger.id, "status": "active", "updated_at": date(trigger.updated_at)},
            doc! {"$set": {"status": "disabled", "schedule_state.pause_reason": reason,
            "updated_at": date(Utc::now().max(trigger.updated_at + Duration::milliseconds(1)))}},
        )
        .session(&mut *session)
        .await?
        .modified_count
        == 1;
    if modified {
        db.collection::<Document>(WORK)
            .delete_one(doc! {"_id": format!("schedule:{}", trigger.id)})
            .session(session)
            .await?;
    }
    Ok(modified)
}

/// Offset only dispatch eligibility; occurrence identity and missed history use
/// the original UTC instant, including cron occurrences around DST transitions.
pub fn dispatch_offset(
    trigger: &str,
    spec: &crate::models::trigger_schedule::ScheduleSpec,
    at: DateTime<Utc>,
) -> AppResult<Duration> {
    use sha2::{Digest, Sha256};
    if matches!(
        spec.timing,
        crate::models::trigger_schedule::ScheduleKind::At { .. }
    ) {
        return Ok(Duration::zero());
    }
    let grace = compute::grace(spec, at)?;
    if grace < Duration::from_std(policy::DISPATCH_JITTER_MIN_GRACE).expect("bounded grace") {
        return Ok(Duration::zero());
    }
    let next_bound = compute::next(spec, at)?.map_or(
        Duration::from_std(policy::DISPATCH_JITTER_MAX).expect("bounded jitter"),
        |next| (next - at) / policy::DISPATCH_JITTER_WINDOW_DIVISOR,
    );
    let cap = Duration::from_std(policy::DISPATCH_JITTER_MAX)
        .expect("bounded jitter")
        .min(grace / policy::DISPATCH_JITTER_WINDOW_DIVISOR)
        .min(next_bound)
        .num_milliseconds()
        .max(0) as u64;
    let hash = Sha256::digest(trigger.as_bytes());
    let value = u64::from_be_bytes(hash[..8].try_into().expect("SHA256 prefix"));
    Ok(Duration::milliseconds((value % (cap + 1)) as i64))
}

pub fn failure_streak(
    previous: Option<&str>,
    count: u32,
    result: Option<&str>,
) -> (Option<String>, u32, bool) {
    match result {
        Some(
            code @ ("insufficient_credits" | "wallet_suspended" | "plan_entitlement_required"),
        ) => (Some(code.into()), 1, true),
        Some(code @ ("credential_invalid" | "model_unavailable" | "model_not_configured")) => {
            let count = if previous == Some(code) {
                count.saturating_add(1)
            } else {
                1
            };
            (
                Some(code.into()),
                count,
                count >= policy::PERSISTENT_FAILURE_PAUSE_THRESHOLD,
            )
        }
        _ => (None, 0, false),
    }
}

pub fn pause_notice(reason: &str, frontend: &str) -> String {
    use super::assistant_links::AssistantPage;
    let remedy = match reason {
        "insufficient_credits" => "Top up your credits in Billing & Usage",
        "wallet_suspended" => {
            "Contact support about your suspended wallet or review Billing & Usage"
        }
        "plan_entitlement_required" => {
            "Review your plan and service entitlement in Billing & Usage"
        }
        "credential_invalid" => "Check the assistant credential and reconnect if needed",
        "model_unavailable" | "model_not_configured" => {
            "Choose an available model in the assistant model selector"
        }
        _ => "Review the failed run in its thread",
    };
    let page = if matches!(
        reason,
        "insufficient_credits" | "wallet_suspended" | "plan_entitlement_required"
    ) {
        AssistantPage::Billing { tab: "overview" }
    } else {
        AssistantPage::NyxBotSettings
    };
    format!(
        "Automation paused: {} {}: {}. After fixing it, open {} and select Resume. Missed runs will not be replayed.",
        super::assistant_nyxagent::TurnError::new(reason).message,
        remedy,
        page.url(frontend),
        AssistantPage::Automations {
            setup: None,
            agent: None
        }
        .url(frontend)
    )
}

/// Run identity, authored generation, streak, pause, durable owner notice and
/// external delivery work commit together. Replays cannot increment the streak.
pub async fn terminal_result(
    db: &Database,
    run: &TriggerRun,
    result: Option<&str>,
    frontend: &str,
) -> AppResult<()> {
    if !matches!(run.outcome, RunOutcome::Started | RunOutcome::Waiting) {
        return Ok(());
    }
    let db = db.clone();
    let run = run.clone();
    let result = result.map(str::to_owned);
    let frontend = frontend.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation =
                record_terminal_in_session(&db, session, &run, result.as_deref(), &frontend).await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

async fn record_terminal_in_session(
    db: &Database,
    session: &mut ClientSession,
    run: &TriggerRun,
    result: Option<&str>,
    frontend: &str,
) -> AppResult<()> {
    super::erasure_admission::fence_in_session(db, &run.user_id, session).await?;
    let Some(run) = db
        .collection::<TriggerRun>(RUNS)
        .find_one_and_update(
            doc! {"_id": &run.id, "outcome": run.outcome.as_str(), "turn_id": &run.turn_id,
            "terminal_result_recorded": {"$ne": true}},
            doc! {"$set": {"terminal_result_recorded": true}},
        )
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    let Some(generation) = run.trigger_generation else {
        return Ok(());
    };
    let Some(trigger) = db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! {
            "_id": &run.trigger_id, "user_id": &run.user_id,
            "status": "active", "updated_at": date(generation),
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    let (code, count, pause) = failure_streak(
        trigger.schedule_state.terminal_failure_code.as_deref(),
        trigger.schedule_state.terminal_failure_count,
        result,
    );
    db.collection::<Document>(TRIGGERS)
        .update_one(
            doc! {"_id": &trigger.id, "updated_at": date(generation)},
            doc! {"$set": {
                "schedule_state.terminal_failure_code": &code,
                "schedule_state.terminal_failure_count": i64::from(count),
            }},
        )
        .session(&mut *session)
        .await?;
    if !pause {
        return Ok(());
    }
    let reason = code.as_deref().expect("pause code");
    if !pause_target_in_session(db, &trigger, reason, session).await? {
        return Ok(());
    }
    let notice_id = Uuid::new_v4().to_string();
    let text = pause_notice(reason, frontend);
    if let Some(thread) = &run.thread_id {
        use crate::models::{
            assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
            assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        };
        let now = Utc::now();
        if let Some(conversation) = db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one_and_update(
                doc! {"_id": thread, "user_id": &run.user_id},
                doc! {"$inc": {"message_count": 1}, "$set": {"updated_at": date(now)}},
            )
            .return_document(ReturnDocument::After)
            .session(&mut *session)
            .await?
        {
            let message = AssistantMessage {
                platform_delivery_recovery_queued: false,
                platform_reply: None,
                platform_origin: None,
                author: None,
                retry_claimed_by: None,
                activity_count: None,
                activity_total: 0,
                report_to: None,
                id: notice_id.clone(),
                conversation_id: thread.clone(),
                user_id: run.user_id.clone(),
                seq: conversation.message_count,
                turn_id: notice_id.clone(),
                role: "event".into(),
                text,
                status: "completed".into(),
                error_code: None,
                upstream_error_code: None,
                error_detail: None,
                turn_metadata: None,
                created_at: now,
                activities: vec![],
                attachments: vec![],
                origin: None,
                via: None,
                continuation_of: None,
                retry_of: None,
                specialist_id: None,
                steering: None,
                voice: None,
                execution_pending: false,
            };
            db.collection::<AssistantMessage>(MESSAGES)
                .insert_one(message)
                .session(&mut *session)
                .await?;
        }
    }
    db.collection::<Document>(WORK)
        .insert_one(doc! {
            "_id": &notice_id, "kind": "pause_notice", "trigger_id": &run.trigger_id,
            "run_id": &run.id, "user_id": &run.user_id, "reason": reason,
            "at": date(Utc::now()), "fence": "", "external_claimed": false,
        })
        .session(&mut *session)
        .await?;
    Ok(())
}
