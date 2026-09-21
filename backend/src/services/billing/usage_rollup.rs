//! Bounded, newest-first, exactly-once hourly/daily fold, including automatic legacy
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
        usage_rollup_daily::{COLLECTION_NAME as DAILY, UsageRollupDaily},
        usage_rollup_hourly::{COLLECTION_NAME as ROLLUPS, UsageRollupHourly},
        usage_rollup_state::{
            COLLECTION_NAME as STATE, STATE_ID, UsageRollupBatch, UsageRollupState,
        },
    },
};

pub const BATCH_SIZE: i64 = 2_000;
pub const PENDING_INDEX: &str = "usage_rollup_pending_window";
const DAILY_PENDING_INDEX: &str = "usage_rollup_daily_pending";
const MAX_BATCHES_PER_TICK: usize = 100;
const TICK_BUDGET: Duration = Duration::from_secs(20);
const BACKFILL_TICK_BUDGET: Duration = Duration::from_secs(45);
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
    time - chrono::Duration::seconds(i64::from(time.minute() * 60 + time.second()))
        - chrono::Duration::nanoseconds(i64::from(time.nanosecond()))
}

pub fn day(time: DateTime<Utc>) -> DateTime<Utc> {
    hour(time) - chrono::Duration::hours(i64::from(time.hour()))
}

pub fn cutoff(now: DateTime<Utc>) -> DateTime<Utc> {
    now - chrono::Duration::seconds(60)
}

pub fn tick_budget(watermark: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    if watermark < now - chrono::Duration::hours(2) {
        BACKFILL_TICK_BUDGET
    } else {
        TICK_BUDGET
    }
}

/// A full equality prefix includes missing pre-deployment markers as null.
/// Unlike a partial missing-field index (unsupported by MongoDB), this index
/// serves backfill and the live tail from the first deployment onward.
pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    let meters = db.collection::<Document>(METERS);
    for (name, keys) in [
        (
            PENDING_INDEX,
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
    db.collection::<Document>(ROLLUPS)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "daily_pending": 1, "hour": -1 })
                .options(
                    IndexOptions::builder()
                        .name(DAILY_PENDING_INDEX.to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    for (collection, bucket) in [(ROLLUPS, "hour"), (DAILY, "day")] {
        for keys in [
            doc! { bucket: 1 },
            doc! { "actor": 1, bucket: 1 },
            doc! { "owner": 1, bucket: 1 },
        ] {
            db.collection::<Document>(collection)
                .create_index(IndexModel::builder().keys(keys).build())
                .await?;
        }
        // Cover the common single-partition reduction: fetching tens of thousands
        // of hourly BSON documents defeats the production dashboard budget even
        // after removing raw edge scans. All values are scalar or embedded objects
        // (never arrays), so these indexes can answer the reduction without FETCH.
        for (name, mut keys) in [
            ("usage_rollup_reduce_window", doc! { bucket: 1 }),
            (
                "usage_rollup_reduce_actor",
                doc! { "actor": 1, bucket: 1, "owner": 1 },
            ),
            (
                "usage_rollup_reduce_owner",
                doc! { "owner": 1, bucket: 1, "actor": 1 },
            ),
        ] {
            keys.insert("single_display_key", 1);
            for field in MEASURES.iter().filter(|field| **field != "rows_folded") {
                keys.insert(*field, 1);
            }
            db.collection::<Document>(collection)
                .create_index(
                    IndexModel::builder()
                        .keys(keys)
                        .options(IndexOptions::builder().name(name.to_owned()).build())
                        .build(),
                )
                .await?;
        }
        // Most summaries contain one display partition and use the main hour
        // index. This small partial index prevents scanning them a second time
        // when expanding multi-partition (or older unaccelerated) summaries.
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { bucket: 1 })
                    .options(
                        IndexOptions::builder()
                            .name(format!("usage_rollup_partitioned_{bucket}"))
                            .partial_filter_expression(doc! { "single_display_key": null })
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
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
            { "$or": [ { "wallet_id": null }, { "released": true, "$and": [
                { "$or": [ { "funding": null }, { "funding.settled": true } ] },
                // Exact settlements do not wait for Lago availability. Before
                // ack, require forwarded=true: a permanent Lago rejection can
                // move finalized -> dead_letter, whose dashboard predicate
                // excludes unforwarded rows. Forwarded never reverts to false.
                { "$or": [
                    { "lago_acked": true }, { "status": "dead_letter" },
                    { "funding.total_charge_micros": { "$ne": null }, "forwarded": true },
                ] },
            ] } ] },
        ],
    }
}

