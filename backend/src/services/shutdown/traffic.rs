//! Fixed-size, process-local traffic history. No request metadata is retained.
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::time::Instant;

const BUCKET_SECS: u64 = 5;
const BUCKETS: usize = 12;

pub(super) struct Traffic {
    started: Instant,
    started_at: DateTime<Utc>,
    received: [AtomicU64; BUCKETS],
    responded: [AtomicU64; BUCKETS],
}

impl Default for Traffic {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            started_at: Utc::now(),
            received: std::array::from_fn(|_| AtomicU64::new(0)),
            responded: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

#[derive(Serialize)]
pub struct Snapshot {
    captured_at: DateTime<Utc>,
    bucket_secs: u64,
    buckets: Vec<Bucket>,
}

#[derive(Serialize)]
struct Bucket {
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    received: u64,
    responded: u64,
}

impl Traffic {
    pub(super) fn received(&self) {
        increment(
            &self.received,
            self.started.elapsed().as_secs() / BUCKET_SECS,
        );
    }

    pub(super) fn responded(&self) {
        increment(
            &self.responded,
            self.started.elapsed().as_secs() / BUCKET_SECS,
        );
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        let elapsed = self.started.elapsed();
        let epoch = elapsed.as_secs() / BUCKET_SECS;
        let captured_at = self.started_at
            + chrono::Duration::milliseconds(elapsed.as_millis().min(i64::MAX as u128) as i64);
        let buckets = (epoch.saturating_sub(BUCKETS as u64 - 1)..=epoch)
            .map(|epoch| Bucket {
                start_at: self.started_at + chrono::Duration::seconds((epoch * BUCKET_SECS) as i64),
                end_at: captured_at.min(
                    self.started_at + chrono::Duration::seconds(((epoch + 1) * BUCKET_SECS) as i64),
                ),
                received: count(&self.received, epoch),
                responded: count(&self.responded, epoch),
            })
            .collect();
        Snapshot {
            captured_at,
            bucket_secs: BUCKET_SECS,
            buckets,
        }
    }
}

// One atomic packs the epoch tag and a saturating count. Reusing a slot cannot
// mix epochs or let a delayed writer reset a newer bucket. Tags last 680 years;
// counts saturate at 2^32-1 requests per five seconds.
fn increment(slots: &[AtomicU64; BUCKETS], epoch: u64) {
    let slot = &slots[epoch as usize % BUCKETS];
    let tag = (epoch + 1) << 32;
    let mut old = slot.load(Ordering::Relaxed);
    loop {
        if old >> 32 > epoch + 1 {
            return;
        }
        let next = if old >> 32 == epoch + 1 {
            tag | u64::from((old as u32).saturating_add(1))
        } else {
            tag | 1
        };
        match slot.compare_exchange_weak(old, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(current) => old = current,
        }
    }
}

fn count(slots: &[AtomicU64; BUCKETS], epoch: u64) -> u64 {
    let value = slots[epoch as usize % BUCKETS].load(Ordering::Relaxed);
    if value >> 32 == epoch + 1 {
        u64::from(value as u32)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_deploy_gap_traffic_concurrent_counts_and_stale_slot_writers() {
        let slots = std::array::from_fn(|_| AtomicU64::new(0));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let slots = &slots;
                scope.spawn(move || {
                    for _ in 0..10_000 {
                        increment(slots, 0);
                    }
                });
            }
        });
        assert_eq!(count(&slots, 0), 80_000);
        increment(&slots, BUCKETS as u64);
        increment(&slots, 0); // A writer delayed across slot reuse cannot reset it.
        assert_eq!(count(&slots, 0), 0);
        assert_eq!(count(&slots, BUCKETS as u64), 1);
        slots[0].store(
            ((BUCKETS as u64 + 1) << 32) | u64::from(u32::MAX),
            Ordering::Relaxed,
        );
        increment(&slots, BUCKETS as u64);
        assert_eq!(count(&slots, BUCKETS as u64), u64::from(u32::MAX));
    }
}
