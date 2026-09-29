//! Resumable exact-accounting cut-over. Each balance and its opening posting
//! commit in one majority transaction. Money workflows wait only for balance
//! cutover; derived rollup normalization has independent background readiness.
use super::ledger;
use crate::errors::{AppError, AppResult};
use crate::models::credits::{Credits, SCALE};
use chrono::Utc;
use dashmap::DashMap;
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, Document, doc};
use std::sync::{Arc, OnceLock};
pub const ABSORBED: &str = "billing_cutover_operations";
const WALLETS: &str = crate::models::billing_wallet::COLLECTION_NAME;
const GRANTS: &str = crate::models::credit_grant::COLLECTION_NAME;
const GRANT_FIELDS: [(&str, &str); 4] = [
    ("amount_micros", "amount"),
    ("remaining_micros", "remaining"),
    ("reserved_micros", "reserved"),
    ("terminal_amount_micros", "terminal_amount"),
];
pub const MIGRATIONS: &str = "billing_migrations";
static READY_LATCHES: OnceLock<DashMap<String, Arc<std::sync::atomic::AtomicBool>>> =
    OnceLock::new();
static ROLLUP_READY_LATCHES: OnceLock<DashMap<String, Arc<std::sync::atomic::AtomicBool>>> =
    OnceLock::new();
pub const BILLING_MARKER: &str = "exact-v2";
pub const ROLLUP_MARKER: &str = "exact-v2-rollups";

fn migration_markers(db: &mongodb::Database) -> mongodb::Collection<Document> {
    db.collection_with_options(
        MIGRATIONS,
        mongodb::options::CollectionOptions::builder()
            .read_concern(mongodb::options::ReadConcern::majority())
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .build(),
    )
}

fn ready_latch(db: &mongodb::Database) -> Arc<std::sync::atomic::AtomicBool> {
    READY_LATCHES
        .get_or_init(DashMap::new)
        .entry(db.name().to_owned())
        .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
        .clone()
}

fn mark_ready(db: &mongodb::Database) {
    ready_latch(db).store(true, std::sync::atomic::Ordering::Release);
}
fn rollup_ready_latch(db: &mongodb::Database) -> Arc<std::sync::atomic::AtomicBool> {
    ROLLUP_READY_LATCHES
        .get_or_init(DashMap::new)
        .entry(db.name().to_owned())
        .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
        .clone()
}
fn mark_rollup_ready(db: &mongodb::Database) {
    rollup_ready_latch(db).store(true, std::sync::atomic::Ordering::Release);
}
pub async fn ready(db: &mongodb::Database) -> AppResult<bool> {
    let latch = ready_latch(db);
    if latch.load(std::sync::atomic::Ordering::Acquire) {
        return Ok(true);
    }
    let ready = migration_markers(db)
        .find_one(doc! { "_id": BILLING_MARKER, "completed_at": { "$type": "date" } , })
        .read_concern(mongodb::options::ReadConcern::majority())
        .await?
        .is_some();
    if ready {
        latch.store(true, std::sync::atomic::Ordering::Release);
    }
    Ok(ready)
}

/// Rollup normalization has a separate durable marker because analytics data
/// is derived and must not delay wallet/grant cutover or billed admission.
pub async fn rollup_ready(db: &mongodb::Database) -> AppResult<bool> {
    let latch = rollup_ready_latch(db);
    if latch.load(std::sync::atomic::Ordering::Acquire) {
        return Ok(true);
    }
    let ready = migration_markers(db)
        .find_one(doc! { "_id": ROLLUP_MARKER, "completed_at": { "$type": "date" } })
        .read_concern(mongodb::options::ReadConcern::majority())
        .await?
        .is_some();
    if ready {
        latch.store(true, std::sync::atomic::Ordering::Release);
    }
    Ok(ready)
}

pub async fn require_ready(db: &mongodb::Database) -> AppResult<()> {
    if !ready(db).await? {
        return Err(AppError::BillingProviderUnavailable(
            "Exact accounting cutover is pending; billing is temporarily unavailable".into(),
        ));
    }
    Ok(())
}

