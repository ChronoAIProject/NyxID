use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecurringUtcPeriod {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UtcPeriodWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

pub fn recurring_utc_window(recurrence: RecurringUtcPeriod, now: DateTime<Utc>) -> UtcPeriodWindow {
    let day = now.date_naive();
    let day_start = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight is valid"));
    match recurrence {
        RecurringUtcPeriod::Daily => UtcPeriodWindow {
            start: day_start,
            end: day_start + Duration::days(1),
        },
        RecurringUtcPeriod::Weekly => {
            let start = day_start - Duration::days(i64::from(day.weekday().num_days_from_monday()));
            UtcPeriodWindow {
                start,
                end: start + Duration::days(7),
            }
        }
        RecurringUtcPeriod::Monthly => {
            let start_date = day.with_day(1).expect("every month has a first day");
            let (next_year, next_month) = if day.month() == 12 {
                (day.year() + 1, 1)
            } else {
                (day.year(), day.month() + 1)
            };
            let next_date = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
                .expect("next month is valid");
            UtcPeriodWindow {
                start: Utc.from_utc_datetime(
                    &start_date.and_hms_opt(0, 0, 0).expect("midnight is valid"),
                ),
                end: Utc
                    .from_utc_datetime(&next_date.and_hms_opt(0, 0, 0).expect("midnight is valid")),
            }
        }
    }
}
