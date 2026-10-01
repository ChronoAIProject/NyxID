//! Per-person NyxBot preferences: destructive-action confirmation and team limits.
use chrono::Utc;
use mongodb::{Database, bson::doc, options::ReturnDocument};

use crate::{
    errors::{AppError, AppResult},
    models::assistant_settings::{
        AssistantSettings, COLLECTION_NAME, DEFAULT_SCHEDULE_MINIMUM_MINUTES,
        MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT, MAX_GROUP_HANDOFFS_LIMIT,
        MAX_GROUP_HANDOFFS_PER_HOUR_LIMIT, MAX_LIVE_SUBAGENTS_LIMIT,
        SCHEDULE_MINIMUM_MINUTES_LIMIT, TRIGGER_RUNS_PER_DAY_LIMIT, TRIGGER_RUNS_PER_HOUR_LIMIT,
    },
};

/// Settings for `user_id`, or the defaults when none were saved.
pub async fn get(db: &Database, user_id: &str) -> AppResult<AssistantSettings> {
    Ok(db
        .collection::<AssistantSettings>(COLLECTION_NAME)
        .find_one(doc! {"_id": user_id})
        .await?
        .unwrap_or_else(|| AssistantSettings::defaults(user_id)))
}

#[derive(Clone, Debug, Default)]
pub struct Update {
    pub timezone: Option<String>,
    pub schedule_minimum_minutes: Option<i32>,
    pub trigger_runs_per_hour: Option<i32>,
    pub trigger_runs_per_day: Option<i32>,
    pub skip_destructive_confirmation: Option<bool>,
    pub max_live_subagents: Option<i32>,
    pub max_concurrent_subagent_turns: Option<i32>,
    pub max_group_handoffs: Option<i32>,
    pub max_group_handoffs_per_hour: Option<i32>,
}

pub async fn update(db: &Database, user_id: &str, update: Update) -> AppResult<AssistantSettings> {
    if update
        .max_live_subagents
        .is_some_and(|value| !(0..=MAX_LIVE_SUBAGENTS_LIMIT).contains(&value))
    {
        return Err(AppError::ValidationError(format!(
            "max_live_subagents must be between 0 and {MAX_LIVE_SUBAGENTS_LIMIT}"
        )));
    }
    if update
        .max_concurrent_subagent_turns
        .is_some_and(|value| !(1..=MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT).contains(&value))
    {
        return Err(AppError::ValidationError(format!(
            "max_concurrent_subagent_turns must be between 1 and \
            {MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT}"
        )));
    }
    if update
        .max_group_handoffs
        .is_some_and(|value| !(0..=MAX_GROUP_HANDOFFS_LIMIT).contains(&value))
    {
        return Err(AppError::ValidationError(format!(
            "max_group_handoffs must be between 0 and {MAX_GROUP_HANDOFFS_LIMIT}"
        )));
    }
    if update
        .max_group_handoffs_per_hour
        .is_some_and(|value| !(0..=MAX_GROUP_HANDOFFS_PER_HOUR_LIMIT).contains(&value))
    {
        return Err(AppError::ValidationError(format!(
            "max_group_handoffs_per_hour must be between 0 and \
            {MAX_GROUP_HANDOFFS_PER_HOUR_LIMIT}"
        )));
    }
    if let Some(zone) = &update.timezone {
        super::trigger_schedule::compute::timezone(zone)?;
    }
    for (name, value, low, high) in [
        (
            "schedule_minimum_minutes",
            update.schedule_minimum_minutes,
            DEFAULT_SCHEDULE_MINIMUM_MINUTES,
            SCHEDULE_MINIMUM_MINUTES_LIMIT,
        ),
        (
            "trigger_runs_per_hour",
            update.trigger_runs_per_hour,
            1,
            TRIGGER_RUNS_PER_HOUR_LIMIT,
        ),
        (
            "trigger_runs_per_day",
            update.trigger_runs_per_day,
            1,
            TRIGGER_RUNS_PER_DAY_LIMIT,
        ),
    ] {
        if value.is_some_and(|v| !(low..=high).contains(&v)) {
            return Err(AppError::ValidationError(format!(
                "{name} must be between {low} and {high}"
            )));
        }
    }
    let mut set = doc! {"updated_at":mongodb::bson::DateTime::from_chrono(Utc::now())};
    let mut defaults = mongodb::bson::to_document(&AssistantSettings::defaults(user_id))
        .map_err(|_| AppError::Internal("Settings serialization failed".into()))?;
    defaults.remove("_id");
    defaults.remove("updated_at");
    if let Some(value) = update.timezone {
        set.insert("timezone", value);
        defaults.remove("timezone");
    }
    if let Some(value) = update.schedule_minimum_minutes {
        set.insert("schedule_minimum_minutes", value);
        defaults.remove("schedule_minimum_minutes");
    }
    if let Some(value) = update.trigger_runs_per_hour {
        set.insert("trigger_runs_per_hour", value);
        defaults.remove("trigger_runs_per_hour");
    }
    if let Some(value) = update.trigger_runs_per_day {
        set.insert("trigger_runs_per_day", value);
        defaults.remove("trigger_runs_per_day");
    }
    if let Some(value) = update.skip_destructive_confirmation {
        set.insert("skip_destructive_confirmation", value);
        defaults.remove("skip_destructive_confirmation");
    }
    if let Some(value) = update.max_live_subagents {
        set.insert("max_live_subagents", value);
        defaults.remove("max_live_subagents");
    }
    if let Some(value) = update.max_concurrent_subagent_turns {
        set.insert("max_concurrent_subagent_turns", value);
        defaults.remove("max_concurrent_subagent_turns");
    }
    if let Some(value) = update.max_group_handoffs {
        set.insert("max_group_handoffs", value);
        defaults.remove("max_group_handoffs");
    }
    if let Some(value) = update.max_group_handoffs_per_hour {
        set.insert("max_group_handoffs_per_hour", value);
        defaults.remove("max_group_handoffs_per_hour");
    }
    db.collection::<AssistantSettings>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":user_id},
            doc! {"$set":set,"$setOnInsert":defaults},
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Internal("Assistant settings unavailable".into()))
}

/// Audit a settings change without free-form values.
pub async fn audit(
    db: &Database,
    user_id: &str,
    before: &AssistantSettings,
    after: &AssistantSettings,
) {
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        &super::audit_service::AuditActor {
            user_id: user_id.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "assistant_settings_updated",
        Some(serde_json::json!({
            "timezone_changed":before.timezone!=after.timezone,
            "schedule_minimum_minutes":after.schedule_minimum_minutes,
            "trigger_runs_per_hour":after.trigger_runs_per_hour,
            "trigger_runs_per_day":after.trigger_runs_per_day,
            "skip_destructive_confirmation": after.skip_destructive_confirmation,
            "previous_skip_destructive_confirmation": before.skip_destructive_confirmation,
            "max_live_subagents": after.max_live_subagents,
            "max_concurrent_subagent_turns": after.max_concurrent_subagent_turns,
            "max_group_handoffs": after.max_group_handoffs,
            "max_group_handoffs_per_hour": after.max_group_handoffs_per_hour,
        })),
    )
    .await;
}
