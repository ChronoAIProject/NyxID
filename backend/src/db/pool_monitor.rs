//! Content-free pool diagnostics. CMAP has no request id or cancellation event,
//! so this deliberately reports saturation, never an invented queue length.
use mongodb::{
    event::{EventHandler, cmap::CmapEvent},
    options::{ClientOptions, ServerAddress},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const SLOW: Duration = Duration::from_secs(1);
const LOG_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Pool {
    checked_out: HashSet<u32>,
    saturated_since: Option<Instant>,
    last_warning: Option<Instant>,
}

impl Pool {
    fn acquire(&mut self, id: u32, max: u32, now: Instant) {
        self.checked_out.insert(id);
        if max != 0 && self.checked_out.len() >= max as usize {
            self.saturated_since.get_or_insert(now);
        }
    }
    fn release(&mut self, id: u32, max: u32) {
        self.checked_out.remove(&id);
        if self.checked_out.len() < max as usize {
            self.saturated_since = None;
        }
    }
    fn warning(&mut self, now: Instant, duration: Duration) -> bool {
        if duration < SLOW
            || self
                .last_warning
                .is_some_and(|last| now.saturating_duration_since(last) < LOG_INTERVAL)
        {
            return false;
        }
        self.last_warning = Some(now);
        true
    }
    fn stalled(&mut self, now: Instant) -> Option<Duration> {
        let duration = now.saturating_duration_since(self.saturated_since?);
        self.warning(now, duration).then_some(duration)
    }
}

/// One monitor per client, with independent accounting for each server pool.
/// Dropping the client releases its handler; the weak timer then exits.
pub(super) fn install(options: &mut ClientOptions) {
    let max = options
        .max_pool_size
        .expect("NyxID configures its pool size");
    let pools = Arc::new(Mutex::new(HashMap::<ServerAddress, Pool>::new()));
    let events = pools.clone();
    options.cmap_event_handler = Some(EventHandler::callback(move |event| {
        let now = Instant::now();
        let mut pools = events.lock().unwrap_or_else(|e| e.into_inner());
        let slow = match event {
            CmapEvent::ConnectionCheckedOut(e) => {
                let pool = pools.entry(e.address).or_default();
                pool.acquire(e.connection_id, max, now);
                pool.warning(now, e.duration).then_some(e.duration)
            }
            CmapEvent::ConnectionCheckoutFailed(e) => pools
                .entry(e.address)
                .or_default()
                .warning(now, e.duration)
                .then_some(e.duration),
            CmapEvent::ConnectionCheckedIn(e) => {
                if let Some(pool) = pools.get_mut(&e.address) {
                    pool.release(e.connection_id, max);
                }
                None
            }
            CmapEvent::ConnectionClosed(e) => {
                if let Some(pool) = pools.get_mut(&e.address) {
                    pool.release(e.connection_id, max);
                }
                None
            }
            CmapEvent::PoolClosed(e) => {
                pools.remove(&e.address);
                None
            }
            _ => None,
        };
        drop(pools);
        if let Some(duration) = slow {
            tracing::warn!(
                max_pool_size = max,
                checkout_ms = duration.as_millis() as u64,
                "MongoDB connection checkout was slow"
            );
        }
    }));
    let weak = Arc::downgrade(&pools);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SLOW);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let Some(pools) = weak.upgrade() else {
                break;
            };
            let warnings: Vec<_> = pools
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values_mut()
                .filter_map(|pool| pool.stalled(Instant::now()))
                .collect();
            for duration in warnings {
                tracing::warn!(
                    max_pool_size = max,
                    saturated_ms = duration.as_millis() as u64,
                    "MongoDB connection pool remains fully occupied"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn warns_during_saturation_without_waiting_for_a_checkout_to_finish() {
        let at = Instant::now();
        let mut pool = Pool::default();
        pool.acquire(1, 2, at);
        assert_eq!(pool.stalled(at + SLOW), None);
        pool.acquire(2, 2, at);
        assert_eq!(pool.stalled(at + SLOW / 2), None);
        assert_eq!(pool.stalled(at + SLOW), Some(SLOW));
        assert_eq!(pool.stalled(at + SLOW * 2), None);
        assert_eq!(
            pool.stalled(at + SLOW + LOG_INTERVAL),
            Some(SLOW + LOG_INTERVAL)
        );
        pool.release(1, 2);
        assert_eq!(pool.stalled(at + LOG_INTERVAL * 2), None);
    }
    #[test]
    fn cancellation_is_not_counted_as_an_infinite_queue_and_closed_sockets_release() {
        let at = Instant::now();
        let mut pool = Pool::default();
        pool.acquire(1, 1, at);
        // CMAP checkout-start/cancellation does not change occupancy at all.
        pool.release(1, 1);
        pool.release(1, 1); // Closed/check-in notifications cannot underflow.
        assert!(pool.checked_out.is_empty());
        assert!(pool.stalled(at + LOG_INTERVAL).is_none());
    }
    #[test]
    fn slow_checkout_logging_is_rate_limited_across_saturation_episodes() {
        let at = Instant::now();
        let mut pool = Pool::default();
        assert!(!pool.warning(at, SLOW / 2));
        assert!(pool.warning(at, SLOW));
        assert!(!pool.warning(at + SLOW, SLOW * 2));
        assert!(pool.warning(at + LOG_INTERVAL, SLOW));
    }
}
