//! Per-process load shedding for repeated, locally refused HTTP proxy requests.
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

const THRESHOLD: usize = 20;
const WINDOW: Duration = Duration::from_secs(60);
pub(crate) const RETRY_SECONDS: u64 = 30;
const RETRY: Duration = Duration::from_secs(RETRY_SECONDS);
const CAPACITY: usize = 4096;
const IDLE: Duration = Duration::from_secs(600);

type Key = [u8; 32];
type RefusalKey = [u8; 32];

pub(crate) fn request_key(
    caller_kind: &str,
    caller: &str,
    method: &str,
    service: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8; 32],
) -> Key {
    let mut digest = Sha256::new();
    for part in [caller_kind, caller, method, service, path] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    digest.update([u8::from(query.is_some())]);
    let query = query.unwrap_or_default();
    digest.update((query.len() as u64).to_be_bytes());
    digest.update(query.as_bytes());
    digest.update(body);
    digest.finalize().into()
}

fn refusal_key(request: &Key, code: u32) -> RefusalKey {
    let mut digest = Sha256::new();
    digest.update(request);
    digest.update(code.to_be_bytes());
    digest.finalize().into()
}

pub(crate) struct RefusalThrottle {
    state: Mutex<State>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    capacity: usize,
}

#[derive(Default)]
struct State {
    cold: HashMap<RefusalKey, Window>,
    insertion_order: BTreeSet<(u64, RefusalKey)>,
    armed: HashMap<Key, Armed>,
    expiry: BTreeSet<(Instant, Key)>,
    serial: u64,
}

struct Window {
    request: Key,
    times: [Instant; THRESHOLD],
    len: usize,
    serial: u64,
}

struct Armed {
    code: u32,
    count: u64,
    retry_at: Instant,
    probing: bool,
    expires_at: Instant,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    Ordinary,
    Probe,
    Throttled,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Transition {
    Armed { code: u32, count: u64 },
    Cleared { code: u32, count: u64 },
}

impl Default for RefusalThrottle {
    fn default() -> Self {
        Self::with_clock(Arc::new(Instant::now), CAPACITY)
    }
}

impl RefusalThrottle {
    pub(crate) fn with_clock(
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
        capacity: usize,
    ) -> Self {
        assert!(capacity > 0);
        Self {
            state: Mutex::new(State::default()),
            clock,
            capacity,
        }
    }

    pub(crate) fn admit(&self, key: &Key) -> Admission {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = (self.clock)();
        state.expire(now);
        let State { armed, expiry, .. } = &mut *state;
        let Some(entry) = armed.get_mut(key) else {
            return Admission::Ordinary;
        };
        expiry.remove(&(entry.expires_at, *key));
        entry.expires_at = entry.retry_at.max(now) + IDLE;
        if !entry.probing && now >= entry.retry_at {
            entry.probing = true;
            Admission::Probe
        } else {
            // An in-flight probe owns its slot until completion or cancellation.
            if !entry.probing {
                expiry.insert((entry.expires_at, *key));
            }
            Admission::Throttled
        }
    }

