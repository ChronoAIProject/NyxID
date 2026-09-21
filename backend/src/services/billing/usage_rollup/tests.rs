use super::*;
use crate::services::admin_usage_service::{self as usage, AdminUsageQuery};
use crate::test_utils::connect_test_database;

fn row(at: DateTime<Utc>) -> Document {
    let id = uuid::Uuid::new_v4().to_string();
    doc! {
        "_id": &id, "transaction_id": format!("{id}:platform"), "billing_request_id": &id,
        "actor_user_id": "actor", "billing_owner_id": "owner", "service_id": "service", "service_slug": "service",
        "layer": "platform", "credential_class": "user_owned", "metric": "tokens", "lago_metric_code": "tokens",
        "status": "finalized", "forwarded": true, "quantity": 100_i64,
        "created_at": bson::DateTime::from_chrono(at), "token_breakdown": { "prompt_tokens": 70_i64, "completion_tokens": 30_i64 },
    }
}
async fn read(db: &Database, from: DateTime<Utc>, to: DateTime<Utc>) -> usage::AdminUsageResponse {
    usage::get_usage(
        db,
        AdminUsageQuery {
            from: Some(from.to_rfc3339()),
            to: Some(to.to_rfc3339()),
            ..Default::default()
        }
        .validate(to)
        .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn crash_after_increment_replays_without_double_count_on_standalone_protocol() {
    let db = connect_test_database("rollup_crash")
        .await
        .expect("MongoDB required");
    ensure_indexes(&db).await.unwrap();
    let now = hour(Utc::now());
    db.collection::<Document>(METERS)
        .insert_many([
            row(now - chrono::Duration::hours(2)),
            row(now - chrono::Duration::hours(1)),
        ])
        .await
        .unwrap();
    initialize(&db).await.unwrap();
    let batch = claim(&db, &state(&db).await.unwrap().unwrap(), now)
        .await
        .unwrap()
        .unwrap();
    apply(&db, &batch, None).await.unwrap();
    // Simulated process death: increments durable, source markers absent. A
    // fresh replica resumes solely from MongoDB, with no in-memory claim state.
    assert_eq!(
        db.collection::<Document>(METERS)
            .count_documents(doc! { "rollup_pending": false })
            .await
            .unwrap(),
        0
    );
    fold_once(&db, now).await.unwrap();
    fold_once(&db, now).await.unwrap();
    let result = read(&db, now - chrono::Duration::days(1), now).await;
    assert_eq!(result.totals.events, 2);
    assert_eq!(result.totals.requests, 2);
    assert_eq!(result.totals.quantities["tokens"], 200);
    assert_eq!(result.freshness.tail_rows, 0);
    // Raw TTL expiry must not erase completed-hour operational history.
    db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        2
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_replicas_fold_once_and_handle_edges_live_rows_and_late_settlement() {
    let db = connect_test_database("rollup_edges")
        .await
        .expect("MongoDB required");
    ensure_indexes(&db).await.unwrap();
    let end = hour(Utc::now());
    let start = end - chrono::Duration::hours(3);
    let mut rows = vec![];
    for minutes in [0, 10, 59, 60, 90, 120, 179, 180] {
        rows.push(row(start + chrono::Duration::minutes(minutes)));
    }
    let mut dead = row(start + chrono::Duration::minutes(95));
    dead.insert("status", "dead_letter");
    dead.insert("forwarded", false);
    let dead_id = dead.get_str("_id").unwrap().to_owned();
    rows.push(dead);
    let mut settling = row(start + chrono::Duration::minutes(100));
    settling.insert("wallet_id", "wallet");
    settling.insert("released", false);
    settling.insert("funding", doc! { "settled": false });
    let settling_id = settling.get_str("_id").unwrap().to_owned();
    rows.push(settling);
    db.collection::<Document>(METERS)
        .insert_many(rows)
        .await
        .unwrap();
    let (a, b) = tokio::join!(fold_once(&db, end), fold_once(&db, end));
    a.unwrap();
    b.unwrap();
    let result = read(&db, start + chrono::Duration::minutes(10), end).await;
    assert_eq!(result.totals.events, 7);
    assert_eq!(result.freshness.tail_rows, 2); // unsettled row and last minute remain live
    assert_eq!(
        db.collection::<Document>(METERS)
            .find_one(doc! { "_id": &settling_id })
            .await
            .unwrap()
            .unwrap()
            .get_bool("rollup_pending")
            .ok(),
        None
    );
    db.collection::<Document>(METERS)
        .update_one(
            doc! { "_id": dead_id },
            doc! { "$set": { "forwarded": true } },
        )
        .await
        .unwrap();
    db.collection::<Document>(METERS).update_one(doc! { "_id": settling_id }, doc! { "$set": { "released": true, "lago_acked": true, "funding": {
        "settled": true, "total_charge_micros": 123_i64, "wallet_funded_micros": 100_i64, "grant_funded_micros": 23_i64,
    } } }).await.unwrap();
    fold_once(&db, end).await.unwrap();
    fold_once(&db, end).await.unwrap();
    let result = read(&db, start + chrono::Duration::minutes(10), end).await;
    assert_eq!(result.totals.events, 8);
    assert_eq!(result.totals.gross_cost_micros, Some(123));
    assert_eq!(result.freshness.tail_rows, 1); // last minute is deliberately live
    assert_eq!(
        read(
            &db,
            start + chrono::Duration::minutes(11),
            start + chrono::Duration::minutes(59)
        )
        .await
        .totals
        .events,
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn aborted_transaction_leaves_sources_and_increments_uncommitted() {
    let db = crate::test_utils::connect_transaction_test_database("rollup_transaction_abort").await;
    ensure_indexes(&db).await.unwrap();
    let now = hour(Utc::now());
    db.collection::<Document>(METERS)
        .insert_one(row(now - chrono::Duration::hours(1)))
        .await
        .unwrap();
    initialize(&db).await.unwrap();
    let batch = claim(&db, &state(&db).await.unwrap().unwrap(), now)
        .await
        .unwrap()
        .unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    apply(&db, &batch, Some(&mut session)).await.unwrap();
    // A reader during the active transaction sees one coherent source: the
    // original live row, never its uncommitted summary as well.
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        1
    );
    session.abort_transaction().await.unwrap();
    assert_eq!(
        db.collection::<Document>(ROLLUPS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(DAILY)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    fold_once(&db, now).await.unwrap();
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn missing_and_invalid_state_return_errors_without_panicking() {
    let db = connect_test_database("rollup_invalid_state").await.unwrap();
    assert!(required_state(&db).await.is_err());
    for invalid in [
        doc! { "_id": STATE_ID, "sequence": "invalid" },
        doc! { "_id": STATE_ID, "sequence": i64::MAX, "rolled_up_through": bson::DateTime::now() },
    ] {
        db.collection::<Document>(STATE)
            .delete_many(doc! {})
            .await
            .unwrap();
        db.collection::<Document>(STATE)
            .insert_one(invalid)
            .await
            .unwrap();
        assert!(fold_once(&db, Utc::now()).await.is_err());
    }
    db.drop().await.unwrap();
}

#[test]
fn tick_budget_gives_incomplete_history_more_time() {
    let now = Utc::now();
    assert_eq!(
        tick_budget(DateTime::UNIX_EPOCH, now),
        Duration::from_secs(45)
    );
    assert_eq!(
        tick_budget(now - chrono::Duration::hours(2), now),
        Duration::from_secs(20)
    );
}

#[tokio::test]
async fn current_hour_exact_unacked_folds_while_legacy_and_unforwarded_remain_live() {
    let db = connect_test_database("rollup_current_hour").await.unwrap();
    ensure_indexes(&db).await.unwrap();
    let now = hour(Utc::now()) + chrono::Duration::minutes(17);
    let at = now - chrono::Duration::minutes(2);
    let mut exact = row(at);
    exact.insert("wallet_id", "wallet");
    exact.insert("released", true);
    exact.insert("lago_acked", false);
    exact.insert(
        "funding",
        doc! { "settled": true, "total_charge_micros": 123_i64, "wallet_funded_micros": 123_i64 },
    );
    let mut legacy = row(at);
    legacy.insert("wallet_id", "wallet");
    legacy.insert("released", true);
    legacy.insert("lago_acked", false);
    let mut unforwarded = exact.clone();
    unforwarded.insert("_id", "unforwarded");
    unforwarded.insert("forwarded", false);
    db.collection::<Document>(METERS)
        .insert_many([
            exact.clone(),
            legacy.clone(),
            unforwarded,
            row(now - chrono::Duration::seconds(30)),
        ])
        .await
        .unwrap();
    assert_eq!(fold_once(&db, now).await.unwrap(), 1);
    assert_eq!(fold_once(&db, now).await.unwrap(), 0);
    let current = required_state(&db).await.unwrap();
    assert_eq!(current.folded_before, Some(cutoff(now)));
    // Unstable legacy rows keep the gap-free watermark at their timestamp.
    assert_eq!(current.rolled_up_through, at);
    let result = read(&db, hour(now), now).await;
    assert_eq!(result.totals.events, 4);
    assert_eq!(result.freshness.tail_rows, 3);
    assert!(result.freshness.validated);
    // Lago can dead-letter a finalized row without setting forwarded. Such a
    // row was deliberately not folded, and disappears only from the live tail.
    db.collection::<Document>(METERS)
        .update_one(
            doc! { "_id": "unforwarded" },
            doc! { "$set": { "status": "dead_letter" } },
        )
        .await
        .unwrap();
    assert_eq!(read(&db, hour(now), now).await.totals.events, 3);
    db.collection::<Document>(METERS)
        .update_many(
            doc! { "wallet_id": "wallet", "forwarded": true },
            doc! { "$set": { "lago_acked": true } },
        )
        .await
        .unwrap();
    assert_eq!(fold_once(&db, now).await.unwrap(), 1);
    assert_eq!(fold_once(&db, now).await.unwrap(), 0);
    assert_eq!(
        required_state(&db).await.unwrap().rolled_up_through,
        cutoff(now)
    );
    // An older custom end cuts through a bucket that already contains later
    // folded sources; the raw edge excludes those sources exactly.
    assert_eq!(read(&db, hour(now), at).await.totals.events, 0);
    assert_eq!(read(&db, hour(now), now).await.totals.events, 3);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn daily_crash_between_tiers_and_before_source_mark_replays_once() {
    for after_daily in [false, true] {
        let db = connect_test_database("rollup_daily_crash").await.unwrap();
        ensure_indexes(&db).await.unwrap();
        let now = day(Utc::now());
        db.collection::<Document>(METERS)
            .insert_many([
                row(now - chrono::Duration::hours(2)),
                row(now - chrono::Duration::hours(1)),
            ])
            .await
            .unwrap();
        initialize(&db).await.unwrap();
        let batch = claim(&db, &required_state(&db).await.unwrap(), cutoff(now))
            .await
            .unwrap()
            .unwrap();
        if after_daily {
            apply(&db, &batch, None).await.unwrap();
        } else {
            apply_increments(
                &db,
                ROLLUPS,
                batch.sequence,
                batch
                    .increments
                    .iter()
                    .map(|r| bson::to_document(r).unwrap())
                    .collect(),
                true,
                None,
            )
            .await
            .unwrap();
        }
        // No source mark exists at either crash point. A new worker must
        // resume both tiers from the durable batch, using one sequence fence.
        assert_eq!(
            db.collection::<Document>(METERS)
                .count_documents(doc! { "rollup_pending": false })
                .await
                .unwrap(),
            0
        );
        fold_once(&db, now).await.unwrap();
        fold_once(&db, now).await.unwrap();
        let daily = db
            .collection::<UsageRollupDaily>(DAILY)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(daily.measures.events, 2);
        assert_eq!(daily.measures.rows_folded, 2);
        assert_eq!(daily.last_batch, batch.sequence);
        assert_eq!(daily.day, now - chrono::Duration::days(1));
        // Prove the completed whole-day read really uses the daily tier.
        db.collection::<Document>(METERS)
            .delete_many(doc! {})
            .await
            .unwrap();
        db.collection::<Document>(ROLLUPS)
            .delete_many(doc! {})
            .await
            .unwrap();
        let result = read(&db, now - chrono::Duration::days(1), now).await;
        assert_eq!(result.totals.events, 2);
        assert_eq!(result.totals.quantities["tokens"], 200);
        assert!(result.freshness.validated);
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pre_tier_hourly_history_bootstraps_after_legacy_batch_recovery_and_raw_expiry() {
    let db = connect_test_database("rollup_daily_bootstrap")
        .await
        .unwrap();
    ensure_indexes(&db).await.unwrap();
    let now = day(Utc::now());
    initialize(&db).await.unwrap();
    // Simulate an older worker with one completed batch and one interrupted
    // hourly-only batch, then raw retention expiry before this deployment.
    for n in [1, 2] {
        db.collection::<Document>(METERS)
            .insert_one(row(now - chrono::Duration::hours(n)))
            .await
            .unwrap();
        let mut batch = claim(&db, &required_state(&db).await.unwrap(), cutoff(now))
            .await
            .unwrap()
            .unwrap();
        batch.daily = false;
        db.collection::<Document>(STATE)
            .update_one(
                doc! { "_id": STATE_ID },
                doc! { "$set": { "batch": bson::to_bson(&batch).unwrap(), "daily_ready": false } },
            )
            .await
            .unwrap();
        apply(&db, &batch, None).await.unwrap();
        if n == 1 {
            finish(&db, &batch, None).await.unwrap();
        }
        // Avoid bootstrapping until the simulated legacy writes finish.
        if n == 1 {
            db.collection::<Document>(STATE)
                .update_one(
                    doc! { "_id": STATE_ID },
                    doc! { "$set": { "daily_ready": true } },
                )
                .await
                .unwrap();
        }
    }
    db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        2
    );
    fold_once(&db, now).await.unwrap(); // finish old immutable batch first
    let batch = claim(&db, &required_state(&db).await.unwrap(), cutoff(now))
        .await
        .unwrap()
        .unwrap();
    assert!(batch.hourly_sources);
    apply(&db, &batch, None).await.unwrap(); // crash before marking hourly sources
    assert!(!required_state(&db).await.unwrap().daily_ready);
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        2
    );
    let (a, b) = tokio::join!(fold_once(&db, now), fold_once(&db, now));
    a.unwrap();
    b.unwrap();
    fold_once(&db, now).await.unwrap();
    assert!(required_state(&db).await.unwrap().daily_ready);
    // Later arrivals in a previously copied hour increment both tiers once.
    db.collection::<Document>(METERS)
        .insert_one(row(now - chrono::Duration::hours(1)))
        .await
        .unwrap();
    fold_once(&db, now).await.unwrap();
    let daily = db
        .collection::<UsageRollupDaily>(DAILY)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(daily.measures.events, 3);
    assert_eq!(daily.measures.rows_folded, 3);
    assert_eq!(
        read(&db, now - chrono::Duration::days(1), now)
            .await
            .totals
            .events,
        3
    );
    db.drop().await.unwrap();
}

/// Only synthetic benchmark history uses direct tier seeding. Real history
/// always uses claim/apply/finish above. Repeating a generated hour preserves
/// cardinality while avoiding a prohibitively expensive 18M-row raw fixture.
pub(crate) async fn seed_benchmark_day(
    db: &Database,
    template: &[Document],
    at: DateTime<Utc>,
    hours: i64,
) {
    let increments: Vec<UsageRollupHourly> = template
        .iter()
        .map(|row| {
            let mut row = row.clone();
            row.insert("hour", bson::DateTime::from_chrono(at));
            for field in MEASURES {
                row.insert(*field, integer(row.get(*field)) * hours);
            }
            let partitions = row.get_document_mut("cost_partitions").unwrap();
            for (_, value) in partitions.iter_mut() {
                let part = value.as_document_mut().unwrap();
                for field in MEASURES {
                    part.insert(*field, integer(part.get(*field)) * hours);
                }
            }
            bson::from_document(row).unwrap()
        })
        .collect();
    let sequence = required_state(db).await.unwrap().sequence + 1;
    let daily: Vec<Document> = daily_increments(&increments)
        .unwrap()
        .iter()
        .map(|r| bson::to_document(r).unwrap())
        .collect();
    for chunk in daily.chunks(BATCH_SIZE as usize) {
        apply_increments(db, DAILY, sequence, chunk.to_vec(), false, None)
            .await
            .unwrap();
    }
    db.collection::<Document>(STATE)
        .update_one(
            doc! { "_id": STATE_ID },
            doc! { "$set": { "sequence": sequence } },
        )
        .await
        .unwrap();
}
