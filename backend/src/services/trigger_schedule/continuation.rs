//! Durable coordinator handoffs. All authority comes from stored run/turn/work
//! identities, never from event text or client input.
use super::*;
use crate::models::{
    assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS},
    assistant_group::COLLECTION_NAME as GROUPS,
    trigger_run::CoordinatorWait,
};
pub const MIGRATIONS: &str = "trigger_migrations";
pub const MIGRATION_ID: &str = "continuation-v2";

pub fn enabled() -> bool {
    std::env::var("AUTOMATION_CONTINUATION_V2_ENABLED").is_ok_and(|v| v == "true")
}

/// Automation wakeups have their own finite budget. The chat event-streak
/// preference must not disable an explicitly scheduled workflow.
pub const AUTOMATION_CONTINUATION_LIMIT: u32 = 32;

/// Flag-off preserves legacy ingress; persisted v2 still requires readiness.
pub fn requires_v2(protocol: u32) -> bool {
    enabled() || protocol >= 2
}

pub async fn ready(db: &Database, flag: bool) -> AppResult<()> {
    if !flag
        || db
            .collection::<Document>(MIGRATIONS)
            .find_one(doc! {"_id": MIGRATION_ID, "completed_at": {"$type": "date"}})
            .await?
            .is_none()
    {
        return Err(AppError::Conflict(
            "automation_continuation_v2_not_ready".into(),
        ));
    }
    Ok(())
}

pub async fn ready_in_session(db: &Database, session: &mut ClientSession) -> AppResult<()> {
    if !enabled()
        || db
            .collection::<Document>(MIGRATIONS)
            .find_one(doc! {"_id": MIGRATION_ID, "completed_at": {"$type": "date"}})
            .session(session)
            .await?
            .is_none()
    {
        return Err(AppError::Conflict(
            "automation_continuation_v2_not_ready".into(),
        ));
    }
    Ok(())
}

/// Returns the exact attributable source. Indexed, bounded existence queries.
/// Used in the settlement transaction so queue/follower writes cannot disappear
/// between the barrier and the waiting transition.
pub async fn source(
    db: &Database,
    run: &TriggerRun,
    session: &mut ClientSession,
) -> AppResult<Option<&'static str>> {
    let Some(thread) = run.thread_id.as_deref() else {
        return Ok(None);
    };
    let Some(conversation) = db
        .collection::<Document>(THREADS)
        .find_one(doc! {"_id": thread, "user_id": &run.user_id})
        .projection(doc! {"pending_events": 1})
        .session(&mut *session)
        .await?
    else {
        // A dangling group follower is not a resumable coordinator source.
        return Ok(None);
    };
    if db
        .collection::<Document>(RUNS)
        .find_one(doc! {"_id":&run.id,"coordinator_capacity_retry":true})
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Ok(Some("event"));
    }

    if conversation
        .get_array("pending_events")
        .is_ok_and(|events| {
            events.iter().any(|event| {
                event.as_document().is_some_and(|event| {
                    event.get_str("trigger_run_id") == Ok(run.id.as_str())
                        && matches!(
                            event.get("continuation_protocol_version"),
                            Some(bson::Bson::Int32(2) | bson::Bson::Int64(2))
                        )
                })
            })
        })
    {
        return Ok(Some("event"));
    }
    if db.collection::<Document>(crate::models::assistant_direct_handoff::COLLECTION_NAME)
        .find_one(doc! {"user_id":&run.user_id,"run_id":&run.id,"parent_thread_id":thread,"state":"pending"})
        .projection(doc! {"_id":1}).session(&mut *session).await?.is_some() {
        return Ok(Some("specialist"));
    }
    if db.collection::<Document>(GROUPS).find_one(doc! {"user_id": &run.user_id, "followers": {"$elemMatch": {"conversation_id": thread, "trigger_run_id": &run.id}}}).projection(doc! {"_id":1}).session(session).await?.is_some() {
        return Ok(Some("group"));
    }
    Ok(None)
}

