use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Barrier, Notify, Semaphore};

fn row(code: &str, price: i64) -> BillingRateCache {
    BillingRateCache {
        id: BillingRateCache::cache_id(code, None),
        lago_metric_code: code.into(),
        model: None,
        credits_per_unit_micros: price * 1_000_000,
        credits_per_unit_pico: Some(price * 1_000_000_000_000),
        synced_at: bson::DateTime::now().to_chrono(),
        retired_at: None,
    }
}
async fn database(name: &str) -> Database {
    crate::test_utils::connect_transaction_test_database(name).await
}
async fn saved(db: &Database, code: &str) -> Option<BillingRateCache> {
    db.collection::<BillingRateCache>(RATES)
        .find_one(doc! {"_id":BillingRateCache::cache_id(code,None)})
        .await
        .unwrap()
}
async fn racing_replace(
    db: &Database,
    row: BillingRateCache,
    barrier: Arc<Barrier>,
) -> AppResult<()> {
    let context = (db.clone(), row, barrier, AtomicBool::new(true));
    transaction_service::run_db_only(db, &context, |session, (db, row, barrier, first)| {
        Box::pin(async move {
            // Establish both transaction snapshots before either singleton write.
            if first.swap(false, Ordering::SeqCst) {
                db.collection::<Document>(EPOCH)
                    .find_one(doc! {"_id":SINGLETON_ID})
                    .session(&mut *session)
                    .await?;
                barrier.wait().await;
            }
            replace_in_session(db, session, row).await
        })
    })
    .await
}
async fn concurrent_writers(initialized: bool) {
    let db = database("rate_epoch_writers").await;
    db.create_collection(EPOCH).await.unwrap();
    db.create_collection(RATES).await.unwrap();
    if initialized {
        replace(&db, &row("initial", 1)).await.unwrap();
    }
    let initial = capture(&db).await.unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let (a, b) = (row("a", 2), row("b", 3));
    let (left, right) = tokio::join!(
        racing_replace(&db, a.clone(), barrier.clone()),
        racing_replace(&db, b.clone(), barrier.clone())
    );
    left.unwrap();
    right.unwrap();
    assert_eq!(capture(&db).await.unwrap(), initial + 2);
    assert_eq!(
        db.collection::<Document>(EPOCH)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(saved(&db, "a").await, Some(a));
    assert_eq!(saved(&db, "b").await, Some(b));
    let epoch = db
        .collection::<Document>(EPOCH)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        epoch.len(),
        2,
        "epoch is coordination metadata, not a price"
    );
    assert!(epoch.get_i64("generation").is_ok());
    db.drop().await.unwrap();
}
#[tokio::test]
async fn singleton_initialization_race_has_one_epoch_and_two_commits() {
    concurrent_writers(false).await;
}
#[tokio::test]
async fn two_explicit_writers_both_advance_the_existing_epoch() {
    concurrent_writers(true).await;
}

