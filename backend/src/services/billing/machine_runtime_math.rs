//! Pure boundaries shared by the durable runtime billing implementation.
pub(super) const SECOND_NS: i64 = 1_000_000_000;

pub(super) fn cumulative_seconds(previous_ns: i64, additional_ns: i64) -> Option<(i64, i64)> {
    if previous_ns < 0 || additional_ns < 0 {
        return None;
    }
    let total = previous_ns.checked_add(additional_ns)?;
    Some((total, total / SECOND_NS - previous_ns / SECOND_NS))
}

pub(super) fn quote_is_live(created_ms: i64, now_ms: i64) -> bool {
    now_ms >= created_ms && now_ms < created_ms.saturating_add(15 * 60 * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quote_fifteen_minute_boundary() {
        assert!(quote_is_live(1000, 900999));
        assert!(!quote_is_live(1000, 901000));
        assert!(!quote_is_live(1000, 999));
    }
    #[test]
    fn windows_carry_fraction_and_final_fraction_is_free() {
        let (n, s) = cumulative_seconds(0, 600_000_000).unwrap();
        assert_eq!(s, 0);
        let (n, s) = cumulative_seconds(n, 600_000_000).unwrap();
        assert_eq!(s, 1);
        let (n, s) = cumulative_seconds(n, 799_999_999).unwrap();
        assert_eq!((n, s), (1_999_999_999, 0));
        assert_eq!(n / SECOND_NS, 1);
        assert_eq!(cumulative_seconds(i64::MAX, 1), None);
        assert_eq!(cumulative_seconds(0, -1), None);
    }
}
