use std::str::FromStr;

use chrono::{
    DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc,
};
use chrono_tz::Tz;
use thiserror::Error;

use crate::model::{
    ActivePrompt, CompanionSettings, CompanionSnapshot, HistoryAction, HistoryEntry, MealId, Mood,
    ValidationError, validate_time,
};

const DUE_WINDOW_MINUTES: i64 = 45;
pub const DEFAULT_SNOOZE_MINUTES: u32 = 10;
pub const MAX_SNOOZE_MINUTES: u32 = 120;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("{0}")]
    Validation(#[from] ValidationError),
    #[error("there is no active meal reminder")]
    NoActivePrompt,
    #[error("the active reminder is for {expected}, not {actual}")]
    WrongMeal { expected: MealId, actual: MealId },
    #[error("snooze minutes must be between 1 and {MAX_SNOOZE_MINUTES}")]
    InvalidSnooze,
    #[error("choiceId must contain between 1 and 160 characters")]
    InvalidChoiceId,
    #[error("no configured meal is available for a demo reminder")]
    NoDemoMeal,
}

#[derive(Debug, Clone)]
pub struct CompanionEngine {
    snapshot: CompanionSnapshot,
}

struct HistoryRecord {
    action: HistoryAction,
    choice_id: Option<String>,
    mood: Option<Mood>,
}

