use super::*;
use crate::models::coordination::LEASE_COLLECTION_NAME;
use crate::models::pool_recovery_diagnostic::COLLECTION_NAME as POOL_DIAGNOSTICS;
use crate::models::usage_meter::POOL_RECOVERY_COLLECTION_NAME;
use crate::services::billing::reconcile::{RECONCILE_LEASE, spawn_reconcile_worker};

async fn checkpoint(db: &mongodb::Database) -> Option<Bson> {
    db.collection::<Document>(LEASE_COLLECTION_NAME)
        .find_one(doc! {"_id":RECONCILE_LEASE})
        .await
        .unwrap()
        .and_then(|mut d| d.remove("checkpoint"))
}
async fn seed_checkpoint(db: &mongodb::Database, checkpoint: Bson) {
    let runtime = lease_runtime("status-seed", 30_000);
    let token = runtime.acquire(db, RECONCILE_LEASE).await.unwrap().unwrap();
    assert!(
        LeaseStore::store_checkpoint(db, &token, checkpoint)
            .await
            .unwrap()
    );
    assert!(LeaseStore::release(db, &token).await.unwrap());
}
fn failure(count: i64) -> Document {
    doc! {"consecutive_passes":count, "error_code":1007_i64,
    "first_failed_at":bson::DateTime::from_millis(1),"last_failed_at":bson::DateTime::from_millis(2)}
}
async fn pool_poison(db: &mongodb::Database) {
    db.collection::<Document>(POOL_RECOVERY_COLLECTION_NAME)
        .insert_one(doc! {"_id":"bad-cursor","name":"expired_attempts"})
        .await
        .unwrap();
}

