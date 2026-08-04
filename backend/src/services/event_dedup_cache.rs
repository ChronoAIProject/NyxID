//! Best-effort idempotency cache for the HTTP Event Gateway.
//!
//! Keyed by `(conversation_id, event_id)`. Bounded by `capacity`, with a
//! per-entry TTL. Insertion order drives eviction (simple FIFO approximation
//! of LRU — good enough for short-TTL deduplication and dramatically simpler
//! than a real LRU).
//!
//! # Known limitation
//!
//! This cache is **per-process**. In multi-replica deployments each instance
//! has its own cache, so duplicate events will be forwarded if they hit
//! different replicas. A Redis-backed or sticky-routing solution is out of
//! scope for NyxID#221 Phase 1/2 and tracked as follow-up work.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
struct Entry {
    inserted_at: Instant,
}

#[derive(Debug)]
struct Inner {
    map: HashMap<(String, String), Entry>,
    order: VecDeque<(String, String)>,
}

/// Bounded FIFO dedup cache with per-entry TTL.
pub struct EventDedupCache {
    inner: Mutex<Inner>,
    capacity: usize,
    ttl: Duration,
    hits: AtomicU64,
    evictions: AtomicU64,
}

impl EventDedupCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::with_capacity(capacity),
                order: VecDeque::with_capacity(capacity),
            }),
            capacity,
            ttl,
            hits: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// Read-only check: returns `true` if the (conversation_id, event_id)
    /// pair is already in the cache within the TTL window.
    ///
    /// Unlike [`insert_if_absent`], this does **not** mutate the cache. The
    /// event-gateway service uses this to decide whether a request is a
    /// duplicate *before* forwarding; the cache is only populated after a
    /// successful forward so that retries following a transient callback
    /// failure are not silently dropped.
    pub fn contains(&self, conversation_id: &str, event_id: &str) -> bool {
        let key = (conversation_id.to_string(), event_id.to_string());
        let now = Instant::now();
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match inner.map.get(&key) {
            Some(entry) => now.duration_since(entry.inserted_at) < self.ttl,
            None => false,
        }
    }

    /// Returns `true` if the (conversation_id, event_id) pair was unseen and
    /// has now been recorded. Returns `false` if it was already in the cache
    /// within the TTL window — the caller should treat this as a duplicate.
    pub fn insert_if_absent(&self, conversation_id: &str, event_id: &str) -> bool {
        let key = (conversation_id.to_string(), event_id.to_string());
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(entry) = inner.map.get(&key) {
            if now.duration_since(entry.inserted_at) < self.ttl {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            // Stale — fall through and replace.
            inner.map.remove(&key);
        }

        // Evict expired front entries opportunistically.
        while let Some(front) = inner.order.front() {
            if let Some(entry) = inner.map.get(front) {
                if now.duration_since(entry.inserted_at) >= self.ttl {
                    let key = inner.order.pop_front().unwrap();
                    inner.map.remove(&key);
                } else {
                    break;
                }
            } else {
                inner.order.pop_front();
            }
        }

        // Enforce capacity via FIFO eviction.
        while inner.map.len() >= self.capacity {
            if let Some(k) = inner.order.pop_front() {
                inner.map.remove(&k);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            } else {
                break;
            }
        }

        inner.map.insert(key.clone(), Entry { inserted_at: now });
        inner.order.push_back(key);
        true
    }

    /// Scan-and-drop expired entries. Call periodically from a background
    /// task. Cheaper variants run opportunistically in `insert_if_absent`.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .map
            .retain(|_, entry| now.duration_since(entry.inserted_at) < self.ttl);
        let surviving = &inner.map;
        let retained: VecDeque<(String, String)> = inner
            .order
            .iter()
            .filter(|key| surviving.contains_key(*key))
            .cloned()
            .collect();
        inner.order = retained;
    }

    pub fn hit_count(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn eviction_count(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn first_insert_returns_true() {
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        assert!(cache.insert_if_absent("conv-1", "evt-1"));
    }

    #[test]
    fn duplicate_insert_returns_false() {
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        assert!(cache.insert_if_absent("conv-1", "evt-1"));
        assert!(!cache.insert_if_absent("conv-1", "evt-1"));
        assert_eq!(cache.hit_count(), 1);
    }

    #[test]
    fn different_conversations_are_isolated() {
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        assert!(cache.insert_if_absent("conv-1", "evt-1"));
        assert!(cache.insert_if_absent("conv-2", "evt-1"));
    }

    #[test]
    fn expired_entry_is_treated_as_unseen() {
        let cache = EventDedupCache::new(16, Duration::from_millis(50));
        assert!(cache.insert_if_absent("conv-1", "evt-1"));
        sleep(Duration::from_millis(80));
        assert!(cache.insert_if_absent("conv-1", "evt-1"));
    }

    #[test]
    fn capacity_eviction_is_fifo() {
        let cache = EventDedupCache::new(3, Duration::from_secs(60));
        assert!(cache.insert_if_absent("conv", "evt-1"));
        assert!(cache.insert_if_absent("conv", "evt-2"));
        assert!(cache.insert_if_absent("conv", "evt-3"));
        assert_eq!(cache.len(), 3);
        // This should evict evt-1
        assert!(cache.insert_if_absent("conv", "evt-4"));
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.eviction_count(), 1);
        // evt-1 is now unseen again
        assert!(cache.insert_if_absent("conv", "evt-1"));
    }

    #[test]
    fn cleanup_removes_expired_entries() {
        let cache = EventDedupCache::new(16, Duration::from_millis(40));
        cache.insert_if_absent("conv", "evt-1");
        cache.insert_if_absent("conv", "evt-2");
        sleep(Duration::from_millis(60));
        cache.cleanup();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn contains_is_read_only() {
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        assert!(!cache.contains("conv", "evt-1"));
        // Still 0 entries — contains() must not insert.
        assert_eq!(cache.len(), 0);
        // Now insert and verify contains() sees it.
        assert!(cache.insert_if_absent("conv", "evt-1"));
        assert!(cache.contains("conv", "evt-1"));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn contains_respects_ttl() {
        let cache = EventDedupCache::new(16, Duration::from_millis(40));
        cache.insert_if_absent("conv", "evt-expiring");
        assert!(cache.contains("conv", "evt-expiring"));
        sleep(Duration::from_millis(60));
        assert!(!cache.contains("conv", "evt-expiring"));
    }

    #[test]
    fn contains_does_not_affect_insert_if_absent() {
        // Regression: the contains() helper must not perturb the cache in
        // a way that breaks the subsequent insert_if_absent() call. This
        // is the flow used by channel_event_service: check → forward →
        // insert on success.
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        assert!(!cache.contains("conv", "evt-flow"));
        assert!(cache.insert_if_absent("conv", "evt-flow"));
        assert!(cache.contains("conv", "evt-flow"));
        // A second insert_if_absent is a no-op (already present).
        assert!(!cache.insert_if_absent("conv", "evt-flow"));
    }

    // --- same event_id in different conversations ---

    #[test]
    fn same_event_id_different_conversations_are_independent() {
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        assert!(cache.insert_if_absent("conv-a", "evt-1"));
        assert!(cache.insert_if_absent("conv-b", "evt-1"));
        assert!(cache.insert_if_absent("conv-c", "evt-1"));
        assert_eq!(cache.len(), 3);
        // Each should be seen as a duplicate within its own conversation
        assert!(!cache.insert_if_absent("conv-a", "evt-1"));
        assert!(!cache.insert_if_absent("conv-b", "evt-1"));
    }

    // --- cleanup preserves non-expired entries ---

    #[test]
    fn cleanup_preserves_fresh_entries() {
        let cache = EventDedupCache::new(16, Duration::from_secs(300));
        cache.insert_if_absent("conv", "evt-fresh-1");
        cache.insert_if_absent("conv", "evt-fresh-2");
        cache.cleanup();
        assert_eq!(cache.len(), 2);
        assert!(cache.contains("conv", "evt-fresh-1"));
        assert!(cache.contains("conv", "evt-fresh-2"));
    }

    // --- hit count tracks duplicates accurately ---

    #[test]
    fn hit_count_increments_per_duplicate() {
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        assert_eq!(cache.hit_count(), 0);
        cache.insert_if_absent("conv", "evt-1");
        assert_eq!(cache.hit_count(), 0); // first insert is not a hit
        cache.insert_if_absent("conv", "evt-1");
        assert_eq!(cache.hit_count(), 1);
        cache.insert_if_absent("conv", "evt-1");
        assert_eq!(cache.hit_count(), 2);
    }

    // --- capacity of 1 ---

    #[test]
    fn capacity_one_evicts_immediately() {
        let cache = EventDedupCache::new(1, Duration::from_secs(60));
        assert!(cache.insert_if_absent("conv", "evt-1"));
        assert_eq!(cache.len(), 1);
        assert!(cache.insert_if_absent("conv", "evt-2"));
        assert_eq!(cache.len(), 1);
        // evt-1 was evicted
        assert!(cache.insert_if_absent("conv", "evt-1"));
        assert_eq!(cache.eviction_count(), 2);
    }

    // --- stale entry replaced on re-insert ---

    #[test]
    fn stale_entry_is_replaced_on_reinsert() {
        let cache = EventDedupCache::new(16, Duration::from_millis(30));
        assert!(cache.insert_if_absent("conv", "evt-stale"));
        sleep(Duration::from_millis(50));
        // Entry expired; re-insert should succeed
        assert!(cache.insert_if_absent("conv", "evt-stale"));
        // And now it's fresh again
        assert!(!cache.insert_if_absent("conv", "evt-stale"));
    }

    // --- interleaved inserts across conversations ---

    #[test]
    fn interleaved_conversations_fifo_eviction() {
        let cache = EventDedupCache::new(3, Duration::from_secs(60));
        cache.insert_if_absent("conv-a", "e1");
        cache.insert_if_absent("conv-b", "e2");
        cache.insert_if_absent("conv-c", "e3");
        // Evicts oldest (conv-a, e1)
        cache.insert_if_absent("conv-d", "e4");
        assert_eq!(cache.len(), 3);
        // conv-a/e1 is gone
        assert!(!cache.contains("conv-a", "e1"));
        assert!(cache.contains("conv-b", "e2"));
        assert!(cache.contains("conv-d", "e4"));
    }

    // --- concurrent access (basic smoke test) ---

    #[test]
    fn concurrent_inserts_do_not_panic() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(EventDedupCache::new(256, Duration::from_secs(60)));
        let mut handles = Vec::new();

        for t in 0..4 {
            let cache = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    cache.insert_if_absent(&format!("conv-{t}"), &format!("evt-{i}"));
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // All 4 threads * 100 events = 400 unique entries (capacity 256)
        assert!(cache.len() <= 256);
    }

    // --- cleanup on empty cache ---

    #[test]
    fn cleanup_on_empty_cache_does_not_panic() {
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        cache.cleanup();
        assert_eq!(cache.len(), 0);
    }

    // --- eviction_count starts at zero ---

    #[test]
    fn eviction_count_starts_at_zero() {
        let cache = EventDedupCache::new(16, Duration::from_secs(60));
        assert_eq!(cache.eviction_count(), 0);
    }
}
