use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, oneshot, Mutex};
use tracing::{info, warn};

use crate::agent_event::AgentEvent;
use crate::bridge_transport::{self, BridgeCommand, BridgeEnvelope, BridgeHello, BridgeResponse};
use crate::session_store::SessionStore;

/// Pending permission approval — uses oneshot channel for instant response.
struct PendingApproval {
    sender: oneshot::Sender<ApprovalDecision>,
}

#[derive(Clone)]
pub struct ApprovalDecision {
    pub allowed: bool,
    pub updated_permissions: Option<serde_json::Value>,
}

/// Manages pending permission approvals with oneshot channels.
#[derive(Default)]
pub struct ApprovalManager {
    pending: Mutex<HashMap<String, PendingApproval>>,
}

impl ApprovalManager {
    pub fn new() -> Self {
        Self { pending: Mutex::new(HashMap::new()) }
    }

    /// Register a pending approval and return a receiver that resolves when user responds.
    pub async fn wait_for_decision(&self, id: String) -> oneshot::Receiver<ApprovalDecision> {
        let (tx, rx) = oneshot::channel();
        let mut pending = self.pending.lock().await;
        pending.insert(id, PendingApproval { sender: tx });
        rx
    }

    /// Resolve a pending approval. Returns true if found.
    pub async fn resolve(&self, id: &str, decision: ApprovalDecision) -> bool {
        let mut pending = self.pending.lock().await;
        if let Some(item) = pending.remove(id) {
            let _ = item.sender.send(decision);
            true
        } else {
            false
        }
    }

    /// Auto-resolve all pending approvals for a session. Returns resolved IDs.
    async fn resolve_session(&self, session_id: &str, allowed: bool) -> Vec<String> {
        let mut pending = self.pending.lock().await;
        let ids: Vec<String> = pending.keys().cloned().collect();
        let mut resolved = Vec::new();
        // Note: we can't match by session_id here because the pending map uses approval ID as key.
        // The caller should pass the specific approval ID instead.
        drop(pending);
        // Resolve all — used when session ends or user approves via terminal
        for id in ids {
            if self.resolve(&id, ApprovalDecision { allowed, updated_permissions: None }).await {
                resolved.push(id);
            }
        }
        resolved
    }
}

/// Bridge server — TCP bridge + HTTP hooks server.
/// Inspired by CCIsland's HTTP hooks approach for Claude Code integration.
pub struct BridgeServer {
    store: Arc<Mutex<SessionStore>>,
    event_tx: broadcast::Sender<AgentEvent>,
    approval_manager: Arc<ApprovalManager>,
    auto_approve: Arc<std::sync::atomic::AtomicBool>,
}