/// Claims the completion barrier before any terminal projection/delivery.
/// Both dispositions (waiting and no work) fence registration and Stop.
pub async fn mark_coordinator_waiting(db: &Database, run: &TriggerRun) -> AppResult<bool> {
    if run.continuation_protocol_version != 2 {
        return Ok(false);
    }
    let work_db = db.clone();
    let saved = run.clone();
    let mut session = db.client().start_session().await?;
    let (waiting, changed) = session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(
                mark_coordinator_waiting_in_session(&work_db, &saved, session).await,
            )
        })
        .await
        .map_err(transactions::map_transaction_error)?;

    if changed {
        audit(db,&run.user_id,"trigger_run_waiting",doc!{"run_id":&run.id,"trigger_id":&run.trigger_id,"reason":"coordinator_pending","thread_id":&run.thread_id,"deadline":run.coordinator_deadline.map(date)}).await;
    }
    Ok(waiting)
}

/// Shared transaction body also lets the race fixture hold a real stale Mongo
/// snapshot. Every registration and every barrier writes thread then run.
pub(crate) async fn mark_coordinator_waiting_in_session(
    db: &Database,
    run: &TriggerRun,
    session: &mut ClientSession,
) -> AppResult<(bool, bool)> {
    let mut identity = doc! {
        "_id": &run.id,
        "user_id": &run.user_id,
        "outcome": "started",
        "turn_id": &run.turn_id,
        "continuation_protocol_version": 2,
        "result_claimed": {"$ne": true},
        "terminal_result_recorded": {"$ne": true},
    };
    let Some(live) = db
        .collection::<TriggerRun>(RUNS)
        .find_one(identity.clone())
        .session(&mut *session)
        .await?
    else {
        return Ok((true, false));
    };
    let thread = live
        .thread_id
        .as_deref()
        .ok_or_else(|| AppError::Conflict("trigger_thread_missing".into()))?;
    identity.insert("thread_id", thread);

    // These must be actual writes even when coordinator_run_ids already holds
    // this run. A handoff committed after our snapshot then causes a transient
    // write conflict, and and_run2 retries the whole source check.
    if db
        .collection::<Document>(THREADS)
        .update_one(
            doc! {"_id": thread, "user_id": &live.user_id, "$or": [
                {"active_turn": null},
                {"active_turn.turn_id": &live.turn_id, "active_turn.stop_requested": {"$ne": true}},
            ]},
            doc! {"$inc": {"automation_handoff_version": 1_i64},
            "$addToSet": {"coordinator_run_ids": &live.id}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::Conflict("trigger_thread_changed".into()));
    }
    if db
        .collection::<Document>(RUNS)
        .update_one(
            identity.clone(),
            doc! {"$inc": {"handoff_version": 1_i64},
            "$set": {"coordinator_handoff_closed": true}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::Conflict("trigger_run_unavailable".into()));
    }

    // The seal persists across the no-work -> terminal-projection gap. A late
    // registration cannot commit after this decision. admit_turn reopens it
    // only with the next exact run/turn/work claim.
    let Some(source) = source(db, &live, session).await? else {
        return Ok((false, false));
    };
    let wait = CoordinatorWait {
        thread_id: thread.into(),
        source: source.into(),
        deadline_at: live
            .coordinator_deadline
            .unwrap_or(live.deadline + Duration::hours(1))
            .min(live.expires_at),
    };
    if db
        .collection::<TriggerRun>(RUNS)
        .update_one(
            identity,
            doc! {"$set": {
                "outcome": "waiting", "wait_kind": "coordinator",
                "coordinator_wait": bson::to_bson(&wait)
                    .map_err(|_| AppError::Internal("Encode coordinator wait".into()))?,
                "waiting_for": [], "reason": "coordinator_pending",
            }},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::Conflict("trigger_run_unavailable".into()));
    }
    db.collection::<Document>(WORK)
        .update_one(
            doc! {"_id": &live.id},
            doc! {"$set": {"at": date(Utc::now()), "fence": "", "deferrals": 0}},
        )
        .session(session)
        .await?;
    Ok((true, true))
}

