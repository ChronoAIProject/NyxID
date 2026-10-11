use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize, Runtime,
    WebviewWindow, Window,
};
use thiserror::Error;

const EDGE_MARGIN_LOGICAL: f64 = 18.0;
const PLACEMENT_FILE_NAME: &str = "window-placement.json";
const PLACEMENT_SCHEMA_VERSION: u8 = 1;
const MAX_PLACEMENT_BYTES: usize = 4096;
const SAVE_DEBOUNCE: Duration = Duration::from_millis(250);
const PROGRAMMATIC_MOVE_WAIT: Duration = Duration::from_millis(250);
const DRAG_SETTLE_DEBOUNCE: Duration = Duration::from_millis(140);
const DRAG_SETTLE_FALLBACK: Duration = Duration::from_millis(420);
pub const WINDOW_DRAG_EVENT: &str = "companion://window-drag";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WindowPlacement {
    schema_version: u8,
    monitor_name: Option<String>,
    monitor_work_x: i32,
    monitor_work_y: i32,
    monitor_work_width: u32,
    monitor_work_height: u32,
    anchor_x: f64,
    anchor_y: f64,
}

impl WindowPlacement {
    fn validate(&self) -> Result<(), WindowPlacementError> {
        if self.schema_version != PLACEMENT_SCHEMA_VERSION {
            return Err(WindowPlacementError::UnsupportedSchema(self.schema_version));
        }
        if self.monitor_work_width == 0
            || self.monitor_work_height == 0
            || !self.anchor_x.is_finite()
            || !self.anchor_y.is_finite()
            || !(0.0..=1.0).contains(&self.anchor_x)
            || !(0.0..=1.0).contains(&self.anchor_y)
            || self
                .monitor_name
                .as_ref()
                .is_some_and(|name| name.len() > 256)
        {
            return Err(WindowPlacementError::Invalid);
        }
        Ok(())
    }

    fn from_geometry(
        monitor: &MonitorGeometry,
        position: PhysicalPosition<i32>,
        size: PhysicalSize<u32>,
    ) -> Self {
        let anchor_x = i64::from(position.x) + i64::from(size.width);
        let anchor_y = i64::from(position.y) + i64::from(size.height);
        let normalized_x = (anchor_x - i64::from(monitor.work_position.x)) as f64
            / f64::from(monitor.work_size.width.max(1));
        let normalized_y = (anchor_y - i64::from(monitor.work_position.y)) as f64
            / f64::from(monitor.work_size.height.max(1));

        Self {
            schema_version: PLACEMENT_SCHEMA_VERSION,
            monitor_name: monitor.name.clone(),
            monitor_work_x: monitor.work_position.x,
            monitor_work_y: monitor.work_position.y,
            monitor_work_width: monitor.work_size.width,
            monitor_work_height: monitor.work_size.height,
            anchor_x: normalized_x.clamp(0.0, 1.0),
            anchor_y: normalized_y.clamp(0.0, 1.0),
        }
    }

    fn absolute_anchor(&self) -> (f64, f64) {
        (
            f64::from(self.monitor_work_x) + self.anchor_x * f64::from(self.monitor_work_width),
            f64::from(self.monitor_work_y) + self.anchor_y * f64::from(self.monitor_work_height),
        )
    }
}

#[derive(Debug, Clone)]
struct WindowPlacementStore {
    path: PathBuf,
}

impl WindowPlacementStore {
    fn in_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            path: directory.into().join(PLACEMENT_FILE_NAME),
        }
    }

    #[cfg(test)]
    fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> Result<Option<WindowPlacement>, WindowPlacementError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        if metadata.len() > MAX_PLACEMENT_BYTES as u64 {
            return Err(WindowPlacementError::TooLarge);
        }

        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take((MAX_PLACEMENT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_PLACEMENT_BYTES {
            return Err(WindowPlacementError::TooLarge);
        }

        let placement: WindowPlacement = serde_json::from_slice(&bytes)?;
        placement.validate()?;
        Ok(Some(placement))
    }

    fn save(&self, placement: &WindowPlacement) -> Result<(), WindowPlacementError> {
        placement.validate()?;
        let bytes = serde_json::to_vec_pretty(placement)?;
        if bytes.len() > MAX_PLACEMENT_BYTES {
            return Err(WindowPlacementError::TooLarge);
        }
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "placement path has no parent")
        })?;
        fs::create_dir_all(parent)?;

        let mut temporary = tempfile::Builder::new()
            .prefix(".window-placement.")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        sync_directory(parent)?;
        Ok(())
    }
}

#[derive(Debug, Error)]
enum WindowPlacementError {
    #[error("window placement storage failed: {0}")]
    Io(#[from] io::Error),
    #[error("window placement JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("window placement JSON exceeds {MAX_PLACEMENT_BYTES} bytes")]
    TooLarge,
    #[error("window placement schema version {0} is unsupported")]
    UnsupportedSchema(u8),
    #[error("window placement values are invalid")]
    Invalid,
}

struct WindowPlacementState {
    store: WindowPlacementStore,
    save_revision: Arc<AtomicU64>,
    write_gate: Arc<Mutex<()>>,
    placement: Arc<Mutex<Option<WindowPlacement>>>,
    movement: Arc<Mutex<WindowMovementState>>,
}

impl WindowPlacementState {
    fn new(store: WindowPlacementStore, placement: Option<WindowPlacement>) -> Self {
        Self {
            store,
            save_revision: Arc::new(AtomicU64::new(0)),
            write_gate: Arc::new(Mutex::new(())),
            placement: Arc::new(Mutex::new(placement)),
            movement: Arc::new(Mutex::new(WindowMovementState::default())),
        }
    }

