//! Pure, bounded schedule computation. Cron uses local calendar time; intervals
//! use elapsed UTC time. A repeated wall time fires at its first instant only.
use crate::{
    errors::{AppError, AppResult},
    models::trigger_schedule::{IntervalUnit, ScheduleKind, ScheduleSpec},
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;
use std::str::FromStr;

fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}

pub fn timezone(value: &str) -> AppResult<Tz> {
    value.parse().map_err(|_| invalid("Unknown IANA timezone"))
}

fn cron(expression: &str) -> AppResult<Cron> {
    if expression.len() > 256 || expression.split_whitespace().count() != 5 {
        return Err(invalid(
            "Cron must have five fields: minute hour day month weekday",
        ));
    }
    Cron::from_str(expression).map_err(|_| invalid("Invalid five-field cron expression"))
}

pub fn interval_seconds(amount: u32, unit: IntervalUnit) -> i64 {
    i64::from(amount)
        * match unit {
            IntervalUnit::Minutes => 60,
            IntervalUnit::Hours => 3600,
            IntervalUnit::Days => 86400,
        }
}

pub fn validate(spec: &ScheduleSpec, minimum_minutes: i32) -> AppResult<()> {
    let minimum = i64::from(
        minimum_minutes.max(crate::models::assistant_settings::DEFAULT_SCHEDULE_MINIMUM_MINUTES),
    ) * 60;
    match &spec.timing {
        ScheduleKind::Cron {
            expression,
            timezone: zone,
        } => {
            timezone(zone)?;
            let pattern = cron(expression)?;
            // The minimum applies to the wall-clock cadence. Inspect every
            // selected minute, including midnight wrap, independently of dates.
            let mut minutes = Vec::new();
            for hour in 0..24 {
                if !pattern.pattern.hour_match(hour).unwrap_or(false) {
                    continue;
                }
                for minute in 0..60 {
                    if pattern.pattern.minute_match(minute).unwrap_or(false) {
                        minutes.push(i64::from(hour * 60 + minute));
                    }
                }
            }
            if minutes.is_empty() {
                return Err(invalid("Cron has no matching time"));
            }
            if minutes.windows(2).any(|v| (v[1] - v[0]) * 60 < minimum)
                || (minutes.len() > 1
                    && (1440 + minutes[0] - minutes[minutes.len() - 1]) * 60 < minimum
                    && matches_adjacent_dates(&pattern))
            {
                return Err(invalid(
                    "Cron cadence is below the owner's minimum interval",
                ));
            }
        }
        ScheduleKind::Every { amount, unit, .. } => {
            if *amount == 0 || interval_seconds(*amount, *unit) < minimum {
                return Err(invalid("Interval must meet the owner's minimum"));
            }
        }
        ScheduleKind::At { .. } => {}
    }
    let start = spec.start;
    let end = spec.end;
    if matches!((start, end), (Some(start), Some(end)) if end <= start) {
        return Err(invalid("Schedule end must be after start"));
    }
    if spec.max_runs.is_some_and(|n| n == 0 || n > 1_000_000) {
        return Err(invalid("max_runs must be between 1 and 1000000"));
    }
    if spec.grace_seconds.is_some_and(|n| n == 0 || n > 86400) {
        return Err(invalid("grace_seconds must be between 1 and 86400"));
    }
    Ok(())
}

fn matches_adjacent_dates(cron: &Cron) -> bool {
    // Gregorian calendar/weekday combinations repeat every 400 years. This is
    // only needed for a sub-minimum midnight gap in an otherwise valid cadence.
    let mut date = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    let mut previous = false;
    for _ in 0..=146097 {
        let matched = cron.pattern.month_match(date.month()).unwrap_or(false)
            && cron
                .pattern
                .day_match(
                    date.year(),
                    date.month(),
                    date.day(),
                    croner::Weekday::from_days_from_sunday(date.weekday().num_days_from_sunday()),
                )
                .unwrap_or(false);
        if previous && matched {
            return true;
        }
        previous = matched;
        date = date.succ_opt().unwrap();
    }
    false
}

