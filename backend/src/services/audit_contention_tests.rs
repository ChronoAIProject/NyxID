//! Exclusive local experiments. Command gates create contention; elapsed times
//! are reported as measurements, never asserted as performance requirements.
use super::*;
use mongodb::{
    Client, Database, IndexModel,
    bson::Document,
    event::{EventHandler, cmap::CmapEvent, command::CommandEvent},
    options::{ClientOptions, IndexOptions},
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Barrier, watch},
    task::JoinSet,
    time::Instant,
};

const KEY: &[u8] = &[7; 32];
type AppendResult = Result<AuditLog, mongodb::error::Error>;
type AppendJobs = JoinSet<AppendResult>;

#[derive(Default)]
struct Occupancy {
    enabled: bool,
    holder: Option<Document>,
    checked_out: HashSet<u32>,
    ordinary: HashSet<u32>,
    max_ordinary: usize,
}

struct Probe {
    client: Client,
    occupancy: Arc<Mutex<Occupancy>>,
    inserts: watch::Receiver<usize>,
}

impl Probe {
    async fn new(pool: u32) -> Self {
        let occupancy = Arc::new(Mutex::new(Occupancy::default()));
        let (sent, inserts) = watch::channel(0);
        let mut options = ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        options.max_pool_size = Some(pool);
        options.min_pool_size = Some(0);
        let state = occupancy.clone();
        options.cmap_event_handler = Some(EventHandler::callback(move |event| {
            let mut state = state.lock().unwrap();
            match event {
                CmapEvent::ConnectionCheckedOut(e) => {
                    state.checked_out.insert(e.connection_id);
                }
                CmapEvent::ConnectionCheckedIn(e) => {
                    state.checked_out.remove(&e.connection_id);
                    state.ordinary.remove(&e.connection_id);
                }
                CmapEvent::ConnectionClosed(e) => {
                    state.checked_out.remove(&e.connection_id);
                    state.ordinary.remove(&e.connection_id);
                }
                _ => {}
            }
        }));
        let state = occupancy.clone();
        options.command_event_handler = Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(e) = event {
                let mut state = state.lock().unwrap();
                if !state.enabled
                    || e.command_name == "ping"
                    || e.command.get_document("lsid").ok() == state.holder.as_ref()
                {
                    return;
                }
                assert!(state.checked_out.contains(&e.connection.id));
                state.ordinary.insert(e.connection.id);
                state.max_ordinary = state.max_ordinary.max(state.ordinary.len());
                if e.command_name == "insert" {
                    sent.send_modify(|n| *n += 1);
                }
            }
        }));
        Self {
            client: Client::with_options(options).unwrap(),
            occupancy,
            inserts,
        }
    }

    fn observe(&self, holder: Option<Document>) {
        let mut state = self.occupancy.lock().unwrap();
        state.holder = holder;
        state.enabled = true;
    }

    async fn first_insert(&mut self) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while *self.inserts.borrow_and_update() == 0 {
                self.inserts.changed().await.unwrap();
            }
        })
        .await
        .expect("ordinary insert command gate");
    }

    fn finish(&self) -> usize {
        let mut state = self.occupancy.lock().unwrap();
        state.enabled = false;
        let peak = state.max_ordinary;
        assert_eq!(
            peak, 1,
            "ordinary appenders must use at most one socket per process"
        );
        peak
    }
}

fn entry() -> AuditLog {
    AuditLog {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: None,
        event_type: "pool_probe_worker".into(),
        event_data: None,
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
        seq: None,
        prev_hash: None,
        entry_hash: None,
        created_at: Utc::now(),
    }
}

