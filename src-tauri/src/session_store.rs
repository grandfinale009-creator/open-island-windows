use crate::agent_session::SessionState;

/// Persistence file path.
fn persistence_path() -> std::path::PathBuf {
    let dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = dir.join("OpenIsland");
    std::fs::create_dir_all(&dir).ok();
    dir.join("sessions.json")
}

/// Thread-safe session store wrapper. Owns the `SessionState` and provides
/// serialization for Tauri frontend communication.
pub struct SessionStore {
    pub state: SessionState,
}

impl SessionStore {
    pub fn new() -> Self {
        let mut state = Self::load_from_disk().unwrap_or_default();
        // Remove stale process sessions from previous runs.
        // Process monitor will re-discover running processes on startup.
        let stale_keys: Vec<String> = state
            .sessions_by_id
            .iter()
            .filter(|(_, s)| !s.is_hook_managed)
            .map(|(k, _)| k.clone())
            .collect();
        for key in stale_keys {
            state.sessions_by_id.remove(&key);
        }
        Self { state }
    }

    /// Load sessions from disk.
    fn load_from_disk() -> Option<SessionState> {
        let path = persistence_path();
        if !path.exists() {
            return None;
        }
        let data = std::fs::read_to_string(&path).ok()?;
        let state: SessionState = serde_json::from_str(&data).ok()?;
        tracing::info!("Restored {} sessions from disk", state.sessions_by_id.len());
        Some(state)
    }

    /// Save sessions to disk.
    pub fn save_to_disk(&self) {
        let path = persistence_path();
        if let Ok(json) = serde_json::to_string_pretty(&self.state) {
            std::fs::write(&path, json).ok();
        }
    }

    /// Snapshot the current state for the frontend.
    pub fn snapshot(&self) -> SessionStateSnapshot {
        let sessions: Vec<SessionSnapshotEntry> = self
            .state
            .visible_sessions()
            .into_iter()
            .map(|s| SessionSnapshotEntry {
                id: s.id.clone(),
                title: s.title.clone(),
                agent: s.tool.short_name().to_string(),
                phase: format!("{:?}", s.phase),
                tool: s.tool.short_name().to_string(),
                current_tool: s.current_tool.clone(),
                summary: s.summary.clone(),
                permission_request: s.permission_request.clone(),
                question_prompt: s.question_prompt.as_ref().and_then(|qp| {
                    let first_q = qp.questions.first()?;
                    Some(QuestionPromptSnapshot {
                        title: qp.title.clone(),
                        question: first_q.question.clone(),
                        options: first_q.options.iter().map(|o| QuestionOptionSnapshot {
                            label: o.label.clone(),
                            description: o.description.clone(),
                        }).collect(),
                        source: s.question_source.clone().unwrap_or_else(|| "cli".to_string()),
                    })
                }),
                origin: s.origin.clone(),
                updated_at: s.updated_at.timestamp_millis(),
                first_seen_at: s.first_seen_at.timestamp_millis(),
                terminal_app: s.terminal_app.clone(),
                tool_history: if s.tool_history.is_empty() {
                    None
                } else {
                    Some(s.tool_history.iter().map(|h| ToolHistorySnapshot {
                        tool: h.tool.clone(),
                        description: h.description.clone(),
                        started_at: h.started_at.timestamp_millis(),
                        finished_at: h.finished_at.map(|t| t.timestamp_millis()),
                        success: h.success,
                    }).collect())
                },
                last_user_prompt: s.last_user_prompt.clone(),
                last_assistant_message: s.last_assistant_message.clone(),
            })
            .collect();

        SessionStateSnapshot {
            running_count: self.state.running_count(),
            waiting_count: self.state.waiting_count(),
            done_count: self.state.done_count(),
            sessions,
        }
    }
}

use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct SessionStateSnapshot {
    pub running_count: usize,
    pub waiting_count: usize,
    pub done_count: usize,
    pub sessions: Vec<SessionSnapshotEntry>,
}

#[derive(Serialize, Clone)]
pub struct SessionSnapshotEntry {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub phase: String,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_request: Option<crate::agent_event::PermissionRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_prompt: Option<QuestionPromptSnapshot>,
    pub origin: Option<String>,
    pub updated_at: i64,
    pub first_seen_at: i64,
    pub terminal_app: Option<String>,
    /// Tool execution history (most recent last).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_history: Option<Vec<ToolHistorySnapshot>>,
    /// Last user prompt text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user_prompt: Option<String>,
    /// Last assistant message text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_assistant_message: Option<String>,
}

/// Simplified question prompt snapshot for frontend display.
#[derive(Serialize, Clone)]
pub struct QuestionPromptSnapshot {
    pub title: Option<String>,
    pub question: String,
    pub options: Vec<QuestionOptionSnapshot>,
    /// "cli" = Claude Code AskUserQuestion tool, "desktop" = Claude Desktop native dialog.
    pub source: String,
}

#[derive(Serialize, Clone)]
pub struct QuestionOptionSnapshot {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct ToolHistorySnapshot {
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub success: bool,
}
