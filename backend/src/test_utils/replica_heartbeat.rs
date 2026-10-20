//! Fixture-owned security observations. A worker must not keep the fixture's
//! lifecycle client alive, or race its database deletion at process shutdown.
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::errors::AppResult;
use mongodb::bson::doc;

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
    let fixture_name = db.name().to_owned();
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
                // Initial readiness created this row. Never upsert during a
                // fixture heartbeat: a completed raw drop between ticks must
                // not recreate its database. Missing/deleting fixture data is
                // a clean terminal observation only for this test worker.
                if !refresh(&background, &fixture_name).await? {
                    return Ok(());
                }
            }
        }));
        std::sync::Arc::new(tokio::sync::Mutex::new(Some(Task { stop, handle })))
    });
    Ok(())
}

async fn refresh(db: &mongodb::Database, fixture_name: &str) -> AppResult<bool> {
    let result = db
        .collection::<crate::models::security_replica::SecurityReplica>(
            crate::models::security_replica::COLLECTION_NAME,
        )
        .update_one(
            doc! {"_id": "test-process"},
            doc! {"$set": {"observed_at": mongodb::bson::DateTime::now()}},
        )
        .await;
    match result {
        Ok(result) => Ok(result.matched_count == 1),
        Err(error) if own_fixture_drop(fixture_name, db.name(), &error) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

// Only the result of the exact fixture-scoped observation above may use this
// classification. No production heartbeat or other database error is ignored.
fn own_fixture_drop(
    fixture_name: &str,
    request_database: &str,
    error: &mongodb::error::Error,
) -> bool {
    if fixture_name != request_database || super::parse_managed_test_db_name(fixture_name).is_none()
    {
        return false;
    }
    match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(error) => matches!(error.code, 215 | 26),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(error)) => {
            matches!(error.code, 215 | 26)
        }
        _ => false,
    }
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
    use crate::services::security_readiness_service;

    #[test]
    fn audit_fixture_heartbeat_clean_drop_is_scoped_to_own_database_and_two_codes() {
        let name = super::super::new_test_db_name("heartbeat_scope");
        for code in [215, 26, 112, 13] {
            let command: mongodb::error::CommandError = mongodb::bson::from_document(
                doc! {"code": code, "codeName": "test", "errmsg": "fixture error"},
            )
            .unwrap();
            let error = mongodb::error::Error::from(mongodb::error::ErrorKind::Command(command));
            assert_eq!(
                own_fixture_drop(&name, &name, &error),
                matches!(code, 215 | 26)
            );
            assert!(!own_fixture_drop(&name, "admin", &error));
            assert!(!own_fixture_drop("admin", "admin", &error));
            let write: mongodb::error::WriteError =
                mongodb::bson::from_document(doc! {"code": code, "errmsg": "fixture error"})
                    .unwrap();
            let error = mongodb::error::Error::from(mongodb::error::ErrorKind::Write(
                mongodb::error::WriteFailure::WriteError(write),
            ));
            assert_eq!(
                own_fixture_drop(&name, &name, &error),
                matches!(code, 215 | 26)
            );
        }
    }

    #[tokio::test]
    async fn audit_fixture_heartbeat_raw_drop_exits_cleanly_without_recreating_database() {
        let db = crate::test_utils::connect_transaction_test_database("heartbeat_raw_drop").await;
        security_readiness_service::ready_test_replica(&db).await;
        assert!(stop(db.name()).await);
        start_with_period(&db, Duration::from_millis(1))
            .await
            .unwrap();
        let slot = TASKS.lock().unwrap().get(db.name()).unwrap().clone();
        // Legacy callers may drop while the fixture client/heartbeat is live.
        db.drop().await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if slot.lock().await.as_ref().unwrap().handle.is_finished() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("raw fixture deletion must finish its heartbeat");
        // Joining an Ok worker cannot emit the heartbeat-failure diagnostic.
        assert!(stop(db.name()).await);
        assert!(!TASKS.lock().unwrap().contains_key(db.name()));
        crate::test_utils::teardown_test_database(&db).await;
        assert!(
            !db.client()
                .list_database_names()
                .await
                .unwrap()
                .contains(&db.name().to_owned())
        );
    }

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
