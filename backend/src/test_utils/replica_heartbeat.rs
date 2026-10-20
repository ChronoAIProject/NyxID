//! Fixture-owned security observations. A worker must not keep the fixture's
//! lifecycle client alive, or race its database deletion at process shutdown.
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::{errors::AppResult, services::security_readiness_service};

struct Task {
    stop: CancellationToken,
    handle: AbortOnDropHandle<AppResult<()>>,
}
type Slot = std::sync::Arc<tokio::sync::Mutex<Option<Task>>>;
static TASKS: LazyLock<Mutex<HashMap<String, Slot>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) async fn start(db: &mongodb::Database) -> AppResult<()> {
    start_with_period(db, Duration::from_secs(10)).await
}

async fn start_with_period(db: &mongodb::Database, period: Duration) -> AppResult<()> {
    // This independent client has no fixture drop guard. Capturing db.clone()
    // would prevent the lifecycle guard from ever reaching its cancellation.
    let mut options = mongodb::options::ClientOptions::parse(
        super::TEST_DB_PINNED_URI
            .get()
            .expect("fixture Mongo URI")
            .as_str(),
    )
    .await?;
    super::local_test_connection(&mut options);
    let background = mongodb::Client::with_options(options)?.database(db.name());
    let mut tasks = TASKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tasks.entry(db.name().to_owned()).or_insert_with(|| {
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let handle = AbortOnDropHandle::new(tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    () = cancelled.cancelled() => return Ok(()),
                    () = tokio::time::sleep(period) => {},
                }
                // Database shutdown or an observation failure terminates the
                // worker with a result; it never panics in a detached task.
                security_readiness_service::observe(
                    &background,
                    &crate::services::node_owner_service::ReplicaIdentity {
                        instance_name: "test-backend".into(),
                        generation_id: "test-process".into(),
                        internal_base_url: "http://127.0.0.1:3002".into(),
                    },
                    &security_readiness_service::RolloutConfig {
                        expected_replicas: vec!["test-backend".into()],
                        drained: true,
                    },
                    false,
                )
                .await?;
            }
        }));
        std::sync::Arc::new(tokio::sync::Mutex::new(Some(Task { stop, handle })))
    });
    Ok(())
}

/// Called before session cleanup/drop on the independent cleanup runtime.
/// Let an in-flight observation finish before dropping Mongo. Concurrent cleanup
/// callers await the same slot; neither may drop the database ahead of the join.
pub(super) async fn stop(name: &str) -> bool {
    let slot = TASKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(name)
        .cloned();
    let Some(slot) = slot else {
        return true;
    };
    let mut task_slot = slot.lock().await;
    let Some(task) = task_slot.as_mut() else {
        return true;
    };
    task.stop.cancel();
    // Keep the handle registered if the caller's existing cleanup budget
    // expires: a subsequent attempt must still join it before deleting data.
    let clean = match (&mut task.handle).await {
        Ok(Ok(())) => true,
        Err(error) if error.is_cancelled() => true, // test runtime already ended
        _ => {
            eprintln!("Fixture replica heartbeat failed before teardown");
            false
        }
    };
    *task_slot = None;
    TASKS.lock().unwrap_or_else(|p| p.into_inner()).remove(name);
    clean
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn audit_fixture_teardown_joins_replica_heartbeat_without_panic_or_database_recreation() {
        let db = crate::test_utils::connect_transaction_test_database("replica_teardown").await;
        security_readiness_service::ready_test_replica(&db).await;
        // Exercise the same worker at a short test cadence, without adding a
        // ten-second sleep to the suite or changing production/test timeouts.
        assert!(stop(db.name()).await);
        start_with_period(&db, Duration::from_millis(1))
            .await
            .unwrap();
        let name = db.name().to_owned();
        let observer = super::super::observe_test_db_drop(&name);
        assert!(TASKS.lock().unwrap().contains_key(&name));
        drop(db);
        let dropped =
            tokio::task::spawn_blocking(move || observer.recv_timeout(Duration::from_secs(20)))
                .await
                .unwrap()
                .unwrap();
        assert!(
            dropped,
            "cleanup must cancel and join the heartbeat before dropping Mongo"
        );
        assert!(!TASKS.lock().unwrap().contains_key(&name));
        tokio::time::sleep(Duration::from_millis(10)).await;
        let client =
            super::super::test_db_cleanup_client(super::super::TEST_DB_PINNED_URI.get().unwrap())
                .await
                .unwrap();
        assert!(!client.list_database_names().await.unwrap().contains(&name));
    }

    #[tokio::test]
    async fn audit_fixture_teardown_explicit_api_joins_before_drop_with_live_client_clones() {
        let db =
            crate::test_utils::connect_transaction_test_database("replica_explicit_drop").await;
        security_readiness_service::ready_test_replica(&db).await;
        assert!(stop(db.name()).await);
        start_with_period(&db, Duration::from_millis(1))
            .await
            .unwrap();
        let clone = db.clone();
        let name = db.name().to_owned();
        // Allow observations to run, including the possibility of one in flight
        // at the cancellation boundary. A failed heartbeat makes teardown fail.
        tokio::time::sleep(Duration::from_millis(10)).await;
        crate::test_utils::teardown_test_database(&db).await;
        assert!(!TASKS.lock().unwrap().contains_key(&name));
        assert!(!super::super::test_db_is_registered_for_cleanup(&name));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            !clone
                .client()
                .list_database_names()
                .await
                .unwrap()
                .contains(&name)
        );
        // Idempotent explicit teardown and eventual client Drop must be safe.
        crate::test_utils::teardown_test_database(&clone).await;
    }
}