/// Runs independently of serving and the billing reconciler. Every replica
/// observes the durable marker; completed deployments perform no further scans.
pub fn spawn(db: mongodb::Database, drained: bool) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = start(&db, drained).await {
                tracing::error!(%error, "exact billing migration will retry");
                let _ = report(&db, "error", &error.to_string()).await;
            }
            if ready(&db).await.unwrap_or(false)
                && !rollup_ready(&db).await.unwrap_or(false)
                && let Err(error) = run(&db).await
            {
                tracing::error!(%error, "rollup normalization will retry");
                let _ = report_rollup(&db, "error", &error.to_string()).await;
            }
            if ready(&db).await.unwrap_or(false) && rollup_ready(&db).await.unwrap_or(false) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

async fn report(db: &mongodb::Database, state: &str, detail: &str) -> AppResult<()> {
    migration_markers(db)
        .update_one(
            doc! { "_id": BILLING_MARKER },
            doc! {
                "$set": {
                    "state": state,
                    "detail": detail,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                },
            },
        )
        .upsert(true)
        .await?;
    Ok(())
}

async fn report_rollup(db: &mongodb::Database, state: &str, detail: &str) -> AppResult<()> {
    migration_markers(db)
        .update_one(
            doc! { "_id": ROLLUP_MARKER },
            doc! {
                "$set": {
                    "state": state,
                    "detail": detail,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                },
            },
        )
        .upsert(true)
        .await?;
    Ok(())
}

/// The old wallet lock parser defaults unknown amount types to zero and old
/// refreshes use raw `$set`. Deserialization fences alone cannot stop a worker
/// that already holds a legacy row. Existing installations must drain those
/// writers once; persist acknowledgement so interrupted migrations can resume.
pub async fn start(db: &mongodb::Database, drained: bool) -> AppResult<()> {
    if ready(db).await? {
        return Ok(());
    }
    let migrations = migration_markers(db);
    let marker = migrations.find_one(doc! { "_id": BILLING_MARKER }).await?;
    let authorized = marker
        .as_ref()
        .is_some_and(|m| m.contains_key("cutover_drained_at") || m.contains_key("completed_at"));
    if !authorized {
        let existing = db
            .collection::<Document>(WALLETS)
            .find_one(doc! {})
            .await?
            .is_some()
            || db
                .collection::<Document>(GRANTS)
                .find_one(doc! {})
                .await?
                .is_some();
        if existing && !drained {
            let detail = "Drain pre-v2 billing writers and set BILLING_EXACT_CUTOVER_DRAINED=true";
            tracing::error!(detail, "exact billing cutover awaiting acknowledgement");
            return report(db, "awaiting_acknowledgement", detail).await;
        }
        migrations
            .update_one(
                doc! { "_id": BILLING_MARKER },
                doc! { "$set": { "cutover_drained_at": bson::DateTime::from_chrono(Utc::now()) } },
            )
            .upsert(true)
            .await?;
    }
    run_billing(db).await
}

pub async fn run(db: &mongodb::Database) -> AppResult<()> {
    run_billing(db).await?;
    if ready(db).await? {
        run_rollup(db).await?;
    }
    Ok(())
}

async fn run_billing(db: &mongodb::Database) -> AppResult<()> {
    if ready(db).await? {
        return Ok(());
    }
    report(db, "running", "Converting legacy balances").await?;
    let mut failures = 0_u64;
    for collection in [WALLETS, GRANTS] {
        if collection == GRANTS {
            convert_empty_grants(db).await?;
        }
        let mut after = String::new();
        loop {
            let mut filter = if collection == WALLETS {
                doc! { "exact_accounting_version": { "$ne": 2 } }
            } else {
                doc! { "amount": { "$exists": false } }
            };
            filter.insert("_id", doc! { "$gt": &after });
            let rows: Vec<Document> = db
                .collection::<Document>(collection)
                .find(filter)
                .projection(doc! { "_id": 1 })
                .sort(doc! { "_id": 1 })
                .limit(100)
                .await?
                .try_collect()
                .await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let id = row
                    .get_str("_id")
                    .map_err(|_| AppError::Internal("billing migration row has no id".into()))?;
                after = id.to_owned();
                let result = migrate_one(db, collection, id).await;
                failures += record_document_result(db, collection, id, result).await?;
            }
            tracing::info!(collection, after, failures, "billing cutover progress");
            tokio::task::yield_now().await;
        }
    }
    // Drained writers cannot introduce new deltas after completion.
    failures += absorb_grant_deltas(db, None).await?;
    if failures != 0 {
        return report(
            db,
            "error",
            &format!("{failures} documents failed; see billing_migration_errors"),
        )
        .await;
    }
    migration_markers(db)
        .update_one(
            doc! { "_id": BILLING_MARKER },
            doc! {
                "$set": {
                    "completed_at": bson::DateTime::from_chrono(Utc::now()),
                    "state": "complete",
                    "detail": "",
                },
            },
        )
        .upsert(true)
        .await?;
    mark_ready(db);
    Ok(())
}

