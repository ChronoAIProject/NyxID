use std::{collections::BTreeMap, future::Future, time::Duration};

use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::doc;
use serde::Serialize;
use tokio::time::Instant;

use crate::{AppState, errors::AppResult, models::assistant_conversation::AssistantConversation};

use super::{Counts, Runtime};

const CLEANUP_RESERVE_DIVISOR: u32 = 10;
const MAX_CLEANUP_RESERVE: Duration = Duration::from_secs(5);
pub(crate) const TURN_OFFER_MARGIN: Duration = Duration::from_secs(3);
// Above half the default cleanup reserve, so that half is the effective bound.
const SNAPSHOT_AUDIT_TIMEOUT: Duration = Duration::from_secs(3);

type ReleaseFuture<'a> = std::pin::Pin<Box<dyn Future<Output = AppResult<u64>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Idle,
    Deadline,
    SecondSignal,
    ListenerFailure,
}

pub struct Drain {
    pub runtime: Runtime,
    pub deadline: Instant,
    pub started: Instant,
    pub started_at: DateTime<Utc>,
    pub counts: Counts,
    open_http: super::diagnostics::OpenHttp,
    progress_stopped: tokio_util::sync::CancellationToken,
    pub signal: &'static str,
    pub timeout: Duration,
    audit_attempted: std::sync::atomic::AtomicBool,
}

impl Drain {
    /// Polled alongside the coordinator, including listener handover and cleanup.
    /// No work guard: diagnostics must never turn an idle drain into a busy one.
    pub async fn progress(&self, state: &AppState) -> std::convert::Infallible {
        let records = async {
            for elapsed in [10, 25, 60, 300] {
                let offset = Duration::from_secs(elapsed);
                if offset >= self.timeout {
                    break;
                }
                tokio::time::sleep_until(self.started + offset).await;
                if Instant::now() >= self.deadline {
                    break;
                }
                let metadata = serde_json::json!({
                    "generation": state.replica_identity.generation_id,
                    "signal": self.signal,
                    "started_at": self.started_at,
                    "budget_secs": self.timeout.as_secs(),
                    "elapsed_secs": self.started.elapsed().as_secs(),
                    "checkpoint_secs": elapsed,
                    "in_flight": self.runtime.counts(),
                    "node_operations": state.node_ws_manager.in_flight_operations(),
                    "open_http": self.runtime.open_http(),
                    "recent_traffic": self.runtime.recent_traffic(),
                });
                tracing::info!(target: "nyxid::shutdown", event = "backend_shutdown_drain_progress", metadata = %metadata, "Backend drain still running");
                snapshot_audit(
                    state.db.clone(),
                    "backend_shutdown_drain_progress",
                    metadata,
                    self.deadline
                        .min(Instant::now() + self.snapshot_audit_timeout()),
                )
                .await;
            }
            std::future::pending::<()>().await;
        };
        tokio::select! {
            biased;
            () = self.progress_stopped.cancelled() => {},
            _ = async {
                tokio::join!(records, async {
                    tokio::time::sleep_until(self.turn_offer_at()).await;
                    self.runtime.0.turn_offer_due.cancel();
                })
            } => {},
        }
        std::future::pending().await
    }

    fn snapshot_audit_timeout(&self) -> Duration {
        SNAPSHOT_AUDIT_TIMEOUT.min(self.cleanup_reserve() / 2)
    }

    pub fn cleanup_reserve(&self) -> Duration {
        (self.timeout / CLEANUP_RESERVE_DIVISOR).min(MAX_CLEANUP_RESERVE)
    }

    pub fn work_deadline(&self) -> Instant {
        self.deadline - self.cleanup_reserve()
    }

    fn turn_offer_at(&self) -> Instant {
        self.started + (self.timeout - self.cleanup_reserve()).saturating_sub(TURN_OFFER_MARGIN)
    }

    /// Zero does not change readiness or stop background intake.
    pub fn begin(state: &AppState, timeout: Duration, signal: &'static str) -> Option<Self> {
        Self::begin_at(state, timeout, signal, None)
    }

    /// Startup has already spent part (or all) of the signal's exit budget.
    pub fn begin_startup(
        state: &AppState,
        timeout: Duration,
        received: super::signals::Received,
    ) -> Option<Self> {
        Self::begin_at(state, timeout, received.signal, Some(received))
    }