impl CompanionEngine {
    pub fn new(snapshot: CompanionSnapshot) -> Result<Self, SchedulerError> {
        snapshot.validate()?;
        Ok(Self { snapshot })
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> &CompanionSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> CompanionSnapshot {
        self.snapshot
    }

    pub fn replace_settings(&mut self, settings: CompanionSettings) -> Result<(), SchedulerError> {
        settings.validate()?;
        self.snapshot.settings = settings;

        let prompt_unavailable = self
            .snapshot
            .runtime
            .active_prompt
            .as_ref()
            .is_some_and(|prompt| !meal_enabled(&self.snapshot.settings, prompt.meal_id));
        let snoozed_meal_unavailable = self.snapshot.runtime.snoozed_until.is_some()
            && self
                .snapshot
                .runtime
                .last_prompt_key
                .as_deref()
                .and_then(meal_id_from_prompt_key)
                .is_none_or(|meal_id| !meal_enabled(&self.snapshot.settings, meal_id));
        if self.snapshot.settings.quiet_mode || prompt_unavailable || snoozed_meal_unavailable {
            self.snapshot.runtime.active_prompt = None;
            self.snapshot.runtime.snoozed_until = None;
        }
        Ok(())
    }

    pub fn refresh_timezone(&mut self, detected_timezone: &str) -> bool {
        if !self.snapshot.settings.refresh_timezone(detected_timezone) {
            return false;
        }

        self.snapshot.runtime.active_prompt = None;
        self.snapshot.runtime.snoozed_until = None;
        self.snapshot.runtime.last_prompt_key = None;
        true
    }

    pub fn set_quiet_mode(&mut self, quiet_mode: bool) {
        self.snapshot.settings.quiet_mode = quiet_mode;
        if quiet_mode {
            self.snapshot.runtime.active_prompt = None;
            self.snapshot.runtime.snoozed_until = None;
        }
    }

    pub fn tick(&mut self, now: DateTime<Utc>) -> Option<ActivePrompt> {
        if !self.snapshot.settings.onboarding_complete || self.snapshot.settings.quiet_mode {
            return None;
        }

        if self
            .snapshot
            .runtime
            .active_prompt
            .as_ref()
            .is_some_and(|prompt| active_prompt_is_current(prompt, now))
        {
            return None;
        }
        self.snapshot.runtime.active_prompt = None;

        if let Some(snoozed_until) = self.snapshot.runtime.snoozed_until.clone() {
            let parsed = DateTime::parse_from_rfc3339(&snoozed_until)
                .ok()
                .map(|value| value.with_timezone(&Utc));
            match parsed {
                Some(until) if now < until => return None,
                Some(until) if now <= until + Duration::minutes(DUE_WINDOW_MINUTES) => {
                    let meal_id = self
                        .snapshot
                        .runtime
                        .last_prompt_key
                        .as_deref()
                        .and_then(meal_id_from_prompt_key);
                    self.snapshot.runtime.snoozed_until = None;
                    if let Some(meal_id) =
                        meal_id.filter(|id| meal_enabled(&self.snapshot.settings, *id))
                    {
                        let prompt = ActivePrompt {
                            meal_id,
                            due_at: rfc3339(until),
                        };
                        self.snapshot.runtime.active_prompt = Some(prompt.clone());
                        return Some(prompt);
                    }
                }
                Some(_) => {
                    self.snapshot.runtime.snoozed_until = None;
                }
                None => {
                    self.snapshot.runtime.snoozed_until = None;
                }
            }
        }

        let timezone = self.snapshot.settings.timezone.parse::<Tz>().ok()?;
        let local_now = now.with_timezone(&timezone);
        let today = local_now.date_naive();
        let dates = [today - Duration::days(1), today];
        let mut candidates = Vec::new();
        for date in dates {
            for meal in self
                .snapshot
                .settings
                .meals
                .iter()
                .filter(|meal| meal.enabled)
            {
                let Ok(time) = validate_time(&meal.time) else {
                    continue;
                };
                let Some(scheduled) = resolve_local_datetime(timezone, date.and_time(time)) else {
                    continue;
                };
                let scheduled_utc = scheduled.with_timezone(&Utc);
                let window_end = scheduled_utc + Duration::minutes(DUE_WINDOW_MINUTES);
                if now < scheduled_utc || now > window_end {
                    continue;
                }

                let prompt_key = scheduled_prompt_key(date, meal.id);
                if self.snapshot.runtime.last_prompt_key.as_deref() == Some(&prompt_key)
                    || has_history(&self.snapshot, meal.id, date)
                {
                    continue;
                }
                candidates.push((scheduled_utc, prompt_key, meal.id));
            }
        }

        candidates.sort_by_key(|(scheduled, _, _)| *scheduled);
        let (scheduled, prompt_key, meal_id) = candidates.pop()?;
        let prompt = ActivePrompt {
            meal_id,
            due_at: rfc3339(scheduled),
        };
        self.snapshot.runtime.active_prompt = Some(prompt.clone());
        self.snapshot.runtime.last_prompt_key = Some(prompt_key);
        Some(prompt)
    }

    pub fn snooze(
        &mut self,
        meal_id: MealId,
        minutes: u32,
        now: DateTime<Utc>,
    ) -> Result<(), SchedulerError> {
        if !(1..=MAX_SNOOZE_MINUTES).contains(&minutes) {
            return Err(SchedulerError::InvalidSnooze);
        }
        self.require_active(meal_id)?;
        self.snapshot.runtime.active_prompt = None;
        self.snapshot.runtime.snoozed_until =
            Some(rfc3339(now + Duration::minutes(i64::from(minutes))));
        Ok(())
    }

    pub fn skip(&mut self, meal_id: MealId, now: DateTime<Utc>) -> Result<(), SchedulerError> {
        let active = self.require_active(meal_id)?.clone();
        self.record_history(
            meal_id,
            active.due_at,
            HistoryRecord {
                action: HistoryAction::Skipped,
                choice_id: None,
                mood: None,
            },
            now,
        );
        self.clear_prompt();
        Ok(())
    }

    pub fn complete(
        &mut self,
        meal_id: MealId,
        choice_id: Option<String>,
        mood: Option<Mood>,
        now: DateTime<Utc>,
    ) -> Result<(), SchedulerError> {
        let active = self.require_active(meal_id)?.clone();
        let choice_id = validate_choice_id(choice_id)?;
        self.record_history(
            meal_id,
            active.due_at,
            HistoryRecord {
                action: HistoryAction::Accepted,
                choice_id,
                mood,
            },
            now,
        );
        self.clear_prompt();
        Ok(())
    }

    pub fn dislike(
        &mut self,
        meal_id: MealId,
        choice_id: String,
        mood: Option<Mood>,
        now: DateTime<Utc>,
    ) -> Result<(), SchedulerError> {
        let active = self.require_active(meal_id)?.clone();
        let choice_id = validate_choice_id(Some(choice_id))?;
        self.record_history(
            meal_id,
            active.due_at,
            HistoryRecord {
                action: HistoryAction::Disliked,
                choice_id,
                mood,
            },
            now,
        );
        Ok(())
    }

    pub fn trigger_demo(
        &mut self,
        requested: Option<MealId>,
        now: DateTime<Utc>,
    ) -> Result<ActivePrompt, SchedulerError> {
        let meal_id = requested
            .or_else(|| {
                self.snapshot
                    .settings
                    .meals
                    .iter()
                    .find(|meal| meal.enabled)
                    .map(|meal| meal.id)
            })
            .ok_or(SchedulerError::NoDemoMeal)?;
        if !self
            .snapshot
            .settings
            .meals
            .iter()
            .any(|meal| meal.id == meal_id)
        {
            return Err(SchedulerError::NoDemoMeal);
        }

        let prompt = ActivePrompt {
            meal_id,
            due_at: rfc3339(now),
        };
        self.snapshot.runtime.active_prompt = Some(prompt.clone());
        self.snapshot.runtime.snoozed_until = None;
        self.snapshot.runtime.last_prompt_key =
            Some(format!("demo:{}:{}", meal_id, now.timestamp_millis()));
        Ok(prompt)
    }

    fn require_active(&self, meal_id: MealId) -> Result<&ActivePrompt, SchedulerError> {
        let active = self
            .snapshot
            .runtime
            .active_prompt
            .as_ref()
            .ok_or(SchedulerError::NoActivePrompt)?;
        if active.meal_id != meal_id {
            return Err(SchedulerError::WrongMeal {
                expected: active.meal_id,
                actual: meal_id,
            });
        }
        Ok(active)
    }

    fn record_history(
        &mut self,
        meal_id: MealId,
        due_at: String,
        record: HistoryRecord,
        now: DateTime<Utc>,
    ) {
        if self
            .snapshot
            .runtime
            .last_prompt_key
            .as_deref()
            .is_some_and(|key| key.starts_with("demo:"))
        {
            return;
        }

        let timezone = self
            .snapshot
            .settings
            .timezone
            .parse::<Tz>()
            .unwrap_or(chrono_tz::UTC);
        let date_key = self
            .snapshot
            .runtime
            .last_prompt_key
            .as_deref()
            .and_then(|key| date_key_from_prompt_key(key, meal_id))
            .or_else(|| {
                DateTime::parse_from_rfc3339(&due_at).ok().map(|value| {
                    value
                        .with_timezone(&timezone)
                        .format("%Y-%m-%d")
                        .to_string()
                })
            })
            .unwrap_or_else(|| now.with_timezone(&timezone).format("%Y-%m-%d").to_string());

        if record.action == HistoryAction::Disliked
            && self.snapshot.history.iter().any(|entry| {
                entry.meal_id == meal_id
                    && entry.date_key == date_key
                    && entry.action == HistoryAction::Disliked
                    && entry.choice_id == record.choice_id
            })
        {
            return;
        }
        self.snapshot.history.push(HistoryEntry {
            meal_id,
            date_key,
            action: record.action,
            choice_id: record.choice_id,
            mood: record.mood,
            at: rfc3339(now),
        });
        self.snapshot.trim_history();
    }

    fn clear_prompt(&mut self) {
        self.snapshot.runtime.active_prompt = None;
        self.snapshot.runtime.snoozed_until = None;
    }
}

fn resolve_local_datetime(timezone: Tz, naive: NaiveDateTime) -> Option<DateTime<Tz>> {
    match timezone.from_local_datetime(&naive) {
        LocalResult::Single(value) => Some(value),
        LocalResult::Ambiguous(first, second) => Some(first.min(second)),
        LocalResult::None => (1..=180).find_map(|minutes| {
            let shifted = naive + Duration::minutes(minutes);
            match timezone.from_local_datetime(&shifted) {
                LocalResult::Single(value) => Some(value),
                LocalResult::Ambiguous(first, second) => Some(first.min(second)),
                LocalResult::None => None,
            }
        }),
    }
}

fn scheduled_prompt_key(date: NaiveDate, meal_id: MealId) -> String {
    format!("{date}:{meal_id}")
}

fn active_prompt_is_current(prompt: &ActivePrompt, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(&prompt.due_at)
        .map(|due_at| now <= due_at.with_timezone(&Utc) + Duration::minutes(DUE_WINDOW_MINUTES))
        .unwrap_or(false)
}

fn meal_enabled(settings: &CompanionSettings, meal_id: MealId) -> bool {
    settings
        .meals
        .iter()
        .any(|meal| meal.id == meal_id && meal.enabled)
}

fn meal_id_from_prompt_key(key: &str) -> Option<MealId> {
    if let Some(rest) = key.strip_prefix("demo:") {
        return rest
            .split(':')
            .next()
            .and_then(|value| MealId::from_str(value).ok());
    }
    key.rsplit_once(':')
        .and_then(|(_, value)| MealId::from_str(value).ok())
}

fn date_key_from_prompt_key(key: &str, meal_id: MealId) -> Option<String> {
    let (date_key, encoded_meal_id) = key.rsplit_once(':')?;
    if MealId::from_str(encoded_meal_id).ok()? != meal_id {
        return None;
    }
    NaiveDate::parse_from_str(date_key, "%Y-%m-%d")
        .ok()
        .map(|date| date.format("%Y-%m-%d").to_string())
}

fn has_history(snapshot: &CompanionSnapshot, meal_id: MealId, date: NaiveDate) -> bool {
    let date_key = date.format("%Y-%m-%d").to_string();
    snapshot.history.iter().any(|entry| {
        entry.meal_id == meal_id
            && entry.date_key == date_key
            && matches!(
                entry.action,
                HistoryAction::Accepted | HistoryAction::Skipped
            )
    })
}

fn validate_choice_id(choice_id: Option<String>) -> Result<Option<String>, SchedulerError> {
    choice_id
        .map(|value| {
            let trimmed = value.trim();
            let length = trimmed.chars().count();
            if !(1..=160).contains(&length) || trimmed.chars().any(char::is_control) {
                return Err(SchedulerError::InvalidChoiceId);
            }
            Ok(trimmed.to_owned())
        })
        .transpose()
}

fn rfc3339(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn engine_with_breakfast_at(time: &str) -> CompanionEngine {
        let mut snapshot = CompanionSnapshot::default();
        snapshot.settings.timezone = "UTC".to_owned();
        snapshot.settings.onboarding_complete = true;
        for meal in &mut snapshot.settings.meals {
            meal.enabled = meal.id == MealId::Breakfast;
            if meal.id == MealId::Breakfast {
                meal.time = time.to_owned();
            }
        }
        CompanionEngine::new(snapshot).unwrap()
    }

    #[test]
    fn prompts_only_inside_the_due_window() {
        let before = Utc.with_ymd_and_hms(2026, 10, 9, 7, 59, 59).unwrap();
        let inside = Utc.with_ymd_and_hms(2026, 10, 9, 8, 44, 59).unwrap();
        let at_end = Utc.with_ymd_and_hms(2026, 10, 9, 8, 45, 0).unwrap();

        assert!(engine_with_breakfast_at("08:00").tick(before).is_none());
        assert!(engine_with_breakfast_at("08:00").tick(inside).is_some());
        assert!(engine_with_breakfast_at("08:00").tick(at_end).is_some());
    }

    #[test]
    fn prompts_for_a_previous_day_meal_across_midnight() {
        let after_midnight = Utc.with_ymd_and_hms(2026, 10, 10, 0, 10, 0).unwrap();
        let prompt = engine_with_breakfast_at("23:50")
            .tick(after_midnight)
            .unwrap();

        assert_eq!(prompt.meal_id, MealId::Breakfast);
        assert_eq!(prompt.due_at, "2026-10-09T23:50:00Z");
    }

    #[test]
    fn onboarding_suppresses_scheduled_prompts() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        let mut snapshot = engine.into_snapshot();
        snapshot.settings.onboarding_complete = false;
        engine = CompanionEngine::new(snapshot).unwrap();

        assert!(engine.tick(due).is_none());
    }

