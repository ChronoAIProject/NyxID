use std::thread;
use std::time::Duration;

use chrono::Utc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, Wry};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::model::{ActivePrompt, CompanionSnapshot, MealId};
use crate::state::CompanionState;
use crate::window::{self, WindowMode};

pub const STATE_CHANGED_EVENT: &str = "companion://state-changed";
pub const MEAL_DUE_EVENT: &str = "companion://meal-due";
const NYXID_ASSISTANT_URL: &str = "https://nyx.chrono-ai.fun/assistant";
const SCHEDULER_INTERVAL: Duration = Duration::from_secs(15);

struct TrayControls {
    pause_item: CheckMenuItem<Wry>,
}

pub fn open_nyxid_assistant(app: &AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(NYXID_ASSISTANT_URL, None::<&str>)
        .map_err(|error| error.to_string())
}

pub fn emit_state_changed(app: &AppHandle, snapshot: &CompanionSnapshot) -> Result<(), String> {
    app.emit(STATE_CHANGED_EVENT, snapshot.clone())
        .map_err(|error| error.to_string())
}

pub fn present_due_prompt(
    app: &AppHandle,
    snapshot: &CompanionSnapshot,
    prompt: &ActivePrompt,
) -> Result<(), String> {
    app.emit(MEAL_DUE_EVENT, prompt.clone())
        .map_err(|error| error.to_string())?;

    if let Err(error) =
        window::set_mode(app, WindowMode::Expanded).and_then(|()| window::present(app))
    {
        eprintln!("failed to present companion window: {error}");
    }

    let meal_label = snapshot
        .settings
        .meals
        .iter()
        .find(|meal| meal.id == prompt.meal_id)
        .map_or_else(
            || fallback_meal_label(prompt.meal_id),
            |meal| meal.label.clone(),
        );
    if let Err(error) = app
        .notification()
        .builder()
        .title("Nyx 饭点提醒")
        .body(format!("该吃{meal_label}啦，来看看 Nyx 为你选了什么。"))
        .show()
    {
        eprintln!("failed to show meal notification: {error}");
    }

    Ok(())
}

pub fn install_tray(app: &mut App<Wry>, quiet_mode: bool) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "显示 Nyx", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, "hide", "隐藏 Nyx", true, None::<&str>)?;
    let pause_item = CheckMenuItem::with_id(
        app,
        "pause-reminders",
        "暂停饭点提醒",
        true,
        quiet_mode,
        None::<&str>,
    )?;
    let open_item = MenuItem::with_id(app, "open-nyxid", "打开 NyxID", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show_item,
            &hide_item,
            &pause_item,
            &open_item,
            &separator,
            &quit_item,
        ],
    )?;

    app.manage(TrayControls {
        pause_item: pause_item.clone(),
    });

    let mut builder = TrayIconBuilder::with_id("companion")
        .tooltip("Nyx 饭点助手")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                let _ = window::show(app);
            }
            "hide" => {
                let _ = window::hide(app);
            }
            "pause-reminders" => toggle_quiet_mode(app),
            "open-nyxid" => {
                if let Err(error) = open_nyxid_assistant(app) {
                    eprintln!("failed to open NyxID: {error}");
                }
            }
            "quit" => {
                window::flush_placement(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = window::show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

pub fn sync_pause_item(app: &AppHandle, quiet_mode: bool) {
    if let Some(controls) = app.try_state::<TrayControls>() {
        let _ = controls.pause_item.set_checked(quiet_mode);
    }
}

pub fn start_scheduler(app: AppHandle) {
    thread::Builder::new()
        .name("companion-meal-scheduler".to_owned())
        .spawn(move || {
            loop {
                run_scheduler_tick(&app);
                thread::sleep(SCHEDULER_INTERVAL);
            }
        })
        .expect("failed to start companion scheduler thread");
}

fn run_scheduler_tick(app: &AppHandle) {
    let detected_timezone = iana_time_zone::get_timezone().ok();
    let now = Utc::now();
    let state = app.state::<CompanionState>();
    if let Err(error) = state.update(
        |_, engine| {
            if let Some(timezone) = detected_timezone.as_deref() {
                engine.refresh_timezone(timezone);
            }
            Ok(engine.tick(now))
        },
        |update| {
            if update.changed {
                if let Err(error) = emit_state_changed(app, &update.snapshot) {
                    eprintln!("failed to emit companion state: {error}");
                }
                sync_pause_item(app, update.snapshot.settings.quiet_mode);
            }
            if let Some(prompt) = update.value.as_ref()
                && let Err(error) = present_due_prompt(app, &update.snapshot, prompt)
            {
                eprintln!("failed to emit meal reminder: {error}");
            }
            Ok(())
        },
    ) {
        eprintln!("companion scheduler tick failed: {error}");
    }
}

fn toggle_quiet_mode(app: &AppHandle) {
    let state = app.state::<CompanionState>();
    if let Err(error) = state.update(
        |snapshot, engine| {
            let quiet_mode = !snapshot.settings.quiet_mode;
            engine.set_quiet_mode(quiet_mode);
            Ok(quiet_mode)
        },
        |update| {
            sync_pause_item(app, update.value);
            if update.changed
                && let Err(error) = emit_state_changed(app, &update.snapshot)
            {
                eprintln!("failed to emit companion state: {error}");
            }
            Ok(())
        },
    ) {
        eprintln!("failed to update quiet mode: {error}");
    }
}

fn fallback_meal_label(meal_id: MealId) -> String {
    match meal_id {
        MealId::Breakfast => "早餐",
        MealId::Lunch => "午餐",
        MealId::Dinner => "晚餐",
    }
    .to_owned()
}
