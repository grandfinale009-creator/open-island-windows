use serde::{Deserialize, Serialize};

/// Agent event enum — the single source of truth for all state transitions.
/// Ported from `Sources/OpenIslandCore/AgentEvent.swift`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum AgentEvent {
    #[serde(rename = "sessionStarted")]
    SessionStarted(SessionStarted),
    #[serde(rename = "activityUpdated")]
    ActivityUpdated(SessionActivityUpdated),
    #[serde(rename = "permissionRequested")]
    PermissionRequested(PermissionRequested),
    #[serde(rename = "questionAsked")]
    QuestionAsked(QuestionAsked),
    #[serde(rename = "sessionCompleted")]
    SessionCompleted(SessionCompleted),
    #[serde(rename = "jumpTargetUpdated")]
    JumpTargetUpdated(JumpTargetUpdated),
    #[serde(rename = "sessionMetadataUpdated")]
    SessionMetadataUpdated(SessionMetadataUpdated),
    #[serde(rename = "actionableStateResolved")]
    ActionableStateResolved(ActionableStateResolved),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionStarted {
    pub session_id: String,
    #[serde(default)]
    pub title: String,
    pub tool: AgentTool,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub initial_phase: Option<SessionPhase>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub jump_target: Option<JumpTarget>,
    #[serde(default)]
    pub is_remote: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionActivityUpdated {
    pub session_id: String,
    #[serde(default)]
    pub phase: Option<SessionPhase>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionRequested {
    pub session_id: String,
    pub permission: PermissionRequest,
    #[serde(default)]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionAsked {
    pub session_id: String,
    pub question: QuestionPrompt,
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Where the question originated: "cli" = Claude Code AskUserQuestion tool,
    /// "desktop" = Claude Desktop native question dialog.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionCompleted {
    pub session_id: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JumpTargetUpdated {
    pub session_id: String,
    pub jump_target: JumpTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMetadataUpdated {
    pub session_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionableStateResolved {
    pub session_id: String,
}

// === Enums ===

#[derive(Debug, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentTool {
    ClaudeCode,
    Codex,
    GeminiCLI,
    OpenCode,
    Qoder,
    QwenCode,
    Factory,
    Codebuddy,
    Cursor,
    KimiCLI,
}

impl AgentTool {
    pub fn display_name(&self) -> &str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::GeminiCLI => "Gemini CLI",
            Self::OpenCode => "OpenCode",
            Self::Qoder => "Qoder",
            Self::QwenCode => "Qwen Code",
            Self::Factory => "Factory",
            Self::Codebuddy => "CodeBuddy",
            Self::Cursor => "Cursor",
            Self::KimiCLI => "Kimi CLI",
        }
    }

    pub fn short_name(&self) -> &str {
        match self {
            Self::ClaudeCode => "Claude",
            Self::Codex => "Codex",
            Self::GeminiCLI => "Gemini",
            Self::OpenCode => "OpenCode",
            Self::Qoder => "Qoder",
            Self::QwenCode => "Qwen",
            Self::Factory => "Factory",
            Self::Codebuddy => "CodeBuddy",
            Self::Cursor => "Cursor",
            Self::KimiCLI => "Kimi",
        }
    }

    pub fn brand_color_hex(&self) -> &str {
        match self {
            Self::ClaudeCode => "#d97742",
            Self::Codex => "#4aa3df",
            Self::GeminiCLI => "#42e86b",
            Self::OpenCode => "#ffb547",
            Self::Qoder => "#ff6b9f",
            Self::QwenCode => "#c084fc",
            Self::Factory => "#6e9fff",
            Self::Codebuddy => "#fca5a5",
            Self::Cursor => "#7a5cff",
            Self::KimiCLI => "#fde047",
        }
    }

    pub fn is_claude_code_fork(&self) -> bool {
        matches!(
            self,
            Self::Qoder | Self::QwenCode | Self::Factory | Self::Codebuddy
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    Running,
    WaitingForApproval,
    WaitingForAnswer,
    Completed,
}

impl SessionPhase {
    pub fn requires_attention(&self) -> bool {
        matches!(self, Self::WaitingForApproval | Self::WaitingForAnswer)
    }
}

// === Structs ===

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JumpTarget {
    #[serde(default)]
    pub terminal_app: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub pane_title: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub terminal_session_id: Option<String>,
    #[serde(default)]
    pub terminal_tty: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionRequest {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub affected_path: Option<String>,
    #[serde(default)]
    pub primary_action_title: Option<String>,
    #[serde(default)]
    pub secondary_action_title: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    /// Permission suggestions from Claude Code (for "always allow" support).
    #[serde(default)]
    pub permission_suggestions: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionPrompt {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub questions: Vec<QuestionPromptItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionPromptItem {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub allows_freeform: bool,
}
