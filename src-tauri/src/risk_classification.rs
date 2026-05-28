use serde::{Deserialize, Serialize};

/// Risk level for tool/command approval — 4-level system from Claude Dynamic Island.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskLevel {
    /// Read-only operations (Read, Grep, Glob, LS) — safe to auto-approve.
    Safe,
    /// File writes (Write, Edit, MultiEdit) — user should review.
    Review,
    /// Shell commands — needs careful inspection.
    Shell,
    /// Destructive commands (rm -rf, git reset --hard) — always require approval.
    Danger,
}

impl RiskLevel {
    /// Whether this risk level can be auto-approved.
    pub fn is_auto_approvable(&self) -> bool {
        matches!(self, RiskLevel::Safe)
    }

    /// Human-readable label
    pub fn label(&self) -> &'static str {
        match self {
            RiskLevel::Safe => "Safe",
            RiskLevel::Review => "Review",
            RiskLevel::Shell => "Shell",
            RiskLevel::Danger => "Danger",
        }
    }

    /// Signal badge text
    pub fn signal(&self) -> &'static str {
        match self {
            RiskLevel::Safe => "READ",
            RiskLevel::Review => "WRITE",
            RiskLevel::Shell => "SHELL",
            RiskLevel::Danger => "DANGER",
        }
    }

    /// Reason description
    pub fn reason(&self) -> &'static str {
        match self {
            RiskLevel::Safe => "Read-only tool, auto-approved",
            RiskLevel::Review => "Modifies files, needs review",
            RiskLevel::Shell => "Shell command, needs confirmation",
            RiskLevel::Danger => "Destructive operation, requires approval",
        }
    }
}

/// Detailed risk assessment for a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub level: RiskLevel,
    /// Human-readable label (e.g. "Safe", "Review", "Shell", "Danger").
    pub label: String,
    /// Signal that triggered this classification (e.g. tool name, command pattern).
    pub signal: String,
    /// Human-readable reason for the classification.
    pub reason: String,
    /// Whether this tool is auto-approved (safe read-only tools).
    pub auto_approved: bool,
}

/// Read-only tools that never modify state.
const READ_ONLY_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "LS", "ListFiles", "Search", "WebSearch",
    "WebFetch", "ReadNotebook",
];

/// File-writing tools.
const WRITE_TOOLS: &[&str] = &[
    "Write", "Edit", "MultiEdit", "Create", "Delete", "Rename",
    "WriteNotebook", "NotebookEditCell",
];

/// Dangerous shell command patterns.
const DESTRUCTIVE_PATTERNS: &[&str] = &[
    "rm -rf", "rm -r", "remove-item -r", "del /f", "del /s", "del /q",
    "rmdir /s", "rd /s",
    "git reset --hard", "git clean -fd", "git push --force",
    "format ", "mkfs", "shutdown", "restart",
    "npm publish", "cargo publish", "pip install",
    "curl ", "wget ", "Invoke-WebRequest",
    "> /dev/", "chmod 777", "chown ",
    "sudo ", "su ",
    "taskkill", "Stop-Process",
];

/// Assess the risk level of a tool invocation.
pub fn assess_risk(tool_name: &str, tool_input: Option<&serde_json::Value>) -> RiskAssessment {
    let lower_tool = tool_name.to_lowercase();

    // Check read-only tools (auto-approved)
    if READ_ONLY_TOOLS.iter().any(|t| t.to_lowercase() == lower_tool) {
        return RiskAssessment {
            level: RiskLevel::Safe,
            label: RiskLevel::Safe.label().to_string(),
            signal: RiskLevel::Safe.signal().to_string(),
            reason: format!("{} is a read-only operation", tool_name),
            auto_approved: true,
        };
    }

    // Check write tools
    if WRITE_TOOLS.iter().any(|t| t.to_lowercase() == lower_tool) {
        let path = extract_path(tool_input);
        return RiskAssessment {
            level: RiskLevel::Review,
            label: RiskLevel::Review.label().to_string(),
            signal: RiskLevel::Review.signal().to_string(),
            reason: match path {
                Some(p) => format!("{} modifies {}", tool_name, p),
                None => format!("{} modifies files", tool_name),
            },
            auto_approved: false,
        };
    }

    // Bash / shell commands — check for destructive patterns
    if lower_tool == "bash" || lower_tool == "shell" || lower_tool == "execute" {
        if let Some(input) = tool_input {
            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                let cmd_lower = cmd.to_lowercase();
                for pattern in DESTRUCTIVE_PATTERNS {
                    if cmd_lower.contains(pattern) {
                        return RiskAssessment {
                            level: RiskLevel::Danger,
                            label: RiskLevel::Danger.label().to_string(),
                            signal: pattern.to_string(),
                            reason: format!("Command contains '{}'", pattern),
                            auto_approved: false,
                        };
                    }
                }
                // Shell commands that aren't destructive are Shell level
                return RiskAssessment {
                    level: RiskLevel::Shell,
                    label: RiskLevel::Shell.label().to_string(),
                    signal: cmd.chars().take(40).collect::<String>(),
                    reason: RiskLevel::Shell.reason().to_string(),
                    auto_approved: false,
                };
            }
        }
        return RiskAssessment {
            level: RiskLevel::Shell,
            label: RiskLevel::Shell.label().to_string(),
            signal: "Bash".to_string(),
            reason: RiskLevel::Shell.reason().to_string(),
            auto_approved: false,
        };
    }

    // Agent / subagent tools
    if lower_tool.contains("agent") || lower_tool == "task" || lower_tool == "dispatch_agent" {
        return RiskAssessment {
            level: RiskLevel::Review,
            label: "Sub-agent".to_string(),
            signal: tool_name.to_string(),
            reason: format!("{} spawns a sub-agent", tool_name),
            auto_approved: false,
        };
    }

    // Default: shell level for unknown tools
    RiskAssessment {
        level: RiskLevel::Shell,
        label: RiskLevel::Shell.label().to_string(),
        signal: tool_name.to_string(),
        reason: format!("{} is not classified", tool_name),
        auto_approved: false,
    }
}