pub fn exhausted(run: &TriggerRun, now: DateTime<Utc>) -> Option<&'static str> {
    if run.coordinator_deadline.is_none_or(|at| at <= now) {
        Some("coordinator_timeout")
    } else if run.continuation_count >= run.continuation_limit {
        Some("continuation_limit_exceeded")
    } else {
        None
    }
}

/// Used by the event sweep only. The scheduler already owns its work lease and
/// passes that claim directly. Admission revalidates all identities transactionally.
pub async fn claim_event(
    db: &Database,
    owner: &str,
    thread: &str,
    id: &str,
) -> AppResult<Option<TurnClaim>> {
    if !enabled() {
        return Ok(None);
    }
    match ready(db, true).await {
        Ok(()) => {}
        Err(AppError::Conflict(_)) => return Ok(None),
        Err(error) => return Err(error),
    }
    let Some(run) = db.collection::<TriggerRun>(RUNS).find_one(doc! {"_id":id,"user_id":owner,"thread_id":thread,"outcome":"waiting","wait_kind":"coordinator","continuation_protocol_version":2}).await? else {return Ok(None)};
    if exhausted(&run, Utc::now()).is_some() {
        return Ok(None);
    }
    let Some(trigger) = db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! {"_id": &run.trigger_id, "user_id":owner,"status":"active"})
        .await?
    else {
        return Ok(None);
    };
    if run.trigger_generation != Some(trigger.updated_at) {
        return Ok(None);
    }
    let Some(job) = claim(db, id, Utc::now()).await? else {
        return Ok(None);
    };
    Ok(Some(TurnClaim {
        run_id: run.id,
        fence: job.get_str("fence").unwrap_or_default().into(),
        trigger_updated_at: trigger.updated_at,
        continuation: true,
    }))
}

/// Operator readiness uses actual persisted leases, not a replica-version claim.
/// `drain_started_at` is an explicit assertion that both ingress paths and ALL
/// workers were stopped then. A migration invocation while enabled is refused.
pub async fn migrate_continuation_v2(
    db: &Database,
    drain_started_at: DateTime<Utc>,
) -> AppResult<Document> {
    migrate_checked(db, drain_started_at, enabled()).await
}

