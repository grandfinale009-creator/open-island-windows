pub mod agent_event;
pub mod agent_session;
pub mod autostart;
pub mod bridge_server;
pub mod bridge_transport;
pub mod display_manager;
pub mod hook_manager;
pub mod native_window;
pub mod process_monitor;
pub mod risk_classification;
pub mod session_store;
pub mod terminal_jump;
pub mod transcript_monitor;
pub mod tray;

use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::{broadcast, Mutex};

use agent_event::AgentEvent;
use bridge_server::{ApprovalManager, BridgeServer};
use hook_manager::HookManager;
use session_store::SessionStore;

/// Auto-approve flag 闂?shared between Tauri commands and bridge server.
pub type AutoApprove = Arc<std::sync::atomic::AtomicBool>;

/// Tauri command: get current session state snapshot.
#[tauri::command]
async fn get_state(
    store: tauri::State<'_, Arc<Mutex<SessionStore>>>,
) -> Result<session_store::SessionStateSnapshot, String> {
    let store = store.lock().await;
    Ok(store.snapshot())
}

/// Tauri command: resolve a permission request.
#[tauri::command]
async fn resolve_permission(
    session_id: String,
    allow: bool,
    always: Option<bool>,
    store: tauri::State<'_, Arc<Mutex<SessionStore>>>,
    event_tx: tauri::State<'_, broadcast::Sender<AgentEvent>>,
    approval_manager: tauri::State<'_, Arc<ApprovalManager>>,
) -> Result<(), String> {
    let is_always = always.unwrap_or(false);
    tracing::info!("[PERM] resolve_permission called: session_id='{}', allow={}, always={}", session_id, allow, is_always);

    // Find the permission request ID and tool name for this session
    let (perm_id, tool_name) = {
        let store = store.lock().await;
        let session = store.state.sessions_by_id.get(&session_id);
        let perm = session.and_then(|s| s.permission_request.as_ref());
        (
            perm.map(|p| p.id.clone()),
            perm.and_then(|p| p.tool_name.clone()),
        )
    };

    // If "always allow" is selected, persist to Claude's settings.json
    if allow && is_always {
        if let Some(ref tool) = tool_name {
            match HookManager::add_allowed_tool(tool) {
                Ok(true) => tracing::info!("[PERM] Tool '{}' added to always-allow list", tool),
                Ok(false) => tracing::info!("[PERM] Tool '{}' already in always-allow list", tool),
                Err(e) => tracing::warn!("[PERM] Failed to add tool to always-allow: {}", e),
            }
        }
    }

    // Notify the approval manager (this unblocks the bridge server)
    if let Some(id) = perm_id {
        let decision = bridge_server::ApprovalDecision {
            allowed: allow,
            updated_permissions: None,
        };
        approval_manager.resolve(&id, decision).await;
        tracing::info!("[PERM] ApprovalManager resolved for perm_id='{}'", id);
    } else {
        tracing::warn!("[PERM] No permission request found for session '{}'", session_id);
    }

    // Update session state
    let mut store = store.lock().await;
    store.state.resolve_permission(&session_id, allow);
    drop(store);

    let _ = event_tx.send(AgentEvent::ActionableStateResolved(
        agent_event::ActionableStateResolved { session_id: session_id.clone() },
    ));
    tracing::info!("[PERM] Permission resolved for session '{}', allowed={}", session_id, allow);
    Ok(())
}

/// Tauri command: approve a permission request (convenience wrapper).
#[tauri::command]
async fn approve_permission(
    session_id: String,
    always: Option<bool>,
    store: tauri::State<'_, Arc<Mutex<SessionStore>>>,
    event_tx: tauri::State<'_, broadcast::Sender<AgentEvent>>,
    approval_manager: tauri::State<'_, Arc<ApprovalManager>>,
) -> Result<(), String> {
    resolve_permission(session_id, true, always, store, event_tx, approval_manager).await
}

/// Tauri command: deny a permission request (convenience wrapper).
#[tauri::command]
async fn deny_permission(
    session_id: String,
    store: tauri::State<'_, Arc<Mutex<SessionStore>>>,
    event_tx: tauri::State<'_, broadcast::Sender<AgentEvent>>,
    approval_manager: tauri::State<'_, Arc<ApprovalManager>>,
) -> Result<(), String> {
    resolve_permission(session_id, false, None, store, event_tx, approval_manager).await
}

/// Tauri command: toggle auto-approve mode.
#[tauri::command]
fn toggle_auto_approve(auto: tauri::State<'_, AutoApprove>) -> Result<bool, String> {
    let current = auto.load(std::sync::atomic::Ordering::Relaxed);
    let new_val = !current;
    auto.store(new_val, std::sync::atomic::Ordering::Relaxed);
    tracing::info!("[PERM] Auto-approve toggled: {}", new_val);
    Ok(new_val)
}

