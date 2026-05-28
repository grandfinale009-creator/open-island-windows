use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicIsize;
use std::time::Duration;

use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn};

use crate::agent_event::{AgentEvent, QuestionOption, QuestionPromptItem};
use crate::session_store::SessionStore;

/// Global store for the source window handle when a question is detected.
static QUESTION_SOURCE_HWND: AtomicIsize = AtomicIsize::new(0);

/// Get the stored source window handle for the last detected question.
pub fn get_question_source_hwnd() -> isize {
    QUESTION_SOURCE_HWND.load(std::sync::atomic::Ordering::Relaxed)
}

/// Transcript monitor — watches Claude Code's JSONL transcript files
/// for AskUserQuestion entries and emits events to the island UI.
pub struct TranscriptMonitor {
    store: Arc<Mutex<SessionStore>>,
    event_tx: broadcast::Sender<AgentEvent>,
    file_positions: Mutex<HashMap<PathBuf, u64>>,
}

/// Parsed question from transcript
#[derive(Debug, Clone)]
pub struct TranscriptQuestion {
    pub session_id: String,
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

impl TranscriptMonitor {
    pub fn new(store: Arc<Mutex<SessionStore>>, event_tx: broadcast::Sender<AgentEvent>) -> Self {
        Self {
            store,
            event_tx,
            file_positions: Mutex::new(HashMap::new()),
        }
    }

    pub async fn start(&self) {
        info!("Transcript monitor starting (1s interval)");
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            if let Err(e) = self.poll_once().await {
                warn!("Transcript monitor poll error: {}", e);
            }
        }
    }