    pub(crate) fn finish(&self, key: Key, probe: bool, code: Option<u32>) -> Option<Transition> {
        if !probe && code.is_none() {
            return None;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = (self.clock)();
        state.expire(now);
        if probe {
            let entry = state.armed.get_mut(&key)?;
            if code == Some(entry.code) {
                entry.count = entry.count.saturating_add(1);
                entry.retry_at = now + RETRY;
                entry.probing = false;
                entry.expires_at = entry.retry_at + IDLE;
                let expires_at = entry.expires_at;
                let transition = Transition::Armed {
                    code: entry.code,
                    count: entry.count,
                };
                state.expiry.insert((expires_at, key));
                return Some(transition);
            }
            let entry = state.armed.remove(&key)?;
            let State {
                cold,
                insertion_order,
                ..
            } = &mut *state;
            cold.retain(|refusal, window| {
                if window.request == key {
                    insertion_order.remove(&(window.serial, *refusal));
                    false
                } else {
                    true
                }
            });
            return Some(Transition::Cleared {
                code: entry.code,
                count: entry.count,
            });
        }
        // Completions already in flight when a throttle armed cannot change its probe.
        if state.armed.contains_key(&key) {
            return None;
        }
        let code = code?;
        let refusal = refusal_key(&key, code);
        if !state.cold.contains_key(&refusal) {
            if state.cold.len() == self.capacity {
                let (_, oldest) = state
                    .insertion_order
                    .pop_first()
                    .expect("cold eviction index");
                state.cold.remove(&oldest);
            }
            state.serial = state.serial.wrapping_add(1);
            let serial = state.serial;
            state.insertion_order.insert((serial, refusal));
            state.cold.insert(
                refusal,
                Window {
                    request: key,
                    times: [now; THRESHOLD],
                    len: 0,
                    serial,
                },
            );
        }
        let entry = state.cold.get_mut(&refusal).expect("inserted refusal");
        let expired =
            entry.times[..entry.len].partition_point(|time| now.duration_since(*time) >= WINDOW);
        entry.times.copy_within(expired..entry.len, 0);
        entry.len -= expired;
        if entry.len < THRESHOLD {
            entry.times[entry.len] = now;
            entry.len += 1;
        }
        if entry.len < THRESHOLD || state.armed.len() == self.capacity {
            return None;
        }
        let entry = state.cold.remove(&refusal).expect("threshold refusal");
        state.insertion_order.remove(&(entry.serial, refusal));
        state.armed.insert(
            key,
            Armed {
                code,
                count: THRESHOLD as u64,
                retry_at: now + RETRY,
                probing: false,
                expires_at: now + RETRY + IDLE,
            },
        );
        state.expiry.insert((now + RETRY + IDLE, key));
        Some(Transition::Armed {
            code,
            count: THRESHOLD as u64,
        })
    }

    pub(crate) fn cancel_probe(&self, key: &Key) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = (self.clock)();
        if let Some(entry) = state.armed.get_mut(key) {
            entry.probing = false;
            entry.retry_at = now + RETRY;
            entry.expires_at = entry.retry_at + IDLE;
            let expires_at = entry.expires_at;
            state.expiry.insert((expires_at, *key));
        }
    }
}

impl State {
    fn expire(&mut self, now: Instant) {
        while self
            .expiry
            .first()
            .is_some_and(|(deadline, _)| *deadline <= now)
        {
            let (_, key) = self.expiry.pop_first().expect("armed expiry index");
            if let Some(entry) = self.armed.remove(&key) {
                tracing::warn!(
                    request_digest = %hex::encode(key), error_code = entry.code,
                    count = entry.count, reason = "idle",
                    "Proxy refusal throttle cleared"
                );
            }
        }
    }
}

pub(crate) struct ProbeGuard<'a> {
    limiter: &'a RefusalThrottle,
    key: Key,
    pending: bool,
}

impl<'a> ProbeGuard<'a> {
    pub(crate) fn new(limiter: &'a RefusalThrottle, key: Key, probe: bool) -> Self {
        Self {
            limiter,
            key,
            pending: probe,
        }
    }

