//! Trigger schedules: derived leased work, durable UUID-v5 occurrence identity,
//! and admission fenced in the same transaction as an assistant turn.
pub mod compute;
use super::api_key_mutation_service as transactions;
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
        db.collection::<Document>(WORK)
            .replace_one(
                doc! { "_id": &id },
                doc! {
                    "_id": &id,
                    "kind": "schedule",
                    "trigger_id": &trigger.id,
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
            "at": date(Utc::now()),
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
                let run = new_run(&trigger, at, at + compute::grace(spec, at)?);
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
) -> AppResult<()> {
    if let Some(run) = finish_update(
        db,
        doc! {
            "_id": run_id,
            "outcome": {
                "$in": [
                    RunOutcome::Pending.as_str(),
                    RunOutcome::Started.as_str(),
                    RunOutcome::Waiting.as_str(),
                ],
            },
        },
        outcome,
        reason,
        None,
    )
    .await?
    {
        finish_record(db, &run).await?;
    }
    Ok(())
}

/// Terminal decisions before turn admission belong only to the current worker.
pub async fn finish_claimed(
    db: &Database,
    job: &Document,
    run_id: &str,
    outcome: RunOutcome,
    reason: Option<&str>,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let work_db = db.clone();
    let job = job.clone();
    let id = run_id.to_owned();

    let reason = reason.map(str::to_owned);
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
                finish_update(
                    &work_db,
                    doc! {
                        "_id": &id,
                        "outcome": { "$in": [RunOutcome::Pending.as_str(),RunOutcome::Waiting.as_str()] },
                    },
                    outcome,
                    reason.as_deref(),
                    Some(session),
                )
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
    session: Option<&mut ClientSession>,
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
    Ok(if let Some(session) = session {
        action.session(session).await?
    } else {
        action.await?
    })
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
pub async fn admit_turn(
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
pub async fn run_thread(
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
        Some(super::assistant_team_service::home_thread(db, keys, &agent).await?)
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
                let modified = work_db
                    .collection::<Trigger>(TRIGGERS)
                    .update_one(
                        doc! {
                            "_id": &saved.id,
                            "status": "active",
                            "updated_at": date(saved.updated_at),
                        },
                        doc! {
                            "$set": {
                                "status": "disabled",
                                "schedule_state.pause_reason": &saved_reason,
                                "updated_at": date(Utc::now()),
                            },
                        },
                    )
                    .session(&mut *session)
                    .await?
                    .modified_count
                    == 1;
                if modified {
                    work_db
                        .collection::<Document>(WORK)
                        .delete_one(doc! { "_id": format!("schedule:{}", saved.id) })
                        .session(&mut *session)
                        .await?;
                }
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
