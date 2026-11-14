//! Read-only drift audit; leased, per-code recovery with durable bounded retries.
use super::*;
use std::collections::BTreeSet;

const REPAIR_INITIAL_DELAY_MS: i64 = 5 * 60 * 1000;
const REPAIR_MAX_DELAY_MS: i64 = 24 * 60 * 60 * 1000;
const REPAIR_MAX_EXPONENT: i64 = 9;
pub(super) const BACKOFF_FIELDS: [&str; 4] = [
    "repair_attempts",
    "repair_next_at",
    "repair_failed_at",
    "repair_reason",
];
pub(super) fn backoff_unset() -> Document {
    BACKOFF_FIELDS
        .into_iter()
        .map(|field| (field.into(), "".into()))
        .collect()
}

struct Audit {
    state: Document,
    desired: desired::Snapshot,
    observed: Vec<serde_json::Value>,
    now: chrono::DateTime<chrono::Utc>,
}
impl Audit {
    fn matches(&self, code: &str, input: Option<&ServicePriceSync>) -> bool {
        super::super::lago_client::standard_charge_matches(&self.observed, code, input)
    }
    fn due(&self, code: &str) -> bool {
        self.desired
            .intents
            .get(code)
            .and_then(|i| i.repair_next_at)
            .is_none_or(|next| next <= self.now)
    }
    fn reason(&self, code: &str) -> Option<&str> {
        self.desired
            .intents
            .get(code)
            .and_then(|i| i.repair_reason.as_deref())
    }
    fn needs_work(&self) -> bool {
        completion_pending(&self.state)
            || self.desired.charges.iter().any(|(code, input)| {
                if self.matches(code, input.as_ref()) {
                    self.reason(code)
                        .is_some_and(|r| r != "cache_write_failed" || self.due(code))
                } else {
                    self.due(code)
                }
            })
    }
}
fn completion_pending(state: &Document) -> bool {
    state.get_bool("dirty").unwrap_or(false)
        && state.get_i64("completion_generation").ok() != state.get_i64("generation").ok()
}
async fn load(
    db: &Database,
    lago: &dyn LagoApi,
    plan: &str,
    configured_plan: &str,
) -> AppResult<Option<Audit>> {
    let Some(state) = db
        .collection::<Document>(PLAN_COLLECTION)
        .aggregate(vec![
            doc! {"$match":{"_id":plan}},
            doc! {"$set":{"audit_now":"$$NOW"}},
        ])
        .await?
        .try_next()
        .await?
    else {
        return Ok(None);
    };
    let now = state
        .get_datetime("audit_now")
        .map_err(|_| unavailable())?
        .to_chrono();
    let desired = desired::load(db, plan, configured_plan).await?;
    let observed = lago.plan_charges(plan).await?;
    Ok(Some(Audit {
        state,
        desired,
        observed,
        now,
    }))
}

/// Clean matching plans, and unresolved codes still backing off, take no lease
/// and write nothing. Re-read under the plan lease before acting on this hint.
pub(in crate::services::billing) async fn repair(
    db: &Database,
    lago: &dyn LagoApi,
    plan: &str,
    configured_plan: &str,
) -> AppResult<()> {
    let Some(audit) = load(db, lago, plan, configured_plan).await? else {
        return Ok(());
    };
    if !audit.needs_work() || !super::super::reconcile::continue_reconcile() {
        return Ok(());
    }
    run(db, plan, None, CacheWrite::None, async {
        let Some(audit) = load(db, lago, plan, configured_plan).await? else {
            return Ok(());
        };
        if !audit.needs_work() {
            return Ok(());
        }
        Box::pin(repair_owned(db, lago, plan, audit)).await
    })
    .await?;
    Ok(())
}