pub async fn state(db: &Database) -> AppResult<Option<UsageRollupState>> {
    // Readers must not enable the daily tier from a local, not-yet-committed
    // readiness marker and then aggregate an older majority snapshot.
    let state = db
        .collection_with_options::<UsageRollupState>(
            STATE,
            mongodb::options::CollectionOptions::builder()
                .read_concern(mongodb::options::ReadConcern::majority())
                .build(),
        )
        .find_one(doc! { "_id": STATE_ID })
        .await?;
    if state.as_ref().is_some_and(|s| {
        s.sequence < 0
            || s.sequence == i64::MAX
            || s.batch.as_ref().is_some_and(|batch| {
                batch.sequence != s.sequence + 1
                    || (batch.hourly_sources && !batch.daily)
                    || batch.row_ids.is_empty()
                    || batch.row_ids.len() > BATCH_SIZE as usize
                    || batch.increments.is_empty()
            })
    }) {
        return Err(AppError::Internal("Invalid usage rollup state".into()));
    }
    Ok(state)
}

async fn required_state(db: &Database) -> AppResult<UsageRollupState> {
    state(db)
        .await?
        .ok_or_else(|| AppError::Internal("Missing usage rollup state".into()))
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
    if !current.daily_ready {
        let increments: Vec<UsageRollupHourly> = db
            .collection::<UsageRollupHourly>(ROLLUPS)
            .find(doc! { "daily_pending": { "$in": [true, null] } })
            .hint(mongodb::options::Hint::Name(DAILY_PENDING_INDEX.to_owned()))
            .sort(doc! { "hour": -1 })
            .limit(BATCH_SIZE)
            .await?
            .try_collect()
            .await?;
        if !increments.is_empty() {
            let batch = UsageRollupBatch {
                sequence: current.sequence + 1,
                daily: true,
                hourly_sources: true,
                row_ids: increments.iter().map(|row| row.id.clone()).collect(),
                increments,
                claimed_at: Utc::now(),
            };
            return publish_batch(db, current, batch, None).await;
        }
        let result = db
            .collection::<Document>(STATE)
            .update_one(
                doc! { "_id": STATE_ID, "sequence": current.sequence, "batch": null },
                doc! { "$set": { "daily_ready": true } },
            )
            .await?;
        if result.matched_count == 0 {
            return Ok(None);
        }
    }
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
            .map(|r| {
                r.get_datetime("created_at")
                    .map(|t| t.to_chrono())
                    .map_err(|_| AppError::Internal("Invalid usage source timestamp".into()))
            })
            .transpose()?
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
    let key = group
        .get_document_mut("_id")
        .map_err(|_| AppError::Internal("Invalid usage group key".into()))?;
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
        let Some(Bson::Document(mut key)) = group.remove("_id") else {
            return Err(AppError::Internal("Missing usage group key".into()));
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
                .map_err(|_| AppError::Internal("Invalid usage cost partitions".into()))?
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
        daily: true,
        hourly_sources: false,
        row_ids: ids,
        increments,
        claimed_at: Utc::now(),
    };
    publish_batch(db, current, batch, Some(cutoff)).await
}