    fn begin_at(
        state: &AppState,
        timeout: Duration,
        signal: &'static str,
        received: Option<super::signals::Received>,
    ) -> Option<Self> {
        if timeout.is_zero() {
            return None;
        }
        let runtime = &state.shutdown;
        let counts = runtime.begin_drain()?;
        let observed = Instant::now();
        let started = received.map_or(observed, |received| received.at);
        let open_http = runtime.open_http();
        tracing::info!(target: "nyxid::shutdown", signal, budget_secs = timeout.as_secs(),
            in_flight = ?counts, open_http = ?open_http, "Backend drain started; a second signal skips the wait and exits immediately after bounded cleanup");
        let drain = Self {
            runtime: runtime.clone(),
            deadline: started
                .checked_add(timeout)
                .unwrap_or_else(|| started + Duration::from_secs(u32::MAX.into())),
            started,
            started_at: received.map_or_else(Utc::now, |received| received.at_utc),
            counts,
            open_http,
            progress_stopped: tokio_util::sync::CancellationToken::new(),
            signal,
            timeout,
            audit_attempted: std::sync::atomic::AtomicBool::new(false),
        };
        let commit = std::env::var("NYXID_GIT_HASH")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| env!("NYXID_GIT_HASH").to_owned());
        let metadata = serde_json::json!({
            "generation": state.replica_identity.generation_id,
            "instance_name": state.replica_identity.instance_name,
            "engine_worker_id": crate::services::assistant_engine_worker::instance().id,
            "build_commit": commit,
            "signal": drain.signal,
            "started_at": drain.started_at,
            "budget_secs": drain.timeout.as_secs(),
            "cleanup_reserve_ms": drain.cleanup_reserve().as_millis(),
            "in_flight_at_start": drain.counts,
            "open_http_at_start": drain.open_http,
            "node_operations_at_start": state.node_ws_manager.in_flight_operations(),
            "recent_traffic": runtime.recent_traffic(),
        });
        // Fit the completion join even when a second signal starts cleanup immediately.
        let audit_deadline = (observed + drain.snapshot_audit_timeout()).min(drain.deadline);
        state.shutdown.spawn_cleanup(snapshot_audit(
            state.db.clone(),
            "backend_shutdown_drain_started",
            metadata,
            audit_deadline,
        ));
        Some(drain)
    }

    pub fn second_signal(&self, signal: &str) -> End {
        tracing::info!(target: "nyxid::shutdown", signal, "Second shutdown signal received; skipping remaining drain wait");
        End::SecondSignal
    }

    pub async fn wait(
        &self,
        state: &AppState,
        work_deadline: Instant,
        second_signal: impl Future<Output = &'static str>,
    ) -> End {
        let idle = async {
            loop {
                self.runtime.idle().await;
                self.runtime.subscriptions_finished().await;
                // Requests remain accepted during the subscription grace.
                if !self.runtime.counts().is_empty() {
                    continue;
                }
                if state.node_ws_manager.in_flight_operations() == 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        tokio::select! {
            biased;
            signal = second_signal => self.second_signal(signal),
            () = tokio::time::sleep_until(work_deadline) => End::Deadline,
            () = idle => End::Idle,
        }
    }

    /// One awaited chained append; a failed or stuck audit never owns exit.
    pub async fn audit(
        &self,
        state: &AppState,
        end: End,
        mut leases_released: BTreeMap<&'static str, u64>,
        audit_deadline: Instant,
    ) {
        // Cancel progress before recording completion.
        self.progress_stopped.cancel();
        if self
            .audit_attempted
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let finished_at = Utc::now();
        let (completed, interrupted) = self.runtime.outcomes();
        for (kind, count) in self.runtime.released() {
            *leases_released.entry(kind).or_default() += count;
        }
        let deadline_reached = end == End::Deadline || Instant::now() >= self.deadline;
        let commit = std::env::var("NYXID_GIT_HASH")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| env!("NYXID_GIT_HASH").to_owned());
        let metadata = serde_json::json!({
            "generation": state.replica_identity.generation_id,
            "instance_name": state.replica_identity.instance_name,
            "engine_worker_id": crate::services::assistant_engine_worker::instance().id,
            "build_commit": commit,
            "signal": self.signal,
            "started_at": self.started_at,
            "finished_at": finished_at,
            "duration_ms": self.started.elapsed().as_millis(),
            "deadline_secs": self.timeout.as_secs(),
            "deadline_at": chrono::Duration::from_std(self.timeout).ok().and_then(|timeout| self.started_at.checked_add_signed(timeout)),
            "work_deadline_at": chrono::Duration::from_std(self.timeout - self.cleanup_reserve()).ok().and_then(|timeout| self.started_at.checked_add_signed(timeout)),
            "cleanup_reserve_ms": self.cleanup_reserve().as_millis(),
            "in_flight_at_start": self.counts,
            "open_http_at_start": self.open_http,
            "open_http_at_finish": self.runtime.open_http(),
            "recent_traffic": self.runtime.recent_traffic(),
            "in_flight_at_finish": self.runtime.counts(),
            "tracked_units_at_finish": self.runtime.tracked_units(),
            "subscriptions_ended": self.runtime.subscriptions_ended(),
            "turns_completed_during_drain": completed,
            "turns_interrupted": interrupted,
            "leases_released": leases_released,
            "claims_finished_during_drain": self.runtime.claims_finished(),
            "deadline_reached": deadline_reached,
            "end": end,
        });
        tracing::info!(target: "nyxid::shutdown", event = "backend_shutdown_drain", metadata = %metadata, "Backend shutdown drain finished");
        snapshot_audit(
            state.db.clone(),
            "backend_shutdown_drain",
            metadata,
            audit_deadline,
        )
        .await;
    }
}

