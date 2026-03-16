use std::sync::atomic::{AtomicU64, Ordering};

/// Local metrics counters reported to the server via `status_update` messages.
pub struct NodeMetrics {
    pub total_requests: AtomicU64,
    pub success_count: AtomicU64,
    pub error_count: AtomicU64,
}

impl NodeMetrics {
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
        }
    }

    pub fn record_success(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.success_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_error(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)] // Used in tests; will be used by status_update reporting
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            success_count: self.success_count.load(Ordering::Relaxed),
            error_count: self.error_count.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Will be used by status_update reporting
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub success_count: u64,
    pub error_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_counters_are_zero() {
        let m = NodeMetrics::new();
        let snap = m.snapshot();
        assert_eq!(snap.total_requests, 0);
        assert_eq!(snap.success_count, 0);
        assert_eq!(snap.error_count, 0);
    }

    #[test]
    fn record_success_increments() {
        let m = NodeMetrics::new();
        m.record_success();
        m.record_success();
        let snap = m.snapshot();
        assert_eq!(snap.total_requests, 2);
        assert_eq!(snap.success_count, 2);
        assert_eq!(snap.error_count, 0);
    }

    #[test]
    fn record_error_increments() {
        let m = NodeMetrics::new();
        m.record_success();
        m.record_error();
        let snap = m.snapshot();
        assert_eq!(snap.total_requests, 2);
        assert_eq!(snap.success_count, 1);
        assert_eq!(snap.error_count, 1);
    }
}
