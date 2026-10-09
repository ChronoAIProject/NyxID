use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, NaiveTime};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_HISTORY_ENTRIES: usize = 120;
pub const MAX_DIETARY_TAGS: usize = 12;
pub const MAX_AVOID_TAGS: usize = 20;
pub const MAX_SNAPSHOT_BYTES: usize = 128 * 1024;

const MAX_NAME_CHARS: usize = 80;
const MAX_LABEL_CHARS: usize = 48;
const MAX_TAG_CHARS: usize = 48;
const MAX_CHOICE_ID_CHARS: usize = 160;
const MAX_PROMPT_KEY_CHARS: usize = 96;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MealId {
    Breakfast,
    Lunch,
    Dinner,
}

impl MealId {
    pub const ALL: [Self; 3] = [Self::Breakfast, Self::Lunch, Self::Dinner];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Breakfast => "breakfast",
            Self::Lunch => "lunch",
            Self::Dinner => "dinner",
        }
    }
}

impl fmt::Display for MealId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MealId {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "breakfast" => Ok(Self::Breakfast),
            "lunch" => Ok(Self::Lunch),
            "dinner" => Ok(Self::Dinner),
            _ => Err(ValidationError::new("unknown meal id")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Budget {
    Low,
    Everyday,
    Flexible,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HistoryAction {
    Accepted,
    Skipped,
    Disliked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    Light,
    Balanced,
    Treat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MealSetting {
    pub id: MealId,
    pub label: String,
    pub time: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionSettings {
    pub companion_name: String,
    pub user_name: String,
    pub timezone: String,
    pub budget: Budget,
    pub dietary: Vec<String>,
    pub avoid: Vec<String>,
    pub quiet_mode: bool,
    pub onboarding_complete: bool,
    pub meals: Vec<MealSetting>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryEntry {
    pub meal_id: MealId,
    pub date_key: String,
    pub action: HistoryAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mood: Option<Mood>,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivePrompt {
    pub meal_id: MealId,
    pub due_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_prompt: Option<ActivePrompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompanionSnapshot {
    pub schema_version: u32,
    pub settings: CompanionSettings,
    pub history: Vec<HistoryEntry>,
    pub runtime: CompanionRuntime,
}

impl Default for CompanionSnapshot {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            settings: CompanionSettings::default(),
            history: Vec::new(),
            runtime: CompanionRuntime::default(),
        }
    }
}

impl Default for CompanionSettings {
    fn default() -> Self {
        let timezone = iana_time_zone::get_timezone()
            .ok()
            .filter(|value| value.parse::<Tz>().is_ok())
            .unwrap_or_else(|| "UTC".to_owned());

        Self {
            companion_name: "Nyx".to_owned(),
            user_name: String::new(),
            timezone,
            budget: Budget::Everyday,
            dietary: Vec::new(),
            avoid: Vec::new(),
            quiet_mode: false,
            onboarding_complete: false,
            meals: vec![
                MealSetting {
                    id: MealId::Breakfast,
                    label: "早餐".to_owned(),
                    time: "08:00".to_owned(),
                    enabled: true,
                },
                MealSetting {
                    id: MealId::Lunch,
                    label: "午餐".to_owned(),
                    time: "12:30".to_owned(),
                    enabled: true,
                },
                MealSetting {
                    id: MealId::Dinner,
                    label: "晚餐".to_owned(),
                    time: "18:30".to_owned(),
                    enabled: true,
                },
            ],
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl CompanionSnapshot {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::new(format!(
                "unsupported schema version {}",
                self.schema_version
            )));
        }

        self.settings.validate()?;

        if self.history.len() > MAX_HISTORY_ENTRIES {
            return Err(ValidationError::new(format!(
                "history may contain at most {MAX_HISTORY_ENTRIES} entries"
            )));
        }
        for entry in &self.history {
            entry.validate()?;
        }
        self.runtime.validate()?;

        let encoded = serde_json::to_vec(self).map_err(|error| {
            ValidationError::new(format!("snapshot cannot be encoded: {error}"))
        })?;
        if encoded.len() > MAX_SNAPSHOT_BYTES {
            return Err(ValidationError::new("snapshot JSON is too large"));
        }

        Ok(())
    }

    pub fn trim_history(&mut self) {
        if self.history.len() > MAX_HISTORY_ENTRIES {
            let overflow = self.history.len() - MAX_HISTORY_ENTRIES;
            self.history.drain(..overflow);
        }
    }
}

impl CompanionSettings {
    pub fn refresh_timezone(&mut self, detected_timezone: &str) -> bool {
        if detected_timezone == self.timezone || detected_timezone.parse::<Tz>().is_err() {
            return false;
        }

        self.timezone = detected_timezone.to_owned();
        true
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_text(&self.companion_name, "companionName", 1, MAX_NAME_CHARS)?;
        validate_text(&self.user_name, "userName", 0, MAX_NAME_CHARS)?;
        validate_text(&self.timezone, "timezone", 1, 64)?;
        self.timezone
            .parse::<Tz>()
            .map_err(|_| ValidationError::new("timezone must be a valid IANA timezone"))?;
        validate_tags(&self.dietary, "dietary", MAX_DIETARY_TAGS)?;
        validate_tags(&self.avoid, "avoid", MAX_AVOID_TAGS)?;

        if self.meals.len() != MealId::ALL.len() {
            return Err(ValidationError::new(
                "meals must contain breakfast, lunch, and dinner exactly once",
            ));
        }

        let mut ids = HashSet::new();
        for meal in &self.meals {
            if !ids.insert(meal.id) {
                return Err(ValidationError::new(format!(
                    "meal {} appears more than once",
                    meal.id
                )));
            }
            validate_text(&meal.label, "meal label", 1, MAX_LABEL_CHARS)?;
            validate_time(&meal.time)?;
        }
        if MealId::ALL.iter().any(|id| !ids.contains(id)) {
            return Err(ValidationError::new(
                "meals must contain breakfast, lunch, and dinner exactly once",
            ));
        }

        Ok(())
    }
}

impl HistoryEntry {
    fn validate(&self) -> Result<(), ValidationError> {
        NaiveDate::parse_from_str(&self.date_key, "%Y-%m-%d")
            .map_err(|_| ValidationError::new("history dateKey must be YYYY-MM-DD"))?;
        DateTime::parse_from_rfc3339(&self.at)
            .map_err(|_| ValidationError::new("history at must be RFC3339"))?;
        if let Some(choice_id) = &self.choice_id {
            validate_text(choice_id, "choiceId", 1, MAX_CHOICE_ID_CHARS)?;
        }
        Ok(())
    }
}

impl CompanionRuntime {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(prompt) = &self.active_prompt {
            DateTime::parse_from_rfc3339(&prompt.due_at)
                .map_err(|_| ValidationError::new("activePrompt.dueAt must be RFC3339"))?;
        }
        if let Some(snoozed_until) = &self.snoozed_until {
            DateTime::parse_from_rfc3339(snoozed_until)
                .map_err(|_| ValidationError::new("snoozedUntil must be RFC3339"))?;
        }
        if let Some(last_prompt_key) = &self.last_prompt_key {
            validate_text(last_prompt_key, "lastPromptKey", 1, MAX_PROMPT_KEY_CHARS)?;
        }
        Ok(())
    }
}

pub fn validate_time(value: &str) -> Result<NaiveTime, ValidationError> {
    if value.len() != 5 || value.as_bytes().get(2) != Some(&b':') {
        return Err(ValidationError::new("meal time must be HH:MM"));
    }
    NaiveTime::parse_from_str(value, "%H:%M")
        .map_err(|_| ValidationError::new("meal time must be a valid HH:MM value"))
}

fn validate_tags(values: &[String], field: &str, maximum: usize) -> Result<(), ValidationError> {
    if values.len() > maximum {
        return Err(ValidationError::new(format!(
            "{field} may contain at most {maximum} entries"
        )));
    }

    for value in values {
        validate_text(value, field, 1, MAX_TAG_CHARS)?;
    }
    Ok(())
}

fn validate_text(
    value: &str,
    field: &str,
    minimum: usize,
    maximum: usize,
) -> Result<(), ValidationError> {
    let trimmed = value.trim();
    let length = trimmed.chars().count();
    if length < minimum || length > maximum {
        return Err(ValidationError::new(format!(
            "{field} must contain between {minimum} and {maximum} characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ValidationError::new(format!(
            "{field} may not contain control characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_snapshot_is_valid() {
        CompanionSnapshot::default().validate().unwrap();
    }

    #[test]
    fn rejects_invalid_timezone_and_time() {
        let mut snapshot = CompanionSnapshot::default();
        snapshot.settings.timezone = "Mars/Olympus".to_owned();
        assert!(snapshot.validate().is_err());

        snapshot.settings.timezone = "UTC".to_owned();
        snapshot.settings.meals[0].time = "8:00".to_owned();
        assert!(snapshot.validate().is_err());
    }

    #[test]
    fn refresh_timezone_preserves_meal_wall_clock_settings() {
        let mut settings = CompanionSettings {
            timezone: "America/Los_Angeles".to_owned(),
            ..CompanionSettings::default()
        };
        let meals = settings.meals.clone();

        assert!(settings.refresh_timezone("Asia/Shanghai"));
        assert_eq!(settings.timezone, "Asia/Shanghai");
        assert_eq!(settings.meals, meals);

        assert!(!settings.refresh_timezone("Mars/Olympus"));
        assert_eq!(settings.timezone, "Asia/Shanghai");
        assert_eq!(settings.meals, meals);
    }

    #[test]
    fn enforces_array_bounds() {
        let mut snapshot = CompanionSnapshot::default();
        snapshot.settings.dietary = (0..=MAX_DIETARY_TAGS)
            .map(|index| format!("tag-{index}"))
            .collect();
        assert!(snapshot.validate().is_err());

        snapshot.settings.dietary.clear();
        snapshot.settings.avoid = (0..=MAX_AVOID_TAGS)
            .map(|index| format!("avoid-{index}"))
            .collect();
        assert!(snapshot.validate().is_err());
    }

    #[test]
    fn optional_runtime_fields_are_omitted() {
        let value = serde_json::to_value(CompanionSnapshot::default()).unwrap();
        let runtime = value["runtime"].as_object().unwrap();
        assert!(!runtime.contains_key("activePrompt"));
        assert!(!runtime.contains_key("snoozedUntil"));
        assert!(!runtime.contains_key("lastPromptKey"));
    }
}
