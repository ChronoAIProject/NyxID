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

    // --- constants ---

    #[test]
    fn default_capacity_is_reasonable() {
        assert_eq!(DPOP_JTI_CACHE_CAPACITY, 16_384);
    }

    #[test]
    fn default_ttl_is_ten_minutes() {
        assert_eq!(DPOP_JTI_CACHE_TTL_SECS, 600);
    }

    // --- multiple distinct JTIs ---

    #[test]
    fn many_distinct_jtis_all_accepted() {
        let cache = DpopJtiCache::new(100, Duration::from_secs(60));
        for i in 0..50 {
            assert!(cache.insert_if_absent(&format!("jti-unique-{i}")));
        }
        assert_eq!(cache.len(), 50);
        // All should be seen as duplicates
        for i in 0..50 {
            assert!(!cache.insert_if_absent(&format!("jti-unique-{i}")));
        }
    }

    // --- capacity of 1 ---

    #[test]
    fn capacity_one_evicts_immediately() {
        let cache = DpopJtiCache::new(1, Duration::from_secs(60));
        assert!(cache.insert_if_absent("jti-a"));
        assert_eq!(cache.len(), 1);
        assert!(cache.insert_if_absent("jti-b"));
        assert_eq!(cache.len(), 1);
        // jti-a was evicted, can be re-inserted
        assert!(cache.insert_if_absent("jti-a"));
        // jti-b was evicted
        assert!(cache.insert_if_absent("jti-b"));
    }

    // --- expired entry does not block fresh insert ---

    #[test]
    fn reinsert_after_expiry_does_not_leave_stale_order() {
        let cache = DpopJtiCache::new(16, Duration::from_millis(30));
        assert!(cache.insert_if_absent("jti-reinsert"));
        sleep(Duration::from_millis(50));
        // Expired: should be treated as unseen
        assert!(cache.insert_if_absent("jti-reinsert"));
        // Now it's fresh: should be duplicate
        assert!(!cache.insert_if_absent("jti-reinsert"));
        assert_eq!(cache.len(), 1);
    }

    // --- opportunistic cleanup during insert ---

    #[test]
    fn opportunistic_cleanup_reclaims_expired_entries() {
        let cache = DpopJtiCache::new(4, Duration::from_millis(30));
        assert!(cache.insert_if_absent("jti-old-1"));
        assert!(cache.insert_if_absent("jti-old-2"));
        assert_eq!(cache.len(), 2);

        sleep(Duration::from_millis(50));

        // Inserting a new JTI should opportunistically clean up expired front entries
        assert!(cache.insert_if_absent("jti-new"));
        // Only the new entry should remain (old ones expired and cleaned)
        assert_eq!(cache.len(), 1);
    }

    // --- concurrent access smoke test ---

    #[test]
    fn concurrent_inserts_do_not_panic() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(DpopJtiCache::new(256, Duration::from_secs(60)));
        let mut handles = Vec::new();

        for t in 0..4 {
            let cache = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    cache.insert_if_absent(&format!("t{t}-jti-{i}"));
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert!(cache.len() <= 256);
        assert!(cache.len() > 0);
    }

    // --- empty string JTI is valid ---

    #[test]
    fn empty_string_jti_is_accepted_and_deduped() {
        let cache = DpopJtiCache::new(16, Duration::from_secs(60));
        assert!(cache.insert_if_absent(""));
        assert!(!cache.insert_if_absent(""));
    }

    // --- FIFO order after mixed expired/fresh ---

    #[test]
    fn fifo_eviction_skips_already_removed_keys() {
        let cache = DpopJtiCache::new(3, Duration::from_millis(40));
        cache.insert_if_absent("jti-a");
        sleep(Duration::from_millis(10));
        cache.insert_if_absent("jti-b");
        sleep(Duration::from_millis(10));
        cache.insert_if_absent("jti-c");

        sleep(Duration::from_millis(30));
        // jti-a has expired; jti-b and jti-c may or may not have expired
        // Insert triggers opportunistic cleanup of expired front entries
        cache.insert_if_absent("jti-d");
        // Should not panic; cache should have at most 3 entries
        assert!(cache.len() <= 3);
    }
}