async fn snapshot_audit(
    db: mongodb::Database,
    event: &'static str,
    metadata: serde_json::Value,
    deadline: Instant,
) {
    let append =
        crate::services::audit_service::log_system_event(db, event, Some(metadata.clone()));
    let audit_outcome = match tokio::time::timeout_at(deadline, append).await {
        Ok(Ok(())) => return,
        Ok(Err(_)) => "failed",
        Err(_) => "timed_out",
    };
    tracing::warn!(target: "nyxid::shutdown", event, audit_outcome, metadata = %metadata, "Shutdown audit unavailable");
}

pub(super) fn shutdown_candidates_filter() -> mongodb::bson::Document {
    doc! {"active_turn.heartbeat_at": {"$type": "date"}}
}

pub(super) fn shutdown_candidates_index() -> mongodb::bson::Document {
    doc! {"active_turn.heartbeat_at": 1, "_id": 1}
}

/// Run only after cancelling and joining local execution. Both the query and
/// settlement transaction check the engine worker, independently of the
/// cluster generation used to release leases.
pub async fn settle_remaining(
    state: &AppState,
) -> AppResult<
    Vec<(
        AssistantConversation,
        crate::services::assistant_nyxagent::TurnError,
    )>,
> {
    let worker = crate::services::assistant_engine_worker::instance();
    let conversations = state.db.collection::<AssistantConversation>(
        crate::models::assistant_conversation::COLLECTION_NAME,
    );
    let ids: Vec<_> = state.shutdown.shutdown_turns().into_values().collect();
    let mut rows = BTreeMap::new();
    if !ids.is_empty() {
        let mut cursor = conversations.find(doc! {"_id": {"$in": ids}}).await?;
        while let Some(row) = cursor.try_next().await? {
            rows.insert(row.id.clone(), row);
        }
    }
    // Covers a committed claim whose request never reached runner registration.
    // Date bounds on the existing heartbeat index exclude inactive/null rows;
    // worker identity is checked in memory and again by the settlement CAS.
    let mut cursor = conversations
        .find(shutdown_candidates_filter())
        .hint(mongodb::options::Hint::Keys(shutdown_candidates_index()))
        .await?;
    while let Some(row) = cursor.try_next().await? {
        if row
            .active_turn
            .as_ref()
            .and_then(|turn| turn.worker_instance.as_ref())
            .is_some_and(|owner| owner.id == worker.id)
        {
            rows.insert(row.id.clone(), row);
        }
    }
    let mut pending = futures::stream::iter(rows.into_values())
        .map(|row| async move {
            let result = crate::services::assistant_nyxagent::settle_shutdown_turn(
                &state.db, &row, &worker.id,
            )
            .await;
            if matches!(&result, Ok(Some(_)))
                && row
                    .active_turn
                    .as_ref()
                    .is_some_and(|turn| turn.stop_requested && turn.restart_admission.is_some())
            {
                crate::handlers::assistant_nyxagent::survivor::cancel(state, &row).await;
            }
            (row, result)
        })
        .buffer_unordered(16);
    let mut settled = Vec::new();
    while let Some((row, result)) = pending.next().await {
        match result {
            Ok(Some(error)) => {
                if let Some(turn) = &row.active_turn {
                    state
                        .shutdown
                        .record_settlement(&turn.turn_id, error.stage == "shutdown");
                }
                settled.push((row, error));
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(code = error.error_code(), "Shutdown turn settlement failed")
            }
        }
    }
    Ok(settled)
}

