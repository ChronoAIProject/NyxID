use chrono::Utc;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::desktop::{emit_state_changed, present_due_prompt, sync_pause_item};
use crate::model::{CompanionSettings, CompanionSnapshot, MealId, Mood};
use crate::scheduler::DEFAULT_SNOOZE_MINUTES;
use crate::state::CompanionState;
use crate::window::{self, WindowMode};

#[tauri::command]
pub fn companion_snapshot(state: State<'_, CompanionState>) -> Result<CompanionSnapshot, String> {
    state.snapshot()
}

#[tauri::command]
pub fn save_companion_settings(
    app: AppHandle,
    state: State<'_, CompanionState>,
    settings: CompanionSettings,
) -> Result<CompanionSnapshot, String> {
    let update = state.update(
        |_, engine| engine.replace_settings(settings),
        |update| {
            if update.changed {
                emit_state_changed(&app, &update.snapshot)?;
                sync_pause_item(&app, update.snapshot.settings.quiet_mode);
            }
            Ok(())
        },
    )?;
    Ok(update.snapshot)
}

#[tauri::command(rename_all = "camelCase")]
pub fn snooze_meal(
    app: AppHandle,
    state: State<'_, CompanionState>,
    meal_id: MealId,
    minutes: Option<u32>,
) -> Result<CompanionSnapshot, String> {
    let update = state.update(
        |_, engine| {
            engine.snooze(
                meal_id,
                minutes.unwrap_or(DEFAULT_SNOOZE_MINUTES),
                Utc::now(),
            )
        },
        |update| emit_state_changed(&app, &update.snapshot),
    )?;
    Ok(update.snapshot)
}

#[tauri::command(rename_all = "camelCase")]
pub fn skip_meal(
    app: AppHandle,
    state: State<'_, CompanionState>,
    meal_id: MealId,
) -> Result<CompanionSnapshot, String> {
    let update = state.update(
        |_, engine| engine.skip(meal_id, Utc::now()),
        |update| emit_state_changed(&app, &update.snapshot),
    )?;
    Ok(update.snapshot)
}

#[tauri::command(rename_all = "camelCase")]
pub fn complete_meal(
    app: AppHandle,
    state: State<'_, CompanionState>,
    meal_id: MealId,
    choice_id: Option<String>,
    mood: Option<Mood>,
) -> Result<CompanionSnapshot, String> {
    let update = state.update(
        |_, engine| engine.complete(meal_id, choice_id, mood, Utc::now()),
        |update| emit_state_changed(&app, &update.snapshot),
    )?;
    Ok(update.snapshot)
}

#[tauri::command(rename_all = "camelCase")]
pub fn dislike_suggestion(
    app: AppHandle,
    state: State<'_, CompanionState>,
    meal_id: MealId,
    choice_id: String,
    mood: Option<Mood>,
) -> Result<CompanionSnapshot, String> {
    let update = state.update(
        |_, engine| engine.dislike(meal_id, choice_id, mood, Utc::now()),
        |update| emit_state_changed(&app, &update.snapshot),
    )?;
    Ok(update.snapshot)
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_quiet_mode(
    app: AppHandle,
    state: State<'_, CompanionState>,
    quiet_mode: bool,
) -> Result<CompanionSnapshot, String> {
    let update = state.update(
        |_, engine| {
            engine.set_quiet_mode(quiet_mode);
            Ok(())
        },
        |update| {
            if update.changed {
                emit_state_changed(&app, &update.snapshot)?;
            }
            sync_pause_item(&app, update.snapshot.settings.quiet_mode);
            Ok(())
        },
    )?;
    Ok(update.snapshot)
}

#[tauri::command(rename_all = "camelCase")]
pub fn trigger_demo_reminder(
    app: AppHandle,
    state: State<'_, CompanionState>,
    meal_id: Option<MealId>,
) -> Result<CompanionSnapshot, String> {
    let update = state.update(
        |_, engine| engine.trigger_demo(meal_id, Utc::now()),
        |update| {
            emit_state_changed(&app, &update.snapshot)?;
            present_due_prompt(&app, &update.snapshot, &update.value)
        },
    )?;
    Ok(update.snapshot)
}

#[tauri::command]
pub fn set_window_mode(app: AppHandle, mode: WindowMode) -> Result<(), String> {
    window::set_mode(&app, mode).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_launch_at_login(app: AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_launch_at_login(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|error| error.to_string())?;
    manager.is_enabled().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_nyxid_assistant(app: AppHandle) -> Result<(), String> {
    crate::desktop::open_nyxid_assistant(&app)
}