    pub(crate) fn finish(mut self, code: Option<u32>) -> Option<Transition> {
        let transition = self.limiter.finish(self.key, self.pending, code);
        self.pending = false;
        transition
    }
}

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        if self.pending {
            self.limiter.cancel_probe(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(capacity: usize) -> (RefusalThrottle, Arc<Mutex<Instant>>) {
        let now = Arc::new(Mutex::new(Instant::now()));
        let clock = now.clone();
        (
            RefusalThrottle::with_clock(Arc::new(move || *clock.lock().unwrap()), capacity),
            now,
        )
    }

    fn key(path: &str) -> Key {
        request_key(
            "key",
            "synthetic",
            "GET",
            "service",
            path,
            None,
            &Sha256::digest([]).into(),
        )
    }
    fn advance(clock: &Mutex<Instant>, duration: Duration) {
        *clock.lock().unwrap() += duration;
    }
    fn refuse(limiter: &RefusalThrottle, key: Key, code: u32) -> Option<Transition> {
        let admission = limiter.admit(&key);
        assert_ne!(admission, Admission::Throttled);
        limiter.finish(key, admission == Admission::Probe, Some(code))
    }
    fn arm(limiter: &RefusalThrottle, key: Key) {
        for _ in 0..THRESHOLD - 1 {
            assert_eq!(refuse(limiter, key, 1003), None);
        }
        assert_eq!(
            refuse(limiter, key, 1003),
            Some(Transition::Armed {
                code: 1003,
                count: 20
            })
        );
    }

    #[test]
    fn twenty_refusals_then_one_probe_rearms_and_different_result_clears() {
        let (limiter, clock) = fixture(8);
        let key = key("/items");
        arm(&limiter, key);
        assert_eq!(limiter.admit(&key), Admission::Throttled);
        advance(&clock, RETRY - Duration::from_nanos(1));
        assert_eq!(limiter.admit(&key), Admission::Throttled);
        advance(&clock, Duration::from_nanos(1));
        assert_eq!(limiter.admit(&key), Admission::Probe);
        for _ in 0..100 {
            assert_eq!(limiter.admit(&key), Admission::Throttled);
        }
        assert_eq!(
            limiter.finish(key, true, Some(1003)),
            Some(Transition::Armed {
                code: 1003,
                count: 21
            })
        );
        assert_eq!(limiter.admit(&key), Admission::Throttled);
        advance(&clock, RETRY);
        assert_eq!(limiter.admit(&key), Admission::Probe);
        assert_eq!(
            limiter.finish(key, true, Some(1002)),
            Some(Transition::Cleared {
                code: 1003,
                count: 21
            })
        );
        arm(&limiter, key);
        advance(&clock, RETRY);
        assert_eq!(limiter.admit(&key), Admission::Probe);
        assert_eq!(
            limiter.finish(key, true, None),
            Some(Transition::Cleared {
                code: 1003,
                count: 20
            })
        );
        assert_eq!(limiter.admit(&key), Admission::Ordinary);
    }

    #[test]
    fn sliding_window_and_error_codes_are_independent() {
        let (limiter, clock) = fixture(8);
        let key = key("/items");
        for _ in 0..19 {
            refuse(&limiter, key, 1003);
            refuse(&limiter, key, 1002);
        }
        advance(&clock, WINDOW);
        assert_eq!(refuse(&limiter, key, 1003), None);
        for _ in 0..18 {
            assert_eq!(refuse(&limiter, key, 1002), None);
        }
        assert_eq!(refuse(&limiter, key, 1003), None);
        assert_eq!(refuse(&limiter, key, 1002), None);
        assert!(matches!(
            refuse(&limiter, key, 1002),
            Some(Transition::Armed { code: 1002, .. })
        ));
    }

    #[test]
    fn sliding_window_keeps_recent_refusals_when_the_first_one_expires() {
        let (limiter, clock) = fixture(8);
        let key = key("/sliding");
        refuse(&limiter, key, 1003);
        advance(&clock, Duration::from_secs(59));
        for _ in 0..18 {
            assert_eq!(refuse(&limiter, key, 1003), None);
        }
        advance(&clock, Duration::from_secs(2));
        assert_eq!(refuse(&limiter, key, 1003), None);
        assert!(matches!(
            refuse(&limiter, key, 1003),
            Some(Transition::Armed { count: 20, .. })
        ));
    }

    #[test]
    fn caller_method_service_and_exact_path_are_independent() {
        let (limiter, _) = fixture(8);
        arm(&limiter, key("/items/1"));
        for other in [
            request_key(
                "key",
                "other",
                "GET",
                "service",
                "/items/1",
                None,
                &Sha256::digest([]).into(),
            ),
            request_key(
                "user",
                "synthetic",
                "GET",
                "service",
                "/items/1",
                None,
                &Sha256::digest([]).into(),
            ),
            request_key(
                "key",
                "synthetic",
                "POST",
                "service",
                "/items/1",
                None,
                &Sha256::digest([]).into(),
            ),
            request_key(
                "key",
                "synthetic",
                "GET",
                "other",
                "/items/1",
                None,
                &Sha256::digest([]).into(),
            ),
            key("/items/2"),
            key("/items/%31"),
            key("/items/1/"),
        ] {
            assert_eq!(limiter.admit(&other), Admission::Ordinary);
        }
    }

    #[test]
    fn cancelled_probe_releases_only_its_probe_after_retry_interval() {
        let (limiter, clock) = fixture(8);
        let key = key("/items");
        arm(&limiter, key);
        advance(&clock, RETRY);
        assert_eq!(limiter.admit(&key), Admission::Probe);
        drop(ProbeGuard::new(&limiter, key, true));
        assert_eq!(limiter.admit(&key), Admission::Throttled);
        advance(&clock, RETRY);
        assert_eq!(limiter.admit(&key), Admission::Probe);
    }

    #[test]
    fn refusal_throttle_round2_idle_armed_entries_expire_on_an_unrelated_lookup() {
        let (limiter, clock) = fixture(2);
        arm(&limiter, key("/abandoned"));
        advance(&clock, RETRY + Duration::from_secs(600));
        assert_eq!(limiter.admit(&key("/unrelated")), Admission::Ordinary);
        assert!(limiter.state.lock().unwrap().armed.is_empty());
    }

    #[test]
    fn refusal_throttle_round2_idle_refresh_protects_active_keys_and_reuses_capacity() {
        let (limiter, clock) = fixture(1);
        let active = key("/active");
        arm(&limiter, active);
        // A late probe arrival extends expiry from its arrival, not the old retry time.
        advance(&clock, RETRY + IDLE - Duration::from_secs(1));
        assert_eq!(limiter.admit(&active), Admission::Probe);
        let probe = ProbeGuard::new(&limiter, active, true);
        // An unrelated lookup cannot expire a probe which is still in flight.
        advance(&clock, IDLE + RETRY);
        assert_eq!(limiter.admit(&key("/other")), Admission::Ordinary);
        assert_eq!(limiter.admit(&active), Admission::Throttled);
        assert_eq!(limiter.state.lock().unwrap().armed.len(), 1);
        drop(probe);
        advance(&clock, RETRY + IDLE - Duration::from_nanos(1));
        assert_eq!(limiter.admit(&key("/other")), Admission::Ordinary);
        assert_eq!(limiter.state.lock().unwrap().armed.len(), 1);
        advance(&clock, Duration::from_nanos(1));
        assert_eq!(limiter.admit(&key("/other")), Admission::Ordinary);
        assert!(limiter.state.lock().unwrap().armed.is_empty());
        arm(&limiter, key("/replacement"));
        let state = limiter.state.lock().unwrap();
        assert_eq!(state.armed.len(), 1);
        assert_eq!(state.expiry.len(), 1);
    }

    #[test]
    fn refusal_throttle_round2_arrivals_and_probe_rearming_keep_one_expiry_record() {
        let (limiter, clock) = fixture(1);
        let active = key("/active");
        arm(&limiter, active);
        for _ in 0..100 {
            assert_eq!(limiter.admit(&active), Admission::Throttled);
            assert_eq!(limiter.state.lock().unwrap().expiry.len(), 1);
        }
        advance(&clock, RETRY);
        assert_eq!(limiter.admit(&active), Admission::Probe);
        assert!(limiter.state.lock().unwrap().expiry.is_empty());
        advance(&clock, IDLE);
        assert!(matches!(
            limiter.finish(active, true, Some(1003)),
            Some(Transition::Armed { .. })
        ));
        advance(&clock, RETRY + IDLE - Duration::from_nanos(1));
        limiter.admit(&key("/unrelated"));
        assert_eq!(limiter.state.lock().unwrap().armed.len(), 1);
        advance(&clock, Duration::from_nanos(1));
        limiter.admit(&key("/unrelated"));
        let state = limiter.state.lock().unwrap();
        assert!(state.armed.is_empty() && state.expiry.is_empty());
    }

    #[test]
    fn refusal_throttle_round2_raw_query_and_body_digest_are_independent() {
        let key = |query: Option<&str>, body: &[u8]| {
            request_key(
                "key",
                "synthetic",
                "POST",
                "service",
                "/items",
                query,
                &Sha256::digest(body).into(),
            )
        };
        let (limiter, _) = fixture(8);
        let first = key(Some("a=1&b=2"), b"first body");
        arm(&limiter, first);
        for independent in [
            key(Some("b=2&a=1"), b"first body"),
            key(Some("a=1&b=2"), b"second body"),
            key(Some("a=%31&b=2"), b"first body"),
            key(None, b"first body"),
            key(Some(""), b"first body"),
        ] {
            assert_eq!(limiter.admit(&independent), Admission::Ordinary);
        }
        assert_ne!(key(None, b""), key(Some(""), b""));
    }

    #[test]
    fn bounded_under_ten_thousand_paths_without_evicting_armed_entries() {
        let (limiter, _) = fixture(CAPACITY);
        let protected = key("/protected");
        arm(&limiter, protected);
        for index in 0..10_000 {
            refuse(&limiter, key(&format!("/{index}")), 1003);
            let state = limiter.state.lock().unwrap();
            assert!(state.cold.len() <= CAPACITY && state.armed.len() <= CAPACITY);
            assert_eq!(state.cold.len(), state.insertion_order.len());
        }
        assert_eq!(limiter.admit(&protected), Admission::Throttled);
        let (limiter, _) = fixture(8);
        arm(&limiter, protected);
        for index in 0..7 {
            arm(&limiter, key(&format!("/armed/{index}")));
        }
        for _ in 0..100 {
            assert_eq!(refuse(&limiter, key("/saturated"), 1003), None);
        }
        let state = limiter.state.lock().unwrap();
        assert_eq!(state.armed.len(), 8);
        assert!(state.armed.contains_key(&protected));
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;

    #[test]
    fn concurrent_requests_claim_exactly_one_probe() {
        let now = Arc::new(Mutex::new(Instant::now()));
        let clock = now.clone();
        let limiter = RefusalThrottle::with_clock(Arc::new(move || *clock.lock().unwrap()), 8);
        let key = request_key(
            "key",
            "synthetic",
            "GET",
            "service",
            "/items",
            None,
            &Sha256::digest([]).into(),
        );
        for _ in 0..THRESHOLD {
            limiter.finish(key, false, Some(1003));
        }
        *now.lock().unwrap() += RETRY;
        let barrier = std::sync::Barrier::new(2);
        let admitted = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        limiter.admit(&key) == Admission::Probe
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| usize::from(handle.join().unwrap()))
                .sum::<usize>()
        });
        assert_eq!(admitted, 1);
    }
}
