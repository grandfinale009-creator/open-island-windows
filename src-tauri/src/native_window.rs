/// Native Win32 window management.
/// Uses CreateRoundRectRgn + SetWindowRgn for OS-level rounded corners.

use tauri::WebviewWindow;

/// Apply native rounded corner region to the window.
/// Only rounds the bottom-left and bottom-right corners (top stays flush with screen edge).
pub fn apply_rounded_region(window: &WebviewWindow, width: i32, height: i32, radius: i32) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let pw = (width as f64 * scale).round() as i32;
    let ph = (height as f64 * scale).round() as i32;
    let pr = (radius as f64 * scale).round() as i32;

    unsafe {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateRectRgn, CreateRoundRectRgn, CombineRgn, SetWindowRgn, DeleteObject,
        };
        use windows_sys::Win32::Graphics::Gdi::RGN_OR;

        // Rectangular top portion (sharp top corners)
        let rect_top = CreateRectRgn(0, 0, pw + 1, pr + 1);
        if rect_top.is_null() { return Err("Failed to create rect region".into()); }

        // Rounded bottom portion (round bottom corners only)
        let round_bottom = CreateRoundRectRgn(0, 0, pw + 1, ph + 1, pr * 2, pr * 2);
        if round_bottom.is_null() {
            DeleteObject(rect_top as _);
            return Err("Failed to create rounded region".into());
        }

        // Combine: rect top + rounded bottom = bottom-only rounded window
        let combine_result = CombineRgn(round_bottom, rect_top, round_bottom, RGN_OR);
        DeleteObject(rect_top as _);
        if combine_result == 0 {
            DeleteObject(round_bottom as _);
            return Err("Failed to combine regions".into());
        }

        let result = SetWindowRgn(hwnd.0 as _, round_bottom, 1);
        if result == 0 {
            DeleteObject(round_bottom as _);
            return Err("Failed to apply window region".into());
        }
    }
    Ok(())
}

pub fn apply_native_topmost(window: &WebviewWindow) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE, SWP_NOOWNERZORDER};
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
        let ok = SetWindowPos(hwnd.0 as _, HWND_TOPMOST, 0, 0, 0, 0, flags);
        if ok == 0 { return Err("Failed to set topmost".into()); }
    }
    Ok(())
}

pub fn apply_no_activate(window: &WebviewWindow) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongW, SetWindowLongW, GWL_EXSTYLE};
        const WS_EX_NOACTIVATE: i32 = 0x08000000;
        let ex_style = GetWindowLongW(hwnd.0 as _, GWL_EXSTYLE);
        SetWindowLongW(hwnd.0 as _, GWL_EXSTYLE, ex_style | WS_EX_NOACTIVATE);
    }
    Ok(())
}

/// Get window dimensions for each notch state.
fn state_dimensions(state: &str) -> (i32, i32, i32) {
    match state {
        "overview" => (380, 175, 14),
        "approval" => (380, 220, 14),
        "ask"      => (340, 165, 14),
        "settings" => (380, 380, 14),
        "approved" => (260, 40, 10),
        _          => (240, 36, 10),
    }
}

/// Setup/resize the window for a given notch state.
pub fn setup_island_window(window: &WebviewWindow, state: &str) -> Result<(), String> {
    let (width, height, radius) = state_dimensions(state);
    let _ = window.set_decorations(false);
    let _ = window.set_shadow(false);
    let _ = window.set_resizable(false);
    let _ = window.set_skip_taskbar(true);
    let _ = window.set_always_on_top(true);
    let _ = window.set_background_color(Some(tauri::window::Color(0, 0, 0, 255)));
    if let Ok(Some(monitor)) = window.current_monitor().or_else(|_| window.primary_monitor()) {
        let scale = monitor.scale_factor();
        let mon_pos = monitor.position();
        let work_size = monitor.work_area().size;
        // Center horizontally within work area, pin to very top of screen (y = monitor top)
        let left = mon_pos.x as f64 / scale + (work_size.width as f64 / scale - width as f64) / 2.0;
        let top = mon_pos.y as f64 / scale;
        let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(width as f64, height as f64)));
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(left, top)));
    }
    let _ = apply_rounded_region(window, width, height, radius);
    let _ = apply_native_topmost(window);
    Ok(())
}