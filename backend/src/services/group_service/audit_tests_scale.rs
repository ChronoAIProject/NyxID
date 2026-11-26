use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const USERS: usize = 20_000;
const ACTIVE_USERS: usize = 1_000;
const WRITERS: usize = 4;
const WRITE_INTERVAL: Duration = Duration::from_millis(160);
const MAX_RETRIES: usize = 128;

// This is an operating-envelope probe, not a CI timing contract.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Timing-dependent contention probe; run through cargo-slot.sh: cargo +1.98.1 test -p nyxid --bin nyxid-server services::group_service::audit_tests::scale:: -- --ignored --test-threads=1 --nocapture"]
async fn group_delete_twenty_thousand_members_under_rotating_writes() {
    use futures::StreamExt;
    for repetition in 0..3 {
        let mut f = fixture(Action::Delete).await;
        let db = f.state.db.clone();
        crate::db::ensure_indexes(&db).await.unwrap();
        let ids: Vec<_> = (0..USERS - 2)
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect();
        for batch in ids.chunks(1_000) {
            db.collection::<User>("users")
                .insert_many(batch.iter().map(|id| {
                    let mut user = test_user(id, UserType::Person);
                    user.display_name = Some("x".repeat(4096));
                    user.group_ids = vec![f.group.clone()];
                    user
                }))
                .await
                .unwrap();
        }
        db.collection::<Document>("users")
            .update_one(
                doc! {"_id":&f.actor},
                doc! {"$addToSet":{"group_ids":&f.group}},
            )
            .await
            .unwrap();
        let transactions = Arc::new(Mutex::new(HashSet::new()));
        let seen = transactions.clone();
        let mut options = mongodb::options::ClientOptions::parse(
            std::env::var("NYXID_TEST_DATABASE_URL").unwrap(),
        )
        .await
        .unwrap();
        options.command_event_handler = Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(e) = event
                && e.command.get_bool("startTransaction") == Ok(true)
            {
                seen.lock()
                    .unwrap()
                    .insert(e.command.get_i64("txnNumber").unwrap());
            }
        }));
        f.state.db = mongodb::Client::with_options(options)
            .unwrap()
            .database(db.name());
        let writes = Arc::new(AtomicU64::new(0));
        let counter = writes.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let cancelled = stop.clone();
        let writer_db = db.clone();
        let task = crate::services::shutdown::process().spawn(async move {
            let start = tokio::time::Instant::now();
            futures::stream::iter(0..WRITERS).for_each_concurrent(WRITERS, |worker| {
                let db = &writer_db;
                let ids = &ids;
                let cancelled = &cancelled;
                let counter = &counter;
                async move {
                    let mut interval = tokio::time::interval_at(start + WRITE_INTERVAL / WRITERS as u32 * worker as u32, WRITE_INTERVAL);
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    let mut position = worker;
                    loop {
                        tokio::select! { _ = cancelled.cancelled() => break, _ = interval.tick() => {} }
                        let index = ((position * 7919 + repetition * 137) % ACTIVE_USERS) * (ids.len() / ACTIVE_USERS);
                        db.collection::<Document>("users").update_one(doc! {"_id":&ids[index]},doc! {"$currentDate":{"last_login_at":true,"updated_at":true}}).await.unwrap();
                        counter.fetch_add(1, Ordering::SeqCst);
                        position += WRITERS;
                    }
                }
            }).await;
        });
        let started = Instant::now();
        let response = request(&f, Action::Delete).await;
        let elapsed = started.elapsed();
        stop.cancel();
        task.await.unwrap();
        let retries = transactions.lock().unwrap().len().saturating_sub(1);
        let writes = writes.load(Ordering::SeqCst);
        println!(
            "GROUP_SCALE users={USERS} padded_users={} repetition={repetition} offered_writes_per_second=25 elapsed_ms={} retries={retries} concurrent_writes={writes} status={}",
            USERS - 2,
            elapsed.as_millis(),
            response.status()
        );
        assert!(response.status().is_success(), "{}", json(response).await);
        assert_eq!(
            db.collection::<Document>("users")
                .count_documents(doc! {"group_ids":&f.group})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>("groups")
                .count_documents(doc! {"_id":&f.group})
                .await
                .unwrap(),
            0
        );
        assert_eq!(snapshot(&db, INTENTS).await.len(), 1);
        assert!(writes > 0, "load must overlap the operation");
        assert!(
            retries <= MAX_RETRIES,
            "retries {retries} exceeds manual probe bound"
        );
        db.drop().await.unwrap();
    }
}
