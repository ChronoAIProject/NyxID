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
    let barrier = Arc::new(Barrier::new(tasks));
    let mut workers = Vec::new();
    for worker in 0..tasks {
        let (db, barrier) = (db.clone(), barrier.clone());
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
    let stats = current_workload(&db, 16, 16).await;
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
