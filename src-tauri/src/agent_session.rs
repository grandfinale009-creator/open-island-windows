use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent_event::{
    AgentEvent, AgentTool, JumpTarget, PermissionRequest, QuestionPrompt, SessionPhase,
};

/// Session attachment state — tracks whether the backing process is alive.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionAttachmentState {
    #[default]
    Attached,
    Stale,
    Detached,
}

/// A single tool execution history entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolHistoryEntry {
    pub tool: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub success: bool,
}

/// Core session model. Ported from `Sources/OpenIslandCore/AgentSession.swift`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSession {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub tool: AgentTool,
    /// Current tool being executed (e.g., "Bash", "Write", "Edit").
    /// Cleared when the tool completes.
    #[serde(default)]
    pub current_tool: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub attachment_state: SessionAttachmentState,
    pub phase: SessionPhase,
    #[serde(default)]
    pub summary: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub first_seen_at: DateTime<Utc>,
    #[serde(default)]
    pub permission_request: Option<PermissionRequest>,
    #[serde(default)]
    pub question_prompt: Option<QuestionPrompt>,
    /// Source of the current question: "cli" or "desktop".
    #[serde(default)]
    pub question_source: Option<String>,
    #[serde(default)]
    pub jump_target: Option<JumpTarget>,
    #[serde(default)]
    pub is_remote: bool,
    #[serde(default)]
    pub is_hook_managed: bool,
    #[serde(default)]
    pub is_session_ended: bool,
    #[serde(default)]
    pub is_process_alive: bool,
    #[serde(default)]
    pub process_not_seen_count: u32,
    #[serde(default)]
    pub terminal_app: Option<String>,
    /// History of tool executions in this session (most recent last).
    #[serde(default)]
    pub tool_history: Vec<ToolHistoryEntry>,
    /// Last user prompt text.
    #[serde(default)]
    pub last_user_prompt: Option<String>,
    /// Last assistant response text.
    #[serde(default)]
    pub last_assistant_message: Option<String>,
}

/// Maximum number of tool history entries to keep per session.
const MAX_TOOL_HISTORY: usize = 50;

impl AgentSession {
    pub fn new(
        id: String,
        title: String,
        tool: AgentTool,
        origin: Option<String>,
        phase: SessionPhase,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            title,
            tool,
            current_tool: None,
            origin,
            attachment_state: SessionAttachmentState::Attached,
            phase,
            summary: None,
            updated_at: now,
            first_seen_at: now,
            permission_request: None,
            question_prompt: None,
            question_source: None,
            jump_target: None,
            is_remote: false,
            is_hook_managed: true,
            is_session_ended: false,
            is_process_alive: true,
            process_not_seen_count: 0,
            terminal_app: None,
            tool_history: Vec::new(),
            last_user_prompt: None,
            last_assistant_message: None,
        }
    }

    /// Push a tool execution to history, closing the previous open entry if any.
    fn record_tool_start(&mut self, tool_name: &str, description: Option<String>) {
        // Close any currently open tool entry
        if let Some(last) = self.tool_history.last_mut() {
            if last.finished_at.is_none() {
                last.finished_at = Some(Utc::now());
            }
        }
        self.tool_history.push(ToolHistoryEntry {
            tool: tool_name.to_string(),
            description,
            started_at: Utc::now(),
            finished_at: None,
            success: true,
        });
        // Trim history if too long
        if self.tool_history.len() > MAX_TOOL_HISTORY {
            let drain_count = self.tool_history.len() - MAX_TOOL_HISTORY;
            self.tool_history.drain(0..drain_count);
        }
    }

    /// Close the current tool entry (mark as finished).
    fn record_tool_end(&mut self, success: bool) {
        if let Some(last) = self.tool_history.last_mut() {
            if last.finished_at.is_none() {
                last.finished_at = Some(Utc::now());
                last.success = success;
            }
        }
    }

    /// Whether this session should be visible in the island UI.
    pub fn is_visible_in_island(&self) -> bool {
        // Sessions needing attention are always visible
        if self.phase.requires_attention() {
            return true;
        }

        let elapsed = Utc::now().signed_duration_since(self.updated_at);
        let minutes_since_update = elapsed.num_minutes();

        // Active sessions with recent activity
        if self.phase == SessionPhase::Running {
            // Hook-managed: visible for 3 minutes after last activity
            if self.is_hook_managed {
                return minutes_since_update < 3;
            }
            // Process-managed: visible while process is alive or 5 minutes after last activity
            return self.is_process_alive || minutes_since_update < 5;
        }

        // Completed sessions stay visible for 2 minutes
        if self.phase == SessionPhase::Completed {
            return minutes_since_update < 2;
        }

        false
    }

    /// Whether this session should be auto-completed (stale session).
    pub fn should_auto_complete(&self) -> bool {
        if self.phase != SessionPhase::Running {
            return false;
        }
        let elapsed = Utc::now().signed_duration_since(self.updated_at);
        // Hook-managed: 3 minutes, Process-managed: 5 minutes
        let threshold = if self.is_hook_managed { 3 } else { 5 };
        elapsed.num_minutes() >= threshold
    }

    /// Whether this session should be removed (very stale).
    pub fn should_remove(&self) -> bool {
        let elapsed = Utc::now().signed_duration_since(self.updated_at);
        // All sessions: remove after 10 minutes of no activity
        elapsed.num_minutes() >= 10
    }
}

