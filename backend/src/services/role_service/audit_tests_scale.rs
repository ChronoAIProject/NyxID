use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const USERS: usize = 20_000;
const GROUPS: usize = 1_000;
const ACTIVE_USERS: usize = 1_000;
const WRITERS: usize = 4;
const WRITE_INTERVAL: Duration = Duration::from_millis(160);
const MAX_RETRIES: usize = 128;
const REPETITIONS: usize = 3;

struct Load {
    stop: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<()>,
    writes: Arc<AtomicU64>,
}
impl Drop for Load {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}
async fn writer(db: mongodb::Database, ids: Vec<String>, offset: usize) -> Load {
    use futures::StreamExt;
    let stop = tokio_util::sync::CancellationToken::new();
    let cancelled = stop.clone();
    let writes = Arc::new(AtomicU64::new(0));
    let counter = writes.clone();
    let start = tokio::time::Instant::now();
    let task = crate::services::shutdown::process().spawn(async move {
        futures::stream::iter(0..WRITERS)
            .for_each_concurrent(WRITERS, |worker| {
                let db = &db;
                let ids = &ids;
                let cancelled = &cancelled;
                let counter = &counter;
                async move {
                    let mut interval = tokio::time::interval_at(
                        start + WRITE_INTERVAL / WRITERS as u32 * worker as u32,
                        WRITE_INTERVAL,
                    );
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    let mut position = worker;
                    loop {
                        tokio::select! {
                            _ = cancelled.cancelled() => break,
                            _ = interval.tick() => {}
                        }
                        // Spread updates over the whole collection rather than
                        // blocking all writers behind one early locked document.
                        let index =
                            ((position * 7919 + offset) % ACTIVE_USERS) * (USERS / ACTIVE_USERS);
                        let result = db
                            .collection::<Document>("users")
                            .update_one(
                                doc! {"_id":&ids[index]},
                                doc! {"$currentDate":{"last_login_at":true,"updated_at":true}},
                            )
                            .await
                            .unwrap();
                        assert_eq!(result.matched_count, 1);
                        counter.fetch_add(1, Ordering::SeqCst);
                        position += WRITERS;
                    }
                }
            })
            .await;
    });
    Load { stop, task, writes }
}

