//! Bounded FIFO replay cache for DPoP proof `jti` claims.
//!
//! Mirrors `services/event_dedup_cache.rs` but single-key. This cache is
//! per-process; multi-replica deployments need a shared Redis-backed
//! equivalent to reject replay across instances.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DPOP_JTI_CACHE_CAPACITY: usize = 16_384;
pub const DPOP_JTI_CACHE_TTL_SECS: u64 = 600;

#[derive(Debug)]
struct Entry {
    inserted_at: Instant,
}

#[derive(Debug)]
struct Inner {
    map: HashMap<String, Entry>,
    order: VecDeque<String>,
}

pub struct DpopJtiCache {
    inner: Mutex<Inner>,
    capacity: usize,
    ttl: Duration,
}

impl DpopJtiCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::with_capacity(capacity),
                order: VecDeque::with_capacity(capacity),
            }),
            capacity,
            ttl,
        }
    }

    pub fn insert_if_absent(&self, jti: &str) -> bool {
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(entry) = inner.map.get(jti) {
            if now.duration_since(entry.inserted_at) < self.ttl {
                return false;
            }
            inner.map.remove(jti);
        }

        while let Some(front) = inner.order.front() {
            if let Some(entry) = inner.map.get(front) {
                if now.duration_since(entry.inserted_at) >= self.ttl {
                    let key = inner.order.pop_front().expect("front exists");
                    inner.map.remove(&key);
                } else {
                    break;
                }
            } else {
                inner.order.pop_front();
            }
        }

        while inner.map.len() >= self.capacity {
            if let Some(key) = inner.order.pop_front() {
                inner.map.remove(&key);
            } else {
                break;
            }
        }

        let key = jti.to_string();
        inner.map.insert(key.clone(), Entry { inserted_at: now });
        inner.order.push_back(key);
        true
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn first_insert_returns_true() {
        let cache = DpopJtiCache::new(16, Duration::from_secs(60));
        assert!(cache.insert_if_absent("jti-1"));
    }

    #[test]
    fn duplicate_insert_returns_false() {
        let cache = DpopJtiCache::new(16, Duration::from_secs(60));
        assert!(cache.insert_if_absent("jti-1"));
        assert!(!cache.insert_if_absent("jti-1"));
    }

    #[test]
    fn expired_entry_is_treated_as_unseen() {
        let cache = DpopJtiCache::new(16, Duration::from_millis(30));
        assert!(cache.insert_if_absent("jti-1"));
        sleep(Duration::from_millis(50));
        assert!(cache.insert_if_absent("jti-1"));
    }

    #[test]
    fn capacity_eviction_is_fifo() {
        let cache = DpopJtiCache::new(2, Duration::from_secs(60));
        assert!(cache.insert_if_absent("jti-1"));
        assert!(cache.insert_if_absent("jti-2"));
        assert_eq!(cache.len(), 2);
        assert!(cache.insert_if_absent("jti-3"));
        assert_eq!(cache.len(), 2);
        assert!(cache.insert_if_absent("jti-1"));
    }
}