async fn run_rollup(db: &mongodb::Database) -> AppResult<()> {
    if rollup_ready(db).await? {
        return Ok(());
    }
    report_rollup(db, "running", "Normalizing derived usage rollups").await?;
    normalize_rollup_mirrors(db).await?;
    migration_markers(db)
        .update_one(
            doc! { "_id": ROLLUP_MARKER },
            doc! {
                "$set": {
                    "completed_at": bson::DateTime::from_chrono(Utc::now()),
                    "state": "complete",
                    "detail": "",
                },
            },
        )
        .upsert(true)
        .await?;
    mark_rollup_ready(db);
    Ok(())
}

const ROLLUP_MONEY: [&str; 5] = [
    "gross_cost",
    "wallet_cost",
    "grant_cost",
    "allowance_cost",
    "legacy_grant_cost",
];

fn legacy_name(field: &str) -> String {
    if field == "legacy_grant_cost" {
        "legacy_grant".to_owned()
    } else {
        format!("{field}_micros")
    }
}

fn object_field(input: Bson, field: &str) -> Bson {
    doc! { "$getField": { "field": field, "input": input } }.into()
}

fn scaled_legacy(input: Bson) -> Bson {
    doc! {
        "$divide": [
            { "$convert": {
                "input": input,
                "to": "decimal",
                "onNull": Bson::Null,
            } },
            1_000_000_i64,
        ]
    }
    .into()
}

/// Preserve an explicitly present exact key (including explicit null), while
/// converting an absent legacy integer/microcredit key to Decimal128 credits.
fn exact_or_legacy(exact: Bson, legacy: Bson) -> Bson {
    doc! {
        "$cond": [
            { "$ne": [ { "$type": exact.clone() }, "missing" ] },
            exact,
            scaled_legacy(legacy),
        ]
    }
    .into()
}

fn clean_object(input: Bson) -> Bson {
    doc! {
        "$arrayToObject": {
            "$filter": {
                "input": { "$objectToArray": { "$ifNull": [input, {}] } },
                "as": "kv",
                "cond": { "$eq": [
                    { "$regexMatch": {
                        "input": "$$kv.k",
                        "regex": "(_micros$|^legacy_grant$)",
                    } },
                    false,
                ] },
            },
        }
    }
    .into()
}

fn normalized_object(input: Bson, root_fallback: bool) -> Bson {
    let mut exact = Document::new();
    for field in ROLLUP_MONEY {
        let old = object_field(input.clone(), &legacy_name(field));
        let fallback: Bson = if root_fallback {
            // The previous pipeline stage has already normalized the root.
            // Never divide that exact fallback by the legacy scale again.
            doc! { "$cond": [
                { "$ne": [{ "$type": old.clone() }, "missing"] },
                scaled_legacy(old),
                format!("${field}"),
            ] }
            .into()
        } else {
            scaled_legacy(old)
        };
        let value = object_field(input.clone(), field);
        exact.insert(
            field,
            doc! { "$cond": [
                { "$ne": [{ "$type": value.clone() }, "missing"] }, value, fallback,
            ] },
        );
    }
    doc! {
        "$mergeObjects": [clean_object(input), exact]
    }
    .into()
}