pub(crate) async fn migrate_checked(
    db: &Database,
    drain_started_at: DateTime<Utc>,
    flag: bool,
) -> AppResult<Document> {
    if flag || Utc::now() < drain_started_at + Duration::seconds(2 * LEASE_SECONDS) {
        return Err(AppError::Conflict("automation_drain_required".into()));
    }
    if db
        .collection::<Document>(WORK)
        .find_one(doc! {"fence":{"$nin":["",null]},"at":{"$gt":date(Utc::now())}})
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("automation_work_leases_active".into()));
    }
    let fence = Uuid::new_v4().to_string();
    db.collection::<Document>(MIGRATIONS).update_one(doc!{"_id":MIGRATION_ID},doc!{"$setOnInsert":{"classified":0_i64,"triggers":0_i64,"ambiguous":[],"ambiguous_count":0_i64}}).upsert(true).await?;
    if db.collection::<Document>(MIGRATIONS).find_one_and_update(doc!{"_id":MIGRATION_ID,"$or":[{"lease_until":null},{"lease_until":{"$lte":date(Utc::now())}}]},doc!{"$set":{"fence":&fence,"lease_until":date(Utc::now()+Duration::minutes(5)),"drain_started_at":date(drain_started_at)},"$unset":{"completed_at":""}}).await?.is_none() {return Err(AppError::Conflict("automation_migration_busy".into()))}
    let mut cursor = String::new();
    loop {
        let batch:Vec<Document>=db.collection::<Document>(RUNS).find(doc!{"_id":{"$gt":&cursor},"outcome":{"$in":["pending","started","waiting"]},"continuation_protocol_version":{"$in":[null,0]}}).projection(doc!{"_id":1}).sort(doc!{"_id":1}).limit(100).await?.try_collect().await?;
        if batch.is_empty() {
            break;
        }
        for candidate in batch {
            cursor = candidate
                .get_str("_id")
                .map_err(|_| AppError::Internal("Migration run identity missing".into()))?
                .into();
            let context = (db.clone(), cursor.clone(), fence.clone());
            // Terminal audit is part of classification. Retry definite audit
            // head/owner-fence collisions as well as Mongo write conflicts;
            // unknown commits retry COMMIT only, never terminal side effects.
            super::super::transaction_service::run_db_only(
                db,
                &context,
                |session, (db, id, fence)| Box::pin(migrate_run_in_session(db, id, fence, session)),
            )
            .await?;
        }
    }
    if db
        .collection::<Document>(RUNS)
        .find_one(doc! {
            "outcome": {"$in": ["pending", "started", "waiting"]},
            "continuation_protocol_version": {"$nin": [null, 0, 2]},
        })
        .projection(doc! {"_id": 1})
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "automation_migration_protocol_unsupported".into(),
        ));
    }
    let mut cursor = String::new();
    loop {
        let batch:Vec<Document>=db.collection::<Document>(TRIGGERS).find(doc!{"_id":{"$gt":&cursor},"status":"active","delivery.type":"assistant","continuation_protocol_version":{"$ne":2}}).projection(doc!{"_id":1}).sort(doc!{"_id":1}).limit(100).await?.try_collect().await?;
        if batch.is_empty() {
            break;
        }
        let ids: Vec<&str> = batch.iter().filter_map(|r| r.get_str("_id").ok()).collect();
        cursor = ids.last().copied().unwrap_or_default().into();
        let work_db = db.clone();
        let fence = fence.clone();
        let ids: Vec<String> = ids.into_iter().map(str::to_owned).collect();
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run2(async move |session| {
                let result: AppResult<()> = async {
                    renew_migration(&work_db, &fence, session).await?;
                    let n = work_db
                        .collection::<Document>(TRIGGERS)
                        .update_many(
                            doc! {"_id":{"$in":&ids},"continuation_protocol_version":{"$ne":2}},
                            doc! {"$set":{"continuation_protocol_version":2}},
                        )
                        .session(&mut *session)
                        .await?
                        .modified_count;
                    work_db
                        .collection::<Document>(MIGRATIONS)
                        .update_one(
                            doc! {"_id":MIGRATION_ID,"fence":&fence},
                            doc! {"$inc":{"triggers":n as i64}},
                        )
                        .session(&mut *session)
                        .await?;
                    Ok(())
                }
                .await;
                transactions::transaction_result(result)
            })
            .await
            .map_err(transactions::map_transaction_error)?;
    }
    db.collection::<Document>(MIGRATIONS)
        .find_one_and_update(
            doc! {"_id":MIGRATION_ID,"fence":fence,"lease_until":{"$gt":date(Utc::now())}},
            doc! {"$set":{"completed_at":date(Utc::now())},"$unset":{"lease_until":""}},
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("automation_migration_lease_lost".into()))
}

