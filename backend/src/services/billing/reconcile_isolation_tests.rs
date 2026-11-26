use super::*;
use crate::services::coordination_service::LeaseStore;
use mongodb::bson::Bson;

async fn epoch(db: &mongodb::Database) -> i64 {
    db.collection::<Document>("billing_rate_epoch")
        .find_one(doc! {})
        .await
        .unwrap()
        .and_then(|d| d.get_i64("generation").ok())
        .unwrap_or(0)
}
async fn failures(db: &mongodb::Database) -> Document {
    db.collection::<Document>("coordination_leases")
        .find_one(doc! {"_id":"billing:reconcile"})
        .await
        .unwrap()
        .and_then(|d| d.get_document("checkpoint").ok().cloned())
        .and_then(|d| d.get_document("failures").ok().cloned())
        .unwrap_or_default()
}
async fn prove(step: &str, collection: &str, poison: Document) {
    let f = Fixture::new("billing_pass_isolation", SUCCESS).await;
    f.set_sync_enabled(false).await;
    f.db.collection::<Document>(collection)
        .insert_one(poison.clone())
        .await
        .unwrap();
    let mut first = None;
    for pass in 1..=2_i64 {
        if collection == "platform_model_attempts" {
            f.db.collection::<Document>(collection)
                .update_one(
                    doc! {"_id":poison.get("_id")},
                    doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
        }
        f.provider.calls.lock().unwrap().clear();
        let before = epoch(&f.db).await;
        assert!(
            f.reconciler.run_once().await.is_err(),
            "failed pass must remain a failed result"
        );
        assert!(
            f.provider.calls.lock().unwrap().contains(&"rates"),
            "{step} starved rate refresh"
        );
        assert!(
            epoch(&f.db).await > before,
            "later cache publication must commit"
        );
        let state = failures(&f.db).await;
        let failure = state
            .get_document(step)
            .unwrap_or_else(|_| panic!("missing {step}: {state:?}"));
        assert_eq!(failure.get_i64("consecutive_passes").unwrap(), pass);
        let date = *failure.get_datetime("first_failed_at").unwrap();
        assert!(failure.get_datetime("last_failed_at").unwrap() >= &date);
        assert!(failure.get_i64("error_code").unwrap() > 0);
        if let Some(first) = first {
            assert_eq!(date, first);
        } else {
            first = Some(date);
        }
        let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap();
        let d = diagnostics
            .iter()
            .find(|d| d.code == "billing_reconcile_steps")
            .expect("Integrity failure entry");
        assert!(d.detail.contains(&format!("step={step}")), "{}", d.detail);
        assert!(
            d.detail.contains(&format!("consecutive_passes={pass}")),
            "{}",
            d.detail
        );
        assert!(!d.detail.contains("owner-1") && !d.detail.contains("poison"));
    }
    f.db.collection::<Document>(collection)
        .delete_one(doc! {"_id":poison.get("_id")})
        .await
        .unwrap();
    f.reconciler.run_once().await.unwrap();
    assert!(failures(&f.db).await.is_empty());
    assert!(
        crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap()
            .iter()
            .all(|d| d.code != "billing_reconcile_steps")
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn billing_pass_isolation_model_recovery() {
    prove(
        "model_recovery",
        "platform_model_attempts",
        doc! {"_id":23,"state":"dispatched","lease_until":bson::DateTime::from_millis(0)},
    )
    .await;
}
#[tokio::test]
async fn billing_pass_isolation_grant_expiry() {
    prove("grant_expiry","credit_grants",doc!{"_id":"poison","status":"active","reserved":crate::models::credits::Credits::ZERO,"expires_at":bson::DateTime::from_millis(0),"issued_ledgered_at":bson::DateTime::from_millis(0)}).await;
}
#[tokio::test]
async fn billing_pass_isolation_schedules() {
    prove(
        "schedules",
        "credit_schedules",
        doc! {"_id":"poison","is_active":true},
    )
    .await;
}
#[tokio::test]
async fn billing_pass_isolation_settlement_recovery() {
    prove(
        "settlement_recovery",
        USAGE,
        doc! {"_id":"poison","status":"failed","released":false,"quantity":1_i64},
    )
    .await;
}
#[tokio::test]
async fn billing_pass_isolation_usage_push() {
    prove("usage_push",USAGE,doc!{"_id":"poison","status":"finalized","released":true,"quantity":1_i64,"wallet_id":"wallet-1","lago_acked":false,"updated_at":bson::DateTime::from_millis(0)}).await;
}
#[tokio::test]
async fn billing_pass_isolation_wallet_refresh() {
    prove("wallet_balances",super::super::BILLING_WALLET,doc!{"_id":"poison","owner_id":"bad-fixture-wallet","plan_kind":"bad","lago_wallet_id":Bson::Null,"balance_synced_at":bson::DateTime::from_millis(0)}).await;
}

#[tokio::test]
async fn billing_pass_isolation_prerequisite_and_clean_commands() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observed = seen.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "billing_pass_clean",
        EventHandler::callback(move |e| {
            if let CommandEvent::Started(e) = e {
                observed.lock().unwrap().push((e.command_name, e.command));
            }
        }),
    )
    .await;
    db.collection::<Document>("billing_migrations")
        .delete_many(doc! {})
        .await
        .unwrap();
    let provider = Arc::new(Provider::default());
    let r = BillingReconciler::new(
        db.clone(),
        Some(provider.clone()),
        Arc::new(billing_enabled_config()),
    );
    db.collection::<Document>(USAGE)
        .insert_one(doc! {
            "_id": "metering-counted", "status": "finalized", "quantity": 1_i64,
            "rollup_pending": false, "released": true, "forwarded": true,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    seen.lock().unwrap().clear();
    r.run_once().await.unwrap();
    assert!(
        seen.lock().unwrap().iter().all(|(_, command)| {
            command.get_str("find").ok() != Some("usage_rollup_state")
                && command.get_str("update").ok() != Some("usage_rollup_state")
        }),
        "closed readiness gate must not run metering retention"
    );
    assert!(
        provider.calls.lock().unwrap().is_empty(),
        "migration readiness must stop all later work"
    );
    assert!(
        db.collection::<Document>(USAGE)
            .find_one(doc! {"_id": "metering-counted"})
            .await
            .unwrap()
            .unwrap()
            .get("expires_at")
            .is_none()
    );
    db.collection::<Document>("billing_migrations")
        .insert_one(doc! {"_id":"exact-v2","completed_at":bson::DateTime::now()})
        .await
        .unwrap();
    r.run_once().await.unwrap();
    assert!(
        db.collection::<Document>(USAGE)
            .find_one(doc! {"_id": "metering-counted"})
            .await
            .unwrap()
            .unwrap()
            .get_datetime("expires_at")
            .is_ok()
    );
    seen.lock().unwrap().clear();
    r.run_once().await.unwrap();
    let commands: Vec<_> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    println!(
        "BILLING_PASS_CLEAN commands={} names={commands:?}",
        commands.len()
    );
    assert_eq!(
        commands.len(),
        53,
        "comparison adds four coverage commands and two discovery reads while unactivated"
    );
    assert_eq!(
        commands.iter().filter(|c| c.as_str() == "delete").count(),
        0
    );
    assert!(failures(&db).await.is_empty());
    db.collection::<Document>("credit_schedules")
        .insert_one(doc! {"_id":"poison","is_active":true})
        .await
        .unwrap();
    db.collection::<Document>("credit_grants").insert_one(doc! {
        "_id":"poison", "status":"active", "reserved":crate::models::credits::Credits::ZERO,
        "expires_at":bson::DateTime::from_millis(0), "issued_ledgered_at":bson::DateTime::from_millis(0),
    }).await.unwrap();
    seen.lock().unwrap().clear();
    let error = r.run_once().await.unwrap_err().to_string();
    assert!(error.contains("schedules:") && error.contains("grant_expiry:"));
    assert_eq!(
        seen.lock().unwrap().len(),
        53,
        "failure publication adds no extra command beyond the holdless reads"
    );
    let count = |command: &str| {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(_, d)| d.get_str(command).ok() == Some("coordination_leases"))
            .count()
    };
    assert_eq!(
        count("update"),
        1,
        "all failure status shares the lease release update"
    );
    assert_eq!(count("delete"), 0);
    db.collection::<Document>("credit_schedules")
        .delete_one(doc! {"_id":"poison"})
        .await
        .unwrap();
    db.collection::<Document>("credit_grants")
        .delete_one(doc! {"_id":"poison"})
        .await
        .unwrap();
    seen.lock().unwrap().clear();
    r.run_once().await.unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        53,
        "clearing status adds no command"
    );
    assert_eq!(count("update"), 1);
    assert_eq!(
        count("delete"),
        0,
        "clearing failures retains the informational warming timestamp"
    );
    println!(
        "BILLING_PASS_STATUS failure_commands=53 clear_commands=53 failed_updates=1 clear_updates=1 added_commands=0"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_pass_isolation_drain_preserves_unattempted_failure() {
    let f = Fixture::new("pass_status_drain", SUCCESS).await;
    f.set_sync_enabled(false).await;
    f.db.collection::<Document>(super::super::BILLING_WALLET)
        .insert_one(doc! {
            "_id":"poison", "owner_id":"bad-fixture-wallet", "plan_kind":"bad",
            "lago_wallet_id":Bson::Null, "balance_synced_at":bson::DateTime::from_millis(0),
        })
        .await
        .unwrap();
    f.db.collection::<Document>(USAGE)
        .insert_one(doc! {
            "_id":"poison-usage", "status":"finalized", "released":true, "quantity":1_i64,
            "wallet_id":"wallet-1", "lago_acked":false, "updated_at":bson::DateTime::from_millis(0),
        })
        .await
        .unwrap();
    assert!(f.reconciler.run_once().await.is_err());
    let before = failures(&f.db).await;
    f.db.collection::<Document>(super::super::BILLING_WALLET)
        .delete_one(doc! {"_id":"poison"})
        .await
        .unwrap();
    // Acknowledged meter/receipt pairs are terminal; drain a fresh delivery.
    f.db.collection::<UsageMeterRow>(USAGE)
        .insert_one(finalized_row("drain-fresh-delivery"))
        .await
        .unwrap();
    f.db.collection::<Document>(USAGE)
        .delete_one(doc! {"_id":"poison-usage"})
        .await
        .unwrap();
    assert!(before.contains_key("usage_push") && before.contains_key("wallet_balances"));
    f.provider.pause_push.store(true, Ordering::SeqCst);
    let shutdown = crate::services::shutdown::Runtime::default();
    let lease = lease_runtime("draining", 30_000);
    let (result, ()) = tokio::join!(
        f.reconciler.run_once_with_shutdown(&lease, &shutdown),
        async {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                f.provider.push_entered.notified(),
            )
            .await
            .unwrap();
            shutdown.begin_drain();
            f.provider.resume_push.notify_one();
        }
    );
    result.unwrap();
    assert_eq!(
        failures(&f.db).await,
        before,
        "partial and unattempted status survive drain unchanged"
    );
    f.provider.pause_push.store(false, Ordering::SeqCst);
    f.reconciler.run_once().await.unwrap();
    assert!(failures(&f.db).await.is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_pass_isolation_status_is_fenced_and_old_holder_preserves_it() {
    use crate::services::billing::reconcile_status::Pass;
    let db = crate::test_utils::connect_transaction_test_database("pass_status_fence").await;
    let runtime = lease_runtime("first", 30_000);
    let token = runtime
        .acquire(&db, super::super::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    let mut first = Pass::new(None);
    first.record::<()>("schedules", Err(AppError::Internal("not persisted".into())));
    assert!(first.release(&db, &token).await.unwrap());
    let before = failures(&db).await;
    // An old binary uses acquire/release, ignores the checkpoint and retains it.
    let old = runtime
        .acquire(&db, super::super::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    assert!(LeaseStore::release(&db, &old).await.unwrap());
    assert_eq!(failures(&db).await, before);
    let (stale, saved) = LeaseStore::acquire_with_checkpoint(
        &db,
        super::super::RECONCILE_LEASE,
        &runtime.holder,
        runtime.ttl,
    )
    .await
    .unwrap()
    .unwrap();
    let mut stale_pass = Pass::new(saved.clone());
    stale_pass.record("schedules", Ok(()));
    db.collection::<Document>("coordination_leases")
        .update_one(
            doc! {"_id":super::super::RECONCILE_LEASE},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert!(
        !stale_pass.release(&db, &stale).await.unwrap(),
        "expiry fences status even before takeover"
    );
    let peer = lease_runtime("successor", 30_000);
    let successor = peer
        .acquire(&db, super::super::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    let mut next = Pass::new(saved);
    next.record::<()>(
        "schedules",
        Err(AppError::Internal("also not persisted".into())),
    );
    assert!(next.release(&db, &successor).await.unwrap());
    assert!(!stale_pass.release(&db, &stale).await.unwrap());
    assert_eq!(
        failures(&db)
            .await
            .get_document("schedules")
            .unwrap()
            .get_i64("consecutive_passes")
            .unwrap(),
        2
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_pass_isolation_status_write_failure_does_not_hide_step_failures() {
    let f = Fixture::new("pass_status_write", SUCCESS).await;
    f.set_sync_enabled(false).await;
    f.db.collection::<Document>("credit_schedules")
        .insert_one(doc! {"_id":"poison","is_active":true})
        .await
        .unwrap();
    f.db.create_collection("coordination_leases").await.unwrap();
    f.db.run_command(
        doc! {"collMod":"coordination_leases", "validator":{"checkpoint":{"$exists":false}}},
    )
    .await
    .unwrap();
    let before = epoch(&f.db).await;
    let error = f.reconciler.run_once().await.unwrap_err().to_string();
    assert!(
        error.contains("schedules:") && error.contains("status_publication:"),
        "{error}"
    );
    assert!(!error.contains("poison"));
    assert!(epoch(&f.db).await > before);
    assert!(
        f.db.collection::<Document>("coordination_leases")
            .find_one(doc! {"_id":super::super::RECONCILE_LEASE})
            .await
            .unwrap()
            .is_none()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_pass_isolation_unreadable_status_does_not_block_work_or_expose_text() {
    let f = Fixture::new("pass_status_unreadable", SUCCESS).await;
    f.set_sync_enabled(false).await;
    let runtime = lease_runtime("seed", 30_000);
    let token = runtime
        .acquire(&f.db, super::super::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    assert!(
        LeaseStore::store_checkpoint(&f.db, &token, Bson::String("untrusted fixture text".into()))
            .await
            .unwrap()
    );
    LeaseStore::release(&f.db, &token).await.unwrap();
    let before = epoch(&f.db).await;
    let error = f.reconciler.run_once().await.unwrap_err().to_string();
    assert!(error.contains("status_unreadable"));
    assert!(!error.contains("untrusted"));
    assert!(epoch(&f.db).await > before);
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    assert!(
        diagnostics
            .iter()
            .all(|d| d.code != "billing_reconcile_steps")
    );
    f.reconciler
        .run_once()
        .await
        .expect("malformed status heals after its first completed pass");
    f.db.drop().await.unwrap();
}

#[path = "reconcile_isolation_round2_tests.rs"]
mod round2;
