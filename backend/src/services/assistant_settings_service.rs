//! Per-person NyxBot preferences: access mode, specialist requests and team limits.
use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};

use crate::{
    errors::{AppError, AppResult},
    models::assistant_settings::{
        AccessMode, AssistantSettings, COLLECTION_NAME, DEFAULT_SCHEDULE_MINIMUM_MINUTES,
        MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT, MAX_GROUP_HANDOFFS_LIMIT,
        MAX_GROUP_HANDOFFS_PER_HOUR_LIMIT, MAX_LIVE_SUBAGENTS_LIMIT,
        SCHEDULE_MINIMUM_MINUTES_LIMIT, TRIGGER_RUNS_PER_DAY_LIMIT, TRIGGER_RUNS_PER_HOUR_LIMIT,
    },
};

/// Settings for `user_id`, or the defaults when none were saved.
pub async fn get(db: &Database, user_id: &str) -> AppResult<AssistantSettings> {
    Ok(stored(db, user_id).await?.0)
}

/// The settings and whether the row stores an access mode. Legacy rows
/// predate access modes: an explicit destructive-action opt-out reads as
/// Autopilot until the next write stores it.
async fn stored(db: &Database, user_id: &str) -> AppResult<(AssistantSettings, bool)> {
    let Some(raw) = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"_id": user_id})
        .await?
    else {
        return Ok((AssistantSettings::defaults(user_id), false));
    };
    let stored_mode = raw.contains_key("access_mode");
    let mut row: AssistantSettings = bson::from_document(raw)
        .map_err(|_| AppError::Internal("Assistant settings unavailable".into()))?;
    if !stored_mode && row.skip_destructive_confirmation {
        row.access_mode = AccessMode::Autopilot;
    }
    Ok((row, stored_mode))
}

#[derive(Clone, Debug, Default)]
pub struct Update {
    pub voice: Option<Option<crate::models::assistant_voice::VoicePreferences>>,
    pub max_auto_continuations: Option<i32>,
    pub timezone: Option<String>,
    pub schedule_minimum_minutes: Option<i32>,
    pub trigger_runs_per_hour: Option<i32>,
    pub trigger_runs_per_day: Option<i32>,
    pub skip_destructive_confirmation: Option<bool>,
    pub access_mode: Option<crate::models::assistant_settings::AccessMode>,
    pub max_live_subagents: Option<i32>,
    pub max_concurrent_subagent_turns: Option<i32>,
    pub max_group_handoffs: Option<i32>,
    pub max_group_handoffs_per_hour: Option<i32>,
    pub specialist_access_requests:
        Option<crate::models::assistant_settings::SpecialistAccessRequests>,
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
            "max_auto_continuations",
            update.max_auto_continuations,
            0,
            super::assistant_continuation::HARD_MAX,
        ),
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
    if let Some(Some(voice)) = &update.voice {
        super::assistant_voice::validate_preferences(voice)?;
    }
    // Rows written before access modes carry only the destructive-action
    // switch; `get` reads an opted-out owner as Autopilot. Every write
    // materializes that mode, so the stored row and what `get` returns agree.
    let (current, stored_mode) = stored(db, user_id).await?;
    let mode = match (update.access_mode, update.skip_destructive_confirmation) {
        (Some(mode), _) => Some(mode),
        // The legacy switch is a view of the mode: turning confirmations off
        // is Autopilot, and turning them back on leaves Autopilot for NyxMode.
        (None, Some(true)) => Some(AccessMode::Autopilot),
        (None, Some(false)) if current.access_mode == AccessMode::Autopilot => {
            Some(AccessMode::Nyxmode)
        }
        (None, _) if !stored_mode && current.access_mode == AccessMode::Autopilot => {
            Some(AccessMode::Autopilot)
        }
        (None, _) => None,
    };
    let mut set = doc! {"updated_at":mongodb::bson::DateTime::from_chrono(Utc::now())};
    let mut defaults = mongodb::bson::to_document(&AssistantSettings::defaults(user_id))
        .map_err(|_| AppError::Internal("Settings serialization failed".into()))?;
    defaults.remove("_id");
    defaults.remove("updated_at");
    if let Some(voice) = update.voice {
        set.insert(
            "voice",
            bson::to_bson(&voice)
                .map_err(|_| AppError::Internal("Voice preferences serialization failed".into()))?,
        );
        defaults.remove("voice");
    }
    if let Some(value) = update.max_auto_continuations {
        set.insert("max_auto_continuations", value);
        defaults.remove("max_auto_continuations");
    }
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
    if let Some(value) = mode {
        set.insert(
            "access_mode",
            bson::to_bson(&value)
                .map_err(|_| AppError::Internal("Access mode serialization failed".into()))?,
        );
        // Keep the legacy field synchronized for replicas that predate modes.
        set.insert(
            "skip_destructive_confirmation",
            value == AccessMode::Autopilot,
        );
        defaults.remove("access_mode");
        defaults.remove("skip_destructive_confirmation");
    } else if let Some(value) = update.skip_destructive_confirmation {
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
    if let Some(value) = update.specialist_access_requests {
        set.insert(
            "specialist_access_requests",
            bson::to_bson(&value)
                .map_err(|_| AppError::Internal("Settings serialization failed".into()))?,
        );
        defaults.remove("specialist_access_requests");
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
            "max_auto_continuations":after.max_auto_continuations,
            "schedule_minimum_minutes":after.schedule_minimum_minutes,
            "trigger_runs_per_hour":after.trigger_runs_per_hour,
            "trigger_runs_per_day":after.trigger_runs_per_day,
            "skip_destructive_confirmation": after.skip_destructive_confirmation,
            "previous_skip_destructive_confirmation": before.skip_destructive_confirmation,
            "max_live_subagents": after.max_live_subagents,
            "max_concurrent_subagent_turns": after.max_concurrent_subagent_turns,
            "max_group_handoffs": after.max_group_handoffs,
            "max_group_handoffs_per_hour": after.max_group_handoffs_per_hour,
            "access_mode": after.access_mode,
            "previous_access_mode": before.access_mode,
            "specialist_access_requests": after.specialist_access_requests,
            "previous_specialist_access_requests": before.specialist_access_requests,
        })),
    )
    .await;
}

/// Audit a change of the access mode governing NyxBot actions: the owner's
/// workspace mode (`agent_id` absent) or one agent's override (`None` means
/// it inherits the workspace mode). No-op when nothing changed.
pub async fn audit_access_mode(
    db: &Database,
    actor: &super::audit_service::AuditActor,
    agent_id: Option<&str>,
    old: Option<AccessMode>,
    new: Option<AccessMode>,
) {
    if old == new {
        return;
    }
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        actor,
        "assistant_access_mode_changed",
        Some(serde_json::json!({
            "scope": if agent_id.is_some() {"agent"} else {"workspace"},
            "agent_id": agent_id,
            "old": old,
            "new": new,
        })),
    )
    .await;
}
