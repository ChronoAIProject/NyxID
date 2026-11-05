//! Integer allocation-time validation before touching NyxID funding documents.
pub(super) const MAX_WINDOW_SECONDS: i64 = 300;
pub(super) const MAX_EVIDENCE_INTERVALS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct IntervalEvidence {
    pub revision: i64,
    pub started_at: i64,
    pub ended_at: i64,
    /// CPU millicores, RAM MiB, retained disk GiB, in that order.
    pub allocation: [i64; 3],
    pub quantity: [i64; 3],
}

pub(super) fn finite_quantities(
    allocation: [i64; 3],
    seconds: i64,
) -> Result<[i64; 3], &'static str> {
    if !(1..=MAX_WINDOW_SECONDS).contains(&seconds) {
        return Err("Sandbox funding requires a finite window of at most 300 seconds");
    }
    allocation_quantities(allocation, seconds)
}

fn allocation_quantities(allocation: [i64; 3], seconds: i64) -> Result<[i64; 3], &'static str> {
    if seconds < 0 || allocation.iter().any(|quantity| *quantity < 0) {
        return Err("Sandbox quantities must be non-negative integers");
    }
    let mut quantity = [0; 3];
    for index in 0..3 {
        quantity[index] = allocation[index]
            .checked_mul(seconds)
            .ok_or("Sandbox allocation quantity overflow")?;
    }
    Ok(quantity)
}

/// Check the original ledger arithmetic, then intersect with funded coverage.
/// Every second needs an explicit contiguous observation, including zero-use
/// intervals. A missing tail never becomes an automatic refund or a charge.
pub(super) fn evidenced_quantities(
    from: i64,
    until: i64,
    allocation: [i64; 3],
    intervals: &[IntervalEvidence],
) -> Result<[i64; 3], &'static str> {
    if from < 0 || until <= from || until - from > MAX_WINDOW_SECONDS {
        return Err("Invalid funded Sandbox interval");
    }
    let maximum = finite_quantities(allocation, until - from)?;
    if intervals.is_empty() || intervals.len() > MAX_EVIDENCE_INTERVALS {
        return Err("Sandbox evidence must be a bounded nonempty ledger range");
    }
    let mut total = [0_i64; 3];
    let mut cursor = from;
    let mut previous: Option<&IntervalEvidence> = None;
    for interval in intervals {
        if interval.revision <= 0
            || interval.started_at < 0
            || interval.ended_at < interval.started_at
        {
            return Err("Invalid Sandbox ledger interval");
        }
        if let Some(last) = previous
            && (last.revision.checked_add(1) != Some(interval.revision)
                || last.ended_at != interval.started_at)
        {
            return Err("Sandbox ledger revisions or intervals are not contiguous");
        }
        let recorded =
            allocation_quantities(interval.allocation, interval.ended_at - interval.started_at)?;
        if recorded != interval.quantity {
            return Err("Sandbox ledger arithmetic does not match its allocation");
        }
        // A range may straddle a window boundary. Unrelated old/future rows
        // cannot pad the receipt, and zero-time transitions remain meaningful.
        if interval.ended_at < from || interval.started_at > until {
            return Err("Sandbox ledger interval is outside funded coverage");
        }
        let start = interval.started_at.max(from);
        let end = interval.ended_at.min(until);
        if start != cursor || end < start {
            return Err("Sandbox ledger has a gap or overlap in funded coverage");
        }
        if end > start {
            if interval
                .allocation
                .iter()
                .zip(allocation)
                .any(|(used, reserved)| *used > reserved)
            {
                return Err("Sandbox allocation exceeds the accepted reservation");
            }
            let quantity = allocation_quantities(interval.allocation, end - start)?;
            for index in 0..3 {
                total[index] = total[index]
                    .checked_add(quantity[index])
                    .ok_or("Sandbox evidence quantity overflow")?;
            }
        }
        cursor = end;
        previous = Some(interval);
    }
    if cursor != until
        || total
            .iter()
            .zip(maximum)
            .any(|(used, reserved)| *used > reserved)
    {
        return Err("Sandbox evidence does not close the funded window");
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(revision: i64, start: i64, end: i64, allocation: [i64; 3]) -> IntervalEvidence {
        IntervalEvidence {
            revision,
            started_at: start,
            ended_at: end,
            allocation,
            quantity: allocation_quantities(allocation, end - start).unwrap(),
        }
    }

    #[test]
    fn finite_resource_holds_reject_negative_overflow_and_unbounded_time() {
        assert_eq!(
            finite_quantities([2000, 8192, 64], 300),
            Ok([600000, 2457600, 19200])
        );
        for seconds in [-1, 0, 301, i64::MAX] {
            assert!(finite_quantities([1, 1, 1], seconds).is_err());
        }
        assert!(finite_quantities([-1, 1, 1], 1).is_err());
        assert!(finite_quantities([i64::MAX, 1, 1], 2).is_err());
    }

    #[test]
    fn active_pause_resume_charges_compute_only_for_observed_running_intervals() {
        let active = [2000, 8192, 64];
        let evidence = [
            interval(10, 100, 110, active),
            interval(11, 110, 130, [0, 0, 64]),
            interval(12, 130, 160, active),
        ];
        assert_eq!(
            evidenced_quantities(100, 160, active, &evidence),
            Ok([80000, 327680, 3840])
        );
    }

    #[test]
    fn only_the_intersection_with_funded_coverage_is_chargeable() {
        let evidence = [interval(20, 50, 250, [1000, 1024, 10])];
        assert_eq!(
            evidenced_quantities(100, 200, [1000, 1024, 10], &evidence),
            Ok([100000, 102400, 1000])
        );
    }

    #[test]
    fn gaps_overlaps_missing_revisions_and_incomplete_tails_are_not_invented() {
        let first = interval(1, 100, 110, [1, 1, 1]);
        for next in [
            interval(2, 111, 120, [1, 1, 1]),
            interval(2, 109, 120, [1, 1, 1]),
            interval(3, 110, 120, [1, 1, 1]),
            interval(2, 110, 119, [1, 1, 1]),
        ] {
            assert!(evidenced_quantities(100, 120, [1, 1, 1], &[first.clone(), next]).is_err());
        }
    }

    #[test]
    fn zero_time_transition_is_valid_but_does_not_prove_an_unobserved_tail() {
        let evidence = [
            interval(3, 100, 110, [1, 1, 1]),
            interval(4, 110, 110, [0, 0, 1]),
            interval(5, 110, 120, [0, 0, 1]),
        ];
        assert_eq!(
            evidenced_quantities(100, 120, [1, 1, 1], &evidence),
            Ok([10, 10, 20])
        );
        assert!(evidenced_quantities(100, 120, [1, 1, 1], &evidence[..2]).is_err());
    }

    #[test]
    fn evidence_cannot_change_arithmetic_or_spend_unreserved_capacity() {
        let mut evidence = interval(1, 100, 120, [1, 1, 1]);
        evidence.quantity[0] += 1;
        assert!(evidenced_quantities(100, 120, [1, 1, 1], &[evidence]).is_err());
        let bigger = interval(1, 100, 120, [2, 1, 1]);
        assert!(evidenced_quantities(100, 120, [1, 1, 1], &[bigger]).is_err());
    }

    #[test]
    fn zero_allocation_requires_explicit_complete_evidence() {
        assert_eq!(
            evidenced_quantities(100, 120, [1, 1, 1], &[interval(1, 100, 120, [0, 0, 0])]),
            Ok([0, 0, 0])
        );
        assert!(evidenced_quantities(100, 120, [1, 1, 1], &[]).is_err());
    }
}