/// All release futures share one deadline, so N leases cannot multiply exit
/// latency. Call after listeners are closed and execution has stopped.
pub async fn release_owned(state: &AppState, deadline: Instant) -> BTreeMap<&'static str, u64> {
    let identity = &state.replica_identity;
    let releases: Vec<(&str, ReleaseFuture<'_>)> = vec![
        (
            "cluster",
            Box::pin(super::release::cluster(&state.db, identity)),
        ),
        (
            "capacity_slots",
            Box::pin(super::release::slots(&state.db, identity)),
        ),
        (
            "claimed_work",
            Box::pin(super::release::registered(&state.shutdown)),
        ),
    ];
    futures::stream::iter(releases)
        .map(|(kind, release)| async move {
            let count = match tokio::time::timeout_at(deadline, release).await {
                Ok(Ok(count)) => count,
                _ => {
                    tracing::warn!(kind, "Shutdown ownership release unavailable");
                    0
                }
            };
            (kind, count)
        })
        .buffer_unordered(5)
        .collect()
        .await
}

#[cfg(test)]
mod startup_budget_tests {
    use super::*;
    use crate::services::shutdown::signals::Received;

    #[tokio::test(start_paused = true)]
    async fn turn_offer_uses_original_work_deadline_and_saturates_for_small_budgets() {
        for (budget, expected_ms) in [(25, 19_500), (2, 0)] {
            let state = crate::test_utils::test_app_state_no_db().await;
            let drain = Drain::begin(&state, Duration::from_secs(budget), "SIGTERM").unwrap();
            assert_eq!(
                drain.turn_offer_at() - drain.started,
                Duration::from_millis(expected_ms)
            );
            let progress = drain.progress(&state);
            tokio::pin!(progress);
            tokio::select! {
                biased;
                never = &mut progress => match never {},
                () = tokio::time::sleep_until(drain.turn_offer_at()) => {},
            }
            tokio::select! {
                biased;
                never = &mut progress => match never {},
                () = state.shutdown.turn_offer_due() => {},
                () = tokio::time::sleep(Duration::from_millis(1)) => panic!("turn offer was late"),
            }
            state.shutdown.stop();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn startup_signal_budget_includes_completed_startup_steps() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let received = Received {
            signal: "SIGTERM",
            at: Instant::now(),
            at_utc: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        };
        tokio::time::advance(Duration::from_secs(8)).await;
        let drain = Drain::begin_startup(&state, Duration::from_secs(25), received).unwrap();
        assert_eq!(drain.started, received.at);
        assert_eq!(drain.started_at, received.at_utc);
        assert_eq!(drain.deadline - Instant::now(), Duration::from_secs(17));
        assert_eq!(
            drain.work_deadline() - Instant::now(),
            Duration::from_millis(14_500)
        );
        state.shutdown.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn startup_signal_expired_budget_does_not_open_another_work_wait() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let received = Received {
            signal: "SIGTERM",
            at: Instant::now(),
            at_utc: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        };
        tokio::time::advance(Duration::from_secs(30)).await;
        let drain = Drain::begin_startup(&state, Duration::from_secs(25), received).unwrap();
        let now = Instant::now();
        assert_eq!(
            drain.deadline.saturating_duration_since(now),
            Duration::ZERO
        );
        let end = drain
            .wait(&state, drain.work_deadline(), std::future::pending())
            .await;
        assert_eq!(end, End::Deadline);
        assert_eq!(Instant::now(), now);
        state.shutdown.stop();
    }
}