fn cron_occurrence(
    expression: &str,
    zone: &str,
    pivot: DateTime<Utc>,
    forward: bool,
) -> AppResult<Option<DateTime<Utc>>> {
    let pattern = cron(expression)?;
    let zone = timezone(zone)?;
    let mut day = pivot.with_timezone(&zone).date_naive();
    // Evaluate local calendar matches, then resolve each to one UTC instant.
    // A gap collapses all missing matches onto its first valid instant; choosing
    // one minimum/maximum also deduplicates a real match at that instant.
    for _ in 0..=146097 {
        let date_matches = pattern.pattern.month_match(day.month()).unwrap_or(false)
            && pattern
                .pattern
                .day_match(
                    day.year(),
                    day.month(),
                    day.day(),
                    croner::Weekday::from_days_from_sunday(day.weekday().num_days_from_sunday()),
                )
                .unwrap_or(false);
        let mut best: Option<DateTime<Utc>> = None;
        if date_matches {
            for hour in 0..24 {
                if !pattern.pattern.hour_match(hour).unwrap_or(false) {
                    continue;
                }
                for minute in 0..60 {
                    if !pattern.pattern.minute_match(minute).unwrap_or(false) {
                        continue;
                    }
                    let local = day.and_hms_opt(hour, minute, 0).unwrap();
                    let candidate = resolve_wall_time(zone, local)?;
                    if (forward && candidate > pivot) || (!forward && candidate < pivot) {
                        best = Some(best.map_or(candidate, |old| {
                            if forward {
                                old.min(candidate)
                            } else {
                                old.max(candidate)
                            }
                        }));
                    }
                }
            }
        }
        if best.is_some() {
            return Ok(best);
        }
        day = if forward {
            day.succ_opt()
        } else {
            day.pred_opt()
        }
        .ok_or_else(|| invalid("Schedule exceeds supported calendar"))?;
    }
    Ok(None)
}

fn resolve_wall_time(zone: Tz, local: chrono::NaiveDateTime) -> AppResult<DateTime<Utc>> {
    if let Some(instant) = zone.from_local_datetime(&local).earliest() {
        return Ok(instant.with_timezone(&Utc));
    }
    // Exponential search followed by binary search finds the end of any IANA
    // gap, including a skipped date, without walking every second/minute.
    let mut high = 1_i64;
    while zone
        .from_local_datetime(&(local + Duration::seconds(high)))
        .earliest()
        .is_none()
    {
        high *= 2;
        if high > 172800 {
            return Err(invalid("Timezone gap exceeds two days"));
        }
    }
    let mut low = 0;
    while high - low > 1 {
        let mid = (low + high) / 2;
        if zone
            .from_local_datetime(&(local + Duration::seconds(mid)))
            .earliest()
            .is_some()
        {
            high = mid;
        } else {
            low = mid;
        }
    }
    Ok(zone
        .from_local_datetime(&(local + Duration::seconds(high)))
        .earliest()
        .ok_or_else(|| invalid("Unresolvable timezone gap"))?
        .with_timezone(&Utc))
}

fn raw(
    spec: &ScheduleSpec,
    pivot: DateTime<Utc>,
    forward: bool,
) -> AppResult<Option<DateTime<Utc>>> {
    Ok(match &spec.timing {
        ScheduleKind::Cron {
            expression,
            timezone,
        } => return cron_occurrence(expression, timezone, pivot, forward),
        ScheduleKind::At { at } => {
            let at = *at;
            ((forward && at > pivot) || (!forward && at < pivot)).then_some(at)
        }
        ScheduleKind::Every {
            amount,
            unit,
            anchor,
        } => {
            let anchor = *anchor;
            let step = interval_seconds(*amount, *unit) * 1000;
            if step == 0 {
                return Err(invalid("Interval cannot be zero"));
            }
            let delta = pivot.timestamp_millis() - anchor.timestamp_millis();
            let periods = if forward {
                delta.div_euclid(step) + 1
            } else {
                (delta - 1).div_euclid(step)
            };
            if periods < 0 {
                if forward { Some(anchor) } else { None }
            } else {
                anchor.checked_add_signed(Duration::milliseconds(periods.saturating_mul(step)))
            }
        }
    })
}

