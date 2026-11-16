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
/// Full access until the next write stores it.
async fn stored(db: &Database, user_id: &str) -> AppResult<(AssistantSettings, bool)> {
    let Some(raw) = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"_id": user_id})
        .await?
    else {
        return Ok((AssistantSettings::defaults(user_id), false));
    };
    effective_settings(raw)
}

/// Decode the effective approval mode of both current and legacy documents.
fn effective_settings(raw: bson::Document) -> AppResult<(AssistantSettings, bool)> {
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
    /// Internal tool fence; never deserialized from REST or MCP.
    pub snapshot: Option<Option<bson::Document>>,
    pub notification_push: Option<PushPatch>,
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
    pub notifications: Option<crate::models::assistant_settings::NotificationPreferences>,
}

pub fn validate(update: &Update) -> AppResult<()> {
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
    Ok(())
}

pub async fn update(db: &Database, user_id: &str, update: Update) -> AppResult<AssistantSettings> {
    validate(&update)?;
    // Rows written before access modes carry only the destructive-action
    // switch; `get` reads an opted-out owner as Full access. Every write
    // materializes that mode, so the stored row and what `get` returns agree.
    let (current, stored_mode) = stored(db, user_id).await?;
    let mode = match (update.access_mode, update.skip_destructive_confirmation) {
        (Some(mode), _) => Some(mode),
        // The legacy switch is a view of the mode: turning confirmations off
        // is Full access, and turning them back on leaves Full access for Auto.
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
    if let Some(value) = update.notifications {
        set.insert(
            "notifications",
            bson::to_bson(&value)
                .map_err(|_| AppError::Internal("Settings serialization failed".into()))?,
        );
        defaults.remove("notifications");
    }
    if let Some(push) = update.notification_push {
        defaults.remove("notifications");
        if matches!(update.snapshot, Some(None)) {
            let mut notifications = current.notifications;
            push.apply(&mut notifications.push);
            set.insert(
                "notifications",
                bson::to_bson(&notifications)
                    .map_err(|_| AppError::Internal("Settings serialization failed".into()))?,
            );
        } else {
            for (kind, value) in push.values() {
                if let Some(value) = value {
                    set.insert(format!("notifications.push.{kind}"), value);
                }
            }
        }
    }
    let mut filter = doc! {"_id":user_id};
    // NyxBot never relaxes its own approvals. Compare the complete snapshot
    // judged by preflight so a concurrent owner edit retries the whole patch.
    let snapshot = update.snapshot;
    let insert = matches!(snapshot, Some(None));
    if let Some(Some(raw)) = &snapshot {
        filter.insert("$expr", doc! {"$eq":["$$ROOT", {"$literal":raw}]});
    }
    let collection = db.collection::<AssistantSettings>(COLLECTION_NAME);
    let result = if insert {
        let mut row = bson::to_document(&AssistantSettings::defaults(user_id))
            .map_err(|_| AppError::Internal("Settings serialization failed".into()))?;
        row.extend(set);
        let row: AssistantSettings = bson::from_document(row)
            .map_err(|_| AppError::Internal("Settings serialization failed".into()))?;
        collection.insert_one(&row).await.map(|_| Some(row))
    } else {
        collection
            .find_one_and_update(filter, doc! {"$set":set,"$setOnInsert":defaults})
            .upsert(snapshot.is_none())
            .return_document(ReturnDocument::After)
            .await
    };
    let updated = result
        .map_err(|error| {
            if snapshot.is_some() && super::service_pool_service::is_duplicate_key(&error) {
                AppError::Conflict("Settings changed; read them again".into())
            } else {
                error.into()
            }
        })?
        .ok_or_else(|| AppError::Conflict("Settings changed; read them again".into()))?;
    super::assistant_notification_service::cache_push_preferences(
        db,
        user_id,
        updated.notifications.push,
    );
    if updated.access_mode == AccessMode::Autopilot && current.access_mode != updated.access_mode {
        super::assistant_acknowledgement_service::retire_full_access_confirmations(
            db,
            Some(user_id),
            None,
        )
        .await?;
    }
    Ok(updated)
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
            "notifications_changed":before.notifications != after.notifications,
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

/// Only fields edited in the NyxBot panel. No voice or legacy approval switch.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelUpdate {
    pub max_auto_continuations: Option<i32>,
    pub max_live_subagents: Option<i32>,
    pub max_concurrent_subagent_turns: Option<i32>,
    pub max_group_handoffs: Option<i32>,
    pub max_group_handoffs_per_hour: Option<i32>,
    pub timezone: Option<String>,
    pub schedule_minimum_minutes: Option<i32>,
    pub trigger_runs_per_hour: Option<i32>,
    pub trigger_runs_per_day: Option<i32>,
    pub access_mode: Option<AccessMode>,
    pub specialist_access_requests:
        Option<crate::models::assistant_settings::SpecialistAccessRequests>,
    pub notifications: Option<PushSettingsPatch>,
}
impl PanelUpdate {
    pub fn into_update(self) -> Update {
        Update {
            max_auto_continuations: self.max_auto_continuations,
            max_live_subagents: self.max_live_subagents,
            max_concurrent_subagent_turns: self.max_concurrent_subagent_turns,
            max_group_handoffs: self.max_group_handoffs,
            max_group_handoffs_per_hour: self.max_group_handoffs_per_hour,
            timezone: self.timezone,
            schedule_minimum_minutes: self.schedule_minimum_minutes,
            trigger_runs_per_hour: self.trigger_runs_per_hour,
            trigger_runs_per_day: self.trigger_runs_per_day,
            access_mode: self.access_mode,
            specialist_access_requests: self.specialist_access_requests,
            notification_push: self.notifications.map(|settings| settings.push),
            ..Default::default()
        }
    }
    pub fn refused_sections(&self, before: &AssistantSettings) -> Vec<&'static str> {
        use crate::models::assistant_settings::SpecialistAccessRequests;
        let rank = |mode| match mode {
            AccessMode::Autopilot => 0,
            AccessMode::Nyxmode => 1,
            AccessMode::AskEveryTime => 2,
        };
        let mut sections = Vec::new();
        if self
            .access_mode
            .is_some_and(|mode| rank(mode) < rank(before.access_mode))
        {
            sections.push("permissions");
        }
        if self.specialist_access_requests == Some(SpecialistAccessRequests::RoutedToNyxbot)
            && before.specialist_access_requests != SpecialistAccessRequests::RoutedToNyxbot
        {
            sections.push("permissions/specialists");
        }
        sections
    }
    pub fn changed_sections(&self, before: &AssistantSettings) -> Vec<&'static str> {
        let mut sections = Vec::new();
        macro_rules! changed {
            ($section:literal, $($field:ident),+) => {
                if $(
                    self.$field
                        .as_ref()
                        .is_some_and(|value| value != &before.$field)
                )||+ {
                    sections.push($section);
                }
            };
        }
        changed!("permissions", access_mode);
        changed!("permissions/specialists", specialist_access_requests);
        changed!("tasks", max_auto_continuations);
        changed!(
            "tasks/specialists",
            max_live_subagents,
            max_concurrent_subagent_turns
        );
        changed!(
            "tasks/groups",
            max_group_handoffs,
            max_group_handoffs_per_hour
        );
        if self
            .timezone
            .as_ref()
            .is_some_and(|zone| Some(zone) != before.timezone.as_ref())
            || self
                .schedule_minimum_minutes
                .is_some_and(|v| v != before.schedule_minimum_minutes)
            || self
                .trigger_runs_per_hour
                .is_some_and(|v| v != before.trigger_runs_per_hour)
            || self
                .trigger_runs_per_day
                .is_some_and(|v| v != before.trigger_runs_per_day)
        {
            sections.push("automations/budgets");
        }
        if self
            .notifications
            .as_ref()
            .is_some_and(|patch| patch.push.changed(&before.notifications.push))
        {
            sections.push("notifications");
        }
        sections
    }
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushSettingsPatch {
    pub push: PushPatch,
}
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushPatch {
    pub automation_result: Option<bool>,
    pub automation_problem: Option<bool>,
    pub needs_decision: Option<bool>,
    pub agent_update: Option<bool>,
    pub chat_update: Option<bool>,
    pub group_mention: Option<bool>,
}
impl PushPatch {
    fn values(&self) -> [(&'static str, Option<bool>); 6] {
        [
            ("automation_result", self.automation_result),
            ("automation_problem", self.automation_problem),
            ("needs_decision", self.needs_decision),
            ("agent_update", self.agent_update),
            ("chat_update", self.chat_update),
            ("group_mention", self.group_mention),
        ]
    }
    fn apply(&self, push: &mut crate::models::assistant_settings::NotificationPushPreferences) {
        macro_rules! apply {
            ($($field:ident),+) => {
                $(
                    if let Some(value) = self.$field {
                        push.$field = value;
                    }
                )+
            };
        }
        apply!(
            automation_result,
            automation_problem,
            needs_decision,
            agent_update,
            chat_update,
            group_mention
        );
    }
    pub fn is_empty(&self) -> bool {
        self.values().iter().all(|(_, value)| value.is_none())
    }
    fn changed(
        &self,
        before: &crate::models::assistant_settings::NotificationPushPreferences,
    ) -> bool {
        let before = [
            before.automation_result,
            before.automation_problem,
            before.needs_decision,
            before.agent_update,
            before.chat_update,
            before.group_mention,
        ];
        self.values()
            .iter()
            .zip(before)
            .any(|((_, value), before)| value.is_some_and(|value| value != before))
    }
}
pub enum PanelOutcome {
    Refused(Vec<&'static str>),
    Unchanged(AssistantSettings),
    Changed {
        before: AssistantSettings,
        after: Box<AssistantSettings>,
        sections: Vec<&'static str>,
    },
}
impl PanelUpdate {
    pub fn preflight(&self, before: AssistantSettings) -> Option<PanelOutcome> {
        let refused = self.refused_sections(&before);
        if !refused.is_empty() {
            Some(PanelOutcome::Refused(refused))
        } else if self.changed_sections(&before).is_empty() {
            Some(PanelOutcome::Unchanged(before))
        } else {
            None
        }
    }
}
/// The snapshot fence keeps no-op detection, approval comparisons and auditing
/// bound to one effective row. Concurrent edits retry from a fresh snapshot.
/// No persistent revision or stored-document change is needed.
pub async fn apply_panel(
    db: &Database,
    user_id: &str,
    patch: PanelUpdate,
) -> AppResult<PanelOutcome> {
    validate(&patch.clone().into_update())?;
    for _ in 0..3 {
        let raw = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":user_id})
            .await?;
        let before = if let Some(raw) = &raw {
            effective_settings(raw.clone())?.0
        } else {
            AssistantSettings::defaults(user_id)
        };
        if let Some(outcome) = patch.preflight(before.clone()) {
            return Ok(outcome);
        }
        let sections = patch.changed_sections(&before);
        let mut update = patch.clone().into_update();
        update.snapshot = Some(raw);
        match self::update(db, user_id, update).await {
            Ok(after) => {
                return Ok(PanelOutcome::Changed {
                    before,
                    after: Box::new(after),
                    sections,
                });
            }
            Err(AppError::Conflict(_)) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(AppError::Conflict(
        "Settings changed; read them again".into(),
    ))
}

#[cfg(test)]
mod panel_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn panel_patch_fences_snapshots_and_preserves_legacy_and_absent_rows() {
        let db = crate::test_utils::connect_transaction_test_database("nyxbot_panel_cas").await;
        let rows = db.collection::<bson::Document>(COLLECTION_NAME);
        let owner = "panel-owner";
        // Defaults are observable without creating a document, even on a no-op.
        let noop = PanelUpdate {
            max_auto_continuations: Some(get(&db, owner).await.unwrap().max_auto_continuations),
            ..Default::default()
        };
        assert!(matches!(
            apply_panel(&db, owner, noop).await.unwrap(),
            PanelOutcome::Unchanged(_)
        ));
        assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 0);
        let patch: PanelUpdate = serde_json::from_value(
            json!({"timezone":"UTC","notifications":{"push":{"chat_update":true}}}),
        )
        .unwrap();
        assert!(matches!(
            apply_panel(&db, owner, patch).await.unwrap(),
            PanelOutcome::Changed { .. }
        ));
        assert_eq!(
            get(&db, owner).await.unwrap().timezone.as_deref(),
            Some("UTC")
        );
        let stale = rows.find_one(doc! {"_id":owner}).await.unwrap().unwrap();
        update(
            &db,
            owner,
            Update {
                access_mode: Some(AccessMode::AskEveryTime),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let attempted = update(
            &db,
            owner,
            Update {
                snapshot: Some(Some(stale)),
                access_mode: Some(AccessMode::Nyxmode),
                max_live_subagents: Some(9),
                ..Default::default()
            },
        )
        .await;
        assert!(matches!(attempted, Err(AppError::Conflict(_))));
        assert_ne!(get(&db, owner).await.unwrap().max_live_subagents, 9);
        for (from, to) in [
            (AccessMode::Autopilot, AccessMode::Nyxmode),
            (AccessMode::Autopilot, AccessMode::AskEveryTime),
            (AccessMode::Nyxmode, AccessMode::AskEveryTime),
        ] {
            update(
                &db,
                owner,
                Update {
                    access_mode: Some(from),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert!(matches!(
                apply_panel(
                    &db,
                    owner,
                    PanelUpdate {
                        access_mode: Some(to),
                        ..Default::default()
                    }
                )
                .await
                .unwrap(),
                PanelOutcome::Changed { .. }
            ));
            assert_eq!(get(&db, owner).await.unwrap().access_mode, to);
        }
        rows.update_one(
            doc! {"_id":owner},
            doc! {"$unset":{"access_mode":""},"$set":{"skip_destructive_confirmation":true}},
        )
        .await
        .unwrap();
        assert_eq!(
            get(&db, owner).await.unwrap().access_mode,
            AccessMode::Autopilot
        );
        let PanelOutcome::Changed { before, after, .. } = apply_panel(
            &db,
            owner,
            PanelUpdate {
                timezone: Some("Asia/Taipei".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap() else {
            panic!("expected timezone change")
        };
        assert_eq!(after.access_mode, AccessMode::Autopilot);
        audit(&db, owner, &before, &after).await;
        let log = db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type":"assistant_settings_updated"})
            .await
            .unwrap()
            .unwrap();
        let data = log.get_document("event_data").unwrap();
        assert_eq!(data.get_str("access_mode").unwrap(), "autopilot");
        assert_eq!(data.get_str("previous_access_mode").unwrap(), "autopilot");
        assert_eq!(
            rows.find_one(doc! {"_id":owner})
                .await
                .unwrap()
                .unwrap()
                .get_str("access_mode")
                .unwrap(),
            "autopilot"
        );
        db.drop().await.unwrap();
    }
}
