/// Multi-monitor support with stable display key persistence.
/// Inspired by EchoIsland's geometry-based approach.
use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewWindow};

/// Geometry-based stable display key format: "Display|x|y|w|h"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayGeometry {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// Display option for UI selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayOption {
    pub index: usize,
    pub key: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub is_current: bool,
}

/// Generate a stable geometry-based key for a monitor.
/// Format: "Display|x|y|width|height" using physical pixel coordinates.
pub fn display_key(geometry: &DisplayGeometry) -> String {
    format!("Display|{}|{}|{}|{}", geometry.x, geometry.y, geometry.width, geometry.height)
}

/// Get geometry from a Tauri monitor.
pub fn geometry_from_monitor(monitor: &tauri::Monitor) -> DisplayGeometry {
    let position = monitor.position();
    let size = monitor.size();
    DisplayGeometry {
        x: position.x as i64,
        y: position.y as i64,
        width: size.width as i64,
        height: size.height as i64,
    }
}

/// List all available displays with stable keys.
pub fn list_displays(app: &tauri::AppHandle, current_monitor: Option<&tauri::Monitor>) -> Vec<DisplayOption> {
    let monitors = app.available_monitors().unwrap_or_default();
    let current_geom = current_monitor.map(geometry_from_monitor);

    monitors.iter().enumerate().map(|(index, monitor)| {
        let geom = geometry_from_monitor(monitor);
        let key = display_key(&geom);
        let is_current = current_geom.as_ref().map_or(false, |c| {
            c.x == geom.x && c.y == geom.y && c.width == geom.width && c.height == geom.height
        });

        DisplayOption {
            index,
            key,
            name: monitor.name().cloned().unwrap_or_else(|| format!("Display {}", index + 1)),
            width: geom.width.max(0) as u32,
            height: geom.height.max(0) as u32,
            is_current,
        }
    }).collect()
}

/// Resolve the preferred display index using a fallback chain:
/// 1. Geometry key match (survives monitor reordering)
/// 2. Saved index (works if monitors didn't reorder)
/// 3. Fallback to current or first display
pub fn resolve_display_index(
    displays: &[DisplayOption],
    preferred_key: Option<&str>,
    preferred_index: usize,
) -> usize {
    // 1. Try geometry key match
    if let Some(key) = preferred_key {
        if let Some(idx) = displays.iter().position(|d| d.key == key) {
            return idx;
        }
    }
    // 2. Try saved index
    if preferred_index < displays.len() {
        return preferred_index;
    }
    // 3. Try current display
    if let Some(idx) = displays.iter().position(|d| d.is_current) {
        return idx;
    }
    // 4. Fallback to first
    0
}

/// Center the island window on the specified monitor.
pub fn center_on_monitor(
    window: &WebviewWindow,
    monitor: &tauri::Monitor,
    width: f64,
    height: f64,
    top_margin: f64,
) -> Result<(), String> {
    let scale = monitor.scale_factor();
    let work_pos = monitor.work_area().position;
    let work_size = monitor.work_area().size;

    let left = work_pos.x as f64 / scale + (work_size.width as f64 / scale - width) / 2.0;
    let top = work_pos.y as f64 / scale + top_margin;

    window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)))
        .map_err(|e| e.to_string())?;
    window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(left, top)))
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Persistence path for display settings.
fn display_settings_path() -> std::path::PathBuf {
    let dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    dir.join("OpenIsland").join("display_settings.json")
}

/// Persisted display settings.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DisplaySettings {
    /// Geometry-based stable key for preferred display.
    pub preferred_key: Option<String>,
    /// Fallback index.
    pub preferred_index: usize,
}

impl DisplaySettings {
    pub fn load() -> Self {
        let path = display_settings_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) {
        let path = display_settings_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }
}

/// Tauri command: list available displays.
#[tauri::command]
pub fn list_available_displays(app: tauri::AppHandle) -> Result<Vec<DisplayOption>, String> {
    let window = app.get_webview_window("main");
    let current = window.as_ref().and_then(|w| w.current_monitor().ok().flatten());
    Ok(list_displays(&app, current.as_ref()))
}

/// Tauri command: set preferred display.
#[tauri::command]
pub fn set_preferred_display(display_key: String) -> Result<(), String> {
    let mut settings = DisplaySettings::load();
    settings.preferred_key = Some(display_key);
    settings.save();
    Ok(())
}
