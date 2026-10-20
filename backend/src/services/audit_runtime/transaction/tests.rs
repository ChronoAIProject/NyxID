use super::*;
use crate::{
    models::{
        audit_log::{AuditLog, COLLECTION_NAME as AUDIT},
        audit_outbox::COLLECTION_NAME as OUTBOX,
    },
    services::{audit_chain_service as chain, audit_state_service as state},
    test_utils::connect_transaction_test_database,
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use std::sync::Arc;
use tokio::sync::Barrier;
use uuid::Uuid;

const KEY: [u8; 32] = [2; 32];

fn entry() -> AuditLog {
    AuditLog {
        id: Uuid::new_v4().to_string(),
        user_id: None,
        event_type: "audit_contention_test".into(),
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

#[derive(Default)]
struct Workload {
    appends: usize,
    body_attempts: u32,
    max_body_attempts: u32,
    max_commit_attempts: u32,
    integrity_errors: usize,
}

async fn current_workload(db: &Database, tasks: usize, per_task: usize) -> Workload {
    observed_workload(db, tasks, per_task, None).await
}

async fn observed_workload(
    db: &Database,
    tasks: usize,
    per_task: usize,
    retry_seen: Option<Arc<tokio::sync::Notify>>,
) -> Workload {
    let barrier = Arc::new(Barrier::new(tasks));
    let mut workers = Vec::new();
    for worker in 0..tasks {
        let (db, barrier, retry_seen) = (db.clone(), barrier.clone(), retry_seen.clone());
        workers.push(tokio::spawn(async move {
            let mut stats = Workload::default();
            barrier.wait().await;
            for offset in 0..per_task {
                // Identity and timestamp exist before any retry callback.
                let event = entry();
                let (mut bodies, mut commits) = (0, 0);
                let result = if (worker + offset) % 2 == 0 {
                    chain::append_chained_entry_observed(&db, event, &KEY, |b, c| {
                        (bodies, commits) = (b, c);
                        if b > 1 && let Some(retry_seen) = &retry_seen {
                            retry_seen.notify_one();
                        }
                    })
                    .await
                    .map(|_| ())
                    .map_err(|error| {
                        if matches!(
                            error.get_custom::<AppError>(),
                            Some(AppError::AuditIntegrityConflict)
                        ) {
                            stats.integrity_errors += 1;
                        }
                        AppError::DatabaseError(error)
                    })
                } else {
                    run_observed(
                        &db,
                        &(db.clone(), event),
                        |session, (db, event)| {
                            Box::pin(async move {
                                db.collection::<bson::Document>("business")
                                    .insert_one(doc! {"_id": &event.id})
                                    .session(&mut *session)
                                    .await?;
                                chain::append_chained_entry_in_session(
                                    db,
                                    session,
                                    event.clone(),
                                    &KEY,
                                )
                                .await
                            })
                        },
                        |b, c| {
                            (bodies, commits) = (b, c);
                            if b > 1 && let Some(retry_seen) = &retry_seen {
                                retry_seen.notify_one();
                            }
                        },
                    )
                    .await
                };
                if matches!(result, Err(AppError::AuditIntegrityConflict)) {
                    stats.integrity_errors += 1;
                }
                assert!(
                    result.is_ok(),
                    "ordinary contention must retry transparently: bodies={bodies} commits={commits} result={result:?}"
                );
                stats.appends += 1;
                stats.body_attempts += bodies;
                stats.max_body_attempts = stats.max_body_attempts.max(bodies);
                stats.max_commit_attempts = stats.max_commit_attempts.max(commits);
                assert!(bodies <= MAX_BODY_ATTEMPTS);
                assert!(commits <= MAX_BODY_ATTEMPTS * MAX_COMMIT_ATTEMPTS);
            }
            stats
        }));
    }
    let mut total = Workload::default();
    for worker in workers {
        let stats = worker.await.unwrap();
        total.appends += stats.appends;
        total.body_attempts += stats.body_attempts;
        total.max_body_attempts = total.max_body_attempts.max(stats.max_body_attempts);
        total.max_commit_attempts = total.max_commit_attempts.max(stats.max_commit_attempts);
        total.integrity_errors += stats.integrity_errors;
    }
    total
}

async fn assert_chain(db: &Database, expected: usize, durable: bool, business: usize) {
    let rows: Vec<AuditLog> = db
        .collection::<AuditLog>(AUDIT)
        .find(doc! {})
        .sort(doc! {"seq": 1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), expected);
    let mut previous = chain::GENESIS_PREV_HASH.to_owned();
    for (offset, event) in rows.iter().enumerate() {
        assert_eq!(event.seq, Some(offset as i64 + 1));
        assert_eq!(event.prev_hash.as_ref(), Some(&previous));
        previous = chain::compute_entry_hash(event, &KEY).unwrap();
        assert_eq!(event.entry_hash.as_ref(), Some(&previous));
    }
    assert_eq!(
        db.collection::<bson::Document>("business")
            .count_documents(doc! {})
            .await
            .unwrap(),
        business as u64
    );
    if durable {
        let head = state::load(db).await.unwrap();
        assert_eq!(head.head_seq, expected as i64);
        assert_eq!(head.head_hash, previous);
        assert_eq!(head.next_order, expected as i64);
        assert_eq!(head.pending_count, expected as i64);
        assert_eq!(
            db.collection::<bson::Document>(OUTBOX)
                .count_documents(doc! {})
                .await
                .unwrap(),
            expected as u64
        );
        let report = chain::verify_chain(db, &KEY, None, None, None)
            .await
            .unwrap();
        assert_eq!(report.status, chain::AuditChainStatus::Ok);
        assert_eq!(report.checked_count, expected as u64);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_head_contention_256_mixed_appends_from_16_tasks_is_transparent_and_bounded() {
    let db = connect_transaction_test_database("audit_head_load").await;
    // Create the business collection before racing transactions to avoid DDL.
    db.create_collection("business").await.unwrap();
    // A raw independent transaction bypasses the process queue, just like a
    // second application process. Keep the real-conflict assertion meaningful
    // after local writers stop needlessly invalidating one another's snapshots.
    let mut external = db.client().start_session().await.unwrap();
    external.start_transaction().await.unwrap();
    db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
        .update_one(doc! {}, doc! {"$inc": {"test_contention_fence": 1}})
        .session(&mut external)
        .await
        .unwrap();
    let retry_seen = Arc::new(tokio::sync::Notify::new());
    let workload = observed_workload(&db, 16, 16, Some(retry_seen.clone()));
    let release = async {
        retry_seen.notified().await;
        external.abort_transaction().await.unwrap();
    };
    let (stats, ()) = tokio::join!(workload, release);
    assert_eq!(stats.appends, 256);
    assert_eq!(stats.integrity_errors, 0);
    assert!(
        stats.body_attempts > 256,
        "must exercise actual transaction contention"
    );
    assert_chain(&db, 256, true, 128).await;
    eprintln!(
        "audit_head_contention appends={} tasks=16 body_attempts={} max_body_attempts={} max_commit_attempts={} integrity_errors={}",
        stats.appends,
        stats.body_attempts,
        stats.max_body_attempts,
        stats.max_commit_attempts,
        stats.integrity_errors
    );
}

#[tokio::test]
async fn audit_transaction_nested_core_and_history_owners_fail_fast_and_abort_outer_body() {
    let db = connect_transaction_test_database("audit_nested_owner").await;
    db.create_collection("business").await.unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        run(&db, &db, |session, db| {
            Box::pin(async move {
                db.collection::<bson::Document>("business")
                    .insert_one(doc! {"_id": "outer"})
                    .session(session)
                    .await?;
                crate::services::service_history::transaction::run(db, async |_| Ok(())).await?;
                Ok(())
            })
        }),
    )
    .await
    .expect("nested owner must fail instead of deadlocking");
    assert!(matches!(result, Err(AppError::DatabaseError(ref error))
        if error.get_custom::<super::turns::NestedTransactionOwner>().is_some()));
    assert_eq!(
        db.collection::<bson::Document>("business")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        crate::services::service_history::transaction::run(&db, async |_| {
            crate::services::api_key_mutation_service::transaction_result(
                run(&db, &(), |_, ()| Box::pin(async { Ok(()) })).await,
            )
        }),
    )
    .await
    .expect("history-to-core nesting must also fail fast");
    assert!(
        result
            .unwrap_err()
            .get_custom::<super::turns::NestedTransactionOwner>()
            .is_some()
    );
    run(&db, &(), |_, ()| Box::pin(async { Ok(()) }))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_head_contention_two_processes_keep_the_same_retry_budgets_and_chain() {
    const CHILD_DB: &str = "NYXID_AUDIT_CONTENTION_CHILD_DB";
    const RETRY_MARKER: &str = "NYXID_AUDIT_CONTENTION_RETRY_MARKER";
    if let Ok(name) = std::env::var(CHILD_DB) {
        let client =
            mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        let retry_seen = Arc::new(tokio::sync::Notify::new());
        let marker = std::env::var(RETRY_MARKER).unwrap();
        let signal = async {
            retry_seen.notified().await;
            std::fs::write(marker, b"retried").unwrap();
        };
        let db = client.database(&name);
        let (stats, ()) = tokio::join!(
            observed_workload(&db, 16, 16, Some(retry_seen.clone())),
            signal,
        );
        assert!(
            stats.max_body_attempts > 1,
            "the independent process must retry the parent's real head lock"
        );
        assert_eq!(stats.appends, 256);
        assert_eq!(stats.integrity_errors, 0);
        eprintln!(
            "audit_two_process child bodies={} max_body_attempts={} max_commit_attempts={}",
            stats.body_attempts, stats.max_body_attempts, stats.max_commit_attempts
        );
        return;
    }
    let db = connect_transaction_test_database("audit_process_contention").await;
    db.create_collection("business").await.unwrap();
    let name = db.name().to_owned();
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("child-retried");
    let child_marker = marker.clone();
    let mut external = db.client().start_session().await.unwrap();
    external.start_transaction().await.unwrap();
    db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
        .update_one(doc! {}, doc! {"$inc": {"test_process_contention_fence": 1}})
        .session(&mut external)
        .await
        .unwrap();
    let child = tokio::task::spawn_blocking(move || {
        std::process::Command::new(std::env::current_exe().unwrap()).args([
            "--exact", "services::audit_runtime::transaction::tests::audit_head_contention_two_processes_keep_the_same_retry_budgets_and_chain",
            "--nocapture",
        ]).env(CHILD_DB, name).env(RETRY_MARKER, child_marker).output().unwrap()
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !marker.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("child must observe real cross-process contention");
    external.abort_transaction().await.unwrap();
    let stats = current_workload(&db, 16, 16).await;
    let child = child.await.unwrap();
    assert!(
        child.status.success(),
        "child stdout={} stderr={}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&child.stderr));
    assert_eq!(stats.appends, 256);
    assert_eq!(stats.integrity_errors, 0);
    assert_chain(&db, 512, true, 256).await;
    eprintln!(
        "audit_two_process parent bodies={} max_body_attempts={} max_commit_attempts={}",
        stats.body_attempts, stats.max_body_attempts, stats.max_commit_attempts
    );
}

#[tokio::test]
async fn audit_transaction_integrity_failure_aborts_once_without_retry() {
    let db = connect_transaction_test_database("audit_integrity_no_retry").await;
    let mut attempts = 0;
    let result: AppResult<()> = run_observed(
        &db,
        &db,
        |session, db| {
            Box::pin(async move {
                db.collection::<bson::Document>("business")
                    .insert_one(doc! {"_id": "abort"})
                    .session(session)
                    .await?;
                Err(AppError::AuditIntegrityConflict)
            })
        },
        |b, _| attempts = b,
    )
    .await;
    assert!(matches!(result, Err(AppError::AuditIntegrityConflict)));
    assert_eq!(attempts, 1);
    assert_eq!(
        db.collection::<bson::Document>("business")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn audit_transaction_write_conflict_and_unknown_commit_are_bounded_without_body_replay() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::Mutex;
    let base = connect_transaction_test_database("audit_retry_faults").await;
    base.create_collection("business").await.unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let capture = commands.clone();
    let mut fault = crate::test_utils::fail_command::Guard::acquire(
        &base,
        Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command_name == "commitTransaction"
            {
                capture.lock().unwrap().push(event.command);
            }
        })),
    )
    .await;
    let db = fault.database().clone();
    for labeled in [true, false] {
        let labels = if labeled {
            vec!["TransientTransactionError"]
        } else {
            vec![]
        };
        fault
            .arm(
                doc! {"times": 2},
                doc! {"failCommands": ["update"], "errorCode": 112, "errorLabels": labels},
            )
            .await
            .unwrap();
        let event = entry();
        let mut attempts = 0;
        run_observed(
            &db,
            &(db.clone(), event),
            |session, (db, event)| {
                Box::pin(async move {
                    db.collection::<bson::Document>("business")
                        .insert_one(doc! {"_id": &event.id})
                        .session(&mut *session)
                        .await?;
                    chain::append_chained_entry_in_session(db, session, event.clone(), &KEY).await
                })
            },
            |b, _| attempts = b,
        )
        .await
        .unwrap();
        assert_eq!(
            attempts, 3,
            "two real conflicts must replay the entire aborted business transaction"
        );
    }
    fault.arm("alwaysOn", doc! {"failCommands": ["update"], "errorCode": 112, "errorLabels": ["TransientTransactionError"]})
        .await
        .unwrap();
    let mut attempts = 0;
    let result = run_observed(
        &db,
        &(db.clone(), entry()),
        |session, (db, event)| {
            Box::pin(chain::append_chained_entry_in_session(
                db,
                session,
                event.clone(),
                &KEY,
            ))
        },
        |b, _| attempts = b,
    )
    .await;
    fault.disarm().await.unwrap();
    assert!(matches!(result, Err(AppError::DatabaseError(_))));
    assert_eq!(attempts, MAX_BODY_ATTEMPTS);
    assert_eq!(state::load(&base).await.unwrap().head_seq, 2);
    // Successful commit with uncertain acknowledgment: callback executes once.
    fault.arm(doc! {"times": 1}, doc! {"failCommands": ["commitTransaction"], "writeConcernError": {"code": 64, "errmsg": "injected uncertain commit"}, "errorLabels": ["UnknownTransactionCommitResult"]})
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let event = entry();
    let mut attempts = 0;
    run_observed(
        &db,
        &(db.clone(), event),
        |session, (db, event)| {
            Box::pin(chain::append_chained_entry_in_session(
                db,
                session,
                event.clone(),
                &KEY,
            ))
        },
        |b, _| attempts = b,
    )
    .await
    .unwrap();
    assert_eq!(attempts, 1);
    let commits = commands.lock().unwrap().clone();
    assert!(commits.len() >= 2);
    for command in &commits[1..] {
        assert_eq!(command.get("lsid"), commits[0].get("lsid"));
        assert_eq!(command.get("txnNumber"), commits[0].get("txnNumber"));
    }
    // All eight commit attempts are unknown: return raw uncertainty, never
    // replay a possibly committed mutation or turn it into integrity failure.
    fault.arm("alwaysOn", doc! {"failCommands": ["commitTransaction"], "writeConcernError": {"code": 64, "errmsg": "injected uncertain commit"}, "errorLabels": ["UnknownTransactionCommitResult"]})
        .await
        .unwrap();
    let mut counts = (0, 0);
    let result = run_observed(
        &db,
        &(db.clone(), entry()),
        |session, (db, event)| {
            Box::pin(chain::append_chained_entry_in_session(
                db,
                session,
                event.clone(),
                &KEY,
            ))
        },
        |b, c| counts = (b, c),
    )
    .await;
    fault.disarm().await.unwrap();
    assert!(
        matches!(result, Err(AppError::DatabaseError(ref error)) if error.contains_label("UnknownTransactionCommitResult"))
    );
    assert_eq!(counts, (1, MAX_COMMIT_ATTEMPTS));
    assert_eq!(state::load(&base).await.unwrap().head_seq, 4);
    assert_eq!(
        base.collection::<bson::Document>("business")
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
}

mod benchmark;
mod core_contract;