#[tokio::test]
async fn concurrent_zero_captures_publish_only_one_snapshot() {
    let db = database("rate_epoch_cold_publishers").await;
    let (a, b) = (capture(&db).await.unwrap(), capture(&db).await.unwrap());
    assert_eq!((a, b), (0, 0));
    assert_eq!(
        db.collection::<Document>(EPOCH)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let barrier = Arc::new(Barrier::new(2));
    let (ar, br) = ([row("a", 1)], [row("b", 2)]);
    let (a, b) = tokio::join!(
        async {
            barrier.wait().await;
            publish_chunk(
                &db,
                Chunk {
                    captured: a,
                    plan: "standard",
                    diagnostics: Some(&[]),
                    rejected: &[],
                    rows: &ar,
                },
            )
            .await
        },
        async {
            barrier.wait().await;
            publish_chunk(
                &db,
                Chunk {
                    captured: b,
                    plan: "standard",
                    diagnostics: Some(&[]),
                    rejected: &[],
                    rows: &br,
                },
            )
            .await
        }
    );
    assert_ne!(a.unwrap().is_some(), b.unwrap().is_some());
    assert_eq!(capture(&db).await.unwrap(), 1);
    assert_eq!(
        db.collection::<Document>(RATES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn a_transaction_retry_keeps_the_original_capture() {
    let db = database("rate_epoch_transaction_retry").await;
    replace(&db, &row("price", 1)).await.unwrap();
    let captured = capture(&db).await.unwrap();
    let rows = [row("price", 1)];
    let reached = Arc::new(Notify::new());
    let resume = Arc::new(Semaphore::new(0));
    let attempts = Arc::new(AtomicUsize::new(0));
    let context = (
        db.clone(),
        Chunk {
            captured,
            plan: "standard",
            diagnostics: Some(&[]),
            rejected: &[],
            rows: &rows,
        },
        reached.clone(),
        resume.clone(),
        attempts.clone(),
    );
    let (result, ()) = tokio::join!(
        transaction_service::run_db_only(
            &db,
            &context,
            |session, (db, chunk, reached, resume, attempts)| Box::pin(async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    // Freeze a real MongoDB snapshot, then let another writer commit.
                    // The first guard gets WriteConflict; the transaction body retries.
                    db.collection::<Document>(EPOCH)
                        .find_one(doc! {"_id":SINGLETON_ID})
                        .session(&mut *session)
                        .await?;
                    reached.notify_one();
                    resume.acquire().await.unwrap().forget();
                }
                publish_in_session(db, session, chunk).await
            })
        ),
        async {
            tokio::time::timeout(std::time::Duration::from_secs(10), reached.notified())
                .await
                .unwrap();
            replace(&db, &row("price", 2)).await.unwrap();
            resume.add_permits(1);
        }
    );
    assert!(
        matches!(result,Err(AppError::DatabaseError(ref error)) if error.get_custom::<Superseded>().is_some())
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        saved(&db, "price").await.unwrap().credits_per_unit_micros,
        2_000_000
    );
    assert_eq!(capture(&db).await.unwrap(), captured + 1);
    assert!(
        db.collection::<Document>("billing_rate_diagnostics")
            .find_one(doc! {})
            .await
            .unwrap()
            .is_none()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn an_existing_singleton_without_generation_defaults_to_zero() {
    let db = database("rate_epoch_default").await;
    db.collection::<Document>(EPOCH)
        .insert_one(doc! {"_id":SINGLETON_ID})
        .await
        .unwrap();
    let captured = capture(&db).await.unwrap();
    assert_eq!(captured, 0);
    let rows = [row("price", 2)];
    assert_eq!(
        publish_chunk(
            &db,
            Chunk {
                captured,
                plan: "standard",
                diagnostics: None,
                rejected: &[],
                rows: &rows
            }
        )
        .await
        .unwrap(),
        Some((1, 1))
    );
    assert_eq!(capture(&db).await.unwrap(), 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn failed_cache_mutation_rolls_back_the_epoch() {
    let db = database("rate_epoch_atomic_failure").await;
    db.create_collection(RATES).await.unwrap();
    db.run_command(doc! {"collMod":RATES,"validator":{"lago_metric_code":{"$ne":"bad"}},"validationLevel":"strict"}).await.unwrap();
    assert!(replace(&db, &row("bad", 1)).await.is_err());
    assert_eq!(capture(&db).await.unwrap(), 0);
    assert!(saved(&db, "bad").await.is_none());
    assert_eq!(
        db.collection::<Document>(EPOCH)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn explicit_and_periodic_command_counts() {
    let commands = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "rate_epoch_counts",
        mongodb::event::EventHandler::callback(move |event| {
            if let mongodb::event::command::CommandEvent::Started(e) = event {
                observed.lock().unwrap().push(e.command_name);
            }
        }),
    )
    .await;
    replace(&db, &row("seed", 1)).await.unwrap();
    let take = |label: &str, expected: usize| {
        let names = std::mem::take(&mut *commands.lock().unwrap());
        println!("RATE_EPOCH_COMMANDS {label}: {names:?}");
        assert_eq!(names.len(), expected, "{label}");
    };
    commands.lock().unwrap().clear();
    let captured = capture(&db).await.unwrap();
    take("capture", 1);
    let rows = [row("price", 2)];
    assert!(
        publish_chunk(
            &db,
            Chunk {
                captured,
                plan: "standard",
                diagnostics: Some(&[]),
                rejected: &[],
                rows: &rows
            }
        )
        .await
        .unwrap()
        .is_some()
    );
    take("publication_two_value_writes", 4);
    replace(&db, &row("price", 3)).await.unwrap();
    take("standalone_explicit", 3);
    retire(&db, "absent").await.unwrap();
    take("absent_retirement", 3);
    let context = db.clone();
    transaction_service::run_db_only(&db, &context, |session, db| {
        Box::pin(async move {
            advance_in_session(db, session).await?;
            db.collection::<BillingRateCache>(RATES)
                .update_many(
                    doc! {"lago_metric_code":"price"},
                    doc! {"$set":{"retired_at":bson::DateTime::now()}},
                )
                .session(session)
                .await?;
            Ok(())
        })
    })
    .await
    .unwrap();
    take("existing_transaction_mutation", 3);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn an_old_unfenced_writer_can_still_bypass_the_epoch() {
    let db = database("rate_epoch_mixed_fleet").await;
    replace(&db, &row("price", 2)).await.unwrap();
    let epoch = capture(&db).await.unwrap();
    db.collection::<BillingRateCache>(RATES)
        .replace_one(doc! {"_id":"price:*"}, row("price", 1))
        .await
        .unwrap();
    assert_eq!(
        saved(&db, "price").await.unwrap().credits_per_unit_micros,
        1_000_000
    );
    assert_eq!(capture(&db).await.unwrap(), epoch);
    db.drop().await.unwrap();
}