async fn database(client: &Client) -> Database {
    let db = client.database(&format!("pool_2277_{}", uuid::Uuid::new_v4().simple()));
    db.collection::<Document>(AUDIT_LOG)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"seq":1})
                .options(
                    IndexOptions::builder()
                        .name("audit_log_seq_unique".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    db
}

async fn verify(db: &Database, count: usize) {
    let report = verify_chain(db, KEY, None, None, None).await.unwrap();
    assert_eq!(report.status, AuditChainStatus::Ok);
    assert_eq!(report.checked_count as usize, count);
    assert_eq!(report.head_seq.unwrap_or(0) as usize, count);
    assert_eq!(
        db.collection::<Document>(AUDIT_LOG)
            .count_documents(doc! {})
            .await
            .unwrap() as usize,
        count
    );
}

fn appenders(
    db: &Database,
    count: usize,
    appender: Option<Arc<OrdinaryAppender>>,
    budget: Duration,
) -> (AppendJobs, Arc<Barrier>) {
    let barrier = Arc::new(Barrier::new(count + 1));
    let mut jobs = JoinSet::new();
    for _ in 0..count {
        let db = db.clone();
        let barrier = barrier.clone();
        let appender = appender.clone();
        jobs.spawn(async move {
            barrier.wait().await;
            if let Some(appender) = appender {
                appender
                    .append_until(&db, entry(), KEY, Instant::now() + budget)
                    .await
            } else {
                append_chained_entry(&db, entry(), KEY).await
            }
        });
    }
    (jobs, barrier)
}

async fn completed(mut jobs: AppendJobs) -> Vec<AppendResult> {
    tokio::time::timeout(Duration::from_secs(180), async {
        let mut results = Vec::new();
        while let Some(result) = jobs.join_next().await {
            results.push(result.unwrap());
        }
        results
    })
    .await
    .expect("audit completion watchdog")
}

async fn lifetime(client: &Client, seconds: i32) {
    client
        .database("admin")
        .run_command(doc! {"setParameter":1,"transactionLifetimeLimitSeconds":seconds})
        .await
        .unwrap();
}

/// Plain insertion is deliberately a negative control: serialization leaves one
/// socket waiting on the holder, which still strands COMMIT with pool one.
async fn plain_insert_control(admin: &Client) {
    lifetime(admin, 5).await;
    let mut probe = Probe::new(1).await;
    let db = database(&probe.client).await;
    let mut holder = probe.client.start_session().await.unwrap();
    holder.start_transaction().await.unwrap();
    append_chained_entry_in_session(&db, &mut holder, entry(), KEY)
        .await
        .unwrap();
    probe.observe(Some(holder.id().clone()));
    let worker_db = db.clone();
    let worker = tokio::spawn(async move {
        // This is the old ordinary two-command wire path under a local gate.
        let gate = tokio::sync::Mutex::new(());
        let _guard = gate.lock().await;
        let tail = read_tail(&worker_db).await.unwrap();
        assert!(tail.is_none());
        let mut row = entry();
        prepare_chained_entry(&mut row, 1, GENESIS_PREV_HASH.into(), KEY).unwrap();
        worker_db
            .collection::<AuditLog>(AUDIT_LOG)
            .insert_one(row)
            .await
    });
    probe.first_insert().await;
    let start = Instant::now();
    let commit = tokio::time::timeout(Duration::from_secs(30), holder.commit_transaction())
        .await
        .unwrap();
    let inserted = worker.await.unwrap();
    let peak = probe.finish();
    eprintln!(
        "2277_A1_PLAIN pool=1 commit_ok={} insert_ok={} peak_ordinary={peak} elapsed_ms={}",
        commit.is_ok(),
        inserted.is_ok(),
        start.elapsed().as_millis()
    );
    assert!(
        commit.is_err(),
        "plain insert must wait on the holder and strand pool-one COMMIT"
    );
    inserted.unwrap();
    verify(&db, 1).await;
    db.drop().await.unwrap();
}

async fn held_sequence(count: usize, pool: u32) {
    let mut probe = Probe::new(pool).await;
    let db = database(&probe.client).await;
    let mut holder = probe.client.start_session().await.unwrap();
    holder.start_transaction().await.unwrap();
    append_chained_entry_in_session(&db, &mut holder, entry(), KEY)
        .await
        .unwrap();
    probe.observe(Some(holder.id().clone()));
    let start = Instant::now();
    let (jobs, gate) = appenders(&db, count, None, APPEND_BUDGET);
    gate.wait().await;
    probe.first_insert().await;
    // Both use the same pool as the appenders. Success proves free capacity.
    db.run_command(doc! {"ping":1}).await.unwrap();
    let commit_start = Instant::now();
    holder.commit_transaction().await.unwrap();
    let commit_ms = commit_start.elapsed().as_millis();
    let results = completed(jobs).await;
    let errors = results.iter().filter(|r| r.is_err()).count();
    let peak = probe.finish();
    eprintln!(
        "2277_A1_HELD pool={pool} N={count} errors={errors} peak_ordinary={peak} commit_ms={commit_ms} elapsed_ms={}",
        start.elapsed().as_millis()
    );
    assert_eq!(errors, 0);
    verify(&db, count + 1).await;
    db.drop().await.unwrap();
}

async fn replica_contention() {
    let one = Probe::new(10).await;
    let two = Probe::new(10).await;
    let db = database(&one.client).await;
    let second = two.client.database(db.name());
    one.observe(None);
    two.observe(None);
    // Independent gates model separate processes, using the exact same method
    // as the process-global production appender, with separate MongoDB clients.
    let (a, ga) = appenders(
        &db,
        200,
        Some(Arc::new(OrdinaryAppender::new())),
        APPEND_BUDGET,
    );
    let (b, gb) = appenders(
        &second,
        200,
        Some(Arc::new(OrdinaryAppender::new())),
        APPEND_BUDGET,
    );
    let start = Instant::now();
    tokio::join!(ga.wait(), gb.wait());
    let (a, b) = tokio::join!(completed(a), completed(b));
    let errors = a.iter().chain(&b).filter(|r| r.is_err()).count();
    let peaks = (one.finish(), two.finish());
    eprintln!(
        "2277_A1_REPLICAS clients=2 N=400 errors={errors} peak_ordinary={peaks:?} elapsed_ms={}",
        start.elapsed().as_millis()
    );
    assert_eq!(errors, 0);
    verify(&db, 400).await;
    db.drop().await.unwrap();
}

async fn abandoned_holder_expires(admin: &Client) {
    lifetime(admin, 5).await;
    let mut probe = Probe::new(10).await;
    let db = database(&probe.client).await;
    let mut holder = probe.client.start_session().await.unwrap();
    holder.start_transaction().await.unwrap();
    append_chained_entry_in_session(&db, &mut holder, entry(), KEY)
        .await
        .unwrap();
    probe.observe(Some(holder.id().clone()));
    let (jobs, gate) = appenders(&db, 13, None, APPEND_BUDGET);
    gate.wait().await;
    probe.first_insert().await;
    db.run_command(doc! {"ping":1}).await.unwrap();
    let start = Instant::now();
    let results = completed(jobs).await;
    let peak = probe.finish();
    eprintln!(
        "2277_A1_ORPHAN N=13 errors={} peak_ordinary={peak} elapsed_ms={}",
        results.iter().filter(|r| r.is_err()).count(),
        start.elapsed().as_millis()
    );
    assert!(results.iter().all(Result::is_ok));
    assert!(holder.commit_transaction().await.is_err());
    verify(&db, 13).await;
    db.drop().await.unwrap();
    lifetime(admin, 10).await;
}

async fn never_commits() {
    let mut probe = Probe::new(10).await;
    let db = database(&probe.client).await;
    let mut holder = probe.client.start_session().await.unwrap();
    holder.start_transaction().await.unwrap();
    append_chained_entry_in_session(&db, &mut holder, entry(), KEY)
        .await
        .unwrap();
    probe.observe(Some(holder.id().clone()));
    // Shortened retry/queue budget exercises expiry before the 10-second
    // holder expires. Production uses 120 seconds, not this test policy.
    let (jobs, gate) = appenders(
        &db,
        13,
        Some(Arc::new(OrdinaryAppender::new())),
        Duration::from_secs(2),
    );
    gate.wait().await;
    probe.first_insert().await;
    let (stop, mut stopping) = watch::channel(false);
    let traffic_db = db.clone();
    let pings = tokio::spawn(async move {
        let mut count = 0;
        loop {
            tokio::select! {
                _ = stopping.changed() => break,
                result = traffic_db.run_command(doc! {"ping":1}) => { result.unwrap(); count += 1; }
            }
        }
        count
    });
    let start = Instant::now();
    let results = completed(jobs).await;
    stop.send_replace(true);
    let pings = pings.await.unwrap();
    let peak = probe.finish();
    eprintln!(
        "2277_A1_DEADLINE N=13 errors={} peak_ordinary={peak} pings={pings} elapsed_ms={}",
        results.iter().filter(|r| r.is_err()).count(),
        start.elapsed().as_millis()
    );
    assert!(results.iter().all(Result::is_err));
    assert!(pings > 0);
    assert!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|error| error.contains_label("TransientTransactionError")
                || error.get_custom::<&str>() == Some(&"audit chain append deadline exceeded"))
    );
    holder.abort_transaction().await.unwrap();
    verify(&db, 0).await;
    db.drop().await.unwrap();
}

