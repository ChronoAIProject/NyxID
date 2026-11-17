use super::*;
use crate::services::billing::reconcile;
use crate::test_utils::connect_test_database;
use mongodb::bson::Bson;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

tokio::task_local! {
    pub(super) static CLOCK: (std::cell::Cell<std::time::Duration>, std::time::Duration);
    static DRAIN_AT: Phase;
    static CRASH_AT: (Phase, Arc<Notify>);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Initialized,
    Selected,
    Dated,
    Advanced,
}

pub(super) async fn point(phase: Phase) {
    if phase == Phase::Advanced {
        let _ = CLOCK.try_with(|(elapsed, per_page)| elapsed.set(elapsed.get() + *per_page));
    }
    if DRAIN_AT.try_with(|stop| *stop == phase).unwrap_or(false) {
        reconcile::drain_for_test();
    }
    let stop = CRASH_AT
        .try_with(|(at, notify)| {
            if *at == phase {
                notify.notify_one();
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
    if stop {
        std::future::pending::<()>().await;
    }
}

fn now() -> DateTime<Utc> {
    bson::DateTime::now().to_chrono()
}

fn row(id: &str, pending: bool) -> Document {
    doc! {
        "_id": id, "status": "finalized", "forwarded": true,
        "rollup_pending": pending, "quantity": 1_i64,
        "created_at": bson::DateTime::from_millis(0),
    }
}

async fn saved(db: &Database, id: &str) -> Document {
    db.collection::<Document>(METERS)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
}

async fn cursor(db: &Database) -> MeteringRetentionState {
    db.collection::<MeteringRetentionState>(STATE)
        .find_one(doc! {"_id": CURSOR_ID})
        .await
        .unwrap()
        .unwrap()
}

async fn backfill_for_test(db: &Database, now: DateTime<Utc>, page_limit: u64) -> AppResult<u64> {
    CLOCK
        .scope(
            (
                std::cell::Cell::new(std::time::Duration::ZERO),
                std::time::Duration::from_nanos(1_000_000_000_u64.div_ceil(page_limit)),
            ),
            backfill(db, now),
        )
        .await
}

#[tokio::test]
async fn bounded_backfill_uses_primary_index_and_preserves_other_shapes() {
    let db = connect_test_database("meter_retention_bound")
        .await
        .unwrap();
    let at = now();
    // IDs put all protected shapes after the first full page.
    let mut rows: Vec<_> = (0..BATCH_SIZE + 5)
        .map(|n| row(&format!("a-{n:03}"), false))
        .collect();
    let mut billed = row("z-billed", false);
    billed.insert("wallet_id", "wallet");
    let mut dated = row("z-dated", false);
    dated.insert(
        "expires_at",
        bson::DateTime::from_chrono(at + chrono::Duration::days(3)),
    );
    let mut null_date = row("a-105", false);
    null_date.insert("wallet_id", Bson::Null);
    null_date.insert("expires_at", Bson::Null);
    let mut dead = row("a-106", false);
    dead.insert("status", "dead_letter");
    let mut no_quantity = row("z-no-quantity", false);
    no_quantity.remove("quantity");
    let mut dead_unforwarded = row("z-dead-unforwarded", false);
    dead_unforwarded.insert("status", "dead_letter");
    dead_unforwarded.insert("forwarded", false);
    let mut failed = row("z-failed", false);
    failed.insert("status", "failed");
    let mut abandoned = row("z-abandoned", false);
    abandoned.insert("status", "abandoned");
    let protected = vec![
        billed,
        dated,
        row("z-uncounted", true),
        no_quantity,
        dead_unforwarded,
        failed,
        abandoned,
    ];
    rows.extend(protected.clone());
    rows.extend([null_date, dead]);
    db.collection::<Document>(METERS)
        .insert_many(rows)
        .await
        .unwrap();

    for after in [None, Some("a-050")] {
        let explain = db
            .run_command(doc! {
                "explain": {"find": METERS, "filter": page_filter(after), "sort": {"_id": 1},
                    "projection": {"_id": 1}, "limit": BATCH_SIZE, "hint": "_id_"},
                "verbosity": "executionStats",
            })
            .await
            .unwrap();
        let rendered = explain.to_string();
        assert!(rendered.contains("IXSCAN"), "{rendered}");
        assert!(!rendered.contains("COLLSCAN"), "{rendered}");
        let stats = explain.get_document("executionStats").unwrap();
        let keys = stats
            .get_i64("totalKeysExamined")
            .or_else(|_| stats.get_i32("totalKeysExamined").map(i64::from))
            .unwrap();
        let docs = stats
            .get_i64("totalDocsExamined")
            .or_else(|_| stats.get_i32("totalDocsExamined").map(i64::from))
            .unwrap();
        assert!(keys <= BATCH_SIZE);
        assert_eq!(docs, 0);
        println!("RETENTION_EXPLAIN after={after:?}: {stats}");
    }
    assert_eq!(
        backfill_for_test(&db, at, 1).await.unwrap(),
        BATCH_SIZE as u64
    );
    assert_eq!(cursor(&db).await.after.as_deref(), Some("a-099"));
    assert_eq!(backfill_for_test(&db, at, 2).await.unwrap(), 7);
    assert_eq!(backfill_for_test(&db, at, 1).await.unwrap(), 0);
    assert!(cursor(&db).await.after.is_none());
    for original in protected {
        assert_eq!(saved(&db, original.get_str("_id").unwrap()).await, original);
    }
    for n in 0..107 {
        assert_eq!(
            *saved(&db, &format!("a-{n:03}"))
                .await
                .get_datetime("expires_at")
                .unwrap(),
            bson::DateTime::from_chrono(at + chrono::Duration::days(ACKED_RETENTION_DAYS))
        );
    }
    // An old replica can finish a row behind the cursor; the next sweep finds it.
    db.collection::<Document>(METERS)
        .update_one(doc! {"_id": "a-000"}, doc! {"$unset": {"expires_at": ""}})
        .await
        .unwrap();
    assert_eq!(
        backfill_for_test(&db, at + chrono::Duration::days(1), 3)
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn one_pass_processes_multiple_pages_before_waiting_for_next_pass() {
    let db = connect_test_database("meter_retention_paced")
        .await
        .unwrap();
    let at = now();
    let rows: Vec<_> = (0..205).map(|n| row(&format!("a-{n:03}"), false)).collect();
    db.collection::<Document>(METERS)
        .insert_many(rows)
        .await
        .unwrap();

    assert_eq!(backfill_for_test(&db, at, 3).await.unwrap(), 205);
    assert_eq!(cursor(&db).await.after.as_deref(), Some("a-204"));
    assert_eq!(backfill_for_test(&db, at, 1).await.unwrap(), 0);
    let saved_state = cursor(&db).await;
    assert!(saved_state.after.is_none());
    assert_eq!(saved_state.completed_at, Some(at));
    assert_eq!(
        saved_state.next_due_at,
        Some(at + chrono::Duration::days(1))
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn elapsed_budget_stops_after_the_current_durable_page() {
    let db = connect_test_database("meter_retention_budget")
        .await
        .unwrap();
    let at = now();
    db.collection::<Document>(METERS)
        .insert_many((0..205).map(|n| row(&format!("a-{n:03}"), false)))
        .await
        .unwrap();

    let changed = CLOCK
        .scope(
            (
                std::cell::Cell::new(std::time::Duration::ZERO),
                std::time::Duration::from_millis(600),
            ),
            backfill(&db, at),
        )
        .await
        .unwrap();
    assert_eq!(changed, 200);
    assert_eq!(cursor(&db).await.after.as_deref(), Some("a-199"));
    assert_eq!(cursor(&db).await.revision, 2);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn dormant_sweep_reads_state_without_writing_or_touching_rows() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observed = seen.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "meter_retention_dormant",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                observed.lock().unwrap().push(event.command_name);
            }
        }),
    )
    .await;
    let at = now();
    db.collection::<Document>(METERS)
        .insert_one(row("row", false))
        .await
        .unwrap();
    db.collection::<MeteringRetentionState>(STATE)
        .insert_one(MeteringRetentionState {
            id: CURSOR_ID.to_owned(),
            revision: 8,
            after: None,
            completed_at: Some(at),
            next_due_at: Some(at + chrono::Duration::hours(23)),
        })
        .await
        .unwrap();

    seen.lock().unwrap().clear();
    assert_eq!(
        backfill(&db, at + chrono::Duration::hours(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(*seen.lock().unwrap(), vec!["find".to_owned()]);
    assert!(saved(&db, "row").await.get("expires_at").is_none());
    assert_eq!(cursor(&db).await.revision, 8);
    let due = at + chrono::Duration::hours(23);
    seen.lock().unwrap().clear();
    assert_eq!(backfill_for_test(&db, due, 2).await.unwrap(), 1);
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["find", "find", "update", "update", "find", "update"]
    );
    let completed = cursor(&db).await;
    assert_eq!(completed.completed_at, Some(due));
    assert_eq!(completed.next_due_at, Some(due + SWEEP_INTERVAL));
    assert_eq!(
        *saved(&db, "row").await.get_datetime("expires_at").unwrap(),
        bson::DateTime::from_chrono(due + chrono::Duration::days(ACKED_RETENTION_DAYS))
    );
    seen.lock().unwrap().clear();
    assert_eq!(backfill(&db, due).await.unwrap(), 0);
    assert_eq!(*seen.lock().unwrap(), vec!["find".to_owned()]);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn backfill_resumes_after_each_durable_write_without_extending_dates() {
    for phase in [Phase::Initialized, Phase::Dated, Phase::Advanced] {
        let db = connect_test_database("meter_retention_crash")
            .await
            .unwrap();
        db.collection::<Document>(METERS)
            .insert_one(row("row", false))
            .await
            .unwrap();
        let at = now();
        let barrier = Arc::new(Notify::new());
        let mut future =
            Box::pin(CRASH_AT.scope((phase, barrier.clone()), backfill_for_test(&db, at, 1)));
        tokio::select! {
            _ = &mut future => panic!("expected interruption at {phase:?}"),
            () = barrier.notified() => {},
        }
        // Dropping the worker future leaves only the acknowledged MongoDB writes.
        drop(future);
        let expiry_before = saved(&db, "row").await.get("expires_at").cloned();
        assert_eq!(
            backfill_for_test(&db, at + chrono::Duration::days(1), 1)
                .await
                .unwrap(),
            if phase == Phase::Initialized { 1 } else { 0 }
        );
        let expiry = saved(&db, "row").await.get("expires_at").cloned().unwrap();
        let expected = expiry_before.unwrap_or_else(|| {
            Bson::DateTime(bson::DateTime::from_chrono(at + chrono::Duration::days(31)))
        });
        assert_eq!(expiry, expected, "{phase:?}");
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn backfill_honors_drain_at_selection_and_between_writes() {
    for phase in [
        Phase::Initialized,
        Phase::Selected,
        Phase::Dated,
        Phase::Advanced,
    ] {
        let db = connect_test_database("meter_retention_drain")
            .await
            .unwrap();
        db.collection::<Document>(METERS)
            .insert_one(row("row", false))
            .await
            .unwrap();
        let at = now();
        let changed =
            reconcile::with_drain_for_test(DRAIN_AT.scope(phase, backfill_for_test(&db, at, 1)))
                .await
                .unwrap();
        assert_eq!(
            changed,
            if matches!(phase, Phase::Dated | Phase::Advanced) {
                1
            } else {
                0
            }
        );
        assert_eq!(
            cursor(&db).await.after.as_deref(),
            if phase == Phase::Advanced {
                Some("row")
            } else {
                None
            }
        );
        assert_eq!(
            backfill_for_test(&db, at, 1).await.unwrap(),
            if matches!(phase, Phase::Initialized | Phase::Selected) {
                1
            } else {
                0
            }
        );
        assert_eq!(
            cursor(&db).await.after.as_deref(),
            if phase == Phase::Advanced {
                None
            } else {
                Some("row")
            }
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "scale evidence over a 100,000-row fixture; run explicitly"]
async fn sweeping_commands_match_durable_progress() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observed = seen.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "meter_retention_throughput",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                observed.lock().unwrap().push(event.command_name);
            }
        }),
    )
    .await;
    db.collection::<Document>(STATE)
        .insert_one(doc! {
            "_id": CURSOR_ID, "revision": 0_i64, "after": null,
        })
        .await
        .unwrap();
    for batch in 0..10 {
        db.collection::<Document>(METERS)
            .insert_many(
                (batch * 10_000..(batch + 1) * 10_000).map(|n| row(&format!("a-{n:06}"), false)),
            )
            .await
            .unwrap();
    }
    // Timing is evidence only; assertions depend on durable progress, never speed.
    for pass in 1..=3 {
        let before = cursor(&db).await;
        if before.next_due_at.is_some() {
            break;
        }
        seen.lock().unwrap().clear();
        let started = std::time::Instant::now();
        let changed = backfill(&db, now()).await.unwrap();
        let elapsed = started.elapsed();
        let commands = seen.lock().unwrap().len();
        let after = cursor(&db).await;
        let completed = i64::from(after.next_due_at.is_some());
        let pages = after.revision - before.revision - completed;
        assert_eq!(commands as i64, 1 + 3 * pages + 2 * completed);
        assert_eq!(changed as i64, 100 * pages);
        println!(
            "RETENTION_THROUGHPUT pass={pass} pages={pages} rows={changed} commands={commands} elapsed_ms={}",
            elapsed.as_millis()
        );
    }
    db.drop().await.unwrap();
}
