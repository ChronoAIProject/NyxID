mod commands;
mod desktop;
mod model;
mod nyxid;
mod persistence;
mod scheduler;
mod state;
mod window;

use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use crate::model::CompanionSnapshot;
use crate::persistence::SnapshotStore;
use crate::scheduler::{CompanionEngine, SchedulerError};
use crate::state::CompanionState;

const AUTOSTART_ARGUMENT: &str = "--autostart";

fn refresh_snapshot_timezone(
    snapshot: &mut CompanionSnapshot,
    detected_timezone: &str,
) -> Result<bool, SchedulerError> {
    let mut engine = CompanionEngine::new(snapshot.clone())?;
    if !engine.refresh_timezone(detected_timezone) {
        return Ok(false);
    }

    *snapshot = engine.into_snapshot();
    Ok(true)
}

pub fn run() {
    tauri::Builder::default()
        // This must stay first so duplicate processes cannot initialize other plugins.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let result = if args.iter().any(|argument| argument == AUTOSTART_ARGUMENT) {
                window::present(app)
            } else {
                window::show(app)
            };
            let _ = result;
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARGUMENT]),
        ))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_data_dir = app.path().app_data_dir()?;
            let store = SnapshotStore::in_directory(&app_data_dir);
            let mut snapshot = match store.load() {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => {
                    let snapshot = CompanionSnapshot::default();
                    store.save(&snapshot)?;
                    snapshot
                }
                Err(error) => {
                    eprintln!("ignoring invalid companion snapshot: {error}");
                    CompanionSnapshot::default()
                }
            };
            if let Ok(timezone) = iana_time_zone::get_timezone()
                && refresh_snapshot_timezone(&mut snapshot, &timezone)?
            {
                store.save(&snapshot)?;
            }
            let quiet_mode = snapshot.settings.quiet_mode;
            app.manage(CompanionState::new(snapshot, store));
            let nyxid_state = nyxid::NyxIdState::system(app_data_dir)?;
            app.manage(nyxid_state.clone());

            window::configure_main_window(app.handle())?;
            desktop::install_tray(app, quiet_mode)?;
            if std::env::args().any(|argument| argument == AUTOSTART_ARGUMENT) {
                window::present(app.handle())?;
            } else {
                window::show(app.handle())?;
            }
            desktop::start_scheduler(app.handle().clone());
            nyxid_state.bootstrap(app.handle().clone());
            Ok(())
        })
        .on_window_event(|app_window, event| match event {
            WindowEvent::Moved(position) => {
                window::handle_window_moved(app_window, *position);
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                window::handle_window_geometry_changed(app_window);
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = app_window.hide();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::companion_snapshot,
            commands::save_companion_settings,
            commands::snooze_meal,
            commands::skip_meal,
            commands::complete_meal,
            commands::dislike_suggestion,
            commands::set_quiet_mode,
            commands::trigger_demo_reminder,
            commands::set_window_mode,
            commands::start_window_drag,
            commands::get_launch_at_login,
            commands::set_launch_at_login,
            commands::open_nyxid_assistant,
            commands::nyxid_status,
            commands::start_nyxid_login,
            commands::cancel_nyxid_login,
            commands::refresh_nyxid_capabilities,
            commands::logout_nyxid,
            commands::send_nyxid_chat,
            commands::recover_nyxid_chat,
            commands::nyxid_chat_history,
            commands::nyxid_chat_stop,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run NyxID Companion");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActivePrompt, MealId};

    #[test]
    fn restart_in_a_new_timezone_clears_runtime_from_the_old_local_day() {
        let directory = tempfile::tempdir().unwrap();
        let store = SnapshotStore::in_directory(directory.path());
        let mut snapshot = CompanionSnapshot::default();
        snapshot.settings.timezone = "UTC".to_owned();
        snapshot.runtime.active_prompt = Some(ActivePrompt {
            meal_id: MealId::Breakfast,
            due_at: "2026-10-09T08:00:00Z".to_owned(),
        });
        snapshot.runtime.snoozed_until = Some("2026-10-09T08:20:00Z".to_owned());
        snapshot.runtime.last_prompt_key = Some("2026-10-09:breakfast".to_owned());
        store.save(&snapshot).unwrap();

        let mut restored = store.load().unwrap().unwrap();
        assert!(refresh_snapshot_timezone(&mut restored, "Asia/Shanghai").unwrap());
        store.save(&restored).unwrap();

        let restarted = store.load().unwrap().unwrap();
        assert_eq!(restarted.settings.timezone, "Asia/Shanghai");
        assert!(restarted.runtime.active_prompt.is_none());
        assert!(restarted.runtime.snoozed_until.is_none());
        assert!(restarted.runtime.last_prompt_key.is_none());
    }
}
