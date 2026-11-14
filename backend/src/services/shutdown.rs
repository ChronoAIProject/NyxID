//! Process-local work accounting. Draining stops pullers, never request admission.
pub mod drain;
#[cfg(test)]
mod inventory_tests;
pub mod release;
pub mod signals;
#[cfg(test)]
mod tests;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde::Serialize;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::models::assistant_conversation::{AssistantConversation, TurnOrigin};

/// Allow load balancer readiness propagation before reconnectable observers leave.
pub const SUBSCRIPTION_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Response extension set only by handlers with a durable resume/resync contract.
#[derive(Clone, Copy)]
pub struct RecoverableSubscription;

static PROCESS: LazyLock<Runtime> = LazyLock::new(Runtime::default);

pub fn process() -> Runtime {
    PROCESS.clone()
}

#[derive(Clone, Default)]
pub struct Runtime(Arc<Inner>);

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    requests: AtomicUsize,
    streams: AtomicUsize,
    subscriptions: AtomicUsize,
    changed: Notify,
    intake_stopped: CancellationToken,
    interrupt: CancellationToken,
    stop: CancellationToken,
}

#[derive(Default)]
struct State {
    draining: bool,
    drain_started: Option<tokio::time::Instant>,
    subscriptions_ended: u64,
    next_id: u64,
    work: BTreeMap<u64, Kind>,
    tasks: BTreeMap<u64, tokio::task::AbortHandle>,
    completed: u64,
    interrupted: Vec<String>,
    completed_ids: std::collections::BTreeSet<String>,
    // Retain exact candidates after an aborted runner drops its Work guard.
    shutdown_turns: BTreeMap<String, String>,
    releases: BTreeMap<u64, RetainedRelease>,
    released: BTreeMap<&'static str, u64>,
    claims_finished: BTreeMap<&'static str, u64>,
}

struct RetainedRelease {
    release: Release,
    abandoned_at: Option<tokio::time::Instant>,
}

#[derive(Clone)]
pub struct Release {
    /// False for claims guarding provider effects with an uncertain outcome.
    pub safe_after_stop: bool,
    pub db: mongodb::Database,
    pub collection: &'static str,
    pub filter: mongodb::bson::Document,
    pub update: mongodb::bson::Document,
}

/// Retained until the normal fenced release succeeds, including when its
/// owning future is dropped. Shutdown retries only the exact stored fence.
pub struct ReleaseTicket {
    runtime: Runtime,
    id: u64,
}

impl ReleaseTicket {
    pub fn completed(self) {
        let mut state = self.runtime.state();
        if let Some(entry) = state.releases.remove(&self.id)
            && state.draining
        {
            *state
                .claims_finished
                .entry(entry.release.collection)
                .or_default() += 1;
        }
    }

    pub fn update_filter(&self, filter: mongodb::bson::Document) {
        if let Some(release) = self.runtime.state().releases.get_mut(&self.id) {
            release.release.filter = filter;
        }
    }
}

impl Drop for ReleaseTicket {
    fn drop(&mut self) {
        if let Some(release) = self.runtime.state().releases.get_mut(&self.id) {
            release.abandoned_at = Some(tokio::time::Instant::now());
        }
    }
}

enum Kind {
    Detached,
    Cleanup,
    Background,
    Infrastructure,
    Upgrade,
    CompletionUpgrade,
    NodeSocket,
    CompletionWorker,
    Turn {
        id: String,
        conversation_id: String,
        origin: &'static str,
    },
}

#[derive(Clone, Default, Debug, Serialize)]
pub struct Counts {
    pub http_requests: usize,
    pub streams: usize,
    pub subscriptions: usize,
    pub detached: usize,
    pub background: usize,
    pub turns: BTreeMap<&'static str, usize>,
}

impl Counts {
    pub fn is_empty(&self) -> bool {
        self.http_requests
            + self.streams
            + self.detached
            + self.background
            + self.turns.values().sum::<usize>()
            == 0
    }
}

