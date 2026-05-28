use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn};

use crate::agent_event::{AgentEvent, AgentTool, SessionPhase};
use crate::session_store::SessionStore;

/// Discovered agent process info.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredProcess {
    pub pid: u32,
    pub name: String,
    pub agent: AgentTool,
}

/// Process monitor — polls system processes every 2 seconds to discover running agents.
/// Only monitors non-Claude agents (Codex, Cursor, Gemini, etc.).
/// Claude sessions are managed entirely by the hook system.
pub struct ProcessMonitor {
    store: Arc<Mutex<SessionStore>>,
    event_tx: broadcast::Sender<AgentEvent>,
}

impl ProcessMonitor {
    pub fn new(store: Arc<Mutex<SessionStore>>, event_tx: broadcast::Sender<AgentEvent>) -> Self {
        Self { store, event_tx }
    }

    /// Start the process monitor polling loop.
    pub async fn start(&self) {
        info!("Process monitor starting (2s interval)");
        let mut interval = tokio::time::interval(Duration::from_secs(2));

        loop {
            interval.tick().await;
            if let Err(e) = self.poll_once().await {
                warn!("Process monitor poll error: {}", e);
            }
        }
    }

    async fn poll_once(&self) -> anyhow::Result<()> {
        let discovered = Self::discover_processes().await?;

        let mut store = self.store.lock().await;
        let mut new_events: Vec<AgentEvent> = Vec::new();

        // Deduplicate: only keep one process per agent type (the main/first one)
        let mut seen_agents: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut deduplicated: Vec<DiscoveredProcess> = Vec::new();
        for proc in discovered {
            let key = proc.agent.short_name().to_lowercase();
            if seen_agents.insert(key) {
                deduplicated.push(proc);
            }
        }

        // Process-created sessions (non-hook) for cleanup
        let process_session_keys: Vec<String> = store
            .state
            .sessions_by_id
            .iter()
            .filter(|(_, s)| !s.is_hook_managed)
            .map(|(k, _)| k.clone())
            .collect();

        // Track which agent types are alive
        let mut alive_agent_types: std::collections::HashSet<String> = std::collections::HashSet::new();

        for proc in &deduplicated {
            let agent_key = proc.agent.short_name().to_lowercase();
            alive_agent_types.insert(agent_key.clone());
            let session_key = format!("{}-{}", agent_key, proc.pid);

            // Check if we already have a session for this agent type
            let existing_key = process_session_keys.iter().find(|k| k.starts_with(&format!("{}-", agent_key)));

            if let Some(existing) = existing_key {
                // Update existing session with new PID if different
                if let Some(session) = store.state.sessions_by_id.get_mut(existing) {
                    session.terminal_app = Some(proc.pid.to_string());
                    session.updated_at = Utc::now();
                }
            } else {
                // New agent — create session
                let event = AgentEvent::SessionStarted(crate::agent_event::SessionStarted {
                    session_id: session_key.clone(),
                    title: proc.agent.display_name().to_string(),
                    tool: proc.agent.clone(),
                    origin: None,
                    initial_phase: Some(SessionPhase::Running),
                    summary: None,
                    timestamp: None,
                    jump_target: None,
                    is_remote: None,
                });

                store.state.apply(&event);
                if let Some(session) = store.state.sessions_by_id.get_mut(&session_key) {
                    session.is_hook_managed = false;
                    session.terminal_app = Some(proc.pid.to_string());
                }
                new_events.push(event);
            }
        }

        // Clean up dead process sessions
        let mut removed_any = false;
        for key in &process_session_keys {
            // Check if the agent type for this session is still alive
            let agent_type = key.split('-').next().unwrap_or("");
            if !alive_agent_types.contains(agent_type) {
                store.state.sessions_by_id.remove(key);
                removed_any = true;
            }
        }

        drop(store);

        // Emit events for new sessions
        let has_new = !new_events.is_empty();
        for event in new_events {
            let _ = self.event_tx.send(event);
        }

        // Trigger frontend update if sessions were removed
        if removed_any {
            let _ = self.event_tx.send(AgentEvent::ActionableStateResolved(
                crate::agent_event::ActionableStateResolved {
                    session_id: "_cleanup".to_string(),
                },
            ));
        }

        // Persist when changes occur
        if has_new || removed_any {
            let store = self.store.lock().await;
            store.save_to_disk();
        }

        Ok(())
    }

    /// Discover all running non-Claude agent processes.
    /// Claude.exe is skipped — Claude sessions are managed by the hook system.
    async fn discover_processes() -> anyhow::Result<Vec<DiscoveredProcess>> {
        let output = tokio::process::Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut processes = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(',').map(|s| s.trim_matches('"')).collect();
            if parts.len() < 2 {
                continue;
            }
            let name = parts[0];
            let pid: u32 = match parts[1].parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Skip claude.exe — Claude sessions managed by hook system
            let lower = name.to_lowercase();
            if lower == "claude.exe" || lower == "claude" {
                continue;
            }

            if let Some(agent) = Self::identify_agent(name) {
                processes.push(DiscoveredProcess {
                    pid,
                    name: name.to_string(),
                    agent,
                });
            }
        }

        Ok(processes)
    }

    /// Identify which agent a process belongs to by executable name.
    /// Uses EchoIsland's adapter pattern with exact process name matching.
    fn identify_agent(name: &str) -> Option<AgentTool> {
        let lower = name.to_lowercase();

        // Exact process name matches (more specific)
        match lower.as_str() {
            "codex.exe" | "codex" => return Some(AgentTool::Codex),
            "cursor.exe" | "cursor" => return Some(AgentTool::Cursor),
            "gemini.exe" | "gemini" => return Some(AgentTool::GeminiCLI),
            "kimi.exe" | "kimi" => return Some(AgentTool::KimiCLI),
            "opencode.exe" | "opencode" => return Some(AgentTool::OpenCode),
            "qwen-code.exe" | "qwen-code" => return Some(AgentTool::QwenCode),
            "factory.exe" | "factory" => return Some(AgentTool::Factory),
            "codebuddy.exe" | "codebuddy" => return Some(AgentTool::Codebuddy),
            _ => {}
        }

        // Substring matches for multi-process agents (IDE spawns multiple processes)
        // Only match the main process, not helper/renderer processes
        if lower.contains("cursor") && !lower.contains("helper") && !lower.contains("renderer") {
            return Some(AgentTool::Cursor);
        }
        if lower.contains("codex") && !lower.contains("helper") {
            return Some(AgentTool::Codex);
        }

        None
    }
}