struct Scale {
    f: Fixture,
    ids: Vec<String>,
    transactions: Arc<Mutex<HashSet<i64>>>,
}
async fn scale_fixture() -> Scale {
    let mut f = fixture(Action::Assign).await;
    let db = f.state.db.clone();
    crate::db::ensure_indexes(&db).await.unwrap();
    let mut ids = vec![f.actor.clone(), f.target.clone()];
    for _ in 2..USERS {
        ids.push(uuid::Uuid::new_v4().to_string());
    }
    let user = |id: &String| {
        let mut user = test_user(id, UserType::Person);
        user.display_name = Some("x".repeat(4096));
        user
    };
    for batch in ids[2..].chunks(1000) {
        db.collection::<User>("users")
            .insert_many(batch.iter().map(user))
            .await
            .unwrap();
    }
    let mut group_ids = vec!["group".to_owned()];
    for _ in 1..GROUPS {
        group_ids.push(uuid::Uuid::new_v4().to_string());
    }
    db.collection::<Document>("groups")
        .insert_many(
            group_ids[1..]
                .iter()
                .map(|id| doc! {"_id":id,"name":id,"slug":id,"role_ids":[&f.role]}),
        )
        .await
        .unwrap();
    let transactions = Arc::new(Mutex::new(HashSet::new()));
    let seen = transactions.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
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
    let bytes = bson::to_vec(&user(&ids[2])).unwrap().len();
    println!(
        "ROLE_SCALE_FIXTURE users={USERS} groups={GROUPS} user_bytes={bytes} offered_writes_per_second=25 rotating_users={ACTIVE_USERS} writers={WRITERS}"
    );
    Scale {
        f,
        ids,
        transactions,
    }
}
async fn measured_operation(s: &Scale, action: Action, repetition: usize, expected_success: bool) {
    let f = &s.f;
    let db = &f.state.db;
    let before_intents = db
        .collection::<Document>(INTENTS)
        .count_documents(doc! {"event.event_type":action.event()})
        .await
        .unwrap();
    s.transactions.lock().unwrap().clear();
    let mut load = writer(db.clone(), s.ids.clone(), repetition * 137).await;
    let before_writes = load.writes.load(Ordering::SeqCst);
    let start = Instant::now();
    let response = match action {
        Action::Bulk => handlers::bulk_assign_role(
            State(f.state.clone()),
            audit_auth(&f.actor),
            HeaderMap::new(),
            Path(f.role.clone()),
            Json(handlers::BulkAssignRequest {
                all: true,
                user_ids: vec![],
            }),
        )
        .await
        .into_response(),
        Action::Delete => request(f, action).await,
        _ => unreachable!(),
    };
    let elapsed = start.elapsed();
    let status = response.status();
    let body = json(response).await;
    load.stop.cancel();
    (&mut load.task).await.unwrap();
    let writes = load.writes.load(Ordering::SeqCst) - before_writes;
    let attempts = s.transactions.lock().unwrap().len();
    let retries = attempts.saturating_sub(1);
    println!(
        "ROLE_SCALE repetition={repetition} operation={action:?} expected_success={expected_success} elapsed_ms={} attempts={attempts} retries={retries} concurrent_writes={writes} observed_writes_per_second={:.1} status={} error_code={}",
        elapsed.as_millis(),
        writes as f64 / elapsed.as_secs_f64(),
        status.as_u16(),
        body.get("error_code").unwrap_or(&serde_json::Value::Null)
    );
    let remaining = db
        .collection::<User>("users")
        .count_documents(doc! {"role_ids":&f.role})
        .await
        .unwrap();
    let intents = db
        .collection::<Document>(INTENTS)
        .count_documents(doc! {"event.event_type":action.event()})
        .await
        .unwrap();
    if !status.is_success() {
        assert_eq!(status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error_code"], 1007);
        assert_eq!(
            remaining,
            if matches!(action, Action::Bulk) {
                0
            } else {
                USERS as u64
            },
            "failure must not partially apply"
        );
        assert_eq!(intents, before_intents, "failure must not add an intent");
        assert_eq!(
            db.collection::<Document>("groups")
                .count_documents(doc! {"role_ids":&f.role})
                .await
                .unwrap(),
            GROUPS as u64
        );
        assert!(
            db.collection::<Role>("roles")
                .find_one(doc! {"_id":&f.role})
                .await
                .unwrap()
                .is_some()
        );
    } else {
        assert_eq!(
            remaining,
            if matches!(action, Action::Bulk) {
                USERS as u64
            } else {
                0
            }
        );
        assert_eq!(intents, before_intents + 1);
        if matches!(action, Action::Bulk) {
            assert_eq!(body["assigned_count"], USERS);
        } else {
            assert_eq!(
                db.collection::<Document>("groups")
                    .count_documents(doc! {"role_ids":&f.role})
                    .await
                    .unwrap(),
                0
            );
        }
    }
    assert_eq!(
        status.is_success(),
        expected_success,
        "{action:?} unexpected outcome: {body}"
    );
    assert!(writes > 0, "load must overlap the operation");
    assert!(
        retries <= MAX_RETRIES,
        "{action:?} exceeded {MAX_RETRIES} retries: {retries}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Timing-dependent contention probe; run: cargo +1.98.1 test -p nyxid --bin nyxid-server services::role_service::audit_tests::scale:: -- --ignored --test-threads=1 --nocapture"]
async fn twenty_thousand_users_bulk_and_delete_converge_under_rotating_writes() {
    let s = scale_fixture().await;
    let db = &s.f.state.db;
    let role = db
        .collection::<Role>("roles")
        .find_one(doc! {"_id":&s.f.role})
        .await
        .unwrap()
        .unwrap();
    for repetition in 1..=REPETITIONS {
        if repetition > 1 {
            db.collection::<Role>("roles")
                .insert_one(&role)
                .await
                .unwrap();
            db.collection::<Document>("groups")
                .update_many(doc! {}, doc! {"$set":{"role_ids":[&s.f.role]}})
                .await
                .unwrap();
        }
        for action in [Action::Bulk, Action::Delete] {
            measured_operation(&s, action, repetition, true).await;
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Timing-dependent contention probe; run: cargo +1.98.1 test -p nyxid --bin nyxid-server services::role_service::audit_tests::scale:: -- --ignored --test-threads=1 --nocapture"]
async fn twenty_thousand_users_intent_failure_rolls_back_bulk_and_delete_under_load() {
    let s = scale_fixture().await;
    let db = &s.f.state.db;
    db.run_command(doc! {"collMod":INTENTS,"validator":{"forbidden":{"$exists":true}}})
        .await
        .unwrap();
    measured_operation(&s, Action::Bulk, 1, false).await;
    db.run_command(doc! {"collMod":INTENTS,"validator":{}})
        .await
        .unwrap();
    measured_operation(&s, Action::Bulk, 2, true).await;
    db.run_command(doc! {"collMod":INTENTS,"validator":{"forbidden":{"$exists":true}}})
        .await
        .unwrap();
    measured_operation(&s, Action::Delete, 3, false).await;
    db.drop().await.unwrap();
}
