//! Per-person NyxBot preferences: destructive-action confirmation and team limits.
use chrono::Utc;
use mongodb::{Database, bson::doc, options::ReturnDocument};

use crate::{
    errors::{AppError, AppResult},
    models::assistant_settings::{
        AssistantSettings, COLLECTION_NAME, MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT,
        MAX_LIVE_SUBAGENTS_LIMIT,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct Update {
    pub skip_destructive_confirmation: Option<bool>,
    pub max_live_subagents: Option<i32>,
    pub max_concurrent_subagent_turns: Option<i32>,
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
    let current = get(db, user_id).await?;
    let next = AssistantSettings {
        user_id: user_id.into(),
        skip_destructive_confirmation: update
            .skip_destructive_confirmation
            .unwrap_or(current.skip_destructive_confirmation),
        max_live_subagents: update
            .max_live_subagents
            .unwrap_or(current.max_live_subagents),
        max_concurrent_subagent_turns: update
            .max_concurrent_subagent_turns
            .unwrap_or(current.max_concurrent_subagent_turns),
        updated_at: Utc::now(),
    };
    db.collection::<AssistantSettings>(COLLECTION_NAME)
        .find_one_and_replace(doc! {"_id": user_id}, &next)
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
            "skip_destructive_confirmation": after.skip_destructive_confirmation,
            "previous_skip_destructive_confirmation": before.skip_destructive_confirmation,
            "max_live_subagents": after.max_live_subagents,
            "max_concurrent_subagent_turns": after.max_concurrent_subagent_turns,
        })),
    )
    .await;
}