/// One classification unit. Exposed within the crate so the regression can
/// abort this real transaction body before retrying the operator migration.
pub(crate) async fn migrate_run_in_session(
    db: &Database,
    id: &str,
    fence: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    renew_migration(db, fence, session).await?;
    let Some(mut run) = db
        .collection::<TriggerRun>(RUNS)
        .find_one(
            doc! {"_id": id, "outcome": {"$in": ["pending", "started", "waiting"]},
            "continuation_protocol_version": {"$ne": 2}},
        )
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    let Some(trigger) = db
        .collection::<Trigger>(TRIGGERS)
        .find_one(doc! {"_id": &run.trigger_id, "user_id": &run.user_id})
        .session(&mut *session)
        .await?
    else {
        // The batch cursor must not hide an orphan or changed target behind a
        // completed readiness marker. Reconcile the target and retry; progress
        // for this run remains uncommitted.
        return Err(AppError::Conflict(
            "automation_migration_trigger_unavailable".into(),
        ));
    };
    if !matches!(&trigger.delivery, TriggerDelivery::Assistant { .. }) {
        // Pending non-assistant deliveries remain reachable through the legacy
        // scheduler and need no continuation stamp. An in-flight assistant
        // whose target changed must be reconciled before readiness is sealed.
        if run.outcome == RunOutcome::Pending {
            return Ok(());
        }
        return Err(AppError::Conflict(
            "automation_migration_trigger_unavailable".into(),
        ));
    }
    run.continuation_protocol_version = 2;
    run.continuation_limit = AUTOMATION_CONTINUATION_LIMIT;
    run.coordinator_deadline = Some(
        run.coordinator_deadline
            .unwrap_or(run.deadline + Duration::hours(1))
            .min(run.expires_at),
    );
    if run.trigger_generation.is_none() {
        run.trigger_generation = Some(trigger.updated_at);
    }
    let mut terminal_reason = None;
    if run.outcome == RunOutcome::Waiting {
        if !run.waiting_for.is_empty() {
            if db
                .collection::<Document>(THREADS)
                .find_one(doc! {"_id": &run.thread_id, "user_id": &run.user_id})
                .projection(doc! {"_id": 1})
                .session(&mut *session)
                .await?
                .is_some()
            {
                run.wait_kind = Some(crate::models::trigger_run::RunWaitKind::Confirmation);
                run.coordinator_wait = None;
            } else {
                terminal_reason = Some("thread_deleted");
            }
        } else if let Some(source) = source(db, &run, session).await? {
            run.wait_kind = Some(crate::models::trigger_run::RunWaitKind::Coordinator);
            run.reason = Some("coordinator_pending".into());
            let thread = run
                .thread_id
                .as_deref()
                .ok_or_else(|| AppError::Conflict("trigger_thread_missing".into()))?;
            run.coordinator_wait = Some(CoordinatorWait {
                thread_id: thread.into(),
                source: source.into(),
                deadline_at: run.coordinator_deadline.expect("initialized deadline"),
            });
        } else {
            terminal_reason = Some("legacy_wait_unclassified");
        }
    }
    if terminal_reason.is_none()
        && matches!(run.outcome, RunOutcome::Started | RunOutcome::Waiting)
        && let Some(thread) = &run.thread_id
    {
        db.collection::<Document>(THREADS)
            .update_one(
                doc! {"_id": thread, "user_id": &run.user_id},
                doc! {"$addToSet": {"coordinator_run_ids": &run.id}},
            )
            .session(&mut *session)
            .await?;
    }
    // Preserve other writers' fields and internal handoff/retry markers. The
    // terminal outcome below belongs exclusively to the normal finish path.
    let changed: Document = bson::to_document(&run)
        .map_err(|_| AppError::Internal("Encode migrated automation run".into()))?
        .into_iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "continuation_protocol_version"
                    | "continuation_limit"
                    | "continuation_count"
                    | "coordinator_deadline"
                    | "trigger_generation"
                    | "wait_kind"
                    | "coordinator_wait"
                    | "reason"
            )
        })
        .collect();
    if db
        .collection::<Document>(RUNS)
        .update_one(
            doc! {"_id": &run.id, "user_id": &run.user_id, "outcome": run.outcome.as_str(),
            "continuation_protocol_version": {"$ne": 2}},
            doc! {"$set": changed},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::Conflict(
            "automation_migration_run_changed".into(),
        ));
    }
    if let Some(reason) = terminal_reason {
        // Stamp v2 before normal terminal cleanup reads the run. Terminal
        // state, history, audit, work removal and progress commit together.
        let finished = Box::pin(super::finish_update(
            db,
            doc! {"_id": &run.id, "user_id": &run.user_id, "trigger_id": &run.trigger_id,
            "outcome": "waiting", "turn_id": &run.turn_id, "continuation_protocol_version": 2},
            RunOutcome::Failed,
            Some(reason),
            "",
            session,
        ))
        .await?
        .ok_or_else(|| AppError::Conflict("automation_migration_run_changed".into()))?;
        Box::pin(super::finish_record_with_session(
            db,
            &finished,
            Some(&mut *session),
        ))
        .await?;
        // Keep bounded recent IDs and a stable metadata-only row per ambiguity.
        db.collection::<Document>(MIGRATIONS)
            .update_one(
                doc! {"_id": MIGRATION_ID, "fence": fence},
                doc! {"$push": {"ambiguous": {"$each": [&run.id], "$slice": -1000}},
                "$inc": {"ambiguous_count": 1_i64}},
            )
            .session(&mut *session)
            .await?;
        db.collection::<Document>(MIGRATIONS)
            .update_one(
                doc! {"_id": format!("continuation-v2:ambiguous:{}", run.id)},
                doc! {"$setOnInsert": {"run_id": &run.id, "reason": reason}},
            )
            .upsert(true)
            .session(&mut *session)
            .await?;
    }
    db.collection::<Document>(MIGRATIONS)
        .update_one(
            doc! {"_id": MIGRATION_ID, "fence": fence},
            doc! {"$inc": {"classified": 1_i64}},
        )
        .session(session)
        .await?;
    Ok(())
}

