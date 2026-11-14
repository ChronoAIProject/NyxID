//! Serialize the complete provider/cache unit, and retain recovery evidence for
//! PUTs that can outlive their lease or HTTP response. Lago cannot fence a PUT.
use super::lago_client::{LagoApi, ServicePriceSync};
use crate::{
    errors::{AppError, AppResult},
    models::billing_plan_sync::{CHARGE_COLLECTION, PLAN_COLLECTION, PlanCharge},
    services::{
        coordination_service::{
            ClusterLeaseRuntime, LeaseStore, LeaseToken, active_lease_filter, cluster_lease_runtime,
        },
        transaction_service,
    },
};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Document, doc},
};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

mod desired;
mod repair;
pub(super) use repair::repair;

#[derive(Clone, Copy)]
pub(super) enum CacheWrite {
    Rate,
    None,
}

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct Context {
    token: LeaseToken,
    service_id: Option<String>,
    cache_write: CacheWrite,
    repairing: bool,
    dispatching: Arc<AtomicBool>,
    unresolved: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
}
tokio::task_local! { static PLAN_LEASE: Context; }

pub(super) fn lost() -> bool {
    PLAN_LEASE
        .try_with(|context| context.lost.load(Ordering::SeqCst))
        .unwrap_or(true)
}

fn unavailable() -> AppError {
    AppError::BillingProviderUnavailable(
        "Plan synchronization lost its lease; retry pending".into(),
    )
}

/// One acquisition attempt, bounded to one second, limits admin latency. Desired prices are
/// already durable; contention leaves them pending. The caller owns request or
/// background work tracking. Cancellation drops renewal, never releases early.
pub(super) async fn run<T>(
    db: &Database,
    plan: &str,
    service_id: Option<&str>,
    cache_write: CacheWrite,
    operation: impl Future<Output = AppResult<T>>,
) -> AppResult<Option<T>> {
    run_with_runtime(
        db,
        plan,
        service_id,
        cache_write,
        cluster_lease_runtime(),
        operation,
    )
    .await
}

pub(super) async fn run_with_runtime<T>(
    db: &Database,
    plan: &str,
    service_id: Option<&str>,
    cache_write: CacheWrite,
    runtime: &ClusterLeaseRuntime,
    operation: impl Future<Output = AppResult<T>>,
) -> AppResult<Option<T>> {
    let acquired = tokio::time::timeout(
        ACQUIRE_TIMEOUT,
        runtime.acquire(db, &format!("billing:plan:{plan}")),
    )
    .await;
    let Some(token) = acquired.unwrap_or(Ok(None))? else {
        return Ok(None);
    };
    let context = Context {
        token: token.clone(),
        service_id: service_id.map(str::to_owned),
        cache_write,
        repairing: false,
        dispatching: Arc::new(AtomicBool::new(false)),
        unresolved: Arc::new(AtomicBool::new(false)),
        lost: Arc::new(AtomicBool::new(false)),
    };
    let result = runtime
        .run_while_renewed(db, &token, PLAN_LEASE.scope(context.clone(), operation))
        .await;
    match result {
        Some(result) => {
            // Provider mutation and the writer's cache/publication have finished.
            if !context.dispatching.load(Ordering::SeqCst)
                && !context.unresolved.load(Ordering::SeqCst)
                && let Err(error) = LeaseStore::release(db, &token).await
            {
                tracing::warn!(
                    plan_code = plan,
                    error_code = error.error_code(),
                    "Plan lease release failed; waiting for expiry"
                );
            } // An ambiguous provider attempt retains its lease until TTL.
            result.map(Some)
        }
        None => Ok(None), // Dispatch evidence survives; a successor must reconcile it.
    }
}

/// Persist intent and a dirty generation transactionally with the live lease
/// fence, before any provider write. A replaced in-flight marker means an old
/// PUT might still arrive: keep checking even after a matching readback.
pub(super) async fn begin(
    db: &Database,
    plan: &str,
    code: &str,
    input: Option<&ServicePriceSync>,
) -> AppResult<String> {
    let context = PLAN_LEASE
        .try_with(Clone::clone)
        .map_err(|_| unavailable())?;
    if context.dispatching.swap(true, Ordering::SeqCst) {
        context.unresolved.store(true, Ordering::SeqCst);
    }
    let lost = context.lost;
    let token = context.token;
    if token.name != format!("billing:plan:{plan}") {
        return Err(unavailable());
    }
    let attempt = uuid::Uuid::new_v4().to_string();
    let charge = PlanCharge {
        id: uuid::Uuid::new_v4().to_string(),
        plan_code: plan.into(),
        metric_code: code.into(),
        service_id: context.service_id,
        writes_rate_cache: matches!(context.cache_write, CacheWrite::Rate),
        metric_name: input.map_or(code, |i| i.metric_name.as_str()).into(),
        metric_description: input
            .map_or("NyxID managed usage", |i| i.metric_description.as_str())
            .into(),
        credits_per_unit: input.map(|i| i.credits_per_unit.clone()),
        repair_attempts: 0,
        repair_next_at: None,
        repair_failed_at: None,
        repair_reason: None,
    };
    let context = (
        db.clone(),
        token,
        charge,
        attempt.clone(),
        context.repairing,
    );
    let owned = transaction_service::run_db_only(
        db,
        &context,
        |session, (db, token, charge, attempt, repairing)| {
            Box::pin(begin_in_session(
                session, db, token, charge, attempt, *repairing,
            ))
        },
    )
    .await?;
    if !owned {
        lost.store(true, Ordering::SeqCst);
        return Err(unavailable());
    }
    Ok(attempt)
}

