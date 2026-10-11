use chrono::Utc;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::desktop::{emit_state_changed, present_due_prompt, sync_pause_item};
use crate::model::{CompanionSettings, CompanionSnapshot, MealId, Mood};
use crate::nyxid::{
    NyxIdChatCommandError, NyxIdChatEvent, NyxIdChatHistory, NyxIdChatRecovery, NyxIdChatRequest,
    NyxIdState, NyxIdView,
};
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
pub fn start_window_drag(app: AppHandle) -> Result<(), String> {
    window::start_drag(&app).map_err(|error| error.to_string())
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

#[tauri::command]
pub fn nyxid_status(state: State<'_, NyxIdState>) -> NyxIdView {
    state.view()
}

#[tauri::command]
pub async fn start_nyxid_login(
    app: AppHandle,
    state: State<'_, NyxIdState>,
) -> Result<NyxIdView, String> {
    Ok(state.inner().clone().start_login(app).await)
}

#[tauri::command]
pub async fn cancel_nyxid_login(
    app: AppHandle,
    state: State<'_, NyxIdState>,
) -> Result<NyxIdView, String> {
    Ok(state.inner().clone().cancel_login(app).await)
}

#[tauri::command]
pub async fn refresh_nyxid_capabilities(
    app: AppHandle,
    state: State<'_, NyxIdState>,
) -> Result<NyxIdView, String> {
    Ok(state.inner().clone().refresh_capabilities(app).await)
}

#[tauri::command]
pub async fn logout_nyxid(
    app: AppHandle,
    state: State<'_, NyxIdState>,
) -> Result<NyxIdView, String> {
    Ok(state.inner().clone().logout(app).await)
}

#[tauri::command]
pub async fn send_nyxid_chat(
    app: AppHandle,
    state: State<'_, NyxIdState>,
    request: NyxIdChatRequest,
) -> Result<NyxIdChatEvent, NyxIdChatCommandError> {
    state.inner().clone().send_chat(app, request).await
}

#[tauri::command]
pub async fn recover_nyxid_chat(
    app: AppHandle,
    state: State<'_, NyxIdState>,
) -> Result<Option<NyxIdChatRecovery>, String> {
    state.inner().clone().recover_chat(app).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn nyxid_chat_history(
    app: AppHandle,
    state: State<'_, NyxIdState>,
    conversation_id: String,
) -> Result<NyxIdChatHistory, String> {
    state
        .inner()
        .clone()
        .chat_history(app, conversation_id)
        .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn nyxid_chat_stop(
    app: AppHandle,
    state: State<'_, NyxIdState>,
    conversation_id: String,
) -> Result<(), String> {
    state.inner().clone().stop_chat(app, conversation_id).await
}
