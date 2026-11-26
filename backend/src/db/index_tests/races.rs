use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

use futures::FutureExt;
use mongodb::{
    Client, Database, IndexModel,
    bson::{Document, doc},
    event::{EventHandler, command::CommandEvent},
    options::ClientOptions,
};
use tokio::sync::Notify;

use super::{definition, ensure_indexes};

const BUILD_PAUSE: &str = "hangAfterInitializingIndexBuild";
const TARGETS: &[(&str, &str)] = &[
    ("nodes", "user_id_1_name_1"),
    ("node_service_bindings", "node_id_1_service_id_1"),
    (
        "coordination_leases",
        "coordination_leases_ephemeral_expiry_ttl",
    ),
];

#[derive(Default)]
struct Trace {
    lists: AtomicUsize,
    drops: AtomicUsize,
    creates: AtomicUsize,
    requests: Mutex<HashMap<i32, String>>,
    errors: Mutex<Vec<(String, i32)>>,
}

struct Gate {
    command: &'static str,
    armed: AtomicBool,
    reached: Notify,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Gate {
    fn new(command: &'static str) -> (Arc<Self>, mpsc::SyncSender<()>) {
        let (tx, rx) = mpsc::sync_channel(1);
        (
            Arc::new(Self {
                command,
                armed: AtomicBool::new(true),
                reached: Notify::new(),
                release: Mutex::new(rx),
            }),
            tx,
        )
    }
}

async fn monitored_client(
    base: &Database,
    collection: &'static str,
    name: &'static str,
    trace: Arc<Trace>,
    gate: Option<Arc<Gate>>,
    collision: Option<tokio::sync::mpsc::UnboundedSender<i32>>,
) -> (Database, String) {
    let mut options = ClientOptions::parse(
        std::env::var("NYXID_TEST_DATABASE_URL").expect("MongoDB failpoint test URL"),
    )
    .await
    .unwrap();
    let app = format!("index-race-{}", uuid::Uuid::new_v4());
    options.app_name = Some(app.clone());
    options.retry_reads = Some(false);
    options.retry_writes = Some(false);
    options.command_event_handler = Some(EventHandler::callback(move |event| match event {
        CommandEvent::Started(event) => {
            let command = event.command_name.as_str();
            let target = event.command.get_str(command) == Ok(collection)
                && match command {
                    "listIndexes" => true,
                    "dropIndexes" => event.command.get_str("index") == Ok(name),
                    "createIndexes" => event.command.get_array("indexes").is_ok_and(|indexes| {
                        indexes.iter().any(|index| {
                            index
                                .as_document()
                                .is_some_and(|index| index.get_str("name") == Ok(name))
                        })
                    }),
                    _ => false,
                };
            if target {
                match command {
                    "listIndexes" => &trace.lists,
                    "dropIndexes" => &trace.drops,
                    _ => &trace.creates,
                }
                .fetch_add(1, Ordering::SeqCst);
                trace
                    .requests
                    .lock()
                    .unwrap()
                    .insert(event.request_id, event.command_name.clone());
                if let Some(gate) = &gate
                    && command == gate.command
                    && gate.armed.swap(false, Ordering::SeqCst)
                {
                    gate.reached.notify_one();
                    let _ = gate.release.lock().unwrap().recv();
                }
            }
        }
        CommandEvent::Failed(event) => {
            if let Some(command) = trace.requests.lock().unwrap().remove(&event.request_id)
                && let mongodb::error::ErrorKind::Command(error) = event.failure.kind.as_ref()
            {
                trace.errors.lock().unwrap().push((command, error.code));
                if matches!(error.code, 276 | 12587)
                    && let Some(collision) = &collision
                {
                    let _ = collision.send(error.code);
                }
            }
        }
        CommandEvent::Succeeded(event) => {
            trace.requests.lock().unwrap().remove(&event.request_id);
        }
        _ => {}
    }));
    (
        Client::with_options(options).unwrap().database(base.name()),
        app,
    )
}

async fn legacy_fixture(collection: &str, name: &str) -> (Database, IndexModel) {
    let db = crate::test_utils::connect_test_database("idx_race")
        .await
        .unwrap();
    ensure_indexes(&db).await.unwrap();
    let desired = definition(&db, collection, name).await;
    // A nonempty collection exercises the real index builder, not MongoDB's
    // empty-collection fast path. All data is synthetic.
    let mut row = match collection {
        "nodes" => doc! {"user_id":"fixture-owner", "name":"fixture-node", "is_active":true},
        "node_service_bindings" => {
            doc! {"node_id":"fixture-node", "service_id":"fixture-service", "is_active":true}
        }
        _ => {
            doc! {"checkpoint":{"cursor":0}, "expires_at":bson::DateTime::from_millis(4_102_444_800_000)}
        }
    };
    row.insert("_id", uuid::Uuid::new_v4().to_string());
    let coll = db.collection::<Document>(collection);
    coll.insert_one(row).await.unwrap();
    reset_legacy(&db, collection, name, &desired).await;
    (db, desired)
}

async fn reset_legacy(db: &Database, collection: &str, name: &str, desired: &IndexModel) {
    let coll = db.collection::<Document>(collection);
    if let Err(error) = coll.drop_index(name).await {
        assert!(
            matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(e) if e.code == 27)
        );
    }
    let mut legacy = desired.clone();
    legacy.options.as_mut().unwrap().partial_filter_expression = None;
    coll.create_index(legacy).await.unwrap();
}

