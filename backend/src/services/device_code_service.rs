#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};

pub const DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD: u32 = 3;
pub const DEVICE_CODE_LOCKOUT_SECS: i64 = 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct SignatureFailureLockout {
    pub failed_poll_count: u32,
    pub locked_until: Option<DateTime<Utc>>,
}

pub fn apply_signature_failure_lockout(
    current_failed_poll_count: u32,
    now: DateTime<Utc>,
) -> SignatureFailureLockout {
    let failed_poll_count = current_failed_poll_count.saturating_add(1);
    let locked_until = (failed_poll_count >= DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD)
        .then_some(now + Duration::seconds(DEVICE_CODE_LOCKOUT_SECS));

    SignatureFailureLockout {
        failed_poll_count,
        locked_until,
    }
}

pub fn is_locked(locked_until: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    locked_until.is_some_and(|until| until > now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_failures_below_threshold_do_not_lock() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(1, now);

        assert_eq!(transition.failed_poll_count, 2);
        assert_eq!(transition.locked_until, None);
    }

    #[test]
    fn signature_failure_at_threshold_locks_for_one_hour() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(2, now);

        assert_eq!(transition.failed_poll_count, 3);
        assert_eq!(
            transition.locked_until.expect("locked").timestamp(),
            (now + Duration::hours(1)).timestamp()
        );
    }

    #[test]
    fn signature_failure_after_threshold_keeps_locking() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(3, now);

        assert_eq!(transition.failed_poll_count, 4);
        assert!(transition.locked_until.is_some());
    }

    #[test]
    fn is_locked_only_when_until_is_in_future() {
        let now = Utc::now();

        assert!(is_locked(Some(now + Duration::seconds(1)), now));
        assert!(!is_locked(Some(now), now));
        assert!(!is_locked(Some(now - Duration::seconds(1)), now));
        assert!(!is_locked(None, now));
    }
}
