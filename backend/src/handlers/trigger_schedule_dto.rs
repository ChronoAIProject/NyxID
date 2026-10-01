//! HTTP and tool schedule specifications: RFC3339 at the API boundary only.
use crate::models::trigger_schedule::{IntervalUnit, ScheduleKind, ScheduleSpec};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleTimingDto {
    Cron {
        expression: String,
        timezone: String,
    },
    Every {
        amount: u32,
        unit: IntervalUnit,
        anchor: DateTime<Utc>,
    },
    At {
        at: DateTime<Utc>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScheduleDto {
    #[serde(flatten)]
    pub timing: ScheduleTimingDto,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub max_runs: Option<u32>,
    pub grace_seconds: Option<u32>,
}

impl From<ScheduleDto> for ScheduleSpec {
    fn from(value: ScheduleDto) -> Self {
        Self {
            timing: match value.timing {
                ScheduleTimingDto::Cron {
                    expression,
                    timezone,
                } => ScheduleKind::Cron {
                    expression,
                    timezone,
                },
                ScheduleTimingDto::Every {
                    amount,
                    unit,
                    anchor,
                } => ScheduleKind::Every {
                    amount,
                    unit,
                    anchor,
                },
                ScheduleTimingDto::At { at } => ScheduleKind::At { at },
            },
            start: value.start,
            end: value.end,
            max_runs: value.max_runs,
            grace_seconds: value.grace_seconds,
        }
    }
}

impl From<ScheduleSpec> for ScheduleDto {
    fn from(value: ScheduleSpec) -> Self {
        Self {
            timing: match value.timing {
                ScheduleKind::Cron {
                    expression,
                    timezone,
                } => ScheduleTimingDto::Cron {
                    expression,
                    timezone,
                },
                ScheduleKind::Every {
                    amount,
                    unit,
                    anchor,
                } => ScheduleTimingDto::Every {
                    amount,
                    unit,
                    anchor,
                },
                ScheduleKind::At { at } => ScheduleTimingDto::At { at },
            },
            start: value.start,
            end: value.end,
            max_runs: value.max_runs,
            grace_seconds: value.grace_seconds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::Bson;
    use serde_json::json;

    #[test]
    fn api_offsets_become_utc_bson_dates_and_round_trip_as_rfc3339() {
        for value in [
            json!({"kind": "at", "at": "2026-10-01T08:00:00+08:00", "start": "2026-10-01T07:00:00+08:00", "end": "2026-10-02T08:00:00+08:00"}),
            json!({"kind": "every", "amount": 1, "unit": "hours", "anchor": "2026-10-01T08:00:00+08:00"}),
        ] {
            let dto: ScheduleDto = serde_json::from_value(value.clone()).unwrap();
            let model: ScheduleSpec = dto.into();
            let stored = bson::to_document(&model).unwrap();
            for key in ["at", "anchor", "start", "end"] {
                if value.get(key).is_some() {
                    assert!(matches!(stored.get(key), Some(Bson::DateTime(_))));
                }
            }
            let restored: ScheduleSpec = bson::from_document(stored).unwrap();
            assert_eq!(restored, model);
            let api = serde_json::to_value(ScheduleDto::from(restored)).unwrap();
            let key = if value["kind"] == "at" {
                "at"
            } else {
                "anchor"
            };
            assert_eq!(api[key], "2026-10-01T00:00:00Z");
        }
        assert!(
            serde_json::from_value::<ScheduleDto>(json!({"kind": "at", "at": "2026-10-01"}))
                .is_err()
        );
    }
}