async fn clear_failpoints(admin: &Database) {
    for name in [BUILD_PAUSE, "failCommand"] {
        admin
            .run_command(doc! {"configureFailPoint":name, "mode":"off"})
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_converters_recover_from_index_build_races() {
    let mut failures = Vec::new();
    for &(collection, name) in TARGETS {
        for inject_refusal in [false, true] {
            let (base, desired) = legacy_fixture(collection, name).await;
            let admin = base.client().database("admin");
            let (gate, release) = Gate::new("dropIndexes");
            let mut release = Some(release);
            let (collision_tx, mut collision_rx) = tokio::sync::mpsc::unbounded_channel();
            let a_trace = Arc::new(Trace::default());
            let b_trace = Arc::new(Trace::default());
            let (a, _) = monitored_client(
                &base,
                collection,
                name,
                a_trace.clone(),
                None,
                Some(collision_tx.clone()),
            )
            .await;
            let (b, b_app) = monitored_client(
                &base,
                collection,
                name,
                b_trace.clone(),
                Some(gate.clone()),
                Some(collision_tx),
            )
            .await;
            let mut b_task = tokio::spawn(async move { ensure_indexes(&b).await });
            gate.reached.notified().await;
            // B has read the legacy definition and is paused before sending its
            // stale drop. A now replaces it and pauses inside MongoDB's builder.
            let configured = admin
                .run_command(doc! {"configureFailPoint":BUILD_PAUSE, "mode":"alwaysOn"})
                .await
                .unwrap();
            let count = configured
                .get_i32("count")
                .map(i64::from)
                .or_else(|_| configured.get_i64("count"))
                .unwrap();
            let mut a_task = tokio::spawn(async move { ensure_indexes(&a).await });
            let mut early_a = None;
            let mut early_b = None;
            let observed = AssertUnwindSafe(async {
                admin.run_command(doc! {"waitForFailPoint":BUILD_PAUSE, "timesEntered":count+1, "maxTimeMS":120_000}).await.unwrap();
                if inject_refusal {
                    admin.run_command(doc! {
                        "configureFailPoint":"failCommand", "mode":{"times":1},
                        "data":{"failCommands":["dropIndexes"], "appName":&b_app, "errorCode":12587},
                    }).await.unwrap();
                }
                release.take().unwrap().send(()).unwrap();
                tokio::select! {
                    biased;
                    code = collision_rx.recv() => code,
                    result = &mut a_task => { early_a = Some(result); None },
                    result = &mut b_task => { early_b = Some(result); None },
                }
            }).catch_unwind().await;
            if let Some(release) = release.take() {
                let _ = release.send(());
            }
            clear_failpoints(&admin).await;
            let a_result = match early_a {
                Some(result) => result,
                None => a_task.await,
            }
            .unwrap();
            let b_result = match early_b {
                Some(result) => result,
                None => b_task.await,
            }
            .unwrap();
            let code = observed
                .unwrap()
                .unwrap_or_else(|| panic!("missing collision: A={a_result:?}, B={b_result:?}"));
            assert_eq!(code, if inject_refusal { 12587 } else { 276 });
            eprintln!(
                "{collection}: injected_refusal={inject_refusal}, observed_code={code}, A_ok={}, B_ok={}",
                a_result.is_ok(),
                b_result.is_ok()
            );
            if a_result.is_err() || b_result.is_err() {
                failures.push(format!(
                    "{collection}: refusal={inject_refusal}, A={a_result:?}, B={b_result:?}"
                ));
            }
            let actual = definition(&base, collection, name).await;
            assert_eq!(
                bson::to_document(&actual).unwrap(),
                bson::to_document(&desired).unwrap()
            );
            if a_result.is_ok() && b_result.is_ok() {
                let retried = if inject_refusal { &b_trace } else { &a_trace };
                assert_eq!(
                    retried.lists.load(Ordering::SeqCst),
                    2,
                    "retry re-lists the definition"
                );
            }
            base.drop().await.unwrap();
        }
    }
    assert!(
        failures.is_empty(),
        "concurrent startup failures: {failures:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_retry_budget_and_non_retryable_errors() {
    // The binding indexes share the ordinary helper; coordination additionally
    // exercises the backfill between drop and create on every attempt.
    for &(collection, name) in &[TARGETS[0], TARGETS[2]] {
        let (base, desired) = legacy_fixture(collection, name).await;
        let admin = base.client().database("admin");
        for command in ["dropIndexes", "createIndexes"] {
            for (code, transient) in [
                (276, false),
                (12587, false),
                (276, true),
                (12587, true),
                (11000, false),
                (13, false),
            ] {
                reset_legacy(&base, collection, name, &desired).await;
                let trace = Arc::new(Trace::default());
                let (gate, release) = Gate::new(command);
                let (db, app) = monitored_client(
                    &base,
                    collection,
                    name,
                    trace.clone(),
                    Some(gate.clone()),
                    None,
                )
                .await;
                let model = desired.clone();
                let operation = tokio::spawn(async move {
                    if collection == "coordination_leases" {
                        crate::services::coordination_service::ensure_indexes(&db).await
                    } else {
                        super::super::ensure_named_index(
                            &db.collection::<Document>(collection),
                            name,
                            model,
                        )
                        .await
                    }
                });
                // Install only after the target command is reached. This keeps
                // coordination's unrelated obsolete-name retirement out of the
                // failure injection and the measured attempt count.
                gate.reached.notified().await;
                let mode = if transient {
                    bson::Bson::Document(doc! {"times":2})
                } else {
                    bson::Bson::String("alwaysOn".into())
                };
                let mut data = doc! {"failCommands":[command], "appName":app, "errorCode":code};
                if code == 11000 {
                    data.insert(
                        "errorExtraInfo",
                        doc! {
                            "keyPattern":{"fixture_key":1}, "keyValue":{"fixture_key":"synthetic"},
                        },
                    );
                }
                let configured = admin
                    .run_command(doc! {
                        "configureFailPoint":"failCommand", "mode":mode, "data":data,
                    })
                    .await;
                release.send(()).unwrap();
                let result = operation.await;
                clear_failpoints(&admin).await;
                configured.unwrap();
                let result = result.unwrap();
                let expected_attempts = if matches!(code, 276 | 12587) { 3 } else { 1 };
                let attempts = if command == "dropIndexes" {
                    &trace.drops
                } else {
                    &trace.creates
                };
                assert_eq!(
                    attempts.load(Ordering::SeqCst),
                    expected_attempts,
                    "{collection} {command} {code} transient={transient}"
                );
                assert_eq!(
                    trace.lists.load(Ordering::SeqCst),
                    expected_attempts,
                    "every attempt re-lists"
                );
                if transient {
                    result.expect("two collisions recover on the third attempt");
                    let actual = definition(&base, collection, name).await;
                    assert_eq!(
                        bson::to_document(&actual).unwrap(),
                        bson::to_document(&desired).unwrap()
                    );
                } else {
                    let error = result.expect_err("persistent injected error is returned");
                    assert!(
                        matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(e) if e.code == code),
                        "{error:?}"
                    );
                }
                assert_eq!(
                    trace.errors.lock().unwrap().len(),
                    if transient { 2 } else { expected_attempts }
                );
            }
        }
        base.drop().await.unwrap();
    }
}