async fn publish_batch(
    db: &Database,
    current: &UsageRollupState,
    batch: UsageRollupBatch,
    cutoff: Option<DateTime<Utc>>,
) -> AppResult<Option<UsageRollupBatch>> {
    let mut update = doc! { "$set": {
        "batch": bson::to_bson(&batch).map_err(|e| AppError::Internal(e.to_string()))?,
    } };
    if let Some(cutoff) = cutoff {
        update.insert(
            "$max",
            doc! { "folded_before": bson::DateTime::from_chrono(cutoff) },
        );
    }
    let result = db
        .collection::<Document>(STATE)
        .update_one(
            doc! { "_id": STATE_ID, "sequence": current.sequence, "batch": null },
            update,
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
    if !batch.hourly_sources {
        let increments = batch
            .increments
            .iter()
            .map(bson::to_document)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        apply_increments(
            db,
            ROLLUPS,
            batch.sequence,
            increments,
            batch.daily,
            session.as_deref_mut(),
        )
        .await?;
    }
    if batch.daily {
        let increments = daily_increments(&batch.increments)?
            .iter()
            .map(bson::to_document)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        apply_increments(db, DAILY, batch.sequence, increments, false, session).await?;
    }
    Ok(())
}

/// Collapse hours within one immutable batch, preserving every legacy cost
/// partition. The per-day fence is the SAME global sequence as the hourly tier.
fn daily_increments(increments: &[UsageRollupHourly]) -> AppResult<Vec<UsageRollupDaily>> {
    let mut combined = std::collections::BTreeMap::<String, Document>::new();
    for increment in increments {
        let mut group =
            bson::to_document(increment).map_err(|e| AppError::Internal(e.to_string()))?;
        group.remove("hour");
        group.insert("day", bson::DateTime::from_chrono(day(increment.hour)));
        let mut key = Document::new();
        for field in DIMENSIONS.iter().chain([&"exact", &"day"]) {
            key.insert(*field, group.get(*field).cloned().unwrap_or(Bson::Null));
        }
        let hash = hex::encode(Sha256::digest(
            bson::to_vec(&key).map_err(|e| AppError::Internal(e.to_string()))?,
        ));
        group.insert("_id", &hash);
        if let Some(existing) = combined.get_mut(&hash) {
            add_measures(existing, &group);
            let parts = group
                .get_document("cost_partitions")
                .map_err(|_| AppError::Internal("Invalid usage cost partitions".into()))?;
            let existing_parts = existing
                .get_document_mut("cost_partitions")
                .map_err(|_| AppError::Internal("Invalid usage cost partitions".into()))?;
            for (key, value) in parts {
                let value = value
                    .as_document()
                    .ok_or_else(|| AppError::Internal("Invalid usage cost partition".into()))?;
                if let Ok(existing) = existing_parts.get_document_mut(key) {
                    add_measures(existing, value);
                } else {
                    existing_parts.insert(key, value.clone());
                }
            }
        } else {
            combined.insert(hash, group);
        }
    }
    combined
        .into_values()
        .map(|group| bson::from_document(group).map_err(|e| AppError::Internal(e.to_string())))
        .collect()
}

fn add_measures(existing: &mut Document, delta: &Document) {
    for field in MEASURES {
        existing.insert(
            *field,
            integer(existing.get(*field)).saturating_add(integer(delta.get(*field))),
        );
    }
}

async fn apply_increments(
    db: &Database,
    collection: &str,
    sequence: i64,
    increments: Vec<Document>,
    daily_complete: bool,
    mut session: Option<&mut mongodb::ClientSession>,
) -> AppResult<()> {
    let mut inserts = Vec::with_capacity(increments.len());
    let mut deltas = Document::new();
    let mut ids = Vec::with_capacity(increments.len());
    for mut initial in increments {
        let id = initial
            .get_str("_id")
            .map_err(|_| AppError::Internal("Missing usage increment id".into()))?
            .to_owned();
        initial.remove("_id");
        deltas.insert(&id, initial.clone());
        ids.push(id.clone());
        for field in MEASURES {
            initial.insert(*field, 0_i64);
        }
        initial.insert("cost_partitions", Document::new());
        initial.insert("last_batch", 0_i64);
        inserts
            .push(doc! { "q": { "_id": &id }, "u": { "$setOnInsert": initial }, "upsert": true });
    }
    // Small idempotent initializers avoid generating a distinct arithmetic
    // program for each summary. The shared update below is compiled once.
    for chunk in inserts.chunks(100) {
        write_command(
            db,
            doc! { "update": collection, "updates": chunk.to_vec(), "ordered": false },
            session.as_deref_mut(),
        )
        .await?;
    }
    let mut set = doc! { "last_batch": sequence };
    if daily_complete {
        set.insert("daily_pending", false);
    }
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
        .map_err(|_| AppError::Internal("Invalid usage group key".into()))?
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
    let command = doc! { "update": collection, "updates": [{
        "q": { "_id": { "$in": ids }, "last_batch": { "$lt": sequence } },
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
    let sources = db.collection::<Document>(if batch.hourly_sources {
        ROLLUPS
    } else {
        METERS
    });
    let (marker, update) = if batch.hourly_sources {
        ("daily_pending", doc! { "$set": { "daily_pending": false } })
    } else {
        (
            "rollup_pending",
            doc! { "$set": { "rollup_pending": false, "rolled_up_at": bson::DateTime::from_chrono(batch.claimed_at) } },
        )
    };
    let action = sources.update_many(
        doc! { "_id": { "$in": &batch.row_ids }, marker: { "$in": [true, null] } },
        update,
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
    let current = required_state(db).await?;
    let batch = match current.batch {
        Some(batch) => batch,
        None => match claim(db, &current, cutoff(now)).await? {
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
            let budget = match state(&db).await {
                Ok(state) => tick_budget(
                    state
                        .filter(|s| s.daily_ready)
                        .map(|s| s.rolled_up_through)
                        .unwrap_or(DateTime::UNIX_EPOCH),
                    Utc::now(),
                ),
                Err(_) => {
                    tracing::warn!("Usage rollup state unavailable; retrying next tick");
                    continue;
                }
            };
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
                if start.elapsed() >= budget {
                    break;
                }
            }
            tracing::debug!(rows_folded = folded, "Usage rollup tick completed");
        }
    }))
}

#[cfg(test)]
pub(crate) mod tests;