    fn placement(&self) -> Option<WindowPlacement> {
        self.placement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn remember_and_schedule_save(&self, placement: WindowPlacement) {
        let mut current = self
            .placement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.as_ref() == Some(&placement) {
            return;
        }
        *current = Some(placement.clone());
        drop(current);

        let revision = self.save_revision.fetch_add(1, Ordering::SeqCst) + 1;
        let save_revision = Arc::clone(&self.save_revision);
        let write_gate = Arc::clone(&self.write_gate);
        let store = self.store.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SAVE_DEBOUNCE).await;
            let result = tokio::task::spawn_blocking(move || {
                let _write = write_gate
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if save_revision.load(Ordering::SeqCst) != revision {
                    return Ok(());
                }
                store.save(&placement)
            })
            .await;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    eprintln!("failed to save companion window placement: {error}");
                }
                Err(error) => {
                    eprintln!("companion window placement writer stopped: {error}");
                }
            }
        });
    }

    fn flush(&self) -> Result<(), WindowPlacementError> {
        let _write = self
            .write_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.save_revision.fetch_add(1, Ordering::SeqCst);
        let placement = self
            .placement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match placement {
            Some(placement) => self.store.save(&placement),
            None => Ok(()),
        }
    }

    fn expect_programmatic_move(
        &self,
        current: Option<PhysicalPosition<i32>>,
        target: PhysicalPosition<i32>,
    ) -> bool {
        let mut movement = self
            .movement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        movement.programmatic.expect(
            target,
            Instant::now() + PROGRAMMATIC_MOVE_WAIT,
            current == Some(target),
        );
        movement.drag.cancel_and_observe(target)
    }

    fn begin_drag(&self, position: PhysicalPosition<i32>) -> u64 {
        let mut movement = self
            .movement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        movement.programmatic.clear();
        movement.drag.begin(position)
    }

    fn observe_move(&self, position: PhysicalPosition<i32>) -> ObservedMove {
        let mut movement = self
            .movement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if movement
            .programmatic
            .should_suppress(position, Instant::now())
        {
            movement.drag.observe_programmatic(position);
            return ObservedMove::Programmatic;
        }

        let observation = movement.drag.observe_user_move(position);
        ObservedMove::User {
            revision: observation.revision,
            direction: observation.direction,
        }
    }

    fn settle_drag(&self, revision: u64) -> bool {
        self.movement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drag
            .settle_if_current(revision)
    }

    fn observe_drag_geometry_change(&self) -> Option<u64> {
        self.movement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drag
            .observe_geometry_change()
    }

    fn drag_is_active(&self) -> bool {
        self.movement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drag
            .active
    }

    fn schedule_drag_settled<R: Runtime>(&self, app: AppHandle<R>, revision: u64, delay: Duration) {
        let movement = Arc::clone(&self.movement);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(delay).await;
            let should_emit = movement
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drag
                .settle_if_current(revision);
            if should_emit {
                if let Some(window) = app.get_webview_window("main")
                    && let Some(placement) = capture_webview_placement(&window)
                    && let Some(state) = app.try_state::<WindowPlacementState>()
                {
                    state.remember_and_schedule_save(placement);
                }
                if let Err(error) = app.emit(WINDOW_DRAG_EVENT, WindowDragPayload::settled()) {
                    eprintln!("failed to emit companion window drag event: {error}");
                }
            }
        });
    }
}

#[derive(Debug, Default)]
struct WindowMovementState {
    programmatic: ProgrammaticMoveGuard,
    drag: DragLifecycleState,
}

#[derive(Debug, Default)]
struct ProgrammaticMoveGuard {
    expected: Option<ExpectedProgrammaticMove>,
}

#[derive(Debug)]
struct ExpectedProgrammaticMove {
    target: PhysicalPosition<i32>,
    wait_until: Instant,
    target_seen: bool,
}

impl ProgrammaticMoveGuard {
    fn expect(&mut self, target: PhysicalPosition<i32>, wait_until: Instant, target_seen: bool) {
        self.expected = Some(ExpectedProgrammaticMove {
            target,
            wait_until,
            target_seen,
        });
    }

    fn clear(&mut self) {
        self.expected = None;
    }

    fn should_suppress(&mut self, position: PhysicalPosition<i32>, now: Instant) -> bool {
        let Some(expected) = self.expected.as_mut() else {
            return false;
        };

        if position == expected.target {
            expected.target_seen = true;
            return true;
        }
        if !expected.target_seen && now <= expected.wait_until {
            return true;
        }

        self.expected = None;
        false
    }
}

#[derive(Debug, Default)]
struct DragLifecycleState {
    revision: u64,
    active: bool,
    last_position: Option<PhysicalPosition<i32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DragMovement {
    revision: u64,
    direction: Option<WindowDragDirection>,
}

impl DragLifecycleState {
    fn next_revision(&mut self) -> u64 {
        self.revision = self.revision.wrapping_add(1);
        if self.revision == 0 {
            self.revision = 1;
        }
        self.revision
    }

    fn begin(&mut self, position: PhysicalPosition<i32>) -> u64 {
        self.active = true;
        self.last_position = Some(position);
        self.next_revision()
    }

    fn observe_user_move(&mut self, position: PhysicalPosition<i32>) -> DragMovement {
        let direction = self.last_position.and_then(|previous| {
            if position.x < previous.x {
                Some(WindowDragDirection::Left)
            } else if position.x > previous.x {
                Some(WindowDragDirection::Right)
            } else {
                None
            }
        });
        self.last_position = Some(position);
        self.active = true;
        DragMovement {
            revision: self.next_revision(),
            direction,
        }
    }

    fn observe_programmatic(&mut self, position: PhysicalPosition<i32>) {
        self.last_position = Some(position);
    }

    fn observe_geometry_change(&mut self) -> Option<u64> {
        self.active.then(|| self.next_revision())
    }

    fn cancel_and_observe(&mut self, position: PhysicalPosition<i32>) -> bool {
        let was_active = self.active;
        self.active = false;
        self.last_position = Some(position);
        self.next_revision();
        was_active
    }