async fn renew_migration(db: &Database, fence: &str, session: &mut ClientSession) -> AppResult<()> {
    if db
        .collection::<Document>(MIGRATIONS)
        .update_one(
            doc! {"_id":MIGRATION_ID,"fence":fence,"lease_until":{"$gt":date(Utc::now())}},
            doc! {"$set":{"lease_until":date(Utc::now()+Duration::minutes(5))}},
        )
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::Conflict("automation_migration_lease_lost".into()));
    }
    Ok(())
}

/// Rollback is permitted only after every nonterminal v2 run, bound event and
/// follower has drained. Disabling the flag alone never makes old readers safe.
pub async fn rollback_ready(db: &Database) -> AppResult<bool> {
    Ok(db.collection::<Document>(RUNS).find_one(doc!{"continuation_protocol_version":2,"outcome":{"$in":["pending","started","waiting"]}}).projection(doc!{"_id":1}).await?.is_none()
        && db.collection::<Document>(THREADS).find_one(doc!{"pending_events.trigger_run_id":{"$type":"string"}}).projection(doc!{"_id":1}).await?.is_none()
        && db.collection::<Document>(GROUPS).find_one(doc!{"followers.trigger_run_id":{"$type":"string"}}).projection(doc!{"_id":1}).await?.is_none()
        && db.collection::<Document>(crate::models::assistant_direct_handoff::COLLECTION_NAME).find_one(doc!{"state":"pending"}).projection(doc!{"_id":1}).await?.is_none())
}

