use std::sync::atomic::{AtomicIsize, Ordering};
use tracing::info;

/// Terminal jump service — activates a window by PID using Win32 API directly.
pub struct TerminalJumpService;

static FOUND_HWND: AtomicIsize = AtomicIsize::new(0);

impl TerminalJumpService {
    /// Jump to a window by process ID using Win32 API.
    pub async fn jump_to_pid(pid: u32) -> anyhow::Result<()> {
        let result = tokio::task::spawn_blocking(move || Self::activate_window_by_pid(pid)).await?;
        result
    }

    /// Activate window by PID using Win32 SetForegroundWindow + ShowWindow.
    fn activate_window_by_pid(target_pid: u32) -> anyhow::Result<()> {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowLongW, GetWindowThreadProcessId, IsWindowVisible,
            SetForegroundWindow, ShowWindow, GWL_STYLE, SW_RESTORE, SW_SHOW, WS_MINIMIZE,
        };

        FOUND_HWND.store(0, Ordering::Relaxed);

        unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: isize) -> i32 {
            let target_pid = lparam as u32;
            let mut pid: u32 = 0;
            unsafe {
                GetWindowThreadProcessId(hwnd, &mut pid as *mut u32);
            }
            if pid == target_pid && unsafe { IsWindowVisible(hwnd) } != 0 {
                FOUND_HWND.store(hwnd as isize, Ordering::Relaxed);
                return 0; // stop
            }
            1 // continue
        }

        unsafe {
            EnumWindows(Some(enum_callback), target_pid as isize);
        }

        let hwnd = FOUND_HWND.load(Ordering::Relaxed) as HWND;
        if hwnd.is_null() {
            return Err(anyhow::anyhow!("No visible window found for PID {}", target_pid));
        }

        unsafe {
            let style = GetWindowLongW(hwnd, GWL_STYLE);
            if style & WS_MINIMIZE as i32 != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            } else {
                ShowWindow(hwnd, SW_SHOW);
            }
            SetForegroundWindow(hwnd);
        }

        info!("Activated window for PID {}", target_pid);
        Ok(())
    }

    /// Jump to a terminal window by process name.
    pub async fn jump_to_process_name(name: &str) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {}.exe", name), "/FO", "CSV", "/NH"])
            .creation_flags(0x08000000)
            .output()
            .await;

        if let Ok(o) = output {
            let stdout = String::from_utf8_lossy(&o.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(',').map(|s| s.trim_matches('"')).collect();
                if parts.len() >= 2 {
                    if let Ok(pid) = parts[1].parse::<u32>() {
                        if Self::jump_to_pid(pid).await.is_ok() {
                            return Ok(());
                        }
                    }
                }
            }
        }

        Err(anyhow::anyhow!("No window found for process {}", name))
    }
}
