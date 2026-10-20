//! Per-person NyxBot preferences: access mode, specialist requests and team limits.
use chrono::{DateTime, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use uuid::Uuid;

use super::{
    audit_outbox_service::{self, AuditEvent},
    audit_runtime::{AuditRuntime, policy::AuditTier, transaction},
};
use crate::{
    errors::{AppError, AppResult},
    models::assistant_settings::{
        AccessMode, AssistantSettings, COLLECTION_NAME, DEFAULT_SCHEDULE_MINIMUM_MINUTES,
        MAX_CONCURRENT_SUBAGENT_TURNS_LIMIT, MAX_GROUP_HANDOFFS_LIMIT,
        MAX_GROUP_HANDOFFS_PER_HOUR_LIMIT, MAX_LIVE_SUBAGENTS_LIMIT,
        SCHEDULE_MINIMUM_MINUTES_LIMIT, TRIGGER_RUNS_PER_DAY_LIMIT, TRIGGER_RUNS_PER_HOUR_LIMIT,
    },
    models::audit_outbox::{AccessModeScope, AuditDetail, AuditOwner, AuditPrincipal, AuditStage},
};

/// Settings for `user_id`, or the defaults when none were saved.
pub async fn get(db: &Database, user_id: &str) -> AppResult<AssistantSettings> {
    Ok(stored(db, user_id).await?.0)
}

pub(super) async fn get_in_session(
    db: &Database,
    session: &mut ClientSession,
    user_id: &str,
) -> AppResult<AssistantSettings> {
    Ok(decode_stored(
        db.collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": user_id})
            .session(session)
            .await?,
        user_id,
    )?
    .0)
}

pub(super) fn access_mode_narrows(before: AccessMode, after: AccessMode) -> bool {
    let rank = |mode| match mode {
        AccessMode::AskEveryTime => 0,
        AccessMode::Nyxmode => 1,
        AccessMode::Autopilot => 2,
    };
    rank(after) < rank(before)
}

/// The settings and whether the row stores an access mode. Legacy rows
/// predate access modes: an explicit destructive-action opt-out reads as
/// Full access until the next write stores it.
async fn stored(db: &Database, user_id: &str) -> AppResult<(AssistantSettings, bool)> {
    decode_stored(
        db.collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": user_id})
            .await?,
        user_id,
    )
}

fn decode_stored(
    raw: Option<bson::Document>,
    user_id: &str,
) -> AppResult<(AssistantSettings, bool)> {
    let Some(raw) = raw else {
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

pub async fn update(
    db: &Database,
    runtime: &AuditRuntime,
    principal: &AuditPrincipal,
    user_id: &str,
    update: Update,
) -> AppResult<AssistantSettings> {
    validate(&update)?;
    let change = SettingsChange {
        db,
        runtime,
        principal,
        user_id,
        update: &update,
        at: Utc::now(),
        settings_id: Uuid::new_v4(),
        settings_operation: Uuid::new_v4(),
        mode_id: Uuid::new_v4(),
        mode_operation: Uuid::new_v4(),
    };
    let (current, updated) = transaction::run(db, &change, |session, change| {
        Box::pin(update_in_session(session, change))
    })
    .await?;
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

fn validate(update: &Update) -> AppResult<()> {
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

struct SettingsChange<'a> {
    db: &'a Database,
    runtime: &'a AuditRuntime,
    principal: &'a AuditPrincipal,
    user_id: &'a str,
    update: &'a Update,
    at: DateTime<Utc>,
    settings_id: Uuid,
    settings_operation: Uuid,
    mode_id: Uuid,
    mode_operation: Uuid,
}

async fn update_in_session(
    session: &mut ClientSession,
    change: &SettingsChange<'_>,
) -> AppResult<(AssistantSettings, AssistantSettings)> {
    let SettingsChange {
        db,
        user_id,
        update,
        ..
    } = change;
    // Settings belong to one person. A verified conversation key retains its
    // own identity; it does not become the person who owns the conversation.
    let authorized = match change.principal {
        AuditPrincipal::Person { id } => id == *user_id,
        AuditPrincipal::ApiKey { owner_id, .. } => owner_id == *user_id,
        AuditPrincipal::System { .. } => true,
        _ => false,
    };
    if !authorized {
        return Err(AppError::Forbidden(
            "Assistant settings belong to their owner".into(),
        ));
    }
    let owner = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": *user_id})
        .session(&mut *session)
        .await?
        .filter(|owner| owner.user_type.is_person())
        .ok_or_else(|| AppError::NotFound("Assistant settings owner not found".into()))?;
    // Rows written before access modes carry only the destructive-action
    // switch; `get` reads an opted-out owner as Full access. Every write
    // materializes that mode, so the stored row and what `get` returns agree.
    let (current, stored_mode) = decode_stored(
        db.collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id": *user_id})
            .session(&mut *session)
            .await?,
        user_id,
    )?;
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
    let mut set = doc! {"updated_at":mongodb::bson::DateTime::from_chrono(change.at)};
    let mut defaults = mongodb::bson::to_document(&AssistantSettings::defaults(user_id))
        .map_err(|_| AppError::Internal("Settings serialization failed".into()))?;
    defaults.remove("_id");
    defaults.remove("updated_at");
    if let Some(voice) = &update.voice {
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
    if let Some(value) = &update.timezone {
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
    let updated = db
        .collection::<AssistantSettings>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":*user_id},
            doc! {"$set":set,"$setOnInsert":defaults},
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Internal("Assistant settings unavailable".into()))?;
    let tier = change_tier(&current, &updated)?;
    let event = AuditEvent {
        operation_id: change.settings_operation.to_string(),
        stage: AuditStage::Committed,
        principal: change.principal.clone(),
        owner: AuditOwner::Person { id: owner.id },
        action: "assistant_settings_updated",
        target_id: Some(current.user_id.clone()),
    };
    audit_outbox_service::enqueue_admitted_in_session(
        db,
        session,
        change.runtime,
        tier,
        &change.settings_id,
        &event,
    )
    .await?;
    if current.access_mode != updated.access_mode {
        let mode_event = AuditEvent {
            operation_id: change.mode_operation.to_string(),
            action: "assistant_access_mode_changed",
            ..event
        }
        .with_detail(AuditDetail::AccessModeChange {
            scope: AccessModeScope::Workspace,
            old: Some(current.access_mode),
            new: Some(updated.access_mode),
        });
        audit_outbox_service::enqueue_admitted_in_session(
            db,
            session,
            change.runtime,
            tier,
            &change.mode_id,
            &mode_event,
        )
        .await?;
    }
    Ok((current, updated))
}

/// A mixed patch takes Standard admission if any changed field widens policy
/// or changes an execution preference. Only an entirely narrowing patch takes
/// Narrowing, evaluated against the transaction's current settings snapshot.
fn change_tier(before: &AssistantSettings, after: &AssistantSettings) -> AppResult<AuditTier> {
    let mode_rank = |mode| match mode {
        AccessMode::AskEveryTime => 0,
        AccessMode::Nyxmode => 1,
        AccessMode::Autopilot => 2,
    };
    let request_rank = |mode| match mode {
        crate::models::assistant_settings::SpecialistAccessRequests::Denied => 0,
        crate::models::assistant_settings::SpecialistAccessRequests::RoutedToNyxbot => 1,
    };
    let movements = [
        mode_rank(after.access_mode).cmp(&mode_rank(before.access_mode)),
        request_rank(after.specialist_access_requests)
            .cmp(&request_rank(before.specialist_access_requests)),
        after
            .max_auto_continuations
            .cmp(&before.max_auto_continuations),
        after.max_live_subagents.cmp(&before.max_live_subagents),
        after
            .max_concurrent_subagent_turns
            .cmp(&before.max_concurrent_subagent_turns),
        after.max_group_handoffs.cmp(&before.max_group_handoffs),
        after
            .max_group_handoffs_per_hour
            .cmp(&before.max_group_handoffs_per_hour),
        before
            .schedule_minimum_minutes
            .cmp(&after.schedule_minimum_minutes),
        after
            .trigger_runs_per_hour
            .cmp(&before.trigger_runs_per_hour),
        after.trigger_runs_per_day.cmp(&before.trigger_runs_per_day),
    ];
    let voice_changed = bson::to_bson(&before.voice)
        .and_then(|before| bson::to_bson(&after.voice).map(|after| before != after))
        .map_err(|_| AppError::Internal("Voice preferences serialization failed".into()))?;
    Ok(
        if movements.contains(&std::cmp::Ordering::Less)
            && !movements.contains(&std::cmp::Ordering::Greater)
            && before.timezone == after.timezone
            && !voice_changed
        {
            AuditTier::Narrowing
        } else {
            AuditTier::Standard
        },
    )
}

#[cfg(test)]
mod w8_tests {
    use super::*;
    use crate::{
        models::{
            audit_log::{AuditLog, COLLECTION_NAME as LOGS},
            audit_outbox::{
                PENDING_COLLECTION_NAME as PENDING, PendingAuditEvent, SystemAuditActor,
            },
            user::{COLLECTION_NAME as USERS, UserType},
        },
        services::audit_state_service,
        test_utils::{connect_transaction_test_database, test_user},
    };
    use futures::TryStreamExt;

    async fn fixture(name: &str) -> (Database, String, AuditPrincipal, AuditRuntime) {
        let db = connect_transaction_test_database(name).await;
        let owner = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let principal = AuditPrincipal::Person { id: owner.clone() };
        (
            db,
            owner,
            principal,
            AuditRuntime::new(Default::default(), false),
        )
    }

    fn change<'a>(
        db: &'a Database,
        runtime: &'a AuditRuntime,
        principal: &'a AuditPrincipal,
        owner: &'a str,
        update: &'a Update,
    ) -> SettingsChange<'a> {
        SettingsChange {
            db,
            runtime,
            principal,
            user_id: owner,
            update,
            at: Utc::now(),
            settings_id: Uuid::new_v4(),
            settings_operation: Uuid::new_v4(),
            mode_id: Uuid::new_v4(),
            mode_operation: Uuid::new_v4(),
        }
    }

    async fn pending(db: &Database) -> Vec<PendingAuditEvent> {
        db.collection(PENDING)
            .find(doc! {})
            .sort(doc! {"order": 1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn w8_assist_settings_commit_abort_replay_and_safe_materialization() {
        let (db, owner, principal, runtime) = fixture("w8_assist_settings_atomic").await;
        let patch = Update {
            access_mode: Some(AccessMode::AskEveryTime),
            timezone: Some("Pacific/Auckland".into()),
            ..Default::default()
        };
        let change = change(&db, &runtime, &principal, &owner, &patch);
        let aborted = transaction::run(&db, &change, |session, change| {
            Box::pin(async move {
                update_in_session(session, change).await?;
                Err::<(), _>(AppError::Conflict("abort after audit reservation".into()))
            })
        })
        .await;
        assert!(matches!(aborted, Err(AppError::Conflict(_))));
        assert!(pending(&db).await.is_empty());
        assert_eq!(
            db.collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            audit_state_service::load(&db).await.unwrap().pending_count,
            0
        );
        for _ in 0..2 {
            let (_, updated) = transaction::run(&db, &change, |session, change| {
                Box::pin(update_in_session(session, change))
            })
            .await
            .unwrap();
            assert_eq!(updated.access_mode, AccessMode::AskEveryTime);
            assert_eq!(updated.timezone.as_deref(), Some("Pacific/Auckland"));
        }
        let events = pending(&db).await;
        assert_eq!(events.len(), 2);
        assert_eq!(
            audit_state_service::load(&db).await.unwrap().pending_count,
            2
        );
        for event in &events {
            assert_eq!(event.event.principal, principal);
            assert_eq!(event.event.owner, AuditOwner::Person { id: owner.clone() });
            assert_eq!(event.event.target_id.as_deref(), Some(owner.as_str()));
            assert_eq!(event.event.stage, AuditStage::Committed);
            assert_eq!(
                event.event.detail,
                (event.event.action == "assistant_access_mode_changed").then_some(
                    AuditDetail::AccessModeChange {
                        scope: AccessModeScope::Workspace,
                        old: Some(AccessMode::Nyxmode),
                        new: Some(AccessMode::AskEveryTime),
                    }
                )
            );
            let mut safe = bson::to_document(&event.event).unwrap();
            safe.remove("detail");
            assert_eq!(safe.len(), 6);
            assert!(!format!("{safe:?}").contains("Pacific/Auckland"));
        }
        while audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
        {}
        let logs: Vec<AuditLog> = db
            .collection(LOGS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(logs.len(), 2);
        assert!(
            logs.iter()
                .all(|log| log.user_id.as_deref() == Some(owner.as_str())
                    && log.entry_hash.is_some()
                    && log.seq.is_some())
        );
        // Local materialization does not release external witness backlog.
        assert_eq!(
            audit_state_service::load(&db).await.unwrap().pending_count,
            2
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn w8_assist_settings_c2_narrowing_standard_mixed_and_bundle_rollback() {
        for case in ["stale", "count", "age", "diverged", "bundle"] {
            let (db, owner, principal, _) = fixture(&format!("w8_assist_settings_{case}")).await;
            let runtime = AuditRuntime::new(Default::default(), true);
            let state = audit_state_service::load(&db).await.unwrap();
            match case {
                "count" | "bundle" => {
                    let count = if case == "bundle" {
                        99_999_i64
                    } else {
                        100_000_i64
                    };
                    db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
                        .update_one(
                            doc! {"singleton": true},
                            doc! {"$set": {
                                "pending_count": count, "next_order": count,
                                "oldest_pending_at": bson::DateTime::now(),
                            }},
                        )
                        .await
                        .unwrap();
                }
                "age" => {
                    db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
                        .update_one(doc! {"singleton": true}, doc! {"$set": {
                            "pending_count": 1_i64, "next_order": 1_i64,
                            "oldest_pending_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(1)),
                        }}).await.unwrap();
                }
                "diverged" => {
                    db.collection::<bson::Document>(
                        crate::models::audit_witness_delivery::COLLECTION_NAME,
                    )
                    .insert_one(doc! {"_id": &state.id, "divergence": {
                        "seq": 0_i64, "local_hash": "a", "witnessed_hash": "b",
                    }})
                    .await
                    .unwrap();
                }
                _ => {}
            }
            let count = audit_state_service::load(&db).await.unwrap().pending_count;
            let widening = update(
                &db,
                &runtime,
                &principal,
                &owner,
                Update {
                    access_mode: Some(AccessMode::Autopilot),
                    ..Default::default()
                },
            )
            .await;
            if case == "stale" {
                assert_eq!(widening.unwrap().access_mode, AccessMode::Autopilot);
            } else {
                assert!(matches!(
                    widening,
                    Err(AppError::AuditAdmissionClosed { .. } | AppError::AuditWitnessDiverged)
                ));
                assert_eq!(
                    get(&db, &owner).await.unwrap().access_mode,
                    AccessMode::Nyxmode
                );
                assert!(pending(&db).await.is_empty());
                assert_eq!(
                    audit_state_service::load(&db).await.unwrap().pending_count,
                    count
                );
            }
            if case != "stale" {
                // A narrowing mode combined with a wider run budget remains Standard.
                assert!(
                    update(
                        &db,
                        &runtime,
                        &principal,
                        &owner,
                        Update {
                            access_mode: Some(AccessMode::AskEveryTime),
                            trigger_runs_per_day: Some(TRIGGER_RUNS_PER_DAY_LIMIT),
                            ..Default::default()
                        }
                    )
                    .await
                    .is_err()
                );
                assert!(pending(&db).await.is_empty());
            }
            let narrowed = update(
                &db,
                &runtime,
                &principal,
                &owner,
                Update {
                    access_mode: Some(AccessMode::AskEveryTime),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(narrowed.access_mode, AccessMode::AskEveryTime);
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn w8_assist_settings_each_authority_limit_narrows_under_divergence() {
        use crate::models::assistant_settings::SpecialistAccessRequests;
        let (db, owner, principal, _) = fixture("w8_assist_settings_limits").await;
        let state = audit_state_service::load(&db).await.unwrap();
        db.collection::<bson::Document>(crate::models::audit_witness_delivery::COLLECTION_NAME)
            .insert_one(doc! {"_id": &state.id, "divergence": {
                "seq": 0_i64, "local_hash": "a", "witnessed_hash": "b",
            }})
            .await
            .unwrap();
        let runtime = AuditRuntime::new(Default::default(), true);
        let mut initial = AssistantSettings::defaults(&owner);
        initial.specialist_access_requests = SpecialistAccessRequests::RoutedToNyxbot;
        let reductions = [
            Update {
                access_mode: Some(AccessMode::AskEveryTime),
                ..Default::default()
            },
            Update {
                specialist_access_requests: Some(SpecialistAccessRequests::Denied),
                ..Default::default()
            },
            Update {
                max_auto_continuations: Some(0),
                ..Default::default()
            },
            Update {
                max_live_subagents: Some(0),
                ..Default::default()
            },
            Update {
                max_concurrent_subagent_turns: Some(1),
                ..Default::default()
            },
            Update {
                max_group_handoffs: Some(0),
                ..Default::default()
            },
            Update {
                max_group_handoffs_per_hour: Some(0),
                ..Default::default()
            },
            Update {
                schedule_minimum_minutes: Some(SCHEDULE_MINIMUM_MINUTES_LIMIT),
                ..Default::default()
            },
            Update {
                trigger_runs_per_hour: Some(1),
                ..Default::default()
            },
            Update {
                trigger_runs_per_day: Some(1),
                ..Default::default()
            },
        ];
        for patch in reductions {
            db.collection::<AssistantSettings>(COLLECTION_NAME)
                .replace_one(doc! {"_id": &owner}, &initial)
                .upsert(true)
                .await
                .unwrap();
            update(&db, &runtime, &principal, &owner, patch)
                .await
                .unwrap();
        }
        assert_eq!(pending(&db).await.len(), 11);
        let unchanged = get(&db, &owner).await.unwrap();
        // A timezone preference is Standard even alongside a lower team limit.
        assert!(matches!(
            update(
                &db,
                &runtime,
                &principal,
                &owner,
                Update {
                    max_live_subagents: Some(0),
                    timezone: Some("Asia/Singapore".into()),
                    ..Default::default()
                }
            )
            .await,
            Err(AppError::AuditWitnessDiverged)
        ));
        assert_eq!(get(&db, &owner).await.unwrap().timezone, unchanged.timezone);
        assert_eq!(pending(&db).await.len(), 11);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn w8_assist_settings_people_orgs_key_and_system_attribution() {
        let (db, owner, principal, runtime) = fixture("w8_assist_settings_actors").await;
        let other = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&other, UserType::Person))
            .await
            .unwrap();
        for org in [Uuid::new_v4().to_string(), Uuid::new_v4().to_string()] {
            db.collection(USERS)
                .insert_one(test_user(&org, UserType::Org))
                .await
                .unwrap();
            let org_key = AuditPrincipal::ApiKey {
                id: Uuid::new_v4().to_string(),
                owner_id: org.clone(),
            };
            assert!(
                update(&db, &runtime, &principal, &org, Update::default())
                    .await
                    .is_err()
            );
            assert!(
                update(&db, &runtime, &org_key, &org, Update::default())
                    .await
                    .is_err()
            );
            assert!(
                update(&db, &runtime, &org_key, &owner, Update::default())
                    .await
                    .is_err()
            );
        }
        assert!(
            update(&db, &runtime, &principal, &other, Update::default())
                .await
                .is_err()
        );
        assert!(pending(&db).await.is_empty());
        let key = AuditPrincipal::ApiKey {
            id: Uuid::new_v4().to_string(),
            owner_id: owner.clone(),
        };
        let system = AuditPrincipal::System {
            actor: SystemAuditActor::Maintenance,
        };
        for (actor, target) in [
            (&principal, owner.as_str()),
            (&key, owner.as_str()),
            (&system, other.as_str()),
        ] {
            update(&db, &runtime, actor, target, Update::default())
                .await
                .unwrap();
        }
        let events = pending(&db).await;
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event.principal, principal);
        assert_eq!(events[1].event.principal, key);
        assert_eq!(events[2].event.principal, system);
        assert_eq!(events[2].event.owner, AuditOwner::Person { id: other });
        while audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
        {}
        let logs: Vec<AuditLog> = db
            .collection(LOGS)
            .find(doc! {})
            .sort(doc! {"seq": 1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(logs[0].user_id.as_deref(), Some(owner.as_str()));
        assert!(logs[1].user_id.is_none());
        assert!(logs[1].api_key_id.is_some());
        assert!(logs[1].api_key_name.is_none());
        assert!(logs[2].user_id.is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn w8_assist_settings_unknown_commit_does_not_repeat_mutation_or_events() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{Arc, Mutex};
        let (base, owner, principal, runtime) = fixture("w8_assist_settings_unknown").await;
        let app = format!("w8-assist-{}", Uuid::new_v4());
        let commands = Arc::new(Mutex::new(Vec::new()));
        let capture = commands.clone();
        let mut options = mongodb::options::ClientOptions::parse(
            std::env::var("NYXID_TEST_DATABASE_URL").unwrap(),
        )
        .await
        .unwrap();
        options.app_name = Some(app.clone());
        options.command_event_handler = Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && (event.command_name == "commitTransaction"
                    || event.command_name == "findAndModify")
            {
                capture
                    .lock()
                    .unwrap()
                    .push((event.command_name, event.command));
            }
        }));
        let db = mongodb::Client::with_options(options)
            .unwrap()
            .database(base.name());
        base.client()
            .database("admin")
            .run_command(doc! {
                "configureFailPoint": "failCommand", "mode": {"times": 1}, "data": {
                    "failCommands": ["commitTransaction"], "appName": &app,
                    "writeConcernError": {"code": 64, "errmsg": "injected uncertain commit"},
                    "errorLabels": ["UnknownTransactionCommitResult"],
                }
            })
            .await
            .unwrap();
        update(
            &db,
            &runtime,
            &principal,
            &owner,
            Update {
                access_mode: Some(AccessMode::AskEveryTime),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let captured = commands.lock().unwrap().clone();
        let commits: Vec<_> = captured
            .iter()
            .filter(|(name, _)| name == "commitTransaction")
            .collect();
        assert_eq!(commits.len(), 2);
        for (_, command) in &commits[1..] {
            assert_eq!(command.get("lsid"), commits[0].1.get("lsid"));
            assert_eq!(command.get("txnNumber"), commits[0].1.get("txnNumber"));
        }
        assert_eq!(
            captured
                .iter()
                .filter(|(name, command)| name == "findAndModify"
                    && command.get_str("findAndModify") == Ok(COLLECTION_NAME))
                .count(),
            1
        );
        assert_eq!(pending(&db).await.len(), 2);
        assert_eq!(
            audit_state_service::load(&db).await.unwrap().pending_count,
            2
        );
        base.drop().await.unwrap();
    }
}