async fn fence_in_session(
    session: &mut mongodb::ClientSession,
    db: &Database,
    token: &LeaseToken,
) -> AppResult<bool> {
    let result = db
        .collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
        .update_one(
            active_lease_filter(&token.name, &token.holder, &token.lease_id),
            vec![doc! {"$set":{"updated_at":"$$NOW"}}],
        )
        .session(session)
        .await?;
    Ok(result.matched_count == 1)
}

async fn begin_in_session(
    session: &mut mongodb::ClientSession,
    db: &Database,
    token: &LeaseToken,
    charge: &PlanCharge,
    attempt: &str,
    repairing: bool,
) -> AppResult<bool> {
    if !fence_in_session(session, db, token).await? {
        return Ok(false);
    }
    let mut fields = mongodb::bson::to_document(charge).map_err(|_| unavailable())?;
    fields.remove("_id");
    for field in repair::BACKOFF_FIELDS {
        fields.remove(field);
    }
    // Repair preserves the original source binding; ordinary writers set it.
    if charge.service_id.is_none() {
        fields.remove("service_id");
    }
    let mut update = doc! {"$set":fields,"$setOnInsert":{"_id":&charge.id}};
    if !repairing {
        update.insert("$unset", doc! {"repair_attempts":"", "repair_next_at":""});
    }
    db.collection::<Document>(CHARGE_COLLECTION)
        .update_one(
            doc! {"plan_code":&charge.plan_code,"metric_code":&charge.metric_code},
            update,
        )
        .upsert(true)
        .session(&mut *session)
        .await?;
    db.collection::<Document>(PLAN_COLLECTION)
        .update_one(doc! {"_id":&charge.plan_code}, vec![doc! {"$set":{
            "generation":{"$add":[{"$ifNull":["$generation",0_i64]},1_i64]},
            "dirty":true,
            "uncertain":{"$or":[{"$ifNull":["$uncertain",false]},{"$ne":[{"$ifNull":["$active_attempt",null]},null]}]},
            "active_attempt":{"$literal":attempt},
        }}]).upsert(true).session(session).await?;
    Ok(true)
}

/// Only a completed successful request can remove its own dispatch marker.
/// Even a late completion dirties the plan again, without clearing a successor.
pub(super) async fn completed(db: &Database, plan: &str, attempt: &str) -> AppResult<()> {
    db.collection::<Document>(PLAN_COLLECTION).update_one(doc!{"_id":plan}, vec![doc!{"$set":{
        "generation":{"$add":["$generation",1_i64]}, "dirty":true,
        "active_attempt":{"$cond":[{"$eq":["$active_attempt",{"$literal":attempt}]},"$$REMOVE","$active_attempt"]},
    }}]).await?;
    // Do not let a late holder publish a cache or synced marker.
    let token = PLAN_LEASE
        .try_with(|context| context.token.clone())
        .map_err(|_| unavailable())?;
    let renewal = LeaseStore::renew(db, &token, cluster_lease_runtime().ttl).await;
    if !matches!(renewal, Ok(true)) {
        PLAN_LEASE.with(|context| context.lost.store(true, Ordering::SeqCst));
        renewal?;
        return Err(unavailable());
    }
    PLAN_LEASE.with(|context| context.dispatching.store(false, Ordering::SeqCst));
    Ok(())
}

/// Recovery of an interrupted explicit writer, under the same plan lease.
/// Keep this cache mutation together for the subsequent rate-epoch change.
async fn restore_cache(
    db: &Database,
    code: &str,
    input: Option<&ServicePriceSync>,
) -> AppResult<()> {
    use crate::models::billing_rate_cache::BillingRateCache;
    let Some(input) = input else {
        super::rate_cache::retire(db, code).await?;
        return Ok(());
    };
    let pico = super::amounts::decimal_to_pico(&input.credits_per_unit)
        .ok_or_else(|| AppError::Internal("stored plan price is invalid".into()))?;
    let cache = BillingRateCache {
        id: BillingRateCache::cache_id(code, None),
        lago_metric_code: code.into(),
        model: None,
        credits_per_unit_micros: pico / 1_000_000,
        credits_per_unit_pico: Some(pico),
        synced_at: chrono::Utc::now(),
        retired_at: None,
    };
    super::rate_cache::replace(db, &cache).await?;
    Ok(())
}

pub(super) async fn repair_other_plans(
    db: &Database,
    lago: &dyn LagoApi,
    configured_plan: &str,
) -> AppResult<()> {
    let mut plans = db
        .collection::<Document>(PLAN_COLLECTION)
        .find(doc! {"_id":{"$ne":configured_plan}})
        .await?;
    while let Some(plan) = plans.try_next().await? {
        if !super::reconcile::continue_reconcile() {
            break;
        }
        if let Ok(code) = plan.get_str("_id")
            && let Err(error) = repair(db, lago, code, configured_plan).await
        {
            tracing::warn!(
                plan_code = code,
                error_code = error.error_code(),
                "Plan repair will retry"
            );
        }
    }
    Ok(())
}