    #[test]
    fn snooze_reissues_once_at_the_requested_time() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        assert!(engine.tick(due).is_some());
        engine.snooze(MealId::Breakfast, 10, due).unwrap();

        assert!(engine.tick(due + Duration::minutes(9)).is_none());
        let prompt = engine.tick(due + Duration::minutes(10)).unwrap();
        assert_eq!(prompt.meal_id, MealId::Breakfast);
        assert!(engine.tick(due + Duration::minutes(11)).is_none());
    }

    #[test]
    fn stale_breakfast_snooze_does_not_block_current_lunch() {
        let breakfast = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let lunch = Utc.with_ymd_and_hms(2026, 10, 9, 12, 31, 0).unwrap();
        let mut snapshot = CompanionSnapshot::default();
        snapshot.settings.timezone = "UTC".to_owned();
        snapshot.settings.onboarding_complete = true;
        snapshot.settings.meals[2].enabled = false;
        let mut engine = CompanionEngine::new(snapshot).unwrap();

        assert_eq!(engine.tick(breakfast).unwrap().meal_id, MealId::Breakfast);
        engine
            .snooze(MealId::Breakfast, DEFAULT_SNOOZE_MINUTES, breakfast)
            .unwrap();

        let prompt = engine.tick(lunch).unwrap();

        assert_eq!(prompt.meal_id, MealId::Lunch);
        assert!(engine.snapshot().runtime.snoozed_until.is_none());
        assert_eq!(
            engine.snapshot().runtime.last_prompt_key.as_deref(),
            Some("2026-10-09:lunch")
        );
    }

    #[test]
    fn cross_midnight_snooze_preserves_the_original_meal_date() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 23, 55, 0).unwrap();
        let mut engine = engine_with_breakfast_at("23:50");
        engine.tick(due).unwrap();
        engine.snooze(MealId::Breakfast, 20, due).unwrap();
        let snoozed = due + Duration::minutes(20);
        engine.tick(snoozed).unwrap();
        engine
            .complete(MealId::Breakfast, Some("toast".to_owned()), None, snoozed)
            .unwrap();

        assert_eq!(engine.snapshot().history[0].date_key, "2026-10-09");
        let next_day = Utc.with_ymd_and_hms(2026, 10, 10, 23, 50, 0).unwrap();
        assert!(engine.tick(next_day).is_some());
    }

    #[test]
    fn disabling_a_snoozed_meal_clears_the_snooze() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        engine.tick(due).unwrap();
        engine.snooze(MealId::Breakfast, 10, due).unwrap();
        let mut settings = engine.snapshot().settings.clone();
        settings.meals[0].enabled = false;

        engine.replace_settings(settings).unwrap();

        assert!(engine.snapshot().runtime.active_prompt.is_none());
        assert!(engine.snapshot().runtime.snoozed_until.is_none());
        assert!(engine.tick(due + Duration::minutes(10)).is_none());
    }

    #[test]
    fn refreshing_timezone_clears_runtime_state_from_the_old_local_day() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        engine.tick(due).unwrap();
        engine.snooze(MealId::Breakfast, 10, due).unwrap();

        assert!(engine.refresh_timezone("Asia/Shanghai"));

        assert_eq!(engine.snapshot().settings.timezone, "Asia/Shanghai");
        assert!(engine.snapshot().runtime.active_prompt.is_none());
        assert!(engine.snapshot().runtime.snoozed_until.is_none());
        assert!(engine.snapshot().runtime.last_prompt_key.is_none());
        assert!(!engine.refresh_timezone("Asia/Shanghai"));
        assert!(!engine.refresh_timezone("Mars/Olympus"));
        assert_eq!(engine.snapshot().settings.timezone, "Asia/Shanghai");
    }

    #[test]
    fn stale_prompt_does_not_block_a_later_meal() {
        let breakfast = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let lunch = Utc.with_ymd_and_hms(2026, 10, 9, 12, 30, 0).unwrap();
        let mut snapshot = CompanionSnapshot::default();
        snapshot.settings.timezone = "UTC".to_owned();
        snapshot.settings.onboarding_complete = true;
        snapshot.settings.meals[2].enabled = false;
        let mut engine = CompanionEngine::new(snapshot).unwrap();

        assert_eq!(engine.tick(breakfast).unwrap().meal_id, MealId::Breakfast);
        assert_eq!(engine.tick(lunch).unwrap().meal_id, MealId::Lunch);
    }

    #[test]
    fn skip_records_history_and_deduplicates_the_window() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        engine.tick(due).unwrap();
        engine.skip(MealId::Breakfast, due).unwrap();

        assert_eq!(engine.snapshot().history.len(), 1);
        assert_eq!(engine.snapshot().history[0].action, HistoryAction::Skipped);
        assert!(engine.tick(due + Duration::minutes(1)).is_none());
    }

    #[test]
    fn complete_records_the_contract_fields() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        engine.tick(due).unwrap();
        engine
            .complete(
                MealId::Breakfast,
                Some("oatmeal".to_owned()),
                Some(Mood::Balanced),
                due,
            )
            .unwrap();

        let history = &engine.snapshot().history[0];
        assert_eq!(history.date_key, "2026-10-09");
        assert_eq!(history.action, HistoryAction::Accepted);
        assert_eq!(history.choice_id.as_deref(), Some("oatmeal"));
        assert_eq!(history.mood, Some(Mood::Balanced));
    }

    #[test]
    fn dislike_records_feedback_and_keeps_the_prompt_open() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        let prompt = engine.tick(due).unwrap();

        let seed = HistoryEntry {
            meal_id: MealId::Lunch,
            date_key: "2026-10-08".to_owned(),
            action: HistoryAction::Disliked,
            choice_id: Some("seed".to_owned()),
            mood: None,
            at: "2026-10-08T12:00:00Z".to_owned(),
        };
        engine.snapshot.history = vec![seed; crate::model::MAX_HISTORY_ENTRIES];

        engine
            .dislike(
                MealId::Breakfast,
                "cold-oatmeal".to_owned(),
                Some(Mood::Light),
                due + Duration::minutes(1),
            )
            .unwrap();

        let history = engine.snapshot().history.last().unwrap();
        assert_eq!(
            engine.snapshot().history.len(),
            crate::model::MAX_HISTORY_ENTRIES
        );
        assert_eq!(history.action, HistoryAction::Disliked);
        assert_eq!(history.choice_id.as_deref(), Some("cold-oatmeal"));
        assert_eq!(engine.snapshot().runtime.active_prompt, Some(prompt));
    }

    #[test]
    fn repeated_dislike_for_the_same_choice_is_idempotent() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        engine.tick(due).unwrap();

        for _ in 0..2 {
            engine
                .dislike(
                    MealId::Breakfast,
                    "cold-oatmeal".to_owned(),
                    Some(Mood::Light),
                    due,
                )
                .unwrap();
        }

        assert_eq!(engine.snapshot().history.len(), 1);
    }

    #[test]
    fn disliked_history_does_not_settle_the_meal() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        engine.snapshot.history.push(HistoryEntry {
            meal_id: MealId::Breakfast,
            date_key: "2026-10-09".to_owned(),
            action: HistoryAction::Disliked,
            choice_id: Some("toast".to_owned()),
            mood: None,
            at: "2026-10-09T08:01:00Z".to_owned(),
        });

        assert!(engine.tick(due).is_some());
    }

    #[test]
    fn demo_dislike_keeps_prompt_without_polluting_history() {
        let now = Utc.with_ymd_and_hms(2026, 10, 9, 7, 0, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        let prompt = engine.trigger_demo(Some(MealId::Breakfast), now).unwrap();

        engine
            .dislike(
                MealId::Breakfast,
                "toast".to_owned(),
                Some(Mood::Balanced),
                now,
            )
            .unwrap();

        assert!(engine.snapshot().history.is_empty());
        assert_eq!(engine.snapshot().runtime.active_prompt, Some(prompt));
    }

    #[test]
    fn persisted_last_prompt_key_prevents_duplicate_after_restart() {
        let due = Utc.with_ymd_and_hms(2026, 10, 9, 8, 5, 0).unwrap();
        let mut engine = engine_with_breakfast_at("08:00");
        engine.tick(due).unwrap();
        let mut snapshot = engine.into_snapshot();
        snapshot.runtime.active_prompt = None;

        let mut restarted = CompanionEngine::new(snapshot).unwrap();
        assert!(restarted.tick(due + Duration::minutes(1)).is_none());
    }
}