#[tokio::test]
async fn billing_pass_round2_pool_keeps_existing_diagnostic() {
    let f = Fixture::new("pass_r2_pool", SUCCESS).await;
    f.set_sync_enabled(false).await;
    pool_poison(&f.db).await;
    for pass in 1..=2_i64 {
        let before = epoch(&f.db).await;
        let error = f.reconciler.run_once().await.unwrap_err().to_string();
        assert!(error.contains("pool_recovery:1007"), "{error}");
        assert!(epoch(&f.db).await > before);
        let old =
            f.db.collection::<Document>(POOL_DIAGNOSTICS)
                .find_one(doc! {"name":"pool_recovery"})
                .await
                .unwrap()
                .expect("existing pool diagnostic must still be written");
        assert_eq!(old.get_i64("failures").unwrap(), pass);
        let samples = old.get_array("samples").unwrap();
        assert_eq!(samples.len(), pass as usize);
        assert!(
            samples
                .iter()
                .all(|s| s.as_document().unwrap().get("request_id") == Some(&Bson::Null))
        );
        assert_eq!(
            failures(&f.db)
                .await
                .get_document("pool_recovery")
                .unwrap()
                .get_i64("consecutive_passes")
                .unwrap(),
            pass
        );
        let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap();
        for code in ["billing_pool_recovery", "billing_reconcile_steps"] {
            assert!(diagnostics.iter().any(|d| d.code == code), "missing {code}");
        }
    }
    f.db.collection::<Document>(POOL_RECOVERY_COLLECTION_NAME)
        .delete_one(doc! {"_id":"bad-cursor"})
        .await
        .unwrap();
    f.reconciler.run_once().await.unwrap();
    assert!(failures(&f.db).await.is_empty());
    assert_eq!(
        f.db.collection::<Document>(POOL_DIAGNOSTICS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .get_i64("failures")
            .unwrap(),
        2,
        "existing history is retained"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_pass_round2_unknown_step_survives_two_passes_and_old_holder() {
    let f = Fixture::new("pass_r2_unknown", SUCCESS).await;
    f.set_sync_enabled(false).await;
    let mut future = failure(7);
    future.insert(
        "extension",
        doc! {"opaque":"$must_stay_literal","expression":{"$add":[1,2]}},
    );
    seed_checkpoint(
        &f.db,
        Bson::Document(
            doc! {"version":1,"failures":{"future_step":&future,"schedules":failure(4)}},
        ),
    )
    .await;
    f.db.collection::<Document>("credit_schedules")
        .insert_one(doc! {"_id":"poison","is_active":true})
        .await
        .unwrap();
    let error = f.reconciler.run_once().await.unwrap_err().to_string();
    assert!(
        !error.contains("status_unreadable"),
        "unknown name is not corruption: {error}"
    );
    assert!(error.contains("schedules:1007"));
    let saved = failures(&f.db).await;
    assert_eq!(saved.get_document("future_step").unwrap(), &future);
    assert_eq!(
        saved
            .get_document("schedules")
            .unwrap()
            .get_i64("consecutive_passes")
            .unwrap(),
        5
    );
    f.db.collection::<Document>("credit_schedules")
        .delete_one(doc! {"_id":"poison"})
        .await
        .unwrap();
    f.reconciler.run_once().await.unwrap();
    let saved = failures(&f.db).await;
    assert_eq!(saved, doc! {"future_step":&future});
    let runtime = lease_runtime("pre-isolation-holder", 30_000);
    let old = runtime
        .acquire(&f.db, RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    LeaseStore::release(&f.db, &old).await.unwrap();
    assert_eq!(failures(&f.db).await, saved);
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    let d = diagnostics
        .iter()
        .find(|d| d.code == "billing_reconcile_steps")
        .unwrap();
    assert!(
        d.detail.contains("step=future_step consecutive_passes=7")
            && d.detail.contains("error_code=1007"),
        "{}",
        d.detail
    );
    assert!(
        !d.detail.contains("must_stay_literal"),
        "unknown metadata payload is not displayed"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn billing_pass_round2_newer_version_survives_two_passes_without_failure() {
    let f = Fixture::new("pass_r2_newer", SUCCESS).await;
    f.set_sync_enabled(false).await;
    let future = Bson::Document(
        doc! {"version":2_i64,"failures":"opaque future format","extension":{"$literal":"preserve"}},
    );
    seed_checkpoint(&f.db, future.clone()).await;
    for _ in 0..2 {
        let before = epoch(&f.db).await;
        f.reconciler
            .run_once()
            .await
            .expect("newer status cannot fail a healthy pass");
        assert!(epoch(&f.db).await > before);
        assert_eq!(checkpoint(&f.db).await, Some(future.clone()));
        let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap();
        let d = diagnostics
            .iter()
            .find(|d| d.code == "billing_reconcile_steps")
            .unwrap();
        assert!(
            d.detail.contains("status_newer_version") && d.detail.contains("version=2"),
            "{}",
            d.detail
        );
        assert!(!d.detail.contains("opaque"));
    }
    // A real failure still returns an error, without overwriting opaque status.
    f.db.collection::<Document>("credit_schedules")
        .insert_one(doc! {"_id":"poison","is_active":true})
        .await
        .unwrap();
    let error = f.reconciler.run_once().await.unwrap_err().to_string();
    assert!(error.contains("schedules:1007") && !error.contains("status_unreadable"));
    assert_eq!(checkpoint(&f.db).await, Some(future));
    f.db.drop().await.unwrap();
}

async fn heals(value: Bson, with_failure: bool) {
    let f = Fixture::new("pass_r2_repair", SUCCESS).await;
    f.set_sync_enabled(false).await;
    seed_checkpoint(&f.db, value).await;
    if with_failure {
        f.db.collection::<Document>("credit_schedules")
            .insert_one(doc! {"_id":"poison","is_active":true})
            .await
            .unwrap();
    }
    let before = epoch(&f.db).await;
    let error = f.reconciler.run_once().await.unwrap_err().to_string();
    assert!(
        error.contains("status_unreadable:1006") && !error.contains("untrusted"),
        "{error}"
    );
    assert!(epoch(&f.db).await > before);
    if with_failure {
        assert!(error.contains("schedules:1007"));
        let state = failures(&f.db).await;
        assert_eq!(state.len(), 1, "only this pass's fresh failure remains");
        assert_eq!(
            state
                .get_document("schedules")
                .unwrap()
                .get_i64("consecutive_passes")
                .unwrap(),
            1
        );
        f.db.collection::<Document>("credit_schedules")
            .delete_one(doc! {"_id":"poison"})
            .await
            .unwrap();
    } else {
        assert!(
            failures(&f.db).await.is_empty(),
            "malformed failures must be replaced by the informational warming checkpoint"
        );
    }
    f.reconciler
        .run_once()
        .await
        .expect("repair must stop perpetual pass failures");
    assert!(failures(&f.db).await.is_empty());
    assert!(
        checkpoint(&f.db)
            .await
            .unwrap()
            .as_document()
            .unwrap()
            .get_datetime("usage_warming_since")
            .is_ok()
    );
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code == "billing_reconcile_steps")
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn billing_pass_round2_malformed_status_heals_clean_and_failed_passes() {
    heals(Bson::String("untrusted fixture text".into()), false).await;
    let mut invalid = failure(-1);
    for (key, value) in [
        ("consecutive_passes", Bson::Int64(-1)),
        ("error_code", Bson::Int64(i64::from(u32::MAX) + 1)),
        ("first_failed_at", Bson::Null),
    ] {
        invalid.insert(key, value);
        heals(
            Bson::Document(doc! {"version":1,"failures":{"schedules":&invalid}}),
            true,
        )
        .await;
        invalid = failure(7);
    }
}
#[tokio::test]
async fn billing_pass_round2_old_format_heals_across_two_passes() {
    for old in [
        doc! {"version":0,"failures":{"schedules":failure(7)}},
        doc! {"failures":{"schedules":failure(7)}},
    ] {
        heals(Bson::Document(old), true).await;
    }
}

async fn broken_read(db: &mongodb::Database, target: &str, row: Document) {
    let source = format!("{target}_broken_read_fixture");
    db.collection::<Document>(&source)
        .insert_one(row)
        .await
        .unwrap();
    db.collection::<Document>(target).drop().await.unwrap();
    db.create_collection(target)
        .view_on(source)
        .pipeline(vec![doc! {"$set":{"explode":{"$divide":[1,"$zero"]}}}])
        .await
        .unwrap();
}

// Exercise the actual periodic caller's log site and worker lifecycle. No
// polling or sleeps: the subscriber signals after the failed-pass event.
struct PassLogged(Arc<tokio::sync::Notify>);
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for PassLogged {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Message(bool);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message"
                    && format!("{value:?}").contains("Billing reconcile sweep failed")
                {
                    self.0 = true;
                }
            }
        }
        let mut message = Message(false);
        event.record(&mut message);
        if message.0 {
            self.0.notify_one();
        }
    }
}
async fn worker_log(reconciler: BillingReconciler) -> String {
    use tracing_subscriber::prelude::*;
    let file = tempfile::NamedTempFile::new().unwrap();
    let logged = Arc::new(tokio::sync::Notify::new());
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_ansi(false)
                .with_target(false)
                .with_writer(file.reopen().unwrap()),
        )
        .with(PassLogged(logged.clone()));
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    let worker =
        spawn_reconcile_worker(reconciler, 300, crate::services::job_health::Job::new(true))
            .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), logged.notified())
        .await
        .unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    std::fs::read_to_string(file.path()).unwrap()
}
#[tokio::test]
async fn billing_pass_round2_one_log_preserves_stable_voice_code_and_all_steps() {
    let f = Fixture::new("pass_r2_log", SUCCESS).await;
    f.set_sync_enabled(false).await;
    pool_poison(&f.db).await;
    f.db.collection::<Document>("credit_schedules")
        .insert_one(doc! {"_id":"poison","is_active":true})
        .await
        .unwrap();
    broken_read(
        &f.db,
        crate::models::assistant_voice::WINDOWS,
        doc! {"_id":"window","settled":false,"sealed":true,"zero":0},
    )
    .await;
    let logs = worker_log(f.reconciler.clone()).await;
    let lines: Vec<_> = logs
        .lines()
        .filter(|s| s.contains("Billing reconcile sweep failed"))
        .collect();
    assert_eq!(lines.len(), 1, "{logs}");
    for code in [
        "pool_recovery:1007",
        "schedules:1007",
        "voice_recovery:1007",
        "billing_voice_window_recovery_failed",
    ] {
        assert!(lines[0].contains(code), "missing {code}: {logs}");
    }
    assert!(lines[0].contains("WARN"));
    assert!(
        !logs.contains("Pool recovery failed;"),
        "pool pass failure shares the aggregate log: {logs}"
    );
    println!("BILLING_PASS_ROUND2_LOG {}", lines[0]);
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn billing_pass_round2_prerequisite_error_logs_once_and_stops() {
    let db = crate::test_utils::connect_transaction_test_database("pass_r2_prerequisite").await;
    broken_read(
        &db,
        "billing_migrations",
        doc! {"_id":"exact-v2","completed_at":bson::DateTime::now(),"zero":0},
    )
    .await;
    let provider = Arc::new(Provider::default());
    let r = BillingReconciler::new(
        db.clone(),
        Some(provider.clone()),
        Arc::new(billing_enabled_config()),
    );
    let logs = worker_log(r).await;
    let lines: Vec<_> = logs
        .lines()
        .filter(|s| s.contains("Billing reconcile sweep failed"))
        .collect();
    assert_eq!(lines.len(), 1, "{logs}");
    assert!(
        lines[0].contains("WARN") && lines[0].contains("exact_readiness:1007"),
        "{logs}"
    );
    assert!(provider.calls.lock().unwrap().is_empty());
    assert!(epoch(&db).await == 0);
    println!("BILLING_PASS_ROUND2_PREREQUISITE {}", lines[0]);
    db.drop().await.unwrap();
}
