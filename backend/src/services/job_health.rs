//! Process-local health of the scheduled workers on this replica.
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    cell::Cell,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::{Instant, Interval};

use crate::config::AppConfig;
use crate::services::shutdown::{Runtime, Work};

const HEARTBEAT: Duration = Duration::from_secs(15);
const STALE: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct Job(Arc<Mutex<Observation>>);

struct Observation {
    enabled: bool,
    since: Instant,
    heartbeat_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
    failed: bool,
}

#[derive(Serialize)]
pub(crate) struct JobStatus {
    status: &'static str,
    heartbeat_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
}

impl Job {
    pub(crate) fn new(enabled: bool) -> Self {
        Self(Arc::new(Mutex::new(Observation {
            enabled,
            since: Instant::now(),
            heartbeat_at: None,
            last_success_at: None,
            last_failure_at: None,
            failed: false,
        })))
    }

    fn record(&self, success: Option<bool>) {
        let mut entry = self.0.lock().unwrap();
        if !entry.enabled {
            return;
        }
        let now = Utc::now();
        entry.since = Instant::now();
        entry.heartbeat_at = Some(now);
        match success {
            Some(true) => entry.last_success_at = Some(now),
            Some(false) => entry.last_failure_at = Some(now),
            None => return,
        }
        entry.failed = success == Some(false);
    }

    pub(crate) fn progress(&self) {
        self.record(None);
    }

    pub(crate) fn finish(&self, success: bool) {
        self.record(Some(success));
    }

    pub(crate) async fn tick(&self, runtime: &Runtime, interval: &mut Interval) -> Option<Work> {
        self.wait(runtime.tick(interval)).await
    }

    // Only for intentional waits with an existing deadline. The general pass
    // observer below never supplies timer-based progress to active work.
    async fn wait<F: Future>(&self, wait: F) -> F::Output {
        tokio::pin!(wait);
        loop {
            self.progress();
            tokio::select! {
                biased;
                result = &mut wait => return result,
                () = tokio::time::sleep(HEARTBEAT) => {},
            }
        }
    }

    pub(crate) async fn observe<T, E>(
        &self,
        pass: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        ACTIVE
            .scope(
                Pass {
                    job: self.clone(),
                    outcome: Cell::new(Outcome::Success),
                },
                async {
                    let result = pass.await;
                    match ACTIVE.with(|pass| pass.outcome.get()) {
                        _ if result.is_err() => self.finish(false),
                        Outcome::Failed => self.finish(false),
                        Outcome::Success => self.finish(true),
                        Outcome::Skipped => {
                            let mut entry = self.0.lock().unwrap();
                            entry.failed = false;
                        }
                    }
                    result
                },
            )
            .await
    }

    fn status(&self) -> JobStatus {
        let entry = self.0.lock().unwrap();
        let status = if !entry.enabled {
            "disabled"
        } else if entry.since.elapsed() >= STALE {
            "stale"
        } else if entry.failed {
            "failed"
        } else if entry.heartbeat_at.is_none() {
            "starting"
        } else {
            "live"
        };
        JobStatus {
            status,
            heartbeat_at: entry.heartbeat_at,
            last_success_at: entry.last_success_at,
            last_failure_at: entry.last_failure_at,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Success,
    Failed,
    Skipped,
}

struct Pass {
    job: Job,
    outcome: Cell<Outcome>,
}

tokio::task_local! {
    // Manual scans and request-path calls to shared services cannot update a worker.
    static ACTIVE: Pass;
}

pub(crate) async fn bounded_wait<F: Future>(
    duration: Duration,
    operation: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    let wait = tokio::time::timeout(duration, operation);
    match ACTIVE.try_with(|pass| pass.job.clone()) {
        Ok(job) => {
            let result = job.wait(wait).await;
            if result.is_err() {
                checkpoint(false);
            }
            result
        }
        Err(_) => wait.await,
    }
}

pub(crate) fn skipped() {
    let _ = ACTIVE.try_with(|pass| pass.outcome.set(Outcome::Skipped));
}

pub(crate) fn checkpoint(ok: bool) {
    let _ = ACTIVE.try_with(|pass| {
        if ok {
            pass.job.progress();
        } else {
            pass.outcome.set(Outcome::Failed);
            pass.job.finish(false);
        }
    });
}

pub(crate) struct Jobs {
    pub(crate) turn_sweep: Job,
    pub(crate) schedules: Job,
    pub(crate) channel_polls: Job,
    pub(crate) billing_reconcile: Job,
    pub(crate) chain_verify: Job,
}

impl Jobs {
    pub(crate) fn new(config: &AppConfig) -> Self {
        Self {
            turn_sweep: Job::new(true),
            schedules: Job::new(true),
            channel_polls: Job::new(config.channel_poll_interval_secs > 0),
            billing_reconcile: Job::new(config.billing_reconcile_interval_secs > 0),
            chain_verify: Job::new(config.chain_verify_interval_secs > 0),
        }
    }

    pub(crate) fn snapshot(&self) -> std::collections::BTreeMap<&'static str, JobStatus> {
        [
            ("turn_sweep", &self.turn_sweep),
            ("schedules", &self.schedules),
            ("channel_polls", &self.channel_polls),
            ("billing_reconcile", &self.billing_reconcile),
            ("chain_verify", &self.chain_verify),
        ]
        .map(|(name, job)| (name, job.status()))
        .into()
    }
}

#[cfg(test)]
mod tests;