    fn settle_if_current(&mut self, revision: u64) -> bool {
        if !self.active || self.revision != revision {
            return false;
        }
        self.active = false;
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservedMove {
    Programmatic,
    User {
        revision: u64,
        direction: Option<WindowDragDirection>,
    },
}

impl Drop for WindowPlacementState {
    fn drop(&mut self) {
        if let Err(error) = self.flush() {
            eprintln!("failed to flush companion window placement: {error}");
        }
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    Compact,
    Expanded,
}

impl WindowMode {
    const fn dimensions(self) -> (f64, f64) {
        match self {
            Self::Compact => (236.0, 236.0),
            Self::Expanded => (404.0, 640.0),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum WindowDragPhase {
    Moving,
    Settled,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum WindowDragDirection {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WindowDragPayload {
    phase: WindowDragPhase,
    direction: Option<WindowDragDirection>,
}

impl WindowDragPayload {
    const fn moving(direction: WindowDragDirection) -> Self {
        Self {
            phase: WindowDragPhase::Moving,
            direction: Some(direction),
        }
    }

    const fn settled() -> Self {
        Self {
            phase: WindowDragPhase::Settled,
            direction: None,
        }
    }
}

pub fn configure_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    window.set_always_on_top(true)?;

    let store = WindowPlacementStore::in_directory(app.path().app_data_dir()?);
    let (width, height) = WindowMode::Compact.dimensions();
    let saved_placement = match store.load() {
        Ok(placement) => placement,
        Err(error) => {
            eprintln!("ignoring invalid companion window placement: {error}");
            None
        }
    };
    let placement_state = WindowPlacementState::new(store, saved_placement.clone());
    if app.try_state::<WindowPlacementState>().is_none() {
        app.manage(placement_state);
    }

    let restored = match saved_placement.as_ref() {
        Some(placement) => restore_placement(&window, LogicalSize::new(width, height), placement)?,
        None => false,
    };
    if !restored {
        if let Some(placement) =
            resize_and_position_logical(&window, LogicalSize::new(width, height))?
            && let Some(state) = window.try_state::<WindowPlacementState>()
        {
            state.remember_and_schedule_save(placement);
        }
    }
    Ok(())
}

pub fn set_mode<R: Runtime>(app: &AppHandle<R>, mode: WindowMode) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    let (width, height) = mode.dimensions();
    resize_at_current_position(&window, LogicalSize::new(width, height))
}

pub fn flush_placement<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<WindowPlacementState>() else {
        return;
    };
    if state.drag_is_active()
        && let Some(window) = app.get_webview_window("main")
        && let Some(placement) = capture_webview_placement(&window)
    {
        state.remember_and_schedule_save(placement);
    }
    if let Err(error) = state.flush() {
        eprintln!("failed to flush companion window placement: {error}");
    }
}

pub fn start_drag<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    let position = window.outer_position()?;
    let revision = app
        .try_state::<WindowPlacementState>()
        .map(|state| state.begin_drag(position));

    if let Err(error) = window.start_dragging() {
        if let (Some(state), Some(revision)) = (app.try_state::<WindowPlacementState>(), revision) {
            state.settle_drag(revision);
        }
        return Err(error);
    }

    if let (Some(state), Some(revision)) = (app.try_state::<WindowPlacementState>(), revision) {
        state.schedule_drag_settled(app.clone(), revision, DRAG_SETTLE_FALLBACK);
    }
    Ok(())
}

pub fn handle_window_moved<R: Runtime>(window: &Window<R>, position: PhysicalPosition<i32>) {
    if window.label() != "main" {
        return;
    }
    let Some(state) = window.try_state::<WindowPlacementState>() else {
        return;
    };
    let ObservedMove::User {
        revision,
        direction,
    } = state.observe_move(position)
    else {
        return;
    };

    if let Some(direction) = direction
        && let Err(error) = window.emit(WINDOW_DRAG_EVENT, WindowDragPayload::moving(direction))
    {
        eprintln!("failed to emit companion window drag event: {error}");
    }
    state.schedule_drag_settled(window.app_handle().clone(), revision, DRAG_SETTLE_DEBOUNCE);

    let Ok(size) = window.outer_size() else {
        return;
    };
    let Ok(monitors) = window.available_monitors() else {
        return;
    };
    let monitor_geometries = monitors
        .iter()
        .map(MonitorGeometry::from)
        .collect::<Vec<_>>();
    let Some(monitor_index) = select_physical_monitor_index(position, size, &monitor_geometries)
    else {
        return;
    };

    state.remember_and_schedule_save(WindowPlacement::from_geometry(
        &monitor_geometries[monitor_index],
        position,
        size,
    ));
}

pub fn handle_window_geometry_changed<R: Runtime>(window: &Window<R>) {
    if window.label() != "main" {
        return;
    }
    let Some(state) = window.try_state::<WindowPlacementState>() else {
        return;
    };
    let Some(revision) = state.observe_drag_geometry_change() else {
        return;
    };
    state.schedule_drag_settled(window.app_handle().clone(), revision, DRAG_SETTLE_DEBOUNCE);
}

fn capture_webview_placement<R: Runtime>(window: &WebviewWindow<R>) -> Option<WindowPlacement> {
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let monitors = window.available_monitors().ok()?;
    let monitor_geometries = monitors
        .iter()
        .map(MonitorGeometry::from)
        .collect::<Vec<_>>();
    let monitor_index = select_physical_monitor_index(position, size, &monitor_geometries)?;
    Some(WindowPlacement::from_geometry(
        &monitor_geometries[monitor_index],
        position,
        size,
    ))
}

fn restore_placement<R: Runtime>(
    window: &WebviewWindow<R>,
    logical_size: LogicalSize<f64>,
    placement: &WindowPlacement,
) -> tauri::Result<bool> {
    let monitors = window.available_monitors()?;
    if monitors.is_empty() {
        return Ok(false);
    }
    let monitor_geometries = monitors
        .iter()
        .map(MonitorGeometry::from)
        .collect::<Vec<_>>();
    let primary = window.primary_monitor()?;
    let primary_index = primary.as_ref().and_then(|primary| {
        let primary = MonitorGeometry::from(primary);
        monitor_geometries
            .iter()
            .position(|monitor| monitor.same_display_as(&primary))
    });
    let target_index = select_monitor_index(placement, &monitor_geometries, primary_index)
        .unwrap_or(primary_index.unwrap_or(0));
    let target = &monitor_geometries[target_index];
    let requested_size: PhysicalSize<u32> = logical_size.to_physical(target.scale_factor);
    let margin = (EDGE_MARGIN_LOGICAL * target.scale_factor).round() as i32;
    let geometry = placement_geometry(placement, target, requested_size, margin);
    apply_geometry(window, geometry, target.scale_factor)?;
    Ok(true)
}

fn resize_at_current_position<R: Runtime>(
    window: &WebviewWindow<R>,
    logical_size: LogicalSize<f64>,
) -> tauri::Result<()> {
    if let Some(placement) = window
        .try_state::<WindowPlacementState>()
        .and_then(|state| state.placement())
        && restore_placement(window, logical_size, &placement)?
    {
        return Ok(());
    }

    let current_position = window.outer_position()?;
    let current_size = window.outer_size()?;
    let monitors = window.available_monitors()?;
    let monitor_geometries = monitors
        .iter()
        .map(MonitorGeometry::from)
        .collect::<Vec<_>>();
    let Some(monitor_index) =
        select_physical_monitor_index(current_position, current_size, &monitor_geometries)
    else {
        return window.set_size(logical_size);
    };
    let monitor = &monitor_geometries[monitor_index];
    let scale_factor = monitor.scale_factor;
    let requested_size: PhysicalSize<u32> = logical_size.to_physical(scale_factor);
    let margin = (EDGE_MARGIN_LOGICAL * scale_factor).round() as i32;
    let geometry = anchored_geometry(
        current_position,
        current_size,
        monitor.work_position,
        monitor.work_size,
        requested_size,
        margin,
    );
    apply_geometry(window, geometry, scale_factor)
}

pub fn show<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    reveal(&window, true)
}

pub fn present<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    reveal(&window, false)
}

fn reveal<R: Runtime>(window: &WebviewWindow<R>, focus: bool) -> tauri::Result<()> {
    if window.is_minimized()? {
        window.unminimize()?;
    }
    position_using_current_size(window)?;
    window.show()?;
    if focus {
        window.set_focus()?;
    }
    Ok(())
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    window.hide()
}

fn position_using_current_size<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
    let current_position = window.outer_position()?;
    let current_size = window.outer_size()?;
    let current_scale_factor = window.scale_factor()?;
    let monitors = window.available_monitors()?;
    let monitor_geometries = monitors
        .iter()
        .map(MonitorGeometry::from)
        .collect::<Vec<_>>();
    if monitor_geometries.is_empty() {
        return Ok(());
    }
    let primary = window.primary_monitor()?;
    let primary_index = primary.as_ref().and_then(|primary| {
        let primary = MonitorGeometry::from(primary);
        monitor_geometries
            .iter()
            .position(|monitor| monitor.same_display_as(&primary))
    });
    let placement = window
        .try_state::<WindowPlacementState>()
        .and_then(|state| state.placement());
    let monitor_index = placement
        .as_ref()
        .and_then(|placement| select_monitor_index(placement, &monitor_geometries, primary_index));
    let monitor_index = monitor_index
        .or_else(|| {
            select_physical_monitor_index(current_position, current_size, &monitor_geometries)
        })
        .or(primary_index)
        .unwrap_or(0);
    let monitor = &monitor_geometries[monitor_index];
    let scale_factor = monitor.scale_factor;
    let requested_size =
        retarget_physical_size(current_size, current_scale_factor, monitor.scale_factor);
    let margin = (EDGE_MARGIN_LOGICAL * scale_factor).round() as i32;
    let geometry = placement.as_ref().map_or_else(
        || {
            anchored_geometry(
                current_position,
                requested_size,
                monitor.work_position,
                monitor.work_size,
                requested_size,
                margin,
            )
        },
        |placement| placement_geometry(placement, monitor, requested_size, margin),
    );
    apply_geometry(window, geometry, scale_factor)
}

fn resize_and_position_logical<R: Runtime>(
    window: &WebviewWindow<R>,
    logical_size: LogicalSize<f64>,
) -> tauri::Result<Option<WindowPlacement>> {
    let current_position = window.outer_position()?;
    let current_size = window.outer_size()?;
    let monitors = window.available_monitors()?;
    let monitor_geometries = monitors
        .iter()
        .map(MonitorGeometry::from)
        .collect::<Vec<_>>();
    let Some(monitor_index) =
        select_physical_monitor_index(current_position, current_size, &monitor_geometries)
    else {
        window.set_size(logical_size)?;
        return Ok(None);
    };
    let monitor = &monitor_geometries[monitor_index];
    let scale_factor = monitor.scale_factor;
    let requested_size: PhysicalSize<u32> = logical_size.to_physical(scale_factor);
    let margin = (EDGE_MARGIN_LOGICAL * scale_factor).round() as i32;
    let geometry = bottom_right_geometry(
        monitor.work_position,
        monitor.work_size,
        requested_size,
        margin,
    );
    let placement = WindowPlacement::from_geometry(monitor, geometry.position, geometry.size);
    apply_geometry(window, geometry, scale_factor)?;
    Ok(Some(placement))
}

fn apply_geometry<R: Runtime>(
    window: &WebviewWindow<R>,
    geometry: WindowGeometry,
    scale_factor: f64,
) -> tauri::Result<()> {
    let current_position = window.outer_position().ok();
    if let Some(state) = window.try_state::<WindowPlacementState>()
        && state.expect_programmatic_move(current_position, geometry.position)
        && let Err(error) = window.emit(WINDOW_DRAG_EVENT, WindowDragPayload::settled())
    {
        eprintln!("failed to emit companion window drag event: {error}");
    }

    let (minimum_width, minimum_height) = WindowMode::Compact.dimensions();
    let requested_minimum: PhysicalSize<u32> =
        LogicalSize::new(minimum_width, minimum_height).to_physical(scale_factor);
    let minimum_size = PhysicalSize::new(
        requested_minimum.width.min(geometry.size.width),
        requested_minimum.height.min(geometry.size.height),
    );

    window.set_min_size(Some(minimum_size))?;
    window.set_size(geometry.size)?;
    window.set_position(geometry.position)
}

fn retarget_physical_size(
    size: PhysicalSize<u32>,
    source_scale_factor: f64,
    target_scale_factor: f64,
) -> PhysicalSize<u32> {
    let logical_size: LogicalSize<f64> = size.to_logical(source_scale_factor);
    logical_size.to_physical(target_scale_factor)
}

#[derive(Debug, Clone, PartialEq)]
struct MonitorGeometry {
    name: Option<String>,
    work_position: PhysicalPosition<i32>,
    work_size: PhysicalSize<u32>,
    scale_factor: f64,
}

impl From<&Monitor> for MonitorGeometry {
    fn from(monitor: &Monitor) -> Self {
        Self {
            name: monitor.name().cloned(),
            work_position: monitor.work_area().position,
            work_size: monitor.work_area().size,
            scale_factor: monitor.scale_factor(),
        }
    }
}

impl MonitorGeometry {
    fn same_display_as(&self, other: &Self) -> bool {
        self.name == other.name
            && self.work_position == other.work_position
            && self.work_size == other.work_size
    }

