use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Hook installation manager — installs/uninstalls CLI hooks into agent config files.
/// Ported from `Sources/OpenIslandCore/ClaudeHookInstallationManager.swift` etc.
pub struct HookManager {
    hooks_binary_path: PathBuf,
}

/// Manifest tracking installed hooks per agent.
#[derive(Debug, Serialize, Deserialize)]
struct HookManifest {
    agent: String,
    installed_at: String,
    binary_path: String,
    config_path: String,
}

impl HookManager {
    pub fn new() -> Self {
        let hooks_binary_path = Self::managed_binary_path();
        Self { hooks_binary_path }
    }

    /// Default path for the hooks binary.
    fn managed_binary_path() -> PathBuf {
        let app_data = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        app_data
            .join("OpenIsland")
            .join("bin")
            .join("open-island-hooks.exe")
    }

    /// Get the hooks binary path.
    pub fn binary_path(&self) -> &PathBuf {
        &self.hooks_binary_path
    }

    // === Claude Code ===

    /// Get Claude Code settings.json path.
    pub fn claude_config_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".claude").join("settings.json")
    }

    /// Check if Claude Code hooks are installed.
    pub fn is_claude_installed(&self) -> bool {
        let config = Self::claude_config_path();
        if !config.exists() {
            return false;
        }
        let content = match std::fs::read_to_string(&config) {
            Ok(c) => c,
            Err(_) => return false,
        };
        content.contains("51515") || content.contains("open-island")
    }

    /// Install HTTP hooks into Claude Code settings.json.
    /// Inspired by CCIsland: Claude Code natively supports HTTP hooks.
    pub fn install_claude(&self) -> Result<()> {
        let config = Self::claude_config_path();

        // Read existing config or create new
        let mut settings: serde_json::Value = if config.exists() {
            let content = std::fs::read_to_string(&config)?;
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        // Ensure parent directory exists
        if let Some(parent) = config.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // HTTP hook entries — Claude Code uses nested format: [{matcher: "", hooks: [{type, url}]}]
        let event_url_map = [
            ("PreToolUse", "http://localhost:51515/hooks/pre-tool-use"),
            ("PostToolUse", "http://localhost:51515/hooks/post-tool-use"),
            ("Stop", "http://localhost:51515/hooks/stop"),
            ("StopFailure", "http://localhost:51515/hooks/stop-failure"),
            ("Notification", "http://localhost:51515/hooks/notification"),
            ("PermissionRequest", "http://localhost:51515/hooks/permission-request"),
        ];

        let hooks = settings
            .as_object_mut()
            .context("settings.json is not an object")?;

        let hooks_obj = hooks
            .entry("hooks")
            .or_insert_with(|| serde_json::json!({}));

        for (event, url) in &event_url_map {
            let hook_matcher = serde_json::json!([{
                "matcher": "",
                "hooks": [{
                    "type": "http",
                    "url": url
                }]
            }]);
            hooks_obj
                .as_object_mut()
                .unwrap()
                .insert((*event).to_string(), hook_matcher);
        }

        // Backup existing
        if config.exists() {
            let backup = config.with_extension("json.bak");
            let _ = std::fs::copy(&config, &backup);
        }

        // Write back
        let json = serde_json::to_string_pretty(&settings)?;
        std::fs::write(&config, json)?;

        // Save manifest
        self.save_manifest("claude", &config)?;

        info!("Claude Code hooks installed at {}", config.display());
        Ok(())
    }

    /// Uninstall hooks from Claude Code settings.json.
    pub fn uninstall_claude(&self) -> Result<()> {
        let config = Self::claude_config_path();
        if !config.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&config)?;
        let mut settings: serde_json::Value = serde_json::from_str(&content)?;

        // Remove hooks that contain open-island-hooks
        if let Some(hooks) = settings.get_mut("hooks") {
            if let Some(hooks_obj) = hooks.as_object_mut() {
                let events_to_clean: Vec<String> = hooks_obj.keys().cloned().collect();
                for event in events_to_clean {
                    if let Some(arr) = hooks_obj.get(&event) {
                        if let Some(arr) = arr.as_array() {
                            let filtered: Vec<_> = arr
                                .iter()
                                .filter(|entry| {
                                    !entry
                                        .to_string()
                                        .contains("open-island-hooks")
                                })
                                .cloned()
                                .collect();
                            hooks_obj.insert(event, serde_json::json!(filtered));
                        }
                    }
                }
            }
        }

        let json = serde_json::to_string_pretty(&settings)?;
        std::fs::write(&config, json)?;

        self.remove_manifest("claude")?;
        info!("Claude Code hooks uninstalled");
        Ok(())
    }

    /// Add a tool to Claude Code's permissions.allow list.
    /// This persists the "always allow" decision across sessions.
    pub fn add_allowed_tool(tool_name: &str) -> Result<bool> {
        let config = Self::claude_config_path();
        if !config.exists() {
            return Ok(false);
        }

        let content = std::fs::read_to_string(&config)?;
        let mut settings: serde_json::Value = serde_json::from_str(&content)?;

        // Get or create permissions.allow array
        let permissions = settings
            .as_object_mut()
            .context("settings.json is not an object")?
            .entry("permissions")
            .or_insert_with(|| serde_json::json!({}));

        let allow_list = permissions
            .as_object_mut()
            .context("permissions is not an object")?
            .entry("allow")
            .or_insert_with(|| serde_json::json!([]));

        let arr = allow_list
            .as_array_mut()
            .context("allow is not an array")?;

        // Check if tool is already in the list
        let tool_pattern = format!("{}*", tool_name);
        let already_exists = arr.iter().any(|v| {
            v.as_str().map_or(false, |s| s == tool_name || s == tool_pattern)
        });

        if !already_exists {
            arr.push(serde_json::Value::String(tool_pattern.clone()));
            let json = serde_json::to_string_pretty(&settings)?;
            std::fs::write(&config, json)?;
            info!("Added '{}' to Claude Code permissions.allow", tool_pattern);
            Ok(true)
        } else {
            info!("Tool '{}' already in Claude Code permissions.allow", tool_name);
            Ok(false)
        }
    }

    // === Codex ===

    pub fn codex_config_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".codex").join("config.toml")
    }

    pub fn codex_hooks_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".codex").join("hooks.json")
    }

    pub fn is_codex_installed(&self) -> bool {
        let hooks = Self::codex_hooks_path();
        if !hooks.exists() {
            return false;
        }
        let content = match std::fs::read_to_string(&hooks) {
            Ok(c) => c,
            Err(_) => return false,
        };
        content.contains("open-island-hooks")
    }

    pub fn install_codex(&self) -> Result<()> {
        let hooks_path = Self::codex_hooks_path();
        let hooks_cmd = format!("\"{}\" --source codex", self.hooks_binary_path.display());

        if let Some(parent) = hooks_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let hook_entry = serde_json::json!({
            "matcher": "",
            "command": hooks_cmd
        });

        let mut hooks: serde_json::Value = if hooks_path.exists() {
            let content = std::fs::read_to_string(&hooks_path)?;
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let events = ["SessionStart", "PreToolUse", "PostToolUse", "Stop"];
        for event in &events {
            hooks.as_object_mut().unwrap().insert(
                event.to_string(),
                serde_json::json!([hook_entry]),
            );
        }

        let json = serde_json::to_string_pretty(&hooks)?;
        std::fs::write(&hooks_path, json)?;

        self.save_manifest("codex", &hooks_path)?;
        info!("Codex hooks installed");
        Ok(())
    }

    pub fn uninstall_codex(&self) -> Result<()> {
        let hooks_path = Self::codex_hooks_path();
        if hooks_path.exists() {
            std::fs::remove_file(&hooks_path)?;
        }
        self.remove_manifest("codex")?;
        info!("Codex hooks uninstalled");
        Ok(())
    }

    // === Gemini ===

    pub fn gemini_config_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gemini").join("settings.json")
    }

    pub fn is_gemini_installed(&self) -> bool {
        let config = Self::gemini_config_path();
        if !config.exists() {
            return false;
        }
        let content = match std::fs::read_to_string(&config) {
            Ok(c) => c,
            Err(_) => return false,
        };
        content.contains("open-island-hooks")
    }

    pub fn install_gemini(&self) -> Result<()> {
        let config = Self::gemini_config_path();
        let hooks_cmd = format!("\"{}\" --source gemini", self.hooks_binary_path.display());

        if let Some(parent) = config.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut settings: serde_json::Value = if config.exists() {
            let content = std::fs::read_to_string(&config)?;
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let hook_entry = serde_json::json!({
            "matcher": "",
            "command": hooks_cmd
        });

        let hooks = settings
            .as_object_mut()
            .context("settings.json is not an object")?;

        let events = ["SessionStart", "SessionEnd", "BeforeAgent", "AfterAgent"];
        for event in &events {
            hooks.insert(
                event.to_string(),
                serde_json::json!([hook_entry]),
            );
        }

        if config.exists() {
            let backup = config.with_extension("json.bak");
            let _ = std::fs::copy(&config, &backup);
        }

        let json = serde_json::to_string_pretty(&settings)?;
        std::fs::write(&config, json)?;

        self.save_manifest("gemini", &config)?;
        info!("Gemini hooks installed");
        Ok(())
    }

    pub fn uninstall_gemini(&self) -> Result<()> {
        let config = Self::gemini_config_path();
        if config.exists() {
            let content = std::fs::read_to_string(&config)?;
            let mut settings: serde_json::Value = serde_json::from_str(&content)?;
            if let Some(obj) = settings.as_object_mut() {
                let events = ["SessionStart", "SessionEnd", "BeforeAgent", "AfterAgent"];
                for event in &events {
                    obj.remove(*event);
                }
            }
            let json = serde_json::to_string_pretty(&settings)?;
            std::fs::write(&config, json)?;
        }
        self.remove_manifest("gemini")?;
        info!("Gemini hooks uninstalled");
        Ok(())
    }

    // === Cursor Agent ===
    // Format from VibeAgentIsland: ~/.cursor-agent/hooks.json

    pub fn cursor_config_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".cursor-agent").join("hooks.json")
    }

    pub fn is_cursor_installed(&self) -> bool {
        let config = Self::cursor_config_path();
        if !config.exists() {
            return false;
        }
        let content = match std::fs::read_to_string(&config) {
            Ok(c) => c,
            Err(_) => return false,
        };
        content.contains("open-island-hooks")
    }

    pub fn install_cursor(&self) -> Result<()> {
        let config = Self::cursor_config_path();
        let hooks_cmd = format!("\"{}\" --source cursor", self.hooks_binary_path.display());

        if let Some(parent) = config.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Cursor Agent uses a nested format: each event has [{hooks: [{command, timeout}]}]
        // PreToolUse/PostToolUse additionally have a "matcher" field
        let make_entry = |with_matcher: bool| -> serde_json::Value {
            let hook_obj = serde_json::json!({
                "command": hooks_cmd,
                "timeout": 120
            });
            if with_matcher {
                serde_json::json!({ "matcher": "*", "hooks": [hook_obj] })
            } else {
                serde_json::json!({ "hooks": [hook_obj] })
            }
        };

        let hooks = serde_json::json!({
            "version": 1,
            "hooks": {
                "SessionStart": [make_entry(false)],
                "SessionEnd": [make_entry(false)],
                "UserPromptSubmit": [make_entry(false)],
                "PreToolUse": [make_entry(true)],
                "PostToolUse": [make_entry(true)],
                "Stop": [make_entry(false)],
                "SubagentStop": [make_entry(false)]
            }
        });

        if config.exists() {
            let backup = config.with_extension("json.bak");
            let _ = std::fs::copy(&config, &backup);
        }

        let json = serde_json::to_string_pretty(&hooks)?;
        std::fs::write(&config, json)?;

        self.save_manifest("cursor", &config)?;
        info!("Cursor Agent hooks installed at {}", config.display());
        Ok(())
    }

    pub fn uninstall_cursor(&self) -> Result<()> {
        let config = Self::cursor_config_path();
        if config.exists() {
            let _ = std::fs::remove_file(&config);
        }
        self.remove_manifest("cursor")?;
        info!("Cursor Agent hooks uninstalled");
        Ok(())
    }

    // === Manifest helpers ===

    fn manifest_dir() -> PathBuf {
        let app_data = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        app_data.join("OpenIsland")
    }

    fn save_manifest(&self, agent: &str, config_path: &PathBuf) -> Result<()> {
        let dir = Self::manifest_dir();
        std::fs::create_dir_all(&dir)?;

        let manifest = HookManifest {
            agent: agent.to_string(),
            installed_at: chrono::Utc::now().to_rfc3339(),
            binary_path: self.hooks_binary_path.to_string_lossy().to_string(),
            config_path: config_path.to_string_lossy().to_string(),
        };

        let path = dir.join(format!("open-island-{}-hook-install.json", agent));
        let json = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn remove_manifest(&self, agent: &str) -> Result<()> {
        let path = Self::manifest_dir().join(format!("open-island-{}-hook-install.json", agent));
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