/// Tauri command: get auto-approve status.
#[tauri::command]
fn get_auto_approve(auto: tauri::State<'_, AutoApprove>) -> Result<bool, String> {
    Ok(auto.load(std::sync::atomic::Ordering::Relaxed))
}

/// Tauri command: get hook installation status.
#[tauri::command]
fn get_hook_status() -> Result<serde_json::Value, String> {
    let hooks = HookManager::new();
    Ok(serde_json::json!({
        "claude": hooks.is_claude_installed(),
        "codex": hooks.is_codex_installed(),
        "gemini": hooks.is_gemini_installed(),
        "cursor": hooks.is_cursor_installed(),
        "binary_path": hooks.binary_path().to_string_lossy(),
    }))
}

/// Tauri command: install hooks for a specific agent.
#[tauri::command]
fn install_hooks(agent: String) -> Result<String, String> {
    let hooks = HookManager::new();
    match agent.as_str() {
        "claude" => hooks.install_claude().map_err(|e| e.to_string()),
        "codex" => hooks.install_codex().map_err(|e| e.to_string()),
        "gemini" => hooks.install_gemini().map_err(|e| e.to_string()),
        "cursor" => hooks.install_cursor().map_err(|e| e.to_string()),
        _ => Err(format!("Unknown agent: {}", agent)),
    }?;
    Ok(format!("{} hooks installed", agent))
}

/// Tauri command: uninstall hooks for a specific agent.
#[tauri::command]
fn uninstall_hooks(agent: String) -> Result<String, String> {
    let hooks = HookManager::new();
    match agent.as_str() {
        "claude" => hooks.uninstall_claude().map_err(|e| e.to_string()),
        "codex" => hooks.uninstall_codex().map_err(|e| e.to_string()),
        "gemini" => hooks.uninstall_gemini().map_err(|e| e.to_string()),
        "cursor" => hooks.uninstall_cursor().map_err(|e| e.to_string()),
        _ => Err(format!("Unknown agent: {}", agent)),
    }?;
    Ok(format!("{} hooks uninstalled", agent))
}

/// Tauri command: get auto-start status.
#[tauri::command]
fn get_autostart_status() -> Result<bool, String> {
    Ok(autostart::AutoStartManager::is_enabled())
}

/// Tauri command: toggle auto-start.
#[tauri::command]
fn toggle_autostart() -> Result<bool, String> {
    autostart::AutoStartManager::toggle().map_err(|e| e.to_string())
}

/// Tauri command: resize the overlay window with native rounded corners.
#[tauri::command]
fn resize_window(app: tauri::AppHandle, state: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        native_window::setup_island_window(&window, &state)?;
    }
    Ok(())
}

/// Tauri command: set island width preset (compact/standard/wide).
#[tauri::command]
fn set_island_width(app: tauri::AppHandle, width: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let (w, h) = match width.as_str() {
            "compact" => (280, 36),
            "wide" => (380, 36),
            _ => (330, 36), // standard
        };
        // Only change width when collapsed
        let _ = window.set_size(tauri::LogicalSize::new(w, h));
    }
    Ok(())
}

/// Tauri command: answer a question from an agent.
#[tauri::command]
async fn answer_question(
    session_id: String,
    answer: String,
    store: tauri::State<'_, Arc<Mutex<SessionStore>>>,
    event_tx: tauri::State<'_, broadcast::Sender<AgentEvent>>,
) -> Result<(), String> {
    tracing::info!("[Q&A] answer_question called: session_id='{}', answer='{}'", session_id, answer);

    // Find the question ID for this session
    let question_id = {
        let store = store.lock().await;
        let session = store.state.sessions_by_id.get(&session_id);
        session.and_then(|s| s.question_prompt.as_ref().map(|q| q.title.clone().unwrap_or_default()))
    };

    // Update session state
    let mut store = store.lock().await;
    store.state.answer_question(&session_id);
    drop(store);

    let _ = event_tx.send(AgentEvent::ActionableStateResolved(
        agent_event::ActionableStateResolved { session_id: session_id.clone() },
    ));
    tracing::info!("[Q&A] Question answered for session '{}', answer='{}' (question was: {:?})", session_id, answer, question_id);
    Ok(())
}