pub async fn cleanup_in_session(
    db: &Database,
    run: &TriggerRun,
    session: &mut ClientSession,
) -> AppResult<()> {
    if run.continuation_protocol_version != 2 {
        return Ok(());
    }
    if let Some(thread) = &run.thread_id {
        db.collection::<Document>(THREADS).update_one(doc!{"_id":thread,"user_id":&run.user_id},doc!{"$pull":{"pending_events":{"trigger_run_id":&run.id},"coordinator_run_ids":&run.id}}).session(&mut *session).await?;
    }
    db.collection::<Document>(crate::models::assistant_direct_handoff::COLLECTION_NAME)
        .update_many(
            doc! {"user_id":&run.user_id,"run_id":&run.id,"state":"pending"},
            doc! {"$set":{"state":"cancelled"}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<Document>(GROUPS)
        .update_many(
            doc! {"user_id":&run.user_id,"followers.trigger_run_id":&run.id},
            doc! {"$pull":{"followers":{"trigger_run_id":&run.id}}},
        )
        .session(session)
        .await?;
    Ok(())
}

/// An active turn owns only its own run. An idle Stop selects the current
/// waiting set inside the transaction, never from a pre-transaction snapshot.
pub async fn cancel_thread(db: &Database, row: &AssistantConversation) -> AppResult<()> {
    let work_db = db.clone();
    let saved = row.clone();
    let mut session = db.client().start_session().await?;
    let runs = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<Vec<TriggerRun>> = Box::pin(async {
                let mut filter = doc! {"_id":&saved.id,"user_id":&saved.user_id};
                if let Some(turn) = &saved.active_turn {
                    filter.insert("active_turn.turn_id", &turn.turn_id);
                    filter.insert("active_turn.stop_requested", true);
                } else {
                    filter.insert("active_turn", bson::Bson::Null);
                }
                // Real write serializes both idle membership and exact active-turn
                // ownership against admission, registration, and completion.
                let Some(current) = work_db
                    .collection::<AssistantConversation>(THREADS)
                    .find_one_and_update(filter, doc! {"$inc":{"automation_handoff_version":1_i64}})
                    .session(&mut *session)
                    .await?
                else {
                    return Ok(Vec::new());
                };
                let ids = match &current.active_turn {
                    Some(turn) => turn.trigger_run_id.iter().cloned().collect(),
                    None => current.coordinator_run_ids,
                };
                let mut finished = Vec::new();
                for id in ids {
                    if let Some(run) = super::finish_update(
                        &work_db,
                        doc! {
                            "_id":id,"user_id":&saved.user_id,"thread_id":&saved.id,
                            "outcome":{"$in":["waiting","started"]},
                        },
                        RunOutcome::Failed,
                        Some("cancelled"),
                        "",
                        session,
                    )
                    .await?
                    {
                        finished.push(run);
                    }
                }
                Ok(finished)
            })
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    for run in runs {
        super::finish_record(db, &run).await?;
    }
    Ok(())
}

/// Deletion has conversation-wide scope, unlike Stop of an unrelated turn.
/// Called before the thread is removed, in the deletion transaction itself.
pub async fn delete_thread_in_session(
    db: &Database,
    owner: &str,
    thread: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(THREADS)
        .update_one(
            doc! {"_id":thread,"user_id":owner},
            doc! {"$inc":{"automation_handoff_version":1_i64}},
        )
        .session(&mut *session)
        .await?;
    let mut cursor = db
        .collection::<TriggerRun>(RUNS)
        .find(doc! {
            "user_id":owner,"thread_id":thread,"outcome":{"$in":["pending","started","waiting"]},
        })
        .session(&mut *session)
        .await?;
    let runs: Vec<TriggerRun> = cursor.stream(&mut *session).try_collect().await?;
    for run in runs {
        if let Some(finished) = super::finish_update(
            db,
            doc! {"_id":&run.id,"outcome":{"$in":["pending","started","waiting"]}},
            RunOutcome::Failed,
            Some("thread_deleted"),
            "",
            session,
        )
        .await?
        {
            super::finish_record_with_session(db, &finished, Some(&mut *session)).await?;
        }
    }
    Ok(())
}

pub async fn defer_group_capacity(
    db: &Database,
    owner: &str,
    thread: &str,
    run: &str,
) -> AppResult<()> {
    db.collection::<Document>(RUNS)
        .update_one(
            doc! {"_id":run,"user_id":owner,"thread_id":thread,"outcome":"started",
            "coordinator_handoff_closed":{"$ne":true},"result_claimed":{"$ne":true},
            "terminal_result_recorded":{"$ne":true}},
            doc! {"$set":{"coordinator_capacity_retry":true}},
        )
        .await?;
    Ok(())
}

pub async fn queue_capacity_retry(db: &Database, run: &TriggerRun) -> AppResult<()> {
    if db
        .collection::<Document>(RUNS)
        .find_one(doc! {"_id":&run.id,"coordinator_capacity_retry":true})
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(());
    }
    let Some(thread) = run.thread_id.as_deref() else {
        return Ok(());
    };
    let mut event=super::super::assistant_team_service::bound_event("group_follower_capacity","The group's follower capacity was full. Retry the group post before advancing the automation; no work was posted by that attempt.".into(),None,None).for_trigger(Some(&run.id));
    event.id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "group-capacity:{}:{}",
            run.id,
            run.turn_id.as_deref().unwrap_or_default()
        )
        .as_bytes(),
    )
    .to_string();
    super::super::assistant_nyxagent::push_events(db, &run.user_id, thread, vec![event]).await?;
    Ok(())
}
