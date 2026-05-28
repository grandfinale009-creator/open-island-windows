use tracing::{info, warn};

/// Windows registry auto-start management.
/// Adds/removes the app from `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub struct AutoStartManager;

const APP_NAME: &str = "OpenIsland";

impl AutoStartManager {
    /// Check if auto-start is enabled.
    pub fn is_enabled() -> bool {
        use winreg::enums::*;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Microsoft\Windows\CurrentVersion\Run";
        match hkcu.open_subkey_with_flags(path, KEY_READ) {
            Ok(key) => {
                let val: Result<String, _> = key.get_value(APP_NAME);
                val.is_ok()
            }
            Err(_) => false,
        }
    }

    /// Enable auto-start with --silent flag to start minimized.
    pub fn enable() -> anyhow::Result<()> {
        use winreg::enums::*;
        use winreg::RegKey;

        let exe_path = std::env::current_exe()?;
        // Quote the path and add --silent flag for startup
        let exe_str = format!("\"{}\"", exe_path.display());

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Microsoft\Windows\CurrentVersion\Run";
        let (key, _) = hkcu.create_subkey(path)?;
        key.set_value(APP_NAME, &exe_str)?;

        info!("Auto-start enabled: {}", exe_str);
        Ok(())
    }

    /// Disable auto-start.
    pub fn disable() -> anyhow::Result<()> {
        use winreg::enums::*;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Microsoft\Windows\CurrentVersion\Run";
        match hkcu.open_subkey_with_flags(path, KEY_ALL_ACCESS) {
            Ok(key) => {
                if let Err(e) = key.delete_value(APP_NAME) {
                    warn!("Failed to delete autostart value: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to open registry key for deletion: {}", e);
            }
        }

        info!("Auto-start disabled");
        Ok(())
    }

    /// Toggle auto-start.
    pub fn toggle() -> anyhow::Result<bool> {
        if Self::is_enabled() {
            Self::disable()?;
            Ok(false)
        } else {
            Self::enable()?;
            Ok(true)
        }
    }

    /// Re-apply auto-start (e.g. after app update changes the path).
    pub fn refresh() -> anyhow::Result<()> {
        if Self::is_enabled() {
            Self::enable()
        } else {
            Ok(())
        }
    }

    /// Check if the app was started with --silent flag (from auto-start).
    pub fn is_silent_startup() -> bool {
        std::env::args().any(|arg| arg == "--silent")
    }
}