impl BridgeServer {
    pub fn new(store: Arc<Mutex<SessionStore>>, event_tx: broadcast::Sender<AgentEvent>) -> Self {
        Self {
            store,
            event_tx,
            approval_manager: Arc::new(ApprovalManager::new()),
            auto_approve: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Set the auto_approve flag (shared with Tauri state).
    pub fn with_auto_approve(mut self, auto: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.auto_approve = auto;
        self
    }

    /// Set the approval_manager (shared with Tauri state).
    pub fn with_approval_manager(mut self, manager: Arc<ApprovalManager>) -> Self {
        self.approval_manager = manager;
        self
    }

    /// Start the bridge server. Runs indefinitely.
    pub async fn start(&self) -> anyhow::Result<()> {
        // Start TCP bridge server
        let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:19841").await?;
        info!("TCP bridge listening on 127.0.0.1:19841");

        // Start HTTP hooks server (for Claude Code HTTP hooks)
        let http_listener = tokio::net::TcpListener::bind("127.0.0.1:51515").await?;
        info!("HTTP hooks server listening on 127.0.0.1:51515");

        // Spawn HTTP server
        let store_http = Arc::clone(&self.store);
        let event_tx_http = self.event_tx.clone();
        let approval_http = Arc::clone(&self.approval_manager);
        let auto_approve_http = Arc::clone(&self.auto_approve);
        tokio::spawn(async move {
            loop {
                if let Ok((stream, _)) = http_listener.accept().await {
                    let store = Arc::clone(&store_http);
                    let event_tx = event_tx_http.clone();
                    let approval = Arc::clone(&approval_http);
                    let auto = Arc::clone(&auto_approve_http);
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_http_hook(stream, store, event_tx, approval, auto).await {
                            warn!("HTTP hook error: {}", e);
                        }
                    });
                }
            }
        });

        // Spawn stale session cleanup task (every 30 seconds)
        let store_cleanup = Arc::clone(&self.store);
        let event_tx_cleanup = self.event_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let mut store = store_cleanup.lock().await;

                // Auto-complete stale hook-managed sessions (no activity for 3+ minutes)
                let completed_ids = store.state.auto_complete_stale_sessions();
                for id in &completed_ids {
                    info!("Auto-completed stale session: {}", id);
                    let _ = event_tx_cleanup.send(AgentEvent::SessionCompleted(
                        crate::agent_event::SessionCompleted {
                            session_id: id.clone(),
                            summary: Some("Session ended (no activity)".to_string()),
                            timestamp: None,
                        }
                    ));
                }

                // Remove very stale sessions (no activity for 10+ minutes)
                let count_before = store.state.sessions_by_id.len();
                store.state.remove_invisible_sessions();
                let removed = store.state.sessions_by_id.len() < count_before;

                if !completed_ids.is_empty() || removed {
                    store.save_to_disk();
                    // Trigger frontend update
                    let _ = event_tx_cleanup.send(AgentEvent::ActionableStateResolved(
                        crate::agent_event::ActionableStateResolved {
                            session_id: "_cleanup".to_string(),
                        }
                    ));
                }
            }
        });

        // Run TCP bridge server
        loop {
            let (stream, addr) = tcp_listener.accept().await?;
            info!("TCP bridge client connected from {}", addr);

            let store = Arc::clone(&self.store);
            let event_tx = self.event_tx.clone();
            let approval = Arc::clone(&self.approval_manager);
            let auto = Arc::clone(&self.auto_approve);

            tokio::spawn(async move {
                if let Err(e) = Self::handle_client(stream, store, event_tx, approval, auto).await {
                    warn!("Client connection error: {}", e);
                }
            });
        }
    }

    /// Handle HTTP hook from Claude Code.
    /// Claude Code sends POST to http://localhost:51515/hooks/<event-name> with JSON body.
    /// For PermissionRequest, uses oneshot channel for instant response when user approves.
    async fn handle_http_hook(
        mut stream: tokio::net::TcpStream,
        store: Arc<Mutex<SessionStore>>,
        event_tx: broadcast::Sender<AgentEvent>,
        approval_manager: Arc<ApprovalManager>,
        auto_approve: Arc<std::sync::atomic::AtomicBool>,
    ) -> anyhow::Result<()> {
        // Read HTTP headers first
        let mut header_buf = Vec::new();
        let mut temp = [0u8; 4096];
        loop {
            let n = stream.read(&mut temp).await?;
            if n == 0 { return Ok(()); }
            header_buf.extend_from_slice(&temp[..n]);
            if header_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if header_buf.len() > 16384 {
                return Ok(());
            }
        }

        let request = String::from_utf8_lossy(&header_buf);

        let first_line = request.lines().next().unwrap_or("");
        let path = first_line.split_whitespace().nth(1).unwrap_or("/");

        let content_length: usize = request.lines()
            .find(|l| l.to_lowercase().starts_with("content-length:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);

        let header_end = request.find("\r\n\r\n").unwrap_or(0) + 4;
        let mut body_bytes = header_buf[header_end..].to_vec();

        while body_bytes.len() < content_length {
            let mut chunk = vec![0u8; 8192];
            let n = stream.read(&mut chunk).await?;
            if n == 0 { break; }
            body_bytes.extend_from_slice(&chunk[..n]);
        }

        let body = String::from_utf8_lossy(&body_bytes);

        let payload: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                warn!("Invalid HTTP hook body: {}", e);
                let response = "HTTP/1.1 400 Bad Request\r\n\r\n";
                stream.write_all(response.as_bytes()).await?;
                return Ok(());
            }
        };

        tracing::info!("HTTP hook: path={} body_len={}", path, body.len());

        let hook_event = match path {
            "/hooks/session-start" => "sessionStart",
            "/hooks/pre-tool-use" => "preToolUse",
            "/hooks/post-tool-use" => "postToolUse",
            "/hooks/post-tool-use-failure" => "postToolUseFailure",
            "/hooks/stop" => "stop",
            "/hooks/stop-failure" => "stopFailure",
            "/hooks/notification" => "notification",
            "/hooks/permission-request" => "permissionRequest",
            "/hooks/user-prompt-submit" => "userPromptSubmit",
            _ => {
                tracing::debug!("Unknown hook path: {}", path);
                let response = "HTTP/1.1 404 Not Found\r\n\r\n";
                stream.write_all(response.as_bytes()).await?;
                return Ok(());
            }
        };

        let session_id = payload.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or(".").to_string();
        let tool_name = payload.get("tool_name").and_then(|v| v.as_str()).map(|s| s.to_string());
        let tool_use_id = payload.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

        tracing::debug!("HTTP hook: {} path={} session={}", hook_event, path, session_id);

        let event = match hook_event {
            "preToolUse" => {
                Some(AgentEvent::ActivityUpdated(crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: Some(crate::agent_event::SessionPhase::Running),
                    summary: None,
                    tool: tool_name,
                    timestamp: None,
                }))
            }
            "sessionStart" => Some(AgentEvent::SessionStarted(crate::agent_event::SessionStarted {
                session_id: session_id.clone(),
                title: payload.get("title").and_then(|v| v.as_str()).unwrap_or("Claude Code").to_string(),
                tool: crate::agent_event::AgentTool::ClaudeCode,
                origin: Some(cwd),
                initial_phase: None,
                summary: None,
                timestamp: None,
                jump_target: None,
                is_remote: None,
            })),
            "sessionEnd" => Some(AgentEvent::SessionCompleted(crate::agent_event::SessionCompleted {
                session_id: session_id.clone(),
                summary: None,
                timestamp: None,
            })),
            "stop" | "stopFailure" => {
                // stop = turn completed (agent finished responding), NOT session ended.
                // Don't change phase — just clear current tool and update summary.
                Some(AgentEvent::ActivityUpdated(crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: None, // Keep current phase
                    summary: payload.get("lastAssistantMessage").and_then(|v| v.as_str()).map(|s| {
                        let s = s.trim();
                        if s.chars().count() > 120 { format!("{}…", s.chars().take(120).collect::<String>()) } else { s.to_string() }
                    }),
                    tool: Some(String::new()), // Empty string = clear current tool
                    timestamp: None,
                }))
            }
            "userPromptSubmit" => Some(AgentEvent::ActivityUpdated(crate::agent_event::SessionActivityUpdated {
                session_id: session_id.clone(),
                phase: Some(crate::agent_event::SessionPhase::Running),
                summary: payload.get("prompt").and_then(|v| v.as_str()).map(|s| s.to_string()),
                tool: None,
                timestamp: None,
            })),
            "permissionRequest" => {
                tracing::info!("[HOOK] PermissionRequest received, session={}, tool={}",
                    session_id, tool_name.as_deref().unwrap_or("unknown"));

                // Auto-approve if global toggle enabled (skip UI)
                if auto_approve.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::info!("[HOOK] Auto-approve enabled, auto-allowing tool={:?}", tool_name);
                    Self::send_http_response(&mut stream, true, None).await?;
                    return Ok(());
                }

                // Risk-based auto-approve: safe tools (Read, Grep, Glob) auto-approved
                let tool_input_val = payload.get("tool_input").cloned();
                let risk = crate::risk_classification::assess_risk(
                    tool_name.as_deref().unwrap_or("unknown"),
                    tool_input_val.as_ref(),
                );
                if risk.auto_approved {
                    tracing::info!("[HOOK] Safe tool auto-approved: {} ({})", tool_name.as_deref().unwrap_or("?"), risk.label);
                    Self::send_http_response(&mut stream, true, None).await?;
                    return Ok(());
                }

                let perm_id = if tool_use_id == "unknown" {
                    format!("perm-{}", chrono::Utc::now().timestamp_millis())
                } else {
                    tool_use_id.clone()
                };

                // Extract permission_suggestions for "always allow" support
                let permission_suggestions = payload.get("permission_suggestions").cloned();

                // Use risk info for better title
                let title = format!("Allow {} [{}]", tool_name.as_deref().unwrap_or("tool"), risk.label);

                // Generate human-readable description
                let description = crate::risk_classification::describe_tool_input(
                    tool_name.as_deref().unwrap_or("unknown"),
                    tool_input_val.as_ref(),
                );

                let perm_event = AgentEvent::PermissionRequested(crate::agent_event::PermissionRequested {
                    session_id: session_id.clone(),
                    permission: crate::agent_event::PermissionRequest {
                        id: perm_id.clone(),
                        title: Some(title),
                        summary: Some(description),
                        affected_path: crate::risk_classification::extract_path_from_input(tool_input_val.as_ref()),
                        primary_action_title: Some("Allow".to_string()),
                        secondary_action_title: Some("Deny".to_string()),
                        tool_name: tool_name.clone(),
                        tool_use_id: Some(tool_use_id.clone()),
                        permission_suggestions,
                    },
                    timestamp: None,
                });
                {
                    let mut store = store.lock().await;
                    store.state.apply(&perm_event);
                }
                let _ = event_tx.send(perm_event);

                // Wait for user decision via oneshot channel (24h timeout)
                let receiver = approval_manager.wait_for_decision(perm_id.clone()).await;
                let decision = tokio::time::timeout(
                    tokio::time::Duration::from_secs(86400),
                    receiver,
                ).await;

                let (approved, updated_perms) = match decision {
                    Ok(Ok(d)) => (d.allowed, d.updated_permissions),
                    _ => (false, None), // Timeout or channel dropped
                };

                // Clear permission state in store
                {
                    let mut store = store.lock().await;
                    store.state.resolve_permission(&session_id, approved);
                }
                let _ = event_tx.send(AgentEvent::ActionableStateResolved(
                    crate::agent_event::ActionableStateResolved { session_id: session_id.clone() },
                ));

                tracing::info!("[HOOK] PermissionRequest decision: approved={}, session={}", approved, session_id);
                Self::send_http_response(&mut stream, approved, updated_perms).await?;
                return Ok(());
            }
            "notification" => {
                // Claude Desktop sends notification hooks when asking questions.
                // Detect question-like notifications and emit QuestionAsked event.
                let message = payload.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let msg_lower = message.to_lowercase();

                // Check if this notification looks like a user-facing question
                if msg_lower.contains('?') || msg_lower.contains("select")
                    || msg_lower.contains("choose") || msg_lower.contains("confirm")
                    || msg_lower.contains("approve") || msg_lower.contains("answer")
                {
                    tracing::info!("[HOOK] Detected question-like notification from Claude Desktop: {}", message);
                    Some(AgentEvent::QuestionAsked(crate::agent_event::QuestionAsked {
                        session_id: session_id.clone(),
                        question: crate::agent_event::QuestionPrompt {
                            title: Some(message.to_string()),
                            questions: vec![crate::agent_event::QuestionPromptItem {
                                question: message.to_string(),
                                header: None,
                                options: vec![],
                                multi_select: false,
                            }],
                        },
                        timestamp: None,
                        source: Some("desktop".to_string()),
                    }))
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(event) = event {
            let mut store = store.lock().await;
            store.state.apply(&event);
            // Mark HTTP-hook-created sessions as hook-managed (prevents process monitor cleanup)
            if let AgentEvent::SessionStarted(ref e) = event {
                if let Some(session) = store.state.sessions_by_id.get_mut(&e.session_id) {
                    session.is_hook_managed = true;
                }
            }
            store.save_to_disk();
            drop(store);
            let _ = event_tx.send(event);
        }

        // Non-blocking response for all events except permissionRequest
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        stream.write_all(response.as_bytes()).await?;

        Ok(())
    }

    /// Send the PermissionRequest HTTP response in Claude Code's expected format.
    async fn send_http_response(
        stream: &mut tokio::net::TcpStream,
        approved: bool,
        updated_permissions: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        let response_payload = if approved {
            let mut decision = serde_json::json!({
                "behavior": "allow"
            });
            if let Some(perms) = updated_permissions {
                decision["updatedPermissions"] = perms;
            } else {
                decision["updatedPermissions"] = serde_json::json!([]);
            }
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": decision
                }
            })
        } else {
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {
                        "behavior": "deny",
                        "message": "Denied by user in Open Island",
                        "interrupt": false
                    }
                }
            })
        };
        let body = serde_json::to_string(&response_payload)?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(), body
        );
        stream.write_all(response.as_bytes()).await?;
        Ok(())
    }

    async fn handle_client(
        stream: tokio::net::TcpStream,
        store: Arc<Mutex<SessionStore>>,
        event_tx: broadcast::Sender<AgentEvent>,
        approval_manager: Arc<ApprovalManager>,
        auto_approve: Arc<std::sync::atomic::AtomicBool>,
    ) -> anyhow::Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Send hello
        let hello = BridgeEnvelope::Hello(BridgeHello {
            protocol_version: 1,
            server_label: Some("open-island-windows".to_string()),
        });
        let hello_bytes = bridge_transport::encode_line(&hello)?;
        writer.write_all(&hello_bytes).await?;

        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line).await?;
            if bytes_read == 0 {
                break; // client disconnected
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let envelope: BridgeEnvelope = match serde_json::from_str(trimmed) {
                Ok(env) => env,
                Err(e) => {
                    warn!("Invalid envelope: {}", e);
                    continue;
                }
            };

            match envelope {
                BridgeEnvelope::Command(cmd) => {
                    let response = Self::process_command(cmd, &store, &event_tx, &approval_manager, &auto_approve).await;
                    let resp_bytes = bridge_transport::encode_line(&response)?;
                    writer.write_all(&resp_bytes).await?;
                }
                _ => {
                    // Ignore non-command envelopes from clients
                }
            }
        }

        Ok(())
    }

    async fn process_command(
        cmd: BridgeCommand,
        store: &Arc<Mutex<SessionStore>>,
        event_tx: &broadcast::Sender<AgentEvent>,
        approval_manager: &Arc<ApprovalManager>,
        _auto_approve: &Arc<std::sync::atomic::AtomicBool>,
    ) -> BridgeEnvelope {
        match cmd {
            BridgeCommand::ProcessClaudeHook { payload } => {
                Self::handle_claude_hook(payload, store, event_tx).await
            }
            BridgeCommand::ProcessCodexHook { payload } => {
                Self::handle_codex_hook(payload, store, event_tx).await
            }
            BridgeCommand::ProcessOpenCodeHook { payload } => {
                Self::handle_opencode_hook(payload, store, event_tx).await
            }
            BridgeCommand::ProcessCursorHook { payload } => {
                Self::handle_cursor_hook(payload, store, event_tx).await
            }
            BridgeCommand::ProcessGeminiHook { payload } => {
                Self::handle_gemini_hook(payload, store, event_tx).await
            }
            BridgeCommand::ResolvePermission {
                session_id,
                resolution,
            } => {
                let allowed = match resolution {
                    crate::bridge_transport::PermissionResolution::Allow => true,
                    crate::bridge_transport::PermissionResolution::AllowAlways => true,
                    crate::bridge_transport::PermissionResolution::Deny { .. } => false,
                };
                let updated_permissions = match &resolution {
                    crate::bridge_transport::PermissionResolution::AllowAlways => {
                        Some(serde_json::json!([]))
                    }
                    _ => None,
                };

                // Resolve via approval manager (for HTTP hook path)
                let store_snapshot = store.lock().await;
                let approval_id = store_snapshot.state.sessions_by_id.get(&session_id)
                    .and_then(|s| s.permission_request.as_ref().map(|p| p.id.clone()));
                drop(store_snapshot);

                if let Some(id) = approval_id {
                    approval_manager.resolve(&id, ApprovalDecision {
                        allowed,
                        updated_permissions,
                    }).await;
                }

                // Also update store directly (for bridge transport path)
                let mut store = store.lock().await;
                store.state.resolve_permission(&session_id, allowed);
                drop(store);
                let _ = event_tx.send(AgentEvent::ActionableStateResolved(
                    crate::agent_event::ActionableStateResolved {
                        session_id: session_id.clone(),
                    },
                ));
                BridgeEnvelope::Response(BridgeResponse::Acknowledged)
            }
            BridgeCommand::AnswerQuestion { session_id, .. } => {
                let mut store = store.lock().await;
                store.state.answer_question(&session_id);
                drop(store);
                let _ = event_tx.send(AgentEvent::ActionableStateResolved(
                    crate::agent_event::ActionableStateResolved {
                        session_id: session_id.clone(),
                    },
                ));
                BridgeEnvelope::Response(BridgeResponse::Acknowledged)
            }
            BridgeCommand::RegisterClient => {
                BridgeEnvelope::Response(BridgeResponse::Acknowledged)
            }
        }
    }

    /// Handle Claude Code hook payloads.
    /// Claude hooks are the most complex — 14 event types.
    async fn handle_claude_hook(
        payload: serde_json::Value,
        store: &Arc<Mutex<SessionStore>>,
        event_tx: &broadcast::Sender<AgentEvent>,
    ) -> BridgeEnvelope {
        let hook_event = payload
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cwd = payload
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();
        let tool_name = payload
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        tracing::debug!("Claude hook: {} session={}", hook_event, session_id);

        let event = match hook_event {
            "sessionStart" => {
                let title = payload
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Claude Session")
                    .to_string();
                Some(AgentEvent::SessionStarted(
                    crate::agent_event::SessionStarted {
                        session_id: session_id.clone(),
                        title,
                        tool: crate::agent_event::AgentTool::ClaudeCode,
                        origin: Some(cwd),
                        initial_phase: None,
                        summary: None,
                        timestamp: None,
                        jump_target: None,
                        is_remote: None,
                    },
                ))
            }
            "sessionEnd" => Some(AgentEvent::SessionCompleted(
                crate::agent_event::SessionCompleted {
                    session_id: session_id.clone(),
                    summary: None,
                    timestamp: None,
                },
            )),
            "stop" | "stopFailure" => Some(AgentEvent::ActivityUpdated(
                crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: None, // Keep current phase — stop = turn completed, not session ended
                    summary: payload
                        .get("lastAssistantMessage")
                        .and_then(|v| v.as_str())
                        .map(|s| {
                            let s = s.trim();
                            if s.chars().count() > 120 { format!("{}…", s.chars().take(120).collect::<String>()) } else { s.to_string() }
                        }),
                    tool: Some(String::new()), // Clear current tool
                    timestamp: None,
                },
            )),
            "preToolUse" => Some(AgentEvent::ActivityUpdated(
                crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: Some(crate::agent_event::SessionPhase::Running),
                    summary: None,
                    tool: tool_name,
                    timestamp: None,
                },
            )),
            "postToolUse" | "postToolUseFailure" => {
                // Update tool activity — keep session running but clear current tool
                Some(AgentEvent::ActivityUpdated(crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: None, // Don't change phase
                    summary: payload.get("tool_response").and_then(|v| v.as_str()).map(|s| {
                        let s = s.trim();
                        if s.chars().count() > 100 { format!("{}…", s.chars().take(100).collect::<String>()) } else { s.to_string() }
                    }),
                    tool: None, // Clear current tool
                    timestamp: None,
                }))
            }
            "userPromptSubmit" => Some(AgentEvent::ActivityUpdated(
                crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: Some(crate::agent_event::SessionPhase::Running),
                    summary: payload
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    tool: None,
                    timestamp: None,
                },
            )),
            "permissionRequest" => {
                let perm_id = payload
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let perm_title = format!(
                    "Allow {}",
                    tool_name.as_deref().unwrap_or("tool")
                );
                Some(AgentEvent::PermissionRequested(
                    crate::agent_event::PermissionRequested {
                        session_id: session_id.clone(),
                        permission: crate::agent_event::PermissionRequest {
                            id: perm_id,
                            title: Some(perm_title),
                            summary: payload
                                .get("tool_input")
                                .map(|v| v.to_string()),
                            affected_path: None,
                            primary_action_title: Some("Allow".to_string()),
                            secondary_action_title: Some("Deny".to_string()),
                            tool_name,
                            tool_use_id: None,
                            permission_suggestions: None,
                        },
                        timestamp: None,
                    },
                ))
            }
            "subagentStart" | "subagentStop" | "notification" | "preCompact" => None,
            _ => None,
        };

        if let Some(event) = event {
            let mut store = store.lock().await;
            store.state.apply(&event);
            if let AgentEvent::SessionStarted(ref e) = event {
                if let Some(session) = store.state.sessions_by_id.get_mut(&e.session_id) {
                    session.is_hook_managed = true;
                }
            }
            store.save_to_disk();
            drop(store);
            let _ = event_tx.send(event);
        }

        BridgeEnvelope::Response(BridgeResponse::Acknowledged)
    }

    async fn handle_codex_hook(
        payload: serde_json::Value,
        store: &Arc<Mutex<SessionStore>>,
        event_tx: &broadcast::Sender<AgentEvent>,
    ) -> BridgeEnvelope {
        let hook_event = payload
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cwd = payload
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();

        let event = match hook_event {
            "SessionStart" => Some(AgentEvent::SessionStarted(
                crate::agent_event::SessionStarted {
                    session_id: session_id.clone(),
                    title: "Codex Session".to_string(),
                    tool: crate::agent_event::AgentTool::Codex,
                    origin: Some(cwd),
                    initial_phase: None,
                    summary: None,
                    timestamp: None,
                    jump_target: None,
                    is_remote: None,
                },
            )),
            "Stop" => Some(AgentEvent::SessionCompleted(
                crate::agent_event::SessionCompleted {
                    session_id: session_id.clone(),
                    summary: payload
                        .get("lastAssistantMessage")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    timestamp: None,
                },
            )),
            "PreToolUse" => Some(AgentEvent::ActivityUpdated(
                crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: Some(crate::agent_event::SessionPhase::Running),
                    summary: None,
                    tool: payload
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    timestamp: None,
                },
            )),
            "PostToolUse" => None,
            "UserPromptSubmit" => Some(AgentEvent::ActivityUpdated(
                crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: Some(crate::agent_event::SessionPhase::Running),
                    summary: payload
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    tool: None,
                    timestamp: None,
                },
            )),
            _ => None,
        };

        if let Some(event) = event {
            let mut store = store.lock().await;
            store.state.apply(&event);
            if let AgentEvent::SessionStarted(ref e) = event {
                if let Some(session) = store.state.sessions_by_id.get_mut(&e.session_id) {
                    session.is_hook_managed = true;
                }
            }
            store.save_to_disk();
            drop(store);
            let _ = event_tx.send(event);
        }

        BridgeEnvelope::Response(BridgeResponse::Acknowledged)
    }

    async fn handle_opencode_hook(
        payload: serde_json::Value,
        store: &Arc<Mutex<SessionStore>>,
        event_tx: &broadcast::Sender<AgentEvent>,
    ) -> BridgeEnvelope {
        // OpenCode uses a nested structure: { "openCodeHook": { ... } }
        let hook = payload.get("openCodeHook").unwrap_or(&payload);
        let hook_event = hook
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let session_id = hook
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cwd = hook
            .get("cwd")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();

        let event = match hook_event {
            "SessionStart" => Some(AgentEvent::SessionStarted(
                crate::agent_event::SessionStarted {
                    session_id: session_id.clone(),
                    title: "OpenCode Session".to_string(),
                    tool: crate::agent_event::AgentTool::OpenCode,
                    origin: Some(cwd),
                    initial_phase: None,
                    summary: None,
                    timestamp: None,
                    jump_target: None,
                    is_remote: None,
                },
            )),
            "SessionEnd" => Some(AgentEvent::SessionCompleted(
                crate::agent_event::SessionCompleted {
                    session_id: session_id.clone(),
                    summary: None,
                    timestamp: None,
                },
            )),
            "Stop" => Some(AgentEvent::SessionCompleted(
                crate::agent_event::SessionCompleted {
                    session_id: session_id.clone(),
                    summary: hook
                        .get("last_assistant_message")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    timestamp: None,
                },
            )),
            "PreToolUse" => Some(AgentEvent::ActivityUpdated(
                crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: Some(crate::agent_event::SessionPhase::Running),
                    summary: None,
                    tool: hook
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    timestamp: None,
                },
            )),
            "PostToolUse" => None,
            "UserPromptSubmit" => Some(AgentEvent::ActivityUpdated(
                crate::agent_event::SessionActivityUpdated {
                    session_id: session_id.clone(),
                    phase: Some(crate::agent_event::SessionPhase::Running),
                    summary: hook
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    tool: None,
                    timestamp: None,
                },
            )),
            "PermissionRequest" => {
                let perm_id = hook
                    .get("permission_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let tool = hook
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let title = hook
                    .get("permission_title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Permission requested")
                    .to_string();
                Some(AgentEvent::PermissionRequested(
                    crate::agent_event::PermissionRequested {
                        session_id: session_id.clone(),
                        permission: crate::agent_event::PermissionRequest {
                            id: perm_id,
                            title: Some(title),
                            summary: hook
                                .get("tool_input")
                                .map(|v| v.to_string()),
                            affected_path: None,
                            primary_action_title: Some("Allow".to_string()),
                            secondary_action_title: Some("Deny".to_string()),
                            tool_name: tool,
                            tool_use_id: None,
                            permission_suggestions: None,
                        },
                        timestamp: None,
                    },
                ))
            }
            "QuestionAsked" => {
                let q_text = hook
                    .get("question_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Question")
                    .to_string();
                Some(AgentEvent::QuestionAsked(
                    crate::agent_event::QuestionAsked {
                        session_id: session_id.clone(),
                        question: crate::agent_event::QuestionPrompt {
                            title: Some(q_text),
                            questions: vec![],
                        },
                        timestamp: None,
                        source: Some("cli".to_string()),
                    },
                ))
            }
            _ => None,
        };

        if let Some(event) = event {
            let mut store = store.lock().await;
            store.state.apply(&event);
            if let AgentEvent::SessionStarted(ref e) = event {
                if let Some(session) = store.state.sessions_by_id.get_mut(&e.session_id) {
                    session.is_hook_managed = true;
                }
            }
            store.save_to_disk();
            drop(store);
            let _ = event_tx.send(event);
        }

        // For permission/question requests, we'd need to hold the connection.
        // For now, just acknowledge.
        BridgeEnvelope::Response(BridgeResponse::Acknowledged)
    }

    async fn handle_cursor_hook(
        _payload: serde_json::Value,
        _store: &Arc<Mutex<SessionStore>>,
        _event_tx: &broadcast::Sender<AgentEvent>,
    ) -> BridgeEnvelope {
        // TODO: Implement Cursor hook handling
        BridgeEnvelope::Response(BridgeResponse::Acknowledged)
    }

    async fn handle_gemini_hook(
        _payload: serde_json::Value,
        _store: &Arc<Mutex<SessionStore>>,
        _event_tx: &broadcast::Sender<AgentEvent>,
    ) -> BridgeEnvelope {
        // TODO: Implement Gemini hook handling
        BridgeEnvelope::Response(BridgeResponse::Acknowledged)
    }
}