impl State {
    fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for kind in self.work.values() {
            match kind {
                Kind::Upgrade | Kind::CompletionUpgrade => counts.streams += 1,
                Kind::Detached | Kind::Cleanup => counts.detached += 1,
                Kind::Background => counts.background += 1,
                Kind::Infrastructure | Kind::NodeSocket | Kind::CompletionWorker => {}
                Kind::Turn { origin, .. } => *counts.turns.entry(origin).or_default() += 1,
            }
        }
        counts
    }
}

impl Runtime {
    pub fn record_release(&self, kind: &'static str, count: u64) {
        let mut state = self.state();
        if state.draining && count > 0 {
            *state.released.entry(kind).or_default() += count;
        }
    }

    pub fn released(&self) -> BTreeMap<&'static str, u64> {
        self.state().released.clone()
    }

    pub fn claims_finished(&self) -> BTreeMap<&'static str, u64> {
        self.state().claims_finished.clone()
    }

    pub fn register_release(&self, release: Release) -> ReleaseTicket {
        let mut state = self.state();
        // These claim TTLs are at most fifteen minutes. Retain failed cleanup
        // for an hour, without accumulating abandoned fences for process life.
        state.releases.retain(|_, entry| {
            entry
                .abandoned_at
                .is_none_or(|at| at.elapsed() < std::time::Duration::from_secs(3600))
        });
        let id = state.next_id;
        state.next_id += 1;
        state.releases.insert(
            id,
            RetainedRelease {
                release,
                abandoned_at: None,
            },
        );
        ReleaseTicket {
            runtime: self.clone(),
            id,
        }
    }

    pub fn releases(&self) -> Vec<Release> {
        self.state()
            .releases
            .values()
            .map(|entry| entry.release.clone())
            .collect()
    }

    pub fn record_settlement(&self, id: &str, interrupted: bool) {
        let mut state = self.state();
        if !state.draining || !state.completed_ids.insert(id.to_owned()) {
            return;
        }
        state.shutdown_turns.remove(id);
        if interrupted {
            if !state.interrupted.iter().any(|turn| turn == id) {
                state.interrupted.push(id.to_owned());
            }
        } else {
            state.interrupted.retain(|turn| turn != id);
            state.completed += 1;
        }
    }
    fn state(&self) -> MutexGuard<'_, State> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn insert(&self, state: &mut State, kind: Kind) -> (Work, u64) {
        if state.draining
            && self.0.interrupt.is_cancelled()
            && let Kind::Turn { id, .. } = &kind
            && !state.completed_ids.contains(id)
            && !state.interrupted.contains(id)
        {
            state.interrupted.push(id.clone());
        }
        if state.draining
            && let Kind::Turn {
                id,
                conversation_id,
                ..
            } = &kind
        {
            state
                .shutdown_turns
                .insert(id.clone(), conversation_id.clone());
        }
        let id = state.next_id;
        state.next_id += 1;
        state.work.insert(id, kind);
        self.0.changed.notify_waiters();
        (
            Work {
                runtime: self.clone(),
                registration: Registration::Mapped(id),
            },
            id,
        )
    }

    fn register_work(&self, kind: Kind) -> Work {
        self.insert(&mut self.state(), kind).0
    }

    fn counter(&self, kind: Counter) -> &AtomicUsize {
        match kind {
            Counter::Request => &self.0.requests,
            Counter::Stream => &self.0.streams,
            Counter::Subscription => &self.0.subscriptions,
        }
    }

    fn register_counted(&self, kind: Counter) -> Work {
        // These guards need no abort handle or turn identity. Keep the HTTP
        // handoff off the task map; register the body before dropping its request.
        if self.counter(kind).fetch_add(1, Ordering::SeqCst) == 0 {
            self.0.changed.notify_waiters();
        }
        Work {
            runtime: self.clone(),
            registration: Registration::Counted(kind),
        }
    }

    fn snapshot(&self, state: &State) -> Counts {
        let mut counts = state.counts();
        counts.http_requests = self.0.requests.load(Ordering::SeqCst);
        counts.streams += self.0.streams.load(Ordering::SeqCst);
        counts.subscriptions = self.0.subscriptions.load(Ordering::SeqCst);
        counts
    }

    /// Serialized with background admission, so the start snapshot includes
    /// every pull which was admitted before the drain began.
    pub fn begin_drain(&self) -> Option<Counts> {
        let mut state = self.state();
        if state.draining {
            return None;
        }
        state.draining = true;
        state.drain_started = Some(tokio::time::Instant::now());
        state.shutdown_turns = state
            .work
            .values()
            .filter_map(|kind| match kind {
                Kind::Turn {
                    id,
                    conversation_id,
                    ..
                } => Some((id.clone(), conversation_id.clone())),
                _ => None,
            })
            .collect();
        let counts = self.snapshot(&state);
        self.0.intake_stopped.cancel();
        Some(counts)
    }

    pub fn is_draining(&self) -> bool {
        self.0.intake_stopped.is_cancelled()
    }

    pub async fn intake_stopped(&self) {
        self.0.intake_stopped.cancelled().await;
    }

    /// Call immediately before a background claim, retaining the guard through
    /// its effects and release. A request-driven operation uses `work` instead.
    pub fn background(&self) -> Option<Work> {
        let mut state = self.state();
        if state.draining {
            None
        } else {
            Some(self.insert(&mut state, Kind::Background).0)
        }
    }

    pub async fn tick(&self, interval: &mut tokio::time::Interval) -> Option<Work> {
        tokio::select! {
            biased;
            () = self.intake_stopped() => None,
            _ = interval.tick() => self.background(),
        }
    }

    pub fn work(&self) -> Work {
        self.register_work(Kind::Detached)
    }

    pub fn completion_work(&self) -> Work {
        self.register_work(Kind::Cleanup)
    }

    pub fn request(&self) -> Work {
        self.register_counted(Counter::Request)
    }

    pub fn stream(&self) -> Work {
        self.register_counted(Counter::Stream)
    }

    pub fn subscription(&self) -> Work {
        self.register_counted(Counter::Subscription)
    }

    pub async fn subscription_close(&self) {
        self.intake_stopped().await;
        let deadline = self
            .state()
            .drain_started
            .map(|at| at + SUBSCRIPTION_DRAIN_GRACE);
        if let Some(deadline) = deadline {
            tokio::time::sleep_until(deadline).await;
        }
    }

    pub async fn subscriptions_finished(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.counts().subscriptions == 0 {
                return;
            }
            changed.await;
        }
    }

    pub fn subscriptions_ended(&self) -> u64 {
        self.state().subscriptions_ended
    }

    pub fn node_socket(&self) -> Work {
        self.register_work(Kind::NodeSocket)
    }

    pub async fn node_sockets_closed(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self
                .state()
                .work
                .values()
                .any(|kind| matches!(kind, Kind::NodeSocket))
            {
                return;
            }
            changed.await;
        }
    }

    pub fn turn(&self, row: &AssistantConversation) -> Option<Work> {
        let turn = row.active_turn.as_ref()?;
        let origin = match turn.origin {
            TurnOrigin::User => "user",
            TurnOrigin::Orchestrator => "orchestrator",
            TurnOrigin::Event => "event",
            TurnOrigin::Channel => "channel",
            TurnOrigin::Group => "group",
            TurnOrigin::Trigger => "trigger",
        };
        Some(self.register_work(Kind::Turn {
            id: turn.turn_id.clone(),
            conversation_id: row.id.clone(),
            origin,
        }))
    }

    /// Registration happens before spawn, not on the child's first poll.
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_tracked(Kind::Detached, future)
    }

    /// Completion/compensation only: this work remains joinable after execution
    /// is cancelled. It must not admit another turn or retry an external effect.
    pub fn spawn_cleanup<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_tracked(Kind::Cleanup, future)
    }

    /// Infrastructure is kept alive during the wait, but joined before release.
    /// Its individual claimed passes are counted by `background` / `tick`.
    pub fn spawn_worker<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_tracked(Kind::Infrastructure, future)
    }

    /// A completion queue stays available to cancelled producers' cleanup.
    /// Each queued item must own a `completion_work` guard through persistence.
    pub fn spawn_completion_worker<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_tracked(Kind::CompletionWorker, future)
    }

    fn spawn_tracked<F>(&self, kind: Kind, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (work, id) = self.insert(&mut self.state(), kind);
        let future = Box::pin(future);
        let task = tokio::spawn(async move {
            let _work = work;
            future.await
        });
        self.register_task(id, task.abort_handle());
        task
    }

    /// JoinSet still owns cancellation; the registry also observes when an
    /// aborted child's future has actually dropped before ownership release.
    pub fn spawn_joined<F>(&self, set: &mut tokio::task::JoinSet<F::Output>, future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_joined_kind(set, Kind::Detached, future);
    }

    pub fn spawn_worker_joined<F>(&self, set: &mut tokio::task::JoinSet<F::Output>, future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn_joined_kind(set, Kind::Infrastructure, future);
    }

    fn spawn_joined_kind<F>(&self, set: &mut tokio::task::JoinSet<F::Output>, kind: Kind, future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (work, id) = self.insert(&mut self.state(), kind);
        let future = Box::pin(future);
        let task = set.spawn(async move {
            let _work = work;
            future.await
        });
        self.register_task(id, task);
    }

    fn register_task(&self, id: u64, task: tokio::task::AbortHandle) {
        let mut state = self.state();
        if state.work.contains_key(&id) {
            if self.0.stop.is_cancelled()
                && !matches!(
                    state.work.get(&id),
                    Some(Kind::Cleanup | Kind::CompletionWorker)
                )
            {
                task.abort();
            }
            state.tasks.insert(id, task);
        }
    }

    /// The guard is created before Axum hands the socket to its detached task.
    pub fn upgrade_callback<T, F, Fut>(
        self,
        callback: F,
    ) -> impl FnOnce(T) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static
    where
        T: Send + 'static,
        F: FnOnce(T) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let work = self.register_work(Kind::Upgrade);
        move |socket| {
            Box::pin(async move {
                let _work = work;
                tokio::select! {
                    biased;
                    () = self.stopped() => {},
                    () = callback(socket) => {},
                }
            })
        }
    }

    /// An upgraded transport with a completion tail stops its own IO on
    /// `stopped`, then persists measured usage before releasing this guard.
    pub fn upgrade_completion_callback<T, F, Fut>(
        self,
        callback: F,
    ) -> impl FnOnce(T) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static
    where
        T: Send + 'static,
        F: FnOnce(T) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let work = self.register_work(Kind::CompletionUpgrade);
        move |socket| {
            Box::pin(async move {
                let _work = work;
                callback(socket).await;
            })
        }
    }

    pub fn stop(&self) {
        self.0.stop.cancel();
        let state = self.state();
        for (id, task) in &state.tasks {
            if !matches!(
                state.work.get(id),
                Some(Kind::Cleanup | Kind::CompletionWorker)
            ) {
                task.abort();
            }
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.0.stop.is_cancelled()
    }

    pub async fn stopped(&self) {
        self.0.stop.cancelled().await;
    }

    pub fn stop_token(&self) -> CancellationToken {
        self.0.stop.clone()
    }

    pub async fn tasks_stopped(&self) {
        self.wait_stopped(false).await;
    }

    pub async fn close_completion_workers(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let empty = {
                let state = self.state();
                let mut empty = true;
                for (id, task) in &state.tasks {
                    if matches!(state.work.get(id), Some(Kind::CompletionWorker)) {
                        empty = false;
                        task.abort();
                    }
                }
                empty
            };
            if empty {
                return;
            }
            changed.await;
        }
    }

    pub async fn execution_stopped(&self) {
        self.wait_stopped(true).await;
    }

    async fn wait_stopped(&self, allow_cleanup: bool) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let stopped = {
                let state = self.state();
                state.tasks.keys().all(|id| {
                    matches!(state.work.get(id), Some(Kind::CompletionWorker))
                        || (allow_cleanup && matches!(state.work.get(id), Some(Kind::Cleanup)))
                }) && !state.work.values().any(|kind| {
                    matches!(kind, Kind::Upgrade)
                        || (!allow_cleanup
                            && matches!(kind, Kind::Cleanup | Kind::CompletionUpgrade))
                }) && self.0.requests.load(Ordering::SeqCst) == 0
                    && (allow_cleanup
                        || (self.0.streams.load(Ordering::SeqCst) == 0
                            && self.0.subscriptions.load(Ordering::SeqCst) == 0))
            };
            if stopped {
                return;
            }
            changed.await;
        }
    }

    pub fn counts(&self) -> Counts {
        self.snapshot(&self.state())
    }

    pub fn tracked_units(&self) -> usize {
        self.state().work.len()
            + self.0.requests.load(Ordering::SeqCst)
            + self.0.streams.load(Ordering::SeqCst)
            + self.0.subscriptions.load(Ordering::SeqCst)
    }

    pub async fn idle(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.counts().is_empty() {
                return;
            }
            changed.await;
        }
    }

    /// An accepted subscription still needs serving during its reconnect grace,
    /// even though it is excluded from the in-flight work/idle count.
    pub async fn serving_needed(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let counts = self.counts();
            if !counts.is_empty() || counts.subscriptions > 0 {
                return;
            }
            changed.await;
        }
    }

    pub fn interrupt(&self) {
        let mut state = self.state();
        if state.draining {
            // Settlement can fail after execution stops. Keep those IDs in
            // the drain audit; an ordinary settlement winning the race removes
            // its ID through record_settlement instead.
            let interrupted: std::collections::BTreeSet<_> = state
                .work
                .values()
                .filter_map(|kind| match kind {
                    Kind::Turn { id, .. }
                        if !state.completed_ids.contains(id) && !state.interrupted.contains(id) =>
                    {
                        Some(id.clone())
                    }
                    _ => None,
                })
                .collect();
            state.interrupted.extend(interrupted);
        }
        self.0.interrupt.cancel();
    }

    pub async fn interrupted(&self) {
        self.0.interrupt.cancelled().await;
    }

    pub fn shutdown_turns(&self) -> BTreeMap<String, String> {
        self.state().shutdown_turns.clone()
    }

    pub fn outcomes(&self) -> (u64, Vec<String>) {
        let state = self.state();
        (state.completed, state.interrupted.clone())
    }
}

#[derive(Clone, Copy)]
enum Counter {
    Request,
    Stream,
    Subscription,
}

enum Registration {
    Mapped(u64),
    Counted(Counter),
}

pub struct Work {
    runtime: Runtime,
    registration: Registration,
}

impl Work {
    pub fn subscription_ended(&self) {
        if matches!(
            self.registration,
            Registration::Counted(Counter::Subscription)
        ) {
            self.runtime.state().subscriptions_ended += 1;
        }
    }

    pub fn settled(&self, interrupted: bool) {
        let Registration::Mapped(registration) = self.registration else {
            return;
        };
        let state = self.runtime.state();
        let Some(Kind::Turn { id, .. }) = state.work.get(&registration) else {
            return;
        };
        let id = id.clone();
        drop(state);
        self.runtime.record_settlement(&id, interrupted);
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        match self.registration {
            Registration::Mapped(id) => {
                let mut state = self.runtime.state();
                state.work.remove(&id);
                state.tasks.remove(&id);
            }
            Registration::Counted(kind) => {
                if self.runtime.counter(kind).fetch_sub(1, Ordering::SeqCst) != 1 {
                    return;
                }
            }
        }
        self.runtime.0.changed.notify_waiters();
    }
}