async fn repair_owned(
    db: &Database,
    lago: &dyn LagoApi,
    plan: &str,
    audit: Audit,
) -> AppResult<()> {
    let mut failed = BTreeSet::new();
    let mut attempted = BTreeSet::new();
    if completion_pending(&audit.state) {
        super::super::taxes::invalidate_plan(db, plan).await?;
    }
    for (code, input) in &audit.desired.charges {
        if !super::super::reconcile::continue_reconcile() {
            return Ok(());
        }
        if audit.matches(code, input.as_ref()) || !audit.due(code) {
            continue;
        }
        attempted.insert(code.clone());
        let mut context = PLAN_LEASE.with(Clone::clone);
        context.service_id = audit.desired.owners.get(code).cloned();
        context.cache_write = if audit.desired.cache_codes.contains(code) {
            CacheWrite::Rate
        } else {
            CacheWrite::None
        };
        context.repairing = true;
        let result = PLAN_LEASE
            .scope(context, async {
                match input {
                    Some(input) => super::super::taxes::sync_charge(db, lago, plan, input).await,
                    None => super::super::taxes::remove_charge(db, lago, plan, code).await,
                }
            })
            .await;
        if result.is_err() {
            record_failure(db, plan, &audit.desired, code, "provider_repair_unverified").await?;
            failed.insert(code.clone());
        }
    }
    let state = db
        .collection::<Document>(PLAN_COLLECTION)
        .find_one(doc! {"_id":plan})
        .await?
        .ok_or_else(unavailable)?;
    let generation = state.get_i64("generation").map_err(|_| unavailable())?;
    // Capture generation before the full-set readback; a late completion must
    // invalidate our final transaction's generation predicate.
    let observed = lago.plan_charges(plan).await?;
    // A previously uncertain PUT may have completed during backoff. Replay
    // after-invalidation when its matching result first becomes observable.
    let recovered_provider = audit.desired.charges.iter().any(|(code, input)| {
        !failed.contains(code)
            && audit.reason(code) == Some("provider_repair_unverified")
            && super::super::lago_client::standard_charge_matches(&observed, code, input.as_ref())
    });
    let pending = completion_pending(&state) || recovered_provider;
    if pending {
        super::super::taxes::invalidate_plan(db, plan).await?;
    }
    let token = PLAN_LEASE.with(|context| context.token.clone());
    if !LeaseStore::renew(db, &token, cluster_lease_runtime().ttl).await? {
        return Err(unavailable());
    }
    let mut recovered = BTreeSet::new();
    // Attempted but unpublished source prices still belong to their normal
    // retry. Do not claim that their interrupted provider/cache unit completed.
    let mut complete = !audit.desired.intents.iter().any(|(code, intent)| {
        intent.credits_per_unit.is_some() && !audit.desired.charges.contains_key(code)
    });
    for (code, input) in &audit.desired.charges {
        if !super::super::reconcile::continue_reconcile() {
            return Ok(());
        }
        if failed.contains(code) {
            complete = false;
            continue;
        }
        if !super::super::lago_client::standard_charge_matches(&observed, code, input.as_ref()) {
            if attempted.contains(code) {
                record_failure(db, plan, &audit.desired, code, "provider_repair_unverified")
                    .await?;
            }
            complete = false;
            continue;
        }
        if audit.reason(code) == Some("cache_write_failed") && !audit.due(code) {
            complete = false;
            continue;
        }
        if (pending || audit.reason(code).is_some())
            && audit.desired.cache_codes.contains(code)
            && restore_cache(db, code, input.as_ref()).await.is_err()
        {
            record_failure(db, plan, &audit.desired, code, "cache_write_failed").await?;
            complete = false;
            continue;
        }
        if audit.reason(code).is_some() {
            recovered.insert(code.clone());
        }
    }
    let context = (
        db.clone(),
        token,
        plan.to_owned(),
        generation,
        complete,
        recovered,
    );
    transaction_service::run_db_only(
        db,
        &context,
        |session, (db, token, plan, generation, complete, recovered)| {
            Box::pin(finish_in_session(
                session,
                db,
                token,
                plan,
                *generation,
                *complete,
                recovered,
            ))
        },
    )
    .await
}

