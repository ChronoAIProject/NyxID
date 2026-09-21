//! Bounded, newest-first, exactly-once hourly fold, including automatic legacy
//! discovery. The durable batch is the claim. Replicas may help finish it;
//! monotonic per-summary sequence fences make every write replay-safe.
//!
//! Commit order: persist immutable batch -> apply fenced increments -> mark
//! sources -> advance the journal. Never retire a batch before all three writes
//! are acknowledged. This protocol works on standalone MongoDB as well as replica
//! sets and does not rely on leases, process clocks, or an evictable dedupe list.
use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Timelike, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, Bson, Document, doc},
    options::IndexOptions,
};
use sha2::{Digest, Sha256};

use crate::services::admin_usage_service::{meter_flags, meter_group};
use crate::{
    config::AppConfig,
    errors::{AppError, AppResult},
    models::{
        usage_meter::COLLECTION_NAME as METERS,
        usage_rollup_hourly::COLLECTION_NAME as ROLLUPS,
        usage_rollup_state::{
            COLLECTION_NAME as STATE, STATE_ID, UsageRollupBatch, UsageRollupState,
        },
    },
};

pub const BATCH_SIZE: i64 = 2_000;
const MAX_BATCHES_PER_TICK: usize = 100;
const TICK_BUDGET: Duration = Duration::from_secs(20);
pub const MEASURES: &[&str] = &[
    "quantity",
    "events",
    "requests",
    "exact_cost_events",
    "legacy_cost_events",
    "legacy_quantity",
    "legacy_allowance_quantity",
    "legacy_grant",
    "gross_cost_micros",
    "wallet_cost_micros",
    "grant_cost_micros",
    "allowance_cost_micros",
    "prompt_tokens",
    "completion_tokens",
    "cached_tokens",
    "cache_creation_tokens",
    "rows_folded",
];
pub const DIMENSIONS: &[&str] = &[
    "actor",
    "owner",
    "service_id",
    "service_slug",
    "class",
    "metric",
    "code",
    "layer",
    "model",
    "billable",
];

pub fn hour(time: DateTime<Utc>) -> DateTime<Utc> {
    time.with_minute(0)
        .unwrap()
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap()
}

