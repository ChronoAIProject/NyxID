use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow,
};

const EDGE_MARGIN_LOGICAL: f64 = 18.0;

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

pub fn configure_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    window.set_always_on_top(true)?;
    set_mode(app, WindowMode::Compact)?;
    Ok(())
}

pub fn set_mode<R: Runtime>(app: &AppHandle<R>, mode: WindowMode) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };
    let (width, height) = mode.dimensions();
    resize_and_position_logical(&window, LogicalSize::new(width, height))
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
    let requested_size = window.outer_size()?;
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(());
    };
    let scale_factor = monitor.scale_factor();
    let work_area = monitor.work_area();
    apply_window_geometry(
        window,
        work_area.position,
        work_area.size,
        requested_size,
        scale_factor,
    )
}

fn resize_and_position_logical<R: Runtime>(
    window: &WebviewWindow<R>,
    logical_size: LogicalSize<f64>,
) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return window.set_size(logical_size);
    };
    let scale_factor = monitor.scale_factor();
    let requested_size: PhysicalSize<u32> = logical_size.to_physical(scale_factor);
    let work_area = monitor.work_area();
    apply_window_geometry(
        window,
        work_area.position,
        work_area.size,
        requested_size,
        scale_factor,
    )
}

fn apply_window_geometry<R: Runtime>(
    window: &WebviewWindow<R>,
    work_position: PhysicalPosition<i32>,
    work_size: PhysicalSize<u32>,
    requested_size: PhysicalSize<u32>,
    scale_factor: f64,
) -> tauri::Result<()> {
    let margin = (EDGE_MARGIN_LOGICAL * scale_factor).round() as i32;
    let geometry = bottom_right_geometry(work_position, work_size, requested_size, margin);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowGeometry {
    size: PhysicalSize<u32>,
    position: PhysicalPosition<i32>,
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
}