    fn center(&self) -> (f64, f64) {
        (
            f64::from(self.work_position.x) + f64::from(self.work_size.width) / 2.0,
            f64::from(self.work_position.y) + f64::from(self.work_size.height) / 2.0,
        )
    }
}

fn select_physical_monitor_index(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    monitors: &[MonitorGeometry],
) -> Option<usize> {
    let interior_anchor = (
        i64::from(position.x) + i64::from(size.width.max(1)) - 1,
        i64::from(position.y) + i64::from(size.height.max(1)) - 1,
    );
    let anchor_as_f64 = (interior_anchor.0 as f64, interior_anchor.1 as f64);
    let mut selected: Option<(usize, u64, bool, f64)> = None;

    for (index, monitor) in monitors.iter().enumerate() {
        let overlap = physical_overlap_area(position, size, monitor);
        let contains_anchor = contains_physical_point(monitor, interior_anchor);
        let distance = squared_distance(monitor.center(), anchor_as_f64);
        let replace = selected.as_ref().is_none_or(
            |(_, selected_overlap, selected_contains_anchor, selected_distance)| {
                overlap > *selected_overlap
                    || (overlap == *selected_overlap
                        && contains_anchor
                        && !selected_contains_anchor)
                    || (overlap == *selected_overlap
                        && contains_anchor == *selected_contains_anchor
                        && distance < *selected_distance)
            },
        );
        if replace {
            selected = Some((index, overlap, contains_anchor, distance));
        }
    }

    selected.map(|(index, _, _, _)| index)
}

fn physical_overlap_area(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    monitor: &MonitorGeometry,
) -> u64 {
    let window_left = i64::from(position.x);
    let window_top = i64::from(position.y);
    let window_right = window_left + i64::from(size.width);
    let window_bottom = window_top + i64::from(size.height);
    let monitor_left = i64::from(monitor.work_position.x);
    let monitor_top = i64::from(monitor.work_position.y);
    let monitor_right = monitor_left + i64::from(monitor.work_size.width);
    let monitor_bottom = monitor_top + i64::from(monitor.work_size.height);
    let overlap_width = window_right.min(monitor_right) - window_left.max(monitor_left);
    let overlap_height = window_bottom.min(monitor_bottom) - window_top.max(monitor_top);

    if overlap_width <= 0 || overlap_height <= 0 {
        return 0;
    }
    u64::try_from(overlap_width)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(overlap_height).unwrap_or(u64::MAX))
}

fn contains_physical_point(monitor: &MonitorGeometry, point: (i64, i64)) -> bool {
    let left = i64::from(monitor.work_position.x);
    let top = i64::from(monitor.work_position.y);
    let right = left + i64::from(monitor.work_size.width);
    let bottom = top + i64::from(monitor.work_size.height);
    point.0 >= left && point.0 < right && point.1 >= top && point.1 < bottom
}

fn select_monitor_index(
    placement: &WindowPlacement,
    monitors: &[MonitorGeometry],
    primary_index: Option<usize>,
) -> Option<usize> {
    if monitors.is_empty() {
        return None;
    }

    let saved_center = (
        f64::from(placement.monitor_work_x) + f64::from(placement.monitor_work_width) / 2.0,
        f64::from(placement.monitor_work_y) + f64::from(placement.monitor_work_height) / 2.0,
    );
    if let Some(name) = placement.monitor_name.as_ref()
        && let Some(index) = closest_index(
            monitors,
            monitors
                .iter()
                .enumerate()
                .filter(|(_, monitor)| monitor.name.as_ref() == Some(name))
                .map(|(index, _)| index),
            saved_center,
        )
    {
        return Some(index);
    }

    if let Some((index, _)) = monitors.iter().enumerate().find(|(_, monitor)| {
        monitor.work_position.x == placement.monitor_work_x
            && monitor.work_position.y == placement.monitor_work_y
            && monitor.work_size.width == placement.monitor_work_width
            && monitor.work_size.height == placement.monitor_work_height
    }) {
        return Some(index);
    }

    closest_index(monitors, 0..monitors.len(), placement.absolute_anchor()).or(primary_index)
}

fn closest_index(
    monitors: &[MonitorGeometry],
    candidates: impl IntoIterator<Item = usize>,
    point: (f64, f64),
) -> Option<usize> {
    candidates.into_iter().min_by(|left, right| {
        squared_distance(monitors[*left].center(), point)
            .total_cmp(&squared_distance(monitors[*right].center(), point))
    })
}

fn squared_distance(left: (f64, f64), right: (f64, f64)) -> f64 {
    let delta_x = left.0 - right.0;
    let delta_y = left.1 - right.1;
    delta_x.mul_add(delta_x, delta_y * delta_y)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowGeometry {
    size: PhysicalSize<u32>,
    position: PhysicalPosition<i32>,
}

fn placement_geometry(
    placement: &WindowPlacement,
    monitor: &MonitorGeometry,
    requested_size: PhysicalSize<u32>,
    margin: i32,
) -> WindowGeometry {
    let anchor_x = f64::from(monitor.work_position.x)
        + placement.anchor_x * f64::from(monitor.work_size.width);
    let anchor_y = f64::from(monitor.work_position.y)
        + placement.anchor_y * f64::from(monitor.work_size.height);
    geometry_for_anchor(
        (anchor_x.round() as i64, anchor_y.round() as i64),
        monitor.work_position,
        monitor.work_size,
        requested_size,
        margin,
    )
}

fn bottom_right_geometry(
    work_position: PhysicalPosition<i32>,
    work_size: PhysicalSize<u32>,
    requested_size: PhysicalSize<u32>,
    margin: i32,
) -> WindowGeometry {
    let margin_u32 = u32::try_from(margin.max(0)).unwrap_or(0);
    let reserved_margin = margin_u32.saturating_mul(2);
    let size = PhysicalSize::new(
        requested_size
            .width
            .min(work_size.width.saturating_sub(reserved_margin).max(1)),
        requested_size
            .height
            .min(work_size.height.saturating_sub(reserved_margin).max(1)),
    );
    let position = bottom_right_position(work_position, work_size, size, margin);
    WindowGeometry { size, position }
}

fn anchored_geometry(
    current_position: PhysicalPosition<i32>,
    current_size: PhysicalSize<u32>,
    work_position: PhysicalPosition<i32>,
    work_size: PhysicalSize<u32>,
    requested_size: PhysicalSize<u32>,
    margin: i32,
) -> WindowGeometry {
    let anchored_x = i64::from(current_position.x) + i64::from(current_size.width);
    let anchored_y = i64::from(current_position.y) + i64::from(current_size.height);
    geometry_for_anchor(
        (anchored_x, anchored_y),
        work_position,
        work_size,
        requested_size,
        margin,
    )
}

fn geometry_for_anchor(
    anchor: (i64, i64),
    work_position: PhysicalPosition<i32>,
    work_size: PhysicalSize<u32>,
    requested_size: PhysicalSize<u32>,
    margin: i32,
) -> WindowGeometry {
    let margin_u32 = u32::try_from(margin.max(0)).unwrap_or(0);
    let reserved_margin = margin_u32.saturating_mul(2);
    let size = PhysicalSize::new(
        requested_size
            .width
            .min(work_size.width.saturating_sub(reserved_margin).max(1)),
        requested_size
            .height
            .min(work_size.height.saturating_sub(reserved_margin).max(1)),
    );
    let minimum_x = i64::from(work_position.x) + i64::from(margin.max(0));
    let minimum_y = i64::from(work_position.y) + i64::from(margin.max(0));
    let maximum_x = i64::from(work_position.x) + i64::from(work_size.width)
        - i64::from(size.width)
        - i64::from(margin.max(0));
    let maximum_y = i64::from(work_position.y) + i64::from(work_size.height)
        - i64::from(size.height)
        - i64::from(margin.max(0));
    let anchored_x = anchor.0 - i64::from(size.width);
    let anchored_y = anchor.1 - i64::from(size.height);
    let position = PhysicalPosition::new(
        anchored_x
            .clamp(minimum_x, maximum_x.max(minimum_x))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        anchored_y
            .clamp(minimum_y, maximum_y.max(minimum_y))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    );
    WindowGeometry { size, position }
}

fn bottom_right_position(
    work_position: PhysicalPosition<i32>,
    work_size: PhysicalSize<u32>,
    window_size: PhysicalSize<u32>,
    margin: i32,
) -> PhysicalPosition<i32> {
    let margin = u32::try_from(margin.max(0)).unwrap_or(0);
    let x_offset = work_size
        .width
        .saturating_sub(window_size.width)
        .saturating_sub(margin);
    let y_offset = work_size
        .height
        .saturating_sub(window_size.height)
        .saturating_sub(margin);
    let x = i64::from(work_position.x)
        .saturating_add(i64::from(x_offset))
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let y = i64::from(work_position.y)
        .saturating_add(i64::from(y_offset))
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    PhysicalPosition::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_placement() -> WindowPlacement {
        WindowPlacement {
            schema_version: PLACEMENT_SCHEMA_VERSION,
            monitor_name: Some("External Display".into()),
            monitor_work_x: 1440,
            monitor_work_y: 0,
            monitor_work_width: 1920,
            monitor_work_height: 1080,
            anchor_x: 0.75,
            anchor_y: 0.8,
        }
    }

    fn monitor(name: &str, x: i32, y: i32, width: u32, height: u32) -> MonitorGeometry {
        MonitorGeometry {
            name: Some(name.into()),
            work_position: PhysicalPosition::new(x, y),
            work_size: PhysicalSize::new(width, height),
            scale_factor: 1.0,
        }
    }

    fn monitor_with_scale(
        name: &str,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        scale_factor: f64,
    ) -> MonitorGeometry {
        MonitorGeometry {
            scale_factor,
            ..monitor(name, x, y, width, height)
        }
    }

    #[test]
    fn placement_store_is_atomic_bounded_and_metadata_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(PLACEMENT_FILE_NAME);
        let store = WindowPlacementStore::at_path(&path);
        let first = saved_placement();
        store.save(&first).unwrap();

        let mut latest = first;
        latest.anchor_x = 0.5;
        latest.anchor_y = 0.6;
        store.save(&latest).unwrap();

        assert_eq!(store.load().unwrap(), Some(latest));
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.len() <= MAX_PLACEMENT_BYTES);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "anchorX",
                "anchorY",
                "monitorName",
                "monitorWorkHeight",
                "monitorWorkWidth",
                "monitorWorkX",
                "monitorWorkY",
                "schemaVersion",
            ]
        );
        assert!(directory.path().read_dir().unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[test]
    fn placement_store_rejects_oversized_and_unknown_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(PLACEMENT_FILE_NAME);
        let store = WindowPlacementStore::at_path(&path);

        fs::write(&path, vec![b' '; MAX_PLACEMENT_BYTES + 1]).unwrap();
        assert!(matches!(store.load(), Err(WindowPlacementError::TooLarge)));

        let mut value = serde_json::to_value(saved_placement()).unwrap();
        value["token"] = serde_json::json!("must-not-be-accepted");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(store.load(), Err(WindowPlacementError::Json(_))));
    }

    #[test]
    fn debounced_updates_persist_only_the_latest_drag_position() {
        let directory = tempfile::tempdir().unwrap();
        let store = WindowPlacementStore::in_directory(directory.path());
        let state = WindowPlacementState::new(store.clone(), None);
        let mut first = saved_placement();
        first.anchor_x = 0.25;
        let mut latest = first.clone();
        latest.anchor_x = 0.6;

        state.remember_and_schedule_save(first);
        state.remember_and_schedule_save(latest.clone());
        std::thread::sleep(SAVE_DEBOUNCE + Duration::from_millis(150));

        assert_eq!(store.load().unwrap(), Some(latest));
    }

    #[test]
    fn explicit_flush_persists_the_latest_position_before_the_debounce() {
        let directory = tempfile::tempdir().unwrap();
        let store = WindowPlacementStore::in_directory(directory.path());
        let state = WindowPlacementState::new(store.clone(), None);
        let latest = saved_placement();

        state.remember_and_schedule_save(latest.clone());
        state.flush().unwrap();

        assert_eq!(store.load().unwrap(), Some(latest));
    }

    #[test]
    fn removed_display_falls_back_and_clamps_the_full_window_on_screen() {
        let mut placement = saved_placement();
        placement.monitor_name = Some("Removed Display".into());
        placement.monitor_work_x = 3840;
        placement.anchor_x = 1.0;
        placement.anchor_y = 1.0;
        let monitors = [
            monitor("Left", -1920, 0, 1920, 1080),
            monitor("Primary", 0, 24, 1440, 876),
        ];

        let index = select_monitor_index(&placement, &monitors, Some(1)).unwrap();
        assert_eq!(index, 1);
        let geometry = placement_geometry(
            &placement,
            &monitors[index],
            PhysicalSize::new(404, 640),
            18,
        );

        assert!(geometry.position.x >= 18);
        assert!(geometry.position.y >= 42);
        assert!(
            i64::from(geometry.position.x) + i64::from(geometry.size.width) <= i64::from(1440 - 18)
        );
        assert!(
            i64::from(geometry.position.y) + i64::from(geometry.size.height)
                <= i64::from(24 + 876 - 18)
        );
    }

    #[test]
    fn matching_display_keeps_the_relative_anchor_after_rearrangement() {
        let placement = saved_placement();
        let monitors = [
            monitor("Primary", 0, 24, 1440, 876),
            monitor("External Display", -2560, 0, 2560, 1440),
        ];

        assert_eq!(
            select_monitor_index(&placement, &monitors, Some(0)),
            Some(1)
        );
        let geometry =
            placement_geometry(&placement, &monitors[1], PhysicalSize::new(404, 640), 18);
        assert_eq!(geometry.position, PhysicalPosition::new(-1044, 512));
    }

    #[test]
    fn mixed_dpi_monitor_selection_uses_physical_window_overlap() {
        let monitors = [
            monitor_with_scale("Retina", 0, 0, 2880, 1800, 2.0),
            monitor_with_scale("External", 2880, 0, 1920, 1080, 1.0),
        ];

        assert_eq!(
            select_physical_monitor_index(
                PhysicalPosition::new(2400, 200),
                PhysicalSize::new(600, 472),
                &monitors,
            ),
            Some(0)
        );
        assert_eq!(
            select_physical_monitor_index(
                PhysicalPosition::new(3000, 200),
                PhysicalSize::new(236, 236),
                &monitors,
            ),
            Some(1)
        );
    }

    #[test]
    fn exact_display_boundaries_are_half_open() {
        let monitors = [
            monitor("Primary", 0, 0, 1440, 900),
            monitor("Right", 1440, 0, 1920, 1080),
        ];

        assert_eq!(
            select_physical_monitor_index(
                PhysicalPosition::new(1340, 200),
                PhysicalSize::new(100, 100),
                &monitors,
            ),
            Some(0)
        );
        assert_eq!(
            select_physical_monitor_index(
                PhysicalPosition::new(1440, 200),
                PhysicalSize::new(100, 100),
                &monitors,
            ),
            Some(1)
        );
    }

    #[test]
    fn bottom_right_position_uses_the_work_area_origin_and_size() {
        let position = bottom_right_position(
            PhysicalPosition::new(0, 24),
            PhysicalSize::new(1440, 876),
            PhysicalSize::new(404, 640),
            18,
        );

        assert_eq!(position, PhysicalPosition::new(1018, 242));
    }

    #[test]
    fn first_launch_placement_is_derived_from_the_target_geometry() {
        let monitor = monitor("Primary", 0, 24, 1440, 876);
        let geometry = bottom_right_geometry(
            monitor.work_position,
            monitor.work_size,
            PhysicalSize::new(236, 236),
            18,
        );
        let placement = WindowPlacement::from_geometry(&monitor, geometry.position, geometry.size);

        assert_eq!(geometry.position, PhysicalPosition::new(1186, 646));
        assert_eq!(
            placement_geometry(&placement, &monitor, geometry.size, 18),
            geometry
        );
    }

    #[test]
    fn bottom_right_position_supports_monitors_left_of_the_primary() {
        let position = bottom_right_position(
            PhysicalPosition::new(-1920, 0),
            PhysicalSize::new(1920, 1080),
            PhysicalSize::new(236, 236),
            18,
        );

        assert_eq!(position, PhysicalPosition::new(-254, 826));
    }

    #[test]
    fn oversized_window_is_clamped_inside_the_work_area() {
        let geometry = bottom_right_geometry(
            PhysicalPosition::new(0, 24),
            PhysicalSize::new(320, 568),
            PhysicalSize::new(404, 640),
            18,
        );

        assert_eq!(geometry.size, PhysicalSize::new(284, 532));
        assert_eq!(geometry.position, PhysicalPosition::new(18, 42));
    }

    #[test]
    fn resizing_keeps_the_user_position_until_an_edge_requires_clamping() {
        let geometry = anchored_geometry(
            PhysicalPosition::new(1186, 622),
            PhysicalSize::new(236, 236),
            PhysicalPosition::new(0, 24),
            PhysicalSize::new(1440, 876),
            PhysicalSize::new(404, 640),
            18,
        );

        assert_eq!(geometry.size, PhysicalSize::new(404, 640));
        assert_eq!(geometry.position, PhysicalPosition::new(1018, 218));
    }

    #[test]
    fn collapsing_restores_the_compact_pet_to_the_same_bottom_right_anchor() {
        let geometry = anchored_geometry(
            PhysicalPosition::new(1018, 218),
            PhysicalSize::new(404, 640),
            PhysicalPosition::new(0, 24),
            PhysicalSize::new(1440, 876),
            PhysicalSize::new(236, 236),
            18,
        );

        assert_eq!(geometry.position, PhysicalPosition::new(1186, 622));
    }

    #[test]
    fn canonical_placement_restores_top_and_left_edge_round_trips() {
        let monitor = monitor("Primary", 0, 24, 1440, 876);
        let compact_size = PhysicalSize::new(236, 236);
        let expanded_size = PhysicalSize::new(404, 640);

        for compact_position in [
            PhysicalPosition::new(18, 300),
            PhysicalPosition::new(500, 42),
            PhysicalPosition::new(18, 42),
        ] {
            let canonical =
                WindowPlacement::from_geometry(&monitor, compact_position, compact_size);
            let expanded = placement_geometry(&canonical, &monitor, expanded_size, 18);
            let collapsed = placement_geometry(&canonical, &monitor, compact_size, 18);

            assert_eq!(collapsed.position, compact_position);
            assert_eq!(collapsed.size, compact_size);
            if compact_position.x == 18 {
                assert_eq!(expanded.position.x, 18);
            }
            if compact_position.y == 42 {
                assert_eq!(expanded.position.y, 42);
            }
        }
    }

    #[test]
    fn programmatic_moves_do_not_replace_canonical_placement() {
        let directory = tempfile::tempdir().unwrap();
        let canonical = saved_placement();
        let state = WindowPlacementState::new(
            WindowPlacementStore::in_directory(directory.path()),
            Some(canonical.clone()),
        );
        let target = PhysicalPosition::new(18, 42);

        state.expect_programmatic_move(Some(PhysicalPosition::new(64, 96)), target);
        assert_eq!(state.observe_move(target), ObservedMove::Programmatic);
        assert_eq!(state.observe_move(target), ObservedMove::Programmatic);
        assert_eq!(state.placement(), Some(canonical));

        assert!(matches!(
            state.observe_move(PhysicalPosition::new(24, 42)),
            ObservedMove::User {
                direction: Some(WindowDragDirection::Right),
                ..
            }
        ));
    }

    #[test]
    fn no_op_programmatic_move_accepts_the_first_real_drag_immediately() {
        let directory = tempfile::tempdir().unwrap();
        let state = WindowPlacementState::new(
            WindowPlacementStore::in_directory(directory.path()),
            Some(saved_placement()),
        );
        let target = PhysicalPosition::new(18, 42);

        state.expect_programmatic_move(Some(target), target);

        assert!(matches!(
            state.observe_move(PhysicalPosition::new(24, 42)),
            ObservedMove::User {
                direction: Some(WindowDragDirection::Right),
                ..
            }
        ));
    }

    #[test]
    fn retargeting_between_dpi_scales_preserves_logical_window_size() {
        assert_eq!(
            retarget_physical_size(PhysicalSize::new(808, 1280), 2.0, 1.0),
            PhysicalSize::new(404, 640)
        );
        assert_eq!(
            retarget_physical_size(PhysicalSize::new(404, 640), 1.0, 2.0),
            PhysicalSize::new(808, 1280)
        );
    }

    #[test]
    fn drag_lifecycle_invalidates_old_debounce_and_supports_fallback_settle() {
        let mut lifecycle = DragLifecycleState::default();
        let fallback_revision = lifecycle.begin(PhysicalPosition::new(100, 100));
        assert!(lifecycle.settle_if_current(fallback_revision));

        let first_revision = lifecycle.begin(PhysicalPosition::new(100, 100));
        let moving_right = lifecycle.observe_user_move(PhysicalPosition::new(120, 100));
        assert_eq!(moving_right.direction, Some(WindowDragDirection::Right));
        assert!(!lifecycle.settle_if_current(first_revision));

        let moving_left = lifecycle.observe_user_move(PhysicalPosition::new(110, 100));
        assert_eq!(moving_left.direction, Some(WindowDragDirection::Left));
        assert!(!lifecycle.settle_if_current(moving_right.revision));
        let resized_revision = lifecycle.observe_geometry_change().unwrap();
        assert!(!lifecycle.settle_if_current(moving_left.revision));
        assert!(lifecycle.settle_if_current(resized_revision));
        assert!(!lifecycle.settle_if_current(resized_revision));
        assert_eq!(lifecycle.observe_geometry_change(), None);
    }

    #[test]
    fn placement_state_keeps_final_geometry_capture_eligible_until_settle() {
        let directory = tempfile::tempdir().unwrap();
        let state = WindowPlacementState::new(
            WindowPlacementStore::in_directory(directory.path()),
            Some(saved_placement()),
        );

        let started_revision = state.begin_drag(PhysicalPosition::new(100, 100));
        assert!(state.drag_is_active());
        let resized_revision = state.observe_drag_geometry_change().unwrap();
        assert!(!state.settle_drag(started_revision));
        assert!(state.drag_is_active());
        assert!(state.settle_drag(resized_revision));
        assert!(!state.drag_is_active());
    }

    #[test]
    fn drag_event_contract_serializes_settled_direction_as_null() {
        assert_eq!(
            serde_json::to_value(WindowDragPayload::moving(WindowDragDirection::Left)).unwrap(),
            serde_json::json!({ "phase": "moving", "direction": "left" })
        );
        assert_eq!(
            serde_json::to_value(WindowDragPayload::settled()).unwrap(),
            serde_json::json!({ "phase": "settled", "direction": null })
        );
    }
}