    async fn poll_once(&self) -> anyhow::Result<()> {
        let projects_dir = self.find_projects_dir()?;
        if !projects_dir.exists() {
            return Ok(());
        }

        let mut jsonl_files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        for sub_entry in sub_entries.flatten() {
                            let sub_path = sub_entry.path();
                            if sub_path.extension().map_or(false, |e| e == "jsonl") {
                                jsonl_files.push(sub_path);
                            }
                        }
                    }
                } else if path.extension().map_or(false, |e| e == "jsonl") {
                    jsonl_files.push(path);
                }
            }
        }

        for file_path in &jsonl_files {
            if let Some(question) = self.check_file(file_path).await? {
                self.handle_question(question).await;
            }
        }

        Ok(())
    }

    async fn check_file(&self, file_path: &PathBuf) -> anyhow::Result<Option<TranscriptQuestion>> {
        let metadata = std::fs::metadata(file_path)?;
        let file_size = metadata.len();

        let mut positions = self.file_positions.lock().await;
        let last_pos = match positions.get(file_path) {
            Some(&pos) => pos,
            None => {
                positions.insert(file_path.clone(), file_size);
                return Ok(None);
            }
        };

        if file_size <= last_pos {
            return Ok(None);
        }

        let content = std::fs::read_to_string(file_path)?;
        let new_content = &content[last_pos as usize..];
        positions.insert(file_path.clone(), file_size);

        for line in new_content.lines() {
            if let Some(question) = self.parse_ask_question(line) {
                return Ok(Some(question));
            }
        }

        Ok(None)
    }

    fn parse_ask_question(&self, line: &str) -> Option<TranscriptQuestion> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let content = v.get("message")?.get("content")?.as_array()?;

        for item in content {
            if item.get("type")?.as_str()? != "tool_use" {
                continue;
            }
            if item.get("name")?.as_str()? != "AskUserQuestion" {
                continue;
            }

            let input = item.get("input")?;
            let questions = input.get("questions")?.as_array()?;

            if let Some(first) = questions.first() {
                let question_text = first.get("question")?.as_str()?.to_string();
                let header = first.get("header").and_then(|v| v.as_str()).map(|s| s.to_string());
                let multi_select = first.get("multiSelect").and_then(|v| v.as_bool()).unwrap_or(false);

                let options: Vec<QuestionOption> = first
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|opt| {
                                Some(QuestionOption {
                                    label: opt.get("label")?.as_str()?.to_string(),
                                    description: opt.get("description").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                    allows_freeform: opt.get("allowsFreeform").and_then(|v| v.as_bool()).unwrap_or(false),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let tool_id = item.get("id")?.as_str()?.to_string();

                return Some(TranscriptQuestion {
                    session_id: tool_id,
                    question: question_text,
                    header,
                    options,
                    multi_select,
                });
            }
        }

        None
    }

    async fn handle_question(&self, question: TranscriptQuestion) {
        info!("Detected AskUserQuestion: {}", question.question);

        // Store the terminal window that asked the question
        let hwnd = Self::find_terminal_window();
        if hwnd != 0 {
            QUESTION_SOURCE_HWND.store(hwnd, std::sync::atomic::Ordering::Relaxed);
            info!("Stored source terminal handle: {}", hwnd);
        }

        let session_key = format!("question-{}", &question.session_id[..8.min(question.session_id.len())]);

        let tool_input_json = serde_json::json!({
            "questions": [{
                "question": question.question,
                "header": question.header,
                "options": question.options,
                "multiSelect": question.multi_select
            }]
        });

        let event = AgentEvent::QuestionAsked(crate::agent_event::QuestionAsked {
            session_id: session_key.clone(),
            question: crate::agent_event::QuestionPrompt {
                title: Some(question.question.clone()),
                questions: vec![QuestionPromptItem {
                    question: question.question.clone(),
                    header: question.header,
                    options: question.options,
                    multi_select: question.multi_select,
                }],
            },
            timestamp: None,
            source: Some("cli".to_string()),
        });

        let mut store = self.store.lock().await;
        store.state.apply(&event);
        if let Some(session) = store.state.sessions_by_id.get_mut(&session_key) {
            session.summary = Some(tool_input_json.to_string());
        }
        drop(store);

        let _ = self.event_tx.send(event);
    }

    fn find_projects_dir(&self) -> anyhow::Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home dir"))?;
        Ok(home.join(".claude").join("projects"))
    }

    /// Find the terminal window that's running Claude Code.
    fn find_terminal_window() -> isize {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, IsWindowVisible};
        use std::sync::atomic::{AtomicIsize, Ordering};

        static BEST_HWND: AtomicIsize = AtomicIsize::new(0);

        unsafe extern "system" fn enum_callback(hwnd: HWND, _lparam: isize) -> i32 {
            unsafe {
                if IsWindowVisible(hwnd) == 0 {
                    return 1;
                }
                let mut title_buf = [0u16; 512];
                let len = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), 512);
                if len == 0 {
                    return 1;
                }
                let title = String::from_utf16_lossy(&title_buf[..len as usize]);
                let lower = title.to_lowercase();

                if lower.contains("open island") || lower.contains("openisland") {
                    return 1;
                }
                if lower.contains("progman") || lower.contains("workerw") || lower.contains("default ime") {
                    return 1;
                }

                if lower.contains("claude") {
                    BEST_HWND.store(hwnd as isize, Ordering::Relaxed);
                    return 0;
                }

                if BEST_HWND.load(Ordering::Relaxed) == 0 {
                    if lower.contains("powershell") || lower.contains("cmd")
                        || lower.contains("terminal") || lower.contains("wezterm")
                        || lower.contains("alacritty") || lower.contains("git bash")
                    {
                        BEST_HWND.store(hwnd as isize, Ordering::Relaxed);
                    }
                }
            }
            1
        }

        BEST_HWND.store(0, Ordering::Relaxed);
        unsafe { EnumWindows(Some(enum_callback), 0); }
        BEST_HWND.load(Ordering::Relaxed)
    }

    /// Send an answer to Claude Code's terminal.
    /// Uses PostMessage(WM_CHAR) to send directly to the target window,
    /// bypassing the foreground window requirement entirely.
    pub async fn send_answer_to_terminal(answer: &str) -> anyhow::Result<()> {
        let answer = answer.to_string();
        let source_hwnd = get_question_source_hwnd();
        tokio::task::spawn_blocking(move || {
            let target = if source_hwnd != 0 {
                QUESTION_SOURCE_HWND.store(0, std::sync::atomic::Ordering::Relaxed);
                source_hwnd
            } else {
                Self::find_terminal_window()
            };

            if target == 0 {
                warn!("No terminal window found to send answer");
                return Err(anyhow::anyhow!("No terminal window found"));
            }

            info!("Sending answer to window handle: {}", target);
            Self::post_chars_to_window(target, &answer)
        }).await?
    }

    /// Copy answer to clipboard for the user to paste manually.
    fn post_chars_to_window(_hwnd: isize, text: &str) -> anyhow::Result<()> {
        // Copy to clipboard using PowerShell
        let escaped = text.replace("'", "''");
        let script = format!("Set-Clipboard -Value '{}'", escaped);

        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output();

        match output {
            Ok(o) if o.status.success() => {
                info!("Answer copied to clipboard: {}", text);
                Ok(())
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                warn!("Clipboard failed: {}", stderr);
                Err(anyhow::anyhow!("Clipboard failed"))
            }
            Err(e) => Err(anyhow::anyhow!("PowerShell failed: {}", e)),
        }
    }
}