/// First occurrence strictly after `after`, with inclusive start/end limits.
pub fn next(spec: &ScheduleSpec, after: DateTime<Utc>) -> AppResult<Option<DateTime<Utc>>> {
    let pivot = spec
        .start
        .map_or(after, |s| after.max(s - Duration::milliseconds(1)));
    let value = raw(spec, pivot, true)?;
    let end = spec.end;
    Ok(value.filter(|v| end.is_none_or(|end| *v <= end)))
}

/// Most recent occurrence at or before `now` (no catch-up enumeration).
pub fn latest(spec: &ScheduleSpec, now: DateTime<Utc>) -> AppResult<Option<DateTime<Utc>>> {
    let pivot = spec.end.map_or(now, |end| now.min(end));
    let value = raw(spec, pivot + Duration::milliseconds(1), false)?;
    let start = spec.start;
    Ok(value.filter(|v| start.is_none_or(|start| *v >= start)))
}

pub fn grace(spec: &ScheduleSpec, at: DateTime<Utc>) -> AppResult<Duration> {
    if let Some(seconds) = spec.grace_seconds {
        return Ok(Duration::seconds(i64::from(seconds)));
    }
    let cadence = match &spec.timing {
        ScheduleKind::Every { amount, unit, .. } => {
            Duration::seconds(interval_seconds(*amount, *unit))
        }
        _ => raw(spec, at, true)?.map_or(Duration::hours(1), |next| next - at),
    };
    Ok(cadence.min(Duration::hours(1)))
}

pub fn preview(
    spec: &ScheduleSpec,
    after: DateTime<Utc>,
    zone: &str,
    remaining: u32,
) -> AppResult<Vec<String>> {
    let tz = timezone(zone)?;
    let mut cursor = after;
    let mut times = Vec::new();
    for _ in 0..3.min(remaining) {
        let Some(next) = next(spec, cursor)? else {
            break;
        };
        times.push(next.with_timezone(&tz).to_rfc3339());
        cursor = next;
    }
    Ok(times)
}

