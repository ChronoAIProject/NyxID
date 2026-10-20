use super::*;
use crate::test_utils::connect_transaction_test_database;
use futures::TryStreamExt;

const KEY: [u8; 32] = [2; 32];
fn entry() -> AuditLog {
    AuditLog {
        id: Uuid::new_v4().to_string(),
        user_id: None,
        event_type: "audit_runtime_test".into(),
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

#[tokio::test]
async fn unwitnessed_append_commits_head_row_and_delivery_reservation() {
    let db = connect_transaction_test_database("audit_state_local").await;
    let inserted = chain::append_chained_entry(&db, entry(), &KEY)
        .await
        .unwrap();
    let state = load(&db).await.unwrap();
    assert!(!state.witness_enrolled);
    assert_eq!(state.head_seq, 1);
    assert_eq!(state.pending_count, 1);
    assert_eq!(state.witnessed_seq, 0);
    assert_eq!(state.head_hash, inserted.entry_hash.unwrap());
    let pending = db
        .collection::<AuditOutbox>(OUTBOX)
        .find_one(doc! {"_id": inserted.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.seq, 1);
    assert!(pending.witnessed_at.is_none());
    let document = bson::to_document(&pending).unwrap();
    assert!(!document.contains_key("event_data"));
    assert!(!document.contains_key("user_agent"));
    let report = chain::verify_chain(&db, &KEY, None, None, None)
        .await
        .unwrap();
    let response = serde_json::to_value(report).unwrap();
    assert_eq!(response["status"], "unwitnessed");
    assert_eq!(response["local_status"], "ok");
}

#[tokio::test]
async fn business_abort_rolls_back_audit_head_outbox_and_mutation() {
    let db = connect_transaction_test_database("audit_state_abort").await;
    let before = load(&db).await.unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    db.collection::<bson::Document>("business")
        .insert_one(doc! {"_id": "mutation"})
        .session(&mut session)
        .await
        .unwrap();
    append_in_session(&db, &mut session, entry(), &KEY)
        .await
        .unwrap();
    session.abort_transaction().await.unwrap();
    let after = load(&db).await.unwrap();
    assert_eq!(before.head_hash, after.head_hash);
    assert_eq!(after.head_seq, 0);
    assert_eq!(after.pending_count, 0);
    for collection in [AUDIT_LOG, OUTBOX, "business"] {
        assert_eq!(
            db.collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn business_commit_and_identical_retry_reserve_once() {
    let db = connect_transaction_test_database("audit_state_replay").await;
    let event = entry();
    let mut session = db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .write_concern(majority())
        .await
        .unwrap();
    db.collection::<bson::Document>("business")
        .insert_one(doc! {"_id": "mutation"})
        .session(&mut session)
        .await
        .unwrap();
    let first = append_in_session(&db, &mut session, event.clone(), &KEY)
        .await
        .unwrap();
    commit(&mut session).await.unwrap();
    let retry = chain::append_chained_entry(&db, event.clone(), &KEY)
        .await
        .unwrap();
    assert_eq!(first.entry_hash, retry.entry_hash);
    assert_eq!(load(&db).await.unwrap().pending_count, 1);
    let mut conflict = event;
    conflict.event_type = "different_operation".into();
    assert!(
        chain::append_chained_entry(&db, conflict, &KEY)
            .await
            .is_err()
    );
    assert_eq!(load(&db).await.unwrap().head_seq, 1);
    assert_eq!(
        db.collection::<AuditOutbox>(OUTBOX)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<bson::Document>("business")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn concurrent_appends_share_one_head_and_pending_counter() {
    let db = connect_transaction_test_database("audit_state_concurrent").await;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            chain::append_chained_entry(&db, entry(), &KEY).await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let state = load(&db).await.unwrap();
    assert_eq!(state.head_seq, 8);
    assert_eq!(state.pending_count, 8);
    assert_eq!(state.next_order, 8);
    let events: Vec<AuditLog> = db
        .collection::<AuditLog>(AUDIT_LOG)
        .find(doc! {})
        .sort(doc! {"seq": 1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    for (i, event) in events.iter().enumerate() {
        assert_eq!(event.seq, Some(i as i64 + 1));
        assert_eq!(
            event.prev_hash.as_deref(),
            Some(if i == 0 {
                chain::GENESIS_PREV_HASH
            } else {
                events[i - 1].entry_hash.as_deref().unwrap()
            })
        );
    }
}

#[tokio::test]
async fn v1_adoption_requires_drain_once_and_preserves_original_bytes() {
    let db = connect_transaction_test_database("audit_state_cutover").await;
    db.collection::<AuditState>(STATE)
        .delete_many(doc! {})
        .await
        .unwrap();
    let mut old = entry();
    chain::prepare_chained_entry(&mut old, 1, chain::GENESIS_PREV_HASH.into(), &KEY).unwrap();
    db.collection::<AuditLog>(AUDIT_LOG)
        .insert_one(&old)
        .await
        .unwrap();
    assert!(matches!(
        initialize(&db, &KEY, &AuditConfig::default()).await,
        Err(AppError::AuditCutoverRequired)
    ));
    assert_eq!(
        db.collection::<AuditState>(STATE)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let config = AuditConfig {
        cutover_drained: true,
        ..Default::default()
    };
    // Acknowledgment alone cannot adopt while the executing roster is absent.
    assert!(matches!(
        initialize(&db, &KEY, &config).await,
        Err(AppError::AuditCutoverRequired)
    ));
    super::super::security_readiness_service::ready_test_replica(&db).await;
    let adopted = initialize(&db, &KEY, &config).await.unwrap();
    assert_eq!(adopted.adopted_v1_seq, 1);
    assert_eq!(adopted.head_hash, old.entry_hash.clone().unwrap());
    db.collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    // Persisted adoption needs neither observations nor a repeated drain flag.
    initialize(&db, &KEY, &AuditConfig::default())
        .await
        .unwrap();
    let unchanged = db
        .collection::<AuditLog>(AUDIT_LOG)
        .find_one(doc! {"_id": &old.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bson::to_document(&old).unwrap(),
        bson::to_document(&unchanged).unwrap()
    );
    let next = chain::append_chained_entry(&db, entry(), &KEY)
        .await
        .unwrap();
    assert_eq!(next.seq, Some(2));
    assert_eq!(next.prev_hash, old.entry_hash);
}

#[tokio::test]
async fn missing_head_never_falls_back_to_surviving_tail() {
    let db = connect_transaction_test_database("audit_state_missing").await;
    chain::append_chained_entry(&db, entry(), &KEY)
        .await
        .unwrap();
    db.collection::<AuditState>(STATE)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(
        chain::append_chained_entry(&db, entry(), &KEY)
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<AuditLog>(AUDIT_LOG)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn replay_refuses_missing_delivery_or_a_rolled_back_head() {
    let db = connect_transaction_test_database("audit_replay_corruption").await;
    let event = entry();
    chain::append_chained_entry(&db, event.clone(), &KEY)
        .await
        .unwrap();
    let head = load(&db).await.unwrap();
    db.collection::<AuditState>(STATE)
        .update_one(
            doc! {"_id": &head.id},
            doc! {"$set": {"head_seq": 0, "head_hash": chain::GENESIS_PREV_HASH}},
        )
        .await
        .unwrap();
    assert!(
        chain::append_chained_entry(&db, event.clone(), &KEY)
            .await
            .is_err()
    );
    db.collection::<AuditState>(STATE)
        .replace_one(doc! {"_id": &head.id}, head)
        .await
        .unwrap();
    db.collection::<AuditOutbox>(OUTBOX)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(chain::append_chained_entry(&db, event, &KEY).await.is_err());
}

#[tokio::test]
async fn removed_live_tail_does_not_reset_durable_sequence() {
    let db = connect_transaction_test_database("audit_state_tail").await;
    chain::append_chained_entry(&db, entry(), &KEY)
        .await
        .unwrap();
    db.collection::<AuditLog>(AUDIT_LOG)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(matches!(
        initialize(&db, &KEY, &AuditConfig::default()).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    assert_eq!(load(&db).await.unwrap().head_seq, 1);
    // Until a protected W8-5 retention checkpoint authorizes a prune, deletion
    // is corruption. The allocator must never infer sequence zero from it.
    let report = chain::verify_chain(&db, &KEY, None, None, None)
        .await
        .unwrap();
    assert_eq!(report.status, chain::AuditChainStatus::Broken);
}

#[tokio::test]
async fn restart_rejects_wrong_chain_key_or_changed_tail() {
    let db = connect_transaction_test_database("audit_state_identity").await;
    chain::append_chained_entry(&db, entry(), &KEY)
        .await
        .unwrap();
    assert!(
        chain::append_chained_entry(&db, entry(), &[3; 32])
            .await
            .is_err()
    );
    let other_chain = AuditConfig {
        chain_id: Some(Uuid::new_v4()),
        ..Default::default()
    };
    assert!(matches!(
        initialize(&db, &KEY, &other_chain).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    assert!(matches!(
        initialize(&db, &[3; 32], &Default::default()).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    db.collection::<bson::Document>(AUDIT_LOG)
        .update_one(doc! {"seq": 1}, doc! {"$set": {"event_type": "changed"}})
        .await
        .unwrap();
    assert!(matches!(
        initialize(&db, &KEY, &Default::default()).await,
        Err(AppError::AuditIntegrityConflict)
    ));
}

#[tokio::test]
async fn unknown_commit_acknowledgment_retries_same_transaction_and_event_identity() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};

    let base = connect_transaction_test_database("audit_commit_unknown").await;
    let commands = Arc::new(Mutex::new(Vec::new()));
    let captured = commands.clone();
    let mut fault = crate::test_utils::fail_command::Guard::acquire(
        &base,
        Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command_name == "commitTransaction"
            {
                captured.lock().unwrap().push(event.command);
            }
        })),
    )
    .await;
    let db = fault.database().clone();
    // Mongo executes the commit, then injects an uncertain durability response.
    // Scope to this client's application name; other test clients are unaffected.
    fault.arm(doc! {"times": 1}, doc! {"failCommands": ["commitTransaction"], "writeConcernError": {"code": 64, "errmsg": "injected uncertain commit"}, "errorLabels": ["UnknownTransactionCommitResult"]})
        .await
        .expect("own Mongo must enable test commands; no skipped fault evidence");
    let event = entry();
    let inserted = chain::append_chained_entry(&db, event.clone(), &KEY)
        .await
        .unwrap();
    let commits = commands.lock().unwrap().clone();
    assert!(
        commits.len() >= 2,
        "must exercise actual commit retransmission"
    );
    for command in &commits[1..] {
        assert_eq!(command.get("lsid"), commits[0].get("lsid"));
        assert_eq!(command.get("txnNumber"), commits[0].get("txnNumber"));
    }
    let replay = chain::append_chained_entry(&db, event, &KEY).await.unwrap();
    assert_eq!(inserted.entry_hash, replay.entry_hash);
    let head = load(&db).await.unwrap();
    assert_eq!(head.head_seq, 1);
    assert_eq!(head.pending_count, 1);
    for name in [AUDIT_LOG, OUTBOX] {
        assert_eq!(
            db.collection::<bson::Document>(name)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }
}

/// A read-only verification must release its snapshot before returning. Relying
/// on ClientSession::drop races short-runtime teardown and can pin DDL until
/// the server's transaction reaper, including after broken/error reports.
#[tokio::test]
async fn audit_verification_closes_snapshot_on_success_break_and_database_error() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let aborts = Arc::new(AtomicUsize::new(0));
    let observed = aborts.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "audit_snapshot_close",
        EventHandler::callback(move |event| {
            if let CommandEvent::Succeeded(event) = event
                && event.command_name == "abortTransaction"
            {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }),
    )
    .await;
    let event = chain::append_chained_entry(&db, entry(), &KEY)
        .await
        .unwrap();
    aborts.store(0, Ordering::SeqCst);
    let report = chain::verify_chain(&db, &KEY, None, None, None)
        .await
        .unwrap();
    assert_eq!(report.status, chain::AuditChainStatus::Ok);
    assert_eq!(
        aborts.load(Ordering::SeqCst),
        1,
        "success must await its own abort"
    );

    db.collection::<bson::Document>(AUDIT_LOG)
        .update_one(
            doc! {"_id": &event.id},
            doc! {"$set": {"event_type": "changed"}},
        )
        .await
        .unwrap();
    aborts.store(0, Ordering::SeqCst);
    let report = chain::verify_chain(&db, &KEY, None, None, None)
        .await
        .unwrap();
    assert_eq!(report.status, chain::AuditChainStatus::Broken);
    assert_eq!(
        aborts.load(Ordering::SeqCst),
        1,
        "broken report must await its own abort"
    );

    db.collection::<bson::Document>(AUDIT_LOG)
        .update_one(doc! {"_id": &event.id}, doc! {"$set": {"event_type": 17}})
        .await
        .unwrap();
    aborts.store(0, Ordering::SeqCst);
    assert!(matches!(
        chain::verify_chain(&db, &KEY, None, None, None).await,
        Err(AppError::DatabaseError(_))
    ));
    assert_eq!(
        aborts.load(Ordering::SeqCst),
        1,
        "database error must await its own abort"
    );
}

#[tokio::test]
async fn audit_initialization_closes_rejected_snapshot_and_preserves_head() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let aborts = Arc::new(AtomicUsize::new(0));
    let observed = aborts.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "audit_rejected_snapshot",
        EventHandler::callback(move |event| {
            if let CommandEvent::Succeeded(event) = event
                && event.command_name == "abortTransaction"
            {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }),
    )
    .await;
    chain::append_chained_entry(&db, entry(), &KEY)
        .await
        .unwrap();
    let before = load(&db).await.unwrap();
    aborts.store(0, Ordering::SeqCst);
    assert!(matches!(
        initialize(&db, &[3; 32], &AuditConfig::default()).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    assert_eq!(aborts.load(Ordering::SeqCst), 1);
    let after = load(&db).await.unwrap();
    assert_eq!(after.head_seq, before.head_seq);
    assert_eq!(after.head_hash, before.head_hash);
    assert_eq!(after.pending_count, before.pending_count);
}

#[tokio::test]
async fn audit_batched_indexes_preserve_constraints_and_repair_removed_index() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let builds = Arc::new(AtomicUsize::new(0));
    let observed = builds.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "audit_batched_indexes",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command_name == "createIndexes"
            {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }),
    )
    .await;
    builds.store(0, Ordering::SeqCst);
    ensure_indexes(&db).await.unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 5);
    for (collection, name, keys, partial) in [
        (
            AUDIT_LOG,
            "audit_log_seq_unique",
            doc! {"seq": 1},
            Some(doc! {"seq": {"$exists": true}}),
        ),
        (STATE, "audit_single_head", doc! {"singleton": 1}, None),
        (
            OUTBOX,
            "audit_outbox_order",
            doc! {"chain_id": 1, "order": 1},
            None,
        ),
        (
            OUTBOX,
            "audit_outbox_seq",
            doc! {"chain_id": 1, "seq": 1},
            None,
        ),
        (
            PENDING,
            "audit_operation_stage",
            doc! {"event.operation_id": 1, "event.stage": 1},
            None,
        ),
    ] {
        let indexes: Vec<_> = db
            .collection::<bson::Document>(collection)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let index = indexes
            .iter()
            .find(|index| {
                index
                    .options
                    .as_ref()
                    .and_then(|options| options.name.as_deref())
                    == Some(name)
            })
            .unwrap();
        assert_eq!(index.keys, keys);
        let options = index.options.as_ref().unwrap();
        assert_eq!(options.unique, Some(true));
        assert_eq!(options.partial_filter_expression, partial);
    }
    for (collection, name, keys) in [
        (
            OUTBOX,
            "witnessed_at_1_order_1",
            doc! {"witnessed_at": 1, "order": 1},
        ),
        (
            PENDING,
            "materialized_seq_1_order_1",
            doc! {"materialized_seq": 1, "order": 1},
        ),
        (
            crate::models::audit_effect_intent::COLLECTION_NAME,
            "terminal_stage_1_terminal_order_1",
            doc! {"terminal_stage": 1, "terminal_order": 1},
        ),
    ] {
        let indexes: Vec<_> = db
            .collection::<bson::Document>(collection)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let index = indexes
            .iter()
            .find(|index| {
                index
                    .options
                    .as_ref()
                    .and_then(|options| options.name.as_deref())
                    == Some(name)
            })
            .unwrap();
        assert_eq!(index.keys, keys);
        assert_ne!(index.options.as_ref().unwrap().unique, Some(true));
    }
    db.collection::<bson::Document>(OUTBOX)
        .drop_index("audit_outbox_order")
        .await
        .unwrap();
    builds.store(0, Ordering::SeqCst);
    ensure_indexes(&db).await.unwrap();
    assert_eq!(
        builds.load(Ordering::SeqCst),
        5,
        "no process readiness cache may hide lost constraints"
    );
    let indexes: Vec<_> = db
        .collection::<bson::Document>(OUTBOX)
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        indexes
            .iter()
            .any(|index| index
                .options
                .as_ref()
                .is_some_and(
                    |options| options.name.as_deref() == Some("audit_outbox_order")
                        && options.unique == Some(true)
                ))
    );
}

#[cfg(test)]
#[path = "audit_profile_tests.rs"]
mod profile;
