//! One memory budget for ingress and managed attachment egress, across owners.
use crate::errors::{AppError, AppResult};
use std::sync::{Arc, LazyLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const CAPACITY: usize = 192 * 1024 * 1024;
pub const CHANNEL_INITIAL_LIMIT: usize = 4 * 1024 * 1024;
static BUDGET: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(CAPACITY)));

/// The permit covers the payload and one copy through extraction/encryption.
pub struct ByteReservation {
    pub payload_limit: usize,
    _permit: OwnedSemaphorePermit,
}
impl std::fmt::Debug for ByteReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ByteReservation([REDACTED])")
    }
}

fn weight(payload_limit: usize) -> AppResult<u32> {
    payload_limit
        .checked_mul(2)
        .filter(|n| *n > 0 && *n <= CAPACITY)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| AppError::BadRequest("Attachment exceeds memory budget".into()))
}

/// Cancellation while waiting removes the waiter; dropping a reservation
/// releases its complete weight. Egress passes the exact stored payload size.
pub async fn reserve(payload_limit: usize) -> AppResult<ByteReservation> {
    let permit = BUDGET
        .clone()
        .acquire_many_owned(weight(payload_limit)?)
        .await
        .map_err(|_| AppError::Internal("Attachment memory budget unavailable".into()))?;
    Ok(ByteReservation {
        payload_limit,
        _permit: permit,
    })
}

pub fn initial_channel_limit(declared: Option<u64>, maximum: usize) -> usize {
    declared
        .filter(|n| *n > 0)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(CHANNEL_INITIAL_LIMIT)
        .min(CHANNEL_INITIAL_LIMIT)
        .min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn channel_media_byte_budget_saturates_across_owners_browser_and_egress_and_cancels() {
        let full = super::super::super::attachment_extraction::MAX_BYTES;
        let mut owners = Vec::new();
        for _ in 0..4 {
            owners.push(reserve(full).await.unwrap());
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), reserve(full))
                .await
                .is_err()
        );
        let egress = reserve(16 * 1024 * 1024).await.unwrap();
        assert_eq!(BUDGET.available_permits(), 0);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), reserve(1))
                .await
                .is_err()
        );
        drop(egress);
        drop(owners);
        assert_eq!(BUDGET.available_permits(), CAPACITY);
        let all = reserve(CAPACITY / 2).await.unwrap();
        assert_eq!(all.payload_limit, CAPACITY / 2);
        drop(all);
        assert_eq!(BUDGET.available_permits(), CAPACITY);
    }

    #[test]
    fn channel_media_byte_budget_uses_payload_limit_not_reservation_for_reads() {
        for size in [None, Some(0), Some(u64::MAX)] {
            assert_eq!(
                initial_channel_limit(size, 20 * 1024 * 1024),
                CHANNEL_INITIAL_LIMIT
            );
        }
        assert_eq!(initial_channel_limit(Some(1024), usize::MAX), 1024);
        assert_eq!(weight(1024).unwrap(), 2048);
        assert!(weight(0).is_err());
        assert!(weight(CAPACITY).is_err());
    }
}
