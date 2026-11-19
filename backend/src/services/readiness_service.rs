//! Bounded public dependency diagnostics, separate from drain-only orchestration.
//! Optional downstream failures must not evict the whole service.
use std::{future::Future, sync::Arc, time::Duration};

use bson::doc;
use serde::Serialize;

use crate::services::shutdown::Runtime;

pub const CHECK_TIMEOUT: Duration = Duration::from_secs(1);
const REUSE_WINDOW: Duration = Duration::from_secs(1);

#[derive(Clone, Default)]
pub struct Probe {
    observation: Arc<tokio::sync::Mutex<Option<Observation>>>,
}

struct Observation {
    started_at: tokio::time::Instant,
    available: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Mongodb,
    Draining,
}

impl Probe {
    pub async fn check(&self, db: &mongodb::Database, shutdown: &Runtime) -> Vec<Failure> {
        self.check_with(shutdown, async {
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
        &self,
        shutdown: &Runtime,
        mongodb: impl Future<Output = Result<(), ()>>,
    ) -> Vec<Failure> {
        let shared = async {
            // One in-flight read per application instance. The outer deadline
            // includes waiting for this lock, server selection and pool checkout.
            let mut observation = self.observation.lock().await;
            if let Some(previous) = observation.as_ref()
                && previous.started_at.elapsed() < REUSE_WINDOW
            {
                return previous.available;
            }
            // Reserve a pessimistic result before polling MongoDB. Cancellation
            // drops the read and lock, but cannot cause a stream of new pings.
            *observation = Some(Observation {
                started_at: tokio::time::Instant::now(),
                available: false,
            });
            let available = mongodb.await.is_ok();
            observation.as_mut().unwrap().available = available;
            // Freshness starts at admission, never at completion. A slow read
            // must not gain another second of cached success after finishing.
            available
        };
        tokio::select! {
            biased;
            _ = shutdown.intake_stopped() => vec![Failure::Draining],
            result = tokio::time::timeout(CHECK_TIMEOUT, shared) => {
                // Recheck after the dependency read so a concurrent drain cannot
                // turn an already completed ping into a new ready success.
                if shutdown.is_draining() {
                    vec![Failure::Draining]
                } else if matches!(result, Ok(true)) {
                    Vec::new()
                } else {
                    vec![Failure::Mongodb]
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::shutdown::Runtime;

    #[tokio::test(start_paused = true)]
    async fn readiness_dependency_failure_and_recovery() {
        let shutdown = Runtime::default();
        let probe = Probe::default();
        assert_eq!(
            probe.check_with(&shutdown, async { Err(()) }).await,
            vec![Failure::Mongodb]
        );
        assert_eq!(
            probe
                .check_with(&shutdown, async { panic!("failure is reused") })
                .await,
            vec![Failure::Mongodb]
        );
        tokio::time::advance(REUSE_WINDOW).await;
        assert!(
            probe
                .check_with(&shutdown, async { Ok(()) })
                .await
                .is_empty()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_dependency_timeout_is_bounded() {
        let start = tokio::time::Instant::now();
        assert_eq!(
            Probe::default()
                .check_with(&Runtime::default(), std::future::pending())
                .await,
            vec![Failure::Mongodb]
        );
        assert_eq!(start.elapsed(), CHECK_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_drain_short_circuits_without_polling_database() {
        let shutdown = Runtime::default();
        shutdown.begin_drain();
        assert_eq!(
            Probe::default()
                .check_with(&shutdown, async {
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
        let probe = Probe::default();
        let (failures, waiting, ()) = tokio::join!(
            probe.check_with(&shutdown, std::future::pending()),
            probe.check_with(&shutdown, async { panic!("waiter must not poll MongoDB") }),
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                drain.begin_drain();
            }
        );
        assert_eq!(failures, vec![Failure::Draining]);
        assert_eq!(waiting, vec![Failure::Draining]);
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_concurrent_requests_share_one_read() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let reads = AtomicUsize::new(0);
        let shutdown = Runtime::default();
        let probe = Probe::default();
        let results = futures::future::join_all((0..50).map(|_| {
            probe.check_with(&shutdown, async {
                reads.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok(())
            })
        }))
        .await;
        assert!(results.iter().all(Vec::is_empty));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_success_expires_from_read_start_and_drain_bypasses_it() {
        let shutdown = Runtime::default();
        let probe = Probe::default();
        assert!(
            probe
                .check_with(&shutdown, async {
                    tokio::time::sleep(Duration::from_millis(900)).await;
                    Ok(())
                })
                .await
                .is_empty()
        );
        assert!(
            probe
                .check_with(&shutdown, async { panic!("success is reused") })
                .await
                .is_empty()
        );
        tokio::time::advance(Duration::from_millis(100)).await;
        assert_eq!(
            probe.check_with(&shutdown, async { Err(()) }).await,
            vec![Failure::Mongodb]
        );
        tokio::time::advance(REUSE_WINDOW).await;
        assert!(
            probe
                .check_with(&shutdown, async { Ok(()) })
                .await
                .is_empty()
        );
        shutdown.begin_drain();
        assert_eq!(
            probe
                .check_with(&shutdown, async { panic!("drain bypasses cached success") })
                .await,
            vec![Failure::Draining]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_cancelled_leader_cannot_restart_a_read_within_the_window() {
        let shutdown = Runtime::default();
        let probe = Probe::default();
        let mut leader = Box::pin(probe.check_with(&shutdown, std::future::pending()));
        assert!(futures::poll!(&mut leader).is_pending());
        drop(leader);
        assert_eq!(
            probe
                .check_with(&shutdown, async {
                    panic!("cancelled result is pessimistic")
                })
                .await,
            vec![Failure::Mongodb]
        );
        tokio::time::advance(REUSE_WINDOW).await;
        assert!(
            probe
                .check_with(&shutdown, async { Ok(()) })
                .await
                .is_empty()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_lock_wait_is_included_in_the_deadline() {
        let shutdown = Runtime::default();
        let probe = Probe::default();
        let _guard = probe.observation.lock().await;
        let start = tokio::time::Instant::now();
        assert_eq!(
            probe
                .check_with(&shutdown, async { panic!("lock not acquired") })
                .await,
            vec![Failure::Mongodb]
        );
        assert_eq!(start.elapsed(), CHECK_TIMEOUT);
    }
}
