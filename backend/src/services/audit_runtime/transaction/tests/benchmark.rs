//! Test-only copy of the previous direct append algorithm at 866f289df. It has
//! no production entrypoint. Shared canonical v1 hashing is unchanged. Compare
//! the actual public append operation (all local guarantees included), not just
//! the hash or a write with reservations/validation disabled.
use super::*;
use rand::Rng;
use std::time::Instant;

async fn legacy_append(db: &Database, mut event: AuditLog) -> mongodb::error::Result<()> {
    let rows = db.collection::<AuditLog>(AUDIT);
    for attempt in 1..=8 {
        let tail = rows
            .find_one(doc! {"seq": {"$exists": true}})
            .sort(doc! {"seq": -1})
            .await?;
        let (seq, previous) = match tail {
            Some(tail) => (tail.seq.unwrap() + 1, tail.entry_hash.unwrap()),
            None => (1, chain::GENESIS_PREV_HASH.to_owned()),
        };
        chain::prepare_chained_entry(&mut event, seq, previous, &KEY)
            .map_err(mongodb::error::Error::custom)?;
        match rows.insert_one(&event).await {
            Ok(_) => return Ok(()),
            Err(error)
                if matches!(error.kind.as_ref(), ErrorKind::Write(
                mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000) =>
            {
                if let Some(existing) = rows.find_one(doc! {"_id": &event.id}).await? {
                    if chain::immutable_entry_matches(&existing, &event)
                        && existing.seq.is_some()
                        && existing.prev_hash.is_some()
                        && existing.entry_hash.as_ref().is_some_and(|hash| {
                            chain::compute_entry_hash(&existing, &KEY)
                                .is_ok_and(|actual| &actual == hash)
                        })
                    {
                        return Ok(());
                    }
                    return Err(mongodb::error::Error::custom(
                        "conflicting legacy audit identity",
                    ));
                }
                if attempt == 8 {
                    return Err(error);
                }
                let base = 5_u64.saturating_mul(1_u64 << attempt.min(5));
                let jitter = rand::thread_rng().gen_range(0..=10);
                tokio::time::sleep(std::time::Duration::from_millis(base + jitter)).await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("last previous-implementation attempt returns")
}

struct Measurement {
    per_sec: f64,
    succeeded: usize,
    exhausted: usize,
}

async fn measure(tasks: usize, legacy: bool) -> Measurement {
    let db = connect_transaction_test_database("audit_append_bench").await;
    let gate = Arc::new(Barrier::new(tasks + 1));
    let mut workers = Vec::new();
    for _ in 0..tasks {
        let (db, gate) = (db.clone(), gate.clone());
        workers.push(tokio::spawn(async move {
            let events: Vec<_> = (0..256 / tasks).map(|_| entry()).collect();
            let (mut succeeded, mut exhausted) = (0, 0);
            gate.wait().await;
            for event in events {
                if legacy {
                    match legacy_append(&db, event).await {
                        Ok(()) => succeeded += 1,
                        // Preserve the old eight-attempt budget. Its ordinary
                        // sequence contention can exhaust; report it instead
                        // of aborting before measuring the durable path.
                        Err(error)
                            if matches!(error.kind.as_ref(), ErrorKind::Write(
                            mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000) =>
                        {
                            exhausted += 1;
                        }
                        Err(error) => panic!("unexpected previous-path error: {error}"),
                    }
                } else {
                    chain::append_chained_entry(&db, event, &KEY).await.unwrap();
                    succeeded += 1;
                }
            }
            (succeeded, exhausted)
        }));
    }
    let started = Instant::now();
    gate.wait().await;
    let (mut succeeded, mut exhausted) = (0, 0);
    for worker in workers {
        let counts = worker.await.unwrap();
        succeeded += counts.0;
        exhausted += counts.1;
    }
    let per_sec = succeeded as f64 / started.elapsed().as_secs_f64();
    assert_eq!(succeeded + exhausted, 256);
    if !legacy {
        assert_eq!(succeeded, 256);
        assert_eq!(exhausted, 0);
    }
    // Setup and full HMAC/state/outbox verification are outside measured time.
    assert_chain(&db, succeeded, !legacy, 0).await;
    Measurement {
        per_sec,
        succeeded,
        exhausted,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual same-machine previous-vs-durable audit throughput benchmark"]
async fn audit_append_throughput_previous_vs_durable() {
    for tasks in [1, 16] {
        let (mut previous, mut durable) = (Vec::new(), Vec::new());
        for round in 0..3 {
            let (old, new) = if round % 2 == 0 {
                let old = measure(tasks, true).await;
                (old, measure(tasks, false).await)
            } else {
                let new = measure(tasks, false).await;
                (measure(tasks, true).await, new)
            };
            eprintln!(
                "audit_append_benchmark tasks={tasks} round={} attempted=256 previous_per_sec={:.2} previous_succeeded={} previous_exhausted={} durable_per_sec={:.2} durable_succeeded={} durable_exhausted={} successful_throughput_ratio={:.2}x",
                round + 1,
                old.per_sec,
                old.succeeded,
                old.exhausted,
                new.per_sec,
                new.succeeded,
                new.exhausted,
                old.per_sec / new.per_sec
            );
            previous.push(old);
            durable.push(new);
        }
        previous.sort_by(|a, b| a.per_sec.total_cmp(&b.per_sec));
        durable.sort_by(|a, b| a.per_sec.total_cmp(&b.per_sec));
        let previous_exhausted: usize = previous.iter().map(|round| round.exhausted).sum();
        eprintln!(
            "audit_append_benchmark MEDIAN tasks={tasks} rounds=3 attempted_per_round=256 previous_per_sec={:.2} previous_exhausted_total={previous_exhausted} durable_per_sec={:.2} durable_exhausted_total=0 successful_throughput_ratio={:.2}x comparable_all_success={}",
            previous[1].per_sec,
            durable[1].per_sec,
            previous[1].per_sec / durable[1].per_sec,
            previous_exhausted == 0
        );
    }
}

// Exact pre-queue durable owner from committed 1d705e83a. Test-only: the
// live append body, snapshot, CAS, HMAC, majority+journal and 32/8 budgets are
// shared. This arm omits only local scheduling, never durable audit work.
async fn unqueued_durable_append(
    db: &Database,
    event: AuditLog,
    bodies: &mut u32,
    commits: &mut u32,
) -> AppResult<AuditLog> {
    let mut session = db.client().start_session().await?;
    for attempt in 1..=MAX_BODY_ATTEMPTS {
        *bodies = attempt;
        session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(state::majority())
            .await?;
        let result = match state::append_in_session(db, &mut session, event.clone(), &KEY).await {
            Ok(value) => commit_observed(&mut session, || *commits += 1)
                .await
                .map(|()| value)
                .map_err(AppError::from),
            Err(error) => Err(error),
        };
        match result {
            Ok(value) => return Ok(value),
            Err(AppError::DatabaseError(error))
                if error.contains_label("UnknownTransactionCommitResult") =>
            {
                return Err(AppError::DatabaseError(error));
            }
            Err(error) => {
                let _ = session.abort_transaction().await;
                if attempt == MAX_BODY_ATTEMPTS || !retryable(&error) {
                    return Err(error);
                }
                backoff(attempt).await;
            }
        }
    }
    unreachable!("last bounded body attempt returns")
}

#[derive(Default)]
struct FifoMeasurement {
    per_sec: f64,
    succeeded: usize,
    failed: usize,
    bodies: u32,
    max_bodies: u32,
    max_commits: u32,
}

async fn measure_durable_fifo(tasks: usize, fifo: bool) -> FifoMeasurement {
    let db = connect_transaction_test_database("audit_fifo_ab").await;
    let barrier = Arc::new(Barrier::new(tasks + 1));
    let mut workers = Vec::new();
    for _ in 0..tasks {
        let (db, barrier) = (db.clone(), barrier.clone());
        workers.push(tokio::spawn(async move {
            let events: Vec<_> = (0..256 / tasks).map(|_| entry()).collect();
            let mut stats = FifoMeasurement::default();
            barrier.wait().await;
            for event in events {
                let (mut bodies, mut commits) = (0, 0);
                let result = if fifo {
                    chain::append_chained_entry_observed(&db, event, &KEY, |b, c| {
                        (bodies, commits) = (b, c);
                    })
                    .await
                    .map_err(AppError::from)
                } else {
                    unqueued_durable_append(&db, event, &mut bodies, &mut commits).await
                };
                stats.bodies += bodies;
                stats.max_bodies = stats.max_bodies.max(bodies);
                stats.max_commits = stats.max_commits.max(commits);
                match result {
                    Ok(_) => stats.succeeded += 1,
                    Err(error) => {
                        stats.failed += 1;
                        // Metadata only; no event or database command bodies.
                        eprintln!("audit_fifo_ab failure fifo={fifo} tasks={tasks} bodies={bodies} commits={commits} transient={} unknown_commit={}",
                            matches!(&error, AppError::DatabaseError(e) if e.contains_label("TransientTransactionError")),
                            matches!(&error, AppError::DatabaseError(e) if e.contains_label("UnknownTransactionCommitResult")));
                    }
                }
            }
            stats
        }));
    }
    let started = Instant::now();
    barrier.wait().await;
    let mut stats = FifoMeasurement::default();
    for worker in workers {
        let result = worker.await.unwrap();
        stats.succeeded += result.succeeded;
        stats.failed += result.failed;
        stats.bodies += result.bodies;
        stats.max_bodies = stats.max_bodies.max(result.max_bodies);
        stats.max_commits = stats.max_commits.max(result.max_commits);
    }
    stats.per_sec = stats.succeeded as f64 / started.elapsed().as_secs_f64();
    assert_chain(&db, stats.succeeded, true, 0).await;
    crate::test_utils::teardown_test_database(&db).await;
    stats
}

// Optional protocol for the external host-wide process monitor. It samples
// throughout each arm and snapshots load/swap/processes at these boundaries.
// Complete in-flight writes and fixture cleanup before rejecting a noisy arm;
// never retain either arm of a contaminated pair in the reported median.
fn host_gate(phase: &str, tasks: usize, round: usize, attempt: usize) -> bool {
    if std::env::var_os("NYXID_FIFO_AB_HOST_GATE").is_none() {
        return true;
    }
    eprintln!(
        "audit_fifo_ab HOST_GATE phase={phase} tasks={tasks} round={round} attempt={attempt}"
    );
    let mut response = String::new();
    std::io::stdin().read_line(&mut response).unwrap();
    match response.trim() {
        "quiet" => true,
        "retry" => false,
        other => panic!("host monitor closed or sent an invalid gate response: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual controlled durable audit FIFO versus unqueued owner benchmark"]
async fn audit_append_throughput_durable_fifo_vs_unqueued() {
    let mut failed = 0;
    for tasks in [1, 16] {
        let (mut unqueued, mut fifo) = (Vec::new(), Vec::new());
        for round in 0..3 {
            let mut attempt = 0;
            let (without, with) = loop {
                attempt += 1;
                assert!(
                    attempt <= 20,
                    "host noise exhausted manual benchmark attempts"
                );
                if !host_gate("start", tasks, round + 1, attempt) {
                    continue;
                }
                let fifo_first = round % 2 != 0;
                let first = measure_durable_fifo(tasks, fifo_first).await;
                if !host_gate("after_first", tasks, round + 1, attempt) {
                    eprintln!(
                        "audit_fifo_ab ABORTED tasks={tasks} round={} attempt={attempt} after=first",
                        round + 1
                    );
                    continue;
                }
                let second = measure_durable_fifo(tasks, !fifo_first).await;
                if !host_gate("after_second", tasks, round + 1, attempt) {
                    eprintln!(
                        "audit_fifo_ab ABORTED tasks={tasks} round={} attempt={attempt} after=second",
                        round + 1
                    );
                    continue;
                }
                break if fifo_first {
                    (second, first)
                } else {
                    (first, second)
                };
            };
            for (queued, result) in [(false, &without), (true, &with)] {
                eprintln!(
                    "audit_fifo_ab tasks={tasks} round={} fifo={queued} attempted=256 succeeded={} failed={} appends_per_sec={:.2} body_attempts={} max_body_attempts={} max_commit_attempts={}",
                    round + 1,
                    result.succeeded,
                    result.failed,
                    result.per_sec,
                    result.bodies,
                    result.max_bodies,
                    result.max_commits
                );
                failed += result.failed;
            }
            unqueued.push(without.per_sec);
            fifo.push(with.per_sec);
        }
        unqueued.sort_by(f64::total_cmp);
        fifo.sort_by(f64::total_cmp);
        eprintln!(
            "audit_fifo_ab MEDIAN tasks={tasks} rounds=3 unqueued_per_sec={:.2} fifo_per_sec={:.2} fifo_over_unqueued={:.3}",
            unqueued[1],
            fifo[1],
            fifo[1] / unqueued[1]
        );
    }
    assert_eq!(
        failed, 0,
        "all durable benchmark appends must succeed within unchanged budgets"
    );
}
