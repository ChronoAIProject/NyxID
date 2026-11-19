//! Public process readiness. Optional downstreams and feature-specific rollout
//! readiness have their own diagnostics and must not evict the whole service.
use std::{future::Future, time::Duration};

use bson::doc;
use serde::Serialize;

use crate::services::shutdown::Runtime;

pub const CHECK_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Mongodb,
    Draining,
}

pub async fn check(db: &mongodb::Database, shutdown: &Runtime) -> Vec<Failure> {
    check_with(shutdown, async {
        // A primary-selected ping uses the same database/client and pool as
        // core requests. The outer deadline also bounds server selection and
        // connection checkout, which maxTimeMS on its own would not cover.
        db.run_command(doc! {"ping": 1})
            .await
            .map(|_| ())
            .map_err(|_| ())
    })
    .await
}

async fn check_with(
    shutdown: &Runtime,
    mongodb: impl Future<Output = Result<(), ()>>,
) -> Vec<Failure> {
    tokio::select! {
        biased;
        _ = shutdown.intake_stopped() => vec![Failure::Draining],
        result = tokio::time::timeout(CHECK_TIMEOUT, mongodb) => {
            // Recheck after the dependency read so a concurrent drain cannot
            // turn an already completed ping into a new ready success.
            if shutdown.is_draining() {
                vec![Failure::Draining]
            } else if matches!(result, Ok(Ok(()))) {
                Vec::new()
            } else {
                vec![Failure::Mongodb]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::shutdown::Runtime;

    #[tokio::test]
    async fn readiness_dependency_failure_and_recovery() {
        let shutdown = Runtime::default();
        assert_eq!(
            check_with(&shutdown, async { Err(()) }).await,
            vec![Failure::Mongodb]
        );
        assert!(check_with(&shutdown, async { Ok(()) }).await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_dependency_timeout_is_bounded() {
        let start = tokio::time::Instant::now();
        assert_eq!(
            check_with(&Runtime::default(), std::future::pending()).await,
            vec![Failure::Mongodb]
        );
        assert_eq!(start.elapsed(), CHECK_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_drain_short_circuits_without_polling_database() {
        let shutdown = Runtime::default();
        shutdown.begin_drain();
        assert_eq!(
            check_with(&shutdown, async {
                panic!("draining must not poll MongoDB")
            })
            .await,
            vec![Failure::Draining]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_drain_interrupts_an_inflight_check() {
        let shutdown = Runtime::default();
        let drain = shutdown.clone();
        let (failures, ()) =
            tokio::join!(check_with(&shutdown, std::future::pending()), async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                drain.begin_drain();
            });
        assert_eq!(failures, vec![Failure::Draining]);
    }
}