fn rollup_normalization_pipeline() -> Vec<Document> {
    let mut root = Document::new();
    let mut unset = Vec::new();
    for field in ROLLUP_MONEY {
        let legacy = legacy_name(field);
        root.insert(
            field,
            exact_or_legacy(
                Bson::String(format!("${field}")),
                Bson::String(format!("${legacy}")),
            ),
        );
        unset.push(Bson::String(legacy));
    }
    let query_costs = doc! {
        "$let": {
            "vars": { "costs": { "$ifNull": ["$query_costs", {}] } },
            "in": normalized_object("$$costs".into(), true),
        }
    };
    let partitions = doc! {
        "$arrayToObject": {
            "$map": {
                "input": { "$objectToArray": { "$ifNull": ["$cost_partitions", {}] } },
                "as": "part",
                "in": {
                    "k": "$$part.k",
                    "v": normalized_object("$$part.v".into(), false),
                },
            },
        }
    };
    let flat_mirrors: Document = ROLLUP_MONEY
        .iter()
        .map(|field| {
            (
                format!("query_{field}"),
                Bson::String(format!("$query_costs.{field}")),
            )
        })
        .collect();
    vec![
        doc! { "$set": root },
        doc! { "$set": { "query_costs": query_costs, "cost_partitions": partitions } },
        doc! { "$set": flat_mirrors },
        doc! { "$unset": Bson::Array(unset) },
    ]
}

async fn normalize_rollup_mirrors(db: &mongodb::Database) -> AppResult<()> {
    for (collection, marker) in [
        (
            crate::models::usage_rollup_hourly::COLLECTION_NAME,
            "hourly",
        ),
        (crate::models::usage_rollup_daily::COLLECTION_NAME, "daily"),
    ] {
        let progress_field = format!("rollup_{marker}_after");
        let mut after = migration_markers(db)
            .find_one(doc! { "_id": ROLLUP_MARKER })
            .await?
            .and_then(|row| row.get_str(&progress_field).ok().map(str::to_owned))
            .unwrap_or_default();
        loop {
            let rows: Vec<Document> = db
                .collection::<Document>(collection)
                .find(doc! { "_id": { "$gt": &after } })
                .projection(doc! { "_id": 1 })
                .sort(doc! { "_id": 1 })
                .limit(100)
                .await?
                .try_collect()
                .await?;
            if rows.is_empty() {
                break;
            }
            let ids: Vec<String> = rows
                .iter()
                .map(|row| {
                    row.get_str("_id")
                        .map(str::to_owned)
                        .map_err(|_| AppError::Internal("rollup migration row has no id".into()))
                })
                .collect::<AppResult<Vec<_>>>()?;
            normalize_rollup_batch(db, collection, &ids).await?;
            let id = ids.last().cloned().ok_or_else(|| {
                AppError::Internal("rollup migration batch has no documents".into())
            })?;
            after = id;
            let mut progress = Document::new();
            progress.insert(progress_field.clone(), &after);
            migration_markers(db)
                .update_one(doc! { "_id": ROLLUP_MARKER }, doc! { "$set": progress })
                .upsert(true)
                .await?;
            tokio::task::yield_now().await;
        }
    }
    Ok(())
}

pub(super) async fn normalize_rollup_batch(
    db: &mongodb::Database,
    collection: &str,
    ids: &[String],
) -> AppResult<()> {
    db.collection::<Document>(collection)
        .update_many(
            doc! { "_id": { "$in": ids } },
            rollup_normalization_pipeline(),
        )
        .await?;
    Ok(())
}