/// Count missed occurrences without materializing rows. Intervals are O(1);
/// cron visits local calendar matches and deduplicates collapsed DST instants.
pub fn missed_count(
    spec: &ScheduleSpec,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> AppResult<u64> {
    if until <= from {
        return Ok(0);
    }
    if let ScheduleKind::Every { amount, unit, .. } = spec.timing {
        return Ok(
            ((until - from).num_milliseconds() / (interval_seconds(amount, unit) * 1000)) as u64,
        );
    }
    let ScheduleKind::Cron {
        expression,
        timezone: zone,
    } = &spec.timing
    else {
        return Ok(1);
    };
    let pattern = cron(expression)?;
    let zone = timezone(zone)?;
    let mut day = from.with_timezone(&zone).date_naive();
    let last = until.with_timezone(&zone).date_naive();
    let mut count = 0;
    let mut previous = None;
    while day <= last {
        if pattern.pattern.month_match(day.month()).unwrap_or(false)
            && pattern
                .pattern
                .day_match(
                    day.year(),
                    day.month(),
                    day.day(),
                    croner::Weekday::from_days_from_sunday(day.weekday().num_days_from_sunday()),
                )
                .unwrap_or(false)
        {
            for hour in 0..24 {
                if !pattern.pattern.hour_match(hour).unwrap_or(false) {
                    continue;
                }
                for minute in 0..60 {
                    if !pattern.pattern.minute_match(minute).unwrap_or(false) {
                        continue;
                    }
                    let at = resolve_wall_time(zone, day.and_hms_opt(hour, minute, 0).unwrap())?;
                    if at >= from && at < until && previous != Some(at) {
                        count += 1;
                        previous = Some(at);
                    }
                }
            }
        }
        day = day
            .succ_opt()
            .ok_or_else(|| invalid("Schedule exceeds supported calendar"))?;
    }
    Ok(count)
}

/// Human-readable calendar fields alongside the authoritative run preview.
pub fn description(spec: &ScheduleSpec) -> String {
    match &spec.timing {
        ScheduleKind::Every {
            amount,
            unit,
            anchor,
        } => format!(
            "Every {amount} {}, anchored at {anchor} (elapsed time)",
            match unit {
                IntervalUnit::Minutes => "minutes",
                IntervalUnit::Hours => "hours",
                IntervalUnit::Days => "days",
            }
        ),
        ScheduleKind::At { at } => format!("Once at {at}"),
        ScheduleKind::Cron {
            expression,
            timezone,
        } => {
            let fields: Vec<_> = expression.split_whitespace().collect();
            if fields.len() != 5 {
                return format!("Calendar schedule in {timezone}");
            }
            let day = match fields[4] {
                "1-5" => "Monday–Friday".to_owned(),
                "*" => "every day".to_owned(),
                "5" => "every Friday".to_owned(),
                other => format!("weekday {other}"),
            };
            if let (Ok(hour), Ok(minute)) = (fields[1].parse::<u8>(), fields[0].parse::<u8>())
                && fields[2] == "*"
                && fields[3] == "*"
            {
                return format!("At {hour:02}:{minute:02} {day}, {timezone}");
            }
            format!(
                "Minutes {}; hours {}; day of month {}; month {}; {day}, {timezone}",
                fields[0], fields[1], fields[2], fields[3]
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec(value: serde_json::Value) -> ScheduleSpec {
        serde_json::from_value::<crate::handlers::trigger_schedule_dto::ScheduleDto>(value)
            .unwrap()
            .into()
    }
    fn t(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }
    #[test]
    fn cron_interval_at_and_limits() {
        let weekday = spec(
            serde_json::json!({"kind":"cron","expression":"0 8 * * 1-5","timezone":"Asia/Singapore"}),
        );
        validate(&weekday, 5).unwrap();
        assert_eq!(
            preview(
                &weekday,
                t("2026-10-02T08:00:00+08:00"),
                "Asia/Singapore",
                3
            )
            .unwrap(),
            vec![
                "2026-10-05T08:00:00+08:00",
                "2026-10-06T08:00:00+08:00",
                "2026-10-07T08:00:00+08:00"
            ]
        );
        let every = spec(
            serde_json::json!({"kind":"every","amount":5,"unit":"minutes","anchor":"2026-10-01T00:00:00Z","end":"2026-10-01T00:10:00Z"}),
        );
        assert_eq!(
            next(&every, t("2026-10-01T00:00:00Z")).unwrap(),
            Some(t("2026-10-01T00:05:00Z"))
        );
        assert_eq!(
            latest(&every, t("2026-10-01T00:10:00Z")).unwrap(),
            Some(t("2026-10-01T00:10:00Z"))
        );
        assert!(next(&every, t("2026-10-01T00:10:00Z")).unwrap().is_none());
        assert_eq!(
            grace(&every, t("2026-10-01T00:00:00Z")).unwrap(),
            Duration::minutes(5)
        );
        let at = spec(serde_json::json!({"kind":"at","at":"2026-10-01T12:00:00+08:00"}));
        assert_eq!(
            preview(&at, t("2026-10-01T00:00:00Z"), "Asia/Singapore", 3)
                .unwrap()
                .len(),
            1
        );
        assert!(next(&at, t("2026-10-01T04:00:00Z")).unwrap().is_none());
    }
    #[test]
    fn dst_spring_gap_and_fall_fold_fire_once() {
        let gap = spec(
            serde_json::json!({"kind":"cron","expression":"30 2 * * *","timezone":"America/New_York"}),
        );
        assert_eq!(
            next(&gap, t("2026-03-07T08:00:00Z")).unwrap(),
            Some(t("2026-03-08T07:00:00Z"))
        );
        for expression in ["*/15 2 * * *", "*/15 2,3 * * *"] {
            let collapsed = spec(
                serde_json::json!({"kind":"cron","expression":expression,"timezone":"America/New_York"}),
            );
            assert_eq!(
                next(&collapsed, t("2026-03-08T06:59:00Z")).unwrap(),
                Some(t("2026-03-08T07:00:00Z"))
            );
            assert_eq!(
                latest(&collapsed, t("2026-03-08T07:00:00Z")).unwrap(),
                Some(t("2026-03-08T07:00:00Z"))
            );
            assert!(
                next(&collapsed, t("2026-03-08T07:00:00Z"))
                    .unwrap()
                    .unwrap()
                    > t("2026-03-08T07:00:00Z")
            );
        }
        let fold = spec(
            serde_json::json!({"kind":"cron","expression":"30 1 * * *","timezone":"America/New_York"}),
        );
        assert_eq!(
            next(&fold, t("2026-11-01T00:00:00Z")).unwrap(),
            Some(t("2026-11-01T05:30:00Z"))
        );
        assert_eq!(
            next(&fold, t("2026-11-01T05:30:00Z")).unwrap(),
            Some(t("2026-11-02T06:30:00Z"))
        );
        assert_eq!(
            latest(&fold, t("2026-11-01T06:45:00Z")).unwrap(),
            Some(t("2026-11-01T05:30:00Z"))
        );
        let interval = spec(
            serde_json::json!({"kind":"every","amount":24,"unit":"hours","anchor":"2026-03-07T07:30:00Z"}),
        );
        assert_eq!(
            next(&interval, t("2026-03-07T07:30:00Z")).unwrap(),
            Some(t("2026-03-08T07:30:00Z"))
        );
    }
    #[test]
    fn invalid_timezone_cron_and_minimum_are_rejected() {
        for value in [
            serde_json::json!({"kind":"cron","expression":"* * * * *","timezone":"UTC"}),
            serde_json::json!({"kind":"cron","expression":"0 8 * * *","timezone":"Mars/Olympus"}),
            serde_json::json!({"kind":"cron","expression":"0 0 8 * * *","timezone":"UTC"}),
            serde_json::json!({"kind":"every","amount":4,"unit":"minutes","anchor":"2026-01-01T00:00:00Z"}),
        ] {
            assert!(validate(&spec(value), 5).is_err());
        }
        let five = spec(
            serde_json::json!({"kind":"every","amount":5,"unit":"minutes","anchor":"2026-01-01T00:00:00Z"}),
        );
        assert!(validate(&five, 10).is_err());
    }
    #[test]
    fn minimum_dates_and_final_interval_grace() {
        let sparse =
            spec(serde_json::json!({"kind":"cron","expression":"0 0,23 * * 1","timezone":"UTC"}));
        assert!(validate(&sparse, 120).is_ok());
        let daily =
            spec(serde_json::json!({"kind":"cron","expression":"0 0,23 * * *","timezone":"UTC"}));
        assert!(validate(&daily, 120).is_err());
        let final_run = spec(
            serde_json::json!({"kind":"every","amount":5,"unit":"minutes","anchor":"2026-10-01T00:00:00Z","end":"2026-10-01T00:10:00Z"}),
        );
        assert_eq!(
            grace(&final_run, t("2026-10-01T00:10:00Z")).unwrap(),
            Duration::minutes(5)
        );
        for limits in [
            serde_json::json!({"max_runs":0}),
            serde_json::json!({"grace_seconds":0}),
            serde_json::json!({"start":"2026-10-02T00:00:00Z","end":"2026-10-01T00:00:00Z"}),
        ] {
            let mut value = serde_json::json!({"kind":"at","at":"2026-10-03T00:00:00Z"});
            value
                .as_object_mut()
                .unwrap()
                .extend(limits.as_object().unwrap().clone());
            assert!(validate(&spec(value), 5).is_err());
        }
    }
    #[test]
    fn next_run_timing_benchmark() {
        let schedule = spec(
            serde_json::json!({"kind":"cron","expression":"0 8 * * 1-5","timezone":"Asia/Singapore"}),
        );
        let now = t("2026-10-01T00:00:00Z");
        let start = std::time::Instant::now();
        for _ in 0..10_000 {
            std::hint::black_box(next(&schedule, now).unwrap());
        }
        eprintln!(
            "S7 next cron: {:.3} us/op (10000 iterations, debug)",
            start.elapsed().as_secs_f64() * 1_000_000.0 / 10000.0
        );
    }
}
