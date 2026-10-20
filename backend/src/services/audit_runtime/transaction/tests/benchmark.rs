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