/// Truncate a string safely at a character boundary.
fn truncate_str(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Generate human-readable description for tool input.
/// Inspired by CCIsland's describe_tool_input.
pub fn describe_tool_input(tool_name: &str, tool_input: Option<&serde_json::Value>) -> String {
    let input = match tool_input {
        Some(v) => v,
        None => return tool_name.to_string(),
    };

    match tool_name {
        "Bash" => {
            let cmd = input.get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("...");
            if cmd.chars().count() > 50 {
                format!("$ {}...", truncate_str(cmd, 47))
            } else {
                format!("$ {}", cmd)
            }
        }
        "Read" => {
            let path = extract_path(Some(input)).unwrap_or_else(|| "unknown".to_string());
            shorten_path(&path)
        }
        "Write" | "Edit" | "MultiEdit" => {
            let path = extract_path(Some(input)).unwrap_or_else(|| "unknown".to_string());
            format!("{} {}", if tool_name == "Write" { "write" } else { "edit" }, shorten_path(&path))
        }
        "Glob" => {
            let pattern = input.get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            format!("glob {}", pattern)
        }
        "Grep" => {
            let pattern = input.get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let path = input.get("path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            if pattern.chars().count() > 25 {
                format!("\"{}...\" in {}", truncate_str(pattern, 22), shorten_path(path))
            } else {
                format!("\"{}\" in {}", pattern, shorten_path(path))
            }
        }
        "WebFetch" => {
            let url = input.get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if url.chars().count() > 35 {
                format!("fetch {}...", truncate_str(url, 32))
            } else {
                format!("fetch {}", url)
            }
        }
        "WebSearch" => {
            let query = input.get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("search \"{}\"", query)
        }
        _ => tool_name.to_string(),
    }
}

/// Shorten a file path for display
fn shorten_path(path: &str) -> String {
    if path.len() <= 35 {
        return path.to_string();
    }
    if let Some(filename) = path.split('/').last().or_else(|| path.split('\\').last()) {
        if filename.len() < 25 {
            return format!(".../{}", filename);
        }
    }
    format!("{}...", &path[..32])
}

/// Extract a file path from tool input, checking common field names.
/// Extract a file path from tool input (public version for use by bridge server).
pub fn extract_path_from_input(tool_input: Option<&serde_json::Value>) -> Option<String> {
    extract_path(tool_input)
}

/// Extract a file path from tool input.
fn extract_path(tool_input: Option<&serde_json::Value>) -> Option<String> {
    let input = tool_input?;
    for key in &["file_path", "path", "notebook_path", "filename"] {
        if let Some(val) = input.get(*key).and_then(|v| v.as_str()) {
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_only_is_safe() {
        let risk = assess_risk("Read", None);
        assert_eq!(risk.level, RiskLevel::Safe);
    }

    #[test]
    fn test_grep_is_safe() {
        let risk = assess_risk("Grep", None);
        assert_eq!(risk.level, RiskLevel::Safe);
    }

    #[test]
    fn test_write_is_review() {
        let risk = assess_risk("Write", None);
        assert_eq!(risk.level, RiskLevel::Review);
    }

    #[test]
    fn test_bash_rm_rf_is_danger() {
        let input = serde_json::json!({"command": "rm -rf /tmp/test"});
        let risk = assess_risk("Bash", Some(&input));
        assert_eq!(risk.level, RiskLevel::Danger);
    }

    #[test]
    fn test_bash_safe_command_is_review() {
        let input = serde_json::json!({"command": "ls -la"});
        let risk = assess_risk("Bash", Some(&input));
        assert_eq!(risk.level, RiskLevel::Review);
    }

    #[test]
    fn test_git_push_force_is_danger() {
        let input = serde_json::json!({"command": "git push --force origin main"});
        let risk = assess_risk("Bash", Some(&input));
        assert_eq!(risk.level, RiskLevel::Danger);
    }

    #[test]
    fn test_unknown_tool_is_review() {
        let risk = assess_risk("SomeNewTool", None);
        assert_eq!(risk.level, RiskLevel::Review);
    }
}

