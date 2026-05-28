use serde::Serialize;
use std::path::PathBuf;

/// Diagnostic information about the island's health and configuration.
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsReport {
    pub hook_status: HookDiagnostics,
    pub server_status: ServerDiagnostics,
    pub config_status: ConfigDiagnostics,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookDiagnostics {
    pub claude_installed: bool,
    pub codex_installed: bool,
    pub gemini_installed: bool,
    pub cursor_installed: bool,
    pub claude_config_path: String,
    pub hooks_valid: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerDiagnostics {
    pub tcp_port: u16,
    pub http_port: u16,
    pub auth_token_prefix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigDiagnostics {
    pub data_dir: String,
    pub sessions_file_exists: bool,
    pub autostart_enabled: bool,
    pub exe_path: String,
}

/// Run a full diagnostic check and return the report.
pub fn run_diagnostics(auth_token: &str) -> DiagnosticsReport {
    let claude_config = crate::hook_manager::HookManager::claude_config_path();
    let hooks = crate::hook_manager::HookManager::new();

    let claude_installed = hooks.is_claude_installed();
    let codex_installed = hooks.is_codex_installed();
    let gemini_installed = hooks.is_gemini_installed();
    let cursor_installed = hooks.is_cursor_installed();

    // Validate Claude hooks config format
    let hooks_valid = validate_claude_hooks(&claude_config);

    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenIsland");
    let sessions_file = data_dir.join("sessions.json");

    DiagnosticsReport {
        hook_status: HookDiagnostics {
            claude_installed,
            codex_installed,
            gemini_installed,
            cursor_installed,
            claude_config_path: claude_config.to_string_lossy().to_string(),
            hooks_valid,
        },
        server_status: ServerDiagnostics {
            tcp_port: 19841,
            http_port: 51515,
            auth_token_prefix: auth_token.chars().take(8).collect::<String>(),
        },
        config_status: ConfigDiagnostics {
            data_dir: data_dir.to_string_lossy().to_string(),
            sessions_file_exists: sessions_file.exists(),
            autostart_enabled: crate::autostart::AutoStartManager::is_enabled(),
            exe_path: std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        },
    }
}

/// Validate that Claude hooks config has the expected format.
fn validate_claude_hooks(config_path: &PathBuf) -> bool {
    if !config_path.exists() {
        return false;
    }
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let settings: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };

    // Check that hooks object exists and has the expected events
    let hooks = match settings.get("hooks") {
        Some(h) => h,
        None => return false,
    };

    let required_events = ["PreToolUse", "PermissionRequest", "Stop"];
    for event in &required_events {
        if hooks.get(*event).is_none() {
            tracing::warn!("[DIAG] Missing hook event: {}", event);
            return false;
        }
    }

    true
}