async fn throughput() {
    let probe = Probe::new(10).await;
    let db = database(&probe.client).await;
    probe.observe(None);
    let (jobs, gate) = appenders(&db, 200, None, APPEND_BUDGET);
    let start = Instant::now();
    gate.wait().await;
    let results = completed(jobs).await;
    let elapsed = start.elapsed();
    let peak = probe.finish();
    eprintln!(
        "2277_A1_THROUGHPUT N=200 errors={} peak_ordinary={peak} elapsed_ms={} appends_per_second={:.1}",
        results.iter().filter(|r| r.is_err()).count(),
        elapsed.as_millis(),
        200.0 / elapsed.as_secs_f64()
    );
    assert!(results.iter().all(Result::is_ok));
    verify(&db, 200).await;
    db.drop().await.unwrap();
}

#[tokio::test]
#[ignore = "exclusive disposable MongoDB; changes transaction lifetime"]
async fn serialized_audits_preserve_entries_without_starving_the_holder() {
    let admin = Probe::new(2).await;
    plain_insert_control(&admin.client).await;
    lifetime(&admin.client, 10).await;
    for count in [10, 13, 200] {
        held_sequence(count, 10).await;
    }
    held_sequence(13, 1).await;
    replica_contention().await;
    abandoned_holder_expires(&admin.client).await;
    never_commits().await;
    throughput().await;
    lifetime(&admin.client, 60).await;
}