/// A full equality prefix includes missing pre-deployment markers as null.
/// Unlike a partial missing-field index (unsupported by MongoDB), this index
/// serves backfill and the live tail from the first deployment onward.
pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    let meters = db.collection::<Document>(METERS);
    for (name, keys) in [
        (
            "usage_rollup_pending_window",
            doc! { "rollup_pending": 1, "status": 1, "created_at": -1 },
        ),
        (
            "usage_rollup_actor_window",
            doc! { "actor_user_id": 1, "created_at": -1 },
        ),
    ] {
        meters
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().name(name.to_string()).build())
                    .build(),
            )
            .await?;
    }
    for keys in [
        doc! { "hour": 1 },
        doc! { "actor": 1, "hour": 1 },
        doc! { "owner": 1, "hour": 1 },
    ] {
        db.collection::<Document>(ROLLUPS)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    // Most summaries contain one display partition and use the main hour
    // index. This small partial index prevents scanning them a second time
    // when expanding multi-partition (or older unaccelerated) summaries.
    db.collection::<Document>(ROLLUPS)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "hour": 1 })
                .options(
                    IndexOptions::builder()
                        .name("usage_rollup_partitioned_hour".to_owned())
                        .partial_filter_expression(doc! { "single_display_key": null })
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub fn pending_filter(cutoff: DateTime<Utc>) -> Document {
    doc! {
        "rollup_pending": { "$in": [true, null] },
        "status": { "$in": ["finalized", "dead_letter"] },
        "created_at": { "$lt": bson::DateTime::from_chrono(cutoff) },
        "quantity": { "$ne": null },
        "$and": [
            { "$or": [{ "status": "finalized" }, { "forwarded": true }] },
            // Finalization fixes free-row measurements. Charged rows remain live
            // until wallet release and funding settlement have both completed.
            // Old released rows without a funding object are immutable legacy.
            { "$or": [ { "wallet_id": null }, { "released": true, "$and": [{ "$or": [ { "funding": null }, { "funding.settled": true } ] }, { "$or": [{ "lago_acked": true }, { "status": "dead_letter" }] }] } ] },
        ],
    }
}

pub async fn state(db: &Database) -> AppResult<Option<UsageRollupState>> {
    Ok(db
        .collection::<UsageRollupState>(STATE)
        .find_one(doc! { "_id": STATE_ID })
        .await?)
}

async fn initialize(db: &Database) -> AppResult<()> {
    let result = db.collection::<Document>(STATE).update_one(doc! { "_id": STATE_ID }, doc! { "$setOnInsert": {
        "sequence": 0_i64, "batch": null, "rolled_up_through": bson::DateTime::from_chrono(DateTime::UNIX_EPOCH),
    } }).upsert(true).await;
    match result {
        Ok(_) => Ok(()),
        Err(error) if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000) => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

async fn claim(
    db: &Database,
    current: &UsageRollupState,
    cutoff: DateTime<Utc>,
) -> AppResult<Option<UsageRollupBatch>> {
    let rows: Vec<Document> = db
        .collection::<Document>(METERS)
        .find(pending_filter(cutoff))
        .sort(doc! { "created_at": -1 })
        .projection(doc! { "_id": 1 })
        .limit(BATCH_SIZE)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        // Only a gap-free sweep can move the watermark; never infer it from the
        // newest folded row during newest-first history backfill.
        let remaining = db.collection::<Document>(METERS).find_one(doc! {
            "rollup_pending": { "$in": [true, null] }, "status": { "$in": ["finalized", "dead_letter"] },
            "quantity": { "$ne": null }, "created_at": { "$lt": bson::DateTime::from_chrono(cutoff) },
            "$or": [{ "status": "finalized" }, { "forwarded": true }],
        }).projection(doc! { "created_at": 1 }).sort(doc! { "created_at": 1 }).await?;
        let watermark = remaining
            .and_then(|r| r.get_datetime("created_at").ok().copied())
            .map(|t| hour(t.to_chrono()))
            .unwrap_or(cutoff);
        db.collection::<Document>(STATE)
            .update_one(
                doc! { "_id": STATE_ID, "sequence": current.sequence, "batch": null },
                doc! { "$set": { "rolled_up_through": bson::DateTime::from_chrono(watermark) } },
            )
            .await?;
        return Ok(None);
    }
    let ids: Vec<String> = rows
        .iter()
        .map(|r| {
            r.get_str("_id")
                .map(str::to_owned)
                .map_err(|e| AppError::Internal(e.to_string()))
        })
        .collect::<AppResult<_>>()?;
    let mut group = meter_group(true);
    let key = group.get_document_mut("_id").expect("literal group key");
    key.insert(
        "hour",
        doc! { "$dateTrunc": { "date": "$created_at", "unit": "hour", "timezone": "UTC" } },
    );
    key.insert("exact", "$exact");
    group.insert("rows_folded", doc! { "$sum": 1_i64 });
    let mut groups: Vec<Document> = db
        .collection::<Document>(METERS)
        .aggregate(vec![
            doc! { "$match": { "_id": { "$in": &ids } } },
            meter_flags(),
            doc! { "$group": group },
        ])
        .await?
        .try_collect()
        .await?;
    let mut combined = std::collections::BTreeMap::<String, Document>::new();
    for group in &mut groups {
        let Bson::Document(mut key) = group.remove("_id").expect("group key") else {
            unreachable!()
        };
        let mut partition_key = Document::new();
        for field in ["api_key", "acked"] {
            if let Some(value) = key.remove(field) {
                partition_key.insert(field, value);
            }
        }
        let hash = hex::encode(Sha256::digest(
            bson::to_vec(&key).map_err(|e| AppError::Internal(e.to_string()))?,
        ));
        group.extend(key);
        group.insert("_id", &hash);
        group.insert("last_batch", current.sequence + 1);
        // Mongo sums of exact micros use Decimal128. Clamp as integers before
        // persistence, never by round-tripping through floating point.
        for field in MEASURES {
            if let Some(Bson::Decimal128(value)) = group.get(*field) {
                let value = value
                    .to_string()
                    .parse::<i128>()
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                group.insert(
                    *field,
                    value.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
                );
            }
        }
        // Partition keys are encoded as hex BSON to be safe Mongo field names.
        // Values retain the exact original group shape, including missing ack.
        let mut partitions = Document::new();
        if group.get_bool("billable").unwrap_or(false) {
            let encoded = hex::encode(
                bson::to_vec(&partition_key).map_err(|e| AppError::Internal(e.to_string()))?,
            );
            let mut partition = doc! { "key": partition_key };
            for field in MEASURES {
                partition.insert(*field, group.get(*field).cloned().unwrap_or(Bson::Int64(0)));
            }
            partitions.insert(encoded, partition);
        }
        group.insert("cost_partitions", partitions.clone());
        if let Some(existing) = combined.get_mut(&hash) {
            for field in MEASURES {
                let value =
                    integer(existing.get(*field)).saturating_add(integer(group.get(*field)));
                existing.insert(*field, value);
            }
            existing
                .get_document_mut("cost_partitions")
                .expect("partitions")
                .extend(partitions);
        } else {
            combined.insert(hash, group.clone());
        }
    }
    let increments = combined
        .into_values()
        .map(|group| bson::from_document(group).map_err(|e| AppError::Internal(e.to_string())))
        .collect::<AppResult<Vec<_>>>()?;
    let batch = UsageRollupBatch {
        sequence: current.sequence + 1,
        row_ids: ids,
        increments,
        claimed_at: Utc::now(),
    };
    let result = db
        .collection::<Document>(STATE)
        .update_one(
            doc! { "_id": STATE_ID, "sequence": current.sequence, "batch": null },
            doc! { "$set": {
                "batch": bson::to_bson(&batch).map_err(|e| AppError::Internal(e.to_string()))?,
            } },
        )
        .await?;
    Ok((result.modified_count == 1).then_some(batch))
}

fn integer(value: Option<&Bson>) -> i64 {
    match value {
        Some(Bson::Int32(n)) => i64::from(*n),
        Some(Bson::Int64(n)) => *n,
        _ => 0,
    }
}
fn saturated_add(existing: Bson, delta: Bson) -> Bson {
    doc! { "$toLong": { "$min": [i64::MAX, { "$add": [
        { "$toDecimal": { "$ifNull": [existing, 0_i64] } },
        { "$toDecimal": { "$ifNull": [delta, 0_i64] } },
    ] }] } }
    .into()
}

async fn write_command(
    db: &Database,
    mut command: Document,
    session: Option<&mut mongodb::ClientSession>,
) -> AppResult<()> {
    if session.is_none() {
        command.insert("writeConcern", doc! { "w": "majority", "j": true });
    }
    let command = db.run_command(command);
    let result = match session {
        Some(session) => command.session(session).await?,
        None => command.await?,
    };
    if result
        .get_array("writeErrors")
        .is_ok_and(|errors| !errors.is_empty())
        || result.contains_key("writeConcernError")
    {
        return Err(AppError::Internal(
            "Usage rollup batch write failed; durable batch retained for retry".into(),
        ));
    }
    Ok(())
}

async fn apply(
    db: &Database,
    batch: &UsageRollupBatch,
    mut session: Option<&mut mongodb::ClientSession>,
) -> AppResult<()> {
    let mut inserts = Vec::with_capacity(batch.increments.len());
    let mut deltas = Document::new();
    let mut ids = Vec::with_capacity(batch.increments.len());
    for increment in &batch.increments {
        let mut initial =
            bson::to_document(increment).map_err(|e| AppError::Internal(e.to_string()))?;
        initial.remove("_id");
        deltas.insert(&increment.id, initial.clone());
        ids.push(increment.id.clone());
        for field in MEASURES {
            initial.insert(*field, 0_i64);
        }
        initial.insert("cost_partitions", Document::new());
        initial.insert("last_batch", 0_i64);
        inserts.push(doc! { "q": { "_id": &increment.id }, "u": { "$setOnInsert": initial }, "upsert": true });
    }
    // Small idempotent initializers avoid generating a distinct arithmetic
    // program for each summary. The shared update below is compiled once.
    for chunk in inserts.chunks(100) {
        write_command(
            db,
            doc! { "update": ROLLUPS, "updates": chunk.to_vec(), "ordered": false },
            session.as_deref_mut(),
        )
        .await?;
    }
    let mut set = doc! { "last_batch": batch.sequence };
    for field in MEASURES {
        set.insert(
            *field,
            saturated_add(format!("${field}").into(), format!("$delta.{field}").into()),
        );
    }
    let mut partition = doc! { "key": "$$this.v.key" };
    for field in MEASURES {
        partition.insert(*field, saturated_add(
            doc! { "$getField": { "field": *field, "input": { "$getField": { "field": "$$this.k", "input": { "$ifNull": ["$cost_partitions", {}] } } } } }.into(),
            format!("$$this.v.{field}").into(),
        ));
    }
    set.insert(
        "cost_partitions",
        doc! { "$arrayToObject": { "$concatArrays": [
            { "$objectToArray": { "$ifNull": ["$cost_partitions", {}] } },
            { "$map": { "input": { "$objectToArray": "$delta.cost_partitions" },
                "in": { "k": "$$this.k", "v": partition } } },
        ] } },
    );
    let mut display_key = Document::new();
    for field in meter_group(true)
        .get_document("_id")
        .expect("group key")
        .keys()
    {
        display_key.insert(
            field,
            match field.as_str() {
                "api_key" => doc! { "$ifNull": ["$$part.v.key.api_key", null] }.into(),
                "acked" => Bson::String("$$part.v.key.acked".into()),
                _ => Bson::String(format!("${field}")),
            },
        );
    }
    let mut query_costs = Document::new();
    for field in [
        "gross_cost_micros",
        "wallet_cost_micros",
        "grant_cost_micros",
        "allowance_cost_micros",
    ] {
        query_costs.insert(field, doc! { "$toDecimal": format!("${field}") });
    }
    let accelerators = doc! {
        "single_display_key": { "$let": {
            "vars": { "part": { "$arrayElemAt": [{ "$objectToArray": "$cost_partitions" }, 0] } },
            "in": { "$cond": [ { "$lte": [{ "$size": { "$objectToArray": "$cost_partitions" } }, 1] }, display_key, null ] },
        } },
        "query_costs": query_costs,
    };
    let command = doc! { "update": ROLLUPS, "updates": [{
        "q": { "_id": { "$in": ids }, "last_batch": { "$lt": batch.sequence } },
        "u": [
            { "$set": { "delta": { "$getField": { "field": "$_id", "input": { "$literal": deltas } } } } },
            { "$set": set }, { "$set": accelerators }, { "$unset": "delta" },
        ], "multi": true,
    }] };
    write_command(db, command, session).await
}

async fn finish(
    db: &Database,
    batch: &UsageRollupBatch,
    mut session: Option<&mut mongodb::ClientSession>,
) -> AppResult<()> {
    let meters = db.collection::<Document>(METERS);
    let action = meters.update_many(
        doc! { "_id": { "$in": &batch.row_ids }, "rollup_pending": { "$in": [true, null] } },
        doc! { "$set": {
            "rollup_pending": false, "rolled_up_at": bson::DateTime::from_chrono(batch.claimed_at),
        } },
    );
    match session.as_deref_mut() {
        Some(session) => action.session(session).await?,
        None => action.await?,
    };
    let states = db.collection::<Document>(STATE);
    let action = states.update_one(
        doc! { "_id": STATE_ID, "batch.sequence": batch.sequence },
        doc! { "$set": { "sequence": batch.sequence, "batch": null } },
    );
    match session {
        Some(session) => action.session(session).await?,
        None => action.await?,
    };
    Ok(())
}

/// One batch (<= 2,000 rows). Any replica can resume the same immutable claim:
/// no need to steal a lease whose former owner may still be running.
pub async fn fold_once(db: &Database, now: DateTime<Utc>) -> AppResult<usize> {
    // Persist the claim and source marks through replica failover as well as
    // process death. Standalone MongoDB also accepts majority+journal writes.
    let durable = db.client().database_with_options(
        db.name(),
        mongodb::options::DatabaseOptions::builder()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .build(),
    );
    let db = &durable;
    initialize(db).await?;
    let current = state(db).await?.expect("initialized state");
    let batch = match current.batch {
        Some(batch) => batch,
        None => match claim(db, &current, hour(now)).await? {
            Some(batch) => batch,
            None => return Ok(0),
        },
    };
    if supports_transactions(db).await? {
        let mut session = db.client().start_session().await?;
        let db = db.clone();
        let transaction_batch = batch.clone();
        session
            .start_transaction()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .and_run2(async move |session| {
                let operation: AppResult<()> = async {
                    apply(&db, &transaction_batch, Some(&mut *session)).await?;
                    finish(&db, &transaction_batch, Some(&mut *session)).await
                }
                .await;
                crate::services::api_key_mutation_service::transaction_result(operation)
            })
            .await
            .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    } else {
        apply(db, &batch, None).await?;
        finish(db, &batch, None).await?;
    }
    Ok(batch.row_ids.len())
}

pub async fn supports_transactions(db: &Database) -> AppResult<bool> {
    let hello = db.run_command(doc! { "hello": 1 }).await?;
    Ok(hello.contains_key("setName") || hello.get_str("msg").ok() == Some("isdbgrid"))
}

pub fn spawn_worker(db: Database, config: Arc<AppConfig>) -> Option<tokio::task::JoinHandle<()>> {
    let seconds = config.billing_reconcile_interval_secs.min(60);
    if seconds == 0 {
        return None;
    }
    Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(seconds));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let start = std::time::Instant::now();
            let mut folded = 0;
            for _ in 0..MAX_BATCHES_PER_TICK {
                match fold_once(&db, Utc::now()).await {
                    Ok(0) => break,
                    Ok(rows) => folded += rows,
                    Err(_) => {
                        tracing::warn!("Usage rollup batch retained for retry");
                        break;
                    }
                }
                if start.elapsed() >= TICK_BUDGET {
                    break;
                }
            }
            tracing::debug!(rows_folded = folded, "Usage rollup tick completed");
        }
    }))
}

#[cfg(test)]
mod tests;