/// Tauri command: jump to a terminal window.
#[tauri::command]
async fn jump_to_terminal(session_id: String, store: tauri::State<'_, Arc<Mutex<SessionStore>>>) -> Result<(), String> {
    let store = store.lock().await;
    let session = store.state.sessions_by_id.get(&session_id);
    if let Some(session) = session {
        // 1. Try terminal_app field (stores PID for process-monitored sessions)
        if let Some(ref app) = session.terminal_app {
            if let Ok(pid) = app.parse::<u32>() {
                terminal_jump::TerminalJumpService::jump_to_pid(pid)
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        // 2. Try jump_target from hooks
        if let Some(ref target) = session.jump_target {
            if let Some(ref tty) = target.terminal_tty {
                if let Ok(pid) = tty.parse::<u32>() {
                    terminal_jump::TerminalJumpService::jump_to_pid(pid)
                        .await
                        .map_err(|e| e.to_string())?;
                    return Ok(());
                }
            }
        }
        // 3. Fallback: jump by agent tool name
        let name = session.tool.short_name().to_lowercase();
        terminal_jump::TerminalJumpService::jump_to_process_name(&name)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Tauri command: dismiss/remove a session.
#[tauri::command]
async fn dismiss_session(
    session_id: String,
    store: tauri::State<'_, Arc<Mutex<SessionStore>>>,
    event_tx: tauri::State<'_, broadcast::Sender<AgentEvent>>,
) -> Result<(), String> {
    let mut store = store.lock().await;
    store.state.sessions_by_id.remove(&session_id);
    drop(store);
    let _ = event_tx.send(AgentEvent::SessionCompleted(
        agent_event::SessionCompleted {
            session_id,
            summary: None,
            timestamp: None,
        },
    ));
    Ok(())
}

/// Run the Tauri application.
pub fn run() {
    tracing_subscriber::fmt::init();

    let store = Arc::new(Mutex::new(SessionStore::new()));
    let (event_tx, _) = broadcast::channel::<AgentEvent>(64);
    let auto_approve: AutoApprove = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let approval_manager = Arc::new(ApprovalManager::new());

    let store_clone = Arc::clone(&store);
    let event_tx_clone = event_tx.clone();
    let auto_approve_clone = Arc::clone(&auto_approve);
    let approval_manager_clone = Arc::clone(&approval_manager);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(store.clone())
        .manage(event_tx.clone())
        .manage(auto_approve.clone())
        .manage(approval_manager.clone())
        .invoke_handler(tauri::generate_handler![
            get_state,
            resolve_permission,
            approve_permission,
            deny_permission,
            answer_question,
            get_hook_status,
            install_hooks,
            uninstall_hooks,
            get_autostart_status,
            toggle_autostart,
            jump_to_terminal,
            resize_window,
            set_island_width,
            dismiss_session,
            toggle_auto_approve,
            get_auto_approve,
            display_manager::list_available_displays,
            display_manager::set_preferred_display,
        ])
        .setup(move |app| {
            // Check if started with --silent flag (from auto-start)
            let is_silent = autostart::AutoStartManager::is_silent_startup();

            // Setup window with native Win32 rounded corners
            if let Some(window) = app.get_webview_window("main") {
                if let Err(e) = native_window::setup_island_window(&window, "compact") {
                    tracing::warn!("Native window setup failed: {}", e);
                }
                // Prevent island from stealing focus 闂?terminal stays in foreground
                if let Err(e) = native_window::apply_no_activate(&window) {
                    tracing::warn!("apply_no_activate failed: {}", e);
                }
                // Only show window if not silent startup
                if !is_silent {
                    let _ = window.show();
                }
            }

            // Start bridge server in background
            let store_for_bridge = Arc::clone(&store_clone);
            let event_tx_for_bridge = event_tx_clone.clone();
            let auto_approve_for_bridge = Arc::clone(&auto_approve_clone);
            let approval_for_bridge = Arc::clone(&approval_manager_clone);
            tauri::async_runtime::spawn(async move {
                let bridge = BridgeServer::new(store_for_bridge, event_tx_for_bridge)
                    .with_auto_approve(auto_approve_for_bridge)
                    .with_approval_manager(approval_for_bridge);
                if let Err(e) = bridge.start().await {
                    tracing::error!("Bridge server error: {}", e);
                }
            });

            // Start event forwarder: broadcast -> Tauri emit
            let app_handle = app.handle().clone();
            let mut event_rx = event_tx_clone.subscribe();
            let store_for_forwarder = Arc::clone(&store_clone);
            tauri::async_runtime::spawn(async move {
                loop {
                    match event_rx.recv().await {
                        Ok(_event) => {
                            let snapshot = {
                                let store = store_for_forwarder.lock().await;
                                store.snapshot()
                            };
                            let _ = app_handle.emit("state-changed", &snapshot);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });

            // Start process monitor
            let store_for_monitor = Arc::clone(&store_clone);
            let event_tx_for_monitor = event_tx_clone.clone();
            tauri::async_runtime::spawn(async move {
                let monitor = process_monitor::ProcessMonitor::new(store_for_monitor, event_tx_for_monitor);
                monitor.start().await;
            });

            // Start transcript monitor (detects AskUserQuestion in JSONL files)
            let store_for_transcript = Arc::clone(&store_clone);
            let event_tx_for_transcript = event_tx_clone.clone();
            tauri::async_runtime::spawn(async move {
                let monitor = transcript_monitor::TranscriptMonitor::new(store_for_transcript, event_tx_for_transcript);
                monitor.start().await;
            });

            // Setup system tray
            let _ = tray::setup_tray(app.handle());

            // Refresh autostart path (in case app was moved)
            if let Err(e) = autostart::AutoStartManager::refresh() {
                tracing::warn!("Failed to refresh autostart: {}", e);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
