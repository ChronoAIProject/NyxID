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
// Above half the default cleanup reserve, so that half is the effective bound.
const START_AUDIT_TIMEOUT: Duration = Duration::from_secs(3);

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
    pub signal: &'static str,
    pub timeout: Duration,
    audit_attempted: std::sync::atomic::AtomicBool,
}

impl Drain {
    pub fn cleanup_reserve(&self) -> Duration {
        (self.timeout / CLEANUP_RESERVE_DIVISOR).min(MAX_CLEANUP_RESERVE)
    }

    pub fn work_deadline(&self) -> Instant {
        self.deadline - self.cleanup_reserve()
    }

    /// Zero does not change readiness or stop background intake.
    pub fn begin(state: &AppState, timeout: Duration, signal: &'static str) -> Option<Self> {
        if timeout.is_zero() {
            return None;
        }
        let runtime = &state.shutdown;
        let counts = runtime.begin_drain()?;
        let started = Instant::now();
        tracing::info!(target: "nyxid::shutdown", signal, budget_secs = timeout.as_secs(),
            in_flight = ?counts, "Backend drain started; a second signal skips the wait and exits immediately after bounded cleanup");
        let drain = Self {
            runtime: runtime.clone(),
            deadline: started
                .checked_add(timeout)
                .unwrap_or_else(|| started + Duration::from_secs(u32::MAX.into())),
            started,
            started_at: Utc::now(),
            counts,
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
            "node_operations_at_start": state.node_ws_manager.in_flight_operations(),
        });
        // Fit the completion join even when a second signal starts cleanup immediately.
        let audit_deadline = started + START_AUDIT_TIMEOUT.min(drain.cleanup_reserve() / 2);
        let db = state.db.clone();
        state.shutdown.spawn_cleanup(async move {
            if !matches!(
                tokio::time::timeout_at(
                    audit_deadline,
                    crate::services::audit_service::log_system_event(
                        db,
                        "backend_shutdown_drain_started",
                        Some(metadata),
                    ),
                )
                .await,
                Ok(Ok(()))
            ) {
                tracing::warn!(target: "nyxid::shutdown", "Shutdown start audit unavailable or timed out");
            }
        });
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
        if !matches!(
            tokio::time::timeout_at(
                audit_deadline,
                crate::services::audit_service::log_system_event(
                    state.db.clone(),
                    "backend_shutdown_drain",
                    Some(metadata)
                )
            )
            .await,
            Ok(Ok(()))
        ) {
            tracing::warn!(target: "nyxid::shutdown", "Shutdown audit unavailable");
        }
    }
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
