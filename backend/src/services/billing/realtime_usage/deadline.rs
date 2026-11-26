use std::{
    future::{Future, poll_fn},
    task::Poll,
    time::{Duration, Instant, SystemTime},
};

/// Monotonic time covers a paused process; wall time also covers host suspend.
/// Capture before the DB marker so the local stop cannot follow its deadline.
#[derive(Clone, Debug)]
pub(crate) struct BridgeDeadline {
    started: Instant,
    wall: SystemTime,
    duration: Duration,
}
impl BridgeDeadline {
    pub(crate) fn new(duration: Duration) -> Self {
        Self {
            started: Instant::now(),
            wall: SystemTime::now(),
            duration,
        }
    }
    pub(crate) fn remaining(&self) -> Duration {
        let elapsed = self
            .started
            .elapsed()
            .max(self.wall.elapsed().unwrap_or_default());
        self.duration.saturating_sub(elapsed)
    }
    pub(crate) fn expired(&self) -> bool {
        self.remaining().is_zero()
    }
    pub(crate) async fn run<T>(&self, future: impl Future<Output = T>) -> Option<T> {
        run_with_remaining(|| self.remaining(), future).await
    }
    #[cfg(test)]
    pub(crate) fn age_for_test(&mut self, monotonic: Duration, wall: Duration) {
        self.started -= monotonic;
        self.wall -= wall;
    }
}

async fn run_with_remaining<T>(
    remaining: impl Fn() -> Duration,
    future: impl Future<Output = T>,
) -> Option<T> {
    tokio::pin!(future);
    let tick = tokio::time::sleep(remaining().min(Duration::from_secs(1)));
    tokio::pin!(tick);
    poll_fn(|cx| {
        // Check before every child poll, including a socket wake after host
        // suspend. An async select loop can resume the pending socket first.
        let left = remaining();
        if left.is_zero() {
            return Poll::Ready(None);
        }
        if tick.as_mut().poll(cx).is_ready() {
            tick.as_mut()
                .reset(tokio::time::Instant::now() + left.min(Duration::from_secs(1)));
            let _ = tick.as_mut().poll(cx);
        }
        future.as_mut().poll(cx).map(Some)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test]
    async fn realtime_ws_clock_jump_between_polls_cannot_resume_a_blocked_effect() {
        let expired = Cell::new(false);
        let polls = Cell::new(0);
        let future = poll_fn(|cx| {
            polls.set(polls.get() + 1);
            if polls.get() > 1 {
                return Poll::Ready("socket effect");
            }
            expired.set(true);
            cx.waker().wake_by_ref();
            Poll::Pending
        });
        let remaining = || {
            if expired.get() {
                Duration::ZERO
            } else {
                Duration::from_secs(3600)
            }
        };
        assert_eq!(run_with_remaining(remaining, future).await, None);
        assert_eq!(polls.get(), 1);
    }
}