async fn finish_in_session(
    session: &mut mongodb::ClientSession,
    db: &Database,
    token: &LeaseToken,
    plan: &str,
    generation: i64,
    complete: bool,
    recovered: &BTreeSet<String>,
) -> AppResult<()> {
    if !fence_in_session(session, db, token).await? {
        return Err(unavailable());
    }
    let mut set = doc! {"completion_generation":generation,"dirty":!complete};
    let mut update = Document::new();
    if complete {
        set.insert("verified_generation", generation);
        update.insert("$unset", doc! {"active_attempt":""});
    }
    update.insert("$set", set);
    let result = db
        .collection::<Document>(PLAN_COLLECTION)
        .update_one(doc! {"_id":plan,"generation":generation}, update)
        .session(&mut *session)
        .await?;
    if result.matched_count != 1 {
        return Err(unavailable());
    }
    if !recovered.is_empty() {
        db.collection::<Document>(CHARGE_COLLECTION)
            .update_many(
                doc! {"plan_code":plan,"metric_code":{"$in":recovered.iter().collect::<Vec<_>>()}},
                doc! {"$unset":backoff_unset()},
            )
            .session(session)
            .await?;
    }
    Ok(())
}

async fn record_failure(
    db: &Database,
    plan: &str,
    desired: &desired::Snapshot,
    code: &str,
    reason: &str,
) -> AppResult<()> {
    let token = PLAN_LEASE.with(|context| context.token.clone());
    let input = desired.charges.get(code).and_then(Option::as_ref);
    let fields = doc! {
        "plan_code":plan, "metric_code":code,
        "service_id":desired.owners.get(code),
        "writes_rate_cache":desired.cache_codes.contains(code),
        "metric_name":input.map_or(code, |i| i.metric_name.as_str()),
        "metric_description":input.map_or("NyxID managed usage", |i| i.metric_description.as_str()),
        "credits_per_unit":input.map(|i| i.credits_per_unit.as_str()),
    };
    let context = (
        db.clone(),
        token,
        fields,
        reason.to_owned(),
        uuid::Uuid::new_v4().to_string(),
    );
    let first = transaction_service::run_db_only(
        db,
        &context,
        |session, (db, token, fields, reason, id)| {
            Box::pin(failure_in_session(session, db, token, fields, reason, id))
        },
    )
    .await?;
    if first {
        tracing::error!(
            plan_code = plan,
            metric_code = code,
            reason,
            "Plan repair is unresolved; backing off"
        );
    }
    Ok(())
}
async fn failure_in_session(
    session: &mut mongodb::ClientSession,
    db: &Database,
    token: &LeaseToken,
    fields: &Document,
    reason: &str,
    id: &str,
) -> AppResult<bool> {
    if !fence_in_session(session, db, token).await? {
        return Err(unavailable());
    }
    let plan = fields.get_str("plan_code").map_err(|_| unavailable())?;
    let code = fields.get_str("metric_code").map_err(|_| unavailable())?;
    let mut set: Document = fields
        .iter()
        .map(|(key, value)| (key.clone(), doc! {"$literal":value}.into()))
        .collect();
    set.extend(doc! {
        "_id":{"$ifNull":["$_id",{"$literal":id}]},
        "repair_attempts":{"$add":[{"$ifNull":["$repair_attempts",0_i64]},1_i64]},
        "repair_failed_at":{"$ifNull":["$repair_failed_at","$$NOW"]},
        "repair_reason":{"$literal":reason},
        "repair_next_at":{"$add":["$$NOW",{"$min":[REPAIR_MAX_DELAY_MS,
            {"$multiply":[REPAIR_INITIAL_DELAY_MS,{"$pow":[2_i64,{"$min":[REPAIR_MAX_EXPONENT,{"$ifNull":["$repair_attempts",0_i64]}]}]}]}]}]},
    });
    let previous = db
        .collection::<Document>(CHARGE_COLLECTION)
        .find_one_and_update(
            doc! {"plan_code":plan,"metric_code":code},
            vec![doc! {"$set":set}],
        )
        .upsert(true)
        .session(&mut *session)
        .await?;
    db.collection::<Document>(PLAN_COLLECTION)
        .update_one(doc! {"_id":plan}, doc! {"$set":{"dirty":true}})
        .session(session)
        .await?;
    Ok(previous.is_none_or(|p| p.get_datetime("repair_failed_at").is_err()))
}