/// Historical empty grants need no journal movement. The numeric type guards
/// leave malformed documents to the isolated per-document error path.
async fn convert_empty_grants(db: &mongodb::Database) -> AppResult<()> {
    let mut filter = doc! {
        "amount": { "$exists": false },
        "remaining_micros": 0,
        "reserved_micros": 0,
        "active_settlement": Bson::Null,
    };
    filter.insert("amount_micros", doc! { "$type": ["int", "long"] });
    filter.insert(
        "$or",
        vec![
            doc! { "terminal_amount_micros": { "$type": ["int", "long"] } },
            doc! { "terminal_amount_micros": { "$exists": false } },
        ],
    );
    let mut set = doc! {
        "exact_accounting_version": 2,
        "issued_ledgered_at": "$$NOW",
        "terminal_ledgered_at": "$$NOW",
    };
    for (old, new) in GRANT_FIELDS {
        set.insert(
            new,
            doc! { "$divide": [{ "$toDecimal": { "$ifNull": [format!("${old}"), 0] } } , 1_000_000] },
        );
    }
    db.collection::<Document>(GRANTS)
        .update_many(
            filter,
            vec![
                doc! { "$set": set },
                doc! { "$unset": GRANT_FIELDS.iter().map(|(old, _)| *old).collect::<Vec<_>>() },
            ],
        )
        .await?;
    Ok(())
}
pub(super) async fn migrate_one(db: &mongodb::Database, name: &str, id: &str) -> AppResult<()> {
    for attempt in 0..16 {
        let mut session = db.client().start_session().await?;
        let database = db.clone();
        let collection_name = name.to_string();
        let document_id = id.to_string();
        let result = session
            .start_transaction()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .and_run2(async move |session| {
                let db = &database;
                let name = collection_name.as_str();
                let id = document_id.as_str();
                let operation = migrate_in_session(db, name, id, session).await;
                crate::services::api_key_mutation_service::transaction_result(operation)
            })
            .await;
        match result {
            Ok(()) => return Ok(()),
            Err(error) if super::ledger::is_duplicate_key_error(&error) && attempt < 15 => continue,
            Err(error) => {
                return Err(
                    crate::services::api_key_mutation_service::map_transaction_error(error),
                );
            }
        }
    }
    Err(AppError::Internal("billing migration contention".into()))
}

async fn record_document_result(
    db: &mongodb::Database,
    collection: &str,
    id: &str,
    result: AppResult<()>,
) -> AppResult<u64> {
    let errors = db.collection::<Document>("billing_migration_errors");
    let key = format!("{collection}:{id}");
    match result {
        Ok(()) => {
            errors.delete_one(doc! { "_id": &key }).await?;
            Ok(0)
        }
        Err(error) => {
            tracing::error!(collection, id, %error, "billing cutover document failed");
            errors
                .update_one(
                    doc! { "_id": &key },
                    doc! {
                        "$set": {
                            "detail": error.to_string(),
                            "updated_at": bson::DateTime::from_chrono(Utc::now()),
                        },
                    },
                )
                .upsert(true)
                .await?;
            Ok(1)
        }
    }
}

/// An old in-flight `$inc` may recreate a removed micro field. Such a field
/// is a delta, never a second balance. A transaction absorbs it and unsets it
/// with a posting so concurrent new workers cannot apply it twice.
pub async fn absorb_grant_deltas(db: &mongodb::Database, owner: Option<&str>) -> AppResult<u64> {
    let mut after = String::new();
    let mut failures = 0;
    loop {
        let mut filter = doc! {
            "_id": { "$gt": &after },
            "amount": { "$type": "decimal" },
            "$or": GRANT_FIELDS.iter().map(|(old, _)| {
                doc! { *old: { "$exists": true } }
            }).collect::<Vec<_>>(),
        };
        if let Some(owner) = owner {
            filter.insert("recipient_user_id", owner);
        }
        let rows: Vec<Document> = db
            .collection::<Document>(GRANTS)
            .find(filter)
            .projection(doc! { "_id": 1 })
            .sort(doc! { "_id": 1 })
            .limit(100)
            .await?
            .try_collect()
            .await?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let id = row
                .get_str("_id")
                .map_err(|_| AppError::Internal("grant delta has no id".into()))?;
            after = id.to_owned();
            let result = absorb_grant_delta(db, id).await;
            failures += record_document_result(db, GRANTS, id, result).await?;
        }
        tracing::info!(after, failures, "billing cutover delta progress");
        tokio::task::yield_now().await;
    }
    Ok(failures)
}

