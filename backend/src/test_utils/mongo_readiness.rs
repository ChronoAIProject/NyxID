use std::fmt;
use std::future::Future;
use std::time::Duration;

use tokio::time::{Instant, timeout_at};

const READINESS_BUDGET: Duration = Duration::from_secs(90);

pub(super) struct Readiness {
    started_at: Instant,
    deadline: Instant,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReadinessError {
    TimedOut(&'static str),
    OperationFailed(&'static str),
}

impl fmt::Display for ReadinessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut(phase) => {
                write!(f, "{phase} timed out at the shared readiness deadline")
            }
            Self::OperationFailed(phase) => write!(f, "{phase} returned a MongoDB error"),
        }
    }
}

impl Readiness {
    pub(super) fn new() -> Self {
        let started_at = Instant::now();
        Self {
            started_at,
            deadline: started_at + READINESS_BUDGET,
        }
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub(super) async fn run<T, E>(
        &self,
        phase: &'static str,
        operation: impl Future<Output = Result<T, E>>,
    ) -> Result<T, ReadinessError> {
        let result = match timeout_at(self.deadline, operation).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(ReadinessError::OperationFailed(phase)),
            Err(_) => Err(ReadinessError::TimedOut(phase)),
        };
        if let Err(error) = &result {
            eprintln!(
                "MongoDB test readiness: {error} after {:.3}s",
                self.elapsed().as_secs_f64(),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn write_probe_can_recover_after_the_old_five_second_limit() {
        let readiness = Readiness::new();
        let result = readiness
            .run("write probe", async {
                tokio::time::sleep(Duration::from_secs(6)).await;
                Ok::<_, ()>(())
            })
            .await;
        assert_eq!(result, Ok(()));
        assert_eq!(readiness.started_at.elapsed(), Duration::from_secs(6));
    }

    #[tokio::test(start_paused = true)]
    async fn initial_lease_can_recover_after_the_old_five_second_limit() {
        let readiness = Readiness::new();
        let result = readiness
            .run("run lease", async {
                tokio::time::sleep(Duration::from_secs(6)).await;
                Ok::<_, ()>(true)
            })
            .await;
        assert_eq!(result, Ok(true));
        assert_eq!(readiness.started_at.elapsed(), Duration::from_secs(6));
    }

    #[tokio::test(start_paused = true)]
    async fn rejected_lease_remains_rejected() {
        let result = Readiness::new()
            .run("run lease", async { Ok::<_, ()>(false) })
            .await;
        assert_eq!(result, Ok(false));
    }

    #[tokio::test(start_paused = true)]
    async fn unresponsive_operation_fails_at_the_deadline_with_its_phase() {
        let readiness = Readiness::new();
        let result = readiness
            .run("write probe", std::future::pending::<Result<(), ()>>())
            .await;
        assert_eq!(result, Err(ReadinessError::TimedOut("write probe")));
        assert_eq!(readiness.started_at.elapsed(), READINESS_BUDGET);
    }

    #[tokio::test(start_paused = true)]
    async fn earlier_stages_consume_the_same_deadline() {
        let readiness = Readiness::new();
        readiness
            .run("write probe", async {
                tokio::time::sleep(Duration::from_secs(80)).await;
                Ok::<_, ()>(())
            })
            .await
            .unwrap();
        let result = readiness
            .run("run lease", async {
                tokio::time::sleep(Duration::from_secs(11)).await;
                Ok::<_, ()>(true)
            })
            .await;
        assert_eq!(result, Err(ReadinessError::TimedOut("run lease")));
        assert_eq!(readiness.started_at.elapsed(), READINESS_BUDGET);
    }

    #[tokio::test(start_paused = true)]
    async fn operation_error_fails_immediately_without_retrying() {
        let readiness = Readiness::new();
        let mut attempts = 0;
        let result = readiness
            .run("ping", async {
                attempts += 1;
                Err::<(), _>("private driver diagnostic")
            })
            .await;
        assert_eq!(result, Err(ReadinessError::OperationFailed("ping")));
        assert_eq!(
            result.unwrap_err().to_string(),
            "ping returned a MongoDB error"
        );
        assert_eq!(attempts, 1);
        assert_eq!(readiness.started_at.elapsed(), Duration::ZERO);
    }
}