/// Pure reducer for session state. Ported from `Sources/OpenIslandCore/SessionState.swift`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub sessions_by_id: std::collections::HashMap<String, AgentSession>,
}

impl SessionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The sole mutation path — apply an event to produce new state.
    pub fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::SessionStarted(e) => {
                let phase = e
                    .initial_phase
                    .clone()
                    .unwrap_or(SessionPhase::Running);
                let mut session = AgentSession::new(
                    e.session_id.clone(),
                    e.title.clone(),
                    e.tool.clone(),
                    e.origin.clone(),
                    phase,
                );
                session.summary = e.summary.clone();
                session.jump_target = e.jump_target.clone();
                session.is_remote = e.is_remote.unwrap_or(false);
                if let Some(ts) = e.timestamp {
                    session.updated_at = DateTime::from_timestamp_millis(ts).unwrap_or(Utc::now());
                }
                self.sessions_by_id.insert(e.session_id.clone(), session);
            }
            AgentEvent::ActivityUpdated(e) => {
                if !self.sessions_by_id.contains_key(&e.session_id) {
                    // Auto-create session for unknown session_id
                    let phase = e.phase.clone().unwrap_or(SessionPhase::Running);
                    let mut session = AgentSession::new(
                        e.session_id.clone(),
                        format!("Claude Code ({})", &e.session_id[..8.min(e.session_id.len())]),
                        AgentTool::ClaudeCode,
                        None,
                        phase,
                    );
                    session.summary = e.summary.clone();
                    self.sessions_by_id.insert(e.session_id.clone(), session);
                }
                if let Some(session) = self.sessions_by_id.get_mut(&e.session_id) {
                    // Auto-resolve WaitingForAnswer when activity resumes —
                    // the user has answered the question (either in CLI or Claude Desktop).
                    if session.phase == SessionPhase::WaitingForAnswer {
                        session.question_prompt = None;
                        session.phase = SessionPhase::Running;
                    }
                    if let Some(ref phase) = e.phase {
                        session.phase = phase.clone();
                    }
                    if let Some(ref summary) = e.summary {
                        session.summary = Some(summary.clone());
                    }
                    // Update current tool and record history
                    if let Some(ref tool) = e.tool {
                        if tool.is_empty() {
                            // Tool finished
                            session.record_tool_end(true);
                            session.current_tool = None;
                        } else {
                            // Tool started — record to history
                            let desc = e.summary.clone();
                            session.record_tool_start(tool, desc);
                            session.current_tool = Some(tool.clone());
                        }
                    }
                    // Update user prompt if present
                    if let Some(ref summary) = e.summary {
                        if e.tool.is_none() {
                            // This is a user prompt submit (no tool)
                            session.last_user_prompt = Some(summary.clone());
                        }
                    }
                    session.updated_at = Utc::now();
                }
            }
            AgentEvent::PermissionRequested(e) => {
                if !self.sessions_by_id.contains_key(&e.session_id) {
                    let mut session = AgentSession::new(
                        e.session_id.clone(),
                        format!("Claude Code ({})", &e.session_id[..8.min(e.session_id.len())]),
                        AgentTool::ClaudeCode,
                        None,
                        SessionPhase::WaitingForApproval,
                    );
                    self.sessions_by_id.insert(e.session_id.clone(), session);
                }
                if let Some(session) = self.sessions_by_id.get_mut(&e.session_id) {
                    session.phase = SessionPhase::WaitingForApproval;
                    session.permission_request = Some(e.permission.clone());
                    session.updated_at = Utc::now();
                }
            }
            AgentEvent::QuestionAsked(e) => {
                if !self.sessions_by_id.contains_key(&e.session_id) {
                    let mut session = AgentSession::new(
                        e.session_id.clone(),
                        format!("Claude Code ({})", &e.session_id[..8.min(e.session_id.len())]),
                        AgentTool::ClaudeCode,
                        None,
                        SessionPhase::WaitingForAnswer,
                    );
                    self.sessions_by_id.insert(e.session_id.clone(), session);
                }
                if let Some(session) = self.sessions_by_id.get_mut(&e.session_id) {
                    session.phase = SessionPhase::WaitingForAnswer;
                    session.question_prompt = Some(e.question.clone());
                    session.question_source = e.source.clone();
                    session.updated_at = Utc::now();
                }
            }
            AgentEvent::SessionCompleted(e) => {
                if let Some(session) = self.sessions_by_id.get_mut(&e.session_id) {
                    session.phase = SessionPhase::Completed;
                    session.is_session_ended = true;
                    // Record last assistant message
                    if let Some(ref summary) = e.summary {
                        session.last_assistant_message = Some(summary.clone());
                        session.summary = Some(summary.clone());
                    }
                    // Close any open tool entry
                    session.record_tool_end(true);
                    session.current_tool = None;
                    session.updated_at = Utc::now();
                }
            }
            AgentEvent::JumpTargetUpdated(e) => {
                if let Some(session) = self.sessions_by_id.get_mut(&e.session_id) {
                    session.jump_target = Some(e.jump_target.clone());
                }
            }
            AgentEvent::SessionMetadataUpdated(e) => {
                if let Some(session) = self.sessions_by_id.get_mut(&e.session_id) {
                    if let Some(ref title) = e.title {
                        session.title = title.clone();
                    }
                    if let Some(ref origin) = e.origin {
                        session.origin = Some(origin.clone());
                    }
                }
            }
            AgentEvent::ActionableStateResolved(e) => {
                if let Some(session) = self.sessions_by_id.get_mut(&e.session_id) {
                    session.permission_request = None;
                    session.question_prompt = None;
                    session.question_source = None;
                    if session.phase.requires_attention() {
                        session.phase = SessionPhase::Running;
                    }
                }
            }
        }
    }

    /// Resolve a permission request for a session.
    pub fn resolve_permission(&mut self, session_id: &str, allowed: bool) {
        if let Some(session) = self.sessions_by_id.get_mut(session_id) {
            session.permission_request = None;
            if session.phase == SessionPhase::WaitingForApproval {
                session.phase = if allowed { SessionPhase::Running } else { SessionPhase::Completed };
            }
            session.updated_at = Utc::now();
        }
    }

    /// Answer a question for a session.
    pub fn answer_question(&mut self, session_id: &str) {
        if let Some(session) = self.sessions_by_id.get_mut(session_id) {
            session.question_prompt = None;
            session.question_source = None;
            if session.phase == SessionPhase::WaitingForAnswer {
                session.phase = SessionPhase::Running;
            }
            session.updated_at = Utc::now();
        }
    }

    /// Update process liveness — 2-poll threshold before marking detached.
    pub fn mark_process_liveness(&mut self, alive_ids: &[String], dead_ids: &[String]) {
        for id in alive_ids {
            if let Some(session) = self.sessions_by_id.get_mut(id) {
                session.is_process_alive = true;
                session.process_not_seen_count = 0;
                if session.attachment_state == SessionAttachmentState::Detached {
                    session.attachment_state = SessionAttachmentState::Attached;
                }
            }
        }
        for id in dead_ids {
            if let Some(session) = self.sessions_by_id.get_mut(id) {
                session.process_not_seen_count += 1;
                if session.process_not_seen_count >= 2 {
                    session.is_process_alive = false;
                    session.attachment_state = SessionAttachmentState::Detached;
                } else {
                    session.attachment_state = SessionAttachmentState::Stale;
                }
            }
        }
    }

    // === Computed properties ===

    pub fn visible_sessions(&self) -> Vec<&AgentSession> {
        let mut sessions: Vec<&AgentSession> = self
            .sessions_by_id
            .values()
            .filter(|s| s.is_visible_in_island())
            .collect();
        // Sort: attention first, then by updated_at descending
        sessions.sort_by(|a, b| {
            let a_attn = a.phase.requires_attention() as u8;
            let b_attn = b.phase.requires_attention() as u8;
            b_attn
                .cmp(&a_attn)
                .then(b.updated_at.cmp(&a.updated_at))
        });
        sessions
    }

    pub fn running_count(&self) -> usize {
        self.sessions_by_id
            .values()
            .filter(|s| s.phase == SessionPhase::Running && s.current_tool.is_some() && s.is_visible_in_island())
            .count()
    }

    pub fn waiting_count(&self) -> usize {
        self.sessions_by_id
            .values()
            .filter(|s| s.phase.requires_attention() && s.is_visible_in_island())
            .count()
    }

    pub fn done_count(&self) -> usize {
        self.sessions_by_id
            .values()
            .filter(|s| s.phase == SessionPhase::Completed && s.is_visible_in_island())
            .count()
    }

    /// Remove sessions that are no longer visible (ended + stale).
    pub fn remove_invisible_sessions(&mut self) {
        self.sessions_by_id.retain(|_, s| !s.should_remove());
    }

    /// Auto-complete stale hook-managed sessions.
    /// Returns the IDs of sessions that were auto-completed.
    pub fn auto_complete_stale_sessions(&mut self) -> Vec<String> {
        let mut completed_ids = Vec::new();
        for (id, session) in &mut self.sessions_by_id {
            if session.should_auto_complete() {
                session.phase = SessionPhase::Completed;
                session.is_session_ended = true;
                session.current_tool = None;
                session.updated_at = Utc::now();
                completed_ids.push(id.clone());
            }
        }
        completed_ids
    }
}