async fn absorb_grant_delta(db: &mongodb::Database, id: &str) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let database = db.clone();
    let document_id = id.to_string();
    session
        .start_transaction()
        .write_concern(
            mongodb::options::WriteConcern::builder()
                .w(mongodb::options::Acknowledgment::Majority)
                .journal(true)
                .build(),
        )
        .and_run2(async move |session| {
            let db = &database;
            let id = document_id.as_str();
            let operation: AppResult<()> = async {
                let Some(mut row) = db
                    .collection::<Document>(GRANTS)
                    .find_one(doc! { "_id": id })
                    .session(&mut *session)
                    .await?
                else {
                    return Ok(());
                };
                let mut delta = Credits::ZERO;
                let mut changed = false;
                for (old, new) in GRANT_FIELDS {
                    if let Some(value) = row.remove(old) {
                        let value = Credits::from_bson(value, 1_000_000)?;
                        let current = Credits::from_bson(
                            row.get(new).cloned().unwrap_or(Bson::Int64(0)),
                            SCALE,
                        )?;
                        row.insert(new, current.checked_add(value)?);
                        if new == "remaining" {
                            delta = value;
                        }
                        changed = true;
                    }
                }
                if !changed {
                    return Ok(());
                }
                let owner = row
                    .get_str("recipient_user_id")
                    .map_err(|_| AppError::Internal("grant delta has no owner".into()))?;
                if delta != Credits::ZERO {
                    let account = format!("grant:{id}");
                    let postings = if delta > Credits::ZERO {
                        ledger::transfer(account, "opening_balance".into(), delta)
                    } else {
                        ledger::transfer("opening_balance".into(), account, -delta)
                    };
                    ledger::append_in_session(
                        db,
                        &mut *session,
                        ledger::exact_entry(
                            owner,
                            id,
                            "legacy_delta",
                            format!("legacy-delta:{}", uuid::Uuid::new_v4()),
                            postings,
                            None,
                        ),
                    )
                    .await?;
                }
                db.collection::<Document>(GRANTS)
                    .replace_one(doc! { "_id": id }, row)
                    .session(&mut *session)
                    .await?;
                Ok(())
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(operation)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

async fn migrate_in_session(
    db: &mongodb::Database,
    name: &str,
    id: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let collection = db.collection::<Document>(name);
    let Some(mut row) = collection
        .find_one(doc! { "_id": id })
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    if row.get_i32("exact_accounting_version").ok() == Some(2)
        || (name == GRANTS && row.contains_key("amount"))
    {
        return Ok(());
    }
    let owner = row
        .get_str(if name == WALLETS {
            "owner_id"
        } else {
            "recipient_user_id"
        })
        .map_err(|_| AppError::Internal("billing migration has no owner".into()))?
        .to_string();
    if name == GRANTS
        && let Ok(lock) = row.get_document("active_settlement")
        && !lock.get_bool("applied").unwrap_or(false)
    {
        let reserved = Credits::from_bson(
            lock.get("reserved_micros")
                .cloned()
                .unwrap_or(Bson::Int64(0)),
            1_000_000,
        )?;
        let consume = Credits::from_bson(
            lock.get("consume_micros")
                .cloned()
                .unwrap_or(Bson::Int64(0)),
            1_000_000,
        )?;
        let remaining = Credits::from_bson(
            row.get("remaining_micros")
                .cloned()
                .unwrap_or(Bson::Int64(0)),
            1_000_000,
        )?
        .checked_sub(consume)?;
        let held = Credits::from_bson(
            row.get("reserved_micros")
                .cloned()
                .unwrap_or(Bson::Int64(0)),
            1_000_000,
        )?
        .checked_sub(reserved)?;
        row.insert("remaining_micros", remaining);
        row.insert("reserved_micros", held);
        row.get_document_mut("active_settlement")
            .expect("lock exists")
            .insert("applied", true);
        if remaining == Credits::ZERO {
            row.insert("status", "consumed");
        }
    }
    let (account, balance) = if name == WALLETS {
        for field in [
            "balance_credits",
            "reserved_credits",
            "pending_lago_debits",
            "pending_topup_expiry_credits",
            "overdraft_cap_credits",
        ] {
            let value =
                Credits::from_bson(row.get(field).cloned().unwrap_or(Bson::Int64(0)), SCALE)?;
            row.insert(field, value);
        }
        if let Ok(lock) = row.get_document_mut("active_settlement") {
            for field in ["reserved_credits", "actual_credits"] {
                let amount =
                    Credits::from_bson(lock.get(field).cloned().unwrap_or(Bson::Int64(0)), SCALE)?;
                lock.insert(field, amount);
            }
        }
        let balance = Credits::from_bson(row["balance_credits"].clone(), SCALE)?.checked_sub(
            Credits::from_bson(row["pending_lago_debits"].clone(), SCALE)?,
        )?;
        (format!("wallet:{owner}"), balance)
    } else {
        for (old, new) in GRANT_FIELDS {
            let value = Credits::from_bson(row.remove(old).unwrap_or(Bson::Int64(0)), 1_000_000)?;
            row.insert(new, value);
        }
        // Opening balances absorb legacy issuance/terminal activity.
        row.insert(
            "issued_ledgered_at",
            bson::DateTime::from_chrono(Utc::now()),
        );
        if matches!(row.get_str("status").ok(), Some("expired" | "revoked")) {
            row.insert(
                "terminal_ledgered_at",
                bson::DateTime::from_chrono(Utc::now()),
            );
        }
        (
            format!("grant:{id}"),
            Credits::from_bson(row["remaining"].clone(), SCALE)?,
        )
    };
    if let Ok(lock) = row.get_document("active_settlement")
        && lock.get_bool("applied").unwrap_or(false)
    {
        let dedupe = if name == WALLETS {
            format!(
                "usage-settled:{}",
                lock.get_str("row_id")
                    .map_err(|_| AppError::Internal("invalid wallet cutover lock".into()))?
            )
        } else {
            format!(
                "grant-consumed:{id}:{}",
                lock.get_str("operation_id")
                    .map_err(|_| AppError::Internal("invalid grant cutover lock".into()))?
            )
        };
        db.collection::<Document>(ABSORBED)
            .update_one(
                doc! { "_id": &dedupe },
                doc! {
                    "$setOnInsert": {
                        "account": &account,
                        "created_at": bson::DateTime::from_chrono(Utc::now()),
                    },
                },
            )
            .upsert(true)
            .session(&mut *session)
            .await?;
    }
    if name == WALLETS
        && let Ok(expiry) = row.get_document("active_topup_expiry")
        && expiry.get_bool("wallet_balance_applied").unwrap_or(false)
    {
        let void_id = expiry.get_str("lago_void_transaction_id").map_err(|_| {
            AppError::Internal("applied legacy expiry has no provider receipt".into())
        })?;
        for item in expiry
            .get_array("items")
            .map_err(|_| AppError::Internal("legacy expiry has no items".into()))?
        {
            let reference = item
                .as_document()
                .and_then(|i| i.get_str("reference_id").ok())
                .ok_or_else(|| AppError::Internal("invalid legacy expiry item".into()))?;
            let dedupe = format!("topup-expired:{reference}:{void_id}");
            db.collection::<Document>(ABSORBED)
                .update_one(
                    doc! { "_id": &dedupe },
                    doc! {
                        "$setOnInsert": {
                            "account": &account,
                            "created_at": bson::DateTime::from_chrono(Utc::now()),
                        },
                    },
                )
                .upsert(true)
                .session(&mut *session)
                .await?;
        }
    }
    if name == GRANTS
        && let Ok(lock) = row.get_document_mut("active_settlement")
    {
        for (old, new) in [
            ("reserved_micros", "reserved"),
            ("consume_micros", "consume"),
        ] {
            let value = Credits::from_bson(lock.remove(old).unwrap_or(Bson::Int64(0)), 1_000_000)?;
            lock.insert(new, value);
        }
    }
    row.insert("exact_accounting_version", 2);
    let postings = if balance >= Credits::ZERO {
        ledger::transfer(account.clone(), "opening_balance".into(), balance)
    } else {
        ledger::transfer("opening_balance".into(), account.clone(), -balance)
    };
    if balance != Credits::ZERO {
        let entry = ledger::exact_entry(
            &owner,
            id,
            "opening_balance",
            format!("opening:{account}"),
            postings,
            None,
        );
        ledger::append_in_session(db, &mut *session, entry).await?;
    }
    collection
        .replace_one(doc! { "_id": id }, row)
        .session(&mut *session)
        .await?;
    Ok(())
}
